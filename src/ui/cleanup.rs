// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::TryRecvError,
    time::{Duration, SystemTime},
};

use gtk::{glib, prelude::*};

use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{Confidence, ScanEvent, ScanHandle, ScanResult, default_options, start_scan},
};

use super::browser::format_file_size;

const PANE_HEIGHT: i32 = 280;
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    Artifacts,
    Duplicates,
}

#[derive(Clone, Copy, Debug)]
enum Row {
    Artifact(usize),
    DupHeader(usize),
    DupFile { group: usize, file: usize },
}

struct CleanupState {
    widget: gtk::Box,
    split: RefCell<Option<gtk::Paned>>,
    status: gtk::Label,
    spinner: gtk::Spinner,
    cancel: gtk::Button,
    artifacts_tab: gtk::ToggleButton,
    duplicates_tab: gtk::ToggleButton,
    confidence: gtk::DropDown,
    filter: gtk::Entry,
    list: gtk::ListView,
    model: gtk::StringList,
    footer: gtk::Label,
    keep_newest: gtk::Button,
    trash: gtk::Button,
    tab: Cell<Tab>,
    min_confidence: Cell<Confidence>,
    scan: RefCell<Option<ScanHandle>>,
    generation: Cell<u64>,
    result: RefCell<Option<ScanResult>>,
    rows: RefCell<Vec<Row>>,
    selected: RefCell<HashSet<PathBuf>>,
    pending_trash: RefCell<HashSet<PathBuf>>,
    syncing: Cell<bool>,
    reveal: Rc<dyn Fn(PathBuf, bool)>,
    trash_items: Rc<dyn Fn(Vec<FileEntry>)>,
}

#[derive(Clone)]
pub struct CleanupPane {
    state: Rc<CleanupState>,
}

impl CleanupPane {
    pub fn new(reveal: Rc<dyn Fn(PathBuf, bool)>, trash_items: Rc<dyn Fn(Vec<FileEntry>)>) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("cleanup-pane");
        widget.set_hexpand(true);
        widget.set_visible(false);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("cleanup-header");
        header.append(&crate::assets::primary_icon(
            crate::assets::icons::ERASER,
            16,
        ));
        let title = gtk::Label::new(Some("Cleanup"));
        title.add_css_class("cleanup-title");
        title.set_xalign(0.0);
        header.append(&title);
        let spinner = gtk::Spinner::new();
        spinner.add_css_class("cleanup-spinner");
        header.append(&spinner);
        let status = gtk::Label::new(None);
        status.add_css_class("cleanup-status");
        status.set_hexpand(true);
        status.set_xalign(0.0);
        status.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        header.append(&status);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.add_css_class("cleanup-header-button");
        cancel.set_visible(false);
        header.append(&cancel);
        let close = gtk::Button::builder()
            .tooltip_text("Close cleanup")
            .has_frame(false)
            .build();
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            16,
        )));
        close.add_css_class("cleanup-header-button");
        header.append(&close);
        widget.append(&header);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toolbar.add_css_class("cleanup-toolbar");
        let artifacts_tab = gtk::ToggleButton::with_label("Artifacts");
        artifacts_tab.add_css_class("cleanup-tab");
        artifacts_tab.set_active(true);
        let duplicates_tab = gtk::ToggleButton::with_label("Duplicates");
        duplicates_tab.add_css_class("cleanup-tab");
        duplicates_tab.set_group(Some(&artifacts_tab));
        toolbar.append(&artifacts_tab);
        toolbar.append(&duplicates_tab);
        let confidence = gtk::DropDown::from_strings(&["High only", "High + medium", "Everything"]);
        confidence.set_selected(1);
        confidence.add_css_class("cleanup-filter");
        toolbar.append(&confidence);
        let filter = gtk::Entry::builder()
            .placeholder_text("Filter paths…")
            .hexpand(true)
            .build();
        filter.add_css_class("cleanup-filter-entry");
        toolbar.append(&filter);
        widget.append(&toolbar);

        let model = gtk::StringList::new(&[]);
        let selection = gtk::NoSelection::new(Some(model.clone()));
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::new(Some(selection), Some(factory.clone()));
        list.add_css_class("cleanup-list");
        list.set_single_click_activate(true);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list)
            .build();
        scroller.add_css_class("cleanup-scroll");
        widget.append(&scroller);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("cleanup-footer");
        let summary = gtk::Label::new(Some("Select items to move to Trash"));
        summary.add_css_class("cleanup-summary");
        summary.set_hexpand(true);
        summary.set_xalign(0.0);
        footer.append(&summary);
        let keep_newest = gtk::Button::with_label("Keep newest");
        keep_newest.add_css_class("cleanup-footer-button");
        keep_newest.set_tooltip_text(Some(
            "Select every duplicate except the newest copy in each set",
        ));
        keep_newest.set_visible(false);
        footer.append(&keep_newest);
        let trash = gtk::Button::with_label("Move to Trash");
        trash.add_css_class("cleanup-footer-button");
        trash.set_sensitive(false);
        footer.append(&trash);
        widget.append(&footer);

        let state = Rc::new(CleanupState {
            widget,
            split: RefCell::new(None),
            status,
            spinner,
            cancel,
            artifacts_tab,
            duplicates_tab,
            confidence,
            filter,
            list,
            model,
            footer: summary,
            keep_newest,
            trash,
            tab: Cell::new(Tab::Artifacts),
            min_confidence: Cell::new(Confidence::Medium),
            scan: RefCell::new(None),
            generation: Cell::new(0),
            result: RefCell::new(None),
            rows: RefCell::new(Vec::new()),
            selected: RefCell::new(HashSet::new()),
            pending_trash: RefCell::new(HashSet::new()),
            syncing: Cell::new(false),
            reveal,
            trash_items,
        });

        bind_factory(&factory, &state);
        connect_controls(&state, &close);
        let activated = Rc::downgrade(&state);
        state.list.connect_activate(move |_, position| {
            if let Some(state) = activated.upgrade() {
                activate_row(&state, position);
            }
        });

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.widget.clone().upcast()
    }

    pub fn attach_split(&self, split: &gtk::Paned) {
        self.state.split.replace(Some(split.clone()));
    }

    pub fn scan(&self, root: PathBuf) {
        if !root.is_dir() {
            return;
        }
        show_pane(&self.state);
        self.state.generation.set(self.state.generation.get() + 1);
        let generation = self.state.generation.get();
        self.state.scan.borrow_mut().take();
        self.state.result.replace(None);
        self.state.selected.borrow_mut().clear();
        self.state.pending_trash.borrow_mut().clear();
        rebuild_rows(&self.state);
        self.state.spinner.set_visible(true);
        self.state.spinner.start();
        self.state.cancel.set_visible(true);
        self.state
            .status
            .set_text(&format!("Scanning {}", root.display()));

        let (handle, receiver) = start_scan(default_options(root));
        self.state.scan.replace(Some(handle));
        let weak = Rc::downgrade(&self.state);
        glib::timeout_add_local(POLL_INTERVAL, move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            loop {
                match receiver.try_recv() {
                    Ok(ScanEvent::Walking {
                        dirs,
                        files,
                        bytes,
                        current,
                    }) => {
                        state.status.set_text(&format!(
                            "{} dirs · {} files · {} · {}",
                            dirs,
                            files,
                            format_file_size(bytes),
                            current.display()
                        ));
                    }
                    Ok(ScanEvent::Hashing {
                        files_done,
                        files_total,
                    }) => {
                        state
                            .status
                            .set_text(&format!("Hashing duplicates {files_done}/{files_total}"));
                    }
                    Ok(ScanEvent::Cancelled) => {
                        finish_scan(&state, None, "Cancelled");
                        return glib::ControlFlow::Break;
                    }
                    Ok(ScanEvent::Done(result)) => {
                        let mut message = format!(
                            "{} dirs · {} files · {} artifacts · {} duplicate sets · {} in {:.1}s",
                            result.dirs_scanned,
                            result.files_scanned,
                            result.candidates.len(),
                            result.dupes.len(),
                            format_file_size(result.bytes_scanned),
                            result.elapsed.as_secs_f32()
                        );
                        if !result.errors.is_empty() {
                            message.push_str(&format!(" · {} errors", result.errors.len()));
                        }
                        finish_scan(&state, Some(*result), &message);
                        return glib::ControlFlow::Break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finish_scan(&state, None, "Scan ended unexpectedly");
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn note_deletion_finished(&self) {
        let pending = self.state.pending_trash.borrow().clone();
        if pending.is_empty() {
            return;
        }
        if let Some(result) = self.state.result.borrow_mut().as_mut() {
            result
                .candidates
                .retain(|candidate| !pending.contains(&candidate.path));
            for group in &mut result.dupes {
                group.files.retain(|file| !pending.contains(&file.path));
            }
            result.dupes.retain(|group| group.files.len() > 1);
        }
        self.state
            .selected
            .borrow_mut()
            .retain(|path| !pending.contains(path));
        self.state.pending_trash.borrow_mut().clear();
        rebuild_rows(&self.state);
    }
}

fn bind_factory(factory: &gtk::SignalListItemFactory, state: &Rc<CleanupState>) {
    let setup_state = Rc::downgrade(state);
    factory.connect_setup(move |_, item| {
        let list_item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("cleanup list item");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("cleanup-row");
        let check = gtk::CheckButton::new();
        check.add_css_class("cleanup-check");
        let icon = gtk::Image::new();
        icon.set_pixel_size(16);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        let name = gtk::Label::new(None);
        name.add_css_class("cleanup-name");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let detail = gtk::Label::new(None);
        detail.add_css_class("cleanup-detail");
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text.append(&name);
        text.append(&detail);
        let meta = gtk::Label::new(None);
        meta.add_css_class("cleanup-meta");
        meta.set_xalign(1.0);
        row.append(&check);
        row.append(&icon);
        row.append(&text);
        row.append(&meta);
        let toggled = setup_state.clone();
        let item_for_toggle = list_item.clone();
        check.connect_toggled(move |check| {
            let Some(state) = toggled.upgrade() else {
                return;
            };
            if state.syncing.get() {
                return;
            }
            set_row_selected(
                &state,
                item_for_toggle.position() as usize,
                check.is_active(),
            );
        });
        list_item.set_child(Some(&row));
    });

    let bound = Rc::downgrade(state);
    factory.connect_bind(move |_, item| {
        let Some(state) = bound.upgrade() else {
            return;
        };
        let list_item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("cleanup list item");
        let Some(row) = list_item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let position = list_item.position() as usize;
        let Some(kind) = state.rows.borrow().get(position).copied() else {
            return;
        };
        let check = first_child::<gtk::CheckButton>(&row);
        let icon = child_at::<gtk::Image>(&row, 1);
        let text = child_at::<gtk::Box>(&row, 2);
        let meta = child_at::<gtk::Label>(&row, 3);
        let name = text.as_ref().and_then(first_child::<gtk::Label>);
        let detail = text
            .as_ref()
            .and_then(|box_| child_at::<gtk::Label>(box_, 1));
        state.syncing.set(true);
        apply_row(
            &state,
            kind,
            check.as_ref(),
            icon.as_ref(),
            name.as_ref(),
            detail.as_ref(),
            meta.as_ref(),
        );
        state.syncing.set(false);
    });
}

fn first_child<T: IsA<gtk::Widget>>(parent: &gtk::Box) -> Option<T> {
    parent.first_child().and_then(|child| child.downcast().ok())
}

fn child_at<T: IsA<gtk::Widget>>(parent: &gtk::Box, index: usize) -> Option<T> {
    let mut child = parent.first_child();
    for _ in 0..index {
        child = child.and_then(|widget| widget.next_sibling());
    }
    child.and_then(|widget| widget.downcast().ok())
}

fn apply_row(
    state: &CleanupState,
    kind: Row,
    check: Option<&gtk::CheckButton>,
    icon: Option<&gtk::Image>,
    name: Option<&gtk::Label>,
    detail: Option<&gtk::Label>,
    meta: Option<&gtk::Label>,
) {
    let result = state.result.borrow();
    let Some(result) = result.as_ref() else {
        return;
    };
    match kind {
        Row::Artifact(index) => {
            let Some(candidate) = result.candidates.get(index) else {
                return;
            };
            if let Some(check) = check {
                check.set_visible(true);
                check.set_active(state.selected.borrow().contains(&candidate.path));
            }
            if let Some(icon) = icon {
                crate::assets::set_primary_icon(icon, crate::assets::icons::FOLDER);
                icon.set_visible(true);
            }
            if let Some(name) = name {
                name.set_text(&format!(
                    "{} / {}",
                    candidate
                        .project()
                        .file_name()
                        .map(|file_name| file_name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| candidate.project().display().to_string()),
                    candidate
                        .path
                        .file_name()
                        .map(|file_name| file_name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| candidate.path.display().to_string())
                ));
            }
            if let Some(detail) = detail {
                detail.set_text(&format!(
                    "{} · {} files · {} · {} · regenerate with {} · {}",
                    candidate.kind,
                    candidate.files,
                    candidate.confidence.label(),
                    age_label(candidate.newest),
                    candidate.regen,
                    candidate.path.display()
                ));
            }
            if let Some(meta) = meta {
                meta.set_text(&format_file_size(candidate.size));
                set_confidence_class(meta, candidate.confidence);
            }
        }
        Row::DupHeader(index) => {
            let Some(group) = result.dupes.get(index) else {
                return;
            };
            if let Some(check) = check {
                check.set_visible(false);
            }
            if let Some(icon) = icon {
                crate::assets::set_primary_icon(icon, crate::assets::icons::COPY);
                icon.set_visible(true);
            }
            if let Some(name) = name {
                name.set_text(&format!(
                    "{} copies · {} wasted",
                    group.files.len(),
                    format_file_size(group.wasted())
                ));
            }
            if let Some(detail) = detail {
                detail.set_text(&format!("{} each", format_file_size(group.size)));
            }
            if let Some(meta) = meta {
                meta.set_text("");
            }
        }
        Row::DupFile { group, file } => {
            let Some(item) = result
                .dupes
                .get(group)
                .and_then(|group| group.files.get(file))
            else {
                return;
            };
            if let Some(check) = check {
                check.set_visible(true);
                check.set_active(state.selected.borrow().contains(&item.path));
            }
            if let Some(icon) = icon {
                crate::assets::set_primary_icon(icon, crate::assets::icons::DOCUMENTS);
                icon.set_visible(true);
            }
            if let Some(name) = name {
                name.set_text(&item.path.display().to_string());
            }
            if let Some(detail) = detail {
                detail.set_text(&age_label(item.modified));
            }
            if let Some(meta) = meta {
                meta.set_text("");
            }
        }
    }
}

fn set_confidence_class(label: &gtk::Label, confidence: Confidence) {
    label.remove_css_class("cleanup-confidence-high");
    label.remove_css_class("cleanup-confidence-medium");
    label.remove_css_class("cleanup-confidence-low");
    label.add_css_class(match confidence {
        Confidence::High => "cleanup-confidence-high",
        Confidence::Medium => "cleanup-confidence-medium",
        Confidence::Low => "cleanup-confidence-low",
    });
}

fn age_label(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return "unknown age".to_owned();
    };
    let days = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO)
        .as_secs()
        / 86_400;
    match days {
        0 => "modified today".to_owned(),
        1 => "modified 1 day ago".to_owned(),
        2..=60 => format!("modified {days} days ago"),
        61..=730 => format!("modified {} mo ago", days / 30),
        _ => format!("modified {} yr ago", days / 365),
    }
}

fn connect_controls(state: &Rc<CleanupState>, close: &gtk::Button) {
    let closed = Rc::downgrade(state);
    close.connect_clicked(move |_| {
        if let Some(state) = closed.upgrade() {
            close_pane(&state);
        }
    });
    let cancelled = Rc::downgrade(state);
    state.cancel.connect_clicked(move |_| {
        if let Some(state) = cancelled.upgrade() {
            if let Some(scan) = state.scan.borrow().as_ref() {
                scan.cancel();
            }
            state.status.set_text("Cancelling…");
        }
    });
    let artifacts = Rc::downgrade(state);
    state.artifacts_tab.connect_toggled(move |button| {
        if button.is_active()
            && let Some(state) = artifacts.upgrade()
        {
            state.tab.set(Tab::Artifacts);
            rebuild_rows(&state);
        }
    });
    let duplicates = Rc::downgrade(state);
    state.duplicates_tab.connect_toggled(move |button| {
        if button.is_active()
            && let Some(state) = duplicates.upgrade()
        {
            state.tab.set(Tab::Duplicates);
            rebuild_rows(&state);
        }
    });
    let confidence = Rc::downgrade(state);
    state.confidence.connect_selected_notify(move |dropdown| {
        if let Some(state) = confidence.upgrade() {
            state.min_confidence.set(match dropdown.selected() {
                0 => Confidence::High,
                2 => Confidence::Low,
                _ => Confidence::Medium,
            });
            rebuild_rows(&state);
        }
    });
    let filtered = Rc::downgrade(state);
    state.filter.connect_changed(move |_| {
        if let Some(state) = filtered.upgrade() {
            rebuild_rows(&state);
        }
    });
    let keep = Rc::downgrade(state);
    state.keep_newest.connect_clicked(move |_| {
        if let Some(state) = keep.upgrade() {
            keep_newest(&state);
        }
    });
    let trash = Rc::downgrade(state);
    state.trash.connect_clicked(move |_| {
        if let Some(state) = trash.upgrade() {
            trash_selected(&state);
        }
    });
}

fn show_pane(state: &CleanupState) {
    state.widget.set_visible(true);
    if let Some(split) = state.split.borrow().as_ref() {
        let height = split.height();
        if height > PANE_HEIGHT + 120 {
            split.set_position(height - PANE_HEIGHT);
        }
    }
}

fn close_pane(state: &CleanupState) {
    state.generation.set(state.generation.get() + 1);
    state.scan.borrow_mut().take();
    state.widget.set_visible(false);
}

fn finish_scan(state: &CleanupState, result: Option<ScanResult>, message: &str) {
    state.scan.borrow_mut().take();
    state.spinner.stop();
    state.spinner.set_visible(false);
    state.cancel.set_visible(false);
    state.status.set_text(message);
    state.result.replace(result);
    rebuild_rows(state);
}

fn rebuild_rows(state: &CleanupState) {
    let needle = state.filter.text().to_lowercase();
    let min_confidence = state.min_confidence.get();
    let mut rows = Vec::new();
    if let Some(result) = state.result.borrow().as_ref() {
        match state.tab.get() {
            Tab::Artifacts => {
                for (index, candidate) in result.candidates.iter().enumerate() {
                    if candidate.confidence > min_confidence {
                        continue;
                    }
                    if !needle.is_empty() {
                        let hay = format!(
                            "{} {}",
                            candidate.path.to_string_lossy().to_lowercase(),
                            candidate.kind.to_lowercase()
                        );
                        if !hay.contains(&needle) {
                            continue;
                        }
                    }
                    rows.push(Row::Artifact(index));
                }
            }
            Tab::Duplicates => {
                for (group_index, group) in result.dupes.iter().enumerate() {
                    let matching: Vec<usize> = group
                        .files
                        .iter()
                        .enumerate()
                        .filter(|(_, file)| {
                            needle.is_empty()
                                || file.path.to_string_lossy().to_lowercase().contains(&needle)
                        })
                        .map(|(index, _)| index)
                        .collect();
                    if matching.is_empty() {
                        continue;
                    }
                    rows.push(Row::DupHeader(group_index));
                    rows.extend(matching.into_iter().map(|file| Row::DupFile {
                        group: group_index,
                        file,
                    }));
                }
            }
        }
    }
    let labels: Vec<String> = (0..rows.len()).map(|index| index.to_string()).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    state.model.splice(0, state.model.n_items(), &refs);
    state.rows.replace(rows);
    state
        .keep_newest
        .set_visible(state.tab.get() == Tab::Duplicates);
    update_footer(state);
}

fn set_row_selected(state: &CleanupState, position: usize, selected: bool) {
    let Some(kind) = state.rows.borrow().get(position).copied() else {
        return;
    };
    let path = match kind {
        Row::Artifact(index) => state.result.borrow().as_ref().and_then(|result| {
            result
                .candidates
                .get(index)
                .map(|candidate| candidate.path.clone())
        }),
        Row::DupFile { group, file } => state.result.borrow().as_ref().and_then(|result| {
            result
                .dupes
                .get(group)
                .and_then(|group| group.files.get(file).map(|item| item.path.clone()))
        }),
        Row::DupHeader(_) => None,
    };
    let Some(path) = path else {
        return;
    };
    if selected {
        state.selected.borrow_mut().insert(path);
    } else {
        state.selected.borrow_mut().remove(&path);
    }
    update_footer(state);
}

fn keep_newest(state: &CleanupState) {
    let result = state.result.borrow();
    let Some(result) = result.as_ref() else {
        return;
    };
    let mut selected = state.selected.borrow_mut();
    for group in &result.dupes {
        let newest = group.files.iter().max_by_key(|file| file.modified);
        for file in &group.files {
            if newest.is_some_and(|keep| keep.path == file.path) {
                selected.remove(&file.path);
            } else {
                selected.insert(file.path.clone());
            }
        }
    }
    drop(selected);
    rebuild_rows(state);
}

fn trash_selected(state: &CleanupState) {
    let selected = state.selected.borrow().clone();
    if selected.is_empty() {
        return;
    }
    let result = state.result.borrow();
    let Some(result) = result.as_ref() else {
        return;
    };
    let mut entries = Vec::new();
    for candidate in &result.candidates {
        if selected.contains(&candidate.path) {
            entries.push(entry_for(&candidate.path, true, candidate.size));
        }
    }
    for group in &result.dupes {
        for file in &group.files {
            if selected.contains(&file.path) {
                entries.push(entry_for(&file.path, false, group.size));
            }
        }
    }
    if entries.is_empty() {
        return;
    }
    state.pending_trash.replace(selected);
    (state.trash_items)(entries);
}

fn entry_for(path: &Path, directory: bool, size: u64) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        native_name: path.file_name().map(OsString::from).unwrap_or_default(),
        display_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        kind: if directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: MetadataValue::Known(size),
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn activate_row(state: &CleanupState, position: u32) {
    let Some(kind) = state.rows.borrow().get(position as usize).copied() else {
        return;
    };
    let path = match kind {
        Row::Artifact(index) => state.result.borrow().as_ref().and_then(|result| {
            result
                .candidates
                .get(index)
                .map(|candidate| (candidate.path.clone(), true))
        }),
        Row::DupHeader(_) => None,
        Row::DupFile { group, file } => state.result.borrow().as_ref().and_then(|result| {
            result
                .dupes
                .get(group)
                .and_then(|group| group.files.get(file).map(|item| (item.path.clone(), false)))
        }),
    };
    if let Some((path, is_directory)) = path {
        (state.reveal)(path, is_directory);
    }
}

fn update_footer(state: &CleanupState) {
    let selected = state.selected.borrow();
    let result = state.result.borrow();
    let Some(result) = result.as_ref() else {
        state.footer.set_text("Select items to move to Trash");
        state.trash.set_sensitive(false);
        return;
    };
    let mut bytes = 0u64;
    let mut all_copies = false;
    for candidate in &result.candidates {
        if selected.contains(&candidate.path) {
            bytes += candidate.size;
        }
    }
    for group in &result.dupes {
        let selected_copies = group
            .files
            .iter()
            .filter(|file| selected.contains(&file.path))
            .count();
        bytes += group.size * selected_copies as u64;
        if selected_copies == group.files.len() && !group.files.is_empty() {
            all_copies = true;
        }
    }
    let count = selected.len();
    state.trash.set_sensitive(count > 0);
    if count == 0 {
        state.footer.set_text("Select items to move to Trash");
    } else if all_copies {
        state.footer.set_text(&format!(
            "{count} selected · {} · a duplicate set has every copy selected",
            format_file_size(bytes)
        ));
    } else {
        state
            .footer
            .set_text(&format!("{count} selected · {}", format_file_size(bytes)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_confidence_is_stricter_than_medium() {
        assert!(Confidence::High < Confidence::Medium);
        assert!(Confidence::Medium < Confidence::Low);
        assert!(Confidence::Low > Confidence::Medium);
    }
}

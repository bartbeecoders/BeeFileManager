// SPDX-License-Identifier: GPL-3.0-or-later

//! Parallel directory walk: finds regenerable artifact directories and metadata
//! for the duplicate finder.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{Instant, SystemTime},
};

use rayon::prelude::*;

use super::{
    dupes::{DupGroup, FileEntry, find_duplicates},
    fsutil::{device_of, disk_size, file_id},
    rules::{self, Confidence},
};

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub root: PathBuf,
    pub find_dupes: bool,
    pub dupe_min_size: u64,
    pub same_filesystem: bool,
    pub skip_hidden: bool,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
    pub files: u64,
    pub kind: &'static str,
    pub confidence: Confidence,
    pub regen: &'static str,
    pub newest: Option<SystemTime>,
}

impl Candidate {
    pub fn project(&self) -> &Path {
        self.path.parent().unwrap_or(&self.path)
    }
}

#[derive(Clone, Debug)]
pub struct ScanResult {
    pub candidates: Vec<Candidate>,
    pub dupes: Vec<DupGroup>,
    pub errors: Vec<String>,
    pub dirs_scanned: u64,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    pub elapsed: std::time::Duration,
}

pub enum ScanEvent {
    Walking {
        dirs: u64,
        files: u64,
        bytes: u64,
        current: PathBuf,
    },
    Hashing {
        files_done: u64,
        files_total: u64,
    },
    Done(Box<ScanResult>),
    Cancelled,
}

struct Ctx {
    opts: ScanOptions,
    cancel: Arc<AtomicBool>,
    tx: Sender<ScanEvent>,
    root_dev: u64,
    candidates: Mutex<Vec<Candidate>>,
    files: Mutex<Vec<FileEntry>>,
    errors: Mutex<Vec<String>>,
    dirs: AtomicU64,
    file_count: AtomicU64,
    bytes: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Ctx {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn note_error(&self, path: &Path, err: &std::io::Error) {
        let mut errors = lock(&self.errors);
        if errors.len() < 500 {
            errors.push(format!("{}: {err}", path.display()));
        }
    }

    fn tick(&self, current: &Path) {
        let dirs = self.dirs.fetch_add(1, Ordering::Relaxed) + 1;
        if dirs.is_multiple_of(64) {
            let _ = self.tx.send(ScanEvent::Walking {
                dirs,
                files: self.file_count.load(Ordering::Relaxed),
                bytes: self.bytes.load(Ordering::Relaxed),
                current: current.to_path_buf(),
            });
        }
    }
}

pub fn run(opts: ScanOptions, cancel: Arc<AtomicBool>, tx: Sender<ScanEvent>) {
    let started = Instant::now();

    let root_dev = std::fs::metadata(&opts.root)
        .map(|metadata| device_of(&metadata))
        .unwrap_or(0);
    let ctx = Ctx {
        opts: opts.clone(),
        cancel: cancel.clone(),
        tx: tx.clone(),
        root_dev,
        candidates: Mutex::new(Vec::new()),
        files: Mutex::new(Vec::new()),
        errors: Mutex::new(Vec::new()),
        dirs: AtomicU64::new(0),
        file_count: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
    };

    walk(&opts.root, &ctx);

    if ctx.cancelled() {
        let _ = tx.send(ScanEvent::Cancelled);
        return;
    }

    let mut candidates = std::mem::take(&mut *lock(&ctx.candidates));
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.size));

    let files = std::mem::take(&mut *lock(&ctx.files));
    let dupes = if opts.find_dupes {
        find_duplicates(files, &cancel, &|files_done, files_total| {
            let _ = tx.send(ScanEvent::Hashing {
                files_done,
                files_total,
            });
        })
    } else {
        Vec::new()
    };

    if ctx.cancelled() {
        let _ = tx.send(ScanEvent::Cancelled);
        return;
    }

    let _ = tx.send(ScanEvent::Done(Box::new(ScanResult {
        candidates,
        dupes,
        errors: std::mem::take(&mut *lock(&ctx.errors)),
        dirs_scanned: ctx.dirs.load(Ordering::Relaxed),
        files_scanned: ctx.file_count.load(Ordering::Relaxed),
        bytes_scanned: ctx.bytes.load(Ordering::Relaxed),
        elapsed: started.elapsed(),
    })));
}

fn is_never_scan(path: &Path) -> bool {
    rules::NEVER_SCAN.iter().any(|item| path == Path::new(item))
}

fn walk(dir: &Path, ctx: &Ctx) {
    if ctx.cancelled() {
        return;
    }
    ctx.tick(dir);

    let entries = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) => return ctx.note_error(dir, &error),
    };

    let mut subdirs: Vec<(String, PathBuf)> = Vec::new();
    let mut local_files: Vec<FileEntry> = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    let mut files_here = 0u64;
    let mut bytes_here = 0u64;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                ctx.note_error(dir, &error);
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                ctx.note_error(&entry.path(), &error);
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        names.insert(name.clone());

        if file_type.is_dir() {
            subdirs.push((name, entry.path()));
        } else if file_type.is_file() {
            files_here += 1;
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            bytes_here += disk_size(&metadata);
            if ctx.opts.find_dupes && metadata.len() >= ctx.opts.dupe_min_size {
                let (dev, ino) = file_id(&metadata);
                local_files.push(FileEntry {
                    path: entry.path(),
                    size: metadata.len(),
                    dev,
                    ino,
                    modified: metadata.modified().ok(),
                });
            }
        }
    }

    ctx.file_count.fetch_add(files_here, Ordering::Relaxed);
    ctx.bytes.fetch_add(bytes_here, Ordering::Relaxed);
    if !local_files.is_empty() {
        lock(&ctx.files).append(&mut local_files);
    }

    let mut recurse: Vec<PathBuf> = Vec::new();
    for (name, path) in subdirs {
        if rules::NEVER_DESCEND
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&name))
            || is_never_scan(&path)
        {
            continue;
        }
        if ctx.opts.same_filesystem
            && std::fs::metadata(&path)
                .map(|metadata| device_of(&metadata))
                .unwrap_or(ctx.root_dev)
                != ctx.root_dev
        {
            continue;
        }

        match rules::classify(&path, &name, &names) {
            Some(rule) => {
                let measured = measure(&path, ctx);
                lock(&ctx.candidates).push(Candidate {
                    path,
                    size: measured.bytes,
                    files: measured.files,
                    kind: rule.kind,
                    confidence: rule.confidence,
                    regen: rule.regen,
                    newest: measured.newest,
                });
            }
            None => {
                if ctx.opts.skip_hidden
                    && name.starts_with('.')
                    && !rules::HIDDEN_STORES
                        .iter()
                        .any(|item| item.eq_ignore_ascii_case(&name))
                {
                    continue;
                }
                recurse.push(path);
            }
        }
    }

    if recurse.len() > 1 {
        recurse.par_iter().for_each(|path| walk(path, ctx));
    } else {
        recurse.iter().for_each(|path| walk(path, ctx));
    }
}

#[derive(Default)]
struct Measured {
    bytes: u64,
    files: u64,
    newest: Option<SystemTime>,
}

impl Measured {
    fn merge(mut self, other: Self) -> Self {
        self.bytes += other.bytes;
        self.files += other.files;
        self.newest = self.newest.max(other.newest);
        self
    }
}

fn measure(dir: &Path, ctx: &Ctx) -> Measured {
    if ctx.cancelled() {
        return Measured::default();
    }
    ctx.tick(dir);

    let mut acc = Measured::default();
    if let Ok(metadata) = std::fs::metadata(dir) {
        acc.bytes += disk_size(&metadata);
        acc.newest = metadata.modified().ok();
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            ctx.note_error(dir, &error);
            return acc;
        }
    };

    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            subdirs.push(entry.path());
        } else if let Ok(metadata) = entry.metadata() {
            acc.bytes += disk_size(&metadata);
            acc.files += 1;
            acc.newest = acc.newest.max(metadata.modified().ok());
        }
    }

    ctx.file_count.fetch_add(acc.files, Ordering::Relaxed);
    ctx.bytes.fetch_add(acc.bytes, Ordering::Relaxed);

    let sub = if subdirs.len() > 1 {
        subdirs
            .par_iter()
            .map(|path| measure(path, ctx))
            .reduce(Measured::default, Measured::merge)
    } else {
        subdirs
            .iter()
            .map(|path| measure(path, ctx))
            .fold(Measured::default(), Measured::merge)
    };
    acc.merge(sub)
}

#[cfg(test)]
mod tests;

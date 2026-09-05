// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::path::PathBuf;

fn entry(path: PathBuf, size: u64, ino: u64) -> FileEntry {
    FileEntry {
        path,
        size,
        dev: 1,
        ino,
        modified: None,
    }
}

fn scan_dir(dir: &std::path::Path) -> Vec<FileEntry> {
    let mut out = Vec::new();
    for (index, item) in std::fs::read_dir(dir)
        .expect("read fixture")
        .flatten()
        .enumerate()
    {
        let metadata = item.metadata().expect("metadata");
        out.push(entry(item.path(), metadata.len(), index as u64));
    }
    out
}

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "beefilemanager-cleanup-dupe-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture");
    dir
}

#[test]
fn finds_identical_files_and_ignores_look_alikes() {
    let dir = tmpdir("identical");
    let body = vec![b'x'; 100 * 1024];
    std::fs::write(dir.join("a.bin"), &body).expect("write a");
    std::fs::write(dir.join("b.bin"), &body).expect("write b");

    let mut tweaked = body.clone();
    *tweaked.last_mut().expect("body") = b'y';
    std::fs::write(dir.join("c.bin"), &tweaked).expect("write c");
    std::fs::write(dir.join("d.bin"), vec![b'x'; 50 * 1024]).expect("write d");

    let groups = find_duplicates(scan_dir(&dir), &AtomicBool::new(false), &|_, _| {});
    assert_eq!(groups.len(), 1);
    let mut names: Vec<String> = groups[0]
        .files
        .iter()
        .map(|file| {
            file.path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into()
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["a.bin", "b.bin"]);
    assert_eq!(groups[0].wasted(), 100 * 1024);

    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn small_files_skip_the_second_pass() {
    let dir = tmpdir("small");
    std::fs::write(dir.join("a.txt"), b"hello world").expect("write a");
    std::fs::write(dir.join("b.txt"), b"hello world").expect("write b");
    std::fs::write(dir.join("c.txt"), b"hello there").expect("write c");

    let groups = find_duplicates(scan_dir(&dir), &AtomicBool::new(false), &|_, _| {});
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].files.len(), 2);

    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn hardlinks_are_not_waste() {
    let a = entry(PathBuf::from("/x/a"), 4096, 7);
    let b = entry(PathBuf::from("/x/b"), 4096, 7);
    assert!(find_duplicates(vec![a, b], &AtomicBool::new(false), &|_, _| {}).is_empty());
}

#[test]
fn empty_files_are_never_duplicates() {
    let a = entry(PathBuf::from("/x/a"), 0, 1);
    let b = entry(PathBuf::from("/x/b"), 0, 2);
    assert!(find_duplicates(vec![a, b], &AtomicBool::new(false), &|_, _| {}).is_empty());
}

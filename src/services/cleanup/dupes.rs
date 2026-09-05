// SPDX-License-Identifier: GPL-3.0-or-later

//! Duplicate file detection: group by size, then by a cheap head hash, then
//! confirm with a full BLAKE3 hash. Only the last stage reads whole files.

use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::SystemTime,
};

use rayon::prelude::*;

pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub dev: u64,
    pub ino: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct DupFile {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct DupGroup {
    pub size: u64,
    pub files: Vec<DupFile>,
}

impl DupGroup {
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64).saturating_sub(1)
    }
}

const HEAD_BYTES: u64 = 16 * 1024;
const READ_BUF: usize = 512 * 1024;

type Hash = [u8; 32];

fn head_hash(path: &std::path::Path, size: u64) -> Option<Hash> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; HEAD_BYTES.min(size) as usize];
    let mut read = 0;
    while read < buf.len() {
        match file.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => return None,
        }
    }
    buf.truncate(read);
    Some(*blake3::hash(&buf).as_bytes())
}

fn full_hash(path: &std::path::Path, cancel: &AtomicBool) -> Option<Hash> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; READ_BUF];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
            }
            Err(_) => return None,
        }
    }
    Some(*hasher.finalize().as_bytes())
}

fn regroup(hashed: Vec<(Hash, FileEntry)>) -> Vec<Vec<FileEntry>> {
    let mut by_hash: HashMap<Hash, Vec<FileEntry>> = HashMap::new();
    for (hash, entry) in hashed {
        by_hash.entry(hash).or_default().push(entry);
    }
    by_hash
        .into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

pub fn find_duplicates(
    files: Vec<FileEntry>,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Vec<DupGroup> {
    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    for file in files.into_iter().filter(|file| file.size > 0) {
        by_size.entry(file.size).or_default().push(file);
    }

    let groups: Vec<Vec<FileEntry>> = by_size
        .into_values()
        .filter(|group| group.len() > 1)
        .map(|group| {
            let mut seen = std::collections::HashSet::new();
            group
                .into_iter()
                .filter(|file| seen.insert((file.dev, file.ino)))
                .collect::<Vec<_>>()
        })
        .filter(|group| group.len() > 1)
        .collect();

    if groups.is_empty() || cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    let total: u64 = groups.iter().map(|group| group.len() as u64).sum();
    let done = AtomicU64::new(0);
    let report = |done: &AtomicU64, total: u64| {
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(32) {
            progress(n, total);
        }
    };

    let refined: Vec<Vec<FileEntry>> = groups
        .into_par_iter()
        .flat_map(|group| {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            let hashed: Vec<(Hash, FileEntry)> = group
                .into_par_iter()
                .filter_map(|file| {
                    report(&done, total);
                    head_hash(&file.path, file.size).map(|hash| (hash, file))
                })
                .collect();
            regroup(hashed)
        })
        .collect();

    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    let total: u64 = refined.iter().map(|group| group.len() as u64).sum();
    let done = AtomicU64::new(0);

    let confirmed: Vec<Vec<FileEntry>> = refined
        .into_par_iter()
        .flat_map(|group| {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            if group.first().is_some_and(|file| file.size <= HEAD_BYTES) {
                return vec![group];
            }
            let hashed: Vec<(Hash, FileEntry)> = group
                .into_par_iter()
                .filter_map(|file| {
                    report(&done, total);
                    full_hash(&file.path, cancel).map(|hash| (hash, file))
                })
                .collect();
            regroup(hashed)
        })
        .collect();

    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    let mut out: Vec<DupGroup> = confirmed
        .into_iter()
        .filter_map(|mut group| {
            group.sort_by(|left, right| left.path.cmp(&right.path));
            let size = group.first()?.size;
            Some(DupGroup {
                size,
                files: group
                    .into_iter()
                    .map(|file| DupFile {
                        path: file.path,
                        modified: file.modified,
                    })
                    .collect(),
            })
        })
        .collect();

    out.sort_by_key(|group| std::cmp::Reverse(group.wasted()));
    out
}

#[cfg(test)]
mod tests;

// SPDX-License-Identifier: GPL-3.0-or-later

mod dupes;
mod fsutil;
mod rules;
mod scan;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

pub use rules::Confidence;
pub use scan::{ScanEvent, ScanOptions, ScanResult};

pub struct ScanHandle {
    cancelled: Arc<AtomicBool>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Walks `opts.root` off the GTK thread. Progress and the final result arrive on the receiver.
pub fn start_scan(opts: ScanOptions) -> (ScanHandle, Receiver<ScanEvent>) {
    let (tx, rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker_tx = tx.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("beefilemanager-cleanup-scan".into())
        .spawn(move || scan::run(opts, worker_cancelled, worker_tx))
    {
        tracing::error!(%error, "unable to start cleanup scan");
        let _ = tx.send(scan::ScanEvent::Cancelled);
    }
    (ScanHandle { cancelled }, rx)
}

pub fn default_options(root: PathBuf) -> ScanOptions {
    ScanOptions {
        root,
        find_dupes: true,
        dupe_min_size: 1024 * 1024,
        same_filesystem: true,
        skip_hidden: true,
    }
}

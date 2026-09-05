// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs::Metadata;

/// Space the file actually occupies on disk, matching what `du` reports.
/// Falls back to the apparent size where block counts aren't available.
pub fn disk_size(md: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        md.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        md.len()
    }
}

/// `(device, inode)` identity, used to avoid counting hardlinks twice.
#[cfg(unix)]
pub fn file_id(md: &Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (md.dev(), md.ino())
}

#[cfg(not(unix))]
pub fn file_id(_md: &Metadata) -> (u64, u64) {
    (0, 0)
}

pub fn device_of(md: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        md.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        0
    }
}

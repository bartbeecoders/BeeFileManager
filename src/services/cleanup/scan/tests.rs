// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::sync::mpsc;

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "beefilemanager-cleanup-scan-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture");
    dir
}

fn collect(opts: ScanOptions) -> ScanResult {
    let (tx, rx) = mpsc::channel();
    run(opts, Arc::new(AtomicBool::new(false)), tx);
    for event in rx {
        if let ScanEvent::Done(result) = event {
            return *result;
        }
    }
    panic!("scan produced no result");
}

#[test]
fn rust_target_is_flagged_and_photos_named_target_are_not() {
    let rust = tmpdir("rust");
    std::fs::write(rust.join("Cargo.toml"), "[package]\nname=\"demo\"\n").expect("manifest");
    std::fs::create_dir_all(rust.join("target/debug")).expect("target");
    std::fs::write(rust.join("target/debug/demo"), vec![0u8; 4096]).expect("artifact");

    let photos = tmpdir("photos");
    std::fs::create_dir_all(photos.join("target")).expect("photos target");
    std::fs::write(photos.join("target/shot.jpg"), vec![0u8; 4096]).expect("photo");

    let rust_result = collect(ScanOptions {
        root: rust.clone(),
        find_dupes: false,
        dupe_min_size: 0,
        same_filesystem: true,
        skip_hidden: true,
    });
    assert_eq!(rust_result.candidates.len(), 1);
    assert_eq!(rust_result.candidates[0].kind, "Rust build output");

    let photo_result = collect(ScanOptions {
        root: photos.clone(),
        find_dupes: false,
        dupe_min_size: 0,
        same_filesystem: true,
        skip_hidden: true,
    });
    assert!(photo_result.candidates.is_empty());

    std::fs::remove_dir_all(&rust).expect("cleanup rust");
    std::fs::remove_dir_all(&photos).expect("cleanup photos");
}

#[test]
fn duplicate_files_are_grouped_and_artifacts_are_excluded() {
    let root = tmpdir("dupes");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"demo\"\n").expect("manifest");
    std::fs::create_dir_all(root.join("target")).expect("target");
    let body = vec![b'z'; 8 * 1024];
    std::fs::write(root.join("a.bin"), &body).expect("a");
    std::fs::write(root.join("b.bin"), &body).expect("b");
    std::fs::write(root.join("target/copy.bin"), &body).expect("inside artifact");

    let result = collect(ScanOptions {
        root: root.clone(),
        find_dupes: true,
        dupe_min_size: 0,
        same_filesystem: true,
        skip_hidden: true,
    });
    assert_eq!(result.dupes.len(), 1);
    assert_eq!(result.dupes[0].files.len(), 2);
    assert_eq!(result.candidates.len(), 1);

    std::fs::remove_dir_all(&root).expect("cleanup");
}

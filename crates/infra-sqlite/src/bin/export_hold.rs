//! Helper binary for cross-process per-target export lock integration tests.
//!
//! Usage:
//!   export_hold <target_dir> <ready_signal_path> <release_signal_path>
//!
//! Acquires the per-target reverse-export lock for `<target_dir>` (the same
//! lock `export_sqlite_to_json` takes before publishing), writes the ready
//! signal, then spins until the release signal appears (or the parent process
//! is killed). Dropping the guard on exit releases the lock.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn main() {
    let mut args = env::args().skip(1);
    let target = PathBuf::from(args.next().expect("target_dir"));
    let ready = PathBuf::from(args.next().expect("ready_signal"));
    let release = PathBuf::from(args.next().expect("release_signal"));

    let _guard = storyforge_infra_sqlite::exporter::acquire_export_lock(&target)
        .unwrap_or_else(|e| panic!("failed to acquire export lock: {e}"));

    if let Some(parent) = ready.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&ready, b"ready").expect("write ready signal");

    while !release.exists() {
        thread::sleep(Duration::from_millis(50));
    }
    // Drop releases the lock on exit.
}

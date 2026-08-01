//! Helper binary for cross-process authority-lease integration tests.
//!
//! Usage:
//!   lease_hold <data_dir> <mode: shared|exclusive> <ready_signal_path> <release_signal_path>
//!
//! Acquires the requested lease under `<data_dir>/storyforge.authority.lock`,
//! writes the ready signal, then spins until the release signal appears (or
//! the parent process is killed).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use storyforge_infra_sqlite::lease::{AuthorityLeaseGuard, LeaseMode};

fn main() {
    let mut args = env::args().skip(1);
    let data_dir = PathBuf::from(args.next().expect("data_dir"));
    let mode_raw = args.next().expect("mode");
    let ready = PathBuf::from(args.next().expect("ready_signal"));
    let release = PathBuf::from(args.next().expect("release_signal"));

    let mode = match mode_raw.as_str() {
        "shared" => LeaseMode::Shared,
        "exclusive" => LeaseMode::Exclusive,
        other => panic!("unknown mode: {other}"),
    };

    let _guard = AuthorityLeaseGuard::acquire(
        data_dir.join(storyforge_infra_sqlite::lease::AUTHORITY_LEASE_FILENAME),
        mode,
    )
    .unwrap_or_else(|e| panic!("failed to acquire {mode:?} lease: {e}"));

    if let Some(parent) = ready.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&ready, b"ready").expect("write ready signal");

    while !release.exists() {
        thread::sleep(Duration::from_millis(50));
    }
    // Drop releases the lease on exit.
}

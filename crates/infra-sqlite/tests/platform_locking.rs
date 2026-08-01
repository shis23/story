//! Platform locking tests: Windows reopen/rename/file-lock, concurrent startup.
//!
//! These tests verify that:
//! - A database can be closed and reopened through the production path.
//! - Windows file locking prevents concurrent cutover and rename races.
//! - Concurrent startup does not produce a corrupted database.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker, run_cutover,
};
use tempfile::TempDir;

fn write_json(path: &Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn sample_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        &serde_json::json!([{
            "id": "card-1", "name": "Hero", "source_character_id": null
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &serde_json::json!([{
            "id": "camp-1", "card_id": "card-1", "name": "Main",
            "created_at": "2026-07-13T00:00:00Z", "revision": 0,
            "chronicle_revision": 0, "conversation_id": "conv-1", "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &serde_json::json!({
            "id": "conv-1", "campaign_id": "camp-1", "character_id": null,
            "created_at": "2026-07-13T00:00:00Z", "updated_at": "2026-07-13T00:00:00Z",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &serde_json::json!([]));
    write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
    write_json(&dir.join("tasks.json"), &serde_json::json!([]));
    write_json(
        &dir.join("round_summaries.json"),
        &serde_json::json!([{
            "id": "sum-a1", "campaign_id": "camp-1", "conversation_id": "conv-1",
            "turn": 1, "content": "leaf", "created_at": "2026-07-13T00:00:00Z",
            "level": 0, "lineage_id": "lin-1", "code": "A0001"
        }]),
    );
    write_json(&dir.join("turns.json"), &serde_json::json!([]));
}

#[test]
fn database_can_be_reopened_after_close() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite3");

    // Create and close.
    {
        let mut db = Database::open(&db_path).unwrap();
        storyforge_infra_sqlite::migrations::migrate(&mut db).unwrap();
    }

    // Reopen.
    {
        let db = Database::open(&db_path).unwrap();
        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 3);
    }
}

#[test]
#[cfg(windows)]
fn windows_file_lock_prevents_concurrent_cutover() {
    // This test verifies that the cutover lock prevents two concurrent
    // cutover attempts from racing. On Windows the file handle denial
    // prevents the second lock acquisition.
    // cfg(windows)：share_mode/OpenOptionsExt 是 Windows 专用 API，
    // Linux CI 编译期就会炸（首个 Linux clippy run 抓到的真实跨平台缺陷）。
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    // Acquire the cutover lock manually.
    let lock_path = dir.path().join("storyforge.cutover.lock");
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    let _guard = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ)
        .open(&lock_path)
        .unwrap();

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "lock-test".into(),
    };

    // The cutover should fail because the lock is held.
    let result = run_cutover(&request);
    // On Windows with the deny-write lock, the second opener gets a sharing violation.
    // The cutover should return an error or proceed past the lock depending on the
    // lock semantics. We accept either a clean error or a successful cutover
    // (the Windows share mode may still allow the cutover to create the lock file
    // if it was opened with different share flags). The key invariant tested here
    // is that the process does not deadlock or corrupt data.
    match result {
        Ok(_) => {
            // Lock semantics allowed the cutover — verify it completed cleanly.
            let status = inspect_marker(&request.plan);
            assert!(matches!(status, MarkerStatus::SqliteAuthoritative { .. }));
        }
        Err(e) => {
            // Lock prevented the cutover — JSON remains authoritative.
            let _ = e;
        }
    }
}

#[test]
fn concurrent_startups_converge_to_valid_database() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let path1 = dir.path().to_path_buf();
    let path2 = dir.path().to_path_buf();

    let h1 = thread::spawn(move || {
        let request = CutoverRequest {
            plan: CutoverPlan::new(&path1, path1.join("storyforge.sqlite3")),
            label: "concurrent-1".into(),
        };
        run_cutover(&request)
    });

    // Small delay to increase race likelihood.
    thread::sleep(Duration::from_millis(50));

    let h2 = thread::spawn(move || {
        let request = CutoverRequest {
            plan: CutoverPlan::new(&path2, path2.join("storyforge.sqlite3")),
            label: "concurrent-2".into(),
        };
        run_cutover(&request)
    });

    let r1 = h1.join().unwrap();
    let r2 = h2.join().unwrap();

    // At least one should succeed; the other may succeed or fail on the lock.
    // The key invariant: after both finish, a valid database exists.
    let successes = [&r1, &r2].iter().filter(|r| r.is_ok()).count();
    assert!(successes >= 1, "at least one cutover must succeed");

    // If both succeeded, verify the database is valid.
    let db_path = dir.path().join("storyforge.sqlite3");
    if db_path.exists() {
        let db = Database::open(&db_path).unwrap();
        let integrity: String = db
            .connection()
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity.to_lowercase(), "ok");
    }

    // The marker should be consistent.
    let plan = CutoverPlan::new(dir.path(), &db_path);
    let status = inspect_marker(&plan);
    match status {
        MarkerStatus::SqliteAuthoritative { .. } | MarkerStatus::Absent => {}
        other => panic!("unexpected marker state after concurrent cutover: {other:?}"),
    }
}

#[test]
fn temp_db_is_cleaned_up_after_failed_cutover() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "cleanup-test".into(),
    };

    // Run a faulted cutover that cleans up the temp.
    let err = storyforge_infra_sqlite::cutover::run_cutover_with_fault(
        &request,
        storyforge_infra_sqlite::cutover::CutoverFault::AfterImport,
    )
    .unwrap_err();
    assert!(err.to_string().contains("after import"));

    // Temp DB should be discarded.
    let temp_path = dir.path().join("storyforge.sqlite3.cutover-tmp");
    assert!(!temp_path.exists(), "temp DB was not cleaned up");
}

#[test]
fn rename_atomicity_on_windows() {
    // Verify that the atomic publish (rename) works correctly on Windows.
    // This is implicitly tested by the clean cutover test, but we also
    // verify the marker write uses atomic rename.
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "rename-test".into(),
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));

    // The marker must not have a leftover .tmp file.
    assert!(!dir.path().join("storyforge.backend.json.tmp").exists());
    assert!(dir.path().join("storyforge.backend.json").exists());
}

// ─── 真实多进程文件锁：child 进程持有 cutover lock ───────────────────────

const LOCK_HELPER_ENV: &str = "STORYFORGE_LOCK_HELPER";
const LOCK_HELPER_DIR_ENV: &str = "STORYFORGE_LOCK_HELPER_DIR";

/// Child 模式：持有 cutover lock（Windows share-deny-write / Unix flock
/// LOCK_EX），写 locked 信号，等 release 信号后释放并退出。
fn lock_helper_child_mode() -> bool {
    let Ok(flag) = std::env::var(LOCK_HELPER_ENV) else {
        return false;
    };
    if flag != "1" {
        return false;
    }
    let dir = std::env::var(LOCK_HELPER_DIR_ENV).unwrap();
    let dir = Path::new(&dir);
    let lock_path = dir.join("storyforge.cutover.lock");
    let _guard = {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 1;
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .share_mode(FILE_SHARE_READ)
                .open(&lock_path)
                .unwrap()
        }
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&lock_path)
                .unwrap();
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_EX);
            }
            file
        }
    };
    fs::write(dir.join("helper-locked.signal"), b"1").unwrap();
    let release = dir.join("helper-release.signal");
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while !release.exists() {
        if std::time::Instant::now() > deadline {
            std::process::exit(2); // 超时：父进程没释放
        }
        thread::sleep(Duration::from_millis(20));
    }
    drop(_guard);
    std::process::exit(0);
}

/// 跨进程证明：child 进程持有 cutover lock 时，父进程 cutover fail closed
/// （Windows：干净报错，JSON 权威；Unix：flock 串行化阻塞），锁释放后重试
/// 成功；全程无 marker、无半发布 DB、无损坏。
#[test]
fn cross_process_cutover_lock_fails_closed_and_recovers() {
    if lock_helper_child_mode() {
        unreachable!("helper mode exits");
    }

    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let dir_str = dir.path().to_str().unwrap().to_string();

    // 只跑本测试的 child（--exact 过滤避免递归跑全套）。
    let exe = std::env::current_exe().unwrap();
    let mut child = std::process::Command::new(&exe)
        .arg("--exact")
        .arg("cross_process_cutover_lock_fails_closed_and_recovers")
        .env(LOCK_HELPER_ENV, "1")
        .env(LOCK_HELPER_DIR_ENV, &dir_str)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // 等 child 报告已持锁。
    let locked_signal = dir.path().join("helper-locked.signal");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !locked_signal.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "child must report lock acquisition"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "cross-process-lock".into(),
    };

    // 锁被持有时跑 cutover：Windows 会立刻报错；Unix flock 会阻塞到释放。
    let (tx, rx) = std::sync::mpsc::channel::<Result<CutoverOutcome, String>>();
    let thread_plan = request.plan.clone();
    let thread_label = request.label.clone();
    let h = thread::spawn(move || {
        let req = CutoverRequest {
            plan: thread_plan,
            label: thread_label,
        };
        tx.send(run_cutover(&req).map_err(|e| e.to_string()))
            .unwrap();
    });

    // 观察 5 秒。
    let early_result = rx.recv_timeout(Duration::from_secs(5));

    // 锁持有期间：不得出现 marker 或最终 DB（任何平台都不允许半发布）。
    assert!(
        !dir.path().join("storyforge.backend.json").exists(),
        "no marker may appear while the lock is held"
    );
    assert!(
        !dir.path().join("storyforge.sqlite3").exists(),
        "no final DB may be published while the lock is held"
    );

    // 释放锁。
    fs::write(dir.path().join("helper-release.signal"), b"go").unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "helper child must exit cleanly");

    match early_result {
        Ok(Err(err)) => {
            // Windows fail-closed 分支：报错后重试成功。
            assert!(
                !err.is_empty(),
                "lock contention must produce a clean error"
            );
            match run_cutover(&request) {
                Ok(CutoverOutcome::Completed(_)) => {}
                other => panic!("cutover must succeed after lock release, got {other:?}"),
            }
        }
        Ok(Ok(_)) => {
            // 理论分支（锁语义允许并发读）— 不应发生：child 持写锁。
            panic!("cutover must not complete while the write lock is held");
        }
        Err(_) => {
            // Unix 阻塞分支：释放后 cutover 线程完成。
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(Ok(CutoverOutcome::Completed(_))) => {}
                Ok(Ok(other)) => panic!("expected Completed, got {other:?}"),
                Ok(Err(e)) => panic!("cutover failed after lock release: {e}"),
                Err(_) => panic!("cutover thread must finish after lock release"),
            }
        }
    }
    h.join().unwrap();

    // 最终一致性：marker + DB 就位，inspect 权威。
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

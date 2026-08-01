//! Gate 5 审查二.7：生产 rollback 入口测试。
//!
//! 覆盖：happy path（marker 翻转、DB 字节不变、产物可验证可导入、无临时残留）、
//! confirm=false 无状态变更、导出/自检故障注入、自检捕获篡改 manifest、
//! marker 写入失败保持 SQLite 权威、marker 状态前置校验、SHARED 租约阻塞、
//! 二次 rollback 拒绝。
//!
//! 布局：`<root>/data/` 为活动数据目录（SQLite 库 + marker），rollback staging
//! 产物生成在 `<root>/.rollback-data-*`，因此残留扫描只扫 `<root>`，不与共享
//! OS temp 目录中其它测试的产物冲突。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{
    BackendMarker, CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker,
    run_cutover,
};
use storyforge_infra_sqlite::exporter::{read_export_manifest, verify_export_tree};
use storyforge_infra_sqlite::importer::JsonImporter;
use storyforge_infra_sqlite::rollback::{
    RollbackFault, RollbackRequest, run_rollback, run_rollback_with_fault, verify_rollback_export,
};
use tempfile::TempDir;

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn sample_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        &json!([{
            "id": "card-1", "name": "Hero", "source_character_id": null
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &json!([{
            "id": "camp-1", "card_id": "card-1", "name": "Main",
            "created_at": "2026-07-13T00:00:00Z", "revision": 0,
            "chronicle_revision": 0, "conversation_id": "conv-1", "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1", "campaign_id": "camp-1", "character_id": null,
            "created_at": "2026-07-13T00:00:00Z", "updated_at": "2026-07-13T00:00:00Z",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &json!([]));
    write_json(&dir.join("knowledge.json"), &json!([]));
    write_json(&dir.join("tasks.json"), &json!([]));
    write_json(&dir.join("round_summaries.json"), &json!([]));
    write_json(&dir.join("turns.json"), &json!([]));
}

/// 建立 SQLite 权威 fixture：`<root>/data/` 为数据目录（cutover 完成，
/// marker = SqliteAuthoritative）。返回 (root, db_path)。
fn sqlite_fixture() -> (TempDir, PathBuf) {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    sample_source(&data_dir);
    let db_path = data_dir.join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(&data_dir, &db_path),
        label: "rollback-fixture".into(),
    };
    match run_cutover(&request).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("cutover must complete: {other:?}"),
    }
    (root, db_path)
}

/// 扫描 `<root>`（staging 产物的唯一产生目录）中的 rollback 残留。
fn rollback_leftovers(root: &Path) -> Vec<String> {
    fs::read_dir(root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".rollback-"))
        .collect()
}

fn wait_for_signal(path: &Path, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for signal {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn rollback_happy_path_flips_marker_and_preserves_db_bytes() {
    let (dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    let before = fs::read(&db_path).unwrap();

    let report = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap();
    assert!(!report.redacted, "rollback export must be lossless");
    assert_eq!(report.cards, 1);
    assert_eq!(report.campaigns, 1);
    assert_eq!(report.instances, 0);
    assert_eq!(report.knowledge, 0);
    assert_eq!(report.tasks, 0);
    assert_eq!(report.summaries, 0);
    assert_eq!(report.conversations, 1);
    assert_eq!(report.turns, 0);
    assert_eq!(report.schema_version, 8);

    assert!(
        matches!(inspect_marker(&plan), MarkerStatus::JsonAuthoritative),
        "marker must flip to JSON authority"
    );
    assert_eq!(
        before,
        fs::read(&db_path).unwrap(),
        "SQLite DB bytes must be untouched by rollback"
    );

    // 产物存在、可验证、可重新导入。
    assert!(
        report.export_dir.exists(),
        "rollback export artifact missing"
    );
    assert!(report.manifest_path.exists());
    verify_export_tree(&report.export_dir).unwrap();
    verify_rollback_export(&report.export_dir).unwrap();
    let manifest = read_export_manifest(&report.manifest_path).unwrap();
    assert_eq!(manifest["mode"], "rollback");
    assert_eq!(manifest["redacted"], json!(false));

    let mut fresh = Database::open(dir.path().join("reimport-check.sqlite3")).unwrap();
    let import = JsonImporter::new(&mut fresh)
        .import_data_dir(&report.export_dir)
        .unwrap();
    assert_eq!(import.cards, 1);
    assert_eq!(import.campaigns, 1);
    assert_eq!(import.conversations, 1);

    // 只留下一个 .rollback-* 产物目录，无其它临时残留。
    let dirs = rollback_leftovers(dir.path());
    assert_eq!(
        dirs.len(),
        1,
        "expected exactly the rollback artifact: {dirs:?}"
    );
}

#[test]
fn rollback_without_confirm_changes_nothing() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    let before = fs::read(&db_path).unwrap();

    let err = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: false,
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("confirm"),
        "missing confirm must fail, got: {err}"
    );
    assert!(
        matches!(
            inspect_marker(&plan),
            MarkerStatus::SqliteAuthoritative { .. }
        ),
        "marker must remain sqlite-authoritative"
    );
    assert_eq!(before, fs::read(&db_path).unwrap(), "DB bytes changed");
    let leftovers = rollback_leftovers(plan.data_dir.parent().unwrap());
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

#[test]
fn rollback_fault_after_export_keeps_sqlite_authoritative() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    let before = fs::read(&db_path).unwrap();

    let err = run_rollback_with_fault(
        &RollbackRequest {
            plan: plan.clone(),
            confirm: true,
        },
        RollbackFault::AfterExport,
    )
    .unwrap_err();
    assert!(err.to_string().contains("injected fault"), "got: {err}");
    assert!(
        matches!(
            inspect_marker(&plan),
            MarkerStatus::SqliteAuthoritative { .. }
        ),
        "marker must remain sqlite-authoritative"
    );
    assert_eq!(before, fs::read(&db_path).unwrap(), "DB bytes changed");
    let leftovers = rollback_leftovers(plan.data_dir.parent().unwrap());
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

#[test]
fn rollback_fault_after_self_check_keeps_sqlite_authoritative() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    let before = fs::read(&db_path).unwrap();

    let err = run_rollback_with_fault(
        &RollbackRequest {
            plan: plan.clone(),
            confirm: true,
        },
        RollbackFault::AfterSelfCheck,
    )
    .unwrap_err();
    assert!(err.to_string().contains("injected fault"), "got: {err}");
    assert!(matches!(
        inspect_marker(&plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
    assert_eq!(before, fs::read(&db_path).unwrap(), "DB bytes changed");
    let leftovers = rollback_leftovers(plan.data_dir.parent().unwrap());
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

#[test]
fn rollback_self_check_rejects_tampered_manifest() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    let report = run_rollback(&RollbackRequest {
        plan,
        confirm: true,
    })
    .unwrap();

    // 篡改产物 manifest 的计数 → 自检必须拒绝（不可恢复证明失效）。
    let mut manifest = read_export_manifest(&report.manifest_path).unwrap();
    manifest.insert(
        "counts".into(),
        json!({"cards": 999, "campaigns": 1, "instances": 0, "knowledge": 0,
               "tasks": 0, "summaries": 1, "conversations": 1, "turns": 0}),
    );
    fs::write(
        &report.manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let err = verify_rollback_export(&report.export_dir).unwrap_err();
    assert!(
        err.to_string().contains("count") || err.to_string().contains("hash"),
        "tampered manifest must fail self-check, got: {err}"
    );
}

#[test]
fn rollback_marker_write_failure_keeps_sqlite_authoritative() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    let before = fs::read(&db_path).unwrap();

    // 占据 marker 的 tmp 路径（原子写第一步）→ 注入 marker 写入失败。
    let marker_tmp = plan.data_dir.join("storyforge.backend.json.tmp");
    fs::create_dir_all(&marker_tmp).unwrap();

    let err = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("marker"),
        "marker write failure must surface, got: {err}"
    );
    assert!(
        matches!(
            inspect_marker(&plan),
            MarkerStatus::SqliteAuthoritative { .. }
        ),
        "marker must remain sqlite-authoritative after failed marker write"
    );
    assert_eq!(before, fs::read(&db_path).unwrap(), "DB bytes changed");
    let leftovers = rollback_leftovers(plan.data_dir.parent().unwrap());
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

#[test]
fn rollback_requires_sqlite_authoritative_marker() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);

    // 手动换成 JsonAuthoritative marker。
    let marker = BackendMarker::json_authoritative();
    let marker_bytes = serde_json::to_vec_pretty(&marker).unwrap();
    let marker_path = plan.marker_path();
    fs::write(&marker_path, &marker_bytes).unwrap();

    let err = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("JsonAuthoritative"),
        "rollback must refuse a JSON-authoritative marker, got: {err}"
    );
    assert_eq!(
        fs::read(&marker_path).unwrap(),
        marker_bytes,
        "marker must be untouched"
    );
    let leftovers = rollback_leftovers(plan.data_dir.parent().unwrap());
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

#[test]
fn rollback_requires_sqlite_authoritative_marker_when_absent() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    fs::remove_file(plan.marker_path()).unwrap();

    let err = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("Absent"),
        "rollback must refuse an absent marker, got: {err}"
    );
    assert!(!matches!(
        inspect_marker(&plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

#[test]
fn shared_authority_lease_blocks_rollback_until_release() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);

    let exe = std::env::var("CARGO_BIN_EXE_lease_hold")
        .expect("CARGO_BIN_EXE_lease_hold must point to the compiled helper binary");
    let signals = TempDir::new().unwrap();
    let ready = signals.path().join("ready");
    let release = signals.path().join("release");
    let mut child = std::process::Command::new(&exe)
        .arg(&plan.data_dir)
        .arg("shared")
        .arg(&ready)
        .arg(&release)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn lease_hold");
    wait_for_signal(&ready, 60);

    // 写者进程持有 SHARED 租约 → rollback 的 EXCLUSIVE 租约必须 fail closed。
    let err = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("authority lease"),
        "rollback must be blocked by a shared writer lease, got: {err}"
    );
    assert!(
        matches!(
            inspect_marker(&plan),
            MarkerStatus::SqliteAuthoritative { .. }
        ),
        "blocked rollback must not touch the marker"
    );

    fs::write(&release, b"go").unwrap();
    assert!(
        child.wait().unwrap().success(),
        "lease_hold must exit cleanly"
    );

    // 释放后 rollback 成功。
    let report = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap();
    assert_eq!(report.cards, 1);
    assert!(matches!(
        inspect_marker(&plan),
        MarkerStatus::JsonAuthoritative
    ));
}

#[test]
fn second_rollback_after_success_is_rejected() {
    let (_dir, db_path) = sqlite_fixture();
    let plan = CutoverPlan::new(db_path.parent().unwrap(), &db_path);
    run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap();

    let err = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("JsonAuthoritative"),
        "second rollback must be rejected, got: {err}"
    );
    assert!(matches!(
        inspect_marker(&plan),
        MarkerStatus::JsonAuthoritative
    ));
}

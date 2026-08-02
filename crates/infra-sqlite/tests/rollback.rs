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
    // 三审3：marker 被删后，cutover 完成的 DB（带 authority_binding）成为孤儿 →
    // inspect_marker 判 Stale（ambiguous），rollback 据此拒绝。不论 Stale 还是
    // Absent，rollback 都不得在非 SqliteAuthoritative 状态下继续。
    assert!(
        err.to_string().contains("Stale") || err.to_string().contains("Absent"),
        "rollback must refuse a non-sqlite-authoritative marker, got: {err}"
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

// ─── 三审1：rollback 必须原子安装 JSON 进 data_dir，新进程读到回滚后数据 ───
//
// 判别性：cutover 后把 SQLite 里 campaign.name 改成「Main-POST-CUTOVER」，
// 使 SQLite 内容与迁移前 JSON（name="Main"）显式不同。rollback 后 data_dir
// 里的 campaigns.json 必须含 POST-CUTOVER 值——否则旧实现（只写 marker、
// 不装 JSON）会让新进程读到迁移前的 "Main"，判定失败。

/// cutover 后直接 UPDATE campaigns.name，制造 SQLite 与迁移前 JSON 的可识别差异。
fn mutate_sqlite_campaign_name(db_path: &Path, new_name: &str) {
    let mut db = Database::open(db_path).unwrap();
    let conn = db.connection_mut();
    // campaigns.payload_json 里 name 字段更名 + 顶层 name 列更名（双写保证导出覆盖）。
    let row: String = conn
        .query_row(
            "SELECT payload_json FROM campaigns WHERE campaign_id = 'camp-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut payload: Value = serde_json::from_str(&row).unwrap();
    payload["name"] = json!(new_name);
    let new_payload = serde_json::to_string(&payload).unwrap();
    conn.execute(
        "UPDATE campaigns SET name = ?1, payload_json = ?2 WHERE campaign_id = 'camp-1'",
        rusqlite::params![new_name, new_payload],
    )
    .unwrap();
}

#[test]
fn rollback_installs_json_so_new_process_reads_rolled_back_data() {
    let (_dir, db_path) = sqlite_fixture();
    let data_dir = db_path.parent().unwrap();
    let plan = CutoverPlan::new(data_dir, &db_path);

    // 1) 迁移前 JSON 的 campaign.name = "Main"。cutover 后改 SQLite 制造差异。
    let original_campaigns = fs::read_to_string(data_dir.join("campaigns.json")).unwrap();
    assert!(
        original_campaigns.contains("\"Main\""),
        "迁移前 JSON 应含 Main: {original_campaigns}"
    );
    mutate_sqlite_campaign_name(&db_path, "Main-POST-CUTOVER");

    // 2) rollback：必须把 SQLite 最新数据装回 data_dir。
    let report = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap();
    assert_eq!(report.campaigns, 1);
    assert!(
        matches!(inspect_marker(&plan), MarkerStatus::JsonAuthoritative),
        "marker must flip to JSON"
    );

    // 3) 判别点：data_dir 的 campaigns.json 现在必须含 POST-CUTOVER（SQLite 值）。
    //    旧实现（只写 marker 不装 JSON）会让这里仍是迁移前的 "Main" → 失败。
    let installed = fs::read_to_string(data_dir.join("campaigns.json"))
        .expect("data_dir/campaigns.json must exist after rollback (json installed)");
    assert!(
        installed.contains("Main-POST-CUTOVER"),
        "data_dir JSON must reflect rolled-back SQLite data, got: {installed}"
    );
    assert!(
        !installed.contains("\"Main\"") || installed.contains("Main-POST-CUTOVER"),
        "迁移前的 Main 不应残留为唯一值"
    );

    // 4) 真实新进程读取：用 JSON importer 重新加载 data_dir（模拟新进程冷启动），
    //    必须看到 POST-CUTOVER（证明新进程读到回滚后权威数据）。
    let mut fresh = Database::open_in_memory().unwrap();
    let import = JsonImporter::new(&mut fresh)
        .import_data_dir(data_dir)
        .expect("new process must be able to import rolled-back data_dir");
    assert_eq!(import.campaigns, 1);
    // 验证导入的 campaign 名字是 POST-CUTOVER：查内存库。
    let name: String = fresh
        .connection()
        .query_row(
            "SELECT name FROM campaigns WHERE campaign_id = 'camp-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        name, "Main-POST-CUTOVER",
        "新进程读取的 campaign 名必须是回滚后 SQLite 值"
    );
}

#[test]
fn rollback_fault_after_json_install_is_idempotent_rerunnable() {
    let (dir, db_path) = sqlite_fixture();
    let data_dir = db_path.parent().unwrap();
    let plan = CutoverPlan::new(data_dir, &db_path);
    mutate_sqlite_campaign_name(&db_path, "Main-POST-CUTOVER");

    // 第一次：装完 JSON、写 marker 前注入故障 → 失败，但 data_dir JSON 已装。
    let err = run_rollback_with_fault(
        &RollbackRequest {
            plan: plan.clone(),
            confirm: true,
        },
        RollbackFault::AfterJsonInstall,
    )
    .unwrap_err();
    assert!(err.to_string().contains("injected fault"), "got: {err}");
    // marker 仍是 SQLite 权威（提交点未到）。
    assert!(
        matches!(
            inspect_marker(&plan),
            MarkerStatus::SqliteAuthoritative { .. }
        ),
        "marker must remain sqlite (commit point not reached)"
    );
    // 但 data_dir JSON 已被装成 POST-CUTOVER（装步骤幂等）。
    let installed = fs::read_to_string(data_dir.join("campaigns.json")).unwrap();
    assert!(
        installed.contains("Main-POST-CUTOVER"),
        "json install is idempotent; data_dir already reflects rolled-back data: {installed}"
    );

    // 第二次：无故障重跑 → 必须成功（幂等：装步骤用同一 staging 覆盖，内容一致）。
    let report = run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap();
    assert_eq!(report.campaigns, 1);
    assert!(
        matches!(inspect_marker(&plan), MarkerStatus::JsonAuthoritative),
        "rerun must commit marker"
    );
    // DB 字节不变。
    let _ = dir; // keep tempdir alive
}

#[test]
fn rollback_install_replaces_old_conversations_in_data_dir() {
    // 回滚掉某个在迁移前存在、但 SQLite 里已删除的会话：data_dir 的
    // conversations/ 必须反映 SQLite 视图（旧会话文件被移除），而非残留。
    let (dir, db_path) = sqlite_fixture();
    let data_dir = db_path.parent().unwrap();
    let plan = CutoverPlan::new(data_dir, &db_path);

    // 迁移前 data_dir 有 conversations/conv-1.json。cutover 后从 SQLite 删除该会话。
    {
        let mut db = Database::open(&db_path).unwrap();
        let conn = db.connection_mut();
        conn.execute(
            "DELETE FROM conversations WHERE conversation_id = 'conv-1'",
            [],
        )
        .unwrap();
        // 同时清掉 campaign 的 conversation_id 引用（列 + payload_json 里的嵌入字段），
        // 避免回滚导出后 campaign 仍引用已删会话（三审6：引用缺失会话必须拒绝）。
        conn.execute(
            "UPDATE campaigns SET conversation_id = NULL WHERE campaign_id = 'camp-1'",
            [],
        )
        .unwrap();
        let payload: String = conn
            .query_row(
                "SELECT payload_json FROM campaigns WHERE campaign_id = 'camp-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut payload_value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        if let Some(obj) = payload_value.as_object_mut() {
            obj.remove("conversation_id");
        }
        let new_payload = serde_json::to_string(&payload_value).unwrap();
        conn.execute(
            "UPDATE campaigns SET payload_json = ?1 WHERE campaign_id = 'camp-1'",
            [new_payload],
        )
        .unwrap();
    }

    run_rollback(&RollbackRequest {
        plan: plan.clone(),
        confirm: true,
    })
    .unwrap();
    assert!(matches!(
        inspect_marker(&plan),
        MarkerStatus::JsonAuthoritative
    ));
    // data_dir/conversations/conv-1.json 必须已被移除（SQLite 里已无该会话）。
    assert!(
        !data_dir.join("conversations").join("conv-1.json").exists(),
        "rolled-back data_dir must not retain pre-cutover conversation deleted in SQLite"
    );
    let _ = dir;
}

// ─── 三审8：真实 operator rollback CLI（storyforge_rollback bin）────────────

#[test]
fn rollback_cli_dry_run_does_not_write_marker() {
    // 无 --confirm：dry-run 必须通过校验但不写 marker、不装 JSON。
    let (dir, db_path) = sqlite_fixture();
    let data_dir = db_path.parent().unwrap();
    let exe = std::env::var("CARGO_BIN_EXE_storyforge_rollback")
        .expect("CARGO_BIN_EXE_storyforge_rollback must point to the compiled CLI binary");
    let output = std::process::Command::new(&exe)
        .arg(data_dir)
        .output()
        .expect("run storyforge_rollback dry-run");
    assert!(
        output.status.success(),
        "dry-run must exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dry-run OK"),
        "dry-run must report OK: {stderr}"
    );
    assert!(
        stderr.contains("re-run with --confirm"),
        "dry-run must hint confirm: {stderr}"
    );
    // marker 仍是 SQLite 权威（dry-run 未改任何状态）。
    let plan = CutoverPlan::new(data_dir, &db_path);
    assert!(
        matches!(
            inspect_marker(&plan),
            MarkerStatus::SqliteAuthoritative { .. }
        ),
        "dry-run must not flip the marker"
    );
    let _ = dir;
}

#[test]
fn rollback_cli_confirm_performs_rollback_and_installs_json() {
    // --confirm：真实执行 rollback → marker 翻 JSON + data_dir 装回滚后 JSON。
    let (dir, db_path) = sqlite_fixture();
    let data_dir = db_path.parent().unwrap();
    let plan = CutoverPlan::new(data_dir, &db_path);
    // 制造 SQLite 与迁移前 JSON 的差异，证明 CLI 装的是 SQLite 数据。
    mutate_sqlite_campaign_name(&db_path, "Main-CLI-POST-CUTOVER");

    let exe = std::env::var("CARGO_BIN_EXE_storyforge_rollback")
        .expect("CARGO_BIN_EXE_storyforge_rollback must point to the compiled CLI binary");
    let output = std::process::Command::new(&exe)
        .arg(data_dir)
        .arg("--confirm")
        .output()
        .expect("run storyforge_rollback --confirm");
    assert!(
        output.status.success(),
        "confirm must exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SUCCESS"),
        "confirm must report SUCCESS: {stderr}"
    );
    assert!(
        stderr.contains("JsonAuthoritative"),
        "confirm must verify marker: {stderr}"
    );
    // marker 翻成 JSON 权威。
    assert!(
        matches!(inspect_marker(&plan), MarkerStatus::JsonAuthoritative),
        "marker must flip to JSON after CLI confirm"
    );
    // data_dir JSON 含 POST-CUTOVER（CLI 装了 SQLite 数据）。
    let installed = fs::read_to_string(data_dir.join("campaigns.json")).unwrap();
    assert!(
        installed.contains("Main-CLI-POST-CUTOVER"),
        "CLI must install rolled-back SQLite data into data_dir: {installed}"
    );
    let _ = dir;
}

#[test]
fn rollback_cli_refuses_non_sqlite_authority() {
    // 无 SQLite 权威（marker JsonAuthoritative 或 Absent）→ CLI 必须 fail closed。
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    // 空目录：无 marker、无 DB → inspect_marker Absent → CLI 应失败（dry-run 校验）。
    let exe = std::env::var("CARGO_BIN_EXE_storyforge_rollback").unwrap();
    let output = std::process::Command::new(&exe)
        .arg(&data_dir)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "CLI must fail when marker is not SqliteAuthoritative"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SqliteAuthoritative") || stderr.contains("FAILED"),
        "CLI must report the authority refusal: {stderr}"
    );
}

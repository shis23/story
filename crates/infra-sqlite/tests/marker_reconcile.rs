//! `reconcile_marker_schema_version` 判别测试（Gate 8 审查 P1-1）。
//!
//! 场景：`sqlite_runtime::activate` 会把数据库迁移到最新 schema，marker
//! 记录的 schema_version 若不同步回写，下次启动 `inspect_marker` 判 Stale
//! 拒绝启动。本套件验证对账只在「sqlite 权威 + db 版本更高」时回写，且
//! 保留 authority 身份字段；版本相等/无 marker/db 更旧时均为 no-op。

use std::fs;
use std::path::Path;

use serde_json::Value;
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker,
    reconcile_marker_schema_version, recover_or_verify,
};
use storyforge_infra_sqlite::migrations::current_version;
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
        &serde_json::json!([{
            "id": "card-1",
            "name": "Hero",
            "source_character_id": null
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &serde_json::json!([{
            "id": "camp-1",
            "card_id": "card-1",
            "name": "Main",
            "created_at": "2026-07-13T00:00:00Z",
            "revision": 0,
            "chronicle_revision": 0,
            "conversation_id": "conv-1",
            "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &serde_json::json!({
            "id": "conv-1",
            "campaign_id": "camp-1",
            "character_id": null,
            "created_at": "2026-07-13T00:00:00Z",
            "updated_at": "2026-07-13T00:00:00Z",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &serde_json::json!([]));
    write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
    write_json(&dir.join("tasks.json"), &serde_json::json!([]));
    write_json(&dir.join("round_summaries.json"), &serde_json::json!([]));
    write_json(&dir.join("turns.json"), &serde_json::json!([]));
    write_json(&dir.join("mvu_translations.json"), &serde_json::json!([]));
    write_json(&dir.join("compress_jobs.json"), &serde_json::json!([]));
    write_json(&dir.join("characters.json"), &serde_json::json!([]));
    write_json(
        &dir.join("campaign_world_info.json"),
        &serde_json::json!([]),
    );
}

fn make_plan(dir: &Path) -> CutoverPlan {
    CutoverPlan::new(dir, dir.join("storyforge.sqlite3"))
}

/// 完成一次真实 cutover，返回 (plan, marker 路径)。
fn completed_cutover(dir: &Path) -> CutoverPlan {
    sample_source(dir);
    let plan = make_plan(dir);
    let request = CutoverRequest {
        plan: plan.clone(),
        label: "marker-reconcile-test".to_string(),
        allow_json_authoritative_flip: false,
    };
    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    assert!(matches!(
        inspect_marker(&plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
    plan
}

fn read_marker(dir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(dir.join("storyforge.backend.json")).unwrap()).unwrap()
}

fn write_marker(dir: &Path, marker: &Value) {
    fs::write(
        dir.join("storyforge.backend.json"),
        serde_json::to_vec_pretty(marker).unwrap(),
    )
    .unwrap();
}

#[test]
fn reconcile_updates_marker_after_startup_migration() {
    let dir = TempDir::new().unwrap();
    let plan = completed_cutover(dir.path());

    let db = storyforge_infra_sqlite::Database::open(&plan.db_path).unwrap();
    let current = current_version(&db).unwrap();
    assert!(current >= 1);

    // 模拟「旧 marker + 已迁移库」：把 marker 的 schema_version 改小。
    let original = read_marker(dir.path());
    let authority_id = original["authority_id"].as_str().unwrap().to_string();
    let nonce = original["cutover_nonce"].as_str().unwrap().to_string();
    let manifest_hash = original["manifest_hash"].as_str().unwrap().to_string();
    let mut stale = original.clone();
    stale["schema_version"] = serde_json::json!(current - 1);
    write_marker(dir.path(), &stale);

    // 对账应回写版本并保留身份字段。
    let reconciled = reconcile_marker_schema_version(&plan.db_path).unwrap();
    assert_eq!(reconciled, Some((current - 1, current)));

    let after = read_marker(dir.path());
    assert_eq!(after["schema_version"], serde_json::json!(current));
    assert_eq!(after["authority_id"].as_str().unwrap(), authority_id);
    assert_eq!(after["cutover_nonce"].as_str().unwrap(), nonce);
    assert_eq!(after["manifest_hash"].as_str().unwrap(), manifest_hash);

    // 修复后 inspect_marker 恢复 SqliteAuthoritative（而非 Stale 锁死）。
    assert!(matches!(
        inspect_marker(&plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

#[test]
fn reconcile_is_noop_when_versions_match() {
    let dir = TempDir::new().unwrap();
    let plan = completed_cutover(dir.path());

    let before = read_marker(dir.path());
    let reconciled = reconcile_marker_schema_version(&plan.db_path).unwrap();
    assert_eq!(reconciled, None);

    // marker 文件不变。
    let after = read_marker(dir.path());
    assert_eq!(before, after);
}

#[test]
fn reconcile_is_noop_without_marker() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let plan = make_plan(dir.path());
    // 未 cutover：无 marker。
    assert!(!dir.path().join("storyforge.backend.json").exists());
    assert_eq!(
        reconcile_marker_schema_version(&plan.db_path).unwrap(),
        None
    );
}

#[test]
fn reconcile_keeps_fail_closed_when_db_older_than_marker() {
    let dir = TempDir::new().unwrap();
    let plan = completed_cutover(dir.path());

    let db = storyforge_infra_sqlite::Database::open(&plan.db_path).unwrap();
    let current = current_version(&db).unwrap();

    // 把 marker 版本改大（db 更旧）：对账必须 no-op，保持 fail-closed。
    let mut too_new = read_marker(dir.path());
    too_new["schema_version"] = serde_json::json!(current + 1);
    write_marker(dir.path(), &too_new);

    assert_eq!(
        reconcile_marker_schema_version(&plan.db_path).unwrap(),
        None
    );
    let after = read_marker(dir.path());
    assert_eq!(after["schema_version"], serde_json::json!(current + 1));
    // inspect_marker 仍判 Stale（版本不符），拒绝启动——符合安全姿态。
    assert!(matches!(inspect_marker(&plan), MarkerStatus::Stale { .. }));
}

#[test]
fn reconcile_is_noop_for_json_authoritative_marker() {
    let dir = TempDir::new().unwrap();
    let plan = make_plan(dir.path());
    // JsonAuthoritative marker（reverse-cutover 产物）：不得对账改写。
    let json_marker = serde_json::json!({
        "version": 1,
        "backend": "json",
        "schema_version": 0,
        "manifest_hash": "",
        "created_at": "2026-07-13T00:00:00Z"
    });
    write_marker(dir.path(), &json_marker);

    assert_eq!(
        reconcile_marker_schema_version(&plan.db_path).unwrap(),
        None
    );
    let after = read_marker(dir.path());
    assert_eq!(after["backend"], serde_json::json!("json"));
}

#[test]
fn json_authoritative_flip_requires_explicit_optin() {
    use storyforge_infra_sqlite::cutover::{CutoverOutcome, run_cutover};
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let plan = make_plan(dir.path());
    let json_marker = serde_json::json!({
        "version": 1,
        "backend": "json",
        "schema_version": 0,
        "manifest_hash": "",
        "created_at": "2026-07-13T00:00:00Z"
    });
    write_marker(dir.path(), &json_marker);

    // 无 opt-in：基础层必须拒绝翻转 JSON 权威（Gate 8 审查 P2-A2）。
    let request = CutoverRequest {
        plan: plan.clone(),
        label: "no-optin".to_string(),
        allow_json_authoritative_flip: false,
    };
    let err = run_cutover(&request).unwrap_err();
    assert!(
        err.to_string().contains("opt-in"),
        "must demand explicit opt-in, got: {err}"
    );
    assert!(
        !dir.path().join("storyforge.sqlite3").exists(),
        "no DB may be created without opt-in"
    );

    // 显式 opt-in（app 层 env=sqlite 映射）：翻转成功。
    let request = CutoverRequest {
        plan,
        label: "optin".to_string(),
        allow_json_authoritative_flip: true,
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

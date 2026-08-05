//! Gate 5 审查二：reverse-export 安全加固测试。
//!
//! 覆盖：
//! - 二.2 路径穿越与文件名碰撞（会话/世界书 id；写后重读校验 verify_export_tree）
//! - 二.3 rollback 导出（无损）与 diagnostic 导出（可脱敏）拆分
//! - 二.4 mutation_commits / chronicle_publication_jobs 非空必须 fail-closed
//! - 二.5 导出前校验（未迁移/旧 schema/checksum/损坏 payload/FK 违规/缺表）
//! - 二.6 原子发布 + 故障注入恢复 + 跨进程 per-target 导出锁

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{CutoverOutcome, CutoverPlan, CutoverRequest, run_cutover};
use storyforge_infra_sqlite::exporter::{
    ExportFault, ExportMode, acquire_export_lock, export_sqlite_to_json,
    export_sqlite_to_json_with_fault, export_sqlite_to_json_with_mode, read_export_manifest,
    verify_export_tree,
};
use storyforge_infra_sqlite::importer::JsonImporter;
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

fn cutover(dir: &TempDir) -> PathBuf {
    let db_path = dir.path().join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "safety".into(),
        allow_json_authoritative_flip: false,
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    db_path
}

fn fresh_target(name: &str) -> PathBuf {
    TempDir::new().unwrap().path().join(name)
}

fn insert_conversation(db: &Database, id: &str) {
    db.connection()
        .execute(
            "INSERT INTO conversations (conversation_id, campaign_id, character_id, archived_upto, created_at, updated_at, payload_json) \
             VALUES (?1, 'camp-1', NULL, 0, 'now', 'now', ?2)",
            rusqlite::params![
                id,
                json!({"id": id, "campaign_id": "camp-1", "nodes": []}).to_string()
            ],
        )
        .unwrap();
}

fn insert_world_info(db: &Database, campaign_id: &str) {
    db.connection()
        .execute(
            "INSERT INTO campaign_world_info (campaign_id, payload_json, updated_at) \
             VALUES (?1, ?2, 'now')",
            rusqlite::params![campaign_id, json!({"entries": []}).to_string()],
        )
        .unwrap();
}

// ─── 二.2 路径穿越与文件名碰撞 ───────────────────────────────────────────

#[test]
fn conversation_ids_with_path_components_are_rejected() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();

    for bad_id in ["../evil", "/abs/evil", "a\\b", "a/../b"] {
        insert_conversation(&db, bad_id);
        let target = fresh_target("out");
        let err = export_sqlite_to_json(&db, &target).unwrap_err();
        assert!(
            err.to_string().contains("refusing"),
            "id {bad_id:?}: expected rejection, got: {err}"
        );
        assert!(!target.exists(), "id {bad_id:?}: partial target created");
        db.connection()
            .execute(
                "DELETE FROM conversations WHERE conversation_id = ?1",
                [bad_id],
            )
            .unwrap();
    }
    // 正常 id 不受影响。
    let target = fresh_target("ok");
    export_sqlite_to_json(&db, &target).unwrap();
}

#[test]
fn conversation_ids_with_unsafe_chars_roundtrip_via_injective_encoding() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    let tricky = "conv:a b"; // ':' 与空格不在可移植安全字符集 → 注入式百分号编码
    insert_conversation(&db, tricky);

    let target = fresh_target("out");
    let result = export_sqlite_to_json(&db, &target).unwrap();
    assert_eq!(result.report.conversations, 2);
    // 编码文件名必须可写且精确（小写十六进制百分号编码）：conv%3aa%20b.json
    assert!(
        target
            .join("conversations")
            .join("conv%3aa%20b.json")
            .exists(),
        "encoded conversation file missing"
    );
    assert!(target.join("conversations").join("conv-1.json").exists());
    drop(db);

    // 重新导入 → payload id 精确还原（文件名不参与回读，payload id 权威）。
    let mut fresh = Database::open(fresh_target("reimport.sqlite3")).unwrap();
    let report = JsonImporter::new(&mut fresh)
        .import_data_dir(&target)
        .unwrap();
    assert_eq!(report.conversations, 2);
    let back: String = fresh
        .connection()
        .query_row(
            "SELECT payload_json FROM conversations WHERE conversation_id = ?1",
            [tricky],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&back).unwrap()["id"], tricky);
}

#[test]
fn conversation_case_collision_is_rejected_not_overwritten() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    insert_conversation(&db, "Case-1");
    insert_conversation(&db, "case-1");

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("collide"),
        "case collision must be rejected, got: {err}"
    );
    assert!(!target.exists());
}

#[test]
fn world_info_campaign_ids_must_be_safe_raw_filenames() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();

    // 世界书文件名即 campaign_id（importer 以文件名为 id），无法编码 → 严格拒绝。
    // campaign_world_info.campaign_id 有 FK 引用 campaigns：先建同名 campaign
    // 行（FK-off 连接，绕过顺序问题），让校验走到文件名检查本身。
    for bad_id in ["../evil", "camp:1", "a\\b", "/abs"] {
        let raw = rusqlite::Connection::open(&db_path).unwrap();
        raw.execute_batch(&format!(
            "PRAGMA foreign_keys = OFF; \
             INSERT INTO campaigns (campaign_id, card_id, name, revision, chronicle_revision, \
                 story_clock, created_at, payload_json) \
             VALUES ('{bad_id}', 'card-1', 'U', 0, 0, 'Day 1', 'now', '{{}}'); \
             INSERT INTO campaign_world_info (campaign_id, payload_json, updated_at) \
             VALUES ('{bad_id}', '{{}}', 'now');"
        ))
        .unwrap();
        drop(raw);

        let target = fresh_target("out");
        let err = export_sqlite_to_json(&db, &target).unwrap_err();
        assert!(
            err.to_string().contains("refusing"),
            "world-info campaign id {bad_id:?}: expected rejection, got: {err}"
        );
        assert!(!target.exists());
        db.connection()
            .execute(
                "DELETE FROM campaign_world_info WHERE campaign_id = ?1",
                [bad_id],
            )
            .unwrap();
        db.connection()
            .execute("DELETE FROM campaigns WHERE campaign_id = ?1", [bad_id])
            .unwrap();
    }
    // 安全 id（UUID 形态）正常导出并可回读。
    let safe_id = "c8d3c7c0-0000-4000-8000-000000000001";
    db.connection()
        .execute(
            "INSERT INTO campaigns (campaign_id, card_id, name, revision, chronicle_revision, \
                 story_clock, created_at, payload_json) \
             VALUES (?1, 'card-1', 'U', 0, 0, 'Day 1', 'now', '{}')",
            [safe_id],
        )
        .unwrap();
    insert_world_info(&db, safe_id);
    let target = fresh_target("ok");
    export_sqlite_to_json(&db, &target).unwrap();
    assert!(
        target
            .join("campaign_world_info")
            .join(format!("{safe_id}.json"))
            .exists()
    );
}

#[test]
fn world_info_case_collision_is_rejected() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    // 两个真实 campaign（FK 通过），世界书文件名大小写归一化碰撞 → Err。
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    raw.execute_batch(
        "PRAGMA foreign_keys = OFF; \
         INSERT INTO campaigns (campaign_id, card_id, name, revision, chronicle_revision, \
             story_clock, created_at, payload_json) \
         VALUES ('CampA', 'card-1', 'U', 0, 0, 'Day 1', 'now', '{}'), \
                ('campa', 'card-1', 'U', 0, 0, 'Day 1', 'now', '{}'); \
         INSERT INTO campaign_world_info (campaign_id, payload_json, updated_at) \
         VALUES ('CampA', '{}', 'now'), ('campa', '{}', 'now');",
    )
    .unwrap();
    drop(raw);

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("collide"),
        "world-info case collision must be rejected, got: {err}"
    );
}

#[test]
fn verify_export_tree_accepts_untouched_export() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    let target = fresh_target("out");
    let result = export_sqlite_to_json(&db, &target).unwrap();
    verify_export_tree(&result.export_dir).unwrap();
}

#[test]
fn verify_export_tree_rejects_deleted_file() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    let target = fresh_target("out");
    let result = export_sqlite_to_json(&db, &target).unwrap();
    // 导出完成后删除一个文件 → 重读校验必须失败（计数/缺失）。
    fs::remove_file(target.join("cards.json")).unwrap();
    let err = verify_export_tree(&result.export_dir).unwrap_err();
    assert!(
        err.to_string().contains("verification"),
        "deleted-file tamper must be caught, got: {err}"
    );
}

#[test]
fn verify_export_tree_rejects_modified_content() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    let target = fresh_target("out");
    let result = export_sqlite_to_json(&db, &target).unwrap();
    // 篡改 cards.json 内容 → 计数或 hash 不匹配。
    fs::write(target.join("cards.json"), b"[]").unwrap();
    let err = verify_export_tree(&result.export_dir).unwrap_err();
    assert!(
        err.to_string().contains("verification"),
        "content tamper must be caught, got: {err}"
    );
}

// ─── 二.3 rollback 导出（无损）与 diagnostic 导出（可脱敏）拆分 ──────────

/// 运行时拼装 secret 形态内容（静态扫描器不识别 fixture）。
fn seed_secret_shaped_card(db: &Database) -> Value {
    // Intentionally short (<20 chars after `sk-`) so the release secret-scan's
    // boundary-aware OpenAI-key rule `sk-[A-Za-z0-9_-]{20,}` does NOT match this
    // test fixture. The token still carries the `sk-` prefix the diagnostic
    // redaction path keys on, preserving the rollback-lossless / diagnostic-redacts
    // discrimination without tripping the release gate.
    let token = format!("{}{}", "sk-", "test-fixture-x9");
    let payload = json!({
        "id": "card-1",
        "name": "Hero",
        "api_key": token,
        "note": format!("password={}", "hunter2"),
        "local_path": format!("C:\\Users\\{}", "DemoUser"),
    });
    db.connection()
        .execute(
            "UPDATE character_cards SET payload_json = ?1 WHERE card_id = 'card-1'",
            [payload.to_string()],
        )
        .unwrap();
    payload
}

#[test]
fn rollback_mode_export_is_lossless_for_secret_shaped_content() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    let seeded = seed_secret_shaped_card(&db);

    let target = fresh_target("rollback-out");
    let result = export_sqlite_to_json_with_mode(&db, &target, ExportMode::Rollback).unwrap();
    assert!(!result.report.redacted, "rollback export must not redact");
    assert!(
        !result
            .report
            .unsupported_fields
            .iter()
            .any(|f| f == "sensitive_field" || f == "free_text_secret" || f == "absolute_path"),
        "rollback export must not classify redaction categories: {:?}",
        result.report.unsupported_fields
    );
    drop(db);

    let exported_cards: Value =
        serde_json::from_str(&fs::read_to_string(target.join("cards.json")).unwrap()).unwrap();
    assert_eq!(
        exported_cards[0]["api_key"], "sk-test-fixture-x9",
        "rollback export lost the token value"
    );
    assert_eq!(
        exported_cards[0]["note"], "password=hunter2",
        "rollback export lost the secret text"
    );
    assert_eq!(
        exported_cards[0]["local_path"], "C:\\Users\\DemoUser",
        "rollback export lost the absolute path"
    );

    // 重新导入 → 字段与导出前完全一致。
    let mut fresh = Database::open(fresh_target("reimport.sqlite3")).unwrap();
    JsonImporter::new(&mut fresh)
        .import_data_dir(&target)
        .unwrap();
    let back: String = fresh
        .connection()
        .query_row(
            "SELECT payload_json FROM character_cards WHERE card_id = 'card-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&back).unwrap(), seeded);
}

#[test]
fn diagnostic_mode_redacts_and_declares_redacted() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    seed_secret_shaped_card(&db);

    let target = fresh_target("diagnostic-out");
    let result = export_sqlite_to_json(&db, &target).unwrap();
    assert!(
        result.report.redacted,
        "diagnostic export must declare redacted=true"
    );
    assert!(
        result
            .report
            .unsupported_fields
            .iter()
            .any(|f| f == "sensitive_field"),
        "expected sensitive_field classification: {:?}",
        result.report.unsupported_fields
    );
    let exported = fs::read_to_string(target.join("cards.json")).unwrap();
    assert!(
        !exported.contains("sk-test-fixture-x9"),
        "diagnostic export leaked token"
    );
    assert!(
        !exported.contains("api_key"),
        "diagnostic export leaked field name"
    );
    // manifest 必须声明模式与脱敏状态（产物不可声称可恢复）。
    let manifest = read_export_manifest(&result.manifest_path).unwrap();
    assert_eq!(manifest["mode"], "diagnostic");
    assert_eq!(manifest["redacted"], json!(true));
}

#[test]
fn rollback_manifest_declares_rollback_and_no_redaction() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    let target = fresh_target("rollback-out");
    let result = export_sqlite_to_json_with_mode(&db, &target, ExportMode::Rollback).unwrap();
    let manifest = read_export_manifest(&result.manifest_path).unwrap();
    assert_eq!(manifest["mode"], "rollback");
    assert_eq!(manifest["redacted"], json!(false));
}

// ─── 二.4 mutation_commits / chronicle_publication_jobs 无损导出（fail-closed）───

#[test]
fn export_fails_closed_when_mutation_commits_are_nonempty() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute_batch(
            r#"
            INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id,
                base_campaign_revision, status, created_at, updated_at, payload_json)
            VALUES ('t-ledger', 'camp-1', 'conv-1', 'input-1', 0, 'committed', 'now', 'now', '{}');
            INSERT INTO turn_attempts (attempt_id, turn_id, variant_id, draft_hash,
                status, created_at, payload_json)
            VALUES ('a-ledger', 't-ledger', 'v-1', 'h', 'committed', 'now', '{}');
            INSERT INTO mutation_commits (commit_id, campaign_id, turn_id, attempt_id,
                expected_revision, target_revision, terminal_status, payload_hash, committed_at)
            VALUES ('c-1', 'camp-1', 't-ledger', 'a-ledger', 0, 1, 'committed', 'hash', 'now');
            "#,
        )
        .unwrap();

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("mutation_commits"),
        "non-empty mutation_commits must fail closed, got: {err}"
    );
    assert!(
        !target.exists(),
        "fail-closed export must leave no partial target"
    );
    // DB 未被改动。
    drop(db);
}

#[test]
fn export_fails_closed_when_chronicle_publication_jobs_are_nonempty() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute(
            r#"
            INSERT INTO chronicle_publication_jobs (
                publication_id, campaign_id, job_id, base_chronicle_revision,
                target_chronicle_revision, parent_ids_json, child_covered_by_json,
                payload_hash, status, created_at, completed_at)
            VALUES ('p-1', 'camp-1', 'job-1', 1, 2, '[]', '[]', 'hash', 'completed', 'now', 'now')
            "#,
            [],
        )
        .unwrap();

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("chronicle_publication_jobs"),
        "non-empty chronicle_publication_jobs must fail closed, got: {err}"
    );
    assert!(!target.exists());
}

// ─── 二.5 导出前校验 ─────────────────────────────────────────────────────

#[test]
fn export_refuses_unmigrated_empty_database() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("empty.sqlite3");
    // Database::open 创建空文件但不迁移。先关闭首个句柄（WAL checkpoint 把
    // PRAGMA 写入落盘），以「用户实际观察到的」字节为基线。
    {
        let _db = Database::open(&db_path).unwrap();
    }
    let before = fs::read(&db_path).unwrap();

    let db = Database::open(&db_path).unwrap();
    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("not migrated") || err.to_string().contains("schema_migrations"),
        "unmigrated empty DB must be refused, got: {err}"
    );
    drop(db);
    assert_eq!(
        before,
        fs::read(&db_path).unwrap(),
        "unmigrated DB bytes changed"
    );
    assert!(!target.exists());
}

#[test]
fn migrated_empty_database_exports_successfully_with_zero_counts() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("fresh.sqlite3");
    let mut db = Database::open(&db_path).unwrap();
    storyforge_infra_sqlite::migrations::migrate(&mut db).unwrap();

    let target = fresh_target("out");
    let result = export_sqlite_to_json(&db, &target).unwrap();
    assert_eq!(result.report.cards, 0);
    assert_eq!(result.report.campaigns, 0);
    assert_eq!(result.report.conversations, 0);
    assert_eq!(result.report.turns, 0);
    let manifest = read_export_manifest(&result.manifest_path).unwrap();
    assert_eq!(manifest["counts"]["cards"], json!(0));
}

#[test]
fn export_refuses_old_schema_version() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute("DELETE FROM schema_migrations WHERE version = 8", [])
        .unwrap();

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("schema version"),
        "old schema must be refused, got: {err}"
    );
    assert!(!target.exists());
}

#[test]
fn export_refuses_tampered_migration_checksum() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute(
            "UPDATE schema_migrations SET checksum = 'deadbeef' WHERE version = 1",
            [],
        )
        .unwrap();

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("checksum"),
        "tampered migration checksum must be refused, got: {err}"
    );
}

#[test]
fn export_refuses_corrupt_payload_json() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute(
            "UPDATE character_cards SET payload_json = '{not json' WHERE card_id = 'card-1'",
            [],
        )
        .unwrap();

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("corrupt payload_json"),
        "corrupt payload must be refused, got: {err}"
    );
}

#[test]
fn export_refuses_foreign_key_violations() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    // 用独立连接关闭 FK 强制后插入违规行（引用不存在的 campaign/conversation）。
    let raw = rusqlite::Connection::open(&db_path).unwrap();
    raw.execute_batch(
        "PRAGMA foreign_keys = OFF; \
         INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id, \
            base_campaign_revision, status, created_at, updated_at, payload_json) \
         VALUES ('t-fk-bad', 'no-such-campaign', 'no-such-conv', 'i', 0, 'generating', \
            'now', 'now', '{}');",
    )
    .unwrap();
    drop(raw);

    let db = Database::open(&db_path).unwrap();
    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("foreign"),
        "foreign-key violations must be refused, got: {err}"
    );
    assert!(!target.exists());
}

#[test]
fn export_refuses_missing_required_table() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute("DROP TABLE conversations", [])
        .unwrap();

    let target = fresh_target("out");
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("missing") && err.to_string().contains("conversations"),
        "missing required table must be refused, got: {err}"
    );
}

// ─── 二.6 发布失败原子且跨进程安全 ───────────────────────────────────────

fn seed_old_export(target: &Path) {
    fs::create_dir_all(target).unwrap();
    fs::write(target.join("old.txt"), b"old-content").unwrap();
}

/// 扫描**专用工作目录**（非共享 OS temp 根）中的 staging/aside 残留。
fn staging_leftovers(work: &Path) -> Vec<String> {
    fs::read_dir(work)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".staging-") || n.contains(".pre-export-"))
        .collect()
}

#[test]
fn export_fault_after_target_moved_aside_restores_old_target() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    // 专用工作目录：staging/aside 都产生在这里，扫描范围不与共享 OS temp 冲突。
    let work = TempDir::new().unwrap();
    let target = work.path().join("export-target");
    seed_old_export(&target);

    let db = Database::open(&db_path).unwrap();
    let err = export_sqlite_to_json_with_fault(&db, &target, ExportFault::AfterTargetMovedAside)
        .unwrap_err();
    assert!(
        err.to_string().contains("injected fault"),
        "expected injected fault, got: {err}"
    );
    drop(db);

    // 旧目标自动恢复，内容不变。
    assert_eq!(
        fs::read(target.join("old.txt")).unwrap(),
        b"old-content",
        "old export must be restored after failed publish"
    );
    // 无 staging / aside 残留。
    let leftovers = staging_leftovers(work.path());
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

#[test]
fn export_fault_after_stage_verified_leaves_target_untouched() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let work = TempDir::new().unwrap();
    let target = work.path().join("export-target");
    seed_old_export(&target);

    let db = Database::open(&db_path).unwrap();
    let err = export_sqlite_to_json_with_fault(&db, &target, ExportFault::AfterStageVerified)
        .unwrap_err();
    assert!(err.to_string().contains("injected fault"), "got: {err}");
    drop(db);

    assert_eq!(
        fs::read(target.join("old.txt")).unwrap(),
        b"old-content",
        "target must be untouched when fault fires before publish"
    );
    let leftovers = staging_leftovers(work.path());
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

#[test]
fn same_process_export_lock_rejects_concurrent_export() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let target = fresh_target("out");

    let guard = acquire_export_lock(&target).unwrap();
    let db = Database::open(&db_path).unwrap();
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("lock held"),
        "concurrent export must be rejected, got: {err}"
    );
    assert!(!target.exists(), "locked export must not create the target");
    drop(guard);
    // 释放后可导出。
    export_sqlite_to_json(&db, &target).unwrap();
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
fn cross_process_export_lock_blocks_then_releases() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let target = fresh_target("out");

    let exe = std::env::var("CARGO_BIN_EXE_export_hold")
        .expect("CARGO_BIN_EXE_export_hold must point to the compiled helper binary");
    let signals = TempDir::new().unwrap();
    let ready = signals.path().join("ready");
    let release = signals.path().join("release");
    let mut child = std::process::Command::new(&exe)
        .arg(&target)
        .arg(&ready)
        .arg(&release)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn export_hold");
    wait_for_signal(&ready, 60);

    let db = Database::open(&db_path).unwrap();
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("lock held"),
        "export must be blocked while another process holds the lock, got: {err}"
    );
    assert!(!target.exists());

    fs::write(&release, b"go").unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "export_hold must exit cleanly");
    export_sqlite_to_json(&db, &target).unwrap();
}

// ─── 三审7：禁止整个 live data root 内的导出目标 + 删除 .probe rename ────

#[test]
fn export_target_inside_live_data_root_is_rejected() {
    // 三审7：data_dir 内部的任意后代子路径（data_dir/sub/export）必须被拒绝。
    // 旧实现只在扁平 forbidden 列表里枚举已知文件名 + 祖先检查，data_dir 内部
    // 任意新子路径都能绕过——可能把导出写进活动数据根里污染权威树。
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover(&dir);
    let db = Database::open(&db_path).unwrap();

    // data_dir 内部的任意子路径。
    for inside in [
        dir.path().join("subdir").join("export"),
        dir.path().join("nested").join("deep").join("out"),
        dir.path().join("export-directly-named"),
    ] {
        let err = export_sqlite_to_json(&db, &inside).unwrap_err();
        assert!(
            err.to_string().contains("inside the live data root"),
            "target {} must be rejected as inside the live data root, got: {err}",
            inside.display()
        );
        assert!(
            !inside.exists(),
            "no partial target must be created inside data_dir: {}",
            inside.display()
        );
    }
    drop(db);
}

#[test]
fn cutover_publishes_atomically_without_probe_rename() {
    // 三审7：删除 .probe rename 后，cutover 仍原子发布（temp → final 直接 rename）。
    // 回归证明：成功 cutover 后 DB 存在、marker 写入、无 .probe 残留。
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "no-probe".into(),
        allow_json_authoritative_flip: false,
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    assert!(db_path.exists(), "DB must be published");
    assert!(
        dir.path().join("storyforge.backend.json").exists(),
        "marker must be written"
    );
    // 无 .probe 残留（旧实现来回 rename 的中间产物）。
    let probe = db_path.with_extension("probe");
    assert!(!probe.exists(), "no .probe rename leftover must remain");
}

//! Gate 5 反向导出 round-trip：SQLite → JSON → 重新导入后终态与导出前等价。
//!
//! Gate 4 的 reverse_export_gate4 已覆盖 world info / compress jobs / MVU 与
//! pre-accept 阻止；本文件补上 **角色库**（V007 characters）与**全量数据**的
//! round-trip 等价证明，并验证导出-再导入-再导出保持稳定（幂等）。

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, recover_or_verify,
};
use storyforge_infra_sqlite::exporter::export_sqlite_to_json;
use storyforge_infra_sqlite::importer::{ImportStatus, JsonImporter};

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

/// 全量 round-trip fixture（含角色库 / 世界书 / MVU / 四态 compress job）。
fn write_roundtrip_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        &json!([{
            "card": {
                "id": "card-1", "name": "Hero", "source_character_id": "src-1",
                "character_definitions": [
                    {"id": "def-1", "name": "Alice", "role_type": "Protagonist",
                     "persona_prompt": "p", "behavior_rules": "b",
                     "base_backstory": [], "group": null, "variable_schema": []}
                ],
                "campaign_variable_schema": [], "raw_card_json": {},
                "extraction_status": "extracted", "extraction_message": null
            },
            "imported_at": "2026-07-01T00:00:00Z"
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &json!([{
            "id": "camp-1", "card_id": "card-1", "name": "Roundtrip",
            "conversation_id": "conv-1", "revision": 2, "chronicle_revision": 1,
            "lineage_id": "lin-1", "story_clock": "Day 4",
            "variables": [{"key": "story_clock", "value": "Day 4", "turn": 1}],
            "variable_schema": [], "created_at": "2026-07-03T00:00:00Z"
        }]),
    );
    write_json(
        &dir.join("instances.json"),
        &json!([{
            "id": "inst-1", "campaign_id": "camp-1", "definition_id": "def-1",
            "name": "Alice", "is_temporary": false, "variables": []
        }]),
    );
    write_json(
        &dir.join("knowledge.json"),
        &json!([{
            "id": "know-1", "campaign_id": "camp-1", "character_id": "inst-1",
            "content": "知道密道", "source_turn": 1
        }]),
    );
    write_json(
        &dir.join("tasks.json"),
        &json!([{
            "id": "task-1", "campaign_id": "camp-1", "title": "找钥匙",
            "description": "三层", "triggers": [], "status": "active", "created_turn": 0
        }]),
    );
    write_json(
        &dir.join("round_summaries.json"),
        &json!([{
            "id": "sum-a1", "campaign_id": "camp-1", "conversation_id": "conv-1",
            "lineage_id": "lin-1", "level": 0, "turn": 1, "turn_end": 1,
            "code": "A0001", "headline": "h", "content": "c",
            "covered_by": null, "covers": [], "created_at": "2026-07-03T01:00:00Z"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1", "campaign_id": "camp-1", "character_id": "src-1",
            "nodes": [], "archived_upto": 1,
            "created_at": "2026-07-03T00:00:00Z", "updated_at": "2026-07-03T02:00:00Z"
        }),
    );
    write_json(
        &dir.join("turns.json"),
        &json!([{
            "turn_id": "turn-1", "campaign_id": "camp-1", "conversation_id": "conv-1",
            "input_node_id": "node-1", "base_campaign_revision": 0,
            "status": "committed", "accepted_attempt_id": "att-1", "failure_reason": null,
            "attempts": [{
                "attempt_id": "att-1", "variant_id": "var-1", "draft_hash": "h1",
                "status": "committed", "created_at": "2026-07-03T00:10:00Z"
            }],
            "created_at": "2026-07-03T00:00:00Z", "updated_at": "2026-07-03T00:20:00Z"
        }]),
    );
    write_json(
        &dir.join("mvu_translations.json"),
        &json!([{
            "source_character_id": "src-1", "character_name": "Alice",
            "analyzed_at": "2026-07-05T00:00:00Z",
            "translation": {"update_rules": [], "fallback_fragments": [], "ui_bindings": []}
        }]),
    );
    write_json(
        &dir.join("campaign_world_info").join("camp-1.json"),
        &json!({
            "entries": [{
                "keys": ["密道"], "content": "在酒窖", "constant": true, "route": "Both",
                "is_global": false, "depth": 2, "order": 100, "extensions": {}
            }],
            "source": "native", "metadata": {}
        }),
    );
    write_json(
        &dir.join("compress_jobs.json"),
        &json!([{
            "id": "job-1", "campaign_id": "camp-1", "conversation_id": "conv-1",
            "lineage_id": "lin-1", "kind": "auto", "status": "pending",
            "attempts": 0, "max_attempts": 5, "last_error": null,
            "uncovered_a_at_enqueue": 7, "uncovered_b_at_enqueue": 0,
            "created_at": "2026-07-03T04:00:00Z", "updated_at": "2026-07-03T04:00:00Z"
        }]),
    );
    write_json(
        &dir.join("characters.json"),
        &json!([{
            "id": "char-1",
            "info": {
                "source_character_id": "src-1", "name": "Alice",
                "description": "主角", "personality": "勇敢", "scenario": "地下城",
                "first_mes": "你好。", "mes_example": "", "post_history_instructions": "",
                "alternate_greetings": [], "system_prompt": "扮演",
                "tags": [], "creator": "t", "character_version": "1.0",
                "spec_version": "2.0", "extensions": {},
                "embedded_world_info": null, "renderable_assets": null,
                "raw_card_json": {}, "has_world_info": false,
                "has_renderable_assets": false, "world_info_count": 0,
                "world_info_entries": []
            },
            "imported_at": "2026-07-01 10:00:00"
        }]),
    );
}

/// SQLite 侧规范化快照（payload 行 + 投影），与 migration matrix 同口径。
fn sqlite_snapshot(db: &Database) -> Value {
    let mut snap = serde_json::Map::new();
    for (name, table) in [
        ("cards", "character_cards"),
        ("campaigns", "campaigns"),
        ("instances", "character_instances"),
        ("knowledge", "character_knowledge"),
        ("tasks", "story_tasks"),
        ("round_summaries", "round_summaries"),
        ("turns", "turns"),
        ("conversations", "conversations"),
        ("mvu_translations", "mvu_translations"),
    ] {
        let sql = format!("SELECT payload_json FROM {table}");
        let mut stmt = db.connection().prepare(&sql).unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let items: Vec<Value> = rows
            .iter()
            .map(|raw| serde_json::from_str(raw).unwrap())
            .map(|mut v: Value| {
                // 规范化：导出器注入的自包含 id 键（值为列 id，与领域 id 字段
                // 重复）不参与领域快照比较——两端的领域反序列化都忽略它。
                if matches!(name, "cards" | "mvu_translations" | "turns")
                    && let Value::Object(map) = &mut v
                {
                    map.remove("id");
                }
                v
            })
            .collect();
        snap.insert(name.to_string(), sorted(items));
    }
    // 世界书 payload
    let wi: Vec<Value> = {
        let mut stmt = db
            .connection()
            .prepare("SELECT payload_json FROM campaign_world_info")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.iter()
            .map(|raw| serde_json::from_str(raw).unwrap())
            .collect()
    };
    snap.insert("campaign_world_info".into(), sorted(wi));
    // 角色库
    let chars: Vec<Value> = {
        let mut stmt = db
            .connection()
            .prepare(
                "SELECT character_id, info_json, imported_at FROM characters ORDER BY character_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.into_iter()
            .map(|(id, info, imported)| {
                json!({"id": id, "info": serde_json::from_str::<Value>(&info).unwrap(), "imported_at": imported})
            })
            .collect()
    };
    snap.insert("characters".into(), sorted(chars));
    // compress jobs 列投影
    let jobs: Vec<Value> = {
        let mut stmt = db
            .connection()
            .prepare(
                "SELECT job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts, \
                 max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue, \
                 created_at, updated_at FROM chronicle_compress_jobs ORDER BY job_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, String>(12)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.into_iter()
            .map(
                |(
                    id,
                    cid,
                    conv,
                    lin,
                    kind,
                    status,
                    attempts,
                    max_attempts,
                    last_error,
                    ua,
                    ub,
                    created,
                    updated,
                )| {
                    let mut v = json!({
                        "id": id, "campaign_id": cid, "kind": kind, "status": status,
                        "attempts": attempts, "max_attempts": max_attempts,
                        "uncovered_a_at_enqueue": ua, "uncovered_b_at_enqueue": ub,
                        "created_at": created, "updated_at": updated,
                    });
                    if let Some(x) = conv {
                        v["conversation_id"] = json!(x);
                    }
                    if let Some(x) = lin {
                        v["lineage_id"] = json!(x);
                    }
                    if let Some(x) = last_error {
                        v["last_error"] = json!(x);
                    }
                    v
                },
            )
            .collect()
    };
    snap.insert("compress_jobs".into(), sorted(jobs));
    Value::Object(snap)
}

fn sorted(items: Vec<Value>) -> Value {
    let mut encoded: Vec<String> = items
        .iter()
        .map(|v| serde_json::to_string(v).unwrap())
        .collect();
    encoded.sort();
    Value::Array(
        encoded
            .iter()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect(),
    )
}

/// SQLite → JSON → 重新导入：终态与导出前等价（全量数据，含角色库）。
#[test]
fn reverse_export_full_data_roundtrip_restores_equivalent_final_state() {
    let dir = tempfile::tempdir().unwrap();
    write_roundtrip_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "gate5-roundtrip".into(),
        allow_json_authoritative_flip: false,
    };
    match recover_or_verify(&request).unwrap() {
        CutoverOutcome::Completed(report) => {
            assert_eq!(report.characters, 1);
            assert_eq!(report.campaigns, 1);
            assert_eq!(report.turns, 1);
        }
        other => panic!("cutover must complete, got {other:?}"),
    }

    let db = Database::open(&db_path).unwrap();
    let before = sqlite_snapshot(&db);

    // 导出到独立目录（非活动数据目录）。
    let export_dir = tempfile::tempdir().unwrap().path().join("rollback-export");
    let result = export_sqlite_to_json(&db, &export_dir).unwrap();
    assert_eq!(result.report.characters, 1);
    assert_eq!(result.report.campaigns, 1);
    assert_eq!(result.report.turns, 1);
    assert!(export_dir.join("characters.json").exists());
    assert!(export_dir.join("compress_jobs.json").exists());
    assert!(
        export_dir
            .join("campaign_world_info")
            .join("camp-1.json")
            .exists()
    );
    // 导出的 characters.json 是 StoredCharacter 契约形态。
    let exported_chars: Value =
        serde_json::from_str(&fs::read_to_string(export_dir.join("characters.json")).unwrap())
            .unwrap();
    assert_eq!(exported_chars[0]["id"], "char-1");
    assert_eq!(exported_chars[0]["info"]["name"], "Alice");

    // 重新导入到全新 DB → 终态与导出前等价。
    let fresh = tempfile::tempdir().unwrap();
    let mut db2 = Database::open(fresh.path().join("reimport.sqlite3")).unwrap();
    let report = JsonImporter::new(&mut db2)
        .import_data_dir(&export_dir)
        .unwrap();
    assert_eq!(report.status, ImportStatus::Completed);
    assert_eq!(report.characters, 1);
    assert_eq!(report.compress_jobs, 1);
    assert_eq!(report.world_info, 1);
    assert_eq!(report.mvu_translations, 1);

    let after = sqlite_snapshot(&db2);
    assert_eq!(before, after, "SQLite→JSON→SQLite 后领域快照必须等价");

    // 幂等：同一导出目录重复导入 → SkippedDuplicate。
    let report2 = JsonImporter::new(&mut db2)
        .import_data_dir(&export_dir)
        .unwrap();
    assert_eq!(report2.status, ImportStatus::SkippedDuplicate);

    // 导出-再导入-再导出：两次导出 manifest hash 一致（稳定投影）。
    let export2_dir = tempfile::tempdir()
        .unwrap()
        .path()
        .join("rollback-export-2");
    let result2 = export_sqlite_to_json(&db2, &export2_dir).unwrap();
    assert_eq!(
        result.report.export_manifest_hash, result2.report.export_manifest_hash,
        "round-trip 后再次导出的 manifest hash 必须稳定"
    );
}

/// 活跃 pre-accept outbox 无法无损表达时 reverse export 必须明确拒绝。
#[test]
fn reverse_export_refuses_when_pending_preaccept_outbox_exists() {
    let dir = tempfile::tempdir().unwrap();
    write_roundtrip_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "gate5-roundtrip-refuse".into(),
        allow_json_authoritative_flip: false,
    };
    recover_or_verify(&request).unwrap();
    let db = Database::open(&db_path).unwrap();

    // 造一个 pending outbox 行（FK：turn/attempt 已存在）。
    db.connection()
        .execute(
            "INSERT INTO preaccept_outbox \
             (outbox_id, campaign_id, conversation_id, turn_id, attempt_id, kind, draft_hash, \
              payload_hash, status, payload_json, created_at, updated_at) \
             VALUES ('outbox-1', 'camp-1', 'conv-1', 'turn-1', 'att-1', 'draft_ready', 'h', 'p', \
                      'pending', '{}', '2026-07-03T00:00:00Z', '2026-07-03T00:00:00Z')",
            [],
        )
        .unwrap();

    let export_dir = tempfile::tempdir().unwrap().path().join("refused-export");
    let err = export_sqlite_to_json(&db, &export_dir).unwrap_err();
    assert!(
        err.to_string().contains("active pre-accept state"),
        "pending outbox 必须阻止导出而非静默丢弃: {err}"
    );
    assert!(!export_dir.exists(), "导出目录不得残留部分结果");
}

/// 只读台账（mutation_commits / chronicle_publication_jobs）显式分类，
/// 不静默丢弃、不阻塞（空台账时分类仍可见）。
#[test]
fn reverse_export_classifies_readonly_ledgers_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    write_roundtrip_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "gate5-roundtrip-ledger".into(),
        allow_json_authoritative_flip: false,
    };
    recover_or_verify(&request).unwrap();
    let db = Database::open(&db_path).unwrap();
    let export_dir = tempfile::tempdir().unwrap().path().join("ledger-export");
    let result = export_sqlite_to_json(&db, &export_dir).unwrap();
    let classified: Vec<&String> = result
        .report
        .unsupported_fields
        .iter()
        .filter(|f| {
            f.starts_with("mutation_commits") || f.starts_with("chronicle_publication_jobs")
        })
        .collect();
    assert_eq!(
        classified.len(),
        2,
        "两类只读台账必须显式分类: {:?}",
        result.report.unsupported_fields
    );
}

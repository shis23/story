//! Gate 4 reverse-export coverage: story-clock normalization, active
//! pre-accept block, and world-info / compress-jobs / MVU round trips.

use std::fs;

use storyforge_domain::Id;
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::JsonImporter;
use storyforge_infra_sqlite::cutover::{CutoverOutcome, CutoverPlan, CutoverRequest, run_cutover};
use storyforge_infra_sqlite::exporter::export_sqlite_to_json;
use storyforge_infra_sqlite::production::SqliteProductionRepository;
use tempfile::TempDir;

fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn sample_source(dir: &std::path::Path) {
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
    write_json(&dir.join("round_summaries.json"), &serde_json::json!([]));
    write_json(&dir.join("turns.json"), &serde_json::json!([]));
}

fn cutover(dir: &TempDir, label: &str) {
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: label.into(),
        allow_json_authoritative_flip: false,
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
}

/// 顶层 story_clock 字段与 variables 权威不一致时，导出必须产出修复后的
/// 一致载荷（以 variables 为准），不能把双表示残留带进反向导出。
#[test]
fn reverse_export_normalizes_campaign_story_clock() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    cutover(&dir, "clock-test");

    let db_path = dir.path().join("storyforge.sqlite3");
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute(
            r#"
            UPDATE campaigns SET payload_json = ?1 WHERE campaign_id = 'camp-1'
            "#,
            rusqlite::params![
                serde_json::json!({
                    "id": "camp-1",
                    "card_id": "card-1",
                    "name": "Main",
                    "created_at": "2026-07-13T00:00:00Z",
                    "revision": 0,
                    "chronicle_revision": 0,
                    "conversation_id": "conv-1",
                    "lineage_id": "lin-1",
                    "story_clock": "Day 1",
                    "variables": [{
                        "key": "story_clock",
                        "value": "Day 47",
                        "last_updated_turn": 3
                    }]
                })
                .to_string()
            ],
        )
        .unwrap();

    let export_dir = TempDir::new().unwrap();
    export_sqlite_to_json(&db, export_dir.path()).unwrap();

    let exported: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(export_dir.path().join("campaigns.json")).unwrap(),
    )
    .unwrap();
    let campaign = &exported[0];
    assert_eq!(campaign["story_clock"], "Day 47");
    assert_eq!(campaign["variables"][0]["value"], "Day 47");
}

/// 活跃 pre-accept 状态（pending outbox）不能无损表达：导出必须明确失败。
#[test]
fn reverse_export_blocks_on_pending_preaccept_outbox() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    cutover(&dir, "pending-outbox-export");

    let db_path = dir.path().join("storyforge.sqlite3");
    let db = Database::open(&db_path).unwrap();
    // preaccept_outbox 有 FK：先建 turn + attempt，再建 pending outbox。
    db.connection()
        .execute(
            r#"
            INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id,
                base_campaign_revision, status, created_at, updated_at, payload_json)
            VALUES ('turn-x', 'camp-1', 'conv-1', 'input-x', 0, 'draft_ready',
                    'now', 'now', '{}')
            "#,
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            r#"
            INSERT INTO turn_attempts (attempt_id, turn_id, variant_id, draft_hash,
                status, created_at, payload_json)
            VALUES ('attempt-x', 'turn-x', 'variant-x', 'hash', 'draft_ready',
                    'now', '{}')
            "#,
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            r#"
            INSERT INTO preaccept_outbox (
                outbox_id, campaign_id, conversation_id, turn_id, attempt_id, kind,
                draft_hash, payload_hash, status, payload_json, created_at, updated_at
            ) VALUES ('outbox-1', 'camp-1', 'conv-1', 'turn-x', 'attempt-x', 'draft',
                      'hash', 'phash', 'pending', '{}', 'now', 'now')
            "#,
            [],
        )
        .unwrap();

    let export_dir = TempDir::new().unwrap();
    let err = export_sqlite_to_json(&db, export_dir.path()).unwrap_err();
    assert!(
        err.to_string().contains("active pre-accept state"),
        "expected active pre-accept block, got: {err}"
    );
}

/// 世界书 / 压缩任务 / MVU 翻译必须随反向导出落地为 JSON 布局文件，
/// 且可被 importer 原样回读（round trip 不丢字段）。
#[test]
fn reverse_export_covers_world_info_compress_jobs_and_mvu_roundtrip() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    cutover(&dir, "extras-export");

    let db_path = dir.path().join("storyforge.sqlite3");
    let mut db = Database::open(&db_path).unwrap();

    // 世界书
    let book = serde_json::json!({
        "entries": [{
            "keys": ["castle"],
            "content": "A stone keep.",
            "constant": false,
            "route": "Selective"
        }]
    });
    SqliteProductionRepository::save_world_info_payload(&mut db, &Id::from_str("camp-1"), &book)
        .unwrap();

    // 压缩任务（pending open）
    db.connection()
        .execute(
            r#"
            INSERT INTO chronicle_compress_jobs (
                job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts,
                max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue,
                created_at, updated_at
            ) VALUES ('job-1', 'camp-1', 'conv-1', 'lin-1', 'auto', 'pending', 1, 5, NULL,
                      200, 0, '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z')
            "#,
            [],
        )
        .unwrap();

    // MVU 翻译
    let mvu = serde_json::json!({
        "source_character_id": "card-1",
        "character_name": "Hero",
        "analyzed_at": "2026-07-13T00:00:00Z",
        "translation": {
            "routing": "native",
            "variable_schema": [],
            "ui_bindings": [],
            "fallback_fragments": [],
            "update_rules": [],
            "analysis_confidence": 0.9
        }
    });
    SqliteProductionRepository::save_mvu_payload(&mut db, &Id::from_str("card-1"), "Hero", &mvu)
        .unwrap();
    drop(db);

    let export_dir = TempDir::new().unwrap();
    let db = Database::open(&db_path).unwrap();
    let result = export_sqlite_to_json(&db, export_dir.path()).unwrap();
    drop(db);

    assert!(export_dir.path().join("compress_jobs.json").exists());
    assert!(
        export_dir
            .path()
            .join("campaign_world_info")
            .join("camp-1.json")
            .exists()
    );
    assert!(export_dir.path().join("mvu_translations.json").exists());
    assert_eq!(
        result.report.unsupported_fields.len(),
        3,
        "expected only the two recovery ledgers + empty preaccept_outbox classified, got: {:?}",
        result.report.unsupported_fields
    );

    let jobs: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(export_dir.path().join("compress_jobs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(jobs[0]["id"], "job-1");
    assert_eq!(jobs[0]["status"], "pending");
    assert_eq!(jobs[0]["uncovered_a_at_enqueue"], 200);

    // 回读：把导出目录导入全新 DB，世界书/任务/MVU 均须保留。
    let dir2 = TempDir::new().unwrap();
    let mut db2 = Database::open(dir2.path().join("storyforge.sqlite3")).unwrap();
    let mut importer = JsonImporter::new(&mut db2);
    let report = importer.import_data_dir(export_dir.path()).unwrap();
    assert_eq!(report.world_info, 1);
    assert_eq!(report.compress_jobs, 1);
    assert_eq!(report.mvu_translations, 1);
    assert_eq!(report.campaigns, 1);
    assert_eq!(report.cards, 1);

    let book_back: Option<String> = db2
        .connection()
        .query_row(
            "SELECT payload_json FROM campaign_world_info WHERE campaign_id = 'camp-1'",
            [],
            |row| row.get(0),
        )
        .ok();
    assert!(book_back.is_some());
    assert!(book_back.unwrap().contains("stone keep"));

    let job_back: String = db2
        .connection()
        .query_row(
            "SELECT status FROM chronicle_compress_jobs WHERE job_id = 'job-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(job_back, "pending");

    // 幂等：同一导出目录重复导入被去重（manifest hash 稳定含 extras）。
    let mut importer2 = JsonImporter::new(&mut db2);
    let report2 = importer2.import_data_dir(export_dir.path()).unwrap();
    assert!(report2.skipped_as_duplicate);
}

/// SQLite 权威读取必须修复分歧 story_clock（variables 优先）并产生可审核警告。
#[test]
fn sqlite_campaign_load_repairs_diverged_story_clock() {
    let mut db = Database::open_in_memory().unwrap();
    storyforge_infra_sqlite::migrations::migrate(&mut db).unwrap();
    let campaign_id = Id::from_str("clock-camp-1");
    let card_id = Id::from_str("clock-card-1");
    let mut campaign = storyforge_domain::campaign::Campaign::new(card_id.clone(), "Clock");
    campaign.id = campaign_id.clone();
    campaign.set_variable("story_clock", serde_json::json!("Day 47"), 5);
    campaign.story_clock = "Day 1".to_string(); // 制造分歧
    SqliteProductionRepository::save_campaign(&mut db, &campaign).unwrap();

    let loaded = SqliteProductionRepository::get_campaign(&db, &campaign_id)
        .unwrap()
        .expect("campaign");
    assert_eq!(
        loaded.story_clock, "Day 47",
        "load must repair from variables"
    );
    assert_eq!(loaded.current_story_clock(), "Day 47");
    assert!(!loaded.story_clock_diverged());

    let all = SqliteProductionRepository::list_campaigns(&db).unwrap();
    assert_eq!(all[0].story_clock, "Day 47");
}

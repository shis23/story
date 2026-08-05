//! Importer diagnostics for duplicate Attempt ownership and invalid source graphs.

use std::fs;

use serde_json::json;
use storyforge_infra_sqlite::{Database, ImportStatus, JsonImporter, SqliteError};
use tempfile::TempDir;

fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn base_source(dir: &std::path::Path) {
    write_json(
        &dir.join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero" }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &json!([{
            "id": "camp-1",
            "card_id": "card-1",
            "name": "Main",
            "created_at": "t",
            "conversation_id": "conv-1",
            "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1",
            "campaign_id": "camp-1",
            "created_at": "2026-07-13T00:00:00Z",
            "updated_at": "2026-07-13T00:00:00Z",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &json!([]));
    write_json(&dir.join("knowledge.json"), &json!([]));
    write_json(&dir.join("tasks.json"), &json!([]));
    write_json(&dir.join("round_summaries.json"), &json!([]));
    write_json(&dir.join("turns.json"), &json!([]));
}

#[test]
fn importer_rejects_duplicate_attempt_ownership_with_clear_diagnostics() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(
        &dir.path().join("turns.json"),
        &json!([
            {
                "turn_id": "turn-1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "input_node_id": "n1",
                "base_campaign_revision": 0,
                "status": "committed",
                "created_at": "t",
                "updated_at": "t",
                "attempts": [{
                    "attempt_id": "attempt-shared",
                    "variant_id": "v1",
                    "draft_hash": "h1",
                    "status": "committed",
                    "created_at": "t"
                }]
            },
            {
                "turn_id": "turn-2",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "input_node_id": "n2",
                "base_campaign_revision": 0,
                "status": "committed",
                "created_at": "t",
                "updated_at": "t",
                "attempts": [{
                    "attempt_id": "attempt-shared",
                    "variant_id": "v2",
                    "draft_hash": "h2",
                    "status": "committed",
                    "created_at": "t"
                }]
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap_err();
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("attempt-shared")
                    && (msg.contains("duplicate")
                        || msg.contains("owned")
                        || msg.contains("rehang")
                        || msg.contains("turn")),
                "{msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }

    // All-or-nothing: no business rows remain.
    let campaigns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaigns", [], |r| r.get(0))
        .unwrap_or(0);
    let turns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(campaigns, 0);
    assert_eq!(turns, 0);
}

#[test]
fn importer_rejects_partial_summary_graph_without_half_import() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(&dir.path().join("turns.json"), &json!([]));
    write_json(
        &dir.path().join("round_summaries.json"),
        &json!([
            {
                "id": "sum-a1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "content": "leaf",
                "created_at": "t",
                "level": 0,
                "covered_by": "missing-parent"
            },
            {
                "id": "sum-b1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "turn_end": 1,
                "content": "stage",
                "created_at": "t",
                "level": 1,
                "covers": ["sum-a1", "missing-child"]
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap_err();
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("missing") || msg.contains("cover") || msg.contains("graph"),
                "{msg}"
            );
        }
        other => panic!("expected CorruptImportInput diagnostics, got {other}"),
    }

    let summaries: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM round_summaries", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(summaries, 0);
}

#[test]
fn importer_still_accepts_valid_source_all_or_nothing() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(&dir.path().join("turns.json"), &json!([]));
    // Consistent graph: A covered by B.
    write_json(
        &dir.path().join("round_summaries.json"),
        &json!([
            {
                "id": "sum-a1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "content": "leaf",
                "created_at": "t",
                "level": 0,
                "lineage_id": "lin-1",
                "code": "A0001",
                "covered_by": "sum-b1"
            },
            {
                "id": "sum-b1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "turn_end": 1,
                "content": "stage",
                "created_at": "t",
                "level": 1,
                "lineage_id": "lin-1",
                "code": "B0001",
                "covers": ["sum-a1"]
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let report = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(report.status, ImportStatus::Completed);
    assert_eq!(report.summaries, 2);
}

#[test]
fn importer_rejects_covers_covered_by_mismatch_and_scope_drift() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(&dir.path().join("turns.json"), &json!([]));
    write_json(
        &dir.path().join("round_summaries.json"),
        &json!([
            {
                "id": "sum-a1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "content": "leaf",
                "created_at": "t",
                "level": 0,
                "lineage_id": "lin-1",
                "code": "A0001",
                "covered_by": "sum-b1"
            },
            {
                "id": "sum-b1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-other",
                "turn": 1,
                "turn_end": 1,
                "content": "stage",
                "created_at": "t",
                "level": 1,
                "lineage_id": "lin-other",
                "code": "B0001",
                "covers": ["sum-a1", "sum-a2"]
            },
            {
                "id": "sum-a2",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 2,
                "content": "leaf2",
                "created_at": "t",
                "level": 0,
                "lineage_id": "lin-1",
                "code": "A0002"
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap_err();
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("cover")
                    || msg.contains("covered_by")
                    || msg.contains("scope")
                    || msg.contains("conversation")
                    || msg.contains("lineage"),
                "{msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
    let summaries: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM round_summaries", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(summaries, 0);
}
// ─── Gate 5 审查一.6：importer 严格验证 ──────────────────────────────────

/// 完整但为空的合法布局：所有必需布局文件都是有效空数组 + 空 conversations 目录。
fn empty_but_valid_layout(dir: &std::path::Path) {
    for name in [
        "cards.json",
        "campaigns.json",
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
    ] {
        write_json(&dir.join(name), &json!([]));
    }
    std::fs::create_dir_all(dir.join("conversations")).unwrap();
}

#[test]
fn empty_but_valid_layout_imports_cleanly_with_zero_counts() {
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path());

    // 干跑同样接受（合法空目录 ≠ 缺失布局文件）。
    let report = storyforge_infra_sqlite::readiness::validate_source_manifest(dir.path()).unwrap();
    assert_eq!(report.cards, 0);
    assert_eq!(report.campaigns, 0);
    assert!(report.issues.is_empty());

    let mut db = Database::open_in_memory().unwrap();
    let report = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(report.status, ImportStatus::Completed);
    assert_eq!(report.cards, 0);
    assert_eq!(report.campaigns, 0);
    assert_eq!(report.conversations, 0);
    assert_eq!(report.turns, 0);
}

#[test]
fn missing_layout_file_imports_as_empty_like_json_store() {
    // Gate 7 候选周期发现 #1：JSON `CampaignStore` 用 load_or_default（缺失 =
    // 空集合）。默认切换后 readiness/importer 同口径——删掉 cards.json 的
    // legacy 树按空集合导入（其余文件数据保留），不得再 fail-closed 把正常
    // 旧用户挡在门外。**存在但损坏**仍由 CorruptImportInput fail-closed。
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path());
    std::fs::remove_file(dir.path().join("cards.json")).unwrap();

    let report = storyforge_infra_sqlite::readiness::validate_source_manifest(dir.path())
        .expect("missing layout file must dry-run as empty (Gate 7)");
    assert_eq!(report.cards, 0);
    assert_eq!(report.campaigns, 0);
    assert!(report.issues.is_empty());

    let mut db = Database::open_in_memory().unwrap();
    let imported = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect("missing layout file must import as empty (Gate 7)");
    assert_eq!(imported.cards, 0);
    assert_eq!(imported.campaigns, 0);
}

#[test]
fn negative_campaign_revision_is_rejected() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{
            "id": "camp-neg", "card_id": "card-1", "name": "负数局",
            "created_at": "2026-07-13T00:00:00Z",
            "revision": -1, "chronicle_revision": 0
        }]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect_err("negative revision must be rejected, not clamped to 0");
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("revision") || msg.contains("negative"),
                "error must explain the negative revision, got: {msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
}
#[test]
fn unknown_turn_status_is_rejected() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(
        &dir.path().join("turns.json"),
        &json!([{
            "turn_id": "turn-x", "campaign_id": "camp-1", "conversation_id": "conv-1",
            "input_node_id": "n1", "base_campaign_revision": 0,
            "status": "flying_pig", "created_at": "2026-07-13T00:00:00Z",
            "updated_at": "2026-07-13T00:00:00Z", "attempts": []
        }]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect_err("unknown enum value must be rejected");
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("status") || msg.contains("flying_pig"),
                "error must mention the invalid status, got: {msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
}

#[test]
fn wrong_bool_type_is_rejected() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(
        &dir.path().join("instances.json"),
        &json!([{
            "id": "inst-x", "campaign_id": "camp-1", "name": "X",
            "is_temporary": "yes"
        }]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect_err("wrong bool type must be rejected, not coerced");
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("is_temporary") || msg.contains("bool"),
                "error must mention the invalid bool, got: {msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
}

#[test]
fn malformed_character_info_is_rejected() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    // info 缺 name（CharacterInfo 必需字段）且类型错误。
    write_json(
        &dir.path().join("characters.json"),
        &json!([{
            "id": "char-bad",
            "info": { "name": 42, "tags": "not-an-array" },
            "imported_at": "2026-07-01T00:00:00Z"
        }]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect_err("malformed CharacterInfo must be rejected");
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("name") || msg.contains("tags") || msg.contains("info"),
                "error must explain the malformed CharacterInfo, got: {msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
}

#[test]
fn conversations_dir_replaced_by_file_is_rejected() {
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path());
    // conversations 必须是目录：替换成文件 → read_dir 错误必须传播。
    std::fs::remove_dir_all(dir.path().join("conversations")).unwrap();
    write_json(&dir.path().join("conversations"), &json!({}));

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect_err("conversations-as-file must fail, not silently yield zero conversations");
    assert!(
        matches!(err, SqliteError::Io(_))
            || err.to_string().to_lowercase().contains("not a directory")
            || err.to_string().contains("conversations"),
        "read_dir error must propagate, got: {err}"
    );
}
#[test]
fn world_info_identical_payloads_distinct_campaigns_hash_differently() {
    // 同一份 book 载荷挂在两个 campaign 下：hash 必须绑定 campaign_id。
    let book = json!({
        "entries": [{
            "keys": ["密道"], "content": "密道在酒窖",
            "constant": true, "route": "Both",
            "is_global": false, "depth": 2, "order": 100,
            "extensions": {}
        }],
        "source": "native",
        "metadata": {"seeded_from": "card"}
    });

    let dir_one = TempDir::new().unwrap();
    empty_but_valid_layout(dir_one.path());
    write_json(
        &dir_one.path().join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero", "source_character_id": "char-1",
                  "character_definitions": [] }]),
    );
    write_json(
        &dir_one.path().join("campaigns.json"),
        &json!([{ "id": "camp-a", "card_id": "card-1", "name": "A",
                  "created_at": "2026-07-13T00:00:00Z" }]),
    );
    write_json(
        &dir_one
            .path()
            .join("campaign_world_info")
            .join("camp-a.json"),
        &book,
    );

    let dir_two = TempDir::new().unwrap();
    empty_but_valid_layout(dir_two.path());
    write_json(
        &dir_two.path().join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero", "source_character_id": "char-1",
                  "character_definitions": [] }]),
    );
    write_json(
        &dir_two.path().join("campaigns.json"),
        &json!([
            { "id": "camp-a", "card_id": "card-1", "name": "A",
              "created_at": "2026-07-13T00:00:00Z" },
            { "id": "camp-b", "card_id": "card-1", "name": "B",
              "created_at": "2026-07-13T00:00:01Z" }
        ]),
    );
    // 两个 campaign 挂同一份载荷（文件内容逐字节相同）。
    write_json(
        &dir_two
            .path()
            .join("campaign_world_info")
            .join("camp-a.json"),
        &book,
    );
    write_json(
        &dir_two
            .path()
            .join("campaign_world_info")
            .join("camp-b.json"),
        &book,
    );

    let one = storyforge_infra_sqlite::readiness::validate_source_manifest(dir_one.path()).unwrap();
    let two = storyforge_infra_sqlite::readiness::validate_source_manifest(dir_two.path()).unwrap();
    assert_ne!(
        one.manifest_hash, two.manifest_hash,
        "world-info hash must bind campaign_id: identical payloads under different \
         campaign ids must hash differently"
    );

    // 导入两侧后：DB 内容 hash 与源 manifest 一致（recompute 同口径绑定）。
    for (dir, expected) in [
        (dir_one.path(), one.manifest_hash.clone()),
        (dir_two.path(), two.manifest_hash.clone()),
    ] {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("db.sqlite3");
        let mut db = Database::open(&db_path).unwrap();
        let report = JsonImporter::new(&mut db).import_data_dir(dir).unwrap();
        assert_eq!(
            report.world_info,
            if expected == one.manifest_hash { 1 } else { 2 }
        );
        assert_eq!(report.source_manifest_hash, expected);
        let recomputed =
            storyforge_infra_sqlite::readiness::recompute_db_content_hash_for_test(&db).unwrap();
        assert_eq!(recomputed, expected, "recompute must bind campaign_id too");
    }
}

#[test]
fn world_info_for_unknown_campaign_is_rejected() {
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path());
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{ "id": "camp-a", "card_id": "card-1", "name": "A",
                  "created_at": "2026-07-13T00:00:00Z" }]),
    );
    // 幽灵 campaign 的世界书文件：campaign_id 无对应 Campaign → 拒绝。
    write_json(
        &dir.path().join("campaign_world_info").join("ghost.json"),
        &json!({ "entries": [], "source": "native", "metadata": {} }),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect_err("world-info for an unknown campaign must be rejected");
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("ghost") || msg.contains("campaign"),
                "error must mention the unknown campaign, got: {msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
}

// ─── 三审6：Campaign 引用不存在的会话文件必须拒绝（区分空白新用户与部分丢失）───

#[test]
fn campaign_referencing_missing_conversation_is_rejected() {
    // 三审6：campaigns.json 引用了 conv-missing，但 conversations/ 里没有该文件
    // （部分文件丢失）→ importer 必须拒绝，绝不静默当作无会话导入。
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path());
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{
            "id": "camp-1", "card_id": "card-1", "name": "Main",
            "created_at": "2026-07-13T00:00:00Z",
            "conversation_id": "conv-missing"
        }]),
    );
    // conversations/ 目录存在但为空（conv-missing.json 缺失）。
    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect_err("campaign referencing a missing conversation must be rejected");
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("conv-missing"),
                "error must mention the missing conversation id, got: {msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
}

#[test]
fn blank_new_user_without_core_files_is_allowed() {
    // 三审6：真正的空白新用户——核心 JSON 文件存在但为空（无 campaigns 引用）→ 允许
    // （不被误判为部分丢失）。这是新安装的初始状态。
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path()); // 空 cards/campaigns/...（无任何引用）
    storyforge_infra_sqlite::readiness::validate_source_manifest(dir.path())
        .expect("blank new user (empty core files, no references) must pass manifest validation");
    let mut db = Database::open_in_memory().unwrap();
    JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect("blank new user must import cleanly (empty)");
}

#[test]
fn campaign_without_conversation_reference_imports_cleanly() {
    // 正向：campaign 存在但不引用任何 conversation（conversation_id 为 null/空）→ 允许。
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path());
    write_json(
        &dir.path().join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero" }]),
    );
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{
            "id": "camp-1", "card_id": "card-1", "name": "Main",
            "created_at": "2026-07-13T00:00:00Z"
            // 无 conversation_id 字段
        }]),
    );
    let mut db = Database::open_in_memory().unwrap();
    JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect("campaign without conversation reference must import cleanly");
}

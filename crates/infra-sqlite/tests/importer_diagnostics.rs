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
fn empty_character_info_strings_import_like_json_app() {
    // Gate 7 候选周期发现 #2：JSON 应用的 CharacterInfo 是普通 String 字段
    // （允许空串，如未填写的 description/system_prompt）。真实 legacy 数据
    // 可能带空 description——必须正常导入（数据不丢、用户不被挡在门外）；
    // 「存在但类型错误/缺字段」仍由 malformed_character_info_is_rejected 拒绝。
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(
        &dir.path().join("characters.json"),
        &json!([{
            "id": "char-empty-desc",
            "info": {
                "id": "char-empty-desc",
                "name": "命定之诗",
                "description": "",
                "personality": "",
                "scenario": "",
                "first_mes": "",
                "system_prompt": "",
                "creator": "",
                "spec_version": "",
                "tags": [],
                "world_info_entries": [],
                "created_at": "2026-07-01T00:00:00Z",
                "updated_at": "2026-07-01T00:00:00Z"
            },
            "imported_at": "2026-07-01T00:00:00Z"
        }]),
    );

    let report = storyforge_infra_sqlite::readiness::validate_source_manifest(dir.path())
        .expect("empty CharacterInfo strings must dry-run cleanly (Gate 7 finding #2)");
    assert_eq!(report.characters, 1);

    let mut db = Database::open_in_memory().unwrap();
    let imported = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect("empty CharacterInfo strings must import (Gate 7 finding #2)");
    assert_eq!(imported.characters, 1);
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
fn world_info_for_unknown_campaign_is_skipped_and_counted() {
    // Gate 8 复评：world_info 孤儿（campaign 不存在于源数据）与孤儿行同口径
    // 跳过+计数——不得阻断默认迁移（原语义整体拒绝，与 P2-A3「被丢对象不
    // 阻塞」冲突）。合法 campaign 的世界书仍正常落库。
    let dir = TempDir::new().unwrap();
    empty_but_valid_layout(dir.path());
    write_json(
        &dir.path().join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero" }]),
    );
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{ "id": "camp-a", "card_id": "card-1", "name": "A",
                  "created_at": "2026-07-13T00:00:00Z" }]),
    );
    // 幽灵 campaign 的世界书文件：campaign_id 无对应 Campaign → 跳过并计数。
    write_json(
        &dir.path().join("campaign_world_info").join("ghost.json"),
        &json!({ "entries": [], "source": "native", "metadata": {} }),
    );
    // 合法 campaign 的世界书保留。
    write_json(
        &dir.path().join("campaign_world_info").join("camp-a.json"),
        &json!({ "entries": [], "source": "native", "metadata": {} }),
    );

    let mut db = Database::open_in_memory().unwrap();
    let imported = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect("orphan world-info must be skipped, not block the import");
    assert_eq!(imported.skipped_orphan_rows, 1, "1 个孤儿 world_info");
    let world_info: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaign_world_info", [], |r| r.get(0))
        .unwrap();
    assert_eq!(world_info, 1, "合法 world_info 必须落库");
}

// ─── 三审6：Campaign 引用不存在的会话文件必须拒绝（区分空白新用户与部分丢失）───

#[test]
fn campaign_referencing_missing_conversation_is_rejected() {
    // 三审6：campaigns.json 引用了 conv-missing，但 conversations/ 里没有该文件
    // （部分文件丢失）→ importer 必须拒绝，绝不静默当作无会话导入。
    // Gate 8 复评：必须是「有效 Campaign」（card 存在）才触发该拒绝——悬空卡
    // Campaign 本身会被丢弃，其缺失会话不再阻断（见
    // dangling_card_campaign_with_missing_conversation_is_skipped_not_blocking）。
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

#[test]
fn orphan_rows_are_skipped_and_counted_not_blocking() {
    // Gate 7 候选周期发现 #3：父对象（campaign/conversation）已删除的残留行
    // 在 JSON 应用里按 campaign 列出时不可达（等价于不存在），SQLite FK 拒绝
    // 插入。导入必须跳过并计数——不静默丢可达数据，也不因死行把用户挡在门外。
    let dir = TempDir::new().unwrap();
    write_json(
        &dir.path().join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero" }]),
    );
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{
            "id": "camp-1", "card_id": "card-1", "name": "Main",
            "created_at": "2026-07-01T00:00:00Z", "conversation_id": "conv-1", "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.path().join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1", "campaign_id": "camp-1", "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z", "nodes": []
        }),
    );
    // 1 个有效实例 + 1 个孤儿实例（campaign 不存在）。
    write_json(
        &dir.path().join("instances.json"),
        &json!([
            { "id": "inst-ok", "campaign_id": "camp-1", "name": "主角" },
            { "id": "inst-orphan", "campaign_id": "gone-camp", "name": "残留" }
        ]),
    );
    // 2 个有效 turn + 1 个孤儿 turn（campaign 不存在）+ 1 个孤儿 turn（会话不存在）。
    write_json(
        &dir.path().join("turns.json"),
        &json!([
            {
                "turn_id": "turn-ok-1", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "input_node_id": "n1", "status": "committed", "attempts": []
            },
            {
                "turn_id": "turn-ok-2", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "input_node_id": "n2", "status": "committed", "attempts": []
            },
            {
                "turn_id": "turn-orphan-camp", "campaign_id": "gone-camp", "conversation_id": "conv-1",
                "input_node_id": "n3", "status": "committed", "attempts": []
            },
            {
                "turn_id": "turn-orphan-conv", "campaign_id": "camp-1", "conversation_id": "gone-conv",
                "input_node_id": "n4", "status": "committed", "attempts": []
            }
        ]),
    );

    // readiness：计数基于过滤后数组（与落库一致）。
    let report = storyforge_infra_sqlite::readiness::validate_source_manifest(dir.path()).unwrap();
    assert_eq!(report.instances, 1, "孤儿实例不计入");
    assert_eq!(report.turns, 2, "孤儿 turn 不计入");
    assert!(report.issues.is_empty());

    let mut db = Database::open_in_memory().unwrap();
    let imported = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect("孤儿行必须被跳过而不是阻断导入（Gate 7 发现 #3）");
    assert_eq!(imported.instances, 1);
    assert_eq!(imported.turns, 2);
    assert_eq!(imported.skipped_orphan_rows, 3, "1 孤儿实例 + 2 孤儿 turn");

    // 落库只有非孤儿行（FK 完整性成立）。
    let instances: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM character_instances", [], |r| r.get(0))
        .unwrap();
    let turns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(instances, 1);
    assert_eq!(turns, 2);

    // 重跑幂等：同一 hash（去重命中），skipped 统计一致。
    let second = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(second.status, ImportStatus::SkippedDuplicate);
    assert_eq!(second.skipped_orphan_rows, 3);
}

#[test]
fn campaign_with_dangling_card_id_is_skipped_and_counted_not_blocking() {
    // Gate 8 审查 P2-A3：JSON `save_card` 按 source_character_id 覆盖去重会
    // 留下被 Campaign 引用的旧卡 id（悬空引用）。SQLite `campaigns.card_id`
    // FK 会硬拒绝插入、静默卡死默认迁移——必须与孤儿行同口径跳过并计数。
    let dir = TempDir::new().unwrap();
    write_json(
        &dir.path().join("cards.json"),
        &json!([{ "id": "card-live", "name": "Hero" }]),
    );
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([
            {
                "id": "camp-ok", "card_id": "card-live", "name": "Main",
                "created_at": "2026-07-01T00:00:00Z", "conversation_id": "conv-1", "lineage_id": "lin-1"
            },
            {
                "id": "camp-dangling", "card_id": "card-removed", "name": "Dangling",
                "created_at": "2026-07-01T00:00:00Z", "conversation_id": "conv-2", "lineage_id": "lin-2"
            }
        ]),
    );
    write_json(
        &dir.path().join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1", "campaign_id": "camp-ok", "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z", "nodes": []
        }),
    );
    write_json(
        &dir.path().join("conversations").join("conv-2.json"),
        &json!({
            "id": "conv-2", "campaign_id": "camp-dangling", "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z", "nodes": []
        }),
    );
    write_json(&dir.path().join("instances.json"), &json!([]));
    write_json(&dir.path().join("knowledge.json"), &json!([]));
    write_json(&dir.path().join("tasks.json"), &json!([]));
    write_json(&dir.path().join("round_summaries.json"), &json!([]));
    write_json(&dir.path().join("turns.json"), &json!([]));
    write_json(&dir.path().join("mvu_translations.json"), &json!([]));
    write_json(&dir.path().join("compress_jobs.json"), &json!([]));
    write_json(&dir.path().join("characters.json"), &json!([]));
    write_json(&dir.path().join("campaign_world_info.json"), &json!([]));

    // readiness：campaign 计数基于过滤后数组。
    let report = storyforge_infra_sqlite::readiness::validate_source_manifest(dir.path()).unwrap();
    assert_eq!(report.campaigns, 1, "悬空卡 Campaign 不计入");

    let mut db = Database::open_in_memory().unwrap();
    let imported = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .expect("悬空卡 Campaign 必须被跳过而不是阻断导入（Gate 8 审查 P2-A3）");
    assert_eq!(imported.campaigns, 1);
    assert_eq!(imported.skipped_orphan_rows, 1, "1 个悬空卡 Campaign");

    // 落库只有非孤儿 Campaign（FK 完整性成立）。
    let campaigns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaigns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(campaigns, 1);
    let dangling: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM campaigns WHERE card_id = 'card-removed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dangling, 0);

    // 重跑幂等：同一 hash（去重命中），skipped 统计一致。
    let second = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(second.status, ImportStatus::SkippedDuplicate);
    assert_eq!(second.skipped_orphan_rows, 1);
}

#[test]
fn dangling_card_campaign_with_world_info_is_skipped_not_blocking() {
    // Gate 8 复评：被 P2-A3 丢弃的悬空卡 Campaign 若仍带 campaign_world_info
    // 文件，其 world_info 孤儿必须跳过+计数而不是 hard-fail（原顺序先
    // strict_validate_world_info 会因孤儿 world_info 整体拒绝默认迁移）。
    let dir = TempDir::new().unwrap();
    write_json(
        &dir.path().join("cards.json"),
        &json!([{ "id": "card-live", "name": "Hero" }]),
    );
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{
            "id": "camp-dangling", "card_id": "card-removed", "name": "Dangling",
            "created_at": "2026-07-01T00:00:00Z", "conversation_id": "conv-2", "lineage_id": "lin-2"
        }]),
    );
    write_json(
        &dir.path()
            .join("campaign_world_info")
            .join("camp-dangling.json"),
        &json!({ "entries": [] }),
    );
    write_json(
        &dir.path().join("conversations").join("conv-2.json"),
        &json!({
            "id": "conv-2", "campaign_id": "camp-dangling", "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z", "nodes": []
        }),
    );
    for name in [
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
        "mvu_translations.json",
        "compress_jobs.json",
        "characters.json",
        "campaign_world_info.json",
    ] {
        write_json(&dir.path().join(name), &json!([]));
    }

    let imported = JsonImporter::new(&mut Database::open_in_memory().unwrap())
        .import_data_dir(dir.path())
        .expect("悬空卡 Campaign 的 world_info 孤儿必须跳过而不是阻断导入");
    assert_eq!(imported.campaigns, 0);
    assert_eq!(
        imported.skipped_orphan_rows, 2,
        "1 悬空卡 Campaign + 1 孤儿 world_info"
    );
}

#[test]
fn dangling_card_campaign_with_missing_conversation_is_skipped_not_blocking() {
    // Gate 8 复评：verify_campaign_conversation_references 移到悬空卡过滤之后
    // ——被丢弃对象的缺失会话引用不得阻断默认迁移（原顺序对原始 campaigns
    // 校验，悬空卡 Campaign 的缺失 conversation 文件会整体拒绝导入）。
    let dir = TempDir::new().unwrap();
    write_json(
        &dir.path().join("cards.json"),
        &json!([{ "id": "card-live", "name": "Hero" }]),
    );
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([
            {
                "id": "camp-ok", "card_id": "card-live", "name": "Main",
                "created_at": "2026-07-01T00:00:00Z", "conversation_id": "conv-1", "lineage_id": "lin-1"
            },
            {
                "id": "camp-dangling", "card_id": "card-removed", "name": "Dangling",
                "created_at": "2026-07-01T00:00:00Z", "conversation_id": "conv-missing", "lineage_id": "lin-2"
            }
        ]),
    );
    write_json(
        &dir.path().join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1", "campaign_id": "camp-ok", "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z", "nodes": []
        }),
    );
    for name in [
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
        "mvu_translations.json",
        "compress_jobs.json",
        "characters.json",
        "campaign_world_info.json",
    ] {
        write_json(&dir.path().join(name), &json!([]));
    }

    let imported = JsonImporter::new(&mut Database::open_in_memory().unwrap())
        .import_data_dir(dir.path())
        .expect("悬空卡 Campaign 的缺失会话引用必须跳过而不是阻断导入");
    assert_eq!(imported.campaigns, 1);
    assert_eq!(imported.skipped_orphan_rows, 1, "1 个悬空卡 Campaign");
}

#[test]
fn campaign_with_missing_card_id_fails_closed() {
    // Gate 8 复评：领域模型 Campaign.card_id 必填；缺失/空是畸形数据，与
    // 「悬空引用→孤儿跳过」严格区分，必须 fail-closed（旧实现把缺失 card_id
    // 当空串静默并入孤儿丢弃）。
    let dir = TempDir::new().unwrap();
    write_json(
        &dir.path().join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero" }]),
    );
    write_json(
        &dir.path().join("campaigns.json"),
        &json!([{
            "id": "camp-malformed", "name": "NoCard",
            "created_at": "2026-07-01T00:00:00Z", "conversation_id": "conv-1", "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.path().join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1", "campaign_id": "camp-malformed", "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z", "nodes": []
        }),
    );
    for name in [
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
        "mvu_translations.json",
        "compress_jobs.json",
        "characters.json",
        "campaign_world_info.json",
    ] {
        write_json(&dir.path().join(name), &json!([]));
    }

    let err = JsonImporter::new(&mut Database::open_in_memory().unwrap())
        .import_data_dir(dir.path())
        .unwrap_err();
    assert!(
        err.to_string().contains("card_id"),
        "缺失 card_id 必须 fail-closed，got: {err}"
    );
}

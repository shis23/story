//! Gate 5 Phase 6：恢复与故障矩阵（infra-sqlite 级）。
//!
//! 补齐 PLAN §10.1/§10.2 中此前无测试覆盖的场景：
//! - 已发布 DB 文件损坏 → marker Stale → startup fail closed（拒绝自愈改写）；
//! - cutover 目标路径存在**无关**数据库（无 marker）→ 拒绝覆盖，绝不静默让位；
//! - 目标路径存在空文件 → 拒绝覆盖；清理后可正常完成；
//! - pre-accept outbox 日志跨 reopen（重启）存活，恢复再次 fail 未完成 Turn。
//!
//! 全部使用独立 temp dir；不依赖测试顺序或全局状态。

use std::fs;
use std::path::Path;

use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::conversation::{Conversation, Role};
use storyforge_domain::turn::{
    AttemptStatus, DerivationComponents, DerivationStatus, Mutation, MutationBatch, TurnRecord,
    TurnStatus,
};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker, recover_or_verify,
    run_cutover,
};
use storyforge_infra_sqlite::preaccept::{
    DraftAttemptRequest, PostprocessApplyOutcome, PostprocessApplyRequest, PreacceptOutboxKind,
    PreacceptOutboxStatus, SqlitePreacceptRepository,
};
use storyforge_infra_sqlite::production::SqliteProductionRepository;
use tempfile::TempDir;

fn write_json(path: &Path, value: serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

/// 最小可迁移源：1 卡 + 1 局 + 1 会话。
fn write_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        serde_json::json!([{
            "card": {
                "id": "card-1", "name": "故障矩阵卡",
                "source_character_id": "char-src-1",
                "character_definitions": [{
                    "id": "def-1", "card_id": "card-1", "name": "主角",
                    "role_type": "protagonist", "persona_prompt": "坚毅",
                    "behavior_rules": "", "base_backstory": [], "group": null,
                    "variable_schema": []
                }],
                "campaign_variable_schema": [],
                "raw_card_json": {},
                "extraction_status": "extracted",
                "extraction_message": null
            },
            "imported_at": "2026-07-01T00:00:00Z"
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        serde_json::json!([{
            "id": "camp-1", "card_id": "card-1", "name": "故障局",
            "conversation_id": "conv-1", "revision": 0,
            "chronicle_revision": 0, "lineage_id": "lin-1",
            "story_clock": "Day 1",
            "variables": [], "variable_schema": [],
            "created_at": "2026-07-02T00:00:00Z"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        serde_json::json!({
            "id": "conv-1", "campaign_id": "camp-1", "character_id": null,
            "created_at": "2026-07-02T00:00:00Z", "updated_at": "2026-07-02T00:00:00Z",
            "nodes": [], "archived_upto": 0
        }),
    );
    for name in [
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
    ] {
        write_json(&dir.join(name), serde_json::json!([]));
    }
}

fn request(dir: &Path) -> CutoverRequest {
    CutoverRequest {
        plan: CutoverPlan::new(dir, dir.join("storyforge.sqlite3")),
        label: "gate5-fault-matrix".into(),
    }
}

/// 破坏 DB 文件：覆盖头部字节并截断一半，模拟磁盘位翻转/部分写入。
fn corrupt_db_file(path: &Path) {
    let bytes = fs::read(path).unwrap();
    let mut bytes = bytes;
    let cut = (bytes.len() / 2).max(64);
    bytes.truncate(cut);
    for b in bytes.iter_mut().take(64) {
        *b ^= 0xA5;
    }
    fs::write(path, &bytes).unwrap();
}

// ─── 1. 已发布 DB 损坏 → fail closed ─────────────────────────────────────

#[test]
fn corrupt_published_db_fails_closed_and_requires_manual_resolution() {
    let dir = TempDir::new().unwrap();
    write_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");

    // 完整 cutover → marker 声称 SQLite authoritative。
    match recover_or_verify(&request(dir.path())).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("expected Completed, got {other:?}"),
    }
    assert!(matches!(
        inspect_marker(&request(dir.path()).plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));

    // 破坏 DB 文件本体。
    corrupt_db_file(&db_path);

    // inspect_marker 必须识别为 Stale（DB 损坏），而不是继续当权威用。
    let status = inspect_marker(&request(dir.path()).plan);
    assert!(
        matches!(status, MarkerStatus::Stale { .. }),
        "corrupt DB must be Stale, got {status:?}"
    );

    // 启动路径（recover_or_verify）必须 fail closed，绝不静默重跑/自愈改写。
    let err = recover_or_verify(&request(dir.path()))
        .expect_err("recover_or_verify must fail closed on corrupt DB");
    assert!(
        err.to_string().contains("stale"),
        "error must mention stale marker, got: {err}"
    );

    // marker 保持原样（未被恢复流程重写）；JSON 源未被触碰。
    assert!(
        dir.path().join("storyforge.backend.json").exists(),
        "marker must be preserved, not rewritten"
    );
    assert!(dir.path().join("campaigns.json").exists());

    // 用户清理损坏 DB + marker 后，正常 cutover 可完成（恢复路径存在）。
    fs::remove_file(&db_path).unwrap();
    fs::remove_file(dir.path().join("storyforge.backend.json")).unwrap();
    match recover_or_verify(&request(dir.path())).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("expected Completed after cleanup, got {other:?}"),
    }
    assert!(matches!(
        inspect_marker(&request(dir.path()).plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

// ─── 2. 目标路径存在无关数据库 → 拒绝覆盖 ────────────────────────────────

#[test]
fn foreign_database_at_cutover_target_is_never_silently_overwritten() {
    let dir = TempDir::new().unwrap();
    write_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");

    // 用户/其他程序在目标路径放了无关 SQLite 数据库（含数据，无 marker）。
    {
        let mut foreign = Database::open(&db_path).unwrap();
        foreign
            .connection_mut()
            .execute_batch(
                "CREATE TABLE user_data (id INTEGER PRIMARY KEY, note TEXT);
                 INSERT INTO user_data (note) VALUES ('do not touch me');",
            )
            .unwrap();
    }

    // 无 marker → inspect 认为 Absent，但 run_cutover 必须拒绝覆盖无关 DB。
    assert!(matches!(
        inspect_marker(&request(dir.path()).plan),
        MarkerStatus::Absent
    ));
    let err = run_cutover(&request(dir.path()))
        .expect_err("cutover must refuse to overwrite a foreign database");
    assert!(
        err.to_string().contains("non-StoryForge"),
        "error must explain the refusal, got: {err}"
    );

    // 用户数据原样保留；marker 未写；JSON 仍权威。
    let reopened = Database::open(&db_path).unwrap();
    let note: String = reopened
        .connection()
        .query_row("SELECT note FROM user_data WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(note, "do not touch me");
    drop(reopened);
    assert!(!dir.path().join("storyforge.backend.json").exists());
    assert!(dir.path().join("campaigns.json").exists());

    // 用户移走无关 DB 后，cutover 可正常完成。
    fs::remove_file(&db_path).unwrap();
    match run_cutover(&request(dir.path())).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("expected Completed after foreign DB removed, got {other:?}"),
    }
    assert!(db_path.exists());
}

// ─── 3. 目标路径存在空文件 → 拒绝覆盖 ────────────────────────────────────

#[test]
fn empty_file_at_cutover_target_is_refused_not_silently_replaced() {
    let dir = TempDir::new().unwrap();
    write_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");
    fs::write(&db_path, b"").unwrap();

    let err = run_cutover(&request(dir.path()))
        .expect_err("cutover must refuse to replace an unknown file at the target");
    assert!(
        err.to_string().contains("non-StoryForge"),
        "error must explain the refusal, got: {err}"
    );
    // 空文件未被替换/删除；无 marker。
    assert!(db_path.exists());
    assert!(!dir.path().join("storyforge.backend.json").exists());

    // 清理后成功。
    fs::remove_file(&db_path).unwrap();
    match run_cutover(&request(dir.path())).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("expected Completed after cleanup, got {other:?}"),
    }
}

// ─── 4. pre-accept outbox 日志跨重启存活 + 恢复再次 fail 未完成 Turn ─────

#[test]
fn preaccept_outbox_journal_survives_reopen_and_recovery_refails_turn() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("recover.sqlite3");
    let mut db = Database::open(&db_path).unwrap();

    let campaign_id = Id::from_str("camp-recover");
    let conversation_id = Id::from_str("conv-recover");
    let mut campaign = Campaign::new(Id::from_str("card-recover"), "恢复局");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    let mut conversation = Conversation::new(None, Some(campaign_id.clone()));
    conversation.id = conversation_id.clone();
    let input_node_id = conversation.append_message(Role::User, "开场".into());
    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();

    let mut turn = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        input_node_id.clone(),
        0,
    );
    turn.turn_id = Id::from_str("turn-recover");
    turn.status = TurnStatus::Generating;
    SqliteProductionRepository::save_turn(&mut db, &turn).unwrap();
    let turn_id = turn.turn_id.clone();

    // draft → postprocess（AwaitingAcceptance），outbox 写入 Applied 行。
    let attempt_id = Id::from_str("att-recover");
    let draft = SqlitePreacceptRepository::create_draft_attempt(
        &mut db,
        DraftAttemptRequest {
            campaign_id: &campaign_id,
            conversation_id: &conversation_id,
            turn_id: &turn_id,
            attempt_id: &attempt_id,
            draft_text: "草稿",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    let mut batch = MutationBatch::new(Id::from_str("commit-recover"), 0);
    batch.mutations = vec![Mutation::SetVariable {
        instance_id: None,
        key: "story_clock".into(),
        value: serde_json::json!("Day 2"),
        turn: 1,
    }];
    let outcome = SqlitePreacceptRepository::apply_postprocess(
        &mut db,
        PostprocessApplyRequest {
            campaign_id: &campaign_id,
            conversation_id: &conversation_id,
            turn_id: &turn_id,
            attempt_id: &attempt_id,
            batch: Some(batch),
            derivation: DerivationComponents {
                summary_derivation: DerivationStatus::Disabled,
                state_derivation: DerivationStatus::Succeeded,
            },
        },
    )
    .unwrap();
    assert!(matches!(outcome, PostprocessApplyOutcome::Applied));

    // 手工插入一条 Pending outbox（模拟进程崩溃时仍在途的 op）。
    {
        let conn = db.connection_mut();
        conn.execute(
            "INSERT INTO preaccept_outbox \
             (outbox_id, campaign_id, conversation_id, turn_id, attempt_id, kind, draft_hash, \
              payload_hash, status, payload_json, created_at, updated_at) \
             VALUES ('outbox-pending', 'camp-recover', 'conv-recover', 'turn-recover', \
                     'att-recover', 'postprocess_apply', 'h', 'h', 'pending', '{}', \
                     '2026-07-03T00:00:00Z', '2026-07-03T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    // ── 模拟重启：drop 连接后重新打开（独立 authority）──────────────
    drop(db);
    let mut db = Database::open(&db_path).unwrap();

    // outbox 日志跨重启存活：Applied（已落）+ Pending（崩溃在途）都在。
    let outbox = SqlitePreacceptRepository::list_outbox_for_turn(&db, &turn_id).unwrap();
    assert!(outbox.iter().any(|r| {
        r.kind == PreacceptOutboxKind::DraftReady && r.status == PreacceptOutboxStatus::Applied
    }));
    assert!(outbox.iter().any(|r| {
        r.kind == PreacceptOutboxKind::PostprocessApply
            && r.status == PreacceptOutboxStatus::Applied
    }));
    assert!(outbox.iter().any(|r| {
        r.kind == PreacceptOutboxKind::PostprocessApply
            && r.status == PreacceptOutboxStatus::Pending
    }));

    // 重启恢复：未完成 Turn（AwaitingAcceptance）必须被 fail。
    let failed = SqlitePreacceptRepository::fail_incomplete_preaccept(&mut db).unwrap();
    assert!(failed >= 1, "recovery must fail the incomplete turn");
    let after = SqliteProductionRepository::get_turn(&db, &turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(after.status, TurnStatus::Failed);
    // 恢复同时 fail 非 terminal Attempt、把 Pending outbox 标记 failed、
    // 并写入 RecoveryFail 审计行。
    assert_eq!(
        after.find_attempt(&attempt_id).unwrap().status,
        AttemptStatus::Failed
    );
    let outbox_after = SqlitePreacceptRepository::list_outbox_for_turn(&db, &turn_id).unwrap();
    assert!(outbox_after.iter().any(|r| {
        r.kind == PreacceptOutboxKind::PostprocessApply && r.status == PreacceptOutboxStatus::Failed
    }));
    assert!(outbox_after.iter().any(|r| {
        r.kind == PreacceptOutboxKind::RecoveryFail && r.status == PreacceptOutboxStatus::Failed
    }));

    // 恢复后再重启：outbox 仍在（日志不被恢复流程清掉），且幂等——再跑恢复
    // 不再 fail 任何 Turn（terminal 保留），也不重复写审计行。
    drop(db);
    let db = Database::open(&db_path).unwrap();
    let outbox2 = SqlitePreacceptRepository::list_outbox_for_turn(&db, &turn_id).unwrap();
    assert_eq!(outbox2.len(), outbox_after.len());
    let mut db = db;
    let again = SqlitePreacceptRepository::fail_incomplete_preaccept(&mut db).unwrap();
    assert_eq!(again, 0, "recovery must be idempotent for terminal turns");
    let persisted = SqliteProductionRepository::get_turn(&db, &turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, TurnStatus::Failed);
    let _ = draft;
}

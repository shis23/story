//! Gate 5 Phase 7：大数据目录 & 性能（infra-sqlite 级）。
//!
//! 生成可复现的大数据 fixture（确定性 id/内容），实测并打印：
//! - JSON → SQLite cutover（迁移）耗时与迁移后计数等价性；
//! - SQLite → JSON reverse export 耗时与导出计数；
//! - 导出产物 → 新 DB reimport 耗时与计数等价性；
//! - 恢复路径（fail_incomplete）在数据量上的耗时。
//!
//! 时间只打印报告（真实数值 + 机器信息），不做脆弱的硬阈值断言；正确性
//! （计数等价、往返一致）是硬断言。全部使用独立 temp dir。

use std::fs;
use std::path::Path;
use std::time::Instant;

use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::character::CharacterDefinition;
use storyforge_domain::conversation::{Conversation, Role, VariantStatus};
use storyforge_domain::story_task::{TaskSource, TaskStatus};
use storyforge_domain::turn::{AttemptStatus, TurnRecord, TurnStatus};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, recover_or_verify,
};
use storyforge_infra_sqlite::exporter::export_sqlite_to_json;
use storyforge_infra_sqlite::importer::JsonImporter;
use storyforge_infra_sqlite::migrations::migrate;
use storyforge_infra_sqlite::production::SqliteProductionRepository;
use tempfile::TempDir;

// ─── fixture 规模（可复现；计数用于等价断言）─────────────────────────────
const N_CARDS: usize = 40;
const N_CAMPAIGNS: usize = 15;
const NODES_PER_CONV: usize = 30;
const INSTANCES_PER_CAMPAIGN: usize = 12;
const KNOWLEDGE_PER_INSTANCE: usize = 6;
const TASKS_PER_CAMPAIGN: usize = 40;
const SUMMARIES_PER_CAMPAIGN: usize = 30;
const TURNS_PER_CAMPAIGN: usize = 10;

fn write_json(path: &Path, value: serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn def(card_id: &str, i: usize) -> CharacterDefinition {
    serde_json::from_value(serde_json::json!({
        "id": format!("def-{card_id}-{i}"),
        "card_id": card_id,
        "name": format!("角色{i}"),
        "role_type": "supporting",
        "persona_prompt": format!("人设 {i}"),
        "behavior_rules": "",
        "base_backstory": ["背景".to_string()],
        "group": null,
        "variable_schema": [{
            "key": "hp", "label": "生命", "value_type": "int",
            "default": 100, "description": null, "group": null
        }],
    }))
    .unwrap()
}

/// 确定性大数据 fixture（固定 id/内容，两次生成逐字节一致）。
fn write_big_source(dir: &Path) -> usize {
    let mut total = 0usize;
    let mut cards = Vec::new();
    for c in 0..N_CARDS {
        let card_id = format!("card-{c:03}");
        let mut defs = Vec::new();
        for i in 0..3 {
            defs.push(def(&card_id, i));
        }
        cards.push(serde_json::json!({
            "card": {
                "id": card_id, "name": format!("卡{c}"),
                "source_character_id": format!("char-src-{c:03}"),
                "character_definitions": defs,
                "campaign_variable_schema": [],
                "raw_card_json": {},
                "extraction_status": "extracted",
                "extraction_message": null
            },
            "imported_at": "2026-07-01T00:00:00Z"
        }));
    }
    total += cards.len();
    write_json(&dir.join("cards.json"), serde_json::Value::Array(cards));

    let mut chars = Vec::new();
    for c in 0..N_CARDS {
        chars.push(serde_json::json!({
            "id": format!("char-{c:03}"),
            "info": {
                "name": format!("角色卡{c}"),
                "source_character_id": format!("char-src-{c:03}"),
                "description": "主角",
                "personality": "勇敢",
                "scenario": "地下城",
                "first_mes": "你好。",
                "mes_example": "",
                "creator": "test",
                "character_version": "1.0",
                "spec_version": "2.0",
                "system_prompt": "扮演",
                "post_history_instructions": "",
                "alternate_greetings": [],
                "tags": ["主角"],
                "extensions": {},
                "raw_card_json": {},
                "embedded_world_info": null,
                "renderable_assets": null,
                "has_world_info": false,
                "world_info_count": 0,
                "world_info_entries": []
            },
            "imported_at": "2026-07-01T00:00:00Z"
        }));
    }
    total += chars.len();
    write_json(
        &dir.join("characters.json"),
        serde_json::Value::Array(chars),
    );

    let mut campaigns = Vec::new();
    let mut conversations = Vec::new();
    let mut instances = Vec::new();
    let mut knowledge = Vec::new();
    let mut tasks = Vec::new();
    let mut summaries = Vec::new();
    let mut turns = Vec::new();
    for k in 0..N_CAMPAIGNS {
        let cid = format!("camp-{k:03}");
        let conv_id = format!("conv-{k:03}");
        campaigns.push(serde_json::json!({
            "id": cid, "card_id": format!("card-{k:03}"),
            "name": format!("局{k}"),
            "conversation_id": conv_id,
            "revision": 3, "chronicle_revision": 2,
            "lineage_id": format!("lin-{k:03}"),
            "story_clock": "Day 3",
            "variables": [{"key": "gold", "value": 42, "turn": 1}],
            "variable_schema": [],
            "created_at": "2026-07-02T00:00:00Z"
        }));

        // conversation：30 节点。
        let mut nodes = Vec::new();
        for n in 0..NODES_PER_CONV {
            nodes.push(serde_json::json!({
                "id": format!("node-{k:03}-{n:03}"),
                "role": if n % 2 == 0 { "user" } else { "assistant" },
                "content": format!("节点 {k}-{n} 的内容，包含一些正文文本用于体积。"),
                "status": "final",
                "created_at": format!("2026-07-02T00:00:{:02}Z", n % 60)
            }));
        }
        conversations.push(serde_json::json!({
            "id": conv_id, "campaign_id": cid, "character_id": null,
            "created_at": "2026-07-02T00:00:00Z",
            "updated_at": "2026-07-02T00:00:00Z",
            "nodes": nodes, "archived_upto": 0
        }));

        for i in 0..INSTANCES_PER_CAMPAIGN {
            let iid = format!("inst-{k:03}-{i:03}");
            instances.push(serde_json::json!({
                "id": iid, "campaign_id": cid,
                "definition_id": format!("def-card-{k:03}-{}", i % 3),
                "name": format!("角色{k}-{i}"),
                "is_temporary": false,
                "persona_override": null,
                "variables": [{"key": "hp", "value": 100, "turn": 0}]
            }));
            for j in 0..KNOWLEDGE_PER_INSTANCE {
                knowledge.push(serde_json::json!({
                    "id": format!("know-{k:03}-{i:03}-{j}"),
                    "campaign_id": cid, "character_id": iid,
                    "knowledge_text": format!("知识 {k}-{i}-{j}：暗门后的宝箱。"),
                    "source": "witnessed",
                    "source_character_id": null,
                    "turn_number": j as u64,
                    "event_id": null,
                    "pinned": false,
                    "propagation": "open"
                }));
            }
        }
        for t in 0..TASKS_PER_CAMPAIGN {
            tasks.push(serde_json::json!({
                "id": format!("task-{k:03}-{t:03}"),
                "campaign_id": cid,
                "title": format!("任务{k}-{t}"),
                "description": format!("任务描述 {k}-{t}"),
                "status": if t % 3 == 0 { "completed" } else { "pending" },
                "source": "user_planned",
                "created_turn": 1,
                "injected_turns": [],
                "triggers": [],
                "related_characters": []
            }));
        }
        for s in 0..SUMMARIES_PER_CAMPAIGN {
            summaries.push(serde_json::json!({
                "id": format!("sum-{k:03}-{s:03}"),
                "campaign_id": cid, "conversation_id": conv_id,
                "turn": s, "content": format!("摘要 {k}-{s} 的内容。"),
                "created_at": "2026-07-02T00:00:00Z",
                "level": 0,
                "lineage_id": format!("lin-{k:03}"),
                "code": format!("A{s:04}")
            }));
        }
        for t in 0..TURNS_PER_CAMPAIGN {
            turns.push(serde_json::json!({
                "turn_id": format!("turn-{k:03}-{t:03}"),
                "campaign_id": cid, "conversation_id": conv_id,
                "input_node_id": format!("node-{k:03}-{:03}", (t * 2) % NODES_PER_CONV),
                "base_campaign_revision": t as u64,
                "status": "committed",
                "accepted_attempt_id": format!("att-{k:03}-{t:03}"),
                "failure_reason": null,
                "intended_terminal_status": "committed",
                "attempts": [{
                    "attempt_id": format!("att-{k:03}-{t:03}"),
                    "variant_id": format!("var-{k:03}-{t:03}"),
                    "draft_hash": format!("hash-{k}-{t}"),
                    "status": "committed",
                    "pending_state_changes": null,
                    "derivation": null,
                    "quality_report": null,
                    "pending_temporary_instances": [],
                    "provenance": null,
                    "created_at": "2026-07-02T00:00:00Z"
                }],
                "created_at": "2026-07-02T00:00:00Z",
                "updated_at": "2026-07-02T00:00:00Z"
            }));
        }
    }
    total += campaigns.len()
        + conversations.len()
        + instances.len()
        + knowledge.len()
        + tasks.len()
        + summaries.len()
        + turns.len();

    write_json(
        &dir.join("campaigns.json"),
        serde_json::Value::Array(campaigns),
    );
    for conv in conversations {
        write_json(
            &dir.join("conversations")
                .join(format!("{}.json", conv["id"].as_str().unwrap())),
            conv,
        );
    }
    write_json(
        &dir.join("instances.json"),
        serde_json::Value::Array(instances),
    );
    write_json(
        &dir.join("knowledge.json"),
        serde_json::Value::Array(knowledge),
    );
    write_json(&dir.join("tasks.json"), serde_json::Value::Array(tasks));
    write_json(
        &dir.join("round_summaries.json"),
        serde_json::Value::Array(summaries),
    );
    write_json(&dir.join("turns.json"), serde_json::Value::Array(turns));
    total
}

fn machine() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let debug = cfg!(debug_assertions);
    format!(
        "{os}/{arch} {}build",
        if debug { "debug" } else { "release" }
    )
}

/// 大数据迁移 / reverse export / reimport / 恢复 全链路计时 + 计数等价。
#[test]
fn bigdata_migration_export_reimport_and_recovery_timings() {
    let dir = TempDir::new().unwrap();
    let entity_count = write_big_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");

    println!(
        "[bigdata] machine={} fixture_entities={entity_count}",
        machine()
    );

    // ── 1. JSON → SQLite cutover ─────────────────────────────────────
    let t0 = Instant::now();
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "bigdata-migration".into(),
    };
    match recover_or_verify(&request).unwrap() {
        CutoverOutcome::Completed(report) => {
            println!(
                "[bigdata] migration=Completed({}ms)",
                t0.elapsed().as_millis()
            );
            println!(
                "[bigdata] import counts: cards={} campaigns={} instances={} knowledge={} tasks={} summaries={} conversations={} turns={} characters={}",
                report.cards,
                report.campaigns,
                report.instances,
                report.knowledge,
                report.tasks,
                report.summaries,
                report.conversations,
                report.turns,
                report.characters
            );
        }
        other => panic!("expected Completed, got {other:?}"),
    }

    // 计数硬断言：导入结果 == fixture 计数。
    let mut db = Database::open(&db_path).unwrap();
    let table_counts = |db: &Database| -> Vec<(String, i64)> {
        [
            "character_cards",
            "campaigns",
            "character_instances",
            "character_knowledge",
            "story_tasks",
            "round_summaries",
            "conversations",
            "turns",
            "characters",
        ]
        .iter()
        .map(|t| {
            let n: i64 = db
                .connection()
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap();
            (t.to_string(), n)
        })
        .collect()
    };
    let expected: Vec<(String, i64)> = vec![
        ("character_cards".into(), N_CARDS as i64),
        ("campaigns".into(), N_CAMPAIGNS as i64),
        (
            "character_instances".into(),
            (N_CAMPAIGNS * INSTANCES_PER_CAMPAIGN) as i64,
        ),
        (
            "character_knowledge".into(),
            (N_CAMPAIGNS * INSTANCES_PER_CAMPAIGN * KNOWLEDGE_PER_INSTANCE) as i64,
        ),
        (
            "story_tasks".into(),
            (N_CAMPAIGNS * TASKS_PER_CAMPAIGN) as i64,
        ),
        (
            "round_summaries".into(),
            (N_CAMPAIGNS * SUMMARIES_PER_CAMPAIGN) as i64,
        ),
        ("conversations".into(), N_CAMPAIGNS as i64),
        ("turns".into(), (N_CAMPAIGNS * TURNS_PER_CAMPAIGN) as i64),
        ("characters".into(), N_CARDS as i64),
    ];
    let actual = table_counts(&db);
    for (table, want) in &expected {
        let got = actual
            .iter()
            .find(|(t, _)| t == table)
            .map(|(_, n)| *n)
            .unwrap_or(-1);
        assert_eq!(
            got, *want,
            "table {table} count must match fixture after migration"
        );
    }

    // ── 2. SQLite → JSON reverse export ──────────────────────────────
    // 三审7：导出目标必须在活动数据根**之外**（data_dir 内部任意子路径都被拒绝）。
    let export_root = TempDir::new().unwrap();
    let export_dir = export_root.path().join("export-out");
    let t1 = Instant::now();
    let exported = export_sqlite_to_json(&db, &export_dir).unwrap();
    println!("[bigdata] reverse_export={}ms", t1.elapsed().as_millis());
    let r = &exported.report;
    assert_eq!(r.cards, N_CARDS);
    assert_eq!(r.campaigns, N_CAMPAIGNS);
    assert_eq!(r.instances, N_CAMPAIGNS * INSTANCES_PER_CAMPAIGN);
    assert_eq!(
        r.knowledge,
        N_CAMPAIGNS * INSTANCES_PER_CAMPAIGN * KNOWLEDGE_PER_INSTANCE
    );
    assert_eq!(r.tasks, N_CAMPAIGNS * TASKS_PER_CAMPAIGN);
    assert_eq!(r.summaries, N_CAMPAIGNS * SUMMARIES_PER_CAMPAIGN);
    assert_eq!(r.conversations, N_CAMPAIGNS);
    assert_eq!(r.turns, N_CAMPAIGNS * TURNS_PER_CAMPAIGN);
    assert_eq!(r.characters, N_CARDS);
    // SQLite-native 台账（outbox/mutation_commits/chronicle jobs）无 JSON 对应
    // 文件 → 显式分类为 unsupported；本 fixture 无 pre-accept 活动，故均为
    // 0 行（无损）。除此之外不允许出现任何 unsupported 字段。
    assert_eq!(
        r.unsupported_fields.len(),
        3,
        "only the three SQLite-native ledgers may be classified unsupported: {:?}",
        r.unsupported_fields
    );
    for field in &r.unsupported_fields {
        assert!(
            field.starts_with("preaccept_outbox:")
                || field.starts_with("mutation_commits:")
                || field.starts_with("chronicle_publication_jobs:"),
            "unexpected unsupported field: {field}"
        );
        assert!(
            field.contains(":0 rows") || field.contains(":empty"),
            "big-data catalog must lose nothing: {field}"
        );
    }

    // ── 3. 导出产物 → 新 DB reimport ─────────────────────────────────
    let reimport_path = dir.path().join("reimport.sqlite3");
    let t2 = Instant::now();
    {
        let mut reimport_db = Database::open(&reimport_path).unwrap();
        migrate(&mut reimport_db).unwrap();
        let mut importer = JsonImporter::new(&mut reimport_db);
        let report = importer.import_data_dir(&export_dir).unwrap();
        println!(
            "[bigdata] reimport={}ms (cards={} campaigns={} turns={} characters={})",
            t2.elapsed().as_millis(),
            report.cards,
            report.campaigns,
            report.turns,
            report.characters
        );
        assert_eq!(report.cards, N_CARDS);
        assert_eq!(report.campaigns, N_CAMPAIGNS);
        assert_eq!(report.turns, N_CAMPAIGNS * TURNS_PER_CAMPAIGN);
        assert_eq!(report.characters, N_CARDS);
    }

    // ── 4. 恢复路径计时（大数据量上的 fail_incomplete）──────────────
    // 造 5 个未完成 Turn（Generating），再计时恢复。
    let t3 = Instant::now();
    let _now = chrono::Utc::now().to_rfc3339();
    for k in 0..5 {
        let mut turn = TurnRecord::new(
            Id::from_str(format!("camp-{k:03}")),
            Id::from_str(format!("conv-{k:03}")),
            Id::from_str(format!("node-{k:03}-000")),
            0,
        );
        turn.turn_id = Id::from_str(format!("turn-{k:03}-recover"));
        turn.status = TurnStatus::Generating;
        SqliteProductionRepository::save_turn(&mut db, &turn).unwrap();
    }
    let t3b = Instant::now();
    let failed = SqliteProductionRepository::fail_incomplete_turns(&mut db).unwrap();
    assert!(failed >= 5);
    println!(
        "[bigdata] recovery_fail_incomplete=({}ms prepare + {}ms run, failed={failed})",
        t3b.duration_since(t3).as_millis(),
        t3b.elapsed()
            .as_millis()
            .saturating_sub(t3.elapsed().as_millis()),
    );

    // ── 5. 重启/重开计时：真实 reopen（冷启动）────────────────────────
    // 审查二：旧方法测的不是真正的重开（复用已有连接/包含预热）。这里先关闭
    // 全部既有句柄，再用**全新** `Database::open` + `current_version` + 一条读
    // 查询完成重开，测量第一次冷重开成本；无人工 sleep，完全确定性。
    drop(db);
    let t5 = Instant::now();
    {
        let reopened = Database::open(&db_path).unwrap();
        let version = storyforge_infra_sqlite::migrations::current_version(&reopened).unwrap();
        let turn_count: i64 = reopened
            .connection()
            .query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))
            .unwrap();
        println!(
            "[bigdata] reopen_restart={}ms (version={version}, turns={turn_count})",
            t5.elapsed().as_millis()
        );
        assert_eq!(version, 8, "reopened DB must be fully migrated");
        // 第 4 步恢复了 5 个 Generating turn（fail_incomplete 只改状态不删行）。
        assert_eq!(
            turn_count,
            (N_CAMPAIGNS * TURNS_PER_CAMPAIGN) as i64 + 5,
            "reopened DB must serve the same data"
        );
    }
    let _ = entity_count;
}

/// 恢复路径正确性：fail_incomplete_turns 只 fail 非终态 Turn，终态保留。
#[test]
fn bigdata_recovery_only_fails_nonterminal_turns() {
    let dir = TempDir::new().unwrap();
    let mut db = Database::open(dir.path().join("recovery.sqlite3")).unwrap();
    migrate(&mut db).unwrap();

    let mut campaign = Campaign::new(Id::from_str("card-rec"), "恢复");
    campaign.id = Id::from_str("camp-rec");
    campaign.conversation_id = Some(Id::from_str("conv-rec"));
    let mut conversation = Conversation::new(None, Some(campaign.id.clone()));
    conversation.id = Id::from_str("conv-rec");
    let input = conversation.append_message(Role::User, "开场".into());
    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();

    let mut active = TurnRecord::new(
        campaign.id.clone(),
        conversation.id.clone(),
        input.clone(),
        0,
    );
    active.turn_id = Id::from_str("turn-active");
    active.status = TurnStatus::AwaitingAcceptance;
    SqliteProductionRepository::save_turn(&mut db, &active).unwrap();

    let mut committed = TurnRecord::new(
        campaign.id.clone(),
        conversation.id.clone(),
        input.clone(),
        1,
    );
    committed.turn_id = Id::from_str("turn-committed");
    committed.status = TurnStatus::Committed;
    committed.accepted_attempt_id = Some(Id::from_str("att-c"));
    committed
        .attempts
        .push(storyforge_domain::turn::TurnAttempt {
            attempt_id: Id::from_str("att-c"),
            variant_id: Id::from_str("var-c"),
            draft_hash: "h".into(),
            status: AttemptStatus::Committed,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "2026-07-02T00:00:00Z".into(),
        });
    SqliteProductionRepository::save_turn(&mut db, &committed).unwrap();

    let failed = SqliteProductionRepository::fail_incomplete_turns(&mut db).unwrap();
    assert_eq!(failed, 1, "only the non-terminal turn may be failed");
    let after_active = SqliteProductionRepository::get_turn(&db, &Id::from_str("turn-active"))
        .unwrap()
        .unwrap();
    assert_eq!(after_active.status, TurnStatus::Failed);
    let after_committed =
        SqliteProductionRepository::get_turn(&db, &Id::from_str("turn-committed"))
            .unwrap()
            .unwrap();
    assert_eq!(after_committed.status, TurnStatus::Committed);
    let _ = VariantStatus::Final;
    let _ = TaskStatus::Active;
    let _ = TaskSource::UserPlanned;
}

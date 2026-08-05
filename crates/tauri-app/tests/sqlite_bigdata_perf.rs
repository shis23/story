//! Gate 5 Phase 7：JSON vs SQLite 操作耗时（tauri-app 级）+ 大数据目录。
//!
//! 同一确定性大数据 fixture，先在 JSON 后端跑一组操作（计时），再 cutover
//! 到 SQLite 跑同一组操作（计时），最后打印对照表并断言计数等价。
//! cutover（迁移）与重启恢复（recover_turns / compress reset）也计时。
//!
//! 时间只打印报告，不做脆弱硬阈值；计数等价是硬断言。
//! `sqlite_runtime::activate` 是进程全局 OnceLock → 本二进制单 `#[test]`。

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use storyforge_domain::Id;
use storyforge_domain::character_knowledge::{
    CharacterKnowledgeEntry, KnowledgeSource, PropagationPolicy,
};
use storyforge_domain::story_task::StoryTask;
use storyforge_domain::turn::TurnRecord;
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, recover_or_verify,
};
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::StorageFacade;
use storyforge_lib::{AppState, TurnWorkflow};

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
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

/// 确定性大数据 fixture（与 infra-sqlite gate5_bigdata_perf 同构）。
fn write_big_source(dir: &Path) -> usize {
    let mut total = 0usize;
    let mut cards = Vec::new();
    for c in 0..N_CARDS {
        let card_id = format!("card-{c:03}");
        let mut defs = Vec::new();
        for i in 0..3 {
            defs.push(serde_json::json!({
                "id": format!("def-{card_id}-{i}"), "card_id": card_id,
                "name": format!("角色{i}"), "role_type": "supporting",
                "persona_prompt": format!("人设 {i}"), "behavior_rules": "",
                "base_backstory": ["背景".to_string()], "group": null,
                "variable_schema": []
            }));
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
                "source_character_id": format!("char-src-{c:03}"),
                "name": format!("角色卡{c}"), "description": "主角",
                "personality": "勇敢", "scenario": "地下城",
                "first_mes": "你好。", "mes_example": "",
                "post_history_instructions": "", "alternate_greetings": [],
                "system_prompt": "扮演", "tags": ["主角"],
                "creator": "test", "character_version": "1.0",
                "spec_version": "2.0", "extensions": {},
                "embedded_world_info": null, "renderable_assets": null,
                "raw_card_json": {}, "has_world_info": false,
                "has_renderable_assets": false, "world_info_count": 0,
                "world_info_entries": []
            },
            "imported_at": "2026-07-01 10:00:00"
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
            "name": format!("局{k}"), "conversation_id": conv_id,
            "revision": 3, "chronicle_revision": 2,
            "lineage_id": format!("lin-{k:03}"), "story_clock": "Day 3",
            "variables": [{"key": "gold", "value": 42, "last_updated_turn": 1}],
            "variable_schema": [], "created_at": "2026-07-02T00:00:00Z"
        }));
        let mut nodes = Vec::new();
        for n in 0..NODES_PER_CONV {
            nodes.push(serde_json::json!({
                "id": format!("node-{k:03}-{n:03}"),
                "parent_id": if n == 0 {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(format!("node-{k:03}-{:03}", n - 1))
                },
                "variants": [{
                    "id": format!("node-{k:03}-{n:03}-v0"),
                    "role": if n % 2 == 0 { "User" } else { "Assistant" },
                    "content": format!("节点 {k}-{n} 的内容。"),
                    "created_at": format!("2026-07-02T00:00:{:02}Z", n % 60),
                    "status": "Final",
                    "provenance": null
                }],
                "active_variant": 0
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
                "name": format!("角色{k}-{i}"), "is_temporary": false,
                "persona_override": null,
                "variables": [{"key": "hp", "value": 100, "last_updated_turn": 0}]
            }));
            for j in 0..KNOWLEDGE_PER_INSTANCE {
                knowledge.push(serde_json::json!({
                    "id": format!("know-{k:03}-{i:03}-{j}"),
                    "campaign_id": cid, "character_id": iid,
                    "knowledge_text": format!("知识 {k}-{i}-{j}。"),
                    "source": "witnessed", "source_character_id": null,
                    "turn_number": j as u64, "event_id": null,
                    "pinned": false, "propagation": "open"
                }));
            }
        }
        for t in 0..TASKS_PER_CAMPAIGN {
            tasks.push(serde_json::json!({
                "id": format!("task-{k:03}-{t:03}"), "campaign_id": cid,
                "title": format!("任务{k}-{t}"), "description": format!("描述 {k}-{t}"),
                "status": if t % 3 == 0 { "completed" } else { "pending" },
                "source": "user_planned", "created_turn": 1,
                "injected_turns": [], "triggers": [], "related_characters": []
            }));
        }
        for s in 0..SUMMARIES_PER_CAMPAIGN {
            summaries.push(serde_json::json!({
                "id": format!("sum-{k:03}-{s:03}"),
                "campaign_id": cid, "conversation_id": conv_id,
                "turn": s, "content": format!("摘要 {k}-{s}。"),
                "created_at": "2026-07-02T00:00:00Z", "level": 0,
                "lineage_id": format!("lin-{k:03}"), "code": format!("A{s:04}")
            }));
        }
        for t in 0..TURNS_PER_CAMPAIGN {
            turns.push(serde_json::json!({
                "turn_id": format!("turn-{k:03}-{t:03}"),
                "campaign_id": cid, "conversation_id": conv_id,
                "input_node_id": format!("node-{k:03}-{:03}", (t * 2) % NODES_PER_CONV),
                "base_campaign_revision": t as u64, "status": "committed",
                "accepted_attempt_id": format!("att-{k:03}-{t:03}"),
                "failure_reason": null, "intended_terminal_status": "committed",
                "attempts": [{
                    "attempt_id": format!("att-{k:03}-{t:03}"),
                    "variant_id": format!("var-{k:03}-{t:03}"),
                    "draft_hash": format!("hash-{k}-{t}"), "status": "committed",
                    "pending_state_changes": null, "derivation": null,
                    "quality_report": null, "pending_temporary_instances": [],
                    "provenance": null, "created_at": "2026-07-02T00:00:00Z"
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

fn json_app_state(dir: &Path) -> Arc<AppState> {
    let storage = Arc::new(StorageFacade::new(
        dir.to_path_buf(),
        PinnedBackend::new(StorageBackend::Json, BackendSource::Default),
    ));
    Arc::new(AppState::new_with_backend(dir.to_path_buf(), storage).expect("JSON AppState"))
}

struct OpPhase {
    timings: Vec<(String, Duration)>,
    /// 操作后的知识/任务计数（等价断言用）。
    knowledge_count: usize,
    task_count: usize,
}

/// 同一组操作，JSON/SQLite 各跑一遍。返回各操作耗时与终态计数。
fn run_op_phase(st: &Arc<AppState>, sqlite: bool) -> OpPhase {
    let mut timings: Vec<(String, Duration)> = Vec::new();
    let storage = st.storage();
    let cid = Id::from_str("camp-000");
    let alice = storage
        .list_instances(&cid)
        .expect("instances")
        .into_iter()
        .find(|i| i.name == "角色0-0")
        .expect("alice")
        .id;

    // ── a. 读操作 ────────────────────────────────────────────────────
    let t = Instant::now();
    let cards = storage.list_cards().expect("cards").len();
    let characters = storage.list_characters().expect("characters").len();
    let campaigns = storage.list_campaigns(None).expect("campaigns").len();
    timings.push(("read list_cards+characters+campaigns".into(), t.elapsed()));
    assert_eq!(cards, N_CARDS);
    assert_eq!(characters, N_CARDS);
    assert_eq!(campaigns, N_CAMPAIGNS);

    let t = Instant::now();
    let instances = storage.list_instances(&cid).expect("instances").len();
    let knowledge = storage.list_knowledge(&cid).expect("knowledge").len();
    let tasks = storage.list_tasks(&cid).expect("tasks").len();
    let summaries = storage.list_summaries(&cid).expect("summaries").len();
    timings.push(("read campaign aggregates".into(), t.elapsed()));
    assert_eq!(instances, INSTANCES_PER_CAMPAIGN);
    assert_eq!(knowledge, INSTANCES_PER_CAMPAIGN * KNOWLEDGE_PER_INSTANCE);
    assert_eq!(tasks, TASKS_PER_CAMPAIGN);
    assert_eq!(summaries, SUMMARIES_PER_CAMPAIGN);

    let t = Instant::now();
    for k in 0..N_CAMPAIGNS {
        storage
            .get_campaign(&Id::from_str(format!("camp-{k:03}")))
            .expect("get_campaign");
    }
    timings.push(("get_campaign ×15".into(), t.elapsed()));

    // ── b. add_knowledge 批量 ×200 ───────────────────────────────────
    let t = Instant::now();
    for b in 0..2 {
        let entries: Vec<CharacterKnowledgeEntry> = (0..100)
            .map(|i| CharacterKnowledgeEntry {
                id: Id::from_str(format!("extra-know-{b}-{i}")),
                campaign_id: cid.clone(),
                character_id: alice.clone(),
                knowledge_text: format!("附加知识 {b}-{i}。"),
                source: KnowledgeSource::Witnessed,
                source_character_id: None,
                source_knowledge_id: None,
                turn_number: 1,
                event_id: None,
                pinned: false,
                propagation: PropagationPolicy::Open,
            })
            .collect();
        storage.add_knowledge(&entries).expect("add_knowledge");
    }
    timings.push(("add_knowledge ×200".into(), t.elapsed()));

    // ── c. add_task ×100 ─────────────────────────────────────────────
    let t = Instant::now();
    for i in 0..100 {
        storage
            .add_task(&StoryTask::from_narrative(
                cid.clone(),
                format!("附加任务{i}"),
                format!("描述{i}"),
                vec![],
                2,
            ))
            .expect("add_task");
    }
    timings.push(("add_task ×100".into(), t.elapsed()));

    // ── d. update_campaign（读改写）×50 ───────────────────────────────
    let t = Instant::now();
    for i in 0..50 {
        let mut campaign = storage
            .get_campaign(&cid)
            .expect("get_campaign")
            .expect("campaign exists")
            .campaign;
        campaign.set_variable("gold", serde_json::json!(i), 1);
        storage.update_campaign(&campaign).expect("update_campaign");
    }
    timings.push(("update_campaign ×50".into(), t.elapsed()));

    // ── e. draft attempt ×10（10 个不同 campaign，避开 active 屏障）──
    let t = Instant::now();
    let workflow = TurnWorkflow::new(storage.clone(), st.conv_store.clone());
    for k in 0..10 {
        let cid_k = Id::from_str(format!("camp-{k:03}"));
        let conv_k = Id::from_str(format!("conv-{k:03}"));
        let user_node = st
            .conv_store
            .append_user_message(&conv_k, format!("消息{k}"))
            .expect("append user message");
        let turn = TurnRecord::new(cid_k.clone(), conv_k.clone(), user_node, 3);
        let turn_id = turn.turn_id.clone();
        storage.save_turn(&turn).expect("save_turn");
        let provisional_variant = if sqlite {
            Id::new()
        } else {
            st.conv_store
                .append_ai_draft(&conv_k, format!("草稿{k}"), None)
                .expect("JSON draft node")
        };
        workflow
            .create_draft_attempt(storyforge_lib::DraftAttemptRequest {
                campaign_id: &cid_k,
                conversation_id: &conv_k,
                turn_id: &turn_id,
                attempt_id: &Id::new(),
                draft_text: &format!("草稿{k}"),
                pending_temporary_instances: vec![],
                provisional_variant_id: Some(&provisional_variant),
                provenance: None,
            })
            .expect("create_draft_attempt");
    }
    timings.push(("draft attempt ×10".into(), t.elapsed()));

    // ── f. delete_knowledge ×50 ──────────────────────────────────────
    let t = Instant::now();
    let mut deleted = 0usize;
    for i in 0..50 {
        if storage
            .delete_knowledge(&Id::from_str(format!("extra-know-0-{i}")))
            .expect("delete_knowledge")
        {
            deleted += 1;
        }
    }
    timings.push(("delete_knowledge ×50".into(), t.elapsed()));
    assert_eq!(deleted, 50);

    let knowledge_count = storage.list_knowledge(&cid).expect("knowledge").len();
    let task_count = storage.list_tasks(&cid).expect("tasks").len();
    OpPhase {
        timings,
        knowledge_count,
        task_count,
    }
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

/// 主测试：同一大数据 fixture → JSON ops（计时）→ cutover（计时）→
/// SQLite ops（计时）→ 对照打印 + 计数等价断言 + 恢复计时。
#[tokio::test]
async fn bigdata_json_vs_sqlite_op_timings() {
    // ── JSON 阶段 ────────────────────────────────────────────────────
    let json_dir = tempfile::tempdir().unwrap();
    let entity_count = write_big_source(json_dir.path());
    println!(
        "[bigdata-ops] machine={} fixture_entities={entity_count}",
        machine()
    );
    let json_state = json_app_state(json_dir.path());
    let json_phase = run_op_phase(&json_state, false);
    drop(json_state);

    // ── SQLite 阶段：同一 fixture cutover → 激活 → 同一 op 序列 ──────
    let sqlite_dir = tempfile::tempdir().unwrap();
    write_big_source(sqlite_dir.path());
    let db_path = sqlite_dir.path().join("storyforge.sqlite3");
    let cutover = CutoverRequest {
        plan: CutoverPlan::new(sqlite_dir.path(), &db_path),
        label: "gate5-bigdata-ops".into(),
        allow_json_authoritative_flip: false,
    };
    let t_cutover = Instant::now();
    match recover_or_verify(&cutover).expect("cutover") {
        CutoverOutcome::Completed(report) => {
            println!(
                "[bigdata-ops] cutover(migration)={}ms (cards={} campaigns={} instances={} knowledge={} tasks={} turns={} characters={})",
                t_cutover.elapsed().as_millis(),
                report.cards,
                report.campaigns,
                report.instances,
                report.knowledge,
                report.tasks,
                report.turns,
                report.characters
            );
            assert_eq!(report.cards, N_CARDS);
            assert_eq!(report.campaigns, N_CAMPAIGNS);
            assert_eq!(report.turns, N_CAMPAIGNS * TURNS_PER_CAMPAIGN);
        }
        other => panic!("expected Completed, got {other:?}"),
    }
    sqlite_runtime::activate(&db_path).expect("activate sqlite");
    let sqlite_storage = Arc::new(StorageFacade::new(
        sqlite_dir.path().to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    ));
    sqlite_storage
        .validate_runtime_authority()
        .expect("facade/runtime authority");
    let sqlite_state = Arc::new(
        AppState::new_with_backend(sqlite_dir.path().to_path_buf(), sqlite_storage)
            .expect("SQLite AppState"),
    );
    let sqlite_phase = run_op_phase(&sqlite_state, true);

    // ── 恢复计时（大数据量上的启动恢复）──────────────────────────────
    let t_recover = Instant::now();
    let failed_turns = sqlite_runtime::recover_turns_on_startup().expect("recover turns");
    let reset_jobs = sqlite_runtime::compress_reset_running_to_pending().expect("reset compress");
    println!(
        "[bigdata-ops] restart_recovery={}ms (failed_turns={failed_turns}, reset_compress_jobs={reset_jobs})",
        t_recover.elapsed().as_millis()
    );
    assert!(
        failed_turns >= 10,
        "the 10 draft turns must be failed by recovery"
    );
    drop(sqlite_state);

    // ── 四.5：真实的冷重开计时（drop 全部句柄 → 全新 Database::open）──
    // 不用已有 runtime 连接（OnceLock 全局句柄是热的）；这是对同一 DB 文件的
    // 全新 OS 文件句柄：open + current_version + 真实读查询全流程计时。
    // 无任何人工 sleep；断言全部确定性（成功 + 版本 + 计数）。
    let cold: Option<(u64, u32, Duration)> = (|| {
        let t_cold = Instant::now();
        let fresh = storyforge_infra_sqlite::Database::open(&db_path).ok()?;
        let version = storyforge_infra_sqlite::migrations::current_version(&fresh).ok()?;
        let count: i64 = fresh
            .connection()
            .query_row("SELECT COUNT(*) FROM campaigns", [], |row| row.get(0))
            .ok()?;
        Some((version as u64, count as u32, t_cold.elapsed()))
    })();
    let (cold_version, cold_count, cold_elapsed) = cold.expect("cold reopen must succeed");
    println!(
        "[bigdata-ops] cold_reopen={}ms (version={cold_version}, campaigns={cold_count})",
        cold_elapsed.as_millis()
    );
    assert!(
        cold_version >= 4,
        "cold reopen must read the current schema, got {cold_version}"
    );
    assert_eq!(
        cold_count as usize, N_CAMPAIGNS,
        "cold reopen read query must see all campaigns"
    );

    // ── 对照打印 ──────────────────────────────────────────────────────
    println!("[bigdata-ops] op_timings (json_ms, sqlite_ms, ratio):");
    for (j, s) in json_phase.timings.iter().zip(sqlite_phase.timings.iter()) {
        assert_eq!(j.0, s.0, "op labels must align");
        let jm = j.1.as_millis();
        let sm = s.1.as_millis();
        let ratio = if jm == 0 {
            "inf".to_string()
        } else {
            format!("{:.2}", sm as f64 / jm as f64)
        };
        println!("[bigdata-ops]   {:<32} {:>6} {:>6}  x{ratio}", j.0, jm, sm);
    }

    // ── 计数等价硬断言 ────────────────────────────────────────────────
    assert_eq!(
        json_phase.knowledge_count, sqlite_phase.knowledge_count,
        "knowledge counts must match: JSON={} SQLite={}",
        json_phase.knowledge_count, sqlite_phase.knowledge_count
    );
    assert_eq!(
        json_phase.task_count, sqlite_phase.task_count,
        "task counts must match: JSON={} SQLite={}",
        json_phase.task_count, sqlite_phase.task_count
    );
    println!(
        "[bigdata-ops] final counts: knowledge={} tasks={}",
        json_phase.knowledge_count, json_phase.task_count
    );
}

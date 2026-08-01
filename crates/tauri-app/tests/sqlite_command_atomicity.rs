//! Gate 5 三.5/三.6/三.7/四.2 SQLite 后端命令原子性判别测试（Wave 2b）。
//!
//! 本二进制**只**驱动 SQLite 后端（`sqlite_runtime::activate` + SQLite
//! AppState），绝不触碰 legacy JSON 权威——满足单权威约束（JSON 侧判别测试
//! 见 `command_atomicity.rs`，独立二进制）。
//!
//! `sqlite_runtime::activate` 是进程全局的，因此本二进制共享一个数据目录 /
//! DB 路径（进程 id 派生），每个测试使用互不冲突的 id 前缀；并行执行安全。

use std::path::Path;
use std::sync::{Arc, OnceLock};

use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{CharacterCard, CharacterExtractionStatus};
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::conversation::Conversation;
use storyforge_domain::story_task::StoryTask;
use storyforge_domain::turn::{AttemptStatus, Mutation, MutationBatch, TurnRecord, TurnStatus};
use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::AppState;
use storyforge_lib::campaign_store::StoredCard;
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::{CharacterInfo, StorageFacade};
use storyforge_lib::turn_lifecycle::compute_draft_hash;

/// 进程共享的 SQLite 权威目录 + DB（首次调用时激活）。
fn app_dir() -> &'static Path {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("sf-sqlite-cmd-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sqlite_runtime::activate(dir.join("storyforge.sqlite3"))
            .expect("activate SQLite authority");
        dir
    })
}

fn sqlite_state() -> Arc<AppState> {
    let data_dir = app_dir().to_path_buf();
    let storage = Arc::new(StorageFacade::new(
        data_dir.clone(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    ));
    storage
        .validate_runtime_authority()
        .expect("facade/runtime authority must match");
    Arc::new(AppState::new_with_backend(data_dir, storage).expect("SQLite AppState must construct"))
}

fn to_state<'a>(state: &'a Arc<AppState>) -> tauri::State<'a, Arc<AppState>> {
    unsafe { std::mem::transmute::<&'a Arc<AppState>, tauri::State<'a, Arc<AppState>>>(state) }
}

fn sample_character_info(name: &str) -> CharacterInfo {
    CharacterInfo {
        source_character_id: Some(format!("src-{name}")),
        name: name.to_string(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: format!("你好，我是{name}"),
        mes_example: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: vec![],
        system_prompt: String::new(),
        tags: vec![],
        creator: "sqlite-cmd-atomicity".into(),
        character_version: "1.0".into(),
        spec_version: "3.0".into(),
        extensions: serde_json::json!({}),
        embedded_world_info: Some(book_with_entries()),
        renderable_assets: None,
        raw_card_json: serde_json::json!({ "spec": "3.0" }),
        has_world_info: true,
        has_renderable_assets: false,
        world_info_count: 1,
        world_info_entries: vec![],
    }
}

fn book_with_entries() -> WorldInfoBook {
    WorldInfoBook {
        entries: vec![WorldInfoEntry {
            st_id: Some(1),
            keys: vec!["圣都".into()],
            secondary_keys: vec![],
            content: "梵尼亚".into(),
            constant: true,
            selective: false,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 2,
            order: 100,
            route: LoreRoute::Constant,
            extensions: serde_json::json!({}),
            extra: Default::default(),
        }],
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}

fn seed_card(card_id: &Id, source_id: &Id) {
    let card = CharacterCard {
        id: card_id.clone(),
        name: "种子卡".into(),
        source_character_id: source_id.clone(),
        character_definitions: vec![],
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({}),
        extraction_status: CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    let stored = StoredCard {
        card,
        imported_at: chrono::Utc::now().to_rfc3339(),
    };
    let payload = serde_json::to_value(stored).unwrap();
    sqlite_runtime::save_card_payload(card_id, "种子卡", Some(source_id.as_str()), None, &payload)
        .expect("seed card payload");
}

fn seed_campaign(card_id: &Id, label: &str) -> Id {
    let mut campaign = Campaign::new(card_id.clone(), format!("camp-{label}"));
    campaign.id = Id::from_str(format!("camp-{label}"));
    let id = campaign.id.clone();
    sqlite_runtime::save_campaign(&campaign).expect("seed campaign");
    id
}

fn seed_active_turn(campaign_id: &Id, conversation: &Conversation, label: &str) {
    // turns 表有 conversation_id → conversations FK：先持久化会话再落 Turn。
    sqlite_runtime::save_conversation(conversation).expect("seed conversation");
    let mut record = TurnRecord::new(
        campaign_id.clone(),
        conversation.id.clone(),
        Id::from_str(format!("input-{label}")),
        0,
    );
    record.status = TurnStatus::Generating;
    record.turn_id = Id::from_str(format!("turn-{label}"));
    sqlite_runtime::save_turn(&record).expect("seed active turn");
}

// ─── 三.5：活动 Turn 屏障 + 写入在同一 UoW 事务 ───────────────────────────

#[test]
fn sqlite_idle_mutation_uow_rolls_back_on_failure() {
    let state = sqlite_state();
    let label = "uow";
    let card_id = Id::from_str("card-uow");
    let source_id = Id::from_str("src-uow");
    seed_card(&card_id, &source_id);
    let campaign_id = seed_campaign(&card_id, label);
    let conv = Conversation::new(None, Some(campaign_id.clone()));
    seed_active_turn(&campaign_id, &conv, label);

    // (a) 活动 Turn 存在 → 写变量被拒，且不产生任何写入（检查与写入同事务）。
    let error = state
        .storage()
        .mutate_idle_campaign(&campaign_id, |campaign| {
            campaign.set_variable("custom_key", serde_json::json!(1), 0);
            Ok(())
        })
        .expect_err("mutate_idle_campaign must be rejected while an active turn exists");
    assert!(error.contains("未完成的轮次"), "error: {error}");
    let after = sqlite_runtime::get_campaign(&campaign_id).unwrap().unwrap();
    assert_eq!(
        after.get_variable("custom_key"),
        None,
        "no write may land while the active turn exists"
    );

    // (b) 无活动 Turn（终态化种子 Turn）→ 闭包内注入失败 → 整个 UoW 回滚：
    //     变量不变、Turn 仍在、DB 无任何残留。
    {
        let mut turn = sqlite_runtime::get_turn(&Id::from_str("turn-uow"))
            .unwrap()
            .unwrap();
        turn.status = TurnStatus::Committed;
        sqlite_runtime::save_turn(&turn).unwrap();
    }
    let error = state
        .storage()
        .mutate_idle_campaign(&campaign_id, |campaign| {
            campaign.set_variable("custom_key", serde_json::json!(1), 0);
            Err("injected failure inside the UoW".to_string())
        })
        .expect_err("closure failure must surface");
    assert!(error.contains("injected"), "error: {error}");
    let after = sqlite_runtime::get_campaign(&campaign_id).unwrap().unwrap();
    assert_eq!(
        after.get_variable("custom_key"),
        None,
        "UoW must roll back the in-memory mutation when the closure fails"
    );
    assert!(
        sqlite_runtime::get_turn(&Id::from_str("turn-uow"))
            .unwrap()
            .is_some(),
        "turn row must survive the rolled-back UoW"
    );
}

#[test]
fn sqlite_idle_instance_and_task_mutation_atomicity() {
    let state = sqlite_state();
    let label = "inst";
    let card_id = Id::from_str("card-inst");
    let source_id = Id::from_str("src-inst");
    seed_card(&card_id, &source_id);
    let campaign_id = seed_campaign(&card_id, label);
    let conv = Conversation::new(None, Some(campaign_id.clone()));
    let mut instance = CharacterInstance::temporary(campaign_id.clone(), "临时角色");
    instance.id = Id::from_str("inst-inst");
    sqlite_runtime::save_instance(&instance).unwrap();
    seed_active_turn(&campaign_id, &conv, label);

    // (a) 活动 Turn → 实例变量写入被拒（同一 UoW 内检查）。
    let error = state
        .storage()
        .mutate_idle_instance(&campaign_id, &instance.id, |inst| {
            inst.set_variable("custom_hp", serde_json::json!(5), 0);
            Ok(())
        })
        .expect_err("mutate_idle_instance must be rejected while an active turn exists");
    assert!(error.contains("未完成的轮次"), "error: {error}");
    let after = sqlite_runtime::list_instances(&campaign_id).unwrap();
    assert_eq!(
        after[0].get_variable("custom_hp"),
        None,
        "instance unchanged"
    );

    // (b) 任务写入：活动 Turn → add_idle_task 被拒且无行；验证闭包失败回滚。
    let task = StoryTask::user_planned(
        campaign_id.clone(),
        "伏笔".to_string(),
        "复仇".to_string(),
        vec![],
        1,
    );
    let error = state
        .storage()
        .add_idle_task(&campaign_id, &task)
        .expect_err("add_idle_task must be rejected while an active turn exists");
    assert!(error.contains("未完成的轮次"), "error: {error}");
    assert!(
        sqlite_runtime::get_task(&task.id).unwrap().is_none(),
        "no task row may be inserted while the active turn exists"
    );
}

// ─── 三.6：create_campaign bundle 无孤儿（SQLite 侧）──────────────────────

#[test]
fn sqlite_create_campaign_bundle_cleanup_on_world_info_failure() {
    let state = sqlite_state();
    let label = "bundle-wi";
    let card_id = Id::from_str("card-bundle-wi");
    let source_id = Id::from_str("src-bundle-wi");
    // 角色库角色：info_json 直接写坏（世界书模板解析阶段失败注入）。
    // 注意：角色 source 必须与卡 source 一致（"src-bundle-wi"），否则损坏
    // 行不会被模板解析命中。
    sqlite_runtime::save_character(&sample_character_info("bundle-wi")).unwrap();
    sqlite_runtime::with_db_raw_write(|conn| {
        conn.execute(
            "UPDATE characters SET info_json = '{ corrupted world info json }' \
             WHERE source_character_id = ?1",
            [source_id.as_str()],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .expect("corrupt character payload");
    seed_card(&card_id, &source_id);

    // 开档：Campaign + 实例已落库，但世界书模板解析失败（角色 payload 损坏）
    // → 旧实现 `tracing::warn!` 吞掉返回 Ok（半成品 bundle）；新实现必须 Err
    // 且补偿删除会话 + Campaign（无孤儿）。
    let error = storyforge_lib::create_campaign(
        card_id.as_str().to_string(),
        format!("bundle-{label}"),
        None,
        to_state(&state),
    )
    .expect_err("create_campaign must fail when world-info template resolution fails");
    assert!(error.to_string().contains("存储写入失败"), "error: {error}");

    // 无孤儿：该卡没有任何 Campaign 行；也没有指向不存在 Campaign 的会话行。
    let campaigns = sqlite_runtime::list_campaigns().unwrap();
    assert!(
        campaigns.iter().all(|c| c.card_id != card_id),
        "no campaign of the failed bundle may survive"
    );
    let orphan_conversations: i64 = sqlite_runtime::with_db_raw(|db| {
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM conversations WHERE campaign_id IS NOT NULL \
                 AND campaign_id NOT IN (SELECT campaign_id FROM campaigns)",
                [],
                |row| row.get(0),
            )
            .expect("query orphan conversations")
    });
    assert_eq!(
        orphan_conversations, 0,
        "no orphan conversation may survive the failed bundle"
    );
}

// ─── 三.7：delete_card 完整级联（SQLite 侧）───────────────────────────────

#[test]
fn sqlite_delete_card_cascades_all_associated_data() {
    let state = sqlite_state();
    let label = "del";
    let card_id = Id::from_str("card-del");
    let source_id = Id::from_str("src-del");
    seed_card(&card_id, &source_id);
    let campaign_id = seed_campaign(&card_id, label);

    // 会话（含 AI 草稿节点）+ Turn。
    let mut conv = Conversation::new(None, Some(campaign_id.clone()));
    let _variant_id = conv.append_ai_draft("正文".to_string(), None);
    let conversation_id = conv.id.clone();
    sqlite_runtime::save_conversation(&conv).unwrap();
    let mut bound = sqlite_runtime::get_campaign(&campaign_id).unwrap().unwrap();
    bound.conversation_id = Some(conversation_id.clone());
    sqlite_runtime::save_campaign(&bound).unwrap();

    let mut record = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        Id::from_str("input-del"),
        0,
    );
    record.status = TurnStatus::Committed;
    record.turn_id = Id::from_str("turn-del");
    sqlite_runtime::save_turn(&record).unwrap();

    // 实例 / 知识 / 任务 / 摘要 / 世界书 / 压缩任务 / MVU。
    let instance = CharacterInstance::temporary(campaign_id.clone(), "临时角色");
    sqlite_runtime::save_instance(&instance).unwrap();
    let knowledge = CharacterKnowledgeEntry {
        id: Id::from_str("k-del"),
        campaign_id: campaign_id.clone(),
        character_id: instance.id.clone(),
        knowledge_text: "刀".into(),
        source: storyforge_domain::character_knowledge::KnowledgeSource::Witnessed,
        source_character_id: None,
        source_knowledge_id: None,
        turn_number: 1,
        event_id: None,
        pinned: false,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    sqlite_runtime::save_knowledge(&knowledge).unwrap();
    let task = StoryTask::user_planned(
        campaign_id.clone(),
        "伏笔".to_string(),
        "复仇".to_string(),
        vec![],
        1,
    );
    sqlite_runtime::save_task(&task).unwrap();
    let summary = storyforge_domain::agent::RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        1,
        "本轮纪要".into(),
    );
    sqlite_runtime::seed_summary(&summary).unwrap();
    state
        .storage()
        .set_world_info(&campaign_id, &book_with_entries())
        .unwrap();
    state
        .storage()
        .enqueue_compress_job(&campaign_id, Some(conversation_id.clone()), None, 200, 0)
        .unwrap();
    let stored_mvu = storyforge_lib::campaign_store::StoredMvuTranslation {
        source_character_id: source_id.clone(),
        character_name: "种子卡".into(),
        translation: storyforge_domain::mvu_translation::MvuTranslation {
            variable_schema: vec![],
            ui_bindings: vec![],
            update_rules: vec![],
            interactions: vec![],
            fallback_fragments: vec![],
            routing: storyforge_domain::mvu_translation::MvuRouting::Native,
            analysis_confidence: 1.0,
            notes: vec![],
        },
        analyzed_at: chrono::Utc::now().to_rfc3339(),
    };
    state.storage().save_mvu(&stored_mvu).unwrap();

    // 活跃指针 + 会话缓存预热。
    *state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(campaign_id.clone());
    state
        .storage()
        .save_active_pointer(Some(&campaign_id))
        .unwrap();
    state.conv_store.invalidate();
    assert!(
        state.conv_store.get(&conversation_id).is_some(),
        "conv cache warmed"
    );

    storyforge_lib::delete_card(card_id.as_str().to_string(), to_state(&state))
        .expect("delete_card must succeed over SQLite");

    // 卡 / Campaign / 会话 / Turn / 实例 / 知识 / 任务 / 摘要 / 世界书 / MVU / 压缩任务。
    assert!(
        sqlite_runtime::get_card_payload(&card_id)
            .unwrap()
            .is_none()
    );
    assert!(
        sqlite_runtime::get_campaign(&campaign_id)
            .unwrap()
            .is_none()
    );
    assert!(
        sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .is_none()
    );
    assert!(
        sqlite_runtime::get_turn(&Id::from_str("turn-del"))
            .unwrap()
            .is_none()
    );
    assert!(
        sqlite_runtime::list_instances(&campaign_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        sqlite_runtime::list_knowledge(&campaign_id)
            .unwrap()
            .is_empty()
    );
    assert!(sqlite_runtime::list_tasks(&campaign_id).unwrap().is_empty());
    assert!(
        sqlite_runtime::list_summaries(&campaign_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        sqlite_runtime::get_world_info(&campaign_id)
            .unwrap()
            .is_none()
    );
    assert!(sqlite_runtime::get_mvu(&source_id).unwrap().is_none());
    assert!(state.storage().list_compress_jobs().unwrap().is_empty());

    // 活跃指针 + 会话缓存 + tool_ctx（进程内状态，in-process 断言）。
    assert!(
        state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none(),
        "active pointer must be cleared"
    );
    assert!(
        state.conv_store.get(&conversation_id).is_none(),
        "conversation cache must be invalidated"
    );
    // 注意：本二进制共享一个 DB（并行测试各自建会话），只能断言本测试的
    // 会话已消失，不能断言整个 store 为空。
}

// ─── 四.2：SQLite MutationBatch 最终持久化状态 == "committed" ─────────────

#[test]
fn sqlite_accept_persists_mutation_batch_committed() {
    let _state = sqlite_state(); // 触发激活（app_dir OnceLock）
    let label = "batch";
    let card_id = Id::from_str("card-batch");
    let source_id = Id::from_str("src-batch");
    seed_card(&card_id, &source_id);
    let campaign_id = seed_campaign(&card_id, label);

    let mut conv = Conversation::new(None, Some(campaign_id.clone()));
    let variant_id = conv.append_ai_draft("正文".to_string(), None);
    let conversation_id = conv.id.clone();
    sqlite_runtime::save_conversation(&conv).unwrap();
    let mut bound = sqlite_runtime::get_campaign(&campaign_id).unwrap().unwrap();
    bound.conversation_id = Some(conversation_id.clone());
    sqlite_runtime::save_campaign(&bound).unwrap();

    let camp = sqlite_runtime::get_campaign(&campaign_id).unwrap().unwrap();
    let mut batch = MutationBatch::new(Id::from_str("commit-batch"), camp.revision);
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: variant_id.clone(),
    });
    batch.mutations.push(Mutation::SetVariable {
        instance_id: None,
        key: "custom_key".into(),
        value: serde_json::json!("Day 1"),
        turn: 1,
    });
    let attempt = storyforge_domain::turn::TurnAttempt {
        attempt_id: Id::from_str("attempt-batch"),
        variant_id: variant_id.clone(),
        draft_hash: compute_draft_hash("正文"),
        status: AttemptStatus::AwaitingAcceptance,
        pending_state_changes: Some(batch),
        derivation: None,
        quality_report: Some(storyforge_domain::turn::QualityReport { warnings: vec![] }),
        pending_temporary_instances: vec![],
        provenance: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let mut record = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        Id::from_str("input-batch"),
        0,
    );
    record.status = TurnStatus::AwaitingAcceptance;
    record.turn_id = Id::from_str("turn-batch");
    record.attempts.push(attempt);
    sqlite_runtime::save_turn(&record).unwrap();

    sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &variant_id, false)
        .expect("SQLite accept must succeed");

    // 直接查询 turns 表原始 payload：batch status 必须 == "committed"。
    let raw_payload: String = sqlite_runtime::with_db_raw(|db| {
        db.connection()
            .query_row(
                "SELECT payload_json FROM turns WHERE turn_id = 'turn-batch'",
                [],
                |row| row.get(0),
            )
            .expect("turn row must exist")
    });
    let turn_value: serde_json::Value = serde_json::from_str(&raw_payload).unwrap();
    let batch_value = &turn_value["attempts"][0]["pending_state_changes"];
    assert_eq!(
        batch_value["status"].as_str(),
        Some("committed"),
        "SQLite persisted MutationBatch status must be 'committed' (raw payload: {raw_payload})"
    );
    assert_ne!(
        batch_value["status"].as_str(),
        Some("prepared"),
        "SQLite persisted MutationBatch must not remain 'prepared' after a successful commit"
    );
    // 内存侧（get_turn 反序列化）同样为 Committed。
    let persisted = sqlite_runtime::get_turn(&Id::from_str("turn-batch"))
        .unwrap()
        .unwrap();
    let persisted_batch = persisted
        .attempts
        .iter()
        .find(|a| a.variant_id == variant_id)
        .and_then(|a| a.pending_state_changes.as_ref())
        .expect("batch must be retained");
    assert_eq!(
        persisted_batch.status,
        storyforge_domain::turn::MutationBatchStatus::Committed
    );
}

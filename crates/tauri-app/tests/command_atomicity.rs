//! Gate 5 三.5/三.6/三.7/三.9/四.2 JSON 后端命令原子性判别测试（Wave 2b）。
//!
//! 本二进制**只**驱动 JSON 后端（StorageBackend::Json facade + AppState），
//! 绝不激活 SQLite——满足单权威约束（SQLite 相关判别测试见
//! `sqlite_command_atomicity.rs`，独立二进制）。
//!
//! 覆盖：
//! - 三.9：TurnStore / CampaignStore / CharacterStore 全部 mutator 在持久化
//!   失败（write_fence 冻结目标文件）时 → Err + 内存不变 + 磁盘不变。
//! - 三.5：活动 Turn 屏障 + 变量写入在同一原子单元（turns 锁守卫内
//!   candidate→persist→swap）；持久化失败整体回滚；并发检查+写入无丢失更新。
//! - 三.6：create_campaign / fork_campaign bundle 失败时无孤儿（会话 + Campaign
//!   全清理；重启后一致）。
//! - 三.7：delete_card 全量级联（会话/Turn/压缩任务/世界书/活跃指针/缓存/
//!   tool_ctx）+ 中途失败整体回滚。
//! - 四.2：JSON accept 提交后持久化 MutationBatch status == "committed"
//!   （与 SQLite 真值一致，不再永久 "prepared"）。

use std::path::Path;
use std::sync::Arc;

use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{
    CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
};
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::story_task::StoryTask;
use storyforge_domain::turn::{AttemptStatus, MutationBatch, TurnRecord, TurnStatus};
use storyforge_domain::variables::default_character_variables;
use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::AppState;
use storyforge_lib::campaign_store::CampaignStore;
use storyforge_lib::storage_backend::{CharacterInfo, StorageFacade};
use storyforge_lib::turn_lifecycle::compute_draft_hash;
use storyforge_lib::turn_store::TurnStore;

// ─── 夹具 ─────────────────────────────────────────────────────────────────

fn temp_dir(label: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("sf-cmd-atomic-{label}-"))
        .tempdir()
        .expect("temp dir")
}

fn json_state(dir: &Path) -> Arc<AppState> {
    let storage = Arc::new(StorageFacade::new(
        dir.to_path_buf(),
        PinnedBackend::new(StorageBackend::Json, BackendSource::Env),
    ));
    storage
        .validate_runtime_authority()
        .expect("JSON facade must be self-consistent without an active SQLite runtime");
    Arc::new(
        AppState::new_with_backend(dir.to_path_buf(), storage)
            .expect("JSON AppState must construct"),
    )
}

fn to_state<'a>(state: &'a Arc<AppState>) -> tauri::State<'a, Arc<AppState>> {
    unsafe { std::mem::transmute::<&'a Arc<AppState>, tauri::State<'a, Arc<AppState>>>(state) }
}

fn make_card(id: &str, with_definitions: bool) -> CharacterCard {
    let mut card = CharacterCard {
        id: Id::from_str(id),
        name: format!("卡-{id}"),
        source_character_id: Id::from_str(format!("src-{id}")),
        character_definitions: vec![],
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({}),
        extraction_status: CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    if with_definitions {
        card.character_definitions.push(CharacterDefinition {
            id: Id::from_str(format!("def-{id}")),
            card_id: card.id.clone(),
            name: "林医生".into(),
            persona_prompt: "外科医生".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        });
    }
    card
}

fn make_campaign(card_id: &Id, label: &str) -> Campaign {
    let mut campaign = Campaign::new(card_id.clone(), label.to_string());
    campaign.id = Id::from_str(format!("camp-{label}"));
    campaign
}

fn sample_character_info(name: &str) -> CharacterInfo {
    CharacterInfo {
        source_character_id: Some(format!("src-{name}")),
        name: name.to_string(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: "你好".into(),
        mes_example: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: vec![],
        system_prompt: String::new(),
        tags: vec![],
        creator: "command-atomicity".into(),
        character_version: "1.0".into(),
        spec_version: "3.0".into(),
        extensions: serde_json::json!({}),
        embedded_world_info: None,
        renderable_assets: None,
        raw_card_json: serde_json::json!({}),
        has_world_info: false,
        has_renderable_assets: false,
        world_info_count: 0,
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

fn active_turn_record(campaign_id: &Id, conversation_id: &Id) -> TurnRecord {
    let mut record = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        Id::from_str("input-node"),
        0,
    );
    record.status = TurnStatus::Generating;
    record
}

fn reload_campaigns(dir: &Path) -> Vec<Campaign> {
    CampaignStore::new(dir).list_campaigns()
}

/// 重开同目录会话存储，返回可见会话数量（重启一致性断言用）。
fn reload_conversation_count(dir: &Path) -> usize {
    let store = storyforge_app_conversation::ConversationStore::new(dir.join("conversations"));
    store.list().len()
}

// ─── 三.9：JSON store mutator 持久化失败原子性 ────────────────────────────

#[test]
fn json_campaign_store_mutators_rollback_on_persist_failure() {
    let dir = temp_dir("campaign-mutators");
    let store = CampaignStore::new(dir.path());
    let campaign = make_campaign(&Id::from_str("card-1"), "atomic");
    let campaign_id = campaign.id.clone();
    store.save_campaign(campaign.clone()).unwrap();
    let instance = CharacterInstance::temporary(campaign_id.clone(), "Lin");
    store.add_instance(instance.clone()).unwrap();

    // update_campaign：候选 → 持久化 → 换入；冻结后必须 Err + 内存/磁盘不变。
    let campaigns_path = dir.path().join("campaigns.json");
    storyforge_infra_util::write_fence::freeze(&campaigns_path);
    let mut renamed = campaign.clone();
    renamed.name = "改名但不得落盘".into();
    let error = store
        .update_campaign(renamed.clone())
        .expect_err("update_campaign must fail when campaigns.json is fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert_eq!(
        store.get_campaign(&campaign_id).unwrap().name,
        "atomic",
        "in-memory campaign must be unchanged after persist failure"
    );
    let reloaded = CampaignStore::new(dir.path());
    assert_eq!(
        reloaded.get_campaign(&campaign_id).unwrap().name,
        "atomic",
        "on-disk campaign must be unchanged after persist failure"
    );
    storyforge_infra_util::write_fence::unfreeze(&campaigns_path);

    // add_instance：候选 → 持久化 → 换入。
    let instances_path = dir.path().join("instances.json");
    storyforge_infra_util::write_fence::freeze(&instances_path);
    let extra = CharacterInstance::temporary(campaign_id.clone(), "Ghost");
    let error = store
        .add_instance(extra)
        .expect_err("add_instance must fail when instances.json is fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert_eq!(
        store.list_instances(&campaign_id).len(),
        1,
        "in-memory instances unchanged"
    );
    assert_eq!(
        reload_campaigns(dir.path()).len(),
        1,
        "campaigns intact on disk"
    );
    storyforge_infra_util::write_fence::unfreeze(&instances_path);

    // save_card / delete_knowledge / add_task / delete_task：同款语义抽查。
    let knowledge_path = dir.path().join("knowledge.json");
    let knowledge = CharacterKnowledgeEntry {
        id: Id::from_str("k-1"),
        campaign_id: campaign_id.clone(),
        character_id: Id::from_str("inst-1"),
        knowledge_text: "刀".into(),
        source: storyforge_domain::character_knowledge::KnowledgeSource::Witnessed,
        source_character_id: None,
        source_knowledge_id: None,
        turn_number: 1,
        event_id: None,
        pinned: false,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    store.add_knowledge(vec![knowledge.clone()]).unwrap();
    storyforge_infra_util::write_fence::freeze(&knowledge_path);
    let error = store
        .delete_knowledge(&knowledge.id)
        .expect_err("delete_knowledge must fail when knowledge.json is fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert_eq!(
        store.list_knowledge(&campaign_id).len(),
        1,
        "in-memory knowledge unchanged"
    );
    storyforge_infra_util::write_fence::unfreeze(&knowledge_path);

    let _ = dir;
}

#[test]
fn json_character_store_mutators_rollback_on_persist_failure() {
    let dir = temp_dir("character-mutators");
    let state = json_state(dir.path());
    let saved = state
        .storage()
        .save_character(sample_character_info("原角色"))
        .unwrap();

    let chars_path = dir.path().join("characters.json");
    // 先加世界书条目（此时未冻结），再冻结验证编辑路径的回滚。
    let idx = state
        .storage()
        .add_character_world_info_entry(
            &saved.id,
            vec!["key".into()],
            "content".into(),
            false,
            false,
        )
        .expect("add entry before fence");
    assert_eq!(idx, 0);
    storyforge_infra_util::write_fence::freeze(&chars_path);
    // save：候选 → 持久化 → 换入。
    let error = state
        .storage()
        .save_character(sample_character_info("不得落盘"))
        .expect_err("save must fail when characters.json is fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert_eq!(
        state.storage().list_characters().unwrap().len(),
        1,
        "in-memory characters unchanged"
    );
    // delete：候选 → 持久化 → 换入。
    let error = state
        .storage()
        .delete_character(&saved.id)
        .expect_err("delete must fail when characters.json is fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert_eq!(
        state.storage().list_characters().unwrap().len(),
        1,
        "in-memory characters unchanged after failed delete"
    );
    // 世界书条目编辑：候选 → 持久化 → 换入。
    let error = state
        .storage()
        .update_character_world_info_route(&saved.id, 0, "Disabled")
        .expect_err("update_world_info_route must fail when fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert_eq!(
        state
            .storage()
            .get_character(&saved.id)
            .unwrap()
            .unwrap()
            .info
            .world_info_entries[0]
            .route,
        "Selective",
        "in-memory route unchanged after persist failure"
    );
    storyforge_infra_util::write_fence::unfreeze(&chars_path);
    // 磁盘也不变（重启后只有种子角色，且路由未被改写）。
    assert_eq!(state.storage().list_characters().unwrap().len(), 1);
    let _ = dir;
}

#[test]
fn json_turn_store_mutators_rollback_on_persist_failure() {
    let dir = temp_dir("turn-mutators");
    let campaign_id = Id::from_str("camp-1");
    let conversation_id = Id::from_str("conv-1");
    let turns_path = dir.path().join("turns.json");
    storyforge_infra_util::write_fence::freeze(&turns_path);

    // create_turn：候选 → 持久化 → 换入。
    let store = TurnStore::new(dir.path());
    let record = active_turn_record(&campaign_id, &conversation_id);
    let turn_id = record.turn_id.clone();
    let error = store
        .create_turn(record)
        .expect_err("create_turn must fail when turns.json is fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert!(
        store.get_turn(&turn_id).is_none(),
        "in-memory turn must not appear after persist failure"
    );
    assert!(
        TurnStore::new(dir.path()).get_turn(&turn_id).is_none(),
        "on-disk turns.json must be unchanged after persist failure"
    );
    storyforge_infra_util::write_fence::unfreeze(&turns_path);

    // save_turn：候选 → 持久化 → 换入。
    let store = TurnStore::new(dir.path());
    let record = active_turn_record(&campaign_id, &conversation_id);
    let turn_id = record.turn_id.clone();
    store.create_turn(record.clone()).unwrap();
    storyforge_infra_util::write_fence::freeze(&turns_path);
    let mut edited = record;
    edited.status = TurnStatus::DraftReady;
    let error = store
        .save_turn(edited.clone())
        .expect_err("save_turn must fail when turns.json is fenced");
    assert!(error.contains("持久化"), "error: {error}");
    assert_eq!(
        store.get_turn(&turn_id).unwrap().status,
        TurnStatus::Generating,
        "in-memory turn unchanged after persist failure"
    );
    storyforge_infra_util::write_fence::unfreeze(&turns_path);
    let _ = dir;
}

// ─── 三.5：活动 Turn TOCTOU → 原子空闲修改 ────────────────────────────────

#[test]
fn json_idle_variable_write_rolls_back_on_persist_failure() {
    let dir = temp_dir("idle-var-freeze");
    let state = json_state(dir.path());
    let card = make_card("card-1", false);
    state.storage().save_card(card.clone()).unwrap();
    let campaign = make_campaign(&card.id, "idle-freeze");
    let campaign_id = campaign.id.clone();
    state
        .storage()
        .create_campaign_with_instances(campaign)
        .unwrap();

    // 冻结 campaigns.json：写变量命令必须 Err，且内存 + 磁盘都保持原值
    // （三.5 原子单元：turns 锁守卫内 candidate→persist→swap）。
    let campaigns_path = dir.path().join("campaigns.json");
    storyforge_infra_util::write_fence::freeze(&campaigns_path);
    let error = storyforge_lib::set_campaign_variable(
        campaign_id.as_str().to_string(),
        "custom_key".into(),
        serde_json::json!("Day 9"),
        None,
        to_state(&state),
    )
    .expect_err("set_campaign_variable must fail when campaigns.json is fenced");
    assert!(error.to_string().contains("持久化失败"), "error: {error}");
    let after = state
        .storage()
        .get_campaign(&campaign_id)
        .unwrap()
        .unwrap()
        .campaign;
    assert_eq!(
        after.get_variable("custom_key"),
        None,
        "in-memory campaign variable must be unchanged after persist failure"
    );
    storyforge_infra_util::write_fence::unfreeze(&campaigns_path);
    let reloaded = CampaignStore::new(dir.path());
    let after_disk = reloaded.get_campaign(&campaign_id).unwrap();
    assert_eq!(
        after_disk.get_variable("custom_key"),
        None,
        "on-disk campaign variable must be unchanged after persist failure"
    );
    let _ = dir;
}

#[test]
fn json_idle_variable_write_rejected_when_active_turn() {
    let dir = temp_dir("idle-var-active");
    let state = json_state(dir.path());
    let card = make_card("card-1", false);
    state.storage().save_card(card.clone()).unwrap();
    let campaign = make_campaign(&card.id, "idle-active");
    let campaign_id = campaign.id.clone();
    state
        .storage()
        .create_campaign_with_instances(campaign)
        .unwrap();

    // 活动 Turn 存在：写变量命令必须被屏障拒绝且不产生任何写入。
    let conversation = state.conv_store.create(None, Some(campaign_id.clone()));
    state
        .storage()
        .json_turn_store("seed active turn")
        .unwrap()
        .create_turn(active_turn_record(&campaign_id, &conversation.id))
        .unwrap();

    let error = storyforge_lib::set_campaign_variable(
        campaign_id.as_str().to_string(),
        "custom_key".into(),
        serde_json::json!("Day 1"),
        None,
        to_state(&state),
    )
    .expect_err("set_campaign_variable must be rejected while an active turn exists");
    assert!(error.to_string().contains("未完成的轮次"), "error: {error}");
    let after = state
        .storage()
        .get_campaign(&campaign_id)
        .unwrap()
        .unwrap()
        .campaign;
    assert_eq!(after.get_variable("custom_key"), None);
    assert!(
        state
            .storage()
            .get_active_turn(&campaign_id)
            .unwrap()
            .is_some(),
        "active turn must still be present"
    );
    let _ = dir;
}

#[test]
fn json_idle_mutation_concurrent_no_lost_update() {
    // 旧实现「先查活动 Turn、再单独 get→改→save」在并发下会丢失更新：
    // A 读到旧 Campaign → B 读到旧 Campaign → A 写回 → B 写回覆盖 A。
    // 新实现检查 + 读改写 + 写盘在同一 turns 锁守卫内串行化，两条写入都保留。
    let dir = temp_dir("idle-var-race");
    let state = Arc::new(json_state(dir.path()));
    let card = make_card("card-1", false);
    state.storage().save_card(card.clone()).unwrap();
    let campaign = make_campaign(&card.id, "idle-race");
    let campaign_id = campaign.id.clone();
    state
        .storage()
        .create_campaign_with_instances(campaign)
        .unwrap();

    for round in 0..80 {
        let key_a = format!("race_a_{round}");
        let key_b = format!("race_b_{round}");
        std::thread::scope(|scope| {
            let state_a = state.clone();
            let state_b = state.clone();
            let cid_a = campaign_id.clone();
            let cid_b = campaign_id.clone();
            let ka = key_a.clone();
            let kb = key_b.clone();
            scope.spawn(move || {
                storyforge_lib::set_campaign_variable(
                    cid_a.as_str().to_string(),
                    ka,
                    serde_json::json!(round),
                    None,
                    to_state(&state_a),
                )
                .expect("thread A write must succeed");
            });
            scope.spawn(move || {
                storyforge_lib::set_campaign_variable(
                    cid_b.as_str().to_string(),
                    kb,
                    serde_json::json!(round),
                    None,
                    to_state(&state_b),
                )
                .expect("thread B write must succeed");
            });
        });
        let after = state
            .storage()
            .get_campaign(&campaign_id)
            .unwrap()
            .unwrap()
            .campaign;
        assert!(
            after.get_variable(&key_a).is_some(),
            "round {round}: variable {key_a} lost (lost update)"
        );
        assert!(
            after.get_variable(&key_b).is_some(),
            "round {round}: variable {key_b} lost (lost update)"
        );
    }
    let _ = dir;
}

#[test]
fn json_idle_instance_and_task_commands_rejected_atomically_with_active_turn() {
    // 三.5 逐路径：add_campaign_instance / create_task / complete_task /
    // abandon_task 的活动 Turn 屏障 + 写入必须在同一原子单元内（旧实现
    // reject_if_active_turn 检查后单独写盘）。本测试断言：活动 Turn 存在时
    // 命令被拒且**不产生任何写入**（实例数 / 任务数不变）。
    let dir = temp_dir("idle-inst-task");
    let state = json_state(dir.path());
    let card = make_card("card-1", true);
    state.storage().save_card(card.clone()).unwrap();
    let campaign = make_campaign(&card.id, "idle-inst-task");
    let campaign_id = campaign.id.clone();
    state
        .storage()
        .create_campaign_with_instances(campaign)
        .unwrap();
    let conversation = state.conv_store.create(None, Some(campaign_id.clone()));
    state
        .storage()
        .json_turn_store("seed active turn")
        .unwrap()
        .create_turn(active_turn_record(&campaign_id, &conversation.id))
        .unwrap();
    let instances_before = state.storage().list_instances(&campaign_id).unwrap().len();

    // add_campaign_instance：被拒 + 实例数不变。
    let error = storyforge_lib::add_campaign_instance(
        campaign_id.as_str().to_string(),
        None,
        Some("新角色".into()),
        None,
        None,
        to_state(&state),
    )
    .expect_err("add_campaign_instance must be rejected while an active turn exists");
    assert!(error.to_string().contains("未完成的轮次"), "error: {error}");
    assert_eq!(
        state.storage().list_instances(&campaign_id).unwrap().len(),
        instances_before,
        "no instance may be added while the active turn exists"
    );

    // create_task：被拒 + 任务数不变。
    let error = storyforge_lib::create_task(
        campaign_id.as_str().to_string(),
        "伏笔".to_string(),
        "复仇".to_string(),
        vec![],
        None,
        to_state(&state),
    )
    .expect_err("create_task must be rejected while an active turn exists");
    assert!(error.to_string().contains("未完成的轮次"), "error: {error}");
    assert!(
        state.storage().list_tasks(&campaign_id).unwrap().is_empty(),
        "no task may be created while the active turn exists"
    );
    let _ = dir;
}

// ─── 三.6：create / fork Campaign bundle 原子性 ────────────────────────────

#[test]
fn json_create_campaign_bundle_cleanup_on_campaign_persist_failure() {
    let dir = temp_dir("create-bundle-campaigns");
    let state = json_state(dir.path());
    let card = make_card("card-1", true);
    state.storage().save_card(card.clone()).unwrap();

    // 冻结 campaigns.json：campaign 落盘失败 → 补偿删除会话；不得留下
    // 「有会话无 Campaign」的孤儿。实例文件（instances.json 未冻结）也要恢复。
    let campaigns_path = dir.path().join("campaigns.json");
    storyforge_infra_util::write_fence::freeze(&campaigns_path);
    let error = storyforge_lib::create_campaign(
        card.id.as_str().to_string(),
        "bundle-fail".into(),
        None,
        to_state(&state),
    )
    .expect_err("create_campaign must fail when campaigns.json is fenced");
    assert!(error.to_string().contains("存储写入失败"), "error: {error}");
    storyforge_infra_util::write_fence::unfreeze(&campaigns_path);

    // 内存：无 Campaign、无会话。
    assert!(
        state.storage().list_campaigns(None).unwrap().is_empty(),
        "no campaign may survive the failed bundle"
    );
    assert!(
        state.conv_store.list().is_empty(),
        "no orphan conversation may survive the failed bundle"
    );
    // 重启（全新 store，同一目录）：一致——无 Campaign、无会话、实例文件原样。
    assert!(reload_campaigns(dir.path()).is_empty());
    assert_eq!(reload_conversation_count(dir.path()), 0);
    let _ = dir;
}

#[test]
fn json_create_campaign_bundle_cleanup_on_world_info_failure() {
    let dir = temp_dir("create-bundle-worldinfo");
    let state = json_state(dir.path());
    let card = make_card("card-1", true);
    state.storage().save_card(card.clone()).unwrap();

    // 把 campaign_world_info 目录位占成普通文件：开档种子世界书阶段
    // create_dir_all 失败（确定性注入）。旧实现 `tracing::warn!` 吞掉该错误，
    // 返回 Ok 留下「有 Campaign 无世界书」的半成品；新实现必须 Err + 全清理。
    let world_info_dir = dir.path().join("campaign_world_info");
    std::fs::remove_dir_all(&world_info_dir).ok();
    std::fs::write(&world_info_dir, b"blocked").unwrap();

    let error = storyforge_lib::create_campaign(
        card.id.as_str().to_string(),
        "bundle-wi-fail".into(),
        None,
        to_state(&state),
    )
    .expect_err("create_campaign must fail when world-info seeding fails");
    assert!(error.to_string().contains("存储写入失败"), "error: {error}");

    assert!(
        state.storage().list_campaigns(None).unwrap().is_empty(),
        "no campaign may survive the failed bundle"
    );
    assert!(
        state.conv_store.list().is_empty(),
        "no orphan conversation may survive the failed bundle"
    );
    assert!(reload_campaigns(dir.path()).is_empty());
    assert_eq!(reload_conversation_count(dir.path()), 0);
    let _ = dir;
}

#[test]
fn json_fork_campaign_bundle_cleanup_on_save_failure() {
    let dir = temp_dir("fork-bundle");
    let state = json_state(dir.path());
    let card = make_card("card-1", true);
    state.storage().save_card(card.clone()).unwrap();
    let source = make_campaign(&card.id, "source");
    let source_id = source.id.clone();
    state
        .storage()
        .create_campaign_with_instances(source)
        .unwrap();
    // 源活动绑定会话 + 世界书（fork 拷贝源）。
    let conversation = state.conv_store.create(None, Some(source_id.clone()));
    let fork_node_id = state
        .conv_store
        .append_user_message(&conversation.id, "源消息".to_string())
        .unwrap();
    let mut bound = state
        .storage()
        .get_campaign(&source_id)
        .unwrap()
        .unwrap()
        .campaign;
    bound.conversation_id = Some(conversation.id.clone());
    state.storage().update_campaign(&bound).unwrap();
    state
        .storage()
        .set_world_info(&source_id, &book_with_entries())
        .unwrap();
    state.conv_store.invalidate();

    // 冻结 campaigns.json：fork Campaign 落盘失败 → 旧实现留下孤儿 fork 会话
    // （fork_at 已持久化）；新实现补偿删除 fork 会话 + fork Campaign。
    let campaigns_path = dir.path().join("campaigns.json");
    storyforge_infra_util::write_fence::freeze(&campaigns_path);
    let error = storyforge_lib::fork_campaign(
        source_id.as_str().to_string(),
        fork_node_id.as_str().to_string(),
        "fork-bundle-fail".into(),
        to_state(&state),
    )
    .expect_err("fork_campaign must fail when campaigns.json is fenced");
    assert!(error.to_string().contains("存储写入失败"), "error: {error}");
    storyforge_infra_util::write_fence::unfreeze(&campaigns_path);

    // 无孤儿 fork：会话列表只剩源会话；Campaign 列表只剩源 Campaign。
    let conv_ids: Vec<Id> = state.conv_store.list().into_iter().map(|s| s.id).collect();
    assert_eq!(
        conv_ids,
        vec![conversation.id.clone()],
        "fork conversation must be cleaned up; only the source conversation may remain"
    );
    let campaigns = state.storage().list_campaigns(None).unwrap();
    assert_eq!(campaigns.len(), 1, "only the source campaign may remain");
    assert_eq!(campaigns[0].campaign.id, source_id);
    // 重启一致。
    assert_eq!(reload_campaigns(dir.path()).len(), 1);
    assert_eq!(reload_conversation_count(dir.path()), 1);
    let _ = dir;
}

// ─── 三.7：delete_card 双后端完整等价（JSON 侧）───────────────────────────

/// 构造「全量卡」：卡 + Campaign + 会话 + Turn + 实例 + 知识 + 任务 + 摘要 +
/// 世界书 + 压缩任务 + 活跃指针。
fn seed_full_card(state: &Arc<AppState>, _dir: &Path, label: &str) -> (Id, Id, Id, Id) {
    let card = make_card(label, true);
    state.storage().save_card(card.clone()).unwrap();
    let campaign = make_campaign(&card.id, label);
    let campaign_id = campaign.id.clone();
    state
        .storage()
        .create_campaign_with_instances(campaign.clone())
        .unwrap();

    // 会话 + 绑定。
    let conversation = state.conv_store.create(None, Some(campaign_id.clone()));
    let mut bound = state
        .storage()
        .get_campaign(&campaign_id)
        .unwrap()
        .unwrap()
        .campaign;
    bound.conversation_id = Some(conversation.id.clone());
    state.storage().update_campaign(&bound).unwrap();
    state.conv_store.invalidate();

    // Turn（终态，避免活动屏障干扰删除路径）；必须经 facade 的 TurnStore
    // 创建（同一进程内实例，delete_card 的前置清理才会看到它）。
    let mut record = active_turn_record(&campaign_id, &conversation.id);
    record.status = TurnStatus::Committed;
    state
        .storage()
        .json_turn_store("seed turn")
        .unwrap()
        .create_turn(record)
        .unwrap();

    // 实例（卡定义之外再加一个临时角色）。
    let instance = CharacterInstance::temporary(campaign_id.clone(), "临时角色");
    state.storage().add_instance(&instance).unwrap();

    // 知识 / 任务。
    let knowledge = CharacterKnowledgeEntry {
        id: Id::from_str(format!("k-{label}")),
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
    state.storage().add_knowledge(&[knowledge]).unwrap();
    let task = StoryTask::user_planned(
        campaign_id.clone(),
        "伏笔".to_string(),
        "复仇".to_string(),
        vec![],
        1,
    );
    state.storage().add_task(&task).unwrap();

    // 摘要（JSON 直连 store）。
    let summary = storyforge_domain::agent::RoundSummary::new(
        campaign_id.clone(),
        conversation.id.clone(),
        1,
        "本轮纪要".into(),
    );
    state
        .storage()
        .json_campaign_store(
            storyforge_lib::storage_backend::BackendCapability::CampaignRead,
            "seed summary",
        )
        .unwrap()
        .add_summary(summary)
        .unwrap();

    // 世界书。
    state
        .storage()
        .set_world_info(&campaign_id, &book_with_entries())
        .unwrap();

    // 压缩任务。
    state
        .storage()
        .enqueue_compress_job(&campaign_id, Some(conversation.id.clone()), None, 200, 0)
        .unwrap();

    // 活跃指针。
    *state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(campaign_id.clone());
    state
        .storage()
        .save_active_pointer(Some(&campaign_id))
        .unwrap();

    (
        card.id.clone(),
        campaign_id,
        conversation.id.clone(),
        instance.id.clone(),
    )
}

#[test]
fn json_delete_card_cascades_all_associated_data() {
    let dir = temp_dir("delete-card");
    let state = json_state(dir.path());
    let (card_id, campaign_id, conversation_id, instance_id) =
        seed_full_card(&state, dir.path(), "full");

    storyforge_lib::delete_card(card_id.as_str().to_string(), to_state(&state))
        .expect("delete_card must succeed");

    // 卡 / Campaign / 实例 / 知识 / 任务 / 摘要 / MVU。
    assert!(state.storage().list_cards().unwrap().is_empty());
    assert!(
        state
            .storage()
            .get_campaign(&campaign_id)
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .storage()
            .list_instances(&campaign_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .storage()
            .list_knowledge(&campaign_id)
            .unwrap()
            .is_empty()
    );
    assert!(state.storage().list_tasks(&campaign_id).unwrap().is_empty());
    assert!(
        state
            .storage()
            .list_summaries(&campaign_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .storage()
            .get_instance(&campaign_id, &instance_id)
            .unwrap()
            .is_none()
    );

    // 会话 / Turn / 压缩任务。
    assert!(state.conv_store.get(&conversation_id).is_none());
    assert_eq!(
        state
            .storage()
            .json_turn_store("assert turns gone")
            .unwrap()
            .list_all()
            .len(),
        0,
        "all turns of the card's campaigns must be deleted"
    );
    assert!(
        state.storage().list_compress_jobs().unwrap().is_empty(),
        "compress jobs of the card's campaigns must be deleted"
    );

    // 世界书文件（campaign_world_info/{campaign_id}.json 必须消失）。
    assert!(
        !dir.path()
            .join("campaign_world_info")
            .join(format!("{}.json", campaign_id.as_str()))
            .exists()
    );

    // 活跃指针 + 会话缓存 + tool_ctx。
    assert!(
        state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none(),
        "active pointer must be cleared when the deleted card owned the active campaign"
    );
    assert!(
        state.conv_store.list().is_empty(),
        "conversation cache must be invalidated"
    );

    // 重启一致：全新 store 看同一目录，全部关联数据消失。
    assert!(reload_campaigns(dir.path()).is_empty());
    assert_eq!(reload_conversation_count(dir.path()), 0);
    assert!(TurnStore::new(dir.path()).list_all().is_empty());
    let _ = dir;
}

#[test]
fn json_delete_card_mid_delete_failure_rolls_back_everything() {
    let dir = temp_dir("delete-card-rollback");
    let state = json_state(dir.path());
    let (card_id, campaign_id, _conversation_id, instance_id) =
        seed_full_card(&state, dir.path(), "rollback");

    // 冻结 instances.json：delete_card 级联写到 instances 文件时失败 →
    // 快照补偿整体回滚：卡 + Campaign + 知识 + 任务 + 摘要 + 世界书全部保留。
    let instances_path = dir.path().join("instances.json");
    storyforge_infra_util::write_fence::freeze(&instances_path);
    let error = state
        .storage()
        .delete_card(&card_id)
        .expect_err("delete_card must fail when instances.json is fenced mid-cascade");
    assert!(error.contains("持久化"), "error: {error}");
    storyforge_infra_util::write_fence::unfreeze(&instances_path);

    // 内存 + 磁盘全部原样（无半删状态）。
    assert_eq!(state.storage().list_cards().unwrap().len(), 1);
    assert!(
        state
            .storage()
            .get_campaign(&campaign_id)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        state.storage().list_instances(&campaign_id).unwrap().len(),
        2
    );
    assert_eq!(
        state.storage().list_knowledge(&campaign_id).unwrap().len(),
        1
    );
    assert_eq!(state.storage().list_tasks(&campaign_id).unwrap().len(), 1);
    assert_eq!(
        state.storage().list_summaries(&campaign_id).unwrap().len(),
        1
    );
    assert!(
        state
            .storage()
            .get_world_info(&campaign_id)
            .unwrap()
            .entries
            .len()
            == 1,
        "world info must survive the failed delete"
    );
    assert!(
        state
            .storage()
            .get_instance(&campaign_id, &instance_id)
            .unwrap()
            .is_some()
    );

    // 重启一致：同一目录全新 store 看到完整数据。
    let reloaded = CampaignStore::new(dir.path());
    assert_eq!(reloaded.list_cards().len(), 1);
    assert_eq!(reloaded.list_campaigns().len(), 1);
    assert_eq!(reloaded.list_instances(&campaign_id).len(), 2);
    let _ = dir;
}

// ─── 三审5：delete_card 命令级跨边界原子（前置 + 聚合整体原子）────────────
//
// 判别性：经命令入口 storyforge_lib::delete_card（前置删会话/Turn/压缩任务 →
// 聚合 delete_card）执行；注入聚合写盘失败（冻结 cards.json）。旧实现：前置
// 已删、聚合失败 → 会话/Turn 文件丢失（半状态）。新实现：快照恢复，重启后
// 卡 + Campaign + 会话 + Turn + 任务全部原样。

#[test]
fn json_delete_card_command_precursor_plus_aggregate_atomic_on_failure() {
    let dir = temp_dir("delete-card-command-atomic");
    let state = json_state(dir.path());
    let (card_id, campaign_id, conversation_id, _instance_id) =
        seed_full_card(&state, dir.path(), "cmd-atomic");

    // 删除前的基线：会话 + Turn 文件存在。
    let conv_path = dir
        .path()
        .join("conversations")
        .join(format!("{conversation_id}.json"));
    let turns_path = dir.path().join("turns.json");
    assert!(conv_path.exists(), "会话文件删除前必须存在");
    assert!(turns_path.exists(), "turns.json 删除前必须存在");
    let turns_before = std::fs::read(&turns_path).unwrap();

    // 冻结 cards.json：命令的聚合 delete_card 写 cards.json 时失败。
    // （前置已先删会话/Turn/压缩任务——旧实现会留下半状态；新实现快照恢复。）
    let cards_path = dir.path().join("cards.json");
    storyforge_infra_util::write_fence::freeze(&cards_path);
    let error = storyforge_lib::delete_card(card_id.as_str().to_string(), to_state(&state))
        .expect_err("delete_card command must fail when cards.json is fenced");
    assert!(
        error.to_string().contains("存储写入失败"),
        "error must report storage failure: {error}"
    );
    storyforge_infra_util::write_fence::unfreeze(&cards_path);

    // 重启一致（全新 store，同目录）：卡 + Campaign + 会话 + Turn 全部原样。
    // 三审5 判别点：旧实现会话/Turn 文件已被前置删除且未恢复 → 这里读不到。
    let reloaded = CampaignStore::new(dir.path());
    assert_eq!(
        reloaded.list_cards().len(),
        1,
        "card must survive the failed command-level delete (snapshot restored)"
    );
    assert_eq!(
        reloaded.list_campaigns().len(),
        1,
        "campaign must survive the failed command-level delete"
    );
    assert_eq!(
        reload_conversation_count(dir.path()),
        1,
        "conversation file must be restored after failed aggregate (precursor atomicity)"
    );
    // turns.json 字节不变（前置删除被快照恢复）。
    let turns_after = std::fs::read(&turns_path).unwrap();
    assert_eq!(
        turns_before, turns_after,
        "turns.json bytes must be unchanged after failed command-level delete"
    );
    let _ = campaign_id;
    let _ = dir;
}

// ─── 四.2：JSON MutationBatch 最终持久化状态 == "committed" ───────────────

#[test]
fn json_accept_persists_mutation_batch_committed() {
    use storyforge_lib::turn_lifecycle::TurnLifecycleService;

    let dir = temp_dir("batch-committed");
    let campaign_store = Arc::new(CampaignStore::new(dir.path()));
    let turn_store = Arc::new(TurnStore::new(dir.path()));
    let conv_store = Arc::new(storyforge_app_conversation::ConversationStore::new(
        dir.path().join("conversations"),
    ));

    let mut campaign = Campaign::new(Id::new(), "batch status");
    campaign.lineage_id = Some(Id::new());
    let campaign_id = campaign.id.clone();
    campaign_store.save_campaign(campaign).unwrap();
    let conversation = conv_store.create(None, None);
    let conversation_id = conversation.id.clone();
    let variant_id = conv_store
        .append_ai_draft(&conversation_id, "正文".to_string(), None)
        .unwrap();
    let mut bound = campaign_store.get_campaign(&campaign_id).unwrap();
    bound.conversation_id = Some(conversation_id.clone());
    campaign_store.update_campaign(bound).unwrap();

    // AwaitingAcceptance Turn + Prepared 候选 batch（write-ahead journal 语义）。
    let camp = campaign_store.get_campaign(&campaign_id).unwrap();
    let mut batch = MutationBatch::new(Id::new(), camp.revision);
    batch
        .mutations
        .push(storyforge_domain::turn::Mutation::FinalizeVariant {
            variant_id: variant_id.clone(),
        });
    batch
        .mutations
        .push(storyforge_domain::turn::Mutation::SetVariable {
            instance_id: None,
            key: "story_clock".into(),
            value: serde_json::json!("Day 1"),
            turn: 1,
        });
    let attempt = storyforge_domain::turn::TurnAttempt {
        attempt_id: Id::new(),
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
        Id::from_str("input-node"),
        0,
    );
    record.status = TurnStatus::AwaitingAcceptance;
    record.attempts.push(attempt);
    turn_store.create_turn(record).unwrap();

    let service = TurnLifecycleService::new(
        campaign_store.as_ref(),
        turn_store.as_ref(),
        conv_store.as_ref(),
    );
    service
        .accept_by_variant(&campaign_id, &conversation_id, &variant_id, false)
        .expect("JSON accept must succeed");

    // 原样读取 turns.json：提交后的 batch status 必须 == "committed"
    // （与 SQLite `accept_turn` 持久化语义一致；旧实现永久落 "prepared"）。
    let raw = std::fs::read_to_string(dir.path().join("turns.json")).unwrap();
    let turns: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let turns = turns.as_array().expect("turns.json must be an array");
    assert_eq!(turns.len(), 1);
    let attempts = turns[0]["attempts"].as_array().expect("attempts array");
    let batch = &attempts[0]["pending_state_changes"];
    assert!(
        !batch.is_null(),
        "committed turn must retain its persisted MutationBatch"
    );
    assert_eq!(
        batch["status"].as_str(),
        Some("committed"),
        "JSON persisted MutationBatch status must be 'committed' after accept (raw turns.json: {raw})"
    );
    assert_ne!(
        batch["status"].as_str(),
        Some("prepared"),
        "JSON persisted MutationBatch must not remain 'prepared' after a successful commit"
    );
    // 内存侧同样为 Committed。
    let persisted_turn = turn_store
        .list_all()
        .into_iter()
        .next()
        .expect("turn persisted");
    let persisted_batch = persisted_turn
        .attempts
        .iter()
        .find(|a| a.variant_id == variant_id)
        .and_then(|a| a.pending_state_changes.as_ref())
        .expect("batch must be retained");
    assert_eq!(
        persisted_batch.status,
        storyforge_domain::turn::MutationBatchStatus::Committed
    );
    let _ = dir;
}

use super::super::*;
use crate::storage_backend::{BackendCapability, CapabilityStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignSummaryDto {
    pub id: String,
    pub card_id: String,
    pub name: String,
    pub created_at: String,
    pub story_clock: String,
    pub instance_count: usize,
    pub fork_from: Option<(String, String)>,
    /// 绑定的对话 ID（一 Campaign 一对话）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

impl From<&storyforge_domain::campaign::Campaign> for CampaignSummaryDto {
    fn from(c: &storyforge_domain::campaign::Campaign) -> Self {
        Self {
            id: c.id.as_str().to_string(),
            card_id: c.card_id.as_str().to_string(),
            name: c.name.clone(),
            created_at: c.created_at.clone(),
            story_clock: c.current_story_clock().to_string(),
            instance_count: 0, // 调用方填
            fork_from: c
                .fork_from
                .as_ref()
                .map(|(cid, nid)| (cid.as_str().to_string(), nid.as_str().to_string())),
            conversation_id: c.conversation_id.as_ref().map(|id| id.as_str().to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInstanceDto {
    pub id: String,
    pub campaign_id: String,
    pub definition_id: Option<String>,
    pub name: String,
    /// Linked card definition role. Ad-hoc temporary instances use `extra`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_type: Option<String>,
    pub persona_override: Option<String>,
    pub behavior_override: Option<String>,
    pub is_temporary: bool,
    pub variables: Vec<storyforge_domain::variables::VariableValue>,
}

impl From<&storyforge_domain::campaign::CharacterInstance> for CharacterInstanceDto {
    fn from(i: &storyforge_domain::campaign::CharacterInstance) -> Self {
        Self {
            id: i.id.as_str().to_string(),
            campaign_id: i.campaign_id.as_str().to_string(),
            definition_id: i.definition_id.as_ref().map(|d| d.as_str().to_string()),
            name: i.name.clone(),
            role_type: i.is_temporary.then(|| "extra".to_string()),
            persona_override: i.persona_override.clone(),
            behavior_override: i.behavior_override.clone(),
            is_temporary: i.is_temporary,
            variables: i.variables.clone(),
        }
    }
}

impl CharacterInstanceDto {
    pub(crate) fn with_role_type(
        instance: &storyforge_domain::campaign::CharacterInstance,
        role_type: &storyforge_domain::character::RoleType,
    ) -> Self {
        let mut dto = Self::from(instance);
        dto.role_type = Some(format!("{role_type:?}").to_lowercase());
        dto
    }
}

/// 跑角色识别 Agent，为已导入的扁平 Character 建 CharacterCard
///
/// 失败时降级：建单角色 Protagonist definition（卡仍可用）。
#[tauri::command]
pub(crate) async fn extract_characters(
    source_character_id: String,
    force: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardSummaryDto, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;
    use storyforge_domain::character::CharacterExtractionStatus;
    use storyforge_domain::variables::{
        extract_campaign_variable_schema_from_extensions, extract_mvu_schema_from_extensions,
    };

    let character_store = state.json_character_store(
        BackendCapability::CharacterCommands,
        "extract characters from imported character card",
    )?;

    // 取原 Character（从 tool_ctx，启动恢复 + import_character 都同步过）
    let character = {
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        // 1. 直接按 Character.id 查（前端 extractCharacters 传卡本身 id 时命中）
        let direct = ctx
            .characters
            .iter()
            .find(|c| c.id.as_str() == source_character_id)
            .map(|c| (*c).clone());
        match direct {
            Some(c) => Some(c),
            None => {
                // 2. 回退：前端可能传了存储 id（import_character 返回的 StoredCharacter.id，
                //    与 Character.id 无关）。用存储 id 查 CharacterStore 拿到 name，再按 name 查 tool_ctx。
                character_store
                    .get(&source_character_id)
                    .and_then(|stored| {
                        ctx.characters
                            .iter()
                            .find(|c| c.name == stored.info.name)
                            .map(|c| (*c).clone())
                    })
            }
        }
    }
    .ok_or_else(|| {
        TauriCommandError::not_found(format!(
            "找不到 source_character_id={source_character_id} 的角色卡"
        ))
    })?;

    // 已存在则直接返回
    let store = state
        .storage()
        .json_campaign_store_owned(BackendCapability::CardCommands, "extract characters")
        .map_err(TauriCommandError::validation)?;
    let force = force.unwrap_or(false);
    let mut card = match prepare_character_extraction_card(store.as_ref(), &character, force)? {
        CharacterExtractionDecision::ReturnExisting(existing) => {
            return Ok(CardSummaryDto::from(&existing));
        }
        CharacterExtractionDecision::Run(card) => card,
    };

    // MVU schema 探测
    let mvu_schema = extract_mvu_schema_from_extensions(&character.extensions);
    card.campaign_variable_schema =
        extract_campaign_variable_schema_from_extensions(&character.extensions);
    if !mvu_schema.is_empty() {
        tracing::info!(
            "卡「{}」探测到 {} 个 MVU 字段",
            character.name,
            mvu_schema.len()
        );
    }

    // 有真实连接时跑识别 Agent；无连接或调用失败时安全降级为源卡自身的单角色定义。
    // 生产态绝不使用开发 Mock，避免示例人物污染持久化数据。
    let (definitions, extraction_status, extraction_message) = match state.active_llm() {
        Some(llm) => {
            let tool_ctx = state.snapshot_tool_ctx();
            let runtime = AgentRuntime::new(llm, tool_ctx);
            let (_cancel_tx, cancel_rx) = watch::channel(false);
            match storyforge_app_agent::extract_characters(
                &runtime,
                &character,
                &mvu_schema,
                cancel_rx,
            )
            .await
            {
                Ok(defs) => (defs, CharacterExtractionStatus::Extracted, None),
                Err(error) => {
                    tracing::warn!("角色识别失败，降级建单角色: {error}");
                    fallback_character_extraction(&character, &mvu_schema)
                }
            }
        }
        None => {
            tracing::info!(
                "未配置活跃 LLM，角色识别按源卡「{}」安全降级",
                character.name
            );
            fallback_character_extraction(&character, &mvu_schema)
        }
    };

    // 建卡 + 回填 card_id
    let definitions = storyforge_app_agent::attach_definitions_to_card(definitions, &card.id);
    card.character_definitions = definitions;
    card.extraction_status = extraction_status;
    card.extraction_message = extraction_message;
    let stored = if force {
        save_character_card_force_rerun_async(store, card).await?
    } else {
        save_character_card_async(store, card).await?
    };

    Ok(CardSummaryDto::from(&stored))
}

pub(crate) async fn save_character_card_async(
    store: Arc<campaign_store::CampaignStore>,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    tokio::task::spawn_blocking(move || save_character_card_to_store(store.as_ref(), card))
        .await
        .map_err(|e| TauriCommandError::internal(format!("保存角色卡任务失败: {e}")))?
}

pub(crate) fn save_character_card_to_store(
    store: &campaign_store::CampaignStore,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    store
        .save_card(card)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
}

pub(crate) async fn save_character_card_force_rerun_async(
    store: Arc<campaign_store::CampaignStore>,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    tokio::task::spawn_blocking(move || {
        save_character_card_force_rerun_to_store(store.as_ref(), card)
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("保存角色卡任务失败: {e}")))?
}

pub(crate) fn save_character_card_force_rerun_to_store(
    store: &campaign_store::CampaignStore,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    store.save_card_if_no_campaigns(card).map_err(|e| {
        if e == campaign_store::FORCE_RERUN_BLOCKED_BY_CAMPAIGN {
            TauriCommandError::validation(e)
        } else {
            TauriCommandError::storage(format!("存储写入失败: {e}"))
        }
    })
}

/// 开档：建 Campaign，把卡里所有 Protagonist/Supporting 定义实例化；
/// 一 Campaign 一对话模型：同时自动建对话、存开场白、双向绑定 conversation_id
#[tauri::command]
pub(crate) fn create_campaign(
    card_id: String,
    name: String,
    opening_message: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    if state
        .storage()
        .capability(BackendCapability::CampaignLifecycle)
        != CapabilityStatus::Supported
    {
        return Err(TauriCommandError::validation(
            "campaign creation is not available in the SQLite opt-in backend yet".to_string(),
        ));
    }
    let store =
        state.json_campaign_store(BackendCapability::CampaignLifecycle, "create campaign")?;
    let character_store = state.json_character_store(
        BackendCapability::CharacterCommands,
        "create campaign from character card",
    )?;
    create_campaign_in_store(
        store,
        character_store,
        state.conv_store.as_ref(),
        card_id,
        name,
        opening_message,
    )
}

pub(crate) fn create_campaign_in_store(
    store: &campaign_store::CampaignStore,
    character_store: &storage::CharacterStore,
    conv_store: &ConversationStore,
    card_id: String,
    name: String,
    opening_message: Option<String>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    use storyforge_domain::conversation::Role as ConvRole;

    let card_id_value = Id::from_str(&card_id);
    let stored_card = store
        .get_card(&card_id_value)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 card id={card_id}")))?;
    let campaign_schema = stored_card.card.effective_campaign_variable_schema();
    let mut campaign = storyforge_domain::campaign::Campaign::new_with_variable_schema(
        card_id_value,
        name,
        &campaign_schema,
    );

    // 自动建对话并绑定到 Campaign
    let conv = conv_store.create(Some(card_id.clone()), Some(campaign.id.clone()));
    campaign.conversation_id = Some(conv.id.clone());
    let (stored, campaign, instance_count) = match store.create_campaign_with_instances(campaign) {
        Ok(result) => result,
        Err(e) => {
            if let Err(delete_err) = conv_store.delete(&conv.id) {
                tracing::warn!("创建 Campaign 失败后清理对话失败: {delete_err}");
            }
            return Err(TauriCommandError::storage(format!("存储写入失败: {e}")));
        }
    };

    // 存开场白（从 CharacterStore 按 source_character_id 查扁平 Character greeting）
    if let Some(opening) = resolve_campaign_opening_message(
        character_store,
        &stored.card.source_character_id,
        opening_message,
    ) && let Err(e) = conv_store.append_final_message(&conv.id, ConvRole::Assistant, opening)
    {
        tracing::warn!("建 Campaign 时追加开场白失败: {e}");
    }

    // 本局世界书：从卡模板（+ 其它卡 is_global）拷贝，活动侧可写、卡侧只读
    if let Err(e) = seed_campaign_world_info_from_card(store, character_store, &campaign, &stored) {
        tracing::warn!("开档拷贝世界书失败 campaign={}: {e}", campaign.id);
    }

    let mut dto = CampaignSummaryDto::from(&campaign);
    dto.instance_count = instance_count;
    Ok(dto)
}

/// 开场壳「开始旅程」选择落库：改写 Campaign 绑定会话的首条开场白。
/// 仅当会话仍处于开场态（唯一一条消息且为 assistant）时允许改写；
/// 已有后续消息时返回校验错误，避免壳选择覆盖真实写作历史。
#[tauri::command]
pub(crate) fn apply_campaign_opening(
    campaign_id: String,
    content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let store = state.json_campaign_store(
        BackendCapability::CampaignLifecycle,
        "apply campaign opening",
    )?;
    apply_campaign_opening_in_store(
        store,
        state.conv_store.as_ref(),
        &Id::from_str(&campaign_id),
        content,
    )
}

pub(crate) fn apply_campaign_opening_in_store(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
    campaign_id: &Id,
    content: String,
) -> Result<(), TauriCommandError> {
    use storyforge_domain::conversation::Role as ConvRole;

    if content.trim().is_empty() {
        return Err(TauriCommandError::validation("开场内容不能为空"));
    }
    let campaign = store
        .get_campaign(campaign_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign id={campaign_id}")))?;
    let conv_id = campaign
        .conversation_id
        .clone()
        .or_else(|| conv_store.find_by_campaign(&campaign.id).map(|c| c.id))
        .ok_or_else(|| {
            TauriCommandError::not_found(format!("campaign 未绑定会话: {campaign_id}"))
        })?;
    let conv = conv_store
        .get(&conv_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到会话 id={conv_id}")))?;
    if conv.nodes.len() != 1 {
        return Err(TauriCommandError::validation(
            "会话已有后续消息，开场选择不可再改写",
        ));
    }
    let node = &conv.nodes[0];
    let is_assistant_opening = node
        .active()
        .map(|v| v.role == ConvRole::Assistant)
        .unwrap_or(false);
    if !is_assistant_opening {
        return Err(TauriCommandError::validation(
            "会话首条消息不是开场白，拒绝改写",
        ));
    }
    let node_id = node.id.clone();
    conv_store
        .edit_variant(&conv_id, &node_id, content)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
}

/// 开档时将卡内嵌世界书（及其它卡 is_global 条目）写入 Campaign 旁路世界书文件。
pub(crate) fn seed_campaign_world_info_from_card(
    store: &campaign_store::CampaignStore,
    character_store: &storage::CharacterStore,
    campaign: &storyforge_domain::campaign::Campaign,
    stored_card: &campaign_store::StoredCard,
) -> Result<(), String> {
    let template = resolve_template_world_info_for_card(character_store, stored_card);
    store.ensure_world_info_from_book(&campaign.id, &template)?;
    Ok(())
}

pub(crate) fn resolve_template_world_info_for_card(
    character_store: &storage::CharacterStore,
    stored_card: &campaign_store::StoredCard,
) -> storyforge_domain::world_info::WorldInfoBook {
    // 优先 CharacterStore 完整书；否则用 card 关联 source 上的 embedded 书
    if let Some(sc) =
        stored_character_for_source_id(character_store, &stored_card.card.source_character_id)
    {
        if let Some(book) = sc.info.embedded_world_info.clone() {
            return merge_global_entries_into_book(character_store, book, &sc.info.name);
        }
        if let Some(book) = world_info_book_from_entries(&sc.info.world_info_entries) {
            return merge_global_entries_into_book(character_store, book, &sc.info.name);
        }
    }
    storyforge_domain::world_info::WorldInfoBook {
        entries: Vec::new(),
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}

pub(crate) fn merge_global_entries_into_book(
    character_store: &storage::CharacterStore,
    mut book: storyforge_domain::world_info::WorldInfoBook,
    active_name: &str,
) -> storyforge_domain::world_info::WorldInfoBook {
    let all = character_store.list();
    for stored in all {
        if stored.info.name == active_name {
            continue;
        }
        for e in &stored.info.world_info_entries {
            if !e.is_global {
                continue;
            }
            let mut entry = world_info_entry_from_info(e);
            if let Some(obj) = entry.extensions.as_object_mut() {
                obj.insert("sf_source".into(), serde_json::json!("merged_global"));
            } else {
                entry.extensions = serde_json::json!({ "sf_source": "merged_global" });
            }
            book.entries.push(entry);
        }
    }
    book
}

/// backend-neutral 版本的全局条目 merge：经 facade 读角色库（SQLite 走 V007
/// characters 表），供 set_active_campaign 激活时注入模板（Gate 4 四审 P1）。
/// 读取错误传播（Gate 4 五审 P2：不得用 unwrap_or_default 静默丢全局条目）。
pub(crate) fn merge_global_entries_into_book_facade(
    facade: &crate::storage_backend::StorageFacade,
    mut book: storyforge_domain::world_info::WorldInfoBook,
    active_name: &str,
) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
    let all = facade
        .list_characters()
        .map_err(|e| format!("merge global world-info entries: 角色库读取失败: {e}"))?;
    for stored in all {
        if stored.info.name == active_name {
            continue;
        }
        for e in &stored.info.world_info_entries {
            if !e.is_global {
                continue;
            }
            let mut entry = world_info_entry_from_info(e);
            if let Some(obj) = entry.extensions.as_object_mut() {
                obj.insert("sf_source".into(), serde_json::json!("merged_global"));
            } else {
                entry.extensions = serde_json::json!({ "sf_source": "merged_global" });
            }
            book.entries.push(entry);
        }
    }
    Ok(book)
}

pub(crate) fn fork_campaign_in_store(
    store: &campaign_store::CampaignStore,
    character_store: &storage::CharacterStore,
    conv_store: &ConversationStore,
    source_campaign_id: Id,
    fork_node_id: Id,
    name: String,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(TauriCommandError::validation("fork campaign name is empty"));
    }

    let source = store.get_campaign(&source_campaign_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("campaign not found: {source_campaign_id}"))
    })?;
    if store.get_card(&source.card_id).is_none() {
        return Err(TauriCommandError::not_found(format!(
            "card not found: {}",
            source.card_id
        )));
    }

    let source_conversation_id = source
        .conversation_id
        .clone()
        .or_else(|| conv_store.find_by_campaign(&source.id).map(|c| c.id))
        .ok_or_else(|| {
            TauriCommandError::not_found(format!(
                "source campaign has no conversation: {}",
                source.id
            ))
        })?;

    let mut campaign = storyforge_domain::campaign::Campaign::fork(
        source.card_id.clone(),
        name.to_string(),
        source.id.clone(),
        fork_node_id.clone(),
    );
    campaign.variables = source.variables.clone();
    campaign.variable_schema = source.variable_schema.clone();
    // Gate 4：story_clock 唯一权威 = variables；fork 时随 variables 拷贝，
    // 顶层字段从权威同步，避免双表示残留。
    campaign.story_clock = source.current_story_clock().to_string();

    let forked_conversation =
        conv_store.fork_at(&source_conversation_id, campaign.id.clone(), &fork_node_id)?;
    campaign.conversation_id = Some(forked_conversation.id);

    store
        .save_campaign(campaign.clone())
        .map_err(|e| TauriCommandError::storage(format!("save fork campaign failed: {e}")))?;

    let mut instance_count = 0;
    for mut instance in store.list_instances(&source.id) {
        instance.id = Id::new();
        instance.campaign_id = campaign.id.clone();
        store
            .add_instance(instance)
            .map_err(|e| TauriCommandError::storage(format!("copy fork instance failed: {e}")))?;
        instance_count += 1;
    }

    // 本局世界书：fork 时深拷贝源活动书（没有则从卡模板 ensure）
    match store.get_world_info(&source.id) {
        Ok(book) if !book.entries.is_empty() => {
            if let Err(e) = store.set_world_info(&campaign.id, book) {
                tracing::warn!("fork 拷贝世界书失败: {e}");
            }
        }
        _ => {
            if let Some(card) = store.get_card(&campaign.card_id)
                && let Err(e) =
                    seed_campaign_world_info_from_card(store, character_store, &campaign, &card)
            {
                tracing::warn!("fork 惰性种子世界书失败: {e}");
            }
        }
    }

    let mut dto = CampaignSummaryDto::from(&campaign);
    dto.instance_count = instance_count;
    Ok(dto)
}

#[tauri::command]
pub(crate) fn fork_campaign(
    source_campaign_id: String,
    fork_node_id: String,
    name: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    if state
        .storage()
        .capability(BackendCapability::CampaignLifecycle)
        != CapabilityStatus::Supported
    {
        return Err(TauriCommandError::validation(
            "SQLite opt-in currently rejects campaign fork rather than writing a JSON shadow copy"
                .to_string(),
        ));
    }
    let source_cid = Id::from_str(&source_campaign_id);

    // Phase A: fork 限制——不允许从有活动 Turn 的 Campaign fork（收敛决策盲区 2）
    if state
        .storage()
        .get_active_turn(&source_cid)
        .map_err(TauriCommandError::storage)?
        .is_some()
    {
        return Err(TauriCommandError::validation(
            "源 Campaign 有未完成的 Turn，请先 Accept、Discard 或 Abandon 后再 fork（阶段 A 只支持从已提交 head fork）".to_string(),
        ));
    }

    let store = state.json_campaign_store(BackendCapability::CampaignLifecycle, "fork campaign")?;
    let character_store = state.json_character_store(
        BackendCapability::CharacterCommands,
        "fork campaign character templates",
    )?;
    fork_campaign_in_store(
        store,
        character_store,
        &state.conv_store,
        source_cid,
        Id::from_str(&fork_node_id),
        name,
    )
}

#[tauri::command]
pub(crate) fn list_campaigns(
    card_id: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<CampaignSummaryDto>, TauriCommandError> {
    let requested_card = card_id.as_ref().map(Id::from_str);
    let campaigns = state
        .storage()
        .list_campaigns(requested_card.as_ref())
        .map_err(TauriCommandError::internal)?;
    Ok(campaigns
        .into_iter()
        .map(|record| {
            let mut dto = CampaignSummaryDto::from(&record.campaign);
            dto.instance_count = record.instance_count;
            dto
        })
        .collect())
}

#[tauri::command]
pub(crate) fn get_campaign(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    let record = state
        .storage()
        .get_campaign(&Id::from_str(&id))
        .map_err(TauriCommandError::internal)?
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign id={id}")))?;
    let mut dto = CampaignSummaryDto::from(&record.campaign);
    dto.instance_count = record.instance_count;
    Ok(dto)
}

/// 删除整局活动及其绑定会话；活跃活动被删除时同步清除活跃指针。
#[tauri::command]
pub(crate) fn delete_campaign(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if state
        .storage()
        .capability(BackendCapability::CampaignLifecycle)
        != CapabilityStatus::Supported
    {
        return Err(TauriCommandError::validation(
            "campaign deletion is not available in the SQLite opt-in backend yet".to_string(),
        ));
    }

    let store =
        state.json_campaign_store(BackendCapability::CampaignLifecycle, "delete campaign")?;
    crate::playthrough_lifecycle::delete_campaign_playthrough_in_store(
        store,
        state.conv_store.as_ref(),
        state.inner().as_ref(),
        &Id::from_str(&id),
    )
}

#[tauri::command]
pub fn set_active_campaign(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let campaign_id = Id::from_str(&id);

    // 写作注入：活跃活动切换后 tool_ctx 改读本局世界书（WorldInfo 能力声明式门控）。
    // 经 backend-neutral facade 读取——SQLite 下不得触碰 JSON store。
    //
    // Gate 4 五审 P1 修复：世界书读取 / 模板解析 / 惰性种子全部前移到活跃指针
    // 提交**之前**——任意一步失败，命令返回错误时指针尚未改变（失败原子性，
    // 不再出现“指针已改、命令却失败”的半成功状态）。指针提交仍走
    // set_active_campaign_in_state：锁内重新校验存在性，删除无法在读取与提交
    // 之间使选择失效。
    let prepared_book: Option<storyforge_domain::world_info::WorldInfoBook> = if state
        .storage()
        .capability(BackendCapability::WorldInfo)
        == CapabilityStatus::Supported
    {
        let mut book = state
            .storage()
            .get_world_info(&campaign_id)
            .map_err(TauriCommandError::storage)?;
        if book.entries.is_empty()
            && let Some(camp) = state
                .storage()
                .get_campaign(&campaign_id)
                .map_err(TauriCommandError::storage)?
        {
            // 模板来源：角色库内嵌世界书（backend-neutral facade），其次 ST 卡
            // 模板（SQLite 专用 facade 方法）。读取错误传播（不静默吞掉）。
            let mut template: Option<storyforge_domain::world_info::WorldInfoBook> = None;
            if let Some(card) = state
                .storage()
                .get_card(&camp.campaign.card_id)
                .map_err(TauriCommandError::storage)?
            {
                template = state
                    .storage()
                    .resolve_character_world_info_template(&card.card.source_character_id)
                    .map_err(TauriCommandError::storage)?;
                if template.is_none() {
                    template = state
                        .storage()
                        .template_world_info_from_card(&serde_json::to_value(&card).map_err(
                            |e| TauriCommandError::internal(format!("卡序列化失败: {e}")),
                        )?)
                        .map_err(TauriCommandError::storage)?;
                }
            }
            if let Some(template) = template {
                book = state
                    .storage()
                    .ensure_world_info_from_book(&campaign_id, &template)
                    .map_err(TauriCommandError::storage)?;
            }
        }
        Some(book)
    } else {
        None
    };

    // 世界书已就绪，此刻才提交活跃指针（失败不改指针）。tool_ctx 世界书写入
    // 作为 after_commit 钩子在锁内执行（Gate 4 六审 P1：与指针提交原子，
    // 删除线程无法在指针提交后、世界书写入前插入清指针）。
    set_active_campaign_in_state(
        state.inner().as_ref(),
        campaign_id.clone(),
        || {
            if !state
                .storage()
                .campaign_exists(&campaign_id)
                .map_err(TauriCommandError::internal)?
            {
                return Err(TauriCommandError::not_found(format!(
                    "找不到 campaign id={id}"
                )));
            }
            Ok(())
        },
        |state, id| {
            if let Some(book) = prepared_book {
                apply_campaign_world_info_to_tool_ctx(state, id, &book);
            }
        },
    )?;
    Ok(())
}

/// Commit an active Campaign selection as one serialized update. The validator
/// and the `after_commit` hook run under the same lock so deletion cannot
/// invalidate a selection between validation, pointer persistence, the
/// in-memory commit and any follow-up state (e.g. tool_ctx world-info write).
///
/// Gate 4 六审 P1：`after_commit` 在锁内执行——激活命令写指针与写 tool_ctx
/// 世界书成为一个原子单元，删除线程清指针必须等待整个提交完成，杜绝
/// “指针已清空、旧 Campaign 世界书又被写回 tool_ctx”的悬挂状态。
///
/// Gate 4 七审 P2：提升为 `pub` 供独立并发测试直接调用——测试用 validate
/// 闭包（锁内执行）作可控屏障，在 after_commit（世界书写入）与删除线程
/// 之间确定性编排，证明删除线程被锁阻挡。
pub fn set_active_campaign_in_state<F, G>(
    state: &AppState,
    campaign_id: Id,
    validate: F,
    after_commit: G,
) -> Result<(), TauriCommandError>
where
    F: FnOnce() -> Result<(), TauriCommandError>,
    G: FnOnce(&AppState, &Id),
{
    let _update = state
        .active_campaign_update
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    validate()?;
    // 指针持久化由 backend adapter 处理：JSON 写 active_campaign.json，
    // SQLite 仅内存（ActiveCampaignPersistence Degraded，显式不落盘）。
    crate::backend_workflows::save_active_pointer(state.storage(), Some(&campaign_id))
        .map_err(TauriCommandError::storage)?;
    *state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(campaign_id.clone());
    after_commit(state, &campaign_id);
    Ok(())
}

#[tauri::command]
pub(crate) fn get_active_campaign(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Option<CampaignSummaryDto>, TauriCommandError> {
    let id = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let Some(id) = id else {
        return Ok(None);
    };
    let Some(record) = state
        .storage()
        .get_campaign(&id)
        .map_err(TauriCommandError::internal)?
    else {
        return Ok(None);
    };
    let mut dto = CampaignSummaryDto::from(&record.campaign);
    dto.instance_count = record.instance_count;
    Ok(Some(dto))
}

#[tauri::command]
pub(crate) fn list_instances(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<CharacterInstanceDto>, TauriCommandError> {
    state
        .storage()
        .require_supported(
            BackendCapability::CampaignInstanceRead,
            "list campaign instances",
        )
        .map_err(TauriCommandError::validation)?;
    let campaign_id = Id::from_str(&campaign_id);
    let instances = state
        .storage()
        .list_instances(&campaign_id)
        .map_err(TauriCommandError::storage)?;
    instances
        .iter()
        .map(|instance| {
            crate::backend_workflows::character_instance_dto_for_backend(state.storage(), instance)
                .map_err(TauriCommandError::storage)
        })
        .collect()
}

#[tauri::command]
pub(crate) fn get_instance(
    campaign_id: String,
    instance_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInstanceDto, TauriCommandError> {
    state
        .storage()
        .require_supported(
            BackendCapability::CampaignInstanceRead,
            "get campaign instance",
        )
        .map_err(TauriCommandError::validation)?;
    let instance = state
        .storage()
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))?;
    crate::backend_workflows::character_instance_dto_for_backend(state.storage(), &instance)
        .map_err(TauriCommandError::storage)
}

pub(crate) fn trimmed_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn validate_custom_character_text(
    label: &str,
    value: Option<&str>,
    max_chars: usize,
) -> Result<(), TauriCommandError> {
    if value.is_some_and(|value| value.chars().count() > max_chars) {
        return Err(TauriCommandError::validation(format!(
            "{label}不能超过 {max_chars} 个字符"
        )));
    }
    Ok(())
}

pub(crate) fn add_campaign_instance_to_store(
    store: &campaign_store::CampaignStore,
    campaign_id: &Id,
    definition_id: Option<&Id>,
    name: Option<String>,
    persona: Option<String>,
    behavior: Option<String>,
) -> Result<CharacterInstanceDto, TauriCommandError> {
    use storyforge_domain::campaign::CharacterInstance;

    let campaign = store
        .get_campaign(campaign_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))?;
    let existing = store.list_instances(campaign_id);

    let instance = if let Some(definition_id) = definition_id {
        let card = store.get_card(&campaign.card_id).ok_or_else(|| {
            TauriCommandError::not_found(format!("找不到 card {}", campaign.card_id))
        })?;
        let definition = card
            .card
            .character_definitions
            .iter()
            .find(|definition| definition.id == *definition_id)
            .ok_or_else(|| TauriCommandError::validation("该角色定义不属于当前活动的角色卡"))?;
        if existing.iter().any(|instance| {
            instance.definition_id.as_ref() == Some(definition_id)
                || instance.name.eq_ignore_ascii_case(&definition.name)
        }) {
            return Err(TauriCommandError::validation(format!(
                "角色「{}」已加入本局",
                definition.name
            )));
        }
        CharacterInstance::from_definition(campaign.id.clone(), definition)
    } else {
        let name = name.unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return Err(TauriCommandError::validation("角色名称不能为空"));
        }
        validate_custom_character_text("角色名称", Some(&name), 80)?;
        let persona = trimmed_optional(persona);
        let behavior = trimmed_optional(behavior);
        validate_custom_character_text("角色人设", persona.as_deref(), 10_000)?;
        validate_custom_character_text("行为规则", behavior.as_deref(), 10_000)?;
        if existing
            .iter()
            .any(|instance| instance.name.eq_ignore_ascii_case(&name))
        {
            return Err(TauriCommandError::validation(format!(
                "本局已有同名角色「{name}」"
            )));
        }
        CharacterInstance::temporary_with_overrides(campaign.id.clone(), name, persona, behavior)
    };

    store
        .add_instance(instance.clone())
        .map_err(|error| TauriCommandError::storage(format!("添加角色失败: {error}")))?;
    Ok(character_instance_dto_from_store(store, &instance))
}

/// Add a card-defined character or an ad-hoc temporary character to a Campaign.
#[tauri::command]
pub(crate) fn add_campaign_instance(
    campaign_id: String,
    definition_id: Option<String>,
    name: Option<String>,
    persona: Option<String>,
    behavior: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInstanceDto, TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    let store = state.json_campaign_store(
        BackendCapability::CampaignLifecycle,
        "add campaign instance",
    )?;
    reject_if_active_turn(state.storage(), &campaign_id)?;
    let definition_id = definition_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(Id::from_str);
    add_campaign_instance_to_store(
        store,
        &campaign_id,
        definition_id.as_ref(),
        name,
        persona,
        behavior,
    )
}

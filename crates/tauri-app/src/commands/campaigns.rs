use super::super::*;
use crate::storage_backend::{BackendCapability, CapabilityStatus};

pub use storyforge_app_service::services::campaigns::*;

/// 跑角色识别 Agent，为已导入的扁平 Character 建 CharacterCard
///
/// 失败时降级：建单角色 Protagonist definition（卡仍可用）。
#[tauri::command]
pub async fn extract_characters(
    source_character_id: String,
    force: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardSummaryDto, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;
    use storyforge_domain::character::CharacterExtractionStatus;
    use storyforge_domain::variables::{
        extract_campaign_variable_schema_from_extensions, extract_mvu_schema_from_extensions,
    };

    let character_store = state.storage();

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
                //    与 Character.id 无关）。用存储 id 查角色库拿到 name，再按 name 查 tool_ctx。
                character_store
                    .get_character(&source_character_id)
                    .ok()
                    .flatten()
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
    let force = force.unwrap_or(false);
    let mut card =
        match prepare_character_extraction_card_for_backend(state.storage(), &character, force)? {
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
        save_character_card_force_rerun_for_backend(state.storage().clone(), card).await?
    } else {
        save_character_card_for_backend(state.storage().clone(), card).await?
    };

    Ok(CardSummaryDto::from(&stored))
}

/// 开档：建 Campaign，把卡里所有 Protagonist/Supporting 定义实例化；
/// 一 Campaign 一对话模型：同时自动建对话、存开场白、双向绑定 conversation_id
#[tauri::command]
pub fn create_campaign(
    card_id: String,
    name: String,
    opening_message: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    create_campaign_for_backend(
        state.storage(),
        state.conv_store.as_ref(),
        card_id,
        name,
        opening_message,
    )
}

/// 开场壳「开始旅程」选择落库：改写 Campaign 绑定会话的首条开场白。
/// 仅当会话仍处于开场态（唯一一条消息且为 assistant）时允许改写；
/// 已有后续消息时返回校验错误，避免壳选择覆盖真实写作历史。
#[tauri::command]
pub fn apply_campaign_opening(
    campaign_id: String,
    content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    apply_campaign_opening_for_backend(
        state.storage(),
        state.conv_store.as_ref(),
        &Id::from_str(&campaign_id),
        content,
    )
}

#[tauri::command]
pub fn fork_campaign(
    source_campaign_id: String,
    fork_node_id: String,
    name: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
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

    let source_conversation_id = state
        .storage()
        .get_campaign(&source_cid)
        .ok()
        .flatten()
        .and_then(|record| record.campaign.conversation_id);
    let forked = fork_campaign_for_backend(
        state.storage(),
        state.conv_store.as_ref(),
        source_cid.clone(),
        Id::from_str(&fork_node_id),
        name,
    )?;
    let forked_campaign_id = Id::from_str(&forked.id);
    // 分支的摘要换了新 id：复用来源局已有向量补建远记忆索引（不重新嵌入）。
    crate::vector_index::index_campaign_summaries(
        state.inner(),
        &forked_campaign_id,
        Some(&source_cid),
    );
    // 会话级远记忆（消息归档）连同归档水位一起带给分支。
    if let (Some(source_conversation), Some(forked_conversation)) =
        (source_conversation_id, forked.conversation_id.as_deref())
    {
        crate::vector_index::copy_message_archives(
            state.inner(),
            &source_conversation,
            &forked_campaign_id,
            &Id::from_str(forked_conversation),
        );
    }
    Ok(forked)
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
pub fn delete_campaign(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    crate::playthrough_lifecycle::delete_campaign_playthrough_in_store(
        state.storage().as_ref(),
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

    // 活跃 runtime 快照在取锁前预加载（纯存储读，无本侧锁；失败仅降级为
    // 「暂不填充」——写作路径的 fill_campaign_context 仍是权威加载点）。
    // 2026-09-01 B5：没有这份快照，重启恢复/切换活动后 Meta 工具在首次
    // 写作前一律回答「当前没有 active Campaign」。
    let prepared_runtime =
        match crate::backend_workflows::load_campaign_context_snapshot_for_backend(
            state.storage(),
            &campaign_id,
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    "active Campaign runtime 快照加载失败（首次写作前 Meta 工具不可用）: {error}"
                );
                None
            }
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
            if let Some(snapshot) = prepared_runtime {
                crate::runtime_support::apply_campaign_runtime_to_tool_ctx(state, snapshot);
            }
        },
    )?;
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

/// Add a card-defined character or an ad-hoc temporary character to a Campaign.
#[tauri::command]
pub fn add_campaign_instance(
    campaign_id: String,
    definition_id: Option<String>,
    name: Option<String>,
    persona: Option<String>,
    behavior: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInstanceDto, TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    let definition_id = definition_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(Id::from_str);
    add_campaign_instance_for_backend(
        state.storage(),
        &campaign_id,
        definition_id.as_ref(),
        name,
        persona,
        behavior,
    )
}

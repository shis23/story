use super::super::*;
use crate::storage_backend::BackendCapability;

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
    extract_characters_for_host(state.inner(), source_character_id, force).await
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
    fork_campaign_for_host(state.inner(), source_campaign_id, fork_node_id, name)
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
    set_active_campaign_for_host(state.inner(), id)
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

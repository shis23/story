use super::super::*;
use crate::storage_backend::BackendCapability;

pub fn require_world_info_supported(
    state: &AppState,
    operation: &str,
) -> Result<(), TauriCommandError> {
    state
        .storage()
        .require_supported(BackendCapability::WorldInfo, operation)
        .map_err(TauriCommandError::validation)
}

// ─── Campaign 本局世界书（卡只读模板 / 活动可写真相源）──────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignWorldInfoEntryDto {
    pub index: usize,
    /// ST entry comment (preferred) or a keyword fallback; exposed to the
    /// Card Shell as the TavernHelper worldbook entry name.
    pub name: String,
    pub keys: Vec<String>,
    pub secondary_keys: Vec<String>,
    /// 列表预览用截断正文（默认）；完整正文走 get_*_world_info_entry
    pub content: String,
    pub constant: bool,
    pub selective: bool,
    pub disabled: bool,
    pub depth: i32,
    pub order: i32,
    pub route: String,
    /// card | merged_global | user
    pub source: String,
    /// 卡模板条目是否全局共享（切到其它角色卡也生效）
    #[serde(default)]
    pub is_global: bool,
    /// 原文是否被截断
    #[serde(default)]
    pub content_truncated: bool,
    /// 原文长度（字符）
    #[serde(default)]
    pub content_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignWorldInfoDto {
    pub campaign_id: String,
    pub entry_count: usize,
    pub constant_count: usize,
    pub selective_count: usize,
    pub entries: Vec<CampaignWorldInfoEntryDto>,
}

pub fn lore_route_to_str(route: &storyforge_domain::world_info::LoreRoute) -> String {
    match route {
        storyforge_domain::world_info::LoreRoute::Constant => "Constant".into(),
        storyforge_domain::world_info::LoreRoute::Selective => "Selective".into(),
        storyforge_domain::world_info::LoreRoute::Both => "Both".into(),
        storyforge_domain::world_info::LoreRoute::Disabled => "Disabled".into(),
    }
}

pub fn parse_lore_route(
    s: &str,
) -> Result<storyforge_domain::world_info::LoreRoute, TauriCommandError> {
    match s {
        "Constant" | "constant" => Ok(storyforge_domain::world_info::LoreRoute::Constant),
        "Selective" | "selective" => Ok(storyforge_domain::world_info::LoreRoute::Selective),
        "Both" | "both" => Ok(storyforge_domain::world_info::LoreRoute::Both),
        "Disabled" | "disabled" => Ok(storyforge_domain::world_info::LoreRoute::Disabled),
        other => Err(TauriCommandError::validation(format!(
            "未知世界书路由: {other}"
        ))),
    }
}

pub fn entry_source_label(entry: &storyforge_domain::world_info::WorldInfoEntry) -> String {
    entry
        .extensions
        .get("sf_source")
        .and_then(|v| v.as_str())
        .unwrap_or("card")
        .to_string()
}

pub fn world_info_entry_name(
    index: usize,
    entry: &storyforge_domain::world_info::WorldInfoEntry,
) -> String {
    entry
        .extra
        .get("comment")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            entry
                .keys
                .iter()
                .map(|key| key.trim())
                .find(|key| !key.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| format!("world-info-{index}"))
}

const WORLD_INFO_LIST_PREVIEW_CHARS: usize = 240;

pub fn truncate_world_info_content(content: &str, max_chars: usize) -> (String, bool, usize) {
    let len = content.chars().count();
    if len <= max_chars {
        return (content.to_string(), false, len);
    }
    let preview: String = content.chars().take(max_chars).collect();
    (format!("{preview}…"), true, len)
}

pub fn world_info_entry_to_dto(
    index: usize,
    e: &storyforge_domain::world_info::WorldInfoEntry,
) -> CampaignWorldInfoEntryDto {
    world_info_entry_to_dto_preview(index, e, WORLD_INFO_LIST_PREVIEW_CHARS)
}

pub fn world_info_entry_to_dto_preview(
    index: usize,
    e: &storyforge_domain::world_info::WorldInfoEntry,
    max_chars: usize,
) -> CampaignWorldInfoEntryDto {
    let (content, truncated, content_len) = truncate_world_info_content(&e.content, max_chars);
    CampaignWorldInfoEntryDto {
        index,
        name: world_info_entry_name(index, e),
        keys: e.keys.clone(),
        secondary_keys: e.secondary_keys.clone(),
        content,
        constant: e.constant,
        selective: e.selective,
        disabled: e.disabled,
        depth: e.depth,
        order: e.order,
        route: lore_route_to_str(&e.route),
        source: entry_source_label(e),
        is_global: crate::services::characters::entry_is_global(e),
        content_truncated: truncated,
        content_len,
    }
}

pub fn world_info_entry_to_dto_full(
    index: usize,
    e: &storyforge_domain::world_info::WorldInfoEntry,
) -> CampaignWorldInfoEntryDto {
    CampaignWorldInfoEntryDto {
        index,
        name: world_info_entry_name(index, e),
        keys: e.keys.clone(),
        secondary_keys: e.secondary_keys.clone(),
        content: e.content.clone(),
        constant: e.constant,
        selective: e.selective,
        disabled: e.disabled,
        depth: e.depth,
        order: e.order,
        route: lore_route_to_str(&e.route),
        source: entry_source_label(e),
        is_global: crate::services::characters::entry_is_global(e),
        content_truncated: false,
        content_len: e.content.chars().count(),
    }
}

pub fn book_to_campaign_world_info_dto(
    campaign_id: &Id,
    book: &storyforge_domain::world_info::WorldInfoBook,
) -> CampaignWorldInfoDto {
    let entries: Vec<_> = book
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| world_info_entry_to_dto(i, e))
        .collect();
    let constant_count = book.constant_entries().len();
    let selective_count = book.selective_entries().len();
    CampaignWorldInfoDto {
        campaign_id: campaign_id.as_str().to_string(),
        entry_count: entries.len(),
        constant_count,
        selective_count,
        entries,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddCampaignWorldInfoDto {
    pub campaign_id: String,
    pub keys: Vec<String>,
    pub content: String,
    pub constant: bool,
    #[serde(default = "default_depth")]
    pub depth: i32,
    #[serde(default = "default_order")]
    pub order: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCampaignWorldInfoDto {
    pub campaign_id: String,
    pub entry_index: usize,
    pub keys: Vec<String>,
    pub content: String,
    pub constant: bool,
    pub disabled: bool,
    pub depth: i32,
    pub order: i32,
    pub route: String,
}

/// 落盘成功后的 tool_ctx 世界书刷新（`active_campaign` 不匹配时内部直接返回）。
///
/// 2026-09-13 域4 修复 T-08：四处写命令此前写的是
/// `if let Ok(book) = state.storage().get_world_info(&id) { apply(...) }`——
/// 重读失败时**静默跳过**刷新，于是世界书已落盘、进程内 `tool_ctx` 却仍是旧快照，
/// 后续写作轮次按旧条目注入，且没有任何日志。这与 `commands/cards.rs` 已确立的
/// 约定（"不得用 if let Ok / .ok() 吞掉"）相矛盾。
///
/// 这里至少留下 error 级日志。**不把命令改成返回 Err** 的理由：写操作已经持久化
/// 成功，报错会诱导调用方重试，而 `add_campaign_world_info_entry` 重试会重复插入
/// 一条条目（非幂等），把"快照过期"升级成"数据重复"反而更糟。
pub fn refresh_tool_ctx_world_info(state: &AppState, campaign_id: &Id, command: &'static str) {
    match state.storage().get_world_info(campaign_id) {
        Ok(book) => apply_campaign_world_info_to_tool_ctx(state, campaign_id, &book),
        Err(e) => tracing::error!(
            campaign = %campaign_id,
            command,
            error = %e,
            "世界书已落盘，但 tool_ctx 刷新失败：本轮之后的写作可能仍按旧世界书注入"
        ),
    }
}

/// 若 `campaign_id` 是当前活跃活动，则把本局世界书写入 tool_ctx（写作注入真相源）。
pub fn apply_campaign_world_info_to_tool_ctx(
    state: &AppState,
    campaign_id: &Id,
    book: &storyforge_domain::world_info::WorldInfoBook,
) {
    let active = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if active.as_ref() != Some(campaign_id) {
        return;
    }
    let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    if book.entries.is_empty() {
        ctx.world_info = None;
    } else {
        ctx.world_info = Some(std::sync::Arc::new(book.clone()));
    }
}

/// 列出本局世界书（Tauri `list_campaign_world_info` 与非 Tauri 宿主共用）：本局书还空着
/// 时按卡模板惰性拷贝，并在它是活跃活动时刷新 tool_ctx。
pub fn list_campaign_world_info_for_host(
    state: &AppState,
    campaign_id: &Id,
) -> Result<CampaignWorldInfoDto, TauriCommandError> {
    require_world_info_supported(state, "list campaign world info")?;
    let book = crate::backend_workflows::load_campaign_world_info_for_backend(
        state.storage(),
        state,
        campaign_id,
    )
    .map_err(TauriCommandError::storage)?;
    apply_campaign_world_info_to_tool_ctx(state, campaign_id, &book);
    Ok(book_to_campaign_world_info_dto(campaign_id, &book))
}

/// 只改本局世界书一条的启用状态（Tauri `set_campaign_world_info_enabled` 与非 Tauri
/// 宿主共用）：不动正文、关键词与注入路由，卡壳核心选择器只拥有这一位。
pub fn set_campaign_world_info_enabled_for_host(
    state: &AppState,
    campaign_id: &Id,
    entry_index: usize,
    enabled: bool,
) -> Result<(), TauriCommandError> {
    require_world_info_supported(state, "set campaign world info enabled")?;
    let book = state
        .storage()
        .set_world_info_entry_enabled(campaign_id, entry_index, enabled)
        .map_err(TauriCommandError::storage)?;
    apply_campaign_world_info_to_tool_ctx(state, campaign_id, &book);
    Ok(())
}

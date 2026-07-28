use super::super::*;

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

pub(crate) fn lore_route_to_str(route: &storyforge_domain::world_info::LoreRoute) -> String {
    match route {
        storyforge_domain::world_info::LoreRoute::Constant => "Constant".into(),
        storyforge_domain::world_info::LoreRoute::Selective => "Selective".into(),
        storyforge_domain::world_info::LoreRoute::Both => "Both".into(),
        storyforge_domain::world_info::LoreRoute::Disabled => "Disabled".into(),
    }
}

pub(crate) fn parse_lore_route(
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

pub(crate) fn entry_source_label(entry: &storyforge_domain::world_info::WorldInfoEntry) -> String {
    entry
        .extensions
        .get("sf_source")
        .and_then(|v| v.as_str())
        .unwrap_or("card")
        .to_string()
}

pub(crate) fn world_info_entry_name(
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

pub(crate) fn truncate_world_info_content(
    content: &str,
    max_chars: usize,
) -> (String, bool, usize) {
    let len = content.chars().count();
    if len <= max_chars {
        return (content.to_string(), false, len);
    }
    let preview: String = content.chars().take(max_chars).collect();
    (format!("{preview}…"), true, len)
}

pub(crate) fn world_info_entry_to_dto(
    index: usize,
    e: &storyforge_domain::world_info::WorldInfoEntry,
) -> CampaignWorldInfoEntryDto {
    world_info_entry_to_dto_preview(index, e, WORLD_INFO_LIST_PREVIEW_CHARS)
}

pub(crate) fn world_info_entry_to_dto_preview(
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
        content_truncated: truncated,
        content_len,
    }
}

pub(crate) fn world_info_entry_to_dto_full(
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
        content_truncated: false,
        content_len: e.content.chars().count(),
    }
}

pub(crate) fn book_to_campaign_world_info_dto(
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

/// 列出本局世界书。若尚未拷贝且可从卡解析模板，则惰性 ensure。
#[tauri::command]
pub(crate) fn list_campaign_world_info(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignWorldInfoDto, TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign world info is not available in the SQLite opt-in backend yet",
        ));
    }
    let store = get_campaign_store();
    let id = Id::from_str(&campaign_id);
    let camp = store
        .get_campaign(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign id={campaign_id}")))?;
    let mut book = store
        .get_world_info(&id)
        .map_err(TauriCommandError::storage)?;
    if book.entries.is_empty()
        && let Some(card) = store.get_card(&camp.card_id)
    {
        let template = resolve_template_world_info_for_card(&card);
        book = store
            .ensure_world_info_from_book(&id, &template)
            .map_err(TauriCommandError::storage)?;
    }
    apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    Ok(book_to_campaign_world_info_dto(&id, &book))
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

#[tauri::command]
pub(crate) fn add_campaign_world_info_entry(
    req: AddCampaignWorldInfoDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign world info is not available in the SQLite opt-in backend yet",
        ));
    }
    let id = Id::from_str(&req.campaign_id);
    let route = if req.constant {
        storyforge_domain::world_info::LoreRoute::Constant
    } else {
        storyforge_domain::world_info::LoreRoute::Selective
    };
    let entry = storyforge_domain::world_info::WorldInfoEntry {
        st_id: None,
        keys: req.keys,
        secondary_keys: vec![],
        content: req.content,
        constant: req.constant,
        selective: !req.constant,
        selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
        disabled: false,
        position: 0,
        depth: req.depth,
        order: req.order,
        route,
        extensions: serde_json::json!({ "sf_source": "user" }),
        extra: Default::default(),
    };
    let store = get_campaign_store();
    let idx = store
        .add_world_info_entry(&id, entry)
        .map_err(TauriCommandError::storage)?;
    if let Ok(book) = store.get_world_info(&id) {
        apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    }
    Ok(idx)
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

#[tauri::command]
pub(crate) fn update_campaign_world_info_entry(
    req: UpdateCampaignWorldInfoDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign world info is not available in the SQLite opt-in backend yet",
        ));
    }
    let id = Id::from_str(&req.campaign_id);
    let store = get_campaign_store();
    let book = store
        .get_world_info(&id)
        .map_err(TauriCommandError::storage)?;
    let prev = book.entries.get(req.entry_index).ok_or_else(|| {
        TauriCommandError::not_found(format!("条目索引越界: {}", req.entry_index))
    })?;
    let route = parse_lore_route(&req.route)?;
    let mut entry = prev.clone();
    entry.keys = req.keys;
    entry.content = req.content;
    entry.constant = req.constant;
    entry.selective = !req.constant;
    entry.disabled =
        req.disabled || matches!(route, storyforge_domain::world_info::LoreRoute::Disabled);
    entry.depth = req.depth;
    entry.order = req.order;
    entry.route = route;
    store
        .update_world_info_entry(&id, req.entry_index, entry)
        .map_err(TauriCommandError::storage)?;
    if let Ok(book) = store.get_world_info(&id) {
        apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    }
    Ok(())
}

/// Toggle a Campaign world-info entry without rewriting its body, keys, or
/// injection route. Card Shell core selectors only own this enabled state.
#[tauri::command]
pub(crate) fn set_campaign_world_info_enabled(
    campaign_id: String,
    entry_index: usize,
    enabled: bool,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign world info is not available in the SQLite opt-in backend yet",
        ));
    }
    let id = Id::from_str(&campaign_id);
    let store = get_campaign_store();
    let book = store
        .set_world_info_entry_enabled(&id, entry_index, enabled)
        .map_err(TauriCommandError::storage)?;
    apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    Ok(())
}

#[tauri::command]
pub(crate) fn delete_campaign_world_info_entry(
    campaign_id: String,
    entry_index: usize,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign world info is not available in the SQLite opt-in backend yet",
        ));
    }
    let id = Id::from_str(&campaign_id);
    let store = get_campaign_store();
    store
        .delete_world_info_entry(&id, entry_index)
        .map_err(TauriCommandError::storage)?;
    if let Ok(book) = store.get_world_info(&id) {
        apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn set_campaign_world_info_route(
    campaign_id: String,
    entry_index: usize,
    route: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign world info is not available in the SQLite opt-in backend yet",
        ));
    }
    let id = Id::from_str(&campaign_id);
    let lore = parse_lore_route(&route)?;
    let store = get_campaign_store();
    store
        .set_world_info_route(&id, entry_index, lore)
        .map_err(TauriCommandError::storage)?;
    if let Ok(book) = store.get_world_info(&id) {
        apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    }
    Ok(())
}

/// 卡模板世界书只读（供角色卡 UI，不可写路径）。
/// `character_id` 可为 CharacterStore.id 或 CharacterCard.source_character_id。
#[tauri::command]
pub(crate) fn get_character_world_info(
    character_id: String,
) -> Result<CampaignWorldInfoDto, TauriCommandError> {
    let stored = get_store()
        .get(&character_id)
        .or_else(|| stored_character_for_source_id(&Id::from_str(&character_id)))
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;
    let book = stored
        .info
        .embedded_world_info
        .clone()
        .or_else(|| world_info_book_from_entries(&stored.info.world_info_entries))
        .unwrap_or_else(|| storyforge_domain::world_info::WorldInfoBook {
            entries: Vec::new(),
            source: storyforge_domain::Source::Native,
            metadata: Default::default(),
        });
    // 伪 campaign_id 槽位仅用于 DTO 复用；UI 标注只读
    let fake = Id::from_str(format!("card:{}", stored.id));
    Ok(book_to_campaign_world_info_dto(&fake, &book))
}

/// 卡模板世界书单条完整正文（展开编辑/预览用，避免列表一次下发 1MB+）。
#[tauri::command]
pub(crate) fn get_character_world_info_entry(
    character_id: String,
    entry_index: usize,
) -> Result<CampaignWorldInfoEntryDto, TauriCommandError> {
    let stored = get_store()
        .get(&character_id)
        .or_else(|| stored_character_for_source_id(&Id::from_str(&character_id)))
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;
    let book = stored
        .info
        .embedded_world_info
        .clone()
        .or_else(|| world_info_book_from_entries(&stored.info.world_info_entries))
        .unwrap_or_else(|| storyforge_domain::world_info::WorldInfoBook {
            entries: Vec::new(),
            source: storyforge_domain::Source::Native,
            metadata: Default::default(),
        });
    let entry = book.entries.get(entry_index).ok_or_else(|| {
        TauriCommandError::not_found(format!("世界书条目索引越界: {entry_index}"))
    })?;
    Ok(world_info_entry_to_dto_full(entry_index, entry))
}

/// 本局世界书单条完整正文。
#[tauri::command]
pub(crate) fn get_campaign_world_info_entry(
    campaign_id: String,
    entry_index: usize,
) -> Result<CampaignWorldInfoEntryDto, TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign world info is not available in the SQLite opt-in backend yet",
        ));
    }
    let id = Id::from_str(&campaign_id);
    let book = get_campaign_store()
        .get_world_info(&id)
        .map_err(TauriCommandError::storage)?;
    let entry = book.entries.get(entry_index).ok_or_else(|| {
        TauriCommandError::not_found(format!("世界书条目索引越界: {entry_index}"))
    })?;
    Ok(world_info_entry_to_dto_full(entry_index, entry))
}

/// 若 `campaign_id` 是当前活跃活动，则把本局世界书写入 tool_ctx（写作注入真相源）。
pub(crate) fn apply_campaign_world_info_to_tool_ctx(
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

use super::super::*;
use crate::storage_backend::BackendCapability;

pub use storyforge_app_service::services::world_info::*;

/// 列出本局世界书。若尚未拷贝且可从卡解析模板，则惰性 ensure。
#[tauri::command]
pub(crate) fn list_campaign_world_info(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignWorldInfoDto, TauriCommandError> {
    require_world_info_supported(state.inner().as_ref(), "list campaign world info")?;
    let id = Id::from_str(&campaign_id);
    let book = crate::backend_workflows::load_campaign_world_info_for_backend(
        state.storage(),
        state.inner(),
        &id,
    )
    .map_err(TauriCommandError::storage)?;
    apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    Ok(book_to_campaign_world_info_dto(&id, &book))
}

#[tauri::command]
pub(crate) fn add_campaign_world_info_entry(
    req: AddCampaignWorldInfoDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    require_world_info_supported(state.inner().as_ref(), "add campaign world info entry")?;
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
    let idx = state
        .storage()
        .add_world_info_entry(&id, entry)
        .map_err(TauriCommandError::storage)?;
    refresh_tool_ctx_world_info(state.inner(), &id, "add_campaign_world_info_entry");
    Ok(idx)
}

#[tauri::command]
pub(crate) fn update_campaign_world_info_entry(
    req: UpdateCampaignWorldInfoDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    require_world_info_supported(state.inner().as_ref(), "update campaign world info entry")?;
    let id = Id::from_str(&req.campaign_id);
    let book = state
        .storage()
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
    state
        .storage()
        .update_world_info_entry(&id, req.entry_index, entry)
        .map_err(TauriCommandError::storage)?;
    refresh_tool_ctx_world_info(state.inner(), &id, "update_campaign_world_info_entry");
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
    require_world_info_supported(state.inner().as_ref(), "set campaign world info enabled")?;
    let id = Id::from_str(&campaign_id);
    let book = state
        .storage()
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
    require_world_info_supported(state.inner().as_ref(), "delete campaign world info entry")?;
    let id = Id::from_str(&campaign_id);
    state
        .storage()
        .delete_world_info_entry(&id, entry_index)
        .map_err(TauriCommandError::storage)?;
    refresh_tool_ctx_world_info(state.inner(), &id, "delete_campaign_world_info_entry");
    Ok(())
}

#[tauri::command]
pub(crate) fn set_campaign_world_info_route(
    campaign_id: String,
    entry_index: usize,
    route: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    require_world_info_supported(state.inner().as_ref(), "set campaign world info route")?;
    let id = Id::from_str(&campaign_id);
    let lore = parse_lore_route(&route)?;
    state
        .storage()
        .set_world_info_route(&id, entry_index, lore)
        .map_err(TauriCommandError::storage)?;
    refresh_tool_ctx_world_info(state.inner(), &id, "set_campaign_world_info_route");
    Ok(())
}

/// 卡模板世界书（角色卡面板；编辑走 add/update/delete_world_info_entry 与
/// update_world_info_route，条目下标与这里返回的 index 一致）。
/// `character_id` 可为 CharacterStore.id 或 CharacterCard.source_character_id。
#[tauri::command]
pub(crate) fn get_character_world_info(
    character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignWorldInfoDto, TauriCommandError> {
    state
        .storage()
        .require_supported(
            BackendCapability::CharacterCommands,
            "get character world info",
        )
        .map_err(TauriCommandError::validation)?;
    // Gate 4 六审 P1：SQLite 下 CharacterCommands 标为 Supported，但 facade
    // 不构造 JSON CharacterStore——必须经 backend-neutral `get_character`
    // 读取（SQLite 走 V007 characters 表），否则前端角色世界书命令必然失败。
    let stored = state
        .storage()
        .get_character(&character_id)
        .map_err(TauriCommandError::storage)?
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
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignWorldInfoEntryDto, TauriCommandError> {
    state
        .storage()
        .require_supported(
            BackendCapability::CharacterCommands,
            "get character world info entry",
        )
        .map_err(TauriCommandError::validation)?;
    // Gate 4 六审 P1：同上，经 backend-neutral `get_character` 读取。
    let stored = state
        .storage()
        .get_character(&character_id)
        .map_err(TauriCommandError::storage)?
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
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignWorldInfoEntryDto, TauriCommandError> {
    require_world_info_supported(state.inner().as_ref(), "get campaign world info entry")?;
    let id = Id::from_str(&campaign_id);
    let book = state
        .storage()
        .get_world_info(&id)
        .map_err(TauriCommandError::storage)?;
    let entry = book.entries.get(entry_index).ok_or_else(|| {
        TauriCommandError::not_found(format!("世界书条目索引越界: {entry_index}"))
    })?;
    Ok(world_info_entry_to_dto_full(entry_index, entry))
}

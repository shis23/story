use super::super::*;

// ─── 角色卡 DTO（保留 M0）───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInfo {
    #[serde(default)]
    pub source_character_id: Option<String>,
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_mes: String,
    #[serde(default)]
    pub mes_example: String,
    #[serde(default)]
    pub post_history_instructions: String,
    #[serde(default)]
    pub alternate_greetings: Vec<String>,
    pub system_prompt: String,
    pub tags: Vec<String>,
    pub creator: String,
    #[serde(default)]
    pub character_version: String,
    pub spec_version: String,
    #[serde(default)]
    pub extensions: serde_json::Value,
    #[serde(default)]
    pub embedded_world_info: Option<storyforge_domain::world_info::WorldInfoBook>,
    #[serde(default)]
    pub renderable_assets: Option<storyforge_domain::character::RenderableAssets>,
    #[serde(default)]
    pub raw_card_json: serde_json::Value,
    pub has_world_info: bool,
    pub has_renderable_assets: bool,
    pub world_info_count: usize,
    pub world_info_entries: Vec<WorldInfoEntryInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfoEntryInfo {
    pub keys: Vec<String>,
    pub content: String,
    pub constant: bool,
    pub route: String,
    /// 是否全局共享（标记后切其他角色卡也生效，独立于当前角色卡）
    #[serde(default)]
    pub is_global: bool,
    /// ST 深度（0=最底部最重要，对齐 LLM 近因效应）。控制蓝灯常驻条目在导演上下文的排序
    #[serde(default = "default_depth")]
    pub depth: i32,
    /// 排序权重（ST order 字段，depth 相同时按 order）
    #[serde(default = "default_order")]
    pub order: i32,
}

pub(crate) fn default_depth() -> i32 {
    2
}
pub(crate) fn default_order() -> i32 {
    100
}

impl From<&storyforge_domain::character::Character> for CharacterInfo {
    fn from(c: &storyforge_domain::character::Character) -> Self {
        let world_info_entries = c
            .embedded_world_info
            .as_ref()
            .map(|b| {
                b.entries
                    .iter()
                    // 禁用条目（v3 enabled:false / v2 disable:true）保留在域模型里供翻译层
                    // 与 campaign 开关使用，但不进 per-card 注入存储（保持旧行为）
                    .filter(|e| !e.disabled)
                    .map(|e| WorldInfoEntryInfo {
                        keys: e.keys.clone(),
                        content: e.content.clone(),
                        constant: e.constant,
                        route: format!("{:?}", e.route),
                        is_global: false,
                        depth: e.depth,
                        order: e.order,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            source_character_id: Some(c.id.as_str().to_string()),
            name: c.name.clone(),
            description: c.description.clone(),
            personality: c.personality.clone(),
            scenario: c.scenario.clone(),
            first_mes: c.first_mes.clone(),
            mes_example: c.mes_example.clone(),
            post_history_instructions: c.post_history_instructions.clone(),
            alternate_greetings: c.alternate_greetings.clone(),
            system_prompt: c.system_prompt.clone(),
            tags: c.tags.clone(),
            creator: c.creator.clone(),
            character_version: c.character_version.clone(),
            spec_version: c.spec_version.clone(),
            extensions: c.extensions.clone(),
            embedded_world_info: c.embedded_world_info.clone(),
            renderable_assets: c.renderable_assets.clone(),
            raw_card_json: c.raw_card_json.clone(),
            has_world_info: c.embedded_world_info.is_some(),
            has_renderable_assets: c.renderable_assets.is_some(),
            world_info_count: c
                .embedded_world_info
                .as_ref()
                .map(|b| b.entries.len())
                .unwrap_or(0),
            world_info_entries,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub creator: String,
    pub spec_version: String,
    pub world_info_count: usize,
    pub has_renderable_assets: bool,
    pub imported_at: String,
}

/// 统一 StoredCharacter → CharacterSummary 映射（消除重复，R-5）
impl From<storage::StoredCharacter> for CharacterSummary {
    fn from(stored: storage::StoredCharacter) -> Self {
        Self {
            id: stored.id,
            name: stored.info.name,
            description: stored.info.description,
            tags: stored.info.tags,
            creator: stored.info.creator,
            spec_version: stored.info.spec_version,
            world_info_count: stored.info.world_info_count,
            has_renderable_assets: stored.info.has_renderable_assets,
            imported_at: stored.imported_at,
        }
    }
}

// ─── M0 角色卡命令（保留）──────────────────────────────────────────────────

#[tauri::command]
pub(crate) fn import_character(
    data: Vec<u8>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterSummary, TauriCommandError> {
    let character =
        storyforge_infra_import::import_character(&data).map_err(TauriCommandError::from)?;
    let info = CharacterInfo::from(&character);
    let stored = get_store()
        .save(info)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;

    // 同步到 tool_ctx：角色卡 + 世界书（覆盖为当前角色的，符合"当前角色"语义）
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        // 避免重复导入同一张卡导致 characters 列表膨胀
        ctx.characters.retain(|c| c.name != character.name);
        // 提取世界书（内嵌的优先）
        let world_info = character.embedded_world_info.clone();
        ctx.characters.push(Arc::new(character));
        if let Some(wi) = world_info {
            // 先清理该角色之前导入的绿灯世界书条目（防重复累积）
            {
                let all_keywords: Vec<String> = wi
                    .entries
                    .iter()
                    .flat_map(|e| e.keys.iter().cloned())
                    .collect();
                if !all_keywords.is_empty()
                    && let Ok(hits) = state.vector_store.search_by_keywords(&all_keywords, 1000)
                {
                    for hit in hits.into_iter().filter(|h| h.kind == VectorKind::WorldInfo) {
                        let _ = state.vector_store.delete(&hit.id);
                    }
                }
            }
            // 把绿灯世界书条目入库到向量存储（search_vectors 工具关键词搜索用）
            for entry in &wi.entries {
                if !entry.disabled && !entry.constant && !entry.keys.is_empty() {
                    let _ = state.vector_store.upsert(VectorRecord {
                        id: Id::new(),
                        content: entry.content.clone(),
                        vector: vec![], // 关键词路径不需要真实向量，后续接嵌入 API 时补充
                        keywords: entry.keys.clone(),
                        kind: VectorKind::WorldInfo,
                        metadata: std::collections::HashMap::new(),
                    });
                }
            }
            ctx.world_info = Some(Arc::new(wi));
        }
    }

    Ok(CharacterSummary {
        id: stored.id,
        name: stored.info.name,
        description: stored.info.description,
        tags: stored.info.tags,
        creator: stored.info.creator,
        spec_version: stored.info.spec_version,
        world_info_count: stored.info.world_info_count,
        has_renderable_assets: stored.info.has_renderable_assets,
        imported_at: stored.imported_at,
    })
}

#[tauri::command]
pub(crate) fn list_characters() -> Vec<CharacterSummary> {
    get_store()
        .list()
        .into_iter()
        .map(CharacterSummary::from)
        .collect()
}

#[tauri::command]
pub(crate) fn get_character(id: String) -> Result<CharacterInfo, TauriCommandError> {
    stored_character_for_id_or_source_in_store(get_store(), &Id::from_str(&id))
        .map(|stored| stored.info)
        .ok_or_else(|| TauriCommandError::from(format!("角色卡不存在: {id}")))
}

pub(crate) fn delete_character_cascade_source_ids(
    stored_id: &str,
    stored_name: Option<&str>,
    stored_source_character_id: Option<&str>,
    characters: &[Arc<storyforge_domain::character::Character>],
) -> Vec<Id> {
    let mut ids = vec![Id::from_str(stored_id)];
    if let Some(source_id) = stored_source_character_id {
        let source_id = Id::from_str(source_id);
        if !ids.iter().any(|id| id == &source_id) {
            ids.push(source_id);
        }
    }
    if let Some(name) = stored_name
        && let Some(character) = characters.iter().find(|c| c.name == name)
        && !ids.iter().any(|id| id == &character.id)
    {
        ids.push(character.id.clone());
    }
    ids
}

#[tauri::command]
pub(crate) fn delete_character(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // 先取出 name（用于同步 tool_ctx）
    let stored = get_store().get(&id);
    let name = stored.as_ref().map(|s| s.info.name.clone());
    let stored_source_character_id = stored
        .as_ref()
        .and_then(|s| s.info.source_character_id.as_deref());
    let source_ids = {
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        delete_character_cascade_source_ids(
            &id,
            name.as_deref(),
            stored_source_character_id,
            &ctx.characters,
        )
    };
    if !get_store()
        .delete(&id)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        return Err(TauriCommandError::not_found(format!("角色卡不存在: {id}")));
    }
    // 同步从 tool_ctx 移除
    if let Some(name) = name {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.characters.retain(|c| c.name != name);
        // 如果移除的是当前世界书来源角色，清空 world_info
        // （简单处理：characters 空了就清 world_info）
        if ctx.characters.is_empty() {
            ctx.world_info = None;
        }
    }
    // 级联删除：该卡的 MVU 翻译 + CampaignStore 的 CharacterCard（含其所有 Campaign）
    //
    // 注意 id 语义：delete_character 的 `id` 是 StoredCharacter.id（存储层 UUID），
    // 而 CharacterCard.source_character_id 是 Character.id（domain 层 UUID，导入时生成）。
    // 两者常不同。新数据使用 CharacterInfo.source_character_id；旧数据兼容 StoredCharacter.id
    // 以及同会话 tool_ctx.characters 中按角色名找到的 Character.id。
    for source_id in &source_ids {
        // #22：MVU 翻译级联删除按后端分流（SQLite → mvu_translations 表）
        if sqlite_runtime::is_sqlite_active() {
            if let Err(e) = sqlite_runtime::delete_mvu(source_id) {
                tracing::warn!("SQLite MVU 翻译级联删除失败（{source_id}）: {e}");
            }
        } else {
            let _ = get_campaign_store().delete_mvu(source_id);
        }
        // 尝试用 StoredCharacter.id 直接查（旧路径，可能命中）
        if let Some(stored_card) = get_campaign_store().get_card_by_source(source_id) {
            // 桥接：通过角色名找到 Character.id，再查 card
            let _ = get_campaign_store().delete_card(&stored_card.card.id);
        }
    }
    // 级联删除：清理向量库中该角色相关的记录（M-2）
    for source_id in &source_ids {
        if let Err(e) = state.vector_store.delete_by_character(source_id) {
            tracing::warn!("清理角色向量记录失败: {e}");
        }
    }
    Ok(())
}

/// 更新世界书条目路由（蓝灯/绿灯/Both/Disabled）
#[tauri::command]
pub(crate) fn update_world_info_route(
    character_id: String,
    entry_index: usize,
    route: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // 验证路由值合法
    match route.as_str() {
        "Constant" | "Selective" | "Both" | "Disabled" => {}
        other => {
            return Err(TauriCommandError::from(format!(
                "无效路由: {other}，应为 Constant/Selective/Both/Disabled"
            )));
        }
    }

    get_store().update_world_info_route(&character_id, entry_index, &route)?;

    // 同步更新 tool_ctx 中的世界书路由
    if let Some(stored) = get_store().get(&character_id)
        && stored.info.world_info_entries.get(entry_index).is_some()
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        if let Some(ref world_info) = ctx.world_info {
            let mut new_book = (**world_info).clone();
            if let Some(entry) = new_book.entries.get_mut(entry_index) {
                entry.route = match route.as_str() {
                    "Constant" => storyforge_domain::world_info::LoreRoute::Constant,
                    "Selective" => storyforge_domain::world_info::LoreRoute::Selective,
                    "Both" => storyforge_domain::world_info::LoreRoute::Both,
                    "Disabled" => storyforge_domain::world_info::LoreRoute::Disabled,
                    _ => unreachable!(),
                };
            }
            ctx.world_info = Some(Arc::new(new_book));
        }
    }

    Ok(())
}

/// 更新世界书条目的 keys/content/constant/is_global/depth/order
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_world_info_entry(
    character_id: String,
    entry_index: usize,
    keys: Vec<String>,
    content: String,
    constant: bool,
    is_global: bool,
    depth: i32,
    order: i32,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    get_store().update_world_info_entry(
        &character_id,
        entry_index,
        keys.clone(),
        content.clone(),
        constant,
        is_global,
        depth,
        order,
    )?;

    // 同步 tool_ctx 的世界书（含全局条目 merge）
    rebuild_world_info_in_tool_ctx(&state);
    Ok(())
}

/// 新增世界书条目，返回新索引
#[tauri::command]
pub(crate) fn add_world_info_entry(
    character_id: String,
    keys: Vec<String>,
    content: String,
    constant: bool,
    is_global: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let new_index = get_store().add_world_info_entry(
        &character_id,
        keys.clone(),
        content.clone(),
        constant,
        is_global.unwrap_or(false),
    )?;

    // 同步 tool_ctx（含全局条目 merge）+ 绿灯条目入向量库
    rebuild_world_info_in_tool_ctx(&state);
    // 新增的绿灯条目入向量库
    if !constant && !keys.is_empty() {
        let _ = state.vector_store.upsert(VectorRecord {
            id: Id::new(),
            content,
            vector: vec![],
            keywords: keys,
            kind: VectorKind::WorldInfo,
            metadata: std::collections::HashMap::new(),
        });
    }
    Ok(new_index)
}

/// 删除世界书条目
#[tauri::command]
pub(crate) fn delete_world_info_entry(
    character_id: String,
    entry_index: usize,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    get_store().delete_world_info_entry(&character_id, entry_index)?;
    rebuild_world_info_in_tool_ctx(&state);
    Ok(())
}

/// 把存储里的世界书重新同步到 tool_ctx（编辑/新增/删除后调用）
/// 重建 tool_ctx 的世界书（含全局条目 merge）
///
/// 规则：
/// - 当前活跃角色卡的所有条目都进 tool_ctx
/// - **所有其他角色卡**里 `is_global=true` 的条目也 merge 进来（全局共享）
/// - 蓝灯（Constant/Both）条目进导演常驻上下文
/// - 绿灯（Selective/Both）条目进向量检索池
pub(crate) fn rebuild_world_info_in_tool_ctx(state: &tauri::State<'_, Arc<AppState>>) {
    use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

    // 有活跃活动时以本局世界书为唯一注入源（卡库写路径 rebuild 不应覆盖）
    if !sqlite_runtime::is_sqlite_active() {
        let active = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(campaign_id) = active
            && let Ok(book) = get_campaign_store().get_world_info(&campaign_id)
            && !book.entries.is_empty()
        {
            apply_campaign_world_info_to_tool_ctx(state.inner(), &campaign_id, &book);
            return;
        }
    }

    let all_chars = get_store().list();

    // 找当前活跃角色名（tool_ctx.characters 里的）
    let active_names: Vec<String> = state
        .tool_ctx
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .characters
        .iter()
        .map(|c| c.name.clone())
        .collect();

    // 收集条目：活跃角色的全部 + 其他角色的 is_global 条目
    let mut entries: Vec<WorldInfoEntry> = Vec::new();
    for stored in &all_chars {
        let is_active = active_names.iter().any(|n| n == &stored.info.name);
        for e in &stored.info.world_info_entries {
            // 活跃角色的条目全收；非活跃角色只收 is_global 的
            if !is_active && !e.is_global {
                continue;
            }
            let route = match e.route.as_str() {
                "Constant" => LoreRoute::Constant,
                "Selective" => LoreRoute::Selective,
                "Both" => LoreRoute::Both,
                "Disabled" => LoreRoute::Disabled,
                _ => LoreRoute::Selective,
            };
            entries.push(WorldInfoEntry {
                st_id: None,
                keys: e.keys.clone(),
                secondary_keys: vec![],
                content: e.content.clone(),
                constant: e.constant,
                selective: !e.constant,
                selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: e.depth,
                order: e.order,
                route,
                extensions: serde_json::json!({}),
                extra: Default::default(),
            });
        }
    }

    let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    if entries.is_empty() {
        ctx.world_info = None;
    } else {
        let book = WorldInfoBook {
            entries,
            source: storyforge_domain::Source::Native,
            metadata: Default::default(),
        };
        ctx.world_info = Some(Arc::new(book));
    }
}

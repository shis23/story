use super::super::*;
use crate::storage_backend::{BackendCapability, CapabilityStatus};

fn require_character_commands(state: &AppState, operation: &str) -> Result<(), TauriCommandError> {
    state
        .storage()
        .require_supported(BackendCapability::CharacterCommands, operation)
        .map_err(TauriCommandError::validation)
}

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
    require_character_commands(state.inner().as_ref(), "import character")?;
    let character =
        storyforge_infra_import::import_character(&data).map_err(TauriCommandError::from)?;
    let info = CharacterInfo::from(&character);
    let stored = state
        .storage()
        .save_character(info)
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
pub(crate) fn list_characters(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<CharacterSummary>, TauriCommandError> {
    require_character_commands(state.inner().as_ref(), "list characters")?;
    Ok(state
        .storage()
        .list_characters()
        .map_err(TauriCommandError::storage)?
        .into_iter()
        .map(CharacterSummary::from)
        .collect())
}

#[tauri::command]
pub(crate) fn get_character(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInfo, TauriCommandError> {
    require_character_commands(state.inner().as_ref(), "get character")?;
    state
        .storage()
        .get_character(&id)
        .map_err(TauriCommandError::storage)?
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
    state
        .storage()
        .require_supported(
            BackendCapability::CharacterCommands,
            "character delete cascade",
        )
        .map_err(TauriCommandError::validation)?;
    // 先取出 name（用于同步 tool_ctx）
    let stored = state
        .storage()
        .get_character(&id)
        .map_err(TauriCommandError::storage)?;
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
    // 整个删除（角色库 + MVU + 卡 + Campaign 全量级联）在一个原子操作内完成，
    // 错误传播给调用方（Gate 4 三审 P1：不得报告成功却只删了一半）。
    //
    // 删除前先取该角色关联的 Campaign ids（用于清活跃指针 + 失效会话缓存）。
    // 候选 source 覆盖：存储 id / 源卡 id / 卡实际 source_character_id。
    let affected_campaign_ids = {
        let mut candidates: Vec<Id> = source_ids.clone();
        if let Some(s) = stored_source_character_id {
            let s_id = Id::from_str(s);
            if !candidates.iter().any(|c| c == &s_id) {
                candidates.push(s_id);
            }
        }
        let id_id = Id::from_str(&id);
        if !candidates.iter().any(|c| c == &id_id) {
            candidates.push(id_id);
        }
        let mut ids = Vec::new();
        for source_id in &candidates {
            if let Ok(Some(stored_card)) = state.storage().get_card_by_source(source_id)
                && let Ok(campaigns) = state.storage().list_campaigns(Some(&stored_card.card.id))
            {
                ids.extend(campaigns.into_iter().map(|r| r.campaign.id));
            }
        }
        ids
    };
    // 被删 Campaign 绑定的会话 ids（删除前收集；删除后 campaign 已不存在）。
    let affected_conv_ids: Vec<Id> = affected_campaign_ids
        .iter()
        .filter_map(|campaign_id| {
            state
                .storage()
                .get_campaign(campaign_id)
                .ok()
                .flatten()
                .and_then(|r| r.campaign.conversation_id)
        })
        .collect();
    let removed = state
        .storage()
        .delete_character_full_cascade(&id, &source_ids)
        .map_err(|e| TauriCommandError::storage(format!("删除失败（已整体回滚）: {e}")))?;
    if !removed {
        return Err(TauriCommandError::not_found(format!("角色卡不存在: {id}")));
    }
    // 应用内状态清理（Gate 4 四审 P1）：删除成功后必须同步内存态，否则
    // ConversationStore 会持续返回已删除会话的缓存、活跃指针残留、后续
    // 修改甚至可能把已删数据重新写回。
    let _update = state
        .active_campaign_update
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut active_campaign = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(active_id) = active_campaign.as_ref()
        && affected_campaign_ids.iter().any(|id| id == active_id)
    {
        crate::backend_workflows::save_active_pointer(state.storage(), None)
            .map_err(|e| TauriCommandError::storage(format!("清除活跃活动指针失败: {e}")))?;
        *active_campaign = None;
    }
    drop(active_campaign);
    drop(_update);
    // 删除被删 Campaign 绑定的会话（文件 + 缓存），避免 ConversationStore
    // 持续返回已从存储删除的会话、后续修改重新写回。
    for conv_id in &affected_conv_ids {
        if let Err(e) = state.conv_store.delete(conv_id) {
            tracing::warn!("删除角色后清理会话失败 conv={conv_id}: {e}");
        }
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
    // 内存态世界书重建（角色库已删除，t_ctx 内世界书条目需要清理）
    // 注意：旧实现引用的 vector_store.delete_by_character 不存在（无效调用），
    // 现改为显式不执行 + 记录。SQLite 角色库删除已含全量持久化级联。
    rebuild_world_info_in_tool_ctx(&state);
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
    require_character_commands(state.inner().as_ref(), "update character world info route")?;
    // 验证路由值合法
    match route.as_str() {
        "Constant" | "Selective" | "Both" | "Disabled" => {}
        other => {
            return Err(TauriCommandError::from(format!(
                "无效路由: {other}，应为 Constant/Selective/Both/Disabled"
            )));
        }
    }

    state
        .storage()
        .update_character_world_info_route(&character_id, entry_index, &route)?;

    // 同步更新 tool_ctx 中的世界书路由
    if let Ok(Some(stored)) = state.storage().get_character(&character_id)
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
    require_character_commands(state.inner().as_ref(), "update character world info entry")?;
    state.storage().update_character_world_info_entry(
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
    require_character_commands(state.inner().as_ref(), "add character world info entry")?;
    let new_index = state.storage().add_character_world_info_entry(
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
    require_character_commands(state.inner().as_ref(), "delete character world info entry")?;
    state
        .storage()
        .delete_character_world_info_entry(&character_id, entry_index)?;
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
    if require_character_commands(state.inner().as_ref(), "rebuild character world info").is_err() {
        return;
    }
    use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

    // 有活跃活动时以本局世界书为唯一注入源（卡库写路径 rebuild 不应覆盖）。
    // 经 backend-neutral facade 读取（SQLite 走 V006 表），不依赖 JSON store
    // （Gate 4 三审 P2：SQLite 下编辑落库后 tool_ctx 必须刷新）。
    if state.storage().capability(BackendCapability::WorldInfo) == CapabilityStatus::Supported {
        let active = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(campaign_id) = active {
            match state.storage().get_world_info(&campaign_id) {
                Ok(book) if !book.entries.is_empty() => {
                    apply_campaign_world_info_to_tool_ctx(state.inner(), &campaign_id, &book);
                    return;
                }
                Ok(_) => {
                    // 本局世界书为空：回退角色库模板合并。
                }
                Err(e) => {
                    // Gate 4 四审 P2：世界书读取失败不得静默吞掉——告警后
                    // 回退角色库世界书（而非假装读到空书）。
                    tracing::warn!(
                        "rebuild world info: 读取活动 Campaign 世界书失败 campaign={}: {e}",
                        campaign_id
                    );
                }
            }
        }
    }

    let all_chars = match state.storage().list_characters() {
        Ok(chars) => chars,
        Err(e) => {
            tracing::warn!("rebuild world info: 角色库读取失败: {e}");
            return;
        }
    };

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

// ─── 世界书编辑纯变换（SQLite 路径经 facade 复用；JSON 路径走 CharacterStore）──
//
// 这些纯函数与 `CharacterStore` 的内联变换保持字节级相同的语义与错误消息
// （索引越界 / 新增条目默认路由 / 计数刷新由 `mutate_character` 统一完成）。

/// 更新世界书条目路由（entry.route）。
pub(crate) fn apply_world_info_route_update(
    info: &mut CharacterInfo,
    entry_index: usize,
    new_route: &str,
) -> Result<(), String> {
    let entry = info
        .world_info_entries
        .get_mut(entry_index)
        .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;
    entry.route = new_route.to_string();
    Ok(())
}

/// 更新世界书条目的 keys / content / constant / is_global / depth / order。
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_world_info_entry_update(
    info: &mut CharacterInfo,
    entry_index: usize,
    keys: Vec<String>,
    content: String,
    constant: bool,
    is_global: bool,
    depth: i32,
    order: i32,
) -> Result<(), String> {
    let entry = info
        .world_info_entries
        .get_mut(entry_index)
        .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;
    entry.keys = keys;
    entry.content = content;
    entry.constant = constant;
    entry.is_global = is_global;
    entry.depth = depth;
    entry.order = order;
    Ok(())
}

/// 新增世界书条目，返回新条目的索引。默认路由：蓝灯→Constant，绿灯→Selective。
pub(crate) fn apply_add_world_info_entry(
    info: &mut CharacterInfo,
    keys: Vec<String>,
    content: String,
    constant: bool,
    is_global: bool,
) -> Result<usize, String> {
    let route = if constant { "Constant" } else { "Selective" }.to_string();
    let entry = WorldInfoEntryInfo {
        keys,
        content,
        constant,
        route,
        is_global,
        depth: 2,
        order: 100,
    };
    info.world_info_entries.push(entry);
    Ok(info.world_info_entries.len() - 1)
}

/// 删除世界书条目。
pub(crate) fn apply_delete_world_info_entry(
    info: &mut CharacterInfo,
    entry_index: usize,
) -> Result<(), String> {
    if entry_index >= info.world_info_entries.len() {
        return Err(format!("世界书条目索引越界: {entry_index}"));
    }
    info.world_info_entries.remove(entry_index);
    Ok(())
}

/// 批量替换世界书条目（meta_accept_patch 持久化用）。
pub(crate) fn apply_update_world_info_entries_bulk(
    info: &mut CharacterInfo,
    entries: Vec<WorldInfoEntryInfo>,
) -> Result<(), String> {
    info.world_info_entries = entries;
    Ok(())
}

#[cfg(test)]
mod world_info_transform_tests {
    use super::*;

    fn info_with_entries(count: usize) -> CharacterInfo {
        let mut info = sample_info("Transform Hero");
        info.world_info_entries = (0..count)
            .map(|i| WorldInfoEntryInfo {
                keys: vec![format!("key{i}")],
                content: format!("content{i}"),
                constant: false,
                route: "Selective".into(),
                is_global: false,
                depth: 2,
                order: 100,
            })
            .collect();
        info
    }

    fn sample_info(name: &str) -> CharacterInfo {
        CharacterInfo {
            source_character_id: Some("source-transform".into()),
            name: name.into(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            post_history_instructions: String::new(),
            alternate_greetings: vec![],
            system_prompt: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
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

    #[test]
    fn route_update_mutates_only_route() {
        let mut info = info_with_entries(1);
        apply_world_info_route_update(&mut info, 0, "Constant").unwrap();
        assert_eq!(info.world_info_entries[0].route, "Constant");
        assert_eq!(info.world_info_entries[0].keys, vec!["key0"]);
        assert!(apply_world_info_route_update(&mut info, 3, "Constant").is_err());
    }

    #[test]
    fn entry_update_replaces_fields() {
        let mut info = info_with_entries(1);
        apply_world_info_entry_update(
            &mut info,
            0,
            vec!["new".into()],
            "new content".into(),
            true,
            true,
            5,
            9,
        )
        .unwrap();
        let entry = &info.world_info_entries[0];
        assert_eq!(entry.keys, vec!["new"]);
        assert_eq!(entry.content, "new content");
        assert!(entry.constant && entry.is_global);
        assert_eq!((entry.depth, entry.order), (5, 9));
        assert!(
            apply_world_info_entry_update(&mut info, 7, vec![], String::new(), false, false, 0, 0)
                .is_err()
        );
    }

    #[test]
    fn add_entry_defaults_route_depth_and_order() {
        let mut info = info_with_entries(0);
        let idx = apply_add_world_info_entry(&mut info, vec!["k".into()], "c".into(), true, true)
            .unwrap();
        assert_eq!(idx, 0);
        let entry = &info.world_info_entries[0];
        assert_eq!(entry.route, "Constant");
        assert_eq!((entry.depth, entry.order), (2, 100));
        assert!(entry.is_global);
        let idx =
            apply_add_world_info_entry(&mut info, vec!["k2".into()], "c2".into(), false, false)
                .unwrap();
        assert_eq!(idx, 1);
        assert_eq!(info.world_info_entries[1].route, "Selective");
    }

    #[test]
    fn delete_entry_removes_and_rejects_out_of_range() {
        let mut info = info_with_entries(2);
        apply_delete_world_info_entry(&mut info, 0).unwrap();
        assert_eq!(info.world_info_entries.len(), 1);
        assert_eq!(info.world_info_entries[0].keys, vec!["key1"]);
        assert!(apply_delete_world_info_entry(&mut info, 5).is_err());
    }

    #[test]
    fn bulk_replace_swaps_the_whole_list() {
        let mut info = info_with_entries(2);
        apply_update_world_info_entries_bulk(&mut info, vec![]).unwrap();
        assert!(info.world_info_entries.is_empty());
    }
}

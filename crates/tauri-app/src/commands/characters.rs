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
            ctx.world_info = Some(Arc::new(wi));
        }
    }
    // 绿灯世界书条目入向量库（search_vectors 用）：按角色库全量重建，取代旧的
    // 「按关键词删旧条目 + 逐条 upsert」（见 vector_index::sync_world_info_vectors）。
    crate::vector_index::sync_world_info_vectors(state.inner());

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
pub fn delete_character(
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
    // 收集错误传播（Gate 4 五审 P2：不得用 if let Ok / .ok() 吞掉——否则删除
    // 成功却漏清活跃指针/会话缓存，出现与四审 P1 同类的半成功状态）。
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
            if let Some(stored_card) =
                state.storage().get_card_by_source(source_id).map_err(|e| {
                    TauriCommandError::storage(format!(
                        "收集受影响 Campaign 时反查卡失败 source={source_id}: {e}"
                    ))
                })?
            {
                let campaigns = state
                    .storage()
                    .list_campaigns(Some(&stored_card.card.id))
                    .map_err(|e| {
                        TauriCommandError::storage(format!(
                            "收集受影响 Campaign 时列出 Campaign 失败 card={}: {e}",
                            stored_card.card.id
                        ))
                    })?;
                ids.extend(campaigns.into_iter().map(|r| r.campaign.id));
            }
        }
        ids
    };
    // 被删 Campaign 绑定的会话 ids（删除前收集；删除后 campaign 已不存在）。
    // 读取失败传播，同样不做静默跳过。
    let affected_conv_ids: Vec<Id> = affected_campaign_ids
        .iter()
        .map(|campaign_id| {
            Ok::<_, TauriCommandError>(
                state
                    .storage()
                    .get_campaign(campaign_id)
                    .map_err(|e| {
                        TauriCommandError::storage(format!(
                            "收集受影响会话时读取 Campaign 失败 campaign={campaign_id}: {e}"
                        ))
                    })?
                    .and_then(|r| r.campaign.conversation_id),
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    // JSON 角色库只按存储 id 删除：调用方传的是源 Character.id（角色卡面板只知道
    // source_character_id）时，先换成解析到的存储 id，否则级联删了卡、主记录却没删。
    let target_id = stored
        .as_ref()
        .map(|s| s.id.clone())
        .unwrap_or_else(|| id.clone());
    let removed = state
        .storage()
        .delete_character_full_cascade(&target_id, &source_ids)
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
    // 级联删掉的 Campaign 的远记忆向量同样要清（派生索引，失败只告警）。
    for campaign_id in &affected_campaign_ids {
        crate::vector_index::purge_campaign_vectors(state.inner(), campaign_id);
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
    // 内存态世界书重建（角色库已删除，t_ctx 内世界书条目需要清理）。
    // `VectorStore::delete_by_character` 按 `owner_character_id`（角色私有知识）
    // 删除，与角色库世界书向量无关，这里不调用。
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
    // 先解析为类型化 LoreRoute 再落库/更新内存：与 world_info 命令共用
    // parse_lore_route（大小写兼容），且存储与 tool_ctx 消费同一份值，
    // 不存在「落库字符串与 unreachable! 映射漂移」的隐患（2026-09-01 审查收口）。
    let parsed_route = crate::commands::world_info::parse_lore_route(route.as_str())?;
    let route_canonical = crate::commands::world_info::lore_route_to_str(&parsed_route);

    state.storage().update_character_world_info_route(
        &character_id,
        entry_index,
        &route_canonical,
    )?;

    // 同步 tool_ctx（含全局条目 merge）+ 检索池向量重建。旧实现按 entry_index 直接
    // 改 tool_ctx 里的合并书——那本书混着多张卡与全局条目，下标对不上；检索池也
    // 不跟着变（改成 Constant / Disabled 的条目仍能被 search_vectors 搜到）。
    rebuild_world_info_in_tool_ctx(&state);
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

    // 同步 tool_ctx（含全局条目 merge）+ 绿灯条目向量重建
    rebuild_world_info_in_tool_ctx(&state);
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
    // 角色库世界书变了（增删改条目、改路由、删卡/删角色）：检索池向量跟着重建，
    // 必须放在下面「活跃活动早退」之前——向量池与本局注入源无关。
    crate::vector_index::sync_world_info_vectors(state.inner());
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

// ─── 卡模板世界书编辑（两个后端共用的纯变换）──────────────────────────────
//
// 卡模板世界书的权威是完整的 `embedded_world_info`（含禁用条目、次级关键词、
// 注释等 ST 字段）：角色卡面板、新开局的种子、导出都读它。`world_info_entries`
// 只是它的投影（去掉禁用条目，供非 Campaign 写作路径与检索池使用），每次编辑后
// 重新生成。2026-10-11 审计前编辑命令只改投影：面板看不到、新开局也拿不到修改。
//
// 条目下标 = 完整世界书里的下标（与 `get_character_world_info` 返回的 index 一致）。
// 「全局共享」标记存在条目扩展 `sf_global` 上，投影时还原为 `is_global`。

const GLOBAL_FLAG: &str = "sf_global";

pub(crate) fn entry_is_global(entry: &storyforge_domain::world_info::WorldInfoEntry) -> bool {
    entry
        .extensions
        .get(GLOBAL_FLAG)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn set_entry_extension(
    entry: &mut storyforge_domain::world_info::WorldInfoEntry,
    key: &str,
    value: serde_json::Value,
) {
    if !entry.extensions.is_object() {
        entry.extensions = serde_json::json!({});
    }
    if let Some(map) = entry.extensions.as_object_mut() {
        map.insert(key.to_string(), value);
    }
}

fn set_entry_global(entry: &mut storyforge_domain::world_info::WorldInfoEntry, is_global: bool) {
    if is_global || entry_is_global(entry) {
        set_entry_extension(entry, GLOBAL_FLAG, serde_json::Value::Bool(is_global));
    }
}

fn route_name(route: &storyforge_domain::world_info::LoreRoute) -> String {
    format!("{route:?}")
}

fn projected_entry(entry: &storyforge_domain::world_info::WorldInfoEntry) -> WorldInfoEntryInfo {
    WorldInfoEntryInfo {
        keys: entry.keys.clone(),
        content: entry.content.clone(),
        constant: entry.constant,
        route: route_name(&entry.route),
        is_global: entry_is_global(entry),
        depth: entry.depth,
        order: entry.order,
    }
}

fn user_entry_from_info(
    info: &WorldInfoEntryInfo,
) -> storyforge_domain::world_info::WorldInfoEntry {
    let mut entry = world_info_entry_from_info(info);
    set_entry_extension(&mut entry, "sf_source", serde_json::json!("user"));
    set_entry_global(&mut entry, info.is_global);
    entry
}

/// 刷新世界书计数（两个后端的写路径在变换后统一调用）。
pub(crate) fn refresh_world_info_counts(info: &mut CharacterInfo) {
    info.world_info_count = info
        .embedded_world_info
        .as_ref()
        .map(|book| book.entries.len())
        .unwrap_or(info.world_info_entries.len());
    info.has_world_info = info.world_info_count > 0;
}

/// 取可编辑的完整世界书。旧数据里投影可能与完整书分叉（只改了投影的旧编辑路径、
/// 没有内嵌书的旧卡）：投影里有、书里没有的条目并回书里，全局标记同步到书上，
/// 绝不因为这次编辑丢掉内容。对未分叉的数据是空操作。
fn card_book_mut(info: &mut CharacterInfo) -> &mut storyforge_domain::world_info::WorldInfoBook {
    let projection = std::mem::take(&mut info.world_info_entries);
    let book = info.embedded_world_info.get_or_insert_with(|| {
        storyforge_domain::world_info::WorldInfoBook {
            entries: Vec::new(),
            source: storyforge_domain::Source::Native,
            metadata: Default::default(),
        }
    });
    let mut matched = vec![false; book.entries.len()];
    for projected in &projection {
        let found = book.entries.iter().enumerate().position(|(i, entry)| {
            !matched[i] && entry.keys == projected.keys && entry.content == projected.content
        });
        match found {
            Some(i) => {
                matched[i] = true;
                set_entry_global(&mut book.entries[i], projected.is_global);
            }
            None => book.entries.push(user_entry_from_info(projected)),
        }
    }
    info.world_info_entries = projection;
    info.embedded_world_info
        .as_mut()
        .expect("book materialized above")
}

/// 从完整世界书重新生成投影（去掉禁用条目）。
pub(crate) fn reproject_world_info(info: &mut CharacterInfo) {
    info.world_info_entries = info
        .embedded_world_info
        .as_ref()
        .map(|book| {
            book.entries
                .iter()
                .filter(|entry| !entry.disabled)
                .map(projected_entry)
                .collect()
        })
        .unwrap_or_default();
    refresh_world_info_counts(info);
}

fn book_entry_mut(
    book: &mut storyforge_domain::world_info::WorldInfoBook,
    entry_index: usize,
) -> Result<&mut storyforge_domain::world_info::WorldInfoEntry, String> {
    book.entries
        .get_mut(entry_index)
        .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))
}

/// 更新世界书条目路由。路由为 Disabled 时条目同时标记禁用，其它路由解除禁用。
pub(crate) fn apply_world_info_route_update(
    info: &mut CharacterInfo,
    entry_index: usize,
    new_route: &str,
) -> Result<(), String> {
    let route =
        crate::commands::world_info::parse_lore_route(new_route).map_err(|e| e.to_string())?;
    let entry = book_entry_mut(card_book_mut(info), entry_index)?;
    entry.disabled = matches!(route, storyforge_domain::world_info::LoreRoute::Disabled);
    entry.route = route;
    reproject_world_info(info);
    Ok(())
}

/// 更新世界书条目的 keys / content / constant / is_global / depth / order
/// （其余 ST 字段——次级关键词、注释、位置等——原样保留）。
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
    let entry = book_entry_mut(card_book_mut(info), entry_index)?;
    entry.keys = keys;
    entry.content = content;
    entry.constant = constant;
    entry.selective = !constant;
    entry.depth = depth;
    entry.order = order;
    set_entry_global(entry, is_global);
    reproject_world_info(info);
    Ok(())
}

/// 新增世界书条目，返回新条目在完整世界书里的下标。默认路由：蓝灯→Constant，绿灯→Selective。
pub(crate) fn apply_add_world_info_entry(
    info: &mut CharacterInfo,
    keys: Vec<String>,
    content: String,
    constant: bool,
    is_global: bool,
) -> Result<usize, String> {
    let route = if constant { "Constant" } else { "Selective" }.to_string();
    let entry = user_entry_from_info(&WorldInfoEntryInfo {
        keys,
        content,
        constant,
        route,
        is_global,
        depth: default_depth(),
        order: default_order(),
    });
    let book = card_book_mut(info);
    book.entries.push(entry);
    let index = book.entries.len() - 1;
    reproject_world_info(info);
    Ok(index)
}

/// 删除世界书条目。
pub(crate) fn apply_delete_world_info_entry(
    info: &mut CharacterInfo,
    entry_index: usize,
) -> Result<(), String> {
    let book = card_book_mut(info);
    if entry_index >= book.entries.len() {
        return Err(format!("世界书条目索引越界: {entry_index}"));
    }
    book.entries.remove(entry_index);
    reproject_world_info(info);
    Ok(())
}

/// 批量替换世界书投影（meta_accept_patch 持久化用），并让完整世界书跟上：
/// 与新投影条目（keys + 正文）相同的旧条目原样复用（保留 ST 字段），只更新路由 /
/// 层级 / 全局标记；新条目追加；不在新投影里的启用条目删除；禁用条目原样保留。
pub(crate) fn apply_update_world_info_entries_bulk(
    info: &mut CharacterInfo,
    entries: Vec<WorldInfoEntryInfo>,
) -> Result<(), String> {
    let old_entries = info
        .embedded_world_info
        .as_ref()
        .map(|book| book.entries.clone())
        .unwrap_or_default();
    let mut used = vec![false; old_entries.len()];
    let mut next = Vec::with_capacity(entries.len());
    for projected in &entries {
        let found = old_entries.iter().enumerate().position(|(i, entry)| {
            !used[i]
                && !entry.disabled
                && entry.keys == projected.keys
                && entry.content == projected.content
        });
        let entry = match found {
            Some(i) => {
                used[i] = true;
                let mut entry = old_entries[i].clone();
                let fresh = world_info_entry_from_info(projected);
                entry.constant = fresh.constant;
                entry.selective = fresh.selective;
                entry.route = fresh.route;
                entry.depth = fresh.depth;
                entry.order = fresh.order;
                set_entry_global(&mut entry, projected.is_global);
                entry
            }
            None => user_entry_from_info(projected),
        };
        next.push(entry);
    }
    next.extend(
        old_entries
            .into_iter()
            .zip(used)
            .filter(|(entry, used)| !used && entry.disabled)
            .map(|(entry, _)| entry),
    );
    let book = info.embedded_world_info.get_or_insert_with(|| {
        storyforge_domain::world_info::WorldInfoBook {
            entries: Vec::new(),
            source: storyforge_domain::Source::Native,
            metadata: Default::default(),
        }
    });
    book.entries = next;
    info.world_info_entries = entries;
    refresh_world_info_counts(info);
    Ok(())
}

#[cfg(test)]
mod world_info_transform_tests {
    use super::*;
    use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

    fn book_entry(i: usize, disabled: bool) -> WorldInfoEntry {
        let mut entry = world_info_entry_from_info(&WorldInfoEntryInfo {
            keys: vec![format!("key{i}")],
            content: format!("content{i}"),
            constant: false,
            route: "Selective".into(),
            is_global: false,
            depth: 2,
            order: 100,
        });
        entry.secondary_keys = vec![format!("secondary{i}")];
        entry.disabled = disabled;
        if disabled {
            entry.route = LoreRoute::Disabled;
        }
        entry
    }

    /// 导入后的样子：完整书（第 1 条禁用）+ 去掉禁用条目的投影。
    fn imported_info() -> CharacterInfo {
        let mut info = sample_info("Transform Hero");
        info.embedded_world_info = Some(WorldInfoBook {
            entries: vec![
                book_entry(0, false),
                book_entry(1, true),
                book_entry(2, false),
            ],
            source: storyforge_domain::Source::Native,
            metadata: Default::default(),
        });
        reproject_world_info(&mut info);
        info
    }

    fn book(info: &CharacterInfo) -> &WorldInfoBook {
        info.embedded_world_info.as_ref().unwrap()
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
    fn edits_land_in_the_full_book_and_the_projection_follows() {
        let mut info = imported_info();
        assert_eq!(info.world_info_entries.len(), 2, "投影不含禁用条目");
        assert_eq!(info.world_info_count, 3);

        // 下标是完整书里的下标：2 = 第三条（投影里的第二条）
        apply_world_info_entry_update(
            &mut info,
            2,
            vec!["new".into()],
            "new content".into(),
            true,
            true,
            5,
            9,
        )
        .unwrap();
        let edited = &book(&info).entries[2];
        assert_eq!(edited.keys, vec!["new"]);
        assert_eq!(
            edited.secondary_keys,
            vec!["secondary2"],
            "其余 ST 字段保留"
        );
        assert!(edited.constant && entry_is_global(edited));
        let projected = &info.world_info_entries[1];
        assert_eq!(projected.content, "new content");
        assert!(projected.is_global);
        assert_eq!((projected.depth, projected.order), (5, 9));
        assert!(
            apply_world_info_entry_update(&mut info, 7, vec![], String::new(), false, false, 0, 0)
                .is_err()
        );
    }

    #[test]
    fn route_update_toggles_disabled_with_the_route() {
        let mut info = imported_info();
        apply_world_info_route_update(&mut info, 1, "Constant").unwrap();
        assert!(
            !book(&info).entries[1].disabled,
            "改成非 Disabled 路由即启用"
        );
        assert_eq!(info.world_info_entries.len(), 3);
        apply_world_info_route_update(&mut info, 0, "Disabled").unwrap();
        assert!(book(&info).entries[0].disabled);
        assert_eq!(info.world_info_entries.len(), 2);
        assert!(apply_world_info_route_update(&mut info, 9, "Constant").is_err());
        assert!(apply_world_info_route_update(&mut info, 0, "nonsense").is_err());
    }

    #[test]
    fn add_and_delete_use_full_book_indexes() {
        let mut info = imported_info();
        let idx = apply_add_world_info_entry(&mut info, vec!["k".into()], "c".into(), true, true)
            .unwrap();
        assert_eq!(idx, 3);
        let added = &book(&info).entries[3];
        assert_eq!(added.route, LoreRoute::Constant);
        assert_eq!((added.depth, added.order), (2, 100));
        assert!(entry_is_global(added));
        assert_eq!(added.extensions["sf_source"], "user");
        assert_eq!(info.world_info_entries.len(), 3);

        apply_delete_world_info_entry(&mut info, 0).unwrap();
        assert_eq!(book(&info).entries.len(), 3);
        assert_eq!(book(&info).entries[0].keys, vec!["key1"]);
        assert!(apply_delete_world_info_entry(&mut info, 5).is_err());
    }

    /// 没有内嵌书的旧卡：第一次编辑时用投影建出完整书。
    #[test]
    fn legacy_card_without_book_is_materialized_from_its_projection() {
        let mut info = sample_info("Legacy");
        info.world_info_entries = vec![WorldInfoEntryInfo {
            keys: vec!["old".into()],
            content: "legacy".into(),
            constant: true,
            route: "Constant".into(),
            is_global: true,
            depth: 2,
            order: 100,
        }];
        apply_world_info_route_update(&mut info, 0, "Both").unwrap();
        assert_eq!(book(&info).entries.len(), 1);
        assert_eq!(book(&info).entries[0].route, LoreRoute::Both);
        assert!(info.world_info_entries[0].is_global, "全局标记不丢");
    }

    /// 旧编辑路径只改了投影（书与投影分叉）：编辑时投影独有的条目并回书里。
    #[test]
    fn diverged_projection_entries_are_kept() {
        let mut info = imported_info();
        info.world_info_entries.push(WorldInfoEntryInfo {
            keys: vec!["meta".into()],
            content: "只在投影里的条目".into(),
            constant: false,
            route: "Selective".into(),
            is_global: false,
            depth: 2,
            order: 100,
        });
        apply_world_info_route_update(&mut info, 0, "Constant").unwrap();
        assert!(
            book(&info)
                .entries
                .iter()
                .any(|e| e.content == "只在投影里的条目")
        );
        assert_eq!(info.world_info_entries.len(), 3);
    }

    #[test]
    fn bulk_replace_keeps_book_fields_and_disabled_entries() {
        let mut info = imported_info();
        let mut kept = info.world_info_entries[1].clone(); // 完整书第 2 条
        kept.route = "Constant".into();
        kept.constant = true;
        kept.is_global = true;
        let added = WorldInfoEntryInfo {
            keys: vec!["patched".into()],
            content: "patched".into(),
            constant: false,
            route: "Selective".into(),
            is_global: false,
            depth: 2,
            order: 100,
        };
        apply_update_world_info_entries_bulk(&mut info, vec![kept, added]).unwrap();
        let entries = &book(&info).entries;
        assert_eq!(entries.len(), 3, "删掉第 0 条，新增一条，禁用条目保留");
        assert_eq!(entries[0].secondary_keys, vec!["secondary2"]);
        assert_eq!(entries[0].route, LoreRoute::Constant);
        assert!(entry_is_global(&entries[0]));
        assert_eq!(entries[1].content, "patched");
        assert!(entries[2].disabled);
        assert_eq!(info.world_info_entries.len(), 2);

        apply_update_world_info_entries_bulk(&mut info, vec![]).unwrap();
        assert!(info.world_info_entries.is_empty());
        assert_eq!(book(&info).entries.len(), 1, "只剩禁用条目");
    }
}

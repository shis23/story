use super::super::*;
use crate::storage_backend::BackendCapability;

pub fn require_character_commands(
    state: &AppState,
    operation: &str,
) -> Result<(), TauriCommandError> {
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

pub fn default_depth() -> i32 {
    2
}
pub fn default_order() -> i32 {
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

pub fn delete_character_cascade_source_ids(
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

pub fn entry_is_global(entry: &storyforge_domain::world_info::WorldInfoEntry) -> bool {
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
pub fn refresh_world_info_counts(info: &mut CharacterInfo) {
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
pub fn reproject_world_info(info: &mut CharacterInfo) {
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
pub fn apply_world_info_route_update(
    info: &mut CharacterInfo,
    entry_index: usize,
    new_route: &str,
) -> Result<(), String> {
    let route =
        crate::services::world_info::parse_lore_route(new_route).map_err(|e| e.to_string())?;
    let entry = book_entry_mut(card_book_mut(info), entry_index)?;
    entry.disabled = matches!(route, storyforge_domain::world_info::LoreRoute::Disabled);
    entry.route = route;
    reproject_world_info(info);
    Ok(())
}

/// 更新世界书条目的 keys / content / constant / is_global / depth / order
/// （其余 ST 字段——次级关键词、注释、位置等——原样保留）。
#[allow(clippy::too_many_arguments)]
pub fn apply_world_info_entry_update(
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
pub fn apply_add_world_info_entry(
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
pub fn apply_delete_world_info_entry(
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
pub fn apply_update_world_info_entries_bulk(
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

/// `import_character` 命令的本体。Tauri 命令与非 Tauri 宿主（DSH sidecar）共用这一份实现。
pub fn import_character_for_host(
    state: &Arc<AppState>,
    data: Vec<u8>,
) -> Result<CharacterSummary, TauriCommandError> {
    require_character_commands(state.as_ref(), "import character")?;
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
    crate::vector_index::sync_world_info_vectors(state);

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

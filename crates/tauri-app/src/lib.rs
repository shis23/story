mod campaign_store;
mod connection_store;
mod preset_store;
mod storage;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use connection_store::ConnectionStore;
use preset_store::PresetStore;
use storage::CharacterStore;
use tokio::sync::watch;

use storyforge_app_agent::ToolContext;
use storyforge_app_conversation::{ConversationStore, PartialRollTarget};
use storyforge_app_logging::{ExportOptions, LogFilter, LogKind, LogLevel, LogStore};
use storyforge_app_pipeline::{PipelineOrchestrator, RegenerateRequest, WritingContext};
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::llm::{LlmConnection, LlmConnectionSummary, LlmProtocol, SamplingParams, ToolMode};
use storyforge_domain::Id;
use storyforge_infra_llm::LlmClient;
use storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord, VectorStore};

// ─── 全局存储（保留 M0 兼容）──────────────────────────────────────────────

static STORE: OnceLock<CharacterStore> = OnceLock::new();

fn get_store() -> &'static CharacterStore {
    STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        CharacterStore::new(&data_dir)
    })
}

static CONN_STORE: OnceLock<ConnectionStore> = OnceLock::new();

fn get_conn_store() -> &'static ConnectionStore {
    CONN_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        ConnectionStore::new(&data_dir)
    })
}

static PRESET_STORE: OnceLock<PresetStore> = OnceLock::new();

fn get_preset_store() -> &'static PresetStore {
    PRESET_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        PresetStore::new(&data_dir)
    })
}

static CAMPAIGN_STORE: OnceLock<campaign_store::CampaignStore> = OnceLock::new();

fn get_campaign_store() -> &'static campaign_store::CampaignStore {
    CAMPAIGN_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        campaign_store::CampaignStore::new(&data_dir)
    })
}

fn get_app_data_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let data_dir = exe_dir.join("data");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir
}

fn load_embed_config(data_dir: &PathBuf) -> Option<storyforge_infra_llm::EmbedConfig> {
    let path = data_dir.join("embed.json");
    if path.exists() {
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    } else {
        None
    }
}

fn save_embed_config(data_dir: &PathBuf, config: &storyforge_infra_llm::EmbedConfig) {
    let path = data_dir.join("embed.json");
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(&path, json);
    }
}

fn load_active_campaign(data_dir: &PathBuf) -> Option<Id> {
    let path = data_dir.join("active_campaign.json");
    let s = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("campaign_id")
        .and_then(|v| v.as_str())
        .map(Id::from_str)
}

fn save_active_campaign(data_dir: &PathBuf, id: Option<&Id>) {
    let path = data_dir.join("active_campaign.json");
    let v = serde_json::json!({ "campaign_id": id.map(|i| i.as_str()).unwrap_or("") });
    if let Ok(json) = serde_json::to_string_pretty(&v) {
        let _ = std::fs::write(&path, json);
    }
}

// ─── AppState（M1 新增，注入到 Tauri managed state）─────────────────────────

/// 应用全局状态
pub struct AppState {
    /// Mock LLM 客户端（fallback，无配置连接时用）
    pub mock_llm: Arc<dyn LlmClient>,
    pub conv_store: Arc<ConversationStore>,
    pub log_store: Arc<LogStore>,
    /// 工具上下文（导入角色卡时同步更新，RwLock 支持运行时写入）
    pub tool_ctx: Arc<RwLock<ToolContext>>,
    /// 当前运行的流水线 cancel sender（None = 无运行中的写作）
    pub current_cancel: Mutex<Option<watch::Sender<bool>>>,
    /// 当前活跃连接构造的 LLM client（None = 用 mock_llm）
    active_llm: Mutex<Option<Arc<dyn LlmClient>>>,
    /// 当前活跃连接的 ID（用于 get_active_connection 快速查询）
    active_conn_id: Mutex<Option<String>>,
    /// 向量存储（关键词搜索 + 后续向量搜索，持久化到 data/vectors.json）
    pub vector_store: Arc<BruteForceStore>,
    /// Meta Agent Patch 存储
    pub meta_patches: Arc<RwLock<Vec<storyforge_app_meta::Patch>>>,
    /// 嵌入配置（持久化到 data/embed.json）
    pub embed_config: Arc<RwLock<Option<storyforge_infra_llm::EmbedConfig>>>,
    /// 当前活跃 Campaign ID（持久化到 data/active_campaign.json）
    pub active_campaign: Mutex<Option<Id>>,
}

impl AppState {
    pub fn new() -> Self {
        let data_dir = get_app_data_dir();
        let conv_dir = data_dir.join("conversations");
        let log_dir = data_dir.join("logs");

        // Mock 作为 fallback（无连接配置时用，保证流水线不崩）
        let mock_llm: Arc<dyn LlmClient> =
            Arc::new(storyforge_infra_llm::mock_client::MockLlmClient::with_defaults());

        let conv_store = Arc::new(ConversationStore::new(conv_dir));
        let log_store = Arc::new(LogStore::new(log_dir));

        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
        }));

        // 启动恢复：从 CharacterStore 把已导入的角色卡 + 世界书同步进 tool_ctx
        // （否则每次重启 dev，tool_ctx 都是空的，写作时报"没有可用角色卡"）
        // 世界书：最后一张卡的条目 + 所有其他卡的 is_global 条目（全局共享）
        {
            let store = get_store();
            let stored_chars = store.list();
            if !stored_chars.is_empty() {
                let mut ctx = tool_ctx.write().unwrap();
                for stored in &stored_chars {
                    ctx.characters.push(Arc::new(stored_info_to_character(stored)));
                }
                // 最后一张作为当前角色，其条目全收；其他卡只收 is_global
                if let Some(last) = stored_chars.last() {
                    let active_name = &last.info.name;
                    ctx.world_info = Some(Arc::new(collect_world_info_for_active(
                        &stored_chars,
                        active_name,
                    )));
                }
                tracing::info!(
                    "启动恢复：从存储载入 {} 张角色卡 + 世界书",
                    stored_chars.len()
                );
            }
        }

        // 向量存储（持久化到 data/vectors.json）
        let vector_store = Arc::new(BruteForceStore::with_persistence(
            data_dir.join("vectors.json"),
        ));

        // 尝试从已持久化的连接恢复活跃 client（挂 LlmInterceptor 记录调用）
        let (active_llm, active_conn_id) = {
            let conn_store = get_conn_store();
            if let Some(conn) = conn_store.active_connection() {
                match storyforge_infra_llm::create_client(&conn) {
                    Ok(client) => {
                        let intercepted: Arc<dyn LlmClient> = Arc::new(
                            storyforge_app_logging::interceptor::LlmInterceptor::new(
                                Arc::from(client),
                                log_store.clone(),
                                conn.name.clone(),
                            ),
                        );
                        (Some(intercepted), Some(conn.id.as_str().to_string()))
                    }
                    Err(e) => {
                        tracing::warn!("启动时恢复活跃连接失败，回退 mock: {e}");
                        (None, None)
                    }
                }
            } else {
                (None, None)
            }
        };

        Self {
            mock_llm,
            conv_store,
            log_store,
            tool_ctx,
            current_cancel: Mutex::new(None),
            active_llm: Mutex::new(active_llm),
            active_conn_id: Mutex::new(active_conn_id),
            vector_store,
            meta_patches: Arc::new(RwLock::new(Vec::new())),
            embed_config: Arc::new(RwLock::new(load_embed_config(&data_dir))),
            active_campaign: Mutex::new(load_active_campaign(&data_dir)),
        }
    }

    /// 取一份 tool_ctx 快照（clone 出 Arc<ToolContext>），供本次流水线使用
    pub fn snapshot_tool_ctx(&self) -> Arc<ToolContext> {
        let ctx = self.tool_ctx.read().unwrap().clone();
        Arc::new(ctx)
    }

    /// 当前活跃的 LLM client（有配置用真实的，否则回退 mock）
    ///
    /// 正常路径前端会拦截（无连接时引导建连接），这里回退 mock 仅防崩。
    pub fn active_llm_or_mock(&self) -> Arc<dyn LlmClient> {
        let guard = self.active_llm.lock().unwrap();
        if let Some(client) = guard.as_ref() {
            client.clone()
        } else {
            self.mock_llm.clone()
        }
    }

    /// 当前活跃连接 ID
    pub fn active_conn_id(&self) -> Option<String> {
        self.active_conn_id.lock().unwrap().clone()
    }

    /// 设置活跃连接（构造 client 并缓存，挂 LlmInterceptor 记录每次调用）
    pub fn set_active_connection(&self, id: &str) -> Result<(), String> {
        let conn_store = get_conn_store();
        let conn = conn_store
            .set_active(id)
            .ok_or_else(|| format!("连接不存在: {id}"))?;

        let client =
            storyforge_infra_llm::create_client(&conn).map_err(|e| format!("构造客户端失败: {e}"))?;

        // 包装 LlmInterceptor：每次 LLM 调用自动记录 payload/响应/token/延迟到 LogStore
        let intercepted: Arc<dyn LlmClient> = Arc::new(
            storyforge_app_logging::interceptor::LlmInterceptor::new(
                Arc::from(client),
                self.log_store.clone(),
                conn.name.clone(),
            ),
        );

        *self.active_llm.lock().unwrap() = Some(intercepted);
        *self.active_conn_id.lock().unwrap() = Some(id.to_string());
        Ok(())
    }

    /// 清除活跃连接（删除时调用）
    pub fn clear_active_connection(&self) {
        *self.active_llm.lock().unwrap() = None;
        *self.active_conn_id.lock().unwrap() = None;
    }

    /// 构造一个新的 PipelineOrchestrator（用活跃 LLM + 当前 tool_ctx 快照 + vector_store）
    pub fn new_pipeline(&self) -> PipelineOrchestrator {
        let llm = self.active_llm_or_mock();
        let mut tool_ctx = (*self.snapshot_tool_ctx()).clone();
        // 注入向量存储（search_vectors 工具用）
        tool_ctx.vector_store = Some(self.vector_store.clone());
        PipelineOrchestrator::new(llm, self.conv_store.clone(), Arc::new(tool_ctx))
    }
}

// ─── 角色卡 DTO（保留 M0）───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInfo {
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_mes: String,
    pub system_prompt: String,
    pub tags: Vec<String>,
    pub creator: String,
    pub spec_version: String,
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

fn default_depth() -> i32 {
    2
}
fn default_order() -> i32 {
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
            name: c.name.clone(),
            description: c.description.clone(),
            personality: c.personality.clone(),
            scenario: c.scenario.clone(),
            first_mes: c.first_mes.clone(),
            system_prompt: c.system_prompt.clone(),
            tags: c.tags.clone(),
            creator: c.creator.clone(),
            spec_version: c.spec_version.clone(),
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

// ─── M0 角色卡命令（保留）──────────────────────────────────────────────────

#[tauri::command]
fn import_character(
    data: Vec<u8>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterSummary, String> {
    let character = storyforge_infra_import::import_character(&data)
        .map_err(|e| format!("导入失败: {e}"))?;
    let info = CharacterInfo::from(&character);
    let stored = get_store().save(info);

    // 同步到 tool_ctx：角色卡 + 世界书（覆盖为当前角色的，符合"当前角色"语义）
    {
        let mut ctx = state.tool_ctx.write().unwrap();
        // 避免重复导入同一张卡导致 characters 列表膨胀
        ctx.characters
            .retain(|c| c.name != character.name);
        // 提取世界书（内嵌的优先）
        let world_info = character
            .embedded_world_info
            .clone();
        ctx.characters.push(Arc::new(character));
        if let Some(wi) = world_info {
            // 先清理该角色之前导入的绿灯世界书条目（防重复累积）
            {
                let all_keywords: Vec<String> = wi.entries.iter()
                    .flat_map(|e| e.keys.iter().cloned())
                    .collect();
                if !all_keywords.is_empty() {
                    if let Ok(hits) = state.vector_store.search_by_keywords(&all_keywords, 1000) {
                        for hit in hits.into_iter().filter(|h| h.kind == VectorKind::WorldInfo) {
                            let _ = state.vector_store.delete(&hit.id);
                        }
                    }
                }
            }
            // 把绿灯世界书条目入库到向量存储（search_vectors 工具关键词搜索用）
            for entry in &wi.entries {
                if !entry.constant && !entry.keys.is_empty() {
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
fn list_characters() -> Vec<CharacterSummary> {
    get_store()
        .list()
        .into_iter()
        .map(|stored| CharacterSummary {
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
        .collect()
}

#[tauri::command]
fn get_character(id: String) -> Result<CharacterInfo, String> {
    get_store()
        .get(&id)
        .map(|stored| stored.info)
        .ok_or_else(|| format!("角色卡不存在: {id}"))
}

#[tauri::command]
fn delete_character(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // 先取出 name（用于同步 tool_ctx）
    let name = get_store().get(&id).map(|s| s.info.name);
    if !get_store().delete(&id) {
        return Err(format!("角色卡不存在: {id}"));
    }
    // 同步从 tool_ctx 移除
    if let Some(name) = name {
        let mut ctx = state.tool_ctx.write().unwrap();
        ctx.characters.retain(|c| c.name != name);
        // 如果移除的是当前世界书来源角色，清空 world_info
        // （简单处理：characters 空了就清 world_info）
        if ctx.characters.is_empty() {
            ctx.world_info = None;
        }
    }
    Ok(())
}

/// 更新世界书条目路由（蓝灯/绿灯/Both/Disabled）
#[tauri::command]
fn update_world_info_route(
    character_id: String,
    entry_index: usize,
    route: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // 验证路由值合法
    match route.as_str() {
        "Constant" | "Selective" | "Both" | "Disabled" => {}
        other => return Err(format!("无效路由: {other}，应为 Constant/Selective/Both/Disabled")),
    }

    get_store().update_world_info_route(&character_id, entry_index, &route)?;

    // 同步更新 tool_ctx 中的世界书路由
    if let Some(stored) = get_store().get(&character_id) {
        if stored.info.world_info_entries.get(entry_index).is_some() {
            let mut ctx = state.tool_ctx.write().unwrap();
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
    }

    Ok(())
}

/// 更新世界书条目的 keys/content/constant/is_global/depth/order
#[tauri::command]
fn update_world_info_entry(
    character_id: String,
    entry_index: usize,
    keys: Vec<String>,
    content: String,
    constant: bool,
    is_global: bool,
    depth: i32,
    order: i32,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
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
fn add_world_info_entry(
    character_id: String,
    keys: Vec<String>,
    content: String,
    constant: bool,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, String> {
    let new_index = get_store().add_world_info_entry(&character_id, keys.clone(), content.clone(), constant)?;

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
fn delete_world_info_entry(
    character_id: String,
    entry_index: usize,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
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
fn rebuild_world_info_in_tool_ctx(state: &tauri::State<'_, Arc<AppState>>) {
    use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

    let all_chars = get_store().list();

    // 找当前活跃角色名（tool_ctx.characters 里的）
    let active_names: Vec<String> = state
        .tool_ctx
        .read()
        .unwrap()
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
            });
        }
    }

    let mut ctx = state.tool_ctx.write().unwrap();
    if entries.is_empty() {
        ctx.world_info = None;
    } else {
        let book = WorldInfoBook {
            entries,
            source: storyforge_domain::Source::Native,
        };
        ctx.world_info = Some(Arc::new(book));
    }
}

#[tauri::command]
fn import_preset(data: Vec<u8>) -> Result<String, String> {
    let preset = storyforge_infra_import::import_preset(&data)
        .map_err(|e| format!("导入失败: {e}"))?;
    let preset_id = get_preset_store().save(preset.clone());
    Ok(format!(
        "预设 '{}' (id: {}) 导入成功，含 {} 条提示词、{} 条正则",
        preset.name,
        preset_id,
        preset.prompts.len(),
        preset.regex_scripts.len()
    ))
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ─── M1 写作命令 ───────────────────────────────────────────────────────────

/// 写作流水线事件（Tauri Channel 用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WritingEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

impl WritingEvent {
    pub fn from_pipeline_event(event: &PipelineEvent) -> Self {
        let (event_type, data) = match event {
            PipelineEvent::Started { session_id } => (
                "started".into(),
                serde_json::json!({ "session_id": session_id }),
            ),
            PipelineEvent::DirectorStarted => (
                "director_started".into(),
                serde_json::json!({}),
            ),
            PipelineEvent::DirectorProgress { delta } => (
                "director_progress".into(),
                serde_json::json!({ "delta": delta }),
            ),
            PipelineEvent::DirectorDone {
                scene_brief,
                subagent_count,
            } => (
                "director_done".into(),
                serde_json::json!({
                    "scene_brief": scene_brief,
                    "subagent_count": subagent_count,
                }),
            ),
            PipelineEvent::SubagentStarted {
                character_id,
                index,
                total,
            } => (
                "subagent_started".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                    "total": total,
                }),
            ),
            PipelineEvent::SubagentProgress {
                character_id,
                index,
                delta,
            } => (
                "subagent_progress".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                    "delta": delta,
                }),
            ),
            PipelineEvent::SubagentDone {
                character_id,
                index,
                full_text,
            } => (
                "subagent_done".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                    "full_text": full_text,
                }),
            ),
            PipelineEvent::SubagentCancelled {
                character_id,
                index,
            } => (
                "subagent_cancelled".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                }),
            ),
            PipelineEvent::EditorStarted => (
                "editor_started".into(),
                serde_json::json!({}),
            ),
            PipelineEvent::EditorProgress { delta } => (
                "editor_progress".into(),
                serde_json::json!({ "delta": delta }),
            ),
            PipelineEvent::DraftReady { text } => (
                "draft_ready".into(),
                serde_json::json!({ "text": text }),
            ),
            PipelineEvent::Committed {
                session_id,
                variant_id,
            } => (
                "committed".into(),
                serde_json::json!({
                    "session_id": session_id,
                    "variant_id": variant_id,
                }),
            ),
            PipelineEvent::Error { message } => (
                "error".into(),
                serde_json::json!({ "message": message }),
            ),
            PipelineEvent::StateChanged { state } => (
                "state_changed".into(),
                serde_json::json!({ "state": format!("{:?}", state) }),
            ),
        };

        Self { event_type, data }
    }
}

/// Tauri command: 启动写作流水线（通过 Channel 推送事件）
///
/// cancel sender 存进 AppState.current_cancel，前端可调 cancel_writing 中止。
/// 返回 { text, conversation_id, node_id } 供前端后续重 roll 定位。
#[tauri::command]
async fn start_writing(
    intent: String,
    character_id: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<WritingEvent>,
) -> Result<serde_json::Value, String> {
    let app = state.inner().clone();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();

    // 前端事件转发任务
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let writing_event = WritingEvent::from_pipeline_event(&event);
            let _ = on_event_clone.send(writing_event);
        }
    });

    // 构造写作上下文（从 tool_ctx 快照读取，导入的角色卡/世界书自动可见）
    let tool_snapshot = app.snapshot_tool_ctx();
    let conv = app.conv_store.create(character_id);
    let conversation_id = conv.id.clone();
    let ctx = WritingContext {
        characters: tool_snapshot.characters.clone(),
        world_info: tool_snapshot.world_info.clone(),
        conversation_id: conversation_id.clone(),
    };

    // 创建 cancel channel，sender 存进 AppState（前端可调 cancel_writing 触发）
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut slot = app.current_cancel.lock().unwrap();
        if let Some(existing) = slot.take() {
            // 上一次写作未正常清理，先取消它
            let _ = existing.send(true);
        }
        *slot = Some(cancel_tx);
    }

    // 每次用最新 tool_ctx 快照构造 orchestrator（保证导入后立刻生效）
    let mut pipeline = app.new_pipeline();
    let result = pipeline
        .start_writing(intent, &ctx, event_tx, cancel_rx)
        .await;

    // 清理 cancel sender
    {
        let mut slot = app.current_cancel.lock().unwrap();
        *slot = None;
    }

    match result {
        Ok((text, node_id, _provenance)) => Ok(serde_json::json!({
            "text": text,
            "conversation_id": conversation_id.to_string(),
            "node_id": node_id.to_string(),
        })),
        Err(e) => Err(format!("写作失败: {e}")),
    }
}

/// Tauri command: 取消当前运行的写作流水线
///
/// 触发 AppState.current_cancel 的 sender，导演/子Agent/编剧全部中止。
#[tauri::command]
fn cancel_writing(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, String> {
    let slot = state.current_cancel.lock().unwrap();
    if let Some(tx) = slot.as_ref() {
        let _ = tx.send(true);
        Ok(true)
    } else {
        Ok(false) // 无运行中的写作
    }
}

// ─── 重 roll 命令 ─────────────────────────────────────────────────────────

/// 重 roll 目标 DTO（前端传字符串，后端转 PartialRollTarget）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegenerateTargetDto {
    /// "director" / "editor" / "subagent:<角色名>"
    pub kind: String,
}

/// 重 roll 请求 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegenerateRequestDto {
    pub conversation_id: String,
    pub node_id: String,
    /// 目标列表（空 = 整体重 roll）
    pub targets: Vec<RegenerateTargetDto>,
    /// 附加提示词（可选）
    pub hint: Option<String>,
    /// 随机种子（可选）
    pub seed: Option<u64>,
}

/// 把 DTO 的 target 字符串解析为 PartialRollTarget
fn parse_target_dto(target: &RegenerateTargetDto) -> Result<PartialRollTarget, String> {
    match target.kind.as_str() {
        "director" => Ok(PartialRollTarget::Director),
        "editor" => Ok(PartialRollTarget::Editor),
        s if s.starts_with("subagent:") => {
            let id = s.trim_start_matches("subagent:");
            if id.is_empty() {
                Err("subagent 目标缺少角色名".into())
            } else {
                Ok(PartialRollTarget::Subagent(id.to_string()))
            }
        }
        other => Err(format!("未知重 roll 目标: {other}")),
    }
}

/// Tauri command: 重 roll（整体/只重编剧/只重某子 Agent，可附 hint）
///
/// 通过 Channel 推送事件，返回新 variant 的成文。
#[tauri::command]
async fn regenerate(
    req: RegenerateRequestDto,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<WritingEvent>,
) -> Result<String, String> {
    let app = state.inner().clone();

    // 解析 targets
    let targets: Vec<PartialRollTarget> = req
        .targets
        .iter()
        .map(parse_target_dto)
        .collect::<Result<_, _>>()?;

    let conversation_id = Id::from_str(&req.conversation_id);
    let node_id = Id::from_str(&req.node_id);

    let pipeline_req = RegenerateRequest {
        conversation_id: conversation_id.clone(),
        node_id: node_id.clone(),
        targets,
        hint: req.hint,
        seed: req.seed,
    };

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let writing_event = WritingEvent::from_pipeline_event(&event);
            let _ = on_event_clone.send(writing_event);
        }
    });

    // tool_ctx 快照（保持与 start_writing 一致）
    let tool_snapshot = app.snapshot_tool_ctx();
    let ctx = WritingContext {
        characters: tool_snapshot.characters.clone(),
        world_info: tool_snapshot.world_info.clone(),
        conversation_id: conversation_id.clone(),
    };

    // cancel channel
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut slot = app.current_cancel.lock().unwrap();
        if let Some(existing) = slot.take() {
            let _ = existing.send(true);
        }
        *slot = Some(cancel_tx);
    }

    let mut pipeline = app.new_pipeline();
    let result = pipeline.regenerate(pipeline_req, &ctx, event_tx, cancel_rx).await;

    {
        let mut slot = app.current_cancel.lock().unwrap();
        *slot = None;
    }

    match result {
        Ok((text, _provenance)) => Ok(text),
        Err(e) => Err(format!("重 roll 失败: {e}")),
    }
}

// ─── LLM 连接管理命令 ──────────────────────────────────────────────────────

/// 列出内置连接模板
#[tauri::command]
fn list_connection_templates() -> Vec<storyforge_domain::llm::ConnectionTemplate> {
    storyforge_domain::llm::builtin_connection_templates()
}

/// 列出已配置的连接（不含 api_key）
#[tauri::command]
fn list_connections(state: tauri::State<'_, Arc<AppState>>) -> Vec<LlmConnectionSummary> {
    let active_id = state.active_conn_id();
    get_conn_store()
        .list()
        .into_iter()
        .map(|stored| LlmConnectionSummary {
            id: stored.id.clone(),
            name: stored.connection.name.clone(),
            base_url: stored.connection.base_url.clone(),
            model: stored.connection.model.clone(),
            protocol: stored.connection.protocol.clone(),
            tool_mode: stored.connection.tool_mode.clone(),
            active: active_id.as_deref() == Some(stored.id.as_str()),
        })
        .collect()
}

/// 查询当前活跃连接（不含 api_key）
#[tauri::command]
fn get_active_connection(state: tauri::State<'_, Arc<AppState>>) -> Option<LlmConnectionSummary> {
    let active_id = state.active_conn_id()?;
    get_conn_store()
        .get(&active_id)
        .map(|stored| LlmConnectionSummary {
            id: stored.id,
            name: stored.connection.name,
            base_url: stored.connection.base_url,
            model: stored.connection.model,
            protocol: stored.connection.protocol,
            tool_mode: stored.connection.tool_mode,
            active: true,
        })
}

/// 创建连接请求 DTO
#[derive(Debug, Clone, Deserialize)]
pub struct CreateConnectionDto {
    /// 模板 id（可选，从模板创建时自动填默认值）
    pub template_id: Option<String>,
    pub name: String,
    pub base_url: String,
    /// 协议字符串："openai" / "anthropic" / "gemini" / "custom:xxx"
    pub protocol: String,
    pub model: String,
    pub api_key: String,
    /// "native" / "text_fallback"
    pub tool_mode: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// 创建连接（从模板或自定义）
///
/// 返回新连接的 id。若这是首个连接，自动设为活跃。
#[tauri::command]
fn create_connection(
    req: CreateConnectionDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let protocol = parse_protocol(&req.protocol)?;
    let tool_mode = parse_tool_mode(&req.tool_mode)?;

    let conn = LlmConnection {
        id: Id::new(),
        name: req.name,
        base_url: req.base_url,
        api_key: req.api_key, // TODO: Android 阶段改为 Keystore + SecretRef
        model: req.model,
        protocol,
        params: SamplingParams {
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_tokens,
        },
        tool_mode,
    };
    let conn_id = conn.id.as_str().to_string();

    // 预先验证：构造 client 看是否成功（base_url 格式等）
    // 注意：不实际发请求，只验证能构造出 client
    storyforge_infra_llm::create_client(&conn)
        .map_err(|e| format!("连接配置无效: {e}"))?;

    let was_empty = get_conn_store().list().is_empty();
    get_conn_store().save(conn);

    // 首个连接自动设为活跃
    if was_empty {
        state.set_active_connection(&conn_id)?;
    }

    Ok(conn_id)
}

/// 删除连接（若为活跃的，同时清除活跃状态）
#[tauri::command]
fn delete_connection(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let was_active = state.active_conn_id().as_deref() == Some(id.as_str());
    if !get_conn_store().delete(&id) {
        return Err(format!("连接不存在: {id}"));
    }
    if was_active {
        state.clear_active_connection();
    }
    Ok(())
}

/// 设置活跃连接
#[tauri::command]
async fn set_active_connection(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    state.set_active_connection(&id)
}

/// 测试连接请求 DTO（用临时配置测试，不持久化）
#[derive(Debug, Clone, Deserialize)]
pub struct TestConnectionDto {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub protocol: String,
    pub tool_mode: String,
}

/// 测试连接结果
#[derive(Debug, Clone, Serialize)]
pub struct TestConnectionResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: Option<u64>,
}

/// 测试连接连通性（发一个最小 ping 请求）
#[tauri::command]
async fn test_connection(req: TestConnectionDto) -> Result<TestConnectionResult, String> {
    let tool_mode = parse_tool_mode(&req.tool_mode)?;
    let protocol = parse_protocol(&req.protocol)?;
    let conn = LlmConnection {
        id: Id::new(),
        name: "test".into(),
        base_url: req.base_url,
        api_key: req.api_key,
        model: req.model,
        protocol,
        params: SamplingParams {
            max_tokens: Some(16),
            ..Default::default()
        },
        tool_mode,
    };

    let client = storyforge_infra_llm::create_client(&conn)
        .map_err(|e| format!("构造客户端失败: {e}"))?;

    let start = std::time::Instant::now();
    let chat_req = storyforge_domain::llm::ChatRequest {
        messages: vec![storyforge_domain::llm::ChatMessage::user("ping")],
        tools: None,
        params: conn.params.clone(),
        model: conn.model.clone(),
    };

    match client.chat(&chat_req).await {
        Ok(_resp) => {
            let latency = start.elapsed().as_millis() as u64;
            Ok(TestConnectionResult {
                success: true,
                message: format!("连通成功（模型: {}）", conn.model),
                latency_ms: Some(latency),
            })
        }
        Err(e) => Ok(TestConnectionResult {
            success: false,
            message: format!("{e}"),
            latency_ms: Some(start.elapsed().as_millis() as u64),
        }),
    }
}

/// 拉取服务商可用模型列表（GET /v1/models）
///
/// 用临时连接配置调用，不持久化。失败时返回空数组（前端走模板兜底）。
#[tauri::command]
async fn list_models(base_url: String, api_key: String) -> Result<Vec<String>, String> {
    let conn = LlmConnection {
        id: Id::new(),
        name: "models-probe".into(),
        base_url,
        api_key,
        model: String::new(),
        protocol: LlmProtocol::OpenAi,
        params: SamplingParams::default(),
        tool_mode: ToolMode::Native,
    };
    match storyforge_infra_llm::fetch_models(&conn).await {
        Ok(models) => Ok(models),
        Err(e) => {
            tracing::warn!("拉取模型列表失败，回退空列表: {e}");
            Ok(vec![])
        }
    }
}

/// 把协议字符串解析为 LlmProtocol
fn parse_protocol(s: &str) -> Result<LlmProtocol, String> {
    match s {
        "openai" => Ok(LlmProtocol::OpenAi),
        "anthropic" => Ok(LlmProtocol::Anthropic),
        "gemini" => Ok(LlmProtocol::Gemini),
        s if s.starts_with("custom:") => Ok(LlmProtocol::Custom(s[7..].to_string())),
        other => Err(format!("未知协议: {other}")),
    }
}

/// 把 tool_mode 字符串解析为 ToolMode
fn parse_tool_mode(s: &str) -> Result<ToolMode, String> {
    match s {
        "native" => Ok(ToolMode::Native),
        "text_fallback" => Ok(ToolMode::TextFallback),
        other => Err(format!("未知工具模式: {other}")),
    }
}

// ─── M1 对话命令 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummaryDto {
    pub id: String,
    pub character_id: Option<String>,
    pub message_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[tauri::command]
fn list_conversations(state: tauri::State<'_, Arc<AppState>>) -> Vec<ConversationSummaryDto> {
    state
        .conv_store
        .list()
        .into_iter()
        .map(|c| ConversationSummaryDto {
            id: c.id.to_string(),
            character_id: c.character_id,
            message_count: c.message_count,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        })
        .collect()
}

#[tauri::command]
fn get_conversation(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let conv_id = storyforge_domain::Id::from_str(&id);
    state
        .conv_store
        .get(&conv_id)
        .map(|c| serde_json::to_value(&c).unwrap_or_default())
        .ok_or_else(|| format!("对话不存在: {id}"))
}

// ─── M1 日志命令 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntryDto {
    pub id: String,
    pub kind: String,
    pub level: String,
    pub timestamp: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogFilterDto {
    pub kind: Option<String>,
    pub level: Option<String>,
    pub keyword: Option<String>,
    pub limit: Option<usize>,
}

#[tauri::command]
fn log_query(
    filter: LogFilterDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<LogEntryDto> {
    let log_filter = LogFilter {
        kind: filter.kind.as_deref().and_then(|k| match k {
            "backend" => Some(LogKind::Backend),
            "llm" => Some(LogKind::LlmCall),
            "frontend" => Some(LogKind::FrontendPlugin),
            _ => None,
        }),
        level: filter.level.as_deref().and_then(|l| match l {
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            _ => None,
        }),
        keyword: filter.keyword,
        since: None,
        until: None,
        limit: filter.limit,
    };

    state
        .log_store
        .query(&log_filter)
        .into_iter()
        .map(|e| LogEntryDto {
            id: e.id.to_string(),
            kind: format!("{:?}", e.kind),
            level: format!("{:?}", e.level),
            timestamp: e.timestamp.to_rfc3339(),
            message: e.message,
        })
        .collect()
}

#[tauri::command]
fn log_clear(kind: Option<String>, state: tauri::State<'_, Arc<AppState>>) {
    let log_kind = kind.as_deref().and_then(|k| match k {
        "backend" => Some(LogKind::Backend),
        "llm" => Some(LogKind::LlmCall),
        "frontend" => Some(LogKind::FrontendPlugin),
        _ => None,
    });
    state.log_store.clear(log_kind);
}

#[tauri::command]
fn log_export_bundle(
    redact_content: bool,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let opts = ExportOptions {
        redact_content,
        ..Default::default()
    };
    Ok(storyforge_app_logging::export_bundle(&state.log_store, &opts))
}

/// 前端日志上报（console.log/warn/error 转发到后端 LogStore）
#[tauri::command]
fn log_append_frontend(
    level: String,
    message: String,
    state: tauri::State<'_, Arc<AppState>>,
) {
    let log_level = match level.as_str() {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    };

    state.log_store.push(storyforge_app_logging::LogEntry {
        id: Id::new(),
        kind: LogKind::FrontendPlugin,
        level: log_level,
        timestamp: Utc::now(),
        message,
        fields: std::collections::HashMap::new(),
        llm_detail: None,
    });
}

// ─── M1 对话操作命令 ──────────────────────────────────────────────────────

/// 编辑当前变体内容
#[tauri::command]
fn edit_variant(
    conversation_id: String,
    node_id: String,
    new_content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .edit_variant(&conv_id, &nid, new_content)
        .map_err(|e| format!("{e}"))
}

/// 采纳当前变体（Draft → Final）
#[tauri::command]
fn accept_variant(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .accept_variant(&conv_id, &nid)
        .map_err(|e| format!("{e}"))
}

/// 软删除当前变体（→ Discarded）
#[tauri::command]
fn soft_delete_variant(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .soft_delete_variant(&conv_id, &nid)
        .map_err(|e| format!("{e}"))
}

/// 添加新变体（分支/swipe）
#[tauri::command]
fn add_variant(
    conversation_id: String,
    node_id: String,
    content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, String> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .add_variant(&conv_id, &nid, content, None)
        .map_err(|e| format!("{e}"))
}

/// 切换变体（左右滑）
#[tauri::command]
fn switch_variant(
    conversation_id: String,
    node_id: String,
    index: usize,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .switch_variant(&conv_id, &nid, index)
        .map_err(|e| format!("{e}"))
}

// ─── M2 记忆系统命令 ─────────────────────────────────────────────────────

/// 配置嵌入 API
#[tauri::command]
fn configure_embedder(
    endpoint: String,
    api_key: String,
    model: String,
    dim: usize,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let config = storyforge_infra_llm::EmbedConfig {
        endpoint,
        api_key,
        model,
        dim,
    };
    let data_dir = get_app_data_dir();
    save_embed_config(&data_dir, &config);
    *state.embed_config.write().unwrap() = Some(config);
    Ok(())
}

/// 获取当前嵌入配置（不含 key）
#[tauri::command]
fn get_embed_config(
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<serde_json::Value> {
    state.embed_config.read().unwrap().as_ref().map(|c| {
        serde_json::json!({
            "endpoint": c.endpoint,
            "model": c.model,
            "dim": c.dim,
            "has_key": !c.api_key.is_empty(),
        })
    })
}

/// 手动触发对话归档（将近期消息压缩为远记忆摘要并入库）
#[tauri::command]
async fn archive_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, String> {
    let conv_id = Id::from_str(&conversation_id);
    let conv = state
        .conv_store
        .get(&conv_id)
        .ok_or_else(|| format!("对话不存在: {conversation_id}"))?;

    // 取所有非 Discarded 消息
    let messages: Vec<String> = conv
        .nodes
        .iter()
        .filter_map(|node| {
            let v = node.active()?;
            if v.status == storyforge_domain::conversation::VariantStatus::Discarded {
                None
            } else {
                Some(v.content.clone())
            }
        })
        .collect();

    let config = state
        .embed_config
        .read()
        .unwrap()
        .clone()
        .ok_or("未配置嵌入 API，请先在设置中配置")?;

    let llm = state.active_llm_or_mock();
    let vector_store = state.vector_store.clone();

    let embedder = Arc::new(storyforge_infra_llm::Embedder::new(config));
    let archiver = storyforge_app_memory::MemoryArchiver::new(
        llm,
        embedder,
        vector_store,
        storyforge_app_memory::ArchiveConfig::default(),
    );

    let summaries = archiver
        .maybe_archive(&messages)
        .await
        .map_err(|e| format!("归档失败: {e}"))?;

    Ok(summaries.len())
}

// ─── Meta Agent 命令 ──────────────────────────────────────────────────────

/// 接受并执行 Patch（修改世界书条目或角色字段）
#[tauri::command]
fn meta_accept_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // 从 PatchStore 取出 patch
    let patch = {
        let patches = state.meta_patches.read().unwrap();
        patches
            .iter()
            .find(|p| p.id == patch_id)
            .cloned()
            .ok_or_else(|| format!("Patch 不存在: {patch_id}"))?
    };

    // 执行 patch：clone 世界书 → 修改 → 写回
    {
        let mut ctx = state.tool_ctx.write().unwrap();
        if let Some(ref world_info) = ctx.world_info {
            let mut entries_json: Vec<serde_json::Value> = world_info
                .entries
                .iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default())
                .collect();

            let mut patch_ctx = storyforge_app_meta::PatchContext {
                world_info_entries: Some(&mut entries_json),
                character_fields: None,
            };

            storyforge_app_meta::execute_patch(&patch, &mut patch_ctx)
                .map_err(|e| format!("{e}"))?;

            // 反序列化回 WorldInfoEntry 并替换
            let new_entries: Vec<storyforge_domain::world_info::WorldInfoEntry> =
                entries_json
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();

            let mut new_book = (**world_info).clone();
            new_book.entries = new_entries;
            ctx.world_info = Some(Arc::new(new_book));
        }
    }

    // 持久化到 CharacterStore（同步 world_info_entries）
    // 从 tool_ctx 取最新的世界书，反序列化回 WorldInfoEntryInfo 写回存储
    {
        let ctx = state.tool_ctx.read().unwrap();
        if let Some(ref world_info) = ctx.world_info {
            // 找到受影响的角色卡，更新其 world_info_entries
            let all_stored = get_store().list();
            if let Some(last) = all_stored.last() {
                let entries: Vec<crate::WorldInfoEntryInfo> = world_info
                    .entries
                    .iter()
                    .map(|e| crate::WorldInfoEntryInfo {
                        keys: e.keys.clone(),
                        content: e.content.clone(),
                        constant: e.constant,
                        route: format!("{:?}", e.route),
                        is_global: false,
                        depth: e.depth,
                        order: e.order,
                    })
                    .collect();
                let _ = get_store().update_world_info_entries_bulk(
                    &last.id,
                    entries,
                );
            }
        }
    }

    // 标记为已执行
    state
        .meta_patches
        .write()
        .unwrap()
        .iter_mut()
        .find(|p| p.id == patch_id)
        .map(|p| p.applied = true);

    Ok(())
}

// ─── P1：角色识别 / CharacterCard / Campaign / 角色实例 / 变量 ──────────────

/// 角色（CharacterDefinition）的精简 DTO（前端展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterDefinitionDto {
    pub id: String,
    pub name: String,
    pub persona_prompt: String,
    pub behavior_rules: String,
    pub base_backstory: Vec<String>,
    pub group: Option<String>,
    pub role_type: String,
    pub variable_schema: Vec<storyforge_domain::variables::VariableField>,
}

impl From<&storyforge_domain::character::CharacterDefinition> for CharacterDefinitionDto {
    fn from(d: &storyforge_domain::character::CharacterDefinition) -> Self {
        Self {
            id: d.id.as_str().to_string(),
            name: d.name.clone(),
            persona_prompt: d.persona_prompt.clone(),
            behavior_rules: d.behavior_rules.clone(),
            base_backstory: d.base_backstory.clone(),
            group: d.group.clone(),
            role_type: format!("{:?}", d.role_type).to_lowercase(),
            variable_schema: d.variable_schema.clone(),
        }
    }
}

/// CharacterCard 的详情 DTO（含卡内角色定义列表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardDetailDto {
    pub id: String,
    pub name: String,
    pub source_character_id: String,
    pub character_definitions: Vec<CharacterDefinitionDto>,
    pub imported_at: String,
    /// 识别是否成功（false = 走降级路径，单角色 Protagonist）
    pub extracted: bool,
}

/// CharacterCard 列表项（轻量）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardSummaryDto {
    pub id: String,
    pub name: String,
    pub source_character_id: String,
    pub character_count: usize,
    pub imported_at: String,
    pub extracted: bool,
}

impl From<&campaign_store::StoredCard> for CardSummaryDto {
    fn from(s: &campaign_store::StoredCard) -> Self {
        Self {
            id: s.card.id.as_str().to_string(),
            name: s.card.name.clone(),
            source_character_id: s.card.source_character_id.as_str().to_string(),
            character_count: s.card.character_definitions.len(),
            imported_at: s.imported_at.clone(),
            extracted: !s.card.character_definitions.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignSummaryDto {
    pub id: String,
    pub card_id: String,
    pub name: String,
    pub created_at: String,
    pub story_clock: String,
    pub instance_count: usize,
    pub fork_from: Option<(String, String)>,
}

impl From<&storyforge_domain::campaign::Campaign> for CampaignSummaryDto {
    fn from(c: &storyforge_domain::campaign::Campaign) -> Self {
        Self {
            id: c.id.as_str().to_string(),
            card_id: c.card_id.as_str().to_string(),
            name: c.name.clone(),
            created_at: c.created_at.clone(),
            story_clock: c.story_clock.clone(),
            instance_count: 0, // 调用方填
            fork_from: c
                .fork_from
                .as_ref()
                .map(|(cid, nid)| (cid.as_str().to_string(), nid.as_str().to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInstanceDto {
    pub id: String,
    pub campaign_id: String,
    pub definition_id: Option<String>,
    pub name: String,
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
            persona_override: i.persona_override.clone(),
            behavior_override: i.behavior_override.clone(),
            is_temporary: i.is_temporary,
            variables: i.variables.clone(),
        }
    }
}

/// 跑角色识别 Agent，为已导入的扁平 Character 建 CharacterCard
///
/// 失败时降级：建单角色 Protagonist definition（卡仍可用）。
#[tauri::command]
async fn extract_characters(
    source_character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardSummaryDto, String> {
    use storyforge_app_agent::AgentRuntime;
    use storyforge_domain::character::CharacterDefinition;
    use storyforge_domain::variables::extract_mvu_schema_from_extensions;

    // 取原 Character（从 tool_ctx，启动恢复 + import_character 都同步过）
    let character = {
        let ctx = state.tool_ctx.read().unwrap();
        ctx.characters
            .iter()
            .find(|c| c.id.as_str() == source_character_id)
            .map(|c| (*c).clone())
    }
    .ok_or_else(|| format!("找不到 source_character_id={source_character_id} 的角色卡"))?;

    // 已存在则直接返回
    let store = get_campaign_store();
    if let Some(existing) = store.get_card_by_source(&character.id) {
        return Ok(CardSummaryDto::from(&existing));
    }

    // MVU schema 探测
    let mvu_schema = extract_mvu_schema_from_extensions(&character.extensions);
    if !mvu_schema.is_empty() {
        tracing::info!(
            "卡「{}」探测到 {} 个 MVU 字段",
            character.name,
            mvu_schema.len()
        );
    }

    // 跑识别 Agent
    let llm = state.active_llm_or_mock();
    let tool_ctx = state.snapshot_tool_ctx();
    let runtime = AgentRuntime::new(llm, tool_ctx);
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let definitions_result =
        storyforge_app_agent::extract_characters(&runtime, &character, &mvu_schema, cancel_rx)
            .await;

    let (definitions, extracted) = match definitions_result {
        Ok(defs) => (defs, true),
        Err(e) => {
            tracing::warn!("角色识别失败，降级建单角色: {e}");
            (
                vec![CharacterDefinition::fallback_from_character(
                    &character,
                    &mvu_schema,
                )],
                false,
            )
        }
    };

    // 建卡 + 回填 card_id
    let mut card =
        storyforge_domain::character::CharacterCard::from_character(&character);
    let definitions = storyforge_app_agent::attach_definitions_to_card(definitions, &card.id);
    card.character_definitions = definitions;
    let stored = store.save_card(card);

    let mut dto = CardSummaryDto::from(&stored);
    dto.extracted = extracted;
    Ok(dto)
}

#[tauri::command]
fn list_cards() -> Vec<CardSummaryDto> {
    get_campaign_store()
        .list_cards()
        .iter()
        .map(CardSummaryDto::from)
        .collect()
}

#[tauri::command]
fn get_card(id: String) -> Result<CardDetailDto, String> {
    let stored = get_campaign_store()
        .get_card(&Id::from_str(&id))
        .ok_or_else(|| format!("找不到 card id={id}"))?;
    Ok(CardDetailDto {
        id: stored.card.id.as_str().to_string(),
        name: stored.card.name.clone(),
        source_character_id: stored.card.source_character_id.as_str().to_string(),
        character_definitions: stored
            .card
            .character_definitions
            .iter()
            .map(CharacterDefinitionDto::from)
            .collect(),
        imported_at: stored.imported_at.clone(),
        extracted: !stored.card.character_definitions.is_empty(),
    })
}

/// 开档：建 Campaign，把卡里所有 Protagonist/Supporting 定义实例化
#[tauri::command]
fn create_campaign(
    card_id: String,
    name: String,
) -> Result<CampaignSummaryDto, String> {
    use storyforge_domain::character::RoleType;
    use storyforge_domain::campaign::CharacterInstance;

    let store = get_campaign_store();
    let stored = store
        .get_card(&Id::from_str(&card_id))
        .ok_or_else(|| format!("找不到 card id={card_id}"))?;

    let campaign =
        storyforge_domain::campaign::Campaign::new(stored.card.id.clone(), name);
    store.save_campaign(campaign.clone());

    // 实例化所有 protagonist/supporting 定义
    let mut instance_count = 0;
    for def in &stored.card.character_definitions {
        if matches!(def.role_type, RoleType::Protagonist | RoleType::Supporting) {
            let inst = CharacterInstance::from_definition(campaign.id.clone(), def);
            store.add_instance(inst);
            instance_count += 1;
        }
    }

    let mut dto = CampaignSummaryDto::from(&campaign);
    dto.instance_count = instance_count;
    Ok(dto)
}

#[tauri::command]
fn list_campaigns(card_id: Option<String>) -> Vec<CampaignSummaryDto> {
    let store = get_campaign_store();
    let campaigns = if let Some(cid) = card_id {
        store.list_campaigns_of_card(&Id::from_str(&cid))
    } else {
        store.list_campaigns()
    };
    campaigns
        .iter()
        .map(|c| {
            let mut dto = CampaignSummaryDto::from(c);
            dto.instance_count = store.list_instances(&c.id).len();
            dto
        })
        .collect()
}

#[tauri::command]
fn get_campaign(id: String) -> Result<CampaignSummaryDto, String> {
    let store = get_campaign_store();
    let c = store
        .get_campaign(&Id::from_str(&id))
        .ok_or_else(|| format!("找不到 campaign id={id}"))?;
    let mut dto = CampaignSummaryDto::from(&c);
    dto.instance_count = store.list_instances(&c.id).len();
    Ok(dto)
}

#[tauri::command]
fn set_active_campaign(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let campaign_id = Id::from_str(&id);
    // 校验存在
    if get_campaign_store()
        .get_campaign(&campaign_id)
        .is_none()
    {
        return Err(format!("找不到 campaign id={id}"));
    }
    *state.active_campaign.lock().unwrap() = Some(campaign_id.clone());
    save_active_campaign(&get_app_data_dir(), Some(&campaign_id));
    Ok(())
}

#[tauri::command]
fn get_active_campaign(state: tauri::State<'_, Arc<AppState>>) -> Option<CampaignSummaryDto> {
    let id = state.active_campaign.lock().unwrap().clone()?;
    let store = get_campaign_store();
    let c = store.get_campaign(&id)?;
    let mut dto = CampaignSummaryDto::from(&c);
    dto.instance_count = store.list_instances(&c.id).len();
    Some(dto)
}

#[tauri::command]
fn list_instances(campaign_id: String) -> Vec<CharacterInstanceDto> {
    get_campaign_store()
        .list_instances(&Id::from_str(&campaign_id))
        .iter()
        .map(CharacterInstanceDto::from)
        .collect()
}

#[tauri::command]
fn get_instance(campaign_id: String, instance_id: String) -> Result<CharacterInstanceDto, String> {
    get_campaign_store()
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map(|i| CharacterInstanceDto::from(&i))
        .ok_or_else(|| format!("找不到 instance {instance_id}"))
}

/// 查角色实例的当前变量值
#[tauri::command]
fn get_character_variables(
    campaign_id: String,
    instance_id: String,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, String> {
    get_campaign_store()
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map(|i| i.variables)
        .ok_or_else(|| format!("找不到 instance {instance_id}"))
}

/// 手动改角色实例变量值（调试/纠错用，turn 用 0 占位）
#[tauri::command]
fn set_character_variable(
    campaign_id: String,
    instance_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
) -> Result<(), String> {
    let store = get_campaign_store();
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| format!("找不到 instance {instance_id}"))?;
    inst.set_variable(&key, value, turn.unwrap_or(0));
    store.update_instance(inst);
    Ok(())
}

/// 查 Campaign 全局变量
#[tauri::command]
fn get_campaign_variables(
    campaign_id: String,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, String> {
    get_campaign_store()
        .get_campaign(&Id::from_str(&campaign_id))
        .map(|c| c.variables)
        .ok_or_else(|| format!("找不到 campaign {campaign_id}"))
}

/// 改 Campaign 全局变量
#[tauri::command]
fn set_campaign_variable(
    campaign_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
) -> Result<(), String> {
    let store = get_campaign_store();
    let mut camp = store
        .get_campaign(&Id::from_str(&campaign_id))
        .ok_or_else(|| format!("找不到 campaign {campaign_id}"))?;
    camp.set_variable(&key, value, turn.unwrap_or(0));
    store.update_campaign(camp);
    Ok(())
}

/// 把临场角色升级为常驻（仅翻 is_temporary flag）
#[tauri::command]
fn promote_temporary_instance(
    campaign_id: String,
    instance_id: String,
) -> Result<(), String> {
    let store = get_campaign_store();
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| format!("找不到 instance {instance_id}"))?;
    if !inst.is_temporary {
        return Err("该角色已是常驻".into());
    }
    inst.promote_to_permanent();
    store.update_instance(inst);
    Ok(())
}

// ─── Tauri app 入口 ────────────────────────────────────────────────────────

pub fn run() {
    // 先构造 AppState（含 log_store），再初始化 tracing 接入 LogStore
    let app_state = Arc::new(AppState::new());
    storyforge_app_logging::init_tracing(app_state.log_store.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            // M0 角色卡命令
            import_character,
            list_characters,
            get_character,
            delete_character,
            update_world_info_route,
            update_world_info_entry,
            add_world_info_entry,
            delete_world_info_entry,
            meta_accept_patch,
            import_preset,
            get_version,
            // LLM 连接管理命令
            list_connection_templates,
            list_connections,
            get_active_connection,
            create_connection,
            delete_connection,
            set_active_connection,
            test_connection,
            list_models,
            // M1 写作命令
            start_writing,
            cancel_writing,
            // M1 对话命令
            list_conversations,
            get_conversation,
            // 重 roll 命令
            regenerate,
            // M1 对话操作命令
            edit_variant,
            accept_variant,
            soft_delete_variant,
            add_variant,
            switch_variant,
            // M1 日志命令
            log_query,
            log_clear,
            log_export_bundle,
            log_append_frontend,
            configure_embedder,
            get_embed_config,
            archive_conversation,
            // P1：角色识别 / CharacterCard / Campaign / 角色实例 / 变量
            extract_characters,
            list_cards,
            get_card,
            create_campaign,
            list_campaigns,
            get_campaign,
            set_active_campaign,
            get_active_campaign,
            list_instances,
            get_instance,
            get_character_variables,
            set_character_variable,
            get_campaign_variables,
            set_campaign_variable,
            promote_temporary_instance,
        ])
        .run(tauri::generate_context!())
        .expect("StoryForge 启动失败");
}

// ─── 启动恢复辅助：StoredCharacter → domain Character / WorldInfoBook ──────────

/// 从存储的 CharacterInfo 构造 domain Character（精简版，导演工具够用）
///
/// 注：CharacterInfo 是导入时的 DTO，丢了 mes_example/embedded_world_info/raw_card_json
/// 等完整字段。启动恢复只填导演工具用得到的字段（name/description/personality/
/// scenario/first_mes/system_prompt），其余留空。
fn stored_info_to_character(stored: &storage::StoredCharacter) -> storyforge_domain::character::Character {
    storyforge_domain::character::Character {
        id: Id::from_str(&stored.id),
        name: stored.info.name.clone(),
        description: stored.info.description.clone(),
        personality: stored.info.personality.clone(),
        scenario: stored.info.scenario.clone(),
        first_mes: stored.info.first_mes.clone(),
        mes_example: String::new(),
        system_prompt: stored.info.system_prompt.clone(),
        post_history_instructions: String::new(),
        tags: stored.info.tags.clone(),
        creator: stored.info.creator.clone(),
        character_version: String::new(),
        alternate_greetings: vec![],
        embedded_world_info: None,
        extensions: serde_json::json!({}),
        renderable_assets: None,
        source: storyforge_domain::Source::Native,
        spec_version: stored.info.spec_version.clone(),
        raw_card_json: serde_json::json!({}),
    }
}

/// 收集世界书条目：当前活跃角色的全部 + 其他角色的 is_global 条目
///
/// `active_name` = 当前激活的角色卡名（来自 tool_ctx.characters 的最后一个）。
/// 返回的 WorldInfoBook 包含所有应生效的条目（含全局共享的）。
fn collect_world_info_for_active(
    all_chars: &[storage::StoredCharacter],
    active_name: &str,
) -> storyforge_domain::world_info::WorldInfoBook {
    use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

    let mut entries: Vec<WorldInfoEntry> = Vec::new();
    for stored in all_chars {
        let is_active = stored.info.name == active_name;
        for e in &stored.info.world_info_entries {
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
            });
        }
    }

    WorldInfoBook {
        entries,
        source: storyforge_domain::Source::Native,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use storyforge_app_agent::ToolContext;
    use storyforge_domain::character::Character;

    /// 验证 tool_ctx 的 RwLock + snapshot 机制：写入后快照能读到
    /// （这是 import_character 同步 tool_ctx 的核心机制）
    #[test]
    fn test_tool_ctx_snapshot_sees_writes() {
        let state = AppState::new();

        // 初始为空
        let snap0 = state.snapshot_tool_ctx();
        assert!(snap0.characters.is_empty());
        assert!(snap0.world_info.is_none());

        // 模拟 import_character 的写入逻辑：手动构造一个 Character
        let char = Arc::new(make_test_character("TestHero"));
        {
            let mut ctx = state.tool_ctx.write().unwrap();
            ctx.characters.push(char);
        }

        // 快照应能读到
        let snap1 = state.snapshot_tool_ctx();
        assert_eq!(snap1.characters.len(), 1);
        assert_eq!(snap1.characters[0].name, "TestHero");
    }

    /// 构造测试用 Character（domain Character 无 Default）
    fn make_test_character(name: &str) -> Character {
        use storyforge_domain::Source;
        Character {
            id: Id::new(),
            name: name.into(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::json!({}),
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::json!({}),
        }
    }

    /// 验证 current_cancel 的存取（cancel_writing 命令的核心机制）
    #[test]
    fn test_current_cancel_slot() {
        let state = AppState::new();

        // 初始无运行中的写作
        {
            let slot = state.current_cancel.lock().unwrap();
            assert!(slot.is_none());
        }

        // 模拟 start_writing 设置 cancel sender
        let (tx, mut rx) = watch::channel(false);
        {
            let mut slot = state.current_cancel.lock().unwrap();
            *slot = Some(tx);
        }

        // 触发取消
        {
            let slot = state.current_cancel.lock().unwrap();
            let tx = slot.as_ref().unwrap();
            let _ = tx.send(true);
        }
        assert!(*rx.borrow(), "cancel 应已触发");

        // 清理
        {
            let mut slot = state.current_cancel.lock().unwrap();
            *slot = None;
        }
    }

    /// 验证 active_llm_or_mock：无活跃连接时回退 mock（关键 fallback 行为）
    #[test]
    fn test_active_llm_fallback_to_mock() {
        let state = AppState::new();
        // 测试环境通常无活跃连接（除非 data/connections.json 恰好有）
        // 这里测 clear 后回退 mock
        state.clear_active_connection();
        assert!(state.active_conn_id().is_none());
        // 应返回 mock（不 panic）
        let _client = state.active_llm_or_mock();
    }

    /// 验证协议字符串解析
    #[test]
    fn test_parse_protocol() {
        assert_eq!(parse_protocol("openai").unwrap(), LlmProtocol::OpenAi);
        assert_eq!(parse_protocol("anthropic").unwrap(), LlmProtocol::Anthropic);
        assert_eq!(parse_protocol("gemini").unwrap(), LlmProtocol::Gemini);
        match parse_protocol("custom:myapi").unwrap() {
            LlmProtocol::Custom(s) => assert_eq!(s, "myapi"),
            other => panic!("应是 Custom，实际: {other:?}"),
        }
        assert!(parse_protocol("unknown").is_err());
    }

    /// 验证 tool_mode 字符串解析
    #[test]
    fn test_parse_tool_mode() {
        assert_eq!(parse_tool_mode("native").unwrap(), ToolMode::Native);
        assert_eq!(parse_tool_mode("text_fallback").unwrap(), ToolMode::TextFallback);
        assert!(parse_tool_mode("unknown").is_err());
    }

    /// 验证 AppState 的 vector_store 字段初始化正常（可读写）
    #[test]
    fn test_vector_store_initialized() {
        let state = AppState::new();
        let initial = state.vector_store.count();

        // 写入一条测试记录，验证可检索
        let _ = state.vector_store.upsert(VectorRecord {
            id: Id::from_str("test-init-vs"),
            content: "测试内容".into(),
            vector: vec![],
            keywords: vec!["测试".into()],
            kind: VectorKind::WorldInfo,
            metadata: std::collections::HashMap::new(),
        });
        assert_eq!(state.vector_store.count(), initial + 1);
        let hits = state.vector_store.search_by_keywords(&["测试".into()], 10).unwrap();
        assert!(hits.iter().any(|h| h.content == "测试内容"));

        // 清理测试记录
        let _ = state.vector_store.delete(&Id::from_str("test-init-vs"));
    }

    /// 验证 vector_store 持久化：写入后重载能查到
    #[test]
    fn test_vector_store_persistence() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_vs_persist_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.json");

        {
            let store = BruteForceStore::with_persistence(path.clone());
            let _ = store.upsert(VectorRecord {
                id: Id::from_str("v1"),
                content: "持久化测试".into(),
                vector: vec![],
                keywords: vec!["持久".into()],
                kind: VectorKind::WorldInfo,
                metadata: std::collections::HashMap::new(),
            });
        }

        // 重载
        {
            let store = BruteForceStore::with_persistence(path.clone());
            assert_eq!(store.count(), 1);
            let hits = store.search_by_keywords(&["持久".into()], 10).unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].content, "持久化测试");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

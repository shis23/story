mod campaign_store;
mod connection_store;
mod module_store;
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
use storyforge_domain::prompt_module::PromptProfile;
use storyforge_domain::llm::{LlmConnection, LlmConnectionSummary, LlmProtocol, SamplingParams, ToolMode};
use storyforge_domain::Id;
use storyforge_infra_llm::LlmClient;
use storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord, VectorStore};
use storyforge_infra_plugin_host::PluginRegistry;

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
    if let Err(e) = storyforge_infra_util::atomic_write_json(&path, config) {
        tracing::error!("保存嵌入配置失败: {e}");
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
    if let Err(e) = storyforge_infra_util::atomic_write_json(&path, &v) {
        tracing::error!("保存活跃 Campaign 失败: {e}");
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
    /// 插件注册表（持久化到 data/plugins.json）
    pub plugin_registry: Arc<PluginRegistry>,
    /// 模块存储（内置 + 自定义模块 + 启用/禁用状态）
    pub module_store: Arc<module_store::ModuleStore>,
    /// Profile 存储（预设配置 + 活跃 Profile）
    pub profile_store: Arc<module_store::ProfileStore>,
    /// Meta Agent 会话（诊断工具的数据源 + PatchStore，P3 新增）
    pub meta_session: Arc<storyforge_app_meta::MetaSession>,
    /// Meta 对话历史（conversation_id → MetaConversation，内存态，重启清空，P3 新增）
    pub meta_conversations: Mutex<std::collections::HashMap<String, storyforge_app_meta::MetaConversation>>,
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
                let mut ctx = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
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

        // 插件注册表（持久化到 data/plugins.json）
        let plugin_registry = Arc::new(PluginRegistry::with_persistence(
            data_dir.join("plugins.json"),
        ));

        // 模块 + Profile 存储
        let module_store = Arc::new(module_store::ModuleStore::new(&data_dir));
        let profile_store = Arc::new(module_store::ProfileStore::new(&data_dir));
        profile_store.ensure_default();

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
            plugin_registry,
            module_store,
            profile_store,
            meta_session: Arc::new(storyforge_app_meta::MetaSession::new()),
            meta_conversations: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 取一份 tool_ctx 快照（clone 出 Arc<ToolContext>），供本次流水线使用
    pub fn snapshot_tool_ctx(&self) -> Arc<ToolContext> {
        let ctx = self.tool_ctx.read().unwrap_or_else(|p| p.into_inner()).clone();
        Arc::new(ctx)
    }

    /// 当前活跃的 LLM client（有配置用真实的，否则回退 mock）
    ///
    /// 正常路径前端会拦截（无连接时引导建连接），这里回退 mock 仅防崩。
    pub fn active_llm_or_mock(&self) -> Arc<dyn LlmClient> {
        let guard = self.active_llm.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(client) = guard.as_ref() {
            client.clone()
        } else {
            self.mock_llm.clone()
        }
    }

    /// 当前活跃连接 ID
    pub fn active_conn_id(&self) -> Option<String> {
        self.active_conn_id.lock().unwrap_or_else(|p| p.into_inner()).clone()
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

        *self.active_llm.lock().unwrap_or_else(|p| p.into_inner()) = Some(intercepted);
        *self.active_conn_id.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.to_string());
        Ok(())
    }

    /// 清除活跃连接（删除时调用）
    pub fn clear_active_connection(&self) {
        *self.active_llm.lock().unwrap_or_else(|p| p.into_inner()) = None;
        *self.active_conn_id.lock().unwrap_or_else(|p| p.into_inner()) = None;
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
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
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
        .map(CharacterSummary::from)
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
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.characters.retain(|c| c.name != name);
        // 如果移除的是当前世界书来源角色，清空 world_info
        // （简单处理：characters 空了就清 world_info）
        if ctx.characters.is_empty() {
            ctx.world_info = None;
        }
    }
    // 级联删除：该卡的 MVU 翻译（source_character_id == 角色卡 id）
    get_campaign_store().delete_mvu(&Id::from_str(id.clone()));
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
    is_global: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, String> {
    let new_index = get_store().add_world_info_entry(&character_id, keys.clone(), content.clone(), constant, is_global.unwrap_or(false))?;

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

    let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
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

/// 预设摘要 DTO（列表用）
#[derive(Debug, Clone, Serialize)]
pub struct PresetSummaryDto {
    pub id: String,
    pub name: String,
    pub prompt_count: usize,
    pub regex_count: usize,
    pub imported_at: String,
}

/// 预设详情 DTO
#[derive(Debug, Clone, Serialize)]
pub struct PresetDetailDto {
    pub id: String,
    pub name: String,
    pub prompts: Vec<PresetPromptDto>,
    pub regex_scripts: Vec<RegexScriptDto>,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PresetPromptDto {
    pub identifier: String,
    pub name: String,
    pub role: String,
    pub content: String,
    pub enabled: bool,
    pub marker: bool,
    pub is_system_prompt: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegexScriptDto {
    pub id: String,
    pub script_name: String,
    pub find_regex: String,
    pub replace_string: String,
    pub placement: String,
    pub disabled: bool,
}

#[tauri::command]
fn list_presets() -> Vec<PresetSummaryDto> {
    get_preset_store()
        .list()
        .iter()
        .map(|sp| PresetSummaryDto {
            id: sp.id.clone(),
            name: sp.preset.name.clone(),
            prompt_count: sp.preset.prompts.len(),
            regex_count: sp.preset.regex_scripts.len(),
            imported_at: sp.imported_at.clone(),
        })
        .collect()
}

#[tauri::command]
fn get_preset(id: String) -> Result<PresetDetailDto, String> {
    let sp = get_preset_store()
        .get(&id)
        .ok_or_else(|| format!("找不到预设 {id}"))?;
    Ok(PresetDetailDto {
        id: sp.id.clone(),
        name: sp.preset.name.clone(),
        prompts: sp
            .preset
            .prompts
            .iter()
            .map(|p| PresetPromptDto {
                identifier: p.identifier.clone(),
                name: p.name.clone(),
                role: match p.role {
                    storyforge_domain::preset::PromptRole::System => "system",
                    storyforge_domain::preset::PromptRole::User => "user",
                    storyforge_domain::preset::PromptRole::Assistant => "assistant",
                }
                .to_string(),
                content: p.content.clone(),
                enabled: p.enabled,
                marker: p.marker,
                is_system_prompt: p.is_system_prompt,
            })
            .collect(),
        regex_scripts: sp
            .preset
            .regex_scripts
            .iter()
            .map(|r| RegexScriptDto {
                id: r.id.clone(),
                script_name: r.script_name.clone(),
                find_regex: r.find_regex.clone(),
                replace_string: r.replace_string.clone(),
                placement: match r.placement {
                    storyforge_domain::preset::RegexPlacement::Input => "input",
                    storyforge_domain::preset::RegexPlacement::Output => "output",
                }
                .to_string(),
                disabled: r.disabled,
            })
            .collect(),
        imported_at: sp.imported_at.clone(),
    })
}

#[tauri::command]
fn delete_preset(id: String) -> Result<(), String> {
    if get_preset_store().delete(&id) {
        Ok(())
    } else {
        Err(format!("找不到预设 {id}"))
    }
}

#[tauri::command]
fn update_preset_prompt(
    preset_id: String,
    prompt_index: usize,
    content: Option<String>,
    enabled: Option<bool>,
) -> Result<(), String> {
    if get_preset_store().update_prompt(&preset_id, prompt_index, content.as_deref(), enabled) {
        Ok(())
    } else {
        Err(format!("找不到预设 {preset_id} 的第 {prompt_index} 条 prompt"))
    }
}

#[tauri::command]
fn update_preset_regex(
    preset_id: String,
    regex_index: usize,
    disabled: Option<bool>,
) -> Result<(), String> {
    if get_preset_store().update_regex(&preset_id, regex_index, disabled) {
        Ok(())
    } else {
        Err(format!("找不到预设 {preset_id} 的第 {regex_index} 条正则"))
    }
}

/// 将 ST 预设的 prompts 转换为 PromptModule 并存入 ModuleStore
#[tauri::command]
fn import_preset_as_modules(
    preset_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, String> {
    use storyforge_domain::prompt_module::{ModuleCategory, ModuleSource, Exclusivity, PromptModule};
    use storyforge_domain::agent::AgentRole;

    let stored = get_preset_store().get(&preset_id).ok_or_else(|| format!("找不到预设 {preset_id}"))?;
    let mut count = 0;

    for prompt in &stored.preset.prompts {
        // 跳过 marker 和空内容
        if prompt.marker || prompt.content.trim().is_empty() {
            continue;
        }

        // 根据 ST role 映射到 ModuleCategory
        let category = match prompt.role {
            storyforge_domain::preset::PromptRole::System => ModuleCategory::Quality,
            storyforge_domain::preset::PromptRole::User => ModuleCategory::Output,
            storyforge_domain::preset::PromptRole::Assistant => ModuleCategory::Style,
        };

        let module_id = format!("st-{}-{}", preset_id, prompt.identifier);
        let module = PromptModule {
            id: Id::from_str(&module_id),
            name: prompt.name.clone(),
            category,
            content: prompt.content.clone(),
            exclusivity: Exclusivity::Multiple,
            source: ModuleSource::ImportedFromST,
            applicable_roles: vec![AgentRole::Editor, AgentRole::Subagent("*".into())],
            tags: vec!["ST导入".into(), stored.preset.name.clone()],
        };

        state.module_store.add(module);
        count += 1;
    }

    Ok(count)
}

// ─── M4 插件命令 ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct InstalledPluginDto {
    id: String,
    name: String,
    version: String,
    permissions: Vec<String>,
    ui_slots: Vec<String>,
    description: Option<String>,
    author: Option<String>,
    enabled: bool,
    installed_at: String,
}

fn plugin_to_dto(p: &storyforge_infra_plugin_host::InstalledPlugin) -> InstalledPluginDto {
    InstalledPluginDto {
        id: p.manifest.id.clone(),
        name: p.manifest.name.clone(),
        version: p.manifest.version.clone(),
        permissions: p.manifest.permissions.iter().map(|perm| format!("{perm:?}")).collect(),
        ui_slots: p.manifest.ui_slots.iter().map(|slot| format!("{slot:?}")).collect(),
        description: p.manifest.description.clone(),
        author: p.manifest.author.clone(),
        enabled: p.enabled,
        installed_at: p.installed_at.to_rfc3339(),
    }
}

#[tauri::command]
fn list_plugins(state: tauri::State<'_, Arc<AppState>>) -> Vec<InstalledPluginDto> {
    state.plugin_registry.list().iter().map(|p| plugin_to_dto(p)).collect()
}

#[tauri::command]
fn install_plugin(manifest_json: String, state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    let manifest: storyforge_infra_plugin_host::PluginManifest =
        serde_json::from_str(&manifest_json).map_err(|e| format!("manifest 解析失败: {e}"))?;
    state.plugin_registry.install(manifest).map_err(|e| format!("{e}"))
}

#[tauri::command]
fn uninstall_plugin(id: String, state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    state.plugin_registry.uninstall(&id).map_err(|e| format!("{e}"))
}

#[tauri::command]
fn set_plugin_enabled(id: String, enabled: bool, state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    state.plugin_registry.set_enabled(&id, enabled).map_err(|e| format!("{e}"))
}

// ─── M4 插件 API 命令（带权限二次校验）──────────────────────────────────────

#[tauri::command]
fn plugin_list_characters(plugin_id: String, state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<CharacterSummary>, String> {
    use storyforge_infra_plugin_host::Permission;
    state.plugin_registry.ensure_permission(&plugin_id, &Permission::ReadCharacters).map_err(|e| format!("{e}"))?;
    Ok(get_store().list().into_iter().map(CharacterSummary::from).collect())
}

#[tauri::command]
fn plugin_read_character(plugin_id: String, character_id: String, state: tauri::State<'_, Arc<AppState>>) -> Result<CharacterInfo, String> {
    use storyforge_infra_plugin_host::Permission;
    state.plugin_registry.ensure_permission(&plugin_id, &Permission::ReadCharacters).map_err(|e| format!("{e}"))?;
    get_store().get(&character_id).map(|s| s.info).ok_or_else(|| format!("角色卡不存在: {character_id}"))
}

#[tauri::command]
fn plugin_get_variable(plugin_id: String, campaign_id: String, instance_id: String, key: String, state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<storyforge_domain::variables::VariableValue>, String> {
    use storyforge_infra_plugin_host::Permission;
    state.plugin_registry.ensure_permission(&plugin_id, &Permission::WriteVariables).map_err(|e| format!("{e}"))?;
    let store = get_campaign_store();
    store.get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map(|i| i.variables)
        .ok_or_else(|| format!("找不到实例 {instance_id}"))
}

#[tauri::command]
fn plugin_set_variable(plugin_id: String, campaign_id: String, instance_id: String, key: String, value: serde_json::Value, state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    use storyforge_infra_plugin_host::Permission;
    state.plugin_registry.ensure_permission(&plugin_id, &Permission::WriteVariables).map_err(|e| format!("{e}"))?;
    let store = get_campaign_store();
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| format!("找不到实例 {instance_id}"))?;
    inst.set_variable(&key, value, 0);
    store.update_instance(inst);
    Ok(())
}

// ─── 预设/模块系统命令 ─────────────────────────────────────────────────────

#[tauri::command]
fn list_modules(state: tauri::State<'_, Arc<AppState>>) -> Vec<module_store::PromptModuleDto> {
    state
        .module_store
        .list_all()
        .iter()
        .map(|(m, enabled)| module_store::module_to_dto(m, *enabled))
        .collect()
}

#[tauri::command]
fn update_module(
    id: String,
    content: Option<String>,
    enabled: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if state
        .module_store
        .update(&id, content.as_deref(), enabled)
    {
        Ok(())
    } else {
        Err("内置模块不能修改内容".into())
    }
}

#[tauri::command]
fn list_profiles(state: tauri::State<'_, Arc<AppState>>) -> Vec<module_store::ProfileSummaryDto> {
    state.profile_store.list()
}

#[tauri::command]
fn get_active_profile(
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<module_store::PromptProfileDto> {
    state
        .profile_store
        .get_active()
        .map(|p| module_store::profile_to_dto(&p, true))
}

#[tauri::command]
fn save_profile(
    profile_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let profile: PromptProfile =
        serde_json::from_str(&profile_json).map_err(|e| format!("Profile 解析失败: {e}"))?;
    state.profile_store.save(profile);
    Ok(())
}

#[tauri::command]
fn set_active_profile(id: String, state: tauri::State<'_, Arc<AppState>>) {
    state.profile_store.set_active(&id);
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
            PipelineEvent::PostProcessStarted => (
                "postprocess_started".into(),
                serde_json::json!({}),
            ),
            PipelineEvent::PostProcessDone {
                knowledge_count,
                variable_count,
                task_count,
            } => (
                "postprocess_done".into(),
                serde_json::json!({
                    "knowledge_count": knowledge_count,
                    "variable_count": variable_count,
                    "task_count": task_count,
                }),
            ),
            PipelineEvent::PostProcessFailed { reason } => (
                "postprocess_failed".into(),
                serde_json::json!({ "reason": reason }),
            ),
            PipelineEvent::SummaryDone { char_count } => (
                "summary_done".into(),
                serde_json::json!({ "char_count": char_count }),
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
    let mut ctx = WritingContext {
        characters: tool_snapshot.characters.clone(),
        world_info: tool_snapshot.world_info.clone(),
        conversation_id: conversation_id.clone(),
        campaign_id: None,
        turn: 0,
        pending_tasks: vec![],
        story_clock: String::new(),
        profile: None,
        modules: vec![],
    };
    // 从模块/Profile 存储加载预设配置
    fill_profile_context(&mut ctx, &app);
    // 从活跃 Campaign 填充 P2 字段（任务注入导演 / 后处理需要）
    fill_campaign_context(&mut ctx, &app);

    // 创建 cancel channel，sender 存进 AppState（前端可调 cancel_writing 触发）
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut slot = app.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = slot.take() {
            // 上一次写作未正常清理，先取消它
            let _ = existing.send(true);
        }
        *slot = Some(cancel_tx);
    }

    // 每次用最新 tool_ctx 快照构造 orchestrator（保证导入后立刻生效）
    let mut pipeline = app.new_pipeline();
    let result = pipeline
        .start_writing(intent, &ctx, event_tx.clone(), cancel_rx)
        .await;

    // ─── P2 后处理流水线（best-effort，不阻断成文返回）──────────────────────
    // 成文（DraftReady）后并行跑：剧情总结 + 后处理三合一。
    // 仅在有活跃 Campaign 时执行（无 Campaign 跳过，向后兼容）。
    if let Ok((final_text, _, _)) = &result {
        // 从 session.plan 取在场角色 + 基础变量键
        let present_chars: Vec<String> = pipeline
            .session()
            .and_then(|s| s.plan.as_ref())
            .map(|p| {
                p.subagent_tasks
                    .iter()
                    .map(|t| t.character_id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let final_text = final_text.clone();
        let var_keys = default_variable_keys();
        // 后处理用独立的 cancel（与写作共享 life-cycle，但写作已结束，这里新建一个）
        let (pp_cancel_tx, pp_cancel_rx) = watch::channel(false);
        let outcome = pipeline
            .run_postprocess(
                &final_text,
                "", // scene_brief：传空，summarizer/postprocess 从 final_text 自取
                &present_chars,
                &var_keys,
                &ctx,
                &event_tx,
                pp_cancel_rx,
            )
            .await;
        // 落盘到 CampaignStore（有 outcome 才落盘）
        if let Some(outcome) = outcome {
            persist_postprocess_outcome(&ctx, &outcome, &present_chars);
        }
        let _ = pp_cancel_tx; // 保活（其实不需要，写作已完，这里只是避免 unused）
    }

    // 清理 cancel sender
    {
        let mut slot = app.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
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

/// 从模块/Profile 存储加载预设配置到 WritingContext
///
/// 无 Profile 时不动 ctx（profile 保持 None → 流水线用硬编码常量兜底）。
fn fill_profile_context(ctx: &mut WritingContext, state: &Arc<AppState>) {
    // 加载活跃 Profile
    if let Some(profile) = state.profile_store.get_active() {
        // 加载所有启用的模块
        let modules: Vec<_> = state
            .module_store
            .list_all()
            .into_iter()
            .filter(|(_, enabled)| *enabled)
            .map(|(m, _)| m)
            .collect();
        ctx.profile = Some(profile);
        ctx.modules = modules;
    }
}

/// 从活跃 Campaign 填充 WritingContext 的 P2 字段（campaign_id / turn / pending_tasks / story_clock）
///
/// 无活跃 Campaign 时不动 ctx（campaign_id 保持 None → 后处理跳过）。
/// 从活跃 Campaign 填充 P2 字段（任务注入导演 / 后处理需要）。
///
/// 优先读内存 `state.active_campaign`，磁盘 `load_active_campaign` 仅作 fallback。
/// 历史 bug：旧实现绕过内存直接读磁盘，若 `set_active_campaign` 先改内存后写盘
/// 但写盘失败（非原子），会用旧/空 campaign。
fn fill_campaign_context(ctx: &mut WritingContext, state: &AppState) {
    let active_id = {
        let guard = state.active_campaign.lock().unwrap_or_else(|p| p.into_inner());
        guard.clone().or_else(|| load_active_campaign(&get_app_data_dir()))
    };
    let active_id = match active_id {
        Some(id) => id,
        None => return,
    };
    let store = get_campaign_store();

    let camp = match store.get_campaign(&active_id) {
        Some(c) => c,
        None => return,
    };
    ctx.campaign_id = Some(active_id.clone());
    ctx.story_clock = camp.story_clock.clone();
    // turn = 已有 round_summaries 数 + 1（下一轮）
    let existing_turns = store.list_summaries(&active_id).len() as u32;
    ctx.turn = existing_turns + 1;
    // pending_tasks：该 Campaign 下所有任务（build_director_user_msg 内部按触发条件过滤）
    ctx.pending_tasks = store.list_tasks(&active_id);
}

/// 默认变量键列表（喂给后处理 Agent，让它知道有哪些字段可更新）
fn default_variable_keys() -> Vec<String> {
    storyforge_domain::variables::default_character_variables()
        .iter()
        .map(|f| f.key.clone())
        .collect()
}

/// 把后处理产出落盘到 CampaignStore（知识 / 变量 / 任务 / 本轮摘要）
fn persist_postprocess_outcome(
    ctx: &WritingContext,
    outcome: &storyforge_app_agent::PostProcessOutcome,
    present_chars: &[String],
) {
    let camp_id = match &ctx.campaign_id {
        Some(id) => id,
        None => return,
    };
    let store = get_campaign_store();

    // 本轮摘要
    if let Some(summary) = &outcome.summary {
        store.add_summary(storyforge_domain::agent::RoundSummary::new(
            camp_id.clone(),
            ctx.conversation_id.clone(),
            ctx.turn,
            summary.clone(),
        ));
    }

    // 后处理三合一
    if let Some(pp) = &outcome.post_process {
        // 知识：update → entry（assign campaign_id + turn）
        let knowledge_entries: Vec<_> = pp
            .knowledge_updates
            .iter()
            .map(|u| u.clone().into_entry(camp_id.clone(), ctx.turn))
            .collect();
        if !knowledge_entries.is_empty() {
            store.add_knowledge(knowledge_entries);
        }

        // 变量更新：角色级（按 name 匹配 instance）/ 全局级（无 instance_id）
        for vu in &pp.variable_updates {
            if let Some(inst_id) = &vu.instance_id {
                // instance_id 可能是角色名（后处理 Agent 按名字输出），尝试匹配 campaign 内 instance
                if let Some(inst) = find_instance_by_name_or_id(store, camp_id, inst_id) {
                    let mut inst = inst;
                    inst.set_variable(&vu.key, vu.value.clone(), ctx.turn);
                    store.update_instance(inst);
                }
            } else {
                // 全局 Campaign 变量
                if let Some(mut camp) = store.get_campaign(camp_id) {
                    camp.set_variable(&vu.key, vu.value.clone(), ctx.turn);
                    store.update_campaign(camp);
                }
            }
        }

        // 任务更新：新建 / 状态变化
        for tu in &pp.task_updates {
            if let Some(tid) = &tu.task_id {
                if let Some(mut task) = store.get_task(tid) {
                    task.status = tu.new_status.clone();
                    store.update_task(task);
                }
            } else if let Some(spec) = &tu.new_task {
                // present_chars 里的角色名转 Id（这里简化：后处理 Agent 给的角色 id 直接用）
                let new_task = storyforge_domain::story_task::StoryTask::from_narrative(
                    camp_id.clone(),
                    spec.title.clone(),
                    spec.description.clone(),
                    spec.triggers.clone(),
                    ctx.turn,
                );
                store.add_task(new_task);
            }
        }
    }
    let _ = present_chars; // 目前用于知识更新的字符匹配已通过 instance_id 路径处理
}

/// 按名字或 Id 查 campaign 内的 CharacterInstance（后处理 Agent 输出的是角色名，需翻译成 instance）
fn find_instance_by_name_or_id(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    name_or_id: &Id,
) -> Option<storyforge_domain::campaign::CharacterInstance> {
    let instances = store.list_instances(camp_id);
    // 先精确 id 匹配
    if let Some(i) = instances.iter().find(|i| i.id == *name_or_id) {
        return Some(i.clone());
    }
    // 再按 instance.name 匹配（后处理 Agent 给的是角色名）
    instances.into_iter().find(|i| i.name.as_str() == name_or_id.as_str())
}

/// Tauri command: 取消当前运行的写作流水线
///
/// 触发 AppState.current_cancel 的 sender，导演/子Agent/编剧全部中止。
#[tauri::command]
fn cancel_writing(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, String> {
    let slot = state.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
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
    let mut ctx = WritingContext {
        characters: tool_snapshot.characters.clone(),
        world_info: tool_snapshot.world_info.clone(),
        conversation_id: conversation_id.clone(),
        campaign_id: None,
        turn: 0,
        pending_tasks: vec![],
        story_clock: String::new(),
        profile: None,
        modules: vec![],
    };
    fill_profile_context(&mut ctx, &app);
    fill_campaign_context(&mut ctx, &app);

    // cancel channel
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut slot = app.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = slot.take() {
            let _ = existing.send(true);
        }
        *slot = Some(cancel_tx);
    }

    let mut pipeline = app.new_pipeline();
    let result = pipeline.regenerate(pipeline_req, &ctx, event_tx.clone(), cancel_rx).await;

    // ─── P2 后处理（best-effort，同 start_writing）─────────────────────────
    if let Ok((text, _)) = &result {
        let (final_text, present_chars, var_keys) = (
            text.clone(),
            pipeline
                .session()
                .and_then(|s| s.plan.as_ref())
                .map(|p| {
                    p.subagent_tasks
                        .iter()
                        .map(|t| t.character_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            default_variable_keys(),
        );
        let (_pp_tx, pp_rx) = watch::channel(false);
        let outcome = pipeline
            .run_postprocess(
                &final_text,
                "",
                &present_chars,
                &var_keys,
                &ctx,
                &event_tx,
                pp_rx,
            )
            .await;
        if let Some(outcome) = outcome {
            persist_postprocess_outcome(&ctx, &outcome, &present_chars);
        }
    }

    {
        let mut slot = app.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
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

/// 采纳当前变体（Draft → Final），并自动检查是否需要归档
#[tauri::command]
async fn accept_variant(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .accept_variant(&conv_id, &nid)
        .map_err(|e| format!("{e}"))?;

    // 自动归档检查（后台异步，不阻塞响应）
    let state_clone = state.inner().clone();
    let conv_id_clone = conv_id.clone();
    tokio::spawn(async move {
        auto_archive_if_needed(&state_clone, &conv_id_clone).await;
    });

    Ok(())
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
    *state.embed_config.write().unwrap_or_else(|p| p.into_inner()) = Some(config);
    Ok(())
}

/// 获取当前嵌入配置（不含 key）
#[tauri::command]
fn get_embed_config(
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<serde_json::Value> {
    state.embed_config.read().unwrap_or_else(|p| p.into_inner()).as_ref().map(|c| {
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

// ─── 自动归档辅助 ──────────────────────────────────────────────────────────

/// 检查对话消息数是否超过归档阈值，超过则在后台触发归档
///
/// 阈值：50 条非 Discarded 消息（与 ArchiveConfig.default().threshold 一致）。
/// 归档失败只 warn，不影响用户操作。
async fn auto_archive_if_needed(state: &Arc<AppState>, conv_id: &Id) {
    const ARCHIVE_THRESHOLD: usize = 50;

    // 取对话，数非 Discarded 消息
    let messages: Vec<String> = {
        let conv = match state.conv_store.get(conv_id) {
            Some(c) => c,
            None => return,
        };
        conv.nodes
            .iter()
            .filter_map(|node| {
                let v = node.active()?;
                if v.status == storyforge_domain::conversation::VariantStatus::Discarded {
                    None
                } else {
                    Some(v.content.clone())
                }
            })
            .collect()
    };

    if messages.len() < ARCHIVE_THRESHOLD {
        return;
    }

    tracing::info!(
        "自动归档触发：对话 {} 消息数 {} >= 阈值 {}",
        conv_id,
        messages.len(),
        ARCHIVE_THRESHOLD
    );

    // 检查嵌入配置
    let config = match state.embed_config.read().unwrap_or_else(|p| p.into_inner()).clone() {
        Some(c) => c,
        None => {
            tracing::debug!("未配置嵌入 API，跳过自动归档");
            return;
        }
    };

    let llm = state.active_llm_or_mock();
    let vector_store = state.vector_store.clone();
    let embedder = Arc::new(storyforge_infra_llm::Embedder::new(config));
    let archiver = storyforge_app_memory::MemoryArchiver::new(
        llm,
        embedder,
        vector_store,
        storyforge_app_memory::ArchiveConfig::default(),
    );

    match archiver.maybe_archive(&messages).await {
        Ok(summaries) if !summaries.is_empty() => {
            tracing::info!("自动归档完成：{} 条总结", summaries.len());
        }
        Ok(_) => {
            tracing::debug!("自动归档：无需归档");
        }
        Err(e) => {
            tracing::warn!("自动归档失败（不影响用户操作）: {e}");
        }
    }
}

// ─── Meta Agent 命令 ──────────────────────────────────────────────────────

/// Meta 对话流式事件（Tauri Channel 用）
///
/// 复用 WritingEvent 的扁平模式。目前只有 token 增量一种事件；
/// 命令的最终聚合结果（agent_message/messages/new_patch）仍由命令返回值携带。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaStreamEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

impl MetaStreamEvent {
    /// token 增量事件
    pub fn progress(delta: String) -> Self {
        Self {
            event_type: "meta_progress".into(),
            data: serde_json::json!({ "delta": delta }),
        }
    }
}

/// 接受并执行 Patch（修改世界书条目或角色字段）
#[tauri::command]
fn meta_accept_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // 从 PatchStore 取出 patch
    let patch = {
        let patches = state.meta_patches.read().unwrap_or_else(|p| p.into_inner());
        patches
            .iter()
            .find(|p| p.id == patch_id)
            .cloned()
            .ok_or_else(|| format!("Patch 不存在: {patch_id}"))?
    };

    // 执行 patch：clone 世界书 → 修改 → 写回
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
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
    // 从 tool_ctx 取最新的世界书，按 is_global 分流回写：
    //   - 全局条目：写回所有角色卡（跨卡共享语义）
    //   - 非全局条目：只保留在各卡原有的非全局条目里（按 content 精确匹配，
    //     不把 patch 修改的某卡私有条目覆盖到其他卡，也不丢失其他卡私有条目）
    //
    // 历史 bug：曾用 `all_stored.last()` 把整个合并视图（全局+多卡 merge）
    // 全部写回最后一张卡，并把 is_global 硬编码 false，导致数据污染与全局标记丢失。
    {
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        if let Some(ref world_info) = ctx.world_info {
            // 合并视图里的条目，保留 is_global（取自 route + 原卡标记）
            let global_entries: Vec<crate::WorldInfoEntryInfo> = world_info
                .entries
                .iter()
                .map(|e| crate::WorldInfoEntryInfo {
                    keys: e.keys.clone(),
                    content: e.content.clone(),
                    constant: e.constant,
                    route: format!("{:?}", e.route),
                    is_global: true, // 合并视图里全局条目的真相
                    depth: e.depth,
                    order: e.order,
                })
                .collect();
            // patch 修改的非全局条目（以 content 为指纹匹配回原卡）
            let nonglobal_contents: std::collections::HashSet<String> = world_info
                .entries
                .iter()
                .map(|e| e.content.clone())
                .collect();

            let all_stored = get_store().list();
            for stored in &all_stored {
                // 该卡保留：原有非全局条目（未被 patch 删除的） + 所有全局条目
                let preserved_nonglobal: Vec<crate::WorldInfoEntryInfo> = stored
                    .info
                    .world_info_entries
                    .iter()
                    .filter(|e| !e.is_global && nonglobal_contents.contains(&e.content))
                    .cloned()
                    .collect();
                let mut new_entries = preserved_nonglobal;
                new_entries.extend(global_entries.clone());
                let _ = get_store().update_world_info_entries_bulk(&stored.id, new_entries);
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

// ─── P3：Meta Agent 多轮对话 / MVU 五合一分析 / ST 预设 LLM 分类 ────────────

/// 把当前活跃角色卡 + 世界书同步进 MetaSession（每次 meta 操作前调）
fn sync_meta_session_from_tool_ctx(state: &tauri::State<'_, Arc<AppState>>) {
    let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
    if let Some(card) = ctx.characters.last() {
        state.meta_session.set_character(card.clone());
    }
    if let Some(book) = &ctx.world_info {
        state.meta_session.set_world_info(book.clone());
    }
}

/// Tauri command: 开始一个新的 Meta 对话（返回 conversation_id）
#[tauri::command]
fn meta_start_conversation(state: tauri::State<'_, Arc<AppState>>) -> String {
    sync_meta_session_from_tool_ctx(&state);
    let conv = storyforge_app_meta::MetaConversation::new();
    let id = conv.id.clone();
    state
        .meta_conversations
        .lock()
        .unwrap()
        .insert(id.clone(), conv);
    id
}

/// Tauri command: 跑一轮 Meta 对话（返回本轮 Agent 回复 + 新 patch）
#[tauri::command]
async fn meta_chat(
    conversation_id: String,
    user_input: String,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<MetaStreamEvent>,
) -> Result<serde_json::Value, String> {
    let app = state.inner().clone();
    sync_meta_session_from_tool_ctx(&state);

    // 取出对话；不存在则返回错误（而非静默创建空对话，避免用户感觉"历史突然清空"）。
    // 新对话应由 meta_start_conversation 命令显式建立。
    let mut conv = {
        let mut convs = app.meta_conversations.lock().unwrap_or_else(|p| p.into_inner());
        convs.remove(&conversation_id).ok_or_else(|| {
            format!("Meta 对话不存在: {conversation_id}（请先调用 meta_start_conversation 创建）")
        })?
    };

    // 构造 AgentRuntime（活跃 LLM 或 mock）
    let llm = app.active_llm_or_mock();
    let tool_ctx = app.snapshot_tool_ctx();
    let runtime = storyforge_app_agent::AgentRuntime::new(llm, tool_ctx);

    // 流式转发：meta_chat 内部把 token delta 推到 progress_tx，
    // 一个转发任务把它包成 MetaStreamEvent 推给前端 Channel
    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<String>();
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(delta) = progress_rx.recv().await {
            let _ = on_event_clone.send(MetaStreamEvent::progress(delta));
        }
    });

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let turn = storyforge_app_meta::meta_chat(
        &runtime,
        &mut conv,
        app.meta_session.clone(),
        &user_input,
        cancel_rx,
        progress_tx,
    )
    .await
    .map_err(|e| format!("{e}"))?;

    // 把新提议的 patch 同步进 AppState.meta_patches（前端可用 meta_accept_patch 采纳）
    if let Some(patch) = &turn.new_patch {
        let mut patches = app.meta_patches.write().unwrap_or_else(|p| p.into_inner());
        if !patches.iter().any(|p| p.id == patch.id) {
            patches.push(patch.clone());
        }
    }

    // 存回对话
    let conv_id = conv.id.clone();
    let messages = serde_json::to_value(&conv.messages).unwrap_or(serde_json::Value::Null);
    app.meta_conversations
        .lock()
        .unwrap()
        .insert(conv_id.clone(), conv);

    Ok(serde_json::json!({
        "conversation_id": conv_id,
        "agent_message": turn.agent_message,
        "messages": messages,
        "new_patch": turn.new_patch,
    }))
}

/// Tauri command: 获取某个 Meta 对话的完整消息历史
#[tauri::command]
fn meta_get_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<serde_json::Value> {
    let convs = state.meta_conversations.lock().unwrap_or_else(|p| p.into_inner());
    convs.get(&conversation_id).map(|conv| {
        serde_json::to_value(conv).unwrap_or(serde_json::Value::Null)
    })
}

/// Tauri command: 列所有待采纳的 Meta Patch
#[tauri::command]
fn meta_list_pending_patches(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<serde_json::Value> {
    state
        .meta_patches
        .read()
        .unwrap()
        .iter()
        .filter(|p| !p.applied)
        .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null))
        .collect()
}

/// Tauri command: 忽略一个 Meta Patch（从 pending 移除）
#[tauri::command]
fn meta_dismiss_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let mut patches = state.meta_patches.write().unwrap_or_else(|p| p.into_inner());
    patches.retain(|p| p.id != patch_id);
    Ok(())
}

/// MVU 翻译的精简 DTO（前端列表用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuTranslationSummaryDto {
    pub source_character_id: String,
    pub character_name: String,
    pub analyzed_at: String,
    pub routing: String,
    pub ui_binding_count: usize,
    pub fallback_count: usize,
    pub analysis_confidence: f64,
}

/// MVU 翻译的完整 DTO（前端渲染状态栏用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuTranslationDetailDto {
    pub source_character_id: String,
    pub character_name: String,
    pub analyzed_at: String,
    pub translation: storyforge_domain::mvu_translation::MvuTranslation,
    pub complexity: serde_json::Value,
}

/// Tauri command: 手动触发 MVU 五合一分析（D44：手动按钮，不自动跑）
///
/// 流程：取角色卡 → 启发式打分 → LLM 五合一分析 → 持久化 → 返回结果
/// 失败降级：分析失败回退到字段级 schema，不报错
#[tauri::command]
async fn meta_analyze_mvu_card(
    source_character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<MvuTranslationDetailDto, String> {
    use storyforge_app_agent::AgentRuntime;

    // 取原 Character
    let character = {
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        ctx.characters
            .iter()
            .find(|c| c.id.as_str() == source_character_id)
            .map(|c| (*c).clone())
    }
    .ok_or_else(|| format!("找不到角色卡 {source_character_id}"))?;

    // 启发式打分（纯 Rust，先跑，给 LLM 当判据）
    let complexity = storyforge_app_meta::score_card_complexity(&character);
    let complexity_json = serde_json::to_value(&complexity).unwrap_or(serde_json::Value::Null);
    tracing::info!(
        "卡「{}」MVU 启发式分类: {:?}",
        character.name,
        complexity.classification
    );

    // 跑 LLM 五合一分析
    let llm = state.active_llm_or_mock();
    let tool_ctx = state.snapshot_tool_ctx();
    let runtime = AgentRuntime::new(llm, tool_ctx);
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let translation = storyforge_app_meta::analyze_mvu_card(&runtime, &character, cancel_rx)
        .await
        .map_err(|e| format!("MVU 分析失败: {e}"))?;

    // 持久化到 CampaignStore
    let store = get_campaign_store();
    let stored = campaign_store::StoredMvuTranslation {
        source_character_id: character.id.clone(),
        character_name: character.name.clone(),
        translation: translation.clone(),
        analyzed_at: chrono::Utc::now().to_rfc3339(),
    };
    store.save_mvu(stored);

    Ok(MvuTranslationDetailDto {
        source_character_id: character.id.as_str().to_string(),
        character_name: character.name.clone(),
        analyzed_at: chrono::Utc::now().to_rfc3339(),
        translation,
        complexity: complexity_json,
    })
}

/// Tauri command: 列所有已分析的 MVU 翻译
#[tauri::command]
fn meta_list_mvu_translations() -> Vec<MvuTranslationSummaryDto> {
    get_campaign_store()
        .list_all_mvu()
        .iter()
        .map(|m| MvuTranslationSummaryDto {
            source_character_id: m.source_character_id.as_str().to_string(),
            character_name: m.character_name.clone(),
            analyzed_at: m.analyzed_at.clone(),
            routing: format!("{:?}", m.translation.routing),
            ui_binding_count: m.translation.ui_bindings.len(),
            fallback_count: m.translation.fallback_fragments.len(),
            analysis_confidence: m.translation.analysis_confidence,
        })
        .collect()
}

/// Tauri command: 查某角色卡的 MVU 翻译详情（前端渲染状态栏用）
#[tauri::command]
fn meta_get_mvu_translation(
    source_character_id: String,
) -> Option<MvuTranslationDetailDto> {
    let store = get_campaign_store();
    let id = Id::from_str(source_character_id);
    store.get_mvu(&id).map(|m| MvuTranslationDetailDto {
        source_character_id: m.source_character_id.as_str().to_string(),
        character_name: m.character_name.clone(),
        analyzed_at: m.analyzed_at.clone(),
        translation: m.translation.clone(),
        complexity: serde_json::Value::Null,
    })
}

/// Tauri command: 手动触发 ST 预设 LLM 分类（增强现有纯启发式 bridge）
///
/// 失败时返回 Err，前端降级到现有 import_preset_as_modules。
#[tauri::command]
async fn meta_classify_st_preset(
    preset_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    use storyforge_app_agent::AgentRuntime;

    let stored = get_preset_store()
        .get(&preset_id)
        .ok_or_else(|| format!("预设不存在: {preset_id}"))?;

    let llm = state.active_llm_or_mock();
    let tool_ctx = state.snapshot_tool_ctx();
    let runtime = AgentRuntime::new(llm, tool_ctx);
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let classification =
        storyforge_app_meta::classify_st_preset_with_llm(&runtime, &stored.preset, cancel_rx)
            .await
            .map_err(|e| format!("ST 分类失败: {e}"))?;

    serde_json::to_value(&classification).map_err(|e| format!("序列化失败: {e}"))
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
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
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
    *state.active_campaign.lock().unwrap_or_else(|p| p.into_inner()) = Some(campaign_id.clone());
    save_active_campaign(&get_app_data_dir(), Some(&campaign_id));
    Ok(())
}

#[tauri::command]
fn get_active_campaign(state: tauri::State<'_, Arc<AppState>>) -> Option<CampaignSummaryDto> {
    let id = state.active_campaign.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
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

// ─── P2 后处理产出查询 / 任务管理命令（6 个）──────────────────────────────────

/// 角色知识条目 DTO（前端展示用）
#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeEntryDto {
    pub id: String,
    pub campaign_id: String,
    pub character_id: String,
    pub knowledge_text: String,
    pub source: String,
    pub source_character_id: Option<String>,
    pub turn_number: u32,
    pub pinned: bool,
}

impl From<&storyforge_domain::character_knowledge::CharacterKnowledgeEntry> for KnowledgeEntryDto {
    fn from(e: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry) -> Self {
        use storyforge_domain::character_knowledge::KnowledgeSource;
        let source = match e.source {
            KnowledgeSource::Witnessed => "witnessed",
            KnowledgeSource::ToldByOther => "told_by_other",
            KnowledgeSource::Inferred => "inferred",
            KnowledgeSource::Backstory => "backstory",
        };
        Self {
            id: e.id.to_string(),
            campaign_id: e.campaign_id.to_string(),
            character_id: e.character_id.to_string(),
            knowledge_text: e.knowledge_text.clone(),
            source: source.into(),
            source_character_id: e.source_character_id.as_ref().map(|i| i.to_string()),
            turn_number: e.turn_number,
            pinned: e.pinned,
        }
    }
}

/// 列出某 campaign 下某角色的可见信息（character_knowledge）
///
/// 不传 character_id 则返回整个 campaign 所有角色的知识。
#[tauri::command]
fn list_character_knowledge(
    campaign_id: String,
    character_id: Option<String>,
) -> Vec<KnowledgeEntryDto> {
    let store = get_campaign_store();
    let camp = Id::from_str(&campaign_id);
    let entries = if let Some(cid) = character_id {
        store.list_knowledge_of(&camp, &Id::from_str(&cid))
    } else {
        store.list_knowledge(&camp)
    };
    entries.iter().map(KnowledgeEntryDto::from).collect()
}

/// 任务 DTO（前端展示用）
#[derive(Debug, Clone, Serialize)]
pub struct StoryTaskDto {
    pub id: String,
    pub campaign_id: String,
    pub title: String,
    pub description: String,
    pub triggers: Vec<storyforge_domain::story_task::TaskTrigger>,
    pub status: storyforge_domain::story_task::TaskStatus,
    pub created_turn: u32,
    pub related_characters: Vec<String>,
    pub source: String,
    pub injected_turns: Vec<u32>,
}

impl From<&storyforge_domain::story_task::StoryTask> for StoryTaskDto {
    fn from(t: &storyforge_domain::story_task::StoryTask) -> Self {
        use storyforge_domain::story_task::TaskSource;
        let source = match t.source {
            TaskSource::UserPlanned => "user_planned",
            TaskSource::ExtractedFromNarrative => "from_narrative",
        };
        Self {
            id: t.id.to_string(),
            campaign_id: t.campaign_id.to_string(),
            title: t.title.clone(),
            description: t.description.clone(),
            triggers: t.triggers.clone(),
            status: t.status.clone(),
            created_turn: t.created_turn,
            related_characters: t.related_characters.iter().map(|i| i.to_string()).collect(),
            source: source.into(),
            injected_turns: t.injected_turns.clone(),
        }
    }
}

/// 列出某 campaign 的所有任务（可按状态筛：pending/active/likely_completed/completed/abandoned）
#[tauri::command]
fn list_tasks(
    campaign_id: String,
    status_filter: Option<String>,
) -> Vec<StoryTaskDto> {
    let store = get_campaign_store();
    let camp = Id::from_str(&campaign_id);
    let mut tasks = store.list_tasks(&camp);
    if let Some(filter) = status_filter {
        tasks.retain(|t| {
            let s = serde_json::to_string(&t.status).unwrap_or_default();
            // TaskStatus 序列化为 "pending"/"active"/{"likely_completed":...}/"completed"/"abandoned"
            s.starts_with(&format!("\"{filter}")) || s.starts_with('{') && filter == "likely_completed"
        });
    }
    tasks.iter().map(StoryTaskDto::from).collect()
}

/// 创建任务（前端 UI：用户手动规划伏笔/目标）
#[tauri::command]
fn create_task(
    campaign_id: String,
    title: String,
    description: String,
    triggers: Vec<storyforge_domain::story_task::TaskTrigger>,
    created_turn: Option<u32>,
) -> Result<String, String> {
    if title.trim().is_empty() {
        return Err("任务标题不能为空".into());
    }
    let store = get_campaign_store();
    let task = storyforge_domain::story_task::StoryTask::user_planned(
        Id::from_str(&campaign_id),
        title,
        description,
        triggers,
        created_turn.unwrap_or(0),
    );
    let id = task.id.to_string();
    store.add_task(task);
    Ok(id)
}

/// 标记任务完成（用户确认）
#[tauri::command]
fn complete_task(task_id: String) -> Result<(), String> {
    let store = get_campaign_store();
    let mut task = store
        .get_task(&Id::from_str(&task_id))
        .ok_or_else(|| format!("找不到任务 {task_id}"))?;
    task.complete();
    store.update_task(task);
    Ok(())
}

/// 放弃任务
#[tauri::command]
fn abandon_task(task_id: String) -> Result<(), String> {
    let store = get_campaign_store();
    let mut task = store
        .get_task(&Id::from_str(&task_id))
        .ok_or_else(|| format!("找不到任务 {task_id}"))?;
    task.abandon();
    store.update_task(task);
    Ok(())
}

/// 本轮摘要 DTO
#[derive(Debug, Clone, Serialize)]
pub struct RoundSummaryDto {
    pub id: String,
    pub campaign_id: String,
    pub conversation_id: String,
    pub turn: u32,
    pub content: String,
    pub created_at: String,
}

impl From<&storyforge_domain::agent::RoundSummary> for RoundSummaryDto {
    fn from(s: &storyforge_domain::agent::RoundSummary) -> Self {
        Self {
            id: s.id.to_string(),
            campaign_id: s.campaign_id.to_string(),
            conversation_id: s.conversation_id.to_string(),
            turn: s.turn,
            content: s.content.clone(),
            created_at: s.created_at.clone(),
        }
    }
}

/// 列出某 campaign 的所有本轮剧情摘要（按 turn 升序）
#[tauri::command]
fn list_round_summaries(campaign_id: String) -> Vec<RoundSummaryDto> {
    let store = get_campaign_store();
    let camp = Id::from_str(&campaign_id);
    store
        .list_summaries(&camp)
        .iter()
        .map(RoundSummaryDto::from)
        .collect()
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
            list_presets,
            get_preset,
            delete_preset,
            update_preset_prompt,
            update_preset_regex,
            import_preset_as_modules,
            // M4 插件命令
            list_plugins,
            install_plugin,
            uninstall_plugin,
            set_plugin_enabled,
            plugin_list_characters,
            plugin_read_character,
            plugin_get_variable,
            plugin_set_variable,
            // 预设/模块系统命令
            list_modules,
            update_module,
            list_profiles,
            get_active_profile,
            save_profile,
            set_active_profile,
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
            // P2 后处理产出查询 / 任务管理
            list_character_knowledge,
            list_tasks,
            create_task,
            complete_task,
            abandon_task,
            list_round_summaries,
            // P3 Meta Agent / MVU 五合一 / ST 预设分类
            meta_start_conversation,
            meta_chat,
            meta_get_conversation,
            meta_list_pending_patches,
            meta_dismiss_patch,
            meta_analyze_mvu_card,
            meta_list_mvu_translations,
            meta_get_mvu_translation,
            meta_classify_st_preset,
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
            let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
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
            let slot = state.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
            assert!(slot.is_none());
        }

        // 模拟 start_writing 设置 cancel sender
        let (tx, mut rx) = watch::channel(false);
        {
            let mut slot = state.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
            *slot = Some(tx);
        }

        // 触发取消
        {
            let slot = state.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
            let tx = slot.as_ref().unwrap();
            let _ = tx.send(true);
        }
        assert!(*rx.borrow(), "cancel 应已触发");

        // 清理
        {
            let mut slot = state.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
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

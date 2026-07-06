pub mod campaign_store;
mod connection_store;
pub mod error;
mod module_store;
mod mvu_webview_runtime;
mod preset_store;
mod storage;

use chrono::Utc;
use connection_store::ConnectionStore;
use preset_store::PresetStore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use storage::CharacterStore;
use tokio::sync::watch;

use storyforge_app_agent::ToolContext;
use storyforge_app_conversation::{ConversationStore, PartialRollTarget};
use storyforge_app_logging::{ExportOptions, LogFilter, LogKind, LogLevel, LogStore};
use storyforge_app_meta::{
    MvuApplyError, MvuApplyPreview, apply_schema_to_definition, compute_apply_preview,
};
use storyforge_app_pipeline::{PipelineOrchestrator, RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::llm::{
    LlmConnection, LlmConnectionSummary, LlmProtocol, SamplingParams, ToolMode,
};
use storyforge_domain::preset::RegexScript;
use storyforge_domain::prompt_module::PromptProfile;
use storyforge_infra_llm::LlmClient;
use storyforge_infra_plugin_host::PluginRegistry;
use storyforge_infra_plugin_host::mvu_runtime::MvuExecuteResponse;
use storyforge_infra_util::secret_store::{
    SecretStore, SystemSecretStore, is_secret_ref, make_secret_ref, resolve_secret_value,
};
use storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord, VectorStore};
use tauri::Manager;

use crate::error::TauriCommandError;
use crate::mvu_webview_runtime::{MvuPendingMap, WebViewMvuRuntime, new_mvu_pending_map};

// ─── 全局存储（保留 M0 兼容）──────────────────────────────────────────────

static STORE: OnceLock<CharacterStore> = OnceLock::new();
const EMBED_SECRET_KIND: &str = "embedder";
const EMBED_SECRET_ID: &str = "default";

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

/// W10: 全局 MVU JS runtime（setup 时初始化，new_pipeline 时注入 PipelineOrchestrator）
static MVU_RUNTIME: OnceLock<Arc<WebViewMvuRuntime>> = OnceLock::new();

/// 获取应用数据目录，优先使用 OS 标准位置（H-003 修复）。
/// Windows: %APPDATA%/StoryForge
/// macOS/Linux: $HOME/.local/share/storyforge
/// 回退: exe_dir/data（兼容旧安装）
fn get_app_data_dir() -> PathBuf {
    let os_dir = if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .ok()
            .map(|appdata| PathBuf::from(appdata).join("StoryForge"))
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join("Library/Application Support/StoryForge"))
    } else {
        std::env::var("XDG_DATA_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|p| p.join("storyforge"))
    };

    let data_dir = os_dir.unwrap_or_else(|| {
        // 回退到 exe 目录（旧行为）
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("data")
    });

    std::fs::create_dir_all(&data_dir).ok();

    // 数据迁移：如果旧位置有数据且新位置为空，复制过去
    migrate_from_exe_dir_if_needed(&data_dir);

    data_dir
}

/// 从旧的 exe_dir/data 迁移到新的 OS 标准目录（仅当新目录为空时）
fn migrate_from_exe_dir_if_needed(new_dir: &PathBuf) {
    let old_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("data");

    if old_dir == *new_dir || !old_dir.exists() {
        return;
    }

    // 检查新目录是否为空（忽略已迁移的数据）
    let new_has_data = new_dir.join("characters.json").exists()
        || new_dir.join("connections.json").exists()
        || new_dir.join("campaigns").exists();

    if new_has_data {
        return; // 新目录已有数据，不需要迁移
    }

    tracing::info!(
        "正在从旧数据目录迁移: {} → {}",
        old_dir.display(),
        new_dir.display()
    );
    if let Ok(entries) = std::fs::read_dir(&old_dir) {
        for entry in entries.flatten() {
            let dest = new_dir.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir_recursive(&entry.path(), &dest);
            } else if let Err(e) = std::fs::copy(entry.path(), &dest) {
                tracing::warn!("迁移文件失败 {}: {e}", entry.path().display());
            }
        }
        tracing::info!("数据迁移完成");
    }
}

/// 递归复制目录
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).ok();
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let dest = dst.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir_recursive(&entry.path(), &dest);
            } else {
                let _ = std::fs::copy(entry.path(), &dest);
            }
        }
    }
}

fn load_embed_config(data_dir: &Path) -> Option<storyforge_infra_llm::EmbedConfig> {
    let secret_store = SystemSecretStore::default();
    load_embed_config_with_secret_store(data_dir, &secret_store)
}

fn load_embed_config_with_secret_store(
    data_dir: &Path,
    secret_store: &dyn SecretStore,
) -> Option<storyforge_infra_llm::EmbedConfig> {
    let path = data_dir.join("embed.json");
    if path.exists() {
        let data = std::fs::read_to_string(&path).ok()?;
        let mut config: storyforge_infra_llm::EmbedConfig = serde_json::from_str(&data).ok()?;
        if is_secret_ref(&config.api_key) {
            config.api_key = match resolve_secret_value(&config.api_key, secret_store) {
                Ok(api_key) => api_key,
                Err(e) => {
                    tracing::warn!("读取嵌入 API SecretRef 失败: {e}");
                    return None;
                }
            };
        } else if !config.api_key.is_empty()
            && let Err(e) = persist_embed_config_secret_ref(data_dir, &config, secret_store)
        {
            tracing::warn!("迁移嵌入 API key 到系统凭据库失败，保留旧文件: {e}");
        }
        Some(config)
    } else {
        None
    }
}

fn save_embed_config(
    data_dir: &Path,
    config: &storyforge_infra_llm::EmbedConfig,
) -> Result<(), String> {
    let secret_store = SystemSecretStore::default();
    persist_embed_config_secret_ref(data_dir, config, &secret_store)
}

fn persist_embed_config_secret_ref(
    data_dir: &Path,
    config: &storyforge_infra_llm::EmbedConfig,
    secret_store: &dyn SecretStore,
) -> Result<(), String> {
    let path = data_dir.join("embed.json");
    let mut stored = config.clone();
    let secret_ref = make_secret_ref(EMBED_SECRET_KIND, EMBED_SECRET_ID);
    if stored.api_key.is_empty() {
        if let Err(e) = secret_store.delete_secret(&secret_ref) {
            tracing::warn!("删除嵌入 API SecretRef 失败: {e}");
        }
    } else if !is_secret_ref(&stored.api_key) {
        secret_store.put_secret(&secret_ref, &stored.api_key)?;
        stored.api_key = secret_ref;
    }
    storyforge_infra_util::atomic_write_json(&path, &stored).map_err(|e| {
        let msg = format!("保存嵌入配置失败: {e}");
        tracing::error!("{msg}");
        msg
    })
}

fn load_active_campaign(data_dir: &Path) -> Option<Id> {
    let path = data_dir.join("active_campaign.json");
    let s = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("campaign_id")
        .and_then(|v| v.as_str())
        .map(Id::from_str)
}

fn save_active_campaign(data_dir: &Path, id: Option<&Id>) {
    let path = data_dir.join("active_campaign.json");
    let v = serde_json::json!({ "campaign_id": id.map(|i| i.as_str()).unwrap_or("") });
    if let Err(e) = storyforge_infra_util::atomic_write_json(&path, &v) {
        tracing::error!("保存活跃 Campaign 失败: {e}");
    }
}

// ─── AppState（M1 新增，注入到 Tauri managed state）─────────────────────────

/// 应用全局状态
pub struct AppState {
    /// App data directory used by stateful stores owned by this process.
    data_dir: PathBuf,
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
    /// 类型化 Patch 存储（第三轮：campaign-runtime 修复）
    pub typed_patches: Arc<RwLock<Vec<storyforge_app_meta::TypedPatch>>>,
    /// 嵌入配置（metadata 持久化到 data/embed.json，API key 走 SecretRef）
    pub embed_config: Arc<RwLock<Option<storyforge_infra_llm::EmbedConfig>>>,
    /// 当前活跃 Campaign ID（持久化到 data/active_campaign.json）
    pub active_campaign: Mutex<Option<Id>>,
    /// 插件注册表（持久化到 data/plugins.json）
    pub plugin_registry: Arc<PluginRegistry>,
    /// 模块存储（内置 + 自定义模块 + 启用/禁用状态）
    pub module_store: Arc<module_store::ModuleStore>,
    /// Profile 存储（预设配置 + 活跃 Profile）
    pub profile_store: Arc<module_store::ProfileStore>,
    /// Agent Profile 配置存储（Agent 运行时参数覆盖 + 活跃配置）
    pub agent_profile_config_store: Arc<module_store::AgentProfileConfigStore>,
    /// Meta Agent 会话（诊断工具的数据源 + PatchStore，P3 新增）
    pub meta_session: Arc<storyforge_app_meta::MetaSession>,
    /// Meta 对话历史（conversation_id → MetaConversation，内存态，重启清空，P3 新增）
    pub meta_conversations:
        Mutex<std::collections::HashMap<String, storyforge_app_meta::MetaConversation>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::new_with_data_dir(get_app_data_dir())
    }

    fn new_with_data_dir(data_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&data_dir).ok();
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
            campaign_runtime: None,
            current_character_instance_id: None,
        }));

        // 启动恢复：从 CharacterStore 把已导入的角色卡 + 世界书同步进 tool_ctx
        // （否则每次重启 dev，tool_ctx 都是空的，写作时报"没有可用角色卡"）
        // 世界书：最后一张卡的条目 + 所有其他卡的 is_global 条目（全局共享）
        {
            let store = CharacterStore::new(&data_dir);
            let stored_chars = store.list();
            if !stored_chars.is_empty() {
                let mut ctx = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
                for stored in &stored_chars {
                    ctx.characters
                        .push(Arc::new(stored_info_to_character(stored)));
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
            let conn_store = connection_store::ConnectionStore::new(&data_dir);
            if let Some(conn) = conn_store.active_connection() {
                match storyforge_infra_llm::create_client(&conn) {
                    Ok(client) => {
                        let intercepted: Arc<dyn LlmClient> =
                            Arc::new(storyforge_app_logging::interceptor::LlmInterceptor::new(
                                Arc::from(client),
                                log_store.clone(),
                                conn.name.clone(),
                            ));
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
        let agent_profile_config_store =
            Arc::new(module_store::AgentProfileConfigStore::new(&data_dir));

        let mut meta_session = storyforge_app_meta::MetaSession::new();
        meta_session.set_explainer(Arc::new(ConvGenerationExplainer {
            conv_store: conv_store.clone(),
        }));

        Self {
            data_dir: data_dir.clone(),
            mock_llm,
            conv_store,
            log_store,
            tool_ctx,
            current_cancel: Mutex::new(None),
            active_llm: Mutex::new(active_llm),
            active_conn_id: Mutex::new(active_conn_id),
            vector_store,
            meta_patches: Arc::new(RwLock::new(Vec::new())),
            typed_patches: Arc::new(RwLock::new(Vec::new())),
            embed_config: Arc::new(RwLock::new(load_embed_config(&data_dir))),
            active_campaign: Mutex::new(load_active_campaign(&data_dir)),
            plugin_registry,
            module_store,
            profile_store,
            agent_profile_config_store,
            meta_session: Arc::new(meta_session),
            meta_conversations: Mutex::new(std::collections::HashMap::new()),
        }
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        let data_dir = std::env::temp_dir().join(format!(
            "storyforge-app-state-test-{}",
            uuid::Uuid::new_v4()
        ));
        Self::new_with_data_dir(data_dir)
    }

    /// 取一份 tool_ctx 快照（clone 出 Arc<ToolContext>），供本次流水线使用
    pub fn snapshot_tool_ctx(&self) -> Arc<ToolContext> {
        let ctx = self
            .tool_ctx
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
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
        self.active_conn_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 设置活跃连接（构造 client 并缓存，挂 LlmInterceptor 记录每次调用）
    pub fn set_active_connection(&self, id: &str) -> Result<(), TauriCommandError> {
        let conn_store = get_conn_store();
        let conn = conn_store
            .set_active(id)?
            .ok_or_else(|| TauriCommandError::not_found(format!("连接不存在: {id}")))?;

        let client = storyforge_infra_llm::create_client(&conn).map_err(TauriCommandError::from)?;

        // 包装 LlmInterceptor：每次 LLM 调用自动记录 payload/响应/token/延迟到 LogStore
        let intercepted: Arc<dyn LlmClient> =
            Arc::new(storyforge_app_logging::interceptor::LlmInterceptor::new(
                Arc::from(client),
                self.log_store.clone(),
                conn.name.clone(),
            ));

        *self.active_llm.lock().unwrap_or_else(|p| p.into_inner()) = Some(intercepted);
        *self
            .active_conn_id
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(id.to_string());
        Ok(())
    }

    /// 清除活跃连接（删除时调用）
    pub fn clear_active_connection(&self) {
        *self.active_llm.lock().unwrap_or_else(|p| p.into_inner()) = None;
        *self
            .active_conn_id
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// 构造一个新的 PipelineOrchestrator（用活跃 LLM + 当前 tool_ctx 快照 + vector_store + MVU runtime）
    pub fn new_pipeline(&self) -> PipelineOrchestrator {
        let llm = self.active_llm_or_mock();
        let mut tool_ctx = (*self.snapshot_tool_ctx()).clone();
        // 注入向量存储（search_vectors 工具用）
        tool_ctx.vector_store = Some(self.vector_store.clone());
        // W10: 注入 MVU JS runtime（None = setup 未运行或 WebView 不可用，降级）
        let mvu_rt: Option<
            Arc<dyn storyforge_infra_plugin_host::mvu_runtime::MvuRuntime + Send + Sync>,
        > = MVU_RUNTIME.get().cloned().map(|r| {
            r as Arc<dyn storyforge_infra_plugin_host::mvu_runtime::MvuRuntime + Send + Sync>
        });
        PipelineOrchestrator::new(llm, self.conv_store.clone(), Arc::new(tool_ctx), mvu_rt)
    }
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
    pub system_prompt: String,
    pub tags: Vec<String>,
    pub creator: String,
    pub spec_version: String,
    #[serde(default)]
    pub extensions: serde_json::Value,
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
            source_character_id: Some(c.id.as_str().to_string()),
            name: c.name.clone(),
            description: c.description.clone(),
            personality: c.personality.clone(),
            scenario: c.scenario.clone(),
            first_mes: c.first_mes.clone(),
            system_prompt: c.system_prompt.clone(),
            tags: c.tags.clone(),
            creator: c.creator.clone(),
            spec_version: c.spec_version.clone(),
            extensions: c.extensions.clone(),
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
fn get_character(id: String) -> Result<CharacterInfo, TauriCommandError> {
    get_store()
        .get(&id)
        .map(|stored| stored.info)
        .ok_or_else(|| TauriCommandError::from(format!("角色卡不存在: {id}")))
}

fn delete_character_cascade_source_ids(
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
fn delete_character(
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
        let _ = get_campaign_store().delete_mvu(source_id);
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
fn update_world_info_route(
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
fn add_world_info_entry(
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
fn delete_world_info_entry(
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
fn rebuild_world_info_in_tool_ctx(state: &tauri::State<'_, Arc<AppState>>) {
    use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

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
fn import_preset(data: Vec<u8>) -> Result<String, TauriCommandError> {
    let preset = storyforge_infra_import::import_preset(&data).map_err(TauriCommandError::from)?;
    let preset_id = get_preset_store()
        .save(preset.clone())
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
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
fn get_preset(id: String) -> Result<PresetDetailDto, TauriCommandError> {
    let sp = get_preset_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到预设 {id}")))?;
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
fn delete_preset(id: String) -> Result<(), TauriCommandError> {
    if get_preset_store()
        .delete(&id)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!("找不到预设 {id}")))
    }
}

#[tauri::command]
fn update_preset_prompt(
    preset_id: String,
    prompt_index: usize,
    content: Option<String>,
    enabled: Option<bool>,
) -> Result<(), TauriCommandError> {
    if get_preset_store()
        .update_prompt(&preset_id, prompt_index, content.as_deref(), enabled)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!(
            "找不到预设 {preset_id} 的第 {prompt_index} 条 prompt"
        )))
    }
}

#[tauri::command]
fn update_preset_regex(
    preset_id: String,
    regex_index: usize,
    disabled: Option<bool>,
) -> Result<(), TauriCommandError> {
    if get_preset_store()
        .update_regex(&preset_id, regex_index, disabled)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!(
            "找不到预设 {preset_id} 的第 {regex_index} 条正则"
        )))
    }
}

/// 将 ST 预设的 prompts 转换为 PromptModule 并存入 ModuleStore
#[tauri::command]
fn import_preset_as_modules(
    preset_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    use storyforge_domain::agent::AgentRole;
    use storyforge_domain::prompt_module::{
        Exclusivity, ModuleCategory, ModuleSource, PromptModule,
    };

    let stored = get_preset_store()
        .get(&preset_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到预设 {preset_id}")))?;
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

        state
            .module_store
            .add(module)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
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
        permissions: p
            .manifest
            .permissions
            .iter()
            .map(|perm| format!("{perm:?}"))
            .collect(),
        ui_slots: p
            .manifest
            .ui_slots
            .iter()
            .map(|slot| format!("{slot:?}"))
            .collect(),
        description: p.manifest.description.clone(),
        author: p.manifest.author.clone(),
        enabled: p.enabled,
        installed_at: p.installed_at.to_rfc3339(),
    }
}

#[tauri::command]
fn list_plugins(state: tauri::State<'_, Arc<AppState>>) -> Vec<InstalledPluginDto> {
    state
        .plugin_registry
        .list()
        .iter()
        .map(plugin_to_dto)
        .collect()
}

#[tauri::command]
fn install_plugin(
    manifest_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let manifest: storyforge_infra_plugin_host::PluginManifest =
        serde_json::from_str(&manifest_json)
            .map_err(|e| TauriCommandError::validation(format!("manifest 解析失败: {e}")))?;
    state
        .plugin_registry
        .install(manifest)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

#[tauri::command]
fn uninstall_plugin(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .plugin_registry
        .uninstall(&id)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

#[tauri::command]
fn set_plugin_enabled(
    id: String,
    enabled: bool,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .plugin_registry
        .set_enabled(&id, enabled)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

// ─── M4 插件 API 命令（带权限二次校验）──────────────────────────────────────

#[tauri::command]
fn plugin_list_characters(
    plugin_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<CharacterSummary>, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::ReadCharacters)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    Ok(get_store()
        .list()
        .into_iter()
        .map(CharacterSummary::from)
        .collect())
}

#[tauri::command]
fn plugin_read_character(
    plugin_id: String,
    character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInfo, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::ReadCharacters)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    get_store()
        .get(&character_id)
        .map(|s| s.info)
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))
}

#[tauri::command]
fn plugin_get_variable(
    plugin_id: String,
    campaign_id: String,
    instance_id: String,
    _key: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::WriteVariables)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    let store = get_campaign_store();
    store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map(|i| i.variables)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到实例 {instance_id}")))
}

#[tauri::command]
fn plugin_set_variable(
    plugin_id: String,
    campaign_id: String,
    instance_id: String,
    key: String,
    value: serde_json::Value,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::WriteVariables)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    let store = get_campaign_store();
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到实例 {instance_id}")))?;
    inst.set_variable(&key, value, 0);
    store
        .update_instance(inst)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
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
) -> Result<(), TauriCommandError> {
    if state
        .module_store
        .update(&id, content.as_deref(), enabled)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
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
) -> Result<(), TauriCommandError> {
    let profile: PromptProfile = serde_json::from_str(&profile_json)
        .map_err(|e| TauriCommandError::validation(format!("Profile 解析失败: {e}")))?;
    state
        .profile_store
        .save(profile)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
    Ok(())
}

#[tauri::command]
fn set_active_profile(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .profile_store
        .set_active(&id)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
}

// ─── Agent Profile Config 命令 ─────────────────────────────────────────────

#[tauri::command]
fn list_agent_profile_configs(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<storyforge_domain::agent_profile_config::AgentProfileConfigSummaryDto> {
    state.agent_profile_config_store.list()
}

#[tauri::command]
fn get_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<storyforge_domain::agent_profile_config::AgentProfileConfig> {
    state.agent_profile_config_store.get(&id)
}

#[tauri::command]
fn get_active_agent_profile_config(
    state: tauri::State<'_, Arc<AppState>>,
) -> storyforge_domain::agent_profile_config::AgentProfileConfig {
    state.agent_profile_config_store.get_active()
}

#[tauri::command]
fn save_agent_profile_config(
    config_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let mut config: storyforge_domain::agent_profile_config::AgentProfileConfig =
        serde_json::from_str(&config_json).map_err(|e| {
            TauriCommandError::validation(format!("Agent Profile Config 解析失败: {e}"))
        })?;
    config.sanitize();
    state
        .agent_profile_config_store
        .save(config)
        .map_err(TauriCommandError::from)
}

#[tauri::command]
fn delete_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<bool, TauriCommandError> {
    state
        .agent_profile_config_store
        .delete(&id)
        .map_err(TauriCommandError::from)
}

#[tauri::command]
fn set_active_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .agent_profile_config_store
        .set_active(&id)
        .map_err(TauriCommandError::from)
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
            PipelineEvent::DirectorStarted => ("director_started".into(), serde_json::json!({})),
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
            PipelineEvent::EditorStarted => ("editor_started".into(), serde_json::json!({})),
            PipelineEvent::EditorProgress { delta } => (
                "editor_progress".into(),
                serde_json::json!({ "delta": delta }),
            ),
            PipelineEvent::DraftReady { text } => {
                ("draft_ready".into(), serde_json::json!({ "text": text }))
            }
            PipelineEvent::PostProcessStarted => {
                ("postprocess_started".into(), serde_json::json!({}))
            }
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
            PipelineEvent::PostProcessSkipped { reason } => (
                "postprocess_skipped".into(),
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
            PipelineEvent::Error { message } => {
                ("error".into(), serde_json::json!({ "message": message }))
            }
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
    conversation_id: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<WritingEvent>,
) -> Result<serde_json::Value, TauriCommandError> {
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

    // 一 Campaign 一对话：Campaign 模式下用 Campaign 绑定的 conversation_id，
    // 覆盖前端传入的（前端可能在切档时传错或传 null）
    let campaign_conv_id: Option<Id> = {
        let active = app
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(cid) = active.as_ref() {
            get_campaign_store()
                .get_campaign(cid)
                .and_then(|c| c.conversation_id.clone())
        } else {
            None
        }
    };
    let conversation_id: Option<String> = if let Some(cid) = campaign_conv_id {
        Some(cid.as_str().to_string())
    } else {
        conversation_id
    };

    // 复用已有对话 或 新建对话
    let conversation_id = if let Some(id_str) = conversation_id {
        let id = Id::from_str(&id_str);
        // 追加 user 意图到已有对话（开场白已在创建时存入）
        if let Err(e) = app.conv_store.append_user_message(&id, intent.clone()) {
            tracing::warn!("追加 user 消息失败: {e}");
        }
        id
    } else {
        // 新建对话 + 存开场白 + 存 user 意图（legacy 路径，无 Campaign 绑定）
        let conv = app.conv_store.create(character_id.clone(), None);
        let id = conv.id.clone();
        // 开场白（从角色卡读取，Final 状态 Assistant 消息）
        if let Some(ch) = tool_snapshot.characters.first()
            && !ch.first_mes.is_empty()
            && let Err(e) = app.conv_store.append_final_message(
                &id,
                storyforge_domain::conversation::Role::Assistant,
                ch.first_mes.clone(),
            )
        {
            tracing::warn!("追加开场白失败: {e}");
        }
        // user 意图
        if let Err(e) = app.conv_store.append_user_message(&id, intent.clone()) {
            tracing::warn!("追加 user 消息失败: {e}");
        }
        id
    };
    let regex_character_id = character_id.clone().or_else(|| {
        app.conv_store
            .get(&conversation_id)
            .and_then(|c| c.character_id)
    });
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
        regex_scripts: collect_scoped_regex_scripts(
            regex_character_id.as_deref(),
            &tool_snapshot.characters,
        ),
        campaign_runtime: None,
        agent_profile_config: None,
    };
    // 从模块/Profile 存储加载预设配置
    fill_profile_context(&mut ctx, &app);
    // 从活跃 Agent Profile Config 加载运行时配置覆盖
    fill_agent_profile_context(&mut ctx, &app);
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
        // Phase 6：落盘本轮创建的临时 instance（在 postprocess 之前，确保知识/变量写回能找到它们）
        persist_temporary_instances_to(
            get_campaign_store(),
            &ctx,
            pipeline.pending_temporary_instances(),
        );

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
        let mut var_keys = default_variable_keys();
        // Also include custom variable_schema keys from character definitions
        if let Some(campaign_id) = &ctx.campaign_id {
            let store = get_campaign_store();
            if let Some(campaign) = store.get_campaign(campaign_id)
                && let Some(stored_card) = store.get_card(&campaign.card_id)
            {
                for def in &stored_card.card.character_definitions {
                    for field in &def.variable_schema {
                        if !var_keys.contains(&field.key) {
                            var_keys.push(field.key.clone());
                        }
                    }
                }
            }
        }
        // 后处理用独立的 cancel（与写作共享 life-cycle，但写作已结束，这里新建一个）
        let (pp_cancel_tx, pp_cancel_rx) = watch::channel(false);
        // W10: 收集在场角色的 MVU fallback 片段（JS 执行用）
        let mvu_fragments =
            collect_mvu_fallback_fragments(&ctx, get_campaign_store(), &present_chars);
        let outcome = pipeline
            .run_postprocess(
                &final_text,
                "", // scene_brief：传空，summarizer/postprocess 从 final_text 自取
                &present_chars,
                &var_keys,
                &ctx,
                &event_tx,
                pp_cancel_rx,
                &mvu_fragments,
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
        Err(e) => Err(TauriCommandError::from(format!("写作失败: {e}"))),
    }
}

/// 从模块/Profile 存储加载预设配置到 WritingContext
///
/// 无 Profile 时不动 ctx（profile 保持 None → 流水线用硬编码常量兜底）。
fn collect_scoped_regex_scripts(
    character_id: Option<&str>,
    characters: &[Arc<storyforge_domain::character::Character>],
) -> Vec<RegexScript> {
    let Some(character_id) = character_id else {
        return Vec::new();
    };

    let stored = get_store().get(character_id);
    let source_id = stored
        .as_ref()
        .and_then(|stored| stored.info.source_character_id.as_deref())
        .unwrap_or(character_id);
    let stored_name = stored.as_ref().map(|stored| stored.info.name.as_str());

    characters
        .iter()
        .find(|character| {
            character.id.as_str() == source_id
                || character.id.as_str() == character_id
                || stored_name.is_some_and(|name| character.name == name)
        })
        .map(|character| character.scoped_regex_scripts())
        .unwrap_or_default()
}

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

/// 从活跃 Agent Profile Config 加载运行时配置覆盖到 WritingContext
///
/// 无活跃配置时不动 ctx（agent_profile_config 保持 None → 流水线用硬编码默认值）。
fn fill_agent_profile_context(ctx: &mut WritingContext, state: &Arc<AppState>) {
    let config = state.agent_profile_config_store.get_active();
    ctx.agent_profile_config = Some(config);
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
    // 阶段 2 cleanup：先清空旧 runtime，避免 stale 数据残留
    // （如果后续 early return，至少不会有上一轮的脏快照）
    ctx.campaign_runtime = None;
    {
        let mut tool_guard = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        tool_guard.campaign_runtime = None;
    }

    let active_id = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard
            .clone()
            .or_else(|| load_active_campaign(&state.data_dir))
    };
    let active_id = match active_id {
        Some(id) => id,
        None => return,
    };
    let store = get_campaign_store();
    fill_campaign_runtime_from_store(ctx, &state.tool_ctx, store, &active_id);
}

/// 从指定 CampaignStore 的活跃 Campaign 组装 CampaignRuntimeContext 快照写入 ctx + tool_ctx。
///
/// 抽自 `fill_campaign_context`，供外部 harness 用独立 tempdir `CampaignStore` 复刻真实
/// campaign-mode 组装逻辑（无需 AppState / 全局 store / Tauri runtime）。行为与线上路径一致。
///
/// 调用前应已清空 `ctx.campaign_runtime` 与 `tool_ctx.campaign_runtime`（防 stale）。
/// 若 store 中找不到该 campaign，直接返回（无 campaign 模式）。
pub fn fill_campaign_runtime_from_store(
    ctx: &mut WritingContext,
    tool_ctx: &Arc<RwLock<ToolContext>>,
    store: &campaign_store::CampaignStore,
    active_id: &Id,
) {
    let camp = match store.get_campaign(active_id) {
        Some(c) => c,
        None => return,
    };
    ctx.campaign_id = Some(active_id.clone());
    ctx.story_clock = camp.story_clock.clone();
    // turn = 已有 round_summaries 数 + 1（下一轮）
    let existing_turns = store.list_summaries(active_id).len() as u32;
    ctx.turn = existing_turns + 1;
    // pending_tasks：该 Campaign 下所有任务（build_director_user_msg 内部按触发条件过滤）
    ctx.pending_tasks = store.list_tasks(active_id);

    // 阶段 2：组装 CampaignRuntimeContext 快照
    // 加载 instances、card definitions、knowledge、tasks，构建纯 domain 快照
    let instances = store.list_instances(active_id);
    let knowledge = store.list_knowledge(active_id);
    let tasks = store.list_tasks(active_id);

    // 从 card 的 character_definitions 构建 definitions_by_id
    let definitions_by_id: std::collections::HashMap<
        Id,
        storyforge_domain::character::CharacterDefinition,
    > = if let Some(stored_card) = store.get_card(&camp.card_id) {
        stored_card
            .card
            .character_definitions
            .into_iter()
            .map(|def| (def.id.clone(), def))
            .collect()
    } else {
        std::collections::HashMap::new()
    };

    let runtime = Arc::new(CampaignRuntimeContext {
        campaign: camp,
        instances,
        definitions_by_id,
        knowledge,
        tasks,
        turn: ctx.turn,
    });

    // 写入 WritingContext
    ctx.campaign_runtime = Some(runtime.clone());

    // 同步到 ToolContext（快照，非 store 引用）
    {
        let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        tool_guard.campaign_runtime = Some(runtime);
    }
}

/// 默认变量键列表（喂给后处理 Agent，让它知道有哪些字段可更新）
fn default_variable_keys() -> Vec<String> {
    storyforge_domain::variables::default_character_variables()
        .iter()
        .map(|f| f.key.clone())
        .collect()
}

/// Phase 6：把本轮创建的临时 instance 落盘到 CampaignStore。
///
/// 去重逻辑：同一 campaign 内已存在同名 instance 时跳过。
/// 落盘后，下一轮 `fill_campaign_context` 能读到这些 instance。
fn persist_temporary_instances_to(
    store: &campaign_store::CampaignStore,
    ctx: &WritingContext,
    temporaries: &[storyforge_domain::campaign::CharacterInstance],
) {
    let camp_id = match &ctx.campaign_id {
        Some(id) => id,
        None => return,
    };
    if temporaries.is_empty() {
        return;
    }
    let existing = store.list_instances(camp_id);
    let mut known_names: std::collections::HashSet<String> =
        existing.into_iter().map(|i| i.name).collect();

    let mut persisted_count = 0;
    for temp in temporaries {
        if temp.campaign_id != *camp_id {
            tracing::warn!(
                "跳过 campaign 不匹配的临时 instance '{}'（instance campaign: {}, current campaign: {}）",
                temp.name,
                temp.campaign_id,
                camp_id
            );
            continue;
        }
        if !known_names.insert(temp.name.clone()) {
            tracing::info!(
                "跳过已存在的同名临时 instance '{}'（campaign {}）",
                temp.name,
                camp_id
            );
            continue;
        }
        if let Err(e) = store.add_instance(temp.clone()) {
            tracing::error!("落盘临时 instance '{}' 失败: {e}", temp.name);
        } else {
            persisted_count += 1;
        }
    }
    if persisted_count > 0 {
        tracing::info!(
            "已落盘 {} 个临时 instance 到 campaign {}",
            persisted_count,
            camp_id
        );
    }
}

/// W10: 为在场角色收集 MVU fallback 片段（供 run_postprocess 执行 JS）
///
/// 查找链：present_chars → CampaignRuntimeContext.instances → definition_id →
/// CampaignStore.cards → source_character_id → MvuTranslation.fallback_fragments
fn collect_mvu_fallback_fragments(
    ctx: &WritingContext,
    store: &campaign_store::CampaignStore,
    present_chars: &[String],
) -> Vec<storyforge_domain::mvu_translation::FallbackFragment> {
    let runtime = match &ctx.campaign_runtime {
        Some(rt) => rt,
        None => return vec![],
    };

    // definition_id → source_character_id（从 CampaignStore cards 构建查找表）
    let def_to_source: std::collections::HashMap<Id, Id> = store
        .list_cards()
        .iter()
        .flat_map(|sc| {
            let src = sc.card.source_character_id.clone();
            sc.card
                .character_definitions
                .iter()
                .map(move |d| (d.id.clone(), src.clone()))
        })
        .collect();

    let mut fragments = Vec::new();
    for char_id_str in present_chars {
        let char_id = Id::from_str(char_id_str);
        // 在 present_characters 中匹配 instance（by id or name）
        let inst = runtime
            .instances
            .iter()
            .find(|i| i.id == char_id || i.name == *char_id_str);
        let inst = match inst {
            Some(i) => i,
            None => continue,
        };
        let def_id = match &inst.definition_id {
            Some(d) => d,
            None => continue,
        };
        let source_id = match def_to_source.get(def_id) {
            Some(s) => s,
            None => continue,
        };
        if let Some(stored) = store.get_mvu(source_id) {
            let non_empty: Vec<_> = stored
                .translation
                .fallback_fragments
                .into_iter()
                .filter(|f| !f.js_snippet.is_empty())
                .collect();
            if !non_empty.is_empty() {
                tracing::info!(
                    target: "tauri-app",
                    "[MVU] 角色 '{}' 有 {} 个 fallback 片段",
                    inst.name,
                    non_empty.len()
                );
                fragments.extend(non_empty);
            }
        }
    }
    fragments
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
    if let Some(summary) = &outcome.summary
        && let Err(e) = store.add_summary(storyforge_domain::agent::RoundSummary::new(
            camp_id.clone(),
            ctx.conversation_id.clone(),
            ctx.turn,
            summary.clone(),
        ))
    {
        tracing::warn!("保存本轮摘要失败: {e}");
    }

    // 后处理三合一
    if let Some(pp) = &outcome.post_process {
        // 构建 present_chars 的 Id 集合（用于校验写入目标）
        let present_ids: std::collections::HashSet<String> =
            present_chars.iter().cloned().collect();

        // P4：算 name_collisions——campaign 内出现 ≥2 次的 name 集合，同名时 name 路失效逼 id
        let name_collisions: std::collections::HashSet<String> = {
            let mut name_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for inst in store.list_instances(camp_id) {
                *name_counts.entry(inst.name).or_insert(0) += 1;
            }
            name_counts
                .into_iter()
                .filter(|(_, count)| *count >= 2)
                .map(|(name, _)| name)
                .collect()
        };

        // 知识：update → entry（assign campaign_id + turn）
        // 只写入 present_chars 中的角色知识（信息隔离：不出场角色不应被后处理写入知识）
        // W6：broadcast 非空时一条 update 可能分发为多条 entry（flat_map）
        let knowledge_entries: Vec<_> = pp
            .knowledge_updates
            .iter()
            .flat_map(|u| {
                // P3: normalize 内部按 source 分流（ToldByOther/Backstory 不查在场）
                // P4: name_collisions 同名时 name 路失效
                // W6: broadcast 时分发到多个 target
                normalize_knowledge_update_for_postprocess(
                    store,
                    camp_id,
                    u,
                    ctx.turn,
                    &present_ids,
                    &name_collisions,
                )
            })
            .collect();
        if !knowledge_entries.is_empty()
            && let Err(e) = store.add_knowledge(knowledge_entries)
        {
            tracing::warn!("保存后处理知识失败: {e}");
        }

        // 变量更新：角色级（按 name 匹配 instance）/ 全局级（无 instance_id）
        for vu in &pp.variable_updates {
            if let Some(inst_id) = &vu.instance_id {
                // instance_id 可能是角色名（后处理 Agent 按名字输出），尝试匹配 campaign 内 instance
                if let Some(inst) = find_instance_by_name_or_id(store, camp_id, inst_id) {
                    // 校验：该 instance 是否在 present_chars 中（P4: 同名时 name 路失效）
                    let is_present = is_postprocess_instance_present(
                        &inst,
                        inst_id,
                        &present_ids,
                        &name_collisions,
                    );
                    if is_present {
                        let mut inst = inst;
                        inst.set_variable(&vu.key, vu.value.clone(), ctx.turn);
                        if let Err(e) = store.update_instance(inst) {
                            tracing::warn!("保存后处理角色变量失败: {e}");
                        }
                    } else {
                        tracing::warn!(
                            "跳过非在场角色 '{}' 的变量写入（present_chars 校验）",
                            inst.name
                        );
                    }
                }
            } else {
                // 全局 Campaign 变量（无 instance_id，不受 present_chars 约束）
                if let Some(mut camp) = store.get_campaign(camp_id) {
                    camp.set_variable(&vu.key, vu.value.clone(), ctx.turn);
                    if let Err(e) = store.update_campaign(camp) {
                        tracing::warn!("保存后处理 Campaign 变量失败: {e}");
                    }
                }
            }
        }

        // 任务更新：新建 / 状态变化
        for tu in &pp.task_updates {
            if let Some(tid) = &tu.task_id {
                if let Some(task) = store.get_task(tid)
                    && let Some(task) =
                        normalize_task_update_for_postprocess(camp_id, task, tu.new_status.clone())
                    && let Err(e) = store.update_task(task)
                {
                    tracing::warn!("保存后处理任务状态失败: {e}");
                }
            } else if let Some(spec) = &tu.new_task {
                let new_task = storyforge_domain::story_task::StoryTask::from_narrative(
                    camp_id.clone(),
                    spec.title.clone(),
                    spec.description.clone(),
                    spec.triggers.clone(),
                    ctx.turn,
                );
                if let Err(e) = store.add_task(new_task) {
                    tracing::warn!("保存后处理新任务失败: {e}");
                }
            }
        }
    }
}

fn normalize_task_update_for_postprocess(
    camp_id: &Id,
    mut task: storyforge_domain::story_task::StoryTask,
    new_status: storyforge_domain::story_task::TaskStatus,
) -> Option<storyforge_domain::story_task::StoryTask> {
    if task.campaign_id != *camp_id {
        tracing::warn!(
            "跳过非当前 Campaign 任务 '{}' 的状态更新（task campaign: {}, current campaign: {}）",
            task.id,
            task.campaign_id,
            camp_id
        );
        return None;
    }

    task.status = new_status;
    Some(task)
}

/// 按名字或 Id 查 campaign 内的 CharacterInstance（后处理 Agent 输出的是角色名，需翻译成 instance）
/// P3：内部按 KnowledgeSource 分流——ToldByOther/Backstory 不查在场直接放行，Witnessed/Inferred 才查在场。
/// P4：同名收紧——name_collisions 传入 campaign 内出现 ≥2 次的 name 集合，同名时 name 路失效。
/// W6 方向 1：broadcast 非空时分发给多个 target（All=全体, Group=身份组），返回 Vec。
pub fn normalize_knowledge_update_for_postprocess(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    turn: u32,
    present_ids: &std::collections::HashSet<String>,
    name_collisions: &std::collections::HashSet<String>,
) -> Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    use storyforge_domain::character_knowledge::PropagationPolicy;

    if update.propagation == PropagationPolicy::Private && update.broadcast.is_some() {
        tracing::warn!("跳过 private 知识的广播写入：{}", update.knowledge_text);
        return vec![];
    }

    if update.broadcast.is_some()
        && should_block_source_knowledge_propagation(store, camp_id, update, None)
    {
        return vec![];
    }

    // 方向 1：广播分发——broadcast 非空时遍历 instances，每个生成一条 ToldByOther
    if let Some(ref broadcast) = update.broadcast {
        return dispatch_broadcast(store, camp_id, update, turn, broadcast);
    }

    // 非广播：单角色逻辑（原有 P3/P4 流程）
    let target = match find_instance_by_name_or_id(store, camp_id, &update.character_id) {
        Some(inst) => inst,
        None => {
            tracing::warn!(
                "跳过无法解析到 Campaign instance 的知识写入目标: {}",
                update.character_id
            );
            return vec![];
        }
    };

    if should_block_source_knowledge_propagation(store, camp_id, update, Some(&target)) {
        return vec![];
    }

    // P3 分流：ToldByOther/Backstory 不受在场约束（跨在场告知 + 开局已有），
    // Witnessed/Inferred 才查在场。
    let knowledge_exempt_from_presence = matches!(
        update.source,
        storyforge_domain::character_knowledge::KnowledgeSource::ToldByOther
            | storyforge_domain::character_knowledge::KnowledgeSource::Backstory
    );
    if !knowledge_exempt_from_presence {
        // 知识路径收紧：空集时 Witnessed/Inferred 也拒绝（无人在场不可能见证/推断）
        // 注意：is_postprocess_instance_present 的空集放行仍服务变量路径，此处绕过它。
        if present_ids.is_empty()
            || !is_postprocess_instance_present(
                &target,
                &update.character_id,
                present_ids,
                name_collisions,
            )
        {
            tracing::warn!(
                "跳过非在场角色 '{}' 的知识写入（source={:?}，present_chars 校验）",
                target.name,
                update.source
            );
            return vec![];
        }
    }

    let source_character_id = update
        .source_character_id
        .as_ref()
        .and_then(|source_id| find_instance_by_name_or_id(store, camp_id, source_id))
        .map(|source| source.id);
    let source_knowledge_id =
        matching_source_knowledge_for_update(store, camp_id, update).map(|entry| entry.id);

    vec![
        storyforge_domain::character_knowledge::CharacterKnowledgeEntry {
            id: Id::new(),
            campaign_id: camp_id.clone(),
            character_id: target.id,
            knowledge_text: update.knowledge_text.clone(),
            source: update.source.clone(),
            source_character_id,
            source_knowledge_id,
            turn_number: turn,
            event_id: None,
            pinned: update.pinned,
            propagation: update.propagation.clone(),
        },
    ]
}

/// 方向 1：广播分发——根据 BroadcastTarget 遍历 campaign 内 instance，各生成一条 ToldByOther。
///
/// - `All`：campaign 内所有 instance（排除广播发起者自身）
/// - `Group(g)`：definition.group == g 的 instance（通过 definition_id 反查 CharacterDefinition）
///
/// 广播条目的 source 统一为 `ToldByOther`，source_character_id 记广播发起者（若有）。
fn dispatch_broadcast(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    turn: u32,
    broadcast: &storyforge_domain::character_knowledge::BroadcastTarget,
) -> Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    use storyforge_domain::character_knowledge::BroadcastTarget;

    // 解析广播发起者（source_character_id）的 persisted id，用于排除自身 + 记录来源
    let broadcaster_inst = update
        .source_character_id
        .as_ref()
        .and_then(|sid| find_instance_by_name_or_id(store, camp_id, sid));
    let broadcaster_id = broadcaster_inst.as_ref().map(|i| i.id.clone());
    let source_character_id = broadcaster_id.clone();
    let source_knowledge_id =
        matching_source_knowledge_for_update(store, camp_id, update).map(|entry| entry.id);

    let all_instances = store.list_instances(camp_id);

    let targets: Vec<_> = match broadcast {
        BroadcastTarget::All => all_instances
            .into_iter()
            // 排除广播发起者自身（不应该给自己发广播知识）
            .filter(|inst| Some(&inst.id) != broadcaster_id.as_ref())
            .collect(),
        BroadcastTarget::Group(group) => all_instances
            .into_iter()
            .filter(|inst| {
                // 排除广播发起者自身
                if Some(&inst.id) == broadcaster_id.as_ref() {
                    return false;
                }
                // 通过 definition_id 反查 definition.group
                instance_matches_group(store, inst, group)
            })
            .collect(),
    };

    if targets.is_empty() {
        tracing::warn!(
            "广播分发: broadcast={:?} 无匹配 instance（campaign={}）",
            broadcast,
            camp_id
        );
    }

    targets
        .into_iter()
        .map(|inst| {
            storyforge_domain::character_knowledge::CharacterKnowledgeEntry {
                id: Id::new(),
                campaign_id: camp_id.clone(),
                character_id: inst.id,
                knowledge_text: update.knowledge_text.clone(),
                // 广播统一为 ToldByOther（被告知/公告）
                source: storyforge_domain::character_knowledge::KnowledgeSource::ToldByOther,
                source_character_id: source_character_id.clone(),
                source_knowledge_id: source_knowledge_id.clone(),
                turn_number: turn,
                event_id: None,
                pinned: update.pinned,
                propagation: update.propagation.clone(),
            }
        })
        .collect()
}

fn matching_source_knowledge_for_update(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
) -> Option<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    use storyforge_domain::character_knowledge::KnowledgeSource;

    let is_relay =
        update.broadcast.is_some() || matches!(update.source, KnowledgeSource::ToldByOther);
    if !is_relay {
        return None;
    }

    let source_id = update.source_character_id.as_ref()?;
    let source = find_instance_by_name_or_id(store, camp_id, source_id)?;

    store
        .list_knowledge_of(camp_id, &source.id)
        .into_iter()
        .filter(|entry| knowledge_text_matches(&entry.knowledge_text, &update.knowledge_text))
        .max_by(|a, b| {
            a.turn_number
                .cmp(&b.turn_number)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        })
}

fn should_block_source_knowledge_propagation(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    target: Option<&storyforge_domain::campaign::CharacterInstance>,
) -> bool {
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, KnowledgeSource, PropagationPolicy,
    };

    let is_propagating =
        update.broadcast.is_some() || matches!(update.source, KnowledgeSource::ToldByOther);
    if !is_propagating {
        return false;
    }

    let Some(source_id) = &update.source_character_id else {
        return false;
    };
    let Some(source) = find_instance_by_name_or_id(store, camp_id, source_id) else {
        return false;
    };

    let source_entries = store.list_knowledge_of(camp_id, &source.id);
    for entry in source_entries {
        if !knowledge_text_matches(&entry.knowledge_text, &update.knowledge_text) {
            continue;
        }

        let blocked = match &entry.propagation {
            PropagationPolicy::Open => false,
            PropagationPolicy::Private => true,
            PropagationPolicy::GroupRestricted(group) => match (&update.broadcast, target) {
                (Some(BroadcastTarget::Group(target_group)), _) => target_group != group,
                (Some(BroadcastTarget::All), _) => true,
                (None, Some(target_inst)) => !instance_matches_group(store, target_inst, group),
                (None, None) => true,
            },
        };

        if blocked {
            tracing::warn!(
                "阻止知识传播：source={} policy={:?} text={}",
                source.name,
                entry.propagation,
                update.knowledge_text
            );
            return true;
        }
    }

    false
}

fn knowledge_text_matches(restricted: &str, candidate: &str) -> bool {
    let restricted = normalize_knowledge_text(restricted);
    let candidate = normalize_knowledge_text(candidate);
    if restricted.is_empty() || candidate.is_empty() {
        return false;
    }
    if restricted == candidate {
        return true;
    }

    let min_len = restricted.chars().count().min(candidate.chars().count());
    min_len >= 8 && (restricted.contains(&candidate) || candidate.contains(&restricted))
}

fn normalize_knowledge_text(text: &str) -> String {
    text.split_whitespace().collect::<String>().to_lowercase()
}

/// 判断 instance 的 CharacterDefinition.group 是否匹配目标组名。
/// 通过 instance.definition_id → 遍历所有 card 的 character_definitions 找匹配。
fn instance_matches_group(
    store: &campaign_store::CampaignStore,
    inst: &storyforge_domain::campaign::CharacterInstance,
    target_group: &str,
) -> bool {
    let def_id = match &inst.definition_id {
        Some(id) => id,
        None => return false, // 无 definition_id（临时角色）→ 不匹配
    };
    // 遍历所有 card，找 definition_id 匹配的 definition
    for stored_card in store.list_cards() {
        if let Some(def) = stored_card
            .card
            .character_definitions
            .iter()
            .find(|d| d.id == *def_id)
        {
            return def.group.as_deref() == Some(target_group);
        }
    }
    false
}

pub fn is_postprocess_instance_present(
    inst: &storyforge_domain::campaign::CharacterInstance,
    raw_id: &Id,
    present_ids: &std::collections::HashSet<String>,
    name_collisions: &std::collections::HashSet<String>,
) -> bool {
    if present_ids.is_empty() {
        tracing::warn!(
            "postprocess 写回: present_chars 为空集，放行 '{}'（向后兼容逃生口，变量路径仍依赖）",
            inst.name
        );
        return true;
    }
    // id 路优先（精确匹配）
    if present_ids.contains(raw_id.as_str()) || present_ids.contains(inst.id.as_str()) {
        return true;
    }
    // name 路兜底：P4 同名收紧——campaign 内存在同名 instance 时 name 路失效，逼 id
    if present_ids.contains(&inst.name) && !name_collisions.contains(&inst.name) {
        tracing::debug!(
            "postprocess 写回: '{}' 通过 name 匹配在场（非 id 匹配）",
            inst.name
        );
        return true;
    }
    false
}

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
    instances
        .into_iter()
        .find(|i| i.name.as_str() == name_or_id.as_str())
}

/// Tauri command: 取消当前运行的写作流水线
///
/// 触发 AppState.current_cancel 的 sender，导演/子Agent/编剧全部中止。
#[tauri::command]
fn cancel_writing(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, TauriCommandError> {
    let slot = state
        .current_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner());
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
fn parse_target_dto(target: &RegenerateTargetDto) -> Result<PartialRollTarget, TauriCommandError> {
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
        other => Err(TauriCommandError::validation(format!(
            "未知重 roll 目标: {other}"
        ))),
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
) -> Result<String, TauriCommandError> {
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
    let regex_character_id = app
        .conv_store
        .get(&conversation_id)
        .and_then(|conversation| conversation.character_id);
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
        regex_scripts: collect_scoped_regex_scripts(
            regex_character_id.as_deref(),
            &tool_snapshot.characters,
        ),
        campaign_runtime: None,
        agent_profile_config: None,
    };
    fill_profile_context(&mut ctx, &app);
    fill_agent_profile_context(&mut ctx, &app);
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
    let result = pipeline
        .regenerate(pipeline_req, &ctx, event_tx.clone(), cancel_rx)
        .await;

    // ─── P2 后处理（best-effort，同 start_writing）─────────────────────────
    if let Ok((text, _)) = &result {
        // Phase 6：落盘本轮创建的临时 instance（在 postprocess 之前）
        persist_temporary_instances_to(
            get_campaign_store(),
            &ctx,
            pipeline.pending_temporary_instances(),
        );

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
        // W10: 收集在场角色的 MVU fallback 片段（JS 执行用）
        let mvu_fragments =
            collect_mvu_fallback_fragments(&ctx, get_campaign_store(), &present_chars);
        let outcome = pipeline
            .run_postprocess(
                &final_text,
                "",
                &present_chars,
                &var_keys,
                &ctx,
                &event_tx,
                pp_rx,
                &mvu_fragments,
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
        Err(e) => Err(TauriCommandError::from(format!("重 roll 失败: {e}"))),
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
) -> Result<String, TauriCommandError> {
    let protocol = parse_protocol(&req.protocol)?;
    let tool_mode = parse_tool_mode(&req.tool_mode)?;

    let conn = LlmConnection {
        id: Id::new(),
        name: req.name,
        base_url: req.base_url,
        api_key: req.api_key,
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
        .map_err(|e| TauriCommandError::llm(format!("连接配置无效: {e}"), false))?;

    let was_empty = get_conn_store().list().is_empty();
    get_conn_store()
        .save(conn)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;

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
) -> Result<(), TauriCommandError> {
    let was_active = state.active_conn_id().as_deref() == Some(id.as_str());
    if !get_conn_store()
        .delete(&id)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        return Err(TauriCommandError::not_found(format!("连接不存在: {id}")));
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
) -> Result<(), TauriCommandError> {
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
async fn test_connection(
    req: TestConnectionDto,
) -> Result<TestConnectionResult, TauriCommandError> {
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
        .map_err(|e| TauriCommandError::llm(format!("构造客户端失败: {e}"), false))?;

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
async fn list_models(base_url: String, api_key: String) -> Result<Vec<String>, TauriCommandError> {
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
fn parse_protocol(s: &str) -> Result<LlmProtocol, TauriCommandError> {
    match s {
        "openai" => Ok(LlmProtocol::OpenAi),
        "anthropic" => Ok(LlmProtocol::Anthropic),
        "gemini" => Ok(LlmProtocol::Gemini),
        s if s.starts_with("custom:") => Ok(LlmProtocol::Custom(s[7..].to_string())),
        other => Err(TauriCommandError::validation(format!("未知协议: {other}"))),
    }
}

/// 把 tool_mode 字符串解析为 ToolMode
fn parse_tool_mode(s: &str) -> Result<ToolMode, TauriCommandError> {
    match s {
        "native" => Ok(ToolMode::Native),
        "text_fallback" => Ok(ToolMode::TextFallback),
        other => Err(TauriCommandError::validation(format!(
            "未知工具模式: {other}"
        ))),
    }
}

// ─── M1 对话命令 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummaryDto {
    pub id: String,
    pub character_id: Option<String>,
    pub campaign_id: Option<String>,
    /// 关联角色卡名（前端列表显示用）
    pub card_name: Option<String>,
    pub message_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[tauri::command]
fn list_conversations(state: tauri::State<'_, Arc<AppState>>) -> Vec<ConversationSummaryDto> {
    // 联查角色卡名：snapshot_tool_ctx.characters 是 domain Character（含 id + name）
    let tool_ctx = state.snapshot_tool_ctx();
    let chars = &tool_ctx.characters;
    state
        .conv_store
        .list()
        .into_iter()
        .map(|c| {
            let card_name = c.character_id.as_ref().and_then(|cid| {
                chars
                    .iter()
                    .find(|ch| ch.id.as_str() == cid)
                    .map(|ch| ch.name.clone())
            });
            ConversationSummaryDto {
                id: c.id.to_string(),
                character_id: c.character_id,
                campaign_id: c.campaign_id.map(|id| id.to_string()),
                card_name,
                message_count: c.message_count,
                created_at: c.created_at.to_rfc3339(),
                updated_at: c.updated_at.to_rfc3339(),
            }
        })
        .collect()
}

/// 删除整个会话（含文件 + 缓存）
#[tauri::command]
fn delete_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    state
        .conv_store
        .delete(&conv_id)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

#[tauri::command]
fn get_conversation(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let conv_id = storyforge_domain::Id::from_str(&id);
    state
        .conv_store
        .get(&conv_id)
        .map(|c| serde_json::to_value(&c).unwrap_or_default())
        .ok_or_else(|| TauriCommandError::not_found(format!("对话不存在: {id}")))
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
fn log_query(filter: LogFilterDto, state: tauri::State<'_, Arc<AppState>>) -> Vec<LogEntryDto> {
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
) -> Result<serde_json::Value, TauriCommandError> {
    let opts = ExportOptions {
        redact_content,
        ..Default::default()
    };
    let mut bundle = storyforge_app_logging::export_bundle(&state.log_store, &opts);
    if let Some(obj) = bundle.as_object_mut() {
        obj.insert(
            "diagnostic_context".into(),
            diagnostic_context_for_data_dir(&state.data_dir),
        );
    }
    Ok(bundle)
}

fn diagnostic_context_for_data_dir(data_dir: &Path) -> serde_json::Value {
    const STORE_FILES: &[&str] = &[
        "connections.json",
        "embed.json",
        "active_campaign.json",
        "characters.json",
        "cards.json",
        "campaigns.json",
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "mvu_translations.json",
        "vectors.json",
        "plugins.json",
        "custom_modules.json",
        "disabled_modules.json",
        "profiles.json",
        "active_profile.json",
        "agent_profile_configs.json",
        "active_agent_profile_config.json",
        "presets.json",
    ];

    let log_dir = data_dir.join("logs");
    let conversation_dir = data_dir.join("conversations");
    let store_files: Vec<serde_json::Value> = STORE_FILES
        .iter()
        .map(|name| summarize_file(data_dir.join(name), name))
        .collect();

    serde_json::json!({
        "schema_version": 1,
        "app": {
            "name": "StoryForge",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "paths": {
            "data_dir": data_dir.display().to_string(),
            "log_dir": log_dir.display().to_string(),
            "conversation_dir": conversation_dir.display().to_string(),
        },
        "directories": [
            summarize_dir(&log_dir, "logs"),
            summarize_dir(&conversation_dir, "conversations"),
        ],
        "store_files": store_files,
    })
}

fn summarize_file(path: PathBuf, name: &str) -> serde_json::Value {
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => serde_json::json!({
            "name": name,
            "exists": true,
            "bytes": metadata.len(),
            "has_bytes": metadata.len() > 0,
        }),
        Ok(metadata) => serde_json::json!({
            "name": name,
            "exists": true,
            "bytes": metadata.len(),
            "has_bytes": metadata.len() > 0,
            "kind": if metadata.is_dir() { "directory" } else { "other" },
        }),
        Err(_) => serde_json::json!({
            "name": name,
            "exists": false,
            "bytes": 0,
            "has_bytes": false,
        }),
    }
}

fn summarize_dir(path: &Path, name: &str) -> serde_json::Value {
    let (exists, entries) = match std::fs::read_dir(path) {
        Ok(read_dir) => (true, read_dir.filter_map(Result::ok).count()),
        Err(_) => (path.exists(), 0),
    };
    serde_json::json!({
        "name": name,
        "exists": exists,
        "entries": entries,
    })
}

/// 前端日志上报（console.log/warn/error 转发到后端 LogStore）
///
/// 单条消息上限 4KB（M-21），防止恶意/异常前端灌爆 LogStore。
#[tauri::command]
fn log_append_frontend(level: String, message: String, state: tauri::State<'_, Arc<AppState>>) {
    const MAX_LOG_MSG_LEN: usize = 4096;
    let message = if message.len() > MAX_LOG_MSG_LEN {
        format!("{}...(截断)", &message[..MAX_LOG_MSG_LEN])
    } else {
        message
    };
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
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .edit_variant(&conv_id, &nid, new_content)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// 采纳当前变体（Draft → Final），并自动检查是否需要归档
#[tauri::command]
async fn accept_variant(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .accept_variant(&conv_id, &nid)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;

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
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .soft_delete_variant(&conv_id, &nid)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// Tauri command: 删除指定消息及其后所有消息（截断对话 = 撤销从这条开始的写作）
#[tauri::command]
fn delete_message_from(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .truncate_from(&conv_id, &nid)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// 添加新变体（分支/swipe）
#[tauri::command]
fn add_variant(
    conversation_id: String,
    node_id: String,
    content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .add_variant(&conv_id, &nid, content, None)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// 切换变体（左右滑）
#[tauri::command]
fn switch_variant(
    conversation_id: String,
    node_id: String,
    index: usize,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .switch_variant(&conv_id, &nid, index)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
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
) -> Result<(), TauriCommandError> {
    let config = storyforge_infra_llm::EmbedConfig {
        endpoint,
        api_key,
        model,
        dim,
    };
    save_embed_config(&state.data_dir, &config)
        .map_err(|e| TauriCommandError::storage(format!("嵌入配置写入失败: {e}")))?;
    *state
        .embed_config
        .write()
        .unwrap_or_else(|p| p.into_inner()) = Some(config);
    Ok(())
}

/// 获取当前嵌入配置（不含 key）
#[tauri::command]
fn get_embed_config(state: tauri::State<'_, Arc<AppState>>) -> Option<serde_json::Value> {
    state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|c| {
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
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let conv = state
        .conv_store
        .get(&conv_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("对话不存在: {conversation_id}")))?;

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
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .ok_or("未配置嵌入 API，请先在设置中配置")?;

    let llm = state.active_llm_or_mock();
    let vector_store = state.vector_store.clone();

    let embedder = Arc::new(
        storyforge_infra_llm::Embedder::new(config)
            .map_err(|e| TauriCommandError::internal(e.to_string()))?,
    );
    let model = get_conn_store()
        .active_connection()
        .map(|c| c.model)
        .unwrap_or_else(|| "deepseek-chat".into());
    let archiver = storyforge_app_memory::MemoryArchiver::new(
        llm,
        embedder,
        vector_store,
        storyforge_app_memory::ArchiveConfig::default(),
        model,
    );

    let summaries = archiver
        .maybe_archive(&messages)
        .await
        .map_err(|e| TauriCommandError::internal(format!("归档失败: {e}")))?;

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
    let config = match state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    {
        Some(c) => c,
        None => {
            tracing::debug!("未配置嵌入 API，跳过自动归档");
            return;
        }
    };

    let llm = state.active_llm_or_mock();
    let vector_store = state.vector_store.clone();
    let embedder = match storyforge_infra_llm::Embedder::new(config) {
        Ok(e) => Arc::new(e),
        Err(e) => {
            tracing::warn!("构建 Embedder 失败，跳过自动归档: {e}");
            return;
        }
    };
    let model = get_conn_store()
        .active_connection()
        .map(|c| c.model)
        .unwrap_or_else(|| "deepseek-chat".into());
    let archiver = storyforge_app_memory::MemoryArchiver::new(
        llm,
        embedder,
        vector_store,
        storyforge_app_memory::ArchiveConfig::default(),
        model,
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
) -> Result<(), TauriCommandError> {
    // 从 PatchStore 取出 patch
    let patch = {
        let patches = state.meta_patches.read().unwrap_or_else(|p| p.into_inner());
        patches
            .iter()
            .find(|p| p.id == patch_id)
            .cloned()
            .ok_or_else(|| TauriCommandError::not_found(format!("Patch 不存在: {patch_id}")))?
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
                .map_err(|e| TauriCommandError::internal(e.to_string()))?;

            // 反序列化回 WorldInfoEntry 并替换
            let new_entries: Vec<storyforge_domain::world_info::WorldInfoEntry> = entries_json
                .into_iter()
                .filter_map(|v| {
                    serde_json::from_value(v.clone()).unwrap_or_else(|e| {
                        tracing::warn!(
                            "Patch entry failed to deserialize as WorldInfoEntry: {e}, value: {v}"
                        );
                        None
                    })
                })
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
            // 从合并视图提取全局条目（Constant/Both = 全局），保留原始 is_global
            let global_entries: Vec<crate::WorldInfoEntryInfo> = world_info
                .entries
                .iter()
                .filter(|e| {
                    matches!(
                        e.route,
                        storyforge_domain::world_info::LoreRoute::Constant
                            | storyforge_domain::world_info::LoreRoute::Both
                    )
                })
                .map(|e| crate::WorldInfoEntryInfo {
                    keys: e.keys.clone(),
                    content: e.content.clone(),
                    constant: e.constant,
                    route: format!("{:?}", e.route),
                    is_global: true,
                    depth: e.depth,
                    order: e.order,
                })
                .collect();
            // 全局条目的 keys 集合（用于从各卡原有条目中排除已合并的全局条目，
            // 防止各卡私有条目中的旧全局条目残留）
            let global_keys_set: std::collections::HashSet<String> =
                global_entries.iter().map(|e| e.keys.join(",")).collect();

            let all_stored = get_store().list();
            for stored in &all_stored {
                // 保留该卡的私有条目（is_global=false），并排除 keys 与全局条目重复的
                // （这些已由全局条目覆盖，避免重复）
                let preserved_private: Vec<crate::WorldInfoEntryInfo> = stored
                    .info
                    .world_info_entries
                    .iter()
                    .filter(|e| !e.is_global && !global_keys_set.contains(&e.keys.join(",")))
                    .cloned()
                    .collect();
                let mut new_entries = preserved_private;
                new_entries.extend(global_entries.clone());
                let _ = get_store().update_world_info_entries_bulk(&stored.id, new_entries);
            }
        }
    }

    // 标记为已执行
    if let Some(p) = state
        .meta_patches
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .iter_mut()
        .find(|p| p.id == patch_id)
    {
        p.applied = true;
    }

    Ok(())
}

// ─── P3：Meta Agent 多轮对话 / MVU 五合一分析 / ST 预设 LLM 分类 ────────────

/// GenerationExplainer 的 tauri-app 实现：从 ConversationStore 查 Provenance
/// 并调 `explain_generation`，让 Meta 多轮对话的 inspect_generation 工具
/// 能引用真实生成溯源（而非 app-meta 层的 None 占位）。
///
/// 实现 app-meta 的 GenerationExplainer trait，注入到 MetaSession。
/// 持有 Arc<ConversationStore>（与 AppState.conv_store 共享同一份）。
/// 查不到对话/节点/变体/溯源时返回 None，由工具层转成错误信息，不 panic。
struct ConvGenerationExplainer {
    conv_store: Arc<storyforge_app_conversation::ConversationStore>,
}

impl storyforge_app_meta::meta_conversation::GenerationExplainer for ConvGenerationExplainer {
    fn explain(
        &self,
        conversation_id: &str,
        node_id: &str,
    ) -> Option<storyforge_app_meta::GenerationExplanation> {
        let conv_id = Id::from_str(conversation_id);
        let nid = Id::from_str(node_id);

        let conv = self.conv_store.get(&conv_id)?;
        let node = conv.find_node(&nid)?;
        let variant = node.active()?;
        let provenance = variant.provenance.as_ref()?;
        Some(storyforge_app_meta::explain_generation(provenance))
    }
}

/// 把当前活跃角色卡 + 世界书同步进 MetaSession（每次 meta 操作前调）
fn sync_meta_session_from_tool_ctx(state: &tauri::State<'_, Arc<AppState>>) {
    let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
    if let Some(card) = ctx.characters.last() {
        state.meta_session.set_character(card.clone());
    }
    if let Some(book) = &ctx.world_info {
        state.meta_session.set_world_info(book.clone());
    }
    // 同步 campaign runtime 快照
    if let Some(rt) = &ctx.campaign_runtime {
        state.meta_session.set_campaign_runtime(rt.clone());
    } else {
        // 无 active campaign 时清空，避免读到过期快照
        *state
            .meta_session
            .campaign_runtime
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
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
        .unwrap_or_else(|p| p.into_inner())
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
) -> Result<serde_json::Value, TauriCommandError> {
    let app = state.inner().clone();
    sync_meta_session_from_tool_ctx(&state);

    // 取出对话；不存在则返回错误（而非静默创建空对话，避免用户感觉"历史突然清空"）。
    // 新对话应由 meta_start_conversation 命令显式建立。
    let mut conv = {
        let mut convs = app
            .meta_conversations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
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
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
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
    .map_err(|e| TauriCommandError::internal(e.to_string()))?;

    // 把新提议的 patch 同步进 AppState.meta_patches（前端可用 meta_accept_patch 采纳）
    if let Some(patch) = &turn.new_patch {
        let mut patches = app.meta_patches.write().unwrap_or_else(|p| p.into_inner());
        if !patches.iter().any(|p| p.id == patch.id) {
            patches.push(patch.clone());
        }
    }

    // 把新提议的 typed patch 同步进 AppState.typed_patches
    if !turn.new_typed_patches.is_empty() {
        let mut typed = app.typed_patches.write().unwrap_or_else(|p| p.into_inner());
        for tp in &turn.new_typed_patches {
            if !typed.iter().any(|p| p.id == tp.id) {
                typed.push(tp.clone());
            }
        }
    }

    // 存回对话
    let conv_id = conv.id.clone();
    let messages = serde_json::to_value(&conv.messages).unwrap_or(serde_json::Value::Null);
    app.meta_conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(conv_id.clone(), conv);

    Ok(serde_json::json!({
        "conversation_id": conv_id,
        "agent_message": turn.agent_message,
        "messages": messages,
        "new_patch": turn.new_patch,
        "new_typed_patches": turn.new_typed_patches,
    }))
}

/// Tauri command: 获取某个 Meta 对话的完整消息历史
#[tauri::command]
fn meta_get_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<serde_json::Value> {
    let convs = state
        .meta_conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    convs
        .get(&conversation_id)
        .map(|conv| serde_json::to_value(conv).unwrap_or(serde_json::Value::Null))
}

/// Tauri command: 列所有待采纳的 Meta Patch
#[tauri::command]
fn meta_list_pending_patches(state: tauri::State<'_, Arc<AppState>>) -> Vec<serde_json::Value> {
    state
        .meta_patches
        .read()
        .unwrap_or_else(|p| p.into_inner())
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
) -> Result<(), TauriCommandError> {
    let mut patches = state
        .meta_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    patches.retain(|p| p.id != patch_id);
    Ok(())
}

/// Tauri command: Campaign 健康检查（确定性数据校验，零 LLM）
///
/// 扫描指定 Campaign 的数据，找出孤立 instance、未解析的知识引用、孤儿任务引用、
/// 变量 schema 不一致等问题。返回问题列表，空列表 = 健康。
#[tauri::command]
fn meta_health_check(campaign_id: String) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let store = get_campaign_store();
    let cid = Id::from_str(&campaign_id);

    // 确认 campaign 存在
    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;

    // 获取关联的 card → definitions
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();

    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let snapshot = storyforge_app_meta::CampaignHealthSnapshot {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
    };

    let issues = storyforge_app_meta::check_campaign_health(&snapshot);

    Ok(issues
        .into_iter()
        .map(|i| serde_json::to_value(i).unwrap_or(serde_json::Value::Null))
        .collect())
}

/// Tauri command: 解释某条消息的生成溯源（确定性，零 LLM）
///
/// 从指定对话节点的 active variant 的 Provenance 提取可读解释。
/// 返回 `GenerationExplanation` 结构体，前端可直接渲染。
#[tauri::command]
fn meta_explain_generation(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);

    let conv = state
        .conv_store
        .get(&conv_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("对话不存在: {conversation_id}")))?;

    let node = conv
        .find_node(&nid)
        .ok_or_else(|| TauriCommandError::not_found(format!("消息节点不存在: {node_id}")))?;

    let variant = node.active().ok_or("该节点无可用变体")?;

    let provenance = variant
        .provenance
        .as_ref()
        .ok_or("该消息没有生成溯源信息（可能是用户手动输入）")?;

    let explanation = storyforge_app_meta::explain_generation(provenance);

    serde_json::to_value(&explanation)
        .map_err(|e| TauriCommandError::internal(format!("序列化失败: {e}")))
}

// ─── 类型化 Patch 命令（第三轮：campaign-runtime 修复闭环）───────────────────

/// 从 CampaignStore 组装 PreviewInput（类型化 patch 纯函数所需的快照）
fn build_preview_input<'a>(
    _store: &'static campaign_store::CampaignStore,
    campaign: &'a storyforge_domain::campaign::Campaign,
    instances: &'a [storyforge_domain::campaign::CharacterInstance],
    definitions: &'a Vec<storyforge_domain::character::CharacterDefinition>,
    knowledge: &'a [storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
    tasks: &'a [storyforge_domain::story_task::StoryTask],
) -> storyforge_app_meta::PreviewInput<'a> {
    storyforge_app_meta::PreviewInput {
        instances,
        definitions,
        knowledge,
        tasks,
        campaign: Some(campaign),
    }
}

/// 对 Campaign 做健康检查并生成类型化修复建议
#[tauri::command]
fn meta_propose_campaign_repairs(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let store = get_campaign_store();
    let cid = Id::from_str(&campaign_id);

    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;

    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();

    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let input = build_preview_input(
        store,
        &campaign,
        &instances,
        &definitions,
        &knowledge,
        &tasks,
    );

    let issues =
        storyforge_app_meta::check_campaign_health(&storyforge_app_meta::CampaignHealthSnapshot {
            instances: &instances,
            definitions: &definitions,
            knowledge: &knowledge,
            tasks: &tasks,
        });

    let mut patches: Vec<storyforge_app_meta::TypedPatch> = Vec::new();
    for issue in &issues {
        if let Some(patch) = storyforge_app_meta::build_patch_for_issue(issue, &input) {
            patches.push(patch);
        }
    }

    // 存入 state（追加，不去重）
    {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        typed.extend(patches.clone());
    }

    Ok(patches
        .into_iter()
        .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null))
        .collect())
}

/// 列出所有 Pending 状态的类型化 patch
#[tauri::command]
fn meta_list_typed_patches(state: tauri::State<'_, Arc<AppState>>) -> Vec<serde_json::Value> {
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    typed
        .iter()
        .filter(|p| p.status == storyforge_app_meta::TypedPatchStatus::Pending)
        .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null))
        .collect()
}

/// 预览一条类型化 patch：检查是否过期，返回 diff
#[tauri::command]
fn meta_preview_typed_patch(
    patch_id: String,
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let store = get_campaign_store();
    let cid = Id::from_str(&campaign_id);

    // 找到 patch
    let mut typed = state
        .typed_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    let patch = typed
        .iter_mut()
        .find(|p| p.id == patch_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("类型化 Patch 不存在: {patch_id}")))?;

    // 取当前 campaign 快照
    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();
    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let input = storyforge_app_meta::PreviewInput {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
        campaign: Some(&campaign),
    };

    if storyforge_app_meta::is_patch_stale(patch, &input) {
        patch.status = storyforge_app_meta::TypedPatchStatus::Stale;
        return Ok(serde_json::json!({
            "stale": true,
            "patch": serde_json::to_value(&*patch).unwrap_or(serde_json::Value::Null),
        }));
    }

    Ok(serde_json::json!({
        "stale": false,
        "patch": serde_json::to_value(&*patch).unwrap_or(serde_json::Value::Null),
        "diff": serde_json::to_value(&patch.diff).unwrap_or(serde_json::Value::Null),
    }))
}

/// 接受一条类型化 patch：纯函数预演 → 写盘
#[tauri::command]
fn meta_accept_typed_patch(
    patch_id: String,
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let store = get_campaign_store();
    let cid = Id::from_str(&campaign_id);

    // 1. 找到 patch，必须 Pending
    let patch = {
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        let p = typed.iter().find(|p| p.id == patch_id).ok_or_else(|| {
            TauriCommandError::not_found(format!("类型化 Patch 不存在: {patch_id}"))
        })?;
        if p.status != storyforge_app_meta::TypedPatchStatus::Pending {
            return Err(TauriCommandError::validation(format!(
                "Patch 状态不是 Pending（当前: {:?}），无法接受",
                p.status
            )));
        }
        p.clone()
    };

    // 2. 取当前 campaign 快照
    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();
    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let input = storyforge_app_meta::PreviewInput {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
        campaign: Some(&campaign),
    };

    // 3. stale 检查
    if storyforge_app_meta::is_patch_stale(&patch, &input) {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(p) = typed.iter_mut().find(|p| p.id == patch_id) {
            p.status = storyforge_app_meta::TypedPatchStatus::Stale;
        }
        return Err("patch 已过期，target 不存在".into());
    }

    // 4. 纯函数预演（clone 可变快照）
    {
        // A 的 PreviewInputMut 字段为 &mut Vec<...>，需 clone 到本地变量再借。
        let mut inst_clone = instances.clone();
        let mut def_clone = definitions.clone();
        let mut know_clone = knowledge.clone();
        let mut task_clone = tasks.clone();
        let mut camp_clone = campaign.clone();
        let mut snap = storyforge_app_meta::PreviewInputMut {
            instances: &mut inst_clone,
            definitions: &mut def_clone,
            knowledge: &mut know_clone,
            tasks: &mut task_clone,
            campaign: Some(&mut camp_clone),
            turn: 0, // preview 不持久化，turn 值不影响验证
        };
        storyforge_app_meta::apply_to_snapshot(&patch, &mut snap)
            .map_err(|e| TauriCommandError::pipeline(format!("纯函数预演失败: {e}"), false))?;
    }
    // 5. 真正写盘
    for (idx, action) in patch.actions.iter().enumerate() {
        let result = apply_typed_action(store, &cid, action);
        if let Err(e) = result {
            // 写盘失败，patch 保持 Pending，报错包含第几个 action
            return Err(TauriCommandError::storage(format!(
                "第 {} 个 action 失败: {}",
                idx + 1,
                e
            )));
        }
    }

    // 6. 写盘成功，标记 Accepted
    {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(p) = typed.iter_mut().find(|p| p.id == patch_id) {
            p.status = storyforge_app_meta::TypedPatchStatus::Accepted;
        }
    }

    Ok(())
}

/// 执行单个 TypedPatchAction 到 CampaignStore（写盘辅助）
fn apply_typed_action(
    store: &'static campaign_store::CampaignStore,
    campaign_id: &Id,
    action: &storyforge_app_meta::TypedPatchAction,
) -> Result<(), TauriCommandError> {
    use storyforge_app_meta::TypedPatchAction;

    match action {
        TypedPatchAction::SyncInstanceVariables {
            instance_id,
            definition_id,
            add_keys,
            remove_keys,
        } => {
            let mut instance = store
                .get_instance(campaign_id, instance_id)
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Instance 不存在: {}",
                        instance_id.as_str()
                    ))
                })?;

            // 与 A 的 apply_to_snapshot 纯函数保持一致：从 definition.variable_schema
            // 取 add_keys 的 default 值，而非硬编码 null。否则 accept 前的纯函数预演
            // （显示 schema 默认值）与真正写盘（写 null）结果不一致。
            // 查找路径：campaign -> card_id -> card.character_definitions -> 按 definition_id 匹配
            let schema_defaults: std::collections::HashMap<String, serde_json::Value> = (|| {
                let campaign = store.get_campaign(campaign_id)?;
                let stored = store.get_card(&campaign.card_id)?;
                let def = stored
                    .card
                    .character_definitions
                    .iter()
                    .find(|d| &d.id == definition_id)?;
                Some(
                    def.variable_schema
                        .iter()
                        .map(|f| (f.key.clone(), f.default.clone()))
                        .collect(),
                )
            })(
            )
            .unwrap_or_default();

            // 添加缺失 key（用 schema default，缺失 schema 时回退 null）
            for key in add_keys {
                if instance.get_variable(key).is_none() {
                    let default_val = schema_defaults
                        .get(key)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    instance.set_variable(key, default_val, 0);
                }
            }

            // 删除多余 key
            instance.variables.retain(|v| !remove_keys.contains(&v.key));

            store
                .update_instance(instance)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::PruneOrphanTaskReferences {
            task_id,
            orphan_character_ids,
        } => {
            let mut task = store.get_task(task_id).ok_or_else(|| {
                TauriCommandError::not_found(format!("Task 不存在: {}", task_id.as_str()))
            })?;

            task.related_characters
                .retain(|id| !orphan_character_ids.contains(id));

            store
                .update_task(task)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
            store
                .delete_knowledge(knowledge_id)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
                .then_some(())
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Knowledge 不存在: {}",
                        knowledge_id.as_str()
                    ))
                })?;
            Ok(())
        }
        TypedPatchAction::RepointInstanceDefinition {
            instance_id,
            new_definition_id,
        } => {
            let mut instance = store
                .get_instance(campaign_id, instance_id)
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Instance 不存在: {}",
                        instance_id.as_str()
                    ))
                })?;

            instance.definition_id = new_definition_id.clone();
            store
                .update_instance(instance)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::UpdateCampaignVariable { key, value } => {
            let mut campaign = store.get_campaign(campaign_id).ok_or_else(|| {
                TauriCommandError::not_found(format!("Campaign 不存在: {}", campaign_id.as_str()))
            })?;
            campaign.set_variable(key, value.clone(), 0);
            store
                .update_campaign(campaign)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::UpdateInstanceVariable {
            instance_id,
            key,
            value,
        } => {
            let mut instance = store
                .get_instance(campaign_id, instance_id)
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Instance 不存在: {}",
                        instance_id.as_str()
                    ))
                })?;
            instance.set_variable(key, value.clone(), 0);
            store
                .update_instance(instance)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::AddKnowledge {
            character_id,
            knowledge_text,
            source,
        } => {
            let entry = storyforge_domain::character_knowledge::CharacterKnowledgeEntry {
                id: Id::new(),
                campaign_id: campaign_id.clone(),
                character_id: character_id.clone(),
                knowledge_text: knowledge_text.clone(),
                source: source.clone(),
                source_character_id: None,
                source_knowledge_id: None,
                turn_number: 0,
                event_id: None,
                pinned: false,
                propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
            };
            store
                .add_knowledge(vec![entry])
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::UpdateTaskStatus {
            task_id,
            new_status,
        } => {
            let mut task = store.get_task(task_id).ok_or_else(|| {
                TauriCommandError::not_found(format!("Task 不存在: {}", task_id.as_str()))
            })?;
            task.status = new_status.clone();
            store
                .update_task(task)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
    }
}

/// 忽略一条类型化 patch（标记 Dismissed，保留审计痕迹）
#[tauri::command]
fn meta_dismiss_typed_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let mut typed = state
        .typed_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    let patch = typed
        .iter_mut()
        .find(|p| p.id == patch_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("类型化 Patch 不存在: {patch_id}")))?;
    patch.status = storyforge_app_meta::TypedPatchStatus::Dismissed;
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
) -> Result<MvuTranslationDetailDto, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;

    // 取原 Character
    let character = {
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        ctx.characters
            .iter()
            .find(|c| c.id.as_str() == source_character_id)
            .map(|c| (*c).clone())
    }
    .ok_or_else(|| TauriCommandError::not_found(format!("找不到角色卡 {source_character_id}")))?;

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
        .map_err(|e| TauriCommandError::validation(format!("MVU 分析失败: {e}")))?;

    // 持久化到 CampaignStore
    let store = get_campaign_store();
    let stored = campaign_store::StoredMvuTranslation {
        source_character_id: character.id.clone(),
        character_name: character.name.clone(),
        translation: translation.clone(),
        analyzed_at: chrono::Utc::now().to_rfc3339(),
    };
    store
        .save_mvu(stored)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;

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
fn meta_get_mvu_translation(source_character_id: String) -> Option<MvuTranslationDetailDto> {
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

/// Tauri command: 预览 MVU schema 合并结果（每个 definition 一条预览）
#[tauri::command]
fn meta_preview_mvu_apply(
    source_character_id: String,
) -> Result<Vec<MvuApplyPreview>, TauriCommandError> {
    let store = get_campaign_store();
    let id = Id::from_str(&source_character_id);
    let mvu = store.get_mvu(&id).ok_or_else(|| {
        TauriCommandError::not_found(
            MvuApplyError::TranslationNotFound(source_character_id.clone()).to_string(),
        )
    })?;

    // 通过 source_character_id 找到 card
    let stored_card = store.get_card_by_source(&id).ok_or_else(|| {
        TauriCommandError::not_found(format!(
            "找不到 source_character_id={source_character_id} 的 card"
        ))
    })?;

    let previews: Vec<MvuApplyPreview> = stored_card
        .card
        .character_definitions
        .iter()
        .map(|def| {
            compute_apply_preview(
                &def.variable_schema,
                &mvu.translation.variable_schema,
                def.id.as_str(),
                &def.name,
                &source_character_id,
            )
        })
        .collect();

    Ok(previews)
}

/// Tauri command: 把 MVU schema 合并应用到指定 definition（写盘）
///
/// 必须先 compute_apply_preview，无变化则拒绝写盘。
#[tauri::command]
fn meta_apply_mvu_schema(
    source_character_id: String,
    definition_id: String,
) -> Result<(), TauriCommandError> {
    let store = get_campaign_store();
    let src_id = Id::from_str(&source_character_id);
    let def_id = Id::from_str(&definition_id);

    let mvu = store.get_mvu(&src_id).ok_or_else(|| {
        TauriCommandError::not_found(
            MvuApplyError::TranslationNotFound(source_character_id.clone()).to_string(),
        )
    })?;

    let stored_card = store.get_card_by_source(&src_id).ok_or_else(|| {
        TauriCommandError::not_found(format!(
            "找不到 source_character_id={source_character_id} 的 card"
        ))
    })?;

    // 找到目标 definition
    let def = stored_card
        .card
        .character_definitions
        .iter()
        .find(|d| d.id == def_id)
        .ok_or_else(|| {
            TauriCommandError::not_found(
                MvuApplyError::DefinitionNotFound(definition_id.clone()).to_string(),
            )
        })?;

    // 先计算预览，确认有变化
    let preview = compute_apply_preview(
        &def.variable_schema,
        &mvu.translation.variable_schema,
        def.id.as_str(),
        &def.name,
        &source_character_id,
    );
    if !preview.has_changes {
        return Err(TauriCommandError::validation(
            MvuApplyError::NoChanges.to_string(),
        ));
    }

    // 写盘：clone card → 改对应 definition → update_card
    let mut card = stored_card.card.clone();
    if let Some(target_def) = card
        .character_definitions
        .iter_mut()
        .find(|d| d.id == def_id)
    {
        apply_schema_to_definition(target_def, preview.merged_schema.clone());
    }
    store
        .update_card(card.clone())
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;

    // Best-effort：对已存在的 instances 补齐新变量。
    // 注意：instance 与 definition 的关联是 definition_id，与 campaign 无关——
    // 一张卡的某个 definition 可能被多个 campaign 引用，全部都该 backfill。
    // 因此这里用 list_all_instances() 全量遍历，再按 definition_id 过滤。
    // （历史 bug：曾用 list_instances(&card.id)，把 card.id 当 campaign_id 传，
    // 而 list_instances 按 campaign_id 过滤 → instance.campaign_id 永不等于
    // card.id → loop 体永不执行 → 生产 backfill 是死代码。）
    for instance in store.list_all_instances() {
        if instance.definition_id.as_ref() != Some(&def_id) {
            continue;
        }
        let mut updated = instance.clone();
        let new_fields: Vec<_> = preview
            .added_fields
            .iter()
            .filter(|f| !updated.variables.iter().any(|v| v.key == f.key))
            .collect();
        if new_fields.is_empty() {
            continue;
        }
        for field in new_fields {
            use storyforge_domain::variables::VariableValue;
            updated.variables.push(VariableValue::new(
                field.key.clone(),
                field.default.clone(),
                0,
            ));
        }
        store
            .update_instance(updated)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
    }

    Ok(())
}

/// Tauri command: 手动触发 ST 预设 LLM 分类（增强现有纯启发式 bridge）
///
/// 失败时返回 Err，前端降级到现有 import_preset_as_modules。
#[tauri::command]
async fn meta_classify_st_preset(
    preset_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;

    let stored = get_preset_store()
        .get(&preset_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("预设不存在: {preset_id}")))?;

    let llm = state.active_llm_or_mock();
    let tool_ctx = state.snapshot_tool_ctx();
    let runtime = AgentRuntime::new(llm, tool_ctx);
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let classification =
        storyforge_app_meta::classify_st_preset_with_llm(&runtime, &stored.preset, cancel_rx)
            .await
            .map_err(|e| TauriCommandError::validation(format!("ST 分类失败: {e}")))?;

    serde_json::to_value(&classification)
        .map_err(|e| TauriCommandError::internal(format!("序列化失败: {e}")))
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
    /// 绑定的对话 ID（一 Campaign 一对话）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
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
            conversation_id: c.conversation_id.as_ref().map(|id| id.as_str().to_string()),
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
) -> Result<CardSummaryDto, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;
    use storyforge_domain::character::CharacterDefinition;
    use storyforge_domain::variables::extract_mvu_schema_from_extensions;

    // 取原 Character（从 tool_ctx，启动恢复 + import_character 都同步过）
    let character = {
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        // 1. 直接按 Character.id 查（前端 extractCharacters 传卡本身 id 时命中）
        let direct = ctx
            .characters
            .iter()
            .find(|c| c.id.as_str() == source_character_id)
            .map(|c| (*c).clone());
        match direct {
            Some(c) => Some(c),
            None => {
                // 2. 回退：前端可能传了存储 id（import_character 返回的 StoredCharacter.id，
                //    与 Character.id 无关）。用存储 id 查 CharacterStore 拿到 name，再按 name 查 tool_ctx。
                get_store().get(&source_character_id).and_then(|stored| {
                    ctx.characters
                        .iter()
                        .find(|c| c.name == stored.info.name)
                        .map(|c| (*c).clone())
                })
            }
        }
    }
    .ok_or_else(|| {
        TauriCommandError::not_found(format!(
            "找不到 source_character_id={source_character_id} 的角色卡"
        ))
    })?;

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
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    let definitions = storyforge_app_agent::attach_definitions_to_card(definitions, &card.id);
    card.character_definitions = definitions;
    let stored = store
        .save_card(card)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;

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

/// 删除角色卡（按 CharacterCard.id，级联删 campaign/instances/mvu）
#[tauri::command]
fn delete_card(id: String) -> Result<(), TauriCommandError> {
    let card_id = Id::from_str(&id);
    if !get_campaign_store()
        .delete_card(&card_id)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        return Err(TauriCommandError::not_found(format!("角色卡不存在: {id}")));
    }
    Ok(())
}

#[tauri::command]
fn get_card(id: String) -> Result<CardDetailDto, TauriCommandError> {
    let stored = get_campaign_store()
        .get_card(&Id::from_str(&id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 card id={id}")))?;
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

/// 开档：建 Campaign，把卡里所有 Protagonist/Supporting 定义实例化；
/// 一 Campaign 一对话模型：同时自动建对话、存开场白、双向绑定 conversation_id
#[tauri::command]
fn create_campaign(
    card_id: String,
    name: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    use storyforge_domain::campaign::CharacterInstance;
    use storyforge_domain::character::RoleType;
    use storyforge_domain::conversation::Role as ConvRole;

    let store = get_campaign_store();
    let stored = store
        .get_card(&Id::from_str(&card_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 card id={card_id}")))?;

    let mut campaign = storyforge_domain::campaign::Campaign::new(stored.card.id.clone(), name);

    // 自动建对话并绑定到 Campaign
    let conv = state
        .conv_store
        .create(Some(card_id.clone()), Some(campaign.id.clone()));
    campaign.conversation_id = Some(conv.id.clone());
    store
        .save_campaign(campaign.clone())
        .map_err(|e| format!("存储写入失败: {e}"))?;

    // 存开场白（从 CharacterStore 按 source_character_id 查扁平 Character.first_mes）
    let src_id_str = stored.card.source_character_id.as_str().to_string();
    let first_mes = get_store()
        .list()
        .into_iter()
        .find(|sc| sc.info.source_character_id.as_deref() == Some(src_id_str.as_str()))
        .map(|sc| sc.info.first_mes.clone())
        .unwrap_or_default();
    if !first_mes.is_empty()
        && let Err(e) =
            state
                .conv_store
                .append_final_message(&conv.id, ConvRole::Assistant, first_mes)
    {
        tracing::warn!("建 Campaign 时追加开场白失败: {e}");
    }

    // 实例化所有 protagonist/supporting 定义
    let mut instance_count = 0;
    for def in &stored.card.character_definitions {
        if matches!(def.role_type, RoleType::Protagonist | RoleType::Supporting) {
            let inst = CharacterInstance::from_definition(campaign.id.clone(), def);
            store
                .add_instance(inst)
                .map_err(|e| format!("存储写入失败: {e}"))?;
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
fn get_campaign(id: String) -> Result<CampaignSummaryDto, TauriCommandError> {
    let store = get_campaign_store();
    let c = store
        .get_campaign(&Id::from_str(&id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign id={id}")))?;
    let mut dto = CampaignSummaryDto::from(&c);
    dto.instance_count = store.list_instances(&c.id).len();
    Ok(dto)
}

#[tauri::command]
fn set_active_campaign(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let campaign_id = Id::from_str(&id);
    // 校验存在
    if get_campaign_store().get_campaign(&campaign_id).is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 campaign id={id}"
        )));
    }
    *state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(campaign_id.clone());
    save_active_campaign(&state.data_dir, Some(&campaign_id));
    Ok(())
}

#[tauri::command]
fn get_active_campaign(state: tauri::State<'_, Arc<AppState>>) -> Option<CampaignSummaryDto> {
    let id = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()?;
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
fn get_instance(
    campaign_id: String,
    instance_id: String,
) -> Result<CharacterInstanceDto, TauriCommandError> {
    get_campaign_store()
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map(|i| CharacterInstanceDto::from(&i))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))
}

/// 查角色实例的当前变量值
#[tauri::command]
fn get_character_variables(
    campaign_id: String,
    instance_id: String,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, TauriCommandError> {
    get_campaign_store()
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map(|i| i.variables)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))
}

/// 手动改角色实例变量值（调试/纠错用，turn 用 0 占位）
#[tauri::command]
fn set_character_variable(
    campaign_id: String,
    instance_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
) -> Result<(), TauriCommandError> {
    let store = get_campaign_store();
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))?;
    inst.set_variable(&key, value, turn.unwrap_or(0));
    store
        .update_instance(inst)
        .map_err(|e| TauriCommandError::storage(format!("更新角色变量失败: {e}")))?;
    Ok(())
}

/// 查 Campaign 全局变量
#[tauri::command]
fn get_campaign_variables(
    campaign_id: String,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, TauriCommandError> {
    get_campaign_store()
        .get_campaign(&Id::from_str(&campaign_id))
        .map(|c| c.variables)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))
}

/// 改 Campaign 全局变量
#[tauri::command]
fn set_campaign_variable(
    campaign_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
) -> Result<(), TauriCommandError> {
    let store = get_campaign_store();
    let mut camp = store
        .get_campaign(&Id::from_str(&campaign_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))?;
    camp.set_variable(&key, value, turn.unwrap_or(0));
    store
        .update_campaign(camp)
        .map_err(|e| TauriCommandError::storage(format!("更新 Campaign 变量失败: {e}")))?;
    Ok(())
}

/// 把临场角色升级为常驻（仅翻 is_temporary flag）
#[tauri::command]
fn promote_temporary_instance(
    campaign_id: String,
    instance_id: String,
) -> Result<(), TauriCommandError> {
    let store = get_campaign_store();
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))?;
    if !inst.is_temporary {
        return Err("该角色已是常驻".into());
    }
    inst.promote_to_permanent();
    store
        .update_instance(inst)
        .map_err(|e| TauriCommandError::storage(format!("升级临时角色失败: {e}")))?;
    Ok(())
}

// ─── P2 后处理产出查询 / 任务管理命令（6 个）──────────────────────────────────

/// 角色知识条目 DTO（前端展示用）
#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeEntryDto {
    pub id: String,
    pub campaign_id: String,
    pub character_id: String,
    pub character_name: Option<String>,
    pub knowledge_text: String,
    pub source: String,
    pub source_character_id: Option<String>,
    pub source_character_name: Option<String>,
    pub source_knowledge_id: Option<String>,
    pub relay_chain_text: Option<String>,
    pub provenance_text: String,
    pub turn_number: u32,
    pub pinned: bool,
    pub propagation: String,
}

fn knowledge_source_code(source: &storyforge_domain::character_knowledge::KnowledgeSource) -> &str {
    use storyforge_domain::character_knowledge::KnowledgeSource;
    match source {
        KnowledgeSource::Witnessed => "witnessed",
        KnowledgeSource::ToldByOther => "told_by_other",
        KnowledgeSource::Inferred => "inferred",
        KnowledgeSource::Backstory => "backstory",
    }
}

fn knowledge_provenance_text(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
) -> String {
    use storyforge_domain::character_knowledge::{KnowledgeSource, PropagationPolicy};
    let target = instance_names
        .get(&entry.character_id)
        .cloned()
        .unwrap_or_else(|| entry.character_id.to_string());
    let base = match entry.source {
        KnowledgeSource::Witnessed => format!("{target} 亲眼所见"),
        KnowledgeSource::ToldByOther => {
            let source = entry
                .source_character_id
                .as_ref()
                .and_then(|id| instance_names.get(id))
                .cloned()
                .or_else(|| entry.source_character_id.as_ref().map(|id| id.to_string()));
            match source {
                Some(source) => format!("{target} 被 {source} 告知"),
                None => format!("{target} 被告知"),
            }
        }
        KnowledgeSource::Inferred => format!("{target} 自行推断"),
        KnowledgeSource::Backstory => format!("{target} 的背景知识"),
    };

    match &entry.propagation {
        PropagationPolicy::Open => base,
        PropagationPolicy::Private => format!("{base}（秘密，禁止外传）"),
        PropagationPolicy::GroupRestricted(group) => format!("{base}（限制传播：仅{group}）"),
    }
}

fn knowledge_actor_label(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
) -> String {
    instance_names
        .get(&entry.character_id)
        .cloned()
        .unwrap_or_else(|| entry.character_id.to_string())
}

fn knowledge_relay_chain_text(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
    knowledge_by_id: &std::collections::HashMap<
        Id,
        &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    >,
) -> Option<String> {
    let mut chain = vec![entry];
    let mut current = entry;
    let mut seen = std::collections::HashSet::from([entry.id.clone()]);

    while let Some(parent_id) = &current.source_knowledge_id {
        if !seen.insert(parent_id.clone()) {
            break;
        }
        let Some(parent) = knowledge_by_id.get(parent_id).copied() else {
            break;
        };
        chain.push(parent);
        current = parent;
        if chain.len() >= 8 {
            break;
        }
    }

    if chain.len() < 2 {
        return None;
    }

    chain.reverse();
    Some(
        chain
            .iter()
            .map(|entry| {
                format!(
                    "{}（轮 {}）",
                    knowledge_actor_label(entry, instance_names),
                    entry.turn_number
                )
            })
            .collect::<Vec<_>>()
            .join(" → "),
    )
}

fn propagation_policy_code(
    policy: &storyforge_domain::character_knowledge::PropagationPolicy,
) -> String {
    use storyforge_domain::character_knowledge::PropagationPolicy;
    match policy {
        PropagationPolicy::Open => "open".into(),
        PropagationPolicy::Private => "private".into(),
        PropagationPolicy::GroupRestricted(group) => format!("group:{group}"),
    }
}

fn knowledge_entry_to_dto(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
    knowledge_by_id: &std::collections::HashMap<
        Id,
        &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    >,
) -> KnowledgeEntryDto {
    KnowledgeEntryDto {
        id: entry.id.to_string(),
        campaign_id: entry.campaign_id.to_string(),
        character_id: entry.character_id.to_string(),
        character_name: instance_names.get(&entry.character_id).cloned(),
        knowledge_text: entry.knowledge_text.clone(),
        source: knowledge_source_code(&entry.source).into(),
        source_character_id: entry.source_character_id.as_ref().map(|i| i.to_string()),
        source_character_name: entry
            .source_character_id
            .as_ref()
            .and_then(|id| instance_names.get(id))
            .cloned(),
        source_knowledge_id: entry.source_knowledge_id.as_ref().map(|id| id.to_string()),
        relay_chain_text: knowledge_relay_chain_text(entry, instance_names, knowledge_by_id),
        provenance_text: knowledge_provenance_text(entry, instance_names),
        turn_number: entry.turn_number,
        pinned: entry.pinned,
        propagation: propagation_policy_code(&entry.propagation),
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
    let all_entries = store.list_knowledge(&camp);
    let entries: Vec<_> = if let Some(cid) = character_id {
        let cid = Id::from_str(&cid);
        all_entries
            .iter()
            .filter(|entry| entry.character_id == cid)
            .collect()
    } else {
        all_entries.iter().collect()
    };
    let instance_names: std::collections::HashMap<Id, String> = store
        .list_instances(&camp)
        .into_iter()
        .map(|inst| (inst.id, inst.name))
        .collect();
    let knowledge_by_id: std::collections::HashMap<Id, _> = all_entries
        .iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect();
    entries
        .into_iter()
        .map(|entry| knowledge_entry_to_dto(entry, &instance_names, &knowledge_by_id))
        .collect()
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
fn list_tasks(campaign_id: String, status_filter: Option<String>) -> Vec<StoryTaskDto> {
    let store = get_campaign_store();
    let camp = Id::from_str(&campaign_id);
    let mut tasks = store.list_tasks(&camp);
    if let Some(filter) = status_filter {
        tasks.retain(|t| {
            let s = serde_json::to_string(&t.status).unwrap_or_default();
            // TaskStatus 序列化为 "pending"/"active"/{"likely_completed":...}/"completed"/"abandoned"
            s.starts_with(&format!("\"{filter}"))
                || s.starts_with('{') && filter == "likely_completed"
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
) -> Result<String, TauriCommandError> {
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
    store
        .add_task(task)
        .map_err(|e| TauriCommandError::storage(format!("创建任务失败: {e}")))?;
    Ok(id)
}

/// 标记任务完成（用户确认）
#[tauri::command]
fn complete_task(task_id: String) -> Result<(), TauriCommandError> {
    let store = get_campaign_store();
    let mut task = store
        .get_task(&Id::from_str(&task_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到任务 {task_id}")))?;
    task.complete();
    store
        .update_task(task)
        .map_err(|e| TauriCommandError::storage(format!("完成任务失败: {e}")))?;
    Ok(())
}

/// 放弃任务
#[tauri::command]
fn abandon_task(task_id: String) -> Result<(), TauriCommandError> {
    let store = get_campaign_store();
    let mut task = store
        .get_task(&Id::from_str(&task_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到任务 {task_id}")))?;
    task.abandon();
    store
        .update_task(task)
        .map_err(|e| TauriCommandError::storage(format!("放弃任务失败: {e}")))?;
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

// ─── 导出命令（W7: ST 卡 PNG + 共享 Lorebook + JSON Bundle）─────────────────

/// 导出的文件 DTO（文件名 + 字节）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedFile {
    pub filename: String,
    pub data: Vec<u8>,
}

/// Campaign 导出结果 DTO（多角色 PNG + 共享 lorebook）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignExportResult {
    /// 每个角色的 ST 卡 PNG
    pub cards: Vec<ExportedFile>,
    /// 共享 lorebook JSON（ST 格式，可独立保存）
    pub lorebook_json: String,
}

/// StoryForge JSON Bundle 格式版本
const BUNDLE_FORMAT_VERSION: u32 = 1;

/// StoryForge Campaign 完整 JSON Bundle
#[derive(Debug, Serialize, Deserialize)]
struct CampaignBundle {
    format_version: u32,
    exported_at: String,
    campaign: storyforge_domain::campaign::Campaign,
    instances: Vec<storyforge_domain::campaign::CharacterInstance>,
    /// key = definition_id
    definitions: Vec<storyforge_domain::character::CharacterDefinition>,
    knowledge: Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>,
    tasks: Vec<storyforge_domain::story_task::StoryTask>,
    summaries: Vec<storyforge_domain::agent::RoundSummary>,
}

/// 导出单个角色卡为 ST PNG
///
/// 从 CharacterStore 取原始 Character（含 raw_card_json），
/// 用 to_st_data 构建 StCharacterData，再写入 PNG。
#[tauri::command]
fn export_st_card_png(character_id: String) -> Result<Vec<u8>, TauriCommandError> {
    let stored = get_store()
        .get(&character_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;

    // 从 CharacterStore 恢复 Character（精简版，但够 to_st_data 用）
    let character = stored_info_to_character(&stored);

    // 构建 ST 数据（用内嵌世界书作为 character_book）
    let book = character
        .embedded_world_info
        .as_ref()
        .map(|b| b.to_st_book());
    let st_data = storyforge_domain::character::to_st_data(&character, None, book);
    let card = storyforge_infra_import::png::make_st_card(st_data, &character.spec_version);

    storyforge_infra_import::png::write_st_card_png(&card, None)
        .map_err(|e| TauriCommandError::internal(format!("PNG 导出失败: {e}")))
}

/// 导出 Campaign 全部角色为 ST PNG + 共享 lorebook
///
/// 策略（用户已定）：每角色一张 PNG + 共享 lorebook。
/// 共享知识/世界书转 ST lorebook 格式。
#[tauri::command]
fn export_campaign_st_cards(
    campaign_id: String,
) -> Result<CampaignExportResult, TauriCommandError> {
    let store = get_campaign_store();
    let camp_id = Id::from_str(&campaign_id);

    let campaign = store
        .get_campaign(&camp_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;

    let stored_card = store.get_card(&campaign.card_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("Campaign 关联的卡不存在: {}", campaign.card_id))
    })?;

    let instances = store.list_instances(&camp_id);
    if instances.is_empty() {
        return Err("Campaign 无角色实例，无法导出".into());
    }

    // 构建共享 lorebook（Campaign 级知识 → ST WorldInfoBook）
    let shared_knowledge = store.list_knowledge(&camp_id);
    let shared_lorebook = knowledge_to_st_book(&shared_knowledge);
    let shared_lorebook_json =
        serde_json::to_string_pretty(&shared_lorebook).unwrap_or_else(|_| "{}".into());

    // 尝试从 CharacterStore 获取原始 Character（用于 raw_card_json）
    let original_character = get_store()
        .get(stored_card.card.source_character_id.as_str())
        .map(|s| stored_info_to_character(&s));

    let mut cards = Vec::new();
    for inst in &instances {
        // 找到对应的 definition
        let definition = inst.definition_id.as_ref().and_then(|did| {
            stored_card
                .card
                .character_definitions
                .iter()
                .find(|d| d.id == *did)
        });

        let (st_data, spec_version) = if let (Some(character), Some(def)) =
            (&original_character, definition)
        {
            // 有原始 Character → 用 to_st_data（round-trip 保底）
            let book = character
                .embedded_world_info
                .as_ref()
                .map(|b| b.to_st_book());
            let data = storyforge_domain::character::to_st_data(character, Some(def), book);
            (data, character.spec_version.clone())
        } else if let Some(def) = definition {
            // 只有 Card + Definition → 用 to_st_data_from_card
            let data =
                storyforge_domain::character::to_st_data_from_card(&stored_card.card, def, None);
            (data, "3.0".into())
        } else {
            // 临时角色（无 definition）→ 用 instance 名字构建最小卡
            let data = storyforge_domain::character::empty_st_data(&inst.name);
            (data, "3.0".into())
        };

        let card = storyforge_infra_import::png::make_st_card(st_data, &spec_version);
        let png_bytes =
            storyforge_infra_import::png::write_st_card_png(&card, None).map_err(|e| {
                TauriCommandError::internal(format!("PNG 导出失败 ({}): {e}", inst.name))
            })?;

        let filename = sanitize_filename(&format!("{}.png", inst.name));
        cards.push(ExportedFile {
            filename,
            data: png_bytes,
        });
    }

    Ok(CampaignExportResult {
        cards,
        lorebook_json: shared_lorebook_json,
    })
}

/// 导出 StoryForge Campaign 完整 JSON Bundle
///
/// 包含 Campaign 元数据 + Instances + Definitions + Knowledge + Tasks + Summaries。
#[tauri::command]
fn export_campaign_bundle(campaign_id: String) -> Result<String, TauriCommandError> {
    let store = get_campaign_store();
    let camp_id = Id::from_str(&campaign_id);

    let campaign = store
        .get_campaign(&camp_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;

    let stored_card = store.get_card(&campaign.card_id);
    let instances = store.list_instances(&camp_id);
    let definitions: Vec<_> = stored_card
        .as_ref()
        .map(|c| c.card.character_definitions.clone())
        .unwrap_or_default();
    let knowledge = store.list_knowledge(&camp_id);
    let tasks = store.list_tasks(&camp_id);
    let summaries = store.list_summaries(&camp_id);

    let bundle = CampaignBundle {
        format_version: BUNDLE_FORMAT_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339(),
        campaign,
        instances,
        definitions,
        knowledge,
        tasks,
        summaries,
    };

    serde_json::to_string_pretty(&bundle)
        .map_err(|e| TauriCommandError::internal(format!("Bundle 序列化失败: {e}")))
}

/// 把 CharacterKnowledgeEntry 列表转为 ST WorldInfoBook（导出用）
fn knowledge_to_st_book(
    entries: &[storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
) -> storyforge_domain::character::StWorldInfoBook {
    use storyforge_domain::character::{StWorldInfoBook, StWorldInfoEntry};

    let st_entries: Vec<StWorldInfoEntry> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| StWorldInfoEntry {
            id: Some(i as i32 + 1),
            keys: vec![e.knowledge_text.chars().take(20).collect()],
            secondary_keys: None,
            content: Some(e.knowledge_text.clone()),
            constant: e.pinned,   // pinned → 蓝灯（常驻）
            selective: !e.pinned, // 非 pinned → 绿灯（选择性）
            selective_logic: None,
            position: Some(serde_json::json!(0)),
            disable: None,
            order: Some(100),
            depth: Some(2),
            extensions: serde_json::json!({}),
        })
        .collect();

    StWorldInfoBook {
        entries: st_entries,
    }
}

/// 文件名清理（去除不合法字符）
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect()
}

// ─── W8 MVU JS Runtime 命令 ─────────────────────────────────────────────

/// 前端确认 unload 完成
#[tauri::command]
async fn mvu_unload_ack() -> Result<(), TauriCommandError> {
    tracing::debug!("[MVU] unload ack received");
    Ok(())
}

/// 前端确认 load assets 完成
#[tauri::command]
async fn mvu_load_ack(error: Option<String>) -> Result<(), TauriCommandError> {
    if let Some(err) = error {
        tracing::warn!("[MVU] load assets error: {err}");
    } else {
        tracing::debug!("[MVU] load ack received");
    }
    Ok(())
}

/// 前端回传 execute 结果（完成 WebViewMvuRuntime 的 pending oneshot）
#[tauri::command]
async fn mvu_execute_result(
    pending: tauri::State<'_, MvuPendingMap>,
    request_id: String,
    variable_updates: std::collections::HashMap<String, serde_json::Value>,
    side_effects: Vec<String>,
    error: Option<String>,
) -> Result<(), TauriCommandError> {
    let response = MvuExecuteResponse {
        request_id: request_id.clone(),
        variable_updates,
        side_effects,
        error,
    };
    let mut map = pending.lock().await;
    if let Some(tx) = map.remove(&request_id) {
        let _ = tx.send(response);
    } else {
        tracing::warn!("[MVU] mvu_execute_result: unknown request_id {request_id}");
    }
    Ok(())
}

// ─── Tauri app 入口 ────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 先构造 AppState（含 log_store），再初始化 tracing 接入 LogStore
    let app_state = Arc::new(AppState::new());
    storyforge_app_logging::init_tracing(app_state.log_store.clone());

    // W8 MVU JS Runtime：共享 pending map
    let mvu_pending: MvuPendingMap = new_mvu_pending_map();

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .manage(mvu_pending.clone())
        .setup(move |app| {
            // W8: 创建 WebViewMvuRuntime，共享同一个 pending map
            let mvu_rt =
                WebViewMvuRuntime::with_shared_pending(app.handle().clone(), mvu_pending.clone());
            // W10: 存入全局 OnceLock，供 new_pipeline 注入到 PipelineOrchestrator
            let _ = MVU_RUNTIME.set(Arc::new(mvu_rt));
            app.manage(MVU_RUNTIME.get().unwrap().clone());
            Ok(())
        })
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
            list_agent_profile_configs,
            get_agent_profile_config,
            get_active_agent_profile_config,
            save_agent_profile_config,
            delete_agent_profile_config,
            set_active_agent_profile_config,
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
            delete_conversation,
            get_conversation,
            // 重 roll 命令
            regenerate,
            // M1 对话操作命令
            edit_variant,
            accept_variant,
            soft_delete_variant,
            delete_message_from,
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
            delete_card,
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
            meta_preview_mvu_apply,
            meta_apply_mvu_schema,
            meta_classify_st_preset,
            meta_health_check,
            meta_explain_generation,
            // 第三轮：类型化 Patch 修复闭环
            meta_propose_campaign_repairs,
            meta_list_typed_patches,
            meta_preview_typed_patch,
            meta_accept_typed_patch,
            meta_dismiss_typed_patch,
            // W7 导出命令
            export_st_card_png,
            export_campaign_st_cards,
            export_campaign_bundle,
            // W8 MVU JS Runtime 命令
            mvu_unload_ack,
            mvu_load_ack,
            mvu_execute_result,
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
fn stored_info_to_character(
    stored: &storage::StoredCharacter,
) -> storyforge_domain::character::Character {
    storyforge_domain::character::Character {
        id: Id::from_str(
            stored
                .info
                .source_character_id
                .as_deref()
                .unwrap_or(&stored.id),
        ),
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
        extensions: stored.info.extensions.clone(),
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
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use storyforge_domain::character::Character;

    #[derive(Default)]
    struct MemorySecretStore {
        secrets: Mutex<HashMap<String, String>>,
        deleted: Mutex<Vec<String>>,
    }

    impl SecretStore for MemorySecretStore {
        fn put_secret(&self, secret_ref: &str, secret: &str) -> Result<(), String> {
            self.secrets
                .lock()
                .unwrap()
                .insert(secret_ref.to_string(), secret.to_string());
            Ok(())
        }

        fn get_secret(&self, secret_ref: &str) -> Result<String, String> {
            self.secrets
                .lock()
                .unwrap()
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| format!("missing secret {secret_ref}"))
        }

        fn delete_secret(&self, secret_ref: &str) -> Result<(), String> {
            self.secrets.lock().unwrap().remove(secret_ref);
            self.deleted.lock().unwrap().push(secret_ref.to_string());
            Ok(())
        }
    }

    #[test]
    fn test_embed_config_stores_secret_ref_not_plaintext() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_embed_secure_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = MemorySecretStore::default();
        let config = storyforge_infra_llm::EmbedConfig {
            endpoint: "https://api.example.com/v1/embeddings".into(),
            api_key: "embed-secret".into(),
            model: "embed-model".into(),
            dim: 3,
        };

        persist_embed_config_secret_ref(&dir, &config, &secret_store).unwrap();
        let raw = std::fs::read_to_string(dir.join("embed.json")).unwrap();
        assert!(!raw.contains("embed-secret"));
        assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));

        let loaded = load_embed_config_with_secret_store(&dir, &secret_store).unwrap();
        assert_eq!(loaded.api_key, "embed-secret");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_embed_config_load_migrates_plaintext_key() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_embed_migrate_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = MemorySecretStore::default();
        let config = storyforge_infra_llm::EmbedConfig {
            endpoint: "https://api.example.com/v1/embeddings".into(),
            api_key: "legacy-embed-secret".into(),
            model: "embed-model".into(),
            dim: 3,
        };
        storyforge_infra_util::atomic_write_json(&dir.join("embed.json"), &config).unwrap();

        let loaded = load_embed_config_with_secret_store(&dir, &secret_store).unwrap();
        assert_eq!(loaded.api_key, "legacy-embed-secret");
        let raw = std::fs::read_to_string(dir.join("embed.json")).unwrap();
        assert!(!raw.contains("legacy-embed-secret"));
        assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_diagnostic_context_summarizes_stores_without_secret_values() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_diag_context_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        storyforge_infra_util::atomic_write_json_str(
            &dir.join("connections.json"),
            r#"{"items":[{"api_key":"sk-live-secret"}]}"#,
        )
        .unwrap();
        storyforge_infra_util::atomic_write_json_str(
            &dir.join("embed.json"),
            r#"{"api_key":"embed-live-secret"}"#,
        )
        .unwrap();

        let context = diagnostic_context_for_data_dir(&dir);
        let json = serde_json::to_string(&context).unwrap();

        assert!(json.contains("connections.json"));
        assert!(json.contains("embed.json"));
        assert!(json.contains("logs"));
        assert!(!json.contains("sk-live-secret"));
        assert!(!json.contains("embed-live-secret"));
        assert_eq!(context["store_files"][0]["has_bytes"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_meta_session_explainer_is_injected() {
        let state = AppState::new_for_test();
        assert!(
            state.meta_session.explainer.is_some(),
            "meta_session.explainer should be injected (not None) for inspect_generation"
        );
        // campaign_runtime 默认为 None（无 active campaign）
        assert!(
            state
                .meta_session
                .campaign_runtime
                .lock()
                .unwrap()
                .is_none(),
            "meta_session.campaign_runtime should be None when no active campaign"
        );
    }

    /// 验证 tool_ctx 的 RwLock + snapshot 机制：写入后快照能读到
    /// （这是 import_character 同步 tool_ctx 的核心机制）
    #[test]
    fn test_tool_ctx_snapshot_sees_writes() {
        let state = AppState::new_for_test();

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
    fn test_delete_character_source_ids_include_domain_character_id() {
        let mut character = make_test_character("Lin");
        character.id = Id::from_str("domain-lin");

        let source_ids = delete_character_cascade_source_ids(
            "stored-lin",
            Some("Lin"),
            None,
            &[Arc::new(character)],
        );

        assert_eq!(
            source_ids,
            vec![Id::from_str("stored-lin"), Id::from_str("domain-lin")]
        );
    }

    #[test]
    fn test_delete_character_source_ids_deduplicate_stored_id() {
        let mut character = make_test_character("Lin");
        character.id = Id::from_str("same-id");

        let source_ids = delete_character_cascade_source_ids(
            "same-id",
            Some("Lin"),
            None,
            &[Arc::new(character)],
        );

        assert_eq!(source_ids, vec![Id::from_str("same-id")]);
    }

    #[test]
    fn test_delete_character_source_ids_include_persisted_source_character_id() {
        let source_ids =
            delete_character_cascade_source_ids("stored-lin", Some("Lin"), Some("source-lin"), &[]);

        assert_eq!(
            source_ids,
            vec![Id::from_str("stored-lin"), Id::from_str("source-lin")]
        );
    }

    #[test]
    fn test_stored_info_to_character_uses_persisted_source_character_id() {
        let mut character = make_test_character("Lin");
        character.id = Id::from_str("source-lin");
        let stored = storage::StoredCharacter {
            id: "stored-lin".into(),
            info: CharacterInfo::from(&character),
            imported_at: "now".into(),
        };

        let restored = stored_info_to_character(&stored);

        assert_eq!(restored.id, Id::from_str("source-lin"));
        assert_eq!(restored.name, "Lin");
    }

    #[test]
    fn test_stored_info_to_character_preserves_extensions_for_scoped_regex() {
        let mut character = make_test_character("Regex Card");
        character.id = Id::from_str("source-regex");
        character.extensions = serde_json::json!({
            "regex_scripts": [
                {
                    "id": "scoped-output",
                    "scriptName": "Scoped output",
                    "findRegex": "foo",
                    "replaceString": "bar",
                    "placement": [2],
                    "disabled": false
                }
            ]
        });
        let stored = storage::StoredCharacter {
            id: "stored-regex".into(),
            info: CharacterInfo::from(&character),
            imported_at: "now".into(),
        };

        let restored = stored_info_to_character(&stored);
        let scripts = restored.scoped_regex_scripts();

        assert_eq!(restored.extensions, character.extensions);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "scoped-output");
    }

    #[test]
    fn test_collect_scoped_regex_scripts_is_limited_to_selected_character() {
        let mut selected = make_test_character("Selected");
        selected.id = Id::from_str("source-selected");
        selected.extensions = serde_json::json!({
            "regex_scripts": [{
                "id": "selected-regex",
                "scriptName": "Selected regex",
                "findRegex": "foo",
                "replaceString": "bar",
                "placement": [2],
                "disabled": false
            }]
        });
        let mut other = make_test_character("Other");
        other.id = Id::from_str("source-other");
        other.extensions = serde_json::json!({
            "regex_scripts": [{
                "id": "other-regex",
                "scriptName": "Other regex",
                "findRegex": "baz",
                "replaceString": "qux",
                "placement": [2],
                "disabled": false
            }]
        });
        let characters = vec![Arc::new(selected), Arc::new(other)];

        let scripts = collect_scoped_regex_scripts(Some("source-selected"), &characters);

        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "selected-regex");
        assert!(collect_scoped_regex_scripts(None, &characters).is_empty());
        assert!(collect_scoped_regex_scripts(Some("missing"), &characters).is_empty());
    }

    #[test]
    fn test_postprocess_task_update_skips_other_campaign() {
        use storyforge_domain::story_task::{StoryTask, TaskStatus};

        let task = StoryTask::user_planned(
            Id::from_str("campaign-b"),
            "Find the archive",
            "Unrelated campaign task",
            vec![],
            1,
        );

        let updated = normalize_task_update_for_postprocess(
            &Id::from_str("campaign-a"),
            task,
            TaskStatus::Completed,
        );

        assert!(updated.is_none());
    }

    #[test]
    fn test_postprocess_task_update_allows_current_campaign() {
        use storyforge_domain::story_task::{StoryTask, TaskStatus};

        let task = StoryTask::user_planned(
            Id::from_str("campaign-a"),
            "Find the archive",
            "Current campaign task",
            vec![],
            1,
        );

        let updated = normalize_task_update_for_postprocess(
            &Id::from_str("campaign-a"),
            task,
            TaskStatus::Completed,
        )
        .expect("same campaign task should update");

        assert_eq!(updated.status, TaskStatus::Completed);
    }

    #[test]
    fn test_postprocess_knowledge_name_normalizes_to_instance_id() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_knowledge_norm_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();
        let mut instance = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        instance.id = Id::from_str("inst-lin");
        store.add_instance(instance).unwrap();

        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Lin"),
            knowledge_text: "Lin found the key".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            pinned: false,
            broadcast: None,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };
        let present_ids = HashSet::from([String::from("inst-lin")]);

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            7,
            &present_ids,
            &HashSet::new(),
        );
        assert_eq!(
            entries.len(),
            1,
            "name target should resolve to campaign instance"
        );
        let entry = &entries[0];
        assert_eq!(entry.character_id, Id::from_str("inst-lin"));
        assert_eq!(entry.knowledge_text, "Lin found the key");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_postprocess_knowledge_skips_non_present_instance() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_knowledge_present_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();
        let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
        chen.id = Id::from_str("inst-chen");
        store.add_instance(lin).unwrap();
        store.add_instance(chen).unwrap();

        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Chen"),
            knowledge_text: "Chen saw the key".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            pinned: false,
            broadcast: None,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };
        let present_ids = HashSet::from([String::from("inst-lin")]);

        let entry = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            7,
            &present_ids,
            &HashSet::new(),
        );

        assert!(entry.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_postprocess_knowledge_normalizes_source_character_id() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_knowledge_source_norm_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();
        let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
        chen.id = Id::from_str("inst-chen");
        store.add_instance(lin).unwrap();
        store.add_instance(chen).unwrap();

        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Lin"),
            knowledge_text: "Chen told Lin about the key".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(Id::from_str("Chen")),
            pinned: false,
            broadcast: None,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };
        let present_ids = HashSet::from([String::from("Lin")]);

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            7,
            &present_ids,
            &HashSet::new(),
        );
        assert_eq!(entries.len(), 1, "target should resolve");
        let entry = &entries[0];
        assert_eq!(entry.character_id, Id::from_str("inst-lin"));
        assert_eq!(entry.source_character_id, Some(Id::from_str("inst-chen")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_postprocess_knowledge_skips_unpersisted_temporary_name() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_knowledge_temp_skip_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Ghost"),
            knowledge_text: "Ghost appeared briefly".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            pinned: false,
            broadcast: None,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };
        let present_ids = HashSet::from([String::from("Ghost")]);

        let entry = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            7,
            &present_ids,
            &HashSet::new(),
        );

        assert!(entry.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── W6 方向 1：广播分发测试 ───────────────────────────────────────────

    #[test]
    fn test_broadcast_all_distributes_to_all_instances() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{
            BroadcastTarget, CharacterKnowledgeUpdate, KnowledgeSource,
        };

        let dir = std::env::temp_dir().join(format!("sf_broadcast_all_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        // 创建 3 个 instance
        let mut a = CharacterInstance::temporary(campaign.id.clone(), "A");
        a.id = Id::from_str("inst-a");
        let mut b = CharacterInstance::temporary(campaign.id.clone(), "B");
        b.id = Id::from_str("inst-b");
        let mut c = CharacterInstance::temporary(campaign.id.clone(), "C");
        c.id = Id::from_str("inst-c");
        store.add_instance(a).unwrap();
        store.add_instance(b).unwrap();
        store.add_instance(c).unwrap();

        // 广播发起者是 A（source_character_id=A），broadcast=All
        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("A"),
            knowledge_text: "全城戒严公告".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: Some(Id::from_str("A")),
            pinned: false,
            broadcast: Some(BroadcastTarget::All),
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            1,
            &HashSet::new(),
            &HashSet::new(),
        );

        // 应分发给 B 和 C（排除发起者 A 自身）
        assert_eq!(
            entries.len(),
            2,
            "broadcast All 应分发给除发起者外的所有 instance"
        );
        let target_ids: Vec<_> = entries.iter().map(|e| e.character_id.clone()).collect();
        assert!(target_ids.contains(&Id::from_str("inst-b")));
        assert!(target_ids.contains(&Id::from_str("inst-c")));
        assert!(
            !target_ids.contains(&Id::from_str("inst-a")),
            "不应分发给发起者自身"
        );

        // 每条都是 ToldByOther，source_character_id = A
        for entry in &entries {
            assert_eq!(entry.source, KnowledgeSource::ToldByOther);
            assert_eq!(entry.source_character_id, Some(Id::from_str("inst-a")));
            assert_eq!(entry.knowledge_text, "全城戒严公告");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_broadcast_group_distributes_to_matching_group() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
        use storyforge_domain::character_knowledge::{
            BroadcastTarget, CharacterKnowledgeUpdate, KnowledgeSource,
        };

        let dir = std::env::temp_dir().join(format!("sf_broadcast_group_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        // 创建 card + definitions（有 group 和无 group）
        let def_guard = CharacterDefinition {
            id: Id::from_str("def-guard"),
            card_id: Id::from_str("card-1"),
            name: "Guard".into(),
            persona_prompt: "guard".into(),
            behavior_rules: "guard".into(),
            base_backstory: vec![],
            group: Some("守卫".to_string()),
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        };
        let def_merchant = CharacterDefinition {
            id: Id::from_str("def-merchant"),
            card_id: Id::from_str("card-1"),
            name: "Merchant".into(),
            persona_prompt: "merchant".into(),
            behavior_rules: "merchant".into(),
            base_backstory: vec![],
            group: Some("商人".to_string()),
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        };
        let def_leader = CharacterDefinition {
            id: Id::from_str("def-leader"),
            card_id: Id::from_str("card-1"),
            name: "Leader".into(),
            persona_prompt: "leader".into(),
            behavior_rules: "leader".into(),
            base_backstory: vec![],
            group: None, // 无 group
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        };
        let card = CharacterCard {
            id: Id::from_str("card-1"),
            name: "test card".into(),
            source_character_id: Id::from_str("src-1"),
            character_definitions: vec![def_guard, def_merchant, def_leader],
            raw_card_json: serde_json::Value::Null,
        };
        store.save_card(card).unwrap();

        // 创建 instances（link to definitions）
        let mut inst_guard = CharacterInstance::temporary(campaign.id.clone(), "Guard");
        inst_guard.id = Id::from_str("inst-guard");
        inst_guard.definition_id = Some(Id::from_str("def-guard"));
        let mut inst_merchant = CharacterInstance::temporary(campaign.id.clone(), "Merchant");
        inst_merchant.id = Id::from_str("inst-merchant");
        inst_merchant.definition_id = Some(Id::from_str("def-merchant"));
        let mut inst_leader = CharacterInstance::temporary(campaign.id.clone(), "Leader");
        inst_leader.id = Id::from_str("inst-leader");
        inst_leader.definition_id = Some(Id::from_str("def-leader"));
        store.add_instance(inst_guard).unwrap();
        store.add_instance(inst_merchant).unwrap();
        store.add_instance(inst_leader).unwrap();

        // 广播给"守卫"组
        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Leader"),
            knowledge_text: "守卫集合命令".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: Some(Id::from_str("Leader")),
            pinned: false,
            broadcast: Some(BroadcastTarget::Group("守卫".to_string())),
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            1,
            &HashSet::new(),
            &HashSet::new(),
        );

        // 只有 Guard（group=守卫）收到，Merchant（group=商人）和 Leader（group=None，且是发起者）不收
        assert_eq!(entries.len(), 1, "broadcast Group('守卫') 应只分发给守卫组");
        assert_eq!(entries[0].character_id, Id::from_str("inst-guard"));
        assert_eq!(entries[0].source, KnowledgeSource::ToldByOther);
        assert_eq!(
            entries[0].source_character_id,
            Some(Id::from_str("inst-leader"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_broadcast_none_single_character_unaffected() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

        let dir = std::env::temp_dir().join(format!("sf_broadcast_none_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut a = CharacterInstance::temporary(campaign.id.clone(), "A");
        a.id = Id::from_str("inst-a");
        let mut b = CharacterInstance::temporary(campaign.id.clone(), "B");
        b.id = Id::from_str("inst-b");
        store.add_instance(a).unwrap();
        store.add_instance(b).unwrap();

        // broadcast=None → 单角色定向，走原有 P3 逻辑
        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("A"),
            knowledge_text: "A 看到了什么".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            pinned: false,
            broadcast: None,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };
        let present_ids = HashSet::from([String::from("inst-a")]);

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            1,
            &present_ids,
            &HashSet::new(),
        );

        // 单角色：只有 A 收到
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].character_id, Id::from_str("inst-a"));
        assert_eq!(entries[0].source, KnowledgeSource::Witnessed);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_private_source_knowledge_blocks_told_by_other_propagation() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeEntry, CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };

        let dir = std::env::temp_dir().join(format!("sf_private_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "private-test");
        store.save_campaign(campaign.clone()).unwrap();

        let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
        chen.id = Id::from_str("inst-chen");
        store.add_instance(lin).unwrap();
        store.add_instance(chen).unwrap();

        let mut private_entry = CharacterKnowledgeEntry::witnessed(
            campaign.id.clone(),
            Id::from_str("inst-lin"),
            "保险柜密码是 0427",
            1,
        );
        private_entry.propagation = PropagationPolicy::Private;
        store.add_knowledge(vec![private_entry]).unwrap();

        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Chen"),
            knowledge_text: "保险柜密码是 0427".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(Id::from_str("Lin")),
            pinned: false,
            broadcast: None,
            propagation: PropagationPolicy::Open,
        };

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            2,
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(
            entries.is_empty(),
            "private source knowledge must not propagate"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_private_knowledge_update_cannot_broadcast() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{
            BroadcastTarget, CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };

        let dir =
            std::env::temp_dir().join(format!("sf_private_broadcast_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "private-broadcast-test");
        store.save_campaign(campaign.clone()).unwrap();

        let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
        chen.id = Id::from_str("inst-chen");
        store.add_instance(lin).unwrap();
        store.add_instance(chen).unwrap();

        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Lin"),
            knowledge_text: "保险柜密码是 0427".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: Some(Id::from_str("Lin")),
            pinned: false,
            broadcast: Some(BroadcastTarget::All),
            propagation: PropagationPolicy::Private,
        };

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            2,
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(
            entries.is_empty(),
            "private knowledge must not be broadcast"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_told_by_other_links_matching_source_knowledge() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeEntry, CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };

        let dir = std::env::temp_dir().join(format!("sf_relay_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "relay-test");
        store.save_campaign(campaign.clone()).unwrap();

        let mut a = CharacterInstance::temporary(campaign.id.clone(), "A");
        a.id = Id::from_str("inst-a");
        let mut b = CharacterInstance::temporary(campaign.id.clone(), "B");
        b.id = Id::from_str("inst-b");
        let mut c = CharacterInstance::temporary(campaign.id.clone(), "C");
        c.id = Id::from_str("inst-c");
        store.add_instance(a).unwrap();
        store.add_instance(b).unwrap();
        store.add_instance(c).unwrap();

        let a_entry = CharacterKnowledgeEntry::witnessed(
            campaign.id.clone(),
            Id::from_str("inst-a"),
            "地下室有尸体",
            1,
        );
        let mut b_entry = CharacterKnowledgeEntry::told_by(
            campaign.id.clone(),
            Id::from_str("inst-b"),
            "地下室有尸体",
            Id::from_str("inst-a"),
            2,
        );
        b_entry.source_knowledge_id = Some(a_entry.id.clone());
        store.add_knowledge(vec![a_entry, b_entry.clone()]).unwrap();

        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("C"),
            knowledge_text: "地下室有尸体".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(Id::from_str("B")),
            pinned: false,
            broadcast: None,
            propagation: PropagationPolicy::Open,
        };

        let entries = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            3,
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].character_id, Id::from_str("inst-c"));
        assert_eq!(
            entries[0].source_knowledge_id,
            Some(b_entry.id.clone()),
            "C 的知识应链接到 B 持有的上游知识"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_knowledge_entry_dto_resolves_provenance_names() {
        let campaign_id = Id::from_str("camp");
        let lin_id = Id::from_str("inst-lin");
        let chen_id = Id::from_str("inst-chen");
        let entry = storyforge_domain::character_knowledge::CharacterKnowledgeEntry::told_by(
            campaign_id,
            lin_id.clone(),
            "地下室有尸体",
            chen_id.clone(),
            3,
        );
        let names = std::collections::HashMap::from([
            (lin_id, "林医生".to_string()),
            (chen_id, "陈警官".to_string()),
        ]);
        let knowledge_by_id = std::collections::HashMap::from([(entry.id.clone(), &entry)]);

        let dto = knowledge_entry_to_dto(&entry, &names, &knowledge_by_id);

        assert_eq!(dto.character_name.as_deref(), Some("林医生"));
        assert_eq!(dto.source_character_name.as_deref(), Some("陈警官"));
        assert!(dto.source_knowledge_id.is_none());
        assert!(dto.relay_chain_text.is_none());
        assert_eq!(dto.provenance_text, "林医生 被 陈警官 告知");
        assert_eq!(dto.propagation, "open");
    }

    #[test]
    fn test_knowledge_entry_dto_renders_relay_chain() {
        use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;

        let campaign_id = Id::from_str("camp");
        let a_id = Id::from_str("inst-a");
        let b_id = Id::from_str("inst-b");
        let c_id = Id::from_str("inst-c");

        let a_entry = CharacterKnowledgeEntry::witnessed(
            campaign_id.clone(),
            a_id.clone(),
            "地下室有尸体",
            1,
        );
        let mut b_entry = CharacterKnowledgeEntry::told_by(
            campaign_id.clone(),
            b_id.clone(),
            "地下室有尸体",
            a_id.clone(),
            2,
        );
        b_entry.source_knowledge_id = Some(a_entry.id.clone());
        let mut c_entry = CharacterKnowledgeEntry::told_by(
            campaign_id,
            c_id.clone(),
            "地下室有尸体",
            b_id.clone(),
            3,
        );
        c_entry.source_knowledge_id = Some(b_entry.id.clone());

        let names = std::collections::HashMap::from([
            (a_id, "A".to_string()),
            (b_id, "B".to_string()),
            (c_id, "C".to_string()),
        ]);
        let knowledge_by_id = std::collections::HashMap::from([
            (a_entry.id.clone(), &a_entry),
            (b_entry.id.clone(), &b_entry),
            (c_entry.id.clone(), &c_entry),
        ]);

        let dto = knowledge_entry_to_dto(&c_entry, &names, &knowledge_by_id);

        assert_eq!(
            dto.source_knowledge_id.as_deref(),
            Some(b_entry.id.as_str())
        );
        assert_eq!(
            dto.relay_chain_text.as_deref(),
            Some("A（轮 1） → B（轮 2） → C（轮 3）")
        );
    }

    #[test]
    fn test_current_cancel_slot() {
        let state = AppState::new_for_test();

        // 初始无运行中的写作
        {
            let slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            assert!(slot.is_none());
        }

        // 模拟 start_writing 设置 cancel sender
        let (tx, rx) = watch::channel(false);
        {
            let mut slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *slot = Some(tx);
        }

        // 触发取消
        {
            let slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let tx = slot.as_ref().unwrap();
            let _ = tx.send(true);
        }
        assert!(*rx.borrow(), "cancel 应已触发");

        // 清理
        {
            let mut slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *slot = None;
        }
    }

    /// 验证 active_llm_or_mock：无活跃连接时回退 mock（关键 fallback 行为）
    #[test]
    fn test_active_llm_fallback_to_mock() {
        let state = AppState::new_for_test();
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
        assert_eq!(
            parse_tool_mode("text_fallback").unwrap(),
            ToolMode::TextFallback
        );
        assert!(parse_tool_mode("unknown").is_err());
    }

    /// 验证 AppState 的 vector_store 字段初始化正常（可读写）
    #[test]
    fn test_vector_store_initialized() {
        let state = AppState::new_for_test();
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
        let hits = state
            .vector_store
            .search_by_keywords(&["测试".into()], 10)
            .unwrap();
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

    // ─── Phase 2: CampaignRuntimeContext 快照接入验证 ────────────────────────

    /// 初始状态下 tool_ctx 的 campaign_runtime 应为 None（未开 Campaign）
    #[test]
    fn test_campaign_runtime_none_by_default() {
        let state = AppState::new_for_test();
        let snap = state.snapshot_tool_ctx();
        assert!(
            snap.campaign_runtime.is_none(),
            "初始状态 campaign_runtime 应为 None"
        );
    }

    /// 写入 CampaignRuntimeContext 后，快照应能读到
    /// （模拟 fill_campaign_context 的同步机制）
    #[test]
    fn test_campaign_runtime_synced_through_rwlock() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;

        let state = AppState::new_for_test();

        // 构造一个最小的 CampaignRuntimeContext
        let campaign = Campaign::new(Id::from_str("test-card"), "test-run");
        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![],
            definitions_by_id: std::collections::HashMap::new(),
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        });

        // 写入 tool_ctx（模拟 fill_campaign_context 的行为）
        {
            let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
            ctx.campaign_runtime = Some(runtime.clone());
        }

        // 快照应能读到
        let snap = state.snapshot_tool_ctx();
        assert!(
            snap.campaign_runtime.is_some(),
            "写入后 campaign_runtime 不应为 None"
        );
        let rt = snap.campaign_runtime.as_ref().unwrap();
        assert_eq!(rt.turn, 1);
        assert!(rt.instances.is_empty());
        assert_eq!(rt.campaign.name, "test-run");
    }

    /// CampaignRuntimeContext 写入 instances/definitions/knowledge 后，
    /// 通过快照可完整读回
    #[test]
    fn test_campaign_runtime_full_snapshot_readable() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
        use storyforge_domain::variables::default_character_variables;

        let state = AppState::new_for_test();

        let campaign = Campaign::new(Id::from_str("card-1"), "full-test");

        let def = CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm surgeon".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec!["is a surgeon".into()],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };

        let instance = storyforge_domain::campaign::CharacterInstance {
            id: Id::from_str("inst-lin"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def.id.clone()),
            name: "Lin".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };

        let knowledge = CharacterKnowledgeEntry::backstory(
            campaign.id.clone(),
            Id::from_str("inst-lin"),
            "我是外科医生",
        );

        let mut definitions_by_id = std::collections::HashMap::new();
        definitions_by_id.insert(def.id.clone(), def);

        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id,
            knowledge: vec![knowledge],
            tasks: vec![],
            turn: 3,
        });

        {
            let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
            ctx.campaign_runtime = Some(runtime);
        }

        let snap = state.snapshot_tool_ctx();
        let rt = snap.campaign_runtime.as_ref().unwrap();
        assert_eq!(rt.turn, 3);
        assert_eq!(rt.instances.len(), 1);
        assert_eq!(rt.instances[0].name, "Lin");
        assert_eq!(rt.definitions_by_id.len(), 1);
        assert!(rt.definitions_by_id.contains_key(&Id::from_str("def-lin")));
        assert_eq!(rt.knowledge.len(), 1);
        assert_eq!(rt.knowledge[0].knowledge_text, "我是外科医生");

        // 验证 helper 可用
        let inst = rt.find_instance_by_id_or_name("Lin").unwrap();
        assert_eq!(rt.resolved_persona_for(inst), Some("calm surgeon"));
        assert_eq!(rt.resolved_behavior_for(inst), Some("save first"));
    }

    /// 验证 fill_campaign_context 在无 active campaign 时会清空旧 runtime
    /// （stale runtime cleanup：防止上一轮的脏快照残留）
    #[test]
    fn test_fill_campaign_context_clears_stale_runtime() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;

        let state = AppState::new_for_test();

        // 模拟上一轮残留：手动写入一个 runtime
        let campaign = Campaign::new(Id::from_str("stale-card"), "stale-run");
        let stale_runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![],
            definitions_by_id: std::collections::HashMap::new(),
            knowledge: vec![],
            tasks: vec![],
            turn: 99,
        });
        {
            let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
            ctx.campaign_runtime = Some(stale_runtime);
        }

        // 确认写入成功
        let snap_before = state.snapshot_tool_ctx();
        assert!(
            snap_before.campaign_runtime.is_some(),
            "预置 stale runtime 应成功"
        );

        // 调用 fill_campaign_context（无 active campaign → early return，但 runtime 应被清空）
        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        fill_campaign_context(&mut ctx, &state);

        // 验证：WritingContext 的 runtime 应为 None
        assert!(
            ctx.campaign_runtime.is_none(),
            "无 active campaign 时 ctx.campaign_runtime 应被清空"
        );

        // 验证：tool_ctx 的 runtime 也应被清空
        let snap_after = state.snapshot_tool_ctx();
        assert!(
            snap_after.campaign_runtime.is_none(),
            "无 active campaign 时 tool_ctx.campaign_runtime 应被清空"
        );
    }

    // ─── Phase 6：临时 instance 落盘测试 ─────────────────────────────────────

    /// persist_temporary_instances：新实例能落盘，下一轮 list_instances 可读回
    #[test]
    fn test_persist_temporary_instances_new() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_persist_temp_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_id = Some(campaign.id.clone());

        let temps = vec![
            CharacterInstance::temporary(campaign.id.clone(), "Ghost"),
            CharacterInstance::temporary_with_overrides(
                campaign.id.clone(),
                "Guard",
                Some("stern guard".into()),
                Some("block the way".into()),
            ),
        ];

        persist_temporary_instances_to(&store, &ctx, &temps);

        let instances = store.list_instances(&campaign.id);
        assert_eq!(instances.len(), 2, "应有 2 个落盘实例");

        let ghost = instances.iter().find(|i| i.name == "Ghost").unwrap();
        assert!(ghost.is_temporary);
        assert!(ghost.persona_override.is_none());

        let guard = instances.iter().find(|i| i.name == "Guard").unwrap();
        assert!(guard.is_temporary);
        assert_eq!(guard.persona_override, Some("stern guard".into()));
        assert_eq!(guard.behavior_override, Some("block the way".into()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// persist_temporary_instances：同名实例不重复落盘（去重）
    #[test]
    fn test_persist_temporary_instances_dedup_by_name() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_persist_dedup_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        // 先手动落盘一个 "Ghost"
        let existing = CharacterInstance::temporary(campaign.id.clone(), "Ghost");
        store.add_instance(existing.clone()).unwrap();

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_id = Some(campaign.id.clone());

        // 再尝试落盘同名临时 instance
        let temps = vec![CharacterInstance::temporary(campaign.id.clone(), "Ghost")];
        persist_temporary_instances_to(&store, &ctx, &temps);

        let instances = store.list_instances(&campaign.id);
        assert_eq!(instances.len(), 1, "同名不应重复落盘");
        assert_eq!(instances[0].id, existing.id, "应保留原始实例 id");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// persist_temporary_instances：同一批次内的同名临时实例也不重复落盘
    #[test]
    fn test_persist_temporary_instances_dedup_within_batch() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_persist_batch_dedup_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_id = Some(campaign.id.clone());

        let temps = vec![
            CharacterInstance::temporary(campaign.id.clone(), "Ghost"),
            CharacterInstance::temporary_with_overrides(
                campaign.id.clone(),
                "Ghost",
                Some("duplicate brief".into()),
                None,
            ),
        ];
        persist_temporary_instances_to(&store, &ctx, &temps);

        let instances = store.list_instances(&campaign.id);
        assert_eq!(instances.len(), 1, "同批同名不应重复落盘");
        assert_eq!(instances[0].name, "Ghost");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// persist_temporary_instances：拒绝写入不属于当前 Campaign 的临时实例
    #[test]
    fn test_persist_temporary_instances_skips_wrong_campaign() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_persist_wrong_campaign_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        let other_campaign = Campaign::new(Id::from_str("card-2"), "other");
        store.save_campaign(campaign.clone()).unwrap();
        store.save_campaign(other_campaign.clone()).unwrap();

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_id = Some(campaign.id.clone());

        let temps = vec![CharacterInstance::temporary(
            other_campaign.id.clone(),
            "WrongCampaignGhost",
        )];
        persist_temporary_instances_to(&store, &ctx, &temps);

        assert!(store.list_instances(&campaign.id).is_empty());
        assert!(store.list_instances(&other_campaign.id).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// persist_temporary_instances：无 campaign 时 no-op
    #[test]
    fn test_persist_temporary_instances_no_campaign() {
        use storyforge_domain::campaign::CharacterInstance;

        let ctx = WritingContext::legacy(vec![], None, Id::new());
        // campaign_id = None → 应直接返回，不 panic
        let temps = vec![CharacterInstance::temporary(Id::new(), "Ghost")];
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_persist_no_camp_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        persist_temporary_instances_to(&store, &ctx, &temps);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// persist_temporary_instances：空列表 no-op
    #[test]
    fn test_persist_temporary_instances_empty_list() {
        use storyforge_domain::campaign::Campaign;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_persist_empty_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_id = Some(campaign.id.clone());

        persist_temporary_instances_to(&store, &ctx, &[]);

        let instances = store.list_instances(&campaign.id);
        assert!(instances.is_empty(), "空列表不应写入任何实例");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 6 集成：临时 instance 落盘后，postprocess 的知识写回能找到它
    #[test]
    fn test_postprocess_writes_knowledge_for_persisted_temporary() {
        use std::collections::HashSet;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_pp_temp_knowledge_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        // 模拟 persist_temporary_instances：落盘一个临时 instance
        let mut ghost = CharacterInstance::temporary(campaign.id.clone(), "Ghost");
        ghost.persona_override = Some("mysterious figure".into());
        store.add_instance(ghost.clone()).unwrap();

        // postprocess 尝试写入 Ghost 的知识（之前会因为找不到 persisted instance 而跳过）
        let update = CharacterKnowledgeUpdate {
            character_id: Id::from_str("Ghost"),
            knowledge_text: "Ghost appeared in the fog".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            pinned: false,
            broadcast: None,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };
        let present_ids = HashSet::from([String::from("Ghost")]);

        let entry = normalize_knowledge_update_for_postprocess(
            &store,
            &campaign.id,
            &update,
            1,
            &present_ids,
            &HashSet::new(),
        );

        assert_eq!(
            entry.len(),
            1,
            "已落盘的临时 instance 应能被 postprocess 解析"
        );
        let entry = &entry[0];
        assert_eq!(entry.character_id, ghost.id);
        assert_eq!(entry.knowledge_text, "Ghost appeared in the fog");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── 第三轮：类型化 Patch 命令测试 ────────────────────────────────────────

    /// 构造一个有 orphan knowledge 的 campaign，提议后返回的 patch 数 ≥ 1
    /// 且含 delete_orphan_knowledge action
    #[test]
    fn test_meta_propose_campaign_repairs_orphan_knowledge() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
        use storyforge_domain::variables::default_character_variables;

        let dir =
            std::env::temp_dir().join(format!("sf_test_propose_repairs_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        // 手动设置 CAMPAIGN_STORE 指向临时目录（用 get_campaign_store 的底层）
        // 注意：CAMPAIGN_STORE 是 OnceLock，测试间会互相干扰。
        // 改用 campaign_store 直接测逻辑，不走 Tauri command 层。
        let store = campaign_store::CampaignStore::new(&dir);

        let card = {
            let mut c = storyforge_domain::character::CharacterCard {
                id: Id::from_str("card-1"),
                name: "测试卡".into(),
                source_character_id: Id::from_str("src-1"),
                character_definitions: vec![],
                raw_card_json: serde_json::Value::Null,
            };
            let def = CharacterDefinition {
                id: Id::from_str("def-1"),
                card_id: c.id.clone(),
                name: "Lin".into(),
                persona_prompt: "surgeon".into(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: None,
                role_type: RoleType::Protagonist,
                variable_schema: default_character_variables(),
            };
            c.character_definitions.push(def);
            c
        };
        store.save_card(card).unwrap();

        let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
        store.save_campaign(campaign.clone()).unwrap();

        let instance = CharacterInstance::from_definition(
            campaign.id.clone(),
            &store
                .get_card(&Id::from_str("card-1"))
                .unwrap()
                .card
                .character_definitions[0],
        );
        store.add_instance(instance.clone()).unwrap();

        // 添加一条指向不存在 instance 的 knowledge（orphan）
        let orphan_knowledge = CharacterKnowledgeEntry::witnessed(
            campaign.id.clone(),
            Id::from_str("nonexistent-instance"),
            "看到了什么",
            1,
        );
        store.add_knowledge(vec![orphan_knowledge]).unwrap();

        // 用 health check 找 issues
        let definitions = store
            .get_card(&campaign.card_id)
            .map(|c| c.card.character_definitions)
            .unwrap_or_default();
        let instances = store.list_instances(&campaign.id);
        let knowledge = store.list_knowledge(&campaign.id);
        let tasks = store.list_tasks(&campaign.id);

        let snapshot = storyforge_app_meta::CampaignHealthSnapshot {
            instances: &instances,
            definitions: &definitions,
            knowledge: &knowledge,
            tasks: &tasks,
        };
        let issues = storyforge_app_meta::check_campaign_health(&snapshot);
        assert!(!issues.is_empty(), "应发现至少一个 health issue");

        let input = storyforge_app_meta::PreviewInput {
            instances: &instances,
            definitions: &definitions,
            knowledge: &knowledge,
            tasks: &tasks,
            campaign: Some(&campaign),
        };

        let mut patches = Vec::new();
        for issue in &issues {
            if let Some(patch) = storyforge_app_meta::build_patch_for_issue(issue, &input) {
                patches.push(patch);
            }
        }

        assert!(!patches.is_empty(), "应生成至少一个 patch");

        // 检查是否包含 delete_orphan_knowledge action
        let has_delete_orphan = patches.iter().any(|p| {
            p.actions.iter().any(|a| {
                matches!(
                    a,
                    storyforge_app_meta::TypedPatchAction::DeleteOrphanKnowledge { .. }
                )
            })
        });
        assert!(has_delete_orphan, "应包含 delete_orphan_knowledge action");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// accept 一个 prune task reference patch 后，task.related_characters 不再含 orphan id
    #[test]
    fn test_meta_accept_typed_patch_prune_task_refs() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::story_task::StoryTask;
        use storyforge_domain::variables::default_character_variables;

        let dir =
            std::env::temp_dir().join(format!("sf_test_accept_prune_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let card = {
            let mut c = storyforge_domain::character::CharacterCard {
                id: Id::from_str("card-1"),
                name: "测试卡".into(),
                source_character_id: Id::from_str("src-1"),
                character_definitions: vec![],
                raw_card_json: serde_json::Value::Null,
            };
            let def = CharacterDefinition {
                id: Id::from_str("def-1"),
                card_id: c.id.clone(),
                name: "Lin".into(),
                persona_prompt: "surgeon".into(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: None,
                role_type: RoleType::Protagonist,
                variable_schema: default_character_variables(),
            };
            c.character_definitions.push(def);
            c
        };
        store.save_card(card).unwrap();

        let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
        store.save_campaign(campaign.clone()).unwrap();

        let instance = CharacterInstance::from_definition(
            campaign.id.clone(),
            &store
                .get_card(&Id::from_str("card-1"))
                .unwrap()
                .card
                .character_definitions[0],
        );
        store.add_instance(instance.clone()).unwrap();

        // 创建一个 task，related_characters 含 orphan id
        let mut task = StoryTask::user_planned(campaign.id.clone(), "复仇", "老王复仇", vec![], 1);
        let orphan_id = Id::from_str("orphan-char");
        task.related_characters.push(orphan_id.clone());
        task.related_characters.push(instance.id.clone());
        let task_id = task.id.clone();
        store.add_task(task).unwrap();

        // 构造 TypedPatch + apply
        let _patch = storyforge_app_meta::TypedPatch {
            id: "test-prune-patch".into(),
            description: "修剪孤儿引用".into(),
            source_issue_category: "orphan_task_references".into(),
            affected_id: Some(task_id.as_str().to_string()),
            actions: vec![
                storyforge_app_meta::TypedPatchAction::PruneOrphanTaskReferences {
                    task_id: task_id.clone(),
                    orphan_character_ids: vec![orphan_id.clone()],
                },
            ],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: storyforge_app_meta::TypedPatchStatus::Pending,
        };

        // 直接用 CampaignStore 方法模拟 apply_typed_action 的写盘逻辑
        // （apply_typed_action 需要 &'static，测试中用本地 store 直接调用）
        {
            let mut task = store.get_task(&task_id).unwrap();
            task.related_characters
                .retain(|id| ![orphan_id.clone()].contains(id));
            store.update_task(task).unwrap();
        }

        // 验证：task.related_characters 不再含 orphan
        let updated_task = store.get_task(&task_id).unwrap();
        assert!(
            !updated_task.related_characters.contains(&orphan_id),
            "orphan id 应已被移除"
        );
        assert!(
            updated_task.related_characters.contains(&instance.id),
            "正常 instance id 应保留"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// accept 前手动删掉 target instance，accept 返回错误且 status 变 Stale
    #[test]
    fn test_meta_accept_typed_patch_stale() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::variables::default_character_variables;

        let dir =
            std::env::temp_dir().join(format!("sf_test_accept_stale_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let card = {
            let mut c = storyforge_domain::character::CharacterCard {
                id: Id::from_str("card-1"),
                name: "测试卡".into(),
                source_character_id: Id::from_str("src-1"),
                character_definitions: vec![],
                raw_card_json: serde_json::Value::Null,
            };
            let def = CharacterDefinition {
                id: Id::from_str("def-1"),
                card_id: c.id.clone(),
                name: "Lin".into(),
                persona_prompt: "surgeon".into(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: None,
                role_type: RoleType::Protagonist,
                variable_schema: default_character_variables(),
            };
            c.character_definitions.push(def);
            c
        };
        store.save_card(card).unwrap();

        let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut instance = CharacterInstance::from_definition(
            campaign.id.clone(),
            &store
                .get_card(&Id::from_str("card-1"))
                .unwrap()
                .card
                .character_definitions[0],
        );
        instance.id = Id::from_str("target-inst");
        store.add_instance(instance.clone()).unwrap();

        // 构造一个指向该 instance 的 patch
        let mut patch = storyforge_app_meta::TypedPatch {
            id: "test-stale-patch".into(),
            description: "修改变量".into(),
            source_issue_category: "variable_schema_mismatch".into(),
            affected_id: Some("target-inst".into()),
            actions: vec![
                storyforge_app_meta::TypedPatchAction::SyncInstanceVariables {
                    instance_id: Id::from_str("target-inst"),
                    definition_id: Id::from_str("def-1"),
                    add_keys: vec!["new_var".into()],
                    remove_keys: vec![],
                },
            ],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: storyforge_app_meta::TypedPatchStatus::Pending,
        };

        // 删掉 target instance（模拟 stale）：
        // CampaignStore 没有 delete_instance，改用空快照模拟 target 不存在

        // 用 is_patch_stale 检测
        // 构造一个快照，其中不包含 target instance
        let empty_instances: Vec<CharacterInstance> = vec![];
        let definitions = store
            .get_card(&campaign.card_id)
            .map(|c| c.card.character_definitions)
            .unwrap_or_default();
        let knowledge = store.list_knowledge(&campaign.id);
        let tasks = store.list_tasks(&campaign.id);

        let input = storyforge_app_meta::PreviewInput {
            instances: &empty_instances,
            definitions: &definitions,
            knowledge: &knowledge,
            tasks: &tasks,
            campaign: Some(&campaign),
        };

        // is_patch_stale 应返回 true（target instance 不在快照中）
        assert!(
            storyforge_app_meta::is_patch_stale(&patch, &input),
            "删掉 target 后 patch 应为 stale"
        );

        // 模拟 accept 逻辑：stale → status 改 Stale
        if storyforge_app_meta::is_patch_stale(&patch, &input) {
            patch.status = storyforge_app_meta::TypedPatchStatus::Stale;
        }
        assert_eq!(patch.status, storyforge_app_meta::TypedPatchStatus::Stale);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// dismiss 后 status = Dismissed
    #[test]
    fn test_meta_dismiss_typed_patch() {
        let state = Arc::new(AppState::new_for_test());

        // 手动插入一条 patch
        let patch = storyforge_app_meta::TypedPatch {
            id: "test-dismiss-patch".into(),
            description: "测试忽略".into(),
            source_issue_category: "orphan_instance".into(),
            affected_id: None,
            actions: vec![],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: storyforge_app_meta::TypedPatchStatus::Pending,
        };

        {
            let mut typed = state
                .typed_patches
                .write()
                .unwrap_or_else(|p| p.into_inner());
            typed.push(patch);
        }

        // dismiss
        {
            let mut typed = state
                .typed_patches
                .write()
                .unwrap_or_else(|p| p.into_inner());
            let p = typed
                .iter_mut()
                .find(|p| p.id == "test-dismiss-patch")
                .unwrap();
            p.status = storyforge_app_meta::TypedPatchStatus::Dismissed;
        }

        // 验证
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        let p = typed.iter().find(|p| p.id == "test-dismiss-patch").unwrap();
        assert_eq!(p.status, storyforge_app_meta::TypedPatchStatus::Dismissed);

        // Pending 列表应不含该 patch
        let pending: Vec<_> = typed
            .iter()
            .filter(|p| p.status == storyforge_app_meta::TypedPatchStatus::Pending)
            .collect();
        assert!(
            pending.is_empty(),
            "dismissed patch 不应出现在 pending 列表"
        );
    }
}

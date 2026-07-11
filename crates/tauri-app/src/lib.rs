pub mod campaign_store;
mod connection_store;
pub mod error;
mod global_regex_store;
mod module_store;
mod mvu_webview_runtime;
mod preset_store;
mod storage;
pub mod turn_coordinator;
pub mod turn_store;

use chrono::Utc;
use connection_store::ConnectionStore;
use preset_store::PresetStore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use storage::CharacterStore;
use tokio::sync::{oneshot, watch};

use storyforge_app_agent::ToolContext;
use storyforge_app_agent::runtime::{PromptHook, PromptHookContext};
use storyforge_app_conversation::{ConversationStore, PartialRollTarget};
use storyforge_app_logging::{ExportOptions, LogFilter, LogKind, LogLevel, LogStore};
use storyforge_app_meta::{
    MvuApplyError, MvuApplyPreview, apply_schema_to_definition, compute_apply_preview,
};
use storyforge_app_pipeline::{PipelineOrchestrator, RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::conversation::{
    Conversation, MessageNode, MessageVariant, Provenance, Role as ConversationRole, VariantStatus,
};
use storyforge_domain::llm::{
    ChatMessage, LlmConnection, LlmConnectionSummary, LlmProtocol, SamplingParams, ToolMode,
};
use storyforge_domain::preset::{
    RegexPlacement, RegexScript, RegexScriptSource, merge_regex_script_sources,
};
use storyforge_domain::prompt_module::PromptProfile;
use storyforge_infra_llm::LlmClient;
use storyforge_infra_plugin_host::PluginRegistry;
use storyforge_infra_plugin_host::mvu_runtime::MvuExecuteResponse;
use storyforge_infra_regex::{
    RegexExecutionTarget, apply_reasoning_regex_to_think_blocks_at_depth,
    apply_regex_scripts_for_target_at_depth,
};
use storyforge_infra_util::secret_store::{
    SecretStore, SystemSecretStore, is_secret_ref, make_secret_ref, resolve_secret_value,
};
use storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord, VectorStore};
use tauri::Manager;

use crate::error::TauriCommandError;
use crate::mvu_webview_runtime::{MvuPendingMap, WebViewMvuRuntime, new_mvu_pending_map};

type PromptHookPendingMap =
    Arc<Mutex<std::collections::HashMap<String, oneshot::Sender<PromptHookReply>>>>;

struct PromptHookPendingGuard {
    request_id: String,
    pending: PromptHookPendingMap,
    active: bool,
}

impl PromptHookPendingGuard {
    fn new(request_id: String, pending: PromptHookPendingMap) -> Self {
        Self {
            request_id,
            pending,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PromptHookPendingGuard {
    fn drop(&mut self) {
        if self.active {
            let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            pending.remove(&self.request_id);
        }
    }
}

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

static CONN_STORE: OnceLock<Arc<ConnectionStore>> = OnceLock::new();

fn get_conn_store() -> Arc<ConnectionStore> {
    CONN_STORE
        .get_or_init(|| {
            let data_dir = get_app_data_dir();
            Arc::new(ConnectionStore::new(&data_dir))
        })
        .clone()
}

static PRESET_STORE: OnceLock<PresetStore> = OnceLock::new();

fn get_preset_store() -> &'static PresetStore {
    PRESET_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        PresetStore::new(&data_dir)
    })
}

static GLOBAL_REGEX_STORE: OnceLock<global_regex_store::GlobalRegexStore> = OnceLock::new();

fn get_global_regex_store() -> &'static global_regex_store::GlobalRegexStore {
    GLOBAL_REGEX_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        global_regex_store::GlobalRegexStore::new(&data_dir)
    })
}

static CAMPAIGN_STORE: OnceLock<campaign_store::CampaignStore> = OnceLock::new();

fn get_campaign_store() -> &'static campaign_store::CampaignStore {
    CAMPAIGN_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        campaign_store::CampaignStore::new(&data_dir)
    })
}

static TURN_STORE: OnceLock<turn_store::TurnStore> = OnceLock::new();

fn get_turn_store() -> &'static turn_store::TurnStore {
    TURN_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        turn_store::TurnStore::new(&data_dir)
    })
}

/// Phase A 屏障：检查活跃 Campaign 是否有未完成的 Turn。
///
/// 在 `start_writing` 追加 user 消息**之前**调用。
/// 如果存在非 terminal Turn，返回错误，阻止新一轮启动。
/// 非 Campaign 模式（无活跃 Campaign）直接放行。
fn check_turn_barrier(state: &Arc<AppState>) -> Result<(), TauriCommandError> {
    let active_campaign_id = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };
    let Some(campaign_id) = active_campaign_id else {
        return Ok(()); // 非 Campaign 模式，放行
    };
    if let Some(turn) = get_turn_store().get_active_turn(&campaign_id) {
        return Err(TauriCommandError::validation(format!(
            "当前有未完成的轮次（turn_id={}, status={:?}），请先 Accept、Discard 或 Abandon 后再开始下一轮",
            turn.turn_id, turn.status
        )));
    }
    Ok(())
}

fn to_json_value<T: Serialize + ?Sized>(
    value: &T,
    label: &str,
) -> Result<serde_json::Value, TauriCommandError> {
    serde_json::to_value(value)
        .map_err(|e| TauriCommandError::internal(format!("{label} 序列化失败: {e}")))
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
        let mut config: storyforge_infra_llm::EmbedConfig = match serde_json::from_str(&data) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!("嵌入配置 JSON 解析失败({e})，尝试 .tmp 备份");
                let tmp = PathBuf::from(format!("{}.tmp", path.display()));
                let tmp_data = std::fs::read_to_string(&tmp).ok()?;
                serde_json::from_str(&tmp_data).ok()?
            }
        };
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
    /// 等待前端插件处理最终 LLM messages prompt hook 的请求。
    prompt_hook_pending: PromptHookPendingMap,
    /// 当前活跃连接构造的 LLM client（None = 用 mock_llm）
    active_llm: Mutex<Option<Arc<dyn LlmClient>>>,
    /// 当前活跃连接的 ID（用于 get_active_connection 快速查询）
    active_conn_id: Mutex<Option<String>>,
    /// 序列化 async “设置活跃连接”中的写盘与内存 client 更新。
    active_connection_update: tokio::sync::Mutex<()>,
    /// 向量存储（关键词搜索 + 后续向量搜索，持久化到 data/vectors.json）
    pub vector_store: Arc<BruteForceStore>,
    /// Meta Agent Patch 存储
    pub meta_patches: Arc<RwLock<Vec<storyforge_app_meta::Patch>>>,
    /// 类型化 Patch 存储（第三轮：campaign-runtime 修复）
    pub typed_patches: Arc<RwLock<Vec<storyforge_app_meta::TypedPatch>>>,
    /// Serialize typed patch accept so preflight and writes cannot interleave.
    typed_patch_accept_lock: Mutex<()>,
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
            regex_scripts: vec![],
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
            prompt_hook_pending: Arc::new(Mutex::new(std::collections::HashMap::new())),
            active_llm: Mutex::new(active_llm),
            active_conn_id: Mutex::new(active_conn_id),
            active_connection_update: tokio::sync::Mutex::new(()),
            vector_store,
            meta_patches: Arc::new(RwLock::new(Vec::new())),
            typed_patches: Arc::new(RwLock::new(Vec::new())),
            typed_patch_accept_lock: Mutex::new(()),
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

    fn apply_active_connection(
        &self,
        id: &str,
        conn: LlmConnection,
    ) -> Result<(), TauriCommandError> {
        let conn_name = conn.name.clone();

        let client = storyforge_infra_llm::create_client(&conn).map_err(TauriCommandError::from)?;

        // 包装 LlmInterceptor：每次 LLM 调用自动记录 payload/响应/token/延迟到 LogStore
        let intercepted: Arc<dyn LlmClient> =
            Arc::new(storyforge_app_logging::interceptor::LlmInterceptor::new(
                Arc::from(client),
                self.log_store.clone(),
                conn_name,
            ));

        *self.active_llm.lock().unwrap_or_else(|p| p.into_inner()) = Some(intercepted);
        *self
            .active_conn_id
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(id.to_string());
        Ok(())
    }

    pub async fn set_active_connection_async(
        self: Arc<Self>,
        id: String,
    ) -> Result<(), TauriCommandError> {
        set_active_connection_with_store_async(self, get_conn_store(), id).await
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
        self.new_pipeline_with_regex(&[])
    }

    pub fn new_pipeline_with_regex(&self, regex_scripts: &[RegexScript]) -> PipelineOrchestrator {
        self.new_pipeline_with_regex_and_prompt_hook(regex_scripts, None)
    }

    pub fn new_pipeline_with_regex_and_prompt_hook(
        &self,
        regex_scripts: &[RegexScript],
        prompt_hook: Option<PromptHook>,
    ) -> PipelineOrchestrator {
        let llm = self.active_llm_or_mock();
        let mut tool_ctx = (*self.snapshot_tool_ctx()).clone();
        // 注入向量存储（search_vectors 工具用）
        tool_ctx.vector_store = Some(self.vector_store.clone());
        tool_ctx.regex_scripts = regex_scripts.to_vec();
        // W10: 注入 MVU JS runtime（None = setup 未运行或 WebView 不可用，降级）
        let mvu_rt: Option<
            Arc<dyn storyforge_infra_plugin_host::mvu_runtime::MvuRuntime + Send + Sync>,
        > = MVU_RUNTIME.get().cloned().map(|r| {
            r as Arc<dyn storyforge_infra_plugin_host::mvu_runtime::MvuRuntime + Send + Sync>
        });
        if let Some(prompt_hook) = prompt_hook {
            PipelineOrchestrator::new_with_prompt_hook(
                llm,
                self.conv_store.clone(),
                Arc::new(tool_ctx),
                mvu_rt,
                prompt_hook,
            )
        } else {
            PipelineOrchestrator::new(llm, self.conv_store.clone(), Arc::new(tool_ctx), mvu_rt)
        }
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
    pub active: bool,
}

/// 预设详情 DTO
#[derive(Debug, Clone, Serialize)]
pub struct PresetDetailDto {
    pub id: String,
    pub name: String,
    pub prompts: Vec<PresetPromptDto>,
    pub regex_scripts: Vec<RegexScriptDto>,
    pub imported_at: String,
    pub active: bool,
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
    pub placement_codes: Vec<i32>,
    pub disabled: bool,
}

fn regex_script_dto(r: &RegexScript) -> RegexScriptDto {
    RegexScriptDto {
        id: r.id.clone(),
        script_name: r.script_name.clone(),
        find_regex: r.find_regex.clone(),
        replace_string: r.replace_string.clone(),
        placement: match r.placement {
            storyforge_domain::preset::RegexPlacement::Input => "input",
            storyforge_domain::preset::RegexPlacement::Output => "output",
            storyforge_domain::preset::RegexPlacement::SlashCommand => "slash_command",
            storyforge_domain::preset::RegexPlacement::WorldInfo => "world_info",
            storyforge_domain::preset::RegexPlacement::Reasoning => "reasoning",
        }
        .to_string(),
        placement_codes: r.placement_codes.clone(),
        disabled: r.disabled,
    }
}

#[tauri::command]
fn list_presets() -> Vec<PresetSummaryDto> {
    let store = get_preset_store();
    let active_id = store.active_id();
    store
        .list()
        .iter()
        .map(|sp| PresetSummaryDto {
            id: sp.id.clone(),
            name: sp.preset.name.clone(),
            prompt_count: sp.preset.prompts.len(),
            regex_count: sp.preset.regex_scripts.len(),
            imported_at: sp.imported_at.clone(),
            active: active_id.as_deref() == Some(sp.id.as_str()),
        })
        .collect()
}

#[tauri::command]
fn get_preset(id: String) -> Result<PresetDetailDto, TauriCommandError> {
    let store = get_preset_store();
    let active_id = store.active_id();
    let sp = store
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
            .map(regex_script_dto)
            .collect(),
        imported_at: sp.imported_at.clone(),
        active: active_id.as_deref() == Some(sp.id.as_str()),
    })
}

#[tauri::command]
fn get_active_preset() -> Option<PresetSummaryDto> {
    let sp = get_preset_store().active()?;
    Some(PresetSummaryDto {
        id: sp.id.clone(),
        name: sp.preset.name.clone(),
        prompt_count: sp.preset.prompts.len(),
        regex_count: sp.preset.regex_scripts.len(),
        imported_at: sp.imported_at.clone(),
        active: true,
    })
}

#[tauri::command]
fn set_active_preset(id: Option<String>) -> Result<(), TauriCommandError> {
    let store = get_preset_store();
    match id {
        Some(id) => {
            if store
                .set_active(&id)
                .map_err(|e| TauriCommandError::storage(format!("storage write failed: {e}")))?
            {
                Ok(())
            } else {
                Err(TauriCommandError::not_found(format!(
                    "preset not found: {id}"
                )))
            }
        }
        None => store
            .clear_active()
            .map_err(|e| TauriCommandError::storage(format!("storage write failed: {e}"))),
    }
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

#[tauri::command]
fn list_global_regex_scripts() -> Vec<RegexScriptDto> {
    get_global_regex_store()
        .list()
        .iter()
        .map(regex_script_dto)
        .collect()
}

#[tauri::command]
fn import_global_regex_settings(settings_json: String) -> Result<usize, TauriCommandError> {
    get_global_regex_store()
        .import_from_settings_json(&settings_json)
        .map_err(|e| TauriCommandError::storage(format!("global regex import failed: {e}")))
}

#[tauri::command]
fn clear_global_regex_scripts() -> Result<(), TauriCommandError> {
    get_global_regex_store()
        .clear()
        .map_err(|e| TauriCommandError::storage(format!("global regex clear failed: {e}")))
}

#[tauri::command]
fn update_global_regex(
    regex_index: usize,
    disabled: Option<bool>,
) -> Result<(), TauriCommandError> {
    if get_global_regex_store()
        .update_regex(regex_index, disabled)
        .map_err(|e| TauriCommandError::storage(format!("global regex update failed: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!(
            "global regex not found at index {regex_index}"
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
    event_subscriptions: Vec<String>,
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
        event_subscriptions: p.manifest.event_subscriptions.clone(),
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
fn plugin_read_world_info(
    plugin_id: String,
    character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInfo, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::ReadWorldInfo)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    get_store()
        .get(&character_id)
        .map(|s| s.info)
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))
}

fn ensure_plugin_any_permission(
    registry: &storyforge_infra_plugin_host::PluginRegistry,
    plugin_id: &str,
    permissions: &[storyforge_infra_plugin_host::Permission],
) -> Result<(), TauriCommandError> {
    let mut last_error = None;
    for permission in permissions {
        match registry.ensure_permission(plugin_id, permission) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }
    Err(TauriCommandError::internal(
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "插件权限不足".to_string()),
    ))
}

#[tauri::command]
fn plugin_get_variable(
    plugin_id: String,
    campaign_id: String,
    instance_id: String,
    _key: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    ensure_plugin_any_permission(
        &state.plugin_registry,
        &plugin_id,
        &[Permission::ReadVariables, Permission::WriteVariables],
    )?;
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
fn export_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    state
        .agent_profile_config_store
        .export_json(&id)
        .map_err(TauriCommandError::from)
}

#[tauri::command]
fn import_agent_profile_config(
    config_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<storyforge_domain::agent_profile_config::AgentProfileConfig, TauriCommandError> {
    state
        .agent_profile_config_store
        .import_json(&config_json)
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
            PipelineEvent::PromptHookRequest {
                request_id,
                role,
                round,
                model,
                messages,
            } => (
                "prompt_hook_request".into(),
                serde_json::json!({
                    "request_id": request_id,
                    "role": role,
                    "round": round,
                    "model": model,
                    "messages": messages,
                }),
            ),
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
#[derive(Debug, Clone, Deserialize)]
struct PromptHookReply {
    #[serde(default)]
    messages: Option<Vec<ChatMessage>>,
    #[serde(default)]
    error: Option<String>,
}

fn resolve_prompt_hook_pending(
    pending: &PromptHookPendingMap,
    request_id: &str,
    messages: Option<Vec<ChatMessage>>,
    error: Option<String>,
) -> bool {
    let sender = pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(request_id);
    if let Some(sender) = sender {
        let _ = sender.send(PromptHookReply { messages, error });
        true
    } else {
        false
    }
}

fn frontend_prompt_hook(
    event_tx: tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    pending: PromptHookPendingMap,
) -> PromptHook {
    Arc::new(move |ctx: PromptHookContext| {
        let event_tx = event_tx.clone();
        let pending = pending.clone();
        Box::pin(async move {
            let request_id = Id::new().as_str().to_string();
            let original_messages = ctx.messages.clone();
            let (tx, rx) = oneshot::channel();
            pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(request_id.clone(), tx);
            let mut pending_guard =
                PromptHookPendingGuard::new(request_id.clone(), pending.clone());

            if event_tx
                .send(PipelineEvent::PromptHookRequest {
                    request_id: request_id.clone(),
                    role: ctx.role,
                    round: ctx.round,
                    model: ctx.model,
                    messages: ctx.messages,
                })
                .is_err()
            {
                return Ok(original_messages);
            }

            match tokio::time::timeout(std::time::Duration::from_secs(8), rx).await {
                Ok(Ok(reply)) => {
                    pending_guard.disarm();
                    if let Some(error) = reply.error {
                        tracing::warn!(
                            "frontend prompt hook returned error for {request_id}: {error}"
                        );
                    }
                    Ok(reply.messages.unwrap_or(original_messages))
                }
                Ok(Err(_)) => {
                    pending_guard.disarm();
                    Ok(original_messages)
                }
                Err(_) => {
                    tracing::warn!("frontend prompt hook timed out for {request_id}");
                    Ok(original_messages)
                }
            }
        })
    })
}

#[tauri::command]
async fn plugin_prompt_hook_result(
    request_id: String,
    messages: Option<Vec<ChatMessage>>,
    error: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if !resolve_prompt_hook_pending(&state.prompt_hook_pending, &request_id, messages, error) {
        tracing::warn!("plugin_prompt_hook_result: unknown request_id {request_id}");
    }
    Ok(())
}

#[tauri::command]
async fn start_writing(
    intent: String,
    character_id: Option<String>,
    conversation_id: Option<String>,
    opening_message: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<WritingEvent>,
) -> Result<serde_json::Value, TauriCommandError> {
    let app = state.inner().clone();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();

    // Phase A 屏障：存在非 terminal Turn → 拒绝启动（在追加 user 消息之前）
    check_turn_barrier(&app)?;

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
    let legacy_opening_character = tool_snapshot.characters.first().cloned();
    let start_target = prepare_start_conversation_async(
        app.clone(),
        get_campaign_store(),
        conversation_id,
        character_id.clone(),
        legacy_opening_character,
        opening_message,
        intent.clone(),
    )
    .await?;
    let conversation_id = start_target.conversation_id;
    let regex_character_id = start_target.regex_character_id;

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
    fill_regex_context(&mut ctx, get_preset_store(), get_global_regex_store());
    // 从模块/Profile 存储加载预设配置
    fill_profile_context(&mut ctx, &app);
    // 从活跃 Agent Profile Config 加载运行时配置覆盖
    fill_agent_profile_context(&mut ctx, &app);
    // 从活跃 Campaign 填充 P2 字段（任务注入导演 / 后处理需要）
    fill_campaign_context_async(&mut ctx, &app).await?;

    // Phase A: Campaign 模式下创建 TurnRecord
    let turn_record = if let Some(campaign_id) = &ctx.campaign_id {
        // 获取当前 Campaign revision 作为 base
        let base_revision = get_campaign_store()
            .get_campaign(campaign_id)
            .map(|c| c.revision)
            .unwrap_or(0);
        let input_node = start_target.input_node_id.unwrap_or_else(|| {
            tracing::warn!("Phase A: user 消息节点 ID 未知，TurnRecord.input_node_id 用 placeholder");
            Id::from_str("unknown-input-node")
        });
        let record = storyforge_domain::turn::TurnRecord::new(
            campaign_id.clone(),
            conversation_id.clone(),
            input_node,
            base_revision,
        );
        if let Err(e) = get_turn_store().create_turn(record.clone()) {
            tracing::error!("Phase A: 创建 TurnRecord 失败: {e}");
            return Err(TauriCommandError::internal(format!("创建 TurnRecord 失败: {e}")));
        }
        Some(record)
    } else {
        None // 非 Campaign 模式，不创建 TurnRecord
    };

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
    let prompt_hook = frontend_prompt_hook(event_tx.clone(), app.prompt_hook_pending.clone());
    let mut pipeline =
        app.new_pipeline_with_regex_and_prompt_hook(&ctx.regex_scripts, Some(prompt_hook));
    let result = pipeline
        .start_writing(intent, &ctx, event_tx.clone(), cancel_rx)
        .await;

    // ─── P2 后处理流水线（后台执行，不阻断成文返回）──────────────────────
    // 成文（DraftReady）后跑：剧情总结 + 后处理三合一。
    // postprocess 放后台 spawn——draft_ready 后立即返回成文给前端，
    // postprocess 在后台跑（知识/变量/摘要写回），通过 event_tx 推进度。
    // 仅在有活跃 Campaign 时执行（无 Campaign 跳过，向后兼容）。
    if let Ok((final_text, draft_node_id, _)) = &result {
        // Phase A: 成文后创建 TurnAttempt 并更新 TurnRecord → DraftReady
        if let Some(ref turn) = turn_record {
            let draft_hash = compute_draft_hash(final_text);
            let attempt = storyforge_domain::turn::TurnAttempt {
                attempt_id: Id::new(),
                variant_id: draft_node_id.clone(),
                draft_hash,
                status: storyforge_domain::turn::AttemptStatus::DraftReady,
                pending_state_changes: None,
                derivation: None,
                provenance: None,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            update_turn_record(&turn.turn_id, |record| {
                record.attempts.push(attempt);
                record.status = storyforge_domain::turn::TurnStatus::DraftReady;
                record.touch();
            });
        }
        // Phase 6：落盘本轮创建的临时 instance（同步，在 postprocess 之前确保知识/变量写回能找到它们）
        persist_temporary_instances_async(&ctx, pipeline.pending_temporary_instances().to_vec())
            .await;

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
        let var_keys = postprocess_variable_keys(&ctx);
        let (pp_cancel_tx, pp_cancel_rx) = watch::channel(false);
        let mvu_fragments =
            collect_mvu_fallback_fragments(&ctx, get_campaign_store(), &present_chars);

        // postprocess 后台跑，不阻塞 start_writing 返回。
        // event_tx 和 pipeline 分别 clone/move 进 spawn 闭包。
        let pp_event_tx = event_tx.clone();
        let pp_turn_id = turn_record.as_ref().map(|t| t.turn_id.clone());
        let pp_attempt_id = turn_record
            .as_ref()
            .and_then(|t| t.active_attempt())
            .map(|a| a.attempt_id.clone());
        tokio::spawn(async move {
            let outcome = pipeline
                .run_postprocess(
                    &final_text,
                    "",
                    &present_chars,
                    &var_keys,
                    &ctx,
                    &pp_event_tx,
                    pp_cancel_rx,
                    &mvu_fragments,
                )
                .await;

            // Phase A: postprocess 产出暂存到 TurnAttempt，不直接写 CampaignStore
            if let (Some(turn_id), Some(attempt_id)) = (pp_turn_id, pp_attempt_id) {
                let derivation = derive_components_from_outcome(&outcome);
                let pc = PostprocessPersistContext::from_writing_context(&ctx);
                let outcome_clone = outcome.clone();
                let batch = match (pc, outcome_clone) {
                    (Some(pc), Some(o)) => {
                        let pc = pc.clone();
                        tokio::task::spawn_blocking(move || {
                            build_mutation_batch(
                                get_campaign_store(),
                                &pc,
                                &o,
                                &present_chars,
                            )
                        })
                        .await
                        .ok()
                    }
                    _ => None,
                };

                update_turn_record(&turn_id, |record| {
                    if let Some(att) = record.find_attempt_mut(&attempt_id) {
                        att.pending_state_changes = batch;
                        att.derivation = Some(derivation);
                        att.status = storyforge_domain::turn::AttemptStatus::AwaitingAcceptance;
                    }
                    record.status = storyforge_domain::turn::TurnStatus::AwaitingAcceptance;
                    record.touch();
                });
            } else {
                // 非 Campaign 路径或无 TurnRecord：保持旧行为（直接写）
                if let Some(outcome) = outcome {
                    persist_postprocess_outcome_async(&ctx, outcome, present_chars).await;
                }
            }
            let _ = pp_cancel_tx;
        });
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
        Err(e) => {
            // Phase A: 写作失败 → TurnRecord 标 Failed（无副作用，安全失败）
            if let Some(ref turn) = turn_record {
                update_turn_record(&turn.turn_id, |record| {
                    record.status = storyforge_domain::turn::TurnStatus::Failed;
                    record.failure_reason = Some(format!("写作失败: {e}"));
                    record.touch();
                });
            }
            Err(TauriCommandError::from(format!("写作失败: {e}")))
        }
    }
}

/// 计算草稿内容的 hash（用于检测编辑后 diff 失效）。
fn compute_draft_hash(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 读取-修改-写回 TurnRecord 的便捷辅助。
fn update_turn_record<F>(turn_id: &Id, f: F)
where
    F: FnOnce(&mut storyforge_domain::turn::TurnRecord),
{
    let store = get_turn_store();
    if let Some(mut record) = store.get_turn(turn_id) {
        f(&mut record);
        if let Err(e) = store.save_turn(record) {
            tracing::error!("Phase A: 保存 TurnRecord 失败: {e}");
        }
    }
}

struct StartConversationTarget {
    conversation_id: Id,
    regex_character_id: Option<String>,
    /// Phase A: 追加的 user 消息节点 ID（TurnRecord.input_node_id 用）
    input_node_id: Option<Id>,
}

async fn prepare_start_conversation_async(
    state: Arc<AppState>,
    campaign_store: &'static campaign_store::CampaignStore,
    requested_conversation_id: Option<String>,
    character_id: Option<String>,
    legacy_opening_character: Option<Arc<storyforge_domain::character::Character>>,
    opening_message: Option<String>,
    intent: String,
) -> Result<StartConversationTarget, TauriCommandError> {
    tokio::task::spawn_blocking(move || {
        prepare_start_conversation(
            state,
            campaign_store,
            requested_conversation_id,
            character_id,
            legacy_opening_character,
            opening_message,
            intent,
        )
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("准备写作对话任务失败: {e}")))
}

fn prepare_start_conversation(
    state: Arc<AppState>,
    campaign_store: &campaign_store::CampaignStore,
    requested_conversation_id: Option<String>,
    character_id: Option<String>,
    legacy_opening_character: Option<Arc<storyforge_domain::character::Character>>,
    opening_message: Option<String>,
    intent: String,
) -> StartConversationTarget {
    let campaign_conv_id: Option<Id> = {
        let active = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        active
            .as_ref()
            .and_then(|cid| campaign_store.get_campaign(cid))
            .and_then(|c| c.conversation_id.clone())
    };
    let conversation_id = campaign_conv_id
        .map(|cid| cid.as_str().to_string())
        .or(requested_conversation_id);

    let (conversation_id, input_node_id) = if let Some(id_str) = conversation_id {
        let id = Id::from_str(&id_str);
        match state.conv_store.append_user_message(&id, intent.clone()) {
            Ok(node_id) => (id, Some(node_id)),
            Err(e) => {
                tracing::warn!("追加 user 消息失败: {e}");
                (id, None)
            }
        }
    } else {
        let conv = state.conv_store.create(character_id.clone(), None);
        let id = conv.id.clone();
        let legacy_opening =
            resolve_legacy_opening_message(legacy_opening_character.as_ref(), opening_message);
        if let Some(opening) = legacy_opening
            && let Err(e) =
                state
                    .conv_store
                    .append_final_message(&id, ConversationRole::Assistant, opening)
        {
            tracing::warn!("追加开场白失败: {e}");
        }
        let node_id = state
            .conv_store
            .append_user_message(&id, intent.clone())
            .map_err(|e| {
                tracing::warn!("追加 user 消息失败: {e}");
            })
            .ok();
        (id, node_id)
    };

    let regex_character_id = character_id.or_else(|| {
        state
            .conv_store
            .get(&conversation_id)
            .and_then(|c| c.character_id)
    });

    StartConversationTarget {
        conversation_id,
        regex_character_id,
        input_node_id,
    }
}

fn resolve_legacy_opening_message(
    character: Option<&Arc<storyforge_domain::character::Character>>,
    requested: Option<String>,
) -> Option<String> {
    let character = character?;
    resolve_opening_message_from_parts(
        &character.first_mes,
        &character.alternate_greetings,
        requested,
        "legacy",
    )
}

fn resolve_campaign_opening_message(
    source_character_id: &Id,
    requested: Option<String>,
) -> Option<String> {
    let stored = stored_character_for_source_id(source_character_id)?;
    resolve_opening_message_from_parts(
        &stored.info.first_mes,
        &stored.info.alternate_greetings,
        requested,
        "campaign",
    )
}

fn stored_character_for_source_id(source_character_id: &Id) -> Option<storage::StoredCharacter> {
    let source = source_character_id.as_str();
    get_store()
        .list()
        .into_iter()
        .find(|sc| sc.info.source_character_id.as_deref() == Some(source) || sc.id == source)
}

fn resolve_opening_message_from_parts(
    first_mes: &str,
    alternate_greetings: &[String],
    requested: Option<String>,
    source_label: &str,
) -> Option<String> {
    let mut choices = Vec::new();
    if !first_mes.trim().is_empty() {
        choices.push(first_mes.to_string());
    }
    choices.extend(
        alternate_greetings
            .iter()
            .filter(|greeting| !greeting.trim().is_empty())
            .cloned(),
    );

    let requested = requested.filter(|message| !message.trim().is_empty());
    if let Some(message) = requested {
        if choices.iter().any(|choice| choice == &message) {
            return Some(message);
        }
        tracing::warn!(
            "Ignoring {source_label} opening_message that is not present on the source character"
        );
    }

    choices.into_iter().next()
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

fn collect_campaign_scoped_regex_scripts(
    campaign_id: &Id,
    store: &campaign_store::CampaignStore,
) -> Vec<RegexScript> {
    store
        .get_campaign(campaign_id)
        .and_then(|campaign| store.get_card(&campaign.card_id))
        .map(|stored_card| stored_card.card.scoped_regex_scripts())
        .unwrap_or_default()
}

fn merge_runtime_regex_scripts(
    scoped_scripts: Vec<RegexScript>,
    preset_store: &PresetStore,
    global_regex_store: &global_regex_store::GlobalRegexStore,
) -> Vec<RegexScript> {
    let global_scripts = global_regex_store.list();
    let preset_scripts = preset_store
        .active()
        .map(|stored| stored.preset.regex_scripts)
        .unwrap_or_default();

    merge_regex_script_sources(&global_scripts, &preset_scripts, &scoped_scripts)
}

fn fill_regex_context(
    ctx: &mut WritingContext,
    preset_store: &PresetStore,
    global_regex_store: &global_regex_store::GlobalRegexStore,
) {
    let scoped_scripts = std::mem::take(&mut ctx.regex_scripts);
    ctx.regex_scripts =
        merge_runtime_regex_scripts(scoped_scripts, preset_store, global_regex_store);
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
#[cfg(test)]
fn fill_campaign_context(ctx: &mut WritingContext, state: &AppState) {
    clear_campaign_runtime(ctx, &state.tool_ctx);

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

async fn fill_campaign_context_async(
    ctx: &mut WritingContext,
    state: &Arc<AppState>,
) -> Result<(), TauriCommandError> {
    clear_campaign_runtime(ctx, &state.tool_ctx);

    let memory_active_id = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };
    let data_dir = state.data_dir.clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        let active_id = memory_active_id.or_else(|| load_active_campaign(&data_dir))?;
        load_campaign_context_snapshot(get_campaign_store(), &active_id)
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("加载 Campaign 快照任务失败: {e}")))?;

    if let Some(snapshot) = snapshot {
        apply_campaign_context_snapshot(ctx, &state.tool_ctx, snapshot);
    }
    Ok(())
}

fn clear_campaign_runtime(ctx: &mut WritingContext, tool_ctx: &Arc<RwLock<ToolContext>>) {
    // 阶段 2 cleanup：先清空旧 runtime，避免 stale 数据残留。
    ctx.campaign_runtime = None;
    let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    tool_guard.campaign_runtime = None;
}

struct CampaignContextSnapshot {
    active_id: Id,
    story_clock: String,
    turn: u32,
    pending_tasks: Vec<storyforge_domain::story_task::StoryTask>,
    scoped_regex_scripts: Vec<RegexScript>,
    runtime: Arc<CampaignRuntimeContext>,
}

fn load_campaign_context_snapshot(
    store: &campaign_store::CampaignStore,
    active_id: &Id,
) -> Option<CampaignContextSnapshot> {
    let camp = store.get_campaign(active_id)?;
    let story_clock = camp.story_clock.clone();
    let existing_turns = store.list_summaries(active_id).len() as u32;
    let turn = existing_turns + 1;
    let tasks = store.list_tasks(active_id);
    let instances = store.list_instances(active_id);
    let knowledge = store.list_knowledge(active_id);

    let stored_card = store.get_card(&camp.card_id);
    let scoped_regex_scripts = stored_card
        .as_ref()
        .map(|stored| stored.card.scoped_regex_scripts())
        .unwrap_or_default();
    let definitions_by_id: std::collections::HashMap<
        Id,
        storyforge_domain::character::CharacterDefinition,
    > = stored_card
        .map(|stored| {
            stored
                .card
                .character_definitions
                .into_iter()
                .map(|def| (def.id.clone(), def))
                .collect()
        })
        .unwrap_or_default();

    let runtime = Arc::new(CampaignRuntimeContext {
        campaign: camp,
        instances,
        definitions_by_id,
        knowledge,
        tasks: tasks.clone(),
        turn,
    });

    Some(CampaignContextSnapshot {
        active_id: active_id.clone(),
        story_clock,
        turn,
        pending_tasks: tasks,
        scoped_regex_scripts,
        runtime,
    })
}

fn apply_campaign_context_snapshot(
    ctx: &mut WritingContext,
    tool_ctx: &Arc<RwLock<ToolContext>>,
    snapshot: CampaignContextSnapshot,
) {
    ctx.campaign_id = Some(snapshot.active_id);
    ctx.story_clock = snapshot.story_clock;
    ctx.turn = snapshot.turn;
    ctx.pending_tasks = snapshot.pending_tasks;
    append_missing_campaign_scoped_regex_scripts(ctx, snapshot.scoped_regex_scripts);
    ctx.campaign_runtime = Some(snapshot.runtime.clone());

    let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    tool_guard.campaign_runtime = Some(snapshot.runtime);
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

    let stored_card = store.get_card(&camp.card_id);
    if let Some(stored_card) = &stored_card {
        append_missing_campaign_scoped_regex_scripts(ctx, stored_card.card.scoped_regex_scripts());
    }

    // 从 card 的 character_definitions 构建 definitions_by_id
    let definitions_by_id: std::collections::HashMap<
        Id,
        storyforge_domain::character::CharacterDefinition,
    > = if let Some(stored_card) = stored_card {
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

fn append_missing_campaign_scoped_regex_scripts(
    ctx: &mut WritingContext,
    scripts: Vec<RegexScript>,
) {
    let mut existing_scoped_ids: std::collections::HashSet<String> = ctx
        .regex_scripts
        .iter()
        .filter(|script| script.source == RegexScriptSource::Scoped)
        .map(|script| script.id.clone())
        .collect();

    for script in scripts {
        if existing_scoped_ids.insert(script.id.clone()) {
            ctx.regex_scripts.push(script);
        }
    }
}

/// 默认变量键列表（喂给后处理 Agent，让它知道有哪些字段可更新）
fn default_variable_keys() -> Vec<String> {
    storyforge_domain::variables::default_character_variables()
        .iter()
        .map(|f| f.key.clone())
        .collect()
}

/// 后处理可更新变量键。
///
/// 基础表提供常用角色/全局变量；CampaignRuntimeContext 提供当前卡自定义 schema、
/// 已存在 Campaign 变量和 instance 变量，覆盖 MVU/initvar 与高玩自定义字段。
fn postprocess_variable_keys(ctx: &WritingContext) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_key = |key: &str| {
        if seen.insert(key.to_string()) {
            keys.push(key.to_string());
        }
    };

    for key in default_variable_keys() {
        push_key(&key);
    }
    for field in storyforge_domain::variables::default_campaign_variables() {
        push_key(&field.key);
    }

    let Some(runtime) = &ctx.campaign_runtime else {
        return keys;
    };

    for variable in &runtime.campaign.variables {
        push_key(&variable.key);
    }

    let mut definitions: Vec<_> = runtime.definitions_by_id.values().collect();
    definitions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    for definition in definitions {
        for field in &definition.variable_schema {
            push_key(&field.key);
        }
    }

    for instance in &runtime.instances {
        for variable in &instance.variables {
            push_key(&variable.key);
        }
    }

    keys
}

#[derive(Debug, Clone)]
struct TemporaryInstancesPersistContext {
    campaign_id: Id,
}

impl TemporaryInstancesPersistContext {
    fn from_writing_context(ctx: &WritingContext) -> Option<Self> {
        Some(Self {
            campaign_id: ctx.campaign_id.clone()?,
        })
    }
}

/// Phase 6：把本轮创建的临时 instance 落盘到 CampaignStore。
///
/// 去重逻辑：同一 campaign 内已存在同名 instance 时跳过。
/// 落盘后，下一轮 `fill_campaign_context_async` 能读到这些 instance。
async fn persist_temporary_instances_async(
    ctx: &WritingContext,
    temporaries: Vec<storyforge_domain::campaign::CharacterInstance>,
) {
    if temporaries.is_empty() {
        return;
    }
    let Some(persist_ctx) = TemporaryInstancesPersistContext::from_writing_context(ctx) else {
        return;
    };

    if let Err(e) = tokio::task::spawn_blocking(move || {
        persist_temporary_instances_to_store(get_campaign_store(), &persist_ctx, &temporaries);
    })
    .await
    {
        tracing::warn!("落盘临时 instance 的阻塞任务失败: {e}");
    }
}

#[cfg(test)]
fn persist_temporary_instances_to(
    store: &campaign_store::CampaignStore,
    ctx: &WritingContext,
    temporaries: &[storyforge_domain::campaign::CharacterInstance],
) {
    let Some(persist_ctx) = TemporaryInstancesPersistContext::from_writing_context(ctx) else {
        return;
    };
    persist_temporary_instances_to_store(store, &persist_ctx, temporaries);
}

fn persist_temporary_instances_to_store(
    store: &campaign_store::CampaignStore,
    persist_ctx: &TemporaryInstancesPersistContext,
    temporaries: &[storyforge_domain::campaign::CharacterInstance],
) {
    if temporaries.is_empty() {
        return;
    }
    let camp_id = &persist_ctx.campaign_id;
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

#[derive(Debug, Clone)]
struct PostprocessPersistContext {
    campaign_id: Id,
    conversation_id: Id,
    turn: u32,
}

impl PostprocessPersistContext {
    fn from_writing_context(ctx: &WritingContext) -> Option<Self> {
        Some(Self {
            campaign_id: ctx.campaign_id.clone()?,
            conversation_id: ctx.conversation_id.clone(),
            turn: ctx.turn,
        })
    }
}

async fn persist_postprocess_outcome_async(
    ctx: &WritingContext,
    outcome: storyforge_app_agent::PostProcessOutcome,
    present_chars: Vec<String>,
) {
    let Some(persist_ctx) = PostprocessPersistContext::from_writing_context(ctx) else {
        return;
    };

    if let Err(e) = tokio::task::spawn_blocking(move || {
        persist_postprocess_outcome_to_store(
            get_campaign_store(),
            &persist_ctx,
            &outcome,
            &present_chars,
        );
    })
    .await
    {
        tracing::warn!("保存后处理结果的阻塞任务失败: {e}");
    }
}

/// Phase A: 从 PostProcessOutcome 推导 DerivationComponents（summary/state 分别追踪）。
fn derive_components_from_outcome(
    outcome: &Option<storyforge_app_agent::PostProcessOutcome>,
) -> storyforge_domain::turn::DerivationComponents {
    use storyforge_domain::turn::{DerivationComponents, DerivationStatus};
    match outcome {
        None => DerivationComponents {
            // 两者都被配置关闭（run_postprocess 返回 None）
            summary_derivation: DerivationStatus::Disabled,
            state_derivation: DerivationStatus::Disabled,
        },
        Some(o) => DerivationComponents {
            summary_derivation: if o.summary.is_some() {
                DerivationStatus::Succeeded
            } else {
                DerivationStatus::Disabled
            },
            state_derivation: if o.post_process.is_some() {
                if o.post_process.as_ref().map(|p| p.parse_succeeded).unwrap_or(false) {
                    DerivationStatus::Succeeded
                } else {
                    DerivationStatus::Failed
                }
            } else {
                DerivationStatus::Disabled
            },
        },
    }
}

/// Phase A: 把后处理产出转换为 MutationBatch（不直接写 CampaignStore）。
///
/// 替代旧的 `persist_postprocess_outcome_to_store` 的写入逻辑，
/// 只构建候选 diff，预分配知识和任务的稳定 ID。
/// accept 时由 CampaignMutationCoordinator::apply_mutation_batch 提交。
fn build_mutation_batch(
    store: &campaign_store::CampaignStore,
    persist_ctx: &PostprocessPersistContext,
    outcome: &storyforge_app_agent::PostProcessOutcome,
    present_chars: &[String],
) -> storyforge_domain::turn::MutationBatch {
    let camp_id = &persist_ctx.campaign_id;
    let commit_id = Id::new();
    let expected_revision = store.get_campaign(camp_id).map(|c| c.revision).unwrap_or(0);
    let mut mutations: Vec<storyforge_domain::turn::Mutation> = vec![];

    // 本轮摘要
    if let Some(summary) = &outcome.summary {
        mutations.push(storyforge_domain::turn::Mutation::UpsertSummary(Box::new(
            storyforge_domain::agent::RoundSummary::new(
                camp_id.clone(),
                persist_ctx.conversation_id.clone(),
                persist_ctx.turn,
                summary.clone(),
            ),
        )));
    }

    // 后处理三合一
    if let Some(pp) = &outcome.post_process {
        let present_ids: std::collections::HashSet<String> =
            present_chars.iter().cloned().collect();

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

        // 知识：broadcast 展开 + 预分配 entry_id（收敛决策步骤 18）
        for u in &pp.knowledge_updates {
            let entries = normalize_knowledge_update_for_postprocess(
                store,
                camp_id,
                u,
                persist_ctx.turn,
                &present_ids,
                &name_collisions,
            );
            for entry in entries {
                let entry_id = entry.id.clone();
                mutations.push(storyforge_domain::turn::Mutation::UpsertKnowledge(
                    Box::new(storyforge_domain::turn::KnowledgeMutation {
                        entry_id,
                        campaign_id: entry.campaign_id.clone(),
                        character_id: entry.character_id.clone(),
                        knowledge_text: entry.knowledge_text.clone(),
                        source: entry.source.clone(),
                        source_character_id: entry.source_character_id.clone(),
                        turn_number: entry.turn_number,
                        event_id: entry.event_id.clone(),
                        pinned: entry.pinned,
                        propagation: entry.propagation.clone(),
                    }),
                ));
            }
        }

        // 变量更新：角色级 / 全局级（绝对值，天然幂等）
        for vu in &pp.variable_updates {
            if let Some(inst_id) = &vu.instance_id {
                if let Some(inst) = find_instance_by_name_or_id(store, camp_id, inst_id) {
                    let is_present = is_postprocess_instance_present(
                        &inst,
                        inst_id,
                        &present_ids,
                        &name_collisions,
                    );
                    if is_present {
                        mutations.push(storyforge_domain::turn::Mutation::SetVariable {
                            instance_id: Some(inst.id.clone()),
                            key: vu.key.clone(),
                            value: vu.value.clone(),
                            turn: persist_ctx.turn,
                        });
                    } else {
                        tracing::warn!(
                            "跳过非在场角色 '{}' 的变量写入（present_chars 校验）",
                            inst.name
                        );
                    }
                }
            } else {
                mutations.push(storyforge_domain::turn::Mutation::SetVariable {
                    instance_id: None,
                    key: vu.key.clone(),
                    value: vu.value.clone(),
                    turn: persist_ctx.turn,
                });
            }
        }

        // 任务更新：已有（绝对状态）/ 新建（预分配 task_id）
        for tu in &pp.task_updates {
            if let Some(tid) = &tu.task_id {
                if let Some(task) = store.get_task(tid)
                    && let Some(task) = normalize_task_update_for_postprocess(
                        camp_id,
                        task,
                        tu.new_status.clone(),
                    )
                {
                    mutations.push(storyforge_domain::turn::Mutation::SetTaskStatus {
                        task_id: task.id.clone(),
                        status: task.status.clone(),
                    });
                }
            } else if let Some(spec) = &tu.new_task {
                let new_task = storyforge_domain::story_task::StoryTask::from_narrative(
                    camp_id.clone(),
                    spec.title.clone(),
                    spec.description.clone(),
                    spec.triggers.clone(),
                    persist_ctx.turn,
                );
                mutations.push(storyforge_domain::turn::Mutation::UpsertNewTask(Box::new(
                    new_task,
                )));
            }
        }
    }

    storyforge_domain::turn::MutationBatch {
        commit_id,
        expected_revision,
        target_revision: expected_revision + 1,
        status: storyforge_domain::turn::MutationBatchStatus::Prepared,
        mutations,
    }
}
fn persist_postprocess_outcome_to_store(
    store: &campaign_store::CampaignStore,
    persist_ctx: &PostprocessPersistContext,
    outcome: &storyforge_app_agent::PostProcessOutcome,
    present_chars: &[String],
) {
    let camp_id = &persist_ctx.campaign_id;

    // 本轮摘要
    if let Some(summary) = &outcome.summary
        && let Err(e) = store.add_summary(storyforge_domain::agent::RoundSummary::new(
            camp_id.clone(),
            persist_ctx.conversation_id.clone(),
            persist_ctx.turn,
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
                    persist_ctx.turn,
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
                        inst.set_variable(&vu.key, vu.value.clone(), persist_ctx.turn);
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
                    camp.set_variable(&vu.key, vu.value.clone(), persist_ctx.turn);
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
                    persist_ctx.turn,
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
    fill_regex_context(&mut ctx, get_preset_store(), get_global_regex_store());
    fill_profile_context(&mut ctx, &app);
    fill_agent_profile_context(&mut ctx, &app);
    fill_campaign_context_async(&mut ctx, &app).await?;

    // cancel channel
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut slot = app.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = slot.take() {
            let _ = existing.send(true);
        }
        *slot = Some(cancel_tx);
    }

    let prompt_hook = frontend_prompt_hook(event_tx.clone(), app.prompt_hook_pending.clone());
    let mut pipeline =
        app.new_pipeline_with_regex_and_prompt_hook(&ctx.regex_scripts, Some(prompt_hook));
    let result = pipeline
        .regenerate(pipeline_req, &ctx, event_tx.clone(), cancel_rx)
        .await;

    // ─── P2 后处理（best-effort，同 start_writing）─────────────────────────
    if let Ok((text, _provenance)) = &result {
        // Phase A: regenerate 创建新 TurnAttempt,旧 Attempt Superseded
        // regenerate 的 replace_active_variant 改变了 node_id 的 active variant,
        // 新 variant 在同一 node 上,用 req 的 node_id 作为 variant_id
        let regen_attempt_id = if let Some(campaign_id) = &ctx.campaign_id {
            if let Some(turn) = get_turn_store().get_active_turn(campaign_id) {
                let new_attempt = storyforge_domain::turn::TurnAttempt {
                    attempt_id: Id::new(),
                    variant_id: node_id.clone(),
                    draft_hash: compute_draft_hash(text),
                    status: storyforge_domain::turn::AttemptStatus::DraftReady,
                    pending_state_changes: None,
                    derivation: None,
                    provenance: None,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                let new_attempt_id = new_attempt.attempt_id.clone();
                update_turn_record(&turn.turn_id, |record| {
                    // 旧活动 Attempt → Superseded
                    for att in &mut record.attempts {
                        if att.status.is_active() {
                            att.status = storyforge_domain::turn::AttemptStatus::Superseded;
                        }
                    }
                    record.attempts.push(new_attempt);
                    record.status = storyforge_domain::turn::TurnStatus::DraftReady;
                    record.touch();
                });
                Some(new_attempt_id)
            } else {
                None
            }
        } else {
            None
        };

        // Phase 6：落盘本轮创建的临时 instance（在 postprocess 之前）
        persist_temporary_instances_async(&ctx, pipeline.pending_temporary_instances().to_vec())
            .await;

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
            postprocess_variable_keys(&ctx),
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

        // Phase A: postprocess 产出暂存到新 TurnAttempt（同 start_writing）
        if let Some(campaign_id) = &ctx.campaign_id
            && let Some(turn) = get_turn_store().get_active_turn(campaign_id)
            && let Some(att_id) = regen_attempt_id
        {
            let derivation = derive_components_from_outcome(&outcome);
            let pc = PostprocessPersistContext::from_writing_context(&ctx);
            let outcome_clone = outcome.clone();
            let batch = match (pc, outcome_clone) {
                (Some(pc), Some(o)) => {
                    let pc = pc.clone();
                    tokio::task::spawn_blocking(move || {
                        build_mutation_batch(get_campaign_store(), &pc, &o, &present_chars)
                    })
                    .await
                    .ok()
                }
                _ => None,
            };
            update_turn_record(&turn.turn_id, |record| {
                if let Some(att) = record.find_attempt_mut(&att_id) {
                    att.pending_state_changes = batch;
                    att.derivation = Some(derivation);
                    att.status = storyforge_domain::turn::AttemptStatus::AwaitingAcceptance;
                }
                record.status = storyforge_domain::turn::TurnStatus::AwaitingAcceptance;
                record.touch();
            });
        } else if let Some(outcome) = outcome {
            // 非 Campaign 路径：保持旧行为（直接写）
            persist_postprocess_outcome_async(&ctx, outcome, present_chars).await;
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
    /// 厂商扩展参数（P3-3），透传到请求体顶层。key=字段名(如 thinking/reasoning_effort),
    /// value=任意 JSON。前端可填如 {"thinking":{"type":"enabled"},"reasoning_effort":"max"}。
    #[serde(default)]
    pub extra: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 创建连接（从模板或自定义）
///
/// 返回新连接的 id。若这是首个连接，自动设为活跃。
#[tauri::command]
async fn create_connection(
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
            extra: req.extra,
        },
        tool_mode,
    };

    // 预先验证：构造 client 看是否成功（base_url 格式等）
    // 注意：不实际发请求，只验证能构造出 client
    storyforge_infra_llm::create_client(&conn)
        .map_err(|e| TauriCommandError::llm(format!("连接配置无效: {e}"), false))?;

    create_connection_with_store_async(state.inner().clone(), get_conn_store(), conn).await
}

/// 删除连接（若为活跃的，同时清除活跃状态）
#[tauri::command]
async fn delete_connection(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    delete_connection_with_store_async(state.inner().clone(), get_conn_store(), id).await
}

/// 设置活跃连接
#[tauri::command]
async fn set_active_connection(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state.inner().clone().set_active_connection_async(id).await
}

async fn set_active_connection_with_store_async(
    state: Arc<AppState>,
    conn_store: Arc<ConnectionStore>,
    id: String,
) -> Result<(), TauriCommandError> {
    let _guard = state.active_connection_update.lock().await;
    let id_for_io = id.clone();
    let conn = tokio::task::spawn_blocking(move || {
        conn_store
            .set_active(&id_for_io)
            .map_err(TauriCommandError::from)?
            .ok_or_else(|| TauriCommandError::not_found(format!("连接不存在: {id_for_io}")))
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("设置活跃连接任务失败: {e}")))??;

    // Keep in-memory active client aligned with the just-persisted active_id.
    state.apply_active_connection(&id, conn)
}

async fn create_connection_with_store_async(
    state: Arc<AppState>,
    conn_store: Arc<ConnectionStore>,
    conn: LlmConnection,
) -> Result<String, TauriCommandError> {
    let _guard = state.active_connection_update.lock().await;
    let conn_id = conn.id.as_str().to_string();
    let id_for_io = conn_id.clone();
    let maybe_active = tokio::task::spawn_blocking(move || {
        let was_empty = conn_store.list().is_empty();
        conn_store
            .save(conn)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
        if was_empty {
            conn_store
                .set_active(&id_for_io)
                .map_err(TauriCommandError::from)?
                .ok_or_else(|| TauriCommandError::not_found(format!("连接不存在: {id_for_io}")))
                .map(Some)
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("创建连接持久化任务失败: {e}")))??;

    if let Some(active_conn) = maybe_active {
        state.apply_active_connection(&conn_id, active_conn)?;
    }

    Ok(conn_id)
}

async fn delete_connection_with_store_async(
    state: Arc<AppState>,
    conn_store: Arc<ConnectionStore>,
    id: String,
) -> Result<(), TauriCommandError> {
    let _guard = state.active_connection_update.lock().await;
    let was_active = state.active_conn_id().as_deref() == Some(id.as_str());
    let id_for_io = id.clone();
    let deleted = tokio::task::spawn_blocking(move || {
        conn_store
            .delete(&id_for_io)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("删除连接持久化任务失败: {e}")))??;

    if !deleted {
        return Err(TauriCommandError::not_found(format!("连接不存在: {id}")));
    }
    if was_active {
        state.clear_active_connection();
    }
    Ok(())
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
    // 联查角色卡名。
    // conversation.character_id 存的是 CharacterCard.id（而非 domain Character 的
    // source_character_id），所以必须用 campaign_store 的卡片表按 card.id 联查,
    // 不能用 tool_ctx.characters（那是扁平 Character,id=source_character_id）。
    // 兜底:character_id 联查不到时,走 campaign_id → campaign.card_id → card.name。
    let store = get_campaign_store();
    let cards = store.list_cards();
    let card_by_id: std::collections::HashMap<&Id, &str> = cards
        .iter()
        .map(|sc| (&sc.card.id, sc.card.name.as_str()))
        .collect();
    state
        .conv_store
        .list()
        .into_iter()
        .map(|c| {
            let card_name = c.character_id.as_ref().and_then(|cid| {
                // 首选:直接按 character_id(=CharacterCard.id)查卡名
                let cid_id = Id::from_str(cid);
                card_by_id.get(&cid_id).map(|n| (*n).to_string())
            }).or_else(|| {
                // 兜底:campaign_id → campaign.card_id → card.name
                c.campaign_id.as_ref().and_then(|camp_id| {
                    store.get_campaign(camp_id).and_then(|campaign| {
                        card_by_id.get(&campaign.card_id).map(|n| (*n).to_string())
                    })
                })
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
    let conversation = state
        .conv_store
        .get(&conv_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("对话不存在: {id}")))?;
    let regex_scripts = collect_conversation_regex_scripts(&conversation, state.inner().as_ref());
    Ok(
        serde_json::to_value(conversation_display_dto(&conversation, &regex_scripts))
            .unwrap_or_default(),
    )
}

#[derive(Debug, Clone, Serialize)]
struct ConversationDisplayDto {
    id: Id,
    character_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    campaign_id: Option<Id>,
    nodes: Vec<MessageNodeDisplayDto>,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
struct MessageNodeDisplayDto {
    id: Id,
    parent_id: Option<Id>,
    variants: Vec<MessageVariantDisplayDto>,
    active_variant: usize,
}

#[derive(Debug, Clone, Serialize)]
struct MessageVariantDisplayDto {
    id: Id,
    role: ConversationRole,
    content: String,
    display_content: String,
    created_at: chrono::DateTime<Utc>,
    status: VariantStatus,
    provenance: Option<Provenance>,
}

fn collect_conversation_regex_scripts(
    conversation: &Conversation,
    state: &AppState,
) -> Vec<RegexScript> {
    let scoped_scripts = if let Some(campaign_id) = &conversation.campaign_id {
        collect_campaign_scoped_regex_scripts(campaign_id, get_campaign_store())
    } else {
        let tool_snapshot = state.snapshot_tool_ctx();
        collect_scoped_regex_scripts(
            conversation.character_id.as_deref(),
            &tool_snapshot.characters,
        )
    };

    merge_runtime_regex_scripts(scoped_scripts, get_preset_store(), get_global_regex_store())
}

fn conversation_display_dto(
    conversation: &Conversation,
    regex_scripts: &[RegexScript],
) -> ConversationDisplayDto {
    let display_scripts = display_only_regex_scripts(regex_scripts);
    ConversationDisplayDto {
        id: conversation.id.clone(),
        character_id: conversation.character_id.clone(),
        campaign_id: conversation.campaign_id.clone(),
        nodes: conversation
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let depth = conversation.nodes.len().saturating_sub(index + 1);
                message_node_display_dto(node, &display_scripts, depth)
            })
            .collect(),
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
    }
}

fn message_node_display_dto(
    node: &MessageNode,
    display_scripts: &[RegexScript],
    depth: usize,
) -> MessageNodeDisplayDto {
    MessageNodeDisplayDto {
        id: node.id.clone(),
        parent_id: node.parent_id.clone(),
        variants: node
            .variants
            .iter()
            .map(|variant| message_variant_display_dto(variant, display_scripts, depth))
            .collect(),
        active_variant: node.active_variant,
    }
}

fn message_variant_display_dto(
    variant: &MessageVariant,
    display_scripts: &[RegexScript],
    depth: usize,
) -> MessageVariantDisplayDto {
    MessageVariantDisplayDto {
        id: variant.id.clone(),
        role: variant.role.clone(),
        content: variant.content.clone(),
        display_content: render_variant_display_content(variant, display_scripts, depth),
        created_at: variant.created_at,
        status: variant.status.clone(),
        provenance: variant.provenance.clone(),
    }
}

fn display_only_regex_scripts(regex_scripts: &[RegexScript]) -> Vec<RegexScript> {
    regex_scripts
        .iter()
        .filter(|script| script.markdown_only.unwrap_or(false))
        .cloned()
        .collect()
}

fn render_variant_display_content(
    variant: &MessageVariant,
    display_scripts: &[RegexScript],
    depth: usize,
) -> String {
    if variant.role != ConversationRole::Assistant || display_scripts.is_empty() {
        return variant.content.clone();
    }

    let reasoning_applied = apply_reasoning_regex_to_think_blocks_at_depth(
        &variant.content,
        display_scripts,
        RegexExecutionTarget::Display,
        depth,
    )
    .and_then(|text| {
        apply_regex_scripts_for_target_at_depth(
            &text,
            display_scripts,
            RegexPlacement::Output,
            RegexExecutionTarget::Display,
            depth,
        )
    });

    reasoning_applied.unwrap_or_else(|e| {
        tracing::warn!("展示正则执行失败，使用原始消息内容: {e}");
        variant.content.clone()
    })
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
    let mut bundle = storyforge_app_logging::export_bundle(&state.log_store, &opts)
        .map_err(TauriCommandError::internal)?;
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
        "active_preset.json",
        "global_regex_scripts.json",
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
    accept_variant_async(state.inner().clone(), conv_id, nid).await
}

async fn accept_variant_async(
    state: Arc<AppState>,
    conv_id: Id,
    node_id: Id,
) -> Result<(), TauriCommandError> {
    // Phase A: Campaign 模式分流
    let active_campaign = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };

    // 自动归档用的 conversation_id（在分支之前 clone，避免 move 问题）
    let archive_conv_id = conv_id.clone();

    if let Some(campaign_id) = active_campaign {
        // Campaign 模式 → TurnCommit
        commit_turn_attempt(&state, &campaign_id, &conv_id, &node_id).await?;
    } else {
        // 非 Campaign 模式 → 保持现有行为
        let conv_store = state.conv_store.clone();
        tokio::task::spawn_blocking(move || conv_store.accept_variant(&conv_id, &node_id))
            .await
            .map_err(|e| TauriCommandError::internal(format!("采纳变体任务失败: {e}")))?
            .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    }

    let state_clone = state.clone();
    tokio::spawn(async move {
        auto_archive_if_needed(&state_clone, &archive_conv_id).await;
    });

    Ok(())
}

/// Phase A: Campaign 模式下的 TurnCommit（accept → 正文 Final + 状态变更 + revision bump）。
async fn commit_turn_attempt(
    state: &Arc<AppState>,
    campaign_id: &Id,
    conv_id: &Id,
    node_id: &Id,
) -> Result<(), TauriCommandError> {
    // 1. 查找包含该变体的 TurnRecord
    let turn = get_turn_store()
        .get_turn_by_variant(node_id)
        .ok_or_else(|| {
            TauriCommandError::validation(
                "该变体没有关联的 TurnRecord，可能是历史草稿。请从此处 fork 或 regenerate。".to_string(),
            )
        })?;

    let attempt = turn.find_attempt_by_variant(node_id).ok_or_else(|| {
        TauriCommandError::validation("该变体没有关联的 TurnAttempt".to_string())
    })?;

    // 2. 校验 attempt 状态
    if !attempt.status.is_active() {
        return Err(TauriCommandError::validation(format!(
            "该 Attempt 状态为 {:?}，不能 accept（只有活动 Attempt 才能 accept）",
            attempt.status
        )));
    }

    // 3. 校验 revision
    let current_revision = get_campaign_store()
        .get_campaign(campaign_id)
        .map(|c| c.revision)
        .ok_or_else(|| TauriCommandError::internal("Campaign 不存在".to_string()))?;

    if turn.base_campaign_revision != current_revision {
        return Err(TauriCommandError::validation(format!(
            "revision 冲突：Turn 基于 revision {}，但当前 Campaign revision 为 {}。该 Turn 已过期。",
            turn.base_campaign_revision, current_revision
        )));
    }

    // 4. 获取候选 MutationBatch（可能为空 — 配置关闭或无推导产出）
    let batch = attempt.pending_state_changes.clone().unwrap_or_else(|| {
        // 空 diff：创建一个只 bump revision 的空 batch
        storyforge_domain::turn::MutationBatch::new(Id::new(), current_revision)
    });

    // 5. CAS Turn → Committing（持久化，在任何副作用之前）
    let turn_id = turn.turn_id.clone();
    let attempt_id = attempt.attempt_id.clone();
    update_turn_record(&turn_id, |record| {
        record.status = storyforge_domain::turn::TurnStatus::Committing;
        if let Some(att) = record.find_attempt_mut(&attempt_id) {
            att.status = storyforge_domain::turn::AttemptStatus::Committing;
        }
        record.touch();
    });

    // 6. 执行 TurnCommit（FinalizeVariant + apply_mutation_batch）
    let conv_store = state.conv_store.clone();
    let conv_id_clone = conv_id.clone();
    let node_id_clone = node_id.clone();
    tokio::task::spawn_blocking(move || {
        // Draft → Final（FinalizeVariant）
        conv_store
            .accept_variant(&conv_id_clone, &node_id_clone)
            .map_err(|e| format!("Draft → Final 失败: {e}"))?;

        // apply_mutation_batch（知识/变量/任务/摘要 + revision bump）
        turn_coordinator::CampaignMutationCoordinator::apply_mutation_batch(
            get_campaign_store(),
            &turn.campaign_id,
            &batch,
        )
        .map_err(|e| e.to_string())?;

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("TurnCommit 任务失败: {e}")))?
    .map_err(|e| TauriCommandError::internal(e))?;

    // 7. 标记 Committed + 其他 attempts Superseded
    update_turn_record(&turn_id, |record| {
        record.status = storyforge_domain::turn::TurnStatus::Committed;
        record.accepted_attempt_id = Some(attempt_id.clone());
        for att in &mut record.attempts {
            if att.attempt_id != attempt_id && att.status.is_active() {
                att.status = storyforge_domain::turn::AttemptStatus::Superseded;
            } else if att.attempt_id == attempt_id {
                att.status = storyforge_domain::turn::AttemptStatus::Committed;
            }
        }
        record.touch();
    });

    Ok(())
}

/// 软删除当前变体（→ Discarded）。
///
/// Phase A: Campaign 模式下同时把对应 TurnAttempt 标 Discarded。
/// Turn 仍开放，允许 regenerate（Discard Attempt ≠ Abandon Turn）。
#[tauri::command]
fn soft_delete_variant(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);

    // Phase A: Campaign 模式下标记 Attempt Discarded
    if let Some(turn) = get_turn_store().get_turn_by_variant(&nid) {
        let attempt_id = turn
            .find_attempt_by_variant(&nid)
            .map(|a| a.attempt_id.clone());
        if let Some(att_id) = attempt_id {
            update_turn_record(&turn.turn_id, |record| {
                if let Some(att) = record.find_attempt_mut(&att_id) {
                    att.status = storyforge_domain::turn::AttemptStatus::Discarded;
                }
                record.touch();
            });
        }
    }

    state
        .conv_store
        .soft_delete_variant(&conv_id, &nid)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// Phase A: 放弃整个 Turn（终止本轮，排除 user 消息和所有 AI 草稿）。
///
/// 与 Discard Attempt 的区别：
/// - Discard Attempt 只丢弃单个 AI 变体，Turn 仍开放。
/// - Abandon Turn 终止整个 Turn，同时把 input user 变体和所有未 accept 的 AI 变体标记 Discarded。
#[tauri::command]
async fn abandon_turn(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);

    let active_campaign = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };

    let Some(campaign_id) = active_campaign else {
        return Err(TauriCommandError::validation(
            "非 Campaign 模式不支持 abandon_turn".to_string(),
        ));
    };

    let turn = get_turn_store().get_active_turn(&campaign_id).ok_or_else(|| {
        TauriCommandError::validation("没有活动 Turn 可以放弃".to_string())
    })?;

    // 把 input user 变体和所有未 accept 的 AI 变体标记 Discarded
    let conv_store = state.conv_store.clone();
    let turn_clone = turn.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        // Discard input user 消息
        conv_store
            .soft_delete_variant(&conv_id, &turn_clone.input_node_id)
            .map_err(|e| e.to_string())?;
        // Discard 所有未 accept 的 AI 变体
        for attempt in &turn_clone.attempts {
            if attempt.status.is_active() {
                conv_store
                    .soft_delete_variant(&conv_id, &attempt.variant_id)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("Abandon Turn 任务失败: {e}")))?
    .map_err(TauriCommandError::internal)?;

    // 标记 Turn Abandoned
    update_turn_record(&turn.turn_id, |record| {
        record.status = storyforge_domain::turn::TurnStatus::Abandoned;
        for att in &mut record.attempts {
            if att.status.is_active() {
                att.status = storyforge_domain::turn::AttemptStatus::Discarded;
            }
        }
        record.touch();
    });

    Ok(())
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
async fn configure_embedder(
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
    configure_embedder_async(state.inner().clone(), config).await
}

async fn configure_embedder_async(
    state: Arc<AppState>,
    config: storyforge_infra_llm::EmbedConfig,
) -> Result<(), TauriCommandError> {
    configure_embedder_with_secret_store_async(
        state,
        config,
        Arc::new(SystemSecretStore::default()) as Arc<dyn SecretStore>,
    )
    .await
}

async fn configure_embedder_with_secret_store_async(
    state: Arc<AppState>,
    config: storyforge_infra_llm::EmbedConfig,
    secret_store: Arc<dyn SecretStore>,
) -> Result<(), TauriCommandError> {
    let data_dir = state.data_dir.clone();
    let config_for_write = config.clone();
    tokio::task::spawn_blocking(move || {
        persist_embed_config_secret_ref(&data_dir, &config_for_write, secret_store.as_ref())
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("嵌入配置持久化任务失败: {e}")))?
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
    let messages = archivable_messages_async(state.conv_store.clone(), conv_id).await?;

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

fn archivable_messages_from_conversation(conv: &Conversation) -> Vec<String> {
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
}

async fn archivable_messages_async(
    conv_store: Arc<ConversationStore>,
    conv_id: Id,
) -> Result<Vec<String>, TauriCommandError> {
    tokio::task::spawn_blocking(move || {
        conv_store
            .get(&conv_id)
            .map(|conv| archivable_messages_from_conversation(&conv))
            .ok_or_else(|| {
                storyforge_app_conversation::ConversationError::NotFound(conv_id.to_string())
            })
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("读取归档消息任务失败: {e}")))?
    .map_err(TauriCommandError::from)
}

// ─── 自动归档辅助 ──────────────────────────────────────────────────────────

/// 检查对话消息数是否超过归档阈值，超过则在后台触发归档
///
/// 阈值：50 条非 Discarded 消息（与 ArchiveConfig.default().threshold 一致）。
/// 归档失败只 warn，不影响用户操作。
async fn auto_archive_if_needed(state: &Arc<AppState>, conv_id: &Id) {
    const ARCHIVE_THRESHOLD: usize = 50;

    // 取对话，数非 Discarded 消息
    let messages = match archivable_messages_async(state.conv_store.clone(), conv_id.clone()).await
    {
        Ok(messages) => messages,
        Err(e) => {
            tracing::debug!("读取自动归档消息失败，跳过: {e}");
            return;
        }
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
                .map(|e| to_json_value(e, "world info entry"))
                .collect::<Result<_, _>>()?;

            let mut patch_ctx = storyforge_app_meta::PatchContext {
                world_info_entries: Some(&mut entries_json),
                character_fields: None,
            };

            storyforge_app_meta::execute_patch(&patch, &mut patch_ctx)
                .map_err(|e| TauriCommandError::internal(e.to_string()))?;

            // 反序列化回 WorldInfoEntry 并替换
            let new_entries: Vec<storyforge_domain::world_info::WorldInfoEntry> = entries_json
                .into_iter()
                .map(|v| {
                    serde_json::from_value(v.clone()).map_err(|e| {
                        TauriCommandError::internal(format!(
                            "Patch entry failed to deserialize as WorldInfoEntry: {e}, value: {v}"
                        ))
                    })
                })
                .collect::<Result<_, _>>()?;

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
        conversation_id: String,
        node_id: String,
    ) -> storyforge_app_meta::meta_conversation::GenerationExplainFuture {
        let conv_store = self.conv_store.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || {
                explain_generation_from_conversation_store(conv_store, conversation_id, node_id)
            })
            .await
            {
                Ok(explanation) => explanation,
                Err(e) => {
                    tracing::warn!("解释生成溯源任务失败: {e}");
                    None
                }
            }
        })
    }
}

fn explain_generation_from_conversation_store(
    conv_store: Arc<ConversationStore>,
    conversation_id: String,
    node_id: String,
) -> Option<storyforge_app_meta::GenerationExplanation> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    let conv = conv_store.get(&conv_id)?;
    let node = conv.find_node(&nid)?;
    let variant = node.active()?;
    let provenance = variant.provenance.as_ref()?;
    Some(storyforge_app_meta::explain_generation(provenance))
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
    let messages = to_json_value(&conv.messages, "meta conversation messages")?;
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
) -> Result<Option<serde_json::Value>, TauriCommandError> {
    let convs = state
        .meta_conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    convs
        .get(&conversation_id)
        .map(|conv| to_json_value(conv, "meta conversation"))
        .transpose()
}

/// Tauri command: 列所有待采纳的 Meta Patch
#[tauri::command]
fn meta_list_pending_patches(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    state
        .meta_patches
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter(|p| !p.applied)
        .map(|p| to_json_value(p, "pending meta patch"))
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

    issues
        .into_iter()
        .map(|i| to_json_value(&i, "campaign health issue"))
        .collect()
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

    to_json_value(&explanation, "generation explanation")
}

// ─── 类型化 Patch 命令（第三轮：campaign-runtime 修复闭环）───────────────────

/// 从 CampaignStore 组装 PreviewInput（类型化 patch 纯函数所需的快照）
fn build_preview_input<'a>(
    _store: &campaign_store::CampaignStore,
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
    meta_propose_campaign_repairs_in_store(store, &campaign_id, state.inner().as_ref())
}

fn meta_propose_campaign_repairs_in_store(
    store: &campaign_store::CampaignStore,
    campaign_id: &str,
    state: &AppState,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let cid = Id::from_str(campaign_id);
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

    let visible_patches = {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        let mut visible = Vec::new();
        for patch in patches {
            if let Some(existing) = typed
                .iter_mut()
                .find(|existing| is_same_pending_typed_patch(existing, &patch))
            {
                existing.description = patch.description.clone();
                existing.diff = patch.diff.clone();
                visible.push(existing.clone());
            } else {
                visible.push(patch.clone());
                typed.push(patch);
            }
        }
        visible
    };

    visible_patches
        .into_iter()
        .map(|p| to_json_value(&p, "typed patch proposal"))
        .collect()
}

fn is_same_pending_typed_patch(
    existing: &storyforge_app_meta::TypedPatch,
    proposed: &storyforge_app_meta::TypedPatch,
) -> bool {
    existing.status == storyforge_app_meta::TypedPatchStatus::Pending
        && existing.source_issue_category == proposed.source_issue_category
        && existing.affected_id == proposed.affected_id
        && canonical_typed_patch_actions_json(&existing.actions)
            == canonical_typed_patch_actions_json(&proposed.actions)
}

fn canonical_typed_patch_actions_json(
    actions: &[storyforge_app_meta::TypedPatchAction],
) -> Option<serde_json::Value> {
    use storyforge_app_meta::TypedPatchAction;

    let mut canonical = actions.to_vec();
    for action in &mut canonical {
        match action {
            TypedPatchAction::SyncInstanceVariables {
                add_keys,
                remove_keys,
                ..
            } => {
                add_keys.sort();
                remove_keys.sort();
            }
            TypedPatchAction::PruneOrphanTaskReferences {
                orphan_character_ids,
                ..
            } => orphan_character_ids.sort_by(|a, b| a.as_str().cmp(b.as_str())),
            _ => {}
        }
    }
    serde_json::to_value(canonical).ok()
}

/// 列出所有 Pending 状态的类型化 patch
#[tauri::command]
fn meta_list_typed_patches(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    typed
        .iter()
        .filter(|p| p.status == storyforge_app_meta::TypedPatchStatus::Pending)
        .map(|p| to_json_value(p, "typed patch"))
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
    meta_preview_typed_patch_in_store(store, &patch_id, &campaign_id, state.inner().as_ref())
}

fn meta_preview_typed_patch_in_store(
    store: &campaign_store::CampaignStore,
    patch_id: &str,
    campaign_id: &str,
    state: &AppState,
) -> Result<serde_json::Value, TauriCommandError> {
    let cid = Id::from_str(campaign_id);
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
        let patch_json = to_json_value(&*patch, "typed patch preview")?;
        return Ok(serde_json::json!({
            "stale": true,
            "patch": patch_json,
        }));
    }

    let patch_json = to_json_value(&*patch, "typed patch preview")?;
    let diff_json = to_json_value(&patch.diff, "typed patch diff")?;
    Ok(serde_json::json!({
        "stale": false,
        "patch": patch_json,
        "diff": diff_json,
    }))
}

/// 接受一条类型化 patch：纯函数预演 → 写盘
#[tauri::command]
fn meta_accept_typed_patch(
    patch_id: String,
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // Phase A 屏障：活动 Turn 存在时拒绝 Meta patch accept（防并发写竞争）
    let cid = Id::from_str(&campaign_id);
    if get_turn_store().get_active_turn(&cid).is_some() {
        return Err(TauriCommandError::validation(
            "当前有未完成的轮次，请先 Accept、Discard 或 Abandon 后再接受 Meta patch".to_string(),
        ));
    }
    let store = get_campaign_store();
    meta_accept_typed_patch_in_store(store, &patch_id, &campaign_id, state.inner().as_ref())
}

fn meta_accept_typed_patch_in_store(
    store: &campaign_store::CampaignStore,
    patch_id: &str,
    campaign_id: &str,
    state: &AppState,
) -> Result<(), TauriCommandError> {
    let _accept_guard = state
        .typed_patch_accept_lock
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let cid = Id::from_str(campaign_id);
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

    if let Err(e) = validate_typed_patch_targets(
        &patch,
        &campaign,
        &definitions,
        &instances,
        &knowledge,
        &tasks,
    ) {
        mark_typed_patch_stale(state, patch_id);
        return Err(e);
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

fn mark_typed_patch_stale(state: &AppState, patch_id: &str) {
    let mut typed = state
        .typed_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(p) = typed.iter_mut().find(|p| p.id == patch_id) {
        p.status = storyforge_app_meta::TypedPatchStatus::Stale;
    }
}

fn validate_typed_patch_targets(
    patch: &storyforge_app_meta::TypedPatch,
    campaign: &storyforge_domain::campaign::Campaign,
    definitions: &[storyforge_domain::character::CharacterDefinition],
    instances: &[storyforge_domain::campaign::CharacterInstance],
    knowledge: &[storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
    tasks: &[storyforge_domain::story_task::StoryTask],
) -> Result<(), TauriCommandError> {
    use storyforge_app_meta::TypedPatchAction;

    for action in &patch.actions {
        match action {
            TypedPatchAction::SyncInstanceVariables {
                instance_id,
                definition_id,
                add_keys,
                ..
            } => {
                let instance = find_instance(instances, instance_id)?;
                if instance.definition_id.as_ref() != Some(definition_id) {
                    return Err(TauriCommandError::validation(format!(
                        "Instance {} 已不再使用 definition {}",
                        instance_id.as_str(),
                        definition_id.as_str()
                    )));
                }
                let definition = definitions
                    .iter()
                    .find(|d| &d.id == definition_id)
                    .ok_or_else(|| {
                        TauriCommandError::not_found(format!(
                            "Definition 不存在: {}",
                            definition_id.as_str()
                        ))
                    })?;
                for key in add_keys {
                    if !definition
                        .variable_schema
                        .iter()
                        .any(|field| field.key == *key)
                    {
                        return Err(TauriCommandError::validation(format!(
                            "Definition {} 缺少变量 schema: {}",
                            definition_id.as_str(),
                            key
                        )));
                    }
                }
            }
            TypedPatchAction::PruneOrphanTaskReferences { task_id, .. }
            | TypedPatchAction::UpdateTaskStatus { task_id, .. } => {
                ensure_task_exists(tasks, task_id)?;
            }
            TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
                if !knowledge.iter().any(|entry| &entry.id == knowledge_id) {
                    return Err(TauriCommandError::not_found(format!(
                        "Knowledge 不存在: {}",
                        knowledge_id.as_str()
                    )));
                }
            }
            TypedPatchAction::RepointInstanceDefinition {
                instance_id,
                new_definition_id,
            } => {
                ensure_instance_exists(instances, instance_id)?;
                if let Some(definition_id) = new_definition_id {
                    ensure_definition_exists(definitions, definition_id)?;
                }
            }
            TypedPatchAction::UpdateCampaignVariable { .. } => {
                if campaign.id.as_str().is_empty() {
                    return Err(TauriCommandError::not_found("Campaign 不存在"));
                }
            }
            TypedPatchAction::UpdateInstanceVariable { instance_id, .. } => {
                ensure_instance_exists(instances, instance_id)?;
            }
            TypedPatchAction::AddKnowledge { character_id, .. } => {
                ensure_instance_exists(instances, character_id)?;
            }
        }
    }
    Ok(())
}

fn ensure_instance_exists(
    instances: &[storyforge_domain::campaign::CharacterInstance],
    instance_id: &Id,
) -> Result<(), TauriCommandError> {
    find_instance(instances, instance_id).map(|_| ())
}

fn find_instance<'a>(
    instances: &'a [storyforge_domain::campaign::CharacterInstance],
    instance_id: &Id,
) -> Result<&'a storyforge_domain::campaign::CharacterInstance, TauriCommandError> {
    instances
        .iter()
        .find(|instance| &instance.id == instance_id)
        .ok_or_else(|| {
            TauriCommandError::not_found(format!("Instance 不存在: {}", instance_id.as_str()))
        })
}

fn ensure_definition_exists(
    definitions: &[storyforge_domain::character::CharacterDefinition],
    definition_id: &Id,
) -> Result<(), TauriCommandError> {
    definitions
        .iter()
        .any(|definition| &definition.id == definition_id)
        .then_some(())
        .ok_or_else(|| {
            TauriCommandError::not_found(format!("Definition 不存在: {}", definition_id.as_str()))
        })
}

fn ensure_task_exists(
    tasks: &[storyforge_domain::story_task::StoryTask],
    task_id: &Id,
) -> Result<(), TauriCommandError> {
    tasks
        .iter()
        .any(|task| &task.id == task_id)
        .then_some(())
        .ok_or_else(|| TauriCommandError::not_found(format!("Task 不存在: {}", task_id.as_str())))
}

/// 执行单个 TypedPatchAction 到 CampaignStore（写盘辅助）
fn apply_typed_action(
    store: &campaign_store::CampaignStore,
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
    meta_dismiss_typed_patch_in_state(&patch_id, state.inner().as_ref())
}

fn meta_dismiss_typed_patch_in_state(
    patch_id: &str,
    state: &AppState,
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
    let complexity_json = to_json_value(&complexity, "MVU complexity")?;
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
    let analyzed_at = chrono::Utc::now().to_rfc3339();
    let stored = campaign_store::StoredMvuTranslation {
        source_character_id: character.id.clone(),
        character_name: character.name.clone(),
        translation: translation.clone(),
        analyzed_at: analyzed_at.clone(),
    };
    save_mvu_translation_async(get_campaign_store(), stored).await?;

    Ok(MvuTranslationDetailDto {
        source_character_id: character.id.as_str().to_string(),
        character_name: character.name.clone(),
        analyzed_at,
        translation,
        complexity: complexity_json,
    })
}

async fn save_mvu_translation_async(
    store: &'static campaign_store::CampaignStore,
    stored: campaign_store::StoredMvuTranslation,
) -> Result<(), TauriCommandError> {
    tokio::task::spawn_blocking(move || save_mvu_translation_to_store(store, stored))
        .await
        .map_err(|e| TauriCommandError::internal(format!("保存 MVU 翻译任务失败: {e}")))?
}

fn save_mvu_translation_to_store(
    store: &campaign_store::CampaignStore,
    stored: campaign_store::StoredMvuTranslation,
) -> Result<(), TauriCommandError> {
    store
        .save_mvu(stored)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
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
    meta_apply_mvu_schema_in_store(store, source_character_id, definition_id)
}

fn meta_apply_mvu_schema_in_store(
    store: &campaign_store::CampaignStore,
    source_character_id: String,
    definition_id: String,
) -> Result<(), TauriCommandError> {
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
    let updated_card = store
        .update_card(card.clone())
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
    if updated_card.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到要更新的 card: {}",
            card.id
        )));
    }

    // Best-effort：对已存在的 instances 补齐合并后 schema 中仍缺失的变量。
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
        let missing_fields: Vec<_> = preview
            .merged_schema
            .iter()
            .filter(|f| !updated.variables.iter().any(|v| v.key == f.key))
            .collect();
        if missing_fields.is_empty() {
            continue;
        }
        for field in missing_fields {
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

    let stored = load_preset_for_classification_async(get_preset_store(), preset_id).await?;

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

async fn load_preset_for_classification_async(
    store: &'static PresetStore,
    preset_id: String,
) -> Result<preset_store::StoredPreset, TauriCommandError> {
    tokio::task::spawn_blocking(move || load_preset_for_classification(store, preset_id))
        .await
        .map_err(|e| TauriCommandError::internal(format!("读取预设任务失败: {e}")))?
}

fn load_preset_for_classification(
    store: &PresetStore,
    preset_id: String,
) -> Result<preset_store::StoredPreset, TauriCommandError> {
    store
        .get(&preset_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("预设不存在: {preset_id}")))
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
    pub first_mes: String,
    pub alternate_greetings: Vec<String>,
    pub character_definitions: Vec<CharacterDefinitionDto>,
    pub definition_count: usize,
    pub character_count: usize,
    pub imported_at: String,
    /// 识别是否成功（false = 未识别/历史状态未知/降级 fallback）
    pub extracted: bool,
    pub extraction_status: String,
    pub extraction_message: Option<String>,
}

/// CharacterCard 列表项（轻量）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardSummaryDto {
    pub id: String,
    pub name: String,
    pub source_character_id: String,
    pub definition_count: usize,
    pub character_count: usize,
    pub imported_at: String,
    pub extracted: bool,
    pub extraction_status: String,
    pub extraction_message: Option<String>,
}

impl From<&campaign_store::StoredCard> for CardSummaryDto {
    fn from(s: &campaign_store::StoredCard) -> Self {
        let definition_count = s.card.character_definitions.len();
        Self {
            id: s.card.id.as_str().to_string(),
            name: s.card.name.clone(),
            source_character_id: s.card.source_character_id.as_str().to_string(),
            definition_count,
            character_count: definition_count,
            imported_at: s.imported_at.clone(),
            extracted: s.card.extraction_succeeded(),
            extraction_status: s.card.extraction_status.as_str().to_string(),
            extraction_message: card_extraction_message(&s.card),
        }
    }
}

const FALLBACK_EXTRACTION_MESSAGE: &str = "识别失败，已按单角色处理，可重新识别。";

fn card_extraction_message(card: &storyforge_domain::character::CharacterCard) -> Option<String> {
    use storyforge_domain::character::CharacterExtractionStatus;

    match card.extraction_status {
        CharacterExtractionStatus::Extracted => card.extraction_message.clone(),
        CharacterExtractionStatus::Fallback => Some(
            card.extraction_message
                .clone()
                .unwrap_or_else(|| FALLBACK_EXTRACTION_MESSAGE.to_string()),
        ),
        CharacterExtractionStatus::Unknown if card.character_definitions.is_empty() => {
            Some("尚未识别角色。".into())
        }
        CharacterExtractionStatus::Unknown => {
            Some("历史角色定义缺少识别状态，可重新识别确认。".into())
        }
    }
}

#[derive(Debug)]
enum CharacterExtractionDecision {
    ReturnExisting(campaign_store::StoredCard),
    Run(storyforge_domain::character::CharacterCard),
}

fn prepare_character_extraction_card(
    store: &campaign_store::CampaignStore,
    character: &storyforge_domain::character::Character,
    force: bool,
) -> Result<CharacterExtractionDecision, TauriCommandError> {
    match store.get_card_by_source(&character.id) {
        Some(existing) if !force => Ok(CharacterExtractionDecision::ReturnExisting(existing)),
        Some(existing) => {
            if !store.list_campaigns_of_card(&existing.card.id).is_empty() {
                return Err(TauriCommandError::validation(
                    campaign_store::FORCE_RERUN_BLOCKED_BY_CAMPAIGN,
                ));
            }
            Ok(CharacterExtractionDecision::Run(existing.card))
        }
        None => Ok(CharacterExtractionDecision::Run(
            storyforge_domain::character::CharacterCard::from_character(character),
        )),
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
    force: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardSummaryDto, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;
    use storyforge_domain::character::{CharacterDefinition, CharacterExtractionStatus};
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
    let force = force.unwrap_or(false);
    let mut card = match prepare_character_extraction_card(store, &character, force)? {
        CharacterExtractionDecision::ReturnExisting(existing) => {
            return Ok(CardSummaryDto::from(&existing));
        }
        CharacterExtractionDecision::Run(card) => card,
    };

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

    let (definitions, extraction_status, extraction_message) = match definitions_result {
        Ok(defs) => (defs, CharacterExtractionStatus::Extracted, None),
        Err(e) => {
            tracing::warn!("角色识别失败，降级建单角色: {e}");
            (
                vec![CharacterDefinition::fallback_from_character(
                    &character,
                    &mvu_schema,
                )],
                CharacterExtractionStatus::Fallback,
                Some(FALLBACK_EXTRACTION_MESSAGE.to_string()),
            )
        }
    };

    // 建卡 + 回填 card_id
    let definitions = storyforge_app_agent::attach_definitions_to_card(definitions, &card.id);
    card.character_definitions = definitions;
    card.extraction_status = extraction_status;
    card.extraction_message = extraction_message;
    let stored = if force {
        save_character_card_force_rerun_async(store, card).await?
    } else {
        save_character_card_async(store, card).await?
    };

    Ok(CardSummaryDto::from(&stored))
}

async fn save_character_card_async(
    store: &'static campaign_store::CampaignStore,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    tokio::task::spawn_blocking(move || save_character_card_to_store(store, card))
        .await
        .map_err(|e| TauriCommandError::internal(format!("保存角色卡任务失败: {e}")))?
}

fn save_character_card_to_store(
    store: &campaign_store::CampaignStore,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    store
        .save_card(card)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
}

async fn save_character_card_force_rerun_async(
    store: &'static campaign_store::CampaignStore,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    tokio::task::spawn_blocking(move || save_character_card_force_rerun_to_store(store, card))
        .await
        .map_err(|e| TauriCommandError::internal(format!("保存角色卡任务失败: {e}")))?
}

fn save_character_card_force_rerun_to_store(
    store: &campaign_store::CampaignStore,
    card: storyforge_domain::character::CharacterCard,
) -> Result<campaign_store::StoredCard, TauriCommandError> {
    store.save_card_if_no_campaigns(card).map_err(|e| {
        if e == campaign_store::FORCE_RERUN_BLOCKED_BY_CAMPAIGN {
            TauriCommandError::validation(e)
        } else {
            TauriCommandError::storage(format!("存储写入失败: {e}"))
        }
    })
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
    let source_character = stored_character_for_source_id(&stored.card.source_character_id);
    let (raw_first_mes, raw_alternate_greetings) = raw_card_greetings(&stored.card.raw_card_json);
    let definition_count = stored.card.character_definitions.len();
    Ok(CardDetailDto {
        id: stored.card.id.as_str().to_string(),
        name: stored.card.name.clone(),
        source_character_id: stored.card.source_character_id.as_str().to_string(),
        first_mes: source_character
            .as_ref()
            .map(|sc| sc.info.first_mes.clone())
            .unwrap_or(raw_first_mes),
        alternate_greetings: source_character
            .as_ref()
            .map(|sc| sc.info.alternate_greetings.clone())
            .unwrap_or(raw_alternate_greetings),
        character_definitions: stored
            .card
            .character_definitions
            .iter()
            .map(CharacterDefinitionDto::from)
            .collect(),
        definition_count,
        character_count: definition_count,
        imported_at: stored.imported_at.clone(),
        extracted: stored.card.extraction_succeeded(),
        extraction_status: stored.card.extraction_status.as_str().to_string(),
        extraction_message: card_extraction_message(&stored.card),
    })
}

fn raw_card_greetings(raw_card_json: &serde_json::Value) -> (String, Vec<String>) {
    let first_mes = raw_card_json
        .get("first_mes")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let alternate_greetings = raw_card_json
        .get("alternate_greetings")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    (first_mes, alternate_greetings)
}

/// 开档：建 Campaign，把卡里所有 Protagonist/Supporting 定义实例化；
/// 一 Campaign 一对话模型：同时自动建对话、存开场白、双向绑定 conversation_id
#[tauri::command]
fn create_campaign(
    card_id: String,
    name: String,
    opening_message: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    create_campaign_in_store(
        get_campaign_store(),
        state.conv_store.as_ref(),
        card_id,
        name,
        opening_message,
    )
}

fn create_campaign_in_store(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
    card_id: String,
    name: String,
    opening_message: Option<String>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    use storyforge_domain::conversation::Role as ConvRole;

    let card_id_value = Id::from_str(&card_id);
    if store.get_card(&card_id_value).is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 card id={card_id}"
        )));
    }

    let mut campaign = storyforge_domain::campaign::Campaign::new(card_id_value, name);

    // 自动建对话并绑定到 Campaign
    let conv = conv_store.create(Some(card_id.clone()), Some(campaign.id.clone()));
    campaign.conversation_id = Some(conv.id.clone());
    let (stored, campaign, instance_count) = match store.create_campaign_with_instances(campaign) {
        Ok(result) => result,
        Err(e) => {
            if let Err(delete_err) = conv_store.delete(&conv.id) {
                tracing::warn!("创建 Campaign 失败后清理对话失败: {delete_err}");
            }
            return Err(TauriCommandError::storage(format!("存储写入失败: {e}")));
        }
    };

    // 存开场白（从 CharacterStore 按 source_character_id 查扁平 Character greeting）
    if let Some(opening) =
        resolve_campaign_opening_message(&stored.card.source_character_id, opening_message)
        && let Err(e) = conv_store.append_final_message(&conv.id, ConvRole::Assistant, opening)
    {
        tracing::warn!("建 Campaign 时追加开场白失败: {e}");
    }

    let mut dto = CampaignSummaryDto::from(&campaign);
    dto.instance_count = instance_count;
    Ok(dto)
}

fn fork_campaign_in_store(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
    source_campaign_id: Id,
    fork_node_id: Id,
    name: String,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(TauriCommandError::validation("fork campaign name is empty"));
    }

    let source = store.get_campaign(&source_campaign_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("campaign not found: {source_campaign_id}"))
    })?;
    if store.get_card(&source.card_id).is_none() {
        return Err(TauriCommandError::not_found(format!(
            "card not found: {}",
            source.card_id
        )));
    }

    let source_conversation_id = source
        .conversation_id
        .clone()
        .or_else(|| conv_store.find_by_campaign(&source.id).map(|c| c.id))
        .ok_or_else(|| {
            TauriCommandError::not_found(format!(
                "source campaign has no conversation: {}",
                source.id
            ))
        })?;

    let mut campaign = storyforge_domain::campaign::Campaign::fork(
        source.card_id.clone(),
        name.to_string(),
        source.id.clone(),
        fork_node_id.clone(),
    );
    campaign.variables = source.variables.clone();
    campaign.story_clock = source.story_clock.clone();

    let forked_conversation =
        conv_store.fork_at(&source_conversation_id, campaign.id.clone(), &fork_node_id)?;
    campaign.conversation_id = Some(forked_conversation.id);

    store
        .save_campaign(campaign.clone())
        .map_err(|e| TauriCommandError::storage(format!("save fork campaign failed: {e}")))?;

    let mut instance_count = 0;
    for mut instance in store.list_instances(&source.id) {
        instance.id = Id::new();
        instance.campaign_id = campaign.id.clone();
        store
            .add_instance(instance)
            .map_err(|e| TauriCommandError::storage(format!("copy fork instance failed: {e}")))?;
        instance_count += 1;
    }

    let mut dto = CampaignSummaryDto::from(&campaign);
    dto.instance_count = instance_count;
    Ok(dto)
}

#[tauri::command]
fn fork_campaign(
    source_campaign_id: String,
    fork_node_id: String,
    name: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignSummaryDto, TauriCommandError> {
    let source_cid = Id::from_str(&source_campaign_id);

    // Phase A: fork 限制——不允许从有活动 Turn 的 Campaign fork（收敛决策盲区 2）
    if get_turn_store().get_active_turn(&source_cid).is_some() {
        return Err(TauriCommandError::validation(
            "源 Campaign 有未完成的 Turn，请先 Accept、Discard 或 Abandon 后再 fork（阶段 A 只支持从已提交 head fork）".to_string(),
        ));
    }

    fork_campaign_in_store(
        get_campaign_store(),
        &state.conv_store,
        source_cid,
        Id::from_str(&fork_node_id),
        name,
    )
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
const BUNDLE_FORMAT_VERSION: u32 = 2;

/// StoryForge Campaign 完整 JSON Bundle
#[derive(Debug, Serialize, Deserialize)]
struct CampaignBundle {
    format_version: u32,
    exported_at: String,
    /// v2 起保留完整 CharacterCard，便于跨设备导入后继续开新档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    card: Option<storyforge_domain::character::CharacterCard>,
    campaign: storyforge_domain::campaign::Campaign,
    instances: Vec<storyforge_domain::campaign::CharacterInstance>,
    /// key = definition_id
    definitions: Vec<storyforge_domain::character::CharacterDefinition>,
    knowledge: Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>,
    tasks: Vec<storyforge_domain::story_task::StoryTask>,
    summaries: Vec<storyforge_domain::agent::RoundSummary>,
}

/// StoryForge Campaign Bundle 导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignImportResult {
    pub campaign_id: String,
    pub card_id: String,
    pub conversation_id: String,
    pub instance_count: usize,
    pub knowledge_count: usize,
    pub task_count: usize,
    pub summary_count: usize,
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
    let camp_id = Id::from_str(&campaign_id);
    export_campaign_bundle_from_store(get_campaign_store(), camp_id)
}

fn export_campaign_bundle_from_store(
    store: &campaign_store::CampaignStore,
    camp_id: Id,
) -> Result<String, TauriCommandError> {
    let campaign = store.get_campaign(&camp_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("Campaign 不存在: {}", camp_id.as_str()))
    })?;

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
        card: stored_card.as_ref().map(|c| c.card.clone()),
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

/// 导入 StoryForge Campaign JSON Bundle。
///
/// 导入始终生成全新 card/campaign/instance/knowledge/task/summary ID，避免覆盖现有数据。
#[tauri::command]
fn import_campaign_bundle(
    bundle_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignImportResult, TauriCommandError> {
    let bundle: CampaignBundle = serde_json::from_str(&bundle_json)
        .map_err(|e| TauriCommandError::validation(format!("Bundle JSON 解析失败: {e}")))?;
    import_campaign_bundle_into_store(get_campaign_store(), state.conv_store.as_ref(), bundle)
}

fn import_campaign_bundle_into_store(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
    bundle: CampaignBundle,
) -> Result<CampaignImportResult, TauriCommandError> {
    use std::collections::HashMap;

    if bundle.format_version == 0 || bundle.format_version > BUNDLE_FORMAT_VERSION {
        return Err(TauriCommandError::validation(format!(
            "不支持的 Campaign Bundle 版本: {}",
            bundle.format_version
        )));
    }

    let old_campaign_id = bundle.campaign.id.clone();
    let new_card_id = Id::new();
    let new_campaign_id = Id::new();
    let new_source_character_id = Id::new();

    let mut definition_id_map: HashMap<Id, Id> = HashMap::new();
    let mut card = bundle
        .card
        .unwrap_or_else(|| storyforge_domain::character::CharacterCard {
            id: bundle.campaign.card_id.clone(),
            name: bundle.campaign.name.clone(),
            source_character_id: new_source_character_id.clone(),
            character_definitions: bundle.definitions.clone(),
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Unknown,
            extraction_message: None,
        });
    let definitions = if card.character_definitions.is_empty() {
        bundle.definitions
    } else {
        card.character_definitions
    };
    card.id = new_card_id.clone();
    card.source_character_id = new_source_character_id;
    if card.name.trim().is_empty() {
        card.name = bundle.campaign.name.clone();
    }
    card.character_definitions = definitions
        .into_iter()
        .map(|mut def| {
            let new_id = Id::new();
            definition_id_map.insert(def.id.clone(), new_id.clone());
            def.id = new_id;
            def.card_id = new_card_id.clone();
            def
        })
        .collect();

    store
        .save_card(card)
        .map_err(|e| TauriCommandError::storage(format!("导入角色卡失败: {e}")))?;

    let mut campaign = bundle.campaign;
    campaign.id = new_campaign_id.clone();
    campaign.card_id = new_card_id.clone();
    campaign.fork_from = None;
    let conversation = conv_store.create(
        Some(new_card_id.as_str().to_string()),
        Some(campaign.id.clone()),
    );
    campaign.conversation_id = Some(conversation.id.clone());
    store
        .save_campaign(campaign)
        .map_err(|e| TauriCommandError::storage(format!("导入 Campaign 失败: {e}")))?;

    let mut instance_id_map: HashMap<Id, Id> = HashMap::new();
    let mut instance_count = 0;
    for instance in &bundle.instances {
        instance_id_map.insert(instance.id.clone(), Id::new());
    }
    for mut instance in bundle.instances {
        let Some(new_instance_id) = instance_id_map.get(&instance.id).cloned() else {
            continue;
        };
        instance.id = new_instance_id;
        instance.campaign_id = new_campaign_id.clone();
        instance.definition_id = instance
            .definition_id
            .and_then(|id| definition_id_map.get(&id).cloned());
        store
            .add_instance(instance)
            .map_err(|e| TauriCommandError::storage(format!("导入角色实例失败: {e}")))?;
        instance_count += 1;
    }

    let mut knowledge_id_map: HashMap<Id, Id> = HashMap::new();
    for entry in &bundle.knowledge {
        knowledge_id_map.insert(entry.id.clone(), Id::new());
    }
    let mut imported_knowledge = Vec::new();
    for mut entry in bundle.knowledge {
        let Some(new_character_id) = instance_id_map.get(&entry.character_id).cloned() else {
            continue;
        };
        let Some(new_entry_id) = knowledge_id_map.get(&entry.id).cloned() else {
            continue;
        };
        entry.id = new_entry_id;
        entry.campaign_id = new_campaign_id.clone();
        entry.character_id = new_character_id;
        entry.source_character_id = entry
            .source_character_id
            .and_then(|id| instance_id_map.get(&id).cloned());
        entry.source_knowledge_id = entry
            .source_knowledge_id
            .and_then(|id| knowledge_id_map.get(&id).cloned());
        imported_knowledge.push(entry);
    }
    let knowledge_count = imported_knowledge.len();
    store
        .add_knowledge(imported_knowledge)
        .map_err(|e| TauriCommandError::storage(format!("导入知识失败: {e}")))?;

    let mut task_count = 0;
    for mut task in bundle.tasks {
        task.id = Id::new();
        task.campaign_id = new_campaign_id.clone();
        task.related_characters = task
            .related_characters
            .into_iter()
            .filter_map(|id| instance_id_map.get(&id).cloned())
            .collect();
        store
            .add_task(task)
            .map_err(|e| TauriCommandError::storage(format!("导入任务失败: {e}")))?;
        task_count += 1;
    }

    let mut summary_count = 0;
    for mut summary in bundle.summaries {
        summary.id = Id::new();
        summary.campaign_id = new_campaign_id.clone();
        summary.conversation_id = conversation.id.clone();
        store
            .add_summary(summary)
            .map_err(|e| TauriCommandError::storage(format!("导入摘要失败: {e}")))?;
        summary_count += 1;
    }

    tracing::info!(
        "Imported Campaign Bundle {} -> {}",
        old_campaign_id,
        new_campaign_id
    );

    Ok(CampaignImportResult {
        campaign_id: new_campaign_id.as_str().to_string(),
        card_id: new_card_id.as_str().to_string(),
        conversation_id: conversation.id.as_str().to_string(),
        instance_count,
        knowledge_count,
        task_count,
        summary_count,
    })
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
        extra: Default::default(),
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
            get_active_preset,
            set_active_preset,
            delete_preset,
            update_preset_prompt,
            update_preset_regex,
            list_global_regex_scripts,
            import_global_regex_settings,
            clear_global_regex_scripts,
            update_global_regex,
            import_preset_as_modules,
            // M4 插件命令
            list_plugins,
            install_plugin,
            uninstall_plugin,
            set_plugin_enabled,
            plugin_list_characters,
            plugin_read_character,
            plugin_read_world_info,
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
            export_agent_profile_config,
            import_agent_profile_config,
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
            plugin_prompt_hook_result,
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
            abandon_turn,
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
            fork_campaign,
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
            import_campaign_bundle,
            // W8 MVU JS Runtime 命令
            mvu_unload_ack,
            mvu_load_ack,
            mvu_execute_result,
        ])
        .run(tauri::generate_context!())
        .expect("StoryForge 启动失败");
}

// ─── 启动恢复辅助：StoredCharacter → domain Character / WorldInfoBook ──────────

/// 从存储的 CharacterInfo 构造 domain Character。
///
/// 新版 CharacterInfo 会持久化 ST round-trip 所需字段；旧数据缺失时仍按展示 DTO
/// 中的 world_info_entries 做近似恢复。
fn stored_info_to_character(
    stored: &storage::StoredCharacter,
) -> storyforge_domain::character::Character {
    let embedded_world_info = stored
        .info
        .embedded_world_info
        .clone()
        .or_else(|| world_info_book_from_entries(&stored.info.world_info_entries));

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
        mes_example: stored.info.mes_example.clone(),
        system_prompt: stored.info.system_prompt.clone(),
        post_history_instructions: stored.info.post_history_instructions.clone(),
        tags: stored.info.tags.clone(),
        creator: stored.info.creator.clone(),
        character_version: stored.info.character_version.clone(),
        alternate_greetings: stored.info.alternate_greetings.clone(),
        embedded_world_info,
        extensions: stored.info.extensions.clone(),
        renderable_assets: stored.info.renderable_assets.clone(),
        source: storyforge_domain::Source::Native,
        spec_version: stored.info.spec_version.clone(),
        raw_card_json: stored.info.raw_card_json.clone(),
    }
}

fn world_info_entry_from_info(
    e: &WorldInfoEntryInfo,
) -> storyforge_domain::world_info::WorldInfoEntry {
    use storyforge_domain::world_info::{LoreRoute, WorldInfoEntry};

    let route = match e.route.as_str() {
        "Constant" => LoreRoute::Constant,
        "Selective" => LoreRoute::Selective,
        "Both" => LoreRoute::Both,
        "Disabled" => LoreRoute::Disabled,
        _ => LoreRoute::Selective,
    };

    WorldInfoEntry {
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
    }
}

fn world_info_book_from_entries(
    entries: &[WorldInfoEntryInfo],
) -> Option<storyforge_domain::world_info::WorldInfoBook> {
    if entries.is_empty() {
        return None;
    }

    Some(storyforge_domain::world_info::WorldInfoBook {
        entries: entries.iter().map(world_info_entry_from_info).collect(),
        source: storyforge_domain::Source::Native,
    })
}

/// 收集世界书条目：当前活跃角色的全部 + 其他角色的 is_global 条目
///
/// `active_name` = 当前激活的角色卡名（来自 tool_ctx.characters 的最后一个）。
/// 返回的 WorldInfoBook 包含所有应生效的条目（含全局共享的）。
fn collect_world_info_for_active(
    all_chars: &[storage::StoredCharacter],
    active_name: &str,
) -> storyforge_domain::world_info::WorldInfoBook {
    use storyforge_domain::world_info::WorldInfoBook;

    let mut entries = Vec::new();
    for stored in all_chars {
        let is_active = stored.info.name == active_name;
        for e in &stored.info.world_info_entries {
            if !is_active && !e.is_global {
                continue;
            }
            entries.push(world_info_entry_from_info(e));
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
    use serde::ser::Error as _;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use storyforge_domain::character::Character;
    use storyforge_domain::preset::{ST_REGEX_PLACEMENT_AI_OUTPUT, ST_REGEX_PLACEMENT_REASONING};

    struct RecordingMockLlm {
        responses: Mutex<std::collections::VecDeque<storyforge_domain::llm::ChatResponse>>,
        requests: Mutex<Vec<storyforge_domain::llm::ChatRequest>>,
    }

    impl RecordingMockLlm {
        fn new(responses: Vec<storyforge_domain::llm::ChatResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<storyforge_domain::llm::ChatRequest> {
            self.requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }

        fn clear_requests(&self) {
            self.requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear();
        }

        fn next_response(&self) -> storyforge_domain::llm::ChatResponse {
            self.responses
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pop_front()
                .unwrap_or_else(|| storyforge_domain::llm::ChatResponse {
                    content: "recording mock fallback response".into(),
                    tool_calls: vec![],
                    finish_reason: Some("stop".into()),
                    usage: None,
                })
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for RecordingMockLlm {
        async fn chat(
            &self,
            req: &storyforge_domain::llm::ChatRequest,
        ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError>
        {
            self.requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(req.clone());
            Ok(self.next_response())
        }

        async fn chat_stream(
            &self,
            req: &storyforge_domain::llm::ChatRequest,
            tx: tokio::sync::mpsc::UnboundedSender<storyforge_domain::llm::StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError>
        {
            self.requests
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(req.clone());
            let response = self.next_response();
            let _ = tx.send(storyforge_domain::llm::StreamChunk {
                delta_content: Some(response.content.clone()),
                delta_tool_calls: if response.tool_calls.is_empty() {
                    None
                } else {
                    Some(response.tool_calls.clone())
                },
                finish_reason: Some("stop".into()),
            });
            Ok(response)
        }
    }

    fn mock_chat_response(content: impl Into<String>) -> storyforge_domain::llm::ChatResponse {
        storyforge_domain::llm::ChatResponse {
            content: content.into(),
            tool_calls: vec![],
            finish_reason: Some("stop".into()),
            usage: None,
        }
    }

    fn command_prompt_hook_channel(
        state: Arc<AppState>,
        marker: &'static str,
    ) -> tauri::ipc::Channel<WritingEvent> {
        tauri::ipc::Channel::new(move |body| {
            let event = body.deserialize::<WritingEvent>()?;
            if event.event_type == "prompt_hook_request" {
                let request_id = event
                    .data
                    .get("request_id")
                    .and_then(|value| value.as_str())
                    .expect("prompt hook request should include request_id");
                let mut messages: Vec<ChatMessage> =
                    serde_json::from_value(event.data["messages"].clone())
                        .expect("prompt hook request should include messages");
                messages.push(ChatMessage::user(marker));
                assert!(
                    resolve_prompt_hook_pending(
                        &state.prompt_hook_pending,
                        request_id,
                        Some(messages),
                        None,
                    ),
                    "pending prompt hook sender should be registered"
                );
            }
            Ok(())
        })
    }

    fn state_with_recording_llm(llm: Arc<RecordingMockLlm>) -> Arc<AppState> {
        let mut state = AppState::new_for_test();
        state.mock_llm = llm as Arc<dyn LlmClient>;
        let state = Arc::new(state);
        {
            let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
            ctx.characters
                .push(Arc::new(make_test_character("Seraphina")));
        }
        state
    }

    fn tauri_state_for_test(state: &Arc<AppState>) -> tauri::State<'_, Arc<AppState>> {
        // Tauri State has no public constructor; command-level tests need the
        // same wrapper type that invoke would provide around managed Arc state.
        unsafe { std::mem::transmute::<&Arc<AppState>, tauri::State<'_, Arc<AppState>>>(state) }
    }

    fn plan_response_json() -> String {
        serde_json::json!({
            "scene_brief": "A compact command prompt hook test scene.",
            "subagent_tasks": [
                {
                    "character_id": "Seraphina",
                    "brief": "Perform a short beat.",
                    "context_package": {
                        "character_brief": "Seraphina, concise test character.",
                        "scene_brief": "A compact command prompt hook test scene.",
                        "relevant_lore": [],
                        "constant_lore": [],
                        "recent_window": [],
                        "task": "Perform a short beat."
                    }
                }
            ]
        })
        .to_string()
    }

    fn any_recorded_request_contains_marker(llm: &RecordingMockLlm, marker: &str) -> bool {
        llm.requests()
            .iter()
            .any(|req| req.messages.iter().any(|message| message.content == marker))
    }

    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(S::Error::custom("forced serialization failure"))
        }
    }

    #[test]
    fn test_to_json_value_returns_structured_error_on_serialization_failure() {
        let err = to_json_value(&FailingSerialize, "failing dto").unwrap_err();

        match err {
            TauriCommandError::Internal { message } => {
                assert!(message.contains("failing dto"));
                assert!(message.contains("forced serialization failure"));
            }
            other => panic!("expected internal serialization error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn frontend_prompt_hook_round_trips_messages_through_pending_reply() {
        use storyforge_app_agent::runtime::PromptHookContext;
        use storyforge_domain::agent::AgentRole;
        use storyforge_domain::llm::ChatMessage;

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
        let pending: PromptHookPendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let hook = frontend_prompt_hook(event_tx, pending.clone());
        let original = vec![ChatMessage::user("before hook")];

        let hook_task = tokio::spawn(hook(PromptHookContext {
            role: AgentRole::Editor,
            round: 1,
            model: "test-model".into(),
            messages: original.clone(),
        }));

        let event = event_rx.recv().await.expect("hook should emit request");
        let request_id = match event {
            PipelineEvent::PromptHookRequest {
                request_id,
                role,
                round,
                model,
                messages,
            } => {
                assert_eq!(role, AgentRole::Editor);
                assert_eq!(round, 1);
                assert_eq!(model, "test-model");
                assert_eq!(messages[0].content, "before hook");
                request_id
            }
            other => panic!("expected prompt hook request, got {other:?}"),
        };

        let sender = pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&request_id)
            .expect("pending reply sender should be registered");
        sender
            .send(PromptHookReply {
                messages: Some(vec![
                    ChatMessage::system("plugin system"),
                    ChatMessage::user("after hook"),
                ]),
                error: None,
            })
            .unwrap();

        let hooked = hook_task.await.unwrap().unwrap();
        assert_eq!(hooked.len(), 2);
        assert_eq!(hooked[0].content, "plugin system");
        assert_eq!(hooked[1].content, "after hook");
    }

    #[tokio::test]
    async fn start_writing_command_prompt_hook_messages_reach_mock_llm() {
        let marker = "START_WRITING_COMMAND_HOOK_MARKER";
        let llm = Arc::new(RecordingMockLlm::new(vec![
            mock_chat_response(plan_response_json()),
            mock_chat_response("Seraphina performs a short beat."),
            mock_chat_response("Final draft from start_writing command."),
        ]));
        let app_state = state_with_recording_llm(llm.clone());
        let channel = command_prompt_hook_channel(app_state.clone(), marker);

        start_writing(
            "Write a tiny command hook test scene.".into(),
            None,
            None,
            None,
            tauri_state_for_test(&app_state),
            channel,
        )
        .await
        .expect("start_writing should complete with recording mock LLM");

        assert!(
            any_recorded_request_contains_marker(&llm, marker),
            "mock LLM should receive marker appended by command prompt hook"
        );
    }

    #[tokio::test]
    async fn regenerate_command_prompt_hook_messages_reach_mock_llm() {
        let setup_marker = "REGENERATE_SETUP_HOOK_MARKER";
        let regenerate_marker = "REGENERATE_COMMAND_HOOK_MARKER";
        let llm = Arc::new(RecordingMockLlm::new(vec![
            mock_chat_response(plan_response_json()),
            mock_chat_response("Seraphina performs a short setup beat."),
            mock_chat_response("Initial draft for regenerate command."),
            mock_chat_response("Regenerated draft from command hook test."),
        ]));
        let app_state = state_with_recording_llm(llm.clone());

        let setup = start_writing(
            "Write setup text for regenerate.".into(),
            None,
            None,
            None,
            tauri_state_for_test(&app_state),
            command_prompt_hook_channel(app_state.clone(), setup_marker),
        )
        .await
        .expect("start_writing setup should complete");
        let conversation_id = setup["conversation_id"]
            .as_str()
            .expect("start_writing result should include conversation_id")
            .to_string();
        let node_id = setup["node_id"]
            .as_str()
            .expect("start_writing result should include node_id")
            .to_string();
        llm.clear_requests();

        regenerate(
            RegenerateRequestDto {
                conversation_id,
                node_id,
                targets: vec![RegenerateTargetDto {
                    kind: "editor".into(),
                }],
                hint: Some("Keep it brief.".into()),
                seed: None,
            },
            tauri_state_for_test(&app_state),
            command_prompt_hook_channel(app_state.clone(), regenerate_marker),
        )
        .await
        .expect("regenerate should complete with recording mock LLM");

        assert!(
            any_recorded_request_contains_marker(&llm, regenerate_marker),
            "mock LLM should receive marker appended by regenerate command prompt hook"
        );
    }

    #[tokio::test]
    async fn frontend_prompt_hook_cleans_pending_request_when_cancelled() {
        use storyforge_app_agent::runtime::PromptHookContext;
        use storyforge_domain::agent::AgentRole;
        use storyforge_domain::llm::ChatMessage;

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
        let pending: PromptHookPendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let hook = frontend_prompt_hook(event_tx, pending.clone());

        let hook_task = tokio::spawn(hook(PromptHookContext {
            role: AgentRole::Editor,
            round: 1,
            model: "test-model".into(),
            messages: vec![ChatMessage::user("before hook")],
        }));

        let event = event_rx.recv().await.expect("hook should emit request");
        let request_id = match event {
            PipelineEvent::PromptHookRequest { request_id, .. } => request_id,
            other => panic!("expected prompt hook request, got {other:?}"),
        };
        assert!(
            pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains_key(&request_id)
        );

        hook_task.abort();
        let _ = hook_task.await;

        assert!(
            !pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains_key(&request_id)
        );
    }

    #[test]
    fn plugin_dto_exposes_modify_prompt_permission_and_event_subscriptions() {
        use storyforge_infra_plugin_host::{InstalledPlugin, Permission, PluginManifest, UiSlot};

        let plugin = InstalledPlugin {
            manifest: PluginManifest {
                id: "prompt-hook".into(),
                name: "Prompt Hook".into(),
                version: "1.0.0".into(),
                permissions: vec![Permission::ModifyPrompt],
                entry_html: String::new(),
                ui_slots: vec![UiSlot::SidebarPanel],
                event_subscriptions: vec!["CHAT_COMPLETION_PROMPT_READY".into()],
                description: None,
                author: None,
            },
            installed_at: chrono::Utc::now(),
            enabled: true,
        };

        let dto = plugin_to_dto(&plugin);

        assert_eq!(dto.permissions, vec!["ModifyPrompt"]);
        assert_eq!(
            dto.event_subscriptions,
            vec!["CHAT_COMPLETION_PROMPT_READY"]
        );
    }

    fn assert_complex_card_raw_extensions(raw_card_json: &serde_json::Value) {
        let extensions = raw_card_json
            .get("extensions")
            .and_then(|value| value.as_object())
            .expect("raw ST card extensions should remain an object");

        for key in ["regex_scripts", "tavern_helper", "xiaobaix-template"] {
            assert!(extensions.contains_key(key), "missing extension key: {key}");
        }
    }

    fn assert_complex_card_raw_world_book(raw_card_json: &serde_json::Value) {
        let entries = raw_card_json
            .get("character_book")
            .and_then(|book| book.get("entries"))
            .and_then(|entries| entries.as_array())
            .expect("raw ST card character_book.entries should remain an array");

        assert_eq!(entries.len(), 441);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("constant")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                })
                .count(),
            85
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("selective")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                })
                .count(),
            340
        );
    }

    struct TempDirGuard(std::path::PathBuf);

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            Self(std::env::temp_dir().join(format!("{prefix}_{}", uuid::Uuid::new_v4())))
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    #[ignore = "requires a local real ST card fixture; run scripts/run-real-card-smoke.ps1"]
    fn test_real_complex_card_fixture_can_create_campaign_and_roundtrip_bundle() {
        use storyforge_domain::character::{
            CharacterCard, CharacterDefinition, CharacterExtractionStatus,
        };

        let fixture_path = std::env::var_os("SF_COMPLEX_CARD_FIXTURE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("test-card.png")
            });
        let bytes = std::fs::read(&fixture_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture_path.display()));
        let character =
            storyforge_infra_import::import_character(&bytes).expect("complex card should import");

        assert_eq!(character.alternate_greetings.len(), 6);
        assert_complex_card_raw_extensions(&character.raw_card_json);
        assert_complex_card_raw_world_book(&character.raw_card_json);

        let dir = TempDirGuard::new("storyforge_test_real_complex_bundle");
        let store = campaign_store::CampaignStore::new(dir.path());
        let conv_store = ConversationStore::new(dir.path().join("conversations"));

        let mut card = CharacterCard::from_character(&character);
        card.extraction_status = CharacterExtractionStatus::Extracted;
        let mut definition = CharacterDefinition::fallback_from_character(&character, &[]);
        definition.card_id = card.id.clone();
        card.character_definitions = vec![definition];

        let stored = save_character_card_to_store(&store, card).unwrap();
        let campaign = create_campaign_in_store(
            &store,
            &conv_store,
            stored.card.id.as_str().to_string(),
            "Complex Fixture Campaign".into(),
            None,
        )
        .unwrap();

        let campaign_id = Id::from_str(&campaign.id);
        let instances = store.list_instances(&campaign_id);
        let expected_instance_count = instances.len();
        assert!(
            expected_instance_count > 0,
            "campaign should create instances"
        );

        let bundle_json = export_campaign_bundle_from_store(&store, campaign_id).unwrap();
        let bundle: CampaignBundle = serde_json::from_str(&bundle_json).unwrap();
        let exported_card = bundle.card.as_ref().expect("bundle should include card");
        let (_, exported_alternate_greetings) = raw_card_greetings(&exported_card.raw_card_json);

        assert_complex_card_raw_extensions(&exported_card.raw_card_json);
        assert_complex_card_raw_world_book(&exported_card.raw_card_json);
        assert_eq!(exported_alternate_greetings.len(), 6);
        assert_eq!(bundle.instances.len(), expected_instance_count);

        let import_dir = TempDirGuard::new("storyforge_test_real_complex_bundle_import");
        let import_store = campaign_store::CampaignStore::new(import_dir.path());
        let import_conv_store = ConversationStore::new(import_dir.path().join("conversations"));
        let result =
            import_campaign_bundle_into_store(&import_store, &import_conv_store, bundle).unwrap();

        assert_eq!(result.instance_count, expected_instance_count);
        let imported_card = import_store
            .get_card(&Id::from_str(&result.card_id))
            .expect("imported bundle card should exist");
        let (_, imported_alternate_greetings) =
            raw_card_greetings(&imported_card.card.raw_card_json);
        let imported_instances = import_store.list_instances(&Id::from_str(&result.campaign_id));

        assert_complex_card_raw_extensions(&imported_card.card.raw_card_json);
        assert_complex_card_raw_world_book(&imported_card.card.raw_card_json);
        assert_eq!(imported_alternate_greetings.len(), 6);
        assert_eq!(imported_instances.len(), expected_instance_count);
    }

    #[tokio::test]
    #[ignore = "requires a local real ST card fixture; run scripts/run-real-card-smoke.ps1"]
    async fn test_real_complex_card_offline_mvu_plumbing_smoke() {
        // This is an offline plumbing smoke. It uses the real complex PNG fixture
        // for import/campaign wiring, but feeds a deterministic MVU tool response
        // and a synthetic postprocess update so it can run without live LLM creds.
        use storyforge_domain::agent::VariableUpdate;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
        use storyforge_domain::character::{
            CharacterCard, CharacterDefinition, CharacterExtractionStatus,
        };
        use storyforge_domain::llm::{ChatResponse, FunctionCall, ToolCall};
        use storyforge_domain::mvu_translation::MvuRouting;

        let fixture_path = std::env::var_os("SF_COMPLEX_CARD_FIXTURE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("test-card.png")
            });
        let bytes = std::fs::read(&fixture_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture_path.display()));
        let character =
            storyforge_infra_import::import_character(&bytes).expect("complex card should import");

        let dir = TempDirGuard::new("storyforge_test_real_complex_offline_mvu_plumbing");
        let store = campaign_store::CampaignStore::new(dir.path());
        let conv_store = ConversationStore::new(dir.path().join("conversations"));

        let mut card = CharacterCard::from_character(&character);
        card.extraction_status = CharacterExtractionStatus::Extracted;
        let mut definition = CharacterDefinition::fallback_from_character(&character, &[]);
        definition.card_id = card.id.clone();
        card.character_definitions = vec![definition];

        let stored = save_character_card_to_store(&store, card).unwrap();
        let campaign = create_campaign_in_store(
            &store,
            &conv_store,
            stored.card.id.as_str().to_string(),
            "Complex Fixture MVU Fallback Campaign".into(),
            None,
        )
        .unwrap();

        let campaign_id = Id::from_str(&campaign.id);
        let initial_instances = store.list_instances(&campaign_id);
        let mut initial_instance = initial_instances
            .first()
            .expect("campaign should create at least one instance")
            .clone();
        let initial_variable_count = initial_instance.variables.len();
        let initial_had_mana = initial_instance
            .variables
            .iter()
            .any(|value| value.key == "mana");
        initial_instance
            .variables
            .retain(|value| value.key != "mana");
        store.update_instance(initial_instance).unwrap();

        let mvu_fixture_json = serde_json::json!({
            "variable_schema": [
                {"key": "hp", "label": "HP", "value_type": "int", "default": 100},
                {"key": "mana", "label": "Mana", "value_type": "int", "default": 30}
            ],
            "ui_bindings": [
                {"element": "hp_bar", "variable_key": "hp", "display": {"kind": "bar", "max": 100}},
                {"element": "mana_text", "variable_key": "mana", "display": {"kind": "text"}}
            ],
            "update_rules": ["damage reduces hp"],
            "interactions": [],
            "fallback_fragments": [
                {
                    "description": "complex card fallback probe",
                    "js_snippet": "variables.__complex_card_probe = true;",
                    "reason": "offline MVU plumbing smoke"
                },
                {
                    "description": "empty fallback fragments are ignored",
                    "js_snippet": "",
                    "reason": "filter coverage"
                }
            ],
            "routing": {"kind": "hybrid", "webview_reason": "offline fixture contains JS fallback"},
            "analysis_confidence": 0.92,
            "notes": ["offline fixture: this smoke validates plumbing, not live LLM analysis"]
        })
        .to_string();
        let mvu_response = ChatResponse {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "mvu-smoke-tool-call".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "emit_mvu_translation".into(),
                    arguments: mvu_fixture_json,
                },
            }],
            finish_reason: Some("tool_calls".into()),
            usage: None,
        };
        let translation = storyforge_app_meta::mvu_import::parse_mvu_translation_from_response(
            &mvu_response,
            &[],
        )
        .expect("offline MVU plumbing path should parse emit_mvu_translation");
        assert!(
            matches!(translation.routing, MvuRouting::Hybrid { .. }),
            "expected hybrid routing, got {:?}; fallback_count={}, schema_count={}",
            translation.routing,
            translation.fallback_fragments.len(),
            translation.variable_schema.len()
        );
        assert_eq!(translation.fallback_fragments.len(), 2);
        assert!(
            translation
                .variable_schema
                .iter()
                .any(|field| field.key == "mana")
        );

        save_mvu_translation_to_store(
            &store,
            campaign_store::StoredMvuTranslation {
                source_character_id: character.id.clone(),
                character_name: character.name.clone(),
                translation,
                analyzed_at: "2026-07-07T00:00:00Z".into(),
            },
        )
        .unwrap();
        meta_apply_mvu_schema_in_store(
            &store,
            character.id.as_str().to_string(),
            stored.card.character_definitions[0].id.as_str().to_string(),
        )
        .unwrap();

        let campaign = store
            .get_campaign(&campaign_id)
            .expect("campaign should exist");
        let instances = store.list_instances(&campaign_id);
        let present_instance_id = instances
            .first()
            .expect("campaign should create at least one instance")
            .id
            .as_str()
            .to_string();
        let instance_after_apply = instances
            .first()
            .expect("campaign should create at least one instance");
        assert!(
            instance_after_apply
                .variables
                .iter()
                .any(|value| value.key == "mana" && value.value == serde_json::json!(30)),
            "MVU apply should backfill existing campaign instances"
        );
        let expected_variable_count_after_apply = if initial_had_mana {
            initial_variable_count
        } else {
            initial_variable_count + 1
        };
        assert!(
            instance_after_apply.variables.len() >= expected_variable_count_after_apply,
            "MVU apply should preserve existing variables while backfilling missing fields"
        );
        let ctx = WritingContext {
            characters: vec![],
            world_info: None,
            conversation_id: campaign.conversation_id.clone().unwrap_or_default(),
            campaign_id: Some(campaign_id.clone()),
            turn: 1,
            pending_tasks: vec![],
            story_clock: String::new(),
            profile: None,
            modules: vec![],
            regex_scripts: vec![],
            campaign_runtime: Some(std::sync::Arc::new(CampaignRuntimeContext {
                campaign,
                instances,
                definitions_by_id: std::collections::HashMap::new(),
                knowledge: vec![],
                tasks: vec![],
                turn: 1,
            })),
            agent_profile_config: None,
        };

        let fragments = collect_mvu_fallback_fragments(
            &ctx,
            &store,
            std::slice::from_ref(&present_instance_id),
        );
        assert_eq!(fragments.len(), 1);
        assert_eq!(
            fragments[0].js_snippet,
            "variables.__complex_card_probe = true;"
        );

        let fragments_by_name = collect_mvu_fallback_fragments(&ctx, &store, &[character.name]);
        assert_eq!(fragments_by_name.len(), 1);

        let persist_ctx = PostprocessPersistContext {
            campaign_id: campaign_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            turn: 2,
        };
        let outcome = storyforge_app_agent::PostProcessOutcome {
            summary: Some("offline MVU plumbing smoke summary".into()),
            post_process: Some(storyforge_domain::agent::PostProcessResult {
                knowledge_updates: vec![],
                variable_updates: vec![VariableUpdate {
                    instance_id: Some(Id::from_str(&present_instance_id)),
                    key: "mana".into(),
                    value: serde_json::json!(64),
                }],
                task_updates: vec![],
                parse_succeeded: true,
            }),
        };
        persist_postprocess_outcome_to_store(
            &store,
            &persist_ctx,
            &outcome,
            std::slice::from_ref(&present_instance_id),
        );

        let updated_instance = store
            .list_instances(&campaign_id)
            .into_iter()
            .find(|inst| inst.id.as_str() == present_instance_id)
            .expect("updated instance should still exist");
        let mana = updated_instance
            .variables
            .iter()
            .find(|value| value.key == "mana")
            .expect("mana variable should exist after MVU apply");
        assert_eq!(mana.value, serde_json::json!(64));
        assert_eq!(mana.last_updated_turn, 2);
    }

    #[test]
    fn export_campaign_bundle_includes_complete_campaign_state() {
        use storyforge_domain::agent::RoundSummary;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
        use storyforge_domain::story_task::{StoryTask, TaskTrigger};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_export_bundle_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let card_id = Id::from_str("export-card");
        let source_id = Id::from_str("export-source");
        let campaign_id = Id::from_str("export-campaign");
        let conversation_id = Id::from_str("export-conversation");
        let def_a = Id::from_str("export-def-a");
        let def_b = Id::from_str("export-def-b");
        let instance_a = Id::from_str("export-instance-a");
        let instance_b = Id::from_str("export-instance-b");
        let knowledge_a = Id::from_str("export-knowledge-a");

        let definitions = vec![
            CharacterDefinition {
                id: def_a.clone(),
                card_id: card_id.clone(),
                name: "Alpha".into(),
                persona_prompt: "alpha persona".into(),
                behavior_rules: "protect the key".into(),
                base_backstory: vec!["Alpha found the sealed door.".into()],
                group: Some("party".into()),
                role_type: RoleType::Protagonist,
                variable_schema: vec![],
            },
            CharacterDefinition {
                id: def_b.clone(),
                card_id: card_id.clone(),
                name: "Beta".into(),
                persona_prompt: "beta persona".into(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: Some("party".into()),
                role_type: RoleType::Supporting,
                variable_schema: vec![],
            },
        ];
        let card = CharacterCard {
            id: card_id.clone(),
            name: "Export Bundle Card".into(),
            source_character_id: source_id,
            character_definitions: definitions.clone(),
            raw_card_json: serde_json::json!({
                "first_mes": "hello from export",
                "alternate_greetings": ["alt export"]
            }),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: Some("ok".into()),
        };
        store.save_card(card).unwrap();

        let mut campaign = Campaign::new(card_id.clone(), "Export Campaign");
        campaign.id = campaign_id.clone();
        campaign.conversation_id = Some(conversation_id.clone());
        campaign.set_variable("story_clock", serde_json::json!("Day 9 - dusk"), 7);
        store.save_campaign(campaign).unwrap();

        let instances = vec![
            CharacterInstance {
                id: instance_a.clone(),
                campaign_id: campaign_id.clone(),
                definition_id: Some(def_a.clone()),
                name: "Alpha".into(),
                persona_override: Some("alpha override".into()),
                behavior_override: None,
                variables: vec![],
                is_temporary: false,
            },
            CharacterInstance {
                id: instance_b.clone(),
                campaign_id: campaign_id.clone(),
                definition_id: Some(def_b.clone()),
                name: "Beta".into(),
                persona_override: None,
                behavior_override: Some("beta override".into()),
                variables: vec![],
                is_temporary: false,
            },
        ];
        for instance in instances {
            store.add_instance(instance).unwrap();
        }

        let mut knowledge = CharacterKnowledgeEntry::witnessed(
            campaign_id.clone(),
            instance_a.clone(),
            "Alpha knows the door code",
            4,
        );
        knowledge.id = knowledge_a.clone();
        knowledge.pinned = true;
        store.add_knowledge(vec![knowledge]).unwrap();

        let mut task = StoryTask::user_planned(
            campaign_id.clone(),
            "Open the sealed door",
            "Use the code later",
            vec![TaskTrigger::TurnReminder { at_turn: 6 }],
            5,
        );
        task.related_characters = vec![instance_a.clone(), instance_b.clone()];
        store.add_task(task).unwrap();

        store
            .add_summary(RoundSummary {
                id: Id::from_str("export-summary"),
                campaign_id: campaign_id.clone(),
                conversation_id: conversation_id.clone(),
                turn: 5,
                content: "Round five reached the sealed door.".into(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
            .unwrap();

        let bundle_json = export_campaign_bundle_from_store(&store, campaign_id.clone()).unwrap();
        let bundle: CampaignBundle = serde_json::from_str(&bundle_json).unwrap();

        assert_eq!(bundle.format_version, BUNDLE_FORMAT_VERSION);
        assert_eq!(bundle.campaign.id, campaign_id);
        assert_eq!(bundle.campaign.card_id, card_id);
        assert_eq!(
            bundle.campaign.conversation_id,
            Some(conversation_id.clone())
        );
        assert_eq!(
            bundle.campaign.get_variable("story_clock").unwrap(),
            &serde_json::json!("Day 9 - dusk")
        );

        let exported_card = bundle.card.as_ref().unwrap();
        assert_eq!(exported_card.id, card_id);
        assert_eq!(
            exported_card.source_character_id,
            Id::from_str("export-source")
        );
        assert_eq!(
            exported_card.extraction_status,
            storyforge_domain::character::CharacterExtractionStatus::Extracted
        );
        assert_eq!(exported_card.extraction_message.as_deref(), Some("ok"));
        let (first_mes, alternate_greetings) = raw_card_greetings(&exported_card.raw_card_json);
        assert_eq!(first_mes, "hello from export");
        assert_eq!(alternate_greetings, vec!["alt export"]);
        assert_eq!(exported_card.character_definitions.len(), 2);
        assert_eq!(bundle.definitions.len(), 2);
        assert_eq!(bundle.definitions[0].id, def_a);
        assert_eq!(
            bundle.definitions[0].base_backstory[0],
            "Alpha found the sealed door."
        );

        assert_eq!(bundle.instances.len(), 2);
        assert_eq!(bundle.instances[0].definition_id, Some(def_a));
        assert_eq!(
            bundle.instances[0].persona_override.as_deref(),
            Some("alpha override")
        );
        assert_eq!(bundle.instances[1].definition_id, Some(def_b));
        assert_eq!(
            bundle.instances[1].behavior_override.as_deref(),
            Some("beta override")
        );
        assert_eq!(bundle.knowledge.len(), 1);
        assert_eq!(bundle.knowledge[0].id, knowledge_a);
        assert_eq!(bundle.knowledge[0].character_id, instance_a);
        assert_eq!(
            bundle.knowledge[0].knowledge_text,
            "Alpha knows the door code"
        );
        assert_eq!(bundle.knowledge[0].turn_number, 4);
        assert_eq!(bundle.knowledge[0].source, KnowledgeSource::Witnessed);
        assert!(bundle.knowledge[0].pinned);
        assert_eq!(bundle.tasks.len(), 1);
        assert_eq!(bundle.tasks[0].title, "Open the sealed door");
        assert_eq!(bundle.tasks[0].description, "Use the code later");
        assert_eq!(
            bundle.tasks[0].triggers,
            vec![TaskTrigger::TurnReminder { at_turn: 6 }]
        );
        assert_eq!(
            bundle.tasks[0].related_characters,
            vec![Id::from_str("export-instance-a"), instance_b]
        );
        assert_eq!(bundle.summaries.len(), 1);
        assert_eq!(bundle.summaries[0].conversation_id, conversation_id);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_campaign_bundle_reports_missing_campaign() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_export_bundle_missing_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let err = export_campaign_bundle_from_store(&store, Id::from_str("missing-campaign"))
            .expect_err("missing campaign should return not_found");

        match err {
            TauriCommandError::NotFound { message } => {
                assert!(message.contains("missing-campaign"));
            }
            other => panic!("expected not_found error, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_campaign_bundle_rewrites_ids_and_references() {
        use storyforge_domain::agent::RoundSummary;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
        use storyforge_domain::story_task::{StoryTask, TaskTrigger};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_bundle_{}",
            uuid::Uuid::new_v4()
        ));
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let old_card_id = Id::from_str("old-card");
        let old_source_id = Id::from_str("old-source");
        let old_campaign_id = Id::from_str("old-campaign");
        let old_conversation_id = Id::from_str("old-conversation");
        let old_def_a = Id::from_str("old-def-a");
        let old_def_b = Id::from_str("old-def-b");
        let old_instance_a = Id::from_str("old-instance-a");
        let old_instance_b = Id::from_str("old-instance-b");
        let old_knowledge_a = Id::from_str("old-knowledge-a");
        let old_knowledge_b = Id::from_str("old-knowledge-b");

        let definitions = vec![
            CharacterDefinition {
                id: old_def_a.clone(),
                card_id: old_card_id.clone(),
                name: "Alpha".into(),
                persona_prompt: "alpha persona".into(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: Some("party".into()),
                role_type: RoleType::Protagonist,
                variable_schema: vec![],
            },
            CharacterDefinition {
                id: old_def_b.clone(),
                card_id: old_card_id.clone(),
                name: "Beta".into(),
                persona_prompt: "beta persona".into(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: Some("party".into()),
                role_type: RoleType::Supporting,
                variable_schema: vec![],
            },
        ];
        let card = CharacterCard {
            id: old_card_id.clone(),
            name: "Bundle Card".into(),
            source_character_id: old_source_id,
            character_definitions: definitions.clone(),
            raw_card_json: serde_json::json!({
                "first_mes": "hello from raw",
                "alternate_greetings": ["alt one", "alt two"]
            }),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let mut campaign = Campaign::new(old_card_id.clone(), "Bundle Campaign");
        campaign.id = old_campaign_id.clone();
        campaign.conversation_id = Some(old_conversation_id.clone());

        let instances = vec![
            CharacterInstance {
                id: old_instance_a.clone(),
                campaign_id: old_campaign_id.clone(),
                definition_id: Some(old_def_a.clone()),
                name: "Alpha".into(),
                persona_override: None,
                behavior_override: None,
                variables: vec![],
                is_temporary: false,
            },
            CharacterInstance {
                id: old_instance_b.clone(),
                campaign_id: old_campaign_id.clone(),
                definition_id: Some(old_def_b.clone()),
                name: "Beta".into(),
                persona_override: None,
                behavior_override: None,
                variables: vec![],
                is_temporary: false,
            },
        ];
        let knowledge = vec![
            CharacterKnowledgeEntry {
                id: old_knowledge_a.clone(),
                campaign_id: old_campaign_id.clone(),
                character_id: old_instance_a.clone(),
                knowledge_text: "Alpha knows the door code".into(),
                source: KnowledgeSource::Backstory,
                source_character_id: None,
                source_knowledge_id: None,
                turn_number: 0,
                event_id: None,
                pinned: true,
                propagation: Default::default(),
            },
            CharacterKnowledgeEntry {
                id: old_knowledge_b,
                campaign_id: old_campaign_id.clone(),
                character_id: old_instance_b.clone(),
                knowledge_text: "Beta heard the door code".into(),
                source: KnowledgeSource::ToldByOther,
                source_character_id: Some(old_instance_a.clone()),
                source_knowledge_id: Some(old_knowledge_a.clone()),
                turn_number: 1,
                event_id: None,
                pinned: false,
                propagation: Default::default(),
            },
        ];
        let tasks = vec![StoryTask::user_planned(
            old_campaign_id.clone(),
            "Open the sealed door",
            "Use the code later",
            vec![TaskTrigger::TurnReminder { at_turn: 2 }],
            1,
        )];
        let mut tasks = tasks;
        tasks[0].related_characters = vec![old_instance_a.clone(), Id::from_str("missing")];
        let summaries = vec![RoundSummary {
            id: Id::from_str("old-summary"),
            campaign_id: old_campaign_id.clone(),
            conversation_id: old_conversation_id,
            turn: 1,
            content: "Round one happened.".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }];

        let result = import_campaign_bundle_into_store(
            &store,
            &conv_store,
            CampaignBundle {
                format_version: BUNDLE_FORMAT_VERSION,
                exported_at: chrono::Utc::now().to_rfc3339(),
                card: Some(card),
                campaign,
                instances,
                definitions,
                knowledge,
                tasks,
                summaries,
            },
        )
        .unwrap();

        assert_ne!(result.card_id, "old-card");
        assert_ne!(result.campaign_id, "old-campaign");
        assert_eq!(result.instance_count, 2);
        assert_eq!(result.knowledge_count, 2);
        assert_eq!(result.task_count, 1);
        assert_eq!(result.summary_count, 1);

        let new_campaign_id = Id::from_str(&result.campaign_id);
        let imported_campaign = store.get_campaign(&new_campaign_id).unwrap();
        assert_eq!(imported_campaign.card_id.as_str(), result.card_id);
        assert_eq!(
            imported_campaign.conversation_id.as_ref().unwrap().as_str(),
            result.conversation_id
        );
        assert!(conv_store.find_by_campaign(&new_campaign_id).is_some());

        let imported_card = store.get_card(&Id::from_str(&result.card_id)).unwrap();
        let (first_mes, alternate_greetings) =
            raw_card_greetings(&imported_card.card.raw_card_json);
        assert_eq!(first_mes, "hello from raw");
        assert_eq!(alternate_greetings, vec!["alt one", "alt two"]);
        assert!(
            imported_card
                .card
                .character_definitions
                .iter()
                .all(|def| def.card_id.as_str() == result.card_id)
        );
        assert!(
            imported_card
                .card
                .character_definitions
                .iter()
                .all(|def| def.id.as_str() != "old-def-a" && def.id.as_str() != "old-def-b")
        );

        let imported_instances = store.list_instances(&new_campaign_id);
        assert_eq!(imported_instances.len(), 2);
        assert!(
            imported_instances
                .iter()
                .all(|inst| inst.campaign_id == new_campaign_id)
        );
        assert!(imported_instances.iter().all(
            |inst| inst.id.as_str() != "old-instance-a" && inst.id.as_str() != "old-instance-b"
        ));

        let imported_knowledge = store.list_knowledge(&new_campaign_id);
        assert_eq!(imported_knowledge.len(), 2);
        let told = imported_knowledge
            .iter()
            .find(|entry| entry.source == KnowledgeSource::ToldByOther)
            .unwrap();
        assert!(told.source_character_id.is_some());
        assert_ne!(
            told.source_character_id.as_ref().unwrap().as_str(),
            "old-instance-a"
        );
        assert!(told.source_knowledge_id.is_some());
        assert_ne!(
            told.source_knowledge_id.as_ref().unwrap().as_str(),
            "old-knowledge-a"
        );

        let imported_tasks = store.list_tasks(&new_campaign_id);
        assert_eq!(imported_tasks.len(), 1);
        assert_eq!(imported_tasks[0].related_characters.len(), 1);
        assert_ne!(
            imported_tasks[0].related_characters[0].as_str(),
            "old-instance-a"
        );

        let imported_summaries = store.list_summaries(&new_campaign_id);
        assert_eq!(imported_summaries.len(), 1);
        assert_eq!(
            imported_summaries[0].conversation_id.as_str(),
            result.conversation_id
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

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

    fn make_test_llm_connection(id: &str, api_key: &str) -> LlmConnection {
        LlmConnection {
            id: Id::from_str(id),
            name: id.into(),
            base_url: "https://api.example.com/v1/chat/completions".into(),
            api_key: api_key.into(),
            model: "test-model".into(),
            protocol: LlmProtocol::OpenAi,
            params: SamplingParams::default(),
            tool_mode: ToolMode::Native,
        }
    }

    #[tokio::test]
    async fn test_set_active_connection_async_persists_and_updates_state() {
        let state = Arc::new(AppState::new_for_test());
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_async_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());
        let store = Arc::new(ConnectionStore::new_with_secret_store(
            &dir,
            secret_store.clone(),
        ));
        store
            .save(make_test_llm_connection(
                "async-active",
                "async-active-secret",
            ))
            .unwrap();

        set_active_connection_with_store_async(state.clone(), store.clone(), "async-active".into())
            .await
            .unwrap();

        assert_eq!(state.active_conn_id().as_deref(), Some("async-active"));
        assert!(store.active_connection().is_some());

        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(!raw.contains("async-active-secret"));
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(stored["active_id"], "async-active");
        assert!(stored["connections"][0]["last_used_at"].is_string());

        let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
        assert_eq!(
            reloaded.active_connection().unwrap().id.as_str(),
            "async-active"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_set_active_connection_async_serializes_concurrent_updates() {
        let state = Arc::new(AppState::new_for_test());
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_async_race_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());
        let store = Arc::new(ConnectionStore::new_with_secret_store(
            &dir,
            secret_store.clone(),
        ));
        store
            .save(make_test_llm_connection("async-a", "async-secret-a"))
            .unwrap();
        store
            .save(make_test_llm_connection("async-b", "async-secret-b"))
            .unwrap();

        let queued_guard = state.active_connection_update.lock().await;
        let task_a = tokio::spawn(set_active_connection_with_store_async(
            state.clone(),
            store.clone(),
            "async-a".into(),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let task_b = tokio::spawn(set_active_connection_with_store_async(
            state.clone(),
            store.clone(),
            "async-b".into(),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        drop(queued_guard);

        task_a.await.unwrap().unwrap();
        task_b.await.unwrap().unwrap();

        assert_eq!(state.active_conn_id().as_deref(), Some("async-b"));
        assert_eq!(store.active_connection().unwrap().id.as_str(), "async-b");

        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(stored["active_id"], "async-b");

        let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
        assert_eq!(reloaded.active_connection().unwrap().id.as_str(), "async-b");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_create_connection_async_auto_activates_first_connection() {
        let state = Arc::new(AppState::new_for_test());
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_async_create_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());
        let store = Arc::new(ConnectionStore::new_with_secret_store(
            &dir,
            secret_store.clone(),
        ));
        let conn = make_test_llm_connection("created-first", "created-first-secret");

        let conn_id = create_connection_with_store_async(state.clone(), store.clone(), conn)
            .await
            .unwrap();

        assert_eq!(conn_id, "created-first");
        assert_eq!(state.active_conn_id().as_deref(), Some("created-first"));

        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(!raw.contains("created-first-secret"));
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(stored["active_id"], "created-first");

        let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
        assert_eq!(
            reloaded.active_connection().unwrap().id.as_str(),
            "created-first"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_delete_connection_async_serializes_with_set_active() {
        let state = Arc::new(AppState::new_for_test());
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_async_delete_race_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());
        let store = Arc::new(ConnectionStore::new_with_secret_store(
            &dir,
            secret_store.clone(),
        ));
        store
            .save(make_test_llm_connection("async-a", "async-secret-a"))
            .unwrap();
        store
            .save(make_test_llm_connection("async-b", "async-secret-b"))
            .unwrap();

        set_active_connection_with_store_async(state.clone(), store.clone(), "async-a".into())
            .await
            .unwrap();

        let queued_guard = state.active_connection_update.lock().await;
        let set_b = tokio::spawn(set_active_connection_with_store_async(
            state.clone(),
            store.clone(),
            "async-b".into(),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let delete_b = tokio::spawn(delete_connection_with_store_async(
            state.clone(),
            store.clone(),
            "async-b".into(),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        drop(queued_guard);

        set_b.await.unwrap().unwrap();
        delete_b.await.unwrap().unwrap();

        assert!(state.active_conn_id().is_none());
        assert!(store.get("async-b").is_none());
        assert!(store.active_connection().is_none());

        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(stored["active_id"].is_null());

        let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
        assert!(reloaded.active_connection().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_test_mvu_translation(
        source_id: &str,
        name: &str,
        analyzed_at: &str,
    ) -> campaign_store::StoredMvuTranslation {
        campaign_store::StoredMvuTranslation {
            source_character_id: Id::from_str(source_id),
            character_name: name.into(),
            translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(
                vec![],
            ),
            analyzed_at: analyzed_at.into(),
        }
    }

    #[tokio::test]
    async fn test_save_mvu_translation_async_persists_and_replaces_existing() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_mvu_async_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: &'static campaign_store::CampaignStore =
            Box::leak(Box::new(campaign_store::CampaignStore::new(&dir)));

        save_mvu_translation_async(
            store,
            make_test_mvu_translation("src-mvu", "MVU 初版", "2026-07-07T00:00:00Z"),
        )
        .await
        .unwrap();
        save_mvu_translation_async(
            store,
            make_test_mvu_translation("src-mvu", "MVU 更新", "2026-07-07T00:00:01Z"),
        )
        .await
        .unwrap();

        let stored = store.get_mvu(&Id::from_str("src-mvu")).unwrap();
        assert_eq!(stored.character_name, "MVU 更新");
        assert_eq!(stored.analyzed_at, "2026-07-07T00:00:01Z");
        assert_eq!(store.list_all_mvu().len(), 1);

        let reloaded = campaign_store::CampaignStore::new(&dir);
        let reloaded_stored = reloaded.get_mvu(&Id::from_str("src-mvu")).unwrap();
        assert_eq!(reloaded_stored.character_name, "MVU 更新");
        assert_eq!(reloaded_stored.analyzed_at, "2026-07-07T00:00:01Z");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_test_character_definition(
        card_id: &Id,
        id: &str,
        name: &str,
    ) -> storyforge_domain::character::CharacterDefinition {
        storyforge_domain::character::CharacterDefinition {
            id: Id::from_str(id),
            card_id: card_id.clone(),
            name: name.into(),
            persona_prompt: format!("{name} persona"),
            behavior_rules: format!("{name} behavior"),
            base_backstory: vec!["backstory".into()],
            group: None,
            role_type: storyforge_domain::character::RoleType::Protagonist,
            variable_schema: vec![],
        }
    }

    fn test_variable_field(
        key: &str,
        label: &str,
        default: serde_json::Value,
    ) -> storyforge_domain::variables::VariableField {
        storyforge_domain::variables::VariableField {
            key: key.into(),
            label: label.into(),
            value_type: storyforge_domain::variables::VariableType::Int,
            default,
            description: None,
            group: Some("status".into()),
        }
    }

    #[test]
    fn test_card_summary_treats_fallback_as_not_extracted() {
        let character = make_test_character("Fallback Card");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.extraction_status = storyforge_domain::character::CharacterExtractionStatus::Fallback;
        card.extraction_message = Some(FALLBACK_EXTRACTION_MESSAGE.into());
        card.character_definitions
            .push(make_test_character_definition(
                &card.id,
                "fallback-def",
                "Fallback Hero",
            ));

        let stored = campaign_store::StoredCard {
            card,
            imported_at: "2026-07-07T00:00:00Z".into(),
        };
        let dto = CardSummaryDto::from(&stored);

        assert!(!dto.extracted);
        assert_eq!(dto.extraction_status, "fallback");
        assert_eq!(dto.definition_count, 1);
        assert_eq!(dto.character_count, 1);
        assert_eq!(
            dto.extraction_message.as_deref(),
            Some(FALLBACK_EXTRACTION_MESSAGE)
        );
    }

    #[test]
    fn test_prepare_character_extraction_force_preserves_existing_card_id() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_extract_force_preserves_id_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let character = make_test_character("Force Rerun Card");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.id = Id::from_str("existing-card-id");
        card.extraction_status = storyforge_domain::character::CharacterExtractionStatus::Fallback;
        card.character_definitions
            .push(make_test_character_definition(
                &card.id,
                "old-fallback-def",
                "Old Fallback",
            ));
        store.save_card(card).unwrap();

        let decision = prepare_character_extraction_card(&store, &character, true).unwrap();

        match decision {
            CharacterExtractionDecision::Run(card) => {
                assert_eq!(card.id, Id::from_str("existing-card-id"));
                assert_eq!(
                    card.extraction_status,
                    storyforge_domain::character::CharacterExtractionStatus::Fallback
                );
            }
            CharacterExtractionDecision::ReturnExisting(_) => {
                panic!("force=true should rerun instead of returning existing card")
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prepare_character_extraction_refuses_force_when_campaign_exists() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_extract_force_campaign_guard_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let character = make_test_character("Existing Campaign Card");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.id = Id::from_str("guarded-card-id");
        card.extraction_status = storyforge_domain::character::CharacterExtractionStatus::Fallback;
        let stored = store.save_card(card).unwrap();
        let campaign =
            storyforge_domain::campaign::Campaign::new(stored.card.id.clone(), "existing run");
        store.save_campaign(campaign).unwrap();

        let err = prepare_character_extraction_card(&store, &character, true).unwrap_err();

        match err {
            TauriCommandError::Validation { message } => {
                assert!(message.contains("已有游玩档"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_force_rerun_commit_rejects_campaign_created_after_prepare() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_extract_force_commit_guard_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let character = make_test_character("Race Guard Card");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.id = Id::from_str("race-card-id");
        card.character_definitions
            .push(make_test_character_definition(
                &card.id,
                "old-race-def",
                "Old Race Def",
            ));
        store.save_card(card.clone()).unwrap();

        let decision = prepare_character_extraction_card(&store, &character, true).unwrap();
        let mut rerun_card = match decision {
            CharacterExtractionDecision::Run(card) => card,
            CharacterExtractionDecision::ReturnExisting(_) => {
                panic!("force=true should rerun instead of returning existing card")
            }
        };
        rerun_card.character_definitions.clear();
        rerun_card
            .character_definitions
            .push(make_test_character_definition(
                &rerun_card.id,
                "new-race-def",
                "New Race Def",
            ));

        let campaign =
            storyforge_domain::campaign::Campaign::new(card.id.clone(), "created during rerun");
        let (_, campaign, instance_count) = store.create_campaign_with_instances(campaign).unwrap();
        assert_eq!(instance_count, 1);

        let err = save_character_card_force_rerun_to_store(&store, rerun_card).unwrap_err();
        match err {
            TauriCommandError::Validation { message } => {
                assert!(message.contains("已有游玩档"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }

        let stored_after = store.get_card(&card.id).unwrap();
        assert!(
            stored_after
                .card
                .character_definitions
                .iter()
                .any(|def| def.id == Id::from_str("old-race-def"))
        );
        assert!(
            !stored_after
                .card
                .character_definitions
                .iter()
                .any(|def| def.id == Id::from_str("new-race-def"))
        );

        let instances = store.list_instances(&campaign.id);
        assert_eq!(instances.len(), 1);
        let definition_id = instances[0].definition_id.as_ref().unwrap();
        assert!(
            stored_after
                .card
                .character_definitions
                .iter()
                .any(|def| &def.id == definition_id)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_meta_apply_mvu_schema_backfills_all_campaign_instances_without_overwriting_values() {
        use storyforge_domain::campaign::Campaign;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_mvu_apply_backfill_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let character = make_test_character("MVU Apply Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        let mut definition = make_test_character_definition(&card.id, "mvu-apply-def", "Hero");
        definition.variable_schema = vec![test_variable_field("hp", "HP", serde_json::json!(100))];
        card.character_definitions.push(definition.clone());
        store.save_card(card.clone()).unwrap();

        store
            .save_mvu(campaign_store::StoredMvuTranslation {
                source_character_id: character.id.clone(),
                character_name: character.name.clone(),
                translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(
                    vec![
                        test_variable_field("hp", "Hit Points", serde_json::json!(200)),
                        test_variable_field("mana", "Mana", serde_json::json!(30)),
                    ],
                ),
                analyzed_at: "2026-07-07T00:00:00Z".into(),
            })
            .unwrap();

        let campaign_a = Campaign::new(card.id.clone(), "Campaign A");
        let campaign_a_id = campaign_a.id.clone();
        let (_, campaign_a, _) = store.create_campaign_with_instances(campaign_a).unwrap();
        let campaign_b = Campaign::new(card.id.clone(), "Campaign B");
        let campaign_b_id = campaign_b.id.clone();
        let (_, campaign_b, _) = store.create_campaign_with_instances(campaign_b).unwrap();

        let mut instance_a = store.list_instances(&campaign_a.id).remove(0);
        let hp = instance_a
            .variables
            .iter_mut()
            .find(|value| value.key == "hp")
            .unwrap();
        hp.value = serde_json::json!(42);
        hp.last_updated_turn = 9;
        store.update_instance(instance_a.clone()).unwrap();

        let mut instance_b = store.list_instances(&campaign_b.id).remove(0);
        instance_b.variables.retain(|value| value.key != "hp");
        store.update_instance(instance_b).unwrap();

        meta_apply_mvu_schema_in_store(
            &store,
            character.id.as_str().to_string(),
            definition.id.as_str().to_string(),
        )
        .unwrap();

        let updated_card = store.get_card(&card.id).unwrap().card;
        let updated_def = updated_card
            .character_definitions
            .iter()
            .find(|def| def.id == definition.id)
            .unwrap();
        let hp_schema = updated_def
            .variable_schema
            .iter()
            .find(|field| field.key == "hp")
            .unwrap();
        assert_eq!(hp_schema.label, "Hit Points");
        assert_eq!(hp_schema.default, serde_json::json!(200));
        assert!(
            updated_def
                .variable_schema
                .iter()
                .any(|field| field.key == "mana" && field.default == serde_json::json!(30))
        );

        for campaign_id in [campaign_a_id, campaign_b_id] {
            let instance = store.list_instances(&campaign_id).remove(0);
            assert_eq!(instance.definition_id.as_ref(), Some(&definition.id));
            let mana = instance
                .variables
                .iter()
                .find(|value| value.key == "mana")
                .unwrap();
            assert_eq!(mana.value, serde_json::json!(30));
            assert_eq!(mana.last_updated_turn, 0);
        }

        let preserved = store.list_instances(&campaign_a.id).remove(0);
        let preserved_hp = preserved
            .variables
            .iter()
            .find(|value| value.key == "hp")
            .unwrap();
        assert_eq!(preserved_hp.value, serde_json::json!(42));
        assert_eq!(preserved_hp.last_updated_turn, 9);

        let restored = store.list_instances(&campaign_b.id).remove(0);
        let restored_hp = restored
            .variables
            .iter()
            .find(|value| value.key == "hp")
            .unwrap();
        assert_eq!(restored_hp.value, serde_json::json!(200));
        assert_eq!(restored_hp.last_updated_turn, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_save_character_card_async_persists_and_replaces_source() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_card_async_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: &'static campaign_store::CampaignStore =
            Box::leak(Box::new(campaign_store::CampaignStore::new(&dir)));

        let character = make_test_character("Async Card Source");
        let mut first = storyforge_domain::character::CharacterCard::from_character(&character);
        first
            .character_definitions
            .push(make_test_character_definition(
                &first.id,
                "def-first",
                "First",
            ));
        let mut second = storyforge_domain::character::CharacterCard::from_character(&character);
        second.name = "Async Card Updated".into();
        second
            .character_definitions
            .push(make_test_character_definition(
                &second.id,
                "def-second",
                "Second",
            ));

        save_character_card_async(store, first).await.unwrap();
        let stored = save_character_card_async(store, second).await.unwrap();

        assert_eq!(stored.card.name, "Async Card Updated");
        assert_eq!(stored.card.character_definitions.len(), 1);
        assert_eq!(store.list_cards().len(), 1);
        assert_eq!(
            store.get_card_by_source(&character.id).unwrap().card.name,
            "Async Card Updated"
        );

        let reloaded = campaign_store::CampaignStore::new(&dir);
        let reloaded_card = reloaded.get_card_by_source(&character.id).unwrap();
        assert_eq!(reloaded_card.card.name, "Async Card Updated");
        assert_eq!(reloaded_card.card.character_definitions[0].name, "Second");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_create_campaign_in_store_cleans_conversation_on_store_failure() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_create_campaign_cleanup_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let character = make_test_character("Cleanup Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.character_definitions
            .push(make_test_character_definition(
                &card.id,
                "cleanup-def",
                "Cleanup",
            ));
        let card_id = card.id.as_str().to_string();
        store.save_card(card).unwrap();
        std::fs::create_dir_all(dir.join("instances.json")).unwrap();

        let err = create_campaign_in_store(
            &store,
            &conv_store,
            card_id,
            "cleanup blocked".into(),
            Some("opening line".into()),
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("instances.json"),
            "expected instances persist failure, got {err}"
        );
        assert!(store.list_campaigns().is_empty());
        assert!(store.list_all_instances().is_empty());
        assert!(conv_store.list().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
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
    fn test_embed_config_recovers_tmp_and_migrates_plaintext_key() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_embed_tmp_migrate_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = MemorySecretStore::default();
        let api_key = ["s", "k", "-embed-recovered-secret"].concat();
        let config = storyforge_infra_llm::EmbedConfig {
            endpoint: "https://api.example.com/v1/embeddings".into(),
            api_key: api_key.clone(),
            model: "embed-model".into(),
            dim: 3,
        };
        let path = dir.join("embed.json");
        std::fs::write(&path, "{ invalid").unwrap();
        storyforge_infra_util::atomic_write_json(
            &PathBuf::from(format!("{}.tmp", path.display())),
            &config,
        )
        .unwrap();

        let loaded = load_embed_config_with_secret_store(&dir, &secret_store).unwrap();
        assert_eq!(loaded.api_key, api_key);

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(&api_key));
        assert!(!raw.contains("Authorization"));
        assert!(!raw.contains("Bearer"));
        assert!(!raw.contains("sk-"));
        assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));
        assert!(!path.with_extension("json.corrupt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_configure_embedder_async_persists_secret_ref_and_updates_state() {
        let state = Arc::new(AppState::new_for_test());
        let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
        let config = storyforge_infra_llm::EmbedConfig {
            endpoint: "https://api.example.com/v1/embeddings".into(),
            api_key: "embed-async-secret".into(),
            model: "embed-model".into(),
            dim: 3,
        };

        configure_embedder_with_secret_store_async(
            state.clone(),
            config.clone(),
            secret_store.clone(),
        )
        .await
        .unwrap();

        let raw = std::fs::read_to_string(state.data_dir.join("embed.json")).unwrap();
        assert!(!raw.contains("embed-async-secret"));
        assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));

        let loaded =
            load_embed_config_with_secret_store(&state.data_dir, secret_store.as_ref()).unwrap();
        assert_eq!(loaded.api_key, "embed-async-secret");

        let in_memory = state
            .embed_config
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .unwrap();
        assert_eq!(in_memory.endpoint, config.endpoint);
        assert_eq!(in_memory.api_key, "embed-async-secret");
        assert_eq!(in_memory.model, config.model);
        assert_eq!(in_memory.dim, config.dim);
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
        assert!(json.contains("active_preset.json"));
        assert!(json.contains("global_regex_scripts.json"));
        assert!(json.contains("logs"));
        assert!(!json.contains("sk-live-secret"));
        assert!(!json.contains("embed-live-secret"));
        assert_eq!(context["store_files"][0]["has_bytes"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_diagnostic_export_bundle_summarizes_secret_stores_without_leaking_keys() {
        let state = AppState::new_for_test();
        let data_dir = state.data_dir.clone();
        let key_prefix = ["s", "k", "-"].concat();
        let connection_key = format!("{key_prefix}test-diagnostic-connection-secret");
        let embed_key = format!("{key_prefix}test-diagnostic-embed-secret");
        let connection_bearer = "diagnostic-connection-bearer-secret";
        let embed_bearer = "diagnostic-embed-bearer-secret";
        storyforge_infra_util::atomic_write_json_str(
            &data_dir.join("connections.json"),
            &format!(
                r#"{{
                "active_id": "test-openai",
                "items": [{{
                    "id": "test-openai",
                    "api_key": "{connection_key}",
                    "headers": {{
                        "Authorization": "Bearer {connection_bearer}"
                    }}
                }}]
            }}"#
            ),
        )
        .unwrap();
        storyforge_infra_util::atomic_write_json_str(
            &data_dir.join("embed.json"),
            &format!(
                r#"{{
                "endpoint": "https://example.invalid/embeddings",
                "api_key": "{embed_key}",
                "headers": {{
                    "Authorization": "Bearer {embed_bearer}"
                }}
            }}"#
            ),
        )
        .unwrap();
        state.log_store.push(storyforge_app_logging::LogEntry {
            id: Id::new(),
            kind: LogKind::Backend,
            level: LogLevel::Info,
            timestamp: Utc::now(),
            message: "diagnostic export marker".into(),
            fields: HashMap::new(),
            llm_detail: None,
        });

        let opts = ExportOptions {
            redact_content: true,
            ..Default::default()
        };
        let mut bundle = storyforge_app_logging::export_bundle(&state.log_store, &opts).unwrap();
        bundle.as_object_mut().unwrap().insert(
            "diagnostic_context".into(),
            diagnostic_context_for_data_dir(&data_dir),
        );
        let store_files = bundle["diagnostic_context"]["store_files"]
            .as_array()
            .unwrap();
        let connection_summary = store_files
            .iter()
            .find(|file| file["name"] == "connections.json")
            .unwrap();
        let embed_summary = store_files
            .iter()
            .find(|file| file["name"] == "embed.json")
            .unwrap();

        assert_eq!(bundle["counts"]["backend"], 1);
        assert_eq!(
            bundle["backend_logs"][0]["message"],
            "diagnostic export marker"
        );
        assert_eq!(connection_summary["exists"], true);
        assert_eq!(connection_summary["has_bytes"], true);
        assert_eq!(embed_summary["exists"], true);
        assert_eq!(embed_summary["has_bytes"], true);
        assert!(connection_summary.get("content").is_none());
        assert!(embed_summary.get("content").is_none());

        let json = serde_json::to_string(&bundle).unwrap();
        for needle in &[
            connection_key,
            embed_key,
            connection_bearer.into(),
            embed_bearer.into(),
            "Authorization".into(),
            "Bearer".into(),
            key_prefix,
            "api_key".into(),
        ] {
            assert!(!json.contains(needle), "bundle leaked {needle}");
        }

        let _ = std::fs::remove_dir_all(data_dir);
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

    #[tokio::test]
    async fn test_conv_generation_explainer_reads_provenance_async() {
        let state = Arc::new(AppState::new_for_test());
        let conversation = state.conv_store.create(Some("card-1".into()), None);
        let node_id = state
            .conv_store
            .append_ai_draft(
                &conversation.id,
                "draft text".into(),
                Some(Provenance {
                    session_id: Id::from_str("sess-1"),
                    plan: None,
                    subagent_results: vec![storyforge_domain::conversation::SubagentSnapshot {
                        character_id: "alice".into(),
                        full_text: "Alice output".into(),
                        character_instance_id: None,
                        display_name: Some("Alice".into()),
                        fallback_reason: None,
                    }],
                    profile_id: Some(Id::from_str("profile-1")),
                    seed: 7,
                    last_hint: Some("try again".into()),
                }),
            )
            .unwrap();
        let explainer = ConvGenerationExplainer {
            conv_store: state.conv_store.clone(),
        };

        let explanation =
            <ConvGenerationExplainer as storyforge_app_meta::meta_conversation::GenerationExplainer>::explain(
                &explainer,
                conversation.id.as_str().to_string(),
                node_id.as_str().to_string(),
            )
            .await
            .unwrap();

        assert_eq!(explanation.seed, 7);
        assert_eq!(explanation.profile_id.as_deref(), Some("profile-1"));
        assert_eq!(explanation.last_hint.as_deref(), Some("try again"));
        assert_eq!(explanation.subagents.len(), 1);
        assert_eq!(explanation.subagents[0].character_id, "alice");
        assert_eq!(explanation.subagents[0].display_name, "Alice");
        assert_eq!(explanation.subagents[0].output_preview, "Alice output");

        let _ = std::fs::remove_dir_all(&state.data_dir);
    }

    #[tokio::test]
    async fn test_conv_generation_explainer_returns_none_for_missing_provenance_or_node() {
        let state = Arc::new(AppState::new_for_test());
        let conversation = state.conv_store.create(Some("card-1".into()), None);
        let node_without_provenance = state
            .conv_store
            .append_ai_draft(&conversation.id, "draft text".into(), None)
            .unwrap();
        let explainer = ConvGenerationExplainer {
            conv_store: state.conv_store.clone(),
        };

        let without_provenance =
            <ConvGenerationExplainer as storyforge_app_meta::meta_conversation::GenerationExplainer>::explain(
                &explainer,
                conversation.id.as_str().to_string(),
                node_without_provenance.as_str().to_string(),
            )
            .await;
        let missing_node =
            <ConvGenerationExplainer as storyforge_app_meta::meta_conversation::GenerationExplainer>::explain(
                &explainer,
                conversation.id.as_str().to_string(),
                Id::new().as_str().to_string(),
            )
            .await;

        assert!(without_provenance.is_none());
        assert!(missing_node.is_none());

        let _ = std::fs::remove_dir_all(&state.data_dir);
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

    #[tokio::test]
    async fn test_accept_variant_async_persists_final_variant() {
        let state = Arc::new(AppState::new_for_test());
        let conversation = state.conv_store.create(Some("card-1".into()), None);
        let node_id = state
            .conv_store
            .append_ai_draft(&conversation.id, "draft text".into(), None)
            .unwrap();

        accept_variant_async(state.clone(), conversation.id.clone(), node_id.clone())
            .await
            .unwrap();

        let updated = state.conv_store.get(&conversation.id).unwrap();
        let node = updated
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .unwrap();
        assert_eq!(node.active().unwrap().status, VariantStatus::Final);

        let reloaded = ConversationStore::new(state.data_dir.join("conversations"));
        let persisted = reloaded.get(&conversation.id).unwrap();
        let persisted_node = persisted
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .unwrap();
        assert_eq!(
            persisted_node.active().unwrap().status,
            VariantStatus::Final
        );
    }

    #[tokio::test]
    async fn test_archivable_messages_async_filters_discarded_variants() {
        let state = Arc::new(AppState::new_for_test());
        let conversation = state.conv_store.create(Some("card-1".into()), None);
        state
            .conv_store
            .append_user_message(&conversation.id, "user intent".into())
            .unwrap();
        let discarded_node_id = state
            .conv_store
            .append_ai_draft(&conversation.id, "discarded draft".into(), None)
            .unwrap();
        state
            .conv_store
            .soft_delete_variant(&conversation.id, &discarded_node_id)
            .unwrap();
        state
            .conv_store
            .append_ai_draft(&conversation.id, "kept draft".into(), None)
            .unwrap();

        let messages = archivable_messages_async(state.conv_store.clone(), conversation.id.clone())
            .await
            .unwrap();

        assert_eq!(messages, vec!["user intent", "kept draft"]);
    }

    #[tokio::test]
    async fn test_prepare_start_conversation_async_persists_legacy_and_existing_paths() {
        let state = Arc::new(AppState::new_for_test());
        let campaign_dir = std::env::temp_dir().join(format!(
            "storyforge_test_start_conversation_campaign_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&campaign_dir).unwrap();
        let campaign_store: &'static campaign_store::CampaignStore =
            Box::leak(Box::new(campaign_store::CampaignStore::new(&campaign_dir)));
        let mut legacy_character = make_test_character("Legacy Starter");
        legacy_character.first_mes = "opening line".into();
        let legacy_character = Arc::new(legacy_character);

        let created = prepare_start_conversation_async(
            state.clone(),
            campaign_store,
            None,
            Some("char-legacy".into()),
            Some(legacy_character),
            None,
            "write first scene".into(),
        )
        .await
        .unwrap();

        assert_eq!(created.regex_character_id.as_deref(), Some("char-legacy"));
        let created_conv = state.conv_store.get(&created.conversation_id).unwrap();
        assert_eq!(created_conv.character_id.as_deref(), Some("char-legacy"));
        assert_eq!(created_conv.nodes.len(), 2);
        assert_eq!(
            created_conv.nodes[0].active().unwrap().role,
            ConversationRole::Assistant
        );
        assert_eq!(
            created_conv.nodes[0].active().unwrap().status,
            VariantStatus::Final
        );
        assert_eq!(created_conv.nodes[0].active_content(), "opening line");
        assert_eq!(
            created_conv.nodes[1].active().unwrap().role,
            ConversationRole::User
        );
        assert_eq!(created_conv.nodes[1].active_content(), "write first scene");

        let reloaded = ConversationStore::new(state.data_dir.join("conversations"));
        let persisted = reloaded.get(&created.conversation_id).unwrap();
        assert_eq!(persisted.nodes.len(), 2);
        assert_eq!(persisted.nodes[0].active_content(), "opening line");
        assert_eq!(persisted.nodes[1].active_content(), "write first scene");

        let existing = state.conv_store.create(Some("char-existing".into()), None);
        let mut ignored_character = make_test_character("Ignored Starter");
        ignored_character.first_mes = "ignored opening".into();
        let reused = prepare_start_conversation_async(
            state.clone(),
            campaign_store,
            Some(existing.id.as_str().to_string()),
            None,
            Some(Arc::new(ignored_character)),
            Some("ignored opening".into()),
            "continue scene".into(),
        )
        .await
        .unwrap();

        assert_eq!(reused.conversation_id, existing.id);
        assert_eq!(reused.regex_character_id.as_deref(), Some("char-existing"));
        let reused_conv = state.conv_store.get(&existing.id).unwrap();
        assert_eq!(reused_conv.nodes.len(), 1);
        assert_eq!(
            reused_conv.nodes[0].active().unwrap().role,
            ConversationRole::User
        );
        assert_eq!(reused_conv.nodes[0].active_content(), "continue scene");

        let mut active_campaign = storyforge_domain::campaign::Campaign::new(
            Id::from_str(format!("card-{}", uuid::Uuid::new_v4())),
            "Active Campaign",
        );
        let campaign_conv = state.conv_store.create(
            Some("char-campaign".into()),
            Some(active_campaign.id.clone()),
        );
        active_campaign.conversation_id = Some(campaign_conv.id.clone());
        campaign_store
            .save_campaign(active_campaign.clone())
            .unwrap();
        {
            let mut active = state
                .active_campaign
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *active = Some(active_campaign.id.clone());
        }

        let requested_conv = state.conv_store.create(Some("char-requested".into()), None);
        let mut ignored_campaign_character = make_test_character("Campaign Ignored");
        ignored_campaign_character.first_mes = "campaign ignored opening".into();
        let campaign_target = prepare_start_conversation_async(
            state.clone(),
            campaign_store,
            Some(requested_conv.id.as_str().to_string()),
            None,
            Some(Arc::new(ignored_campaign_character)),
            Some("campaign ignored opening".into()),
            "campaign intent".into(),
        )
        .await
        .unwrap();

        assert_eq!(campaign_target.conversation_id, campaign_conv.id);
        assert_eq!(
            campaign_target.regex_character_id.as_deref(),
            Some("char-campaign")
        );
        let campaign_conv = state.conv_store.get(&campaign_conv.id).unwrap();
        assert_eq!(campaign_conv.nodes.len(), 1);
        assert_eq!(
            campaign_conv.nodes[0].active().unwrap().role,
            ConversationRole::User
        );
        assert_eq!(campaign_conv.nodes[0].active_content(), "campaign intent");
        let requested_conv = state.conv_store.get(&requested_conv.id).unwrap();
        assert!(requested_conv.nodes.is_empty());

        let _ = std::fs::remove_dir_all(&campaign_dir);
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

    #[test]
    fn stored_world_info_both_route_restores_constant_and_selective_semantics() {
        let mut info = CharacterInfo::from(&make_test_character("Active Card"));
        info.world_info_entries = vec![WorldInfoEntryInfo {
            keys: vec!["harbor".into()],
            content: "BOTH_ROUTE_LORE".into(),
            constant: true,
            route: "Both".into(),
            is_global: false,
            depth: 2,
            order: 100,
        }];

        let stored = storage::StoredCharacter {
            id: "active-card".into(),
            info,
            imported_at: "2026-07-07T00:00:00Z".into(),
        };
        let book = collect_world_info_for_active(&[stored], "Active Card");

        let constant_contents: Vec<_> = book
            .constant_entries()
            .into_iter()
            .map(|entry| entry.content.as_str())
            .collect();
        let triggered_contents: Vec<_> = book
            .triggered_selective_entries("sail to the harbor")
            .into_iter()
            .map(|entry| entry.content.as_str())
            .collect();

        assert_eq!(constant_contents, vec!["BOTH_ROUTE_LORE"]);
        assert_eq!(triggered_contents, vec!["BOTH_ROUTE_LORE"]);
    }

    #[test]
    fn character_info_and_restore_preserve_alternate_greetings() {
        let mut character = make_test_character("Greeter");
        character.first_mes = "default opening".into();
        character.alternate_greetings = vec!["alternate one".into(), "alternate two".into()];

        let info = CharacterInfo::from(&character);
        assert_eq!(
            info.alternate_greetings,
            vec!["alternate one".to_string(), "alternate two".to_string()]
        );

        let stored = storage::StoredCharacter {
            id: "stored-greeter".into(),
            info,
            imported_at: "now".into(),
        };
        let restored = stored_info_to_character(&stored);
        assert_eq!(
            restored.alternate_greetings,
            vec!["alternate one".to_string(), "alternate two".to_string()]
        );
    }

    #[test]
    fn character_info_restore_preserves_st_round_trip_fields() {
        use storyforge_domain::character::to_st_data;
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        let mut character = make_test_character("RoundTrip");
        character.description = "stored description".into();
        character.mes_example = "<START>\n示例对话".into();
        character.post_history_instructions = "历史后指令".into();
        character.character_version = "2.1".into();
        character.raw_card_json = serde_json::json!({
            "name": "RoundTrip",
            "description": "raw description",
            "group_only": true,
            "creator_notes": "keep this unknown field",
            "extensions": {
                "unknown_plugin": {"state": 7}
            }
        });
        character.embedded_world_info = Some(WorldInfoBook {
            source: storyforge_domain::Source::ImportedFromST,
            entries: vec![WorldInfoEntry {
                st_id: Some(9),
                keys: vec!["钥匙".into()],
                secondary_keys: vec!["门".into()],
                content: "世界书内容".into(),
                constant: true,
                selective: false,
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 3,
                order: 12,
                route: LoreRoute::Constant,
                extensions: serde_json::json!({"entry_extra": true}),
            }],
        });

        let stored = storage::StoredCharacter {
            id: "stored-round-trip".into(),
            info: CharacterInfo::from(&character),
            imported_at: "now".into(),
        };

        let restored = stored_info_to_character(&stored);
        let exported = to_st_data(
            &restored,
            None,
            restored
                .embedded_world_info
                .as_ref()
                .map(|b| b.to_st_book()),
        );
        let exported_json = serde_json::to_value(&exported).unwrap();

        assert_eq!(restored.mes_example, "<START>\n示例对话");
        assert_eq!(restored.post_history_instructions, "历史后指令");
        assert_eq!(restored.character_version, "2.1");
        assert_eq!(exported_json["group_only"], true);
        assert_eq!(exported_json["creator_notes"], "keep this unknown field");
        assert_eq!(exported_json["extensions"]["unknown_plugin"]["state"], 7);
        assert_eq!(exported.mes_example, "<START>\n示例对话");
        assert_eq!(exported.post_history_instructions, "历史后指令");
        assert_eq!(exported.character_version, "2.1");
        assert_eq!(exported.character_book.unwrap().entries[0].id, Some(9));
    }

    #[test]
    fn resolve_legacy_opening_message_accepts_only_card_greetings() {
        let mut character = make_test_character("Greeter");
        character.first_mes = "default opening".into();
        character.alternate_greetings = vec!["alternate one".into(), "alternate two".into()];
        let character = Arc::new(character);

        assert_eq!(
            resolve_legacy_opening_message(Some(&character), Some("alternate two".into())),
            Some("alternate two".into())
        );
        assert_eq!(
            resolve_legacy_opening_message(Some(&character), Some("not from this card".into())),
            Some("default opening".into())
        );
        assert_eq!(
            resolve_legacy_opening_message(Some(&character), Some("   ".into())),
            Some("default opening".into())
        );
    }

    #[test]
    fn resolve_opening_message_from_parts_supports_campaign_greetings() {
        let alternates = vec!["alternate one".to_string(), "alternate two".to_string()];

        assert_eq!(
            resolve_opening_message_from_parts(
                "default opening",
                &alternates,
                Some("alternate two".into()),
                "campaign"
            ),
            Some("alternate two".into())
        );
        assert_eq!(
            resolve_opening_message_from_parts(
                "default opening",
                &alternates,
                Some("not from this card".into()),
                "campaign"
            ),
            Some("default opening".into())
        );
        assert_eq!(
            resolve_opening_message_from_parts("", &alternates, None, "campaign"),
            Some("alternate one".into())
        );
        assert_eq!(
            resolve_opening_message_from_parts("", &[], Some("missing".into()), "campaign"),
            None
        );
    }

    fn test_regex_script(
        id: &str,
        source: RegexScriptSource,
    ) -> storyforge_domain::preset::RegexScript {
        storyforge_domain::preset::RegexScript {
            id: id.to_string(),
            script_name: id.to_string(),
            find_regex: id.to_string(),
            replace_string: String::new(),
            placement: storyforge_domain::preset::RegexPlacement::Output,
            placement_codes: vec![ST_REGEX_PLACEMENT_AI_OUTPUT],
            source,
            disabled: false,
            flags: String::new(),
            only_format_formatting: None,
            markdown_only: None,
            prompt_only: None,
            run_on_edit: None,
            substitute_regex: None,
            trim_strings: vec![],
            min_depth: None,
            max_depth: None,
        }
    }

    #[test]
    fn test_conversation_display_dto_applies_markdown_only_output_without_mutating_content() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_message(
            storyforge_domain::conversation::Role::User,
            "<data_block>user</data_block>".into(),
        );
        conversation.append_ai_draft("<data_block>hp=5</data_block> scene".into(), None);

        let mut script = test_regex_script("display-hp", RegexScriptSource::Preset);
        script.find_regex = r"<data_block>hp=5</data_block>".into();
        script.replace_string = "[HP:5]".into();
        script.markdown_only = Some(true);

        let dto = conversation_display_dto(&conversation, &[script]);

        let user_variant = &dto.nodes[0].variants[0];
        assert_eq!(user_variant.content, "<data_block>user</data_block>");
        assert_eq!(
            user_variant.display_content,
            "<data_block>user</data_block>"
        );

        let assistant_variant = &dto.nodes[1].variants[0];
        assert_eq!(
            assistant_variant.content,
            "<data_block>hp=5</data_block> scene"
        );
        assert_eq!(assistant_variant.display_content, "[HP:5] scene");

        assert_eq!(
            conversation.nodes[1].variants[0].content,
            "<data_block>hp=5</data_block> scene"
        );
    }

    #[test]
    fn test_conversation_display_dto_applies_markdown_only_reasoning_without_mutating_content() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_ai_draft("<think>raw chain</think> final raw".into(), None);

        let mut script = test_regex_script("display-reasoning", RegexScriptSource::Preset);
        script.find_regex = r"raw".into();
        script.replace_string = "pretty".into();
        script.placement = RegexPlacement::Reasoning;
        script.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];
        script.markdown_only = Some(true);
        script.flags = "g".into();

        let dto = conversation_display_dto(&conversation, &[script]);

        let assistant_variant = &dto.nodes[0].variants[0];
        assert_eq!(
            assistant_variant.content,
            "<think>raw chain</think> final raw"
        );
        assert_eq!(
            assistant_variant.display_content,
            "<think>pretty chain</think> final raw"
        );
        assert_eq!(
            conversation.nodes[0].variants[0].content,
            "<think>raw chain</think> final raw"
        );
    }

    #[test]
    fn test_conversation_display_dto_does_not_reapply_persisted_output_regex() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_ai_draft("persisted bar".into(), None);

        let mut script = test_regex_script("persisted-output", RegexScriptSource::Preset);
        script.find_regex = "bar".into();
        script.replace_string = "baz".into();

        let dto = conversation_display_dto(&conversation, &[script]);

        let variant = &dto.nodes[0].variants[0];
        assert_eq!(variant.content, "persisted bar");
        assert_eq!(variant.display_content, "persisted bar");
    }

    #[test]
    fn test_conversation_display_dto_respects_display_regex_depth() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_ai_draft("<status>old</status>".into(), None);
        conversation.append_message(
            storyforge_domain::conversation::Role::User,
            "continue".into(),
        );
        conversation.append_ai_draft("<status>new</status>".into(), None);

        let mut script = test_regex_script("recent-status", RegexScriptSource::Preset);
        script.find_regex = r"<status>(.*?)</status>".into();
        script.replace_string = "[$1]".into();
        script.markdown_only = Some(true);
        script.min_depth = Some(0);
        script.max_depth = Some(1);

        let dto = conversation_display_dto(&conversation, &[script]);

        let old_variant = &dto.nodes[0].variants[0];
        let new_variant = &dto.nodes[2].variants[0];
        assert_eq!(old_variant.display_content, "<status>old</status>");
        assert_eq!(new_variant.display_content, "[new]");
    }

    /// 验证 current_cancel 的存取（cancel_writing 命令的核心机制）
    #[test]
    fn test_fork_campaign_in_store_records_source_and_clones_snapshot() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
        use storyforge_domain::variables::default_character_variables;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_campaign_fork_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(dir.join("conversations")).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let mut card = CharacterCard {
            id: Id::from_str("card-1"),
            name: "Test Card".into(),
            source_character_id: Id::from_str("source-card-1"),
            character_definitions: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let definition = CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: card.id.clone(),
            name: "Lin".into(),
            persona_prompt: "calm surgeon".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        card.character_definitions.push(definition.clone());
        store.save_card(card).unwrap();

        let mut source_campaign = Campaign::new(Id::from_str("card-1"), "source run");
        source_campaign.set_variable("story_clock", serde_json::json!("Day 7"), 3);
        let source_conversation =
            conv_store.create(Some("card-1".into()), Some(source_campaign.id.clone()));
        let user_node = conv_store
            .append_user_message(&source_conversation.id, "first user".into())
            .unwrap();
        let fork_node = conv_store
            .append_ai_draft(&source_conversation.id, "branch point".into(), None)
            .unwrap();
        let _later = conv_store
            .append_user_message(&source_conversation.id, "later user".into())
            .unwrap();
        source_campaign.conversation_id = Some(source_conversation.id.clone());
        store.save_campaign(source_campaign.clone()).unwrap();

        let mut source_instance =
            CharacterInstance::from_definition(source_campaign.id.clone(), &definition);
        source_instance.id = Id::from_str("source-inst");
        source_instance.set_variable("hp", serde_json::json!(42), 9);
        store.add_instance(source_instance.clone()).unwrap();

        let dto = fork_campaign_in_store(
            &store,
            &conv_store,
            source_campaign.id.clone(),
            fork_node.clone(),
            "forked run".into(),
        )
        .unwrap();

        let fork_campaign = store.get_campaign(&Id::from_str(&dto.id)).unwrap();
        assert_eq!(
            fork_campaign.fork_from,
            Some((source_campaign.id.clone(), fork_node.clone()))
        );
        assert_eq!(fork_campaign.card_id, source_campaign.card_id);
        assert_eq!(fork_campaign.current_story_clock(), "Day 7");
        assert_ne!(
            fork_campaign.conversation_id,
            source_campaign.conversation_id
        );

        let fork_conversation = conv_store
            .get(fork_campaign.conversation_id.as_ref().unwrap())
            .unwrap();
        assert_eq!(
            fork_conversation.campaign_id,
            Some(fork_campaign.id.clone())
        );
        assert_eq!(fork_conversation.nodes.len(), 2);
        assert_eq!(fork_conversation.nodes[0].id, user_node);
        assert_eq!(fork_conversation.nodes[1].id, fork_node);

        let source_instances = store.list_instances(&source_campaign.id);
        assert_eq!(source_instances.len(), 1);
        assert_eq!(source_instances[0].id, Id::from_str("source-inst"));

        let fork_instances = store.list_instances(&fork_campaign.id);
        assert_eq!(fork_instances.len(), 1);
        assert_ne!(fork_instances[0].id, source_instance.id);
        assert_eq!(fork_instances[0].name, "Lin");
        assert_eq!(
            fork_instances[0].get_variable("hp"),
            Some(&serde_json::json!(42))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

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
    fn test_fill_regex_context_merges_active_preset_before_scoped_scripts() {
        use storyforge_domain::Source;
        use storyforge_domain::preset::{Preset, RegexScriptSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_fill_regex_context_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let preset_store = preset_store::PresetStore::new(&dir);
        let preset_id = preset_store
            .save(Preset {
                name: "runtime preset".into(),
                prompts: vec![],
                regex_scripts: vec![test_regex_script("preset-regex", RegexScriptSource::Scoped)],
                source: Source::ImportedFromST,
            })
            .unwrap();
        assert!(preset_store.set_active(&preset_id).unwrap());

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.regex_scripts = vec![test_regex_script("scoped-regex", RegexScriptSource::Preset)];

        let global_store = global_regex_store::GlobalRegexStore::new(&dir);
        fill_regex_context(&mut ctx, &preset_store, &global_store);

        let ids: Vec<_> = ctx
            .regex_scripts
            .iter()
            .map(|script| script.id.as_str())
            .collect();
        assert_eq!(ids, vec!["preset-regex", "scoped-regex"]);

        let sources: Vec<_> = ctx
            .regex_scripts
            .iter()
            .map(|script| script.source)
            .collect();
        assert_eq!(
            sources,
            vec![RegexScriptSource::Preset, RegexScriptSource::Scoped]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fill_regex_context_merges_global_before_active_preset_and_scoped() {
        use storyforge_domain::Source;
        use storyforge_domain::preset::{Preset, RegexScriptSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_global_regex_context_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let global_store = global_regex_store::GlobalRegexStore::new(&dir);
        global_store
            .replace_all(vec![test_regex_script(
                "global-regex",
                RegexScriptSource::Scoped,
            )])
            .unwrap();

        let preset_store = preset_store::PresetStore::new(&dir);
        let preset_id = preset_store
            .save(Preset {
                name: "runtime preset".into(),
                prompts: vec![],
                regex_scripts: vec![test_regex_script("preset-regex", RegexScriptSource::Scoped)],
                source: Source::ImportedFromST,
            })
            .unwrap();
        assert!(preset_store.set_active(&preset_id).unwrap());

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.regex_scripts = vec![test_regex_script("scoped-regex", RegexScriptSource::Preset)];

        fill_regex_context(&mut ctx, &preset_store, &global_store);

        let ids: Vec<_> = ctx
            .regex_scripts
            .iter()
            .map(|script| script.id.as_str())
            .collect();
        assert_eq!(ids, vec!["global-regex", "preset-regex", "scoped-regex"]);

        let sources: Vec<_> = ctx
            .regex_scripts
            .iter()
            .map(|script| script.source)
            .collect();
        assert_eq!(
            sources,
            vec![
                RegexScriptSource::Global,
                RegexScriptSource::Preset,
                RegexScriptSource::Scoped
            ]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fill_campaign_runtime_adds_active_card_scoped_regex_after_preset() {
        use storyforge_app_agent::tools::ToolContext;
        use storyforge_domain::Source;
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::character::CharacterCard;
        use storyforge_domain::preset::{Preset, RegexScriptSource};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_campaign_scoped_regex_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let preset_store = preset_store::PresetStore::new(&dir);
        let preset_id = preset_store
            .save(Preset {
                name: "runtime preset".into(),
                prompts: vec![],
                regex_scripts: vec![test_regex_script("preset-regex", RegexScriptSource::Scoped)],
                source: Source::ImportedFromST,
            })
            .unwrap();
        assert!(preset_store.set_active(&preset_id).unwrap());

        let campaign_store = campaign_store::CampaignStore::new(&dir);
        let card = CharacterCard {
            id: Id::from_str("card-campaign"),
            name: "Campaign Card".into(),
            source_character_id: Id::from_str("source-campaign"),
            character_definitions: vec![],
            raw_card_json: serde_json::json!({
                "name": "Campaign Card",
                "extensions": {
                    "regex_scripts": [{
                        "id": "campaign-scoped-regex",
                        "scriptName": "Campaign scoped regex",
                        "findRegex": "foo",
                        "replaceString": "bar",
                        "placement": [2],
                        "disabled": false
                    }]
                }
            }),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        campaign_store.save_card(card.clone()).unwrap();
        let campaign = Campaign::new(card.id.clone(), "Campaign runtime");
        campaign_store.save_campaign(campaign.clone()).unwrap();

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.regex_scripts = vec![test_regex_script(
            "legacy-scoped-regex",
            RegexScriptSource::Preset,
        )];
        let global_store = global_regex_store::GlobalRegexStore::new(&dir);
        global_store
            .replace_all(vec![test_regex_script(
                "global-regex",
                RegexScriptSource::Scoped,
            )])
            .unwrap();
        fill_regex_context(&mut ctx, &preset_store, &global_store);
        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        }));

        fill_campaign_runtime_from_store(&mut ctx, &tool_ctx, &campaign_store, &campaign.id);

        let ids: Vec<_> = ctx
            .regex_scripts
            .iter()
            .map(|script| script.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                "global-regex",
                "preset-regex",
                "legacy-scoped-regex",
                "campaign-scoped-regex"
            ]
        );

        let sources: Vec<_> = ctx
            .regex_scripts
            .iter()
            .map(|script| script.source)
            .collect();
        assert_eq!(
            sources,
            vec![
                RegexScriptSource::Global,
                RegexScriptSource::Preset,
                RegexScriptSource::Scoped,
                RegexScriptSource::Scoped
            ]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_append_campaign_scoped_regex_skips_existing_scoped_id() {
        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.regex_scripts = vec![test_regex_script("same-scoped", RegexScriptSource::Scoped)];

        append_missing_campaign_scoped_regex_scripts(
            &mut ctx,
            vec![
                test_regex_script("same-scoped", RegexScriptSource::Scoped),
                test_regex_script("new-scoped", RegexScriptSource::Scoped),
            ],
        );

        let ids: Vec<_> = ctx
            .regex_scripts
            .iter()
            .map(|script| script.id.as_str())
            .collect();
        assert_eq!(ids, vec!["same-scoped", "new-scoped"]);
    }

    #[tokio::test]
    async fn test_load_preset_for_classification_async_returns_preset() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_preset_classify_async_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: &'static PresetStore = Box::leak(Box::new(PresetStore::new(&dir)));
        let preset_id = store
            .save(storyforge_domain::preset::Preset {
                name: "classify preset".into(),
                prompts: vec![],
                regex_scripts: vec![test_regex_script(
                    "classify-regex",
                    RegexScriptSource::Preset,
                )],
                source: storyforge_domain::Source::ImportedFromST,
            })
            .unwrap();

        let stored = load_preset_for_classification_async(store, preset_id.clone())
            .await
            .unwrap();

        assert_eq!(stored.id, preset_id);
        assert_eq!(stored.preset.name, "classify preset");
        assert_eq!(stored.preset.regex_scripts[0].id, "classify-regex");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_load_preset_for_classification_async_missing_returns_not_found() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_preset_classify_missing_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: &'static PresetStore = Box::leak(Box::new(PresetStore::new(&dir)));

        let err = load_preset_for_classification_async(store, "missing-preset".into())
            .await
            .unwrap_err();

        assert!(matches!(err, TauriCommandError::NotFound { .. }));

        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn test_postprocess_persistence_helper_writes_all_campaign_outputs() {
        use storyforge_domain::agent::{PostProcessResult, VariableUpdate};
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };
        use storyforge_domain::story_task::{
            NewTaskSpec, StoryTask, TaskStatus, TaskTrigger, TaskUpdate,
        };

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_postprocess_persist_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        store.add_instance(lin.clone()).unwrap();

        let existing_task = StoryTask::user_planned(
            campaign.id.clone(),
            "Find key",
            "Find the hidden key",
            vec![TaskTrigger::Manual],
            1,
        );
        let existing_task_id = existing_task.id.clone();
        store.add_task(existing_task).unwrap();

        let persist_ctx = PostprocessPersistContext {
            campaign_id: campaign.id.clone(),
            conversation_id: Id::from_str("conv-1"),
            turn: 3,
        };
        let outcome = storyforge_app_agent::PostProcessOutcome {
            summary: Some("Lin found a clue.".into()),
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![CharacterKnowledgeUpdate {
                    character_id: Id::from_str("Lin"),
                    knowledge_text: "The key is under the mat.".into(),
                    source: KnowledgeSource::Witnessed,
                    source_character_id: None,
                    pinned: false,
                    broadcast: None,
                    propagation: PropagationPolicy::Open,
                }],
                variable_updates: vec![
                    VariableUpdate {
                        instance_id: Some(Id::from_str("Lin")),
                        key: "hp".into(),
                        value: serde_json::json!(7),
                    },
                    VariableUpdate {
                        instance_id: None,
                        key: "story_clock".into(),
                        value: serde_json::json!("Day 2"),
                    },
                ],
                task_updates: vec![
                    TaskUpdate {
                        task_id: Some(existing_task_id.clone()),
                        new_status: TaskStatus::Completed,
                        new_task: None,
                    },
                    TaskUpdate {
                        task_id: None,
                        new_status: TaskStatus::Pending,
                        new_task: Some(NewTaskSpec {
                            title: "Follow the clue".into(),
                            description: "Trace where the key leads.".into(),
                            triggers: vec![TaskTrigger::Manual],
                            related_characters: vec![lin.id.clone()],
                        }),
                    },
                ],
                parse_succeeded: true,
            }),
        };

        persist_postprocess_outcome_to_store(
            &store,
            &persist_ctx,
            &outcome,
            &[String::from("Lin")],
        );

        let summaries = store.list_summaries(&campaign.id);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].conversation_id, Id::from_str("conv-1"));
        assert_eq!(summaries[0].turn, 3);
        assert_eq!(summaries[0].content, "Lin found a clue.");

        let knowledge = store.list_knowledge(&campaign.id);
        assert_eq!(knowledge.len(), 1);
        assert_eq!(knowledge[0].character_id, lin.id);
        assert_eq!(knowledge[0].turn_number, 3);
        assert_eq!(knowledge[0].knowledge_text, "The key is under the mat.");

        let updated_lin = store
            .get_instance(&campaign.id, &Id::from_str("inst-lin"))
            .unwrap();
        let hp = updated_lin.get_variable("hp").unwrap();
        assert_eq!(hp, &serde_json::json!(7));

        let updated_campaign = store.get_campaign(&campaign.id).unwrap();
        assert_eq!(updated_campaign.current_story_clock(), "Day 2");

        let updated_task = store.get_task(&existing_task_id).unwrap();
        assert!(matches!(updated_task.status, TaskStatus::Completed));

        let tasks = store.list_tasks(&campaign.id);
        assert!(tasks.iter().any(|task| task.title == "Follow the clue"
            && task.description == "Trace where the key leads."
            && task.created_turn == 3));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn postprocess_skips_unknown_character_target() {
        use storyforge_domain::agent::{PostProcessResult, VariableUpdate};
        use storyforge_domain::campaign::{Campaign, CharacterInstance};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_unknown_char_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        store.add_instance(lin.clone()).unwrap();

        let persist_ctx = PostprocessPersistContext {
            campaign_id: campaign.id.clone(),
            conversation_id: Id::from_str("conv-1"),
            turn: 2,
        };

        // outcome 包含一个未知角色名 + 一个已知角色/全局变量
        let outcome = storyforge_app_agent::PostProcessOutcome {
            summary: None,
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![],
                variable_updates: vec![
                    VariableUpdate {
                        instance_id: Some(Id::from_str("UnknownChar")),
                        key: "level".into(),
                        value: serde_json::json!(99),
                    },
                    VariableUpdate {
                        instance_id: None,
                        key: "story_clock".into(),
                        value: serde_json::json!("Night"),
                    },
                ],
                task_updates: vec![],
                parse_succeeded: true,
            }),
        };

        persist_postprocess_outcome_to_store(
            &store,
            &persist_ctx,
            &outcome,
            &[String::from("Lin")],
        );

        // 未知角色变量不写入（未报错即确认静默跳过）
        let lin_check = store
            .get_instance(&campaign.id, &Id::from_str("inst-lin"))
            .unwrap();
        assert!(
            lin_check.get_variable("level").is_none(),
            "未知角色的变量不应写入任何 instance"
        );

        // 全局变量仍正常写入
        let updated_campaign = store.get_campaign(&campaign.id).unwrap();
        assert_eq!(updated_campaign.current_story_clock(), "Night");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn postprocess_validates_task_belongs_to_campaign() {
        use storyforge_domain::agent::PostProcessResult;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::story_task::{StoryTask, TaskStatus, TaskTrigger, TaskUpdate};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_task_campaign_mismatch_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        // 从当前 campaign
        let campaign = Campaign::new(Id::from_str("card-main"), "Current Campaign");
        store.save_campaign(campaign.clone()).unwrap();
        let inst = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        store.add_instance(inst).unwrap();

        // 创建一个归属于不同 campaign 的任务（模拟写回时引用了其他 campaign 的任务）
        let other_camp_id = Id::from_str("other-campaign");
        let other_task = StoryTask::user_planned(
            other_camp_id.clone(),
            "Intrude",
            "Intrude other campaign",
            vec![TaskTrigger::Manual],
            1,
        );
        let other_task_id = other_task.id.clone();
        store.add_task(other_task).unwrap();

        let persist_ctx = PostprocessPersistContext {
            campaign_id: campaign.id.clone(),
            conversation_id: Id::from_str("conv-1"),
            turn: 1,
        };

        let outcome = storyforge_app_agent::PostProcessOutcome {
            summary: None,
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![],
                variable_updates: vec![],
                task_updates: vec![TaskUpdate {
                    task_id: Some(other_task_id.clone()),
                    new_status: TaskStatus::Completed,
                    new_task: None,
                }],
                parse_succeeded: true,
            }),
        };

        persist_postprocess_outcome_to_store(&store, &persist_ctx, &outcome, &[]);

        // 其他 campaign 的任务状态不应被本 campaign 的写回修改
        let stored_task = store.get_task(&other_task_id).unwrap();
        assert!(
            matches!(stored_task.status, TaskStatus::Pending),
            "其他 campaign 的任务不应被当前 campaign 的写回修改：{stored_task:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn postprocess_empty_present_chars_rejects_witnessed_knowledge() {
        use storyforge_domain::agent::PostProcessResult;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_empty_present_witnessed_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let campaign = Campaign::new(Id::from_str("card-1"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        store.add_instance(lin.clone()).unwrap();

        let persist_ctx = PostprocessPersistContext {
            campaign_id: campaign.id.clone(),
            conversation_id: Id::from_str("conv-1"),
            turn: 1,
        };

        // Witnessed 知识 + present_chars 空集：知识路径收紧拒绝写入
        let outcome = storyforge_app_agent::PostProcessOutcome {
            summary: None,
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![CharacterKnowledgeUpdate {
                    character_id: Id::from_str("Lin"),
                    knowledge_text: "The key is under the mat.".into(),
                    source: KnowledgeSource::Witnessed,
                    source_character_id: None,
                    pinned: false,
                    broadcast: None,
                    propagation: PropagationPolicy::Open,
                }],
                variable_updates: vec![],
                task_updates: vec![],
                parse_succeeded: true,
            }),
        };

        persist_postprocess_outcome_to_store(&store, &persist_ctx, &outcome, &[]);

        let knowledge = store.list_knowledge(&campaign.id);
        assert!(
            knowledge.is_empty(),
            "present_chars 空集时 Witnessed 知识不应写入"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_postprocess_variable_keys_include_runtime_custom_schema_and_values() {
        use std::collections::HashMap;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::variables::{
            VariableField, VariableType, VariableValue, default_character_variables,
        };

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        let mut campaign = Campaign::new(Id::from_str("card-keys"), "Key Campaign");
        campaign.variables.push(VariableValue::new(
            "alarm_level",
            serde_json::json!("red"),
            2,
        ));

        let mut schema = default_character_variables();
        schema.push(VariableField {
            key: "stress".into(),
            label: "Stress".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(0),
            description: None,
            group: Some("state".into()),
        });
        let definition = CharacterDefinition {
            id: Id::from_str("def-keys"),
            card_id: Id::from_str("card-keys"),
            name: "Lin".into(),
            persona_prompt: String::new(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: schema,
        };
        let mut instance = CharacterInstance::from_definition(campaign.id.clone(), &definition);
        instance.variables.push(VariableValue::new(
            "temporary_flag",
            serde_json::json!(true),
            2,
        ));
        let mut definitions_by_id = HashMap::new();
        definitions_by_id.insert(definition.id.clone(), definition);

        ctx.campaign_runtime = Some(Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id,
            knowledge: vec![],
            tasks: vec![],
            turn: 2,
        }));

        let keys = postprocess_variable_keys(&ctx);

        for expected in [
            "hp",
            "story_clock",
            "weather",
            "stress",
            "alarm_level",
            "temporary_flag",
        ] {
            assert!(
                keys.contains(&expected.to_string()),
                "missing key {expected}"
            );
        }
        assert_eq!(keys.iter().filter(|key| key.as_str() == "hp").count(), 1);
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
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
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
        let persist_ctx = TemporaryInstancesPersistContext::from_writing_context(&ctx).unwrap();

        let temps = vec![
            CharacterInstance::temporary(campaign.id.clone(), "Ghost"),
            CharacterInstance::temporary_with_overrides(
                campaign.id.clone(),
                "Guard",
                Some("stern guard".into()),
                Some("block the way".into()),
            ),
        ];

        persist_temporary_instances_to_store(&store, &persist_ctx, &temps);

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

    /// Campaign context snapshots apply the same runtime view to writing and tools.
    #[test]
    fn test_campaign_context_snapshot_applies_runtime_to_contexts() {
        use storyforge_domain::agent::RoundSummary;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
        use storyforge_domain::story_task::{StoryTask, TaskTrigger};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_campaign_snapshot_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let mut campaign = Campaign::new(Id::from_str("card-1"), "run");
        campaign.story_clock = "Day 3".into();
        store.save_campaign(campaign.clone()).unwrap();

        let instance = CharacterInstance::temporary(campaign.id.clone(), "Ghost");
        store.add_instance(instance.clone()).unwrap();
        store
            .add_knowledge(vec![CharacterKnowledgeEntry::witnessed(
                campaign.id.clone(),
                instance.id.clone(),
                "Ghost saw the gate",
                1,
            )])
            .unwrap();
        store
            .add_task(StoryTask::user_planned(
                campaign.id.clone(),
                "Open the gate",
                "The gate must open later",
                vec![TaskTrigger::TurnReminder { at_turn: 2 }],
                1,
            ))
            .unwrap();
        store
            .add_summary(RoundSummary::new(
                campaign.id.clone(),
                Id::new(),
                1,
                "A previous turn happened".into(),
            ))
            .unwrap();

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        }));

        let snapshot = load_campaign_context_snapshot(&store, &campaign.id).unwrap();
        apply_campaign_context_snapshot(&mut ctx, &tool_ctx, snapshot);

        assert_eq!(ctx.campaign_id, Some(campaign.id.clone()));
        assert_eq!(ctx.story_clock, "Day 3");
        assert_eq!(ctx.turn, 2);
        assert_eq!(ctx.pending_tasks.len(), 1);
        let runtime = ctx.campaign_runtime.as_ref().unwrap();
        assert_eq!(runtime.instances.len(), 1);
        assert_eq!(runtime.knowledge.len(), 1);
        assert_eq!(runtime.tasks.len(), 1);
        assert_eq!(runtime.turn, 2);

        let tool_runtime = tool_ctx
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .campaign_runtime
            .clone()
            .unwrap();
        assert_eq!(tool_runtime.campaign.id, campaign.id);
        assert_eq!(tool_runtime.instances[0].id, instance.id);

        let _ = std::fs::remove_dir_all(&dir);
    }

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
                extraction_status:
                    storyforge_domain::character::CharacterExtractionStatus::Extracted,
                extraction_message: None,
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

        let state = AppState::new_for_test();
        let proposals =
            meta_propose_campaign_repairs_in_store(&store, campaign.id.as_str(), &state).unwrap();
        assert!(
            !proposals.is_empty(),
            "command helper should return patches"
        );
        {
            let typed = state
                .typed_patches
                .read()
                .unwrap_or_else(|p| p.into_inner());
            assert!(
                typed.iter().any(|p| {
                    p.actions.iter().any(|a| {
                        matches!(
                            a,
                            storyforge_app_meta::TypedPatchAction::DeleteOrphanKnowledge { .. }
                        )
                    })
                }),
                "command helper should persist a delete orphan knowledge patch"
            );
            assert_eq!(typed.len(), proposals.len());
        }

        let repeated =
            meta_propose_campaign_repairs_in_store(&store, campaign.id.as_str(), &state).unwrap();
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        assert_eq!(repeated.len(), proposals.len());
        assert_eq!(
            typed.len(),
            proposals.len(),
            "repeated repair proposal should return existing pending patches without duplicating them"
        );

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
                extraction_status:
                    storyforge_domain::character::CharacterExtractionStatus::Extracted,
                extraction_message: None,
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

        let state = AppState::new_for_test();
        let patch = storyforge_app_meta::TypedPatch {
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

        {
            let mut typed = state
                .typed_patches
                .write()
                .unwrap_or_else(|p| p.into_inner());
            typed.push(patch);
        }

        let preview = meta_preview_typed_patch_in_store(
            &store,
            "test-prune-patch",
            campaign.id.as_str(),
            &state,
        )
        .unwrap();
        assert_eq!(preview["stale"], serde_json::json!(false));

        meta_accept_typed_patch_in_store(&store, "test-prune-patch", campaign.id.as_str(), &state)
            .unwrap();

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
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            typed
                .iter()
                .find(|p| p.id == "test-prune-patch")
                .unwrap()
                .status,
            storyforge_app_meta::TypedPatchStatus::Accepted
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
                extraction_status:
                    storyforge_domain::character::CharacterExtractionStatus::Extracted,
                extraction_message: None,
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

    #[test]
    fn test_meta_accept_typed_patch_preflights_all_actions_before_writing() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::variables::default_character_variables;

        let dir =
            std::env::temp_dir().join(format!("sf_test_accept_preflight_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let card = {
            let mut c = storyforge_domain::character::CharacterCard {
                id: Id::from_str("card-1"),
                name: "测试卡".into(),
                source_character_id: Id::from_str("src-1"),
                character_definitions: vec![],
                raw_card_json: serde_json::Value::Null,
                extraction_status:
                    storyforge_domain::character::CharacterExtractionStatus::Extracted,
                extraction_message: None,
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
        store.add_instance(instance).unwrap();

        let state = AppState::new_for_test();
        let patch = storyforge_app_meta::TypedPatch {
            id: "test-preflight-patch".into(),
            description: "先校验所有 action 再写盘".into(),
            source_issue_category: "preflight".into(),
            affected_id: Some(campaign.id.as_str().to_string()),
            actions: vec![
                storyforge_app_meta::TypedPatchAction::UpdateCampaignVariable {
                    key: "preflight_marker".into(),
                    value: serde_json::json!("should-not-write"),
                },
                storyforge_app_meta::TypedPatchAction::RepointInstanceDefinition {
                    instance_id: Id::from_str("target-inst"),
                    new_definition_id: Some(Id::from_str("missing-definition")),
                },
            ],
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

        let err = meta_accept_typed_patch_in_store(
            &store,
            "test-preflight-patch",
            campaign.id.as_str(),
            &state,
        )
        .expect_err("invalid later action should fail before any write");
        assert!(
            err.to_string().contains("Definition 不存在"),
            "unexpected error: {:?}",
            err
        );

        let campaign_after = store.get_campaign(&campaign.id).unwrap();
        assert_eq!(campaign_after.get_variable("preflight_marker"), None);
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            typed
                .iter()
                .find(|p| p.id == "test-preflight-patch")
                .unwrap()
                .status,
            storyforge_app_meta::TypedPatchStatus::Stale
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_meta_accept_typed_patch_rejects_stale_definition_binding() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::variables::{VariableField, VariableType, VariableValue};

        let dir =
            std::env::temp_dir().join(format!("sf_test_accept_def_stale_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let card = {
            let mut c = storyforge_domain::character::CharacterCard {
                id: Id::from_str("card-1"),
                name: "测试卡".into(),
                source_character_id: Id::from_str("src-1"),
                character_definitions: vec![],
                raw_card_json: serde_json::Value::Null,
                extraction_status:
                    storyforge_domain::character::CharacterExtractionStatus::Extracted,
                extraction_message: None,
            };
            c.character_definitions.push(CharacterDefinition {
                id: Id::from_str("def-1"),
                card_id: c.id.clone(),
                name: "Old".into(),
                persona_prompt: String::new(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: None,
                role_type: RoleType::Protagonist,
                variable_schema: vec![VariableField {
                    key: "old_hp".into(),
                    label: "Old HP".into(),
                    value_type: VariableType::Int,
                    default: serde_json::json!(10),
                    description: None,
                    group: None,
                }],
            });
            c.character_definitions.push(CharacterDefinition {
                id: Id::from_str("def-2"),
                card_id: c.id.clone(),
                name: "New".into(),
                persona_prompt: String::new(),
                behavior_rules: String::new(),
                base_backstory: vec![],
                group: None,
                role_type: RoleType::Supporting,
                variable_schema: vec![VariableField {
                    key: "new_hp".into(),
                    label: "New HP".into(),
                    value_type: VariableType::Int,
                    default: serde_json::json!(20),
                    description: None,
                    group: None,
                }],
            });
            c
        };
        store.save_card(card).unwrap();

        let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
        store.save_campaign(campaign.clone()).unwrap();
        let definitions = store
            .get_card(&campaign.card_id)
            .unwrap()
            .card
            .character_definitions;
        let mut instance = CharacterInstance::from_definition(campaign.id.clone(), &definitions[1]);
        instance.id = Id::from_str("target-inst");
        instance.definition_id = Some(Id::from_str("def-2"));
        instance.variables = vec![VariableValue::new("new_hp", serde_json::json!(20), 0)];
        store.add_instance(instance).unwrap();

        let state = AppState::new_for_test();
        let patch = storyforge_app_meta::TypedPatch {
            id: "test-stale-definition-patch".into(),
            description: "旧定义变量同步".into(),
            source_issue_category: "variable_schema_mismatch".into(),
            affected_id: Some("target-inst".into()),
            actions: vec![
                storyforge_app_meta::TypedPatchAction::SyncInstanceVariables {
                    instance_id: Id::from_str("target-inst"),
                    definition_id: Id::from_str("def-1"),
                    add_keys: vec!["old_hp".into()],
                    remove_keys: vec!["new_hp".into()],
                },
            ],
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

        let err = meta_accept_typed_patch_in_store(
            &store,
            "test-stale-definition-patch",
            campaign.id.as_str(),
            &state,
        )
        .expect_err("definition mismatch should stale the patch before writing");
        assert!(
            err.to_string().contains("已不再使用 definition"),
            "unexpected error: {:?}",
            err
        );

        let updated = store
            .get_instance(&campaign.id, &Id::from_str("target-inst"))
            .unwrap();
        assert!(updated.get_variable("new_hp").is_some());
        assert!(updated.get_variable("old_hp").is_none());
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            typed
                .iter()
                .find(|p| p.id == "test-stale-definition-patch")
                .unwrap()
                .status,
            storyforge_app_meta::TypedPatchStatus::Stale
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_typed_patch_pending_dedupe_canonicalizes_unordered_actions() {
        let make_patch = |add_keys: Vec<&str>, remove_keys: Vec<&str>, orphan_ids: Vec<&str>| {
            storyforge_app_meta::TypedPatch {
                id: uuid::Uuid::new_v4().to_string(),
                description: "dedupe".into(),
                source_issue_category: "variable_schema_mismatch".into(),
                affected_id: Some("target-inst".into()),
                actions: vec![
                    storyforge_app_meta::TypedPatchAction::SyncInstanceVariables {
                        instance_id: Id::from_str("target-inst"),
                        definition_id: Id::from_str("def-1"),
                        add_keys: add_keys.into_iter().map(str::to_string).collect(),
                        remove_keys: remove_keys.into_iter().map(str::to_string).collect(),
                    },
                    storyforge_app_meta::TypedPatchAction::PruneOrphanTaskReferences {
                        task_id: Id::from_str("task-1"),
                        orphan_character_ids: orphan_ids.into_iter().map(Id::from_str).collect(),
                    },
                ],
                diff: vec![],
                created_at: chrono::Utc::now(),
                status: storyforge_app_meta::TypedPatchStatus::Pending,
            }
        };

        let existing = make_patch(vec!["mana", "hp"], vec!["legacy"], vec!["b", "a"]);
        let proposed = make_patch(vec!["hp", "mana"], vec!["legacy"], vec!["a", "b"]);

        assert!(is_same_pending_typed_patch(&existing, &proposed));
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

        meta_dismiss_typed_patch_in_state("test-dismiss-patch", state.as_ref()).unwrap();

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

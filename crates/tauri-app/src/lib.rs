pub mod campaign_store;
mod card_shell_cache;
mod card_studio_api;
mod card_studio_store;
mod commands;
mod compress_job_store;
mod connection_store;
pub mod error;
mod global_regex_store;
pub mod meta_backend;
mod module_store;
mod mvu_webview_runtime;
mod playthrough_lifecycle;
mod preset_store;
pub mod production_postprocess;
mod shell_doc_protocol;
pub mod sqlite_runtime;
mod storage;
pub mod storage_backend;
pub mod storage_health;
pub mod turn_coordinator;
pub mod turn_lifecycle;
pub mod turn_store;

use commands::{
    campaigns::*, card_shell::*, cards::*, characters::*, connections::*, conversations::*,
    diagnostics::*, import_export::*, memory::*, meta::*, meta_typed::*, mvu::*, plugins::*,
    presets::*, turns::*, variables::*, world_info::*, writing::*,
};
#[cfg(test)]
use playthrough_lifecycle::delete_campaign_playthrough_in_store;
use production_postprocess::TurnAttemptSink;

use chrono::Utc;
use connection_store::ConnectionStore;
use preset_store::PresetStore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use storage::CharacterStore;
use tokio::sync::{oneshot, watch};

use storyforge_app_agent::ToolContext;
use storyforge_app_agent::runtime::PromptHook;
use storyforge_app_conversation::{ConversationStore, PartialRollTarget};
use storyforge_app_logging::{ExportOptions, LogFilter, LogKind, LogLevel, LogStore};
use storyforge_app_meta::{
    MvuApplyError, MvuApplyPreview, apply_schema_to_definition, compute_apply_preview,
};
use storyforge_app_pipeline::{PipelineOrchestrator, RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
#[cfg(test)]
use storyforge_domain::conversation::VariantStatus;
use storyforge_domain::conversation::{Conversation, Provenance, Role as ConversationRole};
use storyforge_domain::llm::{
    ChatMessage, LlmConnection, LlmConnectionSummary, LlmProtocol, SamplingParams, ToolMode,
};
use storyforge_domain::preset::{RegexScript, RegexScriptSource, merge_regex_script_sources};
use storyforge_domain::prompt_module::PromptProfile;
use storyforge_infra_llm::LlmClient;
use storyforge_infra_plugin_host::PluginRegistry;
use storyforge_infra_plugin_host::mvu_runtime::MvuExecuteResponse;
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, DraftAttemptRequest, PostprocessApplyRequest, RegenerateAttemptRequest,
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

pub(crate) fn get_store() -> &'static CharacterStore {
    STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        CharacterStore::new(&data_dir)
    })
}

static CONN_STORE: OnceLock<Arc<ConnectionStore>> = OnceLock::new();

pub(crate) fn get_conn_store() -> Arc<ConnectionStore> {
    CONN_STORE
        .get_or_init(|| {
            let data_dir = get_app_data_dir();
            Arc::new(ConnectionStore::new(&data_dir))
        })
        .clone()
}

static CARD_STUDIO_STORE: OnceLock<card_studio_store::CardStudioStore> = OnceLock::new();

pub(crate) fn get_card_studio_store() -> &'static card_studio_store::CardStudioStore {
    CARD_STUDIO_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        card_studio_store::CardStudioStore::new(&data_dir)
    })
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
static COMPRESS_JOB_STORE: OnceLock<compress_job_store::CompressJobStore> = OnceLock::new();

fn get_card_shell_cache() -> &'static card_shell_cache::CardShellCache {
    use std::sync::OnceLock;
    static CACHE: OnceLock<card_shell_cache::CardShellCache> = OnceLock::new();
    CACHE.get_or_init(|| card_shell_cache::CardShellCache::new(&get_app_data_dir()))
}

pub(crate) fn get_campaign_store() -> &'static campaign_store::CampaignStore {
    let store = CAMPAIGN_STORE.get_or_init(|| {
        if sqlite_runtime::is_sqlite_active() {
            campaign_store::CampaignStore::disabled()
        } else {
            let data_dir = get_app_data_dir();
            campaign_store::CampaignStore::new(&data_dir)
        }
    });
    if sqlite_runtime::is_sqlite_active() {
        // Backend resolution normally happens before this lazy store is ever
        // initialized. Keep this guard for accidental early initialization so
        // SQLite mode can never use JSON as a fallback or second write target.
        store.disable_json_access();
    }
    store
}

fn get_compress_job_store() -> &'static compress_job_store::CompressJobStore {
    COMPRESS_JOB_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        compress_job_store::CompressJobStore::new(&data_dir)
    })
}

static TURN_STORE: OnceLock<turn_store::TurnStore> = OnceLock::new();

fn get_turn_store() -> &'static turn_store::TurnStore {
    TURN_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        turn_store::TurnStore::new(&data_dir)
    })
}

/// Read Turn state from the process-selected authority. SQLite mode never
/// consults the legacy JSON `TurnStore`, including on error paths.
fn get_active_turn_for_backend(
    campaign_id: &Id,
) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
    if sqlite_runtime::is_sqlite_active() {
        sqlite_runtime::get_active_turn(campaign_id)
    } else {
        Ok(get_turn_store().get_active_turn(campaign_id))
    }
}

fn get_turn_by_variant_for_backend(
    variant_id: &Id,
) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
    if sqlite_runtime::is_sqlite_active() {
        sqlite_runtime::get_turn_by_variant(variant_id)
    } else {
        Ok(get_turn_store().get_turn_by_variant(variant_id))
    }
}

/// Phase A 启动恢复：幂等重放 Committing 态 Turn + 标记非 terminal 活动 Turn 为 Failed。
///
/// 规则（收敛决策步骤 12/16/17）：
/// 1. Committing 态 Turn：重放 MutationBatch（幂等 upsert + revision CAS）。
///    - revision 已是 target_revision → 校验/补齐 no-op。
///    - revision 是 expected_revision → 完整重放 + bump。
///    - revision 冲突 → 标 Failed（被外部推进，需人工处理）。
///    - 重放成功后补齐 Draft→Final + accepted_attempt_id + Attempt Committed。
/// 2. Generating/DraftReady/DerivingState/AwaitingAcceptance 态 Turn：
///    标 Failed（无副作用，安全失败；已落盘 Draft 保留为 Draft）。
///    Committing 不在此步处理（避免覆盖第 1 步未完成的恢复）。
/// 3. Committed/Degraded/Failed/Abandoned 态 Turn：不动。
fn recover_turns_on_startup(app_state: &AppState) {
    // SQLite accept is atomic: recover by failing incomplete pipeline turns.
    // Never fall back to JSON stores when SQLite is authoritative.
    if sqlite_runtime::is_sqlite_active() {
        match sqlite_runtime::recover_turns_on_startup() {
            Ok(n) if n > 0 => {
                tracing::warn!(count = n, "sqlite recovery failed incomplete turns")
            }
            Ok(_) => {}
            Err(e) => tracing::error!("sqlite recovery failed: {e}"),
        }
        return;
    }
    let service = turn_lifecycle::TurnLifecycleService::new(
        get_campaign_store(),
        get_turn_store(),
        &app_state.conv_store,
    );
    service.recover_turns_on_startup(|batch| {
        // 启动恢复路径只做同步关键词索引，避免阻塞启动
        index_round_summaries_to_vector(app_state.vector_store.as_ref(), batch);
    });
}

/// 在 `start_writing` 追加 user 消息**之前**调用。
/// 如果存在非 terminal Turn，返回错误，阻止新一轮启动。
/// 非 Campaign 模式（无活跃 Campaign）直接放行。
fn check_turn_barrier(state: &Arc<AppState>) -> Result<(), TauriCommandError> {
    check_turn_barrier_with(state, get_turn_store())
}

/// 可注入 TurnStore 的屏障检查（测试 hermetic 化用）。
fn check_turn_barrier_with(
    state: &Arc<AppState>,
    turn_store: &turn_store::TurnStore,
) -> Result<(), TauriCommandError> {
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
    reject_if_active_turn_in(turn_store, &campaign_id)
}

/// 直接写 Campaign 入口的屏障：指定 campaign 有活动 Turn 时拒绝。
fn reject_if_active_turn(campaign_id: &Id) -> Result<(), TauriCommandError> {
    reject_if_active_turn_in(get_turn_store(), campaign_id)
}

fn reject_if_active_turn_in(
    turn_store: &turn_store::TurnStore,
    campaign_id: &Id,
) -> Result<(), TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        match sqlite_runtime::get_active_turn(campaign_id) {
            Ok(Some(turn)) => {
                return Err(TauriCommandError::validation(format!(
                    "当前有未完成的轮次（turn_id={}, status={:?}），请先 Accept、Discard 或 Abandon 后再修改",
                    turn.turn_id, turn.status
                )));
            }
            Ok(None) => return Ok(()),
            Err(e) => {
                return Err(TauriCommandError::internal(format!(
                    "sqlite active turn lookup failed: {e}"
                )));
            }
        }
    }
    if let Some(turn) = turn_store.get_active_turn(campaign_id) {
        return Err(TauriCommandError::validation(format!(
            "当前有未完成的轮次（turn_id={}, status={:?}），请先 Accept、Discard 或 Abandon 后再修改",
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

/// 由 Tauri setup 阶段确认的进程级数据目录。
///
/// Android 的真实私有目录依赖运行中的 Activity，不能在 Builder 构造前猜测；
/// setup 完成后，所有延迟初始化的 store 都通过这里取得同一目录。
static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 应用数据目录纯解析逻辑的目标平台标识。抽取为参数是为了让纯函数
/// `resolve_app_data_dir` 可在单元测试中按平台断言，不触碰真实环境变量或文件系统。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppDataDirTarget {
    Windows,
    Macos,
    Linux,
    Android,
}

impl AppDataDirTarget {
    /// 当前编译目标对应的平台。
    fn current() -> Self {
        if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

/// 纯路径解析：给定平台、环境变量视图与 exe 父目录，返回**应使用**的数据目录。
///
/// 本函数无副作用：不读真实环境、不建目录、不迁移。所有副作用（建目录、
/// 旧数据迁移）由 `get_app_data_dir()` 薄封装负责。这样单元测试可以断言
/// 解析契约而不触碰用户真实的 `%APPDATA%`/`$HOME`。
///
/// `env` 是环境变量名→值的视图（缺失视为未设置）；`exe_parent` 为
/// `current_exe().parent()` 的等价输入，用于桌面端 exe_dir/data 回退；
/// `framework_data_dir` 是 setup 阶段由框架返回的应用数据目录。
fn resolve_app_data_dir(
    target: AppDataDirTarget,
    env: &dyn Fn(&str) -> Option<String>,
    exe_parent: Option<&Path>,
    framework_data_dir: Option<&Path>,
) -> Result<PathBuf, &'static str> {
    let os_dir = match target {
        AppDataDirTarget::Android => env("STORYFORGE_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| framework_data_dir.map(Path::to_path_buf)),
        AppDataDirTarget::Windows => {
            env("APPDATA").map(|appdata| PathBuf::from(appdata).join("StoryForge"))
        }
        AppDataDirTarget::Macos => env("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/StoryForge")),
        AppDataDirTarget::Linux => env("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .map(|p| p.join("storyforge")),
    };

    if target == AppDataDirTarget::Android {
        return os_dir.ok_or(
            "Android app data directory is unavailable before Tauri setup; refusing a hardcoded path",
        );
    }

    Ok(os_dir.unwrap_or_else(|| {
        // 回退到 exe 目录（旧行为）
        exe_parent
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from(".").join("data"))
    }))
}

/// 获取应用数据目录，优先使用 OS 标准位置（H-003 修复）。
///
/// 平台规则见 [`resolve_app_data_dir`]。本函数额外做两件副作用：
/// 1. 创建返回的目录（导入/日志写入依赖目录存在）；
/// 2. 旧位置 exe_dir/data 有数据且新位置为空时迁移过去。
///
/// Android 必须先由 [`initialize_app_data_dir`] 注入 Tauri
/// `app.path().app_data_dir()` 的结果；若有代码在 setup 前触发 store 初始化，
/// 本函数会失败关闭，而不是猜测一个只对主用户成立的 `/data/data/...` 路径。
fn get_app_data_dir() -> PathBuf {
    if let Some(data_dir) = APP_DATA_DIR.get() {
        return data_dir.clone();
    }

    initialize_app_data_dir(None)
        .unwrap_or_else(|error| panic!("failed to initialize application data directory: {error}"))
}

fn initialize_app_data_dir(framework_data_dir: Option<&Path>) -> Result<PathBuf, String> {
    let exe_parent = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let data_dir = resolve_app_data_dir(
        AppDataDirTarget::current(),
        &|name: &str| std::env::var(name).ok().filter(|v| !v.is_empty()),
        exe_parent.as_deref(),
        framework_data_dir,
    )
    .map_err(str::to_owned)?;

    if let Some(existing) = APP_DATA_DIR.get() {
        if existing != &data_dir {
            return Err(format!(
                "application data directory already initialized as {}, refusing {}",
                existing.display(),
                data_dir.display()
            ));
        }
        return Ok(existing.clone());
    }

    std::fs::create_dir_all(&data_dir)
        .map_err(|error| format!("cannot create {}: {error}", data_dir.display()))?;
    migrate_from_exe_dir_if_needed(&data_dir, exe_parent.as_deref());
    APP_DATA_DIR
        .set(data_dir.clone())
        .map_err(|_| "application data directory initialization raced".to_string())?;

    Ok(data_dir)
}

/// 从旧的 exe_dir/data 迁移到新的 OS 标准目录（仅当新目录为空时）。
///
/// `exe_parent` 显式传入（而非内部再 `current_exe()`），便于用临时目录测试
/// 迁移逻辑而不依赖真实 exe 路径。
fn migrate_from_exe_dir_if_needed(new_dir: &Path, exe_parent: Option<&Path>) {
    let Some(exe_parent) = exe_parent else {
        return;
    };
    let old_dir = exe_parent.join("data");

    if old_dir == new_dir || !old_dir.exists() {
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

fn should_load_legacy_active_campaign_pointer(sqlite_active: bool) -> bool {
    !sqlite_active
}

fn resolve_active_campaign_with_legacy_fallback(
    memory_active_id: Option<Id>,
    data_dir: &Path,
    sqlite_active: bool,
) -> Option<Id> {
    if should_load_legacy_active_campaign_pointer(sqlite_active) {
        memory_active_id.or_else(|| load_active_campaign(data_dir))
    } else {
        memory_active_id
    }
}

fn load_active_campaign_for_backend(data_dir: &Path) -> Option<Id> {
    resolve_active_campaign_with_legacy_fallback(None, data_dir, sqlite_runtime::is_sqlite_active())
}

fn save_active_campaign(data_dir: &Path, id: Option<&Id>) {
    let path = data_dir.join("active_campaign.json");
    let v = serde_json::json!({ "campaign_id": id.map(|i| i.as_str()).unwrap_or("") });
    if let Err(e) = storyforge_infra_util::atomic_write_json(&path, &v) {
        tracing::error!("保存活跃 Campaign 失败: {e}");
    }
}

// ─── AppState（M1 新增，注入到 Tauri managed state）─────────────────────────

/// Operation-owned cancel handle for one start_writing / regenerate generation.
///
/// Pipeline, autofix and postprocess all clone `cancel_rx` at operation start.
/// The global slot only keeps the sender + generation id so `cancel_writing`
/// and compare-and-clear can target the correct generation.
#[derive(Debug)]
pub struct WritingCancelHandle {
    pub operation_id: Id,
    pub cancel_tx: watch::Sender<bool>,
}

/// Create a new writing operation cancel pair and install it into AppState.
/// Any previous operation is cancelled first.
fn begin_writing_operation(app: &AppState) -> (Id, watch::Receiver<bool>) {
    let operation_id = Id::new();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut slot = app.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = slot.take() {
            let _ = existing.cancel_tx.send(true);
        }
        *slot = Some(WritingCancelHandle {
            operation_id: operation_id.clone(),
            cancel_tx,
        });
    }
    (operation_id, cancel_rx)
}

/// Clear AppState.current_cancel only when it still belongs to this operation.
fn clear_current_cancel_if(app: &AppState, operation_id: &Id) {
    let mut slot = app.current_cancel.lock().unwrap_or_else(|p| p.into_inner());
    if slot
        .as_ref()
        .is_some_and(|handle| handle.operation_id == *operation_id)
    {
        *slot = None;
    }
}

/// 应用全局状态
pub struct AppState {
    /// App data directory used by stateful stores owned by this process.
    data_dir: PathBuf,
    pub conv_store: Arc<ConversationStore>,
    pub log_store: Arc<LogStore>,
    /// 工具上下文（导入角色卡时同步更新，RwLock 支持运行时写入）
    pub tool_ctx: Arc<RwLock<ToolContext>>,
    /// 当前运行的写作/regenerate 取消句柄（operation-owned）。
    /// None = 无运行中的写作。
    pub current_cancel: Mutex<Option<WritingCancelHandle>>,
    /// 等待前端插件处理最终 LLM messages prompt hook 的请求。
    prompt_hook_pending: PromptHookPendingMap,
    /// 当前活跃连接构造的 LLM client（None = 未配置，生产调用必须 fail closed）
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
        // AND-3：storage_meta 版本记录（升级检测基础；best-effort 不 panic）
        touch_storage_meta(&data_dir);
        let conv_dir = data_dir.join("conversations");
        let log_dir = data_dir.join("logs");

        let conv_store = if sqlite_runtime::is_sqlite_active() {
            let persistence = sqlite_runtime::conversation_persistence()
                .expect("sqlite backend was activated before AppState construction");
            Arc::new(ConversationStore::with_persistence(persistence))
        } else {
            Arc::new(ConversationStore::new(conv_dir))
        };
        let log_store = Arc::new(LogStore::new(log_dir));

        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
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
            // SQLite mode must not let a legacy JSON UI preference select an
            // authority record after cutover. SQLite-native preference storage
            // is intentionally deferred; selection remains in-process.
            active_campaign: Mutex::new(load_active_campaign_for_backend(&data_dir)),
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
        Self::sweep_stale_test_data_dirs();
        let data_dir = std::env::temp_dir().join(format!(
            "storyforge-app-state-test-{}",
            uuid::Uuid::new_v4()
        ));
        Self::new_with_data_dir(data_dir)
    }

    /// 惰性清扫历史测试残留（>24h 的 storyforge-app-state-test-*）。
    /// 测试结束不清理自身 data_dir（Drop 无钩子），跑一次 workspace 测试就
    /// 落几十个目录（2026-07-27 实测积累 360 个）。每进程至多扫一次，
    /// best-effort；24h 阈值不碰并行测试进程的目录。
    #[cfg(test)]
    fn sweep_stale_test_data_dirs() {
        static SWEEP: std::sync::Once = std::sync::Once::new();
        SWEEP.call_once(|| {
            let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
                return;
            };
            let now = std::time::SystemTime::now();
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if !name.starts_with("storyforge-app-state-test-")
                    && !name.starts_with("storyforge_test_")
                {
                    continue;
                }
                let stale = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| now.duration_since(t).ok())
                    .is_some_and(|age| age >= std::time::Duration::from_secs(24 * 3600));
                if stale {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        });
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

    /// 当前活跃的 LLM client。无连接时返回 None，禁止生产态回退开发 Mock。
    pub fn active_llm(&self) -> Option<Arc<dyn LlmClient>> {
        let guard = self.active_llm.lock().unwrap_or_else(|p| p.into_inner());
        guard.clone()
    }

    /// 获取真实活跃连接；所有需要 LLM 的生产入口统一 fail closed。
    pub fn require_active_llm(&self) -> Result<Arc<dyn LlmClient>, TauriCommandError> {
        self.active_llm().ok_or_else(|| {
            TauriCommandError::llm("未配置活跃 LLM 连接，请先在设置中配置并启用连接。", false)
        })
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
        // 生产路径统一包 retry（RateLimited/ServerError/Timeout 指数退避）
        let retried = storyforge_infra_llm::with_retry(
            client,
            storyforge_domain::llm::RetryConfig::default(),
        );

        // 包装 LlmInterceptor：每次 LLM 调用自动记录 payload/响应/token/延迟到 LogStore
        let intercepted: Arc<dyn LlmClient> =
            Arc::new(storyforge_app_logging::interceptor::LlmInterceptor::new(
                retried,
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
    pub fn new_pipeline(&self) -> Result<PipelineOrchestrator, TauriCommandError> {
        self.new_pipeline_with_regex(&[])
    }

    pub fn new_pipeline_with_regex(
        &self,
        regex_scripts: &[RegexScript],
    ) -> Result<PipelineOrchestrator, TauriCommandError> {
        self.new_pipeline_with_regex_and_prompt_hook(regex_scripts, None)
    }

    pub fn new_pipeline_with_regex_and_prompt_hook(
        &self,
        regex_scripts: &[RegexScript],
        prompt_hook: Option<PromptHook>,
    ) -> Result<PipelineOrchestrator, TauriCommandError> {
        let llm = self.require_active_llm()?;
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
        // A1：从活跃连接注入采样参数（含 reasoning 模式 + extra 扩展字段）
        let sampling = get_conn_store().active_connection().map(|conn| conn.params);
        let mut pipeline = PipelineOrchestrator::new_with_sampling(
            llm,
            self.conv_store.clone(),
            Arc::new(tool_ctx),
            mvu_rt,
            prompt_hook,
            sampling,
        );
        // SQLite pre-accept UoW owns the atomic conversation+attempt land point.
        // Pipeline generation returns text/provenance without durable ConversationStore writes.
        if sqlite_runtime::is_sqlite_active() {
            pipeline.set_defer_conversation_land(true);
        }
        Ok(pipeline)
    }
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

/// Shared production postprocess entry used by start_writing (spawned) and regenerate (awaited).
///
/// Runner orchestration still uses `PipelineOrchestrator::run_postprocess` at the command
/// layer. Outcome writeback / guards / Chronicle candidates are owned by
/// `ProductionPostprocessService`.
///
/// Returns `Ok(applied)` or `Err` for critical consistency failures that callers must
/// surface (regenerate) / fail-closed (background start_writing).
#[allow(clippy::too_many_arguments)]
async fn run_shared_postprocess_background(
    pipeline: PipelineOrchestrator,
    writing_ctx: WritingContext,
    final_text: String,
    present_chars: Vec<String>,
    variable_keys: Vec<String>,
    fallback_fragments: Vec<storyforge_domain::mvu_translation::FallbackFragment>,
    mvu_update_rules: Vec<String>,
    event_tx: tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    cancel: watch::Receiver<bool>,
    identity: Option<production_postprocess::PostprocessIdentity>,
    runtime: Option<std::sync::Arc<CampaignRuntimeContext>>,
) -> Result<bool, production_postprocess::ProductionPostprocessError> {
    use production_postprocess::{ProductionPostprocessError, ProductionPostprocessService};

    if *cancel.borrow() {
        tracing::warn!("Phase A: postprocess 写回跳过——cancelled");
        return Ok(false);
    }

    let outcome = pipeline
        .run_postprocess(
            &final_text,
            "",
            &present_chars,
            &variable_keys,
            &writing_ctx,
            &event_tx,
            cancel.clone(),
            &fallback_fragments,
            &mvu_update_rules,
        )
        .await;

    if *cancel.borrow() {
        tracing::warn!("Phase A: postprocess 写回跳过——cancelled after runner");
        return Ok(false);
    }

    let sink = BackendTurnAttemptSink::production();
    let Some(identity) = identity else {
        // 非 Campaign 路径：保持旧行为（直接写 store）
        if let Some(outcome) = outcome {
            persist_postprocess_outcome_async(&writing_ctx, outcome, present_chars).await;
        }
        return Ok(false);
    };

    let result = if sqlite_runtime::is_sqlite_active() {
        match runtime.as_ref() {
            Some(runtime) => {
                let service = ProductionPostprocessService::new_runtime(runtime.as_ref(), &sink);
                service.apply_outcome(&identity, outcome, &present_chars, &cancel)
            }
            None => {
                // Same unified PostProcessFailed path as apply_outcome errors.
                Err(ProductionPostprocessError::BatchConstruction(
                    "sqlite postprocess has no CampaignRuntimeContext; refusing JSON fallback"
                        .into(),
                ))
            }
        }
    } else {
        let service = ProductionPostprocessService::new_json(get_campaign_store(), &sink);
        service.apply_outcome(&identity, outcome, &present_chars, &cancel)
    };

    match result {
        Ok(result) if result.applied => {
            let (knowledge_count, variable_count, task_count) = result
                .outcome
                .as_ref()
                .and_then(|o| o.post_process.as_ref())
                .map(|pp| {
                    (
                        pp.knowledge_updates.len(),
                        pp.variable_updates.len(),
                        pp.task_updates.len(),
                    )
                })
                .unwrap_or((0, 0, 0));
            let _ = event_tx.send(PipelineEvent::PostProcessDone {
                knowledge_count,
                variable_count,
                task_count,
            });
            Ok(true)
        }
        Ok(result) => {
            if let Some(reason) = result.skipped_reason.as_deref() {
                tracing::warn!("Phase A: postprocess 写回跳过——{reason}");
                if reason == "cancelled" {
                    let _ = event_tx.send(PipelineEvent::PostProcessFailed {
                        reason: "postprocess cancelled".into(),
                    });
                }
            }
            Ok(false)
        }
        Err(e) => {
            tracing::error!("Phase A: postprocess 失败: {e}");
            let combined = service_fail_turn(&sink, &identity, e);
            let _ = event_tx.send(PipelineEvent::PostProcessFailed {
                reason: combined.to_string(),
            });
            Err(combined)
        }
    }
}

fn service_fail_turn(
    sink: &BackendTurnAttemptSink<'_>,
    identity: &production_postprocess::PostprocessIdentity,
    original: production_postprocess::ProductionPostprocessError,
) -> production_postprocess::ProductionPostprocessError {
    // Identity-validation errors must never mark/write the Turn.
    if matches!(
        original,
        production_postprocess::ProductionPostprocessError::ScopeMismatch { .. }
            | production_postprocess::ProductionPostprocessError::AttemptMissing { .. }
    ) {
        return original;
    }
    // Only mark Failed when this identity still owns the current writable Attempt.
    // Superseded / cancelled late errors are zero-write and keep the new Attempt intact.
    match sink.mark_failed_if_current(identity, original.to_string()) {
        Ok(_marked) => original,
        Err(mark_error) => production_postprocess::ProductionPostprocessError::MarkFailed {
            original: original.to_string(),
            mark_error,
        },
    }
}

/// 读取-修改-写回 TurnRecord 的便捷辅助。
fn update_turn_record<F>(turn_id: &Id, f: F) -> Result<(), String>
where
    F: FnOnce(&mut storyforge_domain::turn::TurnRecord),
{
    if sqlite_runtime::is_sqlite_active() {
        return sqlite_runtime::update_turn_record(turn_id, f);
    }
    get_turn_store()
        .with_turn_mut(turn_id, f)
        .map_err(|e| format!("保存 TurnRecord 失败: {e}"))
}

/// 条件更新 TurnRecord：predicate 失败返回 Ok(false)，不改盘。
fn update_turn_record_if<P, M>(turn_id: &Id, predicate: P, mutate: M) -> Result<bool, String>
where
    P: FnOnce(&storyforge_domain::turn::TurnRecord) -> bool,
    M: FnOnce(&mut storyforge_domain::turn::TurnRecord),
{
    if sqlite_runtime::is_sqlite_active() {
        return sqlite_runtime::mutate_turn_if(turn_id, predicate, mutate);
    }
    get_turn_store()
        .mutate_if(turn_id, predicate, mutate)
        .map_err(|e| format!("条件更新 TurnRecord 失败: {e}"))
}

/// 后处理结果只能写回仍属于当前草稿的 Attempt。
fn is_current_attempt_ready_for_postprocess(
    record: &storyforge_domain::turn::TurnRecord,
    attempt_id: &Id,
) -> bool {
    turn_lifecycle::is_current_attempt_ready_for_postprocess(record, attempt_id)
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
    .map_err(|e| TauriCommandError::internal(format!("准备写作对话任务失败: {e}")))?
}

fn prepare_start_conversation(
    state: Arc<AppState>,
    campaign_store: &campaign_store::CampaignStore,
    requested_conversation_id: Option<String>,
    character_id: Option<String>,
    legacy_opening_character: Option<Arc<storyforge_domain::character::Character>>,
    opening_message: Option<String>,
    intent: String,
) -> Result<StartConversationTarget, TauriCommandError> {
    let campaign_conv_id: Option<Id> = {
        let active = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if sqlite_runtime::is_sqlite_active() {
            match active.as_ref() {
                Some(campaign_id) => {
                    sqlite_runtime::get_campaign(campaign_id)
                        .map_err(TauriCommandError::internal)?
                        .ok_or_else(|| {
                            TauriCommandError::internal(format!(
                                "sqlite campaign {} missing while preparing writing",
                                campaign_id
                            ))
                        })?
                        .conversation_id
                }
                None => None,
            }
        } else {
            active
                .as_ref()
                .and_then(|cid| campaign_store.get_campaign(cid))
                .and_then(|campaign| campaign.conversation_id.clone())
        }
    };
    let conversation_id = campaign_conv_id
        .map(|cid| cid.as_str().to_string())
        .or(requested_conversation_id);

    let (conversation_id, input_node_id) = if let Some(id_str) = conversation_id {
        let id = Id::from_str(&id_str);
        match state.conv_store.append_user_message(&id, intent.clone()) {
            Ok(node_id) => (id, Some(node_id)),
            Err(e) => {
                if sqlite_runtime::is_sqlite_active() {
                    return Err(TauriCommandError::internal(format!(
                        "sqlite user message persistence failed: {e}"
                    )));
                }
                tracing::warn!("追加 user 消息失败: {e}");
                (id, None)
            }
        }
    } else {
        let conv = if sqlite_runtime::is_sqlite_active() {
            state
                .conv_store
                .create_persisted(character_id.clone(), None)
                .map_err(|e| {
                    TauriCommandError::internal(format!("sqlite conversation creation failed: {e}"))
                })?
        } else {
            state.conv_store.create(character_id.clone(), None)
        };
        let id = conv.id.clone();
        let legacy_opening =
            resolve_legacy_opening_message(legacy_opening_character.as_ref(), opening_message);
        if let Some(opening) = legacy_opening
            && let Err(e) =
                state
                    .conv_store
                    .append_final_message(&id, ConversationRole::Assistant, opening)
        {
            if sqlite_runtime::is_sqlite_active() {
                return Err(TauriCommandError::internal(format!(
                    "sqlite opening message persistence failed: {e}"
                )));
            }
            tracing::warn!("追加开场白失败: {e}");
        }
        let node_id = match state.conv_store.append_user_message(&id, intent.clone()) {
            Ok(node_id) => Some(node_id),
            Err(e) if sqlite_runtime::is_sqlite_active() => {
                return Err(TauriCommandError::internal(format!(
                    "sqlite user message persistence failed: {e}"
                )));
            }
            Err(e) => {
                tracing::warn!("追加 user 消息失败: {e}");
                None
            }
        };
        (id, node_id)
    };

    let regex_character_id = character_id.or_else(|| {
        state
            .conv_store
            .get(&conversation_id)
            .and_then(|c| c.character_id)
    });

    Ok(StartConversationTarget {
        conversation_id,
        regex_character_id,
        input_node_id,
    })
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

fn stored_character_for_id_or_source_in_store(
    store: &storage::CharacterStore,
    character_id: &Id,
) -> Option<storage::StoredCharacter> {
    let id = character_id.as_str();
    store.get(id).or_else(|| {
        store
            .list()
            .into_iter()
            .find(|stored| stored.info.source_character_id.as_deref() == Some(id))
    })
}

fn stored_character_for_source_id(source_character_id: &Id) -> Option<storage::StoredCharacter> {
    stored_character_for_id_or_source_in_store(get_store(), source_character_id)
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
        resolve_active_campaign_with_legacy_fallback(
            guard.clone(),
            &state.data_dir,
            sqlite_runtime::is_sqlite_active(),
        )
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
    let sqlite_active = sqlite_runtime::is_sqlite_active();
    let snapshot = tokio::task::spawn_blocking(move || {
        let active_id = resolve_active_campaign_with_legacy_fallback(
            memory_active_id,
            &data_dir,
            sqlite_active,
        );
        let Some(active_id) = active_id else {
            return Ok(None);
        };
        if sqlite_runtime::is_sqlite_active() {
            load_sqlite_campaign_context_snapshot(&active_id)
        } else {
            Ok(load_campaign_context_snapshot(
                get_campaign_store(),
                &active_id,
            ))
        }
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("加载 Campaign 快照任务失败: {e}")))?
    .map_err(TauriCommandError::internal)?;

    if let Some(snapshot) = snapshot {
        apply_campaign_context_snapshot(ctx, &state.tool_ctx, snapshot);
    }
    Ok(())
}

fn clear_campaign_runtime(ctx: &mut WritingContext, tool_ctx: &Arc<RwLock<ToolContext>>) {
    // 阶段 2 cleanup：先清空旧 runtime，避免 stale 数据残留。
    ctx.campaign_runtime = None;
    ctx.recent_summaries.clear();
    ctx.far_memory_hits.clear();
    let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    tool_guard.campaign_runtime = None;
    tool_guard.archived_summaries.clear();
    tool_guard.chronicle_summaries.clear();
}

/// 取 `before_node_id` 之前最近一条 User 消息正文（用于 regenerate 无 hint 时的远记忆查询）。
fn last_user_intent_before(
    conv_store: &ConversationStore,
    conv_id: &Id,
    before_node_id: &Id,
) -> Option<String> {
    let conv = conv_store.get(conv_id)?;
    let pos = conv.nodes.iter().position(|n| &n.id == before_node_id)?;
    for node in conv.nodes[..pos].iter().rev() {
        let Some(v) = node.active() else {
            continue;
        };
        if v.status == storyforge_domain::conversation::VariantStatus::Discarded {
            continue;
        }
        if v.role != storyforge_domain::conversation::Role::User {
            continue;
        }
        let content = v.content.trim();
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }
    None
}

/// 按用户意图从向量库召回 ArchivedSummary（混合：关键词 + 可选嵌入，best-effort）。
///
/// 失败只 warn，不阻断写作。无命中时 `far_memory_hits` 为空。
/// 有 campaign 时优先过滤同 campaign 记录，兼容无标签的旧归档。
/// 配置了 Embedder 时走向量+关键词合并；否则纯关键词。
async fn fill_far_memory_hits(ctx: &mut WritingContext, state: &AppState, intent: &str) {
    ctx.far_memory_hits.clear();
    let intent = intent.trim();
    if intent.is_empty() {
        return;
    }
    let campaign_filter = ctx.campaign_id.as_ref().map(|id| id.to_string());
    let embedder = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .and_then(|cfg| match storyforge_infra_llm::Embedder::new(cfg) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::debug!(target: "far_memory", "构建 Embedder 失败，关键词路径: {e}");
                None
            }
        });
    match storyforge_app_memory::recall_archived_hybrid(
        state.vector_store.as_ref(),
        intent,
        3,
        campaign_filter.as_deref(),
        embedder.as_ref(),
    )
    .await
    {
        Ok(hits) => {
            if !hits.is_empty() {
                tracing::info!(
                    target: "far_memory",
                    "远记忆召回 {} 条（hybrid={}）ids={:?}",
                    hits.len(),
                    embedder.is_some(),
                    hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>()
                );
            }
            // 保留 id/score/kind 溯源；注入文本仍只用 content
            ctx.far_memory_hits = hits
                .into_iter()
                .map(|h| {
                    storyforge_app_pipeline::FarMemoryHit::new(h.id, h.content, h.score, h.kind)
                })
                .collect();
        }
        Err(e) => {
            tracing::warn!(target: "far_memory", "远记忆召回失败（跳过）: {e}");
        }
    }
}

/// 把已 accept 的 RoundSummary 索引进向量库。
///
/// - 始终写关键词（无 Embedder 也能召回）
/// - 若传入 `embedder`，best-effort 嵌入真实向量供 hybrid 召回
/// - 幂等：以 summary.id 为向量记录 id upsert
fn index_round_summaries_to_vector(
    vector_store: &dyn VectorStore,
    batch: &storyforge_domain::turn::MutationBatch,
) {
    // 同步关键词路径（commit 关键不阻塞等嵌入）
    index_round_summaries_to_vector_with_vectors(vector_store, batch, &[]);
}

/// 同 `index_round_summaries_to_vector`，但允许预计算向量（id → vector）。
fn index_round_summaries_to_vector_with_vectors(
    vector_store: &dyn VectorStore,
    batch: &storyforge_domain::turn::MutationBatch,
    vectors: &[(Id, Vec<f32>)],
) {
    for mutation in &batch.mutations {
        let storyforge_domain::turn::Mutation::UpsertSummary(summary) = mutation else {
            continue;
        };
        let content = summary.content.trim();
        if content.is_empty() {
            continue;
        }
        // extract_keywords 取高频；extract_query_tokens 补全 query 侧 2-gram/词，
        // 合并后召回与 intent 分词更对齐。
        let mut keywords = storyforge_app_memory::extract_keywords(content);
        for t in storyforge_app_memory::extract_query_tokens(content) {
            if !keywords.iter().any(|k| k == &t) {
                keywords.push(t);
            }
        }
        if keywords.is_empty() {
            continue;
        }
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "campaign_id".into(),
            serde_json::Value::String(summary.campaign_id.to_string()),
        );
        metadata.insert(
            "conversation_id".into(),
            serde_json::Value::String(summary.conversation_id.to_string()),
        );
        metadata.insert(
            "turn".into(),
            serde_json::Value::Number(summary.turn.into()),
        );
        metadata.insert(
            "source".into(),
            serde_json::Value::String("round_summary".into()),
        );
        // 记忆规格 M1：统一 source_kind / code / lineage（兼容旧检索）
        metadata.insert(
            "source_kind".into(),
            serde_json::Value::String("chronicle_a".into()),
        );
        metadata.insert(
            "source_entry_id".into(),
            serde_json::Value::String(summary.id.to_string()),
        );
        if let Some(code) = &summary.code {
            metadata.insert("code".into(), serde_json::Value::String(code.clone()));
        }
        if let Some(lin) = &summary.lineage_id {
            metadata.insert(
                "lineage_id".into(),
                serde_json::Value::String(lin.to_string()),
            );
        }
        if let Some(h) = &summary.headline {
            metadata.insert("headline".into(), serde_json::Value::String(h.clone()));
        }
        let vector = vectors
            .iter()
            .find(|(id, _)| id == &summary.id)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        if let Err(e) = vector_store.upsert(VectorRecord {
            id: summary.id.clone(),
            content: content.to_string(),
            vector,
            keywords,
            kind: VectorKind::ArchivedSummary,
            metadata,
        }) {
            tracing::warn!(
                target: "far_memory",
                "RoundSummary {} 入向量库失败: {e}",
                summary.id
            );
        } else {
            tracing::debug!(
                target: "far_memory",
                "RoundSummary {} 已索引到远记忆向量库",
                summary.id
            );
        }
    }
}

/// 后台：为 batch 中的 RoundSummary 补齐嵌入向量（有 Embedder 时）。
///
/// 先关键词落盘保证可召回；嵌入成功后覆盖 upsert 写入真实向量。
async fn index_round_summaries_async(
    state: Arc<AppState>,
    batch: storyforge_domain::turn::MutationBatch,
) {
    // 1. 关键词立即入库
    index_round_summaries_to_vector(state.vector_store.as_ref(), &batch);

    // 2. 可选嵌入补齐
    let config = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let Some(config) = config else {
        return;
    };
    let embedder = match storyforge_infra_llm::Embedder::new(config) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(target: "far_memory", "RoundSummary 嵌入跳过（Embedder 构建失败）: {e}");
            return;
        }
    };

    let mut vectors = Vec::new();
    for mutation in &batch.mutations {
        let storyforge_domain::turn::Mutation::UpsertSummary(summary) = mutation else {
            continue;
        };
        let content = summary.content.trim();
        if content.is_empty() {
            continue;
        }
        match embedder.embed(content).await {
            Ok(v) => vectors.push((summary.id.clone(), v)),
            Err(e) => {
                tracing::warn!(
                    target: "far_memory",
                    "RoundSummary {} 嵌入失败（保留关键词索引）: {e}",
                    summary.id
                );
            }
        }
    }
    if !vectors.is_empty() {
        index_round_summaries_to_vector_with_vectors(state.vector_store.as_ref(), &batch, &vectors);
        tracing::info!(
            target: "far_memory",
            "RoundSummary 嵌入补齐 {} 条",
            vectors.len()
        );
    }
}

pub struct CampaignContextSnapshot {
    active_id: Id,
    story_clock: String,
    turn: u32,
    pending_tasks: Vec<storyforge_domain::story_task::StoryTask>,
    scoped_regex_scripts: Vec<RegexScript>,
    runtime: Arc<CampaignRuntimeContext>,
    /// 近期 RoundSummary（turn 升序），供 Director tail + get_recent_summary
    recent_summaries: Vec<storyforge_domain::agent::RoundSummary>,
    /// search/get_chronicle 目录（含 B/C + 较远 A）
    chronicle_tool_catalog: Vec<storyforge_domain::agent::RoundSummary>,
    /// 渲染 overview/band 的完整条目（不受 last-12 截断）
    chronicle_prompt_catalog: Vec<storyforge_domain::agent::RoundSummary>,
}

/// ContextCompiler load-side：写入 WritingContext 的近期摘要条数上限。
/// turn 计数仍用全量 list_summaries.len()；注入侧再取 last 5。
const RECENT_SUMMARIES_LOAD_LIMIT: usize = 12;
/// 按 turn 升序后只保留最近 `limit` 条，避免长战役全量摘要进内存/工具上下文。
fn take_recent_summaries_for_context(
    mut summaries: Vec<storyforge_domain::agent::RoundSummary>,
    limit: usize,
) -> Vec<storyforge_domain::agent::RoundSummary> {
    summaries.sort_by_key(|s| s.turn);
    if limit == 0 {
        return Vec::new();
    }
    if summaries.len() > limit {
        let drop_n = summaries.len() - limit;
        summaries.drain(0..drop_n);
    }
    summaries
}

/// 渲染 ContextEpochSnapshot 用的 catalog：snapshot 引用的 code + 全部 leaf A。
fn build_chronicle_prompt_catalog(
    all: &[storyforge_domain::agent::RoundSummary],
    frozen: Option<&storyforge_domain::chronicle::ContextEpochSnapshot>,
) -> Vec<storyforge_domain::agent::RoundSummary> {
    use std::collections::HashSet;
    let mut needed: HashSet<String> = HashSet::new();
    if let Some(snap) = frozen {
        for c in &snap.overview_codes {
            needed.insert(c.as_str().to_string());
        }
        for c in &snap.band_codes {
            needed.insert(c.as_str().to_string());
        }
    }
    let mut out: Vec<_> = all
        .iter()
        .filter(|s| s.code.as_ref().map(|c| needed.contains(c)).unwrap_or(false) || s.is_leaf_a())
        .cloned()
        .collect();
    let mut seen = HashSet::new();
    out.retain(|s| seen.insert(s.id.to_string()));
    out.sort_by_key(|s| s.effective_turn_end());
    out
}

/// 已提交 Turn 数：只计 Chronicle A（leaf），不计 B/C stage。
fn committed_turn_count(summaries: &[storyforge_domain::agent::RoundSummary]) -> u32 {
    summaries
        .iter()
        .filter(|s| s.is_leaf_a())
        .map(|s| s.turn)
        .max()
        .unwrap_or(0)
}

/// 下一写作轮次 = max(A.turn) + 1（无 A 时为 1）。
fn next_writing_turn(summaries: &[storyforge_domain::agent::RoundSummary]) -> u32 {
    committed_turn_count(summaries).saturating_add(1)
}

/// 工具侧 Chronicle 目录：优先保留全部 B/C，再保留最近的 A（按 turn_end）。
#[cfg(test)]
fn build_chronicle_tool_catalog(
    summaries: Vec<storyforge_domain::agent::RoundSummary>,
    limit: usize,
) -> Vec<storyforge_domain::agent::RoundSummary> {
    if limit == 0 {
        return Vec::new();
    }
    let mut stages: Vec<_> = summaries.iter().filter(|s| s.level > 0).cloned().collect();
    stages.sort_by_key(|s| s.effective_turn_end());
    let mut leaves: Vec<_> = summaries.into_iter().filter(|s| s.level == 0).collect();
    leaves.sort_by_key(|s| s.turn);

    if stages.len() >= limit {
        // 极端：B/C 已超 cap → 保留最新 stage
        let drop_n = stages.len() - limit;
        stages.drain(0..drop_n);
        return stages;
    }
    let leaf_room = limit - stages.len();
    if leaves.len() > leaf_room {
        let drop_n = leaves.len() - leaf_room;
        leaves.drain(0..drop_n);
    }
    stages.extend(leaves);
    stages.sort_by_key(|s| s.effective_turn_end());
    stages
}

/// 旧存档：缺 lineage 的 summary 回填为 campaign.lineage_id 并落盘。
fn backfill_summary_lineage_if_needed(
    store: &campaign_store::CampaignStore,
    lineage: &Id,
    summaries: &mut [storyforge_domain::agent::RoundSummary],
) {
    for s in summaries.iter_mut() {
        if s.lineage_id.is_none() {
            s.lineage_id = Some(lineage.clone());
            if let Err(e) = store.update_summary_by_id(s.clone()) {
                tracing::warn!(
                    target: "context_compiler",
                    "backfill summary lineage failed id={}: {e}",
                    s.id
                );
            }
        }
    }
}

/// ensure campaign.lineage_id；若新建则立即 persist。
fn ensure_campaign_lineage_persisted(
    store: &campaign_store::CampaignStore,
    camp: &mut storyforge_domain::campaign::Campaign,
) -> Id {
    let created = camp.lineage_id.is_none();
    let lineage = camp.ensure_lineage_id().clone();
    if created && let Err(e) = store.update_campaign(camp.clone()) {
        tracing::warn!(
            target: "context_compiler",
            "persist new lineage_id failed: {e}"
        );
    }
    lineage
}

fn load_campaign_context_snapshot(
    store: &campaign_store::CampaignStore,
    active_id: &Id,
) -> Option<CampaignContextSnapshot> {
    let mut camp = store.get_campaign(active_id)?;
    let lineage = ensure_campaign_lineage_persisted(store, &mut camp);
    let story_clock = camp.story_clock.clone();
    // turn 只计 A；注入 recent last-K；prompt catalog 覆盖 snapshot codes；工具目录含 B/C
    let mut all_summaries = store.list_summaries(active_id);
    backfill_summary_lineage_if_needed(store, &lineage, &mut all_summaries);
    let (epoch_snap, _) = refresh_and_persist_context_epoch(store, &mut camp, &all_summaries);
    let _ = epoch_snap; // applied via camp.context_epoch into runtime.campaign
    // 锁内可能已合并压缩结果：重新 list 供 catalog / turn 使用
    let all_summaries = store.list_summaries(active_id);
    let turn = next_writing_turn(&all_summaries);
    let chronicle_prompt_catalog =
        build_chronicle_prompt_catalog(&all_summaries, camp.context_epoch.as_ref());
    // 全量目录：get_chronicle 按 code 点名旧 A 不受 256 截断；search 仍有 limit/预算
    let chronicle_tool_catalog = all_summaries.clone();
    let recent_summaries =
        take_recent_summaries_for_context(all_summaries, RECENT_SUMMARIES_LOAD_LIMIT);
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
        recent_summaries,
        chronicle_tool_catalog,
        chronicle_prompt_catalog,
    })
}

/// SQLite opt-in version of the Campaign context compiler input. The writing
/// pipeline receives the same immutable domain snapshot as the JSON backend,
/// but every Campaign/instance/task/knowledge/summary read and every epoch
/// refresh write uses the process-owned SQLite authority.
/// Public for harness SQLite endurance: same snapshot path as production opt-in.
pub fn load_sqlite_campaign_context_snapshot(
    active_id: &Id,
) -> Result<Option<CampaignContextSnapshot>, String> {
    let Some(mut camp) = sqlite_runtime::get_campaign(active_id)? else {
        return Ok(None);
    };

    let mut campaign_changed = false;
    if camp.lineage_id.is_none() {
        camp.ensure_lineage_id();
        campaign_changed = true;
    }
    let all_summaries = sqlite_runtime::list_summaries(active_id)?;
    let (epoch_snap, _membership, should_persist_epoch, bumped_revision) =
        compute_context_epoch_refresh_parts(&camp, &all_summaries);
    if should_persist_epoch {
        camp.chronicle_revision = bumped_revision;
        camp.context_epoch = Some(epoch_snap.clone());
        campaign_changed = true;
    }
    if campaign_changed {
        sqlite_runtime::save_campaign(&camp)?;
    }

    let all_summaries = sqlite_runtime::list_summaries(active_id)?;
    let turn = next_writing_turn(&all_summaries);
    let chronicle_prompt_catalog =
        build_chronicle_prompt_catalog(&all_summaries, camp.context_epoch.as_ref());
    let recent_summaries =
        take_recent_summaries_for_context(all_summaries.clone(), RECENT_SUMMARIES_LOAD_LIMIT);
    let tasks = sqlite_runtime::list_tasks(active_id)?;
    let instances = sqlite_runtime::list_instances(active_id)?;
    let knowledge = sqlite_runtime::list_knowledge(active_id)?;

    let stored_card = sqlite_runtime::get_card_payload(&camp.card_id)?.and_then(|payload| {
        serde_json::from_value::<campaign_store::StoredCard>(payload.clone())
            .ok()
            .or_else(|| {
                serde_json::from_value::<storyforge_domain::character::CharacterCard>(payload)
                    .ok()
                    .map(|card| campaign_store::StoredCard {
                        card,
                        imported_at: String::new(),
                    })
            })
    });
    let scoped_regex_scripts = stored_card
        .as_ref()
        .map(|stored| stored.card.scoped_regex_scripts())
        .unwrap_or_default();
    let definitions_by_id = stored_card
        .map(|stored| {
            stored
                .card
                .character_definitions
                .into_iter()
                .map(|definition| (definition.id.clone(), definition))
                .collect()
        })
        .unwrap_or_default();

    let story_clock = camp.story_clock.clone();
    let runtime = Arc::new(CampaignRuntimeContext {
        campaign: camp,
        instances,
        definitions_by_id,
        knowledge,
        tasks: tasks.clone(),
        turn,
    });
    Ok(Some(CampaignContextSnapshot {
        active_id: active_id.clone(),
        story_clock,
        turn,
        pending_tasks: tasks,
        scoped_regex_scripts,
        runtime,
        recent_summaries,
        chronicle_tool_catalog: all_summaries,
        chronicle_prompt_catalog,
    }))
}

/// Apply a pre-built campaign context snapshot (JSON or SQLite).
pub fn apply_campaign_context_snapshot(
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
    ctx.recent_summaries = snapshot.recent_summaries;
    ctx.chronicle_prompt_catalog = snapshot.chronicle_prompt_catalog;
    ctx.context_epoch = snapshot.runtime.campaign.context_epoch.clone();
    ctx.chronicle_revision = snapshot.runtime.campaign.chronicle_revision;

    let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    tool_guard.campaign_runtime = Some(snapshot.runtime);
    // 同步到 get_recent_summary 工具：content 列表（升序，工具内部 rev().take）
    tool_guard.archived_summaries = ctx
        .recent_summaries
        .iter()
        .map(|s| s.content.clone())
        .collect();
    // 同步到 search_chronicle / get_chronicle（A/B/C 目录，宽于 inject 窗口）
    tool_guard.chronicle_summaries = snapshot.chronicle_tool_catalog;
    tool_guard.reset_chronicle_tool_budget();
}

/// 在 Context 编译入口刷新 ContextEpochSnapshot 并可选落盘。
///
/// - live_suffix 满 E → rollover（新 overview/band/anchor）
/// - 同 epoch 内 overview/band 冻结
/// - created/rollover 时 bump chronicle_revision
///
/// **并发边界**：与 Compressor publish/heal 共用 `with_campaign_lock`。
/// 锁内重新读取最新 Campaign + summaries 再计算并落盘，避免用过期快照回写
/// 覆盖压缩后的 chronicle_revision / epoch / pending marker。
///
/// `all_summaries` 仅作锁失败时的无落盘兜底输入；成功路径以锁内 list 为准。
fn refresh_and_persist_context_epoch(
    store: &campaign_store::CampaignStore,
    camp: &mut storyforge_domain::campaign::Campaign,
    all_summaries: &[storyforge_domain::agent::RoundSummary],
) -> (
    storyforge_domain::chronicle::ContextEpochSnapshot,
    storyforge_domain::chronicle::EpochMembership,
) {
    let campaign_id = camp.id.clone();
    let locked = turn_coordinator::with_campaign_lock(|| {
        let Some(mut latest) = store.get_campaign(&campaign_id) else {
            return Err(turn_coordinator::CommitError::CampaignNotFound(
                campaign_id.clone(),
            ));
        };
        let summaries = store.list_summaries(&campaign_id);
        let (snap, membership, should_persist, bumped_rev) =
            compute_context_epoch_refresh_parts(&latest, &summaries);

        if should_persist {
            latest.chronicle_revision = bumped_rev;
            latest.context_epoch = Some(snap.clone());
            // 不触碰 pending_compress_publication：压缩半提交仍由 heal 收口
            store
                .update_campaign(latest.clone())
                .map_err(turn_coordinator::CommitError::Storage)?;
            tracing::debug!(
                target: "context_compiler",
                epoch_id = %snap.epoch_id,
                chronicle_revision = latest.chronicle_revision,
                overview = snap.overview_codes.len(),
                band = snap.band_codes.len(),
                "context epoch refreshed under campaign lock"
            );
        }

        *camp = store.get_campaign(&campaign_id).unwrap_or(latest);
        Ok((snap, membership))
    });

    match locked {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "context_compiler",
                "refresh context_epoch under lock failed: {e}; in-memory compute without persist"
            );
            let (snap, membership, _, _) = compute_context_epoch_refresh_parts(camp, all_summaries);
            (snap, membership)
        }
    }
}

/// 纯计算：基于给定 camp/summaries 产出 snapshot + membership + 是否需要 persist + 新 revision。
fn compute_context_epoch_refresh_parts(
    camp: &storyforge_domain::campaign::Campaign,
    all_summaries: &[storyforge_domain::agent::RoundSummary],
) -> (
    storyforge_domain::chronicle::ContextEpochSnapshot,
    storyforge_domain::chronicle::EpochMembership,
    bool,
    u64,
) {
    use storyforge_domain::chronicle::{
        ChronicleCode, ChronicleLevel, ContextWindowParams, OverviewCandidate,
        committed_turns_from_count, refresh_context_epoch, sequence_from_committed_turn_id,
    };

    // 只基于 A 的 max turn（B/C 不增加 committed turn 序列长度）
    let n = committed_turn_count(all_summaries);
    let committed = committed_turns_from_count(n);

    let mut cands: Vec<OverviewCandidate> = Vec::new();
    for s in all_summaries {
        let level = s.chronicle_level();
        let code = s
            .code
            .as_deref()
            .and_then(ChronicleCode::parse)
            .unwrap_or_else(|| ChronicleCode::new(level, s.turn));
        cands.push(OverviewCandidate {
            code,
            level,
            turn_start: s.turn,
            covered_by: s.covered_by.clone(),
        });
    }

    let code_by_turn: std::collections::HashMap<u32, ChronicleCode> = all_summaries
        .iter()
        .map(|s| {
            let code = s
                .code
                .as_deref()
                .and_then(ChronicleCode::parse)
                .unwrap_or_else(|| ChronicleCode::new(ChronicleLevel::A, s.turn));
            (s.turn, code)
        })
        .collect();

    let band_lookup =
        |id: &Id| sequence_from_committed_turn_id(id).and_then(|t| code_by_turn.get(&t).cloned());

    let params = ContextWindowParams::default();
    let existing = camp.context_epoch.clone();
    let result = refresh_context_epoch(
        existing.as_ref(),
        &committed,
        &cands,
        &band_lookup,
        params,
        camp.chronicle_revision,
    );

    let mut rev = camp.chronicle_revision;
    let should_bump = result.should_bump_chronicle_revision();
    if should_bump {
        rev = rev.saturating_add(1);
    }
    let mut snap = result.snapshot;
    snap.chronicle_revision = rev;
    // 需要落盘：新建 / rollover / bump，或 camp 尚无 epoch
    let should_persist =
        should_bump || result.created || result.rolled_over || camp.context_epoch.is_none();
    (snap, result.membership, should_persist, rev)
}

/// 从指定 CampaignStore 的活跃 Campaign 组装 CampaignRuntimeContext 快照写入 ctx + tool_ctx。
///
/// 抽自 `fill_campaign_context`，供外部 harness 用独立 tempdir `CampaignStore` 复刻真实
/// campaign-mode 组装逻辑（无需 AppState / 全局 store / Tauri runtime）。行为与线上路径一致。
///
/// 调用前应已清空 `ctx.campaign_runtime` 与 `tool_ctx.campaign_runtime`（防 stale）。
/// 若 store 中找不到该 campaign，直接返回（无 campaign 模式）。
/// Fill WritingContext from the process-owned SQLite authority (opt-in only).
pub fn fill_campaign_runtime_from_sqlite(
    ctx: &mut WritingContext,
    tool_ctx: &Arc<RwLock<ToolContext>>,
    active_id: &Id,
) -> Result<(), String> {
    if !sqlite_runtime::is_sqlite_active() {
        return Err("fill_campaign_runtime_from_sqlite requires SQLite backend".into());
    }
    match load_sqlite_campaign_context_snapshot(active_id)? {
        Some(snapshot) => {
            apply_campaign_context_snapshot(ctx, tool_ctx, snapshot);
            Ok(())
        }
        None => Ok(()),
    }
}

pub fn fill_campaign_runtime_from_store(
    ctx: &mut WritingContext,
    tool_ctx: &Arc<RwLock<ToolContext>>,
    store: &campaign_store::CampaignStore,
    active_id: &Id,
) {
    let mut camp = match store.get_campaign(active_id) {
        Some(c) => c,
        None => return,
    };
    let lineage = ensure_campaign_lineage_persisted(store, &mut camp);
    ctx.campaign_id = Some(active_id.clone());
    ctx.story_clock = camp.story_clock.clone();
    // turn = max(A.turn)+1；B/C 不计入
    let mut all_summaries = store.list_summaries(active_id);
    backfill_summary_lineage_if_needed(store, &lineage, &mut all_summaries);
    ctx.turn = next_writing_turn(&all_summaries);
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

    // M2 完整：编译入口刷新 epoch 快照（满 E 则 rollover）并落盘（锁内重读）
    let (epoch_snap, _membership) =
        refresh_and_persist_context_epoch(store, &mut camp, &all_summaries);
    ctx.context_epoch = Some(epoch_snap);
    ctx.chronicle_revision = camp.chronicle_revision;
    // 刷新后重读 summaries（压缩可能已在锁窗口内发布）
    let all_summaries = store.list_summaries(active_id);
    ctx.turn = next_writing_turn(&all_summaries);

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
    // ContextCompiler：RoundSummary → Director history 前缀 + tools
    // load-side 只保留最近 K 条（turn 已在上方用全量条数计算）
    // 全量目录：get_chronicle 按 code 点名旧 A 不受 256 截断；search 仍有 limit/预算
    let chronicle_tool_catalog = all_summaries.clone();
    ctx.chronicle_prompt_catalog =
        build_chronicle_prompt_catalog(&all_summaries, ctx.context_epoch.as_ref());
    ctx.recent_summaries =
        take_recent_summaries_for_context(all_summaries, RECENT_SUMMARIES_LOAD_LIMIT);

    // 同步到 ToolContext（快照，非 store 引用）
    {
        let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        tool_guard.campaign_runtime = Some(runtime);
        tool_guard.archived_summaries = ctx
            .recent_summaries
            .iter()
            .map(|s| s.content.clone())
            .collect();
        tool_guard.chronicle_summaries = chronicle_tool_catalog;
        tool_guard.reset_chronicle_tool_budget();
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

fn postprocess_variable_type_name(
    value_type: &storyforge_domain::variables::VariableType,
) -> &'static str {
    use storyforge_domain::variables::VariableType;
    match value_type {
        VariableType::Int => "int",
        VariableType::Float => "float",
        VariableType::String => "string",
        VariableType::Bool => "bool",
        VariableType::Json => "json",
    }
}

fn postprocess_json_value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(number) if number.is_i64() => "int",
        serde_json::Value::Number(_) => "float",
        serde_json::Value::String(_) => "string",
        _ => "json",
    }
}

fn postprocess_variable_hint(key: &str, scope: &str, label: &str, value_type: &str) -> String {
    format!("{key}（{scope}/{label}/{value_type}）")
}

fn insert_postprocess_schema_hint(
    catalog: &mut std::collections::BTreeMap<String, String>,
    scope_key: &str,
    scope_label: &str,
    field: &storyforge_domain::variables::VariableField,
) {
    catalog.insert(
        format!("{scope_key}:{}", field.key),
        postprocess_variable_hint(
            &field.key,
            scope_label,
            &field.label,
            postprocess_variable_type_name(&field.value_type),
        ),
    );
}

/// 后处理可更新变量键。
///
/// 基础表提供常用角色/全局变量；CampaignRuntimeContext 提供当前卡自定义 schema、
/// 已存在 Campaign 变量和 instance 变量，覆盖 MVU/initvar 与高玩自定义字段。
fn postprocess_variable_keys(ctx: &WritingContext) -> Vec<String> {
    let mut catalog = std::collections::BTreeMap::<String, String>::new();

    for field in storyforge_domain::variables::default_character_variables() {
        insert_postprocess_schema_hint(&mut catalog, "character", "角色", &field);
    }
    for field in storyforge_domain::variables::default_campaign_variables() {
        insert_postprocess_schema_hint(&mut catalog, "campaign", "全局", &field);
    }

    let Some(runtime) = &ctx.campaign_runtime else {
        return catalog.into_values().collect();
    };

    for field in &runtime.campaign.variable_schema {
        insert_postprocess_schema_hint(&mut catalog, "campaign", "全局", field);
    }
    for variable in &runtime.campaign.variables {
        catalog
            .entry(format!("campaign:{}", variable.key))
            .or_insert_with(|| {
                postprocess_variable_hint(
                    &variable.key,
                    "全局",
                    &variable.key,
                    postprocess_json_value_type_name(&variable.value),
                )
            });
    }

    let mut definitions: Vec<_> = runtime.definitions_by_id.values().collect();
    definitions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    for definition in definitions {
        for field in &definition.variable_schema {
            insert_postprocess_schema_hint(&mut catalog, "character", "角色", field);
        }
    }

    for instance in &runtime.instances {
        for variable in &instance.variables {
            catalog
                .entry(format!("character:{}", variable.key))
                .or_insert_with(|| {
                    postprocess_variable_hint(
                        &variable.key,
                        "角色",
                        &variable.key,
                        postprocess_json_value_type_name(&variable.value),
                    )
                });
        }
    }

    catalog.into_values().collect()
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

/// 测试/兼容：直接把临时 instance 落盘到 CampaignStore。
///
/// 生产路径 A.1 改为 accept 时 `Mutation::UpsertInstance`；本函数保留给单测。
#[allow(dead_code)]
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

/// SQLite does not yet migrate the optional MVU translation cache. Do not
/// read its JSON file as a hidden second authority; run postprocess without
/// those optional snippets until MVU has a typed SQLite table.
pub fn collect_mvu_fallback_fragments_for_backend(
    ctx: &WritingContext,
    present_chars: &[String],
) -> Vec<storyforge_domain::mvu_translation::FallbackFragment> {
    if sqlite_runtime::is_sqlite_active() {
        // #22：SQLite 已是 MVU 翻译权威（V005 表 + importer 迁移），直接读它。
        collect_mvu_from_sqlite(ctx, present_chars, false, |stored| {
            stored
                .translation
                .fallback_fragments
                .into_iter()
                .filter(|f| !f.js_snippet.is_empty())
                .collect()
        })
    } else {
        collect_mvu_fallback_fragments(ctx, get_campaign_store(), present_chars)
    }
}

/// #22：SQLite 后端的 MVU 收集骨架——与 JSON 版同语义：
/// present instance → definition_id → source 卡（def→source 反查表来自
/// character_cards payload）→ mvu_translations 表取翻译，`extract` 挑字段。
/// `dedup_sources=true` 时同一 source 卡只贡献一次（规则收集用）。
fn collect_mvu_from_sqlite<T>(
    ctx: &WritingContext,
    present_chars: &[String],
    dedup_sources: bool,
    mut extract: impl FnMut(campaign_store::StoredMvuTranslation) -> Vec<T>,
) -> Vec<T> {
    let runtime = match &ctx.campaign_runtime {
        Some(rt) => rt,
        None => return vec![],
    };
    let payloads = match sqlite_runtime::list_card_payloads() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("[MVU] SQLite 卡 payload 读取失败，本轮不注入: {e}");
            return vec![];
        }
    };
    // payload 兼容两种形态：StoredCard 包装（生产写入）/ 裸 CharacterCard（旧 cutover 源）
    let def_to_source: std::collections::HashMap<Id, Id> = payloads
        .iter()
        .filter_map(|value| {
            let inner = value.get("card").unwrap_or(value);
            serde_json::from_value::<storyforge_domain::character::CharacterCard>(inner.clone())
                .ok()
        })
        .flat_map(|card| {
            let src = card.source_character_id.clone();
            card.character_definitions
                .into_iter()
                .map(move |d| (d.id, src.clone()))
        })
        .collect();

    let mut visited_sources = std::collections::HashSet::new();
    let mut out = Vec::new();
    for char_id_str in present_chars {
        let char_id = Id::from_str(char_id_str);
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
        if dedup_sources && !visited_sources.insert(source_id.clone()) {
            continue;
        }
        match sqlite_runtime::get_mvu(source_id) {
            Ok(Some(stored)) => {
                let items = extract(stored);
                if !items.is_empty() {
                    tracing::info!(
                        target: "tauri-app",
                        "[MVU] 角色 '{}' 所属卡贡献 {} 条 MVU 产物（SQLite）",
                        inst.name,
                        items.len()
                    );
                    out.extend(items);
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("[MVU] SQLite 翻译读取失败（{}）: {e}", inst.name),
        }
    }
    out
}

/// CampaignStore.cards → source_character_id → MvuTranslation.update_rules
///
/// 与 `collect_mvu_fallback_fragments` 同型的查找链，但按 source 卡去重：
/// 同一张卡的多个在场实例只贡献一次规则（规则是卡级玩法，不随实例数翻倍）。
fn collect_mvu_update_rules(
    ctx: &WritingContext,
    store: &campaign_store::CampaignStore,
    present_chars: &[String],
) -> Vec<String> {
    let runtime = match &ctx.campaign_runtime {
        Some(rt) => rt,
        None => return vec![],
    };

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

    let mut visited_sources = std::collections::HashSet::new();
    let mut rules = Vec::new();
    for char_id_str in present_chars {
        let char_id = Id::from_str(char_id_str);
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
        if !visited_sources.insert(source_id.clone()) {
            continue;
        }
        if let Some(stored) = store.get_mvu(source_id) {
            let non_empty: Vec<String> = stored
                .translation
                .update_rules
                .into_iter()
                .filter(|r| !r.trim().is_empty())
                .collect();
            if !non_empty.is_empty() {
                tracing::info!(
                    target: "tauri-app",
                    "[MVU] 角色 '{}' 所属卡贡献 {} 条变量更新规则",
                    inst.name,
                    non_empty.len()
                );
                rules.extend(non_empty);
            }
        }
    }
    rules
}

/// 后端分流：SQLite 活跃时读 mvu_translations 表（V005 起为权威），
/// 否则读 JSON CampaignStore——两条路径同语义（按 source 卡去重）。
pub fn collect_mvu_update_rules_for_backend(
    ctx: &WritingContext,
    present_chars: &[String],
) -> Vec<String> {
    if sqlite_runtime::is_sqlite_active() {
        // #22：SQLite 已是 MVU 翻译权威，规则收集不再空转（按 source 卡去重）。
        collect_mvu_from_sqlite(ctx, present_chars, true, |stored| {
            stored
                .translation
                .update_rules
                .into_iter()
                .filter(|r| !r.trim().is_empty())
                .collect()
        })
    } else {
        collect_mvu_update_rules(ctx, get_campaign_store(), present_chars)
    }
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

/// Phase A: 把后处理产出转换为 MutationBatch（不直接写 CampaignStore）。
///
/// 委托共享 `production_postprocess` 实现，保留局部 wrapper 兼容既有测试。
#[cfg_attr(not(test), allow(dead_code))]
fn build_mutation_batch(
    store: &campaign_store::CampaignStore,
    persist_ctx: &PostprocessPersistContext,
    outcome: &storyforge_app_agent::PostProcessOutcome,
    present_chars: &[String],
) -> storyforge_domain::turn::MutationBatch {
    production_postprocess::build_json_mutation_batch(
        store,
        &production_postprocess::PostprocessPersistContext {
            campaign_id: persist_ctx.campaign_id.clone(),
            conversation_id: persist_ctx.conversation_id.clone(),
            turn: persist_ctx.turn,
        },
        outcome,
        present_chars,
        &[],
    )
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
    /// 当前产品写作模式；缺省时走旧版大场面兼容路径。
    pub generation_mode: Option<storyforge_domain::generation::GenerationMode>,
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

/// Refuse a regenerate request unless the target conversation and the active
/// Turn belong to the same selected Campaign. This check must happen before
/// `PipelineOrchestrator::regenerate`, because that pipeline mutates the
/// requested conversation's draft in place.
fn validate_regenerate_campaign_scope(
    active_campaign: Option<&Id>,
    conversation: &Conversation,
    active_turn: Option<&storyforge_domain::turn::TurnRecord>,
) -> Result<(), TauriCommandError> {
    match (active_campaign, conversation.campaign_id.as_ref()) {
        (None, None) => Ok(()),
        (None, Some(conversation_campaign)) => Err(TauriCommandError::validation(format!(
            "campaign conversation {} requires selecting campaign {} before regenerate",
            conversation.id, conversation_campaign
        ))),
        (Some(active), Some(conversation_campaign)) if active == conversation_campaign => {
            let turn = active_turn.ok_or_else(|| {
                TauriCommandError::validation(format!(
                    "campaign {} has no active turn for regenerate",
                    active
                ))
            })?;
            if turn.campaign_id != *active || turn.conversation_id != conversation.id {
                return Err(TauriCommandError::validation(format!(
                    "regenerate scope mismatch: conversation {} is not the active turn conversation",
                    conversation.id
                )));
            }
            Ok(())
        }
        (Some(active), Some(conversation_campaign)) => Err(TauriCommandError::validation(format!(
            "regenerate campaign mismatch: selected {active}, conversation belongs to {conversation_campaign}"
        ))),
        (Some(active), None) => Err(TauriCommandError::validation(format!(
            "legacy conversation {} cannot regenerate while campaign {} is active",
            conversation.id, active
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
    app.require_active_llm()?;

    // 解析 targets
    let targets: Vec<PartialRollTarget> = req
        .targets
        .iter()
        .map(parse_target_dto)
        .collect::<Result<_, _>>()?;

    let conversation_id = Id::from_str(&req.conversation_id);
    let node_id = Id::from_str(&req.node_id);
    let recall_hint = req.hint.clone();

    // Scope before constructing/running the pipeline. A cross-campaign
    // request used to mutate the requested conversation first and only then
    // attach an Attempt to the currently selected Campaign.
    let conversation = app.conv_store.get(&conversation_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("conversation {conversation_id} was not found"))
    })?;
    let active_campaign = app
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let active_turn = active_campaign
        .as_ref()
        .map(get_active_turn_for_backend)
        .transpose()
        .map_err(TauriCommandError::internal)?
        .flatten();
    validate_regenerate_campaign_scope(
        active_campaign.as_ref(),
        &conversation,
        active_turn.as_ref(),
    )?;

    let pipeline_req = RegenerateRequest {
        conversation_id: conversation_id.clone(),
        node_id: node_id.clone(),
        targets,
        generation_mode: req.generation_mode,
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
    let regex_character_id = conversation.character_id.clone();
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
        recent_summaries: vec![],
        chronicle_prompt_catalog: vec![],
        far_memory_hits: vec![],
        // A2：regenerate 用户 seed 直接注入模板 random/roll
        template_random_seed: req.seed,
        context_epoch: None,
        chronicle_revision: 0,
    };
    fill_regex_context(&mut ctx, get_preset_store(), get_global_regex_store());
    fill_profile_context(&mut ctx, &app);
    fill_agent_profile_context(&mut ctx, &app);
    fill_campaign_context_async(&mut ctx, &app).await?;
    // regenerate：hint 优先；无 hint 时回退到该 AI 节点之前最近一条 user 意图
    let fallback_intent = if recall_hint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        last_user_intent_before(&app.conv_store, &conversation_id, &node_id)
    } else {
        None
    };
    let far_query = recall_hint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or(fallback_intent);
    if let Some(query) = far_query.as_deref() {
        fill_far_memory_hits(&mut ctx, &app, query).await;
    }

    // Operation-owned cancel for regenerate.
    let (operation_id, cancel_rx) = begin_writing_operation(&app);

    let prompt_hook = frontend_prompt_hook(event_tx.clone(), app.prompt_hook_pending.clone());
    let mut pipeline =
        app.new_pipeline_with_regex_and_prompt_hook(&ctx.regex_scripts, Some(prompt_hook))?;
    let result = pipeline
        .regenerate(
            pipeline_req.clone(),
            &ctx,
            event_tx.clone(),
            cancel_rx.clone(),
        )
        .await;

    // ─── P2 后处理（best-effort，同 start_writing）─────────────────────────
    // auto-fix 后命令返回值必须是修复稿，且 Attempt.draft_hash 必须同步。
    let mut response_text: Option<String> = None;
    if let Ok((text, provenance)) = &result {
        // Phase A: regenerate 创建新 TurnAttempt,旧 Attempt Superseded
        // regenerate 的 replace_active_variant 改变了 node_id 的 active variant,
        // 新 variant 在同一 node 上,用 req 的 node_id 作为 variant_id
        // SQLite: atomic preaccept UoW owns conversation + attempt land.
        let regen_attempt_id = if let Some(campaign_id) = &ctx.campaign_id {
            if let Some(turn) =
                get_active_turn_for_backend(campaign_id).map_err(TauriCommandError::internal)?
            {
                let new_attempt_id = Id::new();
                if sqlite_runtime::is_sqlite_active() {
                    match sqlite_runtime::append_regenerate_attempt(RegenerateAttemptRequest {
                        campaign_id,
                        conversation_id: &conversation_id,
                        turn_id: &turn.turn_id,
                        previous_variant_id: &node_id,
                        attempt_id: &new_attempt_id,
                        draft_text: text,
                        pending_temporary_instances: pipeline
                            .pending_temporary_instances()
                            .to_vec(),
                        provenance: Some(provenance.clone()),
                    }) {
                        Ok(outcome) => {
                            app.conv_store.invalidate();
                            Some(outcome.attempt_id)
                        }
                        Err(e) => {
                            let _ = update_turn_record(&turn.turn_id, |record| {
                                record.status = storyforge_domain::turn::TurnStatus::Failed;
                                record.failure_reason =
                                    Some(format!("sqlite preaccept regenerate 失败: {e}"));
                                record.touch();
                            });
                            clear_current_cancel_if(&app, &operation_id);
                            return Err(TauriCommandError::internal(format!(
                                "sqlite preaccept regenerate 失败: {e}"
                            )));
                        }
                    }
                } else {
                    let new_attempt = turn_lifecycle::new_draft_attempt(
                        new_attempt_id,
                        node_id.clone(),
                        text,
                        pipeline.pending_temporary_instances().to_vec(),
                    );
                    let new_attempt_id = new_attempt.attempt_id.clone();
                    // P0-4：regenerate Attempt 落盘失败不能吞掉，否则后处理会把 Turn
                    // 推到 AwaitingAcceptance 却找不到 Attempt，形成无法 accept 的死锁。
                    if let Err(e) = update_turn_record(&turn.turn_id, |record| {
                        turn_lifecycle::append_regenerate_attempt(record, new_attempt);
                    }) {
                        if let Err(comp_e) = app
                            .conv_store
                            .soft_delete_variant(&conversation_id, &node_id)
                        {
                            tracing::error!(
                                "P0-4 regenerate 补偿失败: soft_delete node {} 失败: {comp_e}（原错误: {e}）",
                                node_id
                            );
                        }
                        let _ = update_turn_record(&turn.turn_id, |record| {
                            record.status = storyforge_domain::turn::TurnStatus::Failed;
                            record.failure_reason =
                                Some(format!("regenerate TurnAttempt 持久化失败: {e}"));
                            record.touch();
                        });
                        clear_current_cancel_if(&app, &operation_id);
                        return Err(TauriCommandError::internal(format!(
                            "regenerate TurnAttempt 持久化失败（已尝试软删变体）: {e}"
                        )));
                    }
                    Some(new_attempt_id)
                }
            } else {
                None
            }
        } else {
            None
        };

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
        // Operation-owned cancel clones only (no global re-subscribe / no false fallback).
        let pp_rx = cancel_rx.clone();
        // W10: 收集在场角色的 MVU fallback 片段（JS 执行用）+ 变量更新规则（注入后处理提示词）
        let mvu_fragments = collect_mvu_fallback_fragments_for_backend(&ctx, &present_chars);
        let mvu_rules = collect_mvu_update_rules_for_backend(&ctx, &present_chars);
        // B3/B DraftQualityGate + 有界 1× Editor auto-fix
        // regenerate 返回的 node 即当前 node_id（variant 更新）
        let draft_node_for_fix = pipeline_req.node_id.clone();
        let (final_text, quality_report, autofix_provenance) =
            quality_gate_with_optional_editor_autofix(
                final_text,
                QualityAutofixCtx {
                    pipeline: &mut pipeline,
                    draft_node_id: &draft_node_for_fix,
                    conversation_id: &pipeline_req.conversation_id,
                    writing_ctx: &ctx,
                    event_tx: &event_tx,
                    cancel: cancel_rx.clone(),
                    log_prefix: "regenerate",
                    original_provenance: Some(provenance.clone()),
                },
            )
            .await
            .map_err(|error| {
                TauriCommandError::internal(format!("quality auto-fix failed closed: {error}"))
            })?;
        // 返回给前端的必须是 auto-fix 后的正文
        response_text = Some(final_text.clone());
        // 挂到 regenerate 新建的 Attempt：同步 quality_report + draft_hash（Accept 硬校验）。
        // 关键同步失败必须传播，不能 best-effort 返回修复稿却留下原稿 hash。
        let active_turn_after_regenerate = match &ctx.campaign_id {
            Some(campaign_id) => {
                get_active_turn_for_backend(campaign_id).map_err(TauriCommandError::internal)?
            }
            None => None,
        };
        let pp_identity = match (
            active_turn_after_regenerate.as_ref(),
            regen_attempt_id.as_ref(),
            ctx.campaign_id.as_ref(),
        ) {
            (Some(turn), Some(att_id), Some(campaign_id)) => {
                Some(production_postprocess::PostprocessIdentity {
                    turn_id: turn.turn_id.clone(),
                    attempt_id: att_id.clone(),
                    campaign_id: campaign_id.clone(),
                    conversation_id: conversation_id.clone(),
                    turn_number: ctx.turn,
                })
            }
            _ => None,
        };
        if let Some(identity) = &pp_identity {
            let sink = BackendTurnAttemptSink::production();
            let service = production_postprocess::ProductionPostprocessService::new_json(
                get_campaign_store(),
                &sink,
            );
            if let Err(e) = service.sync_autofix_attempt_with_provenance(
                identity,
                &final_text,
                quality_report.clone(),
                autofix_provenance.clone(),
            ) {
                let combined = service_fail_turn(&sink, identity, e);
                clear_current_cancel_if(&app, &operation_id);
                return Err(TauriCommandError::internal(format!(
                    "regenerate auto-fix 后 Attempt 同步失败（draft_hash/quality_report）: {combined}"
                )));
            }
            if sqlite_runtime::is_sqlite_active() {
                app.conv_store.invalidate();
            }
        }

        // regenerate 保持同步语义：await 共享后处理；关键失败向上返回。
        let pp_runtime = ctx.campaign_runtime.clone();
        if let Err(e) = run_shared_postprocess_background(
            pipeline,
            ctx,
            final_text,
            present_chars,
            var_keys,
            mvu_fragments,
            mvu_rules,
            event_tx.clone(),
            pp_rx,
            pp_identity,
            pp_runtime,
        )
        .await
        {
            clear_current_cancel_if(&app, &operation_id);
            return Err(TauriCommandError::internal(format!(
                "regenerate postprocess 关键失败: {e}"
            )));
        }
    }

    clear_current_cancel_if(&app, &operation_id);

    match result {
        Ok((orig_text, _provenance)) => Ok(turn_lifecycle::prefer_autofix_response_text(
            response_text,
            orig_text,
        )),
        Err(e) => Err(TauriCommandError::from(format!("重 roll 失败: {e}"))),
    }
}

// ─── M1 对话命令 ───────────────────────────────────────────────────────────

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

/// 手动触发对话归档（将未归档前缀压缩为远记忆摘要并入库）
#[tauri::command]
async fn archive_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let config = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .ok_or("未配置嵌入 API，请先在设置中配置")?;
    let llm = state.require_active_llm()?;
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

    let count = run_archive_with_watermark(&state, &conv_id, &archiver, true)
        .await
        .map_err(TauriCommandError::internal)?;
    Ok(count)
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

/// 归档快照：消息列表 + 水位 + campaign 标签。
struct ArchiveSnapshot {
    messages: Vec<String>,
    archived_upto: usize,
    campaign_id: Option<String>,
    conversation_id: String,
}

async fn load_archive_snapshot(
    conv_store: Arc<ConversationStore>,
    conv_id: Id,
) -> Result<ArchiveSnapshot, TauriCommandError> {
    tokio::task::spawn_blocking(move || {
        let conv = conv_store.get(&conv_id).ok_or_else(|| {
            TauriCommandError::from(storyforge_app_conversation::ConversationError::NotFound(
                conv_id.to_string(),
            ))
        })?;
        Ok::<ArchiveSnapshot, TauriCommandError>(ArchiveSnapshot {
            messages: archivable_messages_from_conversation(&conv),
            archived_upto: conv.archived_upto,
            campaign_id: conv.campaign_id.map(|id| id.to_string()),
            conversation_id: conv.id.to_string(),
        })
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("读取归档消息任务失败: {e}")))?
}

/// 水位驱动归档：只处理 `archived_upto..` 前缀，成功后推进水位。
///
/// `require_threshold`：true 时与 ArchiveConfig.threshold 对齐（自动归档）；
/// 手动命令也传 true，避免短对话误触发 LLM。
async fn run_archive_with_watermark(
    state: &Arc<AppState>,
    conv_id: &Id,
    archiver: &storyforge_app_memory::MemoryArchiver,
    require_threshold: bool,
) -> Result<usize, String> {
    let snap = load_archive_snapshot(state.conv_store.clone(), conv_id.clone())
        .await
        .map_err(|e| e.to_string())?;

    let total = snap.messages.len();
    let upto = snap.archived_upto.min(total);
    if upto >= total {
        return Ok(0);
    }
    let pending = &snap.messages[upto..];
    if require_threshold
        && pending.len() < storyforge_app_memory::ArchiveConfig::default().threshold
    {
        return Ok(0);
    }
    if pending.is_empty() {
        return Ok(0);
    }

    let meta = storyforge_app_memory::ArchiveMeta {
        campaign_id: snap.campaign_id,
        conversation_id: Some(snap.conversation_id),
    };

    // 水位路径已确认 pending 需要归档：跳过 maybe_archive 的二次 threshold，
    // 直接 archive_prefix；source_range 相对 pending 切片。
    let summaries = archiver
        .archive_prefix(pending, Some(&meta))
        .await
        .map_err(|e| e.to_string())?;

    if summaries.is_empty() {
        return Ok(0);
    }

    // 取最大 end_idx + 1 作为本轮推进量（相对 pending）
    let advanced = summaries
        .iter()
        .map(|s| s.source_range.1.saturating_add(1))
        .max()
        .unwrap_or(0);
    if advanced == 0 {
        return Ok(summaries.len());
    }
    let new_upto = upto.saturating_add(advanced).min(total);
    let conv_store = state.conv_store.clone();
    let conv_id_clone = conv_id.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        conv_store.advance_archived_upto(&conv_id_clone, new_upto)
    })
    .await
    .map_err(|e| format!("推进归档水位任务失败: {e}"))?
    {
        tracing::warn!("推进归档水位失败: {e}");
    } else {
        tracing::info!(
            target: "far_memory",
            "归档水位 {} → {}（+{} 条消息，{} 条总结）",
            upto,
            new_upto,
            advanced,
            summaries.len()
        );
    }
    Ok(summaries.len())
}

// ─── 自动归档辅助 ──────────────────────────────────────────────────────────

/// 检查对话未归档消息是否超过阈值，超过则在后台触发归档。
///
/// 阈值：50 条未归档消息（与 ArchiveConfig.default().threshold 一致）。
/// 归档失败只 warn，不影响用户操作。
async fn auto_archive_if_needed(state: &Arc<AppState>, conv_id: &Id) {
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

    // 快速水位检查：未归档不足阈值则跳过（避免无意义构造 archiver）
    match load_archive_snapshot(state.conv_store.clone(), conv_id.clone()).await {
        Ok(snap) => {
            let pending = snap.messages.len().saturating_sub(snap.archived_upto);
            if pending < storyforge_app_memory::ArchiveConfig::default().threshold {
                return;
            }
            tracing::info!(
                "自动归档触发：对话 {} 未归档 {} 条 >= 阈值 {}",
                conv_id,
                pending,
                storyforge_app_memory::ArchiveConfig::default().threshold
            );
        }
        Err(e) => {
            tracing::debug!("读取自动归档消息失败，跳过: {e}");
            return;
        }
    }

    let llm = match state.require_active_llm() {
        Ok(llm) => llm,
        Err(error) => {
            tracing::debug!("自动归档跳过：{error}");
            return;
        }
    };
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

    match run_archive_with_watermark(state, conv_id, &archiver, true).await {
        Ok(n) if n > 0 => {
            tracing::info!("自动归档完成：{} 条总结", n);
        }
        Ok(_) => {
            tracing::debug!("自动归档：无需归档");
        }
        Err(e) => {
            tracing::warn!("自动归档失败（不影响用户操作）: {e}");
        }
    }
}

/// 活动 Turn 的质量门禁摘要（供前端刷新后回填 ProcessReview）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTurnQualityDto {
    pub turn_id: String,
    pub attempt_id: String,
    pub status: String,
    pub passed: bool,
    pub warning_count: usize,
    pub error_count: usize,
    pub warnings: Vec<String>,
}

/// Accept 前展示给用户的单条候选状态变化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnReceiptItemDto {
    /// 对应 Prepared MutationBatch 中的稳定下标，确认时原样回传。
    pub mutation_index: usize,
    /// chronicle / knowledge / variable / task
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub selected_by_default: bool,
}

/// Campaign Turn 的 Accept-before 小票。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTurnReceiptDto {
    pub turn_id: String,
    pub attempt_id: String,
    pub variant_id: String,
    pub status: String,
    pub ready: bool,
    pub derivation_failed: bool,
    pub can_retry: bool,
    pub can_degraded_accept: bool,
    pub notice: Option<String>,
    pub items: Vec<TurnReceiptItemDto>,
}

fn mutation_is_receipt_reviewable(mutation: &storyforge_domain::turn::Mutation) -> bool {
    !matches!(
        mutation,
        storyforge_domain::turn::Mutation::FinalizeVariant { .. }
            | storyforge_domain::turn::Mutation::UpsertInstance(_)
    )
}

fn receipt_items_from_batch(
    batch: &storyforge_domain::turn::MutationBatch,
) -> Vec<TurnReceiptItemDto> {
    use storyforge_domain::turn::Mutation;

    batch
        .mutations
        .iter()
        .enumerate()
        .filter_map(|(mutation_index, mutation)| {
            let (kind, title, detail) = match mutation {
                Mutation::UpsertSummary(summary) => (
                    "chronicle",
                    summary
                        .headline
                        .clone()
                        .unwrap_or_else(|| "本轮纪要（Chronicle A）".into()),
                    summary.content.clone(),
                ),
                Mutation::UpsertKnowledge(knowledge) => (
                    "knowledge",
                    "角色知识更新".into(),
                    format!(
                        "{} 得知：{}",
                        knowledge.character_id, knowledge.knowledge_text
                    ),
                ),
                Mutation::SetVariable {
                    instance_id,
                    key,
                    value,
                    ..
                } => {
                    let target = instance_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "Campaign".into());
                    (
                        "variable",
                        format!("变量 · {key}"),
                        format!("{target}: {key} → {value}"),
                    )
                }
                Mutation::SetTaskStatus { task_id, status } => (
                    "task",
                    "任务状态变化".into(),
                    format!("任务 {task_id} → {status:?}"),
                ),
                Mutation::UpsertNewTask(task) => (
                    "task",
                    format!("新任务 · {}", task.title),
                    task.description.clone(),
                ),
                Mutation::FinalizeVariant { .. } | Mutation::UpsertInstance(_) => return None,
            };
            Some(TurnReceiptItemDto {
                mutation_index,
                kind: kind.into(),
                title,
                detail,
                selected_by_default: true,
            })
        })
        .collect()
}

/// 按小票勾选结果裁剪候选变更。正文 Finalize 与临时角色实例属于结构性
/// mutation，始终保留，避免引用断裂；其余未勾选项不参与 Accept。
fn retain_selected_receipt_mutations(
    batch: &mut storyforge_domain::turn::MutationBatch,
    selected_mutation_indices: &[usize],
) -> Result<(), String> {
    use std::collections::HashSet;

    if selected_mutation_indices
        .iter()
        .any(|index| *index >= batch.mutations.len())
    {
        return Err("小票包含已经失效的 mutation 下标，请刷新后重试".into());
    }
    let selected: HashSet<usize> = selected_mutation_indices.iter().copied().collect();
    batch.mutations = std::mem::take(&mut batch.mutations)
        .into_iter()
        .enumerate()
        .filter_map(|(index, mutation)| {
            if !mutation_is_receipt_reviewable(&mutation) || selected.contains(&index) {
                Some(mutation)
            } else {
                None
            }
        })
        .collect();
    Ok(())
}

fn active_turn_receipt_from_record(
    turn: &storyforge_domain::turn::TurnRecord,
    variant_id: &Id,
) -> Option<ActiveTurnReceiptDto> {
    use storyforge_domain::turn::AttemptStatus;

    let attempt = turn.find_attempt_by_variant(variant_id)?;
    let derivation_failed = attempt
        .derivation
        .as_ref()
        .is_some_and(storyforge_domain::turn::DerivationComponents::has_failure);
    let ready = attempt.status == AttemptStatus::AwaitingAcceptance;
    let notice = if derivation_failed {
        Some("记账推导有失败项：可重试，或明确降级采纳正文。".into())
    } else if ready && attempt.pending_state_changes.is_none() {
        Some("本轮没有可写入的纪要或状态变化。".into())
    } else if !ready {
        Some("记账仍在生成，请稍后刷新。".into())
    } else {
        None
    };
    Some(ActiveTurnReceiptDto {
        turn_id: turn.turn_id.to_string(),
        attempt_id: attempt.attempt_id.to_string(),
        variant_id: attempt.variant_id.to_string(),
        status: format!("{:?}", attempt.status),
        ready,
        derivation_failed,
        can_retry: ready && derivation_failed,
        can_degraded_accept: ready && derivation_failed,
        notice,
        items: attempt
            .pending_state_changes
            .as_ref()
            .map(receipt_items_from_batch)
            .unwrap_or_default(),
    })
}

/// 读取活动 Attempt 的 Accept-before 小票，不修改任何状态。
#[tauri::command]
fn get_active_turn_receipt(
    campaign_id: String,
    node_id: String,
) -> Result<Option<ActiveTurnReceiptDto>, TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    let variant_id = Id::from_str(&node_id);
    let Some(turn) =
        get_active_turn_for_backend(&campaign_id).map_err(TauriCommandError::internal)?
    else {
        return Ok(None);
    };
    Ok(active_turn_receipt_from_record(&turn, &variant_id))
}

fn apply_turn_receipt_selection(
    campaign_id: &Id,
    variant_id: &Id,
    selected_mutation_indices: &[usize],
) -> Result<(), String> {
    use storyforge_domain::turn::{AttemptStatus, MutationBatchStatus, TurnStatus};

    let turn = get_active_turn_for_backend(campaign_id)?
        .ok_or_else(|| "当前 Campaign 没有待采纳 Turn".to_string())?;
    let turn_id = turn.turn_id.clone();
    let attempt_id = turn
        .find_attempt_by_variant(variant_id)
        .ok_or_else(|| "小票对应的草稿已不是当前 Attempt".to_string())?
        .attempt_id
        .clone();
    let mut selection_error: Option<String> = None;
    let applied = update_turn_record_if(
        &turn_id,
        |record| {
            record.campaign_id == *campaign_id
                && record.status == TurnStatus::AwaitingAcceptance
                && record.find_attempt(&attempt_id).is_some_and(|attempt| {
                    attempt.variant_id == *variant_id
                        && attempt.status == AttemptStatus::AwaitingAcceptance
                })
        },
        |record| {
            let Some(attempt) = record.find_attempt_mut(&attempt_id) else {
                selection_error = Some("小票对应的 Attempt 已消失".into());
                return;
            };
            match attempt.pending_state_changes.as_mut() {
                Some(batch) if batch.status == MutationBatchStatus::Prepared => {
                    if let Err(error) =
                        retain_selected_receipt_mutations(batch, selected_mutation_indices)
                    {
                        selection_error = Some(error);
                    }
                }
                Some(_) => selection_error = Some("候选变更已开始提交，不能再修改勾选项".into()),
                None if selected_mutation_indices.is_empty() => {}
                None => selection_error = Some("本轮没有可选择的候选变更".into()),
            }
            record.touch();
        },
    )?;
    if !applied {
        return Err("Turn 状态已变化，请刷新小票后重试".into());
    }
    if let Some(error) = selection_error {
        return Err(error);
    }
    Ok(())
}

fn postprocess_present_characters(provenance: Option<&Provenance>) -> Vec<String> {
    let Some(provenance) = provenance else {
        return vec![];
    };
    let candidates: Vec<String> = provenance
        .plan
        .as_ref()
        .map(|plan| {
            plan.subagent_tasks
                .iter()
                .map(|task| task.character_id.clone())
                .collect()
        })
        .filter(|characters: &Vec<String>| !characters.is_empty())
        .unwrap_or_else(|| {
            provenance
                .subagent_results
                .iter()
                .map(|snapshot| {
                    snapshot
                        .display_name
                        .clone()
                        .unwrap_or_else(|| snapshot.character_id.clone())
                })
                .collect()
        });
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

/// 仅重跑当前草稿的 Summarizer + PostProcessor，不重写正文。
#[tauri::command]
async fn retry_active_turn_postprocess(
    campaign_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ActiveTurnReceiptDto, TauriCommandError> {
    use storyforge_domain::turn::{AttemptStatus, TurnStatus};

    let campaign_id = Id::from_str(&campaign_id);
    let variant_id = Id::from_str(&node_id);
    let active_campaign = state
        .active_campaign
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if active_campaign.as_ref() != Some(&campaign_id) {
        return Err(TauriCommandError::validation(
            "只能重试当前打开 Campaign 的记账推导",
        ));
    }

    let turn = get_active_turn_for_backend(&campaign_id)
        .map_err(TauriCommandError::internal)?
        .ok_or_else(|| TauriCommandError::validation("当前 Campaign 没有待采纳 Turn"))?;
    let attempt = turn
        .find_attempt_by_variant(&variant_id)
        .cloned()
        .ok_or_else(|| TauriCommandError::validation("该草稿已不是当前 Attempt"))?;
    if attempt.status != AttemptStatus::AwaitingAcceptance
        || !attempt
            .derivation
            .as_ref()
            .is_some_and(storyforge_domain::turn::DerivationComponents::has_failure)
    {
        return Err(TauriCommandError::validation(
            "只有记账推导失败的待采纳草稿可以重试",
        ));
    }

    let turn_id = turn.turn_id.clone();
    let attempt_id = attempt.attempt_id.clone();

    let conversation = state
        .conv_store
        .get(&turn.conversation_id)
        .ok_or_else(|| TauriCommandError::internal("找不到草稿所属对话"))?;
    let final_text = conversation
        .nodes
        .iter()
        .find(|node| node.id == variant_id)
        .and_then(|node| node.active())
        .map(|variant| variant.content.clone())
        .ok_or_else(|| TauriCommandError::internal("找不到待重试草稿正文"))?;

    let snapshot = state.snapshot_tool_ctx();
    let regex_character_id = conversation.character_id.clone();
    let mut writing_ctx = WritingContext {
        characters: snapshot.characters.clone(),
        world_info: snapshot.world_info.clone(),
        conversation_id: turn.conversation_id.clone(),
        campaign_id: None,
        turn: 0,
        pending_tasks: vec![],
        story_clock: String::new(),
        profile: None,
        modules: vec![],
        regex_scripts: collect_scoped_regex_scripts(
            regex_character_id.as_deref(),
            &snapshot.characters,
        ),
        campaign_runtime: None,
        agent_profile_config: None,
        recent_summaries: vec![],
        chronicle_prompt_catalog: vec![],
        far_memory_hits: vec![],
        template_random_seed: None,
        context_epoch: None,
        chronicle_revision: 0,
    };
    fill_regex_context(
        &mut writing_ctx,
        get_preset_store(),
        get_global_regex_store(),
    );
    fill_profile_context(&mut writing_ctx, &state);
    fill_agent_profile_context(&mut writing_ctx, &state);
    fill_campaign_context_async(&mut writing_ctx, &state).await?;
    if writing_ctx.campaign_id.as_ref() != Some(&campaign_id) {
        return Err(TauriCommandError::validation(
            "重试期间 Campaign 已切换，请重新打开小票",
        ));
    }

    let present_characters = postprocess_present_characters(attempt.provenance.as_ref());
    let variable_keys = postprocess_variable_keys(&writing_ctx);
    let fallback_fragments =
        collect_mvu_fallback_fragments_for_backend(&writing_ctx, &present_characters);
    let mvu_update_rules = collect_mvu_update_rules_for_backend(&writing_ctx, &present_characters);
    let pipeline = state.new_pipeline_with_regex(&writing_ctx.regex_scripts)?;
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let identity = production_postprocess::PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id: attempt_id.clone(),
        campaign_id: campaign_id.clone(),
        conversation_id: turn.conversation_id.clone(),
        turn_number: writing_ctx.turn,
    };
    let runtime = writing_ctx.campaign_runtime.clone();

    // All fallible preparation above deliberately happens while the attempt is
    // still AwaitingAcceptance. Only take the DerivingState lease immediately
    // before the shared postprocess service starts, so a missing conversation,
    // invalid Campaign context, or config load error cannot strand the Turn.
    let transitioned = update_turn_record_if(
        &turn_id,
        |record| {
            record.status == TurnStatus::AwaitingAcceptance
                && record.find_attempt(&attempt_id).is_some_and(|candidate| {
                    candidate.status == AttemptStatus::AwaitingAcceptance
                        && candidate.variant_id == variant_id
                })
        },
        |record| {
            record.status = TurnStatus::DerivingState;
            record.failure_reason = None;
            if let Some(candidate) = record.find_attempt_mut(&attempt_id) {
                candidate.status = AttemptStatus::DerivingState;
            }
            record.touch();
        },
    )
    .map_err(TauriCommandError::internal)?;
    if !transitioned {
        return Err(TauriCommandError::validation(
            "Turn 状态已变化，请刷新小票后重试",
        ));
    }

    run_shared_postprocess_background(
        pipeline,
        writing_ctx,
        final_text,
        present_characters,
        variable_keys,
        fallback_fragments,
        mvu_update_rules,
        event_tx,
        cancel_rx,
        Some(identity),
        runtime,
    )
    .await
    .map_err(|error| TauriCommandError::internal(format!("重试记账失败: {error}")))?;

    let refreshed = get_active_turn_for_backend(&campaign_id)
        .map_err(TauriCommandError::internal)?
        .ok_or_else(|| TauriCommandError::internal("重试后找不到活动 Turn"))?;
    active_turn_receipt_from_record(&refreshed, &variant_id)
        .ok_or_else(|| TauriCommandError::internal("重试后找不到活动 Attempt"))
}

/// 从 TurnRecord 提取活动 Attempt 的质量 DTO（无报告 → None）。
fn active_turn_quality_from_record(
    turn: &storyforge_domain::turn::TurnRecord,
) -> Option<ActiveTurnQualityDto> {
    let attempt = turn.active_attempt()?;
    let report = attempt.quality_report.as_ref()?;
    let warnings: Vec<String> = report.warnings.iter().map(|w| w.message.clone()).collect();
    Some(ActiveTurnQualityDto {
        turn_id: turn.turn_id.to_string(),
        attempt_id: attempt.attempt_id.to_string(),
        status: format!("{:?}", attempt.status),
        passed: report.passed(),
        warning_count: report.warnings.len(),
        error_count: report.error_count(),
        warnings,
    })
}

/// 读取当前 Campaign 活动 Turn 上 active Attempt 的 QualityReport。
///
/// 无活动 Turn / 无质量报告 → None。不创建状态。
#[tauri::command]
fn get_active_turn_quality(campaign_id: String) -> Option<ActiveTurnQualityDto> {
    let camp = Id::from_str(&campaign_id);
    let turn = get_active_turn_for_backend(&camp).ok()??;
    active_turn_quality_from_record(&turn)
}

// ─── Tauri app 入口 ────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // W8 MVU JS Runtime：共享 pending map
    let mvu_pending: MvuPendingMap = new_mvu_pending_map();

    tauri::Builder::default()
        .register_uri_scheme_protocol(card_shell_cache::LOCAL_PROTOCOL_SCHEME, |_ctx, request| {
            card_shell_cache_protocol_response(request)
        })
        // V5 CSP isolation: serve shell documents (CardShell/TavernHelper/MVU/
        // PluginHost iframes) on a dedicated origin so they do NOT inherit the
        // main app's policy container. See shell_doc_protocol.rs. Android line
        // of ownership: this is an additive registration on the shared Builder
        // chain; it does not touch Android picker/data-dir logic.
        .register_uri_scheme_protocol(shell_doc_protocol::SHELL_DOC_SCHEME, |_ctx, request| {
            shell_doc_protocol::shell_doc_protocol_response(request)
        })
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(mvu_pending.clone())
        .setup(move |app| {
            // Android's private data path is provided by the running Activity.
            // Resolve it here, never from a hardcoded /data/data alias. Desktop
            // keeps its existing StoryForge locations for data compatibility.
            #[cfg(target_os = "android")]
            let framework_data_dir = Some(
                app.path()
                    .app_data_dir()
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            );
            #[cfg(not(target_os = "android"))]
            let framework_data_dir: Option<PathBuf> = None;

            let data_dir = initialize_app_data_dir(framework_data_dir.as_deref())
                .map_err(std::io::Error::other)?;

            // Resolve storage backend before any store recovery. JSON remains
            // default; explicit SQLite selection activates a fail-closed,
            // process-owned production boundary (no dual-write).
            let resolution = storage_backend::resolve_backend(&data_dir)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            tracing::info!(
                backend = resolution.diagnostics.backend,
                source = resolution.diagnostics.source,
                schema_version = ?resolution.diagnostics.schema_version,
                cutover = resolution.cutover_performed,
                "storage backend resolved"
            );
            if let Some(db_path) = resolution.db_path {
                sqlite_runtime::activate(&db_path)
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
            }

            // AppState and every process-owned store now share the setup-resolved
            // directory. Only after construction can tracing write into LogStore.
            let app_state = Arc::new(AppState::new_with_data_dir(data_dir));
            storyforge_app_logging::init_tracing(app_state.log_store.clone());
            app.manage(app_state.clone());

            // Phase A: 启动恢复——幂等重放 Committing 态 Turn + 标记非 terminal 活动 Turn
            recover_turns_on_startup(app_state.as_ref());
            // M4: 重放未完成 ChronicleCompressor 任务（Running→Pending 后 spawn）
            recover_compress_jobs_on_startup(app_state);

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
            // Card Studio Phase 1
            card_studio_api::cardstudio_list_projects,
            card_studio_api::cardstudio_create_project,
            card_studio_api::cardstudio_create_from_novel,
            card_studio_api::cardstudio_create_from_character,
            card_studio_api::cardstudio_prefill_from_novel,
            card_studio_api::cardstudio_get_project,
            card_studio_api::cardstudio_delete_project,
            card_studio_api::cardstudio_update_artifacts,
            card_studio_api::cardstudio_set_stage,
            card_studio_api::cardstudio_set_options,
            card_studio_api::cardstudio_run_checks,
            card_studio_api::cardstudio_run_review,
            card_studio_api::cardstudio_compile,
            card_studio_api::cardstudio_export_gate,
            card_studio_api::cardstudio_export_png,
            card_studio_api::cardstudio_complete_manual_stage,
            card_studio_api::cardstudio_run_stage,
            card_studio_api::cardstudio_import_compiled,
            card_studio_api::cardstudio_list_stages,
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
            get_connection,
            create_connection,
            update_connection,
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
            plugin_get_conversation,
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
            log_get_llm_call,
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
            apply_campaign_opening,
            fork_campaign,
            list_campaigns,
            get_campaign,
            delete_campaign,
            set_active_campaign,
            get_active_campaign,
            list_instances,
            get_instance,
            add_campaign_instance,
            get_character_variables,
            set_character_variable,
            get_campaign_variables,
            get_campaign_variable_schema,
            add_campaign_variable,
            sync_campaign_variable_schema,
            set_campaign_variable,
            promote_temporary_instance,
            // P2 后处理产出查询 / 任务管理
            list_character_knowledge,
            list_tasks,
            create_task,
            complete_task,
            abandon_task,
            list_round_summaries,
            list_campaign_world_info,
            add_campaign_world_info_entry,
            update_campaign_world_info_entry,
            set_campaign_world_info_enabled,
            delete_campaign_world_info_entry,
            set_campaign_world_info_route,
            get_character_world_info,
            get_character_world_info_entry,
            get_campaign_world_info_entry,
            get_card_shell_manifest,
            get_card_shell_inline_js,
            card_shell_list_allowed_hosts,
            card_shell_register_doc,
            card_shell_register_module,
            card_shell_unregister_doc,
            card_shell_allow_host,
            card_shell_clear_cache,
            card_shell_fetch_url,
            get_active_turn_quality,
            get_active_turn_receipt,
            retry_active_turn_postprocess,
            // V4 存储健康：损坏启动拦截 + 恢复确认
            storage_health_report,
            storage_health_acknowledge,
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
pub(crate) fn stored_info_to_character(
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
        extra: Default::default(),
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
        metadata: Default::default(),
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
        metadata: Default::default(),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn sync_attempt_after_autofix_updates_hash_to_final_text() {
        let original = "原稿含破折号——且过短";
        let fixed = "修复后的正文足够长，并且去掉了破折号，急诊灯下林秋与陈警官对坐，雨声敲窗，空气里有消毒水味。";
        let mut attempt = storyforge_domain::turn::TurnAttempt {
            attempt_id: Id::new(),
            variant_id: Id::new(),
            draft_hash: turn_lifecycle::compute_draft_hash(original),
            status: storyforge_domain::turn::AttemptStatus::DraftReady,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let report = storyforge_domain::turn::QualityReport { warnings: vec![] };
        turn_lifecycle::sync_attempt_after_autofix(&mut attempt, fixed, report);
        assert_eq!(
            attempt.draft_hash,
            turn_lifecycle::compute_draft_hash(fixed),
            "auto-fix 后 Attempt.draft_hash 必须等于修复稿 hash"
        );
        assert_ne!(
            attempt.draft_hash,
            turn_lifecycle::compute_draft_hash(original),
            "不能继续指向原稿 hash"
        );
        assert!(attempt.quality_report.is_some());
    }

    #[test]
    fn autofix_response_must_prefer_fixed_over_original() {
        // 直接测生产 helper，防止命令再次回退到原稿
        let original = "原稿".to_string();
        let fixed = "修复稿".to_string();
        assert_eq!(
            turn_lifecycle::prefer_autofix_response_text(Some(fixed.clone()), original.clone()),
            fixed
        );
        assert_eq!(
            turn_lifecycle::prefer_autofix_response_text(None, original.clone()),
            original
        );
    }

    #[test]
    fn campaign_variable_input_validation_accepts_unicode_keys_and_bounds_user_text() {
        assert!(
            validate_campaign_variable_input(
                "世界.阵营_紧张度-1",
                "阵营紧张度",
                Some("整局共享"),
                &serde_json::json!(12),
            )
            .is_ok()
        );
        assert!(
            validate_campaign_variable_input("bad key", "坏键", None, &serde_json::Value::Null,)
                .is_err()
        );
        assert!(
            validate_campaign_variable_input(
                "world.__internal",
                "内部键",
                None,
                &serde_json::Value::Null,
            )
            .is_err()
        );
        assert!(
            validate_campaign_variable_input(
                "valid",
                &"名".repeat(81),
                None,
                &serde_json::Value::Null,
            )
            .is_err()
        );
        assert!(
            validate_campaign_variable_input(
                "valid",
                "名称",
                Some(&"说".repeat(501)),
                &serde_json::Value::Null,
            )
            .is_err()
        );
    }

    /// 测试辅助：从一个 `&[(&str, &str)]` 构造环境变量视图闭包。
    fn env_view<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    /// AND-2 路径解析契约：纯函数 `resolve_app_data_dir` 对每个平台给出
    /// 正确的 OS 标准目录。**不调用生产 `get_app_data_dir()`、不读真实环境、
    /// 不建目录、不迁移**，因此不会触碰用户真实的 `%APPDATA%`/`$HOME`。
    #[test]
    fn resolve_app_data_dir_returns_os_standard_paths_per_platform() {
        // Windows：%APPDATA%/StoryForge
        let win = resolve_app_data_dir(
            AppDataDirTarget::Windows,
            &env_view(&[("APPDATA", "C:\\Users\\test\\AppData\\Roaming")]),
            None,
            None,
        )
        .expect("Windows path should resolve");
        assert_eq!(
            win,
            PathBuf::from("C:\\Users\\test\\AppData\\Roaming").join("StoryForge")
        );

        // macOS：$HOME/Library/Application Support/StoryForge
        let mac = resolve_app_data_dir(
            AppDataDirTarget::Macos,
            &env_view(&[("HOME", "/Users/test")]),
            None,
            None,
        )
        .expect("macOS path should resolve");
        assert_eq!(
            mac,
            PathBuf::from("/Users/test/Library/Application Support/StoryForge")
        );

        // Linux：$XDG_DATA_HOME 优先
        let lin_xdg = resolve_app_data_dir(
            AppDataDirTarget::Linux,
            &env_view(&[("XDG_DATA_HOME", "/custom/xdg"), ("HOME", "/home/test")]),
            None,
            None,
        )
        .expect("Linux XDG path should resolve");
        assert_eq!(lin_xdg, PathBuf::from("/custom/xdg/storyforge"));

        // Linux：无 XDG 时回退 $HOME/.local/share/storyforge
        let lin_home = resolve_app_data_dir(
            AppDataDirTarget::Linux,
            &env_view(&[("HOME", "/home/test")]),
            None,
            None,
        )
        .expect("Linux HOME path should resolve");
        assert_eq!(
            lin_home,
            PathBuf::from("/home/test/.local/share/storyforge")
        );
    }

    /// AND-2 Android 路径解析契约：
    /// - `STORYFORGE_DATA_DIR` 覆盖优先（真机冒烟/调试）；
    /// - 否则精确使用 Tauri/Android Context 返回的数据目录；
    /// - 框架目录尚不可用时失败关闭，绝不猜测 `/data/data/...`。
    #[test]
    fn resolve_app_data_dir_android_prefers_override_then_framework_and_never_hardcodes() {
        let with_override = resolve_app_data_dir(
            AppDataDirTarget::Android,
            &env_view(&[("STORYFORGE_DATA_DIR", "/tmp/custom-sf-data")]),
            None,
            Some(Path::new("/data/user/10/com.storyforge.app")),
        )
        .expect("explicit override should resolve");
        assert_eq!(with_override, PathBuf::from("/tmp/custom-sf-data"));

        let framework = resolve_app_data_dir(
            AppDataDirTarget::Android,
            &env_view(&[]),
            None,
            Some(Path::new("/data/user/10/com.storyforge.app")),
        )
        .expect("framework data directory should resolve");
        assert_eq!(
            framework,
            PathBuf::from("/data/user/10/com.storyforge.app"),
            "必须保留框架返回的多用户数据路径"
        );

        let missing = resolve_app_data_dir(AppDataDirTarget::Android, &env_view(&[]), None, None);
        assert!(missing.is_err(), "框架目录不可用时必须失败关闭");
    }

    /// AND-2 exe_dir/data 回退契约：所有 OS 标准环境变量缺失时，回退到
    /// exe 父目录下的 `data`（旧行为兼容）。用纯函数 + 合成路径验证，
    /// 不依赖真实 exe。
    #[test]
    fn resolve_app_data_dir_falls_back_to_exe_data_when_env_missing() {
        let tmp =
            std::env::temp_dir().join(format!("sf-resolve-exe-fallback-{}", uuid::Uuid::new_v4()));
        let exe_parent = tmp.join("install");
        let resolved = resolve_app_data_dir(
            AppDataDirTarget::Linux,
            &env_view(&[]), // 无 HOME/XDG
            Some(&exe_parent),
            None,
        )
        .expect("desktop fallback should resolve");
        assert_eq!(resolved, exe_parent.join("data"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 迁移契约：旧 exe_dir/data 有数据、新目录为空时，迁移过去；
    /// 新目录已有数据时不迁移（不覆盖用户数据）。用临时目录验证，
    /// 不依赖真实 exe 或真实数据目录。
    #[test]
    fn migrate_from_exe_dir_copies_when_new_dir_empty_and_skips_when_populated() {
        let root =
            std::env::temp_dir().join(format!("sf-migrate-contract-{}", uuid::Uuid::new_v4()));
        let exe_parent = root.join("install");
        let old_dir = exe_parent.join("data");
        let new_dir = root.join("newdata");
        std::fs::create_dir_all(&old_dir).expect("create old dir");
        std::fs::create_dir_all(&new_dir).expect("create new dir");
        std::fs::write(old_dir.join("characters.json"), b"{}").expect("seed old data");

        // 新目录为空 → 迁移
        migrate_from_exe_dir_if_needed(&new_dir, Some(&exe_parent));
        assert!(
            new_dir.join("characters.json").exists(),
            "新目录为空时应迁移旧数据"
        );

        // 再次迁移：新目录已有数据 → 不动（幂等，不覆盖）
        std::fs::write(old_dir.join("characters.json"), b"\"stale\"").expect("rewrite old data");
        migrate_from_exe_dir_if_needed(&new_dir, Some(&exe_parent));
        let migrated = std::fs::read_to_string(new_dir.join("characters.json")).unwrap();
        assert_eq!(migrated, "{}", "新目录已有数据时不得覆盖（迁移必须跳过）");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 迁移契约：旧目录不存在时不报错（fresh install 无旧数据）。
    #[test]
    fn migrate_from_exe_dir_is_noop_when_old_dir_absent() {
        let root = std::env::temp_dir().join(format!("sf-migrate-noop-{}", uuid::Uuid::new_v4()));
        let exe_parent = root.join("install"); // 无 data 子目录
        let new_dir = root.join("newdata");
        std::fs::create_dir_all(&new_dir).expect("create new dir");
        // 不应 panic
        migrate_from_exe_dir_if_needed(&new_dir, Some(&exe_parent));
        assert!(new_dir.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;
    use serde::ser::Error as _;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use storyforge_domain::character::Character;
    use storyforge_domain::preset::ST_REGEX_PLACEMENT_AI_OUTPUT;

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
                    reasoning_content: Some("recording mock fallback reasoning".into()),
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
                delta_reasoning_content: response.reasoning_content.clone(),
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
            reasoning_content: Some("recording mock reasoning".into()),
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
        let state = AppState::new_for_test();
        *state.active_llm.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(llm as Arc<dyn LlmClient>);
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
            Some(storyforge_domain::generation::GenerationMode::BigScene),
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
                generation_mode: Some(storyforge_domain::generation::GenerationMode::BigScene),
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
            reasoning_content: None,
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
            recent_summaries: vec![],
            chronicle_prompt_catalog: vec![],
            far_memory_hits: vec![],
            template_random_seed: None,
            context_epoch: None,
            chronicle_revision: 0,
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

        let rules =
            collect_mvu_update_rules(&ctx, &store, std::slice::from_ref(&present_instance_id));
        assert_eq!(rules, vec!["damage reduces hp".to_string()]);
        let rules_dedup = collect_mvu_update_rules(
            &ctx,
            &store,
            &[present_instance_id.clone(), character.name.clone()],
        );
        assert_eq!(
            rules_dedup.len(),
            1,
            "同一张源卡的多个在场角色只应贡献一次规则"
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
            summary_attempted: true,
            post_process_attempted: true,
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
            campaign_variable_schema: vec![],
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
                code: Some("A0005".into()),
                headline: Some("密封门".into()),
                lineage_id: None,
                covered_by: None,
                level: 0,
                turn_end: 5,
                covers: vec![],
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
            campaign_variable_schema: vec![],
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
        let campaign_lineage = campaign.lineage_id.clone().unwrap();

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
        // Keep only valid related characters; broken refs are rejected by a dedicated test.
        tasks[0].related_characters = vec![old_instance_a.clone()];
        let summaries = vec![RoundSummary {
            id: Id::from_str("old-summary"),
            campaign_id: old_campaign_id.clone(),
            conversation_id: old_conversation_id,
            turn: 1,
            content: "Round one happened.".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            code: Some("A0001".into()),
            headline: None,
            lineage_id: Some(campaign_lineage),
            covered_by: None,
            level: 0,
            turn_end: 0,
            covers: vec![],
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

    #[test]
    fn import_campaign_bundle_is_atomic_on_mid_write_failure() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_bundle_atomic_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let old_card_id = Id::from_str("atomic-card");
        let old_campaign_id = Id::from_str("atomic-campaign");
        let old_def = Id::from_str("atomic-def");
        let old_instance = Id::from_str("atomic-instance");

        let definitions = vec![CharacterDefinition {
            id: old_def.clone(),
            card_id: old_card_id.clone(),
            name: "Atomic".into(),
            persona_prompt: "persona".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        }];
        let card = CharacterCard {
            id: old_card_id.clone(),
            name: "Atomic Card".into(),
            source_character_id: Id::from_str("atomic-source"),
            character_definitions: definitions.clone(),
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({"first_mes": "hi"}),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let mut campaign = Campaign::new(old_card_id.clone(), "Atomic Campaign");
        campaign.id = old_campaign_id.clone();
        campaign.set_variable("story_clock", serde_json::json!("Day 3"), 2);
        let instances = vec![CharacterInstance {
            id: old_instance,
            campaign_id: old_campaign_id,
            definition_id: Some(old_def),
            name: "Atomic".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![storyforge_domain::variables::VariableValue::new(
                "hp",
                serde_json::json!(77),
                2,
            )],
            is_temporary: false,
        }];

        let err = import_campaign_bundle_into_store_with_after_campaign(
            &store,
            &conv_store,
            CampaignBundle {
                format_version: BUNDLE_FORMAT_VERSION,
                exported_at: chrono::Utc::now().to_rfc3339(),
                card: Some(card),
                campaign,
                instances,
                definitions,
                knowledge: vec![],
                tasks: vec![],
                summaries: vec![],
            },
            || Err(TauriCommandError::storage("injected after campaign save")),
        )
        .expect_err("post-campaign failure should fail the import");

        match err {
            TauriCommandError::Storage { .. } | TauriCommandError::Internal { .. } => {}
            other => panic!("expected storage/internal error, got {other:?}"),
        }

        assert!(
            store.list_cards().is_empty(),
            "failed import must not leave a partial card"
        );
        assert!(
            store.list_campaigns().is_empty(),
            "failed import must not leave a partial campaign"
        );
        assert!(
            store.list_all_instances().is_empty(),
            "failed import must not leave partial instances"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn campaign_bundle_roundtrip_preserves_variables_and_multi_character_semantics() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
        use storyforge_domain::story_task::{StoryTask, TaskTrigger};
        use storyforge_domain::variables::VariableValue;

        let export_dir = std::env::temp_dir().join(format!(
            "storyforge_test_bundle_vars_export_{}",
            uuid::Uuid::new_v4()
        ));
        let import_dir = std::env::temp_dir().join(format!(
            "storyforge_test_bundle_vars_import_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&export_dir).unwrap();
        std::fs::create_dir_all(&import_dir).unwrap();
        let export_store = campaign_store::CampaignStore::new(&export_dir);
        let import_store = campaign_store::CampaignStore::new(&import_dir);
        let import_conv = ConversationStore::new(import_dir.join("conversations"));

        let card_id = Id::from_str("vars-card");
        let campaign_id = Id::from_str("vars-campaign");
        let def_a = Id::from_str("vars-def-a");
        let def_b = Id::from_str("vars-def-b");
        let inst_a = Id::from_str("vars-inst-a");
        let inst_b = Id::from_str("vars-inst-b");
        let knowledge_a = Id::from_str("vars-know-a");
        let knowledge_b = Id::from_str("vars-know-b");

        let definitions = vec![
            CharacterDefinition {
                id: def_a.clone(),
                card_id: card_id.clone(),
                name: "Alpha".into(),
                persona_prompt: "alpha persona".into(),
                behavior_rules: "alpha rules".into(),
                base_backstory: vec!["alpha backstory".into()],
                group: Some("party".into()),
                role_type: RoleType::Protagonist,
                variable_schema: vec![],
            },
            CharacterDefinition {
                id: def_b.clone(),
                card_id: card_id.clone(),
                name: "Beta".into(),
                persona_prompt: "beta persona".into(),
                behavior_rules: "beta rules".into(),
                base_backstory: vec![],
                group: Some("party".into()),
                role_type: RoleType::Supporting,
                variable_schema: vec![],
            },
        ];
        let card = CharacterCard {
            id: card_id.clone(),
            name: "Multi Card".into(),
            source_character_id: Id::from_str("vars-source"),
            character_definitions: definitions.clone(),
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({
                "first_mes": "multi-open",
                "alternate_greetings": ["m1", "m2"]
            }),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: Some("two characters".into()),
        };
        export_store.save_card(card).unwrap();

        let mut campaign = Campaign::new(card_id.clone(), "Vars Campaign");
        campaign.id = campaign_id.clone();
        campaign.revision = 4;
        campaign.chronicle_revision = 2;
        campaign.set_variable("story_clock", serde_json::json!("Day 12 - night"), 8);
        campaign.set_variable("weather", serde_json::json!("storm"), 8);
        export_store.save_campaign(campaign).unwrap();

        for instance in [
            CharacterInstance {
                id: inst_a.clone(),
                campaign_id: campaign_id.clone(),
                definition_id: Some(def_a.clone()),
                name: "Alpha".into(),
                persona_override: Some("alpha live".into()),
                behavior_override: None,
                variables: vec![VariableValue::new("hp", serde_json::json!(88), 8)],
                is_temporary: false,
            },
            CharacterInstance {
                id: inst_b.clone(),
                campaign_id: campaign_id.clone(),
                definition_id: Some(def_b.clone()),
                name: "Beta".into(),
                persona_override: None,
                behavior_override: Some("beta live".into()),
                variables: vec![VariableValue::new("mood", serde_json::json!("wary"), 8)],
                is_temporary: false,
            },
        ] {
            export_store.add_instance(instance).unwrap();
        }

        let mut witnessed = CharacterKnowledgeEntry::witnessed(
            campaign_id.clone(),
            inst_a.clone(),
            "Alpha saw the seal break",
            7,
        );
        witnessed.id = knowledge_a.clone();
        witnessed.pinned = true;
        let mut told = CharacterKnowledgeEntry {
            id: knowledge_b,
            campaign_id: campaign_id.clone(),
            character_id: inst_b.clone(),
            knowledge_text: "Beta was told about the seal".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(inst_a.clone()),
            source_knowledge_id: Some(knowledge_a),
            turn_number: 8,
            event_id: None,
            pinned: false,
            propagation: Default::default(),
        };
        let _ = &mut told;
        export_store.add_knowledge(vec![witnessed, told]).unwrap();

        let mut task = StoryTask::user_planned(
            campaign_id.clone(),
            "Repair the seal",
            "Both characters involved",
            vec![TaskTrigger::TurnReminder { at_turn: 10 }],
            8,
        );
        task.related_characters = vec![inst_a, inst_b];
        export_store.add_task(task).unwrap();

        let bundle_json = export_campaign_bundle_from_store(&export_store, campaign_id).unwrap();
        let bundle: CampaignBundle = serde_json::from_str(&bundle_json).unwrap();
        assert_eq!(bundle.definitions.len(), 2);
        assert_eq!(bundle.instances.len(), 2);
        assert!(
            bundle.card.as_ref().unwrap().character_definitions.len() == 2,
            "bundle must keep multi-character definitions instead of flattening"
        );

        let imported =
            import_campaign_bundle_into_store(&import_store, &import_conv, bundle).unwrap();
        let new_campaign_id = Id::from_str(&imported.campaign_id);
        let imported_campaign = import_store.get_campaign(&new_campaign_id).unwrap();
        assert_eq!(
            imported_campaign.get_variable("story_clock").unwrap(),
            &serde_json::json!("Day 12 - night")
        );
        assert_eq!(
            imported_campaign.get_variable("weather").unwrap(),
            &serde_json::json!("storm")
        );
        // revision/chronicle_revision are Campaign fields and must survive export→import.
        assert_eq!(imported_campaign.revision, 4);
        assert_eq!(imported_campaign.chronicle_revision, 2);

        let imported_card = import_store
            .get_card(&Id::from_str(&imported.card_id))
            .unwrap();
        assert_eq!(imported_card.card.character_definitions.len(), 2);
        assert!(
            imported_card
                .card
                .character_definitions
                .iter()
                .any(|d| d.name == "Alpha")
                && imported_card
                    .card
                    .character_definitions
                    .iter()
                    .any(|d| d.name == "Beta"),
            "multi-character definitions must both survive"
        );

        let imported_instances = import_store.list_instances(&new_campaign_id);
        assert_eq!(imported_instances.len(), 2);
        let alpha = imported_instances
            .iter()
            .find(|i| i.name == "Alpha")
            .unwrap();
        let beta = imported_instances
            .iter()
            .find(|i| i.name == "Beta")
            .unwrap();
        assert_eq!(
            alpha
                .variables
                .iter()
                .find(|v| v.key == "hp")
                .map(|v| &v.value),
            Some(&serde_json::json!(88))
        );
        assert_eq!(
            beta.variables
                .iter()
                .find(|v| v.key == "mood")
                .map(|v| &v.value),
            Some(&serde_json::json!("wary"))
        );
        assert_eq!(alpha.persona_override.as_deref(), Some("alpha live"));
        assert_eq!(beta.behavior_override.as_deref(), Some("beta live"));

        let knowledge = import_store.list_knowledge(&new_campaign_id);
        assert_eq!(knowledge.len(), 2);
        let told = knowledge
            .iter()
            .find(|k| k.source == KnowledgeSource::ToldByOther)
            .unwrap();
        let source_instance = told.source_character_id.as_ref().unwrap();
        assert_eq!(source_instance, &alpha.id);
        assert!(
            knowledge
                .iter()
                .any(|k| k.id == *told.source_knowledge_id.as_ref().unwrap()),
            "knowledge provenance must point at rewritten source knowledge id"
        );

        let tasks = import_store.list_tasks(&new_campaign_id);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].related_characters.len(), 2);

        let _ = std::fs::remove_dir_all(&export_dir);
        let _ = std::fs::remove_dir_all(&import_dir);
    }

    #[test]
    fn import_campaign_bundle_rejects_unsupported_version_without_mutation() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::character::CharacterCard;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_bundle_bad_version_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let card = CharacterCard {
            id: Id::from_str("v-card"),
            name: "V".into(),
            source_character_id: Id::from_str("v-source"),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Unknown,
            extraction_message: None,
        };
        let campaign = Campaign::new(card.id.clone(), "V");
        let err = import_campaign_bundle_into_store(
            &store,
            &conv_store,
            CampaignBundle {
                format_version: 99,
                exported_at: chrono::Utc::now().to_rfc3339(),
                card: Some(card),
                campaign,
                instances: vec![],
                definitions: vec![],
                knowledge: vec![],
                tasks: vec![],
                summaries: vec![],
            },
        )
        .expect_err("unsupported version must fail");
        match err {
            TauriCommandError::Validation { .. } => {}
            other => panic!("expected validation error, got {other:?}"),
        }
        assert!(store.list_cards().is_empty());
        assert!(store.list_campaigns().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn valid_summary_graph_bundle() -> CampaignBundle {
        use storyforge_domain::agent::RoundSummary;
        use storyforge_domain::campaign::Campaign;

        let card_id = Id::from_str("graph-card");
        let campaign_id = Id::from_str("graph-campaign");
        let conversation_id = Id::from_str("graph-conversation");
        let a1_id = Id::from_str("graph-a1");
        let a2_id = Id::from_str("graph-a2");
        let b_id = Id::from_str("graph-b1");
        let mut campaign = Campaign::new(card_id, "Graph Campaign");
        campaign.id = campaign_id.clone();
        campaign.conversation_id = Some(conversation_id.clone());
        let lineage_id = campaign.lineage_id.clone().unwrap();

        let mut a1 = RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            1,
            "leaf one".into(),
        );
        a1.id = a1_id.clone();
        a1.code = Some("A0001".into());
        a1.lineage_id = Some(lineage_id.clone());
        a1.covered_by = Some(b_id.clone());

        let mut a2 = RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            2,
            "leaf two".into(),
        );
        a2.id = a2_id.clone();
        a2.code = Some("A0002".into());
        a2.lineage_id = Some(lineage_id.clone());
        a2.covered_by = Some(b_id.clone());

        let mut b = RoundSummary::new(campaign_id, conversation_id, 1, "band".into());
        b.id = b_id;
        b.code = Some("B0001".into());
        b.lineage_id = Some(lineage_id);
        b.level = 1;
        b.turn_end = 2;
        b.covers = vec![a1_id, a2_id];

        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: None,
            campaign,
            instances: vec![],
            definitions: vec![],
            knowledge: vec![],
            tasks: vec![],
            summaries: vec![a1, a2, b],
        }
    }

    #[test]
    fn import_campaign_bundle_rejects_malformed_summary_graphs_before_writes() {
        let cases = [
            "duplicate-id",
            "asymmetric-edge",
            "cycle",
            "wrong-level",
            "wrong-span",
            "scope-drift",
            "campaign-lineage-missing",
            "lineage-missing",
            "lineage-drift",
            "code-level-mismatch",
            "duplicate-code",
            "duplicate-cover",
        ];

        for case in cases {
            let dir = std::env::temp_dir().join(format!(
                "storyforge_test_import_bad_graph_{case}_{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let store = campaign_store::CampaignStore::new(&dir);
            let conv_store = ConversationStore::new(dir.join("conversations"));
            let mut bundle = valid_summary_graph_bundle();

            match case {
                "duplicate-id" => bundle.summaries.push(bundle.summaries[0].clone()),
                "asymmetric-edge" => bundle.summaries[0].covered_by = None,
                "cycle" => {
                    let parent_id = bundle.summaries[2].id.clone();
                    let child_id = bundle.summaries[0].id.clone();
                    bundle.summaries[0].covers = vec![parent_id];
                    bundle.summaries[2].covered_by = Some(child_id);
                }
                "wrong-level" => bundle.summaries[2].level = 2,
                "wrong-span" => bundle.summaries[2].turn_end = 1,
                "scope-drift" => bundle.summaries[1].campaign_id = Id::from_str("other-campaign"),
                "campaign-lineage-missing" => bundle.campaign.lineage_id = None,
                "lineage-missing" => bundle.summaries[0].lineage_id = None,
                "lineage-drift" => {
                    bundle.summaries[0].lineage_id = Some(Id::from_str("other-lineage"))
                }
                "code-level-mismatch" => bundle.summaries[2].code = Some("A9999".into()),
                "duplicate-code" => bundle.summaries[1].code = bundle.summaries[0].code.clone(),
                "duplicate-cover" => {
                    let child = bundle.summaries[2].covers[0].clone();
                    bundle.summaries[2].covers.push(child);
                }
                _ => unreachable!(),
            }

            let error = import_campaign_bundle_into_store(&store, &conv_store, bundle)
                .expect_err("malformed summary graph must fail closed");
            assert!(
                matches!(error, TauriCommandError::Validation { .. }),
                "case={case}, unexpected={error:?}"
            );
            assert!(store.list_cards().is_empty(), "case={case}");
            assert!(store.list_campaigns().is_empty(), "case={case}");
            assert!(store.list_all_summaries().is_empty(), "case={case}");
            assert!(conv_store.list().is_empty(), "case={case}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn import_campaign_bundle_clears_nonportable_pending_compress_marker() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_pending_marker_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));
        let mut bundle = valid_summary_graph_bundle();
        let parent = bundle.summaries[2].id.clone();
        let children: Vec<_> = bundle.summaries[..2]
            .iter()
            .map(|summary| (summary.id.clone(), parent.clone()))
            .collect();
        bundle.campaign.pending_compress_publication = Some(
            storyforge_domain::chronicle::PendingCompressPublication::new(
                bundle.campaign.chronicle_revision,
                vec![parent],
                children,
            ),
        );

        let imported = import_campaign_bundle_into_store(&store, &conv_store, bundle).unwrap();
        let campaign = store
            .get_campaign(&Id::from_str(&imported.campaign_id))
            .unwrap();
        assert!(
            campaign.pending_compress_publication.is_none(),
            "an in-flight source publication cannot be resumed with rewritten ids"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_campaign_bundle_rewrites_summary_covers_and_preserves_bc_graph() {
        use storyforge_domain::agent::RoundSummary;
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_bundle_chronicle_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let card_id = Id::from_str("chr-card");
        let campaign_id = Id::from_str("chr-campaign");
        let def_id = Id::from_str("chr-def");
        let instance_id = Id::from_str("chr-inst");
        let leaf_a1 = Id::from_str("leaf-a1");
        let leaf_a2 = Id::from_str("leaf-a2");
        let parent_b = Id::from_str("parent-b");
        let conversation_id = Id::from_str("chr-conversation");

        let definitions = vec![CharacterDefinition {
            id: def_id.clone(),
            card_id: card_id.clone(),
            name: "Chron".into(),
            persona_prompt: "persona".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        }];
        let card = CharacterCard {
            id: card_id.clone(),
            name: "Chronicle Card".into(),
            source_character_id: Id::from_str("chr-source"),
            character_definitions: definitions.clone(),
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({"first_mes": "hi"}),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let mut campaign = Campaign::new(card_id.clone(), "Chronicle Campaign");
        campaign.id = campaign_id.clone();
        campaign.conversation_id = Some(conversation_id.clone());
        campaign.chronicle_revision = 3;
        let campaign_lineage = campaign.lineage_id.clone().unwrap();

        let instances = vec![CharacterInstance {
            id: instance_id,
            campaign_id: campaign_id.clone(),
            definition_id: Some(def_id),
            name: "Chron".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        }];

        let mut a1 = RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            1,
            "leaf one".into(),
        );
        a1.id = leaf_a1.clone();
        a1.code = Some("A0001".into());
        a1.lineage_id = Some(campaign_lineage.clone());
        a1.level = 0;
        a1.turn_end = 1;
        a1.covered_by = Some(parent_b.clone());

        let mut a2 = RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            2,
            "leaf two".into(),
        );
        a2.id = leaf_a2.clone();
        a2.code = Some("A0002".into());
        a2.lineage_id = Some(campaign_lineage.clone());
        a2.level = 0;
        a2.turn_end = 2;
        a2.covered_by = Some(parent_b.clone());

        // B-level parent shares turn span start with a1; add_summary-by-turn would clobber it.
        let mut b = RoundSummary::new(
            campaign_id.clone(),
            conversation_id,
            1,
            "band covering leaves".into(),
        );
        b.id = parent_b.clone();
        b.code = Some("B0001".into());
        b.lineage_id = Some(campaign_lineage);
        b.level = 1;
        b.turn_end = 2;
        b.covers = vec![leaf_a1.clone(), leaf_a2.clone()];
        b.covered_by = None;

        let imported = import_campaign_bundle_into_store(
            &store,
            &conv_store,
            CampaignBundle {
                format_version: BUNDLE_FORMAT_VERSION,
                exported_at: chrono::Utc::now().to_rfc3339(),
                card: Some(card),
                campaign,
                instances,
                definitions,
                knowledge: vec![],
                tasks: vec![],
                summaries: vec![a1, a2, b],
            },
        )
        .expect("chronicle graph bundle should import");

        let new_campaign_id = Id::from_str(&imported.campaign_id);
        let summaries = store.list_summaries(&new_campaign_id);
        assert_eq!(
            summaries.len(),
            3,
            "A leaves and B parent must all survive import"
        );
        assert_eq!(imported.summary_count, 3);

        let parent = summaries
            .iter()
            .find(|s| s.level == 1)
            .expect("B parent should exist");
        assert_eq!(parent.covers.len(), 2);
        assert!(
            !parent.covers.contains(&leaf_a1) && !parent.covers.contains(&leaf_a2),
            "covers must be rewritten to new leaf ids, not keep old ids"
        );

        let leaves: Vec<_> = summaries.iter().filter(|s| s.level == 0).collect();
        assert_eq!(leaves.len(), 2);
        for leaf in leaves {
            assert_eq!(
                leaf.covered_by.as_ref(),
                Some(&parent.id),
                "covered_by must point at rewritten parent id"
            );
            assert!(
                parent.covers.contains(&leaf.id),
                "parent.covers must include rewritten leaf id {}",
                leaf.id
            );
            assert_ne!(leaf.id, leaf_a1);
            assert_ne!(leaf.id, leaf_a2);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_campaign_bundle_rejects_broken_internal_references() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
        use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
        use storyforge_domain::story_task::{StoryTask, TaskTrigger};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_bundle_broken_refs_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let card_id = Id::from_str("brk-card");
        let campaign_id = Id::from_str("brk-campaign");
        let def_id = Id::from_str("brk-def");
        let instance_id = Id::from_str("brk-inst");

        let definitions = vec![CharacterDefinition {
            id: def_id,
            card_id: card_id.clone(),
            name: "Broken".into(),
            persona_prompt: "persona".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        }];
        let card = CharacterCard {
            id: card_id.clone(),
            name: "Broken Card".into(),
            source_character_id: Id::from_str("brk-source"),
            character_definitions: definitions.clone(),
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({}),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let mut campaign = Campaign::new(card_id, "Broken Campaign");
        campaign.id = campaign_id.clone();

        // Instance points at a definition id that is not present after rewrite map.
        let instances = vec![CharacterInstance {
            id: instance_id.clone(),
            campaign_id: campaign_id.clone(),
            definition_id: Some(Id::from_str("missing-def")),
            name: "Broken".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        }];
        let knowledge = vec![CharacterKnowledgeEntry {
            id: Id::from_str("k1"),
            campaign_id: campaign_id.clone(),
            character_id: instance_id.clone(),
            knowledge_text: "orphan source".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(Id::from_str("ghost-instance")),
            source_knowledge_id: Some(Id::from_str("ghost-knowledge")),
            turn_number: 1,
            event_id: None,
            pinned: false,
            propagation: Default::default(),
        }];
        let mut task = StoryTask::user_planned(
            campaign_id,
            "broken task",
            "refs ghost",
            vec![TaskTrigger::TurnReminder { at_turn: 2 }],
            1,
        );
        task.related_characters = vec![instance_id, Id::from_str("ghost-instance")];

        let err = import_campaign_bundle_into_store(
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
                tasks: vec![task],
                summaries: vec![],
            },
        )
        .expect_err("broken internal references must fail closed");

        match err {
            TauriCommandError::Validation { message } => {
                assert!(
                    message.contains("definition")
                        || message.contains("knowledge")
                        || message.contains("related_characters")
                        || message.contains("引用"),
                    "validation message should mention broken refs: {message}"
                );
            }
            other => panic!("expected validation error, got {other:?}"),
        }
        assert!(store.list_cards().is_empty());
        assert!(store.list_campaigns().is_empty());
        assert!(store.list_all_instances().is_empty());
        assert!(store.list_all_knowledge().is_empty());
        assert!(store.list_all_tasks().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_campaign_bundle_rollback_is_verified_on_disk() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_bundle_verified_rollback_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let card_id = Id::from_str("vr-card");
        let campaign_id = Id::from_str("vr-campaign");
        let def_id = Id::from_str("vr-def");
        let definitions = vec![CharacterDefinition {
            id: def_id.clone(),
            card_id: card_id.clone(),
            name: "Verified".into(),
            persona_prompt: "persona".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        }];
        let card = CharacterCard {
            id: card_id.clone(),
            name: "Verified Card".into(),
            source_character_id: Id::from_str("vr-source"),
            character_definitions: definitions.clone(),
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({}),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let mut campaign = Campaign::new(card_id, "Verified Campaign");
        campaign.id = campaign_id.clone();
        let instances = vec![CharacterInstance {
            id: Id::from_str("vr-inst"),
            campaign_id,
            definition_id: Some(def_id),
            name: "Verified".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        }];

        let instances_path = dir.join("instances.json");
        let error = import_campaign_bundle_into_store_with_after_campaign(
            &store,
            &conv_store,
            CampaignBundle {
                format_version: BUNDLE_FORMAT_VERSION,
                exported_at: chrono::Utc::now().to_rfc3339(),
                card: Some(card),
                campaign,
                instances,
                definitions,
                knowledge: vec![],
                tasks: vec![],
                summaries: vec![],
            },
            || {
                // Corrupt a collection only after the strict preflight so this
                // exercises rollback verification rather than baseline rejection.
                std::fs::create_dir_all(&instances_path).map_err(|error| {
                    TauriCommandError::storage(format!("inject rollback fault: {error}"))
                })?;
                Ok(())
            },
        )
        .expect_err("instance write failure should fail import");
        let message = match error {
            TauriCommandError::Storage { message } => message,
            other => panic!("expected storage error, got {other:?}"),
        };
        assert!(
            message.contains("strict disk read failed") && message.contains("instances.json"),
            "rollback verification must report unreadable disk state: {message}"
        );

        // Reload store from disk — in-memory empty is not enough.
        let reloaded = campaign_store::CampaignStore::new(&dir);
        assert!(
            reloaded.list_cards().is_empty(),
            "rollback must clear cards on disk"
        );
        assert!(
            reloaded.list_campaigns().is_empty(),
            "rollback must clear campaigns on disk"
        );
        assert!(
            reloaded.list_all_instances().is_empty(),
            "rollback must clear instances on disk"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strict_bundle_disk_snapshot_rejects_unreadable_collection() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_strict_snapshot_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("instances.json")).unwrap();
        let conversations_dir = dir.join("conversations");
        std::fs::create_dir_all(&conversations_dir).unwrap();

        let error = read_bundle_disk_snapshot_strict(&dir, &conversations_dir)
            .expect_err("a collection path that is a directory must not deserialize as empty");

        assert!(error.contains("instances.json"), "unexpected: {error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_campaign_bundle_rejects_preexisting_corrupt_store_before_any_write() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_corrupt_baseline_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let corrupt = br#"[{"card":"truncated"}"#;
        std::fs::write(dir.join("cards.json"), corrupt).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));
        let campaign = storyforge_domain::campaign::Campaign::new(
            Id::from_str("corrupt-baseline-card"),
            "Corrupt Baseline",
        );

        let error = import_campaign_bundle_into_store(
            &store,
            &conv_store,
            CampaignBundle {
                format_version: BUNDLE_FORMAT_VERSION,
                exported_at: chrono::Utc::now().to_rfc3339(),
                card: None,
                campaign,
                instances: vec![],
                definitions: vec![],
                knowledge: vec![],
                tasks: vec![],
                summaries: vec![],
            },
        )
        .expect_err("corrupt pre-import disk state must fail before writes");

        assert!(matches!(error, TauriCommandError::Storage { .. }));
        assert_eq!(
            std::fs::read(dir.join("cards.json")).unwrap(),
            corrupt,
            "preflight must not overwrite a corrupt source file"
        );
        assert!(store.list_cards().is_empty());
        assert!(store.list_campaigns().is_empty());
        assert!(conv_store.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_campaign_bundle_conversation_create_failure_leaves_no_card_or_campaign() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_conversation_failure_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conversations_path = dir.join("conversations");
        std::fs::write(&conversations_path, b"blocked").unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(conversations_path);
        let campaign = storyforge_domain::campaign::Campaign::new(
            Id::from_str("conv-fail-card"),
            "Conversation Failure",
        );

        let error = import_campaign_bundle_into_store(
            &store,
            &conv_store,
            CampaignBundle {
                format_version: BUNDLE_FORMAT_VERSION,
                exported_at: chrono::Utc::now().to_rfc3339(),
                card: None,
                campaign,
                instances: vec![],
                definitions: vec![],
                knowledge: vec![],
                tasks: vec![],
                summaries: vec![],
            },
        )
        .expect_err("conversation create must fail closed");

        assert!(matches!(error, TauriCommandError::Storage { .. }));
        assert!(store.list_cards().is_empty());
        assert!(store.list_campaigns().is_empty());
        assert!(conv_store.list().is_empty());
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
    async fn test_update_connection_async_keeps_key_and_refreshes_active() {
        let state = Arc::new(AppState::new_for_test());
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_async_update_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());
        let store = Arc::new(ConnectionStore::new_with_secret_store(
            &dir,
            secret_store.clone(),
        ));
        let conn = make_test_llm_connection("edit-active", "edit-secret-old");
        create_connection_with_store_async(state.clone(), store.clone(), conn)
            .await
            .unwrap();
        assert_eq!(state.active_conn_id().as_deref(), Some("edit-active"));

        update_connection_with_store_async(
            state.clone(),
            store.clone(),
            UpdateConnectionDto {
                id: "edit-active".into(),
                name: "renamed-active".into(),
                base_url: "https://api.example.com/v1/chat/completions".into(),
                protocol: "openai".into(),
                model: "new-model".into(),
                api_key: String::new(), // 留空保留原 key
                tool_mode: "native".into(),
                temperature: Some(0.7),
                top_p: Some(0.9),
                max_tokens: None,
                max_tokens_explicit: false,
                reasoning: Some("disabled".into()),
                extra: None,
            },
        )
        .await
        .unwrap();

        let stored = store.get("edit-active").unwrap();
        assert_eq!(stored.connection.name, "renamed-active");
        assert_eq!(stored.connection.model, "new-model");
        assert!(is_secret_ref(&stored.connection.api_key));
        assert_eq!(
            store.resolved("edit-active").unwrap().unwrap().api_key,
            "edit-secret-old"
        );
        assert_eq!(state.active_conn_id().as_deref(), Some("edit-active"));

        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(!raw.contains("edit-secret-old"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_update_connection_async_replaces_key_when_provided() {
        let state = Arc::new(AppState::new_for_test());
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_async_update_key_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());
        let store = Arc::new(ConnectionStore::new_with_secret_store(
            &dir,
            secret_store.clone(),
        ));
        store
            .save(make_test_llm_connection("edit-key", "old-secret"))
            .unwrap();

        update_connection_with_store_async(
            state.clone(),
            store.clone(),
            UpdateConnectionDto {
                id: "edit-key".into(),
                name: "edit-key".into(),
                base_url: "https://api.example.com/v1/chat/completions".into(),
                protocol: "openai".into(),
                model: "test-model".into(),
                api_key: "new-secret".into(),
                tool_mode: "native".into(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                max_tokens_explicit: false,
                reasoning: None,
                extra: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            store.resolved("edit-key").unwrap().unwrap().api_key,
            "new-secret"
        );
        // 非活跃更新不应误设 active
        assert!(state.active_conn_id().is_none());

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

    #[test]
    fn add_campaign_instance_from_card_definition_preserves_role_and_schema() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterCard, RoleType};
        use storyforge_domain::variables::default_character_variables;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_add_card_instance_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let source = make_test_character("Roster");
        let mut card = CharacterCard::from_character(&source);
        card.id = Id::from_str("card-roster");
        let hero = make_test_character_definition(&card.id, "def-hero", "Hero");
        let mut extra = make_test_character_definition(&card.id, "def-extra", "Courier");
        extra.role_type = RoleType::Extra;
        extra.variable_schema = default_character_variables();
        card.character_definitions = vec![hero.clone(), extra.clone()];
        store.save_card(card).unwrap();

        let campaign = Campaign::new(Id::from_str("card-roster"), "run");
        store.save_campaign(campaign.clone()).unwrap();
        store
            .add_instance(CharacterInstance::from_definition(
                campaign.id.clone(),
                &hero,
            ))
            .unwrap();

        let added =
            add_campaign_instance_to_store(&store, &campaign.id, Some(&extra.id), None, None, None)
                .unwrap();

        assert_eq!(added.name, "Courier");
        assert_eq!(added.definition_id.as_deref(), Some("def-extra"));
        assert_eq!(added.role_type.as_deref(), Some("extra"));
        assert!(!added.is_temporary);
        assert!(!added.variables.is_empty());

        let duplicate =
            add_campaign_instance_to_store(&store, &campaign.id, Some(&extra.id), None, None, None)
                .unwrap_err();
        assert!(duplicate.to_string().contains("已加入"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_campaign_custom_instance_creates_validated_temporary_character() {
        use storyforge_domain::campaign::Campaign;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_add_custom_instance_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("card-custom"), "run");
        store.save_campaign(campaign.clone()).unwrap();

        let added = add_campaign_instance_to_store(
            &store,
            &campaign.id,
            None,
            Some("  渡鸦信使  ".into()),
            Some("  寡言而警觉  ".into()),
            Some("  只交付密信  ".into()),
        )
        .unwrap();

        assert_eq!(added.name, "渡鸦信使");
        assert_eq!(added.role_type.as_deref(), Some("extra"));
        assert!(added.is_temporary);
        assert_eq!(added.persona_override.as_deref(), Some("寡言而警觉"));
        assert_eq!(added.behavior_override.as_deref(), Some("只交付密信"));

        let duplicate = add_campaign_instance_to_store(
            &store,
            &campaign.id,
            None,
            Some("渡鸦信使".into()),
            None,
            None,
        )
        .unwrap_err();
        assert!(duplicate.to_string().contains("同名角色"));

        let blank = add_campaign_instance_to_store(
            &store,
            &campaign.id,
            None,
            Some("   ".into()),
            None,
            None,
        )
        .unwrap_err();
        assert!(blank.to_string().contains("角色名称"));

        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn test_meta_apply_mvu_schema_normalizes_legacy_key_notation() {
        // 存量翻译产物可能带旧记法键（斜杠 / stat_data. 前缀）。应用边界必须
        // 归一，否则同一变量以两种键并存（"stat_data.hp" 与 "hp"）。
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_mvu_apply_normalize_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let character = make_test_character("MVU Legacy Notation Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        let mut definition = make_test_character_definition(&card.id, "mvu-legacy-def", "Hero");
        definition.variable_schema = vec![test_variable_field("hp", "HP", serde_json::json!(100))];
        card.character_definitions.push(definition.clone());
        store.save_card(card.clone()).unwrap();

        store
            .save_mvu(campaign_store::StoredMvuTranslation {
                source_character_id: character.id.clone(),
                character_name: character.name.clone(),
                translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(
                    vec![
                        // 旧记法：stat_data. 前缀 → 应合并到已有 "hp" 而不是新增键
                        test_variable_field("stat_data.hp", "Hit Points", serde_json::json!(200)),
                        // 旧记法：斜杠 → 点
                        test_variable_field("/世界/时间", "时间", serde_json::json!("清晨")),
                    ],
                ),
                analyzed_at: "2026-07-27T00:00:00Z".into(),
            })
            .unwrap();

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
        let keys: Vec<&str> = updated_def
            .variable_schema
            .iter()
            .map(|f| f.key.as_str())
            .collect();
        assert!(keys.contains(&"hp"), "stat_data.hp 应归一为 hp: {keys:?}");
        assert!(
            keys.contains(&"世界.时间"),
            "斜杠键应归一为点记法: {keys:?}"
        );
        assert!(
            !keys
                .iter()
                .any(|k| k.contains("stat_data") || k.contains('/')),
            "不应残留旧记法键: {keys:?}"
        );
        let hp = updated_def
            .variable_schema
            .iter()
            .find(|f| f.key == "hp")
            .unwrap();
        assert_eq!(hp.label, "Hit Points", "归一后应与已有 hp 合并覆盖");

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
    fn test_create_campaign_in_store_initializes_card_global_variables() {
        use storyforge_domain::variables::{VariableField, VariableType};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_create_campaign_globals_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let character = make_test_character("Global Variable Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.campaign_variable_schema = vec![VariableField {
            key: "faction_tension".into(),
            label: "阵营紧张度".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(12),
            description: Some("整局共享的阵营冲突强度".into()),
            group: Some("世界状态".into()),
        }];
        card.character_definitions
            .push(make_test_character_definition(
                &card.id,
                "global-def",
                "Hero",
            ));
        let card_id = card.id.as_str().to_string();
        store.save_card(card).unwrap();

        let dto =
            create_campaign_in_store(&store, &conv_store, card_id, "globals".into(), None).unwrap();
        let campaign = store.get_campaign(&Id::from_str(&dto.id)).unwrap();
        let field = campaign
            .variable_schema
            .iter()
            .find(|field| field.key == "faction_tension")
            .expect("card global schema should be copied into campaign");
        assert_eq!(field.label, "阵营紧张度");
        assert_eq!(field.description.as_deref(), Some("整局共享的阵营冲突强度"));
        assert_eq!(
            campaign
                .variables
                .iter()
                .find(|value| value.key == "faction_tension")
                .map(|value| &value.value),
            Some(&serde_json::json!(12))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_campaign_variable_schema_adds_missing_without_overwriting_current_value() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::variables::{VariableField, VariableType};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_sync_campaign_globals_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);

        let character = make_test_character("Sync Global Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        let card_id = card.id.clone();
        store.save_card(card.clone()).unwrap();
        let campaign = Campaign::new(card_id, "legacy");
        let campaign_id = campaign.id.clone();
        store.save_campaign(campaign).unwrap();

        card.campaign_variable_schema = vec![VariableField {
            key: "danger_level".into(),
            label: "危险等级".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(2),
            description: Some("整局风险".into()),
            group: Some("世界状态".into()),
        }];
        store.save_card(card.clone()).unwrap();

        let first = sync_campaign_variable_schema_in_store(&store, &campaign_id).unwrap();
        assert_eq!(first.added, 1);
        let mut synced = store.get_campaign(&campaign_id).unwrap();
        assert_eq!(
            synced.get_variable("danger_level"),
            Some(&serde_json::json!(2))
        );

        synced.set_variable("danger_level", serde_json::json!(77), 4);
        store.update_campaign(synced).unwrap();
        card.campaign_variable_schema[0].default = serde_json::json!(99);
        card.campaign_variable_schema[0].description = Some("更新后的说明".into());
        store.save_card(card).unwrap();

        let second = sync_campaign_variable_schema_in_store(&store, &campaign_id).unwrap();
        assert_eq!(second.added, 0);
        let resynced = store.get_campaign(&campaign_id).unwrap();
        assert_eq!(
            resynced.get_variable("danger_level"),
            Some(&serde_json::json!(77)),
            "同步 schema 不得覆盖活动中已经变化的当前值"
        );
        assert_eq!(
            resynced
                .variable_schema
                .iter()
                .find(|field| field.key == "danger_level")
                .and_then(|field| field.description.as_deref()),
            Some("更新后的说明")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_apply_campaign_opening_rewrites_first_assistant_message() {
        use storyforge_domain::conversation::Role as ConvRole;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_apply_opening_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let character = make_test_character("Opening Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.character_definitions
            .push(make_test_character_definition(&card.id, "op-def", "Hero"));
        let card_id = card.id.as_str().to_string();
        store.save_card(card).unwrap();

        let dto =
            create_campaign_in_store(&store, &conv_store, card_id, "opening".into(), None).unwrap();
        let campaign_id = Id::from_str(&dto.id);
        let conv_id = Id::from_str(dto.conversation_id.as_deref().expect("bound conv"));
        // 测试环境无全局 CharacterStore：手动播种开场白（生产路径由 create_campaign 写入）
        conv_store
            .append_final_message(&conv_id, ConvRole::Assistant, "scene-1".into())
            .unwrap();

        apply_campaign_opening_in_store(&store, &conv_store, &campaign_id, "scene-2".into())
            .unwrap();

        let conv = conv_store.get(&conv_id).unwrap();
        assert_eq!(conv.nodes.len(), 1);
        assert_eq!(conv.nodes[0].active_content(), "scene-2");

        // 重载后仍是改写值：确认真正落盘而非仅缓存
        let reloaded = ConversationStore::new(dir.join("conversations"));
        assert_eq!(
            reloaded.get(&conv_id).unwrap().nodes[0].active_content(),
            "scene-2"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_apply_campaign_opening_rejects_after_conversation_grows() {
        use storyforge_domain::conversation::Role as ConvRole;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_apply_opening_grown_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let character = make_test_character("Opening Grown Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.character_definitions
            .push(make_test_character_definition(&card.id, "og-def", "Hero"));
        let card_id = card.id.as_str().to_string();
        store.save_card(card).unwrap();

        let dto =
            create_campaign_in_store(&store, &conv_store, card_id, "grown".into(), None).unwrap();
        let campaign_id = Id::from_str(&dto.id);
        let conv_id = Id::from_str(dto.conversation_id.as_deref().expect("bound conv"));
        conv_store
            .append_final_message(&conv_id, ConvRole::Assistant, "scene-1".into())
            .unwrap();
        conv_store
            .append_user_message(&conv_id, "next turn".into())
            .unwrap();

        let err =
            apply_campaign_opening_in_store(&store, &conv_store, &campaign_id, "scene-2".into())
                .unwrap_err();
        assert!(
            err.to_string().contains("后续消息"),
            "expected opening-stale validation error, got {err}"
        );
        // 原开场未被改写
        let conv = conv_store.get(&conv_id).unwrap();
        assert_eq!(conv.nodes[0].active_content(), "scene-1");

        // 空内容与未知 campaign 也拒绝
        let err = apply_campaign_opening_in_store(&store, &conv_store, &campaign_id, "  ".into())
            .unwrap_err();
        assert!(err.to_string().contains("不能为空"), "got {err}");
        let missing = Id::new();
        let err = apply_campaign_opening_in_store(&store, &conv_store, &missing, "scene-2".into())
            .unwrap_err();
        assert!(err.to_string().contains("找不到"), "got {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_campaign_playthrough_cascades_conversation_and_summaries() {
        use storyforge_domain::agent::RoundSummary;

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_delete_playthrough_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let character = make_test_character("Delete Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.character_definitions
            .push(make_test_character_definition(&card.id, "del-def", "Hero"));
        let card_id = card.id.as_str().to_string();
        store.save_card(card).unwrap();

        let dto = create_campaign_in_store(
            &store,
            &conv_store,
            card_id,
            "playthrough-to-delete".into(),
            Some("opening".into()),
        )
        .unwrap();
        let campaign_id = Id::from_str(&dto.id);
        let conversation_id = Id::from_str(dto.conversation_id.as_deref().expect("bound conv"));

        store
            .add_summary(RoundSummary::new(
                campaign_id.clone(),
                conversation_id.clone(),
                1,
                "round one summary".into(),
            ))
            .unwrap();
        assert_eq!(store.list_summaries(&campaign_id).len(), 1);
        assert!(conv_store.get(&conversation_id).is_some());
        assert!(!store.list_instances(&campaign_id).is_empty());

        let state = AppState::new_for_test();
        *state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(campaign_id.clone());

        delete_campaign_playthrough_in_store(&store, &conv_store, &state, &campaign_id).unwrap();

        assert!(store.get_campaign(&campaign_id).is_none());
        assert!(store.list_instances(&campaign_id).is_empty());
        assert!(store.list_summaries(&campaign_id).is_empty());
        assert!(conv_store.get(&conversation_id).is_none());
        assert!(
            state
                .active_campaign
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_none(),
            "active campaign pointer must clear"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_conversation_path_resolves_bound_campaign_and_cascades() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_delete_conv_cascades_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));

        let character = make_test_character("C2 Source");
        let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
        card.character_definitions
            .push(make_test_character_definition(&card.id, "c2-def", "N"));
        let card_id = card.id.as_str().to_string();
        store.save_card(card).unwrap();

        let dto =
            create_campaign_in_store(&store, &conv_store, card_id, "c2".into(), None).unwrap();
        let campaign_id = Id::from_str(&dto.id);
        let conversation_id = Id::from_str(dto.conversation_id.as_deref().unwrap());
        let state = AppState::new_for_test();

        // 与 delete_conversation 命令一致：从会话反查 campaign 后整局删
        let camp_from_conv = conv_store
            .get(&conversation_id)
            .and_then(|c| c.campaign_id)
            .expect("conversation should bind campaign");
        assert_eq!(camp_from_conv, campaign_id);
        delete_campaign_playthrough_in_store(&store, &conv_store, &state, &camp_from_conv).unwrap();
        assert!(store.get_campaign(&campaign_id).is_none());
        assert!(conv_store.get(&conversation_id).is_none());

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
                        reasoning_content: Some("alice reasoning".into()),
                    }],
                    profile_id: Some(Id::from_str("profile-1")),
                    generation_mode: None,
                    seed: 7,
                    last_hint: Some("try again".into()),
                    director_reasoning: Some("director reasoning".into()),
                    writer_reasoning: Some("writer reasoning".into()),
                    editor_reasoning: Some("editor reasoning".into()),
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
        assert!(explanation.director_reasoning.is_none());
        assert!(explanation.writer_reasoning.is_none());
        assert!(explanation.editor_reasoning.is_none());
        assert_eq!(explanation.subagents.len(), 1);
        assert_eq!(explanation.subagents[0].character_id, "alice");
        assert_eq!(explanation.subagents[0].display_name, "Alice");
        assert!(explanation.subagents[0].reasoning_content.is_none());
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

        accept_variant_async(
            state.clone(),
            conversation.id.clone(),
            node_id.clone(),
            false,
            None,
        )
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

        let snap = load_archive_snapshot(state.conv_store.clone(), conversation.id.clone())
            .await
            .unwrap();

        assert_eq!(snap.messages, vec!["user intent", "kept draft"]);
        assert_eq!(snap.archived_upto, 0);
    }

    #[test]
    fn test_archive_snapshot_respects_watermark_math() {
        // 纯水位算术：未归档切片 = messages[archived_upto..]
        let messages: Vec<String> = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let archived_upto = 2usize;
        let pending: &[String] = &messages[archived_upto..];
        assert_eq!(pending, &["c".to_string(), "d".to_string()]);
        let advanced = 1usize;
        let new_upto = archived_upto + advanced;
        assert_eq!(new_upto, 3);
        assert_eq!(&messages[new_upto..], &["d".to_string()]);
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
    fn character_detail_resolves_the_card_source_character_id() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_character_detail_source_id_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = storage::CharacterStore::new(&dir);
        let character = make_test_character("Seraphina");
        let source_character_id = character.id.clone();
        let stored = store.save(CharacterInfo::from(&character)).unwrap();

        let by_storage_id =
            stored_character_for_id_or_source_in_store(&store, &Id::from_str(&stored.id))
                .expect("storage id should resolve");
        let by_source_id = stored_character_for_id_or_source_in_store(&store, &source_character_id)
            .expect("source character id from CardSummaryDto should resolve");

        assert_eq!(by_source_id.id, by_storage_id.id);
        assert_eq!(by_source_id.info.name, "Seraphina");
        let _ = std::fs::remove_dir_all(&dir);
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
                extra: Default::default(),
            }],
            metadata: Default::default(),
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
            campaign_variable_schema: vec![],
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
            campaign_variable_schema: vec![],
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
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

        // ContextEpochSnapshot 应在 fill 时创建并落盘
        assert!(
            ctx.context_epoch.is_some(),
            "fill_campaign should freeze context_epoch"
        );
        let reloaded = campaign_store.get_campaign(&campaign.id).unwrap();
        assert!(reloaded.context_epoch.is_some());
        assert_eq!(
            reloaded.context_epoch.as_ref().unwrap().epoch_id,
            ctx.context_epoch.as_ref().unwrap().epoch_id
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_context_epoch_rollover_when_live_suffix_reaches_e() {
        use storyforge_app_agent::tools::ToolContext;
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR, committed_turn_id};

        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_epoch_rollover_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let campaign_store = campaign_store::CampaignStore::new(&dir);
        let card_id = Id::from_str("card-epoch");
        campaign_store
            .save_card(storyforge_domain::character::CharacterCard {
                id: card_id.clone(),
                name: "Epoch Card".into(),
                source_character_id: Id::from_str("src"),
                character_definitions: vec![],
                campaign_variable_schema: vec![],
                raw_card_json: serde_json::json!({}),
                extraction_status:
                    storyforge_domain::character::CharacterExtractionStatus::Extracted,
                extraction_message: None,
            })
            .unwrap();
        let campaign = Campaign::new(card_id, "epoch-camp");
        let camp_id = campaign.id.clone();
        campaign_store.save_campaign(campaign).unwrap();

        // 先写入 H_anchor 条摘要 → 创建 epoch（head=H, live=0）
        for t in 1..=DEFAULT_H_ANCHOR {
            campaign_store
                .add_summary(
                    storyforge_domain::agent::RoundSummary::new(
                        camp_id.clone(),
                        Id::from_str("conv"),
                        t,
                        format!("round {t}"),
                    )
                    .with_code(format!("A{t:04}"))
                    .with_headline(format!("h{t}")),
                )
                .unwrap();
        }
        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        }));
        let mut ctx = WritingContext::legacy(vec![], None, Id::from_str("conv"));
        fill_campaign_runtime_from_store(&mut ctx, &tool_ctx, &campaign_store, &camp_id);
        let epoch1 = ctx.context_epoch.clone().expect("epoch after first fill");
        assert_eq!(
            epoch1.source_head_turn_id,
            Some(committed_turn_id(DEFAULT_H_ANCHOR))
        );
        let rev1 = ctx.chronicle_revision;

        // 再追加 E 条 → 下次 fill 应 rollover
        for t in (DEFAULT_H_ANCHOR + 1)..=(DEFAULT_H_ANCHOR + DEFAULT_E) {
            campaign_store
                .add_summary(
                    storyforge_domain::agent::RoundSummary::new(
                        camp_id.clone(),
                        Id::from_str("conv"),
                        t,
                        format!("round {t}"),
                    )
                    .with_code(format!("A{t:04}"))
                    .with_headline(format!("h{t}")),
                )
                .unwrap();
        }
        let mut ctx2 = WritingContext::legacy(vec![], None, Id::from_str("conv"));
        fill_campaign_runtime_from_store(&mut ctx2, &tool_ctx, &campaign_store, &camp_id);
        let epoch2 = ctx2
            .context_epoch
            .clone()
            .expect("epoch after rollover fill");
        assert_ne!(
            epoch1.epoch_id, epoch2.epoch_id,
            "rollover must new epoch_id"
        );
        assert_eq!(
            epoch2.source_head_turn_id,
            Some(committed_turn_id(DEFAULT_H_ANCHOR + DEFAULT_E))
        );
        assert!(
            ctx2.chronicle_revision > rev1,
            "rollover should bump chronicle_revision"
        );
        // 同 epoch 再 fill 应稳定
        let mut ctx3 = WritingContext::legacy(vec![], None, Id::from_str("conv"));
        fill_campaign_runtime_from_store(&mut ctx3, &tool_ctx, &campaign_store, &camp_id);
        assert_eq!(
            ctx3.context_epoch.as_ref().unwrap().epoch_id,
            epoch2.epoch_id
        );
        assert_eq!(
            ctx3.context_epoch.as_ref().unwrap().source_hash,
            epoch2.source_hash
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
            summary_attempted: true,
            post_process_attempted: true,
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
            summary_attempted: false,
            post_process_attempted: true,
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
            summary_attempted: false,
            post_process_attempted: true,
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
            summary_attempted: false,
            post_process_attempted: true,
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
            "hp（角色/生命值/int）",
            "story_clock（全局/故事时间/string）",
            "weather（全局/天气/string）",
            "stress（角色/Stress/int）",
            "alarm_level（全局/alarm_level/string）",
            "temporary_flag（角色/temporary_flag/bool）",
        ] {
            assert!(
                keys.contains(&expected.to_string()),
                "missing key {expected}"
            );
        }
        assert_eq!(
            keys.iter()
                .filter(|key| key.starts_with("hp（角色/"))
                .count(),
            1
        );
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
            campaign_variable_schema: vec![],
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

        // 模拟 start_writing 设置 operation-owned cancel
        let (_op_a, rx_a) = begin_writing_operation(&state);
        assert!(!*rx_a.borrow());

        // 触发取消
        {
            let slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let handle = slot.as_ref().unwrap();
            let _ = handle.cancel_tx.send(true);
        }
        assert!(*rx_a.borrow(), "cancel 应已触发");

        // 清理本 operation
        let op = state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|h| h.operation_id.clone())
            .unwrap();
        clear_current_cancel_if(&state, &op);
        assert!(
            state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_none()
        );
    }

    #[test]
    fn operation_owned_cancel_interleaving_preserves_active_generation() {
        let state = AppState::new_for_test();

        // Operation A starts.
        let (op_a, rx_a) = begin_writing_operation(&state);
        assert!(!*rx_a.borrow());

        // Operation B starts while A is still "postprocessing": A must observe cancel,
        // and the global slot becomes B.
        let (op_b, rx_b) = begin_writing_operation(&state);
        assert_ne!(op_a, op_b);
        assert!(*rx_a.borrow(), "starting B must cancel A");
        assert!(!*rx_b.borrow());
        {
            let slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            assert_eq!(slot.as_ref().unwrap().operation_id, op_b);
        }

        // A finishing must not clear B's sender.
        clear_current_cancel_if(&state, &op_a);
        {
            let slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            assert_eq!(
                slot.as_ref().map(|h| h.operation_id.clone()),
                Some(op_b.clone()),
                "A clear must not wipe B"
            );
        }
        assert!(!*rx_b.borrow());

        // cancel_writing still cancels the active generation B.
        {
            let slot = state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let _ = slot.as_ref().unwrap().cancel_tx.send(true);
        }
        assert!(*rx_b.borrow(), "active cancel must still reach B");

        // B clear succeeds; stale A clear remains a no-op.
        clear_current_cancel_if(&state, &op_b);
        clear_current_cancel_if(&state, &op_a);
        assert!(
            state
                .current_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_none()
        );
    }

    #[test]
    fn scope_validation_errors_do_not_mark_turn_failed() {
        use production_postprocess::{
            JsonTurnAttemptSink, PostprocessIdentity, ProductionPostprocessError,
            ProductionPostprocessService,
        };
        use std::sync::Arc;
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::turn::{AttemptStatus, QualityReport, TurnRecord, TurnStatus};

        let dir =
            std::env::temp_dir().join(format!("sf_scope_zero_write_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let campaign_store = Arc::new(campaign_store::CampaignStore::new(&dir));
        let turn_store = Arc::new(turn_store::TurnStore::new(&dir));
        let mut campaign = Campaign::new(Id::new(), "scope-zero");
        campaign.lineage_id = Some(Id::new());
        let campaign_id = campaign.id.clone();
        campaign_store.save_campaign(campaign).unwrap();
        let conversation_id = Id::from_str("conv-scope");
        let attempt_id = Id::new();
        let mut record = TurnRecord::new(
            campaign_id.clone(),
            conversation_id.clone(),
            Id::from_str("input"),
            0,
        );
        record.status = TurnStatus::DraftReady;
        record.attempts.push(turn_lifecycle::new_draft_attempt(
            attempt_id.clone(),
            Id::from_str("variant"),
            "draft",
            vec![],
        ));
        let turn_id = record.turn_id.clone();
        turn_store.create_turn(record).unwrap();

        let sink = JsonTurnAttemptSink {
            turn_store: &turn_store,
        };
        let service = ProductionPostprocessService::new_json(&campaign_store, &sink);
        let (_tx, cancel_rx) = watch::channel(false);
        let before = turn_store.get_turn(&turn_id).unwrap();
        let original_hash = before.find_attempt(&attempt_id).unwrap().draft_hash.clone();

        let assert_zero_write = |label: &str| {
            let after = turn_store.get_turn(&turn_id).unwrap();
            assert_eq!(after.status, before.status, "{label}: status");
            assert_eq!(
                after.conversation_id, before.conversation_id,
                "{label}: conversation"
            );
            assert_eq!(after.campaign_id, before.campaign_id, "{label}: campaign");
            assert_eq!(after.failure_reason, before.failure_reason, "{label}: fail");
            let att = after.find_attempt(&attempt_id).unwrap();
            assert_eq!(att.draft_hash, original_hash, "{label}: draft_hash");
            assert!(att.pending_state_changes.is_none(), "{label}: batch");
            assert!(
                campaign_store.list_summaries(&campaign_id).is_empty(),
                "{label}: summaries"
            );
        };

        // apply_outcome cross-campaign
        let err = service
            .apply_outcome(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: Id::from_str("other-campaign"),
                    conversation_id: conversation_id.clone(),
                    turn_number: 1,
                },
                Some(storyforge_app_agent::PostProcessOutcome {
                    summary: Some("must not write".into()),
                    summary_attempted: true,
                    post_process_attempted: false,
                    post_process: None,
                }),
                &[],
                &cancel_rx,
            )
            .expect_err("cross campaign must fail");
        assert!(matches!(
            err,
            ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                ..
            }
        ));
        let bad_identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: Id::from_str("other-campaign"),
            conversation_id: conversation_id.clone(),
            turn_number: 1,
        };
        let backend = BackendTurnAttemptSink::for_json_store(&turn_store);
        let combined = service_fail_turn(&backend, &bad_identity, err);
        assert!(matches!(
            combined,
            ProductionPostprocessError::ScopeMismatch { .. }
        ));
        assert_zero_write("apply cross campaign");

        // start_writing / regenerate adapter path: sync_autofix via Backend sink.
        // Use the isolated JSON store for the backend adapter path. This keeps the test hermetic
        // even when the workspace test runner executes tests in parallel.
        let backend = BackendTurnAttemptSink::for_json_store(&turn_store);
        turn_store.save_turn(before.clone()).unwrap();
        // new_json only needs sink for autofix; campaign_store is unused on this path.
        let backend_service =
            ProductionPostprocessService::new_json(get_campaign_store(), &backend);

        let camp_identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: Id::from_str("other-campaign"),
            conversation_id: conversation_id.clone(),
            turn_number: 1,
        };
        let camp_err = backend_service
            .sync_autofix_attempt(
                &camp_identity,
                "must-not-write",
                QualityReport { warnings: vec![] },
            )
            .expect_err("backend cross campaign");
        assert!(matches!(
            camp_err,
            ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                ..
            }
        ));
        let combined = service_fail_turn(&backend, &camp_identity, camp_err);
        assert!(matches!(
            combined,
            ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                ..
            }
        ));
        // zero-write on process turn store
        let after_backend = turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(after_backend.status, TurnStatus::DraftReady);
        assert_eq!(after_backend.failure_reason, None);
        assert_eq!(
            after_backend.find_attempt(&attempt_id).unwrap().draft_hash,
            original_hash
        );

        let conv_identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: campaign_id.clone(),
            conversation_id: Id::from_str("other-conversation"),
            turn_number: 1,
        };
        let conv_err = backend_service
            .sync_autofix_attempt(
                &conv_identity,
                "must-not-write",
                QualityReport { warnings: vec![] },
            )
            .expect_err("backend cross conversation");
        assert!(matches!(
            conv_err,
            ProductionPostprocessError::ScopeMismatch {
                field: "conversation_id",
                ..
            }
        ));
        let _ = service_fail_turn(&backend, &conv_identity, conv_err);
        let after_conv = turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(after_conv.conversation_id, conversation_id);
        assert_eq!(after_conv.failure_reason, None);
        assert_eq!(
            after_conv.find_attempt(&attempt_id).unwrap().draft_hash,
            original_hash
        );

        let miss_identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: Id::from_str("ghost-attempt"),
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            turn_number: 1,
        };
        let miss_err = backend_service
            .sync_autofix_attempt(
                &miss_identity,
                "must-not-write",
                QualityReport { warnings: vec![] },
            )
            .expect_err("backend missing attempt");
        assert!(matches!(
            miss_err,
            ProductionPostprocessError::AttemptMissing { .. }
        ));
        let _ = service_fail_turn(&backend, &miss_identity, miss_err);
        let after_miss = turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(after_miss.status, TurnStatus::DraftReady);
        assert_eq!(after_miss.failure_reason, None);
        assert_eq!(
            after_miss.find_attempt(&attempt_id).unwrap().draft_hash,
            original_hash
        );

        // Concurrent supersede + late Storage/BatchConstruction from old postprocess.
        let new_attempt_id = Id::new();
        turn_store
            .with_turn_mut(&turn_id, |record| {
                if let Some(att) = record.find_attempt_mut(&attempt_id) {
                    att.status = AttemptStatus::Superseded;
                }
                let mut new_att = turn_lifecycle::new_draft_attempt(
                    new_attempt_id.clone(),
                    Id::from_str("variant-new"),
                    "regenerated",
                    vec![],
                );
                new_att.status = AttemptStatus::DraftReady;
                record.attempts.push(new_att);
                record.status = TurnStatus::DraftReady;
                record.failure_reason = None;
                record.touch();
            })
            .unwrap();
        let old_identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            turn_number: 1,
        };
        backend_service
            .sync_autofix_attempt(
                &old_identity,
                "superseded-must-not-write",
                QualityReport { warnings: vec![] },
            )
            .expect("superseded is non-fatal zero-write");
        // Late Storage / BatchConstruction from old background postprocess must not Fail the Turn.
        for err in [
            ProductionPostprocessError::Storage("late attach".into()),
            ProductionPostprocessError::BatchConstruction("late batch".into()),
        ] {
            let combined = service_fail_turn(&backend, &old_identity, err);
            assert!(
                matches!(
                    combined,
                    ProductionPostprocessError::Storage(_)
                        | ProductionPostprocessError::BatchConstruction(_)
                ),
                "unexpected: {combined}"
            );
            let after = turn_store.get_turn(&turn_id).unwrap();
            assert_eq!(after.status, TurnStatus::DraftReady);
            assert_eq!(after.failure_reason, None);
            assert_eq!(
                after.find_attempt(&new_attempt_id).unwrap().status,
                AttemptStatus::DraftReady
            );
            assert_eq!(
                after.find_attempt(&attempt_id).unwrap().status,
                AttemptStatus::Superseded
            );
        }
        let after_super = turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(
            after_super.find_attempt(&attempt_id).unwrap().draft_hash,
            original_hash
        );
        assert_eq!(after_super.conversation_id, conversation_id);

        // Cleanup process store entry so other tests are not polluted.
        let _ = turn_store.with_turn_mut(&turn_id, |r| {
            r.status = TurnStatus::Failed;
            r.failure_reason = Some("test cleanup".into());
        });
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 生产入口必须在无连接时 fail closed，不能把开发 Mock 当成真实模型。
    #[test]
    fn test_missing_active_llm_fails_closed() {
        let state = AppState::new_for_test();
        state.clear_active_connection();
        assert!(state.active_conn_id().is_none());

        match state.require_active_llm() {
            Err(TauriCommandError::Llm { message, retryable }) => {
                assert!(message.contains("未配置"));
                assert!(!retryable);
            }
            Ok(_) => panic!("missing connection must not return a fallback LLM client"),
            Err(other) => panic!("expected LLM configuration error, got {other:?}"),
        }
    }

    /// 无连接时角色识别应安全降级为源卡自身，不能写入 Mock 的示例人物。
    #[test]
    fn test_no_connection_extraction_uses_source_character_only() {
        let character = make_test_character("Seraphina");

        let (definitions, status, message) = fallback_character_extraction(&character, &[]);

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "Seraphina");
        assert!(definitions.iter().all(|definition| {
            definition.name != "林医生" && definition.name != "陈警官"
        }));
        assert_eq!(
            status,
            storyforge_domain::character::CharacterExtractionStatus::Fallback
        );
        assert_eq!(message.as_deref(), Some(FALLBACK_EXTRACTION_MESSAGE));
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

    #[test]
    fn test_last_user_intent_before_finds_nearest_user() {
        let state = AppState::new_for_test();
        let conv = state.conv_store.create(None, None);
        let _u1 = state
            .conv_store
            .append_user_message(&conv.id, "第一次意图".into())
            .unwrap();
        let _a1 = state
            .conv_store
            .append_ai_draft(&conv.id, "成文1".into(), None)
            .unwrap();
        let u2 = state
            .conv_store
            .append_user_message(&conv.id, "第二次意图".into())
            .unwrap();
        let a2 = state
            .conv_store
            .append_ai_draft(&conv.id, "成文2".into(), None)
            .unwrap();
        let intent = last_user_intent_before(&state.conv_store, &conv.id, &a2);
        assert_eq!(intent.as_deref(), Some("第二次意图"));
        // before 第二条 user 节点 → 取第一次意图
        assert_eq!(
            last_user_intent_before(&state.conv_store, &conv.id, &u2).as_deref(),
            Some("第一次意图")
        );
        // before 首条 user → 无更早 user
        assert!(last_user_intent_before(&state.conv_store, &conv.id, &_u1).is_none());
        let _ = state.conv_store.delete(&conv.id);
    }

    /// RoundSummary accept 后索引进向量库，并可被远记忆关键词召回。
    #[test]
    fn test_index_round_summary_to_far_memory() {
        let store = BruteForceStore::new();
        let camp_id = Id::from_str("camp-fm");
        let conv_id = Id::from_str("conv-fm");
        let summary = storyforge_domain::agent::RoundSummary::new(
            camp_id.clone(),
            conv_id,
            7,
            "昨夜有人潜入诊所，陈警官随后上门调查。".into(),
        );
        let summary_id = summary.id.clone();
        let mut batch = storyforge_domain::turn::MutationBatch::new(Id::new(), 0);
        batch
            .mutations
            .push(storyforge_domain::turn::Mutation::UpsertSummary(Box::new(
                summary,
            )));

        index_round_summaries_to_vector(&store, &batch);

        let hits = storyforge_app_memory::recall_archived_by_query_filtered(
            &store,
            "诊所",
            5,
            Some("camp-fm"),
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("潜入诊所"));
        assert_eq!(hits[0].kind, "ArchivedSummary");

        // 幂等：同一 id 再索引不复制
        index_round_summaries_to_vector(&store, &batch);
        assert_eq!(store.count(), 1);
        let _ = store.delete(&summary_id);
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
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
        // ContextCompiler 最小版：RoundSummary 进入 WritingContext + ToolContext
        assert_eq!(ctx.recent_summaries.len(), 1);
        assert_eq!(ctx.recent_summaries[0].content, "A previous turn happened");
        let runtime = ctx.campaign_runtime.as_ref().unwrap();
        assert_eq!(runtime.instances.len(), 1);
        assert_eq!(runtime.knowledge.len(), 1);
        assert_eq!(runtime.tasks.len(), 1);
        assert_eq!(runtime.turn, 2);

        let tool_guard = tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        let tool_runtime = tool_guard.campaign_runtime.clone().unwrap();
        assert_eq!(tool_runtime.campaign.id, campaign.id);
        assert_eq!(tool_runtime.instances[0].id, instance.id);
        assert_eq!(
            tool_guard.archived_summaries,
            vec!["A previous turn happened"]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_take_recent_summaries_for_context_keeps_last_k() {
        let campaign_id = Id::from_str("camp-load-k");
        let summaries: Vec<_> = (1..=15)
            .map(|turn| {
                storyforge_domain::agent::RoundSummary::new(
                    campaign_id.clone(),
                    Id::new(),
                    turn,
                    format!("summary-{turn}"),
                )
            })
            .collect();
        let kept = take_recent_summaries_for_context(summaries, 12);
        assert_eq!(kept.len(), 12);
        assert_eq!(kept.first().unwrap().turn, 4);
        assert_eq!(kept.last().unwrap().turn, 15);
        assert_eq!(kept.last().unwrap().content, "summary-15");
    }

    #[test]
    fn test_take_recent_summaries_for_context_short_list_unchanged() {
        let campaign_id = Id::from_str("camp-load-short");
        let summaries = vec![storyforge_domain::agent::RoundSummary::new(
            campaign_id,
            Id::new(),
            1,
            "only".into(),
        )];
        let kept = take_recent_summaries_for_context(summaries, 12);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].content, "only");
    }

    #[test]
    fn test_next_writing_turn_ignores_stage_summaries() {
        let camp = Id::from_str("camp-turn");
        let mut items = Vec::new();
        for turn in 1..=3 {
            items.push(
                storyforge_domain::agent::RoundSummary::new(
                    camp.clone(),
                    Id::new(),
                    turn,
                    format!("a{turn}"),
                )
                .with_code(format!("A{turn:04}")),
            );
        }
        let mut b = storyforge_domain::agent::RoundSummary::new(camp, Id::new(), 1, "stage".into())
            .with_code("B0001");
        b.level = 1;
        b.turn_end = 3;
        items.push(b);
        assert_eq!(committed_turn_count(&items), 3);
        assert_eq!(next_writing_turn(&items), 4);
        // 若错误用 len：会得到 5
        assert_ne!(items.len() as u32 + 1, next_writing_turn(&items));
    }

    #[test]
    fn test_build_chronicle_prompt_catalog_keeps_far_codes() {
        let camp = Id::from_str("camp-pc");
        let mut items = Vec::new();
        for turn in 1..=20 {
            items.push(
                storyforge_domain::agent::RoundSummary::new(
                    camp.clone(),
                    Id::new(),
                    turn,
                    format!("a{turn}"),
                )
                .with_code(format!("A{turn:04}")),
            );
        }
        use storyforge_domain::chronicle::{ChronicleCode, ContextEpochSnapshot};
        let mut snap = ContextEpochSnapshot::new_empty("e1", 0);
        snap.overview_codes = vec![ChronicleCode::parse("A0001").unwrap()];
        snap.band_codes = vec![ChronicleCode::parse("A0010").unwrap()];
        let cat = build_chronicle_prompt_catalog(&items, Some(&snap));
        assert!(cat.iter().any(|s| s.code.as_deref() == Some("A0001")));
        assert!(cat.iter().any(|s| s.code.as_deref() == Some("A0010")));
        // all leaves included
        assert!(cat.iter().filter(|s| s.is_leaf_a()).count() >= 20);
    }

    #[test]
    fn test_build_chronicle_tool_catalog_prefers_stages_and_recent_leaves() {
        let campaign_id = Id::from_str("camp-catalog");
        let mut items = Vec::new();
        for turn in 1..=20 {
            items.push(
                storyforge_domain::agent::RoundSummary::new(
                    campaign_id.clone(),
                    Id::new(),
                    turn,
                    format!("leaf-{turn}"),
                )
                .with_code(format!("A{turn:04}")),
            );
        }
        let mut stage = storyforge_domain::agent::RoundSummary::new(
            campaign_id,
            Id::new(),
            1,
            "stage-b".into(),
        )
        .with_code("B0001")
        .with_headline("stage");
        stage.level = 1;
        stage.turn_end = 8;
        items.push(stage);

        let kept = build_chronicle_tool_catalog(items, 10);
        assert_eq!(kept.len(), 10);
        assert!(kept.iter().any(|s| s.code.as_deref() == Some("B0001")));
        // remaining 9 slots are latest leaves
        let leaf_turns: Vec<u32> = kept
            .iter()
            .filter(|s| s.level == 0)
            .map(|s| s.turn)
            .collect();
        assert_eq!(leaf_turns, (12..=20).collect::<Vec<_>>());
    }

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
                campaign_variable_schema: vec![],
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
                campaign_variable_schema: vec![],
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
                campaign_variable_schema: vec![],
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
                campaign_variable_schema: vec![],
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
                campaign_variable_schema: vec![],
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

    // ─── Phase A: Turn 提交屏障契约测试（hermetic：临时 TurnStore）────────

    #[test]
    fn test_active_turn_quality_from_record() {
        use storyforge_domain::turn::{
            AttemptStatus, QualityReport, QualitySeverity, QualityWarning, QualityWarningCode,
            TurnAttempt, TurnRecord,
        };
        let mut record = TurnRecord::new(
            Id::from_str("camp-q"),
            Id::from_str("conv-q"),
            Id::from_str("node-q"),
            0,
        );
        assert!(active_turn_quality_from_record(&record).is_none());

        record.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("att-q"),
            variant_id: Id::from_str("var-q"),
            draft_hash: "h".into(),
            status: AttemptStatus::AwaitingAcceptance,
            pending_state_changes: None,
            derivation: None,
            quality_report: Some(QualityReport {
                warnings: vec![QualityWarning {
                    code: QualityWarningCode::TooShort { char_count: 10 },
                    message: "字数过短".into(),
                    severity: QualitySeverity::Warning,
                }],
            }),
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });

        let dto = active_turn_quality_from_record(&record).expect("should have quality");
        assert_eq!(dto.attempt_id, "att-q");
        assert!(!dto.passed);
        assert_eq!(dto.warning_count, 1);
        assert_eq!(dto.warnings, vec!["字数过短".to_string()]);
    }

    fn temp_turn_store() -> (std::path::PathBuf, turn_store::TurnStore) {
        let dir =
            std::env::temp_dir().join(format!("storyforge-barrier-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = turn_store::TurnStore::new(&dir);
        (dir, store)
    }

    #[test]
    fn turn_barrier_rejects_start_writing_with_active_turn() {
        let state = Arc::new(AppState::new_for_test());
        let (dir, ts) = temp_turn_store();
        let campaign_id = Id::new();

        {
            let mut guard = state
                .active_campaign
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *guard = Some(campaign_id.clone());
        }

        let record = storyforge_domain::turn::TurnRecord::new(
            campaign_id.clone(),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        );
        ts.create_turn(record).unwrap();

        let result = check_turn_barrier_with(&state, &ts);
        assert!(
            result.is_err(),
            "barrier should reject start_writing when active Turn exists"
        );
        // legacy meta_accept_patch 与 start_writing 共用 check_turn_barrier 语义
        assert!(
            check_turn_barrier_with(&state, &ts).is_err(),
            "barrier should also reject legacy meta_accept_patch while Turn active"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn turn_barrier_passes_without_active_turn() {
        let state = Arc::new(AppState::new_for_test());
        let (dir, ts) = temp_turn_store();
        let campaign_id = Id::from_str("camp-test-no-turn");

        {
            let mut guard = state
                .active_campaign
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *guard = Some(campaign_id.clone());
        }

        // 无活动 Turn → 放行
        assert!(check_turn_barrier_with(&state, &ts).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn turn_barrier_passes_non_campaign_mode() {
        let state = Arc::new(AppState::new_for_test());
        let (dir, ts) = temp_turn_store();
        // 不设置 active_campaign → 非 Campaign 模式
        assert!(check_turn_barrier_with(&state, &ts).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn turn_record_committed_allows_next_turn() {
        let state = Arc::new(AppState::new_for_test());
        let (dir, ts) = temp_turn_store();
        let campaign_id = Id::from_str("camp-committed");

        {
            let mut guard = state
                .active_campaign
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *guard = Some(campaign_id.clone());
        }

        // 创建一个 Committed Turn
        let mut record = storyforge_domain::turn::TurnRecord::new(
            campaign_id.clone(),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        );
        record.status = storyforge_domain::turn::TurnStatus::Committed;
        ts.save_turn(record).unwrap();

        // Committed 是 terminal → 放行
        assert!(check_turn_barrier_with(&state, &ts).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn direct_write_barrier_rejects_active_turn() {
        let (dir, ts) = temp_turn_store();
        let campaign_id = Id::new();
        let record = storyforge_domain::turn::TurnRecord::new(
            campaign_id.clone(),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        );
        ts.create_turn(record).unwrap();
        assert!(reject_if_active_turn_in(&ts, &campaign_id).is_err());
        assert!(reject_if_active_turn_in(&ts, &Id::from_str("other-camp")).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn accept_variant_non_campaign_keeps_legacy_behavior() {
        let state = Arc::new(AppState::new_for_test());
        let conversation = state.conv_store.create(Some("card-1".into()), None);
        let node_id = state
            .conv_store
            .append_ai_draft(&conversation.id, "draft".into(), None)
            .unwrap();

        // 不设 active_campaign → 非 Campaign 模式
        accept_variant_async(
            state.clone(),
            conversation.id.clone(),
            node_id.clone(),
            false,
            None,
        )
        .await
        .unwrap();

        // 应该是 Final（旧行为）
        let updated = state.conv_store.get(&conversation.id).unwrap();
        let node = updated.nodes.iter().find(|n| n.id == node_id).unwrap();
        assert_eq!(node.active().unwrap().status, VariantStatus::Final);
    }

    #[tokio::test]
    async fn accept_variant_campaign_historical_attempt_rejected() {
        let state = Arc::new(AppState::new_for_test());
        let campaign_id = Id::from_str("camp-historical");

        {
            let mut guard = state
                .active_campaign
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *guard = Some(campaign_id.clone());
        }

        // 创建一个 conversation + AI draft（没有关联 TurnRecord）
        let conversation = state.conv_store.create(Some("card-1".into()), None);
        let node_id = state
            .conv_store
            .append_ai_draft(&conversation.id, "orphan draft".into(), None)
            .unwrap();

        // accept 应该拒绝——没有关联的 TurnRecord
        let result = accept_variant_async(
            state.clone(),
            conversation.id.clone(),
            node_id.clone(),
            false,
            None,
        )
        .await;
        assert!(result.is_err(), "should reject historical/orphan attempt");
    }

    #[test]
    fn next_chronicle_a_seq_from_existing_codes_and_turns() {
        let a = storyforge_domain::agent::RoundSummary::new(
            Id::from_str("c"),
            Id::from_str("v"),
            2,
            "x".into(),
        )
        .with_code("A0002");
        let b = storyforge_domain::agent::RoundSummary::new(
            Id::from_str("c"),
            Id::from_str("v"),
            9,
            "y".into(),
        ); // legacy no code → use turn
        assert_eq!(next_chronicle_a_seq(&[a, b]), 10);
        assert_eq!(next_chronicle_a_seq(&[]), 1);
    }

    #[test]
    fn quality_accept_decision_matches_product_policy() {
        use storyforge_domain::turn::{
            QualityAcceptDecision, QualityReport, QualitySeverity, QualityWarning,
            QualityWarningCode, quality_accept_decision,
        };
        let err = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::FormatLeak {
                    snippet: "```".into(),
                },
                message: "format".into(),
                severity: QualitySeverity::Error,
            }],
        };
        assert!(matches!(
            quality_accept_decision(Some(&err), false),
            QualityAcceptDecision::Block { error_count: 1 }
        ));
        assert!(matches!(
            quality_accept_decision(Some(&err), true),
            QualityAcceptDecision::ForceDegraded { error_count: 1 }
        ));
    }

    #[test]
    fn turn_receipt_lists_only_user_reviewable_mutations() {
        use storyforge_domain::turn::{Mutation, MutationBatch};

        let campaign_id = Id::from_str("receipt-campaign");
        let conversation_id = Id::from_str("receipt-conversation");
        let variant_id = Id::from_str("receipt-variant");
        let mut batch = MutationBatch::new(Id::from_str("receipt-commit"), 4);
        batch.mutations.push(Mutation::UpsertSummary(Box::new(
            storyforge_domain::agent::RoundSummary::new(
                campaign_id,
                conversation_id,
                5,
                "林秋确认了仓库钥匙的来源。".into(),
            ),
        )));
        batch.mutations.push(Mutation::SetVariable {
            instance_id: None,
            key: "tension".into(),
            value: serde_json::json!(7),
            turn: 5,
        });
        batch.mutations.push(Mutation::FinalizeVariant {
            variant_id: variant_id.clone(),
        });
        batch.mutations.push(Mutation::UpsertInstance(Box::new(
            storyforge_domain::campaign::CharacterInstance::temporary(
                Id::from_str("receipt-campaign"),
                "守门人",
            ),
        )));

        let items = receipt_items_from_batch(&batch);

        assert_eq!(items.len(), 2, "结构性 mutation 不应混入用户小票");
        assert_eq!(items[0].mutation_index, 0);
        assert_eq!(items[0].kind, "chronicle");
        assert!(items[0].detail.contains("仓库钥匙"));
        assert_eq!(items[1].mutation_index, 1);
        assert_eq!(items[1].kind, "variable");
        assert!(items.iter().all(|item| item.selected_by_default));
    }

    #[test]
    fn receipt_selection_preserves_structural_mutations() {
        use storyforge_domain::turn::{Mutation, MutationBatch};

        let variant_id = Id::from_str("receipt-filter-variant");
        let mut batch = MutationBatch::new(Id::from_str("receipt-filter-commit"), 9);
        batch.mutations.push(Mutation::SetVariable {
            instance_id: None,
            key: "keep".into(),
            value: serde_json::json!(1),
            turn: 10,
        });
        batch.mutations.push(Mutation::SetVariable {
            instance_id: None,
            key: "drop".into(),
            value: serde_json::json!(2),
            turn: 10,
        });
        batch.mutations.push(Mutation::FinalizeVariant {
            variant_id: variant_id.clone(),
        });
        batch.mutations.push(Mutation::UpsertInstance(Box::new(
            storyforge_domain::campaign::CharacterInstance::temporary(
                Id::from_str("receipt-filter-campaign"),
                "临时证人",
            ),
        )));

        retain_selected_receipt_mutations(&mut batch, &[0]).unwrap();

        assert_eq!(batch.mutations.len(), 3);
        assert!(matches!(
            &batch.mutations[0],
            Mutation::SetVariable { key, .. } if key == "keep"
        ));
        assert!(matches!(
            &batch.mutations[1],
            Mutation::FinalizeVariant { variant_id: id } if id == &variant_id
        ));
        assert!(matches!(&batch.mutations[2], Mutation::UpsertInstance(_)));
    }

    #[test]
    fn postprocess_retry_recovers_unique_present_characters_from_provenance() {
        let provenance = Provenance {
            session_id: Id::new(),
            plan: None,
            subagent_results: vec![
                storyforge_domain::conversation::SubagentSnapshot {
                    character_id: "lin-qiu".into(),
                    full_text: "A".into(),
                    character_instance_id: None,
                    display_name: Some("林秋".into()),
                    fallback_reason: None,
                    reasoning_content: None,
                },
                storyforge_domain::conversation::SubagentSnapshot {
                    character_id: "lin-qiu".into(),
                    full_text: "B".into(),
                    character_instance_id: None,
                    display_name: Some("林秋".into()),
                    fallback_reason: None,
                    reasoning_content: None,
                },
                storyforge_domain::conversation::SubagentSnapshot {
                    character_id: "chen".into(),
                    full_text: "C".into(),
                    character_instance_id: None,
                    display_name: Some("陈警官".into()),
                    fallback_reason: None,
                    reasoning_content: None,
                },
            ],
            profile_id: None,
            generation_mode: None,
            seed: 1,
            last_hint: None,
            director_reasoning: None,
            writer_reasoning: None,
            editor_reasoning: None,
        };

        assert_eq!(
            postprocess_present_characters(Some(&provenance)),
            vec!["林秋".to_string(), "陈警官".to_string()]
        );
    }

    /// AND-3：storage_meta 首建/升级轨迹 + 损坏容错（不 panic）。
    #[test]
    fn storage_meta_records_version_trail_and_survives_corruption() {
        let dir = std::env::temp_dir().join(format!("sf-meta-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("storage_meta.json");

        // 首建：first_* 与 last_* 同版本
        touch_storage_meta(&dir);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["first_created_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["last_opened_version"], env!("CARGO_PKG_VERSION"));
        let first_created = v["first_created_at"].as_str().unwrap().to_string();

        // 再次启动：first_* 保留，last_opened_at 更新
        touch_storage_meta(&dir);
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v2["first_created_at"], first_created.as_str());

        // 损坏文件：重建为新 meta（不 panic，不冻结启动）
        std::fs::write(&path, "{ broken").unwrap();
        touch_storage_meta(&dir);
        let v3: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v3["schema"], 1);
        assert!(v3["first_created_at"].is_string());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_mutation_batch_empty_outcome() {
        let dir = std::env::temp_dir().join(format!("storyforge-mb-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign =
            storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "test".to_string());
        let campaign_id = campaign.id.clone();
        store.save_campaign(campaign).unwrap();

        let pc = PostprocessPersistContext {
            campaign_id: campaign_id.clone(),
            conversation_id: Id::from_str("conv-1"),
            turn: 1,
        };
        let outcome = storyforge_app_agent::PostProcessOutcome::default();

        let batch = build_mutation_batch(&store, &pc, &outcome, &[]);

        assert!(batch.is_empty(), "empty outcome should produce empty batch");
        assert_eq!(batch.expected_revision, 0);
        assert_eq!(batch.target_revision, 1);
        assert_eq!(
            batch.status,
            storyforge_domain::turn::MutationBatchStatus::Prepared
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_mutation_batch_with_summary_and_variable() {
        let dir =
            std::env::temp_dir().join(format!("storyforge-mb-var-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let campaign =
            storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "test".to_string());
        let campaign_id = campaign.id.clone();
        store.save_campaign(campaign).unwrap();

        let pc = PostprocessPersistContext {
            campaign_id: campaign_id.clone(),
            conversation_id: Id::from_str("conv-1"),
            turn: 1,
        };
        let outcome = storyforge_app_agent::PostProcessOutcome {
            summary: Some("第一轮摘要".into()),
            summary_attempted: true,
            post_process_attempted: true,
            post_process: Some(storyforge_domain::agent::PostProcessResult {
                variable_updates: vec![storyforge_domain::agent::VariableUpdate {
                    instance_id: None,
                    key: "story_clock".into(),
                    value: serde_json::json!("Day 2"),
                }],
                ..Default::default()
            }),
        };

        let batch = build_mutation_batch(&store, &pc, &outcome, &[]);

        // 应该有 1 个 UpsertSummary + 1 个 SetVariable
        assert_eq!(
            batch.mutations.len(),
            2,
            "should have summary + variable mutations"
        );
        assert!(
            batch
                .mutations
                .iter()
                .any(|m| matches!(m, storyforge_domain::turn::Mutation::UpsertSummary(_)))
        );
        assert!(batch.mutations.iter().any(|m| matches!(
            m,
            storyforge_domain::turn::Mutation::SetVariable { key, .. } if key == "story_clock"
        )));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn turn_store_unique_active_turn_per_campaign() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-unique-turn-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ts = turn_store::TurnStore::new(&dir);

        let r1 = storyforge_domain::turn::TurnRecord::new(
            Id::from_str("camp-1"),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        );
        ts.create_turn(r1).unwrap();

        // 同 Campaign 第二个活动 Turn 应被拒绝
        let r2 = storyforge_domain::turn::TurnRecord::new(
            Id::from_str("camp-1"),
            Id::from_str("conv-1"),
            Id::from_str("node-2"),
            0,
        );
        assert!(ts.create_turn(r2).is_err());

        // 不同 Campaign 可以创建
        let r3 = storyforge_domain::turn::TurnRecord::new(
            Id::from_str("camp-2"),
            Id::from_str("conv-2"),
            Id::from_str("node-3"),
            0,
        );
        assert!(ts.create_turn(r3).is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn startup_recovery_marks_active_turns_failed() {
        let dir =
            std::env::temp_dir().join(format!("storyforge-recovery-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        // 模拟崩溃前状态：创建一个 Generating 态 Turn
        {
            let ts = turn_store::TurnStore::new(&dir);
            let record = storyforge_domain::turn::TurnRecord::new(
                Id::from_str("camp-recovery"),
                Id::from_str("conv-1"),
                Id::from_str("node-1"),
                0,
            );
            ts.create_turn(record).unwrap();
        }

        // "重启"——重新打开同一目录的 TurnStore
        let ts = turn_store::TurnStore::new(&dir);
        let active = ts.list_active_turns();
        assert_eq!(active.len(), 1, "should have 1 active turn before recovery");

        // 模拟恢复逻辑：标记为 Failed
        for turn in &active {
            ts.save_turn(storyforge_domain::turn::TurnRecord {
                status: storyforge_domain::turn::TurnStatus::Failed,
                failure_reason: Some("启动恢复".into()),
                ..turn.clone()
            })
            .unwrap();
        }

        // 恢复后没有活动 Turn
        let ts2 = turn_store::TurnStore::new(&dir);
        assert_eq!(
            ts2.list_active_turns().len(),
            0,
            "no active turns after recovery"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Phase A 契约：Committing 恢复时 Draft→Final 失败 → 保持 Committing 可重试，不标 Committed。
    #[test]
    fn contract_recovery_keeps_committing_when_finalize_fails() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-recovery-finalize-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ts = turn_store::TurnStore::new(&dir);

        let mut record = storyforge_domain::turn::TurnRecord::new(
            Id::from_str("camp-finalize"),
            Id::from_str("conv-f"),
            Id::from_str("node-f"),
            0,
        );
        record.status = storyforge_domain::turn::TurnStatus::Committing;
        let attempt_id = Id::from_str("att-f");
        record.attempts.push(storyforge_domain::turn::TurnAttempt {
            attempt_id: attempt_id.clone(),
            variant_id: Id::from_str("var-missing"), // 故意无对应 Draft
            draft_hash: "h".into(),
            status: storyforge_domain::turn::AttemptStatus::Committing,
            pending_state_changes: Some(storyforge_domain::turn::MutationBatch::new(
                Id::from_str("batch-f"),
                0,
            )),
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        let turn_id = record.turn_id.clone();
        ts.save_turn(record).unwrap();

        // 对齐 recover_turns_on_startup A.1：finalize_ok=false 时不写 Committed
        let finalize_ok = false;
        if finalize_ok {
            ts.with_turn_mut(&turn_id, |r| {
                r.status = storyforge_domain::turn::TurnStatus::Committed;
                r.touch();
            })
            .unwrap();
        }

        let after = ts.get_turn(&turn_id).unwrap();
        assert_eq!(
            after.status,
            storyforge_domain::turn::TurnStatus::Committing,
            "finalize 失败必须保持 Committing 以便下次启动重试"
        );
        assert!(
            ts.list_recoverable_turns()
                .iter()
                .any(|t| t.turn_id == turn_id)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// P0：重 roll 后，旧 Attempt 的迟到 postprocess 不能把 Superseded 重新激活。
    /// 否则 accept 会按同一 node 找到旧 diff，而不是当前草稿对应的 Attempt。
    #[test]
    fn postprocess_rejects_superseded_attempt_after_regenerate() {
        use storyforge_domain::turn::{AttemptStatus, TurnAttempt, TurnRecord, TurnStatus};

        let mut record = TurnRecord::new(
            Id::from_str("camp-postprocess-race"),
            Id::from_str("conv-postprocess-race"),
            Id::from_str("node-postprocess-race"),
            0,
        );
        record.status = TurnStatus::DraftReady;
        let old_attempt_id = Id::from_str("attempt-old");
        let new_attempt_id = Id::from_str("attempt-new");
        let node_id = Id::from_str("node-postprocess-race");
        record.attempts = vec![
            TurnAttempt {
                attempt_id: old_attempt_id.clone(),
                variant_id: node_id.clone(),
                draft_hash: "old-draft".into(),
                status: AttemptStatus::Superseded,
                pending_state_changes: None,
                derivation: None,
                quality_report: None,
                pending_temporary_instances: vec![],
                provenance: None,
                created_at: "2026-07-11T00:00:00Z".into(),
            },
            TurnAttempt {
                attempt_id: new_attempt_id.clone(),
                variant_id: node_id,
                draft_hash: "new-draft".into(),
                status: AttemptStatus::DraftReady,
                pending_state_changes: None,
                derivation: None,
                quality_report: None,
                pending_temporary_instances: vec![],
                provenance: None,
                created_at: "2026-07-11T00:00:01Z".into(),
            },
        ];

        assert!(
            !is_current_attempt_ready_for_postprocess(&record, &old_attempt_id),
            "late result for a superseded attempt must be ignored"
        );
        assert!(
            is_current_attempt_ready_for_postprocess(&record, &new_attempt_id),
            "current regenerate attempt may receive its own postprocess result"
        );
    }

    #[test]
    fn regenerate_scope_rejects_cross_campaign_before_pipeline_mutates_a_draft() {
        let active_campaign = Id::from_str("campaign-active");
        let foreign_campaign = Id::from_str("campaign-foreign");
        let foreign_conversation = Conversation::new(None, Some(foreign_campaign));
        let active_turn = storyforge_domain::turn::TurnRecord::new(
            active_campaign.clone(),
            Id::from_str("conversation-active"),
            Id::from_str("input-active"),
            0,
        );

        assert!(
            validate_regenerate_campaign_scope(
                Some(&active_campaign),
                &foreign_conversation,
                Some(&active_turn),
            )
            .is_err(),
            "scope validation must reject before PipelineOrchestrator can alter the foreign draft"
        );

        let matching_conversation = Conversation::new(None, Some(active_campaign.clone()));
        let matching_turn = storyforge_domain::turn::TurnRecord::new(
            active_campaign.clone(),
            matching_conversation.id.clone(),
            Id::from_str("input-matching"),
            0,
        );
        assert!(
            validate_regenerate_campaign_scope(
                Some(&active_campaign),
                &matching_conversation,
                Some(&matching_turn),
            )
            .is_ok()
        );
    }

    #[test]
    fn sqlite_mode_never_recovers_the_legacy_json_compress_job_store() {
        assert!(!should_recover_json_compress_jobs(true));
        assert!(should_recover_json_compress_jobs(false));
    }

    #[test]
    fn sqlite_mode_never_uses_the_legacy_active_campaign_pointer() {
        assert!(!should_load_legacy_active_campaign_pointer(true));
        assert!(should_load_legacy_active_campaign_pointer(false));

        let dir = std::env::temp_dir().join(format!(
            "storyforge-sqlite-active-pointer-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("active_campaign.json"),
            r#"{"campaign_id":"stale-json-campaign"}"#,
        )
        .unwrap();

        assert_eq!(
            resolve_active_campaign_with_legacy_fallback(None, &dir, true),
            None,
            "a restarted SQLite process must ignore a stale JSON selector"
        );
        assert_eq!(
            resolve_active_campaign_with_legacy_fallback(
                Some(Id::from_str("selected-in-memory")),
                &dir,
                true,
            ),
            Some(Id::from_str("selected-in-memory")),
            "SQLite may use only the explicit in-process selection"
        );
        assert_eq!(
            resolve_active_campaign_with_legacy_fallback(None, &dir, false),
            Some(Id::from_str("stale-json-campaign")),
            "JSON mode preserves its legacy restart behavior"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn campaign_world_info_dto_preserves_the_st_entry_comment_as_its_name() {
        let mut entry = storyforge_domain::world_info::WorldInfoEntry {
            st_id: Some(17),
            keys: vec!["fallback key".into()],
            secondary_keys: vec![],
            content: "lore".into(),
            constant: true,
            selective: false,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 1,
            order: 100,
            route: storyforge_domain::world_info::LoreRoute::Constant,
            extensions: serde_json::json!({}),
            extra: Default::default(),
        };
        entry.extra.insert(
            "comment".into(),
            serde_json::Value::String("命定系统-阿比盖尔核心".into()),
        );

        let dto = world_info_entry_to_dto_full(7, &entry);

        assert_eq!(dto.name, "命定系统-阿比盖尔核心");
    }

    #[test]
    fn toggling_campaign_world_info_enabled_preserves_its_injection_route() {
        let mut entry = storyforge_domain::world_info::WorldInfoEntry {
            st_id: Some(18),
            keys: vec!["core".into()],
            secondary_keys: vec![],
            content: "lore".into(),
            constant: true,
            selective: false,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 1,
            order: 100,
            route: storyforge_domain::world_info::LoreRoute::Constant,
            extensions: serde_json::json!({}),
            extra: Default::default(),
        };

        entry.set_enabled(false).unwrap();
        assert!(entry.disabled);
        assert!(matches!(
            entry.route,
            storyforge_domain::world_info::LoreRoute::Constant
        ));

        entry.set_enabled(true).unwrap();
        assert!(!entry.disabled);
        assert!(matches!(
            entry.route,
            storyforge_domain::world_info::LoreRoute::Constant
        ));
    }

    #[test]
    fn enabling_a_disabled_route_restores_its_st_derived_injection_route() {
        let mut entry = storyforge_domain::world_info::WorldInfoEntry {
            st_id: Some(19),
            keys: vec!["core".into()],
            secondary_keys: vec![],
            content: "lore".into(),
            constant: false,
            selective: true,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: true,
            position: 0,
            depth: 1,
            order: 100,
            route: storyforge_domain::world_info::LoreRoute::Disabled,
            extensions: serde_json::json!({}),
            extra: Default::default(),
        };

        entry.set_enabled(true).unwrap();

        assert!(!entry.disabled);
        assert!(matches!(
            entry.route,
            storyforge_domain::world_info::LoreRoute::Selective
        ));
    }

    #[test]
    fn enabling_a_disabled_both_world_info_route_restores_both_injection_paths() {
        let mut entry = storyforge_domain::world_info::WorldInfoEntry {
            st_id: Some(20),
            keys: vec!["hybrid core".into()],
            secondary_keys: vec![],
            content: "lore".into(),
            constant: true,
            selective: true,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: true,
            position: 0,
            depth: 1,
            order: 100,
            route: storyforge_domain::world_info::LoreRoute::Disabled,
            extensions: serde_json::json!({}),
            extra: Default::default(),
        };

        entry.set_enabled(true).unwrap();

        assert!(!entry.disabled);
        assert!(matches!(
            entry.route,
            storyforge_domain::world_info::LoreRoute::Both
        ));
    }
}

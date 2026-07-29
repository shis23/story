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
mod runtime_support;
mod shell_doc_protocol;
pub mod sqlite_runtime;
mod startup_support;
mod storage;
pub mod storage_backend;
pub mod storage_health;
pub mod turn_coordinator;
pub mod turn_lifecycle;
pub mod turn_store;

use commands::{
    campaigns::*, card_shell::*, cards::*, characters::*, connections::*, conversations::*,
    diagnostics::*, import_export::*, memory::*, meta::*, meta_typed::*, mvu::*, plugins::*,
    presets::*, profiles::*, turns::*, variables::*, world_info::*, writing::*,
};
// Stable crate-root surface used by deterministic/real-LLM harnesses. Keep the
// command module private while exposing only the shared postprocess helpers.
pub use commands::writing::{
    is_postprocess_instance_present, normalize_knowledge_update_for_postprocess,
};
#[cfg(test)]
use playthrough_lifecycle::delete_campaign_playthrough_in_store;
use production_postprocess::TurnAttemptSink;
#[cfg(test)]
use storyforge_domain::conversation::Conversation;

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
use storyforge_app_conversation::ConversationStore;
use storyforge_app_logging::{ExportOptions, LogFilter, LogKind, LogLevel, LogStore};
use storyforge_app_meta::{
    MvuApplyError, MvuApplyPreview, apply_schema_to_definition, compute_apply_preview,
};
use storyforge_app_pipeline::{PipelineOrchestrator, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
#[cfg(test)]
use storyforge_domain::conversation::VariantStatus;
use storyforge_domain::conversation::{Provenance, Role as ConversationRole};
use storyforge_domain::llm::{
    ChatMessage, LlmConnection, LlmConnectionSummary, LlmProtocol, SamplingParams, ToolMode,
};
use storyforge_domain::preset::{RegexScript, RegexScriptSource, merge_regex_script_sources};
use storyforge_domain::prompt_module::PromptProfile;
use storyforge_infra_llm::LlmClient;
use storyforge_infra_plugin_host::PluginRegistry;
use storyforge_infra_plugin_host::mvu_runtime::MvuExecuteResponse;
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, DraftAttemptRequest, PostprocessApplyRequest,
};
use storyforge_infra_util::secret_store::{
    SecretStore, SystemSecretStore, is_secret_ref, make_secret_ref, resolve_secret_value,
};
use storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord, VectorStore};
use tauri::Manager;

use crate::error::TauriCommandError;
use crate::mvu_webview_runtime::{MvuPendingMap, WebViewMvuRuntime, new_mvu_pending_map};
pub use commands::writing_regenerate::{RegenerateRequestDto, RegenerateTargetDto};
pub(crate) use startup_support::*;

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
        .filter(|id| !id.is_empty())
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
    let sqlite_active = sqlite_runtime::is_sqlite_active();
    let active_id = resolve_active_campaign_with_legacy_fallback(None, data_dir, sqlite_active);
    if sqlite_active {
        return active_id;
    }
    let active_id = active_id?;
    if campaign_exists_on_disk(data_dir, &active_id) {
        Some(active_id)
    } else {
        tracing::warn!(
            campaign_id = active_id.as_str(),
            "启动时丢弃指向不存在 Campaign 的活跃指针"
        );
        None
    }
}

fn campaign_exists_on_disk(data_dir: &Path, id: &Id) -> bool {
    let path = data_dir.join("campaigns.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(campaigns) = serde_json::from_str::<Vec<storyforge_domain::campaign::Campaign>>(&raw)
    else {
        return false;
    };
    campaigns.iter().any(|campaign| campaign.id == *id)
}

fn save_active_campaign(data_dir: &Path, id: Option<&Id>) -> Result<(), String> {
    let path = data_dir.join("active_campaign.json");
    let v = serde_json::json!({ "campaign_id": id.map(|i| i.as_str()).unwrap_or("") });
    storyforge_infra_util::atomic_write_json(&path, &v)
        .map_err(|error| format!("保存活跃 Campaign 失败: {error}"))
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
    /// Serialize active Campaign switching with deletion and pointer persistence.
    pub(crate) active_campaign_update: Mutex<()>,
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
            active_campaign_update: Mutex::new(()),
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

pub use runtime_support::*;

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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

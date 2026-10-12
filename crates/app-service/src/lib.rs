//! StoryForge 宿主无关的应用服务层。
//!
//! 由 `crates/tauri-app` 抽出：`AppState`、存储门面与各存储、Turn / 后处理工作流、
//! 派生向量索引，以及命令模块里的 DTO 与辅助函数（`services::*`）。本 crate 不依赖
//! tauri；桌面端（tauri-app）与非 Tauri 宿主（DSH sidecar）共用同一份生产代码。

pub mod backend_workflows;
pub mod background;
pub mod campaign_store;
pub mod card_studio_store;
pub mod compress_job_store;
pub mod connection_store;
pub mod error;
pub mod global_regex_store;
pub mod meta_backend;
pub mod module_store;
pub mod playthrough_lifecycle;
pub mod preset_store;
pub mod production_postprocess;
pub mod runtime_support;
pub mod services;
pub mod sqlite_compress_jobs;
pub mod sqlite_meta_repo;
pub mod sqlite_mvu_repo;
pub mod sqlite_runtime;
pub mod startup_recovery;
pub mod startup_support;
pub mod storage;
pub mod storage_backend;
pub mod storage_health;
pub mod turn_coordinator;
pub mod turn_lifecycle;
pub mod turn_store;
pub mod vector_index;

// 原 tauri-app 根模块对命令模块的 glob 导入：服务层代码沿用 `crate::Xxx` 路径。
use services::{
    cards::*, characters::*, connections::*, conversations::*, diagnostics::*, meta::*, writing::*,
};

use connection_store::ConnectionStore;
use preset_store::PresetStore;
use production_postprocess::TurnAttemptSink;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use storage::CharacterStore;
use tokio::sync::{oneshot, watch};

use storyforge_app_agent::ToolContext;
use storyforge_app_agent::runtime::PromptHook;
use storyforge_app_conversation::ConversationStore;
use storyforge_app_logging::LogStore;
use storyforge_app_meta::{MvuApplyError, apply_schema_to_definition, compute_apply_preview};
use storyforge_app_pipeline::{PipelineOrchestrator, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::conversation::Provenance;
use storyforge_domain::llm::{LlmConnection, LlmProtocol, SamplingParams, ToolMode};
use storyforge_domain::preset::{RegexScript, RegexScriptSource, merge_regex_script_sources};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_plugin_host::PluginRegistry;
use storyforge_infra_util::secret_store::{
    SecretStore, SystemSecretStore, is_secret_ref, make_secret_ref, resolve_secret_value,
};
use storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord, VectorStore};

use crate::error::TauriCommandError;

pub use backend_workflows::*;
pub use runtime_support::*;
pub use startup_support::*;

pub type PromptHookPendingMap =
    Arc<Mutex<std::collections::HashMap<String, oneshot::Sender<PromptHookReply>>>>;

pub struct PromptHookPendingGuard {
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

#[cfg(any(test, feature = "test-support"))]
static STORE: OnceLock<Arc<CharacterStore>> = OnceLock::new();
const EMBED_SECRET_KIND: &str = "embedder";
const EMBED_SECRET_ID: &str = "default";

#[cfg(any(test, feature = "test-support"))]
pub fn get_store() -> &'static CharacterStore {
    let store = STORE.get_or_init(|| Arc::new(CharacterStore::new(&get_app_data_dir())));
    store.as_ref()
}

static CONN_STORE: OnceLock<Arc<ConnectionStore>> = OnceLock::new();

pub fn get_conn_store() -> Arc<ConnectionStore> {
    CONN_STORE
        .get_or_init(|| {
            let data_dir = get_app_data_dir();
            Arc::new(ConnectionStore::new(&data_dir))
        })
        .clone()
}

static PRESET_STORE: OnceLock<PresetStore> = OnceLock::new();

pub fn get_preset_store() -> &'static PresetStore {
    PRESET_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        PresetStore::new(&data_dir)
    })
}

static GLOBAL_REGEX_STORE: OnceLock<global_regex_store::GlobalRegexStore> = OnceLock::new();

pub fn get_global_regex_store() -> &'static global_regex_store::GlobalRegexStore {
    GLOBAL_REGEX_STORE.get_or_init(|| {
        let data_dir = get_app_data_dir();
        global_regex_store::GlobalRegexStore::new(&data_dir)
    })
}

#[cfg(any(test, feature = "test-support"))]
static CAMPAIGN_STORE: OnceLock<Arc<campaign_store::CampaignStore>> = OnceLock::new();

#[cfg(any(test, feature = "test-support"))]
pub fn get_campaign_store() -> &'static campaign_store::CampaignStore {
    let store = CAMPAIGN_STORE
        .get_or_init(|| Arc::new(campaign_store::CampaignStore::new(&get_app_data_dir())));
    store.as_ref()
}

/// Read Turn state from the process-selected authority. SQLite mode never
/// consults the legacy JSON `TurnStore`, including on error paths.
pub fn get_active_turn_for_backend(
    storage: &storage_backend::StorageFacade,
    campaign_id: &Id,
) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
    storage.get_active_turn(campaign_id)
}

pub fn get_turn_by_variant_for_backend(
    storage: &storage_backend::StorageFacade,
    variant_id: &Id,
) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
    storage.get_turn_by_variant(variant_id)
}

// 三审9：启动恢复已抽取为单一公共入口 `startup_recovery::run_startup_recovery`
// （生产 setup hook 与子进程重启测试共同调用）。原先的 `recover_turns_on_startup`
// 自由函数包装器与 `recover_compress_jobs_on_startup` 直调已合并进去。

/// 在 `start_writing` 追加 user 消息**之前**调用。
/// 如果存在非 terminal Turn，返回错误，阻止新一轮启动。
/// 非 Campaign 模式（无活跃 Campaign）直接放行。
pub fn check_turn_barrier(state: &Arc<AppState>) -> Result<(), TauriCommandError> {
    let active_campaign_id = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };
    let Some(campaign_id) = active_campaign_id else {
        return Ok(());
    };
    reject_if_active_turn(state.storage(), &campaign_id)
}

/// 可注入 TurnStore 的屏障检查（测试 hermetic 化用）。
#[cfg(any(test, feature = "test-support"))]
pub fn check_turn_barrier_with(
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
pub fn reject_if_active_turn(
    storage: &storage_backend::StorageFacade,
    campaign_id: &Id,
) -> Result<(), TauriCommandError> {
    match storage.get_active_turn(campaign_id) {
        Ok(Some(turn)) => Err(TauriCommandError::validation(format!(
            "当前有未完成的轮次（turn_id={}, status={:?}），请先 Accept、Discard 或 Abandon 后再修改",
            turn.turn_id, turn.status
        ))),
        Ok(None) => Ok(()),
        Err(error) => Err(TauriCommandError::internal(format!(
            "active turn lookup failed: {error}"
        ))),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn reject_if_active_turn_in(
    turn_store: &turn_store::TurnStore,
    campaign_id: &Id,
) -> Result<(), TauriCommandError> {
    if let Some(turn) = turn_store.get_active_turn(campaign_id) {
        return Err(TauriCommandError::validation(format!(
            "当前有未完成的轮次（turn_id={}, status={:?}），请先 Accept、Discard 或 Abandon 后再修改",
            turn.turn_id, turn.status
        )));
    }
    Ok(())
}

pub fn to_json_value<T: Serialize + ?Sized>(
    value: &T,
    label: &str,
) -> Result<serde_json::Value, TauriCommandError> {
    serde_json::to_value(value)
        .map_err(|e| TauriCommandError::internal(format!("{label} 序列化失败: {e}")))
}

/// W10: 全局 MVU JS runtime。桌面端在 setup 里安装 WebView 实现，`new_pipeline`
/// 注入 PipelineOrchestrator；未安装（非 Tauri 宿主 / WebView 不可用）时降级为 None。
static MVU_RUNTIME: OnceLock<
    Arc<dyn storyforge_infra_plugin_host::mvu_runtime::MvuRuntime + Send + Sync>,
> = OnceLock::new();

/// 安装进程级 MVU JS runtime（只生效一次，重复安装返回 false）。
pub fn install_mvu_runtime(
    runtime: Arc<dyn storyforge_infra_plugin_host::mvu_runtime::MvuRuntime + Send + Sync>,
) -> bool {
    MVU_RUNTIME.set(runtime).is_ok()
}

/// 由 Tauri setup 阶段确认的进程级数据目录。
///
/// Android 的真实私有目录依赖运行中的 Activity，不能在 Builder 构造前猜测；
/// setup 完成后，所有延迟初始化的 store 都通过这里取得同一目录。
static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 应用数据目录纯解析逻辑的目标平台标识。抽取为参数是为了让纯函数
/// `resolve_app_data_dir` 可在单元测试中按平台断言，不触碰真实环境变量或文件系统。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDataDirTarget {
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
pub fn resolve_app_data_dir(
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
pub fn get_app_data_dir() -> PathBuf {
    if let Some(data_dir) = APP_DATA_DIR.get() {
        return data_dir.clone();
    }

    initialize_app_data_dir(None)
        .unwrap_or_else(|error| panic!("failed to initialize application data directory: {error}"))
}

pub fn initialize_app_data_dir(framework_data_dir: Option<&Path>) -> Result<PathBuf, String> {
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
    let migration = migrate_from_exe_dir_if_needed(&data_dir, exe_parent.as_deref());
    report_migration_outcome(&migration);
    APP_DATA_DIR
        .set(data_dir.clone())
        .map_err(|_| "application data directory initialization raced".to_string())?;

    Ok(data_dir)
}

/// 旧数据目录迁移的结果（S-06/R14）。
///
/// 旧实现返回 `()`：调用方除了日志没有任何判断依据，"迁了一半"和"迁完了"
/// 在代码里不可区分，失败还会照常打印"数据迁移完成"。
#[derive(Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// 无需迁移：无 `exe_parent`、旧目录不存在、或新旧同目录。
    NotApplicable,
    /// 新目录已有数据，且上次迁移没有留下失败标记 → 不覆盖用户数据。
    SkippedPopulated,
    /// 本次全部复制成功。
    Completed { copied_files: usize, retried: bool },
    /// 有复制失败。已留下 `.migration_incomplete` 标记：下次启动会重试。
    Incomplete {
        copied_files: usize,
        failures: Vec<String>,
        retried: bool,
    },
}

/// 迁移未完成的标记文件名（写在新目录内）。
///
/// 作用是把"上次没搬完"持久化到下次启动。没有它时，一次部分失败（例如
/// `campaigns/` 目录建好了但里面的文件没拷过来）会让下面的 `new_has_data`
/// 判定成"新目录已有数据" → 永远跳过迁移，旧目录里的数据再也搬不过来。
pub const MIGRATION_INCOMPLETE_MARKER: &str = ".migration_incomplete";

/// 把迁移结果落到日志与健康面（S-06/R14）。
///
/// 启动路径与测试共用这**一个**入口：旧实现把"数据迁移完成"直接写在拷贝循环后面，
/// 于是任何失败都伴随一句假成功日志，且没有任何可断言的返回值。
pub fn report_migration_outcome(outcome: &MigrationOutcome) {
    match outcome {
        MigrationOutcome::Completed {
            copied_files,
            retried,
        } => {
            tracing::info!(copied_files, retried, "数据迁移完成");
        }
        MigrationOutcome::Incomplete {
            copied_files,
            failures,
            retried,
        } => {
            let detail = failures.join("; ");
            tracing::error!(
                copied_files,
                retried,
                failure_count = failures.len(),
                detail = %detail,
                "数据迁移未完成：{} 项失败（旧目录未被修改，标记 {} 已留在新目录，下次启动重试）",
                failures.len(),
                MIGRATION_INCOMPLETE_MARKER
            );
            // 迁移是"尽力而为"的启动步骤，不阻断启动；但失败必须在健康面可见，
            // 否则用户看到的是"启动正常 + 一部分旧数据不见了"。
            // detail 只用相对条目名，不带用户绝对路径（健康报告会进前端）。
            crate::storage_health::record_backend_incident(
                "legacy_dir_migration_incomplete",
                &format!(
                    "从旧安装目录迁移数据未完成（{} 项失败）：{}；旧目录数据未改动，下次启动会重试",
                    failures.len(),
                    detail
                ),
            );
        }
        MigrationOutcome::NotApplicable | MigrationOutcome::SkippedPopulated => {}
    }
}

/// 从旧的 exe_dir/data 迁移到新的 OS 标准目录（仅当新目录为空时）。
///
/// `exe_parent` 显式传入（而非内部再 `current_exe()`），便于用临时目录测试
/// 迁移逻辑而不依赖真实 exe 路径。
///
/// S-06/R14：失败必须作为结果返回给调用方；旧实现在 `let _ =` 吞错之后
/// 无条件 `tracing::info!("数据迁移完成")`，给出假成功信号。
pub fn migrate_from_exe_dir_if_needed(
    new_dir: &Path,
    exe_parent: Option<&Path>,
) -> MigrationOutcome {
    let Some(exe_parent) = exe_parent else {
        return MigrationOutcome::NotApplicable;
    };
    let old_dir = exe_parent.join("data");

    if old_dir == new_dir || !old_dir.exists() {
        return MigrationOutcome::NotApplicable;
    }

    // 检查新目录是否为空（忽略已迁移的数据）
    let new_has_data = new_dir.join("characters.json").exists()
        || new_dir.join("connections.json").exists()
        || new_dir.join("campaigns").exists();

    let marker = new_dir.join(MIGRATION_INCOMPLETE_MARKER);
    let retried = marker.exists();
    if new_has_data && !retried {
        return MigrationOutcome::SkippedPopulated; // 新目录已有数据，不需要迁移
    }

    tracing::info!(
        "正在从旧数据目录迁移: {} → {}{}",
        old_dir.display(),
        new_dir.display(),
        if retried {
            "（上次迁移未完成，本次重试）"
        } else {
            ""
        }
    );

    let mut failures: Vec<String> = Vec::new();
    // 先落标记再拷贝：中途崩溃/断电也必须让下次启动重试，而不是当成迁完了。
    if let Err(e) = std::fs::write(&marker, "migration in progress\n") {
        failures.push(format!("写入迁移标记失败: {e}"));
    }

    let mut copied_files = 0usize;
    match std::fs::read_dir(&old_dir) {
        Err(e) => failures.push(format!("读取旧数据目录失败: {e}")),
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        failures.push(format!("读取旧数据目录条目失败: {e}"));
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let dest = new_dir.join(entry.file_name());
                if entry.path().is_dir() {
                    copied_files += copy_dir_recursive(&entry.path(), &dest, &name, &mut failures);
                } else {
                    match std::fs::copy(entry.path(), &dest) {
                        Ok(_) => copied_files += 1,
                        Err(e) => failures.push(format!("{name}: {e}")),
                    }
                }
            }
        }
    }

    if failures.is_empty() {
        // 只有全部成功才撤销标记。
        if let Err(e) = std::fs::remove_file(&marker) {
            tracing::warn!("迁移完成后清理标记失败: {e}");
        }
        MigrationOutcome::Completed {
            copied_files,
            retried,
        }
    } else {
        // 失败清单同时写进标记文件，排查时可直接读取。
        if let Err(e) = std::fs::write(
            &marker,
            format!("migration incomplete\n{}\n", failures.join("; ")),
        ) {
            tracing::warn!("更新迁移标记失败: {e}");
        }
        MigrationOutcome::Incomplete {
            copied_files,
            failures,
            retried,
        }
    }
}

/// 递归复制目录；返回成功复制的文件数，失败逐条追加到 `failures`。
///
/// S-06/R14：旧实现对 `create_dir_all`、`read_dir`、每个文件的 `fs::copy`
/// 全部静默吞错（`let _ =` / `if let Ok` / `entries.flatten()`），嵌套目录里
/// 的拷贝失败连一行日志都没有。`failures` 里的条目用**相对路径**，避免把
/// 用户绝对路径带进健康报告（会进前端）。
fn copy_dir_recursive(
    src: &std::path::Path,
    dst: &std::path::Path,
    rel: &str,
    failures: &mut Vec<String>,
) -> usize {
    if let Err(e) = std::fs::create_dir_all(dst) {
        failures.push(format!("{rel}: 建目录失败: {e}"));
        return 0;
    }
    let entries = match std::fs::read_dir(src) {
        Ok(entries) => entries,
        Err(e) => {
            failures.push(format!("{rel}: 读目录失败: {e}"));
            return 0;
        }
    };
    let mut copied = 0usize;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                failures.push(format!("{rel}: 读条目失败: {e}"));
                continue;
            }
        };
        let child_rel = format!("{rel}/{}", entry.file_name().to_string_lossy());
        let dest = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copied += copy_dir_recursive(&entry.path(), &dest, &child_rel, failures);
        } else {
            match std::fs::copy(entry.path(), &dest) {
                Ok(_) => copied += 1,
                Err(e) => failures.push(format!("{child_rel}: {e}")),
            }
        }
    }
    copied
}

fn load_embed_config(data_dir: &Path) -> Option<storyforge_infra_llm::EmbedConfig> {
    let secret_store = SystemSecretStore::default();
    load_embed_config_with_secret_store(data_dir, &secret_store)
}

pub fn load_embed_config_with_secret_store(
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

/// 停用嵌入模型：删除 `embed.json`（含 `.tmp` 备份）与系统凭据库里的 key。
pub fn clear_embed_config_files(
    data_dir: &Path,
    secret_store: &dyn SecretStore,
) -> Result<(), String> {
    let secret_ref = make_secret_ref(EMBED_SECRET_KIND, EMBED_SECRET_ID);
    if let Err(e) = secret_store.delete_secret(&secret_ref) {
        tracing::warn!("删除嵌入 API SecretRef 失败: {e}");
    }
    let path = data_dir.join("embed.json");
    for file in [
        path.clone(),
        PathBuf::from(format!("{}.tmp", path.display())),
    ] {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("删除嵌入配置失败 {}: {e}", file.display())),
        }
    }
    Ok(())
}

pub fn persist_embed_config_secret_ref(
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

pub fn load_active_campaign(data_dir: &Path) -> Option<Id> {
    let path = data_dir.join("active_campaign.json");
    let s = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("campaign_id")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())
        .map(Id::from_str)
}

pub fn should_load_legacy_active_campaign_pointer(sqlite_active: bool) -> bool {
    !sqlite_active
}

pub fn resolve_active_campaign_with_legacy_fallback(
    memory_active_id: Option<Id>,
    data_dir: &Path,
    storage: &storage_backend::StorageFacade,
) -> Option<Id> {
    if should_load_legacy_active_campaign_pointer(storage.is_sqlite()) {
        memory_active_id.or_else(|| load_active_campaign(data_dir))
    } else {
        memory_active_id
    }
}

pub fn load_active_campaign_for_backend(
    data_dir: &Path,
    storage: &storage_backend::StorageFacade,
) -> Option<Id> {
    let active_id = resolve_active_campaign_with_legacy_fallback(None, data_dir, storage);
    if storage.is_sqlite() {
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
pub fn begin_writing_operation(app: &AppState) -> (Id, watch::Receiver<bool>) {
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
pub fn clear_current_cancel_if(app: &AppState, operation_id: &Id) {
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
    pub data_dir: PathBuf,
    /// Process-pinned storage authority and capability contract.
    storage: Arc<storage_backend::StorageFacade>,
    pub conv_store: Arc<ConversationStore>,
    pub card_studio_store: card_studio_store::CardStudioStore,
    pub log_store: Arc<LogStore>,
    /// 工具上下文（导入角色卡时同步更新，RwLock 支持运行时写入）
    pub tool_ctx: Arc<RwLock<ToolContext>>,
    /// 当前运行的写作/regenerate 取消句柄（operation-owned）。
    /// None = 无运行中的写作。
    pub current_cancel: Mutex<Option<WritingCancelHandle>>,
    /// 等待前端插件处理最终 LLM messages prompt hook 的请求。
    pub prompt_hook_pending: PromptHookPendingMap,
    /// 当前活跃连接构造的 LLM client（None = 未配置，生产调用必须 fail closed）
    active_llm: Mutex<Option<Arc<dyn LlmClient>>>,
    /// 当前活跃连接的 ID（用于 get_active_connection 快速查询）
    active_conn_id: Mutex<Option<String>>,
    /// 序列化 async “设置活跃连接”中的写盘与内存 client 更新。
    /// 公开锁句柄只为测试能作为可控屏障持有它（同 `active_campaign_update`）。
    pub active_connection_update: tokio::sync::Mutex<()>,
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
    /// Gate 4 六审 P1：独立 SQLite 并发测试需作为可控屏障持有该锁，以钉死
    /// 删除/激活的危险窗口。公开的是锁句柄，无任何 backend 分派逻辑。
    pub active_campaign_update: Mutex<()>,
    /// Turn/Attempt/Accept/Postprocess workflow adapters selected once at
    /// construction from the startup-pinned storage backend (Gate 3).
    pub turn_workflow: Arc<backend_workflows::TurnWorkflow>,
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

impl AppState {
    /// Build the application state over a process-pinned storage backend.
    ///
    /// Gate 4 五审 P2：独立 SQLite 命令测试二进制以真实 SQLite AppState 驱动
    /// 真实命令（审查指出既有“命令级测试”实为 JSON 后端）。签名不变。
    pub fn new_with_backend(
        data_dir: PathBuf,
        storage: Arc<storage_backend::StorageFacade>,
    ) -> Result<Self, String> {
        if storage.data_dir() != data_dir.as_path() {
            return Err(
                "AppState and StorageFacade must share one canonical data directory".to_string(),
            );
        }
        storage.validate_runtime_authority()?;
        let character_store = if storage.is_json() {
            Some(storage.json_character_store_owned(
                storage_backend::BackendCapability::CharacterCommands,
                "hydrate JSON character runtime",
            )?)
        } else {
            None
        };
        std::fs::create_dir_all(&data_dir).ok();
        // AND-3：storage_meta 版本记录（升级检测基础；best-effort 不 panic）
        touch_storage_meta(&data_dir);
        let conv_dir = data_dir.join("conversations");
        let log_dir = data_dir.join("logs");

        let conv_store = if storage.is_sqlite() {
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

        // 启动恢复：从存储把已导入的角色卡 + 世界书同步进 tool_ctx
        // （否则每次重启 dev，tool_ctx 都是空的，写作时报"没有可用角色卡"）
        // 世界书：最后一张卡的条目 + 所有其他卡的 is_global 条目（全局共享）。
        // Gate 4 六审 P1：JSON 走 CharacterStore；SQLite 走 facade
        // `list_characters()`（V007 characters 表），否则 SQLite 重启后
        // tool_ctx.characters 恒为空，写作运行时看不到已导入角色。
        let stored_chars: Vec<storage::StoredCharacter> =
            if let Some(store) = character_store.as_deref() {
                store.list()
            } else {
                match storage.list_characters() {
                    Ok(chars) => chars,
                    Err(e) => {
                        tracing::warn!("启动恢复：SQLite 角色库读取失败，tool_ctx 留空: {e}");
                        Vec::new()
                    }
                }
            };
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

        // 向量存储（持久化到 data/vectors.json）
        let vector_store = Arc::new(BruteForceStore::with_persistence(
            data_dir.join("vectors.json"),
        ));

        // 域1 write_fence：把已冻结的存储路径（如向量库写入失败被隔离）并入既有
        // 健康报告，否则"写入持续 PermissionDenied"在 UI/诊断上无声。
        // 去重与 path 语义见 `storage_health::record_write_fence_state`。
        crate::storage_health::record_write_fence_state();

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

        // Gate 3: the Turn/Attempt/Accept workflow adapter is selected exactly
        // once here; commands never branch on the storage backend.
        let turn_workflow = Arc::new(backend_workflows::TurnWorkflow::new(
            storage.clone(),
            conv_store.clone(),
        ));

        Ok(Self {
            data_dir: data_dir.clone(),
            storage: storage.clone(),
            conv_store,
            card_studio_store: card_studio_store::CardStudioStore::new(&data_dir),
            turn_workflow,
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
            active_campaign: Mutex::new(load_active_campaign_for_backend(&data_dir, &storage)),
            active_campaign_update: Mutex::new(()),
            plugin_registry,
            module_store,
            profile_store,
            agent_profile_config_store,
            meta_session: Arc::new(meta_session),
            meta_conversations: Mutex::new(std::collections::HashMap::new()),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new_for_test() -> Self {
        Self::sweep_stale_test_data_dirs();
        let data_dir = std::env::temp_dir().join(format!(
            "storyforge-app-state-test-{}",
            uuid::Uuid::new_v4()
        ));
        let storage = Arc::new(storage_backend::StorageFacade::new(
            data_dir.clone(),
            storyforge_infra_sqlite::backend::PinnedBackend::new(
                storyforge_infra_sqlite::backend::StorageBackend::Json,
                storyforge_infra_sqlite::backend::BackendSource::Default,
            ),
        ));
        Self::new_with_backend(data_dir, storage).expect("test JSON facade must be self-consistent")
    }

    /// 惰性清扫历史测试残留（>24h 的 storyforge-app-state-test-*）。
    /// 测试结束不清理自身 data_dir（Drop 无钩子），跑一次 workspace 测试就
    /// 落几十个目录（2026-07-27 实测积累 360 个）。每进程至多扫一次，
    /// best-effort；24h 阈值不碰并行测试进程的目录。
    #[cfg(any(test, feature = "test-support"))]
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

    /// Access the process-pinned storage backend facade.
    ///
    /// Gate 4 五审 P2：独立 SQLite 命令测试二进制需经该访问器操作存储
    /// （读取断言/种子），签名不变。
    pub fn storage(&self) -> &Arc<storage_backend::StorageFacade> {
        &self.storage
    }

    pub fn json_character_store(
        &self,
        capability: storage_backend::BackendCapability,
        operation: &str,
    ) -> Result<&CharacterStore, TauriCommandError> {
        self.storage
            .json_character_store(capability, operation)
            .map_err(TauriCommandError::validation)
    }

    /// 当前活跃的 LLM client。无连接时返回 None，禁止生产态回退开发 Mock。
    pub fn active_llm(&self) -> Option<Arc<dyn LlmClient>> {
        let guard = self.active_llm.lock().unwrap_or_else(|p| p.into_inner());
        guard.clone()
    }

    /// 直接安装 LLM client：不经连接存储、不改活跃连接 id。
    ///
    /// 给不经连接设置的宿主（DSH sidecar 把模型调用交还宿主）与测试用；包装
    /// （重试 / 调用日志）由调用方负责。
    pub fn install_llm_client(&self, client: Arc<dyn LlmClient>) {
        *self.active_llm.lock().unwrap_or_else(|p| p.into_inner()) = Some(client);
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
        > = MVU_RUNTIME.get().cloned();
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
        if self.storage.defer_pipeline_conversation_land() {
            pipeline.set_defer_conversation_land(true);
        }
        Ok(pipeline)
    }
}

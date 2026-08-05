//! Application-level storage backend wiring.
//!
//! This module resolves the storage backend at process startup. SQLite is the
//! production default (Gate 7: 默认切换与兼容退场): with no marker and no
//! explicit override the process runs the fail-closed cutover — migrating a
//! legacy JSON tree in place, or initializing a fresh empty SQLite authority
//! for a brand-new user. JSON is retained only as an explicit fallback
//! (`STORYFORGE_STORAGE_BACKEND=json`, a `JsonAuthoritative` reverse-export
//! marker, or a settings config value) — no dual-write, no automatic data
//! deletion.
//!
//! After a successful cutover (or when a valid SQLite marker already exists),
//! `sqlite_runtime` is activated and becomes the sole authority for
//! Campaign / Conversation / Turn Accept and recovery. JSON stores are not
//! consulted for those operations and are never dual-written.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use storyforge_app_conversation::ConversationStore;
#[cfg(test)]
use storyforge_infra_sqlite::backend::DEFAULT_BACKEND_ENV_VAR;
use storyforge_infra_sqlite::backend::{
    BackendDiagnostics, BackendSelection, PinnedBackend, StorageBackend,
};
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker, recover_or_verify,
};
use storyforge_infra_sqlite::lease::hold_process_shared_lease;
use storyforge_infra_sqlite::migrations::current_version;

use crate::campaign_store::{CampaignStore, StoredCard};
use crate::compress_job_store::CompressJobStore;
use crate::error::TauriCommandError;
use crate::sqlite_runtime;
use crate::storage::CharacterStore;
use crate::stored_character_for_id_or_source_in_store;
use crate::turn_store::TurnStore;
use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::CharacterCard;
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::story_task::StoryTask;

/// Gate 4 P1-4: the character library DTO re-exported through the facade so
/// both backends expose one application-level character contract.
pub use crate::commands::characters::CharacterInfo;
/// Gate 4 七审 P1: world-info entry DTO (multi-role atomic write-back tests
/// construct per-role `world_info_entries` values).
pub use crate::commands::characters::WorldInfoEntryInfo;
/// Gate 4 P1-4: the stored-character DTO shared by the JSON and SQLite
/// character libraries.
pub use crate::storage::StoredCharacter;

/// The pinned backend for this process, resolved once at startup.
static PINNED: OnceLock<PinnedBackend> = OnceLock::new();

/// The canonical SQLite database filename in the app data directory.
pub const SQLITE_DB_FILENAME: &str = "storyforge.sqlite3";

/// Result of resolving the backend at startup.
#[derive(Debug, Clone)]
pub struct BackendResolution {
    pub pinned: PinnedBackend,
    pub db_path: Option<PathBuf>,
    pub diagnostics: BackendDiagnostics,
    pub cutover_performed: bool,
}

impl BackendResolution {
    pub fn is_sqlite(&self) -> bool {
        self.pinned.is_sqlite()
    }
}

/// Stable application-facing storage capabilities. Commands and application
/// services consume this contract instead of probing the process-global
/// SQLite handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendCapability {
    CampaignRead,
    CampaignInstanceRead,
    CampaignLifecycle,
    CardCommands,
    CharacterCommands,
    ImportExport,
    CampaignHealth,
    ConversationRead,
    TurnLifecycle,
    Postprocess,
    KnowledgeTaskRead,
    KnowledgeTaskCommands,
    VariableRead,
    VariableCommands,
    WorldInfo,
    TypedMetaPatch,
    MvuTranslation,
    MvuSchemaApply,
    ChroniclePublication,
    ChronicleCompressor,
    ActiveCampaignPersistence,
    StoryClock,
}

impl BackendCapability {
    pub const ALL: [Self; 22] = [
        Self::CampaignRead,
        Self::CampaignInstanceRead,
        Self::CampaignLifecycle,
        Self::CardCommands,
        Self::CharacterCommands,
        Self::ImportExport,
        Self::CampaignHealth,
        Self::ConversationRead,
        Self::TurnLifecycle,
        Self::Postprocess,
        Self::KnowledgeTaskRead,
        Self::KnowledgeTaskCommands,
        Self::VariableRead,
        Self::VariableCommands,
        Self::WorldInfo,
        Self::TypedMetaPatch,
        Self::MvuTranslation,
        Self::MvuSchemaApply,
        Self::ChroniclePublication,
        Self::ChronicleCompressor,
        Self::ActiveCampaignPersistence,
        Self::StoryClock,
    ];
}

/// Availability is explicit so unsupported or recovery-only behavior cannot
/// be mistaken for an empty successful result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStatus {
    Supported,
    Degraded,
    Unsupported,
    MigrationRequired,
    ReadOnlyRecovery,
}

/// Process-lifetime backend facade injected into `AppState`.
///
/// The facade owns the pinned selector and canonical data directory. Domain
/// ports are added to this type as Gate 3 migrates each vertical slice.
#[derive(Clone)]
pub struct StorageFacade {
    data_dir: PathBuf,
    pinned: PinnedBackend,
    json_campaign_store: Option<Arc<CampaignStore>>,
    json_character_store: Option<Arc<CharacterStore>>,
    json_turn_store: Option<Arc<TurnStore>>,
    json_compress_job_store: Option<Arc<CompressJobStore>>,
}

impl std::fmt::Debug for StorageFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorageFacade")
            .field("data_dir", &self.data_dir)
            .field("pinned", &self.pinned)
            .field("json_writers_constructed", &self.has_json_writers())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CampaignRecord {
    pub campaign: Campaign,
    pub instance_count: usize,
}

/// 跨后端 compress job 状态 DTO（id + 状态 + 尝试次数）。
/// Gate 5 等价矩阵：JSON CompressJobStore 与 SQLite compress_jobs 表经同一
/// facade 契约暴露，调用方无需感知物理布局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressJobState {
    pub id: Id,
    pub campaign_id: Id,
    pub status: String,
    pub attempts: u32,
}

impl StorageFacade {
    pub fn new(data_dir: PathBuf, pinned: PinnedBackend) -> Self {
        let (json_campaign_store, json_character_store, json_turn_store, json_compress_job_store) =
            if pinned.is_sqlite() {
                (None, None, None, None)
            } else {
                (
                    Some(Arc::new(CampaignStore::new(&data_dir))),
                    Some(Arc::new(CharacterStore::new(&data_dir))),
                    Some(Arc::new(TurnStore::new(&data_dir))),
                    Some(Arc::new(CompressJobStore::new(&data_dir))),
                )
            };
        Self {
            data_dir,
            pinned,
            json_campaign_store,
            json_character_store,
            json_turn_store,
            json_compress_job_store,
        }
    }

    pub fn backend(&self) -> StorageBackend {
        self.pinned.backend()
    }

    pub fn is_sqlite(&self) -> bool {
        self.pinned.is_sqlite()
    }

    pub fn is_json(&self) -> bool {
        !self.is_sqlite()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Whether this facade owns legacy JSON writer adapters. SQLite facades
    /// never construct them.
    pub fn has_json_writers(&self) -> bool {
        self.json_campaign_store.is_some()
            || self.json_character_store.is_some()
            || self.json_turn_store.is_some()
            || self.json_compress_job_store.is_some()
    }

    pub fn capability(&self, capability: BackendCapability) -> CapabilityStatus {
        if self.is_json() {
            return CapabilityStatus::Supported;
        }
        match capability {
            BackendCapability::CampaignRead
            | BackendCapability::CampaignInstanceRead
            | BackendCapability::CampaignHealth
            | BackendCapability::ConversationRead
            | BackendCapability::TurnLifecycle
            | BackendCapability::Postprocess
            | BackendCapability::KnowledgeTaskRead
            | BackendCapability::VariableRead
            | BackendCapability::MvuTranslation
            | BackendCapability::ChroniclePublication
            | BackendCapability::WorldInfo
            | BackendCapability::TypedMetaPatch
            | BackendCapability::MvuSchemaApply
            | BackendCapability::ChronicleCompressor
            | BackendCapability::StoryClock
            | BackendCapability::CharacterCommands
            | BackendCapability::ImportExport
            // Gate 5：create/update/delete 等价矩阵所需的四族能力补齐为
            // SQLite-native（facade 分派 + sqlite_runtime UoW）。
            | BackendCapability::CampaignLifecycle
            | BackendCapability::CardCommands
            | BackendCapability::KnowledgeTaskCommands
            | BackendCapability::VariableCommands => CapabilityStatus::Supported,
            BackendCapability::ActiveCampaignPersistence => CapabilityStatus::Degraded,
        }
    }

    /// Require a fully supported capability before entering an application
    /// path that would otherwise touch a backend-specific adapter.
    pub fn require_supported(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<(), String> {
        let status = self.capability(capability);
        if status == CapabilityStatus::Supported {
            return Ok(());
        }
        Err(format!(
            "{operation} is unavailable for {:?}: capability {capability:?} is {status:?}",
            self.backend()
        ))
    }

    /// Fail closed when the injected facade and the process-owned SQLite
    /// handle do not describe the same authority.
    pub fn validate_runtime_authority(&self) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::validate_active_path(&self.data_dir.join(SQLITE_DB_FILENAME))
        } else if sqlite_runtime::is_sqlite_active() {
            Err("JSON facade cannot coexist with an active SQLite runtime".to_string())
        } else {
            Ok(())
        }
    }

    pub fn json_campaign_store(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<&CampaignStore, String> {
        self.require_supported(capability, operation)?;
        self.json_campaign_store.as_deref().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CampaignStore for {:?}",
                self.backend()
            )
        })
    }

    pub(crate) fn json_campaign_store_owned(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<Arc<CampaignStore>, String> {
        self.require_supported(capability, operation)?;
        self.json_campaign_store.clone().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CampaignStore for {:?}",
                self.backend()
            )
        })
    }

    pub fn json_character_store(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<&CharacterStore, String> {
        self.require_supported(capability, operation)?;
        self.json_character_store.as_deref().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CharacterStore for {:?}",
                self.backend()
            )
        })
    }

    pub(crate) fn json_character_store_owned(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<Arc<CharacterStore>, String> {
        self.require_supported(capability, operation)?;
        self.json_character_store.clone().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CharacterStore for {:?}",
                self.backend()
            )
        })
    }

    pub fn json_turn_store(&self, operation: &str) -> Result<&TurnStore, String> {
        self.json_turn_store.as_deref().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy TurnStore for {:?}",
                self.backend()
            )
        })
    }

    pub(crate) fn json_compress_job_store(
        &self,
        operation: &str,
    ) -> Result<Arc<CompressJobStore>, String> {
        self.require_supported(BackendCapability::ChronicleCompressor, operation)?;
        self.json_compress_job_store.clone().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CompressJobStore for {:?}",
                self.backend()
            )
        })
    }

    /// 入队（或返回既有 open job）。返回 (job_id, created)。
    pub fn enqueue_compress_job(
        &self,
        campaign_id: &Id,
        conversation_id: Option<Id>,
        lineage_id: Option<Id>,
        uncovered_a: u32,
        uncovered_b: u32,
    ) -> Result<(Id, bool), String> {
        if self.is_sqlite() {
            let (job, created) = sqlite_runtime::compress_enqueue_or_get_open(
                campaign_id,
                conversation_id,
                lineage_id,
                uncovered_a,
                uncovered_b,
            )?;
            Ok((job.id, created))
        } else {
            let store = self.json_compress_job_store("enqueue chronicle compression")?;
            let (job, created) = store.enqueue_or_get_open(
                campaign_id,
                conversation_id,
                lineage_id,
                uncovered_a,
                uncovered_b,
            )?;
            Ok((job.id, created))
        }
    }

    /// 原子 claim：Pending → Running；已被 claim/终态返回 false。
    pub fn claim_compress_job(&self, job_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::compress_try_claim_pending(job_id)
        } else {
            self.json_compress_job_store("claim chronicle compression")?
                .try_claim_pending(job_id)
        }
    }

    /// 成功完成：Running → Succeeded。返回是否发生了转换。
    ///
    /// 双后端等价（Gate 5）：仅 Running 可终态化，迟到/并发 worker 不得改写已
    /// 终态化的 job。SQLite `transition` 用 `WHERE status='running'` 守卫；JSON
    /// 走 `mark_succeeded_if_running`（同守卫）。两者返回真实布尔，不再恒 true。
    pub fn succeed_compress_job(&self, job_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::compress_mark_succeeded(job_id)
        } else {
            let store = self.json_compress_job_store("succeed chronicle compression")?;
            store.mark_succeeded_if_running(job_id)
        }
    }

    /// 失败：未达 max_attempts 回 Pending（可重试），否则 Failed。仅 Running 可迁移。
    pub fn fail_or_retry_compress_job(&self, job_id: &Id, err: &str) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::compress_mark_failed_or_retry(job_id, err)
        } else {
            let store = self.json_compress_job_store("fail chronicle compression")?;
            store.mark_failed_or_retry_if_running(job_id, err)
        }
    }

    /// 全部 compress job 的当前状态（等价矩阵比较用）。
    pub fn list_compress_jobs(&self) -> Result<Vec<CompressJobState>, String> {
        let mut jobs: Vec<CompressJobState> = if self.is_sqlite() {
            sqlite_runtime::compress_list_all()?
                .into_iter()
                .map(|job| CompressJobState {
                    id: job.id,
                    campaign_id: job.campaign_id,
                    status: format!("{:?}", job.status).to_lowercase(),
                    attempts: job.attempts,
                })
                .collect()
        } else {
            self.json_compress_job_store("list chronicle compression")?
                .list_all()
                .into_iter()
                .map(|job| CompressJobState {
                    id: job.id,
                    campaign_id: job.campaign_id,
                    status: format!("{:?}", job.status).to_lowercase(),
                    attempts: job.attempts,
                })
                .collect()
        };
        jobs.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Ok(jobs)
    }

    /// 启动恢复原语（双后端公开入口）：把崩溃中断残留的 Running compress job
    /// reset 回 Pending，便于 worker 重放。SQLite 走 V006 表的单事务 UPDATE；
    /// JSON 走 CompressJobStore::reset_running_to_pending。返回被重置的数量。
    ///
    /// Gate 5 重启恢复证明（审查跟进）：测试可经此公开入口驱动「重启」恢复，
    /// 不必触达 `pub(crate)` 的 legacy store 句柄。
    pub fn reset_running_compress_jobs_to_pending(&self) -> Result<usize, String> {
        if self.is_sqlite() {
            sqlite_runtime::compress_reset_running_to_pending()
        } else {
            Ok(self
                .json_compress_job_store("reset running compress jobs")?
                .reset_running_to_pending())
        }
    }

    pub fn get_active_turn(
        &self,
        campaign_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_active_turn(campaign_id)
        } else {
            Ok(self
                .json_turn_store("get active turn")?
                .get_active_turn(campaign_id))
        }
    }

    pub fn get_turn_by_variant(
        &self,
        variant_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_turn_by_variant(variant_id)
        } else {
            Ok(self
                .json_turn_store("get turn by variant")?
                .get_turn_by_variant(variant_id))
        }
    }

    pub fn get_turn(
        &self,
        turn_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_turn(turn_id)
        } else {
            Ok(self.json_turn_store("get turn")?.get_turn(turn_id))
        }
    }

    pub fn save_turn(&self, turn: &storyforge_domain::turn::TurnRecord) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_turn(turn)
        } else {
            self.json_turn_store("save turn")?.create_turn(turn.clone())
        }
    }

    pub fn update_turn_record<F>(&self, turn_id: &Id, mutate: F) -> Result<(), String>
    where
        F: FnOnce(&mut storyforge_domain::turn::TurnRecord),
    {
        if self.is_sqlite() {
            sqlite_runtime::update_turn_record(turn_id, mutate)
        } else {
            self.json_turn_store("update turn")?
                .with_turn_mut(turn_id, mutate)
        }
    }

    pub fn mutate_turn_if<P, M>(
        &self,
        turn_id: &Id,
        predicate: P,
        mutate: M,
    ) -> Result<bool, String>
    where
        P: FnOnce(&storyforge_domain::turn::TurnRecord) -> bool,
        M: FnOnce(&mut storyforge_domain::turn::TurnRecord),
    {
        if self.is_sqlite() {
            sqlite_runtime::mutate_turn_if(turn_id, predicate, mutate)
        } else {
            self.json_turn_store("mutate turn")?
                .mutate_if(turn_id, predicate, mutate)
        }
    }

    pub fn save_campaign(&self, campaign: &Campaign) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_campaign(campaign)
        } else {
            // JSON 分支必须是 upsert（与 SQLite `save_campaign` 的
            // INSERT ... ON CONFLICT DO UPDATE 语义对齐）。Gate 5 三.6 修复：
            // 旧实现误用 `update_campaign`（仅更新既有行），fork / import 落
            // 新 Campaign 时 JSON 侧静默 no-op，fork 的 Campaign 从未落盘。
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "save campaign")?
                .save_campaign(campaign.clone())
        }
    }

    // ─── Gate 5: CRUD parity 分派（等价 JSON CampaignStore 语义）───────────

    /// JSON `CampaignStore::update_campaign` 语义：仅更新既有 Campaign。
    pub fn update_campaign(&self, campaign: &Campaign) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::update_campaign(campaign).map(|_| ())
        } else {
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "update campaign")?
                .update_campaign(campaign.clone())
        }
    }

    /// JSON `CampaignStore::add_instance` 语义：追加/替换实例。
    pub fn add_instance(&self, instance: &CharacterInstance) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_instance(instance)
        } else {
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "add instance")?
                .add_instance(instance.clone())
        }
    }

    /// JSON `CampaignStore::update_instance` 语义：仅更新既有实例。
    pub fn update_instance(&self, instance: &CharacterInstance) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::update_instance(instance).map(|_| ())
        } else {
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "update instance")?
                .update_instance(instance.clone())
        }
    }

    // ─── Gate 5: 三.5 active-turn 原子空闲修改（backend-neutral）────────────
    //
    // 「检查活动 Turn + 写入」合并进同一原子单元：JSON 在 TurnStore 的 turns
    // 锁守卫内完成 candidate→persist→swap（锁序 turns → CampaignStore 集合锁，
    // 与 `create_turn` 同序、无反向嵌套）；SQLite 在单个 BEGIN IMMEDIATE UoW
    // 内检查 + 写入。彻底消除「先查后写」的 TOCTOU 窗口（审查三.5）。

    /// 活动 Turn 屏障下的 Campaign 读改写。Campaign 不存在 → Ok(None)。
    pub fn mutate_idle_campaign<F>(
        &self,
        campaign_id: &Id,
        f: F,
    ) -> Result<Option<Campaign>, String>
    where
        F: FnOnce(&mut Campaign) -> Result<(), String>,
    {
        if self.is_sqlite() {
            sqlite_runtime::mutate_idle_campaign(campaign_id, f)
        } else {
            let turn_store = self.json_turn_store("idle campaign mutation")?;
            let campaign_store = self.json_campaign_store(
                BackendCapability::CampaignLifecycle,
                "idle campaign mutation",
            )?;
            turn_store.with_idle_turn_guard(campaign_id, || {
                campaign_store.mutate_campaign_candidate(campaign_id, f)
            })
        }
    }

    /// 活动 Turn 屏障下的角色实例读改写。实例不存在 → Ok(None)。
    pub fn mutate_idle_instance<F>(
        &self,
        campaign_id: &Id,
        instance_id: &Id,
        f: F,
    ) -> Result<Option<CharacterInstance>, String>
    where
        F: FnOnce(&mut CharacterInstance) -> Result<(), String>,
    {
        if self.is_sqlite() {
            sqlite_runtime::mutate_idle_instance(campaign_id, instance_id, f)
        } else {
            let turn_store = self.json_turn_store("idle instance mutation")?;
            let campaign_store = self.json_campaign_store(
                BackendCapability::CampaignLifecycle,
                "idle instance mutation",
            )?;
            turn_store.with_idle_turn_guard(campaign_id, || {
                campaign_store.mutate_instance_candidate(campaign_id, instance_id, f)
            })
        }
    }

    /// 活动 Turn 屏障 + Campaign 存在性校验 + 实例写入（同一原子单元）。
    /// `validate` 在写盘前对既有实例列表做重复检查（同名 / 同 definition）。
    pub(crate) fn add_idle_instance(
        &self,
        campaign_id: &Id,
        instance: &CharacterInstance,
        validate: impl FnOnce(&[CharacterInstance]) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::add_idle_instance(campaign_id, instance, validate)
        } else {
            let turn_store = self.json_turn_store("idle instance add")?;
            let campaign_store = self
                .json_campaign_store(BackendCapability::CampaignLifecycle, "idle instance add")?;
            turn_store.with_idle_turn_guard(campaign_id, || {
                campaign_store.add_instance_guarded(campaign_id, instance, validate)
            })
        }
    }

    /// 活动 Turn 屏障下的新建任务（同一原子单元）。
    pub fn add_idle_task(&self, campaign_id: &Id, task: &StoryTask) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::add_idle_task(campaign_id, task)
        } else {
            let turn_store = self.json_turn_store("idle task add")?;
            let campaign_store = self
                .json_campaign_store(BackendCapability::KnowledgeTaskCommands, "idle task add")?;
            turn_store.with_idle_turn_guard(campaign_id, || campaign_store.add_task(task.clone()))
        }
    }

    /// 活动 Turn 屏障下的任务读改写（屏障按任务所属 campaign 校验）。
    /// 任务不存在 → Ok(None)。
    pub fn mutate_idle_task<F>(&self, task_id: &Id, f: F) -> Result<Option<StoryTask>, String>
    where
        F: FnOnce(&mut StoryTask) -> Result<(), String>,
    {
        if self.is_sqlite() {
            sqlite_runtime::mutate_idle_task(task_id, f)
        } else {
            let campaign_store = self.json_campaign_store(
                BackendCapability::KnowledgeTaskCommands,
                "idle task mutation",
            )?;
            let Some(task) = campaign_store.get_task(task_id) else {
                return Ok(None);
            };
            let campaign_id = task.campaign_id.clone();
            let turn_store = self.json_turn_store("idle task mutation")?;
            turn_store.with_idle_turn_guard(&campaign_id, || {
                let mut candidate = campaign_store
                    .get_task(task_id)
                    .ok_or_else(|| format!("任务 {task_id} 不存在"))?;
                f(&mut candidate)?;
                let updated = candidate.clone();
                campaign_store.update_task(candidate)?;
                Ok(Some(updated))
            })
        }
    }

    /// 删除一局活动的全部压缩任务（delete_card 级联用，Gate 5 三.7）。
    ///
    /// SQLite：`delete_card_payload` 的单事务级联已含 chronicle_compress_jobs，
    /// 此处返回 0（no-op）；JSON：CompressJobStore 候选 → 持久化 → 换入删除。
    pub(crate) fn delete_compress_jobs_for_campaign(
        &self,
        campaign_id: &Id,
    ) -> Result<usize, String> {
        if self.is_sqlite() {
            Ok(0)
        } else {
            self.json_compress_job_store("delete card compress jobs cascade")?
                .delete_for_campaign(campaign_id)
        }
    }

    /// JSON `CampaignStore::create_campaign_with_instances` 语义：Campaign +
    /// Protagonist/Supporting 实例原子写入，返回 (StoredCard, Campaign, count)。
    pub fn create_campaign_with_instances(
        &self,
        campaign: Campaign,
    ) -> Result<(StoredCard, Campaign, usize), String> {
        if self.is_sqlite() {
            sqlite_runtime::create_campaign_with_instances(&campaign)
        } else {
            self.json_campaign_store(
                BackendCapability::CampaignLifecycle,
                "create campaign with instances",
            )?
            .create_campaign_with_instances(campaign)
        }
    }

    /// JSON `CampaignStore::delete_campaign` 语义：级联删除一局活动。
    pub fn delete_campaign(&self, id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_campaign_cascade(id)
        } else {
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "delete campaign")?
                .delete_campaign(id)
        }
    }

    /// 删除一局活动的前置产物（会话 + Turn），供 playthrough 删除在删除
    /// Campaign **之前**调用：JSON 多文件无事务，先删会话/Turn、失败时
    /// Campaign 仍在可重试；SQLite 的 `delete_campaign_cascade` 在同一事务内
    /// 级联删除 conversations + turns，前置清理是 no-op（单独的会话删除会
    /// 撞上「有 turn history 拒绝删除」的孤儿守卫，合法级联被误伤）。
    pub fn delete_campaign_precursors(
        &self,
        campaign_id: &Id,
        conversation_ids: &std::collections::HashSet<Id>,
        mut delete_conversation: impl FnMut(&Id) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            return Ok(());
        }
        for conversation_id in conversation_ids {
            delete_conversation(conversation_id)?;
        }
        // turns.json 与 CampaignStore 分开持久化：删除本局 Turn。
        self.json_turn_store("delete campaign turns")?
            .delete_turns_for_campaign(campaign_id)
            .map(|_| ())
    }

    /// JSON `CampaignStore::get_task` 语义。
    pub fn get_task(&self, task_id: &Id) -> Result<Option<StoryTask>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_task(task_id)
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::KnowledgeTaskRead, "get task")?
                .get_task(task_id))
        }
    }

    /// JSON `CampaignStore::add_task` 语义。
    pub fn add_task(&self, task: &StoryTask) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_task(task)
        } else {
            self.json_campaign_store(BackendCapability::KnowledgeTaskCommands, "add task")?
                .add_task(task.clone())
        }
    }

    /// JSON `CampaignStore::update_task` 语义：仅更新既有任务。
    pub fn update_task(&self, task: &StoryTask) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::update_task(task).map(|_| ())
        } else {
            self.json_campaign_store(BackendCapability::KnowledgeTaskCommands, "update task")?
                .update_task(task.clone())
        }
    }

    /// JSON `CampaignStore::delete_task` 语义。
    pub fn delete_task(&self, task_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_task(task_id)
        } else {
            self.json_campaign_store(BackendCapability::KnowledgeTaskCommands, "delete task")?
                .delete_task(task_id)
        }
    }

    /// JSON `CampaignStore::add_knowledge` 语义：批量追加知识条目。
    pub fn add_knowledge(&self, entries: &[CharacterKnowledgeEntry]) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::add_knowledge_batch(entries)
        } else {
            self.json_campaign_store(BackendCapability::KnowledgeTaskCommands, "add knowledge")?
                .add_knowledge(entries.to_vec())
        }
    }

    /// JSON `CampaignStore::delete_knowledge` 语义。
    pub fn delete_knowledge(&self, knowledge_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_knowledge(knowledge_id)
        } else {
            self.json_campaign_store(BackendCapability::KnowledgeTaskCommands, "delete knowledge")?
                .delete_knowledge(knowledge_id)
        }
    }

    /// 列出全部角色卡（StoredCard 形态）。
    pub fn list_cards(&self) -> Result<Vec<StoredCard>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_card_payloads()?
                .into_iter()
                .map(|payload| {
                    serde_json::from_value(payload)
                        .map_err(|e| format!("解析角色卡 payload 失败: {e}"))
                })
                .collect()
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::CardCommands, "list cards")?
                .list_cards())
        }
    }

    /// JSON `CampaignStore::save_card` 语义：按 source_character_id 去重覆盖。
    ///
    /// SQLite 差异（fail-closed，已记录）：JSON 会静默移除被 Campaign 引用的
    /// 旧卡（留下孤儿引用）；SQLite 的 `campaigns.card_id` FK 拒绝删除被引用
    /// 的卡行，返回错误而非静默孤儿化。不触发该分支的路径两后端行为一致。
    pub fn save_card(&self, card: CharacterCard) -> Result<StoredCard, String> {
        if self.is_sqlite() {
            sqlite_runtime::save_card_with_dedupe(card)
        } else {
            self.json_campaign_store(BackendCapability::CardCommands, "save card")?
                .save_card(card)
        }
    }

    /// JSON `CampaignStore::save_card_if_no_campaigns` 语义：被 Campaign 引用的
    /// 卡拒绝覆盖（`FORCE_RERUN_BLOCKED_BY_CAMPAIGN`）。
    pub fn save_card_if_no_campaigns(&self, card: CharacterCard) -> Result<StoredCard, String> {
        if self.is_sqlite() {
            sqlite_runtime::save_card_with_dedupe(card)
        } else {
            self.json_campaign_store(BackendCapability::CardCommands, "save card")?
                .save_card_if_no_campaigns(card)
        }
    }

    pub fn get_card_payload(&self, card_id: &Id) -> Result<Option<serde_json::Value>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_card_payload(card_id)
        } else {
            self.json_campaign_store(BackendCapability::CardCommands, "get card payload")?
                .get_card(card_id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| format!("serialize card payload: {error}"))
        }
    }

    pub fn list_card_payloads(&self) -> Result<Vec<serde_json::Value>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_card_payloads()
        } else {
            self.json_campaign_store(BackendCapability::CardCommands, "list card payloads")?
                .list_cards()
                .into_iter()
                .map(|card| {
                    serde_json::to_value(card)
                        .map_err(|error| format!("serialize card payload: {error}"))
                })
                .collect()
        }
    }

    pub fn save_mvu(
        &self,
        stored: &crate::campaign_store::StoredMvuTranslation,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_mvu(stored)
        } else {
            self.json_campaign_store(BackendCapability::MvuTranslation, "save MVU translation")?
                .save_mvu(stored.clone())
        }
    }

    pub fn get_mvu(
        &self,
        source_character_id: &Id,
    ) -> Result<Option<crate::campaign_store::StoredMvuTranslation>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_mvu(source_character_id)
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::MvuTranslation, "get MVU translation")?
                .get_mvu(source_character_id))
        }
    }

    pub fn list_mvu(&self) -> Result<Vec<crate::campaign_store::StoredMvuTranslation>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_mvu()
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::MvuTranslation, "list MVU translations")?
                .list_all_mvu())
        }
    }

    /// Persist the active Campaign pointer. JSON writes `active_campaign.json`
    /// (legacy bootstrap pointer); SQLite keeps the selection in-process
    /// (`ActiveCampaignPersistence` is Degraded) and this is a no-op.
    pub fn save_active_pointer(&self, campaign_id: Option<&Id>) -> Result<(), String> {
        if !self.is_sqlite() {
            save_active_campaign(&self.data_dir, campaign_id)?;
        }
        Ok(())
    }

    /// Whether the pipeline must defer conversation land to the pre-accept
    /// UoW (SQLite) instead of landing drafts into the ConversationStore.
    pub fn defer_pipeline_conversation_land(&self) -> bool {
        self.is_sqlite()
    }

    pub fn list_campaigns(&self, card_id: Option<&Id>) -> Result<Vec<CampaignRecord>, String> {
        let campaigns = if self.is_sqlite() {
            sqlite_runtime::list_campaigns()?
        } else if let Some(card_id) = card_id {
            let json_store =
                self.json_campaign_store(BackendCapability::CampaignRead, "list campaigns")?;
            json_store.list_campaigns_of_card(card_id)
        } else {
            self.json_campaign_store(BackendCapability::CampaignRead, "list campaigns")?
                .list_campaigns()
        };

        campaigns
            .into_iter()
            .filter(|campaign| card_id.is_none_or(|card_id| campaign.card_id == *card_id))
            .map(|campaign| {
                let instance_count = if self.is_sqlite() {
                    sqlite_runtime::list_instances(&campaign.id)?.len()
                } else {
                    let json_store = self.json_campaign_store(
                        BackendCapability::CampaignRead,
                        "count campaign instances",
                    )?;
                    json_store.list_instances(&campaign.id).len()
                };
                Ok(CampaignRecord {
                    campaign,
                    instance_count,
                })
            })
            .collect()
    }

    pub fn get_campaign(&self, campaign_id: &Id) -> Result<Option<CampaignRecord>, String> {
        let campaign = if self.is_sqlite() {
            sqlite_runtime::get_campaign(campaign_id)?
        } else {
            self.json_campaign_store(BackendCapability::CampaignRead, "get campaign")?
                .get_campaign(campaign_id)
        };
        let Some(campaign) = campaign else {
            return Ok(None);
        };
        let instance_count = if self.is_sqlite() {
            sqlite_runtime::list_instances(&campaign.id)?.len()
        } else {
            let json_store = self
                .json_campaign_store(BackendCapability::CampaignRead, "count campaign instances")?;
            json_store.list_instances(&campaign.id).len()
        };
        Ok(Some(CampaignRecord {
            campaign,
            instance_count,
        }))
    }

    pub fn campaign_exists(&self, campaign_id: &Id) -> Result<bool, String> {
        self.get_campaign(campaign_id)
            .map(|record| record.is_some())
    }

    pub fn list_instances(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::campaign::CharacterInstance>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_instances(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(
                    BackendCapability::CampaignInstanceRead,
                    "list campaign instances",
                )?
                .list_instances(campaign_id))
        }
    }

    pub fn get_instance(
        &self,
        campaign_id: &Id,
        instance_id: &Id,
    ) -> Result<Option<storyforge_domain::campaign::CharacterInstance>, String> {
        Ok(self
            .list_instances(campaign_id)?
            .into_iter()
            .find(|instance| instance.id == *instance_id))
    }

    pub fn list_knowledge(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_knowledge(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(
                    BackendCapability::KnowledgeTaskRead,
                    "list character knowledge",
                )?
                .list_knowledge(campaign_id))
        }
    }

    pub fn list_tasks(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::story_task::StoryTask>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_tasks(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::KnowledgeTaskRead, "list story tasks")?
                .list_tasks(campaign_id))
        }
    }

    pub fn list_summaries(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::agent::RoundSummary>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_summaries(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(
                    BackendCapability::ChroniclePublication,
                    "list round summaries",
                )?
                .list_summaries(campaign_id))
        }
    }

    // ─── Gate 4 P1-4: character library (backend-neutral facade) ──────────

    /// Persist an imported character card. JSON writes `characters.json`;
    /// SQLite writes the V007 `characters` table.
    pub fn save_character(&self, info: CharacterInfo) -> Result<StoredCharacter, String> {
        if self.is_sqlite() {
            sqlite_runtime::save_character(&info)
        } else {
            self.json_character_store(BackendCapability::CharacterCommands, "save character")?
                .save(info)
        }
    }

    /// List every stored character.
    pub fn list_characters(&self) -> Result<Vec<StoredCharacter>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_characters()
        } else {
            Ok(self
                .json_character_store(BackendCapability::CharacterCommands, "list characters")?
                .list())
        }
    }

    /// Get a stored character by stored id or source `Character.id`.
    pub fn get_character(&self, id_or_source: &str) -> Result<Option<StoredCharacter>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_character(id_or_source)
        } else {
            let store =
                self.json_character_store(BackendCapability::CharacterCommands, "get character")?;
            Ok(stored_character_for_id_or_source_in_store(
                store,
                &Id::from_str(id_or_source),
            ))
        }
    }

    /// Delete a stored character by stored id or source `Character.id`.
    pub fn delete_character(&self, id_or_source: &str) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_character(id_or_source)
        } else {
            self.json_character_store(BackendCapability::CharacterCommands, "delete character")?
                .delete(id_or_source)
        }
    }

    /// Update a character world-info entry route (world_info_entries[i].route).
    pub fn update_character_world_info_route(
        &self,
        id_or_source: &str,
        entry_index: usize,
        new_route: &str,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_world_info_route_update(
                    info,
                    entry_index,
                    new_route,
                )
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "update character world info route",
            )?
            .update_world_info_route(id_or_source, entry_index, new_route)
        }
    }

    /// Update a character world-info entry's keys/content/constant/is_global/depth/order.
    #[allow(clippy::too_many_arguments)]
    pub fn update_character_world_info_entry(
        &self,
        id_or_source: &str,
        entry_index: usize,
        keys: Vec<String>,
        content: String,
        constant: bool,
        is_global: bool,
        depth: i32,
        order: i32,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_world_info_entry_update(
                    info,
                    entry_index,
                    keys,
                    content,
                    constant,
                    is_global,
                    depth,
                    order,
                )
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "update character world info entry",
            )?
            .update_world_info_entry(
                id_or_source,
                entry_index,
                keys,
                content,
                constant,
                is_global,
                depth,
                order,
            )
        }
    }

    /// Append a character world-info entry, returning its new index.
    pub fn add_character_world_info_entry(
        &self,
        id_or_source: &str,
        keys: Vec<String>,
        content: String,
        constant: bool,
        is_global: bool,
    ) -> Result<usize, String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_add_world_info_entry(
                    info, keys, content, constant, is_global,
                )
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "add character world info entry",
            )?
            .add_world_info_entry(id_or_source, keys, content, constant, is_global)
        }
    }

    /// Remove a character world-info entry.
    pub fn delete_character_world_info_entry(
        &self,
        id_or_source: &str,
        entry_index: usize,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_delete_world_info_entry(info, entry_index)
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "delete character world info entry",
            )?
            .delete_world_info_entry(id_or_source, entry_index)
        }
    }

    /// Replace a character's whole world-info entry list (bulk patch).
    pub fn update_character_world_info_entries_bulk(
        &self,
        id_or_source: &str,
        entries: Vec<crate::WorldInfoEntryInfo>,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_update_world_info_entries_bulk(info, entries)
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "replace character world info entries",
            )?
            .update_world_info_entries_bulk(id_or_source, entries)
        }
    }

    /// 原子批量替换多个角色的 world_info_entries（Gate 4 七审 P1）。
    ///
    /// 无活动 Campaign 时 `meta_accept_patch` 把 patch 后的全局世界书写回所有
    /// 角色卡。逐角色 `update_character_world_info_entries_bulk` 各自独立事务，
    /// 第二个角色失败时第一个已永久更新——部分提交。本方法在**单一原子操作**
    /// 内更新全部角色：SQLite 走单 UoW 事务、JSON 走单次 persist，任一步失败
    /// 整体回滚。
    pub fn update_character_world_info_entries_bulk_multi(
        &self,
        entries: &[(String, Vec<crate::WorldInfoEntryInfo>)],
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::update_world_info_entries_bulk_multi(entries)
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "replace multiple characters' world info entries atomically",
            )?
            .update_world_info_entries_bulk_multi(entries)
        }
    }

    // ─── Gate 4 P1-4: character delete cascade (backend-neutral) ──────────

    /// MVU translation delete (character delete cascade).
    pub fn delete_mvu(&self, source_character_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_mvu(source_character_id)
        } else {
            self.json_campaign_store(BackendCapability::MvuTranslation, "delete MVU cascade")?
                .delete_mvu(source_character_id)
        }
    }

    /// Look up a card wrapper by its ST source character id (cascade bridge).
    pub fn get_card_by_source(
        &self,
        source_character_id: &Id,
    ) -> Result<Option<StoredCard>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_card_payload_by_source(source_character_id)?
                .map(|payload| {
                    serde_json::from_value(payload)
                        .map_err(|e| format!("解析角色卡 payload 失败: {e}"))
                })
                .transpose()
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::CampaignLifecycle, "get card by source")?
                .get_card_by_source(source_character_id))
        }
    }

    /// Resolve the character-library world-info template for a source
    /// character id, backend-neutral (Gate 4 四审 P1: set_active_campaign 不再
    /// 触碰 JSON store)。JSON 复用既有 CharacterStore 语义；SQLite 读角色库。
    pub fn resolve_character_world_info_template(
        &self,
        source_character_id: &Id,
    ) -> Result<Option<storyforge_domain::world_info::WorldInfoBook>, String> {
        if self.is_sqlite() {
            let Some(stored) = sqlite_runtime::get_character(source_character_id.as_str())? else {
                return Ok(None);
            };
            let info = stored.info;
            if let Some(book) = info.embedded_world_info.clone() {
                return crate::commands::campaigns::merge_global_entries_into_book_facade(
                    self, book, &info.name,
                )
                .map(Some);
            }
            if let Some(book) =
                crate::startup_support::world_info_book_from_entries(&info.world_info_entries)
            {
                return crate::commands::campaigns::merge_global_entries_into_book_facade(
                    self, book, &info.name,
                )
                .map(Some);
            }
            Ok(None)
        } else {
            let store = self.json_character_store(
                BackendCapability::CharacterCommands,
                "world info template",
            )?;
            let Some(stored) =
                stored_character_for_id_or_source_in_store(store, source_character_id)
            else {
                return Ok(None);
            };
            let info = stored.info;
            if let Some(book) = info.embedded_world_info.clone() {
                return Ok(Some(
                    crate::commands::campaigns::merge_global_entries_into_book(
                        store, book, &info.name,
                    ),
                ));
            }
            if let Some(book) =
                crate::startup_support::world_info_book_from_entries(&info.world_info_entries)
            {
                return Ok(Some(
                    crate::commands::campaigns::merge_global_entries_into_book(
                        store, book, &info.name,
                    ),
                ));
            }
            Ok(None)
        }
    }

    /// Delete a card payload with its campaign cascade (character delete).
    pub fn delete_card(&self, card_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_card_payload(card_id)
        } else {
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "delete card")?
                .delete_card(card_id)
        }
    }

    /// Delete one character with its **entire** cascade (MVU + card + all
    /// campaign dependent rows) in a single atomic operation. SQLite: one
    /// transaction; JSON: existing `delete_character` cascade semantics.
    pub fn delete_character_full_cascade(
        &self,
        id: &str,
        extra_source_ids: &[Id],
    ) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_character_full_cascade(id, extra_source_ids)
        } else {
            // JSON 路径：沿用 CharacterStore + CampaignStore 既有级联。
            let character_store = self
                .json_character_store(BackendCapability::CharacterCommands, "delete character")?;
            let removed = character_store.delete(id)?;
            let campaign_store =
                self.json_campaign_store(BackendCapability::CampaignLifecycle, "delete character")?;
            for source_id in extra_source_ids {
                let _ = campaign_store.delete_mvu(source_id);
                if let Some(stored_card) = campaign_store.get_card_by_source(source_id) {
                    let _ = campaign_store.delete_card(&stored_card.card.id);
                }
            }
            if let Some(stored) = character_store.get(id)
                && let Some(source_id) = stored.info.source_character_id.as_ref()
            {
                let source_id = Id::from_str(source_id);
                let _ = campaign_store.delete_mvu(&source_id);
                if let Some(stored_card) = campaign_store.get_card_by_source(&source_id) {
                    let _ = campaign_store.delete_card(&stored_card.card.id);
                }
            }
            Ok(removed)
        }
    }

    // ─── Gate 4 P1-4: ImportExport (backend-neutral facade) ───────────────

    /// Get one card wrapper payload as a `StoredCard`.
    pub fn get_card(&self, card_id: &Id) -> Result<Option<StoredCard>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_card_payload(card_id)?
                .map(|payload| {
                    serde_json::from_value(payload)
                        .map_err(|e| format!("解析角色卡 payload 失败: {e}"))
                })
                .transpose()
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::ImportExport, "get card")?
                .get_card(card_id))
        }
    }

    /// Export the full Campaign bundle JSON. Both backends produce the exact
    /// same `CampaignBundle` structure (format_version 2).
    pub fn export_campaign_bundle(&self, camp_id: &Id) -> Result<String, TauriCommandError> {
        if self.is_sqlite() {
            self.export_campaign_bundle_from_sqlite(camp_id)
        } else {
            let store = self
                .json_campaign_store(BackendCapability::ImportExport, "export campaign bundle")?;
            crate::commands::import_export::export_campaign_bundle_from_store(
                store,
                camp_id.clone(),
            )
        }
    }

    /// Import a Campaign bundle. JSON path keeps its snapshot-verified
    /// rollback; SQLite path lands everything in one transaction.
    pub fn import_campaign_bundle(
        &self,
        bundle: crate::commands::import_export::CampaignBundle,
        conv_store: &ConversationStore,
    ) -> Result<crate::commands::import_export::CampaignImportResult, TauriCommandError> {
        if self.is_sqlite() {
            crate::commands::import_export::import_campaign_bundle_into_sqlite(conv_store, bundle)
        } else {
            let store = self
                .json_campaign_store(BackendCapability::ImportExport, "import campaign bundle")
                .map_err(TauriCommandError::validation)?;
            crate::commands::import_export::import_campaign_bundle_into_store(
                store, conv_store, bundle,
            )
        }
    }

    fn export_campaign_bundle_from_sqlite(
        &self,
        camp_id: &Id,
    ) -> Result<String, TauriCommandError> {
        use crate::commands::import_export::{BUNDLE_FORMAT_VERSION, CampaignBundle};

        let campaign = sqlite_runtime::get_campaign(camp_id)
            .map_err(TauriCommandError::storage)?
            .ok_or_else(|| {
                TauriCommandError::not_found(format!("Campaign 不存在: {}", camp_id.as_str()))
            })?;
        let stored_card = match sqlite_runtime::get_card_payload(&campaign.card_id)
            .map_err(TauriCommandError::storage)?
        {
            Some(payload) => Some(serde_json::from_value::<StoredCard>(payload).map_err(|e| {
                TauriCommandError::storage(format!("角色卡 payload 解析失败: {e}"))
            })?),
            None => None,
        };
        let instances =
            sqlite_runtime::list_instances(camp_id).map_err(TauriCommandError::storage)?;
        let definitions = stored_card
            .as_ref()
            .map(|c| c.card.character_definitions.clone())
            .unwrap_or_default();
        let knowledge =
            sqlite_runtime::list_knowledge(camp_id).map_err(TauriCommandError::storage)?;
        let tasks = sqlite_runtime::list_tasks(camp_id).map_err(TauriCommandError::storage)?;
        let summaries =
            sqlite_runtime::list_summaries(camp_id).map_err(TauriCommandError::storage)?;

        let bundle = CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: stored_card.map(|c| c.card),
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

    // ─── Gate 4: campaign world info (backend-neutral facade) ─────────────

    /// Read the campaign world info book. Missing data yields an empty book on
    /// both backends (JSON treats a missing file as empty; SQLite a missing row).
    pub fn get_world_info(
        &self,
        campaign_id: &Id,
    ) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
        if self.is_sqlite() {
            Ok(sqlite_runtime::get_world_info(campaign_id)?.unwrap_or_else(empty_world_info_book))
        } else {
            self.json_campaign_store(BackendCapability::WorldInfo, "get campaign world info")?
                .get_world_info(campaign_id)
                .map_err(|e| e.to_string())
        }
    }

    pub fn set_world_info(
        &self,
        campaign_id: &Id,
        book: &storyforge_domain::world_info::WorldInfoBook,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::set_world_info(campaign_id, book)
        } else {
            self.json_campaign_store(BackendCapability::WorldInfo, "set campaign world info")?
                .set_world_info(campaign_id, book.clone())
        }
    }

    /// Seed the campaign book from a card template when it is still empty.
    pub fn ensure_world_info_from_book(
        &self,
        campaign_id: &Id,
        template: &storyforge_domain::world_info::WorldInfoBook,
    ) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
        if self.is_sqlite() {
            sqlite_runtime::ensure_world_info_from_book(campaign_id, template)
        } else {
            self.json_campaign_store(BackendCapability::WorldInfo, "ensure campaign world info")?
                .ensure_world_info_from_book(campaign_id, template)
        }
    }

    pub fn add_world_info_entry(
        &self,
        campaign_id: &Id,
        entry: storyforge_domain::world_info::WorldInfoEntry,
    ) -> Result<usize, String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                let mut entry = entry.clone();
                if entry.extensions.is_null() {
                    entry.extensions = serde_json::json!({ "sf_source": "user" });
                } else if let Some(obj) = entry.extensions.as_object_mut() {
                    obj.entry("sf_source")
                        .or_insert_with(|| serde_json::json!("user"));
                }
                book.entries.push(entry);
                Ok(book.entries.len() - 1)
            })
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "add campaign world info entry",
            )?
            .add_world_info_entry(campaign_id, entry)
        }
    }

    pub fn update_world_info_entry(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        entry: storyforge_domain::world_info::WorldInfoEntry,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                if entry_index >= book.entries.len() {
                    return Err(format!("世界书条目索引越界: {entry_index}"));
                }
                book.entries[entry_index] = entry;
                Ok(())
            })?;
            Ok(())
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "update campaign world info entry",
            )?
            .update_world_info_entry(campaign_id, entry_index, entry)
        }
    }

    pub fn delete_world_info_entry(
        &self,
        campaign_id: &Id,
        entry_index: usize,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                if entry_index >= book.entries.len() {
                    return Err(format!("世界书条目索引越界: {entry_index}"));
                }
                book.entries.remove(entry_index);
                Ok(())
            })?;
            Ok(())
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "delete campaign world info entry",
            )?
            .delete_world_info_entry(campaign_id, entry_index)
        }
    }

    pub fn set_world_info_route(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        route: storyforge_domain::world_info::LoreRoute,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                if entry_index >= book.entries.len() {
                    return Err(format!("世界书条目索引越界: {entry_index}"));
                }
                book.entries[entry_index].route = route;
                book.entries[entry_index].disabled = matches!(
                    book.entries[entry_index].route,
                    storyforge_domain::world_info::LoreRoute::Disabled
                );
                Ok(())
            })?;
            Ok(())
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "set campaign world info route",
            )?
            .set_world_info_route(campaign_id, entry_index, route)
        }
    }

    pub fn set_world_info_entry_enabled(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        enabled: bool,
    ) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
        if self.is_sqlite() {
            let book = sqlite_runtime::mutate_world_info(campaign_id, |book| {
                let entry = book
                    .entries
                    .get_mut(entry_index)
                    .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;
                entry.set_enabled(enabled)?;
                Ok(book.clone())
            })?;
            Ok(book)
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "set campaign world info enabled",
            )?
            .set_world_info_entry_enabled(campaign_id, entry_index, enabled)
        }
    }

    /// Template world info resolvable from the SQLite card authority
    /// (`raw_card_json.character_book`), used when a campaign book is empty.
    pub fn template_world_info_from_card(
        &self,
        card_payload: &serde_json::Value,
    ) -> Result<Option<storyforge_domain::world_info::WorldInfoBook>, String> {
        if !self.is_sqlite() {
            return Ok(None);
        }
        let stored: crate::campaign_store::StoredCard =
            serde_json::from_value(card_payload.clone())
                .map_err(|e| format!("invalid SQLite card payload: {e}"))?;
        let template = stored
            .card
            .raw_card_json
            .get("character_book")
            .and_then(|value| {
                serde_json::from_value::<storyforge_domain::character::StWorldInfoBook>(
                    value.clone(),
                )
                .ok()
            })
            .map(storyforge_domain::world_info::WorldInfoBook::from_st);
        Ok(template)
    }
}

fn empty_world_info_book() -> storyforge_domain::world_info::WorldInfoBook {
    storyforge_domain::world_info::WorldInfoBook {
        entries: Vec::new(),
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}

/// Resolve and pin the storage backend for this process.
///
/// SQLite is the default (Gate 7). Running the SQLite branch runs the cutover
/// (or verifies it has already completed); a directory with no legacy JSON
/// layout at all is treated as a fresh user and initialized as an empty SQLite
/// authority. JSON is selected only by an explicit override (`env=json`, a
/// `JsonAuthoritative` marker, or a settings config value); those paths never
/// open a database and never run a cutover.
///
/// This function is safe to call multiple times — the first call pins the
/// backend, and subsequent calls return the cached resolution.
pub fn resolve_backend(data_dir: &Path) -> Result<BackendResolution, BackendWiringError> {
    // If already pinned, return cached resolution.
    if let Some(pinned) = PINNED.get() {
        let schema_version = pinned
            .is_sqlite()
            .then(|| current_version_sqlite(&data_dir.join(SQLITE_DB_FILENAME)));
        return Ok(BackendResolution {
            pinned: pinned.clone(),
            db_path: pinned
                .is_sqlite()
                .then(|| data_dir.join(SQLITE_DB_FILENAME)),
            diagnostics: BackendDiagnostics::from_pinned(pinned, schema_version.flatten()),
            cutover_performed: false,
        });
    }

    let resolution = resolve_backend_inner(data_dir)?;
    let _ = PINNED.set(resolution.pinned.clone());
    Ok(resolution)
}

/// Inner resolution logic without the OnceLock — testable in isolation.
///
/// Marker-first（审查一.1）：**先**检查后端 marker，再结合 env 解析：
/// - 有效 sqlite marker → SQLite 权威（env=json 必须 fail closed，绝不回退 JSON；
///   env=sqlite / 无 env → SQLite）；
/// - JsonAuthoritative marker（官方 reverse-cutover 写入）→ JSON 权威；
///   env=sqlite 是合法全新 opt-in，可重跑 cutover（forward 路径）；
/// - Stale marker → 无论 env 是什么都拒绝（JSON 与 SQLite 都不放行）；
/// - 无 marker → **默认 SQLite**（Gate 7）：env=json 显式回退走 JSON，
///   否则走 cutover / 全新用户初始化。
///
/// JSON 与 SQLite 分支在决议成功后都持有进程级 SHARED authority 租约
/// （`storyforge.authority.lock`），把并发 cutover 挡在门外（审查一.3）。
fn resolve_backend_inner(data_dir: &Path) -> Result<BackendResolution, BackendWiringError> {
    let selection = BackendSelection::from_env(None);
    let db_path = data_dir.join(SQLITE_DB_FILENAME);
    let plan = CutoverPlan::new(data_dir, &db_path);

    // 解析 env（未知值直接拒绝），但**不**用它拍板——marker 优先。
    let env_backend = selection
        .resolve()
        .map_err(|e| BackendWiringError::Selection(format!("{e}")))?;
    let pinned = PinnedBackend::resolve(&selection)
        .map_err(|e| BackendWiringError::Selection(format!("{e}")))?;
    let pinned_source = pinned.source();

    let run_sqlite = || -> Result<BackendResolution, BackendWiringError> {
        // 运行 cutover（或验证已完成；全新目录 → 空 SQLite 权威初始化）。
        let request = CutoverRequest {
            plan: plan.clone(),
            label: "app-startup".to_string(),
        };
        let outcome =
            recover_or_verify(&request).map_err(|e| BackendWiringError::Cutover(format!("{e}")))?;

        let cutover_performed = matches!(outcome, CutoverOutcome::Completed(_));

        let schema_version = {
            let db = storyforge_infra_sqlite::Database::open(&db_path)
                .map_err(|e| BackendWiringError::Cutover(format!("reopen: {e}")))?;
            current_version(&db).unwrap_or(0)
        };

        // SQLite 权威决议成功：持有进程级 SHARED 租约，阻挡并发 cutover。
        hold_process_shared_lease(data_dir)
            .map_err(|e| BackendWiringError::Lease(format!("{e}")))?;

        let sqlite_pinned = PinnedBackend::new(StorageBackend::Sqlite, pinned_source);
        let diag = BackendDiagnostics::from_pinned(&sqlite_pinned, Some(schema_version));
        Ok(BackendResolution {
            pinned: sqlite_pinned,
            db_path: Some(db_path),
            diagnostics: diag,
            cutover_performed,
        })
    };

    let run_json = || -> Result<BackendResolution, BackendWiringError> {
        // JSON 权威决议成功：持有进程级 SHARED 租约（普通 JSON 写者进程）。
        // PinnedBackend::resolve 的默认已翻转为 Sqlite，这里必须显式构造
        // Json pinned（source 保持 env/default，如实反映选择来源）。
        hold_process_shared_lease(data_dir)
            .map_err(|e| BackendWiringError::Lease(format!("{e}")))?;
        let json_pinned = PinnedBackend::new(StorageBackend::Json, pinned_source);
        let diag = BackendDiagnostics::from_pinned(&json_pinned, None);
        Ok(BackendResolution {
            pinned: json_pinned,
            db_path: None,
            diagnostics: diag,
            cutover_performed: false,
        })
    };

    match inspect_marker(&plan) {
        MarkerStatus::SqliteAuthoritative { .. } => {
            // 有效 sqlite marker 在握：env 只能允许 sqlite / 缺省。
            if env_backend == StorageBackend::Json && selection.is_explicit() {
                return Err(BackendWiringError::Selection(
                    "storage backend marker claims SQLite authority, but \
                     STORYFORGE_STORAGE_BACKEND=json was set; refusing to fall back \
                     to JSON (delete the marker to reset authority)"
                        .to_string(),
                ));
            }
            run_sqlite()
        }
        MarkerStatus::JsonAuthoritative => {
            // JSON 权威（reverse-cutover 产物）：env=sqlite 是合法全新 opt-in。
            if env_backend == StorageBackend::Sqlite && selection.is_explicit() {
                run_sqlite()
            } else {
                run_json()
            }
        }
        MarkerStatus::Absent => {
            // Gate 7 默认：无 marker 时 SQLite 是默认后端（旧 JSON 自动迁移 /
            // 全新用户初始化）；env=json 是唯一显式回退 JSON 的通道。
            if env_backend == StorageBackend::Json && selection.is_explicit() {
                run_json()
            } else {
                run_sqlite()
            }
        }
        MarkerStatus::Stale { reason } => {
            // 无论 env 是什么都拒绝：JSON 与 SQLite 都不放行。
            Err(BackendWiringError::Selection(format!(
                "stale backend marker; refusing to start until resolved: {reason}"
            )))
        }
    }
}

/// Check the current marker status without performing a cutover.
pub fn check_marker_status(data_dir: &Path) -> MarkerStatus {
    let plan = CutoverPlan::new(data_dir, data_dir.join(SQLITE_DB_FILENAME));
    inspect_marker(&plan)
}

/// The SQLite database path for the given data directory, regardless of
/// whether SQLite is currently selected.
pub fn sqlite_db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SQLITE_DB_FILENAME)
}

fn current_version_sqlite(db_path: &Path) -> Option<i64> {
    if !db_path.exists() {
        return None;
    }
    let db = storyforge_infra_sqlite::Database::open(db_path).ok()?;
    current_version(&db).ok()
}

/// Persist the JSON active-campaign pointer file (legacy bootstrap sidecar).
/// SQLite keeps the pointer in-process and never writes this file.
fn save_active_campaign(data_dir: &Path, id: Option<&Id>) -> Result<(), String> {
    let path = data_dir.join("active_campaign.json");
    let v = serde_json::json!({ "campaign_id": id.map(|i| i.as_str()).unwrap_or("") });
    storyforge_infra_util::atomic_write_json(&path, &v)
        .map_err(|error| format!("保存活跃 Campaign 失败: {error}"))
}

/// Errors produced during backend wiring.
#[derive(Debug, thiserror::Error)]
pub enum BackendWiringError {
    #[error("backend selection error: {0}")]
    Selection(String),
    #[error("cutover error: {0}")]
    Cutover(String),
    #[error("authority lease error: {0}")]
    Lease(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_json(path: &Path, value: &serde_json::Value) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn sample_source(dir: &Path) {
        write_json(
            &dir.join("cards.json"),
            &serde_json::json!([{
                "id": "card-1", "name": "Hero", "source_character_id": null
            }]),
        );
        write_json(
            &dir.join("campaigns.json"),
            &serde_json::json!([{
                "id": "camp-1", "card_id": "card-1", "name": "Main",
                "created_at": "2026-07-13T00:00:00Z", "revision": 0,
                "chronicle_revision": 0, "conversation_id": "conv-1", "lineage_id": "lin-1"
            }]),
        );
        write_json(
            &dir.join("conversations").join("conv-1.json"),
            &serde_json::json!({
                "id": "conv-1", "campaign_id": "camp-1", "character_id": null,
                "created_at": "2026-07-13T00:00:00Z", "updated_at": "2026-07-13T00:00:00Z",
                "nodes": []
            }),
        );
        write_json(&dir.join("instances.json"), &serde_json::json!([]));
        write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
        write_json(&dir.join("tasks.json"), &serde_json::json!([]));
        write_json(
            &dir.join("round_summaries.json"),
            &serde_json::json!([{
                "id": "sum-a1", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "turn": 1, "content": "leaf", "created_at": "2026-07-13T00:00:00Z",
                "level": 0, "lineage_id": "lin-1", "code": "A0001"
            }]),
        );
        write_json(&dir.join("turns.json"), &serde_json::json!([]));
    }

    #[test]
    fn default_resolution_is_sqlite_fresh_start_without_database_source() {
        // Gate 7：无 env 无 marker 且目录里没有任何 legacy JSON 布局 =
        // 全新用户 → 直接初始化空 SQLite 权威（不导入、不失败）。
        let dir = TempDir::new().unwrap();
        // Ensure env var is not set.
        // SAFETY: test-only; no concurrent threads depend on this env var.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(resolution.is_sqlite());
        assert!(resolution.db_path.is_some());
        assert!(resolution.cutover_performed);
        // Fresh-start created the empty authority DB + marker.
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());
        assert!(dir.path().join("storyforge.backend.json").exists());
    }

    #[test]
    fn default_resolution_migrates_legacy_json_automatically() {
        // Gate 7：无 env + 完整 legacy JSON 树 → 自动 cutover，JSON 原样保留。
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(resolution.is_sqlite());
        assert!(resolution.cutover_performed);
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());
        assert!(dir.path().join("storyforge.backend.json").exists());
        // 旧 JSON 不被删除（§12.2：不因默认化删除用户旧 JSON）。
        assert!(dir.path().join("cards.json").exists());
        assert!(dir.path().join("campaigns.json").exists());
    }

    #[test]
    fn sqlite_resolution_runs_cutover() {
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        // SAFETY: test-only.
        unsafe {
            std::env::set_var("STORYFORGE_STORAGE_BACKEND", "sqlite");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(resolution.is_sqlite());
        assert!(resolution.cutover_performed);
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());

        // Cleanup env for other tests.
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
    }

    #[test]
    fn marker_status_absent_when_no_marker() {
        let dir = TempDir::new().unwrap();
        let status = check_marker_status(dir.path());
        assert_eq!(status, MarkerStatus::Absent);
    }

    #[test]
    fn sqlite_default_does_not_construct_json_writers() {
        // Gate 7 默认决议（无 env）→ SQLite：数据库 + marker 被创建，JSON
        // 文件不被触碰（不双写、不删除）。
        let dir = TempDir::new().unwrap();
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert_eq!(resolution.pinned.backend(), StorageBackend::Sqlite);
        assert!(dir.path().join("storyforge.backend.json").exists());
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());
        // 显式 JSON 回退（唯一回退通道）不创建任何数据库/marker。
        unsafe {
            std::env::set_var("STORYFORGE_STORAGE_BACKEND", "json");
        }
        let json_dir = TempDir::new().unwrap();
        let json_resolution = resolve_backend_inner(json_dir.path()).unwrap();
        assert_eq!(json_resolution.pinned.backend(), StorageBackend::Json);
        assert!(!json_dir.path().join("storyforge.backend.json").exists());
        assert!(!json_dir.path().join(SQLITE_DB_FILENAME).exists());
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
    }

    #[test]
    fn facade_pins_backend_data_dir_and_sqlite_capabilities() {
        let dir = TempDir::new().unwrap();
        let facade = StorageFacade::new(
            dir.path().to_path_buf(),
            PinnedBackend::new(
                StorageBackend::Sqlite,
                storyforge_infra_sqlite::backend::BackendSource::Env,
            ),
        );

        assert_eq!(facade.backend(), StorageBackend::Sqlite);
        assert_eq!(facade.data_dir(), dir.path());
        assert!(!facade.has_json_writers());
        assert_eq!(
            facade.capability(BackendCapability::CampaignRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CampaignInstanceRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::KnowledgeTaskRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::VariableRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CardCommands),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CharacterCommands),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::ImportExport),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CampaignHealth),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::VariableCommands),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::KnowledgeTaskCommands),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CampaignLifecycle),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::WorldInfo),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::TypedMetaPatch),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::ChronicleCompressor),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::StoryClock),
            CapabilityStatus::Supported
        );
        facade
            .require_supported(
                BackendCapability::ChronicleCompressor,
                "chronicle compression",
            )
            .expect("SQLite chronicle compressor must be supported");
        assert_eq!(
            facade.capability(BackendCapability::ActiveCampaignPersistence),
            CapabilityStatus::Degraded
        );

        let supported = [
            BackendCapability::CampaignRead,
            BackendCapability::CampaignInstanceRead,
            BackendCapability::CampaignHealth,
            BackendCapability::ConversationRead,
            BackendCapability::TurnLifecycle,
            BackendCapability::Postprocess,
            BackendCapability::KnowledgeTaskRead,
            BackendCapability::VariableRead,
            BackendCapability::MvuTranslation,
            BackendCapability::ChroniclePublication,
            BackendCapability::WorldInfo,
            BackendCapability::TypedMetaPatch,
            BackendCapability::MvuSchemaApply,
            BackendCapability::ChronicleCompressor,
            BackendCapability::StoryClock,
            BackendCapability::CharacterCommands,
            BackendCapability::ImportExport,
            // Gate 5：四族 create/update/delete 补齐为 SQLite-native。
            BackendCapability::CampaignLifecycle,
            BackendCapability::CardCommands,
            BackendCapability::KnowledgeTaskCommands,
            BackendCapability::VariableCommands,
        ];
        let degraded = [BackendCapability::ActiveCampaignPersistence];
        let unsupported: [BackendCapability; 0] = [];
        assert_eq!(
            supported.len() + degraded.len() + unsupported.len(),
            BackendCapability::ALL.len()
        );
        for capability in supported {
            assert_eq!(facade.capability(capability), CapabilityStatus::Supported);
        }
        for capability in degraded {
            assert_eq!(facade.capability(capability), CapabilityStatus::Degraded);
        }
        for capability in unsupported {
            assert_eq!(facade.capability(capability), CapabilityStatus::Unsupported);
        }

        let character_error = match facade.json_character_store(
            BackendCapability::CharacterCommands,
            "read SQLite character",
        ) {
            Ok(_) => panic!("SQLite facade must not expose a legacy CharacterStore"),
            Err(error) => error,
        };
        assert!(
            character_error.contains("legacy CharacterStore"),
            "SQLite facade must refuse the legacy JSON CharacterStore, got: {character_error}"
        );
        assert!(!dir.path().join("characters.json").exists());
    }

    #[test]
    fn json_facade_reports_current_capabilities_as_supported() {
        let dir = TempDir::new().unwrap();
        let facade = StorageFacade::new(
            dir.path().to_path_buf(),
            PinnedBackend::new(
                StorageBackend::Json,
                storyforge_infra_sqlite::backend::BackendSource::Default,
            ),
        );

        for capability in BackendCapability::ALL {
            assert_eq!(
                facade.capability(capability),
                CapabilityStatus::Supported,
                "JSON capability {capability:?} must remain available"
            );
        }
        assert!(facade.has_json_writers());
    }

    #[test]
    fn app_state_owns_the_explicitly_injected_facade() {
        let dir = TempDir::new().unwrap();
        let facade = std::sync::Arc::new(StorageFacade::new(
            dir.path().to_path_buf(),
            PinnedBackend::new(
                StorageBackend::Json,
                storyforge_infra_sqlite::backend::BackendSource::Default,
            ),
        ));

        let state = crate::AppState::new_with_backend(dir.path().to_path_buf(), facade.clone())
            .expect("matching JSON facade constructs AppState");

        assert!(std::sync::Arc::ptr_eq(state.storage(), &facade));
        assert_eq!(state.storage().backend(), StorageBackend::Json);
        assert_eq!(state.storage().data_dir(), dir.path());
    }
    // ─── Gate 5 审查一.1：marker 优先于环境变量与默认 JSON ────────────────

    /// 跑一次真实 cutover，得到有效 sqlite marker + DB（source 完备）。
    /// Wave-1 决议区测试辅助；当前无调用者（clippy -D warnings 死代码）。
    #[allow(dead_code)]
    fn cut_over(dir: &Path) {
        let plan =
            storyforge_infra_sqlite::cutover::CutoverPlan::new(dir, dir.join(SQLITE_DB_FILENAME));
        let request = storyforge_infra_sqlite::cutover::CutoverRequest {
            plan,
            label: "marker-first".into(),
        };
        match storyforge_infra_sqlite::cutover::run_cutover(&request).unwrap() {
            storyforge_infra_sqlite::cutover::CutoverOutcome::Completed(_) => {}
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    fn set_env_json() {
        // SAFETY: test-only; no concurrent threads depend on this env var.
        unsafe {
            std::env::set_var(DEFAULT_BACKEND_ENV_VAR, "json");
        }
    }

    fn set_env_sqlite() {
        // SAFETY: test-only.
        unsafe {
            std::env::set_var(DEFAULT_BACKEND_ENV_VAR, "sqlite");
        }
    }

    fn clear_env() {
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var(DEFAULT_BACKEND_ENV_VAR);
        }
    }

    #[test]
    fn valid_sqlite_marker_wins_without_env() {
        // 审查核心 bug：重启后无 env 时，默认后端不得重新启用——
        // 有效 sqlite marker 存在时必须以 SQLite 权威启动（Gate 7 后默认
        // 即 SQLite，本测试继续钉住「marker 优先于任何隐式默认」）。
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        set_env_sqlite();
        let first = resolve_backend_inner(dir.path()).unwrap();
        assert!(first.is_sqlite(), "setup cutover must pin sqlite");
        clear_env();

        // marker 存在、无 env → 仍须 SQLite。
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(
            resolution.is_sqlite(),
            "valid sqlite marker must beat any default, got {:?}",
            resolution.pinned.backend()
        );
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());
    }

    #[test]
    fn env_json_with_valid_sqlite_marker_fails_closed() {
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        set_env_sqlite();
        let _ = resolve_backend_inner(dir.path()).unwrap();

        // env=json 不允许把已有 sqlite 权威悄悄退回 JSON。
        set_env_json();
        let err = resolve_backend_inner(dir.path()).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("marker"),
            "env=json + sqlite marker must fail closed mentioning the marker, got: {err}"
        );
        clear_env();
    }

    #[test]
    fn stale_marker_refused_for_both_env_values() {
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        // 版本过新的 marker（inspect_marker 判 Stale）。
        let marker = serde_json::json!({
            "version": 99,
            "backend": "sqlite",
            "schema_version": 1,
            "manifest_hash": "x",
            "created_at": "2026-07-13T00:00:00Z"
        });
        std::fs::write(
            dir.path().join("storyforge.backend.json"),
            serde_json::to_vec_pretty(&marker).unwrap(),
        )
        .unwrap();

        // env=json：不得静默回退 JSON。
        set_env_json();
        let err_json = resolve_backend_inner(dir.path()).unwrap_err();
        assert!(
            err_json.to_string().to_lowercase().contains("stale"),
            "stale marker + env=json must fail closed, got: {err_json}"
        );
        clear_env();

        // env=sqlite：同样拒绝（不得重跑 cutover）。
        set_env_sqlite();
        let err_sqlite = resolve_backend_inner(dir.path()).unwrap_err();
        assert!(
            err_sqlite.to_string().to_lowercase().contains("stale"),
            "stale marker + env=sqlite must fail closed, got: {err_sqlite}"
        );
        clear_env();
    }

    #[test]
    fn json_authoritative_marker_allows_json_and_fresh_sqlite_optin() {
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        // JsonAuthoritative marker（官方 reverse-cutover 才会写）。
        let marker = serde_json::json!({
            "version": 1,
            "backend": "json",
            "schema_version": 0,
            "manifest_hash": "",
            "created_at": "2026-07-13T00:00:00Z"
        });
        std::fs::write(
            dir.path().join("storyforge.backend.json"),
            serde_json::to_vec_pretty(&marker).unwrap(),
        )
        .unwrap();

        clear_env();
        let json_default = resolve_backend_inner(dir.path()).unwrap();
        assert!(!json_default.is_sqlite(), "JSON marker + no env → JSON");

        set_env_json();
        let json_explicit = resolve_backend_inner(dir.path()).unwrap();
        assert!(!json_explicit.is_sqlite(), "JSON marker + env=json → JSON");
        clear_env();

        // env=sqlite：合法的全新 opt-in，可重跑 cutover。
        set_env_sqlite();
        let sqlite = resolve_backend_inner(dir.path()).unwrap();
        assert!(
            sqlite.is_sqlite(),
            "JSON marker + env=sqlite is a legitimate fresh opt-in"
        );
        clear_env();
    }

    #[test]
    fn empty_dir_with_explicit_sqlite_is_fresh_start() {
        // Gate 7 语义变更：空目录（无任何 legacy 布局）+ env=sqlite 与默认
        // 一致 → 全新用户初始化，不再 fail-closed（旧行为是防「误建空库」，
        // 现由「部分布局 fail-closed」承接该保护）。
        let dir = TempDir::new().unwrap();
        set_env_sqlite();
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(resolution.is_sqlite());
        assert!(resolution.cutover_performed);
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());
        clear_env();
    }

    #[test]
    fn partial_legacy_layout_fails_closed_instead_of_fresh_start() {
        // 有数据但坏了（只存在部分核心文件）绝不当作新用户静默建空库：
        // 必须 fail-closed 并给出可操作错误（§12.2「不遇错静默创建空数据库」）。
        let dir = TempDir::new().unwrap();
        write_json(&dir.path().join("cards.json"), &serde_json::json!([]));
        clear_env();
        let err = resolve_backend_inner(dir.path()).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("source")
                || err.to_string().to_lowercase().contains("missing")
                || err.to_string().to_lowercase().contains("import"),
            "partial legacy layout must fail closed, got: {err}"
        );
        // 不得留下任何半成品权威。
        assert!(!dir.path().join(SQLITE_DB_FILENAME).exists());
        assert!(!dir.path().join("storyforge.backend.json").exists());
    }

    #[test]
    fn explicit_json_fallback_touches_no_sqlite_with_legacy_source() {
        // §12.3：显式回退（env=json）在存在 legacy JSON 数据时直接以 JSON
        // 权威启动，不创建数据库/marker、不改动 JSON（无数据倒退）。
        // 注意：JSON→SQLite 的后续切换是跨进程场景（JSON 进程持 SHARED
        // 租约，进程内 cutover 必须 fail-closed——由 lease 设计保证）。
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        set_env_json();
        let json_resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(!json_resolution.is_sqlite());
        assert!(!dir.path().join(SQLITE_DB_FILENAME).exists());
        assert!(!dir.path().join("storyforge.backend.json").exists());
        assert!(dir.path().join("cards.json").exists());
        assert_eq!(
            json_resolution.pinned.source(),
            storyforge_infra_sqlite::backend::BackendSource::Env
        );
        clear_env();
    }
}

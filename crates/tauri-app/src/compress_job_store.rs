//! ChronicleCompressor 持久化任务队列（崩溃可恢复）
//!
//! 文件：`data/compress_jobs.json`
//! Accept 达阈值时入队；后台 worker / 启动恢复消费 Pending|Running。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use storyforge_domain::Id;

use crate::campaign_store::persist;
use crate::storage::json_store::load_json_with_tmp_backup_or_default;

/// 默认最大重试次数（含首次）。
pub const DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressJobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

impl CompressJobStatus {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}

/// 压缩任务意图：自动尝试 A→B，必要时再 B→C。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompressJobKind {
    #[default]
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressJob {
    pub id: Id,
    pub campaign_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<Id>,
    #[serde(default)]
    pub kind: CompressJobKind,
    pub status: CompressJobStatus,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// 入队时 uncovered A 计数（可观测）
    #[serde(default)]
    pub uncovered_a_at_enqueue: u32,
    #[serde(default)]
    pub uncovered_b_at_enqueue: u32,
    pub created_at: String,
    pub updated_at: String,
}

fn default_max_attempts() -> u32 {
    DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl CompressJob {
    pub fn new(
        campaign_id: Id,
        conversation_id: Option<Id>,
        lineage_id: Option<Id>,
        uncovered_a: u32,
        uncovered_b: u32,
    ) -> Self {
        let ts = now_iso();
        Self {
            id: Id::new(),
            campaign_id,
            conversation_id,
            lineage_id,
            kind: CompressJobKind::Auto,
            status: CompressJobStatus::Pending,
            attempts: 0,
            max_attempts: DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS,
            last_error: None,
            uncovered_a_at_enqueue: uncovered_a,
            uncovered_b_at_enqueue: uncovered_b,
            created_at: ts.clone(),
            updated_at: ts,
        }
    }
}

pub struct CompressJobStore {
    path: PathBuf,
    jobs: Mutex<Vec<CompressJob>>,
}

impl CompressJobStore {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("compress_jobs.json");
        let jobs = load_json_with_tmp_backup_or_default(
            &path,
            |e| {
                tracing::warn!(target: "chronicle_compressor", "compress_jobs.json parse error: {e}");
            },
            |p, e| {
                tracing::error!(
                    target: "chronicle_compressor",
                    "compress_jobs recovery failed for {}: {e}",
                    p.display()
                );
            },
        );
        Self {
            path,
            jobs: Mutex::new(jobs),
        }
    }

    pub fn list_all(&self) -> Vec<CompressJob> {
        self.jobs.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn list_open(&self) -> Vec<CompressJob> {
        self.jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|j| j.status.is_open())
            .cloned()
            .collect()
    }

    /// 启动时：Running → Pending（崩溃中断），便于重放。
    ///
    /// 三.9 原子性：候选 → 持久化 → 换入。持久化失败时内存保持 Running、
    /// 返回 0（计入的是**落盘成功**的 reset 数）——绝不在 persist 失败后
    /// 静默把内存改成 Pending（旧实现 `let _ = persist` 正是三.9 审查点）。
    pub fn reset_running_to_pending(&self) -> usize {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let mut candidate = jobs.clone();
        let mut n = 0;
        for j in candidate.iter_mut() {
            if j.status == CompressJobStatus::Running {
                j.status = CompressJobStatus::Pending;
                j.updated_at = now_iso();
                n += 1;
            }
        }
        if n > 0 {
            if let Err(error) = persist(&self.path, &candidate) {
                tracing::error!(
                    target: "chronicle_compressor",
                    "reset_running_to_pending persist failed; in-memory jobs kept Running: {error}"
                );
                return 0;
            }
            *jobs = candidate;
        }
        n
    }

    /// 若该 campaign 已有 open job 则返回已有；否则新建 Pending。
    ///
    /// 三.9 原子性：候选副本上 push → persist 成功 → 换入；写盘失败时内存
    /// 不残留未持久化的 job（旧实现先 push 再 persist，失败后内存已脏）。
    pub fn enqueue_or_get_open(
        &self,
        campaign_id: &Id,
        conversation_id: Option<Id>,
        lineage_id: Option<Id>,
        uncovered_a: u32,
        uncovered_b: u32,
    ) -> Result<(CompressJob, bool), String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = jobs
            .iter()
            .find(|j| &j.campaign_id == campaign_id && j.status.is_open())
        {
            return Ok((existing.clone(), false));
        }
        let job = CompressJob::new(
            campaign_id.clone(),
            conversation_id,
            lineage_id,
            uncovered_a,
            uncovered_b,
        );
        let mut candidate = jobs.clone();
        candidate.push(job.clone());
        persist(&self.path, &candidate)?;
        *jobs = candidate;
        Ok((job, true))
    }

    /// 原子领取：仅 `Pending → Running` 成功。已 Running/终态返回 Ok(false)。
    ///
    /// 防止同一 open job 被多个 worker 重复消费。
    ///
    /// 三.9 原子性：候选 → 持久化 → 换入；写盘失败时内存保持 Pending。
    pub fn try_claim_pending(&self, job_id: &Id) -> Result<bool, String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let Some(j) = jobs.iter().find(|j| &j.id == job_id) else {
            return Err(format!("compress job {job_id} not found"));
        };
        if j.status != CompressJobStatus::Pending {
            return Ok(false);
        }
        let mut candidate = jobs.clone();
        let j = candidate
            .iter_mut()
            .find(|j| &j.id == job_id)
            .expect("candidate mirrors jobs");
        j.status = CompressJobStatus::Running;
        j.attempts = j.attempts.saturating_add(1);
        j.last_error = None;
        j.updated_at = now_iso();
        persist(&self.path, &candidate)?;
        *jobs = candidate;
        Ok(true)
    }

    /// 兼容测试路径：无条件标 Running（会增加 attempts）。生产 worker 请用 `try_claim_pending`。
    #[cfg(test)]
    pub fn mark_running(&self, job_id: &Id) -> Result<(), String> {
        self.update_job(job_id, |j| {
            j.status = CompressJobStatus::Running;
            j.attempts = j.attempts.saturating_add(1);
            j.last_error = None;
        })
    }

    /// 无条件标 Succeeded。仅测试用——生产 worker 必须走
    /// `mark_succeeded_if_running`（经 guarded facade，三.1(a)）。
    #[cfg(test)]
    pub fn mark_succeeded(&self, job_id: &Id) -> Result<(), String> {
        self.update_job(job_id, |j| {
            j.status = CompressJobStatus::Succeeded;
            j.last_error = None;
        })
    }

    /// 无条件失败回队。仅测试用——生产 worker 必须走
    /// `mark_failed_or_retry_if_running`（经 guarded facade，三.1(a)）。
    #[cfg(test)]
    pub fn mark_failed_or_retry(&self, job_id: &Id, err: impl Into<String>) -> Result<(), String> {
        let err = err.into();
        self.update_job(job_id, |j| {
            j.last_error = Some(err.clone());
            if j.attempts >= j.max_attempts {
                j.status = CompressJobStatus::Failed;
            } else {
                j.status = CompressJobStatus::Pending;
            }
        })
    }

    /// 终态化**仅当 job 仍处于 Running**：迟到/并发 worker 不得改写已被其它
    /// worker 终态化的 job。与 SQLite `SqliteCompressJobRepository::transition`
    /// （`WHERE status='running'`）状态机对齐——Gate 5 等价矩阵。
    ///
    /// 返回 `Ok(true)` 表示发生了转换；`Ok(false)` 表示 job 不在 Running
    /// （已被终态化或不存在/状态不匹配），调用方不得再改。
    ///
    /// 三.9 原子性：候选 → 持久化 → 换入；写盘失败时内存保持 Running。
    pub fn mark_succeeded_if_running(&self, job_id: &Id) -> Result<bool, String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let Some(j) = jobs.iter().find(|j| &j.id == job_id) else {
            return Ok(false);
        };
        if j.status != CompressJobStatus::Running {
            return Ok(false);
        }
        let mut candidate = jobs.clone();
        let j = candidate
            .iter_mut()
            .find(|j| &j.id == job_id)
            .expect("candidate mirrors jobs");
        j.status = CompressJobStatus::Succeeded;
        j.last_error = None;
        j.updated_at = now_iso();
        persist(&self.path, &candidate)?;
        *jobs = candidate;
        Ok(true)
    }

    /// 失败回队，**仅当 job 仍处于 Running**（与 SQLite 一致）。迟到结果不得
    /// 把已 Succeeded/Failed 的 job 倒退回 Pending。返回是否发生转换。
    ///
    /// 三.9 原子性：候选 → 持久化 → 换入；写盘失败时内存保持 Running。
    pub fn mark_failed_or_retry_if_running(
        &self,
        job_id: &Id,
        err: impl Into<String>,
    ) -> Result<bool, String> {
        let err = err.into();
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let Some(j) = jobs.iter().find(|j| &j.id == job_id) else {
            return Ok(false);
        };
        if j.status != CompressJobStatus::Running {
            return Ok(false);
        }
        let mut candidate = jobs.clone();
        let j = candidate
            .iter_mut()
            .find(|j| &j.id == job_id)
            .expect("candidate mirrors jobs");
        j.last_error = Some(err);
        if j.attempts >= j.max_attempts {
            j.status = CompressJobStatus::Failed;
        } else {
            j.status = CompressJobStatus::Pending;
        }
        j.updated_at = now_iso();
        persist(&self.path, &candidate)?;
        *jobs = candidate;
        Ok(true)
    }

    #[cfg(test)]
    fn update_job(&self, job_id: &Id, f: impl FnOnce(&mut CompressJob)) -> Result<(), String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let Some(_) = jobs.iter().find(|j| &j.id == job_id) else {
            return Err(format!("compress job {job_id} not found"));
        };
        let mut candidate = jobs.clone();
        let j = candidate
            .iter_mut()
            .find(|j| &j.id == job_id)
            .expect("candidate mirrors jobs");
        f(j);
        j.updated_at = now_iso();
        persist(&self.path, &candidate)?;
        *jobs = candidate;
        Ok(())
    }

    /// 删除某 campaign 的全部压缩任务（delete_card 级联用；Gate 5 三.7）。
    ///
    /// 候选 → 持久化 → 换入：写盘失败时内存保持原值（与其它 mutator 同款，
    /// 三.9 原子性约定）。返回删除条数；无任务时不写盘（幂等 no-op）。
    pub(crate) fn delete_for_campaign(&self, campaign_id: &Id) -> Result<usize, String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let before = jobs.len();
        let mut candidate = jobs.clone();
        candidate.retain(|j| &j.campaign_id != campaign_id);
        let deleted = before - candidate.len();
        if deleted > 0 {
            persist(&self.path, &candidate)?;
            *jobs = candidate;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (PathBuf, CompressJobStore) {
        let dir =
            std::env::temp_dir().join(format!("storyforge-compress-jobs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.clone(), CompressJobStore::new(&dir))
    }

    #[test]
    fn enqueue_dedups_open_job_per_campaign() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (j1, created1) = store
            .enqueue_or_get_open(&camp, None, None, 200, 0)
            .unwrap();
        assert!(created1);
        let (j2, created2) = store
            .enqueue_or_get_open(&camp, None, None, 210, 0)
            .unwrap();
        assert!(!created2);
        assert_eq!(j1.id, j2.id);
        assert_eq!(store.list_open().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn running_resets_to_pending_on_recovery() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store
            .enqueue_or_get_open(&camp, None, None, 200, 0)
            .unwrap();
        store.mark_running(&job.id).unwrap();
        assert_eq!(store.list_all()[0].status, CompressJobStatus::Running);
        assert_eq!(store.reset_running_to_pending(), 1);
        assert_eq!(store.list_all()[0].status, CompressJobStatus::Pending);
        // reload from disk
        let reloaded = CompressJobStore::new(&dir);
        assert_eq!(reloaded.list_open().len(), 1);
        assert_eq!(reloaded.list_all()[0].status, CompressJobStatus::Pending);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_after_max_attempts() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store
            .enqueue_or_get_open(&camp, None, None, 200, 0)
            .unwrap();
        // force low max via mark path: set attempts high by repeated fail
        for _ in 0..DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS {
            store.mark_running(&job.id).unwrap();
            store.mark_failed_or_retry(&job.id, "boom").unwrap();
        }
        let j = store
            .list_all()
            .into_iter()
            .find(|j| j.id == job.id)
            .unwrap();
        assert_eq!(j.status, CompressJobStatus::Failed);
        assert_eq!(j.attempts, DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS);
        assert!(j.last_error.as_deref() == Some("boom"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn succeed_clears_open() {
        let (dir, store) = temp_store();
        let (job, _) = store
            .enqueue_or_get_open(&Id::from_str("c"), None, None, 1, 0)
            .unwrap();
        store.mark_running(&job.id).unwrap();
        store.mark_succeeded(&job.id).unwrap();
        assert!(store.list_open().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn try_claim_pending_only_once() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store
            .enqueue_or_get_open(&camp, None, None, 200, 0)
            .unwrap();
        assert!(store.try_claim_pending(&job.id).unwrap());
        assert!(!store.try_claim_pending(&job.id).unwrap());
        assert_eq!(store.list_all()[0].status, CompressJobStatus::Running);
        assert_eq!(store.list_all()[0].attempts, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn try_claim_rejects_non_pending() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store
            .enqueue_or_get_open(&camp, None, None, 200, 0)
            .unwrap();
        store.mark_succeeded(&job.id).unwrap();
        assert!(!store.try_claim_pending(&job.id).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    // Gate 5 等价（审查跟进 P1）：迟到 worker 不得改写已被其它 worker 终态化的
    // job——与 SQLite `transition`/`mark_failed_or_retry` 的 `WHERE status='running'`
    // 守卫对齐。`mark_succeeded_if_running` / `mark_failed_or_retry_if_running`
    // 必须对非 Running 返回 false 且不改状态。
    #[test]
    fn late_finalize_does_not_overwrite_terminal_job() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store
            .enqueue_or_get_open(&camp, None, None, 200, 0)
            .unwrap();
        // 正常路径：claim → succeed。
        assert!(store.try_claim_pending(&job.id).unwrap());
        assert!(store.mark_succeeded_if_running(&job.id).unwrap());
        // 迟到成功结果：job 已 Succeeded → 不再迁移，状态不变。
        assert!(!store.mark_succeeded_if_running(&job.id).unwrap());
        assert_eq!(
            store
                .list_all()
                .into_iter()
                .find(|j| j.id == job.id)
                .unwrap()
                .status,
            CompressJobStatus::Succeeded
        );
        // 迟到失败结果也不能把 Succeeded 倒退回 Pending/Failed。
        assert!(
            !store
                .mark_failed_or_retry_if_running(&job.id, "late boom")
                .unwrap()
        );

        // 另一 job：claim → fail(retry) → 再 claim → fail(retry) … → failed 终态。
        let (job2, _) = store.enqueue_or_get_open(&camp, None, None, 1, 0).unwrap();
        for _ in 0..DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS {
            assert!(store.try_claim_pending(&job2.id).unwrap());
            assert!(
                store
                    .mark_failed_or_retry_if_running(&job2.id, "boom")
                    .unwrap()
            );
        }
        assert_eq!(
            store
                .list_all()
                .into_iter()
                .find(|j| j.id == job2.id)
                .unwrap()
                .status,
            CompressJobStatus::Failed
        );
        // 迟到成功结果不能复活已 Failed 的 job。
        assert!(!store.mark_succeeded_if_running(&job2.id).unwrap());
        assert_eq!(
            store
                .list_all()
                .into_iter()
                .find(|j| j.id == job2.id)
                .unwrap()
                .status,
            CompressJobStatus::Failed
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn late_finalize_ignores_unknown_job() {
        let (dir, store) = temp_store();
        let unknown = Id::from_str("nope");
        assert!(!store.mark_succeeded_if_running(&unknown).unwrap());
        assert!(
            !store
                .mark_failed_or_retry_if_running(&unknown, "x")
                .unwrap()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    // ─── 三.9 原子性：候选 → 持久化 → 换入，persist 失败回滚内存 ────────
    //
    // write_fence 冻结 compress_jobs.json 让 persist（atomic_write）失败；
    // mutator 必须：返回 Err / 不改内存 / 磁盘原样。旧实现先改内存再
    // persist，失败后内存已脏（判别测试的断言会抓住它）。

    #[test]
    fn try_claim_rolls_back_memory_when_persist_fails() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store
            .enqueue_or_get_open(&camp, None, None, 200, 0)
            .unwrap();
        let jobs_path = dir.join("compress_jobs.json");
        let before = std::fs::read(&jobs_path).unwrap();
        storyforge_infra_util::write_fence::freeze(&jobs_path);

        let result = store.try_claim_pending(&job.id);
        assert!(result.is_err(), "persist 被冻结时 claim 必须失败");
        assert_eq!(
            store.list_all()[0].status,
            CompressJobStatus::Pending,
            "persist 失败后内存 job 必须保持 Pending（不得脏改）"
        );
        assert_eq!(store.list_all()[0].attempts, 0);
        assert!(
            std::fs::read(&jobs_path).unwrap() == before,
            "磁盘必须原样（冻结期间无任何写入）"
        );
        storyforge_infra_util::write_fence::unfreeze(&jobs_path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn enqueue_rolls_back_memory_when_persist_fails() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let jobs_path = dir.join("compress_jobs.json");
        // 首次 persist 前文件尚不存在（store 构造只加载不写盘）。
        let before = std::fs::read(&jobs_path).ok();
        storyforge_infra_util::write_fence::freeze(&jobs_path);

        let result = store.enqueue_or_get_open(&camp, None, None, 1, 0);
        assert!(result.is_err(), "persist 被冻结时 enqueue 必须失败");
        assert!(
            store.list_all().is_empty(),
            "persist 失败后内存不得残留未持久化 job"
        );
        let after = std::fs::read(&jobs_path).ok();
        assert!(after == before, "磁盘必须原样（冻结期间无任何写入）");
        storyforge_infra_util::write_fence::unfreeze(&jobs_path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn succeed_rolls_back_memory_when_persist_fails() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store.enqueue_or_get_open(&camp, None, None, 1, 0).unwrap();
        store.mark_running(&job.id).unwrap();
        let jobs_path = dir.join("compress_jobs.json");
        storyforge_infra_util::write_fence::freeze(&jobs_path);

        let result = store.mark_succeeded_if_running(&job.id);
        assert!(result.is_err());
        assert_eq!(
            store.list_all()[0].status,
            CompressJobStatus::Running,
            "persist 失败后不得脏改状态"
        );
        storyforge_infra_util::write_fence::unfreeze(&jobs_path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fail_retry_rolls_back_memory_when_persist_fails() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store.enqueue_or_get_open(&camp, None, None, 1, 0).unwrap();
        store.mark_running(&job.id).unwrap();
        let jobs_path = dir.join("compress_jobs.json");
        storyforge_infra_util::write_fence::freeze(&jobs_path);

        let result = store.mark_failed_or_retry_if_running(&job.id, "boom");
        assert!(result.is_err());
        let j = &store.list_all()[0];
        assert_eq!(j.status, CompressJobStatus::Running);
        assert!(j.last_error.is_none());
        storyforge_infra_util::write_fence::unfreeze(&jobs_path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reset_running_to_pending_rolls_back_memory_when_persist_fails() {
        let (dir, store) = temp_store();
        let camp = Id::from_str("c1");
        let (job, _) = store.enqueue_or_get_open(&camp, None, None, 1, 0).unwrap();
        store.mark_running(&job.id).unwrap();
        let jobs_path = dir.join("compress_jobs.json");
        storyforge_infra_util::write_fence::freeze(&jobs_path);

        // reset 的公开签名是 usize（存储 facade 依赖）；持久化失败必须返回 0
        // 且内存保持 Running——绝不能一边报 0 一边把内存改成 Pending。
        let reset = store.reset_running_to_pending();
        assert_eq!(reset, 0, "persist 失败时 reset 不得计入");
        assert_eq!(
            store.list_all()[0].status,
            CompressJobStatus::Running,
            "persist 失败后内存 job 必须保持 Running"
        );
        storyforge_infra_util::write_fence::unfreeze(&jobs_path);
        let _ = std::fs::remove_dir_all(dir);
    }
}

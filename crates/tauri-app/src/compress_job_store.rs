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
    pub fn reset_running_to_pending(&self) -> usize {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let mut n = 0;
        for j in jobs.iter_mut() {
            if j.status == CompressJobStatus::Running {
                j.status = CompressJobStatus::Pending;
                j.updated_at = now_iso();
                n += 1;
            }
        }
        if n > 0 {
            let _ = persist(&self.path, &jobs);
        }
        n
    }

    /// 若该 campaign 已有 open job 则返回已有；否则新建 Pending。
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
        jobs.push(job.clone());
        persist(&self.path, &jobs)?;
        Ok((job, true))
    }

    pub fn mark_running(&self, job_id: &Id) -> Result<(), String> {
        self.update_job(job_id, |j| {
            j.status = CompressJobStatus::Running;
            j.attempts = j.attempts.saturating_add(1);
            j.last_error = None;
        })
    }

    pub fn mark_succeeded(&self, job_id: &Id) -> Result<(), String> {
        self.update_job(job_id, |j| {
            j.status = CompressJobStatus::Succeeded;
            j.last_error = None;
        })
    }

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

    fn update_job(
        &self,
        job_id: &Id,
        f: impl FnOnce(&mut CompressJob),
    ) -> Result<(), String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let Some(j) = jobs.iter_mut().find(|j| &j.id == job_id) else {
            return Err(format!("compress job {job_id} not found"));
        };
        f(j);
        j.updated_at = now_iso();
        persist(&self.path, &jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (PathBuf, CompressJobStore) {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-compress-jobs-{}",
            uuid::Uuid::new_v4()
        ));
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
        assert_eq!(
            store.list_all()[0].status,
            CompressJobStatus::Running
        );
        assert_eq!(store.reset_running_to_pending(), 1);
        assert_eq!(
            store.list_all()[0].status,
            CompressJobStatus::Pending
        );
        // reload from disk
        let reloaded = CompressJobStore::new(&dir);
        assert_eq!(reloaded.list_open().len(), 1);
        assert_eq!(
            reloaded.list_all()[0].status,
            CompressJobStatus::Pending
        );
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
        let j = store.list_all().into_iter().find(|j| j.id == job.id).unwrap();
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
}

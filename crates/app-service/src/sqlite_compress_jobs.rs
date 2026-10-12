//! SQLite-native Chronicle compressor job queue (Gate 4).
//!
//! Mirrors the JSON `CompressJobStore` semantics on the V006
//! `chronicle_compress_jobs` table: per-campaign open-job dedup (partial unique
//! index), atomic Pending→Running claim, attempts/max_attempts retry, and
//! Running→Pending recovery after a crash. Every transition is a single
//! transactional statement; the claim is the mutual-exclusion primitive that
//! makes duplicate worker startups idempotent.

use rusqlite::OptionalExtension;
use storyforge_domain::Id;
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::UnitOfWork;
use storyforge_infra_sqlite::error::{Result as SqliteResult, SqliteError};
use storyforge_infra_sqlite::migrations;

/// Mirrors `CompressJobStatus` in `compress_job_store.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// A row of `chronicle_compress_jobs` (mirror of the JSON `CompressJob`).
#[derive(Debug, Clone)]
pub struct SqliteCompressJob {
    pub id: Id,
    pub campaign_id: Id,
    pub conversation_id: Option<Id>,
    pub lineage_id: Option<Id>,
    pub status: CompressJobStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub last_error: Option<String>,
    pub uncovered_a_at_enqueue: u32,
    pub uncovered_b_at_enqueue: u32,
}

pub const DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS: u32 = 5;

pub struct SqliteCompressJobRepository;

impl SqliteCompressJobRepository {
    fn migrate(db: &mut Database) -> SqliteResult<()> {
        migrations::migrate(db).map(|_| ())
    }

    fn parse_status(status: &str) -> SqliteResult<CompressJobStatus> {
        match status {
            "pending" => Ok(CompressJobStatus::Pending),
            "running" => Ok(CompressJobStatus::Running),
            "succeeded" => Ok(CompressJobStatus::Succeeded),
            "failed" => Ok(CompressJobStatus::Failed),
            other => Err(SqliteError::Other(format!(
                "unknown compress job status {other}"
            ))),
        }
    }

    /// 若 campaign 已有 open job 则返回已有；否则新建 Pending（唯一索引兜底并发）。
    pub fn enqueue_or_get_open(
        db: &mut Database,
        campaign_id: &Id,
        conversation_id: Option<Id>,
        lineage_id: Option<Id>,
        uncovered_a: u32,
        uncovered_b: u32,
    ) -> SqliteResult<(SqliteCompressJob, bool)> {
        Self::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        if let Some(existing) = load_open_for_campaign_tx(tx, campaign_id)? {
            uow.commit()?;
            return Ok((existing, false));
        }
        let now = chrono::Utc::now().to_rfc3339();
        let job_id = Id::new();
        tx.execute(
            r#"
            INSERT INTO chronicle_compress_jobs (
                job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts,
                max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, 'auto', 'pending', 0, ?5, NULL, ?6, ?7, ?8, ?8)
            "#,
            rusqlite::params![
                job_id.as_str(),
                campaign_id.as_str(),
                conversation_id.as_ref().map(Id::as_str),
                lineage_id.as_ref().map(Id::as_str),
                DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS as i64,
                uncovered_a as i64,
                uncovered_b as i64,
                now,
            ],
        )?;
        uow.commit()?;
        let job = SqliteCompressJob {
            id: job_id,
            campaign_id: campaign_id.clone(),
            conversation_id,
            lineage_id,
            status: CompressJobStatus::Pending,
            attempts: 0,
            max_attempts: DEFAULT_COMPRESS_JOB_MAX_ATTEMPTS,
            last_error: None,
            uncovered_a_at_enqueue: uncovered_a,
            uncovered_b_at_enqueue: uncovered_b,
        };
        Ok((job, true))
    }

    /// 原子领取：仅 `pending → running` 成功；其它状态返回 `Ok(false)`。
    pub fn try_claim_pending(db: &mut Database, job_id: &Id) -> SqliteResult<bool> {
        Self::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        let changed = tx.execute(
            r#"
            UPDATE chronicle_compress_jobs
            SET status = 'running', attempts = attempts + 1, last_error = NULL,
                updated_at = ?2
            WHERE job_id = ?1 AND status = 'pending'
            "#,
            rusqlite::params![job_id.as_str(), chrono::Utc::now().to_rfc3339()],
        )?;
        uow.commit()?;
        Ok(changed > 0)
    }

    pub fn mark_succeeded(db: &mut Database, job_id: &Id) -> SqliteResult<bool> {
        Self::transition(db, job_id, CompressJobStatus::Succeeded, None)
    }

    /// 失败回队：attempts 达上限 → failed，否则回 pending。
    pub fn mark_failed_or_retry(db: &mut Database, job_id: &Id, err: &str) -> SqliteResult<bool> {
        Self::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        let row = tx
            .query_row(
                "SELECT status, attempts, max_attempts FROM chronicle_compress_jobs WHERE job_id = ?1",
                [job_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((status, attempts, max_attempts)) = row else {
            return Ok(false);
        };
        let next = if attempts >= max_attempts {
            "failed"
        } else {
            "pending"
        };
        let changed = tx.execute(
            "UPDATE chronicle_compress_jobs SET status = ?1, last_error = ?2, updated_at = ?3 WHERE job_id = ?4 AND status = 'running'",
            rusqlite::params![
                next,
                err,
                chrono::Utc::now().to_rfc3339(),
                job_id.as_str()
            ],
        )?;
        let _ = status;
        uow.commit()?;
        Ok(changed > 0)
    }

    /// 只对仍处于 Running 的 job 做终态迁移（迟到结果 / 并发 worker 安全）。
    /// 返回 false 表示 job 已被其它 worker 终态化，调用方不得再改。
    pub fn transition(
        db: &mut Database,
        job_id: &Id,
        next: CompressJobStatus,
        last_error: Option<&str>,
    ) -> SqliteResult<bool> {
        Self::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        let changed = tx.execute(
            r#"
            UPDATE chronicle_compress_jobs
            SET status = ?2, last_error = COALESCE(?3, last_error), updated_at = ?4
            WHERE job_id = ?1 AND status = 'running'
            "#,
            rusqlite::params![
                job_id.as_str(),
                next.as_str(),
                last_error,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        uow.commit()?;
        Ok(changed > 0)
    }

    /// 启动恢复：Running → Pending（崩溃中断可重放）。返回重置数量。
    pub fn reset_running_to_pending(db: &mut Database) -> SqliteResult<usize> {
        Self::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        let changed = tx.execute(
            r#"
            UPDATE chronicle_compress_jobs
            SET status = 'pending', updated_at = ?1
            WHERE status = 'running'
            "#,
            [chrono::Utc::now().to_rfc3339()],
        )?;
        uow.commit()?;
        Ok(changed)
    }

    pub fn list_open(db: &Database) -> SqliteResult<Vec<SqliteCompressJob>> {
        load_jobs(
            db,
            "SELECT job_id, campaign_id, conversation_id, lineage_id, status, attempts, max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue FROM chronicle_compress_jobs WHERE status IN ('pending','running') ORDER BY created_at, job_id",
            [],
        )
    }

    pub fn list_all(db: &Database) -> SqliteResult<Vec<SqliteCompressJob>> {
        load_jobs(
            db,
            "SELECT job_id, campaign_id, conversation_id, lineage_id, status, attempts, max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue FROM chronicle_compress_jobs ORDER BY created_at, job_id",
            [],
        )
    }

    pub fn get(db: &Database, job_id: &Id) -> SqliteResult<Option<SqliteCompressJob>> {
        load_jobs(
            db,
            "SELECT job_id, campaign_id, conversation_id, lineage_id, status, attempts, max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue FROM chronicle_compress_jobs WHERE job_id = ?1",
            [job_id.as_str()],
        )
        .map(|mut jobs| jobs.pop())
    }

    /// 未覆盖 A/B 数量（阈值判定，与 JSON 版 count_uncovered_chronicle_levels 同语义）。
    pub fn count_uncovered(db: &Database, campaign_id: &Id) -> SqliteResult<(usize, usize)> {
        let mut uncovered_a = 0usize;
        let mut uncovered_b = 0usize;
        let mut stmt = db.connection().prepare(
            "SELECT payload_json FROM round_summaries WHERE campaign_id = ?1 AND covered_by IS NULL",
        )?;
        let rows = stmt.query_map([campaign_id.as_str()], |row| row.get::<_, String>(0))?;
        for row in rows {
            let payload = row?;
            let summary: storyforge_domain::agent::RoundSummary =
                serde_json::from_str(&payload).map_err(SqliteError::from)?;
            if summary.is_leaf_a() {
                uncovered_a += 1;
            } else if summary.chronicle_level() == storyforge_domain::chronicle::ChronicleLevel::B {
                uncovered_b += 1;
            }
        }
        Ok((uncovered_a, uncovered_b))
    }
}

fn load_open_for_campaign_tx(
    tx: &rusqlite::Transaction<'_>,
    campaign_id: &Id,
) -> SqliteResult<Option<SqliteCompressJob>> {
    let mut stmt = tx.prepare(
        "SELECT job_id, campaign_id, conversation_id, lineage_id, status, attempts, max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue FROM chronicle_compress_jobs WHERE campaign_id = ?1 AND status IN ('pending','running') LIMIT 1",
    )?;
    let mut rows = stmt.query_map([campaign_id.as_str()], map_job)?;
    Ok(rows.next().transpose()?)
}

fn load_jobs<const N: usize>(
    db: &Database,
    sql: &str,
    params: [&str; N],
) -> SqliteResult<Vec<SqliteCompressJob>> {
    let mut stmt = db.connection().prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), map_job)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<SqliteCompressJob> {
    Ok(SqliteCompressJob {
        id: Id::from_str(&row.get::<_, String>(0)?),
        campaign_id: Id::from_str(&row.get::<_, String>(1)?),
        conversation_id: row.get::<_, Option<String>>(2)?.map(|id| Id::from_str(&id)),
        lineage_id: row.get::<_, Option<String>>(3)?.map(|id| Id::from_str(&id)),
        status: SqliteCompressJobRepository::parse_status(&row.get::<_, String>(4)?)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        attempts: row.get::<_, i64>(5)? as u32,
        max_attempts: row.get::<_, i64>(6)? as u32,
        last_error: row.get(7)?,
        uncovered_a_at_enqueue: row.get::<_, i64>(8)? as u32,
        uncovered_b_at_enqueue: row.get::<_, i64>(9)? as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::agent::RoundSummary;
    use storyforge_infra_sqlite::production::SqliteProductionRepository;
    use storyforge_infra_sqlite::publication::SqliteChronicleRepository;

    fn seed_campaign(db: &mut Database, campaign_id: &Id, card_id: &Id) {
        let mut campaign = storyforge_domain::campaign::Campaign::new(card_id.clone(), "C");
        campaign.id = campaign_id.clone();
        SqliteProductionRepository::save_campaign(db, &campaign).unwrap();
    }

    fn seed_leaf(db: &mut Database, campaign_id: &Id, turn: u32) {
        let conversation_id = Id::from_str("conv-leaves");
        let mut conversation =
            storyforge_domain::conversation::Conversation::new(None, Some(campaign_id.clone()));
        conversation.id = conversation_id.clone();
        SqliteProductionRepository::save_conversation(db, &conversation).unwrap();
        let summary = RoundSummary::new(
            campaign_id.clone(),
            conversation_id,
            turn,
            format!("leaf {turn}"),
        );
        SqliteChronicleRepository::seed_summary(db, &summary).unwrap();
    }

    #[test]
    fn enqueue_dedups_open_job_per_campaign() {
        let mut db = Database::open_in_memory().unwrap();
        let campaign_id = Id::from_str("c1");
        seed_campaign(&mut db, &campaign_id, &Id::from_str("card-1"));
        let (j1, created1) = SqliteCompressJobRepository::enqueue_or_get_open(
            &mut db,
            &campaign_id,
            None,
            None,
            200,
            0,
        )
        .unwrap();
        assert!(created1);
        let (j2, created2) = SqliteCompressJobRepository::enqueue_or_get_open(
            &mut db,
            &campaign_id,
            None,
            None,
            210,
            0,
        )
        .unwrap();
        assert!(!created2);
        assert_eq!(j1.id, j2.id);
        assert_eq!(
            SqliteCompressJobRepository::list_open(&db).unwrap().len(),
            1
        );
    }

    #[test]
    fn claim_is_atomic_and_only_pending() {
        let mut db = Database::open_in_memory().unwrap();
        let campaign_id = Id::from_str("c1");
        seed_campaign(&mut db, &campaign_id, &Id::from_str("card-1"));
        let (job, _) = SqliteCompressJobRepository::enqueue_or_get_open(
            &mut db,
            &campaign_id,
            None,
            None,
            200,
            0,
        )
        .unwrap();
        assert!(SqliteCompressJobRepository::try_claim_pending(&mut db, &job.id).unwrap());
        assert!(!SqliteCompressJobRepository::try_claim_pending(&mut db, &job.id).unwrap());
        let loaded = SqliteCompressJobRepository::get(&db, &job.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.status, CompressJobStatus::Running);
        assert_eq!(loaded.attempts, 1);
    }

    #[test]
    fn crash_recovery_resets_running_to_pending() {
        let mut db = Database::open_in_memory().unwrap();
        let campaign_id = Id::from_str("c1");
        seed_campaign(&mut db, &campaign_id, &Id::from_str("card-1"));
        let (job, _) = SqliteCompressJobRepository::enqueue_or_get_open(
            &mut db,
            &campaign_id,
            None,
            None,
            200,
            0,
        )
        .unwrap();
        SqliteCompressJobRepository::try_claim_pending(&mut db, &job.id).unwrap();
        assert_eq!(
            SqliteCompressJobRepository::reset_running_to_pending(&mut db).unwrap(),
            1
        );
        let loaded = SqliteCompressJobRepository::get(&db, &job.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.status, CompressJobStatus::Pending);
    }

    #[test]
    fn failed_after_max_attempts_else_back_to_pending() {
        let mut db = Database::open_in_memory().unwrap();
        let campaign_id = Id::from_str("c1");
        seed_campaign(&mut db, &campaign_id, &Id::from_str("card-1"));
        let (job, _) = SqliteCompressJobRepository::enqueue_or_get_open(
            &mut db,
            &campaign_id,
            None,
            None,
            200,
            0,
        )
        .unwrap();
        for expected in [CompressJobStatus::Pending; 4] {
            SqliteCompressJobRepository::try_claim_pending(&mut db, &job.id).unwrap();
            SqliteCompressJobRepository::mark_failed_or_retry(&mut db, &job.id, "boom").unwrap();
            let loaded = SqliteCompressJobRepository::get(&db, &job.id)
                .unwrap()
                .unwrap();
            assert_eq!(loaded.status, expected);
        }
        // 第 5 次（attempts=5 >= max 5）→ failed
        SqliteCompressJobRepository::try_claim_pending(&mut db, &job.id).unwrap();
        SqliteCompressJobRepository::mark_failed_or_retry(&mut db, &job.id, "boom").unwrap();
        let loaded = SqliteCompressJobRepository::get(&db, &job.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.status, CompressJobStatus::Failed);
        assert_eq!(loaded.last_error.as_deref(), Some("boom"));
        assert_eq!(loaded.attempts, 5);
    }

    #[test]
    fn late_result_transition_only_applies_to_running() {
        let mut db = Database::open_in_memory().unwrap();
        let campaign_id = Id::from_str("c1");
        seed_campaign(&mut db, &campaign_id, &Id::from_str("card-1"));
        let (job, _) = SqliteCompressJobRepository::enqueue_or_get_open(
            &mut db,
            &campaign_id,
            None,
            None,
            200,
            0,
        )
        .unwrap();
        // worker A 完成：succeeded。
        SqliteCompressJobRepository::try_claim_pending(&mut db, &job.id).unwrap();
        assert!(SqliteCompressJobRepository::mark_succeeded(&mut db, &job.id).unwrap());
        // 迟到 worker B 想终态化 → false，且不能把 succeeded 翻回 pending。
        assert!(
            !SqliteCompressJobRepository::mark_failed_or_retry(&mut db, &job.id, "late").unwrap()
        );
        let loaded = SqliteCompressJobRepository::get(&db, &job.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.status, CompressJobStatus::Succeeded);
        assert_eq!(loaded.last_error, None);
    }

    #[test]
    fn count_uncovered_reads_sqlite_authority() {
        let mut db = Database::open_in_memory().unwrap();
        let campaign_id = Id::from_str("c1");
        seed_campaign(&mut db, &campaign_id, &Id::from_str("card-1"));
        for turn in 1..=5 {
            seed_leaf(&mut db, &campaign_id, turn);
        }
        let (a, b) = SqliteCompressJobRepository::count_uncovered(&db, &campaign_id).unwrap();
        assert_eq!(a, 5);
        assert_eq!(b, 0);
    }
}

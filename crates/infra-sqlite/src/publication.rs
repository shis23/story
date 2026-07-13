//! Typed Chronicle B/C publication Unit of Work.
//!
//! Not wired into the default app backend. Callers must pass an explicit [`Database`].

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;
use sha2::{Digest, Sha256};
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::chronicle::{
    CompressGroup, PendingCompressPublication, validate_compress_covers,
};

use crate::connection::Database;
use crate::error::{Result, SqliteError};
use crate::migrations;
use crate::unit_of_work::UnitOfWork;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Applied,
    AlreadyPublished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishFault {
    None,
    AfterParentInsert,
    AfterChildUpdate,
    AfterRevisionBump,
    AfterMarkerCleanup,
}

pub struct PublishRequest<'a> {
    pub campaign_id: &'a Id,
    pub publication_id: &'a Id,
    pub parents: &'a [RoundSummary],
    pub child_covered_by: &'a [(Id, Id)],
    pub job_id: Option<&'a str>,
}

struct JobRow {
    campaign_id: String,
    payload_hash: String,
    status: String,
    target_chronicle_revision: u64,
}

pub struct SqliteChronicleRepository;

impl SqliteChronicleRepository {
    /// Seed a leaf/stage summary outside the publication UoW (test/bootstrap helper).
    pub fn seed_summary(db: &mut Database, summary: &RoundSummary) -> Result<()> {
        migrations::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        insert_or_exact_summary(tx, summary)?;
        uow.commit()?;
        Ok(())
    }

    pub fn count_publication_jobs(db: &Database) -> Result<usize> {
        let count: i64 = db.connection().query_row(
            "SELECT COUNT(*) FROM chronicle_publication_jobs",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn publish_compress(
        db: &mut Database,
        request: PublishRequest<'_>,
    ) -> Result<PublishOutcome> {
        Self::publish_compress_with_fault(db, request, PublishFault::None)
    }

    #[doc(hidden)]
    pub fn publish_compress_with_fault(
        db: &mut Database,
        request: PublishRequest<'_>,
        fault: PublishFault,
    ) -> Result<PublishOutcome> {
        migrations::migrate(db)?;
        let fingerprint = request_fingerprint(&request)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        if let Some(job) = load_job(tx, request.publication_id)? {
            if job.campaign_id != request.campaign_id.as_str() {
                return Err(SqliteError::Conflict(format!(
                    "publication {} belongs to campaign {}, not {}",
                    request.publication_id, job.campaign_id, request.campaign_id
                )));
            }
            if job.payload_hash != fingerprint {
                return Err(SqliteError::Conflict(format!(
                    "publication {} payload mismatch",
                    request.publication_id
                )));
            }
            if job.status == "completed" {
                // Replay is only a no-op when durable state still matches the job.
                verify_completed_publication_state(tx, &job, &request)?;
                uow.commit()?;
                return Ok(PublishOutcome::AlreadyPublished);
            }
            return Err(SqliteError::Conflict(format!(
                "publication {} is in status {}",
                request.publication_id, job.status
            )));
        }

        if let Some(job_id) = request.job_id
            && job_id_exists(tx, job_id)?
        {
            return Err(SqliteError::Conflict(format!(
                "publication job_id {job_id} is already used"
            )));
        }

        let mut campaign: Campaign = load_payload(
            tx,
            "SELECT payload_json FROM campaigns WHERE campaign_id = ?1",
            request.campaign_id.as_str(),
        )?
        .ok_or_else(|| SqliteError::RecordNotFound(format!("campaign {}", request.campaign_id)))?;
        let structured_chronicle_revision: u64 = tx.query_row(
            "SELECT chronicle_revision FROM campaigns WHERE campaign_id = ?1",
            [request.campaign_id.as_str()],
            |row| row.get(0),
        )?;
        if campaign.chronicle_revision != structured_chronicle_revision {
            return Err(SqliteError::Conflict(format!(
                "campaign {} payload chronicle_revision {} differs from indexed {}",
                campaign.id, campaign.chronicle_revision, structured_chronicle_revision
            )));
        }

        if let Some(existing_pending) = campaign.pending_compress_publication.as_ref()
            && existing_pending.publication_id != *request.publication_id
        {
            return Err(SqliteError::Conflict(format!(
                "campaign {} already has pending publication marker {}",
                campaign.id, existing_pending.publication_id
            )));
        }

        validate_publication_request(tx, &campaign, &request)?;

        let base_rev = campaign.chronicle_revision;
        let pending = PendingCompressPublication {
            publication_id: request.publication_id.clone(),
            base_chronicle_revision: base_rev,
            parent_ids: request.parents.iter().map(|p| p.id.clone()).collect(),
            child_covered_by: request.child_covered_by.to_vec(),
            child_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        campaign.pending_compress_publication = Some(pending.clone());
        write_campaign(tx, &campaign)?;

        // 1) Parent upserts (exact identity / payload)
        for parent in request.parents {
            insert_or_exact_summary(tx, parent)?;
        }
        if fault == PublishFault::AfterParentInsert {
            return Err(SqliteError::Other(
                "injected failure after parent insert".into(),
            ));
        }

        // 2) Child covered_by updates + cover edges
        for (child_id, parent_id) in request.child_covered_by {
            apply_child_cover(tx, &campaign, child_id, parent_id)?;
        }
        for parent in request.parents {
            rewrite_parent_covers(tx, parent)?;
        }
        if fault == PublishFault::AfterChildUpdate {
            return Err(SqliteError::Other(
                "injected failure after child update".into(),
            ));
        }

        // 3) Verify marker, bump revision, clear pending/job completion
        verify_pending_publication(tx, request.campaign_id, &pending)?;
        if campaign.chronicle_revision <= base_rev {
            campaign.bump_chronicle_revision();
        }
        campaign.context_epoch = None;
        if fault == PublishFault::AfterRevisionBump {
            // Persist revision bump then fail so rollback proof covers this point.
            write_campaign(tx, &campaign)?;
            return Err(SqliteError::Other(
                "injected failure after revision bump".into(),
            ));
        }

        campaign.pending_compress_publication = None;
        write_campaign(tx, &campaign)?;
        if fault == PublishFault::AfterMarkerCleanup {
            return Err(SqliteError::Other(
                "injected failure after marker cleanup".into(),
            ));
        }

        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            r#"
            INSERT INTO chronicle_publication_jobs (
                publication_id, campaign_id, job_id, base_chronicle_revision,
                target_chronicle_revision, parent_ids_json, child_covered_by_json,
                payload_hash, status, created_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'completed', ?9, ?10)
            "#,
            rusqlite::params![
                request.publication_id.as_str(),
                request.campaign_id.as_str(),
                request.job_id,
                base_rev,
                campaign.chronicle_revision,
                json(&pending.parent_ids)?,
                json(&request.child_covered_by)?,
                fingerprint,
                now,
                now,
            ],
        )?;

        uow.commit()?;
        Ok(PublishOutcome::Applied)
    }
}

fn validate_publication_request(
    tx: &Transaction<'_>,
    campaign: &Campaign,
    request: &PublishRequest<'_>,
) -> Result<()> {
    if request.parents.is_empty() {
        return Err(SqliteError::Conflict(
            "publication requires at least one parent summary".into(),
        ));
    }
    if request.child_covered_by.is_empty() {
        return Err(SqliteError::Conflict(
            "publication requires child_covered_by mappings".into(),
        ));
    }

    let parent_ids: HashSet<&Id> = request.parents.iter().map(|p| &p.id).collect();
    if parent_ids.len() != request.parents.len() {
        return Err(SqliteError::Conflict(
            "publication parents contain duplicate ids".into(),
        ));
    }

    let mut cover_by_parent: HashMap<&Id, Vec<&Id>> = HashMap::new();
    let mut seen_children = HashSet::new();
    for (child_id, parent_id) in request.child_covered_by {
        if !parent_ids.contains(parent_id) {
            return Err(SqliteError::Conflict(format!(
                "child_covered_by parent {parent_id} is not in parents list"
            )));
        }
        if !seen_children.insert(child_id) {
            return Err(SqliteError::Conflict(format!(
                "child {child_id} appears in multiple covers"
            )));
        }
        cover_by_parent.entry(parent_id).or_default().push(child_id);
    }

    for parent in request.parents {
        if &parent.campaign_id != request.campaign_id {
            return Err(SqliteError::Conflict(format!(
                "parent {} campaign mismatch",
                parent.id
            )));
        }
        if parent.level != 1 && parent.level != 2 {
            return Err(SqliteError::Conflict(format!(
                "parent {} level must be B/C (1/2), got {}",
                parent.id, parent.level
            )));
        }
        if parent.covered_by.is_some() {
            return Err(SqliteError::Conflict(format!(
                "parent {} must not already be covered",
                parent.id
            )));
        }
        let parent_lineage = parent.lineage_id.as_ref().map(Id::as_str);
        let campaign_lineage = campaign.lineage_id.as_ref().map(Id::as_str);
        if parent_lineage.is_none() || parent_lineage != campaign_lineage {
            return Err(SqliteError::Conflict(format!(
                "parent {} lineage does not match campaign lineage",
                parent.id
            )));
        }
        if parent.conversation_id.as_str()
            != campaign
                .conversation_id
                .as_ref()
                .map(Id::as_str)
                .unwrap_or_default()
        {
            return Err(SqliteError::Conflict(format!(
                "parent {} conversation does not match campaign",
                parent.id
            )));
        }

        let expected_covers: Vec<Id> = cover_by_parent
            .get(&parent.id)
            .map(|ids| ids.iter().map(|id| (*id).clone()).collect())
            .unwrap_or_default();
        if expected_covers.is_empty() {
            return Err(SqliteError::Conflict(format!(
                "parent {} has no covered children",
                parent.id
            )));
        }
        // Parent.covers must match the mapping set (order-insensitive compare later via validation).
        if parent.covers.len() != expected_covers.len()
            || parent.covers.iter().collect::<HashSet<_>>()
                != expected_covers.iter().collect::<HashSet<_>>()
        {
            return Err(SqliteError::Conflict(format!(
                "parent {} covers do not match child_covered_by mapping",
                parent.id
            )));
        }

        // Load children and validate continuity / level / lineage / spans.
        let mut children = Vec::with_capacity(parent.covers.len());
        for child_id in &parent.covers {
            let child = load_summary(tx, child_id)?.ok_or_else(|| {
                SqliteError::Conflict(format!(
                    "publication cover child {child_id} is missing for parent {}",
                    parent.id
                ))
            })?;
            if child.campaign_id != campaign.id {
                return Err(SqliteError::Conflict(format!(
                    "child {child_id} campaign mismatch"
                )));
            }
            let child_lineage = child.lineage_id.as_ref().map(Id::as_str);
            let campaign_lineage = campaign.lineage_id.as_ref().map(Id::as_str);
            if child_lineage.is_none() || child_lineage != campaign_lineage {
                return Err(SqliteError::Conflict(format!(
                    "child {child_id} lineage does not match campaign lineage"
                )));
            }
            let expected_conversation = campaign
                .conversation_id
                .as_ref()
                .map(Id::as_str)
                .unwrap_or_default();
            if child.conversation_id.as_str() != expected_conversation {
                return Err(SqliteError::Conflict(format!(
                    "child {child_id} conversation scope does not match campaign"
                )));
            }
            if child.covered_by.is_some() && child.covered_by.as_ref() != Some(&parent.id) {
                return Err(SqliteError::Conflict(format!(
                    "child {child_id} already covered by another parent"
                )));
            }
            let expected_child_level = parent.level.saturating_sub(1);
            if child.level != expected_child_level {
                return Err(SqliteError::Conflict(format!(
                    "parent {} level {} cannot cover child {child_id} level {}",
                    parent.id, parent.level, child.level
                )));
            }
            children.push(child);
        }

        // Validate continuous non-overlapping covers using domain pure function.
        children.sort_by_key(|c| (c.turn, c.effective_turn_end(), c.id.as_str().to_string()));
        let input_ids: Vec<Id> = children.iter().map(|c| c.id.clone()).collect();
        let spans: Vec<(u32, u32)> = children
            .iter()
            .map(|c| (c.turn, c.effective_turn_end()))
            .collect();
        // Children must form a continuous turn band (no gaps between members).
        for window in children.windows(2) {
            let prev_end = window[0].effective_turn_end();
            let next_start = window[1].turn;
            if next_start > prev_end.saturating_add(1) {
                return Err(SqliteError::Conflict(format!(
                    "parent {} covers are not continuous: gap between turn {} and {}",
                    parent.id, prev_end, next_start
                )));
            }
        }
        let group = CompressGroup {
            member_ids: input_ids.clone(),
            turn_start: parent.turn,
            turn_end: parent.effective_turn_end(),
        };
        validate_compress_covers(&input_ids, &spans, std::slice::from_ref(&group)).map_err(
            |err| {
                SqliteError::Conflict(format!(
                    "invalid covers for parent {}: turn span/contiguity validation failed ({err:?})",
                    parent.id
                ))
            },
        )?;

        let expect_start = spans.first().map(|s| s.0).unwrap_or(parent.turn);
        let expect_end = spans
            .last()
            .map(|s| s.1)
            .unwrap_or(parent.effective_turn_end());
        if parent.turn != expect_start || parent.effective_turn_end() != expect_end {
            return Err(SqliteError::Conflict(format!(
                "parent {} turn span {}-{} does not match children {}-{}",
                parent.id,
                parent.turn,
                parent.effective_turn_end(),
                expect_start,
                expect_end
            )));
        }
    }

    // Global non-overlap already enforced by unique children.
    Ok(())
}

fn apply_child_cover(
    tx: &Transaction<'_>,
    campaign: &Campaign,
    child_id: &Id,
    parent_id: &Id,
) -> Result<()> {
    let mut child = load_summary(tx, child_id)?.ok_or_else(|| {
        SqliteError::Conflict(format!(
            "child summary {child_id} missing during cover update"
        ))
    })?;
    if child.campaign_id != campaign.id {
        return Err(SqliteError::Conflict(format!(
            "child {child_id} campaign mismatch"
        )));
    }
    if child.covered_by.as_ref() == Some(parent_id) {
        // Idempotent cover under same parent.
        return Ok(());
    }
    if child.covered_by.is_some() {
        return Err(SqliteError::Conflict(format!(
            "child {child_id} already covered"
        )));
    }
    child.covered_by = Some(parent_id.clone());
    write_summary_row(tx, &child)?;
    Ok(())
}

fn rewrite_parent_covers(tx: &Transaction<'_>, parent: &RoundSummary) -> Result<()> {
    tx.execute(
        "DELETE FROM round_summary_covers WHERE parent_id = ?1",
        [parent.id.as_str()],
    )?;
    for child_id in &parent.covers {
        tx.execute(
            "INSERT INTO round_summary_covers (parent_id, child_id) VALUES (?1, ?2)",
            rusqlite::params![parent.id.as_str(), child_id.as_str()],
        )?;
    }
    Ok(())
}

fn verify_completed_publication_state(
    tx: &Transaction<'_>,
    job: &JobRow,
    request: &PublishRequest<'_>,
) -> Result<()> {
    let campaign: Campaign = load_payload(
        tx,
        "SELECT payload_json FROM campaigns WHERE campaign_id = ?1",
        request.campaign_id.as_str(),
    )?
    .ok_or_else(|| {
        SqliteError::Conflict(format!(
            "completed publication drift: missing campaign {}",
            request.campaign_id
        ))
    })?;
    let indexed_revision: u64 = tx.query_row(
        "SELECT chronicle_revision FROM campaigns WHERE campaign_id = ?1",
        [request.campaign_id.as_str()],
        |row| row.get(0),
    )?;
    if campaign.chronicle_revision != job.target_chronicle_revision
        || indexed_revision != job.target_chronicle_revision
    {
        return Err(SqliteError::Conflict(format!(
            "completed publication drift: campaign revision payload={} indexed={} target={}",
            campaign.chronicle_revision, indexed_revision, job.target_chronicle_revision
        )));
    }
    if campaign.pending_compress_publication.is_some() {
        return Err(SqliteError::Conflict(
            "completed publication drift: campaign still has a pending publication marker".into(),
        ));
    }
    if campaign.context_epoch.is_some() {
        return Err(SqliteError::Conflict(
            "completed publication drift: context epoch must remain cleared".into(),
        ));
    }

    // Re-run the full scope/lineage/level/span validation against live children.
    validate_publication_request(tx, &campaign, request)?;

    for parent in request.parents {
        let existing = load_summary(tx, &parent.id)?.ok_or_else(|| {
            SqliteError::Conflict(format!(
                "completed publication drift: missing parent {}",
                parent.id
            ))
        })?;
        if !json_payloads_equal(&json(&existing)?, &json(parent)?)? {
            return Err(SqliteError::Conflict(format!(
                "completed publication drift: parent {} payload mismatch",
                parent.id
            )));
        }
        let mut stmt = tx.prepare(
            "SELECT child_id FROM round_summary_covers WHERE parent_id = ?1 ORDER BY child_id",
        )?;
        let rows = stmt.query_map([parent.id.as_str()], |row| row.get::<_, String>(0))?;
        let actual_edges: HashSet<String> = rows.collect::<std::result::Result<_, _>>()?;
        let expected_edges: HashSet<String> = parent
            .covers
            .iter()
            .map(|id| id.as_str().to_string())
            .collect();
        if actual_edges != expected_edges {
            return Err(SqliteError::Conflict(format!(
                "completed publication drift: parent {} cover edge set mismatch",
                parent.id
            )));
        }
    }
    for (child_id, parent_id) in request.child_covered_by {
        let covered_by: Option<String> = tx
            .query_row(
                "SELECT covered_by FROM round_summaries WHERE summary_id = ?1 AND campaign_id = ?2",
                rusqlite::params![child_id.as_str(), request.campaign_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if covered_by.as_deref() != Some(parent_id.as_str()) {
            return Err(SqliteError::Conflict(format!(
                "completed publication drift: child {child_id} covered_by incomplete"
            )));
        }
    }
    Ok(())
}

fn job_id_exists(tx: &Transaction<'_>, job_id: &str) -> Result<bool> {
    let found: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM chronicle_publication_jobs WHERE job_id = ?1 LIMIT 1",
            [job_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn verify_pending_publication(
    tx: &Transaction<'_>,
    campaign_id: &Id,
    pending: &PendingCompressPublication,
) -> Result<()> {
    for parent_id in &pending.parent_ids {
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM round_summaries WHERE summary_id = ?1 AND campaign_id = ?2",
                rusqlite::params![parent_id.as_str(), campaign_id.as_str()],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !exists {
            return Err(SqliteError::Conflict(format!(
                "pending publication incomplete: missing parent {parent_id}"
            )));
        }
    }
    for (child_id, parent_id) in &pending.child_covered_by {
        let covered_by: Option<String> = tx
            .query_row(
                "SELECT covered_by FROM round_summaries WHERE summary_id = ?1 AND campaign_id = ?2",
                rusqlite::params![child_id.as_str(), campaign_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if covered_by.as_deref() != Some(parent_id.as_str()) {
            return Err(SqliteError::Conflict(format!(
                "pending publication incomplete: child {child_id} covered_by mismatch"
            )));
        }
    }
    Ok(())
}

fn insert_or_exact_summary(tx: &Transaction<'_>, summary: &RoundSummary) -> Result<bool> {
    let existing: Option<String> = tx
        .query_row(
            "SELECT payload_json FROM round_summaries WHERE summary_id = ?1",
            [summary.id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    let payload = json(summary)?;
    if let Some(existing) = existing {
        if json_payloads_equal(&existing, &payload)? {
            return Ok(false);
        }
        return Err(SqliteError::Conflict(format!(
            "round_summaries {} payload mismatch",
            summary.id
        )));
    }
    write_summary_row(tx, summary)?;
    for child_id in &summary.covers {
        tx.execute(
            "INSERT OR IGNORE INTO round_summary_covers (parent_id, child_id) VALUES (?1, ?2)",
            rusqlite::params![summary.id.as_str(), child_id.as_str()],
        )?;
    }
    Ok(true)
}

fn write_summary_row(tx: &Transaction<'_>, summary: &RoundSummary) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO round_summaries (
            summary_id, campaign_id, conversation_id, lineage_id, level, turn, turn_end,
            code, headline, covered_by, content, created_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(summary_id) DO UPDATE SET
            campaign_id=excluded.campaign_id,
            conversation_id=excluded.conversation_id,
            lineage_id=excluded.lineage_id,
            level=excluded.level,
            turn=excluded.turn,
            turn_end=excluded.turn_end,
            code=excluded.code,
            headline=excluded.headline,
            covered_by=excluded.covered_by,
            content=excluded.content,
            created_at=excluded.created_at,
            payload_json=excluded.payload_json
        "#,
        rusqlite::params![
            summary.id.as_str(),
            summary.campaign_id.as_str(),
            summary.conversation_id.as_str(),
            summary.lineage_id.as_ref().map(Id::as_str),
            summary.level,
            summary.turn,
            summary.effective_turn_end(),
            summary.code,
            summary.headline,
            summary.covered_by.as_ref().map(Id::as_str),
            summary.content,
            summary.created_at,
            json(summary)?,
        ],
    )?;
    Ok(())
}

fn write_campaign(tx: &Transaction<'_>, campaign: &Campaign) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO campaigns (
            campaign_id, card_id, name, conversation_id, revision, chronicle_revision,
            lineage_id, story_clock, created_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(campaign_id) DO UPDATE SET
            card_id=excluded.card_id, name=excluded.name,
            conversation_id=excluded.conversation_id, revision=excluded.revision,
            chronicle_revision=excluded.chronicle_revision, lineage_id=excluded.lineage_id,
            story_clock=excluded.story_clock, created_at=excluded.created_at,
            payload_json=excluded.payload_json
        "#,
        rusqlite::params![
            campaign.id.as_str(),
            campaign.card_id.as_str(),
            campaign.name,
            campaign.conversation_id.as_ref().map(Id::as_str),
            campaign.revision,
            campaign.chronicle_revision,
            campaign.lineage_id.as_ref().map(Id::as_str),
            campaign.current_story_clock(),
            campaign.created_at,
            json(campaign)?,
        ],
    )?;
    Ok(())
}

fn load_job(tx: &Transaction<'_>, publication_id: &Id) -> Result<Option<JobRow>> {
    tx.query_row(
        r#"
        SELECT campaign_id, payload_hash, status, target_chronicle_revision
        FROM chronicle_publication_jobs WHERE publication_id = ?1
        "#,
        [publication_id.as_str()],
        |row| {
            Ok(JobRow {
                campaign_id: row.get(0)?,
                payload_hash: row.get(1)?,
                status: row.get(2)?,
                target_chronicle_revision: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn load_summary(tx: &Transaction<'_>, summary_id: &Id) -> Result<Option<RoundSummary>> {
    load_payload(
        tx,
        "SELECT payload_json FROM round_summaries WHERE summary_id = ?1",
        summary_id.as_str(),
    )
}

fn load_payload<T: serde::de::DeserializeOwned>(
    tx: &Transaction<'_>,
    sql: &str,
    id: &str,
) -> Result<Option<T>> {
    let payload: Option<String> = tx.query_row(sql, [id], |row| row.get(0)).optional()?;
    payload
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}

fn request_fingerprint(request: &PublishRequest<'_>) -> Result<String> {
    // Normalize mapping order so identical publications hash equal.
    let mut covered = request.child_covered_by.to_vec();
    covered.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
    let mut parents = request.parents.to_vec();
    parents.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    let payload = serde_json::json!({
        "campaign_id": request.campaign_id,
        "publication_id": request.publication_id,
        "job_id": request.job_id,
        "parents": parents,
        "child_covered_by": covered,
    });
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&payload)?);
    Ok(hex_encode(hasher.finalize()))
}

fn json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn json_payloads_equal(left: &str, right: &str) -> Result<bool> {
    let left: serde_json::Value = serde_json::from_str(left)?;
    let right: serde_json::Value = serde_json::from_str(right)?;
    Ok(left == right)
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

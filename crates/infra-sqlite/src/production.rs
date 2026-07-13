//! Typed, transactional SQLite repository for production Turn acceptance.
//!
//! This module is deliberately not wired into the default application backend. Callers must
//! explicitly pass a SQLite [`Database`], which prevents accidental JSON/SQLite dual writes.

use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::conversation::{Conversation, VariantStatus};
use storyforge_domain::story_task::StoryTask;
use storyforge_domain::turn::{
    AttemptStatus, Mutation, MutationBatch, MutationBatchStatus, TurnAttempt, TurnRecord,
    TurnStatus,
};

use crate::connection::Database;
use crate::error::{Result, SqliteError};
use crate::migrations;
use crate::unit_of_work::UnitOfWork;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AcceptOutcome {
    Applied,
    AlreadyCommitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptFault {
    None,
    AfterMutations,
    BeforeLedger,
}

pub struct AcceptTurnRequest<'a> {
    pub turn_id: &'a Id,
    pub attempt_id: &'a Id,
    pub draft_hash: &'a str,
    pub batch: &'a MutationBatch,
    pub terminal_status: TurnStatus,
}

pub struct SqliteProductionRepository;

/// Stable draft identity used by the JSON production path and the SQLite adapter.
pub fn compute_draft_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex_encode(hasher.finalize())
}

impl SqliteProductionRepository {
    pub fn bootstrap_campaign(
        db: &mut Database,
        campaign: &Campaign,
        conversation: &Conversation,
    ) -> Result<()> {
        migrations::migrate(db)?;
        if conversation.campaign_id.as_ref() != Some(&campaign.id) {
            return Err(SqliteError::Conflict(format!(
                "conversation {} does not belong to campaign {}",
                conversation.id, campaign.id
            )));
        }

        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO character_cards (card_id, source_character_id, name, imported_at, payload_json) VALUES (?1, NULL, 'sqlite-production-placeholder', NULL, '{}')",
            [campaign.card_id.as_str()],
        )?;
        write_campaign(tx, campaign)?;
        write_conversation(tx, conversation)?;
        uow.commit()?;
        Ok(())
    }

    pub fn save_turn(db: &mut Database, turn: &TurnRecord) -> Result<()> {
        migrations::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        if turn.status.is_active() {
            let competing: Option<String> = tx
                .query_row(
                    r#"
                    SELECT turn_id FROM turns
                    WHERE campaign_id = ?1
                      AND turn_id <> ?2
                      AND status IN ('generating', 'draft_ready', 'deriving_state',
                                     'awaiting_acceptance', 'committing')
                    LIMIT 1
                    "#,
                    rusqlite::params![turn.campaign_id.as_str(), turn.turn_id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(other) = competing {
                return Err(SqliteError::Conflict(format!(
                    "campaign {} already has active turn {other}",
                    turn.campaign_id
                )));
            }
        }

        write_turn(tx, turn)?;
        uow.commit()?;
        Ok(())
    }

    pub fn get_campaign(db: &Database, campaign_id: &Id) -> Result<Option<Campaign>> {
        load_payload(
            db.connection(),
            "SELECT payload_json FROM campaigns WHERE campaign_id = ?1",
            campaign_id.as_str(),
        )
    }

    pub fn get_conversation(db: &Database, conversation_id: &Id) -> Result<Option<Conversation>> {
        load_payload(
            db.connection(),
            "SELECT payload_json FROM conversations WHERE conversation_id = ?1",
            conversation_id.as_str(),
        )
    }

    pub fn get_turn(db: &Database, turn_id: &Id) -> Result<Option<TurnRecord>> {
        load_payload(
            db.connection(),
            "SELECT payload_json FROM turns WHERE turn_id = ?1",
            turn_id.as_str(),
        )
    }

    pub fn get_attempt(db: &Database, attempt_id: &Id) -> Result<Option<TurnAttempt>> {
        load_payload(
            db.connection(),
            "SELECT payload_json FROM turn_attempts WHERE attempt_id = ?1",
            attempt_id.as_str(),
        )
    }

    pub fn get_instance(db: &Database, instance_id: &Id) -> Result<Option<CharacterInstance>> {
        load_payload(
            db.connection(),
            "SELECT payload_json FROM character_instances WHERE instance_id = ?1",
            instance_id.as_str(),
        )
    }

    pub fn get_task(db: &Database, task_id: &Id) -> Result<Option<StoryTask>> {
        load_payload(
            db.connection(),
            "SELECT payload_json FROM story_tasks WHERE task_id = ?1",
            task_id.as_str(),
        )
    }

    pub fn get_knowledge(
        db: &Database,
        knowledge_id: &Id,
    ) -> Result<Option<CharacterKnowledgeEntry>> {
        load_payload(
            db.connection(),
            "SELECT payload_json FROM character_knowledge WHERE knowledge_id = ?1",
            knowledge_id.as_str(),
        )
    }

    pub fn list_summaries(db: &Database, campaign_id: &Id) -> Result<Vec<RoundSummary>> {
        let mut stmt = db.connection().prepare(
            "SELECT payload_json FROM round_summaries WHERE campaign_id = ?1 ORDER BY level, turn, summary_id",
        )?;
        let rows = stmt.query_map([campaign_id.as_str()], |row| row.get::<_, String>(0))?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(serde_json::from_str(&row?)?);
        }
        Ok(summaries)
    }

    pub fn count_commit_ledger(db: &Database) -> Result<usize> {
        let count: i64 =
            db.connection()
                .query_row("SELECT COUNT(*) FROM mutation_commits", [], |row| {
                    row.get(0)
                })?;
        Ok(count as usize)
    }

    pub fn accept_turn(db: &mut Database, request: AcceptTurnRequest<'_>) -> Result<AcceptOutcome> {
        Self::accept_turn_with_fault(db, request, AcceptFault::None)
    }

    #[doc(hidden)]
    pub fn accept_turn_with_fault(
        db: &mut Database,
        request: AcceptTurnRequest<'_>,
        fault: AcceptFault,
    ) -> Result<AcceptOutcome> {
        migrations::migrate(db)?;
        let fingerprint = request_fingerprint(&request)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        if let Some(existing_hash) = tx
            .query_row(
                "SELECT payload_hash FROM mutation_commits WHERE commit_id = ?1",
                [request.batch.commit_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            if existing_hash == fingerprint {
                uow.commit()?;
                return Ok(AcceptOutcome::AlreadyCommitted);
            }
            return Err(SqliteError::Conflict(format!(
                "commit_id {} was already used with a different payload",
                request.batch.commit_id
            )));
        }

        let mut turn: TurnRecord = load_payload_tx(
            tx,
            "SELECT payload_json FROM turns WHERE turn_id = ?1",
            [request.turn_id.as_str()],
        )?
        .ok_or_else(|| SqliteError::RecordNotFound(format!("turn {}", request.turn_id)))?;
        if turn.status != TurnStatus::AwaitingAcceptance {
            return Err(SqliteError::Conflict(format!(
                "turn {} is {:?}, expected AwaitingAcceptance",
                turn.turn_id, turn.status
            )));
        }
        let attempt = turn
            .find_attempt(request.attempt_id)
            .cloned()
            .ok_or_else(|| {
                SqliteError::RecordNotFound(format!("attempt {}", request.attempt_id))
            })?;
        if attempt.status != AttemptStatus::AwaitingAcceptance {
            return Err(SqliteError::Conflict(format!(
                "attempt {} is {:?}, expected AwaitingAcceptance",
                attempt.attempt_id, attempt.status
            )));
        }
        if attempt.draft_hash != request.draft_hash {
            return Err(SqliteError::Conflict("draft_hash mismatch".into()));
        }
        let stored_batch = attempt.pending_state_changes.as_ref().ok_or_else(|| {
            SqliteError::Conflict("persisted attempt has no MutationBatch".into())
        })?;
        if batch_fingerprint(stored_batch)? != batch_fingerprint(request.batch)? {
            return Err(SqliteError::Conflict(
                "request MutationBatch does not match persisted attempt batch".into(),
            ));
        }
        if request.terminal_status != TurnStatus::Committed
            && request.terminal_status != TurnStatus::Degraded
        {
            return Err(SqliteError::Conflict(
                "terminal status must be Committed or Degraded".into(),
            ));
        }

        let mut campaign: Campaign = load_payload_tx(
            tx,
            "SELECT payload_json FROM campaigns WHERE campaign_id = ?1",
            [turn.campaign_id.as_str()],
        )?
        .ok_or_else(|| SqliteError::RecordNotFound(format!("campaign {}", turn.campaign_id)))?;
        let (structured_revision, structured_chronicle_revision): (u64, u64) = tx.query_row(
            "SELECT revision, chronicle_revision FROM campaigns WHERE campaign_id = ?1",
            [turn.campaign_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if campaign.revision != structured_revision
            || campaign.chronicle_revision != structured_chronicle_revision
        {
            return Err(SqliteError::Conflict(format!(
                "campaign {} payload revisions ({}, {}) differ from indexed revisions ({}, {})",
                campaign.id,
                campaign.revision,
                campaign.chronicle_revision,
                structured_revision,
                structured_chronicle_revision
            )));
        }
        if campaign.revision != turn.base_campaign_revision
            || campaign.revision != request.batch.expected_revision
            || request.batch.target_revision != request.batch.expected_revision.saturating_add(1)
        {
            return Err(SqliteError::Conflict(format!(
                "revision conflict: campaign={}, turn_base={}, expected={}, target={}",
                campaign.revision,
                turn.base_campaign_revision,
                request.batch.expected_revision,
                request.batch.target_revision
            )));
        }

        let mut conversation: Conversation = load_payload_tx(
            tx,
            "SELECT payload_json FROM conversations WHERE conversation_id = ?1",
            [turn.conversation_id.as_str()],
        )?
        .ok_or_else(|| {
            SqliteError::RecordNotFound(format!("conversation {}", turn.conversation_id))
        })?;
        let active_text = conversation
            .find_node(&attempt.variant_id)
            .and_then(|node| node.active())
            .map(|variant| variant.content.as_str())
            .ok_or_else(|| {
                SqliteError::RecordNotFound(format!(
                    "active conversation variant {}",
                    attempt.variant_id
                ))
            })?;
        if compute_draft_hash(active_text) != attempt.draft_hash {
            return Err(SqliteError::Conflict(
                "persisted draft_hash does not match current conversation content".into(),
            ));
        }

        for mutation in &request.batch.mutations {
            apply_mutation(tx, &mut campaign, &mut conversation, &attempt, mutation)?;
        }

        if fault == AcceptFault::AfterMutations {
            return Err(SqliteError::Other(
                "injected failure after mutations".into(),
            ));
        }

        campaign.revision = request.batch.target_revision;
        write_campaign(tx, &campaign)?;
        write_conversation(tx, &conversation)?;

        let mut committed_batch = request.batch.clone();
        committed_batch.status = MutationBatchStatus::Committed;
        turn.status = request.terminal_status.clone();
        turn.accepted_attempt_id = Some(request.attempt_id.clone());
        for item in &mut turn.attempts {
            if item.attempt_id == *request.attempt_id {
                item.status = AttemptStatus::Committed;
                item.pending_state_changes = Some(committed_batch.clone());
            } else if item.status.is_active() {
                item.status = AttemptStatus::Superseded;
            }
        }
        turn.touch();
        write_turn(tx, &turn)?;

        if fault == AcceptFault::BeforeLedger {
            return Err(SqliteError::Other("injected failure before ledger".into()));
        }

        tx.execute(
            r#"
            INSERT INTO mutation_commits (
                commit_id, campaign_id, turn_id, attempt_id, expected_revision,
                target_revision, terminal_status, payload_hash, committed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            rusqlite::params![
                request.batch.commit_id.as_str(),
                campaign.id.as_str(),
                request.turn_id.as_str(),
                request.attempt_id.as_str(),
                request.batch.expected_revision,
                request.batch.target_revision,
                enum_text(&request.terminal_status)?,
                fingerprint,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;

        uow.commit()?;
        Ok(AcceptOutcome::Applied)
    }
}

fn apply_mutation(
    tx: &Transaction<'_>,
    campaign: &mut Campaign,
    conversation: &mut Conversation,
    attempt: &TurnAttempt,
    mutation: &Mutation,
) -> Result<()> {
    match mutation {
        Mutation::SetVariable {
            instance_id,
            key,
            value,
            turn,
        } => {
            if let Some(instance_id) = instance_id {
                let mut instance: CharacterInstance = load_payload_tx(
                    tx,
                    "SELECT payload_json FROM character_instances WHERE instance_id = ?1 AND campaign_id = ?2",
                    rusqlite::params![instance_id.as_str(), campaign.id.as_str()],
                )?
                .ok_or_else(|| {
                    SqliteError::RecordNotFound(format!("instance {instance_id}"))
                })?;
                instance.set_variable(key, value.clone(), *turn);
                write_instance(tx, &instance)?;
            } else {
                campaign.set_variable(key, value.clone(), *turn);
            }
        }
        Mutation::UpsertKnowledge(mutation) => {
            let entry = mutation.to_entry();
            upsert_exact(
                tx,
                "character_knowledge",
                "knowledge_id",
                entry.id.as_str(),
                &entry,
                || {
                    tx.execute(
                        "INSERT INTO character_knowledge (knowledge_id, campaign_id, payload_json) VALUES (?1, ?2, ?3)",
                        rusqlite::params![entry.id.as_str(), entry.campaign_id.as_str(), json(&entry)?],
                    )?;
                    Ok(())
                },
            )?;
        }
        Mutation::SetTaskStatus { task_id, status } => {
            let mut task: StoryTask = load_payload_tx(
                tx,
                "SELECT payload_json FROM story_tasks WHERE task_id = ?1",
                [task_id.as_str()],
            )?
            .ok_or_else(|| SqliteError::RecordNotFound(format!("task {task_id}")))?;
            if task.campaign_id != campaign.id {
                return Err(SqliteError::Conflict(format!(
                    "task {task_id} belongs to another campaign"
                )));
            }
            task.status = status.clone();
            write_task(tx, &task)?;
        }
        Mutation::UpsertNewTask(task) => {
            let task = task.as_ref();
            if task.campaign_id != campaign.id {
                return Err(SqliteError::Conflict(format!(
                    "task {} belongs to another campaign",
                    task.id
                )));
            }
            upsert_exact(tx, "story_tasks", "task_id", task.id.as_str(), task, || {
                write_task(tx, task)
            })?;
        }
        Mutation::UpsertSummary(summary) => {
            let summary = summary.as_ref();
            if summary.campaign_id != campaign.id || summary.conversation_id != conversation.id {
                return Err(SqliteError::Conflict(format!(
                    "summary {} scope mismatch",
                    summary.id
                )));
            }
            let inserted = upsert_summary_exact(tx, summary)?;
            if inserted {
                campaign.bump_chronicle_revision();
            }
        }
        Mutation::FinalizeVariant { variant_id } => {
            if *variant_id != attempt.variant_id {
                return Err(SqliteError::Conflict(format!(
                    "FinalizeVariant {} does not match attempt variant {}",
                    variant_id, attempt.variant_id
                )));
            }
            let node = conversation.find_node_mut(variant_id).ok_or_else(|| {
                SqliteError::RecordNotFound(format!("conversation node {variant_id}"))
            })?;
            let variant = node.active_mut().ok_or_else(|| {
                SqliteError::RecordNotFound(format!("active variant for node {variant_id}"))
            })?;
            if variant.status == VariantStatus::Discarded {
                return Err(SqliteError::Conflict(format!(
                    "variant {variant_id} is discarded"
                )));
            }
            variant.status = VariantStatus::Final;
            conversation.updated_at = chrono::Utc::now();
        }
        Mutation::UpsertInstance(instance) => {
            let instance = instance.as_ref();
            if instance.campaign_id != campaign.id {
                return Err(SqliteError::Conflict(format!(
                    "instance {} belongs to another campaign",
                    instance.id
                )));
            }
            let same_name: Option<String> = tx
                .query_row(
                    "SELECT instance_id FROM character_instances WHERE campaign_id = ?1 AND name = ?2 AND instance_id <> ?3 LIMIT 1",
                    rusqlite::params![campaign.id.as_str(), instance.name, instance.id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(other) = same_name {
                return Err(SqliteError::Conflict(format!(
                    "instance name '{}' conflicts with {other}",
                    instance.name
                )));
            }
            upsert_exact(
                tx,
                "character_instances",
                "instance_id",
                instance.id.as_str(),
                instance,
                || write_instance(tx, instance),
            )?;
        }
    }
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

fn write_conversation(tx: &Transaction<'_>, conversation: &Conversation) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO conversations (
            conversation_id, campaign_id, character_id, archived_upto,
            created_at, updated_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(conversation_id) DO UPDATE SET
            campaign_id=excluded.campaign_id, character_id=excluded.character_id,
            archived_upto=excluded.archived_upto, created_at=excluded.created_at,
            updated_at=excluded.updated_at, payload_json=excluded.payload_json
        "#,
        rusqlite::params![
            conversation.id.as_str(),
            conversation.campaign_id.as_ref().map(Id::as_str),
            conversation.character_id,
            conversation.archived_upto as u64,
            conversation.created_at.to_rfc3339(),
            conversation.updated_at.to_rfc3339(),
            json(conversation)?,
        ],
    )?;
    Ok(())
}

fn write_turn(tx: &Transaction<'_>, turn: &TurnRecord) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO turns (
            turn_id, campaign_id, conversation_id, input_node_id, base_campaign_revision,
            status, accepted_attempt_id, failure_reason, created_at, updated_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(turn_id) DO UPDATE SET
            campaign_id=excluded.campaign_id, conversation_id=excluded.conversation_id,
            input_node_id=excluded.input_node_id,
            base_campaign_revision=excluded.base_campaign_revision, status=excluded.status,
            accepted_attempt_id=excluded.accepted_attempt_id,
            failure_reason=excluded.failure_reason, created_at=excluded.created_at,
            updated_at=excluded.updated_at, payload_json=excluded.payload_json
        "#,
        rusqlite::params![
            turn.turn_id.as_str(),
            turn.campaign_id.as_str(),
            turn.conversation_id.as_str(),
            turn.input_node_id.as_str(),
            turn.base_campaign_revision,
            enum_text(&turn.status)?,
            turn.accepted_attempt_id.as_ref().map(Id::as_str),
            turn.failure_reason,
            turn.created_at,
            turn.updated_at,
            json(turn)?,
        ],
    )?;
    for attempt in &turn.attempts {
        tx.execute(
            r#"
            INSERT INTO turn_attempts (
                attempt_id, turn_id, variant_id, draft_hash, status, created_at, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(attempt_id) DO UPDATE SET
                turn_id=excluded.turn_id, variant_id=excluded.variant_id,
                draft_hash=excluded.draft_hash, status=excluded.status,
                created_at=excluded.created_at, payload_json=excluded.payload_json
            "#,
            rusqlite::params![
                attempt.attempt_id.as_str(),
                turn.turn_id.as_str(),
                attempt.variant_id.as_str(),
                attempt.draft_hash,
                enum_text(&attempt.status)?,
                attempt.created_at,
                json(attempt)?,
            ],
        )?;
    }
    Ok(())
}

fn write_instance(tx: &Transaction<'_>, instance: &CharacterInstance) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO character_instances (
            instance_id, campaign_id, definition_id, name, is_temporary, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(instance_id) DO UPDATE SET
            campaign_id=excluded.campaign_id, definition_id=excluded.definition_id,
            name=excluded.name, is_temporary=excluded.is_temporary,
            payload_json=excluded.payload_json
        "#,
        rusqlite::params![
            instance.id.as_str(),
            instance.campaign_id.as_str(),
            instance.definition_id.as_ref().map(Id::as_str),
            instance.name,
            instance.is_temporary,
            json(instance)?,
        ],
    )?;
    Ok(())
}

fn write_task(tx: &Transaction<'_>, task: &StoryTask) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO story_tasks (task_id, campaign_id, payload_json)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(task_id) DO UPDATE SET
            campaign_id=excluded.campaign_id, payload_json=excluded.payload_json
        "#,
        rusqlite::params![task.id.as_str(), task.campaign_id.as_str(), json(task)?],
    )?;
    Ok(())
}

fn upsert_summary_exact(tx: &Transaction<'_>, summary: &RoundSummary) -> Result<bool> {
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
    tx.execute(
        r#"
        INSERT INTO round_summaries (
            summary_id, campaign_id, conversation_id, lineage_id, level, turn, turn_end,
            code, headline, covered_by, content, created_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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
            payload,
        ],
    )?;
    for child_id in &summary.covers {
        tx.execute(
            "INSERT OR IGNORE INTO round_summary_covers (parent_id, child_id) VALUES (?1, ?2)",
            rusqlite::params![summary.id.as_str(), child_id.as_str()],
        )?;
    }
    Ok(true)
}

fn upsert_exact<T, F>(
    tx: &Transaction<'_>,
    table: &str,
    id_column: &str,
    id: &str,
    value: &T,
    insert: F,
) -> Result<bool>
where
    T: Serialize,
    F: FnOnce() -> Result<()>,
{
    let sql = format!("SELECT payload_json FROM {table} WHERE {id_column} = ?1");
    let existing: Option<String> = tx.query_row(&sql, [id], |row| row.get(0)).optional()?;
    let payload = json(value)?;
    if let Some(existing) = existing {
        if json_payloads_equal(&existing, &payload)? {
            return Ok(false);
        }
        return Err(SqliteError::Conflict(format!(
            "{table} {id} payload mismatch"
        )));
    }
    insert()?;
    Ok(true)
}

fn load_payload<T: DeserializeOwned>(
    conn: &rusqlite::Connection,
    sql: &str,
    id: &str,
) -> Result<Option<T>> {
    let payload: Option<String> = conn.query_row(sql, [id], |row| row.get(0)).optional()?;
    payload
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}

fn load_payload_tx<T, P>(tx: &Transaction<'_>, sql: &str, params: P) -> Result<Option<T>>
where
    T: DeserializeOwned,
    P: rusqlite::Params,
{
    let payload: Option<String> = tx.query_row(sql, params, |row| row.get(0)).optional()?;
    payload
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}

fn request_fingerprint(request: &AcceptTurnRequest<'_>) -> Result<String> {
    let payload = serde_json::json!({
        "turn_id": request.turn_id,
        "attempt_id": request.attempt_id,
        "draft_hash": request.draft_hash,
        "batch": batch_payload(request.batch),
        "terminal_status": request.terminal_status,
    });
    hash_json(&payload)
}

fn batch_fingerprint(batch: &MutationBatch) -> Result<String> {
    hash_json(&batch_payload(batch))
}

fn batch_payload(batch: &MutationBatch) -> serde_json::Value {
    serde_json::json!({
        "commit_id": batch.commit_id,
        "expected_revision": batch.expected_revision,
        "target_revision": batch.target_revision,
        "mutations": batch.mutations,
    })
}

fn hash_json(payload: &serde_json::Value) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&payload)?);
    Ok(hex_encode(hasher.finalize()))
}

fn enum_text(value: &impl Serialize) -> Result<String> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| SqliteError::Other("enum did not serialize as string".into()))
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

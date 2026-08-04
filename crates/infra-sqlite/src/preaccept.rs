//! Typed, transactional SQLite repository for **pre-accept** Turn lifecycle writes.
//!
//! Covers AI draft creation, Attempt intermediate state, autofix / postprocess
//! write-back, and recovery lookups. All write APIs require an explicit
//! [`Database`] so callers cannot accidentally dual-write JSON.
//!
//! Accept / Chronicle publication remain in [`crate::production`] and
//! [`crate::publication`]. This module never mutates Campaign revision or the
//! accept commit ledger.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use storyforge_domain::Id;
use storyforge_domain::campaign::CharacterInstance;
use storyforge_domain::conversation::{
    Conversation, MessageVariant, Provenance, Role, VariantStatus,
};
use storyforge_domain::turn::{
    AttemptStatus, DerivationComponents, MutationBatch, QualityReport, TurnAttempt, TurnRecord,
    TurnStatus,
};

use crate::connection::Database;
use crate::error::{Result, SqliteError};
use crate::migrations;
use crate::production::{
    compute_draft_hash, enum_text, load_payload_tx, load_validated_turn, write_conversation,
    write_turn,
};
use crate::unit_of_work::UnitOfWork;

/// Test-only fault injection points inside pre-accept transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreacceptFault {
    #[default]
    None,
    /// Fail after all business writes, before COMMIT (full rollback).
    BeforeCommit,
    /// Fail after conversation write but before turn/outbox finalize.
    AfterConversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreacceptOutboxKind {
    DraftReady,
    AutofixSync,
    PostprocessApply,
    Regenerate,
    EditStale,
    RecoveryFail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreacceptOutboxStatus {
    Pending,
    Applied,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreacceptOutboxRow {
    pub outbox_id: Id,
    pub campaign_id: Id,
    pub conversation_id: Id,
    pub turn_id: Id,
    pub attempt_id: Id,
    pub kind: PreacceptOutboxKind,
    pub draft_hash: String,
    pub payload_hash: String,
    pub status: PreacceptOutboxStatus,
    pub payload_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct DraftAttemptRequest<'a> {
    pub campaign_id: &'a Id,
    pub conversation_id: &'a Id,
    pub turn_id: &'a Id,
    pub attempt_id: &'a Id,
    pub draft_text: &'a str,
    pub pending_temporary_instances: Vec<CharacterInstance>,
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone)]
pub struct DraftAttemptOutcome {
    pub attempt_id: Id,
    pub variant_id: Id,
    pub draft_hash: String,
}

#[derive(Debug, Clone)]
pub struct AutofixSyncRequest<'a> {
    pub campaign_id: &'a Id,
    pub conversation_id: &'a Id,
    pub turn_id: &'a Id,
    pub attempt_id: &'a Id,
    pub final_text: &'a str,
    pub quality_report: QualityReport,
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone)]
pub struct PostprocessApplyRequest<'a> {
    pub campaign_id: &'a Id,
    pub conversation_id: &'a Id,
    pub turn_id: &'a Id,
    pub attempt_id: &'a Id,
    pub batch: Option<MutationBatch>,
    pub derivation: DerivationComponents,
}

/// Result of a pre-accept postprocess write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostprocessApplyOutcome {
    /// Candidate batch/derivation written and outbox Applied.
    Applied,
    /// Identical payload already applied; no mutation and no new outbox row.
    AlreadyApplied,
    /// Attempt belongs to the turn but is no longer the current writable attempt.
    SkippedLate,
}

#[derive(Debug, Clone)]
pub struct RegenerateAttemptRequest<'a> {
    pub campaign_id: &'a Id,
    pub conversation_id: &'a Id,
    pub turn_id: &'a Id,
    pub previous_variant_id: &'a Id,
    pub attempt_id: &'a Id,
    pub draft_text: &'a str,
    pub pending_temporary_instances: Vec<CharacterInstance>,
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone)]
pub struct PreacceptRecoverySnapshot {
    pub turns: Vec<TurnRecord>,
    pub outbox: Vec<PreacceptOutboxRow>,
}

pub struct SqlitePreacceptRepository;

impl SqlitePreacceptRepository {
    pub fn create_draft_attempt(
        db: &mut Database,
        request: DraftAttemptRequest<'_>,
    ) -> Result<DraftAttemptOutcome> {
        Self::create_draft_attempt_with_fault(db, request, PreacceptFault::None)
    }

    #[doc(hidden)]
    pub fn create_draft_attempt_with_fault(
        db: &mut Database,
        request: DraftAttemptRequest<'_>,
        fault: PreacceptFault,
    ) -> Result<DraftAttemptOutcome> {
        migrations::migrate(db)?;
        let draft_hash = compute_draft_hash(request.draft_text);
        let now = chrono::Utc::now().to_rfc3339();
        let outbox_id = Id::new();
        let payload = serde_json::json!({
            "attempt_id": request.attempt_id.as_str(),
            "draft_hash": draft_hash,
            "draft_text_len": request.draft_text.chars().count(),
        });
        let payload_hash = hash_json(&payload)?;

        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        let mut turn = load_turn_tx(tx, request.turn_id)?;
        validate_turn_scope(
            &turn,
            request.campaign_id,
            request.conversation_id,
            request.turn_id,
        )?;
        if turn.status != TurnStatus::Generating {
            // First draft may only attach while Generating; regenerate uses a different API.
            return Err(SqliteError::Conflict(format!(
                "turn {} is {:?}, expected Generating for first draft",
                turn.turn_id, turn.status
            )));
        }
        if turn.attempts.iter().any(|a| a.status.is_active()) {
            return Err(SqliteError::Conflict(format!(
                "turn {} already has an active attempt",
                turn.turn_id
            )));
        }
        if attempt_exists_tx(tx, request.attempt_id)? {
            return Err(SqliteError::Conflict(format!(
                "attempt {} already exists",
                request.attempt_id
            )));
        }

        let mut conversation = load_conversation_tx(tx, request.conversation_id)?;
        validate_conversation_scope(&conversation, request.campaign_id, request.conversation_id)?;

        let variant_id = append_ai_draft_node(
            &mut conversation,
            request.draft_text,
            request.provenance.clone(),
        );
        write_conversation(tx, &conversation)?;
        if fault == PreacceptFault::AfterConversation {
            return Err(SqliteError::Other(
                "injected failure after conversation write".into(),
            ));
        }

        let attempt = TurnAttempt {
            attempt_id: request.attempt_id.clone(),
            variant_id: variant_id.clone(),
            draft_hash: draft_hash.clone(),
            status: AttemptStatus::DraftReady,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: request.pending_temporary_instances.clone(),
            provenance: request.provenance.clone(),
            created_at: now.clone(),
        };
        turn.attempts.push(attempt);
        turn.status = TurnStatus::DraftReady;
        turn.touch();
        write_turn(tx, &turn)?;

        insert_outbox(
            tx,
            &PreacceptOutboxRow {
                outbox_id,
                campaign_id: request.campaign_id.clone(),
                conversation_id: request.conversation_id.clone(),
                turn_id: request.turn_id.clone(),
                attempt_id: request.attempt_id.clone(),
                kind: PreacceptOutboxKind::DraftReady,
                draft_hash: draft_hash.clone(),
                payload_hash,
                status: PreacceptOutboxStatus::Applied,
                payload_json: payload.to_string(),
                created_at: now.clone(),
                updated_at: now,
            },
        )?;

        if fault == PreacceptFault::BeforeCommit {
            return Err(SqliteError::Other("injected failure before commit".into()));
        }

        uow.commit()?;
        Ok(DraftAttemptOutcome {
            attempt_id: request.attempt_id.clone(),
            variant_id,
            draft_hash,
        })
    }

    pub fn sync_autofix(db: &mut Database, request: AutofixSyncRequest<'_>) -> Result<()> {
        Self::sync_autofix_with_fault(db, request, PreacceptFault::None)
    }

    #[doc(hidden)]
    pub fn sync_autofix_with_fault(
        db: &mut Database,
        request: AutofixSyncRequest<'_>,
        fault: PreacceptFault,
    ) -> Result<()> {
        migrations::migrate(db)?;
        let final_hash = compute_draft_hash(request.final_text);
        let now = chrono::Utc::now().to_rfc3339();
        let provenance_hash = match request.provenance.as_ref() {
            Some(provenance) => Some(hash_json(&serde_json::to_value(provenance)?)?),
            None => None,
        };
        let payload = serde_json::json!({
            "attempt_id": request.attempt_id.as_str(),
            "draft_hash": final_hash,
            "quality_report": request.quality_report,
            "provenance_hash": provenance_hash,
        });
        let payload_hash = hash_json(&payload)?;

        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        let mut turn = load_turn_tx(tx, request.turn_id)?;
        validate_turn_scope(
            &turn,
            request.campaign_id,
            request.conversation_id,
            request.turn_id,
        )?;
        if !matches!(
            turn.status,
            TurnStatus::DraftReady | TurnStatus::DerivingState
        ) {
            return Err(SqliteError::Conflict(format!(
                "turn {} is {:?}, not writable for autofix",
                turn.turn_id, turn.status
            )));
        }
        let active = turn.active_attempt().ok_or_else(|| {
            SqliteError::Conflict(format!("turn {} has no active attempt", turn.turn_id))
        })?;
        if &active.attempt_id != request.attempt_id {
            return Err(SqliteError::Conflict(format!(
                "attempt {} is not the active attempt on turn {}",
                request.attempt_id, turn.turn_id
            )));
        }
        let variant_id = active.variant_id.clone();

        // Idempotent: same final hash + full canonical QualityReport already applied.
        if let Some(existing) =
            find_applied_outbox_tx(tx, request.attempt_id, PreacceptOutboxKind::AutofixSync)?
            && existing.payload_hash == payload_hash
        {
            uow.commit()?;
            return Ok(());
        }
        // Different payload after a previous autofix overwrites (single writer).

        let mut conversation = load_conversation_tx(tx, request.conversation_id)?;
        validate_conversation_scope(&conversation, request.campaign_id, request.conversation_id)?;
        {
            let node = conversation
                .find_node_mut(&variant_id)
                .ok_or_else(|| SqliteError::RecordNotFound(format!("variant {variant_id}")))?;
            node.edit_active(request.final_text.to_string())
                .map_err(SqliteError::Other)?;
            if let Some(provenance) = request.provenance.clone()
                && let Some(active) = node.active_mut()
            {
                active.provenance = Some(provenance);
            }
        }
        conversation.updated_at = chrono::Utc::now();
        write_conversation(tx, &conversation)?;
        if fault == PreacceptFault::AfterConversation {
            return Err(SqliteError::Other(
                "injected failure after conversation write".into(),
            ));
        }

        {
            let attempt = turn.find_attempt_mut(request.attempt_id).ok_or_else(|| {
                SqliteError::RecordNotFound(format!("attempt {}", request.attempt_id))
            })?;
            // Stale is terminal and cannot be the active attempt; reactivation is
            // intentionally unsupported. Callers must regenerate a fresh attempt.
            attempt.draft_hash = final_hash.clone();
            attempt.quality_report = Some(request.quality_report.clone());
            if let Some(provenance) = request.provenance.clone() {
                attempt.provenance = Some(provenance);
            }
        }
        turn.touch();
        write_turn(tx, &turn)?;

        upsert_outbox_applied(
            tx,
            request.campaign_id,
            request.conversation_id,
            request.turn_id,
            request.attempt_id,
            PreacceptOutboxKind::AutofixSync,
            &final_hash,
            &payload_hash,
            &payload.to_string(),
            &now,
        )?;

        if fault == PreacceptFault::BeforeCommit {
            return Err(SqliteError::Other("injected failure before commit".into()));
        }
        uow.commit()?;
        Ok(())
    }

    /// Apply postprocess output onto the still-current attempt.
    ///
    /// Ownership and replay rules:
    /// - `attempt_id` must belong to `turn_id` (cross-turn attempt → conflict, zero outbox).
    /// - Identical already-applied payload → [`PostprocessApplyOutcome::AlreadyApplied`]
    ///   with no new outbox row.
    /// - Different payload after an Applied row → conflict, zero new outbox.
    /// - Belonging but non-current attempt → [`PostprocessApplyOutcome::SkippedLate`].
    pub fn apply_postprocess(
        db: &mut Database,
        request: PostprocessApplyRequest<'_>,
    ) -> Result<PostprocessApplyOutcome> {
        Self::apply_postprocess_with_fault(db, request, PreacceptFault::None)
    }

    #[doc(hidden)]
    pub fn apply_postprocess_with_fault(
        db: &mut Database,
        request: PostprocessApplyRequest<'_>,
        fault: PreacceptFault,
    ) -> Result<PostprocessApplyOutcome> {
        migrations::migrate(db)?;
        let now = chrono::Utc::now().to_rfc3339();
        let batch_fingerprint = request.batch.as_ref().map(batch_payload_hash).transpose()?;
        let payload = serde_json::json!({
            "attempt_id": request.attempt_id.as_str(),
            "batch": batch_fingerprint,
            "summary_derivation": enum_text(&request.derivation.summary_derivation)?,
            "state_derivation": enum_text(&request.derivation.state_derivation)?,
        });
        let payload_hash = hash_json(&payload)?;

        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        let mut turn = load_turn_tx(tx, request.turn_id)?;
        validate_turn_scope(
            &turn,
            request.campaign_id,
            request.conversation_id,
            request.turn_id,
        )?;

        // Fail closed on cross-turn / missing attempt ownership before any outbox write.
        let owned = turn.find_attempt(request.attempt_id).cloned();
        let Some(owned_attempt) = owned else {
            return Err(SqliteError::Conflict(format!(
                "attempt {} does not belong to turn {}",
                request.attempt_id, request.turn_id
            )));
        };

        // Replay / conflict checks must run before the late-skip path, otherwise a
        // successful AwaitingAcceptance state would be misclassified as Skipped.
        if let Some(existing) = find_applied_outbox_tx(
            tx,
            request.attempt_id,
            PreacceptOutboxKind::PostprocessApply,
        )? {
            if existing.payload_hash == payload_hash {
                if fault == PreacceptFault::BeforeCommit {
                    return Err(SqliteError::Other("injected failure before commit".into()));
                }
                uow.commit()?;
                return Ok(PostprocessApplyOutcome::AlreadyApplied);
            }
            return Err(SqliteError::Conflict(format!(
                "postprocess for attempt {} already applied with different payload",
                request.attempt_id
            )));
        }

        let ready = matches!(
            turn.status,
            TurnStatus::DraftReady | TurnStatus::DerivingState
        ) && turn.active_attempt().is_some_and(|a| {
            &a.attempt_id == request.attempt_id
                && matches!(
                    a.status,
                    AttemptStatus::DraftReady | AttemptStatus::DerivingState
                )
        });
        if !ready {
            insert_outbox(
                tx,
                &PreacceptOutboxRow {
                    outbox_id: Id::new(),
                    campaign_id: request.campaign_id.clone(),
                    conversation_id: request.conversation_id.clone(),
                    turn_id: request.turn_id.clone(),
                    attempt_id: request.attempt_id.clone(),
                    kind: PreacceptOutboxKind::PostprocessApply,
                    draft_hash: owned_attempt.draft_hash.clone(),
                    payload_hash: payload_hash.clone(),
                    status: PreacceptOutboxStatus::Skipped,
                    payload_json: payload.to_string(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )?;
            if fault == PreacceptFault::BeforeCommit {
                return Err(SqliteError::Other("injected failure before commit".into()));
            }
            uow.commit()?;
            return Ok(PostprocessApplyOutcome::SkippedLate);
        }

        let draft_hash = {
            let attempt = turn.find_attempt_mut(request.attempt_id).ok_or_else(|| {
                SqliteError::RecordNotFound(format!("attempt {}", request.attempt_id))
            })?;
            attempt.pending_state_changes = request.batch.clone();
            attempt.derivation = Some(request.derivation.clone());
            attempt.status = AttemptStatus::AwaitingAcceptance;
            attempt.draft_hash.clone()
        };
        turn.status = TurnStatus::AwaitingAcceptance;
        turn.touch();
        write_turn(tx, &turn)?;

        let conversation = load_conversation_tx(tx, request.conversation_id)?;
        validate_conversation_scope(&conversation, request.campaign_id, request.conversation_id)?;

        upsert_outbox_applied(
            tx,
            request.campaign_id,
            request.conversation_id,
            request.turn_id,
            request.attempt_id,
            PreacceptOutboxKind::PostprocessApply,
            &draft_hash,
            &payload_hash,
            &payload.to_string(),
            &now,
        )?;

        if fault == PreacceptFault::BeforeCommit {
            return Err(SqliteError::Other("injected failure before commit".into()));
        }
        uow.commit()?;
        Ok(PostprocessApplyOutcome::Applied)
    }

    pub fn append_regenerate_attempt(
        db: &mut Database,
        request: RegenerateAttemptRequest<'_>,
    ) -> Result<DraftAttemptOutcome> {
        Self::append_regenerate_attempt_with_fault(db, request, PreacceptFault::None)
    }

    #[doc(hidden)]
    pub fn append_regenerate_attempt_with_fault(
        db: &mut Database,
        request: RegenerateAttemptRequest<'_>,
        fault: PreacceptFault,
    ) -> Result<DraftAttemptOutcome> {
        migrations::migrate(db)?;
        let draft_hash = compute_draft_hash(request.draft_text);
        let now = chrono::Utc::now().to_rfc3339();
        let payload = serde_json::json!({
            "attempt_id": request.attempt_id.as_str(),
            "previous_variant_id": request.previous_variant_id.as_str(),
            "draft_hash": draft_hash,
        });
        let payload_hash = hash_json(&payload)?;

        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        let mut turn = load_turn_tx(tx, request.turn_id)?;
        validate_turn_scope(
            &turn,
            request.campaign_id,
            request.conversation_id,
            request.turn_id,
        )?;
        if turn.status.is_terminal() {
            return Err(SqliteError::Conflict(format!(
                "turn {} is terminal ({:?}); cannot regenerate",
                turn.turn_id, turn.status
            )));
        }
        if turn.status == TurnStatus::Committing {
            return Err(SqliteError::Conflict(format!(
                "turn {} is Committing; cannot regenerate",
                turn.turn_id
            )));
        }
        if attempt_exists_tx(tx, request.attempt_id)? {
            return Err(SqliteError::Conflict(format!(
                "attempt {} already exists",
                request.attempt_id
            )));
        }

        let active = turn.active_attempt().ok_or_else(|| {
            SqliteError::Conflict(format!(
                "turn {} has no active attempt to regenerate",
                turn.turn_id
            ))
        })?;
        if active.status == AttemptStatus::Committing {
            return Err(SqliteError::Conflict(format!(
                "active attempt {} is Committing; cannot regenerate",
                active.attempt_id
            )));
        }
        if &active.variant_id != request.previous_variant_id {
            return Err(SqliteError::Conflict(format!(
                "previous_variant_id {} is not the active attempt variant {} on turn {}",
                request.previous_variant_id, active.variant_id, turn.turn_id
            )));
        }

        let mut conversation = load_conversation_tx(tx, request.conversation_id)?;
        validate_conversation_scope(&conversation, request.campaign_id, request.conversation_id)?;

        // Supersede previous active draft on the same node, then add a new Draft variant.
        {
            let node = conversation
                .find_node_mut(request.previous_variant_id)
                .ok_or_else(|| {
                    SqliteError::RecordNotFound(format!("variant {}", request.previous_variant_id))
                })?;
            // Soft-delete current active only if it is still a Draft (keep Finals).
            if node
                .active()
                .is_some_and(|v| v.status == VariantStatus::Draft)
            {
                node.soft_delete_active().map_err(SqliteError::Other)?;
            }
            let variant = MessageVariant {
                id: Id::new(),
                role: Role::Assistant,
                content: request.draft_text.to_string(),
                created_at: chrono::Utc::now(),
                status: VariantStatus::Draft,
                provenance: request.provenance.clone(),
            };
            node.add_variant(variant);
        }
        conversation.updated_at = chrono::Utc::now();
        write_conversation(tx, &conversation)?;
        if fault == PreacceptFault::AfterConversation {
            return Err(SqliteError::Other(
                "injected failure after conversation write".into(),
            ));
        }

        for att in &mut turn.attempts {
            if att.status.is_active() {
                att.status = AttemptStatus::Superseded;
            }
        }
        let attempt = TurnAttempt {
            attempt_id: request.attempt_id.clone(),
            variant_id: request.previous_variant_id.clone(),
            draft_hash: draft_hash.clone(),
            status: AttemptStatus::DraftReady,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: request.pending_temporary_instances.clone(),
            provenance: request.provenance.clone(),
            created_at: now.clone(),
        };
        turn.attempts.push(attempt);
        turn.status = TurnStatus::DraftReady;
        turn.touch();
        write_turn(tx, &turn)?;

        insert_outbox(
            tx,
            &PreacceptOutboxRow {
                outbox_id: Id::new(),
                campaign_id: request.campaign_id.clone(),
                conversation_id: request.conversation_id.clone(),
                turn_id: request.turn_id.clone(),
                attempt_id: request.attempt_id.clone(),
                kind: PreacceptOutboxKind::Regenerate,
                draft_hash: draft_hash.clone(),
                payload_hash,
                status: PreacceptOutboxStatus::Applied,
                payload_json: payload.to_string(),
                created_at: now.clone(),
                updated_at: now,
            },
        )?;

        if fault == PreacceptFault::BeforeCommit {
            return Err(SqliteError::Other("injected failure before commit".into()));
        }
        uow.commit()?;
        Ok(DraftAttemptOutcome {
            attempt_id: request.attempt_id.clone(),
            variant_id: request.previous_variant_id.clone(),
            draft_hash,
        })
    }

    /// Edit conversation content and mark the active attempt Stale in one UoW.
    ///
    /// Keeps the original `draft_hash` so Accept detects mismatch.
    pub fn mark_stale_after_edit(
        db: &mut Database,
        campaign_id: &Id,
        conversation_id: &Id,
        turn_id: &Id,
        attempt_id: &Id,
        new_content: &str,
    ) -> Result<()> {
        migrations::migrate(db)?;
        let now = chrono::Utc::now().to_rfc3339();
        let payload = serde_json::json!({
            "attempt_id": attempt_id.as_str(),
            "new_content_len": new_content.chars().count(),
        });
        let payload_hash = hash_json(&payload)?;

        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        let mut turn = load_turn_tx(tx, turn_id)?;
        validate_turn_scope(&turn, campaign_id, conversation_id, turn_id)?;
        if turn.status == TurnStatus::Committing {
            return Err(SqliteError::Conflict(format!(
                "turn {turn_id} is Committing; cannot mark stale"
            )));
        }
        if turn.status.is_terminal() {
            return Err(SqliteError::Conflict(format!(
                "turn {turn_id} is terminal ({:?}); cannot mark stale",
                turn.status
            )));
        }
        let attempt = turn
            .find_attempt(attempt_id)
            .ok_or_else(|| SqliteError::RecordNotFound(format!("attempt {attempt_id}")))?;
        if attempt.status == AttemptStatus::Committing {
            return Err(SqliteError::Conflict(format!(
                "attempt {attempt_id} is Committing; cannot mark stale"
            )));
        }
        if !attempt.status.is_active() && attempt.status != AttemptStatus::Stale {
            return Err(SqliteError::Conflict(format!(
                "attempt {attempt_id} is {:?}, cannot mark stale",
                attempt.status
            )));
        }
        let variant_id = attempt.variant_id.clone();
        let original_hash = attempt.draft_hash.clone();

        let mut conversation = load_conversation_tx(tx, conversation_id)?;
        validate_conversation_scope(&conversation, campaign_id, conversation_id)?;
        {
            let node = conversation
                .find_node_mut(&variant_id)
                .ok_or_else(|| SqliteError::RecordNotFound(format!("variant {variant_id}")))?;
            node.edit_active(new_content.to_string())
                .map_err(SqliteError::Other)?;
        }
        conversation.updated_at = chrono::Utc::now();
        write_conversation(tx, &conversation)?;

        {
            // H-5：line 724 已 ok_or_else 校验过 attempt 存在，但中间 write_conversation
            // 写盘——同函数其他分支都用 `?`+ok_or_else，唯独此处 unwrap 不一致，且
            // 未来若插入 reload 不变式会静默失效。改为 fail-closed 的 RecordNotFound
            // （与该 crate 设计一致），避免生产路径 panic = 进程 abort。
            let attempt = turn
                .find_attempt_mut(attempt_id)
                .ok_or_else(|| SqliteError::RecordNotFound(format!("attempt {attempt_id}")))?;
            // Keep original draft_hash deliberately so Accept can detect edit mismatch.
            attempt.status = AttemptStatus::Stale;
        }
        turn.touch();
        write_turn(tx, &turn)?;

        insert_outbox(
            tx,
            &PreacceptOutboxRow {
                outbox_id: Id::new(),
                campaign_id: campaign_id.clone(),
                conversation_id: conversation_id.clone(),
                turn_id: turn_id.clone(),
                attempt_id: attempt_id.clone(),
                kind: PreacceptOutboxKind::EditStale,
                draft_hash: original_hash,
                payload_hash,
                status: PreacceptOutboxStatus::Applied,
                payload_json: payload.to_string(),
                created_at: now.clone(),
                updated_at: now,
            },
        )?;
        uow.commit()?;
        Ok(())
    }

    pub fn list_outbox_for_turn(db: &Database, turn_id: &Id) -> Result<Vec<PreacceptOutboxRow>> {
        list_outbox(
            db.connection(),
            "SELECT outbox_id, campaign_id, conversation_id, turn_id, attempt_id, kind, draft_hash, payload_hash, status, payload_json, created_at, updated_at FROM preaccept_outbox WHERE turn_id = ?1 ORDER BY created_at, outbox_id",
            [turn_id.as_str()],
        )
    }

    pub fn recover_active_state(
        db: &Database,
        campaign_id: &Id,
    ) -> Result<PreacceptRecoverySnapshot> {
        let active = SqliteProductionRepositoryActiveTurns::list_for_campaign(db, campaign_id)?;
        let mut outbox = Vec::new();
        for turn in &active {
            outbox.extend(Self::list_outbox_for_turn(db, &turn.turn_id)?);
        }
        Ok(PreacceptRecoverySnapshot {
            turns: active,
            outbox,
        })
    }

    /// Fail every non-terminal pre-accept turn/attempt and mark open outbox Failed.
    ///
    /// Used by SQLite startup recovery where pre-accept has no multi-file journal.
    pub fn fail_incomplete_preaccept(db: &mut Database) -> Result<usize> {
        migrations::migrate(db)?;
        let active = crate::production::SqliteProductionRepository::list_active_turns(db)?;
        if active.is_empty() {
            return Ok(0);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;
        let mut count = 0usize;
        for mut turn in active {
            // Only fail pre-accept / incomplete states; leave Committing to production recovery.
            if turn.status == TurnStatus::Committing {
                continue;
            }
            turn.status = TurnStatus::Failed;
            turn.failure_reason = Some(
                "sqlite preaccept recovery: incomplete turn failed after process restart".into(),
            );
            for attempt in &mut turn.attempts {
                if matches!(
                    attempt.status,
                    AttemptStatus::Generating
                        | AttemptStatus::DraftReady
                        | AttemptStatus::DerivingState
                        | AttemptStatus::AwaitingAcceptance
                ) {
                    attempt.status = AttemptStatus::Failed;
                }
            }
            turn.touch();
            write_turn(tx, &turn)?;
            tx.execute(
                r#"
                UPDATE preaccept_outbox
                SET status = 'failed', updated_at = ?2
                WHERE turn_id = ?1 AND status = 'pending'
                "#,
                rusqlite::params![turn.turn_id.as_str(), now],
            )?;
            // Audit row for recovery action (best-effort; one per turn).
            if let Some(attempt) = turn.attempts.last() {
                insert_outbox(
                    tx,
                    &PreacceptOutboxRow {
                        outbox_id: Id::new(),
                        campaign_id: turn.campaign_id.clone(),
                        conversation_id: turn.conversation_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        attempt_id: attempt.attempt_id.clone(),
                        kind: PreacceptOutboxKind::RecoveryFail,
                        draft_hash: attempt.draft_hash.clone(),
                        payload_hash: hash_json(&serde_json::json!({
                            "turn_id": turn.turn_id.as_str(),
                            "reason": "startup_fail_incomplete",
                        }))?,
                        status: PreacceptOutboxStatus::Failed,
                        payload_json: "{}".into(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )?;
            }
            count += 1;
        }
        uow.commit()?;
        Ok(count)
    }
}

/// Thin helper so recovery can list active turns without re-exporting query SQL.
struct SqliteProductionRepositoryActiveTurns;
impl SqliteProductionRepositoryActiveTurns {
    fn list_for_campaign(db: &Database, campaign_id: &Id) -> Result<Vec<TurnRecord>> {
        match crate::production::SqliteProductionRepository::get_active_turn(db, campaign_id)? {
            Some(turn) => Ok(vec![turn]),
            None => Ok(vec![]),
        }
    }
}

fn load_turn_tx(tx: &Transaction<'_>, turn_id: &Id) -> Result<TurnRecord> {
    load_validated_turn(tx, turn_id)?
        .ok_or_else(|| SqliteError::RecordNotFound(format!("turn {turn_id}")))
}

fn load_conversation_tx(tx: &Transaction<'_>, conversation_id: &Id) -> Result<Conversation> {
    load_payload_tx(
        tx,
        "SELECT payload_json FROM conversations WHERE conversation_id = ?1",
        [conversation_id.as_str()],
    )?
    .ok_or_else(|| SqliteError::RecordNotFound(format!("conversation {conversation_id}")))
}

fn validate_turn_scope(
    turn: &TurnRecord,
    campaign_id: &Id,
    conversation_id: &Id,
    turn_id: &Id,
) -> Result<()> {
    if &turn.turn_id != turn_id {
        return Err(SqliteError::Conflict(format!(
            "turn scope mismatch: expected {turn_id}, got {}",
            turn.turn_id
        )));
    }
    if &turn.campaign_id != campaign_id {
        return Err(SqliteError::Conflict(format!(
            "campaign scope mismatch: turn belongs to {}, request {}",
            turn.campaign_id, campaign_id
        )));
    }
    if &turn.conversation_id != conversation_id {
        return Err(SqliteError::Conflict(format!(
            "conversation scope mismatch: turn belongs to {}, request {}",
            turn.conversation_id, conversation_id
        )));
    }
    Ok(())
}

fn validate_conversation_scope(
    conversation: &Conversation,
    campaign_id: &Id,
    conversation_id: &Id,
) -> Result<()> {
    if &conversation.id != conversation_id {
        return Err(SqliteError::Conflict(format!(
            "conversation id drift: structured {}, payload {}",
            conversation_id, conversation.id
        )));
    }
    match &conversation.campaign_id {
        Some(cid) if cid == campaign_id => Ok(()),
        Some(cid) => Err(SqliteError::Conflict(format!(
            "conversation {conversation_id} campaign scope mismatch: {cid} != {campaign_id}"
        ))),
        None => Err(SqliteError::Conflict(format!(
            "conversation {conversation_id} has no campaign_id; refusing preaccept write"
        ))),
    }
}

fn attempt_exists_tx(tx: &Transaction<'_>, attempt_id: &Id) -> Result<bool> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM turn_attempts WHERE attempt_id = ?1)",
        [attempt_id.as_str()],
        |row| row.get(0),
    )?;
    Ok(exists)
}

fn append_ai_draft_node(
    conversation: &mut Conversation,
    content: &str,
    provenance: Option<Provenance>,
) -> Id {
    conversation.append_ai_draft(content.to_string(), provenance)
}

fn insert_outbox(tx: &Transaction<'_>, row: &PreacceptOutboxRow) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO preaccept_outbox (
            outbox_id, campaign_id, conversation_id, turn_id, attempt_id,
            kind, draft_hash, payload_hash, status, payload_json, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        rusqlite::params![
            row.outbox_id.as_str(),
            row.campaign_id.as_str(),
            row.conversation_id.as_str(),
            row.turn_id.as_str(),
            row.attempt_id.as_str(),
            enum_text(&row.kind)?,
            row.draft_hash,
            row.payload_hash,
            enum_text(&row.status)?,
            row.payload_json,
            row.created_at,
            row.updated_at,
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn upsert_outbox_applied(
    tx: &Transaction<'_>,
    campaign_id: &Id,
    conversation_id: &Id,
    turn_id: &Id,
    attempt_id: &Id,
    kind: PreacceptOutboxKind,
    draft_hash: &str,
    payload_hash: &str,
    payload_json: &str,
    now: &str,
) -> Result<()> {
    // Close any pending row for this attempt+kind, then insert applied.
    tx.execute(
        r#"
        UPDATE preaccept_outbox
        SET status = 'applied', updated_at = ?3, payload_hash = ?4, payload_json = ?5, draft_hash = ?6
        WHERE attempt_id = ?1 AND kind = ?2 AND status = 'pending'
        "#,
        rusqlite::params![
            attempt_id.as_str(),
            enum_text(&kind)?,
            now,
            payload_hash,
            payload_json,
            draft_hash,
        ],
    )?;
    let pending_updated = tx.changes();
    if pending_updated > 0 {
        return Ok(());
    }
    insert_outbox(
        tx,
        &PreacceptOutboxRow {
            outbox_id: Id::new(),
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            kind,
            draft_hash: draft_hash.to_string(),
            payload_hash: payload_hash.to_string(),
            status: PreacceptOutboxStatus::Applied,
            payload_json: payload_json.to_string(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
}

fn find_applied_outbox_tx(
    tx: &Transaction<'_>,
    attempt_id: &Id,
    kind: PreacceptOutboxKind,
) -> Result<Option<PreacceptOutboxRow>> {
    let kind_text = enum_text(&kind)?;
    let row = tx
        .query_row(
            r#"
            SELECT outbox_id, campaign_id, conversation_id, turn_id, attempt_id, kind,
                   draft_hash, payload_hash, status, payload_json, created_at, updated_at
            FROM preaccept_outbox
            WHERE attempt_id = ?1 AND kind = ?2 AND status = 'applied'
            ORDER BY updated_at DESC, outbox_id DESC
            LIMIT 1
            "#,
            rusqlite::params![attempt_id.as_str(), kind_text],
            map_outbox_row,
        )
        .optional()?;
    Ok(row)
}

fn list_outbox(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<PreacceptOutboxRow>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, map_outbox_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn map_outbox_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PreacceptOutboxRow> {
    let kind: String = row.get(5)?;
    let status: String = row.get(8)?;
    Ok(PreacceptOutboxRow {
        outbox_id: Id::from_str(row.get::<_, String>(0)?),
        campaign_id: Id::from_str(row.get::<_, String>(1)?),
        conversation_id: Id::from_str(row.get::<_, String>(2)?),
        turn_id: Id::from_str(row.get::<_, String>(3)?),
        attempt_id: Id::from_str(row.get::<_, String>(4)?),
        kind: parse_outbox_kind(&kind).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
        })?,
        draft_hash: row.get(6)?,
        payload_hash: row.get(7)?,
        status: parse_outbox_status(&status).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, e.into())
        })?,
        payload_json: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn parse_outbox_kind(text: &str) -> std::result::Result<PreacceptOutboxKind, String> {
    match text {
        "draft_ready" => Ok(PreacceptOutboxKind::DraftReady),
        "autofix_sync" => Ok(PreacceptOutboxKind::AutofixSync),
        "postprocess_apply" => Ok(PreacceptOutboxKind::PostprocessApply),
        "regenerate" => Ok(PreacceptOutboxKind::Regenerate),
        "edit_stale" => Ok(PreacceptOutboxKind::EditStale),
        "recovery_fail" => Ok(PreacceptOutboxKind::RecoveryFail),
        other => Err(format!("unknown preaccept outbox kind: {other}")),
    }
}

fn parse_outbox_status(text: &str) -> std::result::Result<PreacceptOutboxStatus, String> {
    match text {
        "pending" => Ok(PreacceptOutboxStatus::Pending),
        "applied" => Ok(PreacceptOutboxStatus::Applied),
        "failed" => Ok(PreacceptOutboxStatus::Failed),
        "skipped" => Ok(PreacceptOutboxStatus::Skipped),
        other => Err(format!("unknown preaccept outbox status: {other}")),
    }
}

fn batch_payload_hash(batch: &MutationBatch) -> Result<String> {
    let payload = serde_json::json!({
        "commit_id": batch.commit_id.as_str(),
        "expected_revision": batch.expected_revision,
        "target_revision": batch.target_revision,
        "mutations": batch.mutations,
    });
    hash_json(&payload)
}

fn hash_json(payload: &serde_json::Value) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(payload)?);
    Ok(hex_encode(hasher.finalize()))
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

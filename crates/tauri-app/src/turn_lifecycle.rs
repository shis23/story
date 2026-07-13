//! Shared Turn lifecycle application service.
//!
//! Owns the production sequence from a finished pipeline draft through quality
//! handling, postprocess attachment, MutationBatch preparation, accept/commit
//! and startup recovery. Tauri command handlers and harness `CommitProbe` must
//! call this module instead of re-implementing the state machine.

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::CharacterInstance;
use storyforge_domain::turn::{
    AttemptStatus, Mutation, MutationBatch, QualityAcceptDecision, QualityReport, QualitySeverity,
    TurnAttempt, TurnRecord, TurnStatus, quality_accept_decision,
};

use crate::campaign_store::CampaignStore;
use crate::turn_coordinator::{self, CampaignMutationCoordinator, CommitError};
use crate::turn_store::TurnStore;

/// Stable draft identity used by Accept and autofix synchronization.
pub fn compute_draft_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Prefer autofix response text when present; otherwise keep the pipeline draft.
pub fn prefer_autofix_response_text(response_text: Option<String>, original: String) -> String {
    response_text.unwrap_or(original)
}

/// After auto-fix, quality_report and draft_hash must both track the final text.
pub fn sync_attempt_after_autofix(
    attempt: &mut TurnAttempt,
    final_text: &str,
    report: QualityReport,
) {
    attempt.quality_report = Some(report);
    attempt.draft_hash = compute_draft_hash(final_text);
}

/// Late postprocess may only write the still-current attempt.
pub fn is_current_attempt_ready_for_postprocess(record: &TurnRecord, attempt_id: &Id) -> bool {
    matches!(
        record.status,
        TurnStatus::DraftReady | TurnStatus::DerivingState
    ) && record.active_attempt().is_some_and(|attempt| {
        attempt.attempt_id == *attempt_id
            && matches!(
                attempt.status,
                AttemptStatus::DraftReady | AttemptStatus::DerivingState
            )
    })
}

/// Next Chronicle A sequence, matching production Accept / MutationBatch builders.
pub fn next_chronicle_a_seq(existing: &[RoundSummary]) -> u32 {
    let mut max_seq = 0u32;
    for s in existing {
        if let Some(code) = s.code.as_deref() {
            if let Some(parsed) = storyforge_domain::chronicle::ChronicleCode::parse(code)
                && parsed.level() == Some(storyforge_domain::chronicle::ChronicleLevel::A)
                && let Ok(n) = code[1..].parse::<u32>()
            {
                max_seq = max_seq.max(n);
            }
        } else {
            max_seq = max_seq.max(s.turn);
        }
    }
    max_seq.saturating_add(1).max(1)
}

/// Ensure empty diffs still finalize the variant and temporary instances land on accept.
pub fn prepare_commit_batch(
    attempt: &TurnAttempt,
    variant_id: &Id,
    current_revision: u64,
) -> MutationBatch {
    let mut batch = attempt
        .pending_state_changes
        .clone()
        .unwrap_or_else(|| MutationBatch::new(Id::new(), current_revision));
    let has_finalize = batch
        .mutations
        .iter()
        .any(|m| matches!(m, Mutation::FinalizeVariant { .. }));
    if !has_finalize {
        batch.mutations.push(Mutation::FinalizeVariant {
            variant_id: variant_id.clone(),
        });
    }
    let mut instance_prelude = Vec::new();
    for temp in &attempt.pending_temporary_instances {
        let already = batch.mutations.iter().any(|m| {
            matches!(
                m,
                Mutation::UpsertInstance(inst) if inst.id == temp.id
            )
        });
        if !already {
            instance_prelude.push(Mutation::UpsertInstance(Box::new(temp.clone())));
        }
    }
    // Temporary instances must exist before state/knowledge mutations can target them.
    batch.mutations.splice(0..0, instance_prelude);
    batch
}

/// Attach postprocess output onto an attempt that is still the current draft.
pub fn apply_postprocess_to_attempt(
    attempt: &mut TurnAttempt,
    batch: Option<MutationBatch>,
    derivation: storyforge_domain::turn::DerivationComponents,
) {
    attempt.pending_state_changes = batch;
    attempt.derivation = Some(derivation);
    attempt.status = AttemptStatus::AwaitingAcceptance;
}

/// Mark the accepted attempt Committed and supersede other active attempts.
pub fn finalize_committed_turn(record: &mut TurnRecord, attempt_id: &Id, turn_status: TurnStatus) {
    record.status = turn_status;
    record.accepted_attempt_id = Some(attempt_id.clone());
    for att in &mut record.attempts {
        if att.attempt_id != *attempt_id && att.status.is_active() {
            att.status = AttemptStatus::Superseded;
        } else if att.attempt_id == *attempt_id {
            att.status = AttemptStatus::Committed;
        }
    }
    record.touch();
}

/// Supersede active attempts and append the regenerate draft attempt.
pub fn append_regenerate_attempt(record: &mut TurnRecord, new_attempt: TurnAttempt) {
    for att in &mut record.attempts {
        if att.status.is_active() {
            att.status = AttemptStatus::Superseded;
        }
    }
    record.attempts.push(new_attempt);
    record.status = TurnStatus::DraftReady;
    record.touch();
}

/// Build a DraftReady attempt after pipeline returns final text.
pub fn new_draft_attempt(
    attempt_id: Id,
    variant_id: Id,
    draft_text: &str,
    pending_temporary_instances: Vec<CharacterInstance>,
) -> TurnAttempt {
    TurnAttempt {
        attempt_id,
        variant_id,
        draft_hash: compute_draft_hash(draft_text),
        status: AttemptStatus::DraftReady,
        pending_state_changes: None,
        derivation: None,
        quality_report: None,
        pending_temporary_instances,
        provenance: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptError {
    NoTurnRecord,
    NoAttempt,
    InvalidAttemptStatus(String),
    QualityBlocked {
        error_count: usize,
    },
    RevisionConflict {
        base: u64,
        current: u64,
    },
    DraftHashMismatch,
    CasFailed,
    CampaignMissing,
    CampaignScopeMismatch {
        turn_campaign: String,
        requested: String,
    },
    ConversationScopeMismatch {
        turn_conversation: String,
        requested: String,
    },
    Storage(String),
    Commit(String),
}

impl std::fmt::Display for AcceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTurnRecord => write!(
                f,
                "该变体没有关联的 TurnRecord，可能是历史草稿。请从此处 fork 或 regenerate。"
            ),
            Self::NoAttempt => write!(f, "该变体没有关联的 TurnAttempt"),
            Self::InvalidAttemptStatus(status) => write!(
                f,
                "该 Attempt 状态为 {status}，不能 accept（只有 AwaitingAcceptance 态才能 accept）"
            ),
            Self::QualityBlocked { error_count } => write!(
                f,
                "质量门禁拦截：存在 {error_count} 个 Error 级问题。可修复后重 roll，或 force_accept=true 强制接受（将标记为 Degraded）。"
            ),
            Self::RevisionConflict { base, current } => write!(
                f,
                "revision 冲突：Turn 基于 revision {base}，但当前 Campaign revision 为 {current}。该 Turn 已过期。"
            ),
            Self::DraftHashMismatch => write!(
                f,
                "draft_hash 不匹配：草稿已被编辑但未重新推导状态。请重新推导后再 accept，或 Discard 后 regenerate。"
            ),
            Self::CasFailed => write!(
                f,
                "Turn 状态已变化，无法进入 Committing（可能已被并发 accept 或 postprocess 未完成）"
            ),
            Self::CampaignMissing => write!(f, "Campaign 不存在"),
            Self::CampaignScopeMismatch {
                turn_campaign,
                requested,
            } => write!(
                f,
                "Campaign 归属不匹配：Turn 属于 {turn_campaign}，请求 accept 的是 {requested}。请切回原 Campaign 或 regenerate。"
            ),
            Self::ConversationScopeMismatch {
                turn_conversation,
                requested,
            } => write!(
                f,
                "Conversation 归属不匹配：Turn 属于 {turn_conversation}，请求 accept 的是 {requested}。"
            ),
            Self::Storage(msg) => write!(f, "{msg}"),
            Self::Commit(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AcceptError {}

#[derive(Debug, Clone)]
pub struct AcceptOutcome {
    pub turn_id: Id,
    pub attempt_id: Id,
    pub turn_status: TurnStatus,
    pub attempt_status: AttemptStatus,
    pub commit_as_degraded: bool,
    pub campaign_revision_before: u64,
    pub campaign_revision_after: u64,
    pub batch: MutationBatch,
}

/// Shared lifecycle service over the production JSON stores.
pub struct TurnLifecycleService<'a> {
    pub campaign_store: &'a CampaignStore,
    pub turn_store: &'a TurnStore,
    pub conv_store: &'a ConversationStore,
}

impl<'a> TurnLifecycleService<'a> {
    pub fn new(
        campaign_store: &'a CampaignStore,
        turn_store: &'a TurnStore,
        conv_store: &'a ConversationStore,
    ) -> Self {
        Self {
            campaign_store,
            turn_store,
            conv_store,
        }
    }

    pub fn read_variant_content(&self, conv_id: &Id, node_id: &Id) -> String {
        let conv = match self.conv_store.get(conv_id) {
            Some(c) => c,
            None => return String::new(),
        };
        conv.nodes
            .iter()
            .find(|n| &n.id == node_id)
            .and_then(|node| node.active())
            .map(|v| v.content.clone())
            .unwrap_or_default()
    }

    fn preflight_variant_acceptance(
        &self,
        conversation_id: &Id,
        variant_id: &Id,
    ) -> Result<(), CommitError> {
        let conversation = self.conv_store.get(conversation_id).ok_or_else(|| {
            CommitError::Storage(format!("conversation {conversation_id} 暂不可用"))
        })?;
        let node = conversation
            .nodes
            .iter()
            .find(|node| node.id == *variant_id)
            .ok_or_else(|| CommitError::Storage(format!("variant node {variant_id} 暂不可用")))?;
        let variant = node
            .active()
            .ok_or_else(|| CommitError::Storage(format!("variant {variant_id} 无 active 版本")))?;
        if variant.status == storyforge_domain::conversation::VariantStatus::Discarded {
            return Err(CommitError::Storage(format!(
                "variant {variant_id} 已丢弃，不能 Final"
            )));
        }
        Ok(())
    }

    fn preflight_finalize_mutation(
        batch: &MutationBatch,
        variant_id: &Id,
    ) -> Result<(), CommitError> {
        let finalize_ids: Vec<&Id> = batch
            .mutations
            .iter()
            .filter_map(|mutation| match mutation {
                Mutation::FinalizeVariant { variant_id } => Some(variant_id),
                _ => None,
            })
            .collect();
        if finalize_ids.len() != 1 || finalize_ids[0] != variant_id {
            return Err(CommitError::MutationConflict(format!(
                "MutationBatch 必须且只能 Finalize 当前 variant {variant_id}"
            )));
        }
        Ok(())
    }

    pub fn update_turn_record<F>(&self, turn_id: &Id, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut TurnRecord),
    {
        self.turn_store
            .with_turn_mut(turn_id, f)
            .map_err(|e| format!("保存 TurnRecord 失败: {e}"))
    }

    pub fn update_turn_record_if<P, M>(
        &self,
        turn_id: &Id,
        predicate: P,
        mutate: M,
    ) -> Result<bool, String>
    where
        P: FnOnce(&TurnRecord) -> bool,
        M: FnOnce(&mut TurnRecord),
    {
        self.turn_store
            .mutate_if(turn_id, predicate, mutate)
            .map_err(|e| format!("条件更新 TurnRecord 失败: {e}"))
    }

    /// Production Accept: quality gate, draft_hash, revision CAS, MutationBatch apply.
    pub fn accept_by_variant(
        &self,
        campaign_id: &Id,
        conversation_id: &Id,
        variant_id: &Id,
        force_accept: bool,
    ) -> Result<AcceptOutcome, AcceptError> {
        let turn = self
            .turn_store
            .get_turn_by_variant(variant_id)
            .ok_or(AcceptError::NoTurnRecord)?;
        let attempt = turn
            .find_attempt_by_variant(variant_id)
            .ok_or(AcceptError::NoAttempt)?
            .clone();

        if &turn.campaign_id != campaign_id {
            return Err(AcceptError::CampaignScopeMismatch {
                turn_campaign: turn.campaign_id.to_string(),
                requested: campaign_id.to_string(),
            });
        }
        if &turn.conversation_id != conversation_id {
            return Err(AcceptError::ConversationScopeMismatch {
                turn_conversation: turn.conversation_id.to_string(),
                requested: conversation_id.to_string(),
            });
        }

        if attempt.status != AttemptStatus::AwaitingAcceptance {
            return Err(AcceptError::InvalidAttemptStatus(format!(
                "{:?}",
                attempt.status
            )));
        }

        let quality_decision =
            quality_accept_decision(attempt.quality_report.as_ref(), force_accept);
        let commit_as_degraded = match &quality_decision {
            QualityAcceptDecision::AllowCommit => false,
            QualityAcceptDecision::ForceDegraded { .. } => true,
            QualityAcceptDecision::Block { error_count } => {
                return Err(AcceptError::QualityBlocked {
                    error_count: *error_count,
                });
            }
        };

        // Always scope revision / mutations to the Turn's own campaign.
        let owner_campaign_id = turn.campaign_id.clone();
        let camp = self
            .campaign_store
            .get_campaign(&owner_campaign_id)
            .ok_or(AcceptError::CampaignMissing)?;
        let campaign_revision_before = camp.revision;
        if turn.base_campaign_revision != campaign_revision_before {
            return Err(AcceptError::RevisionConflict {
                base: turn.base_campaign_revision,
                current: campaign_revision_before,
            });
        }

        let current_text = self.read_variant_content(conversation_id, variant_id);
        if compute_draft_hash(&current_text) != attempt.draft_hash {
            return Err(AcceptError::DraftHashMismatch);
        }

        let batch = prepare_commit_batch(&attempt, variant_id, campaign_revision_before);
        let turn_id = turn.turn_id.clone();
        let attempt_id = attempt.attempt_id.clone();
        let batch_for_store = batch.clone();
        let final_status = if commit_as_degraded {
            TurnStatus::Degraded
        } else {
            TurnStatus::Committed
        };
        let intended = final_status.clone();
        let cas_ok = self
            .update_turn_record_if(
                &turn_id,
                |record| {
                    record.status == TurnStatus::AwaitingAcceptance
                        && record
                            .find_attempt(&attempt_id)
                            .is_some_and(|a| a.status == AttemptStatus::AwaitingAcceptance)
                },
                |record| {
                    record.status = TurnStatus::Committing;
                    record.intended_terminal_status = Some(intended);
                    if let Some(att) = record.find_attempt_mut(&attempt_id) {
                        att.status = AttemptStatus::Committing;
                        att.pending_state_changes = Some(batch_for_store);
                    }
                    record.touch();
                },
            )
            .map_err(AcceptError::Storage)?;
        if !cas_ok {
            return Err(AcceptError::CasFailed);
        }

        let apply_result = turn_coordinator::with_campaign_lock(|| {
            // Reject every permanent error before any durable side effect. Apply the
            // idempotent Campaign batch before Final so a later conversation write
            // failure remains recoverable from the Committing journal.
            self.preflight_variant_acceptance(conversation_id, variant_id)?;
            Self::preflight_finalize_mutation(&batch, variant_id)?;
            CampaignMutationCoordinator::preflight_mutation_batch(
                self.campaign_store,
                &owner_campaign_id,
                &batch,
            )?;
            CampaignMutationCoordinator::apply_mutation_batch(
                self.campaign_store,
                &owner_campaign_id,
                &batch,
            )?;
            self.conv_store
                .accept_variant(conversation_id, variant_id)
                .map_err(|e| CommitError::Storage(format!("Draft → Final 失败: {e}")))?;
            Ok(())
        });

        if let Err(error) = apply_result {
            let permanent = matches!(
                &error,
                CommitError::CampaignNotFound(_)
                    | CommitError::RevisionConflict { .. }
                    | CommitError::MutationConflict(_)
            );
            if permanent {
                let reason = format!("commit preflight 失败: {error}");
                self.update_turn_record(&turn_id, |record| {
                    record.status = TurnStatus::Failed;
                    record.failure_reason = Some(reason);
                    record.intended_terminal_status = None;
                    if let Some(attempt) = record.find_attempt_mut(&attempt_id) {
                        attempt.status = AttemptStatus::Failed;
                    }
                    record.touch();
                })
                .map_err(AcceptError::Storage)?;
            }
            // Storage/lock failures remain Committing for idempotent recovery.
            return Err(AcceptError::Commit(error.to_string()));
        }

        // Side effects landed: terminal mark must surface persistence failure.
        self.mark_terminal_after_side_effects(&turn_id, &attempt_id, final_status.clone())?;

        let campaign_revision_after = self
            .campaign_store
            .get_campaign(&owner_campaign_id)
            .map(|c| c.revision)
            .unwrap_or(campaign_revision_before);

        Ok(AcceptOutcome {
            turn_id,
            attempt_id,
            turn_status: final_status,
            attempt_status: AttemptStatus::Committed,
            commit_as_degraded,
            campaign_revision_before,
            campaign_revision_after,
            batch,
        })
    }

    /// Persist terminal Turn/Attempt state after side effects. Errors must not be swallowed.
    pub fn mark_terminal_after_side_effects(
        &self,
        turn_id: &Id,
        attempt_id: &Id,
        turn_status: TurnStatus,
    ) -> Result<(), AcceptError> {
        self.update_turn_record(turn_id, |record| {
            finalize_committed_turn(record, attempt_id, turn_status);
            record.intended_terminal_status = None;
        })
        .map_err(AcceptError::Storage)
    }

    fn recovery_terminal_status(turn: &TurnRecord, attempt: &TurnAttempt) -> Option<TurnStatus> {
        match turn.intended_terminal_status.as_ref() {
            Some(TurnStatus::Committed) => Some(TurnStatus::Committed),
            Some(TurnStatus::Degraded) => Some(TurnStatus::Degraded),
            Some(_) => None,
            None => {
                // Legacy journals predate intended_terminal_status. An Error-level
                // quality report proves the only legal path was force accept, so the
                // conservative reconstruction is Degraded. Otherwise do not guess.
                let had_quality_error = attempt.quality_report.as_ref().is_some_and(|report| {
                    report
                        .warnings
                        .iter()
                        .any(|warning| warning.severity == QualitySeverity::Error)
                });
                had_quality_error.then_some(TurnStatus::Degraded)
            }
        }
    }

    fn record_recovery_issue(&self, turn_id: &Id, message: String) {
        if let Err(error) = self.update_turn_record(turn_id, |record| {
            // Remain Committing/recoverable; failure_reason is diagnostic only.
            record.failure_reason = Some(message);
            record.touch();
        }) {
            tracing::error!(
                target: "turn_recovery",
                turn_id = %turn_id,
                "failed to persist recovery diagnostic: {error}"
            );
        }
    }

    /// Startup recovery: replay Committing turns; fail non-side-effect active turns.
    ///
    /// `on_recovered_batch` is invoked after a successful mutation replay so callers can
    /// index RoundSummary far-memory without embedding vector deps in this module.
    pub fn recover_turns_on_startup(&self, mut on_recovered_batch: impl FnMut(&MutationBatch)) {
        let recoverable = self.turn_store.list_recoverable_turns();
        for turn in &recoverable {
            let campaign_id = turn.campaign_id.clone();
            let turn_id = turn.turn_id.clone();
            let attempt = turn.attempts.iter().find(|a| {
                a.status == AttemptStatus::Committing || a.status == AttemptStatus::Committed
            });
            let Some(attempt) = attempt else {
                self.record_recovery_issue(
                    &turn_id,
                    "启动恢复缺少 Committing Attempt，保持 Committing".into(),
                );
                continue;
            };
            let Some(terminal) = Self::recovery_terminal_status(turn, attempt) else {
                self.record_recovery_issue(
                    &turn_id,
                    "旧 Turn 缺少可证明的 intended_terminal_status，拒绝自动升级".into(),
                );
                continue;
            };

            let attempt_id = attempt.attempt_id.clone();
            let variant_id = attempt.variant_id.clone();
            let mut batch = attempt.pending_state_changes.clone().unwrap_or_else(|| {
                MutationBatch::new(
                    Id::from_str(format!("legacy-empty-{}", turn.turn_id)),
                    turn.base_campaign_revision,
                )
            });
            if !batch
                .mutations
                .iter()
                .any(|mutation| matches!(mutation, Mutation::FinalizeVariant { .. }))
            {
                batch.mutations.push(Mutation::FinalizeVariant {
                    variant_id: variant_id.clone(),
                });
            }

            let replay = turn_coordinator::with_campaign_lock(|| {
                self.preflight_variant_acceptance(&turn.conversation_id, &variant_id)?;
                Self::preflight_finalize_mutation(&batch, &variant_id)?;
                CampaignMutationCoordinator::preflight_mutation_batch(
                    self.campaign_store,
                    &campaign_id,
                    &batch,
                )?;
                CampaignMutationCoordinator::apply_mutation_batch(
                    self.campaign_store,
                    &campaign_id,
                    &batch,
                )?;
                self.conv_store
                    .accept_variant(&turn.conversation_id, &variant_id)
                    .map_err(|error| {
                        CommitError::Storage(format!("恢复 Draft → Final 失败: {error}"))
                    })?;
                Ok(())
            });

            match replay {
                Ok(()) => {
                    on_recovered_batch(&batch);
                    if let Err(error) = self.mark_terminal_after_side_effects(
                        &turn_id,
                        &attempt_id,
                        terminal.clone(),
                    ) {
                        // TurnStore restores the in-memory journal on persistence failure,
                        // so the record remains Committing and the idempotent replay is safe.
                        tracing::error!(
                            target: "turn_recovery",
                            turn_id = %turn_id,
                            "terminal mark failed; keep recoverable: {error}"
                        );
                    }
                }
                Err(CommitError::RevisionConflict { expected, actual }) => {
                    if let Err(error) = self.update_turn_record(&turn_id, |record| {
                        record.status = TurnStatus::Failed;
                        record.failure_reason = Some(format!(
                            "启动恢复 revision 冲突: expected={expected}, actual={actual}"
                        ));
                        record.intended_terminal_status = None;
                        if let Some(attempt) = record.find_attempt_mut(&attempt_id) {
                            attempt.status = AttemptStatus::Failed;
                        }
                        record.touch();
                    }) {
                        tracing::error!(target: "turn_recovery", turn_id = %turn_id, "failed to persist revision conflict: {error}");
                    }
                }
                Err(CommitError::MutationConflict(message)) => {
                    if let Err(error) = self.update_turn_record(&turn_id, |record| {
                        record.status = TurnStatus::Failed;
                        record.failure_reason =
                            Some(format!("启动恢复 mutation preflight 失败: {message}"));
                        record.intended_terminal_status = None;
                        if let Some(attempt) = record.find_attempt_mut(&attempt_id) {
                            attempt.status = AttemptStatus::Failed;
                        }
                        record.touch();
                    }) {
                        tracing::error!(target: "turn_recovery", turn_id = %turn_id, "failed to persist mutation conflict: {error}");
                    }
                }
                Err(CommitError::CampaignNotFound(_)) => {
                    self.record_recovery_issue(
                        &turn_id,
                        format!("启动恢复 Campaign {campaign_id} 不存在，保持 Committing"),
                    );
                }
                Err(error) => {
                    self.record_recovery_issue(
                        &turn_id,
                        format!("启动恢复遇到可重试错误，保持 Committing: {error}"),
                    );
                }
            }
        }

        let active = self.turn_store.list_active_turns();
        for turn in &active {
            if turn.status.has_side_effects_started() {
                continue;
            }
            let turn_id = turn.turn_id.clone();
            let status = turn.status.clone();
            let _ = self.update_turn_record(&turn_id, |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(format!("启动恢复：崩溃时处于 {status:?} 态"));
                record.intended_terminal_status = None;
                record.touch();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::turn::{QualityReport, QualityWarning, QualityWarningCode};

    struct Fixture {
        _data_dir: std::path::PathBuf,
        campaign_store: Arc<CampaignStore>,
        turn_store: Arc<TurnStore>,
        conv_store: Arc<ConversationStore>,
        campaign_id: Id,
        conversation_id: Id,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let data_dir = std::env::temp_dir().join(format!(
                "sf_turn_lifecycle_{label}_{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&data_dir).unwrap();
            let campaign_store = Arc::new(CampaignStore::new(&data_dir));
            let turn_store = Arc::new(TurnStore::new(&data_dir));
            let conv_store = Arc::new(ConversationStore::new(data_dir.join("conversations")));

            let mut campaign = Campaign::new(Id::new(), format!("camp-{label}"));
            campaign.lineage_id = Some(Id::new());
            let campaign_id = campaign.id.clone();
            campaign_store.save_campaign(campaign).unwrap();
            let conversation = conv_store.create(None, None);
            let conversation_id = conversation.id.clone();
            if let Some(mut camp) = campaign_store.get_campaign(&campaign_id) {
                camp.conversation_id = Some(conversation_id.clone());
                campaign_store.update_campaign(camp).unwrap();
            }

            Self {
                _data_dir: data_dir,
                campaign_store,
                turn_store,
                conv_store,
                campaign_id,
                conversation_id,
            }
        }

        fn service(&self) -> TurnLifecycleService<'_> {
            TurnLifecycleService::new(&self.campaign_store, &self.turn_store, &self.conv_store)
        }

        fn append_draft(&self, text: &str) -> Id {
            self.conv_store
                .append_ai_draft(&self.conversation_id, text.to_string(), None)
                .unwrap()
        }

        fn variant_status(
            &self,
            variant_id: &Id,
        ) -> storyforge_domain::conversation::VariantStatus {
            self.conv_store
                .get(&self.conversation_id)
                .and_then(|conversation| {
                    conversation
                        .nodes
                        .iter()
                        .find(|node| node.id == *variant_id)
                        .and_then(|node| node.active())
                        .map(|variant| variant.status.clone())
                })
                .expect("variant should exist")
        }

        fn prepare_awaiting(
            &self,
            variant_id: &Id,
            draft: &str,
            quality: Option<QualityReport>,
            summary: Option<&str>,
        ) -> TurnRecord {
            let camp = self.campaign_store.get_campaign(&self.campaign_id).unwrap();
            let mut batch = MutationBatch::new(Id::new(), camp.revision);
            if let Some(summary) = summary {
                let existing = self.campaign_store.list_summaries(&self.campaign_id);
                let seq = next_chronicle_a_seq(&existing);
                let code = storyforge_domain::chronicle::ChronicleCode::new(
                    storyforge_domain::chronicle::ChronicleLevel::A,
                    seq,
                );
                let headline = storyforge_domain::chronicle::truncate_headline(summary, 40);
                batch.mutations.push(Mutation::UpsertSummary(Box::new(
                    RoundSummary::new(
                        self.campaign_id.clone(),
                        self.conversation_id.clone(),
                        1,
                        summary.to_string(),
                    )
                    .with_code(code.as_str())
                    .with_headline(headline)
                    .with_lineage(camp.lineage_id.clone().unwrap_or_default()),
                )));
            }
            batch.mutations.push(Mutation::FinalizeVariant {
                variant_id: variant_id.clone(),
            });

            let attempt = TurnAttempt {
                attempt_id: Id::new(),
                variant_id: variant_id.clone(),
                draft_hash: compute_draft_hash(draft),
                status: AttemptStatus::AwaitingAcceptance,
                pending_state_changes: Some(batch),
                derivation: None,
                quality_report: quality,
                pending_temporary_instances: vec![],
                provenance: None,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            let mut record = TurnRecord::new(
                self.campaign_id.clone(),
                self.conversation_id.clone(),
                Id::from_str("input-node"),
                camp.revision,
            );
            record.status = TurnStatus::AwaitingAcceptance;
            record.attempts.push(attempt);
            self.turn_store.create_turn(record.clone()).unwrap();
            record
        }
    }

    #[test]
    fn compute_draft_hash_is_stable_sha256_hex() {
        let a = compute_draft_hash("hello-turn");
        let b = compute_draft_hash("hello-turn");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, compute_draft_hash("hello-turn!"));
    }

    #[test]
    fn autofix_sync_updates_hash_to_final_text() {
        let mut attempt = new_draft_attempt(Id::new(), Id::new(), "original", vec![]);
        let report = QualityReport { warnings: vec![] };
        sync_attempt_after_autofix(&mut attempt, "fixed draft", report);
        assert_eq!(attempt.draft_hash, compute_draft_hash("fixed draft"));
        assert!(attempt.quality_report.is_some());
    }

    #[test]
    fn postprocess_race_guard_rejects_superseded_attempt() {
        let mut record = TurnRecord::new(
            Id::from_str("c"),
            Id::from_str("conv"),
            Id::from_str("n"),
            0,
        );
        record.status = TurnStatus::DraftReady;
        let old = Id::from_str("old");
        let new = Id::from_str("new");
        record.attempts = vec![
            TurnAttempt {
                attempt_id: old.clone(),
                variant_id: Id::from_str("n"),
                draft_hash: "old".into(),
                status: AttemptStatus::Superseded,
                pending_state_changes: None,
                derivation: None,
                quality_report: None,
                pending_temporary_instances: vec![],
                provenance: None,
                created_at: "t0".into(),
            },
            TurnAttempt {
                attempt_id: new.clone(),
                variant_id: Id::from_str("n"),
                draft_hash: "new".into(),
                status: AttemptStatus::DraftReady,
                pending_state_changes: None,
                derivation: None,
                quality_report: None,
                pending_temporary_instances: vec![],
                provenance: None,
                created_at: "t1".into(),
            },
        ];
        assert!(!is_current_attempt_ready_for_postprocess(&record, &old));
        assert!(is_current_attempt_ready_for_postprocess(&record, &new));
    }

    #[test]
    fn accept_happy_path_commits_and_bumps_revision() {
        let fx = Fixture::new("accept_ok");
        let draft = "码头灯火摇曳，角色低声约定银鸦标记。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            Some("第1轮：银鸦标记在码头确立。"),
        );

        let outcome = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect("accept should succeed");
        assert_eq!(outcome.turn_status, TurnStatus::Committed);
        assert_eq!(
            outcome.campaign_revision_after,
            outcome.campaign_revision_before + 1
        );
        assert_eq!(fx.campaign_store.list_summaries(&fx.campaign_id).len(), 1);

        let turn = fx.turn_store.get_turn(&outcome.turn_id).unwrap();
        assert_eq!(turn.status, TurnStatus::Committed);
        assert_eq!(turn.accepted_attempt_id.as_ref(), Some(&outcome.attempt_id));
    }

    #[test]
    fn accept_blocks_quality_errors_without_force() {
        let fx = Fixture::new("quality_block");
        let draft = "足够长的正文用于质量门禁拦截测试——角色在雨夜推进调查。".repeat(2);
        let variant_id = fx.append_draft(&draft);
        let report = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::FormatLeak {
                    snippet: "```".into(),
                },
                message: "format".into(),
                severity: QualitySeverity::Error,
            }],
        };
        fx.prepare_awaiting(&variant_id, &draft, Some(report), Some("blocked"));
        let err = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect_err("quality must block");
        assert!(matches!(err, AcceptError::QualityBlocked { .. }));
        assert!(fx.campaign_store.list_summaries(&fx.campaign_id).is_empty());
    }

    #[test]
    fn accept_force_marks_degraded() {
        let fx = Fixture::new("force_degraded");
        let draft = "强制接受路径：角色带着警告继续推进主线，正文足够长以通过字数下限。".repeat(2);
        let variant_id = fx.append_draft(&draft);
        let report = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::FormatLeak {
                    snippet: "```".into(),
                },
                message: "format".into(),
                severity: QualitySeverity::Error,
            }],
        };
        fx.prepare_awaiting(&variant_id, &draft, Some(report), Some("degraded A"));
        let outcome = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, true)
            .expect("force accept");
        assert_eq!(outcome.turn_status, TurnStatus::Degraded);
        assert!(outcome.commit_as_degraded);
    }

    #[test]
    fn accept_rejects_edited_draft_hash_mismatch() {
        let fx = Fixture::new("hash_mismatch");
        let draft = "原始草稿正文足够长用于 hash 校验。".repeat(2);
        let variant_id = fx.append_draft(&draft);
        fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        // Edit live variant without re-deriving.
        fx.conv_store
            .edit_variant(
                &fx.conversation_id,
                &variant_id,
                "编辑后的草稿正文足够长用于 hash 校验。".repeat(2),
            )
            .expect("edit live draft");
        let err = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect_err("edited draft must fail hash");
        assert_eq!(err, AcceptError::DraftHashMismatch);
    }

    #[test]
    fn regenerate_supersedes_old_attempt_and_accepts_only_new() {
        let fx = Fixture::new("regen");
        let old_text = "旧 regenerate 草稿不可复活。".repeat(3);
        let new_text = "新 regenerate 草稿才是 accept 目标。".repeat(3);
        let variant_id = fx.append_draft(&old_text);

        let camp = fx.campaign_store.get_campaign(&fx.campaign_id).unwrap();
        let mut record = TurnRecord::new(
            fx.campaign_id.clone(),
            fx.conversation_id.clone(),
            Id::from_str("input"),
            camp.revision,
        );
        record.status = TurnStatus::DraftReady;
        let old_attempt = new_draft_attempt(Id::new(), variant_id.clone(), &old_text, vec![]);
        let old_id = old_attempt.attempt_id.clone();
        record.attempts.push(old_attempt);
        fx.turn_store.create_turn(record).unwrap();

        // Simulate regenerate: replace live content + supersede + new attempt.
        fx.conv_store
            .edit_variant(&fx.conversation_id, &variant_id, new_text.clone())
            .expect("replace live draft");
        let turn = fx.turn_store.get_active_turn(&fx.campaign_id).unwrap();
        let new_attempt = new_draft_attempt(Id::new(), variant_id.clone(), &new_text, vec![]);
        let new_id = new_attempt.attempt_id.clone();
        fx.service()
            .update_turn_record(&turn.turn_id, |record| {
                append_regenerate_attempt(record, new_attempt);
            })
            .unwrap();

        // Attach postprocess only to new attempt.
        let mut batch = MutationBatch::new(Id::new(), camp.revision);
        batch.mutations.push(Mutation::FinalizeVariant {
            variant_id: variant_id.clone(),
        });
        fx.service()
            .update_turn_record_if(
                &turn.turn_id,
                |record| is_current_attempt_ready_for_postprocess(record, &new_id),
                |record| {
                    if let Some(att) = record.find_attempt_mut(&new_id) {
                        apply_postprocess_to_attempt(
                            att,
                            Some(batch),
                            storyforge_domain::turn::DerivationComponents {
                                summary_derivation:
                                    storyforge_domain::turn::DerivationStatus::Disabled,
                                state_derivation:
                                    storyforge_domain::turn::DerivationStatus::Disabled,
                            },
                        );
                    }
                    record.status = TurnStatus::AwaitingAcceptance;
                    record.touch();
                },
            )
            .unwrap();

        // Late write for old attempt must be rejected by predicate.
        let late_ok = fx
            .service()
            .update_turn_record_if(
                &turn.turn_id,
                |record| is_current_attempt_ready_for_postprocess(record, &old_id),
                |_| unreachable!("must not mutate superseded attempt"),
            )
            .unwrap();
        assert!(!late_ok);

        let outcome = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect("new attempt accept");
        assert_eq!(outcome.attempt_id, new_id);
        let after = fx.turn_store.get_turn(&outcome.turn_id).unwrap();
        let old = after.find_attempt(&old_id).unwrap();
        assert_eq!(old.status, AttemptStatus::Superseded);
    }

    #[test]
    fn duplicate_accept_is_rejected_after_commit() {
        let fx = Fixture::new("dup_accept");
        let draft = "重复 accept 应被状态机拒绝。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        fx.service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .unwrap();
        let err = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect_err("second accept must fail");
        // After commit, no active attempt remains, so lookup by variant yields no TurnRecord/Attempt.
        assert!(
            matches!(
                err,
                AcceptError::NoTurnRecord
                    | AcceptError::NoAttempt
                    | AcceptError::InvalidAttemptStatus(_)
            ),
            "unexpected second-accept error: {err}"
        );
        let camp = fx.campaign_store.get_campaign(&fx.campaign_id).unwrap();
        assert_eq!(camp.revision, 1, "revision must bump only once");
    }

    #[test]
    fn recovery_fails_active_and_keeps_committing_when_finalize_missing() {
        let fx = Fixture::new("recovery");
        // Active generating turn → Failed
        let mut active = TurnRecord::new(
            fx.campaign_id.clone(),
            fx.conversation_id.clone(),
            Id::from_str("in-active"),
            0,
        );
        active.status = TurnStatus::Generating;
        let active_id = active.turn_id.clone();
        fx.turn_store.create_turn(active).unwrap();

        // Committing with missing variant → stay Committing (other campaign).
        let mut other = Campaign::new(Id::new(), "other");
        other.id = Id::from_str("camp-other");
        other.lineage_id = Some(Id::new());
        fx.campaign_store.save_campaign(other).unwrap();
        let mut committing = TurnRecord::new(
            Id::from_str("camp-other"),
            fx.conversation_id.clone(),
            Id::from_str("in-commit"),
            0,
        );
        committing.status = TurnStatus::Committing;
        let missing_variant = Id::from_str("missing-variant");
        committing.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("att-commit"),
            variant_id: missing_variant,
            draft_hash: "h".into(),
            status: AttemptStatus::Committing,
            pending_state_changes: Some(MutationBatch::new(Id::from_str("b"), 0)),
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "t".into(),
        });
        let committing_id = committing.turn_id.clone();
        fx.turn_store.create_turn(committing).unwrap();

        fx.service().recover_turns_on_startup(|_| {});

        let after_active = fx.turn_store.get_turn(&active_id).unwrap();
        assert_eq!(after_active.status, TurnStatus::Failed);
        assert!(
            fx.turn_store
                .list_active_turns()
                .iter()
                .all(|t| { t.campaign_id != fx.campaign_id })
        );
        let still = fx.turn_store.get_turn(&committing_id).unwrap();
        assert_eq!(
            still.status,
            TurnStatus::Committing,
            "missing finalize must keep Committing"
        );
        assert!(
            fx.turn_store
                .list_recoverable_turns()
                .iter()
                .any(|t| t.turn_id == committing_id)
        );
    }

    #[test]
    fn failure_after_draft_cannot_accept_mismatched_hash_text() {
        // Contract: returned autofix text and Attempt hash must match, else accept fails.
        let fx = Fixture::new("hash_contract");
        let original = "原稿正文足够长。".repeat(4);
        let fixed = "修复稿正文足够长。".repeat(4);
        let variant_id = fx.append_draft(&original);
        let mut attempt = new_draft_attempt(Id::new(), variant_id.clone(), &original, vec![]);
        // Simulate bug: response text fixed but hash not synced.
        let report = QualityReport { warnings: vec![] };
        attempt.quality_report = Some(report);
        // intentionally leave draft_hash as original while live content becomes fixed
        fx.conv_store
            .edit_variant(&fx.conversation_id, &variant_id, fixed.clone())
            .expect("edit draft to fixed text");
        let camp = fx.campaign_store.get_campaign(&fx.campaign_id).unwrap();
        let mut batch = MutationBatch::new(Id::new(), camp.revision);
        batch.mutations.push(Mutation::FinalizeVariant {
            variant_id: variant_id.clone(),
        });
        attempt.status = AttemptStatus::AwaitingAcceptance;
        attempt.pending_state_changes = Some(batch);
        let mut record = TurnRecord::new(
            fx.campaign_id.clone(),
            fx.conversation_id.clone(),
            Id::from_str("in"),
            camp.revision,
        );
        record.status = TurnStatus::AwaitingAcceptance;
        record.attempts.push(attempt);
        fx.turn_store.create_turn(record).unwrap();

        let err = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect_err("mismatched hash must not accept fixed text");
        assert_eq!(err, AcceptError::DraftHashMismatch);

        // After proper sync, accept works.
        let turn = fx.turn_store.get_turn_by_variant(&variant_id).unwrap();
        let att_id = turn.attempts[0].attempt_id.clone();
        fx.service()
            .update_turn_record(&turn.turn_id, |record| {
                if let Some(att) = record.find_attempt_mut(&att_id) {
                    sync_attempt_after_autofix(att, &fixed, QualityReport { warnings: vec![] });
                }
            })
            .unwrap();
        let outcome = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect("synced hash accepts");
        assert_eq!(outcome.turn_status, TurnStatus::Committed);
    }

    #[test]
    fn accept_rejects_cross_campaign_and_does_not_mutate_active_campaign() {
        let fx = Fixture::new("cross_camp");
        let draft = "旧 Campaign 草稿正文足够长用于跨档隔离。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            Some("旧档摘要不应写到新档"),
        );

        // Active campaign is a different one (user switched campaigns).
        let mut other = Campaign::new(Id::new(), "active-other");
        other.lineage_id = Some(Id::new());
        let other_id = other.id.clone();
        fx.campaign_store.save_campaign(other).unwrap();
        let other_before = fx.campaign_store.get_campaign(&other_id).unwrap().revision;
        let owner_before = fx
            .campaign_store
            .get_campaign(&fx.campaign_id)
            .unwrap()
            .revision;

        let err = fx
            .service()
            .accept_by_variant(&other_id, &fx.conversation_id, &variant_id, false)
            .expect_err("must not accept under foreign active campaign");
        assert!(
            matches!(err, AcceptError::CampaignScopeMismatch { .. }),
            "unexpected err: {err}"
        );

        let other_after = fx.campaign_store.get_campaign(&other_id).unwrap();
        assert_eq!(
            other_after.revision, other_before,
            "active campaign must not bump"
        );
        assert!(
            fx.campaign_store.list_summaries(&other_id).is_empty(),
            "active campaign must not receive mutations"
        );
        let owner_after = fx.campaign_store.get_campaign(&fx.campaign_id).unwrap();
        assert_eq!(
            owner_after.revision, owner_before,
            "owner campaign untouched on reject"
        );
        assert!(fx.campaign_store.list_summaries(&fx.campaign_id).is_empty());
        let still_draft = fx
            .conv_store
            .get(&fx.conversation_id)
            .and_then(|c| {
                c.nodes
                    .iter()
                    .find(|n| n.id == variant_id)
                    .and_then(|n| n.active())
                    .map(|v| {
                        matches!(
                            v.status,
                            storyforge_domain::conversation::VariantStatus::Draft
                        )
                    })
            })
            .unwrap_or(false);
        assert!(still_draft, "foreign accept must not finalize variant");
    }

    #[test]
    fn accept_rejects_conversation_scope_mismatch() {
        let fx = Fixture::new("cross_conv");
        let draft = "对话归属校验草稿正文足够长。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        let foreign_conv = fx.conv_store.create(None, None).id;
        let err = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &foreign_conv, &variant_id, false)
            .expect_err("conversation mismatch");
        assert!(
            matches!(err, AcceptError::ConversationScopeMismatch { .. }),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn recovery_preserves_force_degraded_terminal_status() {
        let fx = Fixture::new("recover_degraded");
        let draft = "force accept 崩溃恢复必须保持 Degraded。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let record = fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport {
                warnings: vec![QualityWarning {
                    code: QualityWarningCode::FormatLeak {
                        snippet: "```".into(),
                    },
                    message: "format".into(),
                    severity: QualitySeverity::Error,
                }],
            }),
            Some("degraded summary"),
        );
        let attempt_id = record.attempts[0].attempt_id.clone();
        // Simulate crash after CAS into Committing with intended Degraded.
        fx.service()
            .update_turn_record(&record.turn_id, |r| {
                r.status = TurnStatus::Committing;
                r.intended_terminal_status = Some(TurnStatus::Degraded);
                if let Some(att) = r.find_attempt_mut(&attempt_id) {
                    att.status = AttemptStatus::Committing;
                }
                r.touch();
            })
            .unwrap();

        fx.service().recover_turns_on_startup(|_| {});
        let after = fx.turn_store.get_turn(&record.turn_id).unwrap();
        assert_eq!(
            after.status,
            TurnStatus::Degraded,
            "force-accept recovery must not upgrade Degraded to Committed"
        );
        assert_eq!(after.accepted_attempt_id.as_ref(), Some(&attempt_id));
    }

    #[test]
    fn recovery_does_not_apply_campaign_mutations_when_finalize_fails() {
        let fx = Fixture::new("recover_finalize_order");
        let mut other = Campaign::new(Id::new(), "owner");
        other.lineage_id = Some(Id::new());
        let camp_id = other.id.clone();
        fx.campaign_store.save_campaign(other).unwrap();
        let before = fx.campaign_store.get_campaign(&camp_id).unwrap().revision;

        let mut committing = TurnRecord::new(
            camp_id.clone(),
            fx.conversation_id.clone(),
            Id::from_str("in-finalize-order"),
            before,
        );
        committing.status = TurnStatus::Committing;
        committing.intended_terminal_status = Some(TurnStatus::Committed);
        let missing_variant = Id::from_str("missing-variant-order");
        let mut batch = MutationBatch::new(Id::from_str("batch-order"), before);
        batch.mutations.push(Mutation::SetVariable {
            instance_id: None,
            key: "should_not_write".into(),
            value: serde_json::json!(1),
            turn: 1,
        });
        committing.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("att-order"),
            variant_id: missing_variant,
            draft_hash: "h".into(),
            status: AttemptStatus::Committing,
            pending_state_changes: Some(batch),
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "t".into(),
        });
        let turn_id = committing.turn_id.clone();
        fx.turn_store.create_turn(committing).unwrap();

        fx.service().recover_turns_on_startup(|_| {});

        let after_camp = fx.campaign_store.get_campaign(&camp_id).unwrap();
        assert_eq!(
            after_camp.revision, before,
            "finalize failure must not apply campaign mutations / bump revision"
        );
        let still = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(still.status, TurnStatus::Committing);
    }

    #[test]
    fn final_persist_failure_recovers_without_reapplying_campaign_batch() {
        let fx = Fixture::new("recover_final_persist");
        let draft = "Final 持久化失败后必须只补 Final，不能重复推进 Campaign revision。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let record = fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        let conversation_path = fx
            ._data_dir
            .join("conversations")
            .join(format!("{}.json", fx.conversation_id));
        let durable_draft = std::fs::read(&conversation_path).unwrap();

        // Make only the Draft -> Final persistence fail. Campaign mutation has already
        // committed at this point, so the journal must remain Committing for replay.
        std::fs::remove_file(&conversation_path).unwrap();
        std::fs::create_dir(&conversation_path).unwrap();
        let error = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect_err("Final persistence should fail");
        assert!(
            matches!(error, AcceptError::Commit(_)),
            "unexpected: {error}"
        );
        assert_eq!(
            fx.campaign_store
                .get_campaign(&fx.campaign_id)
                .unwrap()
                .revision,
            1
        );
        assert_eq!(
            fx.turn_store.get_turn(&record.turn_id).unwrap().status,
            TurnStatus::Committing
        );

        // Restore the last durable Draft and simulate process restart. Recovery sees
        // revision==target (Campaign no-op), then persists Final and closes the journal.
        std::fs::remove_dir(&conversation_path).unwrap();
        std::fs::write(&conversation_path, durable_draft).unwrap();
        fx.conv_store.invalidate();
        assert_eq!(
            fx.variant_status(&variant_id),
            storyforge_domain::conversation::VariantStatus::Draft
        );

        fx.service().recover_turns_on_startup(|_| {});

        assert_eq!(
            fx.campaign_store
                .get_campaign(&fx.campaign_id)
                .unwrap()
                .revision,
            1,
            "idempotent recovery must not bump Campaign revision twice"
        );
        assert_eq!(
            fx.variant_status(&variant_id),
            storyforge_domain::conversation::VariantStatus::Final
        );
        assert_eq!(
            fx.turn_store.get_turn(&record.turn_id).unwrap().status,
            TurnStatus::Committed
        );
    }

    #[test]
    fn accept_surfaces_terminal_mark_storage_failure() {
        let fx = Fixture::new("terminal_mark");
        let draft = "终态落盘失败必须向上返回错误。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let record = fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        let attempt_id = record.attempts[0].attempt_id.clone();
        // Move turn into Committing with intended terminal, as if side effects already landed.
        fx.service()
            .update_turn_record(&record.turn_id, |r| {
                r.status = TurnStatus::Committing;
                r.intended_terminal_status = Some(TurnStatus::Committed);
                if let Some(att) = r.find_attempt_mut(&attempt_id) {
                    att.status = AttemptStatus::Committing;
                }
                r.touch();
            })
            .unwrap();

        // Break TurnStore persistence so terminal mark cannot be saved.
        let turns_path = fx._data_dir.join("turns.json");
        std::fs::remove_file(&turns_path).ok();
        std::fs::create_dir_all(&turns_path).unwrap();

        let err = fx
            .service()
            .mark_terminal_after_side_effects(&record.turn_id, &attempt_id, TurnStatus::Committed)
            .expect_err("terminal mark failure must surface");
        assert!(matches!(err, AcceptError::Storage(_)), "unexpected: {err}");
    }

    #[test]
    fn permanent_mutation_error_does_not_finalize_variant() {
        let fx = Fixture::new("preflight_before_final");
        let draft = "永久 mutation 错误必须在正文 Final 前被拦截。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let record = fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        let attempt_id = record.attempts[0].attempt_id.clone();
        let revision_before = fx
            .campaign_store
            .get_campaign(&fx.campaign_id)
            .unwrap()
            .revision;
        fx.service()
            .update_turn_record(&record.turn_id, |turn| {
                let attempt = turn.find_attempt_mut(&attempt_id).unwrap();
                let batch = attempt.pending_state_changes.as_mut().unwrap();
                batch.mutations.insert(
                    0,
                    Mutation::SetVariable {
                        instance_id: None,
                        key: "must_not_partially_write".into(),
                        value: serde_json::json!(true),
                        turn: 1,
                    },
                );
                batch.mutations.insert(
                    1,
                    Mutation::SetVariable {
                        instance_id: Some(Id::from_str("missing-instance")),
                        key: "mood".into(),
                        value: serde_json::json!("broken"),
                        turn: 1,
                    },
                );
            })
            .unwrap();

        let err = fx
            .service()
            .accept_by_variant(&fx.campaign_id, &fx.conversation_id, &variant_id, false)
            .expect_err("missing instance is a permanent mutation error");
        assert!(matches!(err, AcceptError::Commit(_)), "unexpected: {err}");
        assert_eq!(
            fx.variant_status(&variant_id),
            storyforge_domain::conversation::VariantStatus::Draft,
            "permanent mutation errors must not expose a Final draft"
        );
        let campaign = fx.campaign_store.get_campaign(&fx.campaign_id).unwrap();
        assert_eq!(campaign.revision, revision_before);
        assert!(campaign.get_variable("must_not_partially_write").is_none());
        let failed_turn = fx.turn_store.get_turn(&record.turn_id).unwrap();
        assert_eq!(failed_turn.status, TurnStatus::Failed);
        assert_eq!(
            failed_turn.find_attempt(&attempt_id).unwrap().status,
            AttemptStatus::Failed
        );
    }

    #[test]
    fn recovery_with_no_batch_and_missing_campaign_stays_committing() {
        let fx = Fixture::new("recover_missing_campaign_no_batch");
        let draft = "缺失 Campaign 的旧式空 batch 不得伪装成成功。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let missing_campaign = Id::from_str("missing-owner-campaign");
        let mut record = TurnRecord::new(
            missing_campaign,
            fx.conversation_id.clone(),
            Id::from_str("input-missing-campaign"),
            0,
        );
        record.status = TurnStatus::Committing;
        record.intended_terminal_status = Some(TurnStatus::Committed);
        record.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("attempt-missing-campaign"),
            variant_id: variant_id.clone(),
            draft_hash: compute_draft_hash(&draft),
            status: AttemptStatus::Committing,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "t".into(),
        });
        let turn_id = record.turn_id.clone();
        fx.turn_store.create_turn(record).unwrap();

        fx.service().recover_turns_on_startup(|_| {});

        assert_eq!(
            fx.turn_store.get_turn(&turn_id).unwrap().status,
            TurnStatus::Committing
        );
    }

    #[test]
    fn legacy_force_accept_without_intended_status_recovers_degraded() {
        let fx = Fixture::new("legacy_force_degraded");
        let draft = "旧 force accept 记录缺字段时不得升级成 Committed。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let record = fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport {
                warnings: vec![QualityWarning {
                    code: QualityWarningCode::FormatLeak {
                        snippet: "```".into(),
                    },
                    message: "format".into(),
                    severity: QualitySeverity::Error,
                }],
            }),
            None,
        );
        let attempt_id = record.attempts[0].attempt_id.clone();
        fx.service()
            .update_turn_record(&record.turn_id, |turn| {
                turn.status = TurnStatus::Committing;
                turn.intended_terminal_status = None;
                turn.find_attempt_mut(&attempt_id).unwrap().status = AttemptStatus::Committing;
            })
            .unwrap();

        fx.service().recover_turns_on_startup(|_| {});

        assert_eq!(
            fx.turn_store.get_turn(&record.turn_id).unwrap().status,
            TurnStatus::Degraded
        );
    }

    #[test]
    fn legacy_unknown_terminal_intent_stays_committing() {
        let fx = Fixture::new("legacy_unknown_terminal");
        let draft = "旧 journal 无法证明终态时必须等待人工恢复。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let record = fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        let attempt_id = record.attempts[0].attempt_id.clone();
        fx.service()
            .update_turn_record(&record.turn_id, |turn| {
                turn.status = TurnStatus::Committing;
                turn.intended_terminal_status = None;
                turn.find_attempt_mut(&attempt_id).unwrap().status = AttemptStatus::Committing;
            })
            .unwrap();

        fx.service().recover_turns_on_startup(|_| {});

        let after = fx.turn_store.get_turn(&record.turn_id).unwrap();
        assert_eq!(after.status, TurnStatus::Committing);
        assert!(
            after
                .failure_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("intended_terminal_status"))
        );
        assert_eq!(
            fx.variant_status(&variant_id),
            storyforge_domain::conversation::VariantStatus::Draft
        );
    }

    #[test]
    fn recovery_terminal_write_failure_keeps_turn_recoverable() {
        let fx = Fixture::new("recover_terminal_write_failure");
        let draft = "恢复终态写盘失败时必须保留 Committing 以便重试。".repeat(3);
        let variant_id = fx.append_draft(&draft);
        let record = fx.prepare_awaiting(
            &variant_id,
            &draft,
            Some(QualityReport { warnings: vec![] }),
            None,
        );
        let attempt_id = record.attempts[0].attempt_id.clone();
        fx.service()
            .update_turn_record(&record.turn_id, |turn| {
                turn.status = TurnStatus::Committing;
                turn.intended_terminal_status = Some(TurnStatus::Committed);
                turn.find_attempt_mut(&attempt_id).unwrap().status = AttemptStatus::Committing;
            })
            .unwrap();

        let turns_path = fx._data_dir.join("turns.json");
        std::fs::remove_file(&turns_path).unwrap();
        std::fs::create_dir_all(&turns_path).unwrap();

        fx.service().recover_turns_on_startup(|_| {});

        let after = fx.turn_store.get_turn(&record.turn_id).unwrap();
        assert_eq!(after.status, TurnStatus::Committing);
        assert!(
            fx.turn_store
                .list_recoverable_turns()
                .iter()
                .any(|turn| turn.turn_id == record.turn_id)
        );
    }
}

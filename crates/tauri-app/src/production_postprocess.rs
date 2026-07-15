//! Shared ProductionPostprocessService.
//!
//! # Call graph (production writing after draft ready)
//!
//! ```text
//! start_writing / regenerate (Tauri command)
//!   ├─ pipeline final_text
//!   ├─ quality_gate_with_optional_editor_autofix   (command layer)
//!   ├─ ProductionPostprocessService::sync_autofix_attempt
//!   │     └─ draft_hash + quality_report must match final_text
//!   ├─ PostprocessRunner::run                       (Summarizer ∥ PostProcessor)
//!   │     └─ injectable: real pipeline runner | fixed harness outcome
//!   ├─ derive_components_from_outcome
//!   ├─ build_mutation_batch (Chronicle A / knowledge / variables / tasks)
//!   └─ attach_to_attempt
//!         ├─ cancel guard
//!         ├─ late-result / superseded-attempt guard
//!         └─ Turn → AwaitingAcceptance
//!
//! Accept is owned by TurnLifecycleService (separate slice).
//! ```
//!
//! Command layers keep DTO/event/task spawning. The state machine for Attempt
//! writeback, Chronicle A candidate construction, and failure semantics lives here
//! so Tauri and harness cannot diverge.

use std::sync::Arc;

use async_trait::async_trait;
use storyforge_app_agent::PostProcessOutcome;
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::turn::{
    AttemptStatus, DerivationComponents, DerivationStatus, Mutation, MutationBatch,
    MutationBatchStatus, QualityReport, TurnStatus,
};
use tokio::sync::watch;

use crate::campaign_store::CampaignStore;
use crate::turn_lifecycle::{
    apply_postprocess_to_attempt, is_current_attempt_ready_for_postprocess, next_chronicle_a_seq,
    sync_attempt_after_autofix,
};
use crate::turn_store::TurnStore;

/// Identity of the active Turn/Attempt that postprocess may write.
#[derive(Debug, Clone)]
pub struct PostprocessIdentity {
    pub turn_id: Id,
    pub attempt_id: Id,
    pub campaign_id: Id,
    pub conversation_id: Id,
    pub turn_number: u32,
}

/// Inputs for one production postprocess pass.
#[derive(Debug, Clone)]
pub struct ProductionPostprocessRequest {
    /// When None, campaign-scoped writeback is skipped (legacy non-campaign path).
    pub identity: Option<PostprocessIdentity>,
    pub final_text: String,
    pub quality_report: Option<QualityReport>,
    pub present_chars: Vec<String>,
    pub cancel: watch::Receiver<bool>,
}

/// Distinguishes best-effort agent degradation from hard storage failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductionPostprocessError {
    /// Autofix hash/report sync failed; caller must not return fixed text.
    AutofixSync(String),
    /// Mutation batch could not be built from campaign state.
    BatchConstruction(String),
    /// Persisting Attempt/Turn failed; must propagate.
    Storage(String),
    /// Identity campaign/conversation does not match the stored Turn.
    ScopeMismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    /// Target attempt is missing on the Turn.
    AttemptMissing { turn_id: String, attempt_id: String },
    /// Failed while trying to mark the Turn Failed after another error.
    MarkFailed {
        original: String,
        mark_error: String,
    },
}

impl std::fmt::Display for ProductionPostprocessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AutofixSync(msg) => write!(f, "auto-fix 后 Attempt 同步失败: {msg}"),
            Self::BatchConstruction(msg) => {
                write!(f, "postprocess mutation batch construction failed: {msg}")
            }
            Self::Storage(msg) => write!(f, "postprocess storage failed: {msg}"),
            Self::ScopeMismatch {
                field,
                expected,
                actual,
            } => write!(
                f,
                "postprocess scope mismatch on {field}: expected {expected}, actual {actual}"
            ),
            Self::AttemptMissing {
                turn_id,
                attempt_id,
            } => write!(
                f,
                "postprocess target attempt {attempt_id} missing on turn {turn_id}"
            ),
            Self::MarkFailed {
                original,
                mark_error,
            } => write!(
                f,
                "postprocess failed ({original}) and mark_failed also failed: {mark_error}"
            ),
        }
    }
}

impl std::error::Error for ProductionPostprocessError {}

/// Outcome of applying (or skipping) production postprocess writeback.
#[derive(Debug, Clone)]
pub struct ProductionPostprocessResult {
    /// True when Attempt was written and Turn became AwaitingAcceptance.
    pub applied: bool,
    pub skipped_reason: Option<String>,
    pub derivation: DerivationComponents,
    pub summary_text: Option<String>,
    pub batch: Option<MutationBatch>,
    pub outcome: Option<PostProcessOutcome>,
}

/// Injectable Summarizer/PostProcessor runner.
#[async_trait]
pub trait PostprocessRunner: Send + Sync {
    async fn run(
        &self,
        final_text: &str,
        present_chars: &[String],
        cancel: watch::Receiver<bool>,
    ) -> Option<PostProcessOutcome>;
}

/// Deterministic runner used by harness / contract tests (no real model).
pub struct FixedPostprocessRunner {
    pub outcome: Option<PostProcessOutcome>,
}

#[async_trait]
impl PostprocessRunner for FixedPostprocessRunner {
    async fn run(
        &self,
        _final_text: &str,
        _present_chars: &[String],
        cancel: watch::Receiver<bool>,
    ) -> Option<PostProcessOutcome> {
        if *cancel.borrow() {
            return None;
        }
        self.outcome.clone()
    }
}

/// Persistence adapter for Attempt/Turn mutations (JSON TurnStore or Tauri backend router).
pub trait TurnAttemptSink: Send + Sync {
    /// Load the turn for identity/scope checks. None = missing.
    fn load_turn(
        &self,
        turn_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String>;

    /// Atomically validate identity + writable status, then sync draft_hash/quality.
    ///
    /// `campaign_id`, `conversation_id`, `attempt_id`, and allowed statuses must be
    /// checked inside the same durable mutation. Typed scope/attempt failures must
    /// remain `ScopeMismatch` / `AttemptMissing` (never stringified as AutofixSync).
    fn sync_autofix(
        &self,
        identity: &PostprocessIdentity,
        final_text: &str,
        report: QualityReport,
    ) -> Result<(), ProductionPostprocessError>;

    /// Returns Ok(true) when the attempt was current and writeback applied.
    /// Scope and attempt presence must already be validated by the service.
    fn attach_postprocess(
        &self,
        identity: &PostprocessIdentity,
        batch: Option<MutationBatch>,
        derivation: DerivationComponents,
    ) -> Result<bool, String>;

    fn mark_failed(&self, turn_id: &Id, reason: String) -> Result<(), String>;
}

/// Allowed Turn/Attempt statuses for autofix draft_hash writeback.
fn is_attempt_writable_for_autofix(
    record: &storyforge_domain::turn::TurnRecord,
    attempt_id: &Id,
) -> bool {
    matches!(
        record.status,
        TurnStatus::DraftReady | TurnStatus::DerivingState
    ) && record.find_attempt(attempt_id).is_some_and(|attempt| {
        matches!(
            attempt.status,
            AttemptStatus::DraftReady | AttemptStatus::DerivingState
        )
    })
}

/// Outcome of the autofix precondition checked under the sink lock.
enum AutofixPrecondition {
    /// campaign_id / conversation_id / attempt_id failed typed validation.
    Typed(ProductionPostprocessError),
    /// Scope+attempt identity matches but status is no longer writable
    /// (superseded / committed / failed). Zero-write, non-fatal no-op.
    NotWritable,
}

/// Map a locked TurnRecord into an autofix precondition (zero-write).
fn autofix_precondition(
    record: &storyforge_domain::turn::TurnRecord,
    identity: &PostprocessIdentity,
) -> AutofixPrecondition {
    if record.campaign_id != identity.campaign_id {
        return AutofixPrecondition::Typed(ProductionPostprocessError::ScopeMismatch {
            field: "campaign_id",
            expected: identity.campaign_id.to_string(),
            actual: record.campaign_id.to_string(),
        });
    }
    if record.conversation_id != identity.conversation_id {
        return AutofixPrecondition::Typed(ProductionPostprocessError::ScopeMismatch {
            field: "conversation_id",
            expected: identity.conversation_id.to_string(),
            actual: record.conversation_id.to_string(),
        });
    }
    if record.find_attempt(&identity.attempt_id).is_none() {
        return AutofixPrecondition::Typed(ProductionPostprocessError::AttemptMissing {
            turn_id: identity.turn_id.to_string(),
            attempt_id: identity.attempt_id.to_string(),
        });
    }
    AutofixPrecondition::NotWritable
}

/// JSON TurnStore sink used by harness and isolated unit tests.
pub struct JsonTurnAttemptSink<'a> {
    pub turn_store: &'a TurnStore,
}

impl TurnAttemptSink for JsonTurnAttemptSink<'_> {
    fn load_turn(
        &self,
        turn_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        Ok(self.turn_store.get_turn(turn_id))
    }

    fn sync_autofix(
        &self,
        identity: &PostprocessIdentity,
        final_text: &str,
        report: QualityReport,
    ) -> Result<(), ProductionPostprocessError> {
        // Capture typed validation failure under the same lock as the write decision.
        let mut precondition: Option<AutofixPrecondition> = None;
        let applied = self
            .turn_store
            .mutate_if(
                &identity.turn_id,
                |record| {
                    if record.campaign_id != identity.campaign_id
                        || record.conversation_id != identity.conversation_id
                        || record.find_attempt(&identity.attempt_id).is_none()
                        || !is_attempt_writable_for_autofix(record, &identity.attempt_id)
                    {
                        precondition = Some(autofix_precondition(record, identity));
                        return false;
                    }
                    true
                },
                |record| {
                    if let Some(att) = record.find_attempt_mut(&identity.attempt_id) {
                        sync_attempt_after_autofix(att, final_text, report);
                    }
                    record.touch();
                },
            )
            .map_err(ProductionPostprocessError::AutofixSync)?;
        if applied {
            Ok(())
        } else {
            match precondition {
                Some(AutofixPrecondition::Typed(err)) => Err(err),
                // Concurrent supersede / late status: leave durable state untouched.
                Some(AutofixPrecondition::NotWritable) | None => Ok(()),
            }
        }
    }

    fn attach_postprocess(
        &self,
        identity: &PostprocessIdentity,
        batch: Option<MutationBatch>,
        derivation: DerivationComponents,
    ) -> Result<bool, String> {
        self.turn_store
            .mutate_if(
                &identity.turn_id,
                |record| {
                    record.campaign_id == identity.campaign_id
                        && record.conversation_id == identity.conversation_id
                        && is_current_attempt_ready_for_postprocess(record, &identity.attempt_id)
                },
                |record| {
                    if let Some(att) = record.find_attempt_mut(&identity.attempt_id) {
                        apply_postprocess_to_attempt(att, batch, derivation);
                    }
                    record.status = TurnStatus::AwaitingAcceptance;
                    record.touch();
                },
            )
            .map_err(|e| e.to_string())
    }

    fn mark_failed(&self, turn_id: &Id, reason: String) -> Result<(), String> {
        self.turn_store
            .with_turn_mut(turn_id, |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(reason);
                record.touch();
            })
            .map_err(|e| e.to_string())
    }
}

/// Validate campaign/conversation identity against the durable Turn record.
pub fn validate_identity_scope(
    record: &storyforge_domain::turn::TurnRecord,
    identity: &PostprocessIdentity,
) -> Result<(), ProductionPostprocessError> {
    if record.campaign_id != identity.campaign_id {
        return Err(ProductionPostprocessError::ScopeMismatch {
            field: "campaign_id",
            expected: identity.campaign_id.to_string(),
            actual: record.campaign_id.to_string(),
        });
    }
    if record.conversation_id != identity.conversation_id {
        return Err(ProductionPostprocessError::ScopeMismatch {
            field: "conversation_id",
            expected: identity.conversation_id.to_string(),
            actual: record.conversation_id.to_string(),
        });
    }
    if record.find_attempt(&identity.attempt_id).is_none() {
        return Err(ProductionPostprocessError::AttemptMissing {
            turn_id: identity.turn_id.to_string(),
            attempt_id: identity.attempt_id.to_string(),
        });
    }
    Ok(())
}

/// Snapshot required to build Chronicle A / knowledge / task mutations.
#[derive(Debug, Clone)]
pub struct PostprocessPersistContext {
    pub campaign_id: Id,
    pub conversation_id: Id,
    pub turn: u32,
}

impl PostprocessPersistContext {
    pub fn from_identity(identity: &PostprocessIdentity) -> Self {
        Self {
            campaign_id: identity.campaign_id.clone(),
            conversation_id: identity.conversation_id.clone(),
            turn: identity.turn_number,
        }
    }
}

/// How mutation batches are built (JSON store vs pure runtime snapshot).
pub enum MutationBatchSource<'a> {
    JsonStore(&'a CampaignStore),
    Runtime(&'a CampaignRuntimeContext),
}

/// Shared production postprocess application service.
pub struct ProductionPostprocessService<'a> {
    pub batch_source: MutationBatchSource<'a>,
    pub sink: &'a dyn TurnAttemptSink,
}

impl<'a> ProductionPostprocessService<'a> {
    pub fn new_json(campaign_store: &'a CampaignStore, sink: &'a dyn TurnAttemptSink) -> Self {
        Self {
            batch_source: MutationBatchSource::JsonStore(campaign_store),
            sink,
        }
    }

    pub fn new_runtime(runtime: &'a CampaignRuntimeContext, sink: &'a dyn TurnAttemptSink) -> Self {
        Self {
            batch_source: MutationBatchSource::Runtime(runtime),
            sink,
        }
    }

    /// Sync quality_report + draft_hash onto the target attempt after autofix.
    ///
    /// Identity/status validation is atomic inside the sink. Typed `ScopeMismatch`
    /// / `AttemptMissing` must propagate unchanged so adapters never `mark_failed`.
    /// Storage failure must also propagate so callers never return fixed text with a stale hash.
    pub fn sync_autofix_attempt(
        &self,
        identity: &PostprocessIdentity,
        final_text: &str,
        report: QualityReport,
    ) -> Result<(), ProductionPostprocessError> {
        self.sink.sync_autofix(identity, final_text, report)
    }

    /// Map runner output into DerivationComponents (summary/state tracked separately).
    pub fn derive_components(outcome: &Option<PostProcessOutcome>) -> DerivationComponents {
        match outcome {
            None => DerivationComponents {
                summary_derivation: DerivationStatus::Disabled,
                state_derivation: DerivationStatus::Disabled,
            },
            Some(o) => DerivationComponents {
                summary_derivation: if o.summary.is_some() {
                    DerivationStatus::Succeeded
                } else {
                    DerivationStatus::Disabled
                },
                state_derivation: if let Some(pp) = &o.post_process {
                    if pp.parse_succeeded {
                        DerivationStatus::Succeeded
                    } else {
                        DerivationStatus::Failed
                    }
                } else {
                    DerivationStatus::Disabled
                },
            },
        }
    }

    /// Build a Prepared MutationBatch (Chronicle A + candidates) without writing Campaign.
    pub fn build_mutation_batch(
        &self,
        persist_ctx: &PostprocessPersistContext,
        outcome: &PostProcessOutcome,
        present_chars: &[String],
    ) -> Result<MutationBatch, ProductionPostprocessError> {
        match self.batch_source {
            MutationBatchSource::JsonStore(store) => Ok(build_json_mutation_batch(
                store,
                persist_ctx,
                outcome,
                present_chars,
            )),
            MutationBatchSource::Runtime(runtime) => {
                if runtime.campaign.id != persist_ctx.campaign_id {
                    return Err(ProductionPostprocessError::BatchConstruction(format!(
                        "postprocess campaign scope mismatch: runtime={}, requested={}",
                        runtime.campaign.id, persist_ctx.campaign_id
                    )));
                }
                Ok(build_runtime_mutation_batch(
                    persist_ctx,
                    outcome,
                    present_chars,
                    runtime,
                ))
            }
        }
    }

    /// Attach an already-produced outcome onto the active Attempt with cancel/late guards.
    pub fn apply_outcome(
        &self,
        identity: &PostprocessIdentity,
        outcome: Option<PostProcessOutcome>,
        present_chars: &[String],
        cancel: &watch::Receiver<bool>,
    ) -> Result<ProductionPostprocessResult, ProductionPostprocessError> {
        // Cancel first: never build or attach mutation candidates after cancellation.
        if *cancel.borrow() {
            return Ok(ProductionPostprocessResult {
                applied: false,
                skipped_reason: Some("cancelled".into()),
                derivation: DerivationComponents {
                    summary_derivation: DerivationStatus::Disabled,
                    state_derivation: DerivationStatus::Disabled,
                },
                summary_text: None,
                batch: None,
                outcome: None,
            });
        }

        let record = self
            .sink
            .load_turn(&identity.turn_id)
            .map_err(ProductionPostprocessError::Storage)?
            .ok_or_else(|| {
                ProductionPostprocessError::Storage(format!(
                    "TurnRecord {} 不存在",
                    identity.turn_id
                ))
            })?;
        validate_identity_scope(&record, identity)?;

        let derivation = Self::derive_components(&outcome);
        let summary_text = outcome.as_ref().and_then(|o| o.summary.clone());

        // Re-check cancel after potentially slow batch construction inputs are ready.
        if *cancel.borrow() {
            return Ok(ProductionPostprocessResult {
                applied: false,
                skipped_reason: Some("cancelled".into()),
                derivation: DerivationComponents {
                    summary_derivation: DerivationStatus::Disabled,
                    state_derivation: DerivationStatus::Disabled,
                },
                summary_text: None,
                batch: None,
                outcome: None,
            });
        }

        let batch = match &outcome {
            Some(o) => {
                let pc = PostprocessPersistContext::from_identity(identity);
                Some(self.build_mutation_batch(&pc, o, present_chars)?)
            }
            None => None,
        };

        if *cancel.borrow() {
            return Ok(ProductionPostprocessResult {
                applied: false,
                skipped_reason: Some("cancelled".into()),
                derivation: DerivationComponents {
                    summary_derivation: DerivationStatus::Disabled,
                    state_derivation: DerivationStatus::Disabled,
                },
                summary_text: None,
                batch: None,
                outcome: None,
            });
        }

        let applied = self
            .sink
            .attach_postprocess(identity, batch.clone(), derivation.clone())
            .map_err(ProductionPostprocessError::Storage)?;

        if applied {
            Ok(ProductionPostprocessResult {
                applied: true,
                skipped_reason: None,
                derivation,
                summary_text,
                batch,
                outcome,
            })
        } else {
            Ok(ProductionPostprocessResult {
                applied: false,
                skipped_reason: Some("late_or_superseded_attempt".into()),
                derivation,
                summary_text,
                batch: None,
                outcome,
            })
        }
    }

    /// Full shared entry: optional autofix sync → runner → attach.
    pub async fn run(
        &self,
        req: ProductionPostprocessRequest,
        runner: Arc<dyn PostprocessRunner>,
        sync_autofix: bool,
    ) -> Result<ProductionPostprocessResult, ProductionPostprocessError> {
        if sync_autofix
            && let (Some(identity), Some(report)) = (&req.identity, req.quality_report.clone())
        {
            self.sync_autofix_attempt(identity, &req.final_text, report)?;
        }

        if *req.cancel.borrow() {
            return Ok(ProductionPostprocessResult {
                applied: false,
                skipped_reason: Some("cancelled".into()),
                derivation: DerivationComponents {
                    summary_derivation: DerivationStatus::Disabled,
                    state_derivation: DerivationStatus::Disabled,
                },
                summary_text: None,
                batch: None,
                outcome: None,
            });
        }

        let outcome = runner
            .run(&req.final_text, &req.present_chars, req.cancel.clone())
            .await;

        // Cancel can land while the runner is finishing; discard outcome writeback.
        if *req.cancel.borrow() {
            return Ok(ProductionPostprocessResult {
                applied: false,
                skipped_reason: Some("cancelled".into()),
                derivation: DerivationComponents {
                    summary_derivation: DerivationStatus::Disabled,
                    state_derivation: DerivationStatus::Disabled,
                },
                summary_text: None,
                batch: None,
                outcome: None,
            });
        }

        let Some(identity) = &req.identity else {
            return Ok(ProductionPostprocessResult {
                applied: false,
                skipped_reason: Some("no_campaign_identity".into()),
                derivation: Self::derive_components(&outcome),
                summary_text: outcome.as_ref().and_then(|o| o.summary.clone()),
                batch: None,
                outcome,
            });
        };

        self.apply_outcome(identity, outcome, &req.present_chars, &req.cancel)
    }

    /// Mark the Turn Failed when a hard storage/batch error must surface.
    pub fn mark_turn_failed(
        &self,
        turn_id: &Id,
        reason: impl Into<String>,
    ) -> Result<(), ProductionPostprocessError> {
        self.sink
            .mark_failed(turn_id, reason.into())
            .map_err(ProductionPostprocessError::Storage)
    }

    /// Best-effort fail-closed helper for adapters: try mark_failed and combine errors.
    ///
    /// Identity-validation errors must never write / mark the Turn.
    pub fn fail_turn_or_combine(
        &self,
        turn_id: &Id,
        original: ProductionPostprocessError,
    ) -> ProductionPostprocessError {
        if matches!(
            original,
            ProductionPostprocessError::ScopeMismatch { .. }
                | ProductionPostprocessError::AttemptMissing { .. }
        ) {
            return original;
        }
        match self.mark_turn_failed(turn_id, original.to_string()) {
            Ok(()) => original,
            Err(mark_error) => ProductionPostprocessError::MarkFailed {
                original: original.to_string(),
                mark_error: mark_error.to_string(),
            },
        }
    }
}

/// JSON CampaignStore mutation builder (production default backend).
pub fn build_json_mutation_batch(
    store: &CampaignStore,
    persist_ctx: &PostprocessPersistContext,
    outcome: &PostProcessOutcome,
    present_chars: &[String],
) -> MutationBatch {
    let camp_id = &persist_ctx.campaign_id;
    let commit_id = Id::new();
    let expected_revision = store.get_campaign(camp_id).map(|c| c.revision).unwrap_or(0);
    let mut mutations: Vec<Mutation> = vec![];

    if let Some(summary) = &outcome.summary {
        let existing = store.list_summaries(camp_id);
        let next_seq = next_chronicle_a_seq(&existing);
        let code = storyforge_domain::chronicle::ChronicleCode::new(
            storyforge_domain::chronicle::ChronicleLevel::A,
            next_seq,
        );
        let headline = storyforge_domain::chronicle::truncate_headline(summary, 40);
        let lineage = store
            .get_campaign(camp_id)
            .and_then(|c| c.lineage_id)
            .unwrap_or_default();
        mutations.push(Mutation::UpsertSummary(Box::new(
            RoundSummary::new(
                camp_id.clone(),
                persist_ctx.conversation_id.clone(),
                persist_ctx.turn,
                summary.clone(),
            )
            .with_code(code.as_str())
            .with_headline(headline)
            .with_lineage(lineage),
        )));
    }

    if let Some(pp) = &outcome.post_process {
        let present_ids: std::collections::HashSet<String> =
            present_chars.iter().cloned().collect();
        let name_collisions: std::collections::HashSet<String> = {
            let mut name_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for inst in store.list_instances(camp_id) {
                *name_counts.entry(inst.name).or_insert(0) += 1;
            }
            name_counts
                .into_iter()
                .filter(|(_, count)| *count >= 2)
                .map(|(name, _)| name)
                .collect()
        };

        for u in &pp.knowledge_updates {
            let entries = crate::normalize_knowledge_update_for_postprocess(
                store,
                camp_id,
                u,
                persist_ctx.turn,
                &present_ids,
                &name_collisions,
            );
            for entry in entries {
                mutations.push(Mutation::UpsertKnowledge(Box::new(
                    storyforge_domain::turn::KnowledgeMutation {
                        entry_id: entry.id.clone(),
                        campaign_id: entry.campaign_id.clone(),
                        character_id: entry.character_id.clone(),
                        knowledge_text: entry.knowledge_text.clone(),
                        source: entry.source.clone(),
                        source_character_id: entry.source_character_id.clone(),
                        turn_number: entry.turn_number,
                        event_id: entry.event_id.clone(),
                        pinned: entry.pinned,
                        propagation: entry.propagation.clone(),
                    },
                )));
            }
        }

        for vu in &pp.variable_updates {
            if let Some(inst_id) = &vu.instance_id {
                if let Some(inst) = crate::find_instance_by_name_or_id(store, camp_id, inst_id) {
                    let is_present = crate::is_postprocess_instance_present(
                        &inst,
                        inst_id,
                        &present_ids,
                        &name_collisions,
                    );
                    if is_present {
                        mutations.push(Mutation::SetVariable {
                            instance_id: Some(inst.id.clone()),
                            key: vu.key.clone(),
                            value: vu.value.clone(),
                            turn: persist_ctx.turn,
                        });
                    }
                }
            } else {
                mutations.push(Mutation::SetVariable {
                    instance_id: None,
                    key: vu.key.clone(),
                    value: vu.value.clone(),
                    turn: persist_ctx.turn,
                });
            }
        }

        for tu in &pp.task_updates {
            if let Some(tid) = &tu.task_id {
                if let Some(task) = store.get_task(tid)
                    && let Some(task) = crate::normalize_task_update_for_postprocess(
                        camp_id,
                        task,
                        tu.new_status.clone(),
                    )
                {
                    mutations.push(Mutation::SetTaskStatus {
                        task_id: task.id.clone(),
                        status: task.status.clone(),
                    });
                }
            } else if let Some(spec) = &tu.new_task {
                let new_task = storyforge_domain::story_task::StoryTask::from_narrative(
                    camp_id.clone(),
                    spec.title.clone(),
                    spec.description.clone(),
                    spec.triggers.clone(),
                    persist_ctx.turn,
                );
                mutations.push(Mutation::UpsertNewTask(Box::new(new_task)));
            }
        }
    }

    MutationBatch {
        commit_id,
        expected_revision,
        target_revision: expected_revision + 1,
        status: MutationBatchStatus::Prepared,
        mutations,
    }
}

/// Pure runtime-based builder (no JSON store reads). Used by SQLite opt-in path.
pub fn build_runtime_mutation_batch(
    persist_ctx: &PostprocessPersistContext,
    outcome: &PostProcessOutcome,
    present_chars: &[String],
    runtime: &CampaignRuntimeContext,
) -> MutationBatch {
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, KnowledgeSource, PropagationPolicy,
    };
    use storyforge_domain::turn::KnowledgeMutation;

    let campaign = &runtime.campaign;
    let mut mutations = Vec::new();
    let present_ids: std::collections::HashSet<String> = present_chars.iter().cloned().collect();
    let name_collisions: std::collections::HashSet<String> = {
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for instance in &runtime.instances {
            *counts.entry(instance.name.clone()).or_default() += 1;
        }
        counts
            .into_iter()
            .filter_map(|(name, count)| (count >= 2).then_some(name))
            .collect()
    };
    let resolve_instance = |value: &Id| {
        runtime
            .instances
            .iter()
            .find(|instance| instance.id == *value || instance.name == value.as_str())
            .cloned()
    };
    let is_group_member = |instance: &storyforge_domain::campaign::CharacterInstance,
                           group: &str| {
        instance
            .definition_id
            .as_ref()
            .and_then(|id| runtime.definitions_by_id.get(id))
            .and_then(|definition| definition.group.as_deref())
            == Some(group)
    };
    let source_entry_for = |source_id: &Id, text: &str| {
        runtime
            .knowledge
            .iter()
            .filter(|entry| entry.character_id == *source_id)
            .filter(|entry| crate::knowledge_text_matches(&entry.knowledge_text, text))
            .max_by(|left, right| {
                left.turn_number
                    .cmp(&right.turn_number)
                    .then_with(|| left.id.as_str().cmp(right.id.as_str()))
            })
            .cloned()
    };
    let source_policy_blocks =
        |update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
         target: Option<&storyforge_domain::campaign::CharacterInstance>| {
            let propagating =
                update.broadcast.is_some() || matches!(update.source, KnowledgeSource::ToldByOther);
            if !propagating {
                return false;
            }
            let Some(source_raw) = update.source_character_id.as_ref() else {
                return false;
            };
            let Some(source) = resolve_instance(source_raw) else {
                return false;
            };
            let Some(entry) = source_entry_for(&source.id, &update.knowledge_text) else {
                return false;
            };
            match &entry.propagation {
                PropagationPolicy::Open => false,
                PropagationPolicy::Private => true,
                PropagationPolicy::GroupRestricted(group) => match (&update.broadcast, target) {
                    (Some(BroadcastTarget::Group(target_group)), _) => target_group != group,
                    (Some(BroadcastTarget::All), _) => true,
                    (None, Some(target_instance)) => !is_group_member(target_instance, group),
                    (None, None) => true,
                },
            }
        };

    if let Some(summary) = &outcome.summary {
        // Match previous SQLite opt-in builder: use turn number as A-seq when
        // the runtime snapshot has no live summary index to scan.
        let code = storyforge_domain::chronicle::ChronicleCode::new(
            storyforge_domain::chronicle::ChronicleLevel::A,
            persist_ctx.turn,
        );
        let lineage = campaign.lineage_id.clone().unwrap_or_default();
        mutations.push(Mutation::UpsertSummary(Box::new(
            RoundSummary::new(
                persist_ctx.campaign_id.clone(),
                persist_ctx.conversation_id.clone(),
                persist_ctx.turn,
                summary.clone(),
            )
            .with_code(code.as_str())
            .with_headline(storyforge_domain::chronicle::truncate_headline(summary, 40))
            .with_lineage(lineage),
        )));
    }

    if let Some(postprocess) = &outcome.post_process {
        for update in &postprocess.knowledge_updates {
            if update.propagation == PropagationPolicy::Private && update.broadcast.is_some() {
                continue;
            }
            if source_policy_blocks(update, None) {
                continue;
            }
            let source_instance = update
                .source_character_id
                .as_ref()
                .and_then(resolve_instance);
            let source_character_id = source_instance.as_ref().map(|instance| instance.id.clone());
            let targets: Vec<_> = match &update.broadcast {
                Some(BroadcastTarget::All) => runtime
                    .instances
                    .iter()
                    .filter(|instance| Some(&instance.id) != source_character_id.as_ref())
                    .cloned()
                    .collect(),
                Some(BroadcastTarget::Group(group)) => runtime
                    .instances
                    .iter()
                    .filter(|instance| {
                        Some(&instance.id) != source_character_id.as_ref()
                            && is_group_member(instance, group)
                    })
                    .cloned()
                    .collect(),
                None => resolve_instance(&update.character_id).into_iter().collect(),
            };

            for target in targets {
                if source_policy_blocks(update, Some(&target)) {
                    continue;
                }
                let presence_exempt = matches!(
                    update.source,
                    KnowledgeSource::ToldByOther | KnowledgeSource::Backstory
                );
                if update.broadcast.is_none()
                    && !presence_exempt
                    && (present_ids.is_empty()
                        || !crate::is_postprocess_instance_present(
                            &target,
                            &update.character_id,
                            &present_ids,
                            &name_collisions,
                        ))
                {
                    continue;
                }
                mutations.push(Mutation::UpsertKnowledge(Box::new(KnowledgeMutation {
                    entry_id: Id::new(),
                    campaign_id: persist_ctx.campaign_id.clone(),
                    character_id: target.id,
                    knowledge_text: update.knowledge_text.clone(),
                    source: if update.broadcast.is_some() {
                        KnowledgeSource::ToldByOther
                    } else {
                        update.source.clone()
                    },
                    source_character_id: source_character_id.clone(),
                    turn_number: persist_ctx.turn,
                    event_id: None,
                    pinned: update.pinned,
                    propagation: update.propagation.clone(),
                })));
            }
        }

        for update in &postprocess.variable_updates {
            if let Some(instance_raw) = &update.instance_id {
                if let Some(instance) = resolve_instance(instance_raw)
                    && crate::is_postprocess_instance_present(
                        &instance,
                        instance_raw,
                        &present_ids,
                        &name_collisions,
                    )
                {
                    mutations.push(Mutation::SetVariable {
                        instance_id: Some(instance.id),
                        key: update.key.clone(),
                        value: update.value.clone(),
                        turn: persist_ctx.turn,
                    });
                }
            } else {
                mutations.push(Mutation::SetVariable {
                    instance_id: None,
                    key: update.key.clone(),
                    value: update.value.clone(),
                    turn: persist_ctx.turn,
                });
            }
        }

        for update in &postprocess.task_updates {
            if let Some(task_id) = &update.task_id {
                if let Some(task) = runtime
                    .tasks
                    .iter()
                    .find(|task| task.id == *task_id)
                    .cloned()
                    && let Some(task) = crate::normalize_task_update_for_postprocess(
                        &persist_ctx.campaign_id,
                        task,
                        update.new_status.clone(),
                    )
                {
                    mutations.push(Mutation::SetTaskStatus {
                        task_id: task.id,
                        status: task.status,
                    });
                }
            } else if let Some(spec) = &update.new_task {
                mutations.push(Mutation::UpsertNewTask(Box::new(
                    storyforge_domain::story_task::StoryTask::from_narrative(
                        persist_ctx.campaign_id.clone(),
                        spec.title.clone(),
                        spec.description.clone(),
                        spec.triggers.clone(),
                        persist_ctx.turn,
                    ),
                )));
            }
        }
    }

    MutationBatch {
        commit_id: Id::new(),
        expected_revision: campaign.revision,
        target_revision: campaign.revision + 1,
        status: MutationBatchStatus::Prepared,
        mutations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn_lifecycle;
    use std::sync::Arc;
    use storyforge_app_conversation::ConversationStore;
    use storyforge_domain::agent::{PostProcessResult, VariableUpdate};
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::turn::{AttemptStatus, TurnRecord};

    struct Fx {
        _data_dir: std::path::PathBuf,
        campaign_store: Arc<CampaignStore>,
        turn_store: Arc<TurnStore>,
        conv_store: Arc<ConversationStore>,
        campaign_id: Id,
        conversation_id: Id,
    }

    impl Fx {
        fn new(label: &str) -> Self {
            let data_dir =
                std::env::temp_dir().join(format!("sf_pp_{label}_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&data_dir).unwrap();
            let campaign_store = Arc::new(CampaignStore::new(&data_dir));
            let turn_store = Arc::new(TurnStore::new(&data_dir));
            let conv_store = Arc::new(ConversationStore::new(data_dir.join("conversations")));
            let mut campaign = Campaign::new(Id::new(), label);
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

        fn sink(&self) -> JsonTurnAttemptSink<'_> {
            JsonTurnAttemptSink {
                turn_store: &self.turn_store,
            }
        }

        fn service<'s>(
            &'s self,
            sink: &'s JsonTurnAttemptSink<'s>,
        ) -> ProductionPostprocessService<'s> {
            ProductionPostprocessService::new_json(&self.campaign_store, sink)
        }

        fn seed_draft_attempt(&self, draft: &str) -> (Id, Id, Id) {
            let variant_id = self
                .conv_store
                .append_ai_draft(&self.conversation_id, draft.to_string(), None)
                .unwrap();
            let camp = self.campaign_store.get_campaign(&self.campaign_id).unwrap();
            let attempt =
                turn_lifecycle::new_draft_attempt(Id::new(), variant_id.clone(), draft, vec![]);
            let attempt_id = attempt.attempt_id.clone();
            let mut record = TurnRecord::new(
                self.campaign_id.clone(),
                self.conversation_id.clone(),
                Id::from_str("input"),
                camp.revision,
            );
            record.status = TurnStatus::DraftReady;
            record.attempts.push(attempt);
            let turn_id = record.turn_id.clone();
            self.turn_store.create_turn(record).unwrap();
            (turn_id, attempt_id, variant_id)
        }
    }

    fn sample_outcome(summary: &str) -> PostProcessOutcome {
        PostProcessOutcome {
            summary: Some(summary.into()),
            post_process: Some(PostProcessResult {
                variable_updates: vec![VariableUpdate {
                    instance_id: None,
                    key: "story_clock".into(),
                    value: serde_json::json!("Day 2"),
                }],
                parse_succeeded: true,
                ..Default::default()
            }),
        }
    }

    #[tokio::test]
    async fn happy_path_aligns_identity_scope_and_candidates() {
        let fx = Fx::new("happy");
        let draft = "码头灯火摇曳，角色低声约定银鸦标记。".repeat(3);
        let (turn_id, attempt_id, _variant) = fx.seed_draft_attempt(&draft);
        let identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: fx.campaign_id.clone(),
            conversation_id: fx.conversation_id.clone(),
            turn_number: 1,
        };
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let runner: Arc<dyn PostprocessRunner> = Arc::new(FixedPostprocessRunner {
            outcome: Some(sample_outcome("第1轮：银鸦标记在码头确立。")),
        });
        let sink = fx.sink();
        let result = fx
            .service(&sink)
            .run(
                ProductionPostprocessRequest {
                    identity: Some(identity),
                    final_text: draft.clone(),
                    quality_report: Some(QualityReport { warnings: vec![] }),
                    present_chars: vec![],
                    cancel: cancel_rx,
                },
                runner,
                true,
            )
            .await
            .expect("postprocess should apply");

        assert!(result.applied);
        assert_eq!(
            result.summary_text.as_deref(),
            Some("第1轮：银鸦标记在码头确立。")
        );
        let batch = result.batch.expect("batch");
        assert!(
            batch
                .mutations
                .iter()
                .any(|m| matches!(m, Mutation::UpsertSummary(_)))
        );
        assert!(batch.mutations.iter().any(|m| matches!(
            m,
            Mutation::SetVariable { key, .. } if key == "story_clock"
        )));
        let summary = batch
            .mutations
            .iter()
            .find_map(|m| match m {
                Mutation::UpsertSummary(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(summary.campaign_id, fx.campaign_id);
        assert_eq!(summary.conversation_id, fx.conversation_id);
        assert_eq!(summary.turn, 1);
        assert!(summary.code.as_deref().unwrap_or("").starts_with('A'));

        let turn = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(turn.status, TurnStatus::AwaitingAcceptance);
        let att = turn.find_attempt(&attempt_id).unwrap();
        assert_eq!(att.status, AttemptStatus::AwaitingAcceptance);
        assert_eq!(att.draft_hash, turn_lifecycle::compute_draft_hash(&draft));
        assert!(att.pending_state_changes.is_some());
        assert_eq!(
            att.derivation.as_ref().unwrap().summary_derivation,
            DerivationStatus::Succeeded
        );
        assert_eq!(
            att.derivation.as_ref().unwrap().state_derivation,
            DerivationStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn late_or_cancelled_results_do_not_write_current_turn() {
        let fx = Fx::new("late");
        let draft = "旧 Attempt 的迟到结果不得写回。".repeat(3);
        let (turn_id, old_id, variant) = fx.seed_draft_attempt(&draft);

        // Supersede + new attempt (regenerate).
        let new_text = "新 Attempt 才是当前草稿。".repeat(3);
        fx.conv_store
            .edit_variant(&fx.conversation_id, &variant, new_text.clone())
            .unwrap();
        let new_attempt =
            turn_lifecycle::new_draft_attempt(Id::new(), variant.clone(), &new_text, vec![]);
        let new_id = new_attempt.attempt_id.clone();
        fx.turn_store
            .with_turn_mut(&turn_id, |record| {
                turn_lifecycle::append_regenerate_attempt(record, new_attempt);
            })
            .unwrap();

        let sink = fx.sink();
        let service = fx.service(&sink);
        let (_tx, cancel_rx) = watch::channel(false);
        let late = service
            .apply_outcome(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: old_id.clone(),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                Some(sample_outcome("迟到摘要")),
                &[],
                &cancel_rx,
            )
            .unwrap();
        assert!(!late.applied);
        assert_eq!(
            late.skipped_reason.as_deref(),
            Some("late_or_superseded_attempt")
        );

        let (cancel_tx, cancel_rx2) = watch::channel(false);
        let _ = cancel_tx.send(true);
        let cancelled = service
            .apply_outcome(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: new_id.clone(),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                Some(sample_outcome("取消后摘要")),
                &[],
                &cancel_rx2,
            )
            .unwrap();
        assert!(!cancelled.applied);
        assert_eq!(cancelled.skipped_reason.as_deref(), Some("cancelled"));

        let turn = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(turn.status, TurnStatus::DraftReady);
        assert!(
            turn.find_attempt(&old_id)
                .unwrap()
                .pending_state_changes
                .is_none()
        );
        assert!(
            turn.find_attempt(&new_id)
                .unwrap()
                .pending_state_changes
                .is_none()
        );
    }

    #[tokio::test]
    async fn agent_degrade_still_finishes_attempt_with_explicit_derivation() {
        let fx = Fx::new("degrade");
        let draft = "Summarizer 降级时 Attempt 终态仍可接受。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&draft);
        let (_tx, cancel_rx) = watch::channel(false);
        let runner: Arc<dyn PostprocessRunner> = Arc::new(FixedPostprocessRunner {
            outcome: Some(PostProcessOutcome {
                summary: None,
                post_process: Some(PostProcessResult {
                    parse_succeeded: false,
                    ..Default::default()
                }),
            }),
        });
        let sink = fx.sink();
        let result = fx
            .service(&sink)
            .run(
                ProductionPostprocessRequest {
                    identity: Some(PostprocessIdentity {
                        turn_id: turn_id.clone(),
                        attempt_id: attempt_id.clone(),
                        campaign_id: fx.campaign_id.clone(),
                        conversation_id: fx.conversation_id.clone(),
                        turn_number: 1,
                    }),
                    final_text: draft,
                    quality_report: None,
                    present_chars: vec![],
                    cancel: cancel_rx,
                },
                runner,
                false,
            )
            .await
            .unwrap();
        assert!(result.applied);
        assert_eq!(
            result.derivation.summary_derivation,
            DerivationStatus::Disabled
        );
        assert_eq!(result.derivation.state_derivation, DerivationStatus::Failed);
        let turn = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(turn.status, TurnStatus::AwaitingAcceptance);
        assert_eq!(
            turn.find_attempt(&attempt_id).unwrap().status,
            AttemptStatus::AwaitingAcceptance
        );
    }

    #[tokio::test]
    async fn autofix_sync_aligns_text_and_draft_hash() {
        let fx = Fx::new("autofix");
        let original = "原始草稿正文足够长。".repeat(3);
        let fixed = "修复后草稿正文足够长且不同。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&original);
        let sink = fx.sink();
        fx.service(&sink)
            .sync_autofix_attempt(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                &fixed,
                QualityReport { warnings: vec![] },
            )
            .unwrap();
        let att = fx
            .turn_store
            .get_turn(&turn_id)
            .unwrap()
            .find_attempt(&attempt_id)
            .unwrap()
            .clone();
        assert_eq!(att.draft_hash, turn_lifecycle::compute_draft_hash(&fixed));
        assert_ne!(
            att.draft_hash,
            turn_lifecycle::compute_draft_hash(&original)
        );
        assert!(att.quality_report.is_some());
    }

    #[tokio::test]
    async fn repeat_apply_is_idempotent_without_duplicate_write() {
        let fx = Fx::new("idempotent");
        let draft = "重复 apply 不得二次改写 Attempt。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&draft);
        let identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: fx.campaign_id.clone(),
            conversation_id: fx.conversation_id.clone(),
            turn_number: 1,
        };
        let (_tx, cancel_rx) = watch::channel(false);
        let sink = fx.sink();
        let service = fx.service(&sink);
        let first = service
            .apply_outcome(&identity, Some(sample_outcome("唯一摘要")), &[], &cancel_rx)
            .unwrap();
        assert!(first.applied);
        let commit_id = first
            .batch
            .as_ref()
            .map(|b| b.commit_id.clone())
            .expect("batch");

        let second = service
            .apply_outcome(
                &identity,
                Some(sample_outcome("重复摘要应被守卫拒绝")),
                &[],
                &cancel_rx,
            )
            .unwrap();
        assert!(!second.applied);
        assert_eq!(
            second.skipped_reason.as_deref(),
            Some("late_or_superseded_attempt")
        );

        let att = fx
            .turn_store
            .get_turn(&turn_id)
            .unwrap()
            .find_attempt(&attempt_id)
            .unwrap()
            .clone();
        let kept = att.pending_state_changes.unwrap();
        assert_eq!(kept.commit_id, commit_id);
        let summary_count = kept
            .mutations
            .iter()
            .filter(|m| matches!(m, Mutation::UpsertSummary(_)))
            .count();
        assert_eq!(summary_count, 1);
    }

    #[tokio::test]
    async fn cancel_after_runner_discards_outcome_and_does_not_await_acceptance() {
        let fx = Fx::new("cancel_race");
        let draft = "取消必须丢弃已完成 runner 的 outcome。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&draft);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let runner: Arc<dyn PostprocessRunner> = Arc::new(CancellableFixedRunner {
            outcome: Some(sample_outcome("取消后不应写回")),
            cancel_tx,
        });
        let sink = fx.sink();
        let result = fx
            .service(&sink)
            .run(
                ProductionPostprocessRequest {
                    identity: Some(PostprocessIdentity {
                        turn_id: turn_id.clone(),
                        attempt_id: attempt_id.clone(),
                        campaign_id: fx.campaign_id.clone(),
                        conversation_id: fx.conversation_id.clone(),
                        turn_number: 1,
                    }),
                    final_text: draft,
                    quality_report: None,
                    present_chars: vec![],
                    cancel: cancel_rx,
                },
                runner,
                false,
            )
            .await
            .unwrap();
        assert!(!result.applied);
        assert_eq!(result.skipped_reason.as_deref(), Some("cancelled"));
        assert!(result.batch.is_none());
        assert!(result.outcome.is_none());
        let turn = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(turn.status, TurnStatus::DraftReady);
        assert!(
            turn.find_attempt(&attempt_id)
                .unwrap()
                .pending_state_changes
                .is_none()
        );
        assert!(turn.find_attempt(&attempt_id).unwrap().derivation.is_none());
    }

    #[tokio::test]
    async fn scope_mismatch_campaign_or_conversation_writes_nothing() {
        let fx = Fx::new("scope");
        let draft = "跨 scope 写回必须零 mutation。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&draft);
        let sink = fx.sink();
        let service = fx.service(&sink);
        let (_tx, cancel_rx) = watch::channel(false);

        let bad_campaign = service
            .apply_outcome(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: Id::from_str("other-campaign"),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                Some(sample_outcome("跨 campaign")),
                &[],
                &cancel_rx,
            )
            .expect_err("campaign mismatch must fail");
        assert!(matches!(
            bad_campaign,
            ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                ..
            }
        ));

        let bad_conversation = service
            .apply_outcome(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: Id::from_str("other-conversation"),
                    turn_number: 1,
                },
                Some(sample_outcome("跨 conversation")),
                &[],
                &cancel_rx,
            )
            .expect_err("conversation mismatch must fail");
        assert!(matches!(
            bad_conversation,
            ProductionPostprocessError::ScopeMismatch {
                field: "conversation_id",
                ..
            }
        ));

        // Same revision number on a different campaign must still fail by id, not revision.
        let mut foreign = Campaign::new(Id::new(), "foreign");
        foreign.lineage_id = Some(Id::new());
        let foreign_id = foreign.id.clone();
        fx.campaign_store.save_campaign(foreign).unwrap();
        let same_rev_other_campaign = service
            .apply_outcome(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: foreign_id,
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                Some(sample_outcome("revision 相同仍越界")),
                &[],
                &cancel_rx,
            )
            .expect_err("same revision different campaign must fail");
        assert!(matches!(
            same_rev_other_campaign,
            ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                ..
            }
        ));

        let turn = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(turn.status, TurnStatus::DraftReady);
        assert!(
            turn.find_attempt(&attempt_id)
                .unwrap()
                .pending_state_changes
                .is_none()
        );
        assert!(fx.campaign_store.list_summaries(&fx.campaign_id).is_empty());
    }

    #[tokio::test]
    async fn missing_attempt_returns_error_not_ok() {
        let fx = Fx::new("missing_attempt");
        let draft = "缺失 Attempt 必须 Err。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&draft);
        let sink = fx.sink();
        let before = fx.turn_store.get_turn(&turn_id).unwrap();
        let err = fx
            .service(&sink)
            .sync_autofix_attempt(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: Id::from_str("ghost-attempt"),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                "fixed",
                QualityReport { warnings: vec![] },
            )
            .expect_err("missing attempt must not Ok");
        assert!(
            matches!(err, ProductionPostprocessError::AttemptMissing { .. }),
            "unexpected: {err}"
        );
        // Typed validation errors must not mark Failed or mutate draft_hash.
        let after = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(after.status, before.status);
        assert_eq!(after.failure_reason, before.failure_reason);
        assert_eq!(
            after.find_attempt(&attempt_id).unwrap().draft_hash,
            before.find_attempt(&attempt_id).unwrap().draft_hash
        );

        let (_tx, cancel_rx) = watch::channel(false);
        let apply_err = fx
            .service(&sink)
            .apply_outcome(
                &PostprocessIdentity {
                    turn_id,
                    attempt_id: Id::from_str("ghost-attempt"),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                Some(sample_outcome("ghost")),
                &[],
                &cancel_rx,
            )
            .expect_err("apply missing attempt must Err");
        assert!(matches!(
            apply_err,
            ProductionPostprocessError::AttemptMissing { .. }
        ));
    }

    #[tokio::test]
    async fn sync_autofix_preserves_typed_scope_and_attempt_errors() {
        let fx = Fx::new("sync_autofix_typed");
        let draft = "typed identity validation 必须保留。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&draft);
        let sink = fx.sink();
        let service = fx.service(&sink);
        let before = fx.turn_store.get_turn(&turn_id).unwrap();
        let original_hash = before.find_attempt(&attempt_id).unwrap().draft_hash.clone();

        let camp_err = service
            .sync_autofix_attempt(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: Id::from_str("other-campaign"),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                "must-not-write",
                QualityReport { warnings: vec![] },
            )
            .expect_err("cross campaign must fail");
        assert!(matches!(
            camp_err,
            ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                ..
            }
        ));
        // service_fail_turn / fail_turn_or_combine must not reclassify or mark.
        let combined = service.fail_turn_or_combine(&turn_id, camp_err);
        assert!(matches!(
            combined,
            ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                ..
            }
        ));

        let conv_err = service
            .sync_autofix_attempt(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: Id::from_str("other-conversation"),
                    turn_number: 1,
                },
                "must-not-write",
                QualityReport { warnings: vec![] },
            )
            .expect_err("cross conversation must fail");
        assert!(matches!(
            conv_err,
            ProductionPostprocessError::ScopeMismatch {
                field: "conversation_id",
                ..
            }
        ));

        // Concurrent supersede: zero-write, no mark_failed.
        fx.turn_store
            .with_turn_mut(&turn_id, |record| {
                if let Some(att) = record.find_attempt_mut(&attempt_id) {
                    att.status = AttemptStatus::Superseded;
                }
                record.touch();
            })
            .unwrap();
        let superseded_before = fx.turn_store.get_turn(&turn_id).unwrap();
        service
            .sync_autofix_attempt(
                &PostprocessIdentity {
                    turn_id: turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: fx.campaign_id.clone(),
                    conversation_id: fx.conversation_id.clone(),
                    turn_number: 1,
                },
                "superseded-must-not-write",
                QualityReport { warnings: vec![] },
            )
            .expect("superseded autofix is non-fatal zero-write");
        let superseded_after = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(superseded_after.status, superseded_before.status);
        assert_eq!(
            superseded_after.failure_reason,
            superseded_before.failure_reason
        );
        assert_eq!(
            superseded_after
                .find_attempt(&attempt_id)
                .unwrap()
                .draft_hash,
            original_hash
        );
        assert_eq!(
            superseded_after.find_attempt(&attempt_id).unwrap().status,
            AttemptStatus::Superseded
        );
        assert!(fx.campaign_store.list_summaries(&fx.campaign_id).is_empty());
    }

    #[tokio::test]
    async fn storage_attach_failure_propagates_and_fail_turn_combines_mark_errors() {
        let fx = Fx::new("storage_fail");
        let draft = "存储失败必须向上返回。".repeat(3);
        let (turn_id, attempt_id, _) = fx.seed_draft_attempt(&draft);
        let identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: fx.campaign_id.clone(),
            conversation_id: fx.conversation_id.clone(),
            turn_number: 1,
        };
        let sink = FailingAttachSink {
            inner: fx.sink(),
            fail_attach: true,
            fail_mark: false,
        };
        let service = ProductionPostprocessService::new_json(&fx.campaign_store, &sink);
        let (_tx, cancel_rx) = watch::channel(false);
        let err = service
            .apply_outcome(&identity, Some(sample_outcome("存储失败")), &[], &cancel_rx)
            .expect_err("attach storage failure must propagate");
        assert!(matches!(err, ProductionPostprocessError::Storage(_)));

        // mark_failed success path after storage failure.
        let sink_ok_mark = FailingAttachSink {
            inner: fx.sink(),
            fail_attach: true,
            fail_mark: false,
        };
        let service_ok = ProductionPostprocessService::new_json(&fx.campaign_store, &sink_ok_mark);
        let combined = service_ok.fail_turn_or_combine(
            &turn_id,
            ProductionPostprocessError::Storage("attach boom".into()),
        );
        assert!(matches!(combined, ProductionPostprocessError::Storage(_)));
        assert_eq!(
            fx.turn_store.get_turn(&turn_id).unwrap().status,
            TurnStatus::Failed
        );

        // mark_failed itself failing must combine errors, never silent Ok.
        let sink_fail_mark = FailingAttachSink {
            inner: fx.sink(),
            fail_attach: true,
            fail_mark: true,
        };
        let service_fail =
            ProductionPostprocessService::new_json(&fx.campaign_store, &sink_fail_mark);
        let mark_err = service_fail.fail_turn_or_combine(
            &turn_id,
            ProductionPostprocessError::Storage("attach boom".into()),
        );
        assert!(matches!(
            mark_err,
            ProductionPostprocessError::MarkFailed { .. }
        ));
    }

    /// Cancels the shared cancel channel while producing a fixed outcome.
    struct CancellableFixedRunner {
        outcome: Option<PostProcessOutcome>,
        cancel_tx: watch::Sender<bool>,
    }

    #[async_trait]
    impl PostprocessRunner for CancellableFixedRunner {
        async fn run(
            &self,
            _final_text: &str,
            _present_chars: &[String],
            _cancel: watch::Receiver<bool>,
        ) -> Option<PostProcessOutcome> {
            let _ = self.cancel_tx.send(true);
            self.outcome.clone()
        }
    }

    struct FailingAttachSink<'a> {
        inner: JsonTurnAttemptSink<'a>,
        fail_attach: bool,
        fail_mark: bool,
    }

    impl TurnAttemptSink for FailingAttachSink<'_> {
        fn load_turn(
            &self,
            turn_id: &Id,
        ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
            self.inner.load_turn(turn_id)
        }

        fn sync_autofix(
            &self,
            identity: &PostprocessIdentity,
            final_text: &str,
            report: QualityReport,
        ) -> Result<(), ProductionPostprocessError> {
            self.inner.sync_autofix(identity, final_text, report)
        }

        fn attach_postprocess(
            &self,
            identity: &PostprocessIdentity,
            batch: Option<MutationBatch>,
            derivation: DerivationComponents,
        ) -> Result<bool, String> {
            if self.fail_attach {
                return Err("injected attach storage failure".into());
            }
            self.inner.attach_postprocess(identity, batch, derivation)
        }

        fn mark_failed(&self, turn_id: &Id, reason: String) -> Result<(), String> {
            if self.fail_mark {
                return Err("injected mark_failed storage failure".into());
            }
            self.inner.mark_failed(turn_id, reason)
        }
    }
}

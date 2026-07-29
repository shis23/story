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
use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::{PipelineOrchestrator, RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::{PipelineEvent, RoundSummary};
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::conversation::Provenance;
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

/// Shared inputs for the production DraftQualityGate + bounded one-shot Editor
/// auto-fix. Tauri commands and the SQLite endurance harness use this exact
/// orchestration path.
pub struct QualityAutofixRequest<'a> {
    pub pipeline: &'a mut PipelineOrchestrator,
    pub draft_node_id: &'a Id,
    pub conversation_id: &'a Id,
    pub writing_ctx: &'a WritingContext,
    pub event_tx: &'a tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    pub cancel: watch::Receiver<bool>,
    pub log_prefix: &'a str,
    /// 当前已落 Draft 的原始溯源；二次质量门失败时与原稿一同恢复。
    pub original_provenance: Option<storyforge_domain::conversation::Provenance>,
}

fn quality_report_needs_editor_autofix(report: &QualityReport) -> bool {
    !report.passed()
}

fn autofix_result_can_replace_original(report: &QualityReport) -> bool {
    !report.has_errors()
}

/// Run the deterministic NarrativeContract quality gate, then perform at most
/// one real Editor-only regenerate whenever any finding exists. The full report
/// is returned as the Editor hint, including Warning-level style findings.
pub async fn run_quality_gate_with_optional_editor_autofix(
    mut final_text: String,
    request: QualityAutofixRequest<'_>,
) -> Result<
    (
        String,
        QualityReport,
        Option<storyforge_domain::conversation::Provenance>,
    ),
    storyforge_app_pipeline::PipelineError,
> {
    let QualityAutofixRequest {
        pipeline,
        draft_node_id,
        conversation_id,
        writing_ctx,
        event_tx,
        cancel,
        log_prefix,
        original_provenance,
    } = request;
    let original_text = final_text.clone();
    let contract = pipeline
        .session()
        .and_then(|session| session.plan.as_ref())
        .map(|plan| {
            storyforge_domain::narrative_contract::NarrativeContract::from_plan_and_runtime(
                plan,
                writing_ctx.campaign_runtime.as_deref(),
            )
        });
    let mut quality_report = storyforge_app_pipeline::quality_gate::run_quality_gate_with_contract(
        &final_text,
        contract.as_ref(),
    );
    emit_quality_checked(event_tx, &quality_report);

    if !quality_report.passed() {
        for warning in &quality_report.warnings {
            tracing::info!(target: "quality_gate", "{log_prefix} quality warning: {:?}", warning.code);
        }
    }

    if quality_report_needs_editor_autofix(&quality_report) {
        let hint = storyforge_app_pipeline::quality_gate::build_quality_fix_hint(&quality_report);
        tracing::info!(
            target: "quality_gate",
            "{log_prefix} quality errors={}, warnings={}, trying bounded 1x Editor auto-fix",
            quality_report.error_count(),
            quality_report.warnings.len()
        );
        let regenerate = RegenerateRequest {
            conversation_id: conversation_id.clone(),
            node_id: draft_node_id.clone(),
            targets: vec![PartialRollTarget::Editor],
            generation_mode: None,
            hint: Some(hint),
            seed: None,
        };
        match pipeline
            .regenerate(regenerate, writing_ctx, event_tx.clone(), cancel)
            .await
        {
            Ok((fixed_text, fixed_provenance)) => {
                let contract = pipeline
                    .session()
                    .and_then(|session| session.plan.as_ref())
                    .map(|plan| {
                        storyforge_domain::narrative_contract::NarrativeContract::from_plan_and_runtime(
                            plan,
                            writing_ctx.campaign_runtime.as_deref(),
                        )
                    });
                let fixed_report =
                    storyforge_app_pipeline::quality_gate::run_quality_gate_with_contract(
                        &fixed_text,
                        contract.as_ref(),
                    );
                emit_quality_checked(event_tx, &fixed_report);
                if !autofix_result_can_replace_original(&fixed_report) {
                    tracing::warn!(
                        target: "quality_gate",
                        "{log_prefix} bounded Editor auto-fix still has {} error(s); restoring original draft",
                        fixed_report.error_count()
                    );
                    pipeline.sync_autofix_draft(
                        conversation_id,
                        draft_node_id,
                        original_text.clone(),
                        original_provenance.clone(),
                    )?;
                    final_text = original_text;
                    quality_report =
                        storyforge_app_pipeline::quality_gate::run_quality_gate_with_contract(
                            &final_text,
                            contract.as_ref(),
                        );
                    emit_quality_checked(event_tx, &quality_report);
                } else {
                    final_text = fixed_text;
                    quality_report = fixed_report;
                    return Ok((final_text, quality_report, Some(fixed_provenance)));
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "quality_gate",
                    "{log_prefix} bounded Editor auto-fix failed; retaining original draft: {error}"
                );
            }
        }
    }

    Ok((final_text, quality_report, None))
}

fn emit_quality_checked(
    event_tx: &tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    report: &QualityReport,
) {
    let warnings = report
        .warnings
        .iter()
        .map(|warning| warning.message.clone())
        .collect();
    let _ = event_tx.send(PipelineEvent::QualityChecked {
        passed: report.passed(),
        warning_count: report.warnings.len(),
        error_count: report.error_count(),
        warnings,
    });
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

    /// 与 `sync_autofix` 相同，但当 Editor auto-fix 真正被采用时，同时把最终
    /// provenance/reasoning 与正文、draft_hash 原子对齐。默认实现保持旧适配器兼容。
    fn sync_autofix_with_provenance(
        &self,
        identity: &PostprocessIdentity,
        final_text: &str,
        report: QualityReport,
        _provenance: Option<Provenance>,
    ) -> Result<(), ProductionPostprocessError> {
        self.sync_autofix(identity, final_text, report)
    }

    /// Returns Ok(true) when the attempt was current and writeback applied.
    /// Scope and attempt presence must already be validated by the service.
    fn attach_postprocess(
        &self,
        identity: &PostprocessIdentity,
        batch: Option<MutationBatch>,
        derivation: DerivationComponents,
    ) -> Result<bool, String>;

    /// Conditionally mark Turn Failed only when `identity` still owns the current
    /// writable Attempt. Must re-check campaign/conversation/attempt under the same
    /// durable write. Returns Ok(true) when marked, Ok(false) when zero-write
    /// (superseded / cancelled / not current), Err on storage failure.
    fn mark_failed_if_current(
        &self,
        identity: &PostprocessIdentity,
        reason: String,
    ) -> Result<bool, String>;
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

    fn mark_failed_if_current(
        &self,
        identity: &PostprocessIdentity,
        reason: String,
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
                    record.status = TurnStatus::Failed;
                    record.failure_reason = Some(reason);
                    record.touch();
                },
            )
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

    pub fn sync_autofix_attempt_with_provenance(
        &self,
        identity: &PostprocessIdentity,
        final_text: &str,
        report: QualityReport,
        provenance: Option<Provenance>,
    ) -> Result<(), ProductionPostprocessError> {
        self.sink
            .sync_autofix_with_provenance(identity, final_text, report, provenance)
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
                } else if o.summary_attempted {
                    DerivationStatus::Failed
                } else {
                    DerivationStatus::Disabled
                },
                state_derivation: if let Some(pp) = &o.post_process {
                    if pp.parse_succeeded {
                        DerivationStatus::Succeeded
                    } else {
                        DerivationStatus::Failed
                    }
                } else if o.post_process_attempted {
                    DerivationStatus::Failed
                } else {
                    DerivationStatus::Disabled
                },
            },
        }
    }

    /// Build a Prepared MutationBatch (Chronicle A + candidates) without writing Campaign.
    ///
    /// `pending_temporary_instances`: attempt 上尚未落盘的临时角色（accept 时才经
    /// `Mutation::UpsertInstance` 前置落库）。批构建阶段必须能解析它们，否则本轮
    /// 临时角色的知识/变量目标会被静默跳过（V2）。
    pub fn build_mutation_batch(
        &self,
        persist_ctx: &PostprocessPersistContext,
        outcome: &PostProcessOutcome,
        present_chars: &[String],
        pending_temporary_instances: &[storyforge_domain::campaign::CharacterInstance],
    ) -> Result<MutationBatch, ProductionPostprocessError> {
        match self.batch_source {
            MutationBatchSource::JsonStore(store) => Ok(build_json_mutation_batch(
                store,
                persist_ctx,
                outcome,
                present_chars,
                pending_temporary_instances,
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
                    pending_temporary_instances,
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

        // V2: attempt 上挂着的临时角色在 accept 前尚未进 store/runtime 快照，
        // 批构建必须把它们纳入解析域，否则其知识/变量更新被静默丢弃。
        let pending_temporary_instances: Vec<storyforge_domain::campaign::CharacterInstance> =
            record
                .attempts
                .iter()
                .find(|attempt| attempt.attempt_id == identity.attempt_id)
                .map(|attempt| attempt.pending_temporary_instances.clone())
                .unwrap_or_default();

        let batch = match &outcome {
            Some(o) => {
                let pc = PostprocessPersistContext::from_identity(identity);
                Some(self.build_mutation_batch(
                    &pc,
                    o,
                    present_chars,
                    &pending_temporary_instances,
                )?)
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

    /// Mark the Turn Failed only when `identity` still owns the current writable Attempt.
    ///
    /// Returns Ok(true) when marked; Ok(false) when the identity is no longer current
    /// (superseded / cancelled) and durable state was left untouched.
    pub fn mark_turn_failed_if_current(
        &self,
        identity: &PostprocessIdentity,
        reason: impl Into<String>,
    ) -> Result<bool, ProductionPostprocessError> {
        self.sink
            .mark_failed_if_current(identity, reason.into())
            .map_err(ProductionPostprocessError::Storage)
    }

    /// Best-effort fail-closed helper for adapters: try mark_failed_if_current and combine.
    ///
    /// Identity-validation errors never write. Storage/BatchConstruction only mark
    /// Failed when `identity` is still the current Attempt; otherwise zero-write.
    pub fn fail_turn_or_combine(
        &self,
        identity: &PostprocessIdentity,
        original: ProductionPostprocessError,
    ) -> ProductionPostprocessError {
        if matches!(
            original,
            ProductionPostprocessError::ScopeMismatch { .. }
                | ProductionPostprocessError::AttemptMissing { .. }
        ) {
            return original;
        }
        match self.mark_turn_failed_if_current(identity, original.to_string()) {
            Ok(_marked) => original,
            Err(mark_error) => ProductionPostprocessError::MarkFailed {
                original: original.to_string(),
                mark_error: mark_error.to_string(),
            },
        }
    }
}

/// JSON CampaignStore mutation builder (production default backend).
///
/// 与 SQLite 路径共用同一组纯解析函数（`build_knowledge_mutations` /
/// `build_variable_mutations` / `build_task_mutations`）；本函数只负责把 store
/// 当前状态投影成 `CampaignRuntimeContext` 快照，再委托共享段。这样 knowledge /
/// variable / task 的目标解析、在场/同名收紧、广播分发与传播策略只实现一次。
///
/// 后端差异**仅**保留在两处（均为既有有意行为，不可削弱）：
/// - Chronicle A seq：JSON 扫描 `store.list_summaries` 取 max+1（容忍 code 间隙）。
/// - revision 基线：从 `store.get_campaign` 实时读取（CAS 写入需要当前值）。
///
/// `pending_temporary_instances` 参与名字/ID 解析（V2）：accept 时
/// `prepare_commit_batch` 会把它们的 `UpsertInstance` 前置，指向其 id 的
/// mutation 落库安全。
pub fn build_json_mutation_batch(
    store: &CampaignStore,
    persist_ctx: &PostprocessPersistContext,
    outcome: &PostProcessOutcome,
    present_chars: &[String],
    pending_temporary_instances: &[storyforge_domain::campaign::CharacterInstance],
) -> MutationBatch {
    let camp_id = &persist_ctx.campaign_id;
    let commit_id = Id::new();
    let campaign = store.get_campaign(camp_id);
    let expected_revision = campaign.as_ref().map(|c| c.revision).unwrap_or(0);
    let mut mutations: Vec<Mutation> = vec![];

    // Chronicle A seq：扫描已有摘要（容忍 code 间隙/重排），runtime 快照做不到这一点。
    if let Some(summary) = &outcome.summary {
        let existing = store.list_summaries(camp_id);
        let next_seq = next_chronicle_a_seq(&existing);
        let lineage = campaign
            .as_ref()
            .and_then(|c| c.lineage_id.clone())
            .unwrap_or_default();
        mutations.push(build_summary_mutation(
            persist_ctx,
            summary,
            next_seq,
            lineage,
        ));
    }

    if let Some(pp) = &outcome.post_process {
        // 把 store 当前状态投影成 runtime 快照，交给共享段（与 SQLite 路径同函数）。
        let runtime = project_store_runtime_context(store, camp_id, campaign.as_ref());
        let instances = merge_temporary_instances(&runtime, pending_temporary_instances);
        let present_ids: std::collections::HashSet<String> =
            present_chars.iter().cloned().collect();
        let name_collisions = compute_name_collisions(&instances);

        mutations.extend(build_knowledge_mutations(
            persist_ctx,
            pp,
            &present_ids,
            &instances,
            &name_collisions,
            &runtime,
        ));
        mutations.extend(build_variable_mutations(
            persist_ctx,
            pp,
            &present_ids,
            &instances,
            &name_collisions,
        ));
        mutations.extend(build_task_mutations(persist_ctx, pp, &runtime));
    }

    MutationBatch {
        commit_id,
        expected_revision,
        target_revision: expected_revision + 1,
        status: MutationBatchStatus::Prepared,
        mutations,
    }
}

/// 把 JSON `CampaignStore` 的当前状态投影成纯 domain `CampaignRuntimeContext`。
///
/// 供 `build_json_mutation_batch` 委托共享段使用；与 `runtime_support` 启动期组装的
/// runtime 快照语义一致（同源 instances / knowledge / tasks / definitions）。
fn project_store_runtime_context(
    store: &CampaignStore,
    camp_id: &Id,
    campaign: Option<&storyforge_domain::campaign::Campaign>,
) -> CampaignRuntimeContext {
    let campaign = campaign
        .cloned()
        .unwrap_or_else(|| storyforge_domain::campaign::Campaign::new(Id::from_str("missing"), ""));
    let instances = store.list_instances(camp_id);
    let knowledge = store.list_knowledge(camp_id);
    let tasks = store.list_tasks(camp_id);
    // definitions_by_id：遍历所有 card 的 character_definitions（与既有
    // `instance_matches_group` 的 store.list_cards 扫描同源）。
    let definitions_by_id = store
        .list_cards()
        .into_iter()
        .flat_map(|stored| stored.card.character_definitions)
        .map(|def| (def.id.clone(), def))
        .collect();
    CampaignRuntimeContext {
        campaign,
        instances,
        definitions_by_id,
        knowledge,
        tasks,
        // turn：共享段统一用 persist_ctx.turn；快照字段不参与 builder 计算，置 0。
        turn: 0,
    }
}

// ─── 共享纯函数：postprocess mutation 解析域（JSON 与 SQLite 共用） ───────────
//
// 以下函数把 runtime builder 原先以闭包实现的解析/在场/同名/广播/传播规则提升为
// 模块级纯函数。JSON 与 SQLite 两条路径都通过同一组函数构建 MutationBatch 的
// knowledge / variable / task 段，确保字段语义一致；差异只存在于 Chronicle A seq
// 与 revision 基线的来源（见各自 builder 的 summary/revision 段）。

/// 有效实例集 = runtime 快照实例 + attempt 挂载的临时实例（按 id 去重，跨 campaign 拒绝）。
fn merge_temporary_instances(
    runtime: &CampaignRuntimeContext,
    pending_temporary_instances: &[storyforge_domain::campaign::CharacterInstance],
) -> Vec<storyforge_domain::campaign::CharacterInstance> {
    let mut all = runtime.instances.clone();
    for temp in pending_temporary_instances {
        if temp.campaign_id == runtime.campaign.id && !all.iter().any(|i| i.id == temp.id) {
            all.push(temp.clone());
        }
    }
    all
}

/// campaign 内出现 ≥2 次的 name 集合（同名时 name 路失效，逼 id）。
fn compute_name_collisions(
    instances: &[storyforge_domain::campaign::CharacterInstance],
) -> std::collections::HashSet<String> {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for instance in instances {
        *counts.entry(instance.name.clone()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(name, count)| (count >= 2).then_some(name))
        .collect()
}

/// 按 id 或 name 解析 instance（id 优先）。
fn resolve_instance_by_id_or_name(
    instances: &[storyforge_domain::campaign::CharacterInstance],
    value: &Id,
) -> Option<storyforge_domain::campaign::CharacterInstance> {
    instances
        .iter()
        .find(|instance| instance.id == *value)
        .or_else(|| {
            instances
                .iter()
                .find(|instance| instance.name == value.as_str())
        })
        .cloned()
}

/// instance 是否属于指定 group（通过 definition_id 反查 definitions_by_id）。
fn instance_matches_group(
    runtime: &CampaignRuntimeContext,
    instance: &storyforge_domain::campaign::CharacterInstance,
    group: &str,
) -> bool {
    instance
        .definition_id
        .as_ref()
        .and_then(|id| runtime.definitions_by_id.get(id))
        .and_then(|definition| definition.group.as_deref())
        == Some(group)
}

/// 在 runtime.knowledge 中查找 source 的匹配条目（最新 turn，相同 text 规则）。
fn source_entry_for(
    runtime: &CampaignRuntimeContext,
    source_id: &Id,
    text: &str,
) -> Option<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
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
}

/// 判断一条知识更新是否被源端传播策略阻止（Open=放行，Private=阻止，
/// GroupRestricted=按 group/target 判定）。
///
/// `resolve`/`group_member`/`source_lookup` 以闭包传入，让本函数完全脱离具体数据源
/// （JSON store 与 runtime 快照都能复用同一逻辑）。
fn source_propagation_blocks(
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    target: Option<&storyforge_domain::campaign::CharacterInstance>,
    resolve: impl Fn(&Id) -> Option<storyforge_domain::campaign::CharacterInstance>,
    group_member: impl Fn(&storyforge_domain::campaign::CharacterInstance, &str) -> bool,
    source_lookup: impl Fn(
        &Id,
        &str,
    ) -> Option<storyforge_domain::character_knowledge::PropagationPolicy>,
) -> bool {
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, KnowledgeSource, PropagationPolicy,
    };

    let propagating =
        update.broadcast.is_some() || matches!(update.source, KnowledgeSource::ToldByOther);
    if !propagating {
        return false;
    }
    let Some(source_raw) = update.source_character_id.as_ref() else {
        return false;
    };
    let Some(source) = resolve(source_raw) else {
        return false;
    };
    let Some(policy) = source_lookup(&source.id, &update.knowledge_text) else {
        return false;
    };
    match policy {
        PropagationPolicy::Open => false,
        PropagationPolicy::Private => true,
        PropagationPolicy::GroupRestricted(group) => match (&update.broadcast, target) {
            (Some(BroadcastTarget::Group(target_group)), _) => target_group != &group,
            (Some(BroadcastTarget::All), _) => true,
            (None, Some(target_instance)) => !group_member(target_instance, &group),
            (None, None) => true,
        },
    }
}

/// 构建知识段 mutation（广播分发 + 单目标 + 在场/同名/传播策略收紧）。
///
/// 返回的 KnowledgeMutation 序列与原 runtime builder 逐条等价；JSON 与 SQLite
/// 共用本函数后，character_id / source / propagation 等字段不再因后端不同而分叉。
fn build_knowledge_mutations(
    persist_ctx: &PostprocessPersistContext,
    postprocess: &storyforge_domain::agent::PostProcessResult,
    present_ids: &std::collections::HashSet<String>,
    instances: &[storyforge_domain::campaign::CharacterInstance],
    name_collisions: &std::collections::HashSet<String>,
    runtime: &CampaignRuntimeContext,
) -> Vec<Mutation> {
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, KnowledgeSource, PropagationPolicy,
    };
    use storyforge_domain::turn::KnowledgeMutation;

    let resolve = |value: &Id| resolve_instance_by_id_or_name(instances, value);
    let group_member = |inst: &storyforge_domain::campaign::CharacterInstance, g: &str| {
        instance_matches_group(runtime, inst, g)
    };
    let source_lookup =
        |sid: &Id, text: &str| source_entry_for(runtime, sid, text).map(|e| e.propagation);

    let mut mutations = Vec::new();
    for update in &postprocess.knowledge_updates {
        if update.propagation == PropagationPolicy::Private && update.broadcast.is_some() {
            continue;
        }
        if source_propagation_blocks(update, None, resolve, group_member, source_lookup) {
            continue;
        }
        let source_instance = update.source_character_id.as_ref().and_then(resolve);
        let source_character_id = source_instance.as_ref().map(|i| i.id.clone());
        let targets: Vec<_> = match &update.broadcast {
            Some(BroadcastTarget::All) => instances
                .iter()
                .filter(|instance| Some(&instance.id) != source_character_id.as_ref())
                .cloned()
                .collect(),
            Some(BroadcastTarget::Group(group)) => instances
                .iter()
                .filter(|instance| {
                    Some(&instance.id) != source_character_id.as_ref()
                        && instance_matches_group(runtime, instance, group)
                })
                .cloned()
                .collect(),
            None => resolve(&update.character_id).into_iter().collect(),
        };

        for target in targets {
            if source_propagation_blocks(
                update,
                Some(&target),
                resolve,
                group_member,
                source_lookup,
            ) {
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
                        present_ids,
                        name_collisions,
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
    mutations
}

/// 构建变量段 mutation（角色级按 name/id 解析 + 在场/同名收紧；全局级无约束）。
fn build_variable_mutations(
    persist_ctx: &PostprocessPersistContext,
    postprocess: &storyforge_domain::agent::PostProcessResult,
    present_ids: &std::collections::HashSet<String>,
    instances: &[storyforge_domain::campaign::CharacterInstance],
    name_collisions: &std::collections::HashSet<String>,
) -> Vec<Mutation> {
    let mut mutations = Vec::new();
    for update in &postprocess.variable_updates {
        if let Some(instance_raw) = &update.instance_id {
            if let Some(instance) = resolve_instance_by_id_or_name(instances, instance_raw)
                && crate::is_postprocess_instance_present(
                    &instance,
                    instance_raw,
                    present_ids,
                    name_collisions,
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
    mutations
}

/// 构建任务段 mutation（已有任务状态更新 + 新建任务）。
fn build_task_mutations(
    persist_ctx: &PostprocessPersistContext,
    postprocess: &storyforge_domain::agent::PostProcessResult,
    runtime: &CampaignRuntimeContext,
) -> Vec<Mutation> {
    let mut mutations = Vec::new();
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
    mutations
}

/// 构建 Chronicle A 摘要 mutation。
///
/// `a_seq` 由调用方决定：JSON 路径扫描 `store.list_summaries` 取 max+1（容忍 code
/// 间隙/重排），Runtime 路径在没有活动摘要索引时退化为 `persist_ctx.turn`。本函数
/// 只负责按给定 seq 组装 RoundSummary，不再各自计算。
fn build_summary_mutation(
    persist_ctx: &PostprocessPersistContext,
    summary: &str,
    a_seq: u32,
    lineage: Id,
) -> Mutation {
    let code = storyforge_domain::chronicle::ChronicleCode::new(
        storyforge_domain::chronicle::ChronicleLevel::A,
        a_seq,
    );
    Mutation::UpsertSummary(Box::new(
        RoundSummary::new(
            persist_ctx.campaign_id.clone(),
            persist_ctx.conversation_id.clone(),
            persist_ctx.turn,
            summary.to_string(),
        )
        .with_code(code.as_str())
        .with_headline(storyforge_domain::chronicle::truncate_headline(summary, 40))
        .with_lineage(lineage),
    ))
}

/// Pure runtime-based builder (no JSON store reads). Used by SQLite opt-in path.
///
/// `runtime` 是轮前快照；`pending_temporary_instances` 是本 attempt 生成期间
/// 新建、accept 前尚未进快照的临时角色（V2：必须纳入解析域）。
pub fn build_runtime_mutation_batch(
    persist_ctx: &PostprocessPersistContext,
    outcome: &PostProcessOutcome,
    present_chars: &[String],
    runtime: &CampaignRuntimeContext,
    pending_temporary_instances: &[storyforge_domain::campaign::CharacterInstance],
) -> MutationBatch {
    let campaign = &runtime.campaign;
    let instances = merge_temporary_instances(runtime, pending_temporary_instances);
    let present_ids: std::collections::HashSet<String> = present_chars.iter().cloned().collect();
    let name_collisions = compute_name_collisions(&instances);

    let mut mutations = Vec::new();
    if let Some(summary) = &outcome.summary {
        // Runtime 快照不含活动摘要索引：用 turn 号作为 A-seq（与既有 SQLite 行为一致）。
        let lineage = campaign.lineage_id.clone().unwrap_or_default();
        mutations.push(build_summary_mutation(
            persist_ctx,
            summary,
            persist_ctx.turn,
            lineage,
        ));
    }

    if let Some(postprocess) = &outcome.post_process {
        mutations.extend(build_knowledge_mutations(
            persist_ctx,
            postprocess,
            &present_ids,
            &instances,
            &name_collisions,
            runtime,
        ));
        mutations.extend(build_variable_mutations(
            persist_ctx,
            postprocess,
            &present_ids,
            &instances,
            &name_collisions,
        ));
        mutations.extend(build_task_mutations(persist_ctx, postprocess, runtime));
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
            self.seed_draft_attempt_with_temps(draft, vec![])
        }

        fn seed_draft_attempt_with_temps(
            &self,
            draft: &str,
            temps: Vec<storyforge_domain::campaign::CharacterInstance>,
        ) -> (Id, Id, Id) {
            let variant_id = self
                .conv_store
                .append_ai_draft(&self.conversation_id, draft.to_string(), None)
                .unwrap();
            let camp = self.campaign_store.get_campaign(&self.campaign_id).unwrap();
            let attempt =
                turn_lifecycle::new_draft_attempt(Id::new(), variant_id.clone(), draft, temps);
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
            summary_attempted: true,
            post_process_attempted: true,
        }
    }

    #[test]
    fn instance_resolution_prefers_exact_id_over_an_earlier_name_match() {
        let campaign_id = Id::from_str("campaign-id-priority");
        let requested_id = Id::from_str("instance-target-id");

        let mut earlier_name_match = storyforge_domain::campaign::CharacterInstance::temporary(
            campaign_id.clone(),
            requested_id.to_string(),
        );
        earlier_name_match.id = Id::from_str("instance-name-match");

        let mut later_id_match =
            storyforge_domain::campaign::CharacterInstance::temporary(campaign_id, "Actual target");
        later_id_match.id = requested_id.clone();

        let resolved = resolve_instance_by_id_or_name(
            &[earlier_name_match, later_id_match.clone()],
            &requested_id,
        )
        .expect("the exact id must resolve");

        assert_eq!(resolved.id, later_id_match.id);
    }

    #[test]
    fn style_warnings_require_editor_autofix_feedback() {
        use storyforge_domain::turn::{QualitySeverity, QualityWarning, QualityWarningCode};

        let report = QualityReport {
            warnings: vec![
                QualityWarning {
                    code: QualityWarningCode::EmDashDensity { count: 1 },
                    message: "草稿含破折号 1 处".into(),
                    severity: QualitySeverity::Warning,
                },
                QualityWarning {
                    code: QualityWarningCode::NegationThenAffirmation {
                        sample: "不是甲，而是乙".into(),
                    },
                    message: "草稿含否后肯结构".into(),
                    severity: QualitySeverity::Warning,
                },
            ],
        };

        assert!(quality_report_needs_editor_autofix(&report));
    }

    #[test]
    fn every_quality_warning_is_returned_to_editor_once() {
        use storyforge_domain::turn::{QualitySeverity, QualityWarning, QualityWarningCode};

        let report = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::NgramRepetition {
                    n: 4,
                    count: 3,
                    sample: "重复片段".into(),
                },
                message: "轻微重复".into(),
                severity: QualitySeverity::Warning,
            }],
        };

        assert!(quality_report_needs_editor_autofix(&report));
    }

    #[test]
    fn autofix_with_remaining_error_cannot_replace_original() {
        use storyforge_domain::turn::{QualitySeverity, QualityWarning, QualityWarningCode};

        let error_report = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::PrivateKnowledgeLeak {
                    secret_fingerprint: "deadbeef".into(),
                    owner_id: None,
                },
                message: "private leak".into(),
                severity: QualitySeverity::Error,
            }],
        };
        assert!(!autofix_result_can_replace_original(&error_report));

        let warning_only = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::EmDashDensity { count: 1 },
                message: "one dash".into(),
                severity: QualitySeverity::Warning,
            }],
        };
        assert!(autofix_result_can_replace_original(&warning_only));
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

    /// V2 回归：attempt 上未落盘的临时角色必须能被 postprocess 批构建解析。
    /// 之前 build_json_mutation_batch 只查 store.list_instances → 本轮临时角色的
    /// 知识/变量更新被静默丢弃（accept 后永久丢失）。
    #[tokio::test]
    async fn pending_temporary_instances_resolve_in_json_batch() {
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };

        let fx = Fx::new("temp_json");
        let temp = storyforge_domain::campaign::CharacterInstance::temporary_with_overrides(
            fx.campaign_id.clone(),
            "苏禾",
            None,
            None,
        );
        let draft = "集市尽头，临时角色苏禾目睹了银鸦标记的交接。".repeat(3);
        let (turn_id, attempt_id, _variant) =
            fx.seed_draft_attempt_with_temps(&draft, vec![temp.clone()]);
        let identity = PostprocessIdentity {
            turn_id,
            attempt_id,
            campaign_id: fx.campaign_id.clone(),
            conversation_id: fx.conversation_id.clone(),
            turn_number: 1,
        };
        let outcome = PostProcessOutcome {
            summary: None,
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![CharacterKnowledgeUpdate {
                    character_id: Id::from_str("苏禾"),
                    knowledge_text: "银鸦标记在集市完成了交接".into(),
                    source: KnowledgeSource::Witnessed,
                    source_character_id: None,
                    pinned: false,
                    broadcast: None,
                    propagation: PropagationPolicy::Open,
                }],
                variable_updates: vec![VariableUpdate {
                    instance_id: Some(Id::from_str("苏禾")),
                    key: "警觉度".into(),
                    value: serde_json::json!(3),
                }],
                parse_succeeded: true,
                ..Default::default()
            }),
            summary_attempted: false,
            post_process_attempted: true,
        };
        let sink = fx.sink();
        let (_tx, cancel_rx) = watch::channel(false);
        let result = fx
            .service(&sink)
            .apply_outcome(&identity, Some(outcome), &["苏禾".to_string()], &cancel_rx)
            .unwrap();
        assert!(result.applied);
        let batch = result.batch.expect("batch");
        // 知识条目必须解析到临时实例的最终 id
        assert!(
            batch.mutations.iter().any(|m| matches!(
                m,
                Mutation::UpsertKnowledge(k) if k.character_id == temp.id
            )),
            "临时角色的知识更新必须解析到 pending 实例 id：{:?}",
            batch.mutations
        );
        // 变量更新同样必须落到临时实例 id
        assert!(
            batch.mutations.iter().any(|m| matches!(
                m,
                Mutation::SetVariable { instance_id: Some(id), key, .. }
                    if *id == temp.id && key == "警觉度"
            )),
            "临时角色的变量更新必须解析到 pending 实例 id：{:?}",
            batch.mutations
        );
    }

    /// V2 回归（SQLite 路径）：轮前快照 runtime 不含临时实例时，
    /// pending_temporary_instances 参数必须补上解析域；不传则回到旧的静默丢弃。
    #[test]
    fn pending_temporary_instances_resolve_in_runtime_batch() {
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };

        let campaign = Campaign::new(Id::new(), "rt");
        let camp_id = campaign.id.clone();
        let temp = storyforge_domain::campaign::CharacterInstance::temporary_with_overrides(
            camp_id.clone(),
            "苏禾",
            None,
            None,
        );
        let runtime = CampaignRuntimeContext {
            campaign,
            instances: vec![],
            definitions_by_id: Default::default(),
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        };
        let persist_ctx = PostprocessPersistContext {
            campaign_id: camp_id,
            conversation_id: Id::new(),
            turn: 1,
        };
        let outcome = PostProcessOutcome {
            summary: None,
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![CharacterKnowledgeUpdate {
                    character_id: Id::from_str("苏禾"),
                    knowledge_text: "银鸦标记在集市完成了交接".into(),
                    source: KnowledgeSource::Witnessed,
                    source_character_id: None,
                    pinned: false,
                    broadcast: None,
                    propagation: PropagationPolicy::Open,
                }],
                variable_updates: vec![VariableUpdate {
                    instance_id: Some(Id::from_str("苏禾")),
                    key: "警觉度".into(),
                    value: serde_json::json!(3),
                }],
                parse_succeeded: true,
                ..Default::default()
            }),
            summary_attempted: false,
            post_process_attempted: true,
        };
        let present = vec!["苏禾".to_string()];

        let with_temps = build_runtime_mutation_batch(
            &persist_ctx,
            &outcome,
            &present,
            &runtime,
            std::slice::from_ref(&temp),
        );
        assert!(with_temps.mutations.iter().any(|m| matches!(
            m,
            Mutation::UpsertKnowledge(k) if k.character_id == temp.id
        )));
        assert!(with_temps.mutations.iter().any(|m| matches!(
            m,
            Mutation::SetVariable { instance_id: Some(id), .. } if *id == temp.id
        )));

        // 对照：不带 temps 时两条更新都解析失败（旧缺陷行为，证明参数是修复点）
        let without_temps =
            build_runtime_mutation_batch(&persist_ctx, &outcome, &present, &runtime, &[]);
        assert!(!without_temps.mutations.iter().any(|m| matches!(
            m,
            Mutation::UpsertKnowledge(_)
                | Mutation::SetVariable {
                    instance_id: Some(_),
                    ..
                }
        )));
    }

    /// 后端 builder 一致性（Batch 2.3）：给定相同的活动状态、相同 outcome 和
    /// 相同 present_chars，JSON 与 Runtime 路径必须产生**语义相同**的 mutation 序列。
    ///
    /// 一致性规则（已知差异排除后）：
    /// - `entry_id` / `commit_id` 非确定（每路独立 `Id::new()`），忽略。
    /// - Chronicle A seq：JSON 扫 `list_summaries` 取 max+1，Runtime 用 `persist_ctx.turn`
    ///   ——存在 summary 时记录该差异并在统一后断言相等（当前先 skip summary 断言）。
    /// - summary / 变量 / 任务 / 知识条目：character_id / key / value / status / propagation
    ///   必须按相同顺序一一对应。
    ///
    /// 这是一个**保护性契约**：它现在应当 PASS（捕捉当前两条路径的实际差异面），
    /// 一旦 builder 统一后仍必须保持 PASS。
    #[test]
    fn json_and_runtime_builders_produce_equivalent_mutations() {
        use storyforge_domain::character::CharacterCard;
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };
        use storyforge_domain::story_task::{NewTaskSpec, TaskStatus, TaskUpdate};

        // 用 JSON store 建立活动状态，再投影成等效 runtime 快照。
        let fx = Fx::new("parity");
        let camp_id = fx.campaign_id.clone();
        let conv_id = fx.conversation_id.clone();

        // 两个持久化 instance：Lin（有 definition，group=heroes）+ Chen（无 definition）
        let def = storyforge_domain::character::CharacterDefinition {
            id: Id::new(),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm".into(),
            behavior_rules: "rule".into(),
            base_backstory: vec![],
            group: Some("heroes".into()),
            role_type: storyforge_domain::character::RoleType::Protagonist,
            variable_schema: storyforge_domain::variables::default_character_variables(),
        };
        let inst_lin = storyforge_domain::campaign::CharacterInstance {
            id: Id::new(),
            campaign_id: camp_id.clone(),
            definition_id: Some(def.id.clone()),
            name: "Lin".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        let inst_chen = storyforge_domain::campaign::CharacterInstance {
            id: Id::new(),
            campaign_id: camp_id.clone(),
            definition_id: None,
            name: "Chen".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        // 建一张 card 承载 definition（JSON 路径 instance_matches_group 遍历 store.list_cards）
        let card = CharacterCard {
            id: Id::from_str("card-1"),
            name: "c".into(),
            source_character_id: Id::from_str("src-1"),
            character_definitions: vec![def.clone()],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Unknown,
            extraction_message: None,
        };
        fx.campaign_store.save_card(card).unwrap();
        fx.campaign_store.add_instance(inst_lin.clone()).unwrap();
        fx.campaign_store.add_instance(inst_chen.clone()).unwrap();

        let persist_ctx = PostprocessPersistContext {
            campaign_id: camp_id.clone(),
            conversation_id: conv_id.clone(),
            turn: 2,
        };
        // Lin 广播给 Group("heroes") 的一条知识 + Chen 的一条变量 + 一个新任务
        let outcome = PostProcessOutcome {
            summary: None, // summary 路径 seq 策略不同，单独不比较
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![CharacterKnowledgeUpdate {
                    character_id: Id::from_str("Lin"),
                    knowledge_text: "广播给英雄组".into(),
                    source: KnowledgeSource::ToldByOther,
                    source_character_id: Some(Id::from_str("Lin")),
                    pinned: false,
                    broadcast: Some(
                        storyforge_domain::character_knowledge::BroadcastTarget::Group(
                            "heroes".into(),
                        ),
                    ),
                    propagation: PropagationPolicy::GroupRestricted("heroes".into()),
                }],
                variable_updates: vec![VariableUpdate {
                    instance_id: Some(Id::from_str("Chen")),
                    key: "警觉度".into(),
                    value: serde_json::json!(5),
                }],
                task_updates: vec![TaskUpdate {
                    task_id: None,
                    new_status: TaskStatus::Pending,
                    new_task: Some(NewTaskSpec {
                        title: "调查集市".into(),
                        description: "夜间行动".into(),
                        triggers: vec![],
                        related_characters: vec![],
                    }),
                }],
                parse_succeeded: true,
            }),
            summary_attempted: false,
            post_process_attempted: true,
        };
        let present = vec!["Lin".to_string(), "Chen".to_string()];

        // JSON 路径
        let json_batch =
            build_json_mutation_batch(&fx.campaign_store, &persist_ctx, &outcome, &present, &[]);
        // Runtime 快照（与 store 当前持久化状态投影一致）
        let runtime = CampaignRuntimeContext {
            campaign: fx.campaign_store.get_campaign(&camp_id).unwrap(),
            instances: vec![inst_lin.clone(), inst_chen.clone()],
            definitions_by_id: [(def.id.clone(), def.clone())].into_iter().collect(),
            knowledge: vec![],
            tasks: vec![],
            turn: 2,
        };
        let runtime_batch =
            build_runtime_mutation_batch(&persist_ctx, &outcome, &present, &runtime, &[]);

        // helper：把每条 mutation 投影成可比较的稳定签名（剔除非确定 entry_id/commit_id）
        fn signature(m: &Mutation) -> String {
            match m {
                Mutation::UpsertKnowledge(k) => {
                    format!(
                        "K|char={}|text={}|source={:?}|prop={:?}",
                        k.character_id, k.knowledge_text, k.source, k.propagation
                    )
                }
                Mutation::SetVariable {
                    instance_id,
                    key,
                    value,
                    turn,
                } => {
                    format!(
                        "V|inst={:?}|key={}|val={}|turn={}",
                        instance_id, key, value, turn
                    )
                }
                Mutation::UpsertNewTask(t) => {
                    format!("T+|title={}|desc={:?}", t.title, t.description)
                }
                Mutation::SetTaskStatus { task_id, status } => {
                    format!("T=|id={}|status={:?}", task_id, status)
                }
                Mutation::UpsertSummary(s) => {
                    format!("S|turn={}|text={}", s.turn, s.content)
                }
                _ => format!("OTHER|{m:?}"),
            }
        }
        let json_sigs: Vec<String> = json_batch.mutations.iter().map(signature).collect();
        let runtime_sigs: Vec<String> = runtime_batch.mutations.iter().map(signature).collect();
        assert_eq!(
            json_sigs, runtime_sigs,
            "JSON 与 Runtime builder mutation 签名不一致\nJSON:    {json_sigs:?}\nRuntime: {runtime_sigs:?}"
        );

        // revision 基线必须取自同一活动 campaign：JSON 经 get_campaign，Runtime 用快照字段
        let camp_revision = fx
            .campaign_store
            .get_campaign(&camp_id)
            .map(|c| c.revision)
            .unwrap_or(0);
        assert_eq!(json_batch.expected_revision, camp_revision);
        assert_eq!(runtime_batch.expected_revision, camp_revision);
        assert_eq!(json_batch.target_revision, json_batch.expected_revision + 1);
        assert_eq!(
            runtime_batch.target_revision,
            runtime_batch.expected_revision + 1
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
                summary_attempted: true,
                post_process_attempted: true,
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
            DerivationStatus::Failed
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
        let camp_identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: attempt_id.clone(),
            campaign_id: Id::from_str("other-campaign"),
            conversation_id: fx.conversation_id.clone(),
            turn_number: 1,
        };
        let combined = service.fail_turn_or_combine(&camp_identity, camp_err);
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

        // mark_failed_if_current success path after storage failure (identity still current).
        let sink_ok_mark = FailingAttachSink {
            inner: fx.sink(),
            fail_attach: true,
            fail_mark: false,
        };
        let service_ok = ProductionPostprocessService::new_json(&fx.campaign_store, &sink_ok_mark);
        let combined = service_ok.fail_turn_or_combine(
            &identity,
            ProductionPostprocessError::Storage("attach boom".into()),
        );
        assert!(matches!(combined, ProductionPostprocessError::Storage(_)));
        assert_eq!(
            fx.turn_store.get_turn(&turn_id).unwrap().status,
            TurnStatus::Failed
        );

        // Reset to DraftReady for the mark_failed storage-failure case.
        fx.turn_store
            .with_turn_mut(&turn_id, |record| {
                record.status = TurnStatus::DraftReady;
                record.failure_reason = None;
                if let Some(att) = record.find_attempt_mut(&attempt_id) {
                    att.status = AttemptStatus::DraftReady;
                }
                record.touch();
            })
            .unwrap();

        // mark_failed_if_current itself failing must combine errors, never silent Ok.
        let sink_fail_mark = FailingAttachSink {
            inner: fx.sink(),
            fail_attach: true,
            fail_mark: true,
        };
        let service_fail =
            ProductionPostprocessService::new_json(&fx.campaign_store, &sink_fail_mark);
        let mark_err = service_fail.fail_turn_or_combine(
            &identity,
            ProductionPostprocessError::Storage("attach boom".into()),
        );
        assert!(matches!(
            mark_err,
            ProductionPostprocessError::MarkFailed { .. }
        ));
    }

    #[tokio::test]
    async fn late_storage_or_batch_error_after_regenerate_does_not_fail_new_attempt() {
        let fx = Fx::new("late_mark_after_regen");
        let draft = "旧 postprocess 在 regenerate 后不得标 Failed。".repeat(3);
        let (turn_id, old_attempt_id, variant_id) = fx.seed_draft_attempt(&draft);
        let old_identity = PostprocessIdentity {
            turn_id: turn_id.clone(),
            attempt_id: old_attempt_id.clone(),
            campaign_id: fx.campaign_id.clone(),
            conversation_id: fx.conversation_id.clone(),
            turn_number: 1,
        };

        // Simulate regenerate: supersede old attempt, create a new current DraftReady attempt.
        let new_attempt_id = Id::new();
        fx.turn_store
            .with_turn_mut(&turn_id, |record| {
                if let Some(old) = record.find_attempt_mut(&old_attempt_id) {
                    old.status = AttemptStatus::Superseded;
                }
                let mut new_attempt = turn_lifecycle::new_draft_attempt(
                    new_attempt_id.clone(),
                    variant_id.clone(),
                    "regenerated draft",
                    vec![],
                );
                new_attempt.status = AttemptStatus::DraftReady;
                record.attempts.push(new_attempt);
                record.status = TurnStatus::DraftReady;
                record.failure_reason = None;
                record.touch();
            })
            .unwrap();
        let before = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(before.status, TurnStatus::DraftReady);
        assert_eq!(
            before.find_attempt(&new_attempt_id).unwrap().status,
            AttemptStatus::DraftReady
        );

        // Old background postprocess hits BatchConstruction / Storage after regenerate.
        let sink = FailingAttachSink {
            inner: fx.sink(),
            fail_attach: true,
            fail_mark: false,
        };
        let service = ProductionPostprocessService::new_json(&fx.campaign_store, &sink);

        for original in [
            ProductionPostprocessError::BatchConstruction("late batch boom".into()),
            ProductionPostprocessError::Storage("late attach boom".into()),
        ] {
            let combined = service.fail_turn_or_combine(&old_identity, original);
            assert!(
                matches!(
                    combined,
                    ProductionPostprocessError::BatchConstruction(_)
                        | ProductionPostprocessError::Storage(_)
                ),
                "unexpected: {combined}"
            );
            let after = fx.turn_store.get_turn(&turn_id).unwrap();
            assert_eq!(
                after.status,
                TurnStatus::DraftReady,
                "old late error must not Fail turn"
            );
            assert_eq!(after.failure_reason, before.failure_reason);
            assert_eq!(
                after.find_attempt(&new_attempt_id).unwrap().status,
                AttemptStatus::DraftReady,
                "new attempt must stay DraftReady"
            );
            assert_eq!(
                after.find_attempt(&old_attempt_id).unwrap().status,
                AttemptStatus::Superseded
            );
            assert!(
                after
                    .find_attempt(&new_attempt_id)
                    .unwrap()
                    .pending_state_changes
                    .is_none()
            );
        }

        // Identity-scoped sink mark itself is also a zero-write.
        let marked = sink
            .mark_failed_if_current(&old_identity, "should not write".into())
            .expect("mark is non-fatal when not current");
        assert!(!marked, "superseded identity must not mark");
        let after_mark = fx.turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(after_mark.status, TurnStatus::DraftReady);
        assert_eq!(after_mark.failure_reason, None);
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

        fn mark_failed_if_current(
            &self,
            identity: &PostprocessIdentity,
            reason: String,
        ) -> Result<bool, String> {
            if self.fail_mark {
                return Err("injected mark_failed storage failure".into());
            }
            self.inner.mark_failed_if_current(identity, reason)
        }
    }
}

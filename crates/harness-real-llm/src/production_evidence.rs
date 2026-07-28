//! M5 多轮 evidence runner。
//!
//! 该模块明确拆分三段：writer（真实入口为 production pipeline）、Chronicle 候选
//! （当前真实入口仅有 synthetic fixture，不是生产 postprocess）、以及
//! `CommitProbeEnv` 的 production-faithful Accept 探针。

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::chronicle::ContextWindowParams;
use tokio::time::Instant as TokioInstant;

use crate::HarnessEnv;
use crate::budget::BudgetedLlmClient;
use crate::commit_probe::{CommitProbeEnv, ProductionAcceptInput};
use crate::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceTurnRecord, EvidenceWriter, short_hash16,
};

#[derive(Debug, Clone)]
pub struct ProductionEvidenceConfig {
    pub turns: u32,
    pub suite: String,
    pub run_id: String,
    pub calls_path: PathBuf,
    pub turns_path: PathBuf,
    pub model_label: String,
    /// 可选的 suite hard deadline；None 时使用剩余 calls × per-call timeout。
    pub hard_deadline: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChronicleCandidateSource {
    /// Harness 生成的确定性 Chronicle A 候选；不等于生产 Summarizer/postprocess。
    SyntheticChronicleFixture,
    /// 通过共享 `ProductionPostprocessService` 写入 Attempt 的生产后处理路径。
    ProductionPostprocessService,
}

impl ChronicleCandidateSource {
    fn evidence_label(self) -> &'static str {
        match self {
            Self::SyntheticChronicleFixture => "synthetic_chronicle_fixture",
            Self::ProductionPostprocessService => "production_postprocess_service",
        }
    }
}

/// Verified proof that ProductionPostprocessService applied durable writeback.
#[derive(Debug, Clone)]
pub struct ProductionPostprocessProof {
    pub turn_id: Id,
    pub attempt_id: Id,
    pub input_node_id: Id,
    pub turn_index: u32,
    pub variant_id: Id,
    pub draft_hash: String,
    pub summary_text: Option<String>,
    /// Canonical digest of durable MutationBatch mutations (order-sensitive).
    pub batch_digest: Option<String>,
    pub applied: bool,
}

/// Stable digest for a prepared MutationBatch used by harness proofs.
pub fn mutation_batch_digest(batch: &storyforge_domain::turn::MutationBatch) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(batch.commit_id.as_str().as_bytes());
    hasher.update(batch.expected_revision.to_le_bytes());
    hasher.update(batch.target_revision.to_le_bytes());
    for mutation in &batch.mutations {
        let encoded = serde_json::to_string(mutation).unwrap_or_default();
        hasher.update(encoded.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone)]
pub struct WrittenProductionTurn {
    pub draft_text: String,
    pub variant_id: Id,
    pub summary_text: Option<String>,
    pub chronicle_source: Option<ChronicleCandidateSource>,
    /// Only set when shared service writeback was verified. Writers cannot
    /// self-certify with a bare bool + chronicle_source pair.
    pub postprocess_proof: Option<ProductionPostprocessProof>,
}

#[async_trait]
pub trait ProductionTurnWriter: Send {
    fn write_path(&self) -> &'static str {
        "custom_writer_fixture"
    }

    async fn write_turn(
        &mut self,
        env: &HarnessEnv,
        ctx: &WritingContext,
        turn_index: u32,
        intent: &str,
    ) -> Result<WrittenProductionTurn, String>;
}

/// 真实入口使用的 writer：复用 `PipelineOrchestrator::start_writing`。
pub struct PipelineProductionTurnWriter;

#[async_trait]
impl ProductionTurnWriter for PipelineProductionTurnWriter {
    fn write_path(&self) -> &'static str {
        "production_pipeline"
    }

    async fn write_turn(
        &mut self,
        env: &HarnessEnv,
        ctx: &WritingContext,
        turn_index: u32,
        intent: &str,
    ) -> Result<WrittenProductionTurn, String> {
        let mut pipeline = env.new_pipeline();
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let (draft_text, variant_id, _provenance) = pipeline
            .start_writing(intent.to_string(), ctx, event_tx, cancel_rx)
            .await
            .map_err(|e| format!("pipeline write failed: {e}"))?;
        if draft_text.trim().is_empty() {
            return Err("pipeline write returned empty draft".into());
        }
        // Real-model pipeline path still does not auto-run production postprocess here:
        // callers that need the shared service should use FixedProductionPostprocessWriter
        // or call env.apply_production_postprocess explicitly.
        let summary_text = Some(format!(
            "第{turn_index}轮已接受；正文指纹 {}。",
            short_hash16(&draft_text)
        ));
        Ok(WrittenProductionTurn {
            draft_text,
            variant_id,
            summary_text,
            chronicle_source: Some(ChronicleCandidateSource::SyntheticChronicleFixture),
            postprocess_proof: None,
        })
    }
}

/// Deterministic writer that routes Chronicle/Attempt writeback through the shared
/// ProductionPostprocessService (no real Summarizer/PostProcessor LLM calls).
pub struct FixedProductionPostprocessWriter {
    pub summary_template: String,
}

#[async_trait]
impl ProductionTurnWriter for FixedProductionPostprocessWriter {
    fn write_path(&self) -> &'static str {
        "production_postprocess_service"
    }

    async fn write_turn(
        &mut self,
        env: &HarnessEnv,
        ctx: &WritingContext,
        turn_index: u32,
        _intent: &str,
    ) -> Result<WrittenProductionTurn, String> {
        let draft = format!(
            "第{turn_index}轮生产后处理正文：雾港调查继续推进，保留足够长度供 Accept 校验。{}",
            "线索稳定。".repeat(8)
        );
        let variant_id = env
            .conv_store
            .append_ai_draft(&ctx.conversation_id, draft.clone(), None)
            .map_err(|e| e.to_string())?;
        let summary = self
            .summary_template
            .replace("{turn}", &turn_index.to_string());
        // input node is the user message for this turn (created by the evidence loop).
        // Fall back to a stable synthetic id only when the conversation has no user node yet.
        let input_node_id = env
            .conv_store
            .get(&ctx.conversation_id)
            .and_then(|c| {
                c.nodes
                    .iter()
                    .rev()
                    .find(|n| {
                        n.active()
                            .is_some_and(|v| v.role == storyforge_domain::conversation::Role::User)
                    })
                    .map(|n| n.id.clone())
            })
            .unwrap_or_else(|| Id::from_str(format!("harness-input-{turn_index}")));
        let proof = env
            .apply_production_postprocess(
                ctx,
                &variant_id,
                &draft,
                Some(summary.clone()),
                turn_index,
                input_node_id,
            )
            .await?;
        Ok(WrittenProductionTurn {
            draft_text: draft,
            variant_id,
            summary_text: Some(summary),
            chronicle_source: Some(ChronicleCandidateSource::ProductionPostprocessService),
            postprocess_proof: Some(proof),
        })
    }
}

/// Verify writer-claimed production postprocess against durable store state.
///
/// `expected_input_node_id` / `expected_turn_index` are the evidence loop's current
/// turn identity. A valid proof from a different input/turn must not pass.
pub fn verify_production_postprocess_claim(
    env: &HarnessEnv,
    campaign_id: &Id,
    conversation_id: &Id,
    expected_input_node_id: &Id,
    expected_turn_index: u32,
    written: &WrittenProductionTurn,
) -> Result<bool, ProductionEvidenceError> {
    match (written.chronicle_source, written.postprocess_proof.as_ref()) {
        (Some(ChronicleCandidateSource::ProductionPostprocessService), None) => {
            Err(ProductionEvidenceError::InvalidConfig(
                "production_postprocess_service claimed without verified proof".into(),
            ))
        }
        (Some(ChronicleCandidateSource::ProductionPostprocessService), Some(proof)) => {
            if !proof.applied {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof marked not applied".into(),
                ));
            }
            if proof.variant_id != written.variant_id {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof variant_id mismatch".into(),
                ));
            }
            // Bind proof to the evidence loop's current turn identity.
            if &proof.input_node_id != expected_input_node_id {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof input_node_id does not match evidence loop turn"
                        .into(),
                ));
            }
            if proof.turn_index != expected_turn_index {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof turn_index does not match evidence loop turn"
                        .into(),
                ));
            }
            let expected_hash =
                storyforge_tauri_app::turn_lifecycle::compute_draft_hash(&written.draft_text);
            if proof.draft_hash != expected_hash {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof draft_hash mismatch".into(),
                ));
            }
            if proof.summary_text.as_ref() != written.summary_text.as_ref() {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof summary mismatch".into(),
                ));
            }
            let turn = env.turn_store.get_turn(&proof.turn_id).ok_or_else(|| {
                ProductionEvidenceError::Store(format!(
                    "missing turn {} for production postprocess proof",
                    proof.turn_id
                ))
            })?;
            if &turn.campaign_id != campaign_id || &turn.conversation_id != conversation_id {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof scope mismatch".into(),
                ));
            }
            if turn.input_node_id != proof.input_node_id
                || &turn.input_node_id != expected_input_node_id
            {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof input_node_id mismatch".into(),
                ));
            }
            if turn.status != storyforge_domain::turn::TurnStatus::AwaitingAcceptance {
                return Err(ProductionEvidenceError::InvalidConfig(format!(
                    "turn status {:?} is not AwaitingAcceptance after production postprocess",
                    turn.status
                )));
            }
            let attempt = turn.find_attempt(&proof.attempt_id).ok_or_else(|| {
                ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof attempt missing".into(),
                )
            })?;
            if attempt.status != storyforge_domain::turn::AttemptStatus::AwaitingAcceptance {
                return Err(ProductionEvidenceError::InvalidConfig(format!(
                    "attempt status {:?} is not AwaitingAcceptance",
                    attempt.status
                )));
            }
            if attempt.variant_id != written.variant_id || attempt.variant_id != proof.variant_id {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof variant mismatch".into(),
                ));
            }
            if attempt.draft_hash != proof.draft_hash {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "durable attempt draft_hash mismatches proof".into(),
                ));
            }
            // Reject reuse of an old attempt proof against a different live attempt.
            if let Some(active) = turn.active_attempt()
                && active.attempt_id != proof.attempt_id
            {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "production postprocess proof targets non-active attempt".into(),
                ));
            }
            if written.summary_text.is_some() {
                let summary_ok = attempt.pending_state_changes.as_ref().is_some_and(|b| {
                    b.mutations.iter().any(|m| match m {
                        storyforge_domain::turn::Mutation::UpsertSummary(s) => {
                            Some(s.content.as_str()) == written.summary_text.as_deref()
                                && s.turn == proof.turn_index
                                && s.turn == expected_turn_index
                        }
                        _ => false,
                    })
                });
                if !summary_ok {
                    return Err(ProductionEvidenceError::InvalidConfig(
                        "summary claimed but durable batch lacks exact UpsertSummary".into(),
                    ));
                }
            }
            match (&proof.batch_digest, &attempt.pending_state_changes) {
                (Some(expected), Some(batch)) => {
                    let actual = mutation_batch_digest(batch);
                    if &actual != expected {
                        return Err(ProductionEvidenceError::InvalidConfig(
                            "production postprocess proof batch_digest mismatch".into(),
                        ));
                    }
                    // MutationBatch must also target the evidence loop's current turn_index.
                    let turn_bound = batch.mutations.iter().any(|m| match m {
                        storyforge_domain::turn::Mutation::UpsertSummary(s) => {
                            s.turn == expected_turn_index
                        }
                        _ => false,
                    }) || written.summary_text.is_none();
                    if written.summary_text.is_some() && !turn_bound {
                        return Err(ProductionEvidenceError::InvalidConfig(
                            "production postprocess proof MutationBatch turn_index mismatch".into(),
                        ));
                    }
                }
                (None, None) => {}
                _ => {
                    return Err(ProductionEvidenceError::InvalidConfig(
                        "production postprocess proof batch presence mismatch".into(),
                    ));
                }
            }
            Ok(true)
        }
        (Some(ChronicleCandidateSource::SyntheticChronicleFixture), Some(_)) => {
            Err(ProductionEvidenceError::InvalidConfig(
                "synthetic_chronicle_fixture cannot carry production postprocess proof".into(),
            ))
        }
        (None, Some(_)) => Err(ProductionEvidenceError::InvalidConfig(
            "postprocess proof present without chronicle_source".into(),
        )),
        _ => Ok(false),
    }
}

#[derive(Debug, Clone)]
pub struct ProductionEvidenceReport {
    pub turns_requested: u32,
    pub turns_accepted: u32,
    pub calls_used: u32,
    pub max_calls: u32,
    pub crossed_h_plus_e: bool,
    pub epoch_rolled_over: bool,
    pub observed_epoch_ids16: Vec<String>,
    pub calls_path: PathBuf,
    pub turns_path: PathBuf,
    pub elapsed_ms: u128,
    pub write_path: String,
    pub chronicle_path: String,
    pub production_postprocess_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionEvidenceStage {
    AfterWriteBeforeCallEvidence,
    AfterCallEvidence,
    AfterAccept,
    AfterTurnEvidence,
    BeforeFinalFill,
}

#[async_trait]
pub trait ProductionEvidenceStageHook: Send + Sync {
    async fn on_stage(&self, _stage: ProductionEvidenceStage, _turn_index: u32) {}
}

struct NoopStageHook;

#[async_trait]
impl ProductionEvidenceStageHook for NoopStageHook {}

#[derive(Debug)]
pub enum ProductionEvidenceError {
    InvalidConfig(String),
    Store(String),
    Writer(String),
    Evidence(std::io::Error),
    ZeroCalls { turn_index: u32 },
    Accept(String),
    SuiteTimeout,
    EpochNotCrossed(String),
}

impl fmt::Display for ProductionEvidenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(msg) => write!(f, "invalid production evidence config: {msg}"),
            Self::Store(msg) => write!(f, "production evidence store error: {msg}"),
            Self::Writer(msg) => write!(f, "production evidence writer error: {msg}"),
            Self::Evidence(err) => write!(f, "production evidence I/O error: {err}"),
            Self::ZeroCalls { turn_index } => {
                write!(
                    f,
                    "turn {turn_index} produced zero LLM calls; refusing PASS"
                )
            }
            Self::Accept(msg) => write!(f, "production-faithful Accept probe failed: {msg}"),
            Self::SuiteTimeout => write!(f, "production evidence suite timeout exhausted"),
            Self::EpochNotCrossed(msg) => {
                write!(f, "production ContextEpoch did not roll over: {msg}")
            }
        }
    }
}

impl std::error::Error for ProductionEvidenceError {}

impl From<std::io::Error> for ProductionEvidenceError {
    fn from(value: std::io::Error) -> Self {
        Self::Evidence(value)
    }
}

/// fixture 预检。真实 suite 必须在构造 LLM 调用前执行，缺失即失败。
pub fn require_fixture_file(path: &Path) -> Result<(), ProductionEvidenceError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(ProductionEvidenceError::InvalidConfig(format!(
            "required fixture missing: {}",
            path.display()
        )))
    }
}

/// 真实 eval 不再猜测仓库根 fixture；必须显式提供环境变量路径。
pub fn require_explicit_fixture_path(
    path: Option<PathBuf>,
) -> Result<PathBuf, ProductionEvidenceError> {
    let path = path.ok_or_else(|| {
        ProductionEvidenceError::InvalidConfig(
            "STORYFORGE_EVAL_FIXTURE_CARD is required for real eval".into(),
        )
    })?;
    require_fixture_file(&path)?;
    Ok(path)
}

pub async fn run_production_evidence_loop<W: ProductionTurnWriter>(
    env: &HarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    campaign_id: Id,
    conversation_id: Id,
    cfg: &ProductionEvidenceConfig,
    writer: &mut W,
) -> Result<ProductionEvidenceReport, ProductionEvidenceError> {
    run_production_evidence_loop_with_hook(
        env,
        llm,
        campaign_id,
        conversation_id,
        cfg,
        writer,
        &NoopStageHook,
    )
    .await
}

struct SuiteDeadline {
    started: TokioInstant,
    duration: Duration,
}

impl SuiteDeadline {
    fn check(&self) -> Result<(), ProductionEvidenceError> {
        if self.started.elapsed() >= self.duration {
            Err(ProductionEvidenceError::SuiteTimeout)
        } else {
            Ok(())
        }
    }

    fn remaining(&self) -> Result<Duration, ProductionEvidenceError> {
        self.duration
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(ProductionEvidenceError::SuiteTimeout)
    }

    async fn run_hook<H: ProductionEvidenceStageHook + ?Sized>(
        &self,
        hook: &H,
        stage: ProductionEvidenceStage,
        turn_index: u32,
    ) -> Result<(), ProductionEvidenceError> {
        tokio::time::timeout(self.remaining()?, hook.on_stage(stage, turn_index))
            .await
            .map_err(|_| ProductionEvidenceError::SuiteTimeout)?;
        self.check()
    }
}

fn flush_new_samples(
    llm: &BudgetedLlmClient,
    call_writer: &EvidenceWriter,
    sample_cursor: &mut usize,
    cfg: &ProductionEvidenceConfig,
    turn_index: u32,
) -> Result<usize, ProductionEvidenceError> {
    let samples = llm.samples();
    let turn_samples = samples.get(*sample_cursor..).unwrap_or(&[]);
    for sample in turn_samples {
        call_writer.write_call(sample.to_evidence_call(
            &cfg.run_id,
            &cfg.suite,
            turn_index,
            &cfg.model_label,
            vec![AssertionResult {
                name: "call_recorded".into(),
                passed: sample.outcome == "ok",
                detail: Some(format!("outcome={}", sample.outcome)),
            }],
        ))?;
    }
    let written = turn_samples.len();
    *sample_cursor = samples.len();
    Ok(written)
}

pub async fn run_production_evidence_loop_with_hook<
    W: ProductionTurnWriter,
    H: ProductionEvidenceStageHook + ?Sized,
>(
    env: &HarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    campaign_id: Id,
    conversation_id: Id,
    cfg: &ProductionEvidenceConfig,
    writer: &mut W,
    hook: &H,
) -> Result<ProductionEvidenceReport, ProductionEvidenceError> {
    let params = ContextWindowParams::default();
    let max_near_raw = params.max_near_raw_turns();
    if cfg.turns <= max_near_raw {
        return Err(ProductionEvidenceError::InvalidConfig(format!(
            "turns={} must be > H_anchor+E={max_near_raw}",
            cfg.turns
        )));
    }
    if cfg.turns > llm.max_turns() {
        return Err(ProductionEvidenceError::InvalidConfig(format!(
            "turns={} exceeds STORYFORGE_EVAL_MAX_TURNS={}",
            cfg.turns,
            llm.max_turns()
        )));
    }
    if llm.max_calls() == 0 {
        return Err(ProductionEvidenceError::InvalidConfig(
            "max_calls must be >= 1".into(),
        ));
    }
    let campaign = env
        .campaign_store
        .get_campaign(&campaign_id)
        .ok_or_else(|| ProductionEvidenceError::Store("campaign missing".into()))?;
    if campaign.conversation_id.as_ref() != Some(&conversation_id) {
        return Err(ProductionEvidenceError::InvalidConfig(
            "campaign must be bound to the supplied conversation".into(),
        ));
    }

    // setup/extract 可能已使用同一个 BudgetedLlmClient。预算仍是 suite-wide，
    // 但 runner 的 turn evidence 只能从进入 runner 时的新增 sample 开始。
    let calls_before = llm.calls_used();
    let mut sample_cursor = llm.samples().len();
    let remaining_calls = llm.max_calls().saturating_sub(calls_before);
    let default_deadline = Duration::from_secs(
        llm.timeout_secs()
            .saturating_mul(u64::from(remaining_calls)),
    );
    let deadline = SuiteDeadline {
        started: TokioInstant::now(),
        duration: cfg.hard_deadline.unwrap_or(default_deadline),
    };
    deadline.check()?;

    let call_writer = EvidenceWriter::create(&cfg.calls_path, &cfg.run_id)?;
    deadline.check()?;
    let turn_writer = EvidenceWriter::create(&cfg.turns_path, &cfg.run_id)?;
    deadline.check()?;
    let probe = CommitProbeEnv::from_shared(
        env.data_dir.clone(),
        env.campaign_store.clone(),
        env.turn_store.clone(),
        env.conv_store.clone(),
    );

    let mut turns_accepted = 0u32;
    let mut production_service_turns = 0u32;
    let mut synthetic_turns = 0u32;
    let mut observed_epochs = Vec::new();
    let mut observed_epoch_set = BTreeSet::new();
    let write_path = writer.write_path().to_string();
    let mut observed_chronicle_path = "none".to_string();

    for turn_index in 1..=cfg.turns {
        let turn_started = Instant::now();
        deadline.check()?;
        let intent =
            format!("第{turn_index}轮：继续当前场景，推进角色可回应的行动，并保持前文连续。");
        let input_node_id = env
            .conv_store
            .append_user_message(&conversation_id, intent.clone())
            .map_err(|e| ProductionEvidenceError::Store(e.to_string()))?;
        deadline.check()?;
        let base_ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = env.fill_campaign_context(base_ctx);
        deadline.check()?;
        let epoch = ctx.context_epoch.clone().ok_or_else(|| {
            ProductionEvidenceError::EpochNotCrossed(format!(
                "turn {turn_index} context has no epoch"
            ))
        })?;
        let epoch_id16 = short_hash16(&epoch.epoch_id);
        if observed_epoch_set.insert(epoch_id16.clone()) {
            observed_epochs.push(epoch_id16.clone());
        }

        llm.set_tag(format!("turn{turn_index}"));
        llm.set_role("pipeline");
        let written_result = match tokio::time::timeout(
            deadline.remaining()?,
            writer.write_turn(env, &ctx, turn_index, &intent),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                // writer future 取消前可能已完成一个或多个 LLM 调用；尽最大可能脱敏落盘。
                flush_new_samples(&llm, &call_writer, &mut sample_cursor, cfg, turn_index)?;
                return Err(ProductionEvidenceError::SuiteTimeout);
            }
        };

        if let Err(err) = deadline
            .run_hook(
                hook,
                ProductionEvidenceStage::AfterWriteBeforeCallEvidence,
                turn_index,
            )
            .await
        {
            flush_new_samples(&llm, &call_writer, &mut sample_cursor, cfg, turn_index)?;
            return Err(err);
        }
        let calls_this_turn =
            flush_new_samples(&llm, &call_writer, &mut sample_cursor, cfg, turn_index)?;
        deadline.check()?;
        deadline
            .run_hook(hook, ProductionEvidenceStage::AfterCallEvidence, turn_index)
            .await?;

        let written = written_result.map_err(ProductionEvidenceError::Writer)?;
        if calls_this_turn == 0 {
            return Err(ProductionEvidenceError::ZeroCalls { turn_index });
        }

        let chronicle_path = match (&written.summary_text, written.chronicle_source) {
            (Some(_), Some(source)) => source.evidence_label(),
            (None, None) => "none",
            _ => {
                return Err(ProductionEvidenceError::InvalidConfig(
                    "summary_text and chronicle_source must either both be present or both absent"
                        .into(),
                ));
            }
        };
        if chronicle_path != "none" {
            observed_chronicle_path = chronicle_path.into();
        }
        // Fail-closed verification of production postprocess claims.
        // Bind to this loop iteration's input_node_id + turn_index so a valid
        // older-turn proof cannot certify the current turn.
        let production_postprocess_complete = verify_production_postprocess_claim(
            env,
            &campaign_id,
            &conversation_id,
            &input_node_id,
            turn_index,
            &written,
        )?;
        if production_postprocess_complete {
            production_service_turns += 1;
        } else if matches!(
            written.chronicle_source,
            Some(ChronicleCandidateSource::SyntheticChronicleFixture)
        ) {
            synthetic_turns += 1;
        }

        deadline.check()?;
        let accept_input = ProductionAcceptInput {
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            variant_id: written.variant_id.clone(),
            draft_text: written.draft_text.clone(),
            summary_text: written.summary_text.clone(),
            turn_number: turn_index,
            quality_report: Some(storyforge_domain::turn::QualityReport { warnings: vec![] }),
            force_accept: false,
        };
        // Only skip prepare when verified production proof exists.
        if !production_postprocess_complete {
            probe.prepare_awaiting_accept_with_input_node(&accept_input, input_node_id);
        }
        deadline.check()?;
        let accept = probe.accept_production(&accept_input);
        deadline.check()?;
        deadline
            .run_hook(hook, ProductionEvidenceStage::AfterAccept, turn_index)
            .await?;
        let elapsed_ms = turn_started.elapsed().as_millis();
        turn_writer.write_turn(EvidenceTurnRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: cfg.run_id.clone(),
            suite: cfg.suite.clone(),
            turn_index,
            kind: if chronicle_path == "synthetic_chronicle_fixture" {
                "pipeline_write_synthetic_chronicle_accept".into()
            } else if chronicle_path == "production_postprocess_service" {
                "pipeline_write_production_postprocess_accept".into()
            } else {
                "pipeline_write_no_chronicle_accept".into()
            },
            write_path: write_path.clone(),
            chronicle_path: chronicle_path.into(),
            accept_path: "production_faithful_commit_probe".into(),
            production_postprocess_complete,
            draft_accepted: accept.ok,
            force_accept: false,
            quality_error_count: 0,
            quality_warning_count: 0,
            autofix_attempts: 0,
            campaign_revision_before: accept.campaign_revision_before,
            campaign_revision_after: accept.campaign_revision_after,
            chronicle_revision_before: accept.chronicle_revision_before,
            chronicle_revision_after: accept.chronicle_revision_after,
            summary_code: accept.summary_code.clone(),
            attempt_status: format!("{:?}", accept.attempt_status),
            turn_status: format!("{:?}", accept.turn_status),
            draft_hash16: short_hash16(&accept.draft_hash),
            text_len: written.draft_text.chars().count(),
            text_sha16: short_hash16(&written.draft_text),
            early_fact_reachable: None,
            context_epoch_id16: Some(epoch_id16),
            context_epoch_source_hash16: Some(short_hash16(&epoch.source_hash)),
            context_epoch_anchor_count: Some(epoch.raw_anchor_turn_ids.len()),
            assertion_results: accept.assertions.clone(),
            elapsed_ms,
            recorded_at_unix_ms: 0,
        })?;
        deadline.check()?;
        deadline
            .run_hook(hook, ProductionEvidenceStage::AfterTurnEvidence, turn_index)
            .await?;
        if !accept.ok {
            return Err(ProductionEvidenceError::Accept(
                accept
                    .error
                    .unwrap_or_else(|| "unknown accept failure".into()),
            ));
        }
        turns_accepted += 1;
    }

    deadline
        .run_hook(hook, ProductionEvidenceStage::BeforeFinalFill, cfg.turns)
        .await?;
    deadline.check()?;
    // 最后一轮 Accept 后再走一次生产编译入口，观察边界上的 rollover。
    let final_ctx =
        env.fill_campaign_context(WritingContext::legacy(vec![], None, conversation_id));
    deadline.check()?;
    let final_epoch = final_ctx.context_epoch.ok_or_else(|| {
        ProductionEvidenceError::EpochNotCrossed("final context has no epoch".into())
    })?;
    let final_epoch16 = short_hash16(&final_epoch.epoch_id);
    if observed_epoch_set.insert(final_epoch16.clone()) {
        observed_epochs.push(final_epoch16);
    }

    let crossed_h_plus_e = turns_accepted > max_near_raw;
    let epoch_rolled_over = observed_epochs.len() >= 2;
    if !crossed_h_plus_e || !epoch_rolled_over {
        return Err(ProductionEvidenceError::EpochNotCrossed(format!(
            "accepted={turns_accepted}, H+E={max_near_raw}, unique_epoch_ids={}",
            observed_epochs.len()
        )));
    }
    let calls_used = llm.calls_used().saturating_sub(calls_before);
    if calls_used == 0 {
        return Err(ProductionEvidenceError::ZeroCalls { turn_index: 0 });
    }

    Ok(ProductionEvidenceReport {
        turns_requested: cfg.turns,
        turns_accepted,
        calls_used,
        max_calls: llm.max_calls(),
        crossed_h_plus_e,
        epoch_rolled_over,
        observed_epoch_ids16: observed_epochs,
        calls_path: cfg.calls_path.clone(),
        turns_path: cfg.turns_path.clone(),
        elapsed_ms: deadline.started.elapsed().as_millis(),
        write_path,
        chronicle_path: observed_chronicle_path,
        // Overall complete only when every accepted turn was verified production service.
        production_postprocess_complete: turns_accepted > 0
            && production_service_turns == turns_accepted
            && synthetic_turns == 0,
    })
}

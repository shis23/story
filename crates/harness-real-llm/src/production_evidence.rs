//! M5 生产证据多轮 runner。
//!
//! 该模块只负责编排：每轮 user input → 生产 Context 装配 → 写作 →
//! `CommitProbeEnv` 的生产忠实 CommitTurn/Accept。模型调用仍全部经过同一个
//! [`BudgetedLlmClient`]，证据沿用 `eval-m5-phaseb-v1` call/turn JSONL。

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::chronicle::ContextWindowParams;

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
}

#[derive(Debug, Clone)]
pub struct WrittenProductionTurn {
    pub draft_text: String,
    pub variant_id: Id,
    pub summary_text: Option<String>,
}

#[async_trait]
pub trait ProductionTurnWriter: Send {
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
        // Summarizer/postprocess 的 Tauri 后台任务不对 harness 暴露；Accept 探针仍使用
        // 生产 MutationBatch/Chronicle A 路径。摘要只保留轮号与正文指纹，不写证据正文。
        let summary_text = Some(format!(
            "第{turn_index}轮已接受；正文指纹 {}。",
            short_hash16(&draft_text)
        ));
        Ok(WrittenProductionTurn {
            draft_text,
            variant_id,
            summary_text,
        })
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
}

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
            Self::Accept(msg) => write!(f, "production CommitTurn/Accept failed: {msg}"),
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

pub async fn run_production_evidence_loop<W: ProductionTurnWriter>(
    env: &HarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    campaign_id: Id,
    conversation_id: Id,
    cfg: &ProductionEvidenceConfig,
    writer: &mut W,
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

    let call_writer = EvidenceWriter::create(&cfg.calls_path, &cfg.run_id)?;
    let turn_writer = EvidenceWriter::create(&cfg.turns_path, &cfg.run_id)?;
    let probe = CommitProbeEnv::from_shared(
        env.data_dir.clone(),
        env.campaign_store.clone(),
        env.turn_store.clone(),
        env.conv_store.clone(),
    );

    let started = Instant::now();
    let suite_timeout_secs = llm
        .timeout_secs()
        .saturating_mul(u64::from(llm.max_calls()))
        .max(llm.timeout_secs());
    let suite_timeout = Duration::from_secs(suite_timeout_secs);
    let mut turns_accepted = 0u32;
    let mut sample_cursor = 0usize;
    let mut observed_epochs = Vec::new();
    let mut observed_epoch_set = BTreeSet::new();

    for turn_index in 1..=cfg.turns {
        let turn_started = Instant::now();
        if started.elapsed() >= suite_timeout {
            return Err(ProductionEvidenceError::SuiteTimeout);
        }
        let intent =
            format!("第{turn_index}轮：继续当前场景，推进角色可回应的行动，并保持前文连续。");
        let input_node_id = env
            .conv_store
            .append_user_message(&conversation_id, intent.clone())
            .map_err(|e| ProductionEvidenceError::Store(e.to_string()))?;
        let base_ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = env.fill_campaign_context(base_ctx);
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
        let remaining = suite_timeout.saturating_sub(started.elapsed());
        let written_result =
            tokio::time::timeout(remaining, writer.write_turn(env, &ctx, turn_index, &intent))
                .await
                .map_err(|_| ProductionEvidenceError::SuiteTimeout)?;

        let samples = llm.samples();
        let turn_samples = samples.get(sample_cursor..).unwrap_or(&[]);
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
        sample_cursor = samples.len();

        let written = written_result.map_err(ProductionEvidenceError::Writer)?;
        if turn_samples.is_empty() {
            return Err(ProductionEvidenceError::ZeroCalls { turn_index });
        }

        let accept_input = ProductionAcceptInput {
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            variant_id: written.variant_id.clone(),
            draft_text: written.draft_text.clone(),
            summary_text: written.summary_text,
            turn_number: turn_index,
            quality_report: Some(storyforge_domain::turn::QualityReport { warnings: vec![] }),
            force_accept: false,
        };
        probe.prepare_awaiting_accept_with_input_node(&accept_input, input_node_id);
        let accept = probe.accept_production(&accept_input);
        let elapsed_ms = turn_started.elapsed().as_millis();
        turn_writer.write_turn(EvidenceTurnRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: cfg.run_id.clone(),
            suite: cfg.suite.clone(),
            turn_index,
            kind: "production_write_commit_accept".into(),
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
        if !accept.ok {
            return Err(ProductionEvidenceError::Accept(
                accept
                    .error
                    .unwrap_or_else(|| "unknown accept failure".into()),
            ));
        }
        turns_accepted += 1;
    }

    // 最后一轮 Accept 后再走一次生产编译入口，观察边界上的 rollover。
    let final_ctx =
        env.fill_campaign_context(WritingContext::legacy(vec![], None, conversation_id));
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
    if llm.calls_used() == 0 {
        return Err(ProductionEvidenceError::ZeroCalls { turn_index: 0 });
    }

    Ok(ProductionEvidenceReport {
        turns_requested: cfg.turns,
        turns_accepted,
        calls_used: llm.calls_used(),
        max_calls: llm.max_calls(),
        crossed_h_plus_e,
        epoch_rolled_over,
        observed_epoch_ids16: observed_epochs,
        calls_path: cfg.calls_path.clone(),
        turns_path: cfg.turns_path.clone(),
        elapsed_ms: started.elapsed().as_millis(),
    })
}

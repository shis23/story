//! SQLite-backed M5 endurance real-model entry (default `#[ignore]`).
//!
//! Requires:
//! - STORYFORGE_EVAL_REAL_LLM=1
//! - LLM_BASE_URL / LLM_API_KEY / LLM_MODEL
//! - LLM_EXTRA_JSON for provider-specific thinking controls when comparing reasoning arms
//! - Prefer STORYFORGE_EVAL_EVIDENCE_ROOT (gitignored durable dir)
//!
//! Stages: STORYFORGE_EVAL_ENDURANCE_STAGE=canary|coverage|stability|full
//!
//! This binary path always uses SQLite authority (`SqliteHarnessEnv`). It does
//! not dual-write JSON and never claims GUI/device coverage.

use std::path::PathBuf;
use std::sync::Arc;

use harness_real_llm::budget::BudgetedLlmClient;
use harness_real_llm::coverage_ledger::{
    CoverageLedger, ObservationKey, RuntimeCoverageProfile, action_with_runtime_profile,
};
use harness_real_llm::endurance::*;
use harness_real_llm::evidence::{EvidenceWriter, RealLlmRunBudget, short_hash16};
use harness_real_llm::resolve_llm_connection;
use harness_real_llm::sqlite_endurance::{
    SqliteHarnessEnv, fixture_source_hash16, fixture_turn_spec, is_retryable_quality_blocked_error,
};
use sha2::{Digest, Sha256};
use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::conversation::{Conversation, Role, VariantStatus};
use storyforge_domain::llm::{LlmProtocol, ReasoningMode, ToolMode};
use storyforge_domain::turn::{AttemptStatus, TurnRecord, TurnStatus};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_llm::mock_client::MockLlmClient;
use storyforge_infra_sqlite::preaccept::DraftAttemptRequest;

fn parse_target_stage() -> EnduranceStage {
    let raw = std::env::var("STORYFORGE_EVAL_ENDURANCE_STAGE").unwrap_or_default();
    EnduranceStage::from_label(&raw).unwrap_or(EnduranceStage::Canary)
}

fn scheduled_action_for_run(
    schedule: &EnduranceSchedule,
    turn: u32,
    supplemental_matrix: bool,
) -> ScheduledAction {
    if matches!(parse_target_stage(), EnduranceStage::LongCoverage) {
        let spec = fixture_turn_spec(turn)
            .unwrap_or_else(|error| panic!("fixture schedule unavailable at turn {turn}: {error}"));
        let world_info_route = match spec.world_info_route.to_ascii_lowercase().as_str() {
            "selective" => WorldInfoRouteSlot::Selective,
            "both" => WorldInfoRouteSlot::Both,
            "disabled" => WorldInfoRouteSlot::Constant,
            _ => WorldInfoRouteSlot::Constant,
        };
        let write = || ScheduledAction::Write {
            subagent_count: spec.subagent_count.max(1),
            reasoning_mode: ReasoningModeSlot::Disabled,
            tool_mode: ToolModeSlot::Native,
            world_info_route,
        };
        return match spec.action.as_str() {
            "regenerate_overall" => ScheduledAction::RegenerateOverall,
            "regenerate_editor" => ScheduledAction::RegenerateEditor,
            "regenerate_subagent" => ScheduledAction::RegenerateSubagent,
            "private_probe_owner_recall" => ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::OwnerRecall,
            },
            "private_probe_non_owner" => ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::NonOwnerLeak,
            },
            "private_probe_narration" => ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::NarrationLeak,
            },
            "private_probe_must_not_reveal" => ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::MustNotReveal,
            },
            "early_fact_inject" => ScheduledAction::EarlyFactInject {
                probe_id: spec
                    .summary_probe
                    .clone()
                    .unwrap_or_else(|| format!("fixture-early-fact-{turn}")),
            },
            "early_fact_check" => ScheduledAction::EarlyFactCheck {
                probe_id: spec
                    .must_retrieve
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("fixture-early-fact-{turn}")),
            },
            "quality_autofix_warning_only"
            | "quality_autofix_error"
            | "quality_autofix_warning_ngram"
            | "quality_autofix_warning_short"
            | "quality_autofix_private_leak" => ScheduledAction::QualityAutofix { fixable: true },
            "cache_stable" => ScheduledAction::CacheStable,
            "cache_invalidate" | "context_epoch_rollover" => ScheduledAction::CacheInvalidate,
            _ => write(),
        };
    }
    if !supplemental_matrix {
        return schedule.action_for_turn(turn);
    }
    match turn {
        1 => ScheduledAction::Write {
            subagent_count: 1,
            reasoning_mode: ReasoningModeSlot::Disabled,
            tool_mode: ToolModeSlot::Native,
            world_info_route: WorldInfoRouteSlot::Constant,
        },
        2 => ScheduledAction::Write {
            subagent_count: 2,
            reasoning_mode: ReasoningModeSlot::Disabled,
            tool_mode: ToolModeSlot::Native,
            world_info_route: WorldInfoRouteSlot::Selective,
        },
        3 => ScheduledAction::EarlyFactInject {
            probe_id: "EF-SUPPLEMENT-ALPHA".into(),
        },
        4 => ScheduledAction::Write {
            subagent_count: 3,
            reasoning_mode: ReasoningModeSlot::Disabled,
            tool_mode: ToolModeSlot::Native,
            world_info_route: WorldInfoRouteSlot::Both,
        },
        5 => ScheduledAction::RegenerateOverall,
        6 => ScheduledAction::RegenerateEditor,
        7 => ScheduledAction::RegenerateSubagent,
        8 => ScheduledAction::PrivateProbe {
            probe_kind: PrivateProbeKind::OwnerRecall,
        },
        9 => ScheduledAction::PrivateProbe {
            probe_kind: PrivateProbeKind::NonOwnerLeak,
        },
        10 => ScheduledAction::CacheInvalidate,
        11 => ScheduledAction::EarlyFactCheck {
            probe_id: "EF-SUPPLEMENT-ALPHA".into(),
        },
        12 => ScheduledAction::QualityAutofix { fixable: true },
        _ => schedule.action_for_turn(turn),
    }
}

fn parse_eval_reasoning_mode() -> ReasoningMode {
    match std::env::var("STORYFORGE_EVAL_REASONING_MODE")
        .unwrap_or_else(|_| "disabled".into())
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "disabled" | "off" | "none" => ReasoningMode::Disabled,
        "native" | "thinking" => ReasoningMode::Native,
        "prompted" | "cot" => ReasoningMode::Prompted,
        other => panic!(
            "STORYFORGE_EVAL_REASONING_MODE must be disabled, native, or prompted; got {other}"
        ),
    }
}

fn endpoint_identity_hash16(base_url: &str, protocol: &LlmProtocol) -> String {
    let url = base_url.trim().trim_end_matches('/');
    let normalized = if url.ends_with("/v1/chat/completions") {
        url.to_string()
    } else if url.ends_with("/v1") {
        format!("{url}/chat/completions")
    } else {
        format!("{url}/v1/chat/completions")
    };
    let protocol = serde_json::to_string(protocol).unwrap_or_else(|_| "invalid-protocol".into());
    short_hash16(&format!("{normalized}|{protocol}"))
}

fn model_identity_sha256(model: &str) -> String {
    format!("{:x}", Sha256::digest(model.as_bytes()))
}

fn provider_extra_hash16(extra: Option<&serde_json::Map<String, serde_json::Value>>) -> String {
    let canonical = serde_json::to_string(&extra).expect("provider extra must serialize");
    short_hash16(&canonical)
}

fn eval_pipeline_sampling(
    base: &storyforge_domain::llm::SamplingParams,
    reasoning: ReasoningMode,
) -> storyforge_domain::llm::SamplingParams {
    let mut effective = base.clone();
    effective.reasoning = reasoning;
    effective
}

#[test]
fn eval_pipeline_sampling_preserves_provider_thinking_extra() {
    let base = storyforge_domain::llm::SamplingParams {
        extra: Some(serde_json::Map::from_iter([(
            "thinking".into(),
            serde_json::json!({"type": "disabled"}),
        )])),
        ..storyforge_domain::llm::SamplingParams::default()
    };

    let effective = eval_pipeline_sampling(&base, ReasoningMode::Native);

    assert_eq!(effective.reasoning, ReasoningMode::Native);
    assert_eq!(effective.extra.unwrap()["thinking"]["type"], "disabled");
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitProvenance {
    commit: String,
    branch: String,
}

fn resolve_git_provenance() -> Result<GitProvenance, String> {
    fn git(args: &[&str]) -> Result<String, String> {
        let output = std::process::Command::new("git")
            .args(args)
            .output()
            .map_err(|error| format!("git invocation failed: {error}"))?;
        if !output.status.success() {
            return Err(format!("git command failed: {}", args.join(" ")));
        }
        String::from_utf8(output.stdout)
            .map_err(|error| format!("git output was not UTF-8: {error}"))
            .map(|value| value.trim().to_string())
    }

    let commit = git(&["rev-parse", "HEAD"])?;
    if commit.is_empty() {
        return Err("git HEAD is empty".into());
    }
    let branch = git(&["branch", "--show-current"])?;
    let status = git(&["status", "--porcelain", "--untracked-files=normal"])?;
    if !status.is_empty() {
        return Err("real evidence requires a clean git worktree".into());
    }
    Ok(GitProvenance {
        commit,
        branch: if branch.is_empty() {
            "detached".into()
        } else {
            branch
        },
    })
}

fn require_endurance_budget() -> RealLlmRunBudget {
    let budget = RealLlmRunBudget::from_env();
    if !budget.enabled {
        panic!(
            "STORYFORGE_EVAL_REAL_LLM not enabled; refusing real model calls. \
             Set STORYFORGE_EVAL_REAL_LLM=1 and LLM_BASE_URL/API_KEY/MODEL."
        );
    }
    if budget.max_calls == 0 {
        panic!("STORYFORGE_EVAL_MAX_CALLS must be >= 1");
    }
    budget
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn checkpoint_stop_after(turn_index: u32) -> bool {
    std::env::var("STORYFORGE_EVAL_STOP_AFTER_ACCEPTED_TURNS")
        .ok()
        .into_iter()
        .flat_map(|raw| {
            raw.split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter_map(|value| value.parse::<u32>().ok())
        .any(|turn| turn == turn_index)
}

fn safe_error_summary(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    let category = if error.contains("PlanParse") || error.contains("Plan 解析") {
        "plan_parse"
    } else if lower.contains("timeout") || error.contains("超时") {
        "timeout"
    } else if lower.contains("rate limit") || error.contains("520") {
        "provider_transient"
    } else if error.contains("LlmError") || lower.contains("client_error") {
        "llm_client"
    } else if error.contains("Storage") || lower.contains("sqlite") {
        "storage"
    } else if lower.contains("authority binding drifted")
        || lower.contains("checkpoint")
        || lower.contains("accepted-state")
    {
        "authority"
    } else {
        "pipeline"
    };
    format!(
        "category={category} bytes={} hash16={}",
        error.len(),
        short_hash16(error)
    )
}

const MAX_WRITE_ATTEMPTS: u32 = 5;
const MAX_PROBE_ATTEMPTS: u32 = 3;
// The SQLite real-model supplement is bounded by accepted turns, per-turn
// attempts, probe attempts, request timeouts, and the suite deadline. Keep the
// accounting field effectively unbounded instead of imposing an arbitrary
// 220-call ceiling.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupplementalProbe {
    CharacterExtractor,
    Meta,
    Cache,
}

impl SupplementalProbe {
    fn label(self) -> &'static str {
        match self {
            Self::CharacterExtractor => "character_extractor",
            Self::Meta => "meta",
            Self::Cache => "cache",
        }
    }

    fn attempts(self, state: &EnduranceProbeState) -> u32 {
        match self {
            Self::CharacterExtractor => state.character_extractor_attempts,
            Self::Meta => state.meta_attempts,
            Self::Cache => state.cache_attempts,
        }
    }

    fn completed(self, state: &EnduranceProbeState) -> bool {
        match self {
            Self::CharacterExtractor => state.character_extractor_completed,
            Self::Meta => state.meta_completed,
            Self::Cache => state.cache_completed,
        }
    }

    fn begin(self, state: &mut EnduranceProbeState) -> Result<(), String> {
        if self.completed(state) {
            return Err(format!("{} probe is already completed", self.label()));
        }
        if self.attempts(state) >= MAX_PROBE_ATTEMPTS {
            return Err(format!("{} probe retry budget exhausted", self.label()));
        }
        match self {
            Self::CharacterExtractor => {
                state.character_extractor_attempts =
                    state.character_extractor_attempts.saturating_add(1);
            }
            Self::Meta => {
                state.meta_attempts = state.meta_attempts.saturating_add(1);
            }
            Self::Cache => {
                state.cache_attempts = state.cache_attempts.saturating_add(1);
            }
        }
        Ok(())
    }

    fn mark_completed(self, state: &mut EnduranceProbeState) {
        match self {
            Self::CharacterExtractor => state.character_extractor_completed = true,
            Self::Meta => state.meta_completed = true,
            Self::Cache => state.cache_completed = true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteFailureClass {
    Transient,
    QualityBlocked,
    Fatal,
}

impl WriteFailureClass {
    fn label(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::QualityBlocked => "quality_blocked",
            Self::Fatal => "fatal",
        }
    }

    fn retryable(self) -> bool {
        self != Self::Fatal
    }
}

fn write_retry_delay(attempt: u32, failure: WriteFailureClass) -> std::time::Duration {
    let seconds = match failure {
        WriteFailureClass::QualityBlocked => 2,
        WriteFailureClass::Transient => match attempt {
            0 | 1 => 5,
            2 => 15,
            3 => 30,
            _ => 60,
        },
        WriteFailureClass::Fatal => 0,
    };
    std::time::Duration::from_secs(seconds)
}

fn retry_delay_within_deadline(
    deadline: &SuiteDeadline,
    requested: std::time::Duration,
) -> Result<std::time::Duration, EnduranceError> {
    let remaining = deadline.remaining()?;
    if requested >= remaining {
        Err(EnduranceError::SuiteTimeout)
    } else {
        Ok(requested)
    }
}

fn classify_write_failure(error: &str, action: &ScheduledAction) -> WriteFailureClass {
    if error.starts_with("nonretryable_accept:") {
        return WriteFailureClass::Fatal;
    }
    // Provider/relay transient failures wrapped in PipelineError::Llm
    // ("LLM 错误: 服务端错误 (5xx): ..." / "LLM 错误: 超时" / "LLM 错误: 速率限制 (429): ...")
    // MUST be retryable. The relay's forwarded error body can incidentally
    // contain words like "storage"/"authority" (e.g. "upstream storage error"),
    // which would otherwise trip the StoryForge-internal fatal-keyword check
    // below and fail-closed a long endurance run on a single transient 5xx.
    // LlmError Display prefixes are stable (thiserror), so match them first.
    if error.contains("LLM 错误: 服务端错误 (5xx)")
        || error.contains("LLM 错误: 超时")
        || error.contains("LLM 错误: 速率限制 (429)")
        || error.contains("LLM 错误: HTTP 请求失败")
    {
        return WriteFailureClass::Transient;
    }
    let lower = error.to_ascii_lowercase();
    if lower.contains("storage")
        || lower.contains("sqlite")
        || lower.contains("authority")
        || lower.contains("scope mismatch")
        || lower.contains("accepted-state")
    {
        return WriteFailureClass::Fatal;
    }
    if is_retryable_quality_blocked_error(error) {
        // A quality-blocked Accept is still pre-commit. Give the model-facing
        // Editor autofix path the same bounded retry budget as other transient
        // generation failures; the scheduled action assertion remains strict
        // and requires the eventual accepted attempt to have zero errors.
        let _ = action;
        return WriteFailureClass::QualityBlocked;
    }

    if error.contains("PlanParse")
        || error.contains("Plan 解析")
        || error.contains("未找到有效 Plan")
        || error.contains("already has active turn")
        || error.contains("timeout")
        || error.contains("Timeout")
        || error.contains("超时")
        || error.contains("空闲超时")
        || error.contains("流式空闲")
        || error.contains("HTTP 请求失败")
        || error.contains("520")
        || error.contains("rate limit")
        || error.contains("client_error")
        || error.contains("LlmError")
        || error.contains("Internal")
        || error.contains("pipeline missing completed agent events")
        || error.contains("所有子 Agent 均失败")
        || error.contains("不在旧 Plan")
        || error.contains("部分重 roll")
        || error.contains("expected Generating")
        || error.contains("is Failed")
    {
        return WriteFailureClass::Transient;
    }
    // A pipeline-facing model failure is safe to retry here: no Accept has
    // occurred, the active turn is recovered before the next attempt, and the
    // retry budget is bounded by MAX_WRITE_ATTEMPTS. Treat only explicit
    // authority/storage/scope failures as fatal; provider/model and parser
    // failures otherwise use the existing transient path.
    WriteFailureClass::Transient
}

fn next_retry_state(
    previous: Option<&EnduranceRetryState>,
    turn_index: u32,
    failure: WriteFailureClass,
) -> Result<EnduranceRetryState, String> {
    if let Some(previous) = previous
        && previous.turn_index != turn_index
    {
        return Err("retry state belongs to a different turn".into());
    }
    let attempts_used = previous
        .map(|state| state.attempts_used)
        .unwrap_or(0)
        .saturating_add(1);
    if attempts_used > MAX_WRITE_ATTEMPTS {
        return Err("write retry budget exhausted".into());
    }
    let quality_blocked_attempts = previous
        .map(|state| state.quality_blocked_attempts)
        .unwrap_or(0)
        + u32::from(failure == WriteFailureClass::QualityBlocked);
    Ok(EnduranceRetryState {
        turn_index,
        attempts_used,
        quality_blocked_attempts,
        last_failure_kind: failure.label().into(),
        can_retry: failure.retryable() && attempts_used < MAX_WRITE_ATTEMPTS,
    })
}

fn checkpoint_after_failed_attempt(
    previous: Option<EnduranceCheckpoint>,
    turn_index: u32,
    durable_calls: u32,
    retry_state: EnduranceRetryState,
) -> Result<EnduranceCheckpoint, String> {
    let mut checkpoint = previous.ok_or_else(|| "missing base checkpoint".to_string())?;
    if checkpoint.accepted_turn_number.saturating_add(1) != turn_index {
        return Err("retry checkpoint turn is not the next unaccepted turn".into());
    }
    if retry_state.turn_index != turn_index {
        return Err("retry checkpoint state turn mismatch".into());
    }
    if durable_calls < checkpoint.calls_used {
        return Err("durable call count moved backwards".into());
    }
    checkpoint.calls_used = durable_calls;
    checkpoint.retry_state = Some(retry_state);
    checkpoint.recorded_at_unix_ms = 0;
    Ok(checkpoint)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcceptedAttemptAuditFailure {
    ZeroCalls,
    BudgetExhausted,
}

fn accepted_attempt_audit_failure(
    calls_this_attempt: usize,
    durable_calls: u32,
    max_calls: u32,
) -> Option<AcceptedAttemptAuditFailure> {
    if calls_this_attempt == 0 {
        Some(AcceptedAttemptAuditFailure::ZeroCalls)
    } else if durable_calls > max_calls {
        Some(AcceptedAttemptAuditFailure::BudgetExhausted)
    } else {
        None
    }
}

fn validate_cache_epoch_transition(
    action: &ScheduledAction,
    previous_epoch_id16: Option<&str>,
    current_epoch_id16: Option<&str>,
) -> Result<(), EnduranceError> {
    let (previous, current) = match action {
        ScheduledAction::CacheStable | ScheduledAction::CacheInvalidate => {
            let previous = previous_epoch_id16.ok_or_else(|| {
                EnduranceError::InvalidConfig(
                    "cache assertion is missing the previous context epoch".into(),
                )
            })?;
            let current = current_epoch_id16.ok_or_else(|| {
                EnduranceError::InvalidConfig(
                    "cache assertion is missing the current context epoch".into(),
                )
            })?;
            (previous, current)
        }
        _ => return Ok(()),
    };

    match action {
        ScheduledAction::CacheStable if previous != current => Err(EnduranceError::InvalidConfig(
            "cache-stable row changed the context epoch".into(),
        )),
        ScheduledAction::CacheInvalidate if previous == current => {
            Err(EnduranceError::InvalidConfig(
                "cache-invalidate row did not change the context epoch".into(),
            ))
        }
        _ => Ok(()),
    }
}

fn persist_checkpoint_and_integrity(
    paths: &EnduranceEvidencePaths,
    checkpoint: &EnduranceCheckpoint,
    label: &str,
) -> Result<(), EnduranceError> {
    if checkpoint.sqlite_authority.is_some() {
        validate_live_sqlite_resume_authority(checkpoint)?;
    }
    let mut bound = checkpoint.clone();
    // Accept may be followed by production context fill / publication that
    // advances chronicle_revision without changing accepted conversation.
    // Rebind the durable checkpoint to the live monotone chronicle before seal.
    if let Some(campaign_id) = bound.campaign_id.as_deref() {
        let campaign_id = storyforge_domain::Id::from_str(campaign_id);
        if let Some(campaign) = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
            .map_err(EnduranceError::Writer)?
        {
            if campaign.revision != bound.campaign_revision {
                return Err(EnduranceError::InvalidConfig(
                    "checkpoint campaign revision does not match live SQLite authority".into(),
                ));
            }
            if campaign.chronicle_revision < bound.chronicle_revision {
                return Err(EnduranceError::InvalidConfig(
                    "SQLite chronicle revision regressed below checkpoint".into(),
                ));
            }
            bound.chronicle_revision = campaign.chronicle_revision;
        }
    }
    bound.sqlite_authority = Some(capture_sqlite_authority_binding(&bound)?);
    write_checkpoint(&paths.checkpoint_jsonl, &bound)?;
    harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&paths.root)
        .map(|_| ())
        .map_err(|error| {
            EnduranceError::InvalidConfig(format!("{label} checkpoint integrity baseline: {error}"))
        })
}

fn validate_sqlite_authority_values(
    checkpoint: &EnduranceCheckpoint,
    campaign_revision: u64,
    chronicle_revision: u64,
    actual: &EnduranceSqliteAuthority,
) -> Result<(), String> {
    if checkpoint.campaign_revision != campaign_revision {
        return Err("SQLite campaign revision does not match checkpoint".into());
    }
    // fill_campaign_context / compressor may publish after Accept and
    // monotonically bump chronicle_revision. Never allow regression.
    if chronicle_revision < checkpoint.chronicle_revision {
        return Err("SQLite chronicle revision regressed below checkpoint".into());
    }
    if actual.committed_turns != u64::from(checkpoint.accepted_turn_number) {
        return Err("SQLite committed-turn count does not match checkpoint".into());
    }
    let recorded = checkpoint
        .sqlite_authority
        .as_ref()
        .ok_or_else(|| "checkpoint is missing SQLite authority binding".to_string())?;
    // Accepted conversation prefix + committed turns are the hard identity of
    // durable story progress. Campaign payload (context_epoch / chronicle_revision)
    // may change during fill_campaign_runtime_from_sqlite before Accept; that
    // rewrites accepted_content_sha256 without accepting a turn. Tolerate that
    // only when chronicle advanced and the conversation binding is unchanged.
    if recorded.committed_turns != actual.committed_turns
        || recorded.accepted_conversation_nodes != actual.accepted_conversation_nodes
        || recorded.accepted_conversation_sha256 != actual.accepted_conversation_sha256
    {
        return Err("SQLite accepted-state authority binding drifted from checkpoint".into());
    }
    if recorded.accepted_content_sha256 != actual.accepted_content_sha256
        && chronicle_revision <= checkpoint.chronicle_revision
    {
        return Err("SQLite accepted-state authority binding drifted from checkpoint".into());
    }
    Ok(())
}

fn capture_sqlite_authority_binding(
    checkpoint: &EnduranceCheckpoint,
) -> Result<EnduranceSqliteAuthority, EnduranceError> {
    let campaign_id = checkpoint
        .campaign_id
        .as_deref()
        .map(storyforge_domain::Id::from_str)
        .ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "SQLite checkpoint is missing its campaign identity".into(),
            )
        })?;
    let campaign = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
        .map_err(EnduranceError::Writer)?
        .ok_or_else(|| EnduranceError::Writer("checkpoint SQLite campaign is missing".into()))?;
    let conversation_id = checkpoint
        .conversation_id
        .as_deref()
        .map(storyforge_domain::Id::from_str)
        .ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "SQLite checkpoint is missing its conversation identity".into(),
            )
        })?;
    let conversation = storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
        .map_err(EnduranceError::Writer)?
        .ok_or_else(|| {
            EnduranceError::Writer("checkpoint SQLite conversation is missing".into())
        })?;
    if conversation.campaign_id.as_ref() != Some(&campaign_id) {
        return Err(EnduranceError::InvalidConfig(
            "checkpoint SQLite conversation campaign scope drifted".into(),
        ));
    }
    let audit = storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot()
        .map_err(EnduranceError::Writer)?;
    if checkpoint.campaign_revision != campaign.revision
        || u64::from(checkpoint.accepted_turn_number) != audit.committed_turns
    {
        return Err(EnduranceError::InvalidConfig(
            "checkpoint revisions/count do not match live SQLite authority".into(),
        ));
    }
    if campaign.chronicle_revision < checkpoint.chronicle_revision {
        return Err(EnduranceError::InvalidConfig(
            "SQLite chronicle revision regressed below checkpoint".into(),
        ));
    }
    Ok(EnduranceSqliteAuthority {
        accepted_content_sha256: audit.accepted_content_sha256,
        committed_turns: audit.committed_turns,
        accepted_conversation_nodes: conversation.nodes.len() as u64,
        accepted_conversation_sha256: conversation_prefix_sha256(
            &conversation,
            conversation.nodes.len(),
        )?,
    })
}

fn conversation_prefix_sha256(
    conversation: &Conversation,
    node_count: usize,
) -> Result<String, EnduranceError> {
    if node_count > conversation.nodes.len() {
        return Err(EnduranceError::InvalidConfig(
            "SQLite conversation is shorter than its accepted checkpoint prefix".into(),
        ));
    }
    let mut value = serde_json::to_value(conversation).map_err(|error| {
        EnduranceError::InvalidConfig(format!("serialize SQLite conversation prefix: {error}"))
    })?;
    let object = value.as_object_mut().ok_or_else(|| {
        EnduranceError::InvalidConfig("SQLite conversation payload is not an object".into())
    })?;
    object.remove("updated_at");
    let nodes = object
        .get_mut("nodes")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| {
            EnduranceError::InvalidConfig("SQLite conversation payload has no node array".into())
        })?;
    nodes.truncate(node_count);
    let encoded = serde_json::to_vec(&value).map_err(|error| {
        EnduranceError::InvalidConfig(format!("encode SQLite conversation prefix: {error}"))
    })?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn validate_live_sqlite_resume_preconditions(
    checkpoint: &EnduranceCheckpoint,
) -> Result<(), EnduranceError> {
    let campaign_id = checkpoint
        .campaign_id
        .as_deref()
        .map(storyforge_domain::Id::from_str)
        .ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "resume checkpoint is missing SQLite campaign identity".into(),
            )
        })?;
    let recorded = checkpoint.sqlite_authority.as_ref().ok_or_else(|| {
        EnduranceError::InvalidConfig("checkpoint is missing SQLite authority binding".into())
    })?;
    let active_turns = storyforge_tauri_app::sqlite_runtime::list_active_turns()
        .map_err(EnduranceError::Writer)?;
    if active_turns.is_empty() {
        return validate_live_sqlite_resume_authority(checkpoint);
    }
    if active_turns.len() != 1 {
        return Err(EnduranceError::InvalidConfig(
            "SQLite resume recovery requires exactly one global active Turn".into(),
        ));
    }
    let active = &active_turns[0];
    let conversation_id = checkpoint
        .conversation_id
        .as_deref()
        .map(storyforge_domain::Id::from_str)
        .ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "resume checkpoint is missing SQLite conversation identity".into(),
            )
        })?;
    if active.campaign_id != campaign_id
        || active.conversation_id != conversation_id
        || active.base_campaign_revision != checkpoint.campaign_revision
        || !active.status.is_active()
        || active.accepted_attempt_id.is_some()
    {
        return Err(EnduranceError::InvalidConfig(
            "global active Turn is outside the recoverable checkpoint scope".into(),
        ));
    }

    let actual = capture_sqlite_authority_binding(checkpoint)?;
    if recorded.committed_turns != actual.committed_turns {
        return Err(EnduranceError::InvalidConfig(
            "SQLite committed-turn authority drifted before recovery".into(),
        ));
    }
    let conversation = storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
        .map_err(EnduranceError::Writer)?
        .ok_or_else(|| EnduranceError::Writer("resume SQLite conversation is missing".into()))?;
    let accepted_nodes = usize::try_from(recorded.accepted_conversation_nodes).map_err(|_| {
        EnduranceError::InvalidConfig("accepted conversation node count exceeds usize".into())
    })?;
    if conversation.nodes.len() < accepted_nodes
        || conversation_prefix_sha256(&conversation, accepted_nodes)?
            != recorded.accepted_conversation_sha256
    {
        return Err(EnduranceError::InvalidConfig(
            "SQLite accepted conversation prefix drifted before recovery".into(),
        ));
    }

    match conversation
        .nodes
        .iter()
        .position(|node| node.id == active.input_node_id)
    {
        None if conversation.nodes.len() == accepted_nodes => Ok(()),
        Some(index) if index == accepted_nodes => {
            let expected_parent = index
                .checked_sub(1)
                .map(|parent_index| &conversation.nodes[parent_index].id);
            if conversation.nodes[index].parent_id.as_ref() != expected_parent {
                return Err(EnduranceError::InvalidConfig(
                    "active Turn input is detached from the accepted conversation prefix".into(),
                ));
            }
            let input = conversation.nodes[index].active().ok_or_else(|| {
                EnduranceError::InvalidConfig("active Turn input node has no active variant".into())
            })?;
            if input.role != Role::User || input.status != VariantStatus::Final {
                return Err(EnduranceError::InvalidConfig(
                    "active Turn input anchor is not a final User node".into(),
                ));
            }
            if conversation.nodes[index..]
                .windows(2)
                .any(|pair| pair[1].parent_id.as_ref() != Some(&pair[0].id))
            {
                return Err(EnduranceError::InvalidConfig(
                    "active Turn conversation tail is not a contiguous chain".into(),
                ));
            }
            Ok(())
        }
        _ => Err(EnduranceError::InvalidConfig(
            "active Turn input anchor would truncate accepted conversation history".into(),
        )),
    }
}

fn validate_live_sqlite_resume_authority(
    checkpoint: &EnduranceCheckpoint,
) -> Result<(), EnduranceError> {
    if !storyforge_tauri_app::sqlite_runtime::list_active_turns()
        .map_err(EnduranceError::Writer)?
        .is_empty()
    {
        return Err(EnduranceError::InvalidConfig(
            "SQLite authority cannot be finalized while an active Turn remains".into(),
        ));
    }
    let campaign_id = checkpoint
        .campaign_id
        .as_deref()
        .map(storyforge_domain::Id::from_str)
        .ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "resume checkpoint is missing SQLite campaign identity".into(),
            )
        })?;
    let campaign = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
        .map_err(EnduranceError::Writer)?
        .ok_or_else(|| EnduranceError::Writer("resume SQLite campaign is missing".into()))?;
    let actual = capture_sqlite_authority_binding(checkpoint)?;
    validate_sqlite_authority_values(
        checkpoint,
        campaign.revision,
        campaign.chronicle_revision,
        &actual,
    )
    .map_err(EnduranceError::InvalidConfig)
}

fn validate_continuous_turn_start_authority(
    checkpoint: &EnduranceCheckpoint,
    turn_index: u32,
) -> Result<(), EnduranceError> {
    if checkpoint.accepted_turn_number.saturating_add(1) != turn_index {
        return Err(EnduranceError::InvalidConfig(
            "durable checkpoint is not the immediate predecessor of this turn".into(),
        ));
    }
    if let Some(retry) = checkpoint.retry_state.as_ref()
        && retry.turn_index != turn_index
    {
        return Err(EnduranceError::InvalidConfig(
            "durable retry state belongs to another turn".into(),
        ));
    }
    validate_live_sqlite_resume_authority(checkpoint)
}

fn begin_supplemental_probe(
    paths: &EnduranceEvidencePaths,
    probe: SupplementalProbe,
) -> Result<(), EnduranceError> {
    let mut checkpoint = read_latest_checkpoint(&paths.checkpoint_jsonl).ok_or_else(|| {
        EnduranceError::InvalidConfig(format!(
            "{} probe cannot begin without a durable checkpoint",
            probe.label()
        ))
    })?;
    probe
        .begin(&mut checkpoint.probe_state)
        .map_err(EnduranceError::InvalidConfig)?;
    persist_checkpoint_and_integrity(paths, &checkpoint, probe.label())
}

fn finish_supplemental_probe(
    paths: &EnduranceEvidencePaths,
    probe: SupplementalProbe,
    completed: bool,
    model_label: &str,
) -> Result<u32, EnduranceError> {
    let mut checkpoint = read_latest_checkpoint(&paths.checkpoint_jsonl).ok_or_else(|| {
        EnduranceError::InvalidConfig(format!(
            "{} probe cannot finish without a durable checkpoint",
            probe.label()
        ))
    })?;
    let durable_calls = harness_real_llm::evidence_retention::reconcile_dangling_call_reservations(
        &paths.root,
        &checkpoint.run_id,
        model_label,
    )
    .map_err(|error| {
        EnduranceError::InvalidConfig(format!(
            "{} call ledger reconciliation rejected: {error}",
            probe.label()
        ))
    })?;
    if durable_calls < checkpoint.calls_used {
        return Err(EnduranceError::InvalidConfig(format!(
            "{} probe durable call count moved backwards",
            probe.label()
        )));
    }
    checkpoint.calls_used = durable_calls;
    if completed {
        probe.mark_completed(&mut checkpoint.probe_state);
    }
    persist_checkpoint_and_integrity(paths, &checkpoint, probe.label())?;
    Ok(durable_calls)
}

fn latest_probe_state(
    paths: &EnduranceEvidencePaths,
) -> Result<EnduranceProbeState, EnduranceError> {
    read_latest_checkpoint(&paths.checkpoint_jsonl)
        .map(|checkpoint| checkpoint.probe_state)
        .ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "latest checkpoint is unavailable while preserving probe state".into(),
            )
        })
}

fn durable_probe_metrics(
    path: &std::path::Path,
    role: &str,
) -> Result<(usize, u32), EnduranceError> {
    let rows = harness_real_llm::evidence::read_evidence_lines(path)?;
    let mut calls = 0usize;
    let mut cached_tokens = 0u32;
    for row in rows {
        if row.get("role").and_then(|value| value.as_str()) != Some(role) {
            continue;
        }
        calls = calls.saturating_add(1);
        cached_tokens = cached_tokens.saturating_add(
            row.get("cached_tokens")
                .and_then(|value| value.as_u64())
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0),
        );
    }
    Ok((calls, cached_tokens))
}

fn durable_cache_probe_metrics(path: &std::path::Path) -> Result<(usize, u32), EnduranceError> {
    let rows = harness_real_llm::evidence::read_evidence_lines(path)?;
    let mut selected = Vec::new();
    for tag in [
        "sqlite-cache-stable-a",
        "sqlite-cache-stable-b",
        "sqlite-cache-invalidated",
    ] {
        let row = rows
            .iter()
            .rev()
            .find(|row| {
                row.get("role").and_then(|value| value.as_str()) == Some("cache_probe")
                    && row.get("tag").and_then(|value| value.as_str()) == Some(tag)
                    && row.get("outcome").and_then(|value| value.as_str()) == Some("ok")
            })
            .ok_or_else(|| {
                EnduranceError::InvalidConfig(format!(
                    "completed cache probe is missing successful durable tag {tag}"
                ))
            })?;
        selected.push(row);
    }
    let stable_a = selected[0]
        .get("request_fp16")
        .and_then(|value| value.as_str());
    let stable_b = selected[1]
        .get("request_fp16")
        .and_then(|value| value.as_str());
    let invalidated = selected[2]
        .get("request_fp16")
        .and_then(|value| value.as_str());
    if stable_a.is_none() || stable_a != stable_b || stable_b == invalidated {
        return Err(EnduranceError::InvalidConfig(
            "durable cache probe fingerprints do not prove stable/stable/invalidated".into(),
        ));
    }
    let cached_tokens = selected.iter().fold(0u32, |total, row| {
        total.saturating_add(
            row.get("cached_tokens")
                .and_then(|value| value.as_u64())
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0),
        )
    });
    Ok((selected.len(), cached_tokens))
}

fn last_request_fp16_for_turn(
    path: &std::path::Path,
    turn_index: u32,
) -> Result<Option<String>, String> {
    let rows = harness_real_llm::evidence::read_evidence_lines(path)
        .map_err(|error| format!("read call evidence for resume fingerprint: {error}"))?;
    Ok(rows.iter().rev().find_map(|row| {
        let matches_turn =
            row.get("turn_index").and_then(|value| value.as_u64()) == Some(u64::from(turn_index));
        let succeeded = row.get("outcome").and_then(|value| value.as_str()) == Some("ok");
        (matches_turn && succeeded)
            .then(|| {
                row.get("request_fp16")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .flatten()
    }))
}

fn has_successful_tool_result<'a>(
    steps: impl IntoIterator<Item = &'a harness_real_llm::evidence::EvidenceToolStep>,
    tool_names: &[&str],
) -> bool {
    steps.into_iter().any(|step| {
        step.kind == "result"
            && step.ok == Some(true)
            && tool_names.contains(&step.tool_name.as_str())
    })
}

fn durable_successful_write_tool_call_steps(path: &std::path::Path) -> Result<usize, String> {
    let rows = harness_real_llm::evidence::read_evidence_lines(path)
        .map_err(|error| format!("read durable tool-call evidence: {error}"))?;
    Ok(rows
        .iter()
        .filter(|row| row.get("outcome").and_then(|value| value.as_str()) == Some("ok"))
        .filter(|row| {
            row.get("assertion_results")
                .and_then(|value| value.as_array())
                .map(|assertions| {
                    assertions.iter().any(|assertion| {
                        assertion.get("name").and_then(|value| value.as_str())
                            == Some("successful_write_attempt")
                            && assertion.get("passed").and_then(|value| value.as_bool())
                                == Some(true)
                    })
                })
                .unwrap_or(false)
        })
        .flat_map(|row| {
            row.get("tool_steps")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
        })
        .filter(|step| step.get("kind").and_then(|value| value.as_str()) == Some("call"))
        .count())
}

fn validate_resume_identity(
    checkpoint: &EnduranceCheckpoint,
    expected_stage: EnduranceStage,
    expected: &EnduranceRunIdentity,
) -> Result<(), &'static str> {
    if checkpoint.stage != expected_stage.label() {
        return Err("checkpoint stage does not match requested stage");
    }
    match checkpoint.run_identity.as_ref() {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => Err("checkpoint run identity does not match fixture/model/runtime modes"),
        None => Err("checkpoint is missing required run identity"),
    }
}

fn evidence_run_dir(stage: EnduranceStage) -> (String, PathBuf, EnduranceEvidencePaths) {
    use harness_real_llm::evidence_retention::{
        EVIDENCE_DIR_ENV, EvidenceRootPolicy, open_endurance_run_paths, resolve_evidence_root,
        resolve_explicit_resume_run_dir,
    };

    let allow_ephemeral = std::env::var("STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE")
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false);
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let policy = EvidenceRootPolicy {
        repo_root,
        allow_ephemeral,
        require_explicit: !allow_ephemeral,
    };

    if let Ok(raw) = std::env::var(EVIDENCE_DIR_ENV) {
        let raw = raw.trim();
        if !raw.is_empty() {
            let path = PathBuf::from(raw);
            if let Some(validated) = resolve_explicit_resume_run_dir(&path, &policy)
                .unwrap_or_else(|e| panic!("fail-closed resume evidence dir rejected: {e}"))
            {
                let name = validated
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                let paths = EnduranceEvidencePaths::new(validated.clone());
                return (name, validated, paths);
            }
        }
    }

    let root = resolve_evidence_root(None, None, &policy)
        .unwrap_or_else(|e| panic!("illegal or missing evidence root: {e}"));
    let (run_id, paths) = open_endurance_run_paths(&root, stage.label())
        .unwrap_or_else(|e| panic!("failed to open exclusive evidence run dir: {e}"));
    let dir = paths.root.clone();
    (run_id, dir, paths)
}

#[allow(clippy::too_many_arguments)]
fn flush_samples(
    llm: &BudgetedLlmClient,
    call_writer: &EvidenceWriter,
    tool_writer: Option<&EvidenceWriter>,
    cursor: &mut usize,
    successful_attempt: Option<bool>,
    run_id: &str,
    model_label: &str,
    turn_index: u32,
) -> Result<usize, EnduranceError> {
    let samples = llm.samples();
    let turn_samples = samples.get(*cursor..).unwrap_or(&[]);
    let mut call_records = Vec::with_capacity(turn_samples.len());
    for sample in turn_samples {
        let mut assertions = vec![harness_real_llm::evidence::AssertionResult {
            name: "call_recorded".into(),
            passed: sample.outcome == "ok",
            detail: Some(format!("outcome={}", sample.outcome)),
        }];
        if let Some(passed) = successful_attempt {
            assertions.push(harness_real_llm::evidence::AssertionResult {
                name: "successful_write_attempt".into(),
                passed,
                detail: Some(passed.to_string()),
            });
        }
        let rec = sample.to_evidence_call(
            run_id,
            "endurance_sqlite",
            turn_index,
            model_label,
            assertions,
        );
        call_writer.write_call(rec.clone())?;
        call_records.push(rec);
    }
    if let Some(tool_writer) = tool_writer
        && let Some(trace) = harness_real_llm::evidence::aggregate_tool_trace(
            run_id,
            "endurance_sqlite",
            turn_index,
            model_label,
            &call_records,
        )
    {
        tool_writer.write_tool_trace(trace)?;
    }
    let written = turn_samples.len();
    *cursor = samples.len();
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
async fn run_sqlite_meta_probe(
    env: &SqliteHarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    conversation_id: &storyforge_domain::Id,
    call_writer: &EvidenceWriter,
    tool_writer: &EvidenceWriter,
    sample_cursor: &mut usize,
    run_id: &str,
    model_label: &str,
    evidence_turn_index: u32,
) -> Result<usize, EnduranceError> {
    use storyforge_app_agent::runtime::AgentRuntime;
    use storyforge_app_meta::{MetaConversation, MetaSession, meta_chat};

    let writing = env
        .fill_campaign_context(WritingContext::legacy(
            vec![],
            None,
            conversation_id.clone(),
        ))
        .map_err(EnduranceError::Writer)?;
    let campaign_runtime = writing.campaign_runtime.ok_or_else(|| {
        EnduranceError::InvalidConfig("Meta probe has no SQLite campaign runtime".into())
    })?;
    if campaign_runtime.tasks.is_empty() {
        return Err(EnduranceError::InvalidConfig(
            "Meta inspect_tasks probe requires a seeded SQLite task".into(),
        ));
    }

    let session = Arc::new(MetaSession::new());
    session.set_campaign_runtime(campaign_runtime);
    let tool_ctx = Arc::new(
        env.tool_ctx
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
    );
    let runtime = AgentRuntime::new(llm.clone() as Arc<dyn LlmClient>, tool_ctx);
    let mut conversation = MetaConversation::new();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let (progress_tx, _progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let sample_start = llm.samples().len();
    llm.set_tag("sqlite-meta-probe");
    llm.set_role("meta");
    let probe_result = meta_chat(
        &runtime,
        &mut conversation,
        session,
        "请先实际调用 inspect_campaign 和 inspect_tasks，再用一句话报告；不要猜测。",
        cancel_rx,
        progress_tx,
    )
    .await
    .map_err(|error| EnduranceError::Writer(format!("SQLite Meta probe: {error}")));
    let written = flush_samples(
        &llm,
        call_writer,
        Some(tool_writer),
        sample_cursor,
        None,
        run_id,
        model_label,
        evidence_turn_index,
    )?;
    probe_result?;
    let samples = llm.samples();
    let probe_steps = samples
        .get(sample_start..)
        .unwrap_or_default()
        .iter()
        .flat_map(|sample| sample.tool_steps.iter())
        .collect::<Vec<_>>();
    for required in ["inspect_campaign", "inspect_tasks"] {
        if !has_successful_tool_result(probe_steps.iter().copied(), &[required]) {
            return Err(EnduranceError::InvalidConfig(format!(
                "Meta probe did not receive a successful result from required tool {required}"
            )));
        }
    }
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
async fn run_character_extractor_probe(
    env: &SqliteHarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    call_writer: &EvidenceWriter,
    tool_writer: &EvidenceWriter,
    sample_cursor: &mut usize,
    run_id: &str,
    model_label: &str,
) -> Result<usize, EnduranceError> {
    use storyforge_app_agent::runtime::AgentRuntime;

    let tool_ctx = Arc::new(
        env.tool_ctx
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
    );
    let character = tool_ctx
        .characters
        .last()
        .cloned()
        .ok_or_else(|| EnduranceError::InvalidConfig("extractor fixture missing".into()))?;
    let runtime = AgentRuntime::new(llm.clone() as Arc<dyn LlmClient>, tool_ctx);
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let sample_start = llm.samples().len();
    llm.set_tag("sqlite-character-extractor-probe");
    llm.set_role("character_extractor");
    let probe_result =
        storyforge_app_agent::extract_characters(&runtime, &character, &[], cancel_rx)
            .await
            .map_err(|error| EnduranceError::Writer(format!("CharacterExtractor probe: {error}")));
    let written = flush_samples(
        &llm,
        call_writer,
        Some(tool_writer),
        sample_cursor,
        None,
        run_id,
        model_label,
        0,
    )?;
    let definitions = probe_result?;
    if definitions.is_empty() {
        return Err(EnduranceError::InvalidConfig(
            "CharacterExtractor returned no definitions".into(),
        ));
    }
    let samples = llm.samples();
    let emitted = samples
        .get(sample_start..)
        .unwrap_or_default()
        .iter()
        .flat_map(|sample| sample.tool_steps.iter())
        .any(|step| step.kind == "call" && step.tool_name == "emit_characters");
    if !emitted {
        return Err(EnduranceError::InvalidConfig(
            "CharacterExtractor returned definitions without an emit_characters call".into(),
        ));
    }
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
async fn run_cache_fingerprint_probe(
    llm: Arc<BudgetedLlmClient>,
    call_writer: &EvidenceWriter,
    tool_writer: &EvidenceWriter,
    sample_cursor: &mut usize,
    run_id: &str,
    model_label: &str,
    evidence_turn_index: u32,
) -> Result<(usize, u32), EnduranceError> {
    use storyforge_domain::llm::{ChatMessage, ChatRequest, SamplingParams};

    let stable = ChatRequest {
        model: model_label.to_string(),
        messages: vec![
            ChatMessage::system("Synthetic cache probe; answer with OK."),
            ChatMessage::user("stable-input-v1"),
        ],
        tools: None,
        params: SamplingParams::default(),
    };
    let sample_start = llm.samples().len();
    llm.set_role("cache_probe");
    let probe_result = async {
        llm.set_tag("sqlite-cache-stable-a");
        llm.chat(&stable)
            .await
            .map_err(|error| EnduranceError::Writer(format!("cache probe A: {error}")))?;
        llm.set_tag("sqlite-cache-stable-b");
        llm.chat(&stable)
            .await
            .map_err(|error| EnduranceError::Writer(format!("cache probe B: {error}")))?;
        let mut changed = stable.clone();
        changed.messages[1] = ChatMessage::user("stable-input-v2");
        llm.set_tag("sqlite-cache-invalidated");
        llm.chat(&changed)
            .await
            .map_err(|error| EnduranceError::Writer(format!("cache probe C: {error}")))?;

        let samples = llm.samples();
        let probe = samples.get(sample_start..).unwrap_or_default();
        if probe.len() != 3
            || probe[0].request_fp16 != probe[1].request_fp16
            || probe[1].request_fp16 == probe[2].request_fp16
        {
            return Err(EnduranceError::InvalidConfig(
                "cache probe did not observe stable/stable/invalidated request fingerprints".into(),
            ));
        }
        Ok(probe.iter().map(|sample| sample.cached_tokens).sum())
    }
    .await;
    let written = flush_samples(
        &llm,
        call_writer,
        Some(tool_writer),
        sample_cursor,
        None,
        run_id,
        model_label,
        evidence_turn_index,
    )?;
    let cached_tokens = probe_result?;
    Ok((written, cached_tokens))
}

trait HardDeadlineExt {
    fn hard_deadline_override(&self, turns: u32, stage: EnduranceStage) -> std::time::Duration;
}

impl HardDeadlineExt for RealLlmRunBudget {
    fn hard_deadline_override(&self, turns: u32, stage: EnduranceStage) -> std::time::Duration {
        let per_call = self.timeout_secs.max(1);
        let remaining_calls = self.max_calls.max(1) as u64;
        let by_calls = per_call.saturating_mul(remaining_calls);
        let by_turns = per_call.saturating_mul(turns as u64).saturating_mul(3);
        let floor = match stage {
            EnduranceStage::DryRun => 60,
            EnduranceStage::Canary => 20 * 60,
            EnduranceStage::Coverage => 60 * 60,
            EnduranceStage::Stability => 2 * 60 * 60,
            EnduranceStage::LongCoverage => 24 * 60 * 60,
            EnduranceStage::Full => 4 * 60 * 60,
        };
        if matches!(stage, EnduranceStage::LongCoverage) {
            return std::time::Duration::from_secs(24 * 60 * 60);
        }
        let total = by_calls.max(by_turns).max(floor).min(6 * 60 * 60);
        std::time::Duration::from_secs(total)
    }
}

struct SqliteEnduranceStageContext<'a> {
    env: &'a SqliteHarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    campaign_id: storyforge_domain::Id,
    conversation_id: storyforge_domain::Id,
    stage: EnduranceStage,
    budget: &'a RealLlmRunBudget,
    call_limit: u32,
    paths: &'a EnduranceEvidencePaths,
    runtime_profile: RuntimeCoverageProfile,
    model_label: &'a str,
    run_identity: EnduranceRunIdentity,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CoverageLedgerEvidenceRow {
    schema_version: String,
    run_id: String,
    observation: harness_real_llm::coverage_ledger::ObservedCoverage,
}

fn parse_coverage_ledger_evidence(
    line: &str,
    expected_run_id: &str,
) -> Result<harness_real_llm::coverage_ledger::ObservedCoverage, EnduranceError> {
    let evidence: CoverageLedgerEvidenceRow = serde_json::from_str(line).map_err(|error| {
        EnduranceError::InvalidConfig(format!("coverage ledger parse: {error}"))
    })?;
    if evidence.schema_version != harness_real_llm::evidence::EVIDENCE_SCHEMA_VERSION
        || evidence.run_id != expected_run_id
    {
        return Err(EnduranceError::InvalidConfig(
            "coverage ledger identity or schema drift".into(),
        ));
    }
    Ok(evidence.observation)
}

async fn run_sqlite_endurance_stage(
    context: SqliteEnduranceStageContext<'_>,
) -> Result<EnduranceStageManifestRow, EnduranceError> {
    let SqliteEnduranceStageContext {
        env,
        llm,
        campaign_id,
        conversation_id,
        stage,
        budget,
        call_limit,
        paths,
        runtime_profile,
        model_label,
        run_identity,
    } = context;
    let target_turns = stage.target_turns();
    let meta_probe_enabled = env_flag("STORYFORGE_EVAL_META_PROBE");
    let extractor_probe_enabled = env_flag("STORYFORGE_EVAL_CHARACTER_EXTRACTOR_PROBE");
    let cache_probe_enabled = env_flag("STORYFORGE_EVAL_CACHE_PROBE");
    let supplemental_matrix = env_flag("STORYFORGE_EVAL_SUPPLEMENTAL_MATRIX");
    if run_identity.supplemental_matrix != supplemental_matrix
        || run_identity.meta_probe != meta_probe_enabled
        || run_identity.character_extractor_probe != extractor_probe_enabled
        || run_identity.cache_probe != cache_probe_enabled
    {
        return Err(EnduranceError::InvalidConfig(
            "run identity does not match requested supplemental probe matrix".into(),
        ));
    }
    if supplemental_matrix
        && !matches!(
            stage,
            EnduranceStage::Coverage | EnduranceStage::LongCoverage
        )
    {
        return Err(EnduranceError::InvalidConfig(
            "supplemental matrix requires coverage or long_coverage stage".into(),
        ));
    }
    let schedule = EnduranceSchedule::new(target_turns);
    let planned: Vec<(u32, ScheduledAction)> = (1..=target_turns)
        .map(|n| {
            (
                n,
                action_with_runtime_profile(
                    scheduled_action_for_run(&schedule, n, supplemental_matrix),
                    runtime_profile,
                ),
            )
        })
        .collect();
    let mut ledger = CoverageLedger::plan_from_schedule(planned);

    let (start_turn, resume_cp, run_id) = if paths.checkpoint_jsonl.exists() {
        let (next, cp) = resume_from_evidence_dir(&paths.root, None)?;
        let run_id = cp.run_id.clone();
        (next, Some(cp), run_id)
    } else if let Some(name) = paths
        .root
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| harness_real_llm::evidence_retention::is_controlled_run_dirname(s))
    {
        (1, None, name.to_string())
    } else {
        let run_id = harness_real_llm::evidence_retention::allocate_run_id(
            paths.root.parent().unwrap_or(paths.root.as_path()),
            stage.label(),
        )
        .unwrap_or_else(|_| format!("run-{}-{}", stage.label(), uuid::Uuid::new_v4()));
        (1, None, run_id)
    };

    if let Some(checkpoint) = resume_cp.as_ref() {
        let recorded =
            harness_real_llm::evidence_retention::count_budgeted_call_records(&paths.root, &run_id)
                .map_err(|error| {
                    EnduranceError::InvalidConfig(format!("resume call ledger rejected: {error}"))
                })?;
        if recorded != checkpoint.calls_used {
            return Err(EnduranceError::InvalidConfig(format!(
                "resume call count mismatch: checkpoint={} durable={recorded}",
                checkpoint.calls_used
            )));
        }
    }
    let mut sample_cursor = llm.samples().len();
    let deadline = SuiteDeadline::new(budget.hard_deadline_override(target_turns, stage));
    deadline.check()?;

    let is_resume = resume_cp.is_some();
    let call_writer = if is_resume {
        EvidenceWriter::open_append(&paths.calls_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.calls_jsonl, &run_id)?
    };
    let tool_writer = if is_resume {
        EvidenceWriter::open_append(&paths.tool_trace_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.tool_trace_jsonl, &run_id)?
    };
    let turn_writer = if is_resume {
        EvidenceWriter::open_append(&paths.turns_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.turns_jsonl, &run_id)?
    };
    let ledger_path = paths.coverage_ledger_jsonl.clone();
    if !is_resume && ledger_path.exists() {
        let _ = std::fs::remove_file(&ledger_path);
    }
    // Resume must reload previously sealed coverage observations; exact-set
    // verification is suite-wide, not process-local.
    if is_resume && ledger_path.exists() {
        let prior = std::fs::read_to_string(&ledger_path).map_err(EnduranceError::EvidenceIo)?;
        for line in prior.lines().filter(|l| !l.trim().is_empty()) {
            ledger.record(parse_coverage_ledger_evidence(line, &run_id)?);
        }
    }

    let mut turns_accepted = start_turn.saturating_sub(1);
    let authoritative_campaign = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
        .map_err(EnduranceError::Writer)?
        .ok_or_else(|| EnduranceError::Writer("active SQLite campaign missing".into()))?;
    let mut last_draft_hash16 = resume_cp
        .as_ref()
        .map(|checkpoint| checkpoint.last_draft_hash16.clone())
        .unwrap_or_default();
    let mut last_campaign_revision = resume_cp
        .as_ref()
        .map(|checkpoint| checkpoint.campaign_revision)
        .unwrap_or(authoritative_campaign.revision);
    let mut last_chronicle_revision = resume_cp
        .as_ref()
        .map(|checkpoint| checkpoint.chronicle_revision)
        .unwrap_or(authoritative_campaign.chronicle_revision);
    let mut epoch_tracker = EpochTracker::default();
    if let Some(cp) = resume_cp.as_ref() {
        for eid in &cp.observed_epoch_ids16 {
            epoch_tracker.observe(eid);
        }
    }

    if !is_resume {
        let initial_checkpoint = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.clone(),
            stage: stage.label().into(),
            accepted_turn_number: 0,
            calls_used: 0,
            max_calls: call_limit,
            campaign_revision: last_campaign_revision,
            chronicle_revision: last_chronicle_revision,
            last_draft_hash16: String::new(),
            last_summary_code: None,
            context_epoch_id16: None,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: Some(campaign_id.as_str().to_string()),
            conversation_id: Some(conversation_id.as_str().to_string()),
            data_dir_rel: Some("campaign_data".into()),
            observed_epoch_ids16: vec![],
            run_identity: Some(run_identity.clone()),
            retry_state: None,
            probe_state: EnduranceProbeState::default(),
            sqlite_authority: None,
            recorded_at_unix_ms: 0,
        };
        persist_checkpoint_and_integrity(paths, &initial_checkpoint, "initial")?;
    }

    if extractor_probe_enabled {
        let completed = read_latest_checkpoint(&paths.checkpoint_jsonl)
            .map(|checkpoint| {
                SupplementalProbe::CharacterExtractor.completed(&checkpoint.probe_state)
            })
            .unwrap_or(false);
        if !completed {
            llm.set_evidence_turn_index(0);
            begin_supplemental_probe(paths, SupplementalProbe::CharacterExtractor)?;
            let probe_result = run_character_extractor_probe(
                env,
                llm.clone(),
                &call_writer,
                &tool_writer,
                &mut sample_cursor,
                &run_id,
                model_label,
            )
            .await;
            let durable_calls = finish_supplemental_probe(
                paths,
                SupplementalProbe::CharacterExtractor,
                probe_result.is_ok(),
                model_label,
            )?;
            if durable_calls > call_limit {
                return Err(EnduranceError::BudgetExhausted {
                    calls_used: durable_calls,
                    max_calls: call_limit,
                });
            }
            probe_result?;
        }
    }
    let (extractor_probe_calls, _) = if extractor_probe_enabled {
        let metrics = durable_probe_metrics(&paths.calls_jsonl, "character_extractor")?;
        if metrics.0 == 0 {
            return Err(EnduranceError::InvalidConfig(
                "completed CharacterExtractor probe has no durable call evidence".into(),
            ));
        }
        metrics
    } else {
        (0, 0)
    };
    let mut last_context_epoch_id16 = resume_cp
        .as_ref()
        .and_then(|checkpoint| checkpoint.context_epoch_id16.clone());

    for turn_index in start_turn..=target_turns {
        deadline.check()?;
        let turn_start_checkpoint =
            read_latest_checkpoint(&paths.checkpoint_jsonl).ok_or_else(|| {
                EnduranceError::InvalidConfig(
                    "continuous turn start has no durable checkpoint".into(),
                )
            })?;
        validate_continuous_turn_start_authority(&turn_start_checkpoint, turn_index)?;
        let action = action_with_runtime_profile(
            scheduled_action_for_run(&schedule, turn_index, supplemental_matrix),
            runtime_profile,
        );
        let fixture_spec = if matches!(stage, EnduranceStage::LongCoverage) {
            Some(fixture_turn_spec(turn_index).map_err(EnduranceError::InvalidConfig)?)
        } else {
            None
        };
        let row = ledger
            .planned
            .iter()
            .find(|r| r.turn_index == turn_index)
            .cloned()
            .ok_or_else(|| {
                EnduranceError::InvalidConfig(format!("missing coverage row for turn {turn_index}"))
            })?;
        let intent = if let Some(spec) = fixture_spec.as_ref() {
            spec.intent.clone()
        } else {
            match &action {
                ScheduledAction::Write {
                    subagent_count,
                    world_info_route,
                    ..
                } => {
                    let route_focus = match world_info_route {
                        WorldInfoRouteSlot::Constant => "use the always-injected harbor/dock lore",
                        WorldInfoRouteSlot::Selective => {
                            "before writing, call search_world_info with keyword 徽章 and use the result"
                        }
                        WorldInfoRouteSlot::Both => {
                            "before writing, call search_world_info with keyword 账本 and use both routed lore sources"
                        }
                    };
                    format!(
                        "turn {turn_index}: advance scene with exactly {subagent_count} character tasks; focus on {route_focus}; keep continuity."
                    )
                }
                ScheduledAction::RegenerateOverall => {
                    format!(
                        "turn {turn_index}: continue the investigation from a slightly different angle while keeping continuity."
                    )
                }
                ScheduledAction::RegenerateEditor => {
                    format!(
                        "turn {turn_index}: polish the latest scene prose while preserving plot facts."
                    )
                }
                ScheduledAction::RegenerateSubagent => {
                    format!(
                        "turn {turn_index}: rework one supporting character performance without changing the overall beat."
                    )
                }
                ScheduledAction::PrivateProbe { probe_kind } => {
                    let focus = match probe_kind {
                        PrivateProbeKind::OwnerRecall => {
                            "let the owner act on private context without spelling it out"
                        }
                        PrivateProbeKind::NonOwnerLeak => {
                            "show a non-owner viewpoint that must not know owner-only facts"
                        }
                        PrivateProbeKind::NarrationLeak => {
                            "keep the narrator from exposing owner-only facts"
                        }
                        PrivateProbeKind::MustNotReveal => {
                            "resist any request to reveal protected fixture values"
                        }
                    };
                    format!(
                        "turn {turn_index}: continue the scene with careful information isolation; {focus}."
                    )
                }
                ScheduledAction::EarlyFactInject { probe_id } => {
                    // Keep probe id out of user intent (evidence/schedule only). Model sees a
                    // normal continuity beat; harness records probe_id in checkpoint metadata.
                    let _ = probe_id;
                    format!(
                        "turn {turn_index}: advance the investigation with a concrete scene detail the party can recall later."
                    )
                }
                ScheduledAction::EarlyFactCheck { probe_id } => {
                    let _ = probe_id;
                    format!(
                        "turn {turn_index}: before writing, call get_recent_summary or search_chronicle, then continue the scene using an earlier investigation detail for continuity."
                    )
                }
                ScheduledAction::QualityAutofix { fixable } => {
                    format!("turn {turn_index}: quality autofix test (fixable={fixable}).")
                }
                ScheduledAction::CacheStable => {
                    format!("turn {turn_index}: cache stability observation.")
                }
                ScheduledAction::CacheInvalidate => {
                    format!(
                        "turn {turn_index}: introduce a small continuity disturbance and continue the investigation."
                    )
                }
            }
        };

        llm.set_evidence_turn_index(turn_index);
        llm.set_tag(format!("sqlite-turn{turn_index}"));
        llm.set_role("pipeline");

        // Empty targets = full regenerate path in pipeline (not Director-only partial).
        // Director-only is rejected when provenance still holds prior subagent results.
        let regen_targets = match &action {
            ScheduledAction::RegenerateOverall => Some(Vec::new()),
            ScheduledAction::RegenerateEditor => Some(vec![PartialRollTarget::Editor]),
            ScheduledAction::RegenerateSubagent => {
                Some(vec![PartialRollTarget::Subagent("pending".into())])
            }
            _ => None,
        };
        // Early-fact inject embeds the probe token in SQLite RoundSummary only.
        let summary_probe_id = match &action {
            ScheduledAction::EarlyFactInject { probe_id } => Some(probe_id.as_str()),
            _ => None,
        };

        let mut successful_attempt_sample_start = None;
        let mut accepted_calls_this_attempt = None;
        let mut written = None;
        let mut last_err = None;
        let mut current_retry_state = match read_latest_checkpoint(&paths.checkpoint_jsonl)
            .and_then(|checkpoint| checkpoint.retry_state)
        {
            Some(state) if state.turn_index == turn_index => Some(state),
            Some(_) => {
                return Err(EnduranceError::InvalidConfig(
                    "durable retry state belongs to a different turn".into(),
                ));
            }
            None => None,
        };
        if let Some(state) = current_retry_state.as_ref()
            && (!state.can_retry || state.attempts_used >= MAX_WRITE_ATTEMPTS)
        {
            return Err(EnduranceError::InvalidConfig(format!(
                "turn {turn_index} durable retry budget exhausted after {} attempts",
                state.attempts_used
            )));
        }
        let first_attempt = current_retry_state
            .as_ref()
            .map(|state| state.attempts_used.saturating_add(1))
            .unwrap_or(1);
        let base_failure_checkpoint = || {
            read_latest_checkpoint(&paths.checkpoint_jsonl).or_else(|| {
                Some(EnduranceCheckpoint {
                    schema_version: EnduranceCheckpoint::schema_version().into(),
                    run_id: run_id.clone(),
                    stage: stage.label().into(),
                    accepted_turn_number: turns_accepted,
                    calls_used: 0,
                    max_calls: call_limit,
                    campaign_revision: last_campaign_revision,
                    chronicle_revision: last_chronicle_revision,
                    last_draft_hash16: last_draft_hash16.clone(),
                    last_summary_code: None,
                    context_epoch_id16: None,
                    early_fact_probe_ids: vec![],
                    early_fact_checked_passed: vec![],
                    campaign_id: Some(campaign_id.as_str().to_string()),
                    conversation_id: Some(conversation_id.as_str().to_string()),
                    data_dir_rel: Some("campaign_data".into()),
                    observed_epoch_ids16: epoch_tracker.observed.clone(),
                    run_identity: Some(run_identity.clone()),
                    retry_state: None,
                    probe_state: EnduranceProbeState::default(),
                    sqlite_authority: None,
                    recorded_at_unix_ms: 0,
                })
            })
        };
        let persist_failed_attempt =
            |retry_state: EnduranceRetryState, runner_calls: u32| -> Result<(), EnduranceError> {
                let failure_base = base_failure_checkpoint().ok_or_else(|| {
                    EnduranceError::InvalidConfig(
                        "failed attempt has no durable checkpoint authority".into(),
                    )
                })?;
                validate_live_sqlite_resume_preconditions(&failure_base)?;
                // Always close and truncate an incomplete pre-land Turn before
                // binding a retry checkpoint. If Accept raced to a terminal
                // commit, the unchanged checkpoint revisions/count will make
                // the authority capture below fail closed.
                env.recover_checkpoint_validated_incomplete_turn(&campaign_id)
                    .map_err(EnduranceError::Writer)?;
                let failure_checkpoint = checkpoint_after_failed_attempt(
                    Some(failure_base),
                    turn_index,
                    runner_calls,
                    retry_state,
                )
                .map_err(EnduranceError::InvalidConfig)?;
                validate_live_sqlite_resume_authority(&failure_checkpoint)?;
                persist_checkpoint_and_integrity(paths, &failure_checkpoint, "failed-attempt")
            };
        for attempt in first_attempt..=MAX_WRITE_ATTEMPTS {
            deadline.check()?;
            let attempt_checkpoint =
                read_latest_checkpoint(&paths.checkpoint_jsonl).ok_or_else(|| {
                    EnduranceError::InvalidConfig(
                        "attempt dispatch has no durable checkpoint authority".into(),
                    )
                })?;
            validate_continuous_turn_start_authority(&attempt_checkpoint, turn_index)?;
            let attempt_sample_start = llm.samples().len();
            let fut = async {
                if let Some(targets) = regen_targets.clone() {
                    env.regenerate_accept_turn(
                        &conversation_id,
                        &intent,
                        turn_index,
                        &row.row_id,
                        targets,
                    )
                    .await
                } else {
                    let quality_fault = fixture_spec
                        .as_ref()
                        .and_then(|spec| spec.fault_profile.as_deref())
                        .or_else(|| {
                            matches!(&action, ScheduledAction::QualityAutofix { fixable: true })
                                .then_some("legacy_error_meta_and_format_leak")
                        });
                    env.write_accept_turn_with_fault_profile(
                        &conversation_id,
                        &intent,
                        turn_index,
                        &row.row_id,
                        summary_probe_id,
                        quality_fault,
                    )
                    .await
                }
            };
            let result = match tokio::time::timeout(deadline.remaining()?, fut).await {
                Ok(r) => r,
                Err(_) => {
                    flush_samples(
                        &llm,
                        &call_writer,
                        Some(&tool_writer),
                        &mut sample_cursor,
                        Some(false),
                        &run_id,
                        model_label,
                        turn_index,
                    )?;
                    let runner_calls =
                        harness_real_llm::evidence_retention::reconcile_dangling_call_reservations(
                            &paths.root,
                            &run_id,
                            model_label,
                        )
                        .map_err(|error| {
                            EnduranceError::InvalidConfig(format!(
                                "durable call ledger rejected after suite timeout: {error}"
                            ))
                        })?;
                    // An outer deadline can race a just-completed SQLite Accept.
                    // Persist the call ledger but make the attempt non-retryable:
                    // a later process must fail closed instead of replaying an
                    // ambiguously committed turn.
                    let retry_state = next_retry_state(
                        current_retry_state.as_ref(),
                        turn_index,
                        WriteFailureClass::Fatal,
                    )
                    .map_err(EnduranceError::InvalidConfig)?;
                    persist_failed_attempt(retry_state, runner_calls)?;
                    if runner_calls > call_limit {
                        return Err(EnduranceError::BudgetExhausted {
                            calls_used: runner_calls,
                            max_calls: call_limit,
                        });
                    }
                    return Err(EnduranceError::SuiteTimeout);
                }
            };
            let successful_write_attempt = matches!(&result, Ok(written) if written.accept.ok);
            let calls_this_turn = flush_samples(
                &llm,
                &call_writer,
                Some(&tool_writer),
                &mut sample_cursor,
                Some(successful_write_attempt),
                &run_id,
                model_label,
                turn_index,
            )?;
            let runner_calls =
                harness_real_llm::evidence_retention::reconcile_dangling_call_reservations(
                    &paths.root,
                    &run_id,
                    model_label,
                )
                .map_err(|error| {
                    EnduranceError::InvalidConfig(format!(
                        "durable call ledger reconciliation rejected: {error}"
                    ))
                })?;
            match result {
                Ok(w) => {
                    accepted_calls_this_attempt = Some(calls_this_turn);
                    successful_attempt_sample_start = Some(attempt_sample_start);
                    written = Some(w);
                    break;
                }
                Err(err) => {
                    let failure_class = classify_write_failure(&err, &action);
                    let transient = failure_class.retryable();
                    let retry_state =
                        next_retry_state(current_retry_state.as_ref(), turn_index, failure_class)
                            .map_err(EnduranceError::InvalidConfig)?;
                    persist_failed_attempt(retry_state.clone(), runner_calls)?;
                    current_retry_state = Some(retry_state);
                    eprintln!(
                        "[sqlite endurance {}] turn {turn_index} attempt {attempt}/{MAX_WRITE_ATTEMPTS} failed (transient={transient}, {})",
                        stage.label(),
                        safe_error_summary(&err)
                    );
                    last_err = Some(err);
                    if runner_calls > call_limit {
                        return Err(EnduranceError::BudgetExhausted {
                            calls_used: runner_calls,
                            max_calls: call_limit,
                        });
                    }
                    if !transient || attempt == MAX_WRITE_ATTEMPTS {
                        break;
                    }
                    let retry_delay = retry_delay_within_deadline(
                        &deadline,
                        write_retry_delay(attempt, failure_class),
                    )?;
                    tokio::time::sleep(retry_delay).await;
                }
            }
        }
        let written = written.ok_or_else(|| {
            EnduranceError::Writer(last_err.unwrap_or_else(|| "sqlite write failed".into()))
        })?;
        if !written.accept.ok {
            return Err(EnduranceError::Accept(
                written
                    .accept
                    .error
                    .unwrap_or_else(|| "sqlite accept failed".into()),
            ));
        }

        // Durable accepted marker comes first. If any later audit/evidence step
        // fails or the process stops, resume must never replay this committed
        // turn. Missing later evidence is a fail-closed audit gap, not a retry.
        turns_accepted += 1;
        last_campaign_revision = written.accept.campaign_revision_after;
        last_chronicle_revision = written.accept.chronicle_revision_after;
        last_draft_hash16 = short_hash16(&written.accept.draft_hash);
        let durable_calls =
            harness_real_llm::evidence_retention::count_budgeted_call_records(&paths.root, &run_id)
                .map_err(|error| {
                    EnduranceError::InvalidConfig(format!("accepted call ledger rejected: {error}"))
                })?;
        let accepted_checkpoint = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.clone(),
            stage: stage.label().into(),
            accepted_turn_number: turn_index,
            calls_used: durable_calls,
            max_calls: call_limit,
            campaign_revision: last_campaign_revision,
            chronicle_revision: last_chronicle_revision,
            last_draft_hash16: last_draft_hash16.clone(),
            last_summary_code: written.accept.summary_code.clone(),
            context_epoch_id16: None,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: Some(campaign_id.as_str().to_string()),
            conversation_id: Some(conversation_id.as_str().to_string()),
            data_dir_rel: Some("campaign_data".into()),
            observed_epoch_ids16: epoch_tracker.observed.clone(),
            run_identity: Some(run_identity.clone()),
            retry_state: None,
            probe_state: latest_probe_state(paths)?,
            sqlite_authority: None,
            recorded_at_unix_ms: 0,
        };
        persist_checkpoint_and_integrity(paths, &accepted_checkpoint, "accepted")?;
        let accepted_audit_failure = accepted_attempt_audit_failure(
            accepted_calls_this_attempt.unwrap_or(0),
            durable_calls,
            call_limit,
        );
        if accepted_audit_failure == Some(AcceptedAttemptAuditFailure::ZeroCalls) {
            return Err(EnduranceError::ZeroCalls { turn_index });
        }

        let current_samples = llm.samples();
        let successful_attempt_sample_start = successful_attempt_sample_start.ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "accepted turn is missing its successful-attempt sample boundary".into(),
            )
        })?;
        let current_turn_samples = current_samples
            .get(successful_attempt_sample_start..)
            .unwrap_or_default();
        let world_info_tool_succeeded = has_successful_tool_result(
            current_turn_samples
                .iter()
                .flat_map(|sample| sample.tool_steps.iter()),
            &["search_world_info"],
        );
        let remote_memory_tool_succeeded = has_successful_tool_result(
            current_turn_samples
                .iter()
                .flat_map(|sample| sample.tool_steps.iter()),
            &[
                "get_recent_summary",
                "search_chronicle",
                "get_chronicle",
                "search_vectors",
            ],
        );

        if !env
            .private_final_output_has_no_leak(&written.draft_text)
            .map_err(EnduranceError::Writer)?
        {
            return Err(EnduranceError::SecretViolation(
                "synthetic owner-only fixture value appeared in final narrative".into(),
            ));
        }

        let mut observed = written.observed.clone();
        match &action {
            ScheduledAction::Write {
                reasoning_mode,
                tool_mode,
                world_info_route,
                ..
            } => {
                observed
                    .observations
                    .insert(ObservationKey::CoverageLabel(format!(
                        "subagent_count:{}",
                        written.actual_subagent_count
                    )));
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent(format!(
                        "reasoning_mode:{}",
                        reasoning_mode.label()
                    )));
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent(format!(
                        "tool_mode:{}",
                        tool_mode.label()
                    )));
                if !env.world_info_route_observed(*world_info_route, &intent) {
                    return Err(EnduranceError::InvalidConfig(format!(
                        "world-info route {} was not available from the live fixture",
                        world_info_route.label()
                    )));
                }
                if matches!(
                    world_info_route,
                    WorldInfoRouteSlot::Selective | WorldInfoRouteSlot::Both
                ) && !world_info_tool_succeeded
                {
                    return Err(EnduranceError::InvalidConfig(format!(
                        "world-info route {} did not produce a successful search_world_info result",
                        world_info_route.label()
                    )));
                }
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent(format!(
                        "world_info_route_available:{}",
                        world_info_route.label()
                    )));
            }
            ScheduledAction::RegenerateOverall => {
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent("regenerate:overall".into()));
            }
            ScheduledAction::RegenerateEditor => {
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent("regenerate:editor_only".into()));
            }
            ScheduledAction::RegenerateSubagent => {
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent("regenerate:subagent_only".into()));
            }
            ScheduledAction::PrivateProbe { probe_kind } => {
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent(format!(
                        "private_final_output:{}:no_leak",
                        SqliteHarnessEnv::private_probe_slug(probe_kind)
                    )));
            }
            ScheduledAction::EarlyFactInject { probe_id } => {
                let contents = env
                    .list_summary_contents(&campaign_id)
                    .map_err(EnduranceError::Writer)?;
                if !contents.iter().any(|content| content.contains(probe_id)) {
                    return Err(EnduranceError::Writer(format!(
                        "early fact probe was not persisted by the actual Summarizer path: {}",
                        short_hash16(probe_id)
                    )));
                }
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent(format!(
                        "early_fact:injected:{probe_id}"
                    )));
            }
            ScheduledAction::QualityAutofix { fixable } => {
                if *fixable && (!written.autofix_applied || written.quality_error_count != 0) {
                    return Err(EnduranceError::InvalidConfig(
                        "quality autofix row did not complete a real Editor rewrite to zero errors"
                            .into(),
                    ));
                }
                if *fixable {
                    observed.observations.insert(ObservationKey::AutofixSynced);
                    observed
                        .observations
                        .insert(ObservationKey::OutboxKind("autofix_sync".into()));
                    observed
                        .observations
                        .insert(ObservationKey::ToolEvent("autofix:fixed".into()));
                } else {
                    observed
                        .observations
                        .insert(ObservationKey::ToolEvent("autofix:rejected".into()));
                }
            }
            ScheduledAction::CacheStable => {
                // Validated against the frozen context epoch after production
                // context fill. Full request fingerprints legitimately change
                // as the conversation tail advances.
            }
            ScheduledAction::CacheInvalidate => {
                // Validated against the frozen context epoch below.
            }
            ScheduledAction::EarlyFactCheck { .. } => {}
        }
        if let ScheduledAction::EarlyFactCheck { probe_id } = &action {
            if !remote_memory_tool_succeeded {
                return Err(EnduranceError::InvalidConfig(
                    "early-fact check did not receive a successful remote-memory tool result"
                        .into(),
                ));
            }
            let contents = env
                .list_summary_contents(&campaign_id)
                .map_err(EnduranceError::Writer)?;
            let reachable = contents.iter().any(|c| c.contains(probe_id))
                || contents.iter().any(|c| c.contains(&short_hash16(probe_id)));
            if !reachable {
                return Err(EnduranceError::Writer(format!(
                    "early fact probe not reachable via sqlite list_summaries \
                     (row={}, probe_hash={})",
                    row.row_id,
                    short_hash16(probe_id)
                )));
            }
            observed
                .observations
                .insert(harness_real_llm::coverage_ledger::ObservationKey::EarlyFactChecked);
            observed.observations.insert(
                harness_real_llm::coverage_ledger::ObservationKey::CommandPath(
                    "sqlite_runtime::list_summaries".into(),
                ),
            );
            observed.observations.insert(
                harness_real_llm::coverage_ledger::ObservationKey::ServicePath(
                    "early_fact_check".into(),
                ),
            );
            observed
                .observations
                .insert(ObservationKey::ToolEvent(format!(
                    "early_fact:sqlite_reachable_and_remote_tool_succeeded:{probe_id}"
                )));
        }
        let ctx = env
            .fill_campaign_context(WritingContext::legacy(
                vec![],
                None,
                conversation_id.clone(),
            ))
            .map_err(EnduranceError::Writer)?;
        // Production context fill can publish/compress and advance chronicle after
        // Accept. Keep the durable runner cursor on the live monotone value so
        // turn-audit authority binding does not fail closed on an expected bump.
        let live_campaign = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
            .map_err(EnduranceError::Writer)?
            .ok_or_else(|| EnduranceError::Writer("campaign missing after context fill".into()))?;
        if live_campaign.revision != last_campaign_revision {
            return Err(EnduranceError::InvalidConfig(
                "campaign revision drifted during post-accept context fill".into(),
            ));
        }
        if live_campaign.chronicle_revision < last_chronicle_revision {
            return Err(EnduranceError::InvalidConfig(
                "SQLite chronicle revision regressed during post-accept context fill".into(),
            ));
        }
        last_chronicle_revision = live_campaign.chronicle_revision;
        let epoch_id16 = ctx
            .context_epoch
            .as_ref()
            .map(|e| short_hash16(&e.epoch_id));
        validate_cache_epoch_transition(
            &action,
            last_context_epoch_id16.as_deref(),
            epoch_id16.as_deref(),
        )?;
        match &action {
            ScheduledAction::CacheStable => {
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent("cache:stable".into()));
            }
            ScheduledAction::CacheInvalidate => {
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent("cache:invalidated".into()));
            }
            _ => {}
        }
        last_context_epoch_id16 = epoch_id16.clone();
        if let Some(ref eid) = epoch_id16 {
            epoch_tracker.observe(eid);
        }

        ledger.record(observed.clone());
        let line = serde_json::to_string(&CoverageLedgerEvidenceRow {
            schema_version: harness_real_llm::evidence::EVIDENCE_SCHEMA_VERSION.into(),
            run_id: run_id.clone(),
            observation: observed,
        })
        .map_err(|error| {
            EnduranceError::InvalidConfig(format!("coverage ledger serialization: {error}"))
        })?;
        // Durable per-turn append so resume can rebuild exact-set without replaying.
        {
            use std::io::Write;
            if let Some(parent) = ledger_path.parent() {
                std::fs::create_dir_all(parent).map_err(EnduranceError::EvidenceIo)?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&ledger_path)
                .map_err(EnduranceError::EvidenceIo)?;
            writeln!(f, "{line}").map_err(EnduranceError::EvidenceIo)?;
            f.sync_data().map_err(EnduranceError::EvidenceIo)?;
        }

        let turn_rec = build_endurance_turn_record(EnduranceTurnRecordInput {
            run_id: &run_id,
            stage,
            turn_index,
            action: &action,
            draft_accepted: true,
            campaign_revision_before: written.accept.campaign_revision_before,
            campaign_revision_after: written.accept.campaign_revision_after,
            chronicle_revision_before: written.accept.chronicle_revision_before,
            chronicle_revision_after: last_chronicle_revision,
            summary_code: written.accept.summary_code.clone(),
            draft_hash16: last_draft_hash16.clone(),
            text_len: written.draft_text.chars().count(),
            text_sha16: short_hash16(&written.draft_text),
            context_epoch_id16: epoch_id16.clone(),
            context_epoch_source_hash16: ctx
                .context_epoch
                .as_ref()
                .map(|e| short_hash16(&e.source_hash)),
            context_epoch_anchor_count: ctx
                .context_epoch
                .as_ref()
                .map(|e| e.raw_anchor_turn_ids.len()),
            attempt_status: format!("{:?}", written.accept.attempt_status),
            turn_status: format!("{:?}", written.accept.turn_status),
            assertions: vec![
                harness_real_llm::evidence::AssertionResult {
                    name: "sqlite_authoritative".into(),
                    passed: true,
                    detail: Some("true".into()),
                },
                harness_real_llm::evidence::AssertionResult {
                    name: "production_postprocess_complete".into(),
                    passed: written.postprocess_proof.applied,
                    detail: Some(format!("{}", written.postprocess_proof.applied)),
                },
            ],
            elapsed_ms: deadline.started.elapsed().as_millis(),
            write_path: Some("pipeline.start_writing+sqlite_runtime::create_draft_attempt"),
            chronicle_path: Some("production_postprocess_service_real_agents"),
            accept_path: Some("sqlite_runtime::accept_by_variant"),
            production_postprocess_complete: Some(written.postprocess_proof.applied),
        });
        turn_writer.write_turn(turn_rec)?;

        let cp = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.clone(),
            stage: stage.label().into(),
            accepted_turn_number: turn_index,
            calls_used: durable_calls,
            max_calls: call_limit,
            campaign_revision: last_campaign_revision,
            chronicle_revision: last_chronicle_revision,
            last_draft_hash16: last_draft_hash16.clone(),
            last_summary_code: written.accept.summary_code.clone(),
            context_epoch_id16: epoch_id16,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: Some(campaign_id.as_str().to_string()),
            conversation_id: Some(conversation_id.as_str().to_string()),
            data_dir_rel: Some("campaign_data".into()),
            observed_epoch_ids16: epoch_tracker.observed.clone(),
            run_identity: Some(run_identity.clone()),
            retry_state: None,
            probe_state: latest_probe_state(paths)?,
            sqlite_authority: None,
            recorded_at_unix_ms: 0,
        };
        persist_checkpoint_and_integrity(paths, &cp, "turn-audit")?;
        if checkpoint_stop_after(turn_index) && turn_index < target_turns {
            eprintln!(
                "[sqlite endurance {}] graceful checkpoint stop after turn {turn_index}",
                stage.label()
            );
            return Err(EnduranceError::CheckpointStop { turn_index });
        }
        if accepted_audit_failure == Some(AcceptedAttemptAuditFailure::BudgetExhausted) {
            return Err(EnduranceError::BudgetExhausted {
                calls_used: durable_calls,
                max_calls: call_limit,
            });
        }
        eprintln!(
            "[sqlite endurance {}] turn {turn_index}/{target_turns} accepted",
            stage.label()
        );
    }

    let mut meta_probe_calls = 0usize;
    if meta_probe_enabled {
        let completed = SupplementalProbe::Meta.completed(&latest_probe_state(paths)?);
        if !completed {
            llm.set_evidence_turn_index(target_turns.saturating_add(1));
            begin_supplemental_probe(paths, SupplementalProbe::Meta)?;
            let probe_result = run_sqlite_meta_probe(
                env,
                llm.clone(),
                &conversation_id,
                &call_writer,
                &tool_writer,
                &mut sample_cursor,
                &run_id,
                model_label,
                target_turns.saturating_add(1),
            )
            .await;
            let durable_calls = finish_supplemental_probe(
                paths,
                SupplementalProbe::Meta,
                probe_result.is_ok(),
                model_label,
            )?;
            if durable_calls > call_limit {
                return Err(EnduranceError::BudgetExhausted {
                    calls_used: durable_calls,
                    max_calls: call_limit,
                });
            }
            probe_result?;
        }
        meta_probe_calls = durable_probe_metrics(&paths.calls_jsonl, "meta")?.0;
        if meta_probe_calls == 0 {
            return Err(EnduranceError::InvalidConfig(
                "completed Meta probe has no durable call evidence".into(),
            ));
        }
    }

    let mut cache_probe_calls = 0usize;
    let mut cache_probe_cached_tokens = 0u32;
    if cache_probe_enabled {
        let completed = SupplementalProbe::Cache.completed(&latest_probe_state(paths)?);
        if !completed {
            llm.set_evidence_turn_index(target_turns.saturating_add(2));
            begin_supplemental_probe(paths, SupplementalProbe::Cache)?;
            let probe_result = run_cache_fingerprint_probe(
                llm.clone(),
                &call_writer,
                &tool_writer,
                &mut sample_cursor,
                &run_id,
                model_label,
                target_turns.saturating_add(2),
            )
            .await;
            let durable_calls = finish_supplemental_probe(
                paths,
                SupplementalProbe::Cache,
                probe_result.is_ok(),
                model_label,
            )?;
            if durable_calls > call_limit {
                return Err(EnduranceError::BudgetExhausted {
                    calls_used: durable_calls,
                    max_calls: call_limit,
                });
            }
            probe_result?;
        }
        (cache_probe_calls, cache_probe_cached_tokens) =
            durable_cache_probe_metrics(&paths.calls_jsonl)?;
        if cache_probe_calls == 0 {
            return Err(EnduranceError::InvalidConfig(
                "completed cache probe has no durable call evidence".into(),
            ));
        }
    }

    if let Err(ms) = ledger.exact_set_verify() {
        return Err(EnduranceError::InvalidConfig(format!(
            "coverage ledger exact-set failed: {ms:?}"
        )));
    }
    let tool_call_steps = durable_successful_write_tool_call_steps(&paths.calls_jsonl)
        .map_err(EnduranceError::InvalidConfig)?;
    if tool_call_steps == 0 {
        return Err(EnduranceError::InvalidConfig(format!(
            "configured {} tool transport produced no actual tool-call steps",
            runtime_profile.tool_mode.label()
        )));
    }
    if let Err(msg) = paths.check_no_secrets() {
        return Err(EnduranceError::SecretViolation(msg));
    }

    let total_size = paths.total_size() as usize;
    if total_size > 2 * 1024 * 1024 {
        return Err(EnduranceError::InvalidConfig(format!(
            "evidence size {total_size} exceeds 2MB budget"
        )));
    }

    let calls_used =
        harness_real_llm::evidence_retention::count_budgeted_call_records(&paths.root, &run_id)
            .map_err(|error| {
                EnduranceError::InvalidConfig(format!("final call ledger rejected: {error}"))
            })?;
    let acceptance = classify_acceptance_with_call_limit(
        stage,
        turns_accepted,
        calls_used,
        call_limit,
        turns_accepted >= target_turns,
        false,
        false,
    );
    let mut coverage_assertions = ledger.assertion_results();
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "actual_tool_mode".into(),
        passed: true,
        detail: Some(format!(
            "{};tool_call_steps={tool_call_steps}",
            runtime_profile.tool_mode.label()
        )),
    });
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "actual_reasoning_mode".into(),
        passed: true,
        detail: Some(runtime_profile.reasoning_mode.label().into()),
    });
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "sqlite_meta_campaign_tasks_probe".into(),
        passed: !meta_probe_enabled || meta_probe_calls > 0,
        detail: Some(if meta_probe_enabled {
            format!("calls={meta_probe_calls};tools=inspect_campaign,inspect_tasks")
        } else {
            "not_requested".into()
        }),
    });
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "character_extractor_real_probe".into(),
        passed: !extractor_probe_enabled || extractor_probe_calls > 0,
        detail: Some(if extractor_probe_enabled {
            format!("calls={extractor_probe_calls};tool=emit_characters")
        } else {
            "not_requested".into()
        }),
    });
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "cache_fingerprint_real_probe".into(),
        passed: !cache_probe_enabled || cache_probe_calls == 3,
        detail: Some(if cache_probe_enabled {
            format!(
                "calls={cache_probe_calls};cached_tokens={cache_probe_cached_tokens};provider_cache_hit_claimed={}",
                cache_probe_cached_tokens > 0
            )
        } else {
            "not_requested".into()
        }),
    });
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "sqlite_authoritative".into(),
        passed: true,
        detail: Some("true".into()),
    });
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "json_fallback".into(),
        passed: true,
        detail: Some("false".into()),
    });
    coverage_assertions.push(harness_real_llm::evidence::AssertionResult {
        name: "gui_device_claimed".into(),
        passed: true,
        detail: Some("false".into()),
    });

    let row = EnduranceStageManifestRow {
        schema_version: EnduranceCheckpoint::schema_version().into(),
        run_id,
        stage: stage.label().into(),
        target_turns,
        accepted_turns: turns_accepted,
        calls_used,
        max_calls: call_limit,
        elapsed_ms: deadline.started.elapsed().as_millis(),
        acceptance: acceptance.label().into(),
        summary_codes: vec![],
        observed_epoch_ids16: epoch_tracker.observed.clone(),
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        coverage_assertions,
        recorded_at_unix_ms: 0,
    };
    write_manifest_row(&paths.manifest_jsonl, &row)?;
    if acceptance != AcceptanceLevel::Pass && stage == EnduranceStage::Full {
        return Err(EnduranceError::InvalidConfig(format!(
            "full stage did not pass: acceptance={acceptance}"
        )));
    }
    Ok(row)
}

#[test]
fn supplemental_matrix_fits_special_paths_into_twelve_accepted_turns() {
    let schedule = EnduranceSchedule::new(12);
    assert!(matches!(
        scheduled_action_for_run(&schedule, 4, true),
        ScheduledAction::Write {
            subagent_count: 3,
            world_info_route: WorldInfoRouteSlot::Both,
            ..
        }
    ));
    assert!(matches!(
        scheduled_action_for_run(&schedule, 5, true),
        ScheduledAction::RegenerateOverall
    ));
    assert!(matches!(
        scheduled_action_for_run(&schedule, 6, true),
        ScheduledAction::RegenerateEditor
    ));
    assert!(matches!(
        scheduled_action_for_run(&schedule, 7, true),
        ScheduledAction::RegenerateSubagent
    ));
    assert!(matches!(
        scheduled_action_for_run(&schedule, 12, true),
        ScheduledAction::QualityAutofix { fixable: true }
    ));
}

#[test]
fn coverage_ledger_rows_bind_run_identity_at_the_top_level() {
    let observation = harness_real_llm::coverage_ledger::ObservedCoverage {
        row_id: "row-1".into(),
        turn_index: 1,
        command_path: "command".into(),
        service_path: "service".into(),
        agent_events: vec![],
        turn_id16: "1111111111111111".into(),
        attempt_id16: "2222222222222222".into(),
        variant_id16: "3333333333333333".into(),
        sqlite_post: harness_real_llm::coverage_ledger::SqlitePostcondition::default(),
        observations: Default::default(),
    };
    let row = CoverageLedgerEvidenceRow {
        schema_version: harness_real_llm::evidence::EVIDENCE_SCHEMA_VERSION.into(),
        run_id: "run-coverage-a".into(),
        observation,
    };
    let line = serde_json::to_string(&row).unwrap();
    assert!(parse_coverage_ledger_evidence(&line, "run-coverage-a").is_ok());
    assert!(parse_coverage_ledger_evidence(&line, "run-coverage-b").is_err());
    let missing_identity = serde_json::to_string(&row.observation).unwrap();
    assert!(parse_coverage_ledger_evidence(&missing_identity, "run-coverage-a").is_err());
}

#[test]
fn successful_tool_result_requires_matching_ok_result() {
    use harness_real_llm::evidence::EvidenceToolStep;

    let failed = EvidenceToolStep {
        kind: "result".into(),
        tool_name: "search_world_info".into(),
        call_id16: None,
        args_hash16: None,
        args_len: None,
        result_hash16: None,
        result_len: None,
        ok: Some(false),
        detail: None,
    };
    let succeeded = EvidenceToolStep {
        ok: Some(true),
        ..failed.clone()
    };
    assert!(!has_successful_tool_result(
        [&failed],
        &["search_world_info"]
    ));
    assert!(has_successful_tool_result(
        [&succeeded],
        &["search_world_info"]
    ));
    assert!(!has_successful_tool_result(
        [&succeeded],
        &["get_recent_summary"]
    ));
}

#[test]
fn durable_tool_mode_proof_ignores_failed_retry_calls() {
    let dir =
        std::env::temp_dir().join(format!("sf-sqlite-durable-tools-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("calls.jsonl");
    std::fs::write(
        &path,
        concat!(
            "{\"outcome\":\"ok\",\"assertion_results\":[{\"name\":\"successful_write_attempt\",\"passed\":false}],\"tool_steps\":[{\"kind\":\"call\"}]}\n",
            "{\"outcome\":\"client_error\",\"assertion_results\":[{\"name\":\"successful_write_attempt\",\"passed\":true}],\"tool_steps\":[{\"kind\":\"call\"}]}\n",
            "{\"outcome\":\"ok\",\"assertion_results\":[{\"name\":\"successful_write_attempt\",\"passed\":true}],\"tool_steps\":[{\"kind\":\"call\"},{\"kind\":\"result\"}]}\n"
        ),
    )
    .unwrap();
    let count = durable_successful_write_tool_call_steps(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(count, 1);
}

#[test]
fn write_retry_policy_is_typed_bounded_and_autofix_strict() {
    let private_probe = ScheduledAction::PrivateProbe {
        probe_kind: PrivateProbeKind::NonOwnerLeak,
    };
    assert_eq!(
        classify_write_failure("retryable_quality_blocked:error_count=1", &private_probe),
        WriteFailureClass::QualityBlocked
    );
    assert_eq!(
        classify_write_failure("stream idle timeout", &private_probe),
        WriteFailureClass::Transient
    );
    assert_eq!(
        classify_write_failure(
            "regenerate failed: editor response was not parseable",
            &private_probe,
        ),
        WriteFailureClass::Transient,
        "model-facing pipeline failures must use the bounded retry path"
    );
    assert_eq!(
        classify_write_failure(
            "pipeline missing completed agent events: [\"postprocessor\"]",
            &private_probe,
        ),
        WriteFailureClass::Transient,
        "a pre-Accept coverage gap must use the existing bounded retry path"
    );
    assert_eq!(
        classify_write_failure("campaign scope mismatch", &private_probe),
        WriteFailureClass::Fatal
    );
    assert_eq!(
        classify_write_failure("storage timeout during authority write", &private_probe),
        WriteFailureClass::Fatal
    );
    assert_eq!(
        classify_write_failure("Internal sqlite authority drift", &private_probe),
        WriteFailureClass::Fatal
    );
    assert_eq!(
        classify_write_failure(
            "nonretryable_accept: storage timeout after terminal mark",
            &private_probe,
        ),
        WriteFailureClass::Fatal
    );
    assert_eq!(
        classify_write_failure(
            "nonretryable_accept: Internal scope conflict",
            &private_probe,
        ),
        WriteFailureClass::Fatal
    );
    assert_eq!(
        classify_write_failure(
            "retryable_quality_blocked:error_count=1",
            &ScheduledAction::QualityAutofix { fixable: true },
        ),
        WriteFailureClass::QualityBlocked,
        "a residual quality failure may retry, but the accepted row still requires zero errors"
    );

    // Gate 6 §11.2: a provider/relay transient failure wrapped in
    // PipelineError::Llm must be retryable EVEN IF the relay's forwarded error
    // body incidentally contains a StoryForge-internal fatal keyword. Long
    // endurance runs (Stability 30 / Full 100) must survive a single transient
    // 5xx/timeout from the relay without fail-closing the whole stage.
    assert_eq!(
        classify_write_failure(
            "LLM 错误: 服务端错误 (5xx): upstream storage backend unavailable",
            &private_probe,
        ),
        WriteFailureClass::Transient,
        "relay 5xx with incidental 'storage' in the body must retry, not fail-closed"
    );
    assert_eq!(
        classify_write_failure(
            "LLM 错误: 服务端错误 (5xx): authority check failed upstream",
            &private_probe,
        ),
        WriteFailureClass::Transient,
        "relay 5xx with incidental 'authority' in the body must retry"
    );
    assert_eq!(
        classify_write_failure("LLM 错误: 超时", &private_probe),
        WriteFailureClass::Transient,
        "provider timeout must retry"
    );
    assert_eq!(
        classify_write_failure("LLM 错误: 速率限制 (429): too many requests", &private_probe),
        WriteFailureClass::Transient,
        "provider rate-limit must retry"
    );
    // Regression guard: genuine StoryForge-internal storage/authority failures
    // (NOT wrapped in PipelineError::Llm) stay Fatal.
    assert_eq!(
        classify_write_failure("storage timeout during authority write", &private_probe),
        WriteFailureClass::Fatal,
        "internal StoryForge storage failure stays fatal (no LLM error wrapper)"
    );

    let mut state = None;
    for attempt in 1..=MAX_WRITE_ATTEMPTS {
        state =
            Some(next_retry_state(state.as_ref(), 9, WriteFailureClass::QualityBlocked).unwrap());
        assert_eq!(state.as_ref().unwrap().attempts_used, attempt);
    }
    assert!(!state.as_ref().unwrap().can_retry);
    assert!(
        next_retry_state(state.as_ref(), 9, WriteFailureClass::QualityBlocked).is_err(),
        "a process restart must not reset the five-attempt ceiling"
    );
}

#[test]
fn transient_write_retries_use_recovery_sized_backoff() {
    assert_eq!(
        write_retry_delay(1, WriteFailureClass::Transient),
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        write_retry_delay(2, WriteFailureClass::Transient),
        std::time::Duration::from_secs(15)
    );
    assert_eq!(
        write_retry_delay(3, WriteFailureClass::Transient),
        std::time::Duration::from_secs(30)
    );
    assert_eq!(
        write_retry_delay(4, WriteFailureClass::Transient),
        std::time::Duration::from_secs(60)
    );
    assert_eq!(
        write_retry_delay(4, WriteFailureClass::QualityBlocked),
        std::time::Duration::from_secs(2)
    );

    let ample = SuiteDeadline::new(std::time::Duration::from_secs(60));
    assert_eq!(
        retry_delay_within_deadline(&ample, write_retry_delay(1, WriteFailureClass::Transient))
            .unwrap(),
        std::time::Duration::from_secs(5)
    );
    let exhausted = SuiteDeadline::new(std::time::Duration::from_millis(1));
    assert!(
        retry_delay_within_deadline(
            &exhausted,
            write_retry_delay(4, WriteFailureClass::Transient)
        )
        .is_err()
    );
}

#[test]
fn retry_checkpoint_accounts_for_failed_calls_without_advancing_acceptance() {
    let previous = EnduranceCheckpoint {
        schema_version: EnduranceCheckpoint::schema_version().into(),
        run_id: "run-coverage-00000000-0000-0000-0000-000000000001".into(),
        stage: "coverage".into(),
        accepted_turn_number: 8,
        calls_used: 144,
        max_calls: 220,
        campaign_revision: 8,
        chronicle_revision: 8,
        last_draft_hash16: "draft12345678901".into(),
        last_summary_code: None,
        context_epoch_id16: None,
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        campaign_id: Some("campaign".into()),
        conversation_id: Some("conversation".into()),
        data_dir_rel: Some("campaign_data".into()),
        observed_epoch_ids16: vec![],
        run_identity: None,
        retry_state: None,
        probe_state: EnduranceProbeState::default(),
        sqlite_authority: None,
        recorded_at_unix_ms: 0,
    };
    let retry = EnduranceRetryState {
        turn_index: 9,
        attempts_used: 1,
        quality_blocked_attempts: 1,
        last_failure_kind: "quality_blocked".into(),
        can_retry: true,
    };
    let updated = checkpoint_after_failed_attempt(Some(previous), 9, 155, retry).unwrap();
    assert_eq!(updated.accepted_turn_number, 8);
    assert_eq!(updated.calls_used, 155);
    assert_eq!(updated.retry_state.unwrap().attempts_used, 1);
}

#[test]
fn accepted_attempt_anomalies_are_deferred_until_after_durable_checkpoint() {
    assert_eq!(
        accepted_attempt_audit_failure(0, 15, 220),
        Some(AcceptedAttemptAuditFailure::ZeroCalls)
    );
    assert_eq!(
        accepted_attempt_audit_failure(3, 221, 220),
        Some(AcceptedAttemptAuditFailure::BudgetExhausted)
    );
    assert_eq!(accepted_attempt_audit_failure(3, 220, 220), None);
}

#[test]
fn cache_transition_assertions_use_context_epoch_not_full_request_fingerprint() {
    assert!(
        validate_cache_epoch_transition(
            &ScheduledAction::CacheStable,
            Some("epoch-a"),
            Some("epoch-a"),
        )
        .is_ok()
    );
    assert!(
        validate_cache_epoch_transition(
            &ScheduledAction::CacheInvalidate,
            Some("epoch-a"),
            Some("epoch-b"),
        )
        .is_ok()
    );

    let stable_error = validate_cache_epoch_transition(
        &ScheduledAction::CacheStable,
        Some("epoch-a"),
        Some("epoch-b"),
    )
    .unwrap_err()
    .to_string();
    assert!(stable_error.contains("context epoch"));

    let invalidate_error = validate_cache_epoch_transition(
        &ScheduledAction::CacheInvalidate,
        Some("epoch-a"),
        Some("epoch-a"),
    )
    .unwrap_err()
    .to_string();
    assert!(invalidate_error.contains("context epoch"));
}

#[test]
fn cache_transition_assertions_fail_closed_without_both_epoch_ids() {
    assert!(
        validate_cache_epoch_transition(&ScheduledAction::CacheStable, None, Some("epoch-a"))
            .is_err()
    );
    assert!(
        validate_cache_epoch_transition(&ScheduledAction::CacheInvalidate, Some("epoch-a"), None)
            .is_err()
    );
}

#[test]
fn supplemental_probe_state_is_bounded_and_terminal() {
    let mut state = EnduranceProbeState::default();
    SupplementalProbe::Meta.begin(&mut state).unwrap();
    SupplementalProbe::Meta.begin(&mut state).unwrap();
    SupplementalProbe::Meta.begin(&mut state).unwrap();
    assert!(SupplementalProbe::Meta.begin(&mut state).is_err());
    SupplementalProbe::Meta.mark_completed(&mut state);
    assert!(SupplementalProbe::Meta.completed(&state));
    assert!(SupplementalProbe::Meta.begin(&mut state).is_err());
}

#[test]
fn resume_fingerprint_uses_last_accepted_turn_not_failed_call_tail() {
    let dir = std::env::temp_dir().join(format!(
        "sf-sqlite-resume-fingerprint-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("endurance_calls.jsonl");
    std::fs::write(
        &path,
        concat!(
            "{\"turn_index\":8,\"request_fp16\":\"accepted-first\"}\n",
            "{\"turn_index\":8,\"request_fp16\":\"accepted-last\",\"outcome\":\"ok\"}\n",
            "{\"turn_index\":8,\"request_fp16\":\"same-turn-failed-tail\",\"outcome\":\"client_error\"}\n",
            "{\"turn_index\":9,\"request_fp16\":\"failed-tail\",\"outcome\":\"client_error\"}\n"
        ),
    )
    .unwrap();
    let observed = last_request_fp16_for_turn(&path, 8).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(observed, Some("accepted-last".into()));
}

#[test]
fn sqlite_retry_recovery_and_authority_rebinding_share_one_process() {
    let dir =
        std::env::temp_dir().join(format!("sf-sqlite-retry-recovery-{}", uuid::Uuid::new_v4()));
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = SqliteHarnessEnv::bootstrap(dir, llm).unwrap();
    let campaign_id = env.active_campaign_id().unwrap();
    let campaign = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
        .unwrap()
        .unwrap();
    let conversation_id = campaign.conversation_id.unwrap();
    let mut conversation = storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
        .unwrap()
        .unwrap();
    let original_nodes = conversation.nodes.len();
    let mut checkpoint = EnduranceCheckpoint {
        schema_version: EnduranceCheckpoint::schema_version().into(),
        run_id: "run-coverage-00000000-0000-0000-0000-000000000099".into(),
        stage: "coverage".into(),
        accepted_turn_number: 0,
        calls_used: 0,
        max_calls: 220,
        campaign_revision: campaign.revision,
        chronicle_revision: campaign.chronicle_revision,
        last_draft_hash16: String::new(),
        last_summary_code: None,
        context_epoch_id16: None,
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        campaign_id: Some(campaign_id.as_str().into()),
        conversation_id: Some(conversation_id.as_str().into()),
        data_dir_rel: Some("campaign_data".into()),
        observed_epoch_ids16: vec![],
        run_identity: None,
        retry_state: None,
        probe_state: EnduranceProbeState::default(),
        sqlite_authority: None,
        recorded_at_unix_ms: 0,
    };
    checkpoint.sqlite_authority = Some(capture_sqlite_authority_binding(&checkpoint).unwrap());
    let accepted_before = checkpoint.sqlite_authority.clone().unwrap();
    let input_node_id = conversation.append_message(Role::User, "retry fixture input".into());
    storyforge_tauri_app::sqlite_runtime::save_conversation(&conversation).unwrap();

    let turn = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        input_node_id,
        campaign.revision,
    );
    let turn_id = turn.turn_id.clone();
    storyforge_tauri_app::sqlite_runtime::save_turn(&turn).unwrap();
    let attempt_id = storyforge_domain::Id::new();
    storyforge_tauri_app::sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &turn_id,
        attempt_id: &attempt_id,
        draft_text: "unaccepted retry draft",
        pending_temporary_instances: vec![],
        provenance: None,
    })
    .unwrap();

    let with_unaccepted_tail = capture_sqlite_authority_binding(&checkpoint).unwrap();
    assert_ne!(
        accepted_before.accepted_content_sha256,
        with_unaccepted_tail.accepted_content_sha256
    );
    validate_live_sqlite_resume_preconditions(&checkpoint).unwrap();
    env.recover_checkpoint_validated_incomplete_turn(&campaign_id)
        .unwrap();
    validate_live_sqlite_resume_authority(&checkpoint).unwrap();

    let recovered_turn = storyforge_tauri_app::sqlite_runtime::get_turn(&turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered_turn.status, TurnStatus::Failed);
    assert_eq!(
        recovered_turn.find_attempt(&attempt_id).unwrap().status,
        AttemptStatus::Failed
    );
    assert!(
        storyforge_tauri_app::sqlite_runtime::get_active_turn(&campaign_id)
            .unwrap()
            .is_none()
    );
    let recovered_conversation =
        storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .unwrap();
    assert_eq!(recovered_conversation.nodes.len(), original_nodes);
    assert_eq!(
        capture_sqlite_authority_binding(&checkpoint).unwrap(),
        accepted_before,
        "recovery must restore the exact accepted-state projection"
    );

    // Continuous (non-resume) turn starts must validate the preceding durable
    // authority instead of silently rebinding unrelated SQLite drift.
    let baseline_campaign = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
        .unwrap()
        .unwrap();
    let mut between_turn_drift = baseline_campaign.clone();
    between_turn_drift.name.push_str("-between-turn-drift");
    storyforge_tauri_app::sqlite_runtime::save_campaign(&between_turn_drift).unwrap();
    let before_continuous_reject =
        storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert!(validate_continuous_turn_start_authority(&checkpoint, 1).is_err());
    let after_continuous_reject =
        storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert_eq!(
        before_continuous_reject.canonical_content_sha256,
        after_continuous_reject.canonical_content_sha256,
        "continuous authority rejection must be zero-write"
    );
    storyforge_tauri_app::sqlite_runtime::save_campaign(&baseline_campaign).unwrap();
    validate_continuous_turn_start_authority(&checkpoint, 1).unwrap();

    // A tail input must be linked directly to the accepted prefix. A detached
    // input at the correct numeric index is still unsafe and must remain zero-write.
    let mut detached_conversation =
        storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .unwrap();
    let detached_prefix_len = detached_conversation.nodes.len();
    let detached_input =
        detached_conversation.append_message(Role::User, "detached tail fixture".into());
    detached_conversation.nodes[detached_prefix_len].parent_id = Some(storyforge_domain::Id::new());
    storyforge_tauri_app::sqlite_runtime::save_conversation(&detached_conversation).unwrap();
    let detached_turn = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        detached_input,
        baseline_campaign.revision,
    );
    storyforge_tauri_app::sqlite_runtime::save_turn(&detached_turn).unwrap();
    let before_detached = storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert!(validate_live_sqlite_resume_preconditions(&checkpoint).is_err());
    assert!(
        env.recover_checkpoint_validated_incomplete_turn(&campaign_id)
            .is_err()
    );
    let after_detached = storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert_eq!(
        before_detached.canonical_content_sha256, after_detached.canonical_content_sha256,
        "detached input rejection must be zero-write"
    );
    storyforge_tauri_app::sqlite_runtime::recover_turns_on_startup().unwrap();
    let mut restored_conversation =
        storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .unwrap();
    restored_conversation.nodes.truncate(detached_prefix_len);
    storyforge_tauri_app::sqlite_runtime::save_conversation(&restored_conversation).unwrap();
    validate_live_sqlite_resume_authority(&checkpoint).unwrap();

    // Crash idempotency: truncation may commit before the Turn/Attempt failure
    // transaction. A validated restart must finish closing that active Turn
    // without requiring the now-absent input anchor.
    let mut crash_conversation =
        storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .unwrap();
    let crash_input = crash_conversation.append_message(Role::User, "crash-window fixture".into());
    storyforge_tauri_app::sqlite_runtime::save_conversation(&crash_conversation).unwrap();
    let crash_turn = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        crash_input,
        campaign.revision,
    );
    let crash_turn_id = crash_turn.turn_id.clone();
    storyforge_tauri_app::sqlite_runtime::save_turn(&crash_turn).unwrap();
    let crash_attempt_id = storyforge_domain::Id::new();
    storyforge_tauri_app::sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &crash_turn_id,
        attempt_id: &crash_attempt_id,
        draft_text: "crash-window draft",
        pending_temporary_instances: vec![],
        provenance: None,
    })
    .unwrap();
    let mut already_truncated =
        storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .unwrap();
    already_truncated.nodes.truncate(original_nodes);
    storyforge_tauri_app::sqlite_runtime::save_conversation(&already_truncated).unwrap();
    validate_live_sqlite_resume_preconditions(&checkpoint).unwrap();
    env.recover_checkpoint_validated_incomplete_turn(&campaign_id)
        .unwrap();
    validate_live_sqlite_resume_authority(&checkpoint).unwrap();
    let recovered_crash_turn = storyforge_tauri_app::sqlite_runtime::get_turn(&crash_turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered_crash_turn.status, TurnStatus::Failed);
    assert_eq!(
        recovered_crash_turn
            .find_attempt(&crash_attempt_id)
            .unwrap()
            .status,
        AttemptStatus::Failed
    );

    // A forged active Turn may not point into the already checkpointed
    // conversation prefix: preflight must reject before any recovery write.
    let mut accepted_conversation = recovered_conversation;
    let accepted_node =
        accepted_conversation.append_message(Role::User, "accepted prefix fixture".into());
    storyforge_tauri_app::sqlite_runtime::save_conversation(&accepted_conversation).unwrap();
    checkpoint.sqlite_authority = Some(capture_sqlite_authority_binding(&checkpoint).unwrap());
    let forged = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        accepted_node,
        campaign.revision,
    );
    let forged_id = forged.turn_id.clone();
    storyforge_tauri_app::sqlite_runtime::save_turn(&forged).unwrap();
    let before_rejected_preflight =
        storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert!(validate_live_sqlite_resume_preconditions(&checkpoint).is_err());
    let after_rejected_preflight =
        storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert_eq!(
        before_rejected_preflight.canonical_content_sha256,
        after_rejected_preflight.canonical_content_sha256,
        "unsafe anchor rejection must be zero-write"
    );
    assert_eq!(
        storyforge_tauri_app::sqlite_runtime::get_turn(&forged_id)
            .unwrap()
            .unwrap()
            .status,
        TurnStatus::Generating
    );
    storyforge_tauri_app::sqlite_runtime::recover_turns_on_startup().unwrap();

    // Even with a valid recoverable tail, unrelated accepted-state drift must
    // be detected after recovery and before a checkpoint can be rebound.
    let original_campaign = storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
        .unwrap()
        .unwrap();
    checkpoint.sqlite_authority = Some(capture_sqlite_authority_binding(&checkpoint).unwrap());
    let mut drifted_campaign = original_campaign.clone();
    drifted_campaign.name.push_str("-drift");
    storyforge_tauri_app::sqlite_runtime::save_campaign(&drifted_campaign).unwrap();
    let mut active_conversation =
        storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .unwrap();
    let safe_input =
        active_conversation.append_message(Role::User, "recoverable tail fixture".into());
    storyforge_tauri_app::sqlite_runtime::save_conversation(&active_conversation).unwrap();
    let safe_active = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        safe_input,
        original_campaign.revision,
    );
    storyforge_tauri_app::sqlite_runtime::save_turn(&safe_active).unwrap();
    validate_live_sqlite_resume_preconditions(&checkpoint).unwrap();
    env.recover_checkpoint_validated_incomplete_turn(&campaign_id)
        .unwrap();
    assert!(
        validate_live_sqlite_resume_authority(&checkpoint).is_err(),
        "post-recovery validation must expose unrelated campaign drift"
    );
    storyforge_tauri_app::sqlite_runtime::save_campaign(&original_campaign).unwrap();
    validate_live_sqlite_resume_authority(&checkpoint).unwrap();

    // The global startup recovery primitive must never be reached when more
    // than one active Turn exists.
    let missing_input_a = storyforge_domain::Id::new();
    let missing_input_b = storyforge_domain::Id::new();
    storyforge_tauri_app::sqlite_runtime::save_turn(&TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        missing_input_a,
        original_campaign.revision,
    ))
    .unwrap();
    let mut foreign_campaign = storyforge_domain::campaign::Campaign::new(
        original_campaign.card_id.clone(),
        "foreign active fixture",
    );
    let foreign_conversation = Conversation::new(None, Some(foreign_campaign.id.clone()));
    foreign_campaign.conversation_id = Some(foreign_conversation.id.clone());
    storyforge_tauri_app::sqlite_runtime::save_campaign(&foreign_campaign).unwrap();
    storyforge_tauri_app::sqlite_runtime::save_conversation(&foreign_conversation).unwrap();
    storyforge_tauri_app::sqlite_runtime::save_turn(&TurnRecord::new(
        foreign_campaign.id,
        foreign_conversation.id,
        missing_input_b,
        foreign_campaign.revision,
    ))
    .unwrap();
    let before_multiple = storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert!(validate_live_sqlite_resume_preconditions(&checkpoint).is_err());
    assert!(env.recover_incomplete_turn_for_retry(&campaign_id).is_err());
    let after_multiple = storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot().unwrap();
    assert_eq!(
        before_multiple.canonical_content_sha256, after_multiple.canonical_content_sha256,
        "multiple-active rejection must be zero-write"
    );
    storyforge_tauri_app::sqlite_runtime::recover_turns_on_startup().unwrap();

    assert_context_epoch_fill_rebinding(&original_campaign.card_id);
    assert_turn_audit_rebinding(&original_campaign.card_id);
}

#[test]
fn resume_identity_rejects_stage_or_runtime_drift() {
    let expected = EnduranceRunIdentity {
        fixture_hash16: "fixture123456789".into(),
        model_sha256: "b".repeat(64),
        endpoint_hash16: "endpoint12345678".into(),
        provider_extra_hash16: "extra1234567890".into(),
        tool_mode: "native".into(),
        reasoning_mode: "disabled".into(),
        code_revision16: "revision1234567".into(),
        supplemental_matrix: true,
        meta_probe: true,
        character_extractor_probe: true,
        cache_probe: true,
    };
    let mut checkpoint = EnduranceCheckpoint {
        schema_version: EnduranceCheckpoint::schema_version().into(),
        run_id: "run-coverage-00000000-0000-0000-0000-000000000001".into(),
        stage: "coverage".into(),
        accepted_turn_number: 1,
        calls_used: 1,
        max_calls: 220,
        campaign_revision: 1,
        chronicle_revision: 1,
        last_draft_hash16: "draft12345678901".into(),
        last_summary_code: None,
        context_epoch_id16: None,
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        campaign_id: None,
        conversation_id: None,
        data_dir_rel: Some("campaign_data".into()),
        observed_epoch_ids16: vec![],
        run_identity: Some(expected.clone()),
        retry_state: None,
        probe_state: EnduranceProbeState::default(),
        sqlite_authority: None,
        recorded_at_unix_ms: 0,
    };
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_ok());
    checkpoint.stage = "canary".into();
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_err());
    checkpoint.stage = "coverage".into();
    checkpoint.run_identity.as_mut().unwrap().tool_mode = "text_fallback".into();
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_err());
    checkpoint.run_identity.as_mut().unwrap().tool_mode = "native".into();
    checkpoint.run_identity.as_mut().unwrap().code_revision16 = "different123456".into();
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_err());
    checkpoint.run_identity.as_mut().unwrap().code_revision16 = expected.code_revision16.clone();
    checkpoint.run_identity.as_mut().unwrap().endpoint_hash16 = "different-endpt".into();
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_err());
    checkpoint.run_identity.as_mut().unwrap().endpoint_hash16 = expected.endpoint_hash16.clone();
    checkpoint
        .run_identity
        .as_mut()
        .unwrap()
        .provider_extra_hash16 = "different-extra".into();
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_err());

    let authority = EnduranceSqliteAuthority {
        accepted_content_sha256: "a".repeat(64),
        committed_turns: 1,
        accepted_conversation_nodes: 2,
        accepted_conversation_sha256: "c".repeat(64),
    };
    checkpoint.sqlite_authority = Some(authority.clone());
    assert!(validate_sqlite_authority_values(&checkpoint, 1, 1, &authority).is_ok());
    // Monotone chronicle advance after Accept/context-fill is allowed.
    assert!(validate_sqlite_authority_values(&checkpoint, 1, 2, &authority).is_ok());
    assert!(validate_sqlite_authority_values(&checkpoint, 2, 1, &authority).is_err());
    // Chronicle must never regress.
    assert!(validate_sqlite_authority_values(&checkpoint, 1, 0, &authority).is_err());
    let drifted = EnduranceSqliteAuthority {
        accepted_content_sha256: "b".repeat(64),
        ..authority
    };
    // Same chronicle with opaque payload drift is still fail-closed.
    assert!(validate_sqlite_authority_values(&checkpoint, 1, 1, &drifted).is_err());
    // Context-epoch / chronicle fill rewrites campaign payload (accepted_content hash)
    // without accepting turns; tolerate only when conversation binding is stable and
    // chronicle advanced.
    assert!(validate_sqlite_authority_values(&checkpoint, 1, 2, &drifted).is_ok());
    let conversation_drifted = EnduranceSqliteAuthority {
        accepted_conversation_sha256: "d".repeat(64),
        ..authority
    };
    assert!(validate_sqlite_authority_values(&checkpoint, 1, 2, &conversation_drifted).is_err());
}

fn create_isolated_sqlite_campaign(
    card_id: &storyforge_domain::Id,
    name: &str,
) -> (storyforge_domain::campaign::Campaign, Conversation) {
    let mut campaign = storyforge_domain::campaign::Campaign::new(card_id.clone(), name);
    let conversation = Conversation::new(None, Some(campaign.id.clone()));
    campaign.conversation_id = Some(conversation.id.clone());
    storyforge_tauri_app::sqlite_runtime::save_campaign(&campaign).unwrap();
    storyforge_tauri_app::sqlite_runtime::save_conversation(&conversation).unwrap();
    (campaign, conversation)
}

fn assert_context_epoch_fill_rebinding(card_id: &storyforge_domain::Id) {
    // Reproduces the live fail-closed path:
    // fill_campaign_runtime_from_sqlite bumps chronicle/context_epoch → accepted_content
    // hash changes → failed-attempt recovery must still rebind authority.
    let (campaign, conversation) =
        create_isolated_sqlite_campaign(card_id, "context epoch authority fixture");
    let campaign_id = campaign.id.clone();
    let conversation_id = conversation.id.clone();
    let mut checkpoint = EnduranceCheckpoint {
        schema_version: EnduranceCheckpoint::schema_version().into(),
        run_id: "run-coverage-00000000-0000-0000-0000-000000000654".into(),
        stage: "coverage".into(),
        accepted_turn_number: 0,
        calls_used: 0,
        max_calls: 220,
        campaign_revision: campaign.revision,
        chronicle_revision: campaign.chronicle_revision,
        last_draft_hash16: String::new(),
        last_summary_code: None,
        context_epoch_id16: None,
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        campaign_id: Some(campaign_id.as_str().into()),
        conversation_id: Some(conversation_id.as_str().into()),
        data_dir_rel: Some("campaign_data".into()),
        observed_epoch_ids16: vec![],
        run_identity: None,
        retry_state: None,
        probe_state: EnduranceProbeState::default(),
        sqlite_authority: None,
        recorded_at_unix_ms: 0,
    };
    checkpoint.sqlite_authority = Some(capture_sqlite_authority_binding(&checkpoint).unwrap());
    let before = checkpoint.sqlite_authority.clone().unwrap();

    // Production fill path may persist epoch + chronicle before Accept.
    let mut filled = campaign.clone();
    filled.bump_chronicle_revision();
    filled.name.push_str("-epoch-refresh");
    storyforge_tauri_app::sqlite_runtime::save_campaign(&filled).unwrap();

    let after = capture_sqlite_authority_binding(&checkpoint).unwrap();
    assert_ne!(
        before.accepted_content_sha256, after.accepted_content_sha256,
        "context fill must change accepted_content projection via campaign payload"
    );
    assert_eq!(
        before.accepted_conversation_sha256,
        after.accepted_conversation_sha256
    );
    assert_eq!(before.committed_turns, after.committed_turns);

    // Continuous turn start / failed-attempt rebind must not fail closed on this.
    validate_live_sqlite_resume_authority(&checkpoint).unwrap();
    validate_continuous_turn_start_authority(&checkpoint, 1).unwrap();

    // Unrelated conversation drift remains fail-closed.
    let mut conversation = storyforge_tauri_app::sqlite_runtime::get_conversation(&conversation_id)
        .unwrap()
        .unwrap();
    conversation.append_message(Role::User, "forged accepted prefix".into());
    storyforge_tauri_app::sqlite_runtime::save_conversation(&conversation).unwrap();
    assert!(validate_live_sqlite_resume_authority(&checkpoint).is_err());
}

fn assert_turn_audit_rebinding(card_id: &storyforge_domain::Id) {
    let dir = std::env::temp_dir().join(format!(
        "sf-sqlite-chronicle-advance-{}",
        uuid::Uuid::new_v4()
    ));
    let (campaign, conversation) =
        create_isolated_sqlite_campaign(card_id, "turn audit authority fixture");
    let campaign_id = campaign.id.clone();
    let conversation_id = conversation.id.clone();
    let mut checkpoint = EnduranceCheckpoint {
        schema_version: EnduranceCheckpoint::schema_version().into(),
        run_id: "run-coverage-00000000-0000-0000-0000-000000000321".into(),
        stage: "coverage".into(),
        accepted_turn_number: 0,
        calls_used: 0,
        max_calls: 220,
        campaign_revision: campaign.revision,
        chronicle_revision: campaign.chronicle_revision,
        last_draft_hash16: String::new(),
        last_summary_code: None,
        context_epoch_id16: None,
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        campaign_id: Some(campaign_id.as_str().into()),
        conversation_id: Some(conversation_id.as_str().into()),
        data_dir_rel: Some("campaign_data".into()),
        observed_epoch_ids16: vec![],
        run_identity: None,
        retry_state: None,
        probe_state: EnduranceProbeState::default(),
        sqlite_authority: None,
        recorded_at_unix_ms: 0,
    };
    checkpoint.sqlite_authority = Some(capture_sqlite_authority_binding(&checkpoint).unwrap());

    let mut advanced = campaign.clone();
    advanced.bump_chronicle_revision();
    storyforge_tauri_app::sqlite_runtime::save_campaign(&advanced).unwrap();
    assert!(
        advanced.chronicle_revision > checkpoint.chronicle_revision,
        "fixture must actually advance chronicle"
    );

    // Capture against an unbound checkpoint is allowed when only chronicle advanced.
    let mut unbound = checkpoint.clone();
    unbound.sqlite_authority = None;
    unbound.chronicle_revision = advanced.chronicle_revision;
    let actual = capture_sqlite_authority_binding(&unbound).unwrap();
    assert_eq!(actual.committed_turns, 0);
    unbound.sqlite_authority = Some(actual.clone());
    validate_sqlite_authority_values(
        &unbound,
        advanced.revision,
        advanced.chronicle_revision,
        &actual,
    )
    .unwrap();

    // Turn-audit checkpoints are rebound with sqlite_authority=None, same as production.
    let paths = EnduranceEvidencePaths::new(dir.clone());
    std::fs::create_dir_all(&paths.root).unwrap();
    let mut turn_audit = checkpoint.clone();
    turn_audit.sqlite_authority = None;
    persist_checkpoint_and_integrity(&paths, &turn_audit, "turn-audit").unwrap();
    let rebound = read_latest_checkpoint(&paths.checkpoint_jsonl).unwrap();
    assert_eq!(rebound.campaign_revision, advanced.revision);
    assert_eq!(rebound.chronicle_revision, advanced.chronicle_revision);
    assert!(rebound.sqlite_authority.is_some());
    validate_live_sqlite_resume_authority(&rebound).unwrap();
}

#[test]
fn model_identity_hashes_the_full_name_not_only_the_display_prefix() {
    let prefix = "m".repeat(64);
    let first = format!("{prefix}-first");
    let second = format!("{prefix}-second");
    assert_eq!(
        first.chars().take(64).collect::<String>(),
        second.chars().take(64).collect::<String>()
    );
    assert_ne!(
        model_identity_sha256(&first),
        model_identity_sha256(&second)
    );
}

#[tokio::test]
#[ignore = "requires STORYFORGE_EVAL_REAL_LLM=1 and LLM credentials; default skip"]
async fn endurance_sqlite_real_llm_staged() {
    // Document storage backend for operators; this test always uses SqliteHarnessEnv.
    unsafe {
        std::env::set_var("STORYFORGE_STORAGE_BACKEND", "sqlite");
    }
    let budget = require_endurance_budget();
    let stage = parse_target_stage();
    let initial_git = resolve_git_provenance()
        .unwrap_or_else(|error| panic!("git provenance unavailable: {error}"));

    let (run_id_for_log, dir, paths) = evidence_run_dir(stage);
    eprintln!(
        "[sqlite endurance] evidence run_id={} dir_name={}",
        run_id_for_log,
        dir.file_name().and_then(|s| s.to_str()).unwrap_or("<run>")
    );
    let data_dir = dir.join("campaign_data");

    let resume_cp = if paths.checkpoint_jsonl.exists() {
        match resume_from_evidence_dir(&paths.root, Some(run_id_for_log.as_str())) {
            Ok((_, cp)) => Some(cp),
            Err(err) => panic!("fail-closed resume preflight: {err}"),
        }
    } else {
        None
    };

    if let Some(checkpoint) = resume_cp.as_ref() {
        let recorded = harness_real_llm::evidence_retention::count_budgeted_call_records(
            &paths.root,
            &checkpoint.run_id,
        )
        .unwrap_or_else(|error| panic!("fail-closed pre-recovery call ledger: {error}"));
        assert_eq!(
            recorded, checkpoint.calls_used,
            "call ledger must match checkpoint before any SQLite recovery write"
        );
    }

    let mut budget = budget;
    budget.max_turns = stage.target_turns();
    budget.max_calls = stage.max_calls();
    let prior_calls = resume_cp
        .as_ref()
        .map(|checkpoint| checkpoint.calls_used)
        .unwrap_or(0);
    if let Some(cp) = resume_cp.as_ref() {
        assert_eq!(
            cp.max_calls,
            stage.max_calls(),
            "resume checkpoint call-accounting policy changed"
        );
        eprintln!(
            "[sqlite endurance] resume accounting: prior_calls={} max_calls={}",
            cp.calls_used,
            stage.max_calls()
        );
    }
    let connection = resolve_llm_connection()
        .unwrap_or_else(|error| panic!("real LLM connection unavailable: {error}"));
    let model_label = connection.model.chars().take(64).collect::<String>();
    let reasoning_override = parse_eval_reasoning_mode();
    let runtime_profile = RuntimeCoverageProfile {
        reasoning_mode: match &reasoning_override {
            ReasoningMode::Disabled => ReasoningModeSlot::Disabled,
            ReasoningMode::Native => ReasoningModeSlot::Native,
            ReasoningMode::Prompted => ReasoningModeSlot::Prompted,
        },
        tool_mode: match &connection.tool_mode {
            ToolMode::Native => ToolModeSlot::Native,
            ToolMode::TextFallback => ToolModeSlot::TextFallback,
        },
    };
    let run_identity = EnduranceRunIdentity {
        fixture_hash16: fixture_source_hash16()
            .unwrap_or_else(|error| panic!("fixture identity unavailable: {error}")),
        model_sha256: model_identity_sha256(&connection.model),
        endpoint_hash16: endpoint_identity_hash16(&connection.base_url, &connection.protocol),
        provider_extra_hash16: provider_extra_hash16(connection.params.extra.as_ref()),
        tool_mode: runtime_profile.tool_mode.label().into(),
        reasoning_mode: runtime_profile.reasoning_mode.label().into(),
        code_revision16: short_hash16(&initial_git.commit),
        supplemental_matrix: env_flag("STORYFORGE_EVAL_SUPPLEMENTAL_MATRIX"),
        meta_probe: env_flag("STORYFORGE_EVAL_META_PROBE"),
        character_extractor_probe: env_flag("STORYFORGE_EVAL_CHARACTER_EXTRACTOR_PROBE"),
        cache_probe: env_flag("STORYFORGE_EVAL_CACHE_PROBE"),
    };
    if let Some(checkpoint) = resume_cp.as_ref() {
        validate_resume_identity(checkpoint, stage, &run_identity)
            .unwrap_or_else(|error| panic!("fail-closed resume identity: {error}"));
    }
    let reservation_writer = if resume_cp.is_some() {
        harness_real_llm::evidence_retention::DurableCallReservationWriter::open_append(
            &paths.root,
            run_id_for_log.clone(),
        )
    } else {
        harness_real_llm::evidence_retention::DurableCallReservationWriter::create(
            &paths.root,
            run_id_for_log.clone(),
        )
    }
    .unwrap_or_else(|error| panic!("durable call reservation ledger unavailable: {error}"));
    let real_client = storyforge_infra_llm::create_client(&connection)
        .unwrap_or_else(|error| panic!("construct real LLM client: {error}"));
    let llm = BudgetedLlmClient::wrap_with_reasoning_provider_extra_and_reservations(
        Arc::from(real_client),
        &budget,
        Some(reasoning_override.clone()),
        connection.params.extra.clone(),
        prior_calls,
        run_id_for_log.clone(),
        Arc::new(reservation_writer),
    )
    .unwrap_or_else(|error| panic!("durable budgeted client unavailable: {error}"));

    let mut env = if resume_cp.is_some() && data_dir.join("storyforge.sqlite3").exists() {
        let env =
            SqliteHarnessEnv::open_existing(data_dir.clone(), llm.clone() as Arc<dyn LlmClient>)
                .unwrap_or_else(|e| panic!("sqlite open_existing: {e}"));
        if let Some(cid) = resume_cp.as_ref().and_then(|c| c.campaign_id.as_ref()) {
            env.set_active_campaign(storyforge_domain::Id::from_str(cid));
        }
        env
    } else {
        SqliteHarnessEnv::bootstrap(data_dir.clone(), llm.clone() as Arc<dyn LlmClient>)
            .unwrap_or_else(|e| panic!("sqlite bootstrap: {e}"))
    };
    // Preserve connection-level provider extensions (for example `thinking`) while
    // applying the arm reasoning mode used by prompt assembly and capture checks.
    env.set_pipeline_sampling(Some(eval_pipeline_sampling(
        &connection.params,
        reasoning_override,
    )));

    let campaign_id = if let Some(cp) = resume_cp.as_ref().and_then(|c| c.campaign_id.as_ref()) {
        let id = storyforge_domain::Id::from_str(cp);
        env.set_active_campaign(id.clone());
        id
    } else {
        env.active_campaign_id()
            .expect("sqlite bootstrap sets active campaign")
    };
    if let Some(checkpoint) = resume_cp.as_ref() {
        validate_live_sqlite_resume_preconditions(checkpoint)
            .unwrap_or_else(|error| panic!("fail-closed SQLite authority preflight: {error}"));
        env.recover_checkpoint_validated_incomplete_turn(&campaign_id)
            .unwrap_or_else(|error| panic!("sqlite retry recovery: {error}"));
        validate_live_sqlite_resume_authority(checkpoint).unwrap_or_else(|error| {
            panic!("fail-closed SQLite authority after retry recovery: {error}")
        });
        let reconciled =
            harness_real_llm::evidence_retention::reconcile_dangling_call_reservations(
                &paths.root,
                &checkpoint.run_id,
                &model_label,
            )
            .unwrap_or_else(|error| panic!("reconcile interrupted provider calls: {error}"));
        assert_eq!(
            reconciled, checkpoint.calls_used,
            "resume call reservations changed after preflight"
        );
        let mut refreshed = read_latest_checkpoint(&paths.checkpoint_jsonl)
            .expect("resume checkpoint remains readable after SQLite recovery");
        refreshed.calls_used = reconciled;
        refreshed.max_calls = stage.max_calls();
        persist_checkpoint_and_integrity(&paths, &refreshed, "post-recovery")
            .unwrap_or_else(|error| panic!("refresh post-recovery SQLite binding: {error}"));
    }
    let conversation_id =
        if let Some(cp) = resume_cp.as_ref().and_then(|c| c.conversation_id.as_ref()) {
            storyforge_domain::Id::from_str(cp)
        } else {
            storyforge_tauri_app::sqlite_runtime::get_campaign(&campaign_id)
                .expect("get campaign")
                .expect("campaign present")
                .conversation_id
                .expect("conversation bound on campaign")
        };

    match run_sqlite_endurance_stage(SqliteEnduranceStageContext {
        env: &env,
        llm: llm.clone(),
        campaign_id,
        conversation_id,
        stage,
        budget: &budget,
        call_limit: stage.max_calls(),
        paths: &paths,
        runtime_profile,
        model_label: &model_label,
        run_identity,
    })
    .await
    {
        Ok(row) => {
            eprintln!(
                "SQLITE ENDURANCE {} PASS: turns={}/{}, calls={}/{}, acceptance={}, fixture={}",
                row.stage,
                row.accepted_turns,
                row.target_turns,
                row.calls_used,
                row.max_calls,
                row.acceptance,
                env.fixture_hash16
            );
            let audit = storyforge_tauri_app::sqlite_runtime::capture_audit_snapshot()
                .unwrap_or_else(|error| panic!("capture consistent SQLite audit: {error}"));
            assert_eq!(
                audit.committed_turns,
                u64::from(row.accepted_turns),
                "SQLite committed-turn count must match the accepted evidence row"
            );
            harness_real_llm::evidence_retention::write_sqlite_audit_subject(
                &paths.root,
                &harness_real_llm::evidence_retention::SqliteAuditSubject {
                    schema_version:
                        harness_real_llm::evidence_retention::SQLITE_AUDIT_SCHEMA_VERSION.into(),
                    run_id: row.run_id.clone(),
                    sqlite_schema_version: audit.sqlite_schema_version,
                    canonical_content_sha256: audit.canonical_content_sha256,
                    accepted_content_sha256: audit.accepted_content_sha256,
                    turns: audit.turns,
                    attempts: audit.attempts,
                    committed_turns: audit.committed_turns,
                    outbox_rows: audit.outbox_rows,
                    round_summaries: audit.round_summaries,
                    publication_jobs: audit.publication_jobs,
                    ledger_entries: audit.ledger_entries,
                },
            )
            .unwrap_or_else(|error| panic!("write SQLite audit subject: {error}"));

            assert!(paths.check_no_secrets().is_ok());
            assert!(paths.total_size() < 2 * 1024 * 1024);

            let final_git = resolve_git_provenance()
                .unwrap_or_else(|error| panic!("final git provenance unavailable: {error}"));
            assert_eq!(
                final_git, initial_git,
                "git provenance changed while the evidence run was active"
            );

            let manifest = harness_real_llm::evidence_retention::seal_run(
                &paths.root,
                harness_real_llm::evidence_retention::SealOptions {
                    run_id: row.run_id.clone(),
                    status: harness_real_llm::evidence_retention::RunStatus::Completed,
                    stage: row.stage.clone(),
                    model_label: connection.model.clone(),
                    budget: harness_real_llm::evidence_retention::BudgetSummary {
                        max_calls: row.max_calls,
                        max_turns: row.target_turns,
                        timeout_secs: budget.timeout_secs,
                        max_tokens: budget.max_tokens,
                    },
                    commit: final_git.commit,
                    branch: final_git.branch,
                },
            )
            .unwrap_or_else(|err| {
                panic!(
                    "{}",
                    harness_real_llm::evidence_retention::format_seal_hard_error(&err)
                )
            });
            eprintln!(
                "[sqlite endurance] sealed files={} status=completed",
                manifest.files.len()
            );
            harness_real_llm::evidence_retention::verify_run(&paths.root).unwrap_or_else(|err| {
                panic!(
                    "{}",
                    harness_real_llm::evidence_retention::format_seal_hard_error(&err)
                )
            });
        }
        Err(EnduranceError::CheckpointStop { turn_index }) => {
            eprintln!(
                "SQLITE ENDURANCE {} CHECKPOINT STOP: accepted_turn={turn_index}",
                stage.label()
            );
        }
        Err(err) => {
            let safe = safe_error_summary(&err.to_string());
            eprintln!("SQLITE ENDURANCE {} FAIL CLOSED: {safe}", stage.label());
            panic!("sqlite endurance stage {stage} failed closed: {safe}");
        }
    }
}

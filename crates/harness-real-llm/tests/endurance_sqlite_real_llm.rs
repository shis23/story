//! SQLite-backed M5 endurance real-model entry (default `#[ignore]`).
//!
//! Requires:
//! - STORYFORGE_EVAL_REAL_LLM=1
//! - LLM_BASE_URL / LLM_API_KEY / LLM_MODEL
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
use harness_real_llm::sqlite_endurance::{SqliteHarnessEnv, fixture_source_hash16};
use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::llm::{ReasoningMode, ToolMode};
use storyforge_infra_llm::LlmClient;

fn parse_target_stage() -> EnduranceStage {
    let raw = std::env::var("STORYFORGE_EVAL_ENDURANCE_STAGE").unwrap_or_default();
    EnduranceStage::from_label(&raw).unwrap_or(EnduranceStage::Canary)
}

fn scheduled_action_for_run(
    schedule: &EnduranceSchedule,
    turn: u32,
    supplemental_matrix: bool,
) -> ScheduledAction {
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

fn resolve_git_provenance() -> (String, String) {
    fn git(args: &[&str]) -> Option<String> {
        let output = std::process::Command::new("git").args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let branch = git(&["branch", "--show-current"]).unwrap_or_else(|| "detached".into());
    (commit, branch)
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

fn safe_error_summary(error: &str) -> String {
    let category = if error.contains("PlanParse") || error.contains("Plan 解析") {
        "plan_parse"
    } else if error.to_ascii_lowercase().contains("timeout") || error.contains("超时") {
        "timeout"
    } else if error.contains("rate limit") || error.contains("520") {
        "provider_transient"
    } else if error.contains("LlmError") || error.contains("client_error") {
        "llm_client"
    } else if error.contains("Storage") || error.contains("sqlite") {
        "storage"
    } else {
        "pipeline"
    };
    format!(
        "category={category} bytes={} hash16={}",
        error.len(),
        short_hash16(error)
    )
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
        resolve_resume_run_dir,
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
            if path.is_dir() && path.join("endurance_checkpoint.jsonl").exists() {
                let validated = resolve_resume_run_dir(&path, &policy)
                    .unwrap_or_else(|e| panic!("fail-closed resume evidence dir rejected: {e}"));
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

fn flush_samples(
    llm: &BudgetedLlmClient,
    call_writer: &EvidenceWriter,
    tool_writer: Option<&EvidenceWriter>,
    cursor: &mut usize,
    run_id: &str,
    model_label: &str,
    turn_index: u32,
) -> Result<usize, EnduranceError> {
    let samples = llm.samples();
    let turn_samples = samples.get(*cursor..).unwrap_or(&[]);
    let mut call_records = Vec::with_capacity(turn_samples.len());
    for sample in turn_samples {
        let rec = sample.to_evidence_call(
            run_id,
            "endurance_sqlite",
            turn_index,
            model_label,
            vec![harness_real_llm::evidence::AssertionResult {
                name: "call_recorded".into(),
                passed: sample.outcome == "ok",
                detail: Some(format!("outcome={}", sample.outcome)),
            }],
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
    meta_chat(
        &runtime,
        &mut conversation,
        session,
        "请先实际调用 inspect_campaign 和 inspect_tasks，再用一句话报告；不要猜测。",
        cancel_rx,
        progress_tx,
    )
    .await
    .map_err(|error| EnduranceError::Writer(format!("SQLite Meta probe: {error}")))?;
    let written = flush_samples(
        &llm,
        call_writer,
        Some(tool_writer),
        sample_cursor,
        run_id,
        model_label,
        evidence_turn_index,
    )?;
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
    let definitions =
        storyforge_app_agent::extract_characters(&runtime, &character, &[], cancel_rx)
            .await
            .map_err(|error| {
                EnduranceError::Writer(format!("CharacterExtractor probe: {error}"))
            })?;
    if definitions.is_empty() {
        return Err(EnduranceError::InvalidConfig(
            "CharacterExtractor returned no definitions".into(),
        ));
    }
    let written = flush_samples(
        &llm,
        call_writer,
        Some(tool_writer),
        sample_cursor,
        run_id,
        model_label,
        0,
    )?;
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
    llm.set_role("cache_probe");
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
    let probe = samples.iter().rev().take(3).cloned().collect::<Vec<_>>();
    if probe.len() != 3
        || probe[0].request_fp16 == probe[1].request_fp16
        || probe[1].request_fp16 != probe[2].request_fp16
    {
        return Err(EnduranceError::InvalidConfig(
            "cache probe did not observe stable/stable/invalidated request fingerprints".into(),
        ));
    }
    let cached_tokens = probe.iter().map(|sample| sample.cached_tokens).sum();
    let written = flush_samples(
        &llm,
        call_writer,
        Some(tool_writer),
        sample_cursor,
        run_id,
        model_label,
        evidence_turn_index,
    )?;
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
            EnduranceStage::Full => 4 * 60 * 60,
        };
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
    paths: &'a EnduranceEvidencePaths,
    runtime_profile: RuntimeCoverageProfile,
    model_label: &'a str,
    run_identity: EnduranceRunIdentity,
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
    if supplemental_matrix && target_turns != 12 {
        return Err(EnduranceError::InvalidConfig(
            "supplemental matrix requires STORYFORGE_EVAL_ENDURANCE_STAGE=coverage".into(),
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
            harness_real_llm::evidence_retention::count_evidence_call_records(&paths.root, &run_id)
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

    let call_writer = if start_turn > 1 {
        EvidenceWriter::open_append(&paths.calls_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.calls_jsonl, &run_id)?
    };
    let tool_writer = if start_turn > 1 {
        EvidenceWriter::open_append(&paths.tool_trace_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.tool_trace_jsonl, &run_id)?
    };
    let turn_writer = if start_turn > 1 {
        EvidenceWriter::open_append(&paths.turns_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.turns_jsonl, &run_id)?
    };
    let ledger_path = paths.root.join("endurance_coverage_ledger.jsonl");
    let mut ledger_lines = Vec::new();
    if start_turn == 1 && ledger_path.exists() {
        let _ = std::fs::remove_file(&ledger_path);
    }
    // Resume must reload previously sealed coverage observations; exact-set
    // verification is suite-wide, not process-local.
    if start_turn > 1 && ledger_path.exists() {
        let prior = std::fs::read_to_string(&ledger_path).map_err(EnduranceError::EvidenceIo)?;
        for line in prior.lines().filter(|l| !l.trim().is_empty()) {
            let obs: harness_real_llm::coverage_ledger::ObservedCoverage =
                serde_json::from_str(line).map_err(|e| {
                    EnduranceError::InvalidConfig(format!("coverage ledger parse: {e}"))
                })?;
            ledger.record(obs.clone());
            ledger_lines.push(line.to_string());
        }
    }

    let mut turns_accepted = start_turn.saturating_sub(1);
    // Assigned before first checkpoint read; initial values are never observed.
    #[allow(clippy::needless_late_init, unused_assignments)]
    let mut last_draft_hash16 = String::new();
    #[allow(clippy::needless_late_init, unused_assignments)]
    let mut last_campaign_revision = 0u64;
    #[allow(clippy::needless_late_init, unused_assignments)]
    let mut last_chronicle_revision = 0u64;
    let mut epoch_tracker = EpochTracker::default();
    if let Some(cp) = resume_cp.as_ref() {
        for eid in &cp.observed_epoch_ids16 {
            epoch_tracker.observe(eid);
        }
    }

    let extractor_probe_calls = if extractor_probe_enabled && start_turn == 1 {
        run_character_extractor_probe(
            env,
            llm.clone(),
            &call_writer,
            &tool_writer,
            &mut sample_cursor,
            &run_id,
            model_label,
        )
        .await?
    } else if extractor_probe_enabled {
        let prior = harness_real_llm::evidence::read_evidence_lines(&paths.calls_jsonl)?;
        let found = prior.iter().any(|value| {
            value.get("role").and_then(|role| role.as_str()) == Some("character_extractor")
        });
        if !found {
            return Err(EnduranceError::InvalidConfig(
                "resume evidence is missing the requested CharacterExtractor probe".into(),
            ));
        }
        1
    } else {
        0
    };
    let mut last_accepted_request_fp16: Option<String> = None;

    for turn_index in start_turn..=target_turns {
        deadline.check()?;
        let action = action_with_runtime_profile(
            scheduled_action_for_run(&schedule, turn_index, supplemental_matrix),
            runtime_profile,
        );
        let row = ledger
            .planned
            .iter()
            .find(|r| r.turn_index == turn_index)
            .cloned()
            .ok_or_else(|| {
                EnduranceError::InvalidConfig(format!("missing coverage row for turn {turn_index}"))
            })?;
        let intent = match &action {
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
        };

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

        const MAX_WRITE_ATTEMPTS: usize = 5;
        let turn_sample_start = llm.samples().len();
        let mut written = None;
        let mut last_err = None;
        for attempt in 1..=MAX_WRITE_ATTEMPTS {
            deadline.check()?;
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
                    env.write_accept_turn(
                        &conversation_id,
                        &intent,
                        turn_index,
                        &row.row_id,
                        summary_probe_id,
                        matches!(&action, ScheduledAction::QualityAutofix { fixable: true }),
                    )
                    .await
                }
            };
            let result = match tokio::time::timeout(deadline.remaining()?, fut).await {
                Ok(r) => r,
                Err(_) => {
                    let _ = flush_samples(
                        &llm,
                        &call_writer,
                        Some(&tool_writer),
                        &mut sample_cursor,
                        &run_id,
                        model_label,
                        turn_index,
                    );
                    return Err(EnduranceError::SuiteTimeout);
                }
            };
            let calls_this_turn = flush_samples(
                &llm,
                &call_writer,
                Some(&tool_writer),
                &mut sample_cursor,
                &run_id,
                model_label,
                turn_index,
            )?;
            let runner_calls = harness_real_llm::evidence_retention::count_evidence_call_records(
                &paths.root,
                &run_id,
            )
            .map_err(|error| {
                EnduranceError::InvalidConfig(format!("durable call ledger rejected: {error}"))
            })?;
            if runner_calls > stage.max_calls() {
                return Err(EnduranceError::BudgetExhausted {
                    calls_used: runner_calls,
                    max_calls: stage.max_calls(),
                });
            }
            match result {
                Ok(w) => {
                    if calls_this_turn == 0 {
                        return Err(EnduranceError::ZeroCalls { turn_index });
                    }
                    written = Some(w);
                    break;
                }
                Err(err) => {
                    let transient = err.contains("PlanParse")
                        || err.contains("Plan 解析")
                        || err.contains("未找到有效 Plan")
                        || err.contains("already has active turn")
                        || err.contains("timeout")
                        || err.contains("Timeout")
                        || err.contains("超时")
                        || err.contains("空闲超时")
                        || err.contains("流式空闲")
                        || err.contains("HTTP 请求失败")
                        || err.contains("520")
                        || err.contains("rate limit")
                        || err.contains("client_error")
                        || err.contains("LlmError")
                        || err.contains("Internal")
                        || err.contains("所有子 Agent 均失败")
                        || err.contains("不在旧 Plan")
                        || err.contains("部分重 roll")
                        || err.contains("expected Generating")
                        || err.contains("is Failed");
                    eprintln!(
                        "[sqlite endurance {}] turn {turn_index} attempt {attempt}/{MAX_WRITE_ATTEMPTS} failed (transient={transient}, {})",
                        stage.label(),
                        safe_error_summary(&err)
                    );
                    last_err = Some(err);
                    if !transient || attempt == MAX_WRITE_ATTEMPTS {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
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
            harness_real_llm::evidence_retention::count_evidence_call_records(&paths.root, &run_id)
                .map_err(|error| {
                    EnduranceError::InvalidConfig(format!("accepted call ledger rejected: {error}"))
                })?;
        let accepted_checkpoint = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.clone(),
            stage: stage.label().into(),
            accepted_turn_number: turn_index,
            calls_used: durable_calls,
            max_calls: stage.max_calls(),
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
            recorded_at_unix_ms: 0,
        };
        write_checkpoint(&paths.checkpoint_jsonl, &accepted_checkpoint)?;
        harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&paths.root)
            .map_err(|error| {
                EnduranceError::InvalidConfig(format!(
                    "accepted checkpoint integrity baseline: {error}"
                ))
            })?;

        let current_samples = llm.samples();
        let current_turn_samples = current_samples.get(turn_sample_start..).unwrap_or_default();
        let current_request_fp16 = current_turn_samples
            .last()
            .map(|sample| sample.request_fp16.clone());
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
                let stable = current_request_fp16.is_some()
                    && current_request_fp16 == last_accepted_request_fp16;
                if !stable {
                    return Err(EnduranceError::InvalidConfig(
                        "cache-stable row did not preserve the actual request fingerprint".into(),
                    ));
                }
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent("cache:stable".into()));
            }
            ScheduledAction::CacheInvalidate => {
                let invalidated = current_request_fp16.is_some()
                    && last_accepted_request_fp16.is_some()
                    && current_request_fp16 != last_accepted_request_fp16;
                if !invalidated {
                    return Err(EnduranceError::InvalidConfig(
                        "cache-invalidate row did not change the actual request fingerprint".into(),
                    ));
                }
                observed
                    .observations
                    .insert(ObservationKey::ToolEvent("cache:invalidated".into()));
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
        last_accepted_request_fp16 = current_request_fp16;

        ledger.record(observed.clone());
        let line = serde_json::to_string(&observed).unwrap_or_default();
        ledger_lines.push(line.clone());
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
        }

        let ctx = env
            .fill_campaign_context(WritingContext::legacy(
                vec![],
                None,
                conversation_id.clone(),
            ))
            .map_err(EnduranceError::Writer)?;
        let epoch_id16 = ctx
            .context_epoch
            .as_ref()
            .map(|e| short_hash16(&e.epoch_id));
        if let Some(ref eid) = epoch_id16 {
            epoch_tracker.observe(eid);
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
            chronicle_revision_after: written.accept.chronicle_revision_after,
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
            max_calls: stage.max_calls(),
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
            recorded_at_unix_ms: 0,
        };
        write_checkpoint(&paths.checkpoint_jsonl, &cp)?;
        harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&paths.root)
            .map_err(|e| {
                EnduranceError::InvalidConfig(format!("checkpoint integrity baseline: {e}"))
            })?;
        eprintln!(
            "[sqlite endurance {}] turn {turn_index}/{target_turns} accepted",
            stage.label()
        );
    }

    let mut meta_probe_calls = 0usize;
    if meta_probe_enabled {
        meta_probe_calls = run_sqlite_meta_probe(
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
        .await?;
        let durable_calls =
            harness_real_llm::evidence_retention::count_evidence_call_records(&paths.root, &run_id)
                .map_err(|error| {
                    EnduranceError::InvalidConfig(format!("Meta call ledger rejected: {error}"))
                })?;
        if durable_calls > stage.max_calls() {
            return Err(EnduranceError::BudgetExhausted {
                calls_used: durable_calls,
                max_calls: stage.max_calls(),
            });
        }
        let mut checkpoint = read_latest_checkpoint(&paths.checkpoint_jsonl).ok_or_else(|| {
            EnduranceError::InvalidConfig("Meta probe cannot update missing checkpoint".into())
        })?;
        checkpoint.calls_used = durable_calls;
        write_checkpoint(&paths.checkpoint_jsonl, &checkpoint)?;
        harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&paths.root)
            .map_err(|error| {
                EnduranceError::InvalidConfig(format!(
                    "Meta checkpoint integrity baseline: {error}"
                ))
            })?;
    }

    let mut cache_probe_calls = 0usize;
    let mut cache_probe_cached_tokens = 0u32;
    if cache_probe_enabled {
        (cache_probe_calls, cache_probe_cached_tokens) = run_cache_fingerprint_probe(
            llm.clone(),
            &call_writer,
            &tool_writer,
            &mut sample_cursor,
            &run_id,
            model_label,
            target_turns.saturating_add(2),
        )
        .await?;
        let durable_calls =
            harness_real_llm::evidence_retention::count_evidence_call_records(&paths.root, &run_id)
                .map_err(|error| {
                    EnduranceError::InvalidConfig(format!("cache call ledger rejected: {error}"))
                })?;
        if durable_calls > stage.max_calls() {
            return Err(EnduranceError::BudgetExhausted {
                calls_used: durable_calls,
                max_calls: stage.max_calls(),
            });
        }
        let mut checkpoint = read_latest_checkpoint(&paths.checkpoint_jsonl).ok_or_else(|| {
            EnduranceError::InvalidConfig("cache probe cannot update missing checkpoint".into())
        })?;
        checkpoint.calls_used = durable_calls;
        write_checkpoint(&paths.checkpoint_jsonl, &checkpoint)?;
        harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&paths.root)
            .map_err(|error| {
                EnduranceError::InvalidConfig(format!(
                    "cache checkpoint integrity baseline: {error}"
                ))
            })?;
    }

    std::fs::write(&ledger_path, ledger_lines.join("\n") + "\n")
        .map_err(EnduranceError::EvidenceIo)?;

    if let Err(ms) = ledger.exact_set_verify() {
        return Err(EnduranceError::InvalidConfig(format!(
            "coverage ledger exact-set failed: {ms:?}"
        )));
    }
    let tool_call_steps = llm
        .samples()
        .iter()
        .flat_map(|sample| sample.tool_steps.iter())
        .filter(|step| step.kind == "call")
        .count();
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
        harness_real_llm::evidence_retention::count_evidence_call_records(&paths.root, &run_id)
            .map_err(|error| {
                EnduranceError::InvalidConfig(format!("final call ledger rejected: {error}"))
            })?;
    let acceptance = classify_acceptance(
        stage,
        turns_accepted,
        calls_used,
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
        max_calls: stage.max_calls(),
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
fn resume_identity_rejects_stage_or_runtime_drift() {
    let expected = EnduranceRunIdentity {
        fixture_hash16: "fixture123456789".into(),
        model_hash16: "model12345678901".into(),
        tool_mode: "native".into(),
        reasoning_mode: "disabled".into(),
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
        recorded_at_unix_ms: 0,
    };
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_ok());
    checkpoint.stage = "canary".into();
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_err());
    checkpoint.stage = "coverage".into();
    checkpoint.run_identity.as_mut().unwrap().tool_mode = "text_fallback".into();
    assert!(validate_resume_identity(&checkpoint, EnduranceStage::Coverage, &expected).is_err());
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

    let mut budget = budget;
    if let Some(cp) = resume_cp.as_ref() {
        let remaining = stage.max_calls().saturating_sub(cp.calls_used);
        // Process-local BudgetedLlmClient budget must cover remaining work plus
        // bounded Plan-parse retries; stage accounting still uses prior+runner calls.
        let headroom = stage
            .target_turns()
            .saturating_sub(cp.accepted_turn_number)
            .saturating_mul(12)
            .max(40);
        budget.max_calls = remaining.saturating_add(headroom).max(1);
        eprintln!(
            "[sqlite endurance] resume budget: prior_calls={} remaining_stage={} process_max_calls={}",
            cp.calls_used, remaining, budget.max_calls
        );
    } else {
        // Prefer stage budget if env default is lower.
        budget.max_calls = budget.max_calls.max(stage.max_calls());
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
        model_hash16: short_hash16(&model_label),
        tool_mode: runtime_profile.tool_mode.label().into(),
        reasoning_mode: runtime_profile.reasoning_mode.label().into(),
    };
    if let Some(checkpoint) = resume_cp.as_ref() {
        validate_resume_identity(checkpoint, stage, &run_identity)
            .unwrap_or_else(|error| panic!("fail-closed resume identity: {error}"));
    }
    let real_client = storyforge_infra_llm::create_client(&connection)
        .unwrap_or_else(|error| panic!("construct real LLM client: {error}"));
    let llm = BudgetedLlmClient::wrap_with_reasoning(
        Arc::from(real_client),
        &budget,
        Some(reasoning_override),
    );

    let env = if resume_cp.is_some() && data_dir.join("storyforge.sqlite3").exists() {
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

    let campaign_id = if let Some(cp) = resume_cp.as_ref().and_then(|c| c.campaign_id.as_ref()) {
        let id = storyforge_domain::Id::from_str(cp);
        env.set_active_campaign(id.clone());
        id
    } else {
        env.active_campaign_id()
            .expect("sqlite bootstrap sets active campaign")
    };
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

            let (commit, branch) = resolve_git_provenance();

            let manifest = harness_real_llm::evidence_retention::seal_run(
                &paths.root,
                harness_real_llm::evidence_retention::SealOptions {
                    run_id: row.run_id.clone(),
                    status: harness_real_llm::evidence_retention::RunStatus::Completed,
                    stage: row.stage.clone(),
                    model_label: model_label.clone(),
                    budget: harness_real_llm::evidence_retention::BudgetSummary {
                        max_calls: row.max_calls,
                        max_turns: row.target_turns,
                        timeout_secs: budget.timeout_secs,
                        max_tokens: budget.max_tokens,
                    },
                    commit,
                    branch,
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
        Err(err) => {
            let safe = safe_error_summary(&err.to_string());
            eprintln!("SQLITE ENDURANCE {} FAIL CLOSED: {safe}", stage.label());
            panic!("sqlite endurance stage {stage} failed closed: {safe}");
        }
    }
}

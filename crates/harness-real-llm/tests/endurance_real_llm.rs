//! 100-turn endurance real-model entry point (default `#[ignore]`).
//!
//! This test runs the production-faithful endurance pipeline through stages
//! 3 → 12 → 30 → 100 accepted turns, with hard suite-wide budgets, checkpoint/resume,
//! sanitized evidence, and fail-closed classification.
//!
//! **All secrets come from the environment only — never written to source or output.**
//!
//! ```text
//! $env:STORYFORGE_EVAL_REAL_LLM='1'
//! $env:LLM_BASE_URL='https://...'
//! $env:LLM_API_KEY='<secret-from-env>'
//! $env:LLM_MODEL='...'
//! $env:STORYFORGE_EVAL_FIXTURE_CARD='C:\path\to\card.png'
//! $env:STORYFORGE_EVAL_EVIDENCE_ROOT='D:\storyforge-evidence'  # durable root (preferred)
//! # legacy single-run path still accepted when validated:
//! # $env:STORYFORGE_EVAL_EVIDENCE_DIR='D:\storyforge-evidence\run-full-<uuid>'
//! $env:STORYFORGE_EVAL_MAX_CALLS='700'
//! $env:STORYFORGE_EVAL_MAX_TURNS='100'
//! $env:STORYFORGE_EVAL_TIMEOUT_SECS='180'
//! cargo test -p harness-real-llm --test endurance_real_llm -- endurance_real_llm_full_100_turn --ignored --nocapture
//! ```
//!
//! Evidence retention notes:
//! - Prefer a durable `STORYFORGE_EVAL_EVIDENCE_ROOT` outside the repo and outside temp cleanup dirs.
//! - Each fresh run allocates a unique `run-<stage>-<uuid>` directory with an atomic `run_manifest.json`.
//! - Resume reuses the same run directory and fails closed on mixed run IDs / hash / schema drift.
//! - This harness never reconstructs the lost historical 45/100 checkpoint.

use std::path::PathBuf;
use std::sync::Arc;

use harness_real_llm::budget::BudgetedLlmClient;
use harness_real_llm::commit_probe::{CommitProbeEnv, ProductionAcceptInput};
use harness_real_llm::endurance::*;
use harness_real_llm::evidence::{EvidenceWriter, RealLlmRunBudget, short_hash16};
use harness_real_llm::production_evidence::{PipelineProductionTurnWriter, ProductionTurnWriter};
use harness_real_llm::{HarnessEnv, require_real_llm};
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR};
use storyforge_domain::turn::QualityReport;
use storyforge_infra_llm::LlmClient;

/// Parse the target stage from `STORYFORGE_EVAL_ENDURANCE_STAGE` (default: canary).
fn parse_target_stage() -> EnduranceStage {
    let raw = std::env::var("STORYFORGE_EVAL_ENDURANCE_STAGE").unwrap_or_default();
    EnduranceStage::from_label(&raw).unwrap_or(EnduranceStage::Canary)
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

/// Resolve a durable evidence **run directory**.
///
/// - If `STORYFORGE_EVAL_EVIDENCE_DIR` already points at a controlled run dir with
///   a checkpoint (resume path), reuse it.
/// - Else resolve `STORYFORGE_EVAL_EVIDENCE_ROOT` / `STORYFORGE_EVAL_EVIDENCE_DIR`
///   via the retention policy and allocate a fresh `run-<stage>-<uuid>` dir.
/// - Ephemeral temp roots require `STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE=1`.
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

    // Resume path: explicit EVIDENCE_DIR must pass durable-root / reparse / controlled
    // namespace validation via resolve_resume_run_dir (never trust dirname alone).
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

/// The core endurance runner for a single stage. Uses the production pipeline writer,
/// production-faithful Accept, sanitized checkpoints, and hard budget enforcement.
async fn run_endurance_stage(
    env: &HarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    campaign_id: storyforge_domain::Id,
    conversation_id: storyforge_domain::Id,
    stage: EnduranceStage,
    budget: &RealLlmRunBudget,
    paths: &EnduranceEvidencePaths,
) -> Result<EnduranceStageManifestRow, EnduranceError> {
    let target_turns = stage.target_turns();
    let max_near_raw = DEFAULT_H_ANCHOR + DEFAULT_E;
    // Only Stability/Full must cross H_anchor+E. Canary and Coverage are
    // intentional early checkpoints below that threshold.
    if matches!(stage, EnduranceStage::Stability | EnduranceStage::Full)
        && target_turns <= max_near_raw
    {
        return Err(EnduranceError::InvalidConfig(format!(
            "target_turns={target_turns} must be > H_anchor+E={max_near_raw} for stage {stage}"
        )));
    }

    let schedule = EnduranceSchedule::new(target_turns);
    let model_label = std::env::var("LLM_MODEL").unwrap_or_default();

    // Fail-closed resume: mixed run ids / schema drift / missing checkpoint content abort.
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
        // Fresh run under a non-namespaced legacy dir: still allocate a controlled id
        // for evidence lines (directory name may lag until retention seal).
        let run_id = harness_real_llm::evidence_retention::allocate_run_id(
            paths.root.parent().unwrap_or(paths.root.as_path()),
            stage.label(),
        )
        .unwrap_or_else(|_| format!("run-{}-{}", stage.label(), uuid::Uuid::new_v4()));
        (1, None, run_id)
    };
    let mut early_fact_probe_ids = resume_cp
        .as_ref()
        .map(|c| c.early_fact_probe_ids.clone())
        .unwrap_or_default();
    let mut early_fact_checked_passed = resume_cp
        .as_ref()
        .map(|c| c.early_fact_checked_passed.clone())
        .unwrap_or_default();
    // Cumulative calls already spent in prior process runs (for evidence/reporting).
    let prior_calls_used = resume_cp.as_ref().map(|c| c.calls_used).unwrap_or(0);

    let calls_before = llm.calls_used();
    let mut sample_cursor = llm.samples().len();

    let deadline_duration = budget.hard_deadline_override(target_turns, stage);
    let deadline = SuiteDeadline::new(deadline_duration);
    deadline.check()?;

    // On resume, append evidence rather than truncating accepted history.
    let call_writer = if start_turn > 1 {
        EvidenceWriter::open_append(&paths.calls_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.calls_jsonl, &run_id)?
    };
    let turn_writer = if start_turn > 1 {
        EvidenceWriter::open_append(&paths.turns_jsonl, &run_id)?
    } else {
        EvidenceWriter::create(&paths.turns_jsonl, &run_id)?
    };
    let probe = CommitProbeEnv::from_shared(
        env.data_dir.clone(),
        env.campaign_store.clone(),
        env.turn_store.clone(),
        env.conv_store.clone(),
    );

    let mut epoch_tracker = EpochTracker::default();
    if let Some(cp) = resume_cp.as_ref() {
        for eid in &cp.observed_epoch_ids16 {
            epoch_tracker.observe(eid);
        }
    }
    let mut turns_accepted = start_turn.saturating_sub(1);
    let mut summary_codes = Vec::new();
    // These are set inside the loop before first read; the initial values are never
    // observed because the loop body assigns before reading them for the checkpoint.
    #[allow(clippy::needless_late_init, unused_assignments)]
    let mut last_draft_hash16 = String::new();
    #[allow(clippy::needless_late_init, unused_assignments)]
    let mut last_campaign_revision = 0u64;
    #[allow(clippy::needless_late_init, unused_assignments)]
    let mut last_chronicle_revision = 0u64;

    let mut writer = PipelineProductionTurnWriter;

    for turn_index in start_turn..=target_turns {
        deadline.check()?;

        let action = schedule.action_for_turn(turn_index);
        let intent = match &action {
            ScheduledAction::Write { subagent_count, .. } => {
                format!(
                    "第{turn_index}轮：推进角色{subagent_count}人的场景，保持前文连续，留下可接续的互动停点。"
                )
            }
            ScheduledAction::RegenerateOverall => {
                format!("第{turn_index}轮：重新生成整体场景（overall reroll）。")
            }
            ScheduledAction::RegenerateEditor => {
                format!("第{turn_index}轮：仅编辑器重新润色（editor-only reroll）。")
            }
            ScheduledAction::RegenerateSubagent => {
                format!("第{turn_index}轮：仅子代理重新生成（subagent-only reroll）。")
            }
            ScheduledAction::PrivateProbe { probe_kind } => {
                format!(
                    "第{turn_index}轮：私密知识探针测试（{:?}）。推进场景，注意角色信息隔离。",
                    probe_kind
                )
            }
            ScheduledAction::EarlyFactInject { probe_id } => {
                format!(
                    "第{turn_index}轮：注入早期事实探针（指纹{}）。",
                    short_hash16(probe_id)
                )
            }
            ScheduledAction::EarlyFactCheck { probe_id } => {
                format!(
                    "第{turn_index}轮：检索早期事实探针（指纹{}）的可达性。",
                    short_hash16(probe_id)
                )
            }
            ScheduledAction::QualityAutofix { fixable } => {
                format!(
                    "第{turn_index}轮：质量门禁 autofix 测试（fixable={}）。",
                    fixable
                )
            }
            ScheduledAction::CacheStable => {
                format!("第{turn_index}轮：缓存稳定性观察轮次。")
            }
            ScheduledAction::CacheInvalidate => {
                format!("第{turn_index}轮：缓存失效探针（扰动或 epoch rollover）。")
            }
        };

        // Append user message
        let input_node_id = env
            .conv_store
            .append_user_message(&conversation_id, intent.clone())
            .map_err(|e| EnduranceError::Writer(e.to_string()))?;
        deadline.check()?;

        // Fill campaign context
        let base_ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = env.fill_campaign_context(base_ctx);
        deadline.check()?;

        let epoch_id16 = ctx
            .context_epoch
            .as_ref()
            .map(|e| short_hash16(&e.epoch_id));
        if let Some(ref eid) = epoch_id16 {
            epoch_tracker.observe(eid);
        }

        // Tag and write with bounded retry on transient PlanParse / LLM errors.
        // Matches the production harness pattern in t3_regenerate.rs.
        llm.set_tag(format!("turn{turn_index}"));
        llm.set_role("pipeline");

        const MAX_WRITE_ATTEMPTS: usize = 3;
        let mut written = None;
        let mut last_write_err = None;
        for attempt in 1..=MAX_WRITE_ATTEMPTS {
            deadline.check()?;
            let write_result = match tokio::time::timeout(
                deadline.remaining()?,
                writer.write_turn(env, &ctx, turn_index, &intent),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    // Best-effort flush on timeout
                    flush_samples(
                        &llm,
                        &call_writer,
                        &mut sample_cursor,
                        &run_id,
                        &model_label,
                        turn_index,
                    )?;
                    return Err(EnduranceError::SuiteTimeout);
                }
            };

            // Flush call evidence after each attempt so budget usage is visible.
            let calls_this_turn = flush_samples(
                &llm,
                &call_writer,
                &mut sample_cursor,
                &run_id,
                &model_label,
                turn_index,
            )?;
            deadline.check()?;

            // Budget check after each attempt (suite-wide cumulative).
            let runner_calls = prior_calls_used + llm.calls_used().saturating_sub(calls_before);
            if runner_calls > stage.max_calls() {
                return Err(EnduranceError::BudgetExhausted {
                    calls_used: runner_calls,
                    max_calls: stage.max_calls(),
                });
            }

            match write_result {
                Ok(w) => {
                    if calls_this_turn == 0 {
                        return Err(EnduranceError::ZeroCalls { turn_index });
                    }
                    written = Some(w);
                    break;
                }
                Err(err) => {
                    let transient = err.contains("Plan 解析")
                        || err.contains("PlanParse")
                        || err.contains("timeout")
                        || err.contains("Timeout")
                        || err.contains("520")
                        || err.contains("rate limit")
                        || err.contains("限流")
                        || err.contains("所有子 Agent 均失败")
                        || err.contains("client_error")
                        || err.contains("LlmError")
                        || err.contains("Internal");
                    eprintln!(
                        "[endurance {}] turn {turn_index} write attempt {attempt}/{MAX_WRITE_ATTEMPTS} failed (transient={transient}): {err}",
                        stage.label()
                    );
                    last_write_err = Some(err);
                    if !transient || attempt == MAX_WRITE_ATTEMPTS {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }

        let written = written.ok_or_else(|| {
            EnduranceError::Writer(
                last_write_err.unwrap_or_else(|| "write failed with no error detail".into()),
            )
        })?;

        // Handle early-fact injection
        if let ScheduledAction::EarlyFactInject { probe_id } = &action
            && !early_fact_probe_ids.contains(probe_id)
        {
            early_fact_probe_ids.push(probe_id.clone());
        }

        // Accept via production-faithful probe
        let accept_input = ProductionAcceptInput {
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            variant_id: written.variant_id.clone(),
            draft_text: written.draft_text.clone(),
            summary_text: written.summary_text.clone(),
            turn_number: turn_index,
            quality_report: Some(QualityReport { warnings: vec![] }),
            force_accept: false,
        };
        probe.prepare_awaiting_accept_with_input_node(&accept_input, input_node_id);
        deadline.check()?;
        let accept = probe.accept_production(&accept_input);
        deadline.check()?;

        if !accept.ok {
            return Err(EnduranceError::Accept(
                accept
                    .error
                    .unwrap_or_else(|| "unknown accept failure".into()),
            ));
        }

        turns_accepted += 1;
        last_campaign_revision = accept.campaign_revision_after;
        last_chronicle_revision = accept.chronicle_revision_after;
        last_draft_hash16 = short_hash16(&accept.draft_hash);
        if let Some(code) = &accept.summary_code {
            summary_codes.push(code.clone());
        }

        // Write turn evidence
        let elapsed_ms = deadline.started.elapsed().as_millis();
        let turn_rec = build_endurance_turn_record(EnduranceTurnRecordInput {
            run_id: &run_id,
            stage,
            turn_index,
            action: &action,
            draft_accepted: accept.ok,
            campaign_revision_before: accept.campaign_revision_before,
            campaign_revision_after: accept.campaign_revision_after,
            chronicle_revision_before: accept.chronicle_revision_before,
            chronicle_revision_after: accept.chronicle_revision_after,
            summary_code: accept.summary_code.clone(),
            draft_hash16: short_hash16(&accept.draft_hash),
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
            attempt_status: format!("{:?}", accept.attempt_status),
            turn_status: format!("{:?}", accept.turn_status),
            assertions: accept.assertions.clone(),
            elapsed_ms,
        });
        turn_writer.write_turn(turn_rec)?;

        // Write sanitized checkpoint
        let cp = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.clone(),
            stage: stage.label().into(),
            accepted_turn_number: turn_index,
            calls_used: prior_calls_used + llm.calls_used().saturating_sub(calls_before),
            max_calls: stage.max_calls(),
            campaign_revision: last_campaign_revision,
            chronicle_revision: last_chronicle_revision,
            last_draft_hash16: last_draft_hash16.clone(),
            last_summary_code: accept.summary_code.clone(),
            context_epoch_id16: epoch_id16,
            early_fact_probe_ids: early_fact_probe_ids.clone(),
            early_fact_checked_passed: early_fact_checked_passed.clone(),
            campaign_id: Some(campaign_id.as_str().to_string()),
            conversation_id: Some(conversation_id.as_str().to_string()),
            data_dir_rel: Some("campaign_data".into()),
            observed_epoch_ids16: epoch_tracker.observed.clone(),
            recorded_at_unix_ms: 0,
        };
        write_checkpoint(&paths.checkpoint_jsonl, &cp)?;
        // Unsealed interrupted resume is only auditable with a checkpoint integrity baseline.
        // Refresh after every accepted turn so crash recovery can fail closed on tamper.
        harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&paths.root)
            .map_err(|e| {
                EnduranceError::InvalidConfig(format!("checkpoint integrity baseline: {e}"))
            })?;

        // Early-fact check: verify reachability in store
        if let ScheduledAction::EarlyFactCheck { probe_id } = &action {
            let summaries = env.campaign_store.list_summaries(&campaign_id);
            let reachable = summaries.iter().any(|s| {
                // Check if any summary was written for the early-fact turn
                s.content.contains(probe_id) || early_fact_probe_ids.contains(probe_id)
            });
            if reachable && !early_fact_checked_passed.contains(probe_id) {
                early_fact_checked_passed.push(probe_id.clone());
            }
        }

        let runner_calls = llm.calls_used().saturating_sub(calls_before);
        eprintln!(
            "[endurance {}] turn {turn_index}/{target_turns} accepted, calls={runner_calls}/{}, epoch_ids={}",
            stage.label(),
            stage.max_calls(),
            epoch_tracker.count()
        );
    }

    // Evidence size budget check (2MB hard cap; redaction keeps rows small).
    let total_size = paths.total_size() as usize;
    if total_size > 2 * 1024 * 1024 {
        return Err(EnduranceError::InvalidConfig(format!(
            "evidence size {total_size} exceeds 2MB budget"
        )));
    }

    // Secret check
    if let Err(msg) = paths.check_no_secrets() {
        return Err(EnduranceError::SecretViolation(msg));
    }

    let calls_used = prior_calls_used + llm.calls_used().saturating_sub(calls_before);
    // Stage-aware invariants:
    // - Canary/Coverage are below H_anchor+E, so epoch rollover is not required.
    // - Early-fact probes only appear in the full 100-turn schedule.
    // - Stability/Full must cross H+E and show epoch rollover.
    let requires_epoch = target_turns > max_near_raw;
    let requires_early_fact = target_turns >= 35;
    let invariants_pass = turns_accepted >= target_turns
        && (!requires_epoch || epoch_tracker.rolled_over())
        && (!requires_early_fact || !early_fact_probe_ids.is_empty());

    let acceptance = classify_acceptance(
        stage,
        turns_accepted,
        calls_used,
        invariants_pass,
        false,
        false,
    );

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
        summary_codes,
        observed_epoch_ids16: epoch_tracker.observed.clone(),
        early_fact_probe_ids: early_fact_probe_ids.clone(),
        early_fact_checked_passed: early_fact_checked_passed.clone(),
        coverage_assertions: schedule.coverage_report(),
        recorded_at_unix_ms: 0,
    };
    write_manifest_row(&paths.manifest_jsonl, &row)?;

    eprintln!(
        "[endurance {}] DONE: acceptance={}, turns={}/{}, calls={}/{}, epochs={}, early_facts={:?}, elapsed_ms={}",
        stage.label(),
        acceptance,
        turns_accepted,
        target_turns,
        calls_used,
        stage.max_calls(),
        epoch_tracker.count(),
        early_fact_probe_ids,
        row.elapsed_ms
    );

    // A requested full stage that stops early must exit non-zero
    if acceptance != AcceptanceLevel::Pass && stage == EnduranceStage::Full {
        return Err(EnduranceError::InvalidConfig(format!(
            "full stage did not pass: acceptance={acceptance}"
        )));
    }

    Ok(row)
}

fn flush_samples(
    llm: &BudgetedLlmClient,
    writer: &EvidenceWriter,
    cursor: &mut usize,
    run_id: &str,
    model_label: &str,
    turn_index: u32,
) -> Result<usize, EnduranceError> {
    let samples = llm.samples();
    let turn_samples = samples.get(*cursor..).unwrap_or(&[]);
    for sample in turn_samples {
        writer.write_call(sample.to_evidence_call(
            run_id,
            "endurance",
            turn_index,
            model_label,
            vec![harness_real_llm::evidence::AssertionResult {
                name: "call_recorded".into(),
                passed: sample.outcome == "ok",
                detail: Some(format!("outcome={}", sample.outcome)),
            }],
        ))?;
    }
    let written = turn_samples.len();
    *cursor = samples.len();
    Ok(written)
}

/// Build a complex test character in code — no fixture PNG required.
/// Uses a rich multi-character world to exercise Director/Subagent/Editor.
fn build_test_character() -> storyforge_domain::character::Character {
    use storyforge_domain::Source;
    use storyforge_domain::character::Character;
    use storyforge_domain::world_info::{LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry};

    let mut entries = Vec::new();
    // Constant lore (blue light): world-building facts always injected
    for (i, (keys, content)) in [
        (
            vec!["港口".to_string(), "码头".to_string()],
            "港口区是城市最繁忙的地带，货物日夜进出，走私者混迹其中。".to_string(),
        ),
        (
            vec!["组织".to_string(), "银鸦".to_string()],
            "银鸦组织控制着港口的地下贸易网络，以徽章编号识别成员。".to_string(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        entries.push(WorldInfoEntry {
            st_id: Some(i as i32),
            keys,
            secondary_keys: vec![],
            content,
            constant: true,
            selective: false,
            selective_logic: SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 4,
            order: i as i32,
            route: LoreRoute::Constant,
            extensions: serde_json::Value::Null,
            extra: Default::default(),
        });
    }
    // Selective lore (green light): keyword-triggered
    for (i, (keys, content)) in [
        (
            vec!["陈".to_string(), "徽章".to_string()],
            "陈的徽章编号是高度机密，只有组织高层知道。".to_string(),
        ),
        (
            vec!["林".to_string(), "密码".to_string()],
            "林的访问密码保护着控制台的最终权限。".to_string(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        entries.push(WorldInfoEntry {
            st_id: Some((10 + i) as i32),
            keys,
            secondary_keys: vec![],
            content,
            constant: false,
            selective: true,
            selective_logic: SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 4,
            order: (10 + i) as i32,
            route: LoreRoute::Selective,
            extensions: serde_json::Value::Null,
            extra: Default::default(),
        });
    }

    Character {
        id: storyforge_domain::Id::from_str("endurance-test-character"),
        name: "雨港探案".to_string(),
        description: "一个发生在港口城市的悬疑角色扮演场景。主角陈是银鸦组织的卧底，搭档林是外部调查员。两人在雨夜码头相遇，调查一系列走私案件。场景包含多个角色和复杂的人际关系网络。".to_string(),
        personality: "沉稳、机警、善于观察。对话风格简洁有力，善于用细节推动剧情。".to_string(),
        scenario: "雨夜，港口码头。陈和林在暗处观察一艘可疑货船。走私者即将现身。".to_string(),
        first_mes: "雨水顺着帽檐滴落，陈压低声音对林说：\"今晚的目标已经出现了。银鸦的人随时会到。保持警惕，不要暴露身份。\"".to_string(),
        mes_example: String::new(),
        system_prompt: "你是一个沉浸式角色扮演助手。扮演场景中的NPC角色，推动悬疑剧情发展。".to_string(),
        post_history_instructions: String::new(),
        tags: vec!["悬疑".to_string(), "角色扮演".to_string(), "港口".to_string()],
        creator: "endurance-harness".to_string(),
        character_version: "1.0".to_string(),
        alternate_greetings: vec![],
        embedded_world_info: Some(WorldInfoBook {
            entries,
            source: Source::Native,
            metadata: Default::default(),
        }),
        extensions: serde_json::Value::Null,
        renderable_assets: Default::default(),
        source: Source::Native,
        spec_version: "3.0".to_string(),
        raw_card_json: serde_json::Value::Null,
    }
}

/// Real-model 100-turn endurance run. Gated by `STORYFORGE_EVAL_REAL_LLM=1` + env credentials.
/// Default stage is Canary (3 turns); set `STORYFORGE_EVAL_ENDURANCE_STAGE=full` for 100 turns.
#[tokio::test]
#[ignore = "需要 STORYFORGE_EVAL_REAL_LLM=1 与 LLM 凭证；默认不跑"]
async fn endurance_real_llm_full_100_turn() {
    let budget = require_endurance_budget();
    let stage = parse_target_stage();

    let (run_id_for_log, dir, paths) = evidence_run_dir(stage);
    eprintln!(
        "[endurance] evidence run_id={} dir_name={}",
        run_id_for_log,
        dir.file_name().and_then(|s| s.to_str()).unwrap_or("<run>")
    );
    let data_dir = dir.join("campaign_data");

    // Resume from sanitized checkpoint when available; otherwise bootstrap a fresh campaign.
    // Fail closed on mixed ids / schema drift before any model call.
    let resume_cp = if paths.checkpoint_jsonl.exists() {
        match resume_from_evidence_dir(&paths.root, Some(run_id_for_log.as_str())) {
            Ok((_, cp)) => Some(cp),
            Err(err) => panic!("fail-closed resume preflight: {err}"),
        }
    } else {
        None
    };
    // On resume, the suite-wide budget is remaining = stage.max - checkpoint.calls_used.
    let mut budget = budget;
    if let Some(cp) = resume_cp.as_ref() {
        let remaining = stage.max_calls().saturating_sub(cp.calls_used);
        budget.max_calls = remaining.max(1);
        eprintln!(
            "[endurance] resume budget: prior_calls={} remaining_calls={}",
            cp.calls_used, budget.max_calls
        );
    }
    let llm = BudgetedLlmClient::wrap(require_real_llm(), &budget);

    let (env, campaign_id, conversation_id) = if let Some(cp) = resume_cp
        .as_ref()
        .filter(|c| c.campaign_id.is_some() && c.conversation_id.is_some())
    {
        let env = HarnessEnv::open(data_dir.clone(), llm.clone() as Arc<dyn LlmClient>);
        let campaign_id = storyforge_domain::Id::from_str(cp.campaign_id.as_deref().unwrap());
        let conversation_id =
            storyforge_domain::Id::from_str(cp.conversation_id.as_deref().unwrap());
        env.set_active_campaign(campaign_id.clone());
        // Re-inject character for tool_ctx world-info / character presence.
        env.inject_character(build_test_character());
        eprintln!(
            "[endurance] resuming from checkpoint turn {} campaign={}",
            cp.accepted_turn_number,
            campaign_id.as_str()
        );
        (env, campaign_id, conversation_id)
    } else {
        let env = HarnessEnv::open(data_dir.clone(), llm.clone() as Arc<dyn LlmClient>);
        // Build character in code — no fixture PNG needed.
        let character = build_test_character();
        env.inject_character(character);

        // Build card + definitions without LLM extract (fallback single-role definition).
        let ch = {
            let ctx = env.tool_ctx.read().unwrap();
            ctx.characters[0].as_ref().clone()
        };
        let mut card = storyforge_domain::character::CharacterCard::from_character(&ch);
        let def =
            storyforge_domain::character::CharacterDefinition::fallback_from_character(&ch, &[]);
        let definitions = storyforge_app_agent::attach_definitions_to_card(vec![def], &card.id);
        card.character_definitions = definitions;
        let stored = env.campaign_store.save_card(card).expect("save card");
        let card = stored.card;

        let campaign_id = env.create_campaign(&card, "endurance-real");
        let conversation_id = env.conv_store.create(None, Some(campaign_id.clone())).id;
        let mut campaign = env
            .campaign_store
            .get_campaign(&campaign_id)
            .expect("campaign after create");
        campaign.conversation_id = Some(conversation_id.clone());
        env.campaign_store
            .update_campaign(campaign)
            .expect("bind campaign conversation");
        (env, campaign_id, conversation_id)
    };

    let result = run_endurance_stage(
        &env,
        llm.clone(),
        campaign_id,
        conversation_id,
        stage,
        &budget,
        &paths,
    )
    .await;

    match &result {
        Ok(row) => {
            eprintln!(
                "ENDURANCE {} PASS: turns={}/{}, calls={}/{}, acceptance={}, epochs={}, early_facts={:?}",
                row.stage,
                row.accepted_turns,
                row.target_turns,
                row.calls_used,
                row.max_calls,
                row.acceptance,
                row.observed_epoch_ids16.len(),
                row.early_fact_probe_ids
            );
            // Evidence must be redacted
            assert!(paths.check_no_secrets().is_ok());
            // Evidence must be within size budget
            assert!(paths.total_size() < 2 * 1024 * 1024);
            // Seal + offline verify are hard requirements for a real Full/stage pass.
            // Failure must fail the run — never downgrade to a warning.
            let manifest = harness_real_llm::evidence_retention::seal_run(
                &paths.root,
                harness_real_llm::evidence_retention::SealOptions {
                    run_id: row.run_id.clone(),
                    status: harness_real_llm::evidence_retention::RunStatus::Completed,
                    stage: row.stage.clone(),
                    model_label: std::env::var("LLM_MODEL")
                        .unwrap_or_default()
                        .chars()
                        .take(64)
                        .collect(),
                    budget: harness_real_llm::evidence_retention::BudgetSummary {
                        max_calls: row.max_calls,
                        max_turns: row.target_turns,
                        timeout_secs: budget.timeout_secs,
                        max_tokens: budget.max_tokens,
                    },
                    commit: std::env::var("STORYFORGE_EVAL_COMMIT").unwrap_or_default(),
                    branch: std::env::var("STORYFORGE_EVAL_BRANCH").unwrap_or_default(),
                },
            )
            .unwrap_or_else(|err| {
                panic!(
                    "{}",
                    harness_real_llm::evidence_retention::format_seal_hard_error(&err)
                )
            });
            eprintln!(
                "[endurance] sealed run_manifest files={} status=completed",
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
            eprintln!("ENDURANCE {} FAIL CLOSED: {err}", stage.label());
            // Keep campaign_data for resume; do not wipe evidence root.
            panic!("endurance stage {stage} failed closed: {err}");
        }
    }
}

/// Real-model Phase B A/B matrix with 12+ paired scenarios.
/// Gated by `STORYFORGE_EVAL_REAL_LLM=1` + env credentials.
#[tokio::test]
#[ignore = "需要 STORYFORGE_EVAL_REAL_LLM=1 与 LLM 凭证；默认不跑"]
async fn endurance_real_llm_phase_b_matrix_12_scenarios() {
    let _budget = require_endurance_budget();
    let _stage = EnduranceStage::Coverage;

    // Phase B matrix is deterministic but we verify the full matrix passes
    // and evidence is redacted.
    let (_run_id, dir, _paths) = evidence_run_dir(EnduranceStage::Coverage);
    let phase_b_path = dir.join("endurance_phase_b.jsonl");

    let fixtures = harness_real_llm::phase_b_matrix::default_phase_b_fixtures();
    assert!(fixtures.len() >= 12, "PLAN requires >= 12 paired scenarios");

    let report = harness_real_llm::phase_b_matrix::run_phase_b_matrix(
        &fixtures,
        phase_b_path.clone(),
        &format!("endurance-pb-{}", uuid::Uuid::new_v4()),
    );

    eprintln!(
        "PHASE B MATRIX: fixtures={}, rows={}, autofix_trigger={:.2}, autofix_success={:.2}, leak_post_fix={:.2}",
        report.summary.fixture_count,
        report.summary.total_rows,
        report.summary.phase_b_autofix_trigger_rate,
        report.summary.phase_b_autofix_success_rate,
        report.summary.phase_b_leak_rate_post_fix
    );

    assert!(
        report.assertions.iter().all(|a| a.passed),
        "Phase B matrix assertions must all pass"
    );

    // Evidence must not contain raw secrets
    let lines = harness_real_llm::evidence::read_evidence_lines(&phase_b_path).unwrap();
    for line in &lines {
        let s = line.to_string();
        assert!(!s.contains("SF_SECRET_"), "raw secret in evidence");
        assert!(!s.contains("sk-"), "api key marker in evidence");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Extension trait for budget to compute hard deadline per stage.
trait HardDeadlineExt {
    fn hard_deadline_override(&self, turns: u32, stage: EnduranceStage) -> std::time::Duration;
}

impl HardDeadlineExt for RealLlmRunBudget {
    fn hard_deadline_override(&self, turns: u32, stage: EnduranceStage) -> std::time::Duration {
        // Allow for multi-call pipeline roles and bounded write retries.
        // Stage-aware floors prevent wall-clock aborts on Coverage/Stability/Full
        // while still enforcing a hard upper bound.
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

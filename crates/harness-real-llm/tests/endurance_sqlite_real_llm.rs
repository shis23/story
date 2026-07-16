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
use harness_real_llm::coverage_ledger::CoverageLedger;
use harness_real_llm::endurance::*;
use harness_real_llm::evidence::{EvidenceWriter, RealLlmRunBudget, short_hash16};
use harness_real_llm::require_real_llm;
use harness_real_llm::sqlite_endurance::SqliteHarnessEnv;
use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::WritingContext;
use storyforge_infra_llm::LlmClient;

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
            "endurance_sqlite",
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

async fn run_sqlite_endurance_stage(
    env: &SqliteHarnessEnv,
    llm: Arc<BudgetedLlmClient>,
    campaign_id: storyforge_domain::Id,
    conversation_id: storyforge_domain::Id,
    stage: EnduranceStage,
    budget: &RealLlmRunBudget,
    paths: &EnduranceEvidencePaths,
) -> Result<EnduranceStageManifestRow, EnduranceError> {
    let target_turns = stage.target_turns();
    let schedule = EnduranceSchedule::new(target_turns);
    let planned: Vec<(u32, ScheduledAction)> = (1..=target_turns)
        .map(|n| (n, schedule.action_for_turn(n)))
        .collect();
    let mut ledger = CoverageLedger::plan_from_schedule(planned);
    let model_label = std::env::var("LLM_MODEL").unwrap_or_default();

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

    let prior_calls_used = resume_cp.as_ref().map(|c| c.calls_used).unwrap_or(0);
    let calls_before = llm.calls_used();
    let mut sample_cursor = llm.samples().len();
    let deadline = SuiteDeadline::new(budget.hard_deadline_override(target_turns, stage));
    deadline.check()?;

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

    for turn_index in start_turn..=target_turns {
        deadline.check()?;
        let action = schedule.action_for_turn(turn_index);
        let row = ledger
            .planned
            .iter()
            .find(|r| r.turn_index == turn_index)
            .cloned()
            .ok_or_else(|| {
                EnduranceError::InvalidConfig(format!("missing coverage row for turn {turn_index}"))
            })?;
        let intent = match &action {
            ScheduledAction::Write { subagent_count, .. } => {
                format!(
                    "turn {turn_index}: advance scene with {subagent_count} characters; keep continuity."
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
                let _ = probe_kind;
                format!(
                    "turn {turn_index}: continue the scene with careful information isolation between characters."
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
                    "turn {turn_index}: continue the scene and revisit earlier investigation details for continuity."
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
                        &mut sample_cursor,
                        &run_id,
                        &model_label,
                        turn_index,
                    );
                    return Err(EnduranceError::SuiteTimeout);
                }
            };
            let calls_this_turn = flush_samples(
                &llm,
                &call_writer,
                &mut sample_cursor,
                &run_id,
                &model_label,
                turn_index,
            )?;
            let runner_calls = prior_calls_used + llm.calls_used().saturating_sub(calls_before);
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
                        "[sqlite endurance {}] turn {turn_index} attempt {attempt}/{MAX_WRITE_ATTEMPTS} failed (transient={transient}): {err}",
                        stage.label()
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

        let mut observed = written.observed.clone();
        if let ScheduledAction::EarlyFactCheck { probe_id } = &action {
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
        }

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

        turns_accepted += 1;
        last_campaign_revision = written.accept.campaign_revision_after;
        last_chronicle_revision = written.accept.chronicle_revision_after;
        last_draft_hash16 = short_hash16(&written.accept.draft_hash);

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
            chronicle_path: Some("production_postprocess_service_fixed_runner"),
            accept_path: Some("sqlite_runtime::accept_by_variant"),
            production_postprocess_complete: Some(written.postprocess_proof.applied),
        });
        turn_writer.write_turn(turn_rec)?;

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
            last_summary_code: written.accept.summary_code.clone(),
            context_epoch_id16: epoch_id16,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: Some(campaign_id.as_str().to_string()),
            conversation_id: Some(conversation_id.as_str().to_string()),
            data_dir_rel: Some("campaign_data".into()),
            observed_epoch_ids16: epoch_tracker.observed.clone(),
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

    std::fs::write(&ledger_path, ledger_lines.join("\n") + "\n")
        .map_err(EnduranceError::EvidenceIo)?;

    if let Err(ms) = ledger.exact_set_verify() {
        return Err(EnduranceError::InvalidConfig(format!(
            "coverage ledger exact-set failed: {ms:?}"
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

    let calls_used = prior_calls_used + llm.calls_used().saturating_sub(calls_before);
    let acceptance = classify_acceptance(
        stage,
        turns_accepted,
        calls_used,
        turns_accepted >= target_turns,
        false,
        false,
    );
    let mut coverage_assertions = schedule.coverage_report();
    coverage_assertions.extend(ledger.assertion_results());
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
    let llm = BudgetedLlmClient::wrap(require_real_llm(), &budget);

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

    match run_sqlite_endurance_stage(
        &env,
        llm.clone(),
        campaign_id,
        conversation_id,
        stage,
        &budget,
        &paths,
    )
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
            assert!(paths.check_no_secrets().is_ok());
            assert!(paths.total_size() < 2 * 1024 * 1024);

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
            eprintln!("SQLITE ENDURANCE {} FAIL CLOSED: {err}", stage.label());
            panic!("sqlite endurance stage {stage} failed closed: {err}");
        }
    }
}

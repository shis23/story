//! Gate B deterministic proof: SQLite fixture + coverage ledger exact-set.
//!
//! No real model. Does not activate the process SQLite OnceLock (unit-level
//! ledger/fixture only). Lifecycle UoW proof lives in tauri-app Gate A tests.

use harness_real_llm::coverage_ledger::{
    CoverageLedger, LedgerMismatch, ObservationKey, ObservedCoverage, RuntimeCoverageProfile,
    SqlitePostcondition, action_with_runtime_profile, default_required_for,
};
use harness_real_llm::endurance::{
    EnduranceSchedule, EnduranceStage, PrivateProbeKind, ReasoningModeSlot, ScheduledAction,
    ToolModeSlot, WorldInfoRouteSlot,
};
use harness_real_llm::sqlite_endurance::PipelineObservationCollector;
use storyforge_domain::agent::PipelineEvent;

fn generic_pipeline_observations() -> std::collections::BTreeSet<ObservationKey> {
    [
        ObservationKey::SqliteAuthoritative,
        ObservationKey::JsonFallbackFalse,
        ObservationKey::CommandPath("sqlite_runtime::create_draft_attempt".into()),
        ObservationKey::ServicePath("pipeline.start_writing".into()),
        ObservationKey::DraftLanded,
        ObservationKey::PostprocessApplied,
        ObservationKey::Accepted,
        ObservationKey::OutboxKind("draft_ready".into()),
        ObservationKey::OutboxKind("postprocess_apply".into()),
        ObservationKey::AgentRole("pipeline".into()),
    ]
    .into_iter()
    .collect()
}

fn observed_for(
    row_id: String,
    observations: std::collections::BTreeSet<ObservationKey>,
) -> ObservedCoverage {
    ObservedCoverage {
        row_id,
        turn_index: 1,
        command_path: "sqlite_runtime::create_draft_attempt".into(),
        service_path: "pipeline.start_writing".into(),
        agent_events: vec!["pipeline".into()],
        turn_id16: "t".into(),
        attempt_id16: "a".into(),
        variant_id16: "v".into(),
        sqlite_post: SqlitePostcondition {
            sqlite_authoritative: true,
            json_fallback: false,
            turn_status: "Committed".into(),
            attempt_status: "Committed".into(),
            outbox_kinds: vec!["draft_ready".into(), "postprocess_apply".into()],
            campaign_revision_after: 1,
            postprocess_applied: true,
            batch_digest16: Some("abcd".into()),
        },
        observations,
    }
}

#[test]
fn m5_fixture_file_is_committed_and_parseable() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/m5_sqlite_endurance_v1.json");
    assert!(path.exists(), "missing {}", path.display());
    let text = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["fixture_version"], "m5_sqlite_endurance_v1");
    assert_eq!(v["safe_probe_facts"].as_array().unwrap().len(), 3);
    assert!(v["definitions"].as_array().unwrap().len() >= 4);
    // same display name, different IDs
    let names: Vec<_> = v["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.iter().filter(|n| *n == "码头工人").count() >= 2);
}

#[test]
fn canary_schedule_ledger_exact_set_passes_when_actions_really_observed() {
    let schedule = EnduranceSchedule::new(EnduranceStage::Canary.target_turns());
    let planned: Vec<(u32, ScheduledAction)> =
        (1..=3).map(|n| (n, schedule.action_for_turn(n))).collect();
    let mut ledger = CoverageLedger::plan_from_schedule(planned);
    assert_eq!(ledger.planned.len(), 3);

    for row in ledger.planned.clone() {
        let agent_events = row
            .required_observations
            .iter()
            .filter_map(|observation| match observation {
                ObservationKey::AgentRole(role) => Some(role.clone()),
                _ => None,
            })
            .collect();
        ledger.record(ObservedCoverage {
            row_id: row.row_id.clone(),
            turn_index: row.turn_index,
            command_path: "sqlite_runtime::create_draft_attempt".into(),
            service_path: "pipeline.start_writing".into(),
            agent_events,
            turn_id16: format!("t{}", row.turn_index),
            attempt_id16: format!("a{}", row.turn_index),
            variant_id16: format!("v{}", row.turn_index),
            sqlite_post: SqlitePostcondition {
                sqlite_authoritative: true,
                json_fallback: false,
                turn_status: "Committed".into(),
                attempt_status: "Committed".into(),
                outbox_kinds: vec!["draft_ready".into(), "postprocess_apply".into()],
                campaign_revision_after: row.turn_index as u64,
                postprocess_applied: true,
                batch_digest16: Some("abcd".into()),
            },
            observations: row.required_observations.clone(),
        });
    }
    assert!(ledger.exact_set_verify().is_ok());
    assert!(ledger.assertion_results().iter().all(|a| a.passed));
}

#[test]
fn ledger_fails_when_json_fallback_or_unobserved_row() {
    let mut ledger = CoverageLedger::plan_from_schedule([(
        1,
        ScheduledAction::Write {
            subagent_count: 1,
            reasoning_mode: ReasoningModeSlot::Disabled,
            tool_mode: ToolModeSlot::Native,
            world_info_route: WorldInfoRouteSlot::Constant,
        },
    )]);
    // unobserved planned row
    assert!(ledger.exact_set_verify().is_err());

    let row = ledger.planned[0].clone();
    let mut observations = row.required_observations.clone();
    observations.insert(ObservationKey::CoverageLabel("extra".into()));
    ledger.record(ObservedCoverage {
        row_id: row.row_id.clone(),
        turn_index: 1,
        command_path: "x".into(),
        service_path: "y".into(),
        agent_events: vec![],
        turn_id16: "t".into(),
        attempt_id16: "a".into(),
        variant_id16: "v".into(),
        sqlite_post: SqlitePostcondition {
            sqlite_authoritative: false,
            json_fallback: true,
            ..Default::default()
        },
        observations,
    });
    let err = ledger.exact_set_verify().unwrap_err();
    assert!(err.iter().any(|m| matches!(
        m,
        harness_real_llm::coverage_ledger::LedgerMismatch::Postcondition { .. }
    )));
}

#[test]
fn write_rows_require_real_agent_events_and_actual_runtime_modes() {
    let required = default_required_for(&ScheduledAction::Write {
        subagent_count: 3,
        reasoning_mode: ReasoningModeSlot::Native,
        tool_mode: ToolModeSlot::TextFallback,
        world_info_route: WorldInfoRouteSlot::Both,
    });

    for role in [
        "director",
        "subagent",
        "editor",
        "summarizer",
        "postprocessor",
    ] {
        assert!(
            required.contains(&ObservationKey::AgentRole(role.into())),
            "missing actual {role} event"
        );
    }
    for event in [
        "reasoning_mode:native",
        "tool_mode:text_fallback",
        "world_info_route_available:both",
    ] {
        assert!(
            required.contains(&ObservationKey::ToolEvent(event.into())),
            "missing actual runtime observation {event}"
        );
    }
    assert!(required.contains(&ObservationKey::CoverageLabel("subagent_count:3".into())));
    assert!(!required.contains(&ObservationKey::AgentRole("pipeline".into())));
}

#[test]
fn claimed_agent_role_observations_require_matching_recorded_events() {
    let mut ledger = CoverageLedger::plan_from_schedule([(
        1,
        ScheduledAction::Write {
            subagent_count: 1,
            reasoning_mode: ReasoningModeSlot::Disabled,
            tool_mode: ToolModeSlot::Native,
            world_info_route: WorldInfoRouteSlot::Constant,
        },
    )]);
    let row = ledger.planned[0].clone();
    let mut observed = observed_for(row.row_id, row.required_observations);
    observed.agent_events = vec!["pipeline".into()];
    ledger.record(observed);

    assert!(
        ledger.exact_set_verify().is_err(),
        "static role observations must not replace actual recorded agent events"
    );
}

#[test]
fn generic_pipeline_observations_cannot_satisfy_special_actions() {
    let cases = [
        (
            ScheduledAction::QualityAutofix { fixable: true },
            "tool:quality_gate:evaluated",
        ),
        (
            ScheduledAction::QualityAutofix { fixable: false },
            "tool:autofix:rejected",
        ),
        (
            ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::MustNotReveal,
            },
            "tool:private_final_output:must_not_reveal:no_leak",
        ),
        (ScheduledAction::CacheStable, "tool:cache:stable"),
        (ScheduledAction::CacheInvalidate, "tool:cache:invalidated"),
    ];

    for (action, expected_missing) in cases {
        let mut ledger = CoverageLedger::plan_from_schedule([(1, action)]);
        let row_id = ledger.planned[0].row_id.clone();
        ledger.record(observed_for(
            row_id.clone(),
            generic_pipeline_observations(),
        ));
        let mismatches = ledger.exact_set_verify().unwrap_err();
        assert!(
            mismatches.iter().any(|m| matches!(
                m,
                LedgerMismatch::Missing { row_id: id, missing }
                    if id == &row_id && missing.contains(expected_missing)
            )),
            "special row {row_id} did not require {expected_missing}: {mismatches:?}"
        );
    }
}

#[test]
fn quality_private_and_cache_rows_require_operation_specific_proof() {
    let quality_fixed = default_required_for(&ScheduledAction::QualityAutofix { fixable: true });
    assert!(quality_fixed.contains(&ObservationKey::ToolEvent("quality_gate:evaluated".into())));
    assert!(quality_fixed.contains(&ObservationKey::AutofixSynced));
    assert!(quality_fixed.contains(&ObservationKey::OutboxKind("autofix_sync".into())));
    assert!(quality_fixed.contains(&ObservationKey::ToolEvent("autofix:fixed".into())));

    let quality_rejected =
        default_required_for(&ScheduledAction::QualityAutofix { fixable: false });
    assert!(quality_rejected.contains(&ObservationKey::ToolEvent("autofix:rejected".into())));

    let private = default_required_for(&ScheduledAction::PrivateProbe {
        probe_kind: PrivateProbeKind::NonOwnerLeak,
    });
    assert!(private.contains(&ObservationKey::ToolEvent(
        "private_final_output:non_owner_leak:no_leak".into()
    )));

    let stable = default_required_for(&ScheduledAction::CacheStable);
    assert!(stable.contains(&ObservationKey::ToolEvent("cache:stable".into())));
    let invalidated = default_required_for(&ScheduledAction::CacheInvalidate);
    assert!(invalidated.contains(&ObservationKey::ToolEvent("cache:invalidated".into())));
}

#[test]
fn pipeline_observation_collector_only_attests_real_agent_events() {
    let mut collector = PipelineObservationCollector::default();
    for started_only in [
        PipelineEvent::DirectorStarted,
        PipelineEvent::SubagentStarted {
            character_id: "private-id-a".into(),
            index: 0,
            total: 2,
        },
        PipelineEvent::EditorStarted,
    ] {
        collector.observe(&started_only);
    }
    assert!(
        collector.agent_events().is_empty(),
        "started events alone must never attest agent completion"
    );
    assert_eq!(collector.subagent_count(), 0);

    for event in [
        PipelineEvent::DirectorProgress {
            delta: "must never be retained in evidence".into(),
        },
        PipelineEvent::DirectorDone {
            scene_brief: "also not retained".into(),
            subagent_count: 2,
        },
        PipelineEvent::SubagentDone {
            character_id: "private-id-a".into(),
            index: 0,
            full_text: "must never be retained".into(),
        },
        PipelineEvent::SubagentDone {
            character_id: "private-id-b".into(),
            index: 1,
            full_text: "must never be retained".into(),
        },
        PipelineEvent::DraftReady {
            text: "must never be retained".into(),
        },
        PipelineEvent::QualityChecked {
            passed: false,
            warning_count: 1,
            error_count: 2,
            warnings: vec!["must not be retained".into()],
        },
        PipelineEvent::DraftReady {
            text: "fixed text must never be retained".into(),
        },
        PipelineEvent::QualityChecked {
            passed: true,
            warning_count: 0,
            error_count: 0,
            warnings: vec!["must not be retained".into()],
        },
        PipelineEvent::PostProcessStarted,
        PipelineEvent::SummaryDone { char_count: 80 },
        PipelineEvent::PostProcessDone {
            knowledge_count: 1,
            variable_count: 1,
            task_count: 0,
        },
    ] {
        collector.observe(&event);
    }

    assert_eq!(
        collector.agent_events(),
        vec![
            "director".to_string(),
            "editor".to_string(),
            "postprocessor".to_string(),
            "subagent".to_string(),
            "summarizer".to_string(),
        ]
    );
    assert_eq!(collector.subagent_count(), 2);
    assert!(collector.quality_checked());
    assert_eq!(collector.quality_error_counts(), &[2, 0]);
    assert!(collector.completed_quality_autofix());
    let safe = serde_json::to_string(&collector).unwrap();
    assert!(!safe.contains("must never"));
    assert!(!safe.contains("private-id"));
    assert!(!safe.contains("also not retained"));
}

#[test]
fn run_level_connection_modes_replace_schedule_only_labels() {
    let scheduled = ScheduledAction::Write {
        subagent_count: 2,
        reasoning_mode: ReasoningModeSlot::Prompted,
        tool_mode: ToolModeSlot::TextFallback,
        world_info_route: WorldInfoRouteSlot::Selective,
    };
    let actual = action_with_runtime_profile(
        scheduled,
        RuntimeCoverageProfile {
            reasoning_mode: ReasoningModeSlot::Native,
            tool_mode: ToolModeSlot::Native,
        },
    );
    assert!(matches!(
        actual,
        ScheduledAction::Write {
            subagent_count: 2,
            reasoning_mode: ReasoningModeSlot::Native,
            tool_mode: ToolModeSlot::Native,
            world_info_route: WorldInfoRouteSlot::Selective,
        }
    ));
}

#[test]
fn regenerate_and_early_fact_rows_name_the_actual_operation() {
    let overall = default_required_for(&ScheduledAction::RegenerateOverall);
    assert!(overall.contains(&ObservationKey::ToolEvent("regenerate:overall".into())));
    for role in [
        "director",
        "subagent",
        "editor",
        "summarizer",
        "postprocessor",
    ] {
        assert!(overall.contains(&ObservationKey::AgentRole(role.into())));
    }

    let editor = default_required_for(&ScheduledAction::RegenerateEditor);
    assert!(editor.contains(&ObservationKey::ToolEvent("regenerate:editor_only".into())));
    assert!(editor.contains(&ObservationKey::AgentRole("editor".into())));
    assert!(editor.contains(&ObservationKey::AgentRole("director".into())));
    assert!(editor.contains(&ObservationKey::AgentRole("subagent".into())));

    let subagent = default_required_for(&ScheduledAction::RegenerateSubagent);
    assert!(subagent.contains(&ObservationKey::ToolEvent(
        "regenerate:subagent_only".into()
    )));
    assert!(subagent.contains(&ObservationKey::AgentRole("subagent".into())));
    assert!(subagent.contains(&ObservationKey::AgentRole("editor".into())));
    assert!(subagent.contains(&ObservationKey::AgentRole("director".into())));

    let injected = default_required_for(&ScheduledAction::EarlyFactInject {
        probe_id: "EF-ALPHA-4471".into(),
    });
    assert!(injected.contains(&ObservationKey::ToolEvent(
        "early_fact:injected:EF-ALPHA-4471".into()
    )));

    let checked = default_required_for(&ScheduledAction::EarlyFactCheck {
        probe_id: "EF-ALPHA-4471".into(),
    });
    assert!(checked.contains(&ObservationKey::EarlyFactChecked));
    assert!(checked.contains(&ObservationKey::ToolEvent(
        "early_fact:sqlite_reachable_and_remote_tool_succeeded:EF-ALPHA-4471".into()
    )));
}

//! Gate B deterministic proof: SQLite fixture + coverage ledger exact-set.
//!
//! No real model. Does not activate the process SQLite OnceLock (unit-level
//! ledger/fixture only). Lifecycle UoW proof lives in tauri-app Gate A tests.

use harness_real_llm::coverage_ledger::{
    CoverageLedger, ObservationKey, ObservedCoverage, SqlitePostcondition,
};
use harness_real_llm::endurance::{
    EnduranceSchedule, EnduranceStage, ReasoningModeSlot, ScheduledAction, ToolModeSlot,
    WorldInfoRouteSlot,
};

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
        ledger.record(ObservedCoverage {
            row_id: row.row_id.clone(),
            turn_index: row.turn_index,
            command_path: "sqlite_runtime::create_draft_attempt".into(),
            service_path: "pipeline.start_writing".into(),
            agent_events: vec!["pipeline".into()],
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

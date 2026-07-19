//! Coverage ledger: planned CoverageRow vs observed exact-set for SQLite M5.
//!
//! Plan tags alone do not count. A row is satisfied only when the actual
//! command/service path, agent events, and SQLite postconditions are recorded
//! and the expected observation set equals the observed set.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::endurance::{PrivateProbeKind, ReasoningModeSlot, ScheduledAction, ToolModeSlot};
use crate::evidence::AssertionResult;

/// Safe observation keys (no story text / secrets).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKey {
    CommandPath(String),
    ServicePath(String),
    AgentRole(String),
    ToolEvent(String),
    SqliteAuthoritative,
    JsonFallbackFalse,
    DraftLanded,
    AutofixSynced,
    PostprocessApplied,
    PostprocessFailed,
    Accepted,
    OutboxKind(String),
    Regenerated,
    EditStale,
    EarlyFactChecked,
    CoverageLabel(String),
}

impl ObservationKey {
    pub fn as_tag(&self) -> String {
        match self {
            Self::CommandPath(s) => format!("cmd:{s}"),
            Self::ServicePath(s) => format!("svc:{s}"),
            Self::AgentRole(s) => format!("role:{s}"),
            Self::ToolEvent(s) => format!("tool:{s}"),
            Self::SqliteAuthoritative => "sqlite_authoritative".into(),
            Self::JsonFallbackFalse => "json_fallback_false".into(),
            Self::DraftLanded => "draft_landed".into(),
            Self::AutofixSynced => "autofix_synced".into(),
            Self::PostprocessApplied => "postprocess_applied".into(),
            Self::PostprocessFailed => "postprocess_failed".into(),
            Self::Accepted => "accepted".into(),
            Self::OutboxKind(s) => format!("outbox:{s}"),
            Self::Regenerated => "regenerated".into(),
            Self::EditStale => "edit_stale".into(),
            Self::EarlyFactChecked => "early_fact_checked".into(),
            Self::CoverageLabel(s) => format!("label:{s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageRow {
    pub row_id: String,
    pub turn_index: u32,
    pub planned: ScheduledAction,
    pub required_observations: BTreeSet<ObservationKey>,
}

/// Sanitized run-level modes resolved from the actual LLM connection. Tool
/// transport and native reasoning are connection properties, not per-turn
/// schedule labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCoverageProfile {
    pub reasoning_mode: ReasoningModeSlot,
    pub tool_mode: ToolModeSlot,
}

pub fn action_with_runtime_profile(
    action: ScheduledAction,
    profile: RuntimeCoverageProfile,
) -> ScheduledAction {
    match action {
        ScheduledAction::Write {
            subagent_count,
            world_info_route,
            ..
        } => ScheduledAction::Write {
            subagent_count,
            reasoning_mode: profile.reasoning_mode,
            tool_mode: profile.tool_mode,
            world_info_route,
        },
        other => other,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SqlitePostcondition {
    pub sqlite_authoritative: bool,
    pub json_fallback: bool,
    pub turn_status: String,
    pub attempt_status: String,
    pub outbox_kinds: Vec<String>,
    pub campaign_revision_after: u64,
    pub postprocess_applied: bool,
    pub batch_digest16: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedCoverage {
    pub row_id: String,
    pub turn_index: u32,
    pub command_path: String,
    pub service_path: String,
    pub agent_events: Vec<String>,
    pub turn_id16: String,
    pub attempt_id16: String,
    pub variant_id16: String,
    pub sqlite_post: SqlitePostcondition,
    pub observations: BTreeSet<ObservationKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoverageLedger {
    pub planned: Vec<CoverageRow>,
    pub observed: Vec<ObservedCoverage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerMismatch {
    Missing {
        row_id: String,
        missing: BTreeSet<String>,
    },
    Extra {
        row_id: String,
        extra: BTreeSet<String>,
    },
    Unobserved {
        row_id: String,
    },
    Duplicate {
        row_id: String,
    },
    UnexpectedObserved {
        row_id: String,
    },
    Postcondition {
        row_id: String,
        reason: String,
    },
    AgentEvents {
        row_id: String,
        missing: BTreeSet<String>,
    },
}

impl fmt::Display for LedgerMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { row_id, missing } => {
                write!(f, "row {row_id} missing observations: {missing:?}")
            }
            Self::Extra { row_id, extra } => {
                write!(f, "row {row_id} extra observations: {extra:?}")
            }
            Self::Unobserved { row_id } => write!(f, "row {row_id} planned but never observed"),
            Self::Duplicate { row_id } => write!(f, "row {row_id} observed more than once"),
            Self::UnexpectedObserved { row_id } => {
                write!(f, "row {row_id} observed but not planned")
            }
            Self::Postcondition { row_id, reason } => {
                write!(f, "row {row_id} SQLite postcondition failed: {reason}")
            }
            Self::AgentEvents { row_id, missing } => {
                write!(
                    f,
                    "row {row_id} claimed agent roles without recorded events: {missing:?}"
                )
            }
        }
    }
}

impl CoverageLedger {
    pub fn new(planned: Vec<CoverageRow>) -> Self {
        Self {
            planned,
            observed: Vec::new(),
        }
    }

    pub fn plan_from_schedule(
        schedule_actions: impl IntoIterator<Item = (u32, ScheduledAction)>,
    ) -> Self {
        let planned = schedule_actions
            .into_iter()
            .map(|(turn_index, planned)| {
                let row_id = format!("t{turn_index:03}-{}", action_slug(&planned));
                let required_observations = default_required_for(&planned);
                CoverageRow {
                    row_id,
                    turn_index,
                    planned,
                    required_observations,
                }
            })
            .collect();
        Self::new(planned)
    }

    pub fn record(&mut self, observed: ObservedCoverage) {
        self.observed.push(observed);
    }

    pub fn exact_set_verify(&self) -> Result<(), Vec<LedgerMismatch>> {
        let mut mismatches = Vec::new();
        let mut seen = BTreeSet::new();

        for obs in &self.observed {
            if !seen.insert(obs.row_id.clone()) {
                mismatches.push(LedgerMismatch::Duplicate {
                    row_id: obs.row_id.clone(),
                });
            }
            if !self.planned.iter().any(|p| p.row_id == obs.row_id) {
                mismatches.push(LedgerMismatch::UnexpectedObserved {
                    row_id: obs.row_id.clone(),
                });
            }
            if !obs.sqlite_post.sqlite_authoritative || obs.sqlite_post.json_fallback {
                mismatches.push(LedgerMismatch::Postcondition {
                    row_id: obs.row_id.clone(),
                    reason: format!(
                        "sqlite_authoritative={} json_fallback={}",
                        obs.sqlite_post.sqlite_authoritative, obs.sqlite_post.json_fallback
                    ),
                });
            }
        }

        for plan in &self.planned {
            let matches: Vec<_> = self
                .observed
                .iter()
                .filter(|o| o.row_id == plan.row_id)
                .collect();
            if matches.is_empty() {
                mismatches.push(LedgerMismatch::Unobserved {
                    row_id: plan.row_id.clone(),
                });
                continue;
            }
            let obs = matches[0];
            let required: BTreeSet<String> = plan
                .required_observations
                .iter()
                .map(ObservationKey::as_tag)
                .collect();
            let got: BTreeSet<String> = obs
                .observations
                .iter()
                .map(ObservationKey::as_tag)
                .collect();
            let mut missing: BTreeSet<String> = required.difference(&got).cloned().collect();
            let mut extra: BTreeSet<String> = got.difference(&required).cloned().collect();
            // Postprocess is explicitly best-effort. A durable failed
            // postprocess is an auditable degraded outcome, not an applied
            // outcome; accept the mutually exclusive marker without claiming
            // that the mutation was applied.
            if !obs.sqlite_post.postprocess_applied
                && missing.contains("postprocess_applied")
                && extra.contains("postprocess_failed")
            {
                missing.remove("postprocess_applied");
                missing.remove("outbox:postprocess_apply");
                extra.remove("postprocess_failed");
            }
            if !missing.is_empty() {
                mismatches.push(LedgerMismatch::Missing {
                    row_id: plan.row_id.clone(),
                    missing,
                });
            }
            if !extra.is_empty() {
                mismatches.push(LedgerMismatch::Extra {
                    row_id: plan.row_id.clone(),
                    extra,
                });
            }

            let required_agent_events: BTreeSet<String> = plan
                .required_observations
                .iter()
                .filter_map(|observation| match observation {
                    ObservationKey::AgentRole(role) => Some(role.clone()),
                    _ => None,
                })
                .collect();
            let actual_agent_events: BTreeSet<String> = obs.agent_events.iter().cloned().collect();
            let missing_agent_events: BTreeSet<String> = required_agent_events
                .difference(&actual_agent_events)
                .cloned()
                .collect();
            if !missing_agent_events.is_empty() {
                mismatches.push(LedgerMismatch::AgentEvents {
                    row_id: plan.row_id.clone(),
                    missing: missing_agent_events,
                });
            }
        }

        if mismatches.is_empty() {
            Ok(())
        } else {
            Err(mismatches)
        }
    }

    pub fn assertion_results(&self) -> Vec<AssertionResult> {
        match self.exact_set_verify() {
            Ok(()) => vec![AssertionResult {
                name: "coverage_ledger_exact_set".into(),
                passed: true,
                detail: Some(format!(
                    "planned={} observed={}",
                    self.planned.len(),
                    self.observed.len()
                )),
            }],
            Err(ms) => ms
                .into_iter()
                .map(|m| AssertionResult {
                    name: "coverage_ledger_exact_set".into(),
                    passed: false,
                    detail: Some(m.to_string()),
                })
                .collect(),
        }
    }
}

fn action_slug(action: &ScheduledAction) -> String {
    match action {
        ScheduledAction::Write { .. } => "write".into(),
        ScheduledAction::RegenerateOverall => "regen_overall".into(),
        ScheduledAction::RegenerateEditor => "regen_editor".into(),
        ScheduledAction::RegenerateSubagent => "regen_subagent".into(),
        ScheduledAction::PrivateProbe { .. } => "private_probe".into(),
        ScheduledAction::EarlyFactInject { .. } => "early_fact_inject".into(),
        ScheduledAction::EarlyFactCheck { .. } => "early_fact_check".into(),
        ScheduledAction::QualityAutofix { .. } => "quality_autofix".into(),
        ScheduledAction::CacheStable => "cache_stable".into(),
        ScheduledAction::CacheInvalidate => "cache_invalidate".into(),
    }
}

/// Baseline required observations for SQLite production-faithful writes.
pub fn default_required_for(action: &ScheduledAction) -> BTreeSet<ObservationKey> {
    let mut set = BTreeSet::new();
    set.insert(ObservationKey::SqliteAuthoritative);
    set.insert(ObservationKey::JsonFallbackFalse);
    set.insert(ObservationKey::Accepted);
    match action {
        ScheduledAction::Write {
            subagent_count,
            reasoning_mode,
            tool_mode,
            world_info_route,
        } => {
            insert_write_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::CoverageLabel(format!(
                "subagent_count:{subagent_count}"
            )));
            set.insert(ObservationKey::ToolEvent(format!(
                "reasoning_mode:{}",
                reasoning_mode.label()
            )));
            set.insert(ObservationKey::ToolEvent(format!(
                "tool_mode:{}",
                tool_mode.label()
            )));
            set.insert(ObservationKey::ToolEvent(format!(
                "world_info_route_available:{}",
                world_info_route.label()
            )));
        }
        ScheduledAction::RegenerateOverall => {
            insert_regenerate_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent("regenerate:overall".into()));
        }
        ScheduledAction::RegenerateEditor => {
            insert_regenerate_lifecycle(&mut set);
            // The production-faithful harness first lands a complete draft,
            // then performs the Editor-only regenerate UoW.
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent("regenerate:editor_only".into()));
        }
        ScheduledAction::RegenerateSubagent => {
            insert_regenerate_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent("regenerate:subagent_only".into()));
        }
        ScheduledAction::PrivateProbe { probe_kind } => {
            insert_write_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent(format!(
                "private_final_output:{}:no_leak",
                private_probe_slug(probe_kind)
            )));
        }
        ScheduledAction::EarlyFactInject { probe_id } => {
            insert_write_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent(format!(
                "early_fact:injected:{probe_id}"
            )));
        }
        ScheduledAction::QualityAutofix { fixable } => {
            insert_write_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent("quality_gate:evaluated".into()));
            if *fixable {
                set.insert(ObservationKey::AutofixSynced);
                set.insert(ObservationKey::OutboxKind("autofix_sync".into()));
                set.insert(ObservationKey::ToolEvent("autofix:fixed".into()));
            } else {
                set.insert(ObservationKey::ToolEvent("autofix:rejected".into()));
            }
        }
        ScheduledAction::CacheStable => {
            insert_write_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent("cache:stable".into()));
        }
        ScheduledAction::CacheInvalidate => {
            insert_write_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
            set.insert(ObservationKey::ToolEvent("cache:invalidated".into()));
        }
        ScheduledAction::EarlyFactCheck { probe_id } => {
            set.insert(ObservationKey::CommandPath(
                "sqlite_runtime::list_summaries".into(),
            ));
            set.insert(ObservationKey::ServicePath("early_fact_check".into()));
            set.insert(ObservationKey::EarlyFactChecked);
            set.insert(ObservationKey::ToolEvent(format!(
                "early_fact:sqlite_reachable_and_remote_tool_succeeded:{probe_id}"
            )));
            // Early-fact check turns still write+accept to keep continuity.
            insert_write_lifecycle(&mut set);
            insert_full_pipeline_agent_events(&mut set);
        }
    }
    set
}

fn insert_write_lifecycle(set: &mut BTreeSet<ObservationKey>) {
    set.insert(ObservationKey::CommandPath(
        "sqlite_runtime::create_draft_attempt".into(),
    ));
    set.insert(ObservationKey::ServicePath("pipeline.start_writing".into()));
    set.insert(ObservationKey::DraftLanded);
    set.insert(ObservationKey::PostprocessApplied);
    set.insert(ObservationKey::OutboxKind("draft_ready".into()));
    set.insert(ObservationKey::OutboxKind("postprocess_apply".into()));
    set.insert(ObservationKey::ToolEvent("quality_gate:evaluated".into()));
}

fn insert_regenerate_lifecycle(set: &mut BTreeSet<ObservationKey>) {
    set.insert(ObservationKey::CommandPath(
        "sqlite_runtime::append_regenerate_attempt".into(),
    ));
    set.insert(ObservationKey::ServicePath("pipeline.regenerate".into()));
    set.insert(ObservationKey::Regenerated);
    set.insert(ObservationKey::PostprocessApplied);
    set.insert(ObservationKey::OutboxKind("regenerate".into()));
    set.insert(ObservationKey::OutboxKind("postprocess_apply".into()));
    set.insert(ObservationKey::ToolEvent("quality_gate:evaluated".into()));
}

fn insert_full_pipeline_agent_events(set: &mut BTreeSet<ObservationKey>) {
    insert_agent_events(
        set,
        &[
            "director",
            "subagent",
            "editor",
            "summarizer",
            "postprocessor",
        ],
    );
}

fn insert_agent_events(set: &mut BTreeSet<ObservationKey>, roles: &[&str]) {
    for role in roles {
        set.insert(ObservationKey::AgentRole((*role).into()));
    }
}

fn private_probe_slug(probe_kind: &PrivateProbeKind) -> &'static str {
    match probe_kind {
        PrivateProbeKind::OwnerRecall => "owner_recall",
        PrivateProbeKind::NonOwnerLeak => "non_owner_leak",
        PrivateProbeKind::NarrationLeak => "narration_leak",
        PrivateProbeKind::MustNotReveal => "must_not_reveal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endurance::{ReasoningModeSlot, ToolModeSlot, WorldInfoRouteSlot};

    fn write_action() -> ScheduledAction {
        ScheduledAction::Write {
            subagent_count: 1,
            reasoning_mode: ReasoningModeSlot::Disabled,
            tool_mode: ToolModeSlot::Native,
            world_info_route: WorldInfoRouteSlot::Constant,
        }
    }

    #[test]
    fn exact_set_passes_when_planned_equals_observed() {
        let mut ledger = CoverageLedger::plan_from_schedule([(1, write_action())]);
        let row = &ledger.planned[0];
        let observations = row.required_observations.clone();
        let agent_events = observations
            .iter()
            .filter_map(|observation| match observation {
                ObservationKey::AgentRole(role) => Some(role.clone()),
                _ => None,
            })
            .collect();
        ledger.record(ObservedCoverage {
            row_id: row.row_id.clone(),
            turn_index: 1,
            command_path: "sqlite_runtime::create_draft_attempt".into(),
            service_path: "pipeline.start_writing".into(),
            agent_events,
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
                batch_digest16: Some("deadbeef".into()),
            },
            observations,
        });
        assert!(ledger.exact_set_verify().is_ok());
    }

    #[test]
    fn exact_set_fails_on_missing_observation() {
        let mut ledger = CoverageLedger::plan_from_schedule([(1, write_action())]);
        let row = &ledger.planned[0];
        let mut observations = row.required_observations.clone();
        observations.remove(&ObservationKey::PostprocessApplied);
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
                sqlite_authoritative: true,
                json_fallback: false,
                ..Default::default()
            },
            observations,
        });
        let err = ledger.exact_set_verify().unwrap_err();
        assert!(
            err.iter()
                .any(|m| matches!(m, LedgerMismatch::Missing { .. }))
        );
    }

    #[test]
    fn exact_set_accepts_audited_degraded_postprocess_without_claiming_apply() {
        let mut ledger = CoverageLedger::plan_from_schedule([(1, write_action())]);
        let row = &ledger.planned[0];
        let mut observations = row.required_observations.clone();
        observations.remove(&ObservationKey::PostprocessApplied);
        observations.insert(ObservationKey::PostprocessFailed);
        let agent_events = observations
            .iter()
            .filter_map(|observation| match observation {
                ObservationKey::AgentRole(role) => Some(role.clone()),
                _ => None,
            })
            .collect();
        ledger.record(ObservedCoverage {
            row_id: row.row_id.clone(),
            turn_index: 1,
            command_path: "sqlite_runtime::create_draft_attempt".into(),
            service_path: "pipeline.start_writing".into(),
            agent_events,
            turn_id16: "t".into(),
            attempt_id16: "a".into(),
            variant_id16: "v".into(),
            sqlite_post: SqlitePostcondition {
                sqlite_authoritative: true,
                json_fallback: false,
                postprocess_applied: false,
                outbox_kinds: vec!["draft_ready".into()],
                ..Default::default()
            },
            observations,
        });
        assert!(ledger.exact_set_verify().is_ok());
    }
}

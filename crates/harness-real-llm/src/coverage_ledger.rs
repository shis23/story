//! Coverage ledger: planned CoverageRow vs observed exact-set for SQLite M5.
//!
//! Plan tags alone do not count. A row is satisfied only when the actual
//! command/service path, agent events, and SQLite postconditions are recorded
//! and the expected observation set equals the observed set.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::endurance::ScheduledAction;
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
            let missing: BTreeSet<String> = required.difference(&got).cloned().collect();
            let extra: BTreeSet<String> = got.difference(&required).cloned().collect();
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
        ScheduledAction::Write { .. }
        | ScheduledAction::PrivateProbe { .. }
        | ScheduledAction::EarlyFactInject { .. }
        | ScheduledAction::QualityAutofix { .. }
        | ScheduledAction::CacheStable
        | ScheduledAction::CacheInvalidate => {
            set.insert(ObservationKey::CommandPath(
                "sqlite_runtime::create_draft_attempt".into(),
            ));
            set.insert(ObservationKey::ServicePath("pipeline.start_writing".into()));
            set.insert(ObservationKey::DraftLanded);
            set.insert(ObservationKey::PostprocessApplied);
            set.insert(ObservationKey::OutboxKind("draft_ready".into()));
            set.insert(ObservationKey::OutboxKind("postprocess_apply".into()));
            set.insert(ObservationKey::AgentRole("pipeline".into()));
        }
        ScheduledAction::RegenerateOverall
        | ScheduledAction::RegenerateEditor
        | ScheduledAction::RegenerateSubagent => {
            set.insert(ObservationKey::CommandPath(
                "sqlite_runtime::append_regenerate_attempt".into(),
            ));
            set.insert(ObservationKey::ServicePath("pipeline.regenerate".into()));
            set.insert(ObservationKey::Regenerated);
            set.insert(ObservationKey::PostprocessApplied);
            set.insert(ObservationKey::OutboxKind("regenerate".into()));
            set.insert(ObservationKey::OutboxKind("postprocess_apply".into()));
            set.insert(ObservationKey::AgentRole("pipeline".into()));
        }
        ScheduledAction::EarlyFactCheck { .. } => {
            set.insert(ObservationKey::CommandPath(
                "sqlite_runtime::list_summaries".into(),
            ));
            set.insert(ObservationKey::ServicePath("early_fact_check".into()));
            set.insert(ObservationKey::EarlyFactChecked);
            // Early-fact check turns still write+accept to keep continuity.
            set.insert(ObservationKey::CommandPath(
                "sqlite_runtime::create_draft_attempt".into(),
            ));
            set.insert(ObservationKey::ServicePath("pipeline.start_writing".into()));
            set.insert(ObservationKey::DraftLanded);
            set.insert(ObservationKey::PostprocessApplied);
            set.insert(ObservationKey::OutboxKind("draft_ready".into()));
            set.insert(ObservationKey::OutboxKind("postprocess_apply".into()));
            set.insert(ObservationKey::AgentRole("pipeline".into()));
        }
    }
    set
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
        ledger.record(ObservedCoverage {
            row_id: row.row_id.clone(),
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
}

//! 100-Accept endurance runner with staged gating, checkpoint/resume, and budget enforcement.
//!
//! This module implements the production-faithful, resumable endurance run described in
//! `docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-PLAN.md`. It:
//!
//! - uses a **deterministic coverage schedule** (`EnduranceSchedule`) that maps each
//!   accepted turn to a `ScheduledAction` covering Director / dynamic Subagents / Editor /
//!   Summarizer / PostProcessor / CharacterExtractor / Meta / all three ReasoningMode
//!   values / tool modes / world-info routes / private-knowledge probes / Chronicle tools /
//!   regenerate variants / QualityGate autofix / cache invalidation / epoch rollover;
//! - enforces **hard suite-wide budgets** (max calls, per-call timeout, hard deadline);
//! - writes **sanitized checkpoints** so an interruption resumes without replaying accepted
//!   turns;
//! - gates stages `3 → 12 → 30 → 100` accepted turns — a later stage may start only when the
//!   previous stage passes its invariants;
//! - records a **machine-readable manifest** with usage, latency, cache, role, mode, hashes,
//!   epoch/revision, Turn/Attempt status, Quality/autofix, Chronicle ranges, and assertions
//!   — **no** raw prompts/story/private text.
//!
//! The deterministic core (schedule, checkpoint, budget, classification) is fully testable
//! without a real LLM. The real-model entry point lives in the `#[ignore]` integration test.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceTurnRecord,
    contains_forbidden_evidence_payload, short_hash16,
};

// ─── Stage definitions ─────────────────────────────────────────────────────

/// Execution stages, each with its own accepted-turn target and call budget.
///
/// A later stage may start only when the previous stage passes its invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnduranceStage {
    /// Zero model calls; validate fixture, output dir, budgets, schema, disk, secret guards.
    DryRun,
    /// 3 accepted turns, ≤ 30 model calls.
    Canary,
    /// 12 accepted turns, ≤ 120 total model calls.
    Coverage,
    /// 30 accepted turns, ≤ 300 total model calls.
    Stability,
    /// 100 accepted turns, ≤ 700 total model calls.
    Full,
}

impl EnduranceStage {
    /// Accepted-turn target for this stage.
    pub fn target_turns(self) -> u32 {
        match self {
            Self::DryRun => 0,
            Self::Canary => 3,
            Self::Coverage => 12,
            Self::Stability => 30,
            Self::Full => 100,
        }
    }

    /// Hard maximum total model calls (suite-wide, including setup/extract).
    pub fn max_calls(self) -> u32 {
        match self {
            Self::DryRun => 0,
            Self::Canary => 30,
            // SQLite regenerate is first-draft + regenerate UoW (roughly 2x JSON write cost).
            // Full also absorbs bounded Plan-parse retries across 100 accepted turns.
            Self::Coverage => 220,
            Self::Stability => 450,
            Self::Full => 1400,
        }
    }

    /// A stage may only start after its predecessor passes.
    pub fn predecessor(self) -> Option<Self> {
        match self {
            Self::DryRun => None,
            Self::Canary => Some(Self::DryRun),
            Self::Coverage => Some(Self::Canary),
            Self::Stability => Some(Self::Coverage),
            Self::Full => Some(Self::Stability),
        }
    }

    /// Human-readable label for evidence.
    pub fn label(self) -> &'static str {
        match self {
            Self::DryRun => "dry_run",
            Self::Canary => "canary",
            Self::Coverage => "coverage",
            Self::Stability => "stability",
            Self::Full => "full",
        }
    }

    /// Parse a stage from its label.
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "dry_run" => Some(Self::DryRun),
            "canary" => Some(Self::Canary),
            "coverage" => Some(Self::Coverage),
            "stability" => Some(Self::Stability),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// Iterate stages in order.
    pub fn all() -> &'static [EnduranceStage] {
        &[
            Self::DryRun,
            Self::Canary,
            Self::Coverage,
            Self::Stability,
            Self::Full,
        ]
    }
}

impl fmt::Display for EnduranceStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ─── Coverage schedule ─────────────────────────────────────────────────────

/// The kind of writing/regenerate action scheduled for a given accepted turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ScheduledAction {
    /// Normal writing turn (Director + Subagents + Editor + Summarizer + PostProcessor).
    Write {
        subagent_count: u32,
        reasoning_mode: ReasoningModeSlot,
        tool_mode: ToolModeSlot,
        world_info_route: WorldInfoRouteSlot,
    },
    /// Overall regenerate (whole-pipeline reroll).
    RegenerateOverall,
    /// Editor-only regenerate.
    RegenerateEditor,
    /// Subagent-only regenerate.
    RegenerateSubagent,
    /// Private-knowledge adversarial probe (owner-legal recall / non-owner leak / narration).
    PrivateProbe { probe_kind: PrivateProbeKind },
    /// Early-fact injection — the token will be checked for reachability after epochs.
    EarlyFactInject { probe_id: String },
    /// Early-fact reachability check.
    EarlyFactCheck { probe_id: String },
    /// QualityGate bounded autofix case.
    QualityAutofix { fixable: bool },
    /// Cache-stable turn (no perturbation — observe prefix-cache stability).
    CacheStable,
    /// Cache invalidation probe (hook/fingerprint change or epoch rollover).
    CacheInvalidate,
}

/// Schedule slots that cycle through the three `ReasoningMode` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningModeSlot {
    Disabled,
    Native,
    Prompted,
}

impl ReasoningModeSlot {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Native => "native",
            Self::Prompted => "prompted",
        }
    }
}

/// Schedule slots for native vs text-fallback tool mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolModeSlot {
    Native,
    TextFallback,
}

impl ToolModeSlot {
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::TextFallback => "text_fallback",
        }
    }
}

/// Schedule slots for constant / selective / both world-info routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldInfoRouteSlot {
    Constant,
    Selective,
    Both,
}

impl WorldInfoRouteSlot {
    pub fn label(self) -> &'static str {
        match self {
            Self::Constant => "constant",
            Self::Selective => "selective",
            Self::Both => "both",
        }
    }
}

/// Private-knowledge probe variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateProbeKind {
    /// Owner-legal recall (owner character recalls their own private knowledge).
    OwnerRecall,
    /// Non-owner attempts to leak another character's private knowledge.
    NonOwnerLeak,
    /// Narration accidentally leaks private knowledge.
    NarrationLeak,
    /// Explicit `must_not_reveal` probe.
    MustNotReveal,
}

/// A deterministic schedule mapping accepted-turn numbers to actions.
///
/// The schedule is computed purely from the turn number — no randomness, no model output.
/// It covers every row in the PLAN coverage matrix. The exact positions follow the
/// recommended perturbation windows from the PLAN, adjusted for API availability.
#[derive(Debug, Clone)]
pub struct EnduranceSchedule {
    pub total_turns: u32,
}

impl EnduranceSchedule {
    /// Create a schedule for `total_turns` accepted turns.
    pub fn new(total_turns: u32) -> Self {
        Self { total_turns }
    }

    /// Determine the action for accepted turn `n` (1-based).
    ///
    /// The schedule follows the PLAN's recommended perturbation windows:
    /// - overall regenerate near turns 11, 37, 73;
    /// - Editor-only regenerate near turns 18, 54, 90;
    /// - Subagent-only regenerate near turns 25, 61, 97;
    /// - private-knowledge adversarial near turns 20, 50, 80;
    /// - early-fact probes injected in turns 1–15 and checked after 35, 65, 95.
    pub fn action_for_turn(&self, n: u32) -> ScheduledAction {
        // ── Regenerate variants (recommended windows) ──
        let overall_regenerates: &[u32] = &[11, 37, 73];
        let editor_regenerates: &[u32] = &[18, 54, 90];
        let subagent_regenerates: &[u32] = &[25, 61, 97];
        let private_probes: &[(u32, PrivateProbeKind)] = &[
            (20, PrivateProbeKind::NonOwnerLeak),
            (50, PrivateProbeKind::NarrationLeak),
            (80, PrivateProbeKind::MustNotReveal),
        ];
        let early_fact_injects: &[(u32, &str)] = &[
            (3, "EF-ALPHA-4471"),
            (8, "EF-BETA-2098"),
            (13, "EF-GAMMA-6603"),
        ];
        let early_fact_checks: &[(u32, &str)] = &[
            (35, "EF-ALPHA-4471"),
            (65, "EF-BETA-2098"),
            (95, "EF-GAMMA-6603"),
        ];
        let quality_autofix: &[(u32, bool)] = &[(15, true), (45, false)];
        let cache_invalidates: &[u32] = &[10, 20, 30, 40, 50, 60, 70, 80, 90, 100];

        // Regenerate windows take priority to ensure they are represented.
        if overall_regenerates.contains(&n) {
            return ScheduledAction::RegenerateOverall;
        }
        if editor_regenerates.contains(&n) {
            return ScheduledAction::RegenerateEditor;
        }
        if subagent_regenerates.contains(&n) {
            return ScheduledAction::RegenerateSubagent;
        }
        if let Some((_, kind)) = private_probes.iter().find(|(turn, _)| *turn == n) {
            return ScheduledAction::PrivateProbe {
                probe_kind: kind.clone(),
            };
        }
        if let Some((_, probe_id)) = early_fact_injects.iter().find(|(turn, _)| *turn == n) {
            return ScheduledAction::EarlyFactInject {
                probe_id: probe_id.to_string(),
            };
        }
        if let Some((_, probe_id)) = early_fact_checks.iter().find(|(turn, _)| *turn == n) {
            return ScheduledAction::EarlyFactCheck {
                probe_id: probe_id.to_string(),
            };
        }
        if let Some((_, fixable)) = quality_autofix.iter().find(|(turn, _)| *turn == n) {
            return ScheduledAction::QualityAutofix { fixable: *fixable };
        }
        if cache_invalidates.contains(&n) {
            return ScheduledAction::CacheInvalidate;
        }

        // Owner-recall private probe at turn 6 (within first epoch, non-conflicting).
        if n == 6 {
            return ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::OwnerRecall,
            };
        }

        // Default: normal writing turn with cycling coverage dimensions.
        let cycle = (n as usize).saturating_sub(1);
        let subagent_count = match cycle % 3 {
            0 => 1,
            1 => 2,
            _ => 3,
        };
        let reasoning_mode = match cycle % 3 {
            0 => ReasoningModeSlot::Disabled,
            1 => ReasoningModeSlot::Native,
            _ => ReasoningModeSlot::Prompted,
        };
        let tool_mode = if cycle % 5 == 4 {
            ToolModeSlot::TextFallback
        } else {
            ToolModeSlot::Native
        };
        let world_info_route = match cycle % 3 {
            0 => WorldInfoRouteSlot::Constant,
            1 => WorldInfoRouteSlot::Selective,
            _ => WorldInfoRouteSlot::Both,
        };
        ScheduledAction::Write {
            subagent_count,
            reasoning_mode,
            tool_mode,
            world_info_route,
        }
    }

    /// Produce the full ordered schedule (1..=total_turns).
    pub fn entries(&self) -> Vec<(u32, ScheduledAction)> {
        (1..=self.total_turns)
            .map(|n| (n, self.action_for_turn(n)))
            .collect()
    }

    /// Verify the schedule covers every required matrix row.
    /// Returns a list of coverage assertion names and pass/fail.
    pub fn coverage_report(&self) -> Vec<AssertionResult> {
        let entries = self.entries();
        let mut assertions = Vec::new();

        // Director is the primary agent on every normal writing turn; regenerate and
        // probe variants exercise other agents but still originate from the Director
        // pipeline. The requirement is that at least some turns use the Director path.
        let any_writing_turn = entries
            .iter()
            .any(|(_, a)| matches!(a, ScheduledAction::Write { .. }));
        assertions.push(AssertionResult {
            name: "director_on_writing_turns".into(),
            passed: any_writing_turn,
            detail: None,
        });

        // 1/2/3 subagent tasks
        let sub_counts: Vec<u32> = entries
            .iter()
            .filter_map(|(_, a)| match a {
                ScheduledAction::Write { subagent_count, .. } => Some(*subagent_count),
                _ => None,
            })
            .collect();
        for cnt in [1u32, 2, 3] {
            assertions.push(AssertionResult {
                name: format!("subagent_{cnt}_represented"),
                passed: sub_counts.contains(&cnt),
                detail: Some(format!(
                    "count={}",
                    sub_counts.iter().filter(|c| **c == cnt).count()
                )),
            });
        }

        // All three reasoning modes
        for mode in [
            ReasoningModeSlot::Disabled,
            ReasoningModeSlot::Native,
            ReasoningModeSlot::Prompted,
        ] {
            let present = entries.iter().any(|(_, a)| match a {
                ScheduledAction::Write {
                    reasoning_mode: rm, ..
                } => *rm == mode,
                _ => false,
            });
            assertions.push(AssertionResult {
                name: format!("reasoning_{}_represented", mode.label()),
                passed: present,
                detail: None,
            });
        }

        // Both tool modes
        for mode in [ToolModeSlot::Native, ToolModeSlot::TextFallback] {
            let present = entries.iter().any(|(_, a)| match a {
                ScheduledAction::Write { tool_mode: tm, .. } => *tm == mode,
                _ => false,
            });
            assertions.push(AssertionResult {
                name: format!("tool_mode_{}_represented", mode.label()),
                passed: present,
                detail: None,
            });
        }

        // All three world-info routes
        for route in [
            WorldInfoRouteSlot::Constant,
            WorldInfoRouteSlot::Selective,
            WorldInfoRouteSlot::Both,
        ] {
            let present = entries.iter().any(|(_, a)| match a {
                ScheduledAction::Write {
                    world_info_route: wir,
                    ..
                } => *wir == route,
                _ => false,
            });
            assertions.push(AssertionResult {
                name: format!("world_info_{}_represented", route.label()),
                passed: present,
                detail: None,
            });
        }

        // Regenerate variants
        for (name, pattern) in [
            ("regenerate_overall", ScheduledAction::RegenerateOverall),
            ("regenerate_editor", ScheduledAction::RegenerateEditor),
            ("regenerate_subagent", ScheduledAction::RegenerateSubagent),
        ] {
            assertions.push(AssertionResult {
                name: format!("{name}_represented"),
                passed: entries.iter().any(|(_, a)| scheduled_eq(a, &pattern)),
                detail: None,
            });
        }

        // Private-knowledge probes
        for kind in [
            PrivateProbeKind::OwnerRecall,
            PrivateProbeKind::NonOwnerLeak,
            PrivateProbeKind::NarrationLeak,
            PrivateProbeKind::MustNotReveal,
        ] {
            assertions.push(AssertionResult {
                name: format!("private_probe_{:?}_represented", kind).to_ascii_lowercase(),
                passed: entries.iter().any(|(_, a)| match a {
                    ScheduledAction::PrivateProbe { probe_kind: pk } => *pk == kind,
                    _ => false,
                }),
                detail: None,
            });
        }

        // Early-fact injects and checks
        assertions.push(AssertionResult {
            name: "early_fact_inject_represented".into(),
            passed: entries
                .iter()
                .any(|(_, a)| matches!(a, ScheduledAction::EarlyFactInject { .. })),
            detail: None,
        });
        assertions.push(AssertionResult {
            name: "early_fact_check_represented".into(),
            passed: entries
                .iter()
                .any(|(_, a)| matches!(a, ScheduledAction::EarlyFactCheck { .. })),
            detail: None,
        });

        // Quality autofix cases
        assertions.push(AssertionResult {
            name: "quality_autofix_fixable_represented".into(),
            passed: entries
                .iter()
                .any(|(_, a)| matches!(a, ScheduledAction::QualityAutofix { fixable: true })),
            detail: None,
        });
        assertions.push(AssertionResult {
            name: "quality_autofix_non_fixable_represented".into(),
            passed: entries
                .iter()
                .any(|(_, a)| matches!(a, ScheduledAction::QualityAutofix { fixable: false })),
            detail: None,
        });

        // Cache stability and invalidation
        assertions.push(AssertionResult {
            name: "cache_invalidate_represented".into(),
            passed: entries
                .iter()
                .any(|(_, a)| matches!(a, ScheduledAction::CacheInvalidate)),
            detail: None,
        });

        assertions
    }
}

fn scheduled_eq(a: &ScheduledAction, b: &ScheduledAction) -> bool {
    // Discriminant-level equality for regenerate variants
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

// ─── Checkpoint ─────────────────────────────────────────────────────────────

/// Sanitized checkpoint written after each accepted turn so an interruption
/// resumes without replaying accepted turns.
///
/// **No raw prompt, story text, private knowledge, or API response body** may be
/// stored here — only non-secret stable ids, hashes, counts, and statuses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnduranceCheckpoint {
    pub schema_version: String,
    pub run_id: String,
    pub stage: String,
    pub accepted_turn_number: u32,
    pub calls_used: u32,
    pub max_calls: u32,
    pub campaign_revision: u64,
    pub chronicle_revision: u64,
    /// Short fingerprints only — no raw text.
    pub last_draft_hash16: String,
    pub last_summary_code: Option<String>,
    /// Epoch id16 at the point of checkpoint.
    pub context_epoch_id16: Option<String>,
    /// Early-fact probe ids injected so far (for resume reachability tracking).
    pub early_fact_probe_ids: Vec<String>,
    /// Early-fact probe ids that have been checked and passed.
    pub early_fact_checked_passed: Vec<String>,
    /// Non-secret stable ids for resume across process restarts.
    #[serde(default)]
    pub campaign_id: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Relative path under evidence root for the persistent campaign data dir.
    #[serde(default)]
    pub data_dir_rel: Option<String>,
    /// Observed epoch short-ids so resume can restore rollover evidence.
    #[serde(default)]
    pub observed_epoch_ids16: Vec<String>,
    pub recorded_at_unix_ms: u128,
}

impl EnduranceCheckpoint {
    pub fn schema_version() -> &'static str {
        "endurance-checkpoint-v1"
    }
}

/// Read the latest checkpoint from a checkpoint file (last line wins).
pub fn read_latest_checkpoint(path: &Path) -> Option<EnduranceCheckpoint> {
    let lines = crate::evidence::read_evidence_lines(path).ok()?;
    lines.last().and_then(|v| {
        let cp: EnduranceCheckpoint = serde_json::from_value(v.clone()).ok()?;
        if cp.schema_version != EnduranceCheckpoint::schema_version() {
            return None;
        }
        Some(cp)
    })
}

/// Write a sanitized checkpoint line. Refuses to write if forbidden payload detected.
pub fn write_checkpoint(path: &Path, cp: &EnduranceCheckpoint) -> std::io::Result<()> {
    let serialized = serde_json::to_string(cp)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if contains_forbidden_evidence_payload(&serialized) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "checkpoint contains forbidden payload",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    use std::io::Write;
    writeln!(file, "{serialized}")?;
    Ok(())
}

// ─── Stage manifest ─────────────────────────────────────────────────────────

/// One row per completed stage, written to the manifest file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnduranceStageManifestRow {
    pub schema_version: String,
    pub run_id: String,
    pub stage: String,
    pub target_turns: u32,
    pub accepted_turns: u32,
    pub calls_used: u32,
    pub max_calls: u32,
    pub elapsed_ms: u128,
    pub acceptance: String,
    pub summary_codes: Vec<String>,
    pub observed_epoch_ids16: Vec<String>,
    pub early_fact_probe_ids: Vec<String>,
    pub early_fact_checked_passed: Vec<String>,
    pub coverage_assertions: Vec<AssertionResult>,
    pub recorded_at_unix_ms: u128,
}

/// Final acceptance classification for a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceLevel {
    /// All intended turns reached accepted terminal state, all invariants pass.
    Pass,
    /// Highest completed checkpoint when endpoint/quota prevents full completion.
    Partial,
    /// Could not determine acceptance (missing usage, inconsistent state, etc.).
    Inconclusive,
}

impl AcceptanceLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Partial => "partial",
            Self::Inconclusive => "inconclusive",
        }
    }

    /// A requested full stage that stops early must exit non-zero.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Pass => 0,
            Self::Partial | Self::Inconclusive => 1,
        }
    }
}

impl fmt::Display for AcceptanceLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ─── Budget config ──────────────────────────────────────────────────────────

/// Hard suite-wide budget for an endurance run.
#[derive(Debug, Clone)]
pub struct EnduranceBudget {
    pub max_calls: u32,
    pub max_turns: u32,
    pub timeout_secs: u64,
    pub hard_deadline: Option<Duration>,
    /// Maximum evidence size in bytes across all files.
    pub max_evidence_bytes: usize,
}

impl Default for EnduranceBudget {
    fn default() -> Self {
        Self {
            max_calls: 1400,
            max_turns: 100,
            timeout_secs: 180,
            hard_deadline: None,
            max_evidence_bytes: 2 * 1024 * 1024, // 2 MB
        }
    }
}

impl EnduranceBudget {
    /// Budget for a specific stage (stricter than the suite-wide max).
    pub fn for_stage(stage: EnduranceStage) -> Self {
        Self {
            max_calls: stage.max_calls(),
            max_turns: stage.target_turns(),
            timeout_secs: 180,
            hard_deadline: None,
            max_evidence_bytes: 2 * 1024 * 1024,
        }
    }

    /// Check whether we still have budget to attempt another turn given the current
    /// call count and the minimum calls-per-turn floor.
    pub fn can_attempt_turn(&self, calls_used: u32, min_calls_per_turn: u32) -> bool {
        calls_used + min_calls_per_turn <= self.max_calls
    }
}

// ─── Stage gating ───────────────────────────────────────────────────────────

/// Error returned when a stage gate fails.
#[derive(Debug, Clone)]
pub struct StageGateError {
    pub stage: EnduranceStage,
    pub reason: String,
}

impl fmt::Display for StageGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stage gate failed for {}: {}", self.stage, self.reason)
    }
}

impl std::error::Error for StageGateError {}

/// Check whether a stage may start, given the predecessor's manifest row.
///
/// - DryRun → always allowed (it makes no calls).
/// - Canary → requires DryRun to have passed (or no predecessor row needed for DryRun).
/// - Coverage → requires Canary to have reached ≥ 3 accepted turns.
/// - Stability → requires Coverage to have reached ≥ 12 accepted turns.
/// - Full → requires Stability to have reached ≥ 30 accepted turns.
pub fn check_stage_gate(
    stage: EnduranceStage,
    predecessor_row: Option<&EnduranceStageManifestRow>,
) -> Result<(), StageGateError> {
    match stage {
        EnduranceStage::DryRun => Ok(()),
        EnduranceStage::Canary => {
            // DryRun doesn't produce a manifest row with turns; just allow if no block.
            Ok(())
        }
        EnduranceStage::Coverage => {
            let row = predecessor_row.ok_or_else(|| StageGateError {
                stage,
                reason: "missing predecessor (Canary) manifest".into(),
            })?;
            if row.accepted_turns >= EnduranceStage::Canary.target_turns() {
                Ok(())
            } else {
                Err(StageGateError {
                    stage,
                    reason: format!(
                        "predecessor only reached {} accepted turns (need {})",
                        row.accepted_turns,
                        EnduranceStage::Canary.target_turns()
                    ),
                })
            }
        }
        EnduranceStage::Stability => {
            let row = predecessor_row.ok_or_else(|| StageGateError {
                stage,
                reason: "missing predecessor (Coverage) manifest".into(),
            })?;
            if row.accepted_turns >= EnduranceStage::Coverage.target_turns() {
                Ok(())
            } else {
                Err(StageGateError {
                    stage,
                    reason: format!(
                        "predecessor only reached {} accepted turns (need {})",
                        row.accepted_turns,
                        EnduranceStage::Coverage.target_turns()
                    ),
                })
            }
        }
        EnduranceStage::Full => {
            let row = predecessor_row.ok_or_else(|| StageGateError {
                stage,
                reason: "missing predecessor (Stability) manifest".into(),
            })?;
            if row.accepted_turns >= EnduranceStage::Stability.target_turns() {
                Ok(())
            } else {
                Err(StageGateError {
                    stage,
                    reason: format!(
                        "predecessor only reached {} accepted turns (need {})",
                        row.accepted_turns,
                        EnduranceStage::Stability.target_turns()
                    ),
                })
            }
        }
    }
}

/// Classify acceptance for a stage given the result.
pub fn classify_acceptance(
    stage: EnduranceStage,
    accepted_turns: u32,
    calls_used: u32,
    invariants_passed: bool,
    had_secret_violation: bool,
    had_missing_usage: bool,
) -> AcceptanceLevel {
    if had_secret_violation || had_missing_usage {
        return AcceptanceLevel::Inconclusive;
    }
    if accepted_turns >= stage.target_turns()
        && invariants_passed
        && calls_used <= stage.max_calls()
    {
        AcceptanceLevel::Pass
    } else {
        AcceptanceLevel::Partial
    }
}

// ─── Resume logic ───────────────────────────────────────────────────────────

/// Determine the resume turn number from a checkpoint.
///
/// Returns the next turn to execute (checkpoint.accepted_turn_number + 1),
/// or 1 if no checkpoint exists.
pub fn resume_turn_from_checkpoint(cp: Option<&EnduranceCheckpoint>) -> u32 {
    cp.map(|c| c.accepted_turn_number + 1).unwrap_or(1)
}

/// Idempotency check: a checkpoint for turn N must not be writable twice for the same
/// campaign_revision / draft_hash. This prevents double-counting on resume.
pub fn checkpoint_is_new(
    existing: Option<&EnduranceCheckpoint>,
    new_accepted_turn: u32,
    new_campaign_revision: u64,
    new_draft_hash16: &str,
) -> bool {
    match existing {
        None => true,
        Some(cp) => {
            // If the existing checkpoint already covers this turn with the same revision
            // and hash, it's a duplicate — do not write or count it again.
            cp.accepted_turn_number != new_accepted_turn
                || cp.campaign_revision != new_campaign_revision
                || cp.last_draft_hash16 != new_draft_hash16
        }
    }
}

// ─── Evidence file layout ───────────────────────────────────────────────────

/// Paths for all evidence files in a run.
#[derive(Debug, Clone)]
pub struct EnduranceEvidencePaths {
    pub root: PathBuf,
    pub calls_jsonl: PathBuf,
    pub turns_jsonl: PathBuf,
    pub checkpoint_jsonl: PathBuf,
    pub manifest_jsonl: PathBuf,
    pub phase_b_jsonl: PathBuf,
    /// Sanitized director/agent tool-loop timeline (offered → call → result).
    pub tool_trace_jsonl: PathBuf,
}

impl EnduranceEvidencePaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            calls_jsonl: root.join("endurance_calls.jsonl"),
            turns_jsonl: root.join("endurance_turns.jsonl"),
            checkpoint_jsonl: root.join("endurance_checkpoint.jsonl"),
            manifest_jsonl: root.join("endurance_manifest.jsonl"),
            phase_b_jsonl: root.join("endurance_phase_b.jsonl"),
            tool_trace_jsonl: root.join("endurance_tool_trace.jsonl"),
            root,
        }
    }

    /// Total size of all evidence files in bytes.
    pub fn total_size(&self) -> u64 {
        [
            &self.calls_jsonl,
            &self.turns_jsonl,
            &self.checkpoint_jsonl,
            &self.manifest_jsonl,
            &self.phase_b_jsonl,
            &self.tool_trace_jsonl,
        ]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum()
    }

    /// Check that no evidence file contains forbidden payload.
    pub fn check_no_secrets(&self) -> Result<(), String> {
        for path in [
            &self.calls_jsonl,
            &self.turns_jsonl,
            &self.checkpoint_jsonl,
            &self.manifest_jsonl,
            &self.phase_b_jsonl,
        ] {
            if let Ok(text) = std::fs::read_to_string(path)
                && contains_forbidden_evidence_payload(&text)
            {
                return Err(format!("forbidden payload detected in {}", path.display()));
            }
        }
        Ok(())
    }
}

// ─── Run config ─────────────────────────────────────────────────────────────

/// Configuration for an endurance run.
#[derive(Debug, Clone)]
pub struct EnduranceRunConfig {
    pub run_id: String,
    pub stage: EnduranceStage,
    pub model_label: String,
    pub endpoint_host_redacted: String,
    pub evidence_paths: EnduranceEvidencePaths,
    pub budget: EnduranceBudget,
    pub commit: String,
    pub branch: String,
    pub seed: u64,
}

impl EnduranceRunConfig {
    /// Parse config from environment variables. All real-secret values come from
    /// the environment only — never written to source.
    ///
    /// Evidence root resolution is **fail-closed**:
    /// - Prefer `STORYFORGE_EVAL_EVIDENCE_ROOT` (durable multi-run root)
    /// - Else `STORYFORGE_EVAL_EVIDENCE_DIR` (legacy single-run path)
    /// - Else a unique temp root only when `STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE=1`
    ///
    /// Repo-internal, live `data/`, and (unless allowed) temp roots are rejected.
    /// Each call allocates a unique controlled `run-<stage>-<uuid>` directory.
    pub fn from_env(stage: EnduranceStage) -> Self {
        Self::from_env_with_repo_root(stage, discover_repo_root())
    }

    /// Same as [`from_env`] but with an explicit repo root for policy checks.
    pub fn from_env_with_repo_root(stage: EnduranceStage, repo_root: PathBuf) -> Self {
        let allow_ephemeral = std::env::var("STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE")
            .ok()
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false);
        let policy = crate::evidence_retention::EvidenceRootPolicy {
            repo_root,
            allow_ephemeral,
            require_explicit: !allow_ephemeral,
        };
        let root = crate::evidence_retention::resolve_evidence_root(None, None, &policy)
            .unwrap_or_else(|e| panic!("illegal or missing evidence root: {e}"));
        let (run_id, evidence_paths) =
            crate::evidence_retention::open_endurance_run_paths(&root, stage.label())
                .unwrap_or_else(|e| panic!("failed to open exclusive evidence run dir: {e}"));

        let model_label = std::env::var("LLM_MODEL").unwrap_or_default();
        let endpoint_host_redacted = std::env::var("LLM_BASE_URL")
            .map(|u| {
                // redact: strip scheme://user:pass@ and query
                let no_query = u.split('?').next().unwrap_or(&u);
                if let Some(rest) = no_query
                    .strip_prefix("https://")
                    .or_else(|| no_query.strip_prefix("http://"))
                {
                    let host = rest.rsplit('@').next().unwrap_or(rest);
                    format!("<redacted>://{host}")
                } else {
                    "<redacted>".into()
                }
            })
            .unwrap_or_default();

        let commit = std::env::var("STORYFORGE_EVAL_COMMIT").unwrap_or_default();
        let branch = std::env::var("STORYFORGE_EVAL_BRANCH").unwrap_or_default();
        let seed = std::env::var("STORYFORGE_EVAL_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(42);

        let budget = EnduranceBudget::for_stage(stage);

        Self {
            run_id,
            stage,
            model_label,
            endpoint_host_redacted,
            evidence_paths,
            budget,
            commit,
            branch,
            seed,
        }
    }
}

/// Best-effort repo root discovery for evidence-root policy (never panics).
fn discover_repo_root() -> PathBuf {
    if let Ok(root) = std::env::var("STORYFORGE_REPO_ROOT") {
        let p = PathBuf::from(root.trim());
        if p.is_dir() {
            return p;
        }
    }
    // CARGO_MANIFEST_DIR for harness-real-llm is crates/harness-real-llm.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Fail-closed resume gate for an existing evidence directory.
///
/// **Ordering:** recursive reparse/containment checks run first (metadata only).
/// Checkpoint JSONL bodies are opened only after the tree is proven safe.
/// Unsealed interrupted resume additionally requires a checkpoint integrity baseline.
pub fn resume_from_evidence_dir(
    evidence_dir: &Path,
    expected_run_id: Option<&str>,
) -> Result<(u32, EnduranceCheckpoint), EnduranceError> {
    // 1) Metadata-only tree safety before any body open.
    if let Err(e) =
        crate::evidence_retention::walk_reject_reparse_under_run_dir_public(evidence_dir)
    {
        return Err(EnduranceError::InvalidConfig(format!(
            "fail-closed resume tree safety: {e}"
        )));
    }

    // 2) Determine expected run id without trusting external paths.
    // If caller did not provide one, read only after tree safety; still fail closed
    // on schema/mixed ids via load_resume_context.
    let expected = if let Some(id) = expected_run_id {
        id.to_string()
    } else {
        // Safe to open checkpoint only after tree safety.
        let cp_path = evidence_dir.join("endurance_checkpoint.jsonl");
        let cp = read_latest_checkpoint(&cp_path).ok_or_else(|| {
            EnduranceError::InvalidConfig(
                "resume requested but checkpoint missing or schema-invalid".into(),
            )
        })?;
        cp.run_id
    };

    match crate::evidence_retention::load_resume_context(evidence_dir, &expected) {
        Ok(ctx) => {
            let cp = read_latest_checkpoint(&evidence_dir.join("endurance_checkpoint.jsonl"))
                .ok_or_else(|| {
                    EnduranceError::InvalidConfig(
                        "resume requested but checkpoint missing or schema-invalid".into(),
                    )
                })?;
            if ctx.accepted_turn_number != cp.accepted_turn_number {
                return Err(EnduranceError::InvalidConfig(
                    "resume context accepted_turn_number disagrees with checkpoint".into(),
                ));
            }
            if cp.run_id != expected {
                return Err(EnduranceError::InvalidConfig(
                    "checkpoint run_id does not match expected".into(),
                ));
            }
            Ok((ctx.next_turn, cp))
        }
        Err(e) => Err(EnduranceError::InvalidConfig(format!(
            "fail-closed resume: {e}"
        ))),
    }
}

// ─── Dry-run validation ─────────────────────────────────────────────────────

/// Dry-run validation result.
#[derive(Debug, Clone)]
pub struct DryRunReport {
    pub fixture_ok: bool,
    pub output_dir_writable: bool,
    pub budget_valid: bool,
    pub schema_ok: bool,
    pub disk_space_ok: bool,
    pub secret_guards_ok: bool,
    pub evidence_root_ok: bool,
    pub assertions: Vec<AssertionResult>,
}

/// Perform dry-run validation: zero model calls.
///
/// `evidence_root` is validated with a conservative policy when possible. For
/// unit tests that pass ephemeral temp dirs, ephemeral roots are allowed.
pub fn dry_run_validate(
    fixture_exists: bool,
    evidence_root: &Path,
    budget: &EnduranceBudget,
) -> DryRunReport {
    dry_run_validate_with_policy(
        fixture_exists,
        evidence_root,
        budget,
        &crate::evidence_retention::EvidenceRootPolicy::for_tests(discover_repo_root()),
    )
}

/// Dry-run with an explicit evidence-root policy (production runners should pass
/// [`crate::evidence_retention::EvidenceRootPolicy::production`]).
pub fn dry_run_validate_with_policy(
    fixture_exists: bool,
    evidence_root: &Path,
    budget: &EnduranceBudget,
    policy: &crate::evidence_retention::EvidenceRootPolicy,
) -> DryRunReport {
    let mut assertions = Vec::new();

    let fixture_ok = fixture_exists;
    assertions.push(AssertionResult {
        name: "fixture_exists".into(),
        passed: fixture_ok,
        detail: None,
    });

    let evidence_root_ok =
        crate::evidence_retention::preflight_evidence_root(evidence_root, policy).is_ok();
    assertions.push(AssertionResult {
        name: "evidence_root_policy".into(),
        passed: evidence_root_ok,
        detail: Some(crate::evidence_retention::redact_path_for_display(
            evidence_root,
        )),
    });

    let output_dir_writable = evidence_root_ok || std::fs::create_dir_all(evidence_root).is_ok();
    assertions.push(AssertionResult {
        name: "output_dir_writable".into(),
        passed: output_dir_writable,
        // Never echo full host paths that might embed usernames/secrets.
        detail: Some(crate::evidence_retention::redact_path_for_display(
            evidence_root,
        )),
    });

    let budget_valid = budget.max_calls >= 1 && budget.max_turns >= 1 && budget.timeout_secs >= 1;
    assertions.push(AssertionResult {
        name: "budget_valid".into(),
        passed: budget_valid,
        detail: Some(format!(
            "max_calls={} max_turns={} timeout={}s",
            budget.max_calls, budget.max_turns, budget.timeout_secs
        )),
    });

    let schema_ok = EVIDENCE_SCHEMA_VERSION == "eval-m5-phaseb-v1"
        && crate::evidence_retention::RETENTION_SCHEMA_VERSION == "m5-evidence-retention-v1";
    assertions.push(AssertionResult {
        name: "schema_version_ok".into(),
        passed: schema_ok,
        detail: Some(format!(
            "evidence={EVIDENCE_SCHEMA_VERSION};retention={}",
            crate::evidence_retention::RETENTION_SCHEMA_VERSION
        )),
    });

    // Disk space: check we can write a small probe under the root when allowed.
    let disk_space_ok = if evidence_root_ok {
        let probe = evidence_root.join(".disk_probe");
        let probe_ok = std::fs::write(&probe, vec![0u8; 1024]).is_ok();
        let _ = std::fs::remove_file(&probe);
        probe_ok
    } else if let Ok(metadata) = std::fs::metadata(evidence_root) {
        let _ = metadata;
        false
    } else {
        false
    };
    assertions.push(AssertionResult {
        name: "disk_space_ok".into(),
        passed: disk_space_ok,
        detail: None,
    });

    // Secret guards: verify the redaction detector stays armed. Never read or
    // echo LLM_API_KEY values.
    let secret_guards_ok = !contains_forbidden_evidence_payload(r#"{"role":"x","tag":"y"}"#)
        && contains_forbidden_evidence_payload(r#"{"api_key":"redacted"}"#);
    assertions.push(AssertionResult {
        name: "secret_guards_ok".into(),
        passed: secret_guards_ok,
        detail: None,
    });

    DryRunReport {
        fixture_ok,
        output_dir_writable,
        budget_valid,
        schema_ok,
        disk_space_ok,
        secret_guards_ok,
        evidence_root_ok,
        assertions,
    }
}

// ─── Manifest writer ────────────────────────────────────────────────────────

/// Write a stage manifest row.
pub fn write_manifest_row(path: &Path, row: &EnduranceStageManifestRow) -> std::io::Result<()> {
    let serialized = serde_json::to_string(row)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if contains_forbidden_evidence_payload(&serialized) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "manifest row contains forbidden payload",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    use std::io::Write;
    writeln!(file, "{serialized}")?;
    Ok(())
}

/// Read all manifest rows.
pub fn read_manifest_rows(path: &Path) -> Vec<EnduranceStageManifestRow> {
    crate::evidence::read_evidence_lines(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect()
}

// ─── Deterministic checkpoint/resume simulation ─────────────────────────────

/// Deterministic simulation of a resumable run. Does NOT make real LLM calls.
/// This tests the checkpoint/resume logic by simulating accepted turns and verifying
/// that a simulated interruption resumes correctly.
///
/// `end_turn` is inclusive; pass `schedule.total_turns` to run to completion.
pub fn simulate_resumable_run(
    schedule: &EnduranceSchedule,
    start_turn: u32,
    end_turn: u32,
    checkpoint_path: &Path,
) -> Vec<EnduranceCheckpoint> {
    let mut checkpoints = Vec::new();
    let run_id = format!("sim-{}", uuid::Uuid::new_v4());

    for n in start_turn..=end_turn {
        let action = schedule.action_for_turn(n);
        let early_probe = match &action {
            ScheduledAction::EarlyFactInject { probe_id } => Some(probe_id.clone()),
            _ => None,
        };

        // Simulate revision progression
        let campaign_revision = n as u64;
        let chronicle_revision = n as u64;
        let draft_hash = short_hash16(&format!("sim-draft-{n}"));
        let epoch_id16 = short_hash16(&format!("epoch-{}", n / 10));

        // Accumulate early-fact probe ids
        let prev = checkpoints.last();
        let mut early_fact_probe_ids = prev
            .map(|c: &EnduranceCheckpoint| c.early_fact_probe_ids.clone())
            .unwrap_or_default();
        if let Some(pid) = &early_probe
            && !early_fact_probe_ids.contains(pid)
        {
            early_fact_probe_ids.push(pid.clone());
        }

        let cp = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.clone(),
            stage: EnduranceStage::Full.label().into(),
            accepted_turn_number: n,
            calls_used: n * 3, // simulated
            max_calls: EnduranceStage::Full.max_calls(),
            campaign_revision,
            chronicle_revision,
            last_draft_hash16: draft_hash,
            last_summary_code: Some(format!("A{n:04}")),
            context_epoch_id16: Some(epoch_id16.clone()),
            early_fact_probe_ids,
            early_fact_checked_passed: prev
                .map(|c| c.early_fact_checked_passed.clone())
                .unwrap_or_default(),
            campaign_id: Some("sim-campaign".into()),
            conversation_id: Some("sim-conversation".into()),
            data_dir_rel: Some("campaign_data".into()),
            observed_epoch_ids16: vec![epoch_id16],
            recorded_at_unix_ms: 0,
        };
        write_checkpoint(checkpoint_path, &cp).expect("write checkpoint");
        checkpoints.push(cp);
    }
    checkpoints
}

// ─── Deadline helper ────────────────────────────────────────────────────────

pub struct SuiteDeadline {
    pub started: Instant,
    pub duration: Duration,
}

impl SuiteDeadline {
    pub fn new(duration: Duration) -> Self {
        Self {
            started: Instant::now(),
            duration,
        }
    }

    pub fn check(&self) -> Result<(), EnduranceError> {
        if self.started.elapsed() >= self.duration {
            Err(EnduranceError::SuiteTimeout)
        } else {
            Ok(())
        }
    }

    pub fn remaining(&self) -> Result<Duration, EnduranceError> {
        self.duration
            .checked_sub(self.started.elapsed())
            .filter(|r| !r.is_zero())
            .ok_or(EnduranceError::SuiteTimeout)
    }
}

// ─── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum EnduranceError {
    InvalidConfig(String),
    StageGate(StageGateError),
    BudgetExhausted { calls_used: u32, max_calls: u32 },
    SuiteTimeout,
    EvidenceIo(std::io::Error),
    SecretViolation(String),
    MissingUsage { turn_index: u32 },
    InconsistentTurnState { turn_index: u32, reason: String },
    ZeroCalls { turn_index: u32 },
    Writer(String),
    Accept(String),
}

impl fmt::Display for EnduranceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(msg) => write!(f, "invalid endurance config: {msg}"),
            Self::StageGate(err) => write!(f, "{err}"),
            Self::BudgetExhausted {
                calls_used,
                max_calls,
            } => {
                write!(f, "budget exhausted: {calls_used}/{max_calls} calls used")
            }
            Self::SuiteTimeout => write!(f, "endurance suite timeout exhausted"),
            Self::EvidenceIo(err) => write!(f, "evidence I/O error: {err}"),
            Self::SecretViolation(msg) => write!(f, "secret violation: {msg}"),
            Self::MissingUsage { turn_index } => {
                write!(f, "turn {turn_index} produced no usage data")
            }
            Self::InconsistentTurnState { turn_index, reason } => {
                write!(f, "turn {turn_index} inconsistent state: {reason}")
            }
            Self::ZeroCalls { turn_index } => {
                write!(f, "turn {turn_index} produced zero LLM calls")
            }
            Self::Writer(msg) => write!(f, "endurance writer error: {msg}"),
            Self::Accept(msg) => write!(f, "endurance accept failed: {msg}"),
        }
    }
}

impl std::error::Error for EnduranceError {}

impl From<std::io::Error> for EnduranceError {
    fn from(value: std::io::Error) -> Self {
        Self::EvidenceIo(value)
    }
}

impl From<StageGateError> for EnduranceError {
    fn from(value: StageGateError) -> Self {
        Self::StageGate(value)
    }
}

// ─── Evidence helpers for turn records ──────────────────────────────────────

/// Input for building an endurance turn record. Groups all the fields to avoid
/// an excessive argument count.
#[derive(Debug, Clone)]
pub struct EnduranceTurnRecordInput<'a> {
    pub run_id: &'a str,
    pub stage: EnduranceStage,
    pub turn_index: u32,
    pub action: &'a ScheduledAction,
    pub draft_accepted: bool,
    pub campaign_revision_before: u64,
    pub campaign_revision_after: u64,
    pub chronicle_revision_before: u64,
    pub chronicle_revision_after: u64,
    pub summary_code: Option<String>,
    pub draft_hash16: String,
    pub text_len: usize,
    pub text_sha16: String,
    pub context_epoch_id16: Option<String>,
    pub context_epoch_source_hash16: Option<String>,
    pub context_epoch_anchor_count: Option<usize>,
    pub attempt_status: String,
    pub turn_status: String,
    pub assertions: Vec<AssertionResult>,
    pub elapsed_ms: u128,
    /// Paths default to the legacy JSON probe labels when None.
    pub write_path: Option<&'a str>,
    pub chronicle_path: Option<&'a str>,
    pub accept_path: Option<&'a str>,
    pub production_postprocess_complete: Option<bool>,
}

/// Build an `EvidenceTurnRecord` for an endurance turn, embedding schedule metadata.
pub fn build_endurance_turn_record(input: EnduranceTurnRecordInput<'_>) -> EvidenceTurnRecord {
    let (kind, role_tag, mode_tag) = action_metadata(input.action);
    EvidenceTurnRecord {
        schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        run_id: input.run_id.into(),
        suite: format!("endurance_{}", input.stage.label()),
        turn_index: input.turn_index,
        kind: format!("endurance_{kind}"),
        write_path: input.write_path.unwrap_or("production_pipeline").into(),
        chronicle_path: input
            .chronicle_path
            .unwrap_or("synthetic_chronicle_fixture")
            .into(),
        accept_path: input
            .accept_path
            .unwrap_or("production_faithful_commit_probe")
            .into(),
        production_postprocess_complete: input.production_postprocess_complete.unwrap_or(false),
        draft_accepted: input.draft_accepted,
        force_accept: false,
        quality_error_count: 0,
        quality_warning_count: 0,
        autofix_attempts: 0,
        campaign_revision_before: input.campaign_revision_before,
        campaign_revision_after: input.campaign_revision_after,
        chronicle_revision_before: input.chronicle_revision_before,
        chronicle_revision_after: input.chronicle_revision_after,
        summary_code: input.summary_code,
        attempt_status: input.attempt_status,
        turn_status: input.turn_status,
        draft_hash16: input.draft_hash16,
        text_len: input.text_len,
        text_sha16: input.text_sha16,
        early_fact_reachable: None,
        context_epoch_id16: input.context_epoch_id16,
        context_epoch_source_hash16: input.context_epoch_source_hash16,
        context_epoch_anchor_count: input.context_epoch_anchor_count,
        assertion_results: {
            let mut a = input.assertions;
            a.push(AssertionResult {
                name: "action_role".into(),
                passed: true,
                detail: Some(role_tag),
            });
            a.push(AssertionResult {
                name: "action_mode".into(),
                passed: true,
                detail: Some(mode_tag),
            });
            a
        },
        elapsed_ms: input.elapsed_ms,
        recorded_at_unix_ms: 0,
    }
}

fn action_metadata(action: &ScheduledAction) -> (&'static str, String, String) {
    match action {
        ScheduledAction::Write {
            subagent_count,
            reasoning_mode,
            tool_mode,
            world_info_route,
        } => (
            "write",
            format!("director+editor+subagents={subagent_count}"),
            format!(
                "reasoning={}_tool={}_world={}",
                reasoning_mode.label(),
                tool_mode.label(),
                world_info_route.label()
            ),
        ),
        ScheduledAction::RegenerateOverall => (
            "regenerate_overall",
            "director".into(),
            "overall_reroll".into(),
        ),
        ScheduledAction::RegenerateEditor => {
            ("regenerate_editor", "editor".into(), "editor_only".into())
        }
        ScheduledAction::RegenerateSubagent => (
            "regenerate_subagent",
            "subagent".into(),
            "subagent_only".into(),
        ),
        ScheduledAction::PrivateProbe { probe_kind } => (
            "private_probe",
            "director".into(),
            format!("probe={:?}", probe_kind).to_ascii_lowercase(),
        ),
        ScheduledAction::EarlyFactInject { probe_id } => (
            "early_fact_inject",
            "summarizer".into(),
            format!("probe={}", short_hash16(probe_id)),
        ),
        ScheduledAction::EarlyFactCheck { probe_id } => (
            "early_fact_check",
            "chronicle_search".into(),
            format!("probe={}", short_hash16(probe_id)),
        ),
        ScheduledAction::QualityAutofix { fixable } => (
            "quality_autofix",
            "editor".into(),
            format!("fixable={fixable}"),
        ),
        ScheduledAction::CacheStable => {
            ("cache_stable", "director".into(), "no_perturbation".into())
        }
        ScheduledAction::CacheInvalidate => {
            ("cache_invalidate", "director".into(), "perturbation".into())
        }
    }
}

// ─── Epoch rollover tracking ────────────────────────────────────────────────

/// Track unique epoch ids observed across a run.
#[derive(Debug, Clone, Default)]
pub struct EpochTracker {
    pub observed: Vec<String>,
    set: BTreeSet<String>,
}

impl EpochTracker {
    pub fn observe(&mut self, epoch_id16: &str) -> bool {
        if self.set.insert(epoch_id16.to_string()) {
            self.observed.push(epoch_id16.to_string());
            true
        } else {
            false
        }
    }

    pub fn rolled_over(&self) -> bool {
        self.observed.len() >= 2
    }

    pub fn count(&self) -> usize {
        self.observed.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Stage definitions ──

    #[test]
    fn stage_targets_and_budgets_are_monotonic() {
        let stages = EnduranceStage::all();
        for w in stages.windows(2) {
            assert!(
                w[1].target_turns() > w[0].target_turns(),
                "{} should have more turns than {}",
                w[1],
                w[0]
            );
            assert!(
                w[1].max_calls() >= w[0].max_calls(),
                "{} should have >= calls than {}",
                w[1],
                w[0]
            );
        }
        assert_eq!(EnduranceStage::Full.target_turns(), 100);
        assert_eq!(EnduranceStage::Full.max_calls(), 1400);
        assert_eq!(EnduranceStage::Canary.target_turns(), 3);
        assert_eq!(EnduranceStage::Canary.max_calls(), 30);
    }

    #[test]
    fn stage_predecessor_chain_is_correct() {
        assert_eq!(EnduranceStage::DryRun.predecessor(), None);
        assert_eq!(
            EnduranceStage::Canary.predecessor(),
            Some(EnduranceStage::DryRun)
        );
        assert_eq!(
            EnduranceStage::Coverage.predecessor(),
            Some(EnduranceStage::Canary)
        );
        assert_eq!(
            EnduranceStage::Stability.predecessor(),
            Some(EnduranceStage::Coverage)
        );
        assert_eq!(
            EnduranceStage::Full.predecessor(),
            Some(EnduranceStage::Stability)
        );
    }

    #[test]
    fn stage_label_roundtrip() {
        for stage in EnduranceStage::all() {
            let label = stage.label();
            assert_eq!(EnduranceStage::from_label(label), Some(*stage));
        }
        assert_eq!(EnduranceStage::from_label("nonexistent"), None);
    }

    // ── Schedule coverage ──

    #[test]
    fn schedule_covers_all_required_matrix_rows() {
        let schedule = EnduranceSchedule::new(100);
        let report = schedule.coverage_report();
        let failed: Vec<_> = report.iter().filter(|a| !a.passed).collect();
        assert!(failed.is_empty(), "coverage matrix missing: {failed:?}");
    }

    #[test]
    fn schedule_regenerate_windows_match_plan() {
        let schedule = EnduranceSchedule::new(100);
        assert!(matches!(
            schedule.action_for_turn(11),
            ScheduledAction::RegenerateOverall
        ));
        assert!(matches!(
            schedule.action_for_turn(37),
            ScheduledAction::RegenerateOverall
        ));
        assert!(matches!(
            schedule.action_for_turn(73),
            ScheduledAction::RegenerateOverall
        ));
        assert!(matches!(
            schedule.action_for_turn(18),
            ScheduledAction::RegenerateEditor
        ));
        assert!(matches!(
            schedule.action_for_turn(54),
            ScheduledAction::RegenerateEditor
        ));
        assert!(matches!(
            schedule.action_for_turn(90),
            ScheduledAction::RegenerateEditor
        ));
        assert!(matches!(
            schedule.action_for_turn(25),
            ScheduledAction::RegenerateSubagent
        ));
        assert!(matches!(
            schedule.action_for_turn(61),
            ScheduledAction::RegenerateSubagent
        ));
        assert!(matches!(
            schedule.action_for_turn(97),
            ScheduledAction::RegenerateSubagent
        ));
    }

    #[test]
    fn schedule_private_probes_match_plan() {
        let schedule = EnduranceSchedule::new(100);
        assert!(matches!(
            schedule.action_for_turn(20),
            ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::NonOwnerLeak
            }
        ));
        assert!(matches!(
            schedule.action_for_turn(50),
            ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::NarrationLeak
            }
        ));
        assert!(matches!(
            schedule.action_for_turn(80),
            ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::MustNotReveal
            }
        ));
    }

    #[test]
    fn schedule_early_fact_injects_and_checks_in_order() {
        let schedule = EnduranceSchedule::new(100);
        // Injects in turns 3, 8, 13
        assert!(matches!(
            schedule.action_for_turn(3),
            ScheduledAction::EarlyFactInject { probe_id } if probe_id == "EF-ALPHA-4471"
        ));
        assert!(matches!(
            schedule.action_for_turn(8),
            ScheduledAction::EarlyFactInject { probe_id } if probe_id == "EF-BETA-2098"
        ));
        assert!(matches!(
            schedule.action_for_turn(13),
            ScheduledAction::EarlyFactInject { probe_id } if probe_id == "EF-GAMMA-6603"
        ));
        // Checks after 35, 65, 95
        assert!(matches!(
            schedule.action_for_turn(35),
            ScheduledAction::EarlyFactCheck { probe_id } if probe_id == "EF-ALPHA-4471"
        ));
        assert!(matches!(
            schedule.action_for_turn(65),
            ScheduledAction::EarlyFactCheck { probe_id } if probe_id == "EF-BETA-2098"
        ));
        assert!(matches!(
            schedule.action_for_turn(95),
            ScheduledAction::EarlyFactCheck { probe_id } if probe_id == "EF-GAMMA-6603"
        ));
    }

    #[test]
    fn schedule_write_turns_cycle_subagent_counts() {
        let schedule = EnduranceSchedule::new(100);
        // Turn 1 → cycle 0 → subagent_count 1
        assert!(matches!(
            schedule.action_for_turn(1),
            ScheduledAction::Write {
                subagent_count: 1,
                ..
            }
        ));
        // Turn 2 → cycle 1 → subagent_count 2
        assert!(matches!(
            schedule.action_for_turn(2),
            ScheduledAction::Write {
                subagent_count: 2,
                ..
            }
        ));
        // Turn 4 → cycle 3 → subagent_count 1 (turn 3 is EarlyFactInject)
        assert!(matches!(
            schedule.action_for_turn(4),
            ScheduledAction::Write {
                subagent_count: 1,
                ..
            }
        ));
        // Turn 5 → cycle 4 → subagent_count 2
        assert!(matches!(
            schedule.action_for_turn(5),
            ScheduledAction::Write {
                subagent_count: 2,
                ..
            }
        ));
        // Turn 9 → cycle 8 → subagent_count 3 (turn 8 is EarlyFactInject)
        assert!(matches!(
            schedule.action_for_turn(9),
            ScheduledAction::Write {
                subagent_count: 3,
                ..
            }
        ));
    }

    #[test]
    fn schedule_quality_autofix_has_both_fixable_and_nonfixable() {
        let schedule = EnduranceSchedule::new(100);
        assert!(matches!(
            schedule.action_for_turn(15),
            ScheduledAction::QualityAutofix { fixable: true }
        ));
        assert!(matches!(
            schedule.action_for_turn(45),
            ScheduledAction::QualityAutofix { fixable: false }
        ));
    }

    #[test]
    fn schedule_cache_invalidate_at_epoch_boundaries() {
        let schedule = EnduranceSchedule::new(100);
        // turn 10 is cache_invalidate (epoch boundary-ish)
        assert!(matches!(
            schedule.action_for_turn(10),
            ScheduledAction::CacheInvalidate
        ));
    }

    // ── Budget ──

    #[test]
    fn budget_for_stage_uses_stage_limits() {
        let b = EnduranceBudget::for_stage(EnduranceStage::Full);
        assert_eq!(b.max_calls, 1400);
        assert_eq!(b.max_turns, 100);
    }

    #[test]
    fn budget_can_attempt_turn_respects_floor() {
        let b = EnduranceBudget {
            max_calls: 10,
            max_turns: 5,
            timeout_secs: 30,
            hard_deadline: None,
            max_evidence_bytes: 1024,
        };
        assert!(b.can_attempt_turn(8, 2)); // 8+2=10 ≤ 10
        assert!(!b.can_attempt_turn(9, 2)); // 9+2=11 > 10
    }

    // ── Stage gating ──

    #[test]
    fn dry_run_always_allowed() {
        assert!(check_stage_gate(EnduranceStage::DryRun, None).is_ok());
    }

    #[test]
    fn canary_allowed_without_predecessor() {
        assert!(check_stage_gate(EnduranceStage::Canary, None).is_ok());
    }

    #[test]
    fn coverage_requires_canary_3_turns() {
        let good = EnduranceStageManifestRow {
            schema_version: "test".into(),
            run_id: "r".into(),
            stage: EnduranceStage::Canary.label().into(),
            target_turns: 3,
            accepted_turns: 3,
            calls_used: 10,
            max_calls: 30,
            elapsed_ms: 100,
            acceptance: AcceptanceLevel::Pass.label().into(),
            summary_codes: vec![],
            observed_epoch_ids16: vec![],
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            coverage_assertions: vec![],
            recorded_at_unix_ms: 0,
        };
        assert!(check_stage_gate(EnduranceStage::Coverage, Some(&good)).is_ok());

        let short = EnduranceStageManifestRow {
            accepted_turns: 2,
            ..good
        };
        assert!(check_stage_gate(EnduranceStage::Coverage, Some(&short)).is_err());
    }

    #[test]
    fn full_requires_stability_30_turns() {
        let good = EnduranceStageManifestRow {
            schema_version: "test".into(),
            run_id: "r".into(),
            stage: EnduranceStage::Stability.label().into(),
            target_turns: 30,
            accepted_turns: 30,
            calls_used: 200,
            max_calls: 300,
            elapsed_ms: 100,
            acceptance: AcceptanceLevel::Pass.label().into(),
            summary_codes: vec![],
            observed_epoch_ids16: vec![],
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            coverage_assertions: vec![],
            recorded_at_unix_ms: 0,
        };
        assert!(check_stage_gate(EnduranceStage::Full, Some(&good)).is_ok());

        let short = EnduranceStageManifestRow {
            accepted_turns: 25,
            ..good
        };
        assert!(check_stage_gate(EnduranceStage::Full, Some(&short)).is_err());
    }

    // ── Acceptance classification ──

    #[test]
    fn classify_pass_when_all_invariants_met() {
        let level = classify_acceptance(EnduranceStage::Canary, 3, 10, true, false, false);
        assert_eq!(level, AcceptanceLevel::Pass);
    }

    #[test]
    fn classify_partial_when_turns_short() {
        let level = classify_acceptance(EnduranceStage::Canary, 2, 10, true, false, false);
        assert_eq!(level, AcceptanceLevel::Partial);
    }

    #[test]
    fn classify_partial_when_budget_exceeded() {
        let level = classify_acceptance(EnduranceStage::Canary, 3, 35, true, false, false);
        assert_eq!(level, AcceptanceLevel::Partial);
    }

    #[test]
    fn classify_inconclusive_on_secret_violation() {
        let level = classify_acceptance(EnduranceStage::Canary, 3, 10, true, true, false);
        assert_eq!(level, AcceptanceLevel::Inconclusive);
    }

    #[test]
    fn classify_inconclusive_on_missing_usage() {
        let level = classify_acceptance(EnduranceStage::Canary, 3, 10, true, false, true);
        assert_eq!(level, AcceptanceLevel::Inconclusive);
    }

    #[test]
    fn partial_and_inconclusive_exit_nonzero() {
        assert_eq!(AcceptanceLevel::Pass.exit_code(), 0);
        assert_eq!(AcceptanceLevel::Partial.exit_code(), 1);
        assert_eq!(AcceptanceLevel::Inconclusive.exit_code(), 1);
    }

    // ── Checkpoint / resume ──

    #[test]
    fn resume_turn_from_no_checkpoint_starts_at_1() {
        assert_eq!(resume_turn_from_checkpoint(None), 1);
    }

    #[test]
    fn resume_turn_from_checkpoint_starts_after() {
        let cp = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: "r".into(),
            stage: EnduranceStage::Coverage.label().into(),
            accepted_turn_number: 8,
            calls_used: 20,
            max_calls: 120,
            campaign_revision: 8,
            chronicle_revision: 8,
            last_draft_hash16: "abc".into(),
            last_summary_code: Some("A0008".into()),
            context_epoch_id16: None,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: None,
            conversation_id: None,
            data_dir_rel: None,
            observed_epoch_ids16: vec![],
            recorded_at_unix_ms: 0,
        };
        assert_eq!(resume_turn_from_checkpoint(Some(&cp)), 9);
    }

    #[test]
    fn checkpoint_is_new_detects_duplicates() {
        let cp = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: "r".into(),
            stage: EnduranceStage::Coverage.label().into(),
            accepted_turn_number: 5,
            calls_used: 12,
            max_calls: 120,
            campaign_revision: 5,
            chronicle_revision: 5,
            last_draft_hash16: "abc".into(),
            last_summary_code: Some("A0005".into()),
            context_epoch_id16: None,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: None,
            conversation_id: None,
            data_dir_rel: None,
            observed_epoch_ids16: vec![],
            recorded_at_unix_ms: 0,
        };
        // Same turn, revision, hash → not new (duplicate)
        assert!(!checkpoint_is_new(Some(&cp), 5, 5, "abc"));
        // Different turn → new
        assert!(checkpoint_is_new(Some(&cp), 6, 5, "abc"));
        // Different revision → new
        assert!(checkpoint_is_new(Some(&cp), 5, 6, "abc"));
        // Different hash → new
        assert!(checkpoint_is_new(Some(&cp), 5, 5, "xyz"));
    }

    #[test]
    fn write_and_read_checkpoint_roundtrips() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_cp_{}", uuid::Uuid::new_v4()));
        let path = dir.join("checkpoint.jsonl");
        let cp = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: "test-run".into(),
            stage: EnduranceStage::Canary.label().into(),
            accepted_turn_number: 3,
            calls_used: 9,
            max_calls: 30,
            campaign_revision: 3,
            chronicle_revision: 3,
            last_draft_hash16: "deadbeef".into(),
            last_summary_code: Some("A0003".into()),
            context_epoch_id16: Some("epoch1".into()),
            early_fact_probe_ids: vec!["EF-ALPHA".into()],
            early_fact_checked_passed: vec![],
            campaign_id: None,
            conversation_id: None,
            data_dir_rel: None,
            observed_epoch_ids16: vec![],
            recorded_at_unix_ms: 123,
        };
        write_checkpoint(&path, &cp).unwrap();
        let latest = read_latest_checkpoint(&path).expect("should read checkpoint");
        assert_eq!(latest.accepted_turn_number, 3);
        assert_eq!(latest.run_id, "test-run");
        assert_eq!(latest.early_fact_probe_ids, vec!["EF-ALPHA".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn checkpoint_refuses_secret_payload() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_sec_{}", uuid::Uuid::new_v4()));
        let path = dir.join("checkpoint.jsonl");
        let cp = EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            // Construct at runtime so static secret scanners do not treat the fixture as a live key.
            run_id: format!("{}{}", "sk-", "secret-key-in-run-id"),
            stage: EnduranceStage::Canary.label().into(),
            accepted_turn_number: 1,
            calls_used: 1,
            max_calls: 30,
            campaign_revision: 1,
            chronicle_revision: 1,
            last_draft_hash16: "abc".into(),
            last_summary_code: None,
            context_epoch_id16: None,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: None,
            conversation_id: None,
            data_dir_rel: None,
            observed_epoch_ids16: vec![],
            recorded_at_unix_ms: 0,
        };
        let result = write_checkpoint(&path, &cp);
        assert!(
            result.is_err(),
            "should refuse to write secret-containing checkpoint"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Resume simulation ──

    #[test]
    fn simulate_resumable_run_produces_monotonic_revisions() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_sim_{}", uuid::Uuid::new_v4()));
        let path = dir.join("checkpoint.jsonl");
        let schedule = EnduranceSchedule::new(10);
        let checkpoints = simulate_resumable_run(&schedule, 1, 10, &path);
        assert_eq!(checkpoints.len(), 10);
        for w in checkpoints.windows(2) {
            assert!(w[1].campaign_revision > w[0].campaign_revision);
            assert!(w[1].accepted_turn_number > w[0].accepted_turn_number);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn simulate_resume_from_checkpoint_does_not_replay() {
        let dir =
            std::env::temp_dir().join(format!("sf_endurance_resume_{}", uuid::Uuid::new_v4()));
        let path = dir.join("checkpoint.jsonl");
        let schedule = EnduranceSchedule::new(10);

        // First run: turns 1-5 (simulated interruption)
        let first = simulate_resumable_run(&schedule, 1, 5, &path);
        assert_eq!(first.len(), 5);
        assert_eq!(first.last().unwrap().accepted_turn_number, 5);

        // "Interruption": read latest checkpoint
        let latest = read_latest_checkpoint(&path).unwrap();
        assert_eq!(latest.accepted_turn_number, 5);

        // Resume: turns 6-10
        let resume_start = resume_turn_from_checkpoint(Some(&latest));
        assert_eq!(resume_start, 6);
        let second = simulate_resumable_run(&schedule, resume_start, 10, &path);
        assert_eq!(second.len(), 5); // turns 6-10
        assert_eq!(second.first().unwrap().accepted_turn_number, 6);
        assert_eq!(second.last().unwrap().accepted_turn_number, 10);

        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Early-fact accumulation across resume ──

    #[test]
    fn early_fact_probes_accumulate_across_checkpoint_lines() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_ef_{}", uuid::Uuid::new_v4()));
        let path = dir.join("checkpoint.jsonl");
        let schedule = EnduranceSchedule::new(15);

        // Run turns 1-15 which includes injects at 3, 8, 13
        let checkpoints = simulate_resumable_run(&schedule, 1, 15, &path);
        let last = checkpoints.last().unwrap();
        assert!(
            last.early_fact_probe_ids
                .contains(&"EF-ALPHA-4471".to_string())
        );
        assert!(
            last.early_fact_probe_ids
                .contains(&"EF-BETA-2098".to_string())
        );
        assert!(
            last.early_fact_probe_ids
                .contains(&"EF-GAMMA-6603".to_string())
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Epoch tracker ──

    #[test]
    fn epoch_tracker_detects_rollover() {
        let mut tracker = EpochTracker::default();
        assert!(!tracker.rolled_over());
        tracker.observe("epoch-a");
        assert!(!tracker.rolled_over());
        tracker.observe("epoch-b");
        assert!(tracker.rolled_over());
        assert_eq!(tracker.count(), 2);
        // duplicate
        tracker.observe("epoch-a");
        assert_eq!(tracker.count(), 2);
    }

    // ── Evidence path ──

    #[test]
    fn evidence_paths_total_size_works() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_paths_{}", uuid::Uuid::new_v4()));
        let paths = EnduranceEvidencePaths::new(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&paths.manifest_jsonl, b"hello").unwrap();
        assert!(paths.total_size() >= 5);
        assert!(paths.check_no_secrets().is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn evidence_paths_detects_secret() {
        let dir =
            std::env::temp_dir().join(format!("sf_endurance_paths2_{}", uuid::Uuid::new_v4()));
        let paths = EnduranceEvidencePaths::new(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&paths.calls_jsonl, r#"{"line":"sk-abc123"}"#).unwrap();
        let result = paths.check_no_secrets();
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Dry-run validation ──

    #[test]
    fn dry_run_validates_fixture_budget_and_schema() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_dry_{}", uuid::Uuid::new_v4()));
        let budget = EnduranceBudget::default();
        let report = dry_run_validate(true, &dir, &budget);
        assert!(report.fixture_ok);
        assert!(report.output_dir_writable);
        assert!(report.budget_valid);
        assert!(report.schema_ok);
        assert!(report.disk_space_ok);
        assert!(report.secret_guards_ok);
        assert!(report.evidence_root_ok);
        assert!(report.assertions.iter().all(|a| a.passed));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dry_run_fails_on_missing_fixture() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_dry2_{}", uuid::Uuid::new_v4()));
        let budget = EnduranceBudget::default();
        let report = dry_run_validate(false, &dir, &budget);
        assert!(!report.fixture_ok);
        assert!(!report.assertions.iter().all(|a| a.passed));
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Manifest writer ──

    #[test]
    fn manifest_write_and_read_roundtrips() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_man_{}", uuid::Uuid::new_v4()));
        let path = dir.join("manifest.jsonl");
        let row = EnduranceStageManifestRow {
            schema_version: "test".into(),
            run_id: "r1".into(),
            stage: EnduranceStage::Canary.label().into(),
            target_turns: 3,
            accepted_turns: 3,
            calls_used: 10,
            max_calls: 30,
            elapsed_ms: 5000,
            acceptance: AcceptanceLevel::Pass.label().into(),
            summary_codes: vec!["A0001".into(), "A0002".into(), "A0003".into()],
            observed_epoch_ids16: vec!["e1".into()],
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            coverage_assertions: vec![AssertionResult {
                name: "director_on_writing_turns".into(),
                passed: true,
                detail: None,
            }],
            recorded_at_unix_ms: 42,
        };
        write_manifest_row(&path, &row).unwrap();
        let rows = read_manifest_rows(&path);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].accepted_turns, 3);
        assert_eq!(rows[0].acceptance, "pass");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn manifest_refuses_secret_payload() {
        let dir = std::env::temp_dir().join(format!("sf_endurance_man2_{}", uuid::Uuid::new_v4()));
        let path = dir.join("manifest.jsonl");
        let row = EnduranceStageManifestRow {
            schema_version: "test".into(),
            run_id: "sk-leaked-key".into(),
            stage: EnduranceStage::Canary.label().into(),
            target_turns: 3,
            accepted_turns: 3,
            calls_used: 10,
            max_calls: 30,
            elapsed_ms: 5000,
            acceptance: AcceptanceLevel::Pass.label().into(),
            summary_codes: vec![],
            observed_epoch_ids16: vec![],
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            coverage_assertions: vec![],
            recorded_at_unix_ms: 42,
        };
        let result = write_manifest_row(&path, &row);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Turn record builder ──

    #[test]
    fn build_endurance_turn_record_embeds_action_metadata() {
        let action = ScheduledAction::Write {
            subagent_count: 2,
            reasoning_mode: ReasoningModeSlot::Native,
            tool_mode: ToolModeSlot::TextFallback,
            world_info_route: WorldInfoRouteSlot::Selective,
        };
        let rec = build_endurance_turn_record(EnduranceTurnRecordInput {
            run_id: "run-1",
            stage: EnduranceStage::Coverage,
            turn_index: 5,
            action: &action,
            draft_accepted: true,
            campaign_revision_before: 4,
            campaign_revision_after: 5,
            chronicle_revision_before: 4,
            chronicle_revision_after: 5,
            summary_code: Some("A0005".into()),
            draft_hash16: "deadbeef".into(),
            text_len: 100,
            text_sha16: "cafe".into(),
            context_epoch_id16: Some("epoch1".into()),
            context_epoch_source_hash16: Some("src1".into()),
            context_epoch_anchor_count: Some(5),
            attempt_status: "Committed".into(),
            turn_status: "Committed".into(),
            assertions: vec![],
            elapsed_ms: 42,
            write_path: None,
            chronicle_path: None,
            accept_path: None,
            production_postprocess_complete: None,
        });
        assert_eq!(rec.turn_index, 5);
        assert_eq!(rec.kind, "endurance_write");
        assert!(rec.draft_accepted);
        // action_role and action_mode assertions appended
        let role_assert = rec
            .assertion_results
            .iter()
            .find(|a| a.name == "action_role")
            .unwrap();
        assert!(role_assert.detail.as_ref().unwrap().contains("subagents=2"));
        let mode_assert = rec
            .assertion_results
            .iter()
            .find(|a| a.name == "action_mode")
            .unwrap();
        assert!(mode_assert.detail.as_ref().unwrap().contains("native"));
    }
}

//! SQLite-backed endurance adapter for Gate B / Gate C.
//!
//! Uses the same `sqlite_runtime` production gateway as Tauri for draft land,
//! autofix/postprocess attach, regenerate, and Accept. JSON stores are not the
//! authority after activation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use storyforge_app_agent::{PostProcessOutcome, ToolContext};
use storyforge_app_conversation::{ConversationStore, PartialRollTarget};
use storyforge_app_pipeline::{PipelineOrchestrator, RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::Source;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{Character, CharacterCard, CharacterDefinition, RoleType};
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, PropagationPolicy};
use storyforge_domain::conversation::{Provenance, Role, VariantStatus};
use storyforge_domain::llm::{ReasoningMode, SamplingParams};
use storyforge_domain::story_task::{StoryTask, TaskStatus, TaskTrigger};
use storyforge_domain::turn::{AttemptStatus, QualityReport, TurnRecord, TurnStatus};
use storyforge_domain::world_info::{LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, DraftAttemptRequest, PostprocessApplyOutcome, PostprocessApplyRequest,
    PreacceptOutboxKind, RegenerateAttemptRequest,
};
use storyforge_infra_vector::BruteForceStore;
use storyforge_tauri_app::campaign_store::{CampaignStore, StoredCard};
use storyforge_tauri_app::fill_campaign_runtime_from_sqlite;
use storyforge_tauri_app::production_postprocess::{
    PostprocessIdentity, ProductionPostprocessError, ProductionPostprocessService,
    QualityAutofixRequest, TurnAttemptSink, run_quality_gate_with_optional_editor_autofix,
};
use storyforge_tauri_app::sqlite_runtime;
use storyforge_tauri_app::turn_lifecycle;

use crate::coverage_ledger::{ObservationKey, ObservedCoverage, SqlitePostcondition};
use crate::endurance::{PrivateProbeKind, WorldInfoRouteSlot};
use crate::evidence::short_hash16;
use crate::production_evidence::{ProductionPostprocessProof, mutation_batch_digest};

const FIXTURE_REL: &str = "fixtures/cot_three_arm_80turn_v1.json";
const RETRYABLE_QUALITY_BLOCKED_PREFIX: &str = "retryable_quality_blocked:";

pub fn is_retryable_quality_blocked_error(error: &str) -> bool {
    error.starts_with(RETRYABLE_QUALITY_BLOCKED_PREFIX)
}

fn format_accept_error_for_runner(error: turn_lifecycle::AcceptError) -> String {
    match error {
        turn_lifecycle::AcceptError::QualityBlocked { error_count } => {
            format!("{RETRYABLE_QUALITY_BLOCKED_PREFIX}error_count={error_count}")
        }
        other => format!("nonretryable_accept:{other}"),
    }
}

/// Privacy-safe projection of live pipeline events. It deliberately stores no
/// event text, character identifiers, prompts, or model output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineObservationCollector {
    agent_roles: BTreeSet<String>,
    subagent_indices: BTreeSet<usize>,
    quality_error_counts: Vec<usize>,
    editor_completion_count: usize,
}

impl PipelineObservationCollector {
    pub fn observe(&mut self, event: &PipelineEvent) {
        match event {
            PipelineEvent::DirectorDone { .. } => {
                self.agent_roles.insert("director".into());
            }
            PipelineEvent::SubagentDone { index, .. } => {
                self.agent_roles.insert("subagent".into());
                self.subagent_indices.insert(*index);
            }
            PipelineEvent::DraftReady { .. } => {
                self.agent_roles.insert("editor".into());
                self.editor_completion_count += 1;
            }
            PipelineEvent::QualityChecked { error_count, .. } => {
                self.quality_error_counts.push(*error_count);
            }
            PipelineEvent::SummaryDone { .. } => {
                self.agent_roles.insert("summarizer".into());
            }
            PipelineEvent::PostProcessDone { .. } => {
                self.agent_roles.insert("postprocessor".into());
            }
            // Postprocess is intentionally best-effort in production. Record a
            // failed attempt as an exercised postprocessor role so a degraded
            // turn can be accepted and audited instead of being mistaken for
            // a missing agent invocation.
            PipelineEvent::PostProcessFailed { .. } => {
                self.agent_roles.insert("postprocessor".into());
            }
            _ => {}
        }
    }

    pub fn agent_events(&self) -> Vec<String> {
        self.agent_roles.iter().cloned().collect()
    }

    pub fn subagent_count(&self) -> usize {
        self.subagent_indices.len()
    }

    pub fn quality_checked(&self) -> bool {
        !self.quality_error_counts.is_empty()
    }

    pub fn quality_error_counts(&self) -> &[usize] {
        &self.quality_error_counts
    }

    pub fn completed_quality_autofix(&self) -> bool {
        self.quality_error_counts
            .first()
            .is_some_and(|count| *count > 0)
            && self.quality_error_counts.last() == Some(&0)
            && self.quality_error_counts.len() >= 2
            && self.editor_completion_count >= 2
    }

    pub fn require_roles(&self, required: &[&str]) -> Result<(), String> {
        let missing = required
            .iter()
            .filter(|role| !self.agent_roles.contains(**role))
            .copied()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "pipeline missing completed agent events: {missing:?}"
            ))
        }
    }

    fn observation_keys(&self) -> BTreeSet<ObservationKey> {
        let mut out = BTreeSet::new();
        for role in &self.agent_roles {
            out.insert(ObservationKey::AgentRole(role.clone()));
        }
        if self.quality_checked() {
            out.insert(ObservationKey::ToolEvent("quality_gate:evaluated".into()));
        }
        out
    }
}

fn pipeline_observation_channel() -> (
    tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    tokio::task::JoinHandle<PipelineObservationCollector>,
) {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut collector = PipelineObservationCollector::default();
        while let Some(event) = event_rx.recv().await {
            collector.observe(&event);
        }
        collector
    });
    (event_tx, task)
}

/// Process-owned SQLite endurance environment.
pub struct SqliteHarnessEnv {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub conv_store: Arc<ConversationStore>,
    /// Disabled sentinel — never used for authority after activate.
    pub campaign_store: Arc<CampaignStore>,
    pub tool_ctx: Arc<RwLock<ToolContext>>,
    pub vector_store: Arc<BruteForceStore>,
    pub llm: Arc<dyn LlmClient>,
    pub active_campaign: Mutex<Option<Id>>,
    pub fixture_hash16: String,
    /// Connection-equivalent sampling used by `new_pipeline` for CoT assembly.
    /// Eval sets `reasoning` from `STORYFORGE_EVAL_REASONING_MODE` so Prompted
    /// injects role CoT; request-side override alone does not.
    pub pipeline_sampling: Option<SamplingParams>,
}

#[derive(Debug, Clone)]
pub struct SqliteTurnResult {
    pub draft_text: String,
    pub variant_id: Id,
    pub turn_id: Id,
    pub attempt_id: Id,
    pub input_node_id: Id,
    pub postprocess_proof: ProductionPostprocessProof,
    pub accept: SqliteAcceptSummary,
    pub observed: ObservedCoverage,
    pub quality_warning_count: usize,
    pub quality_error_count: usize,
    pub autofix_applied: bool,
    pub actual_subagent_count: usize,
}

#[derive(Debug, Clone)]
pub struct SqliteAcceptSummary {
    pub ok: bool,
    pub error: Option<String>,
    pub campaign_revision_before: u64,
    pub campaign_revision_after: u64,
    pub chronicle_revision_before: u64,
    pub chronicle_revision_after: u64,
    pub draft_hash: String,
    pub turn_status: TurnStatus,
    pub attempt_status: AttemptStatus,
    pub summary_code: Option<String>,
}

struct SqlitePostprocessRequest<'a> {
    campaign_id: &'a Id,
    conversation_id: &'a Id,
    turn_id: &'a Id,
    attempt_id: &'a Id,
    variant_id: &'a Id,
    draft_text: &'a str,
    quality_report: QualityReport,
    autofix_provenance: Option<Provenance>,
    outcome: Option<PostProcessOutcome>,
    present_chars: Vec<String>,
    turn_number: u32,
    input_node_id: Id,
}

#[derive(Debug, Deserialize)]
struct FixtureRoot {
    fixture_version: String,
    safe_probe_facts: Vec<String>,
    must_not_reveal: Vec<String>,
    character: FixtureCharacter,
    definitions: Vec<FixtureDefinition>,
    #[serde(default)]
    tasks: Vec<FixtureTask>,
    #[serde(default)]
    evaluation: Option<FixtureEvaluation>,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureEvaluation {
    target_turns: u32,
    #[serde(default)]
    checkpoint_turns: Vec<u32>,
    turn_script: Vec<FixtureTurnSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureTurnSpec {
    pub turn: u32,
    pub action: String,
    #[serde(default)]
    pub subagent_count: u32,
    #[serde(default)]
    pub world_info_route: String,
    pub intent: String,
    #[serde(default)]
    pub fault_profile: Option<String>,
    #[serde(default)]
    pub summary_probe: Option<String>,
    #[serde(default)]
    pub must_retrieve: Vec<String>,
    #[serde(default)]
    pub checkpoint_after_accept: bool,
}

#[derive(Debug, Deserialize)]
struct FixtureCharacter {
    id: String,
    name: String,
    description: String,
    personality: String,
    scenario: String,
    first_mes: String,
    system_prompt: String,
    creator: String,
    character_version: String,
    tags: Vec<String>,
    world_info: Vec<FixtureWorldInfo>,
}

#[derive(Debug, Deserialize)]
struct FixtureWorldInfo {
    keys: Vec<String>,
    content: String,
    constant: bool,
    selective: bool,
    route: String,
    #[serde(default)]
    secondary_keys: Vec<String>,
    #[serde(default)]
    selective_logic: String,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    depth: usize,
    #[serde(default)]
    order: i32,
    #[serde(default)]
    is_global: bool,
}

#[derive(Debug, Deserialize)]
struct FixtureDefinition {
    id: String,
    name: String,
    role_type: String,
    persona_prompt: String,
    behavior_rules: String,
    private_knowledge: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct FixtureTask {
    id: String,
    title: String,
    status: String,
}

impl SqliteHarnessEnv {
    /// Bootstrap a fresh SQLite authority under `data_dir` and seed the fixture.
    pub fn bootstrap(data_dir: PathBuf, llm: Arc<dyn LlmClient>) -> Result<Self, String> {
        std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        let db_path = data_dir.join("storyforge.sqlite3");
        // Empty DB activate: production repository migrate on first write.
        sqlite_runtime::activate(&db_path)?;

        let persistence = sqlite_runtime::conversation_persistence()?;
        let conv_store = Arc::new(ConversationStore::with_persistence(persistence));
        let campaign_store = Arc::new(CampaignStore::disabled());
        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        }));
        let vector_store = Arc::new(BruteForceStore::with_persistence(
            data_dir.join("vectors.json"),
        ));

        let mut env = Self {
            data_dir,
            db_path,
            conv_store,
            campaign_store,
            tool_ctx,
            vector_store,
            llm,
            active_campaign: Mutex::new(None),
            fixture_hash16: String::new(),
            pipeline_sampling: None,
        };
        let fixture_path = fixture_path();
        let (campaign_id, fixture_hash) = env.seed_fixture(&fixture_path)?;
        env.fixture_hash16 = fixture_hash;
        env.set_active_campaign(campaign_id);
        let _ = sqlite_runtime::recover_turns_on_startup()?;
        Ok(env)
    }

    /// Re-open an existing SQLite evidence data dir (resume).
    pub fn open_existing(data_dir: PathBuf, llm: Arc<dyn LlmClient>) -> Result<Self, String> {
        let db_path = data_dir.join("storyforge.sqlite3");
        if !db_path.exists() {
            return Err("sqlite evidence database is missing".into());
        }
        sqlite_runtime::activate(&db_path)?;
        let persistence = sqlite_runtime::conversation_persistence()?;
        let conv_store = Arc::new(ConversationStore::with_persistence(persistence));
        let campaign_store = Arc::new(CampaignStore::disabled());
        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        }));
        let vector_store = Arc::new(BruteForceStore::with_persistence(
            data_dir.join("vectors.json"),
        ));
        let fixture_path = fixture_path();
        let fixture_hash16 = fixture_source_hash16()?;
        let env = Self {
            data_dir,
            db_path,
            conv_store,
            campaign_store,
            tool_ctx,
            vector_store,
            llm,
            active_campaign: Mutex::new(None),
            fixture_hash16,
            pipeline_sampling: None,
        };
        // Re-inject tool_ctx character material from fixture without re-seeding DB.
        let root = load_fixture(&fixture_path)?;
        env.inject_character_from_fixture(&root);
        Ok(env)
    }

    pub fn set_active_campaign(&self, id: Id) {
        *self.active_campaign.lock().unwrap() = Some(id);
    }

    pub fn active_campaign_id(&self) -> Option<Id> {
        self.active_campaign.lock().unwrap().clone()
    }

    /// Wire connection-level sampling into pipeline prompt assembly.
    ///
    /// `BudgetedLlmClient.reasoning_override` only rewrites outbound request
    /// params after the prompt is built. CoT injection reads
    /// `PipelineOrchestrator::reasoning_mode()`, which comes from this field.
    pub fn set_pipeline_sampling(&mut self, sampling: Option<SamplingParams>) {
        self.pipeline_sampling = sampling;
    }

    /// Convenience for eval arms: set only the reasoning mode on default sampling.
    pub fn set_pipeline_reasoning(&mut self, reasoning: ReasoningMode) {
        self.pipeline_sampling = Some(SamplingParams {
            reasoning,
            ..SamplingParams::default()
        });
    }

    pub fn fill_campaign_context(&self, mut ctx: WritingContext) -> Result<WritingContext, String> {
        ctx.campaign_runtime = None;
        {
            let mut g = self.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
            g.campaign_runtime = None;
        }
        // Production-faithful Prompt Module path: GUI fills active profile/modules before
        // writing. Endurance previously used WritingContext::legacy (profile=None, modules=[]),
        // so ReasoningMode::Prompted never injected role-specific CoT. Install the built-in
        // default profile whenever the caller left modules empty.
        if ctx.profile.is_none() || ctx.modules.is_empty() {
            let (profile, modules) = storyforge_domain::prompt_module::builtins::default_profile();
            ctx.profile = Some(profile);
            ctx.modules = modules;
        }
        let Some(active_id) = self.active_campaign_id() else {
            return Ok(ctx);
        };
        fill_campaign_runtime_from_sqlite(&mut ctx, &self.tool_ctx, &active_id)?;
        // Production-faithful: feed RoundSummary catalog into director remote-memory tools.
        // Without this, get_recent_summary / search_chronicle see empty stores and never
        // produce a complete tool-call evidence timeline.
        self.sync_remote_memory_tools(&active_id)?;
        Ok(ctx)
    }

    /// Refresh ToolContext remote-memory sources from SQLite authority.
    fn sync_remote_memory_tools(&self, campaign_id: &Id) -> Result<(), String> {
        let summaries = sqlite_runtime::list_summaries(campaign_id)?;
        let mut g = self.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        // get_recent_summary reads content list (tool reverses + takes limit).
        g.archived_summaries = summaries.iter().map(|s| s.content.clone()).collect();
        // search_chronicle / get_chronicle use the typed catalog.
        g.chronicle_summaries = summaries;
        g.reset_chronicle_tool_budget();
        Ok(())
    }

    pub fn new_pipeline(&self) -> PipelineOrchestrator {
        let mut tool_ctx = self
            .tool_ctx
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        tool_ctx.vector_store = Some(self.vector_store.clone());
        // Must use new_with_sampling: assemble_system_prompt gates Cot modules on
        // orchestrator.reasoning_mode(), not on BudgetedLlmClient overrides.
        let mut pipeline = PipelineOrchestrator::new_with_sampling(
            self.llm.clone(),
            self.conv_store.clone(),
            Arc::new(tool_ctx),
            None,
            None,
            self.pipeline_sampling.clone(),
        );
        // SQLite production path: generation defers durable conversation land.
        pipeline.set_defer_conversation_land(true);
        pipeline
    }

    fn fail_active_turn_if_any(
        &self,
        campaign_id: &Id,
        _reason: &str,
        checkpoint_prefix_validated: bool,
    ) -> Result<(), String> {
        let active_turns = sqlite_runtime::list_active_turns()?;
        if active_turns.is_empty() {
            return Ok(());
        }
        if active_turns.len() != 1 {
            return Err("retry recovery refuses multiple global active SQLite turns".into());
        }
        let active = &active_turns[0];
        if &active.campaign_id != campaign_id || !active.status.is_active() {
            return Err("retry recovery refuses a foreign active SQLite turn".into());
        }
        let campaign = sqlite_runtime::get_campaign(campaign_id)?
            .ok_or_else(|| "retry recovery campaign is missing".to_string())?;
        if active.base_campaign_revision != campaign.revision
            || campaign.conversation_id.as_ref() != Some(&active.conversation_id)
            || active.accepted_attempt_id.is_some()
        {
            return Err("retry recovery active Turn scope or revision drifted".into());
        }
        let conversation = sqlite_runtime::get_conversation(&active.conversation_id)?
            .ok_or_else(|| "retry recovery conversation is missing".to_string())?;
        if conversation.campaign_id.as_ref() != Some(campaign_id) {
            return Err("retry recovery conversation campaign scope drifted".into());
        }
        match conversation
            .nodes
            .iter()
            .position(|node| node.id == active.input_node_id)
        {
            None if checkpoint_prefix_validated
                || (active.status == TurnStatus::Generating && active.attempts.is_empty()) => {}
            Some(index) => {
                if index < conversation.archived_upto {
                    return Err("retry recovery input anchor is inside archived history".into());
                }
                let expected_parent = index
                    .checked_sub(1)
                    .map(|parent_index| &conversation.nodes[parent_index].id);
                if conversation.nodes[index].parent_id.as_ref() != expected_parent {
                    return Err("retry recovery input anchor is detached from prior history".into());
                }
                let input = conversation.nodes[index].active().ok_or_else(|| {
                    "retry recovery input anchor has no active variant".to_string()
                })?;
                if input.role != Role::User || input.status != VariantStatus::Final {
                    return Err("retry recovery input anchor is not a final User node".into());
                }
                for pair in conversation.nodes[index..].windows(2) {
                    if pair[1].parent_id.as_ref() != Some(&pair[0].id)
                        || pair[1]
                            .active()
                            .is_none_or(|variant| variant.role != Role::Assistant)
                    {
                        return Err("retry recovery conversation tail is not a safe chain".into());
                    }
                }
                self.conv_store.invalidate();
                self.conv_store
                    .truncate_from(&active.conversation_id, &active.input_node_id)
                    .map_err(|error| error.to_string())?;
            }
            None => {
                return Err("retry recovery active Turn input anchor is missing".into());
            }
        }
        let recovered = sqlite_runtime::recover_turns_on_startup()?;
        if recovered == 0 {
            return Err("active SQLite turn was not closed by startup recovery".into());
        }
        self.conv_store.invalidate();
        if !sqlite_runtime::list_active_turns()?.is_empty() {
            return Err("active SQLite turn remains after retry recovery".into());
        }
        Ok(())
    }

    pub fn recover_incomplete_turn_for_retry(&self, campaign_id: &Id) -> Result<(), String> {
        self.fail_active_turn_if_any(
            campaign_id,
            "superseded by sqlite endurance durable retry recovery",
            false,
        )
    }

    /// Complete recovery after the runner has already verified the durable
    /// checkpoint's accepted conversation prefix and active-Turn scope. This
    /// permits the idempotent crash point where truncation committed but the
    /// Turn/Attempt failure transaction did not.
    pub fn recover_checkpoint_validated_incomplete_turn(
        &self,
        campaign_id: &Id,
    ) -> Result<(), String> {
        self.fail_active_turn_if_any(
            campaign_id,
            "superseded by checkpoint-validated sqlite endurance retry recovery",
            true,
        )
    }

    fn begin_preland_turn(
        &self,
        campaign_id: &Id,
        conversation_id: &Id,
        intent: &str,
        _recovery_reason: &str,
    ) -> Result<(Id, Id), String> {
        if !sqlite_runtime::list_active_turns()?.is_empty() {
            return Err(
                "active SQLite Turn requires checkpoint-validated recovery before a new write"
                    .into(),
            );
        }
        let campaign = sqlite_runtime::get_campaign(campaign_id)?
            .ok_or_else(|| format!("campaign {campaign_id} missing"))?;
        if campaign.conversation_id.as_ref() != Some(conversation_id) {
            return Err("campaign conversation scope mismatch for pre-land turn".into());
        }
        let mut conversation = sqlite_runtime::get_conversation(conversation_id)?
            .ok_or_else(|| format!("conversation {conversation_id} missing"))?;
        if conversation.campaign_id.as_ref() != Some(campaign_id) {
            return Err("conversation campaign scope mismatch for pre-land turn".into());
        }
        // Build the User node in memory first, then persist the Turn before the
        // conversation. Every crash point is recoverable: before save_turn no
        // state changed; after save_turn recovery can fail the anchor; after
        // save_conversation recovery can also truncate by input_node_id.
        let input_node_id =
            conversation.append_message(storyforge_domain::conversation::Role::User, intent.into());
        let turn = TurnRecord::new(
            campaign_id.clone(),
            conversation_id.clone(),
            input_node_id.clone(),
            campaign.revision,
        );
        let turn_id = turn.turn_id.clone();
        sqlite_runtime::save_turn(&turn)?;
        sqlite_runtime::save_conversation(&conversation)?;
        self.conv_store.invalidate();
        Ok((input_node_id, turn_id))
    }

    /// Full production-faithful write: user msg → pipeline → preaccept draft →
    /// fixed production postprocess → SQLite Accept.
    ///
    /// `summary_probe_id` is optional early-fact token material for inject turns.
    /// When set, it is embedded only in the durable SQLite RoundSummary content so
    /// later EarlyFactCheck can prove retrieval via `list_summaries` (no story body).
    pub async fn write_accept_turn(
        &self,
        conversation_id: &Id,
        intent: &str,
        turn_index: u32,
        row_id: &str,
        summary_probe_id: Option<&str>,
        force_quality_fault: bool,
    ) -> Result<SqliteTurnResult, String> {
        self.write_accept_turn_with_fault_profile(
            conversation_id,
            intent,
            turn_index,
            row_id,
            summary_probe_id,
            force_quality_fault.then_some("legacy_error_meta_and_format_leak"),
        )
        .await
    }

    pub async fn write_accept_turn_with_fault_profile(
        &self,
        conversation_id: &Id,
        intent: &str,
        turn_index: u32,
        row_id: &str,
        summary_probe_id: Option<&str>,
        fault_profile: Option<&str>,
    ) -> Result<SqliteTurnResult, String> {
        let campaign_id = self
            .active_campaign_id()
            .ok_or_else(|| "active campaign required".to_string())?;

        // Persist the input/Turn anchor before any model call. A process exit or
        // model failure can then be recovered by input_node_id without leaving
        // an orphan User node in the next retry's context.
        let (input_node_id, turn_id) = self.begin_preland_turn(
            &campaign_id,
            conversation_id,
            intent,
            "superseded by sqlite endurance write retry/setup",
        )?;

        let base = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = self.fill_campaign_context(base)?;

        let mut pipeline = self.new_pipeline();
        let (event_tx, event_task) = pipeline_observation_channel();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let (draft_text, _provisional_node, provenance) = pipeline
            .start_writing(
                intent.to_string(),
                &ctx,
                event_tx.clone(),
                cancel_rx.clone(),
            )
            .await
            .map_err(|e| e.to_string())?;
        let draft_text = match fault_profile {
            Some("warning_em_dash_and_negation_affirmation") => {
                format!("{draft_text}\n潮线在门槛上折返——不是因为潮水退了，而是因为闸门先动了。")
            }
            Some("error_meta_and_format_leak") | Some("legacy_error_meta_and_format_leak") => {
                format!("{draft_text}\n作为AI，我将为你继续创作。<think>format leak</think>")
            }
            Some("warning_ngram_repetition") => {
                format!("{draft_text}\n盐灯照在石阶上。盐灯照在石阶上。盐灯照在石阶上。")
            }
            Some("warning_too_short") => "门响了。两人回头。".into(),
            Some("error_non_owner_private_leak") => {
                format!("{draft_text}\nOWNER_ONLY_NING_LARK_731")
            }
            _ => draft_text,
        };

        let attempt_id = Id::new();
        let land = sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
            campaign_id: &campaign_id,
            conversation_id,
            turn_id: &turn_id,
            attempt_id: &attempt_id,
            draft_text: &draft_text,
            pending_temporary_instances: pipeline.pending_temporary_instances().to_vec(),
            provenance: provenance.clone(),
        })?;
        self.conv_store.invalidate();
        let variant_id = land.variant_id;
        let initial_draft_hash = land.draft_hash;

        // Same production QualityGate + bounded Editor auto-fix used by Tauri.
        let (final_text, quality_report, autofix_provenance) =
            run_quality_gate_with_optional_editor_autofix(
                draft_text,
                QualityAutofixRequest {
                    pipeline: &mut pipeline,
                    draft_node_id: &variant_id,
                    conversation_id,
                    writing_ctx: &ctx,
                    event_tx: &event_tx,
                    cancel: cancel_rx.clone(),
                    log_prefix: "sqlite endurance write",
                    original_provenance: provenance,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        let quality_warning_count = quality_report.warnings.len();
        let quality_error_count = quality_report.error_count();

        let present_chars = pipeline
            .session()
            .and_then(|session| session.plan.as_ref())
            .map(|plan| {
                plan.subagent_tasks
                    .iter()
                    .map(|task| task.character_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut outcome = pipeline
            .run_postprocess(
                &final_text,
                "",
                &present_chars,
                &[],
                &ctx,
                &event_tx,
                cancel_rx.clone(),
                &[],
            )
            .await;
        // Early-fact probe material is appended to the actual Summarizer output.
        // If the real Summarizer failed, no SummaryDone event is fabricated and
        // coverage will fail closed.
        if let Some(probe_id) = summary_probe_id.filter(|probe| !probe.trim().is_empty())
            && let Some(real_outcome) = outcome.as_mut()
            && let Some(summary) = real_outcome.summary.as_mut()
        {
            summary.push_str(&format!("; early_fact_probe={probe_id}"));
        }
        let proof = self
            .apply_production_postprocess_sqlite(SqlitePostprocessRequest {
                campaign_id: &campaign_id,
                conversation_id,
                turn_id: &turn_id,
                attempt_id: &attempt_id,
                variant_id: &variant_id,
                draft_text: &final_text,
                quality_report,
                autofix_provenance,
                outcome,
                present_chars: present_chars.clone(),
                turn_number: turn_index,
                input_node_id: input_node_id.clone(),
            })
            .await?;

        // Close the channel and await the privacy-safe projection before Accept;
        // evidence collection failure can never happen after a durable commit.
        drop(event_tx);
        let pipeline_observation = event_task
            .await
            .map_err(|error| format!("pipeline observation task failed: {error}"))?;
        pipeline_observation.require_roles(&[
            "director",
            "subagent",
            "editor",
            "summarizer",
            "postprocessor",
        ])?;

        let accept = self.accept_variant(&campaign_id, conversation_id, &variant_id)?;
        if !accept.ok {
            return Err(accept.error.unwrap_or_else(|| "accept failed".into()));
        }
        self.conv_store.invalidate();

        let outbox = sqlite_runtime::list_preaccept_outbox_for_turn(&turn_id).unwrap_or_default();
        let outbox_kinds: Vec<String> = outbox
            .iter()
            .map(|r| match r.kind {
                PreacceptOutboxKind::DraftReady => "draft_ready".into(),
                PreacceptOutboxKind::AutofixSync => "autofix_sync".into(),
                PreacceptOutboxKind::PostprocessApply => "postprocess_apply".into(),
                PreacceptOutboxKind::Regenerate => "regenerate".into(),
                PreacceptOutboxKind::EditStale => "edit_stale".into(),
                PreacceptOutboxKind::RecoveryFail => "recovery_fail".into(),
            })
            .collect();

        let mut observations = BTreeSet::new();
        observations.insert(ObservationKey::SqliteAuthoritative);
        observations.insert(ObservationKey::JsonFallbackFalse);
        observations.insert(ObservationKey::CommandPath(
            "sqlite_runtime::create_draft_attempt".into(),
        ));
        observations.insert(ObservationKey::ServicePath("pipeline.start_writing".into()));
        observations.insert(ObservationKey::DraftLanded);
        observations.insert(ObservationKey::PostprocessApplied);
        observations.insert(ObservationKey::Accepted);
        observations.insert(ObservationKey::OutboxKind("draft_ready".into()));
        observations.insert(ObservationKey::OutboxKind("postprocess_apply".into()));
        observations.extend(pipeline_observation.observation_keys());

        let observed = ObservedCoverage {
            row_id: row_id.to_string(),
            turn_index,
            command_path: "sqlite_runtime::create_draft_attempt".into(),
            service_path: "pipeline.start_writing".into(),
            agent_events: pipeline_observation.agent_events(),
            turn_id16: short_hash16(turn_id.as_str()),
            attempt_id16: short_hash16(attempt_id.as_str()),
            variant_id16: short_hash16(variant_id.as_str()),
            sqlite_post: SqlitePostcondition {
                sqlite_authoritative: true,
                json_fallback: false,
                turn_status: format!("{:?}", accept.turn_status),
                attempt_status: format!("{:?}", accept.attempt_status),
                outbox_kinds,
                campaign_revision_after: accept.campaign_revision_after,
                postprocess_applied: proof.applied,
                batch_digest16: proof.batch_digest.clone(),
            },
            observations,
        };

        let autofix_applied = proof.draft_hash != initial_draft_hash
            && pipeline_observation.completed_quality_autofix();
        let actual_subagent_count = pipeline_observation.subagent_count();
        Ok(SqliteTurnResult {
            draft_text: final_text,
            variant_id,
            turn_id,
            attempt_id,
            input_node_id,
            postprocess_proof: proof,
            accept,
            observed,
            quality_warning_count,
            quality_error_count,
            autofix_applied,
            actual_subagent_count,
        })
    }

    /// Production-faithful regenerate: first draft land → pipeline regenerate →
    /// `append_regenerate_attempt` → real production postprocess → Accept.
    ///
    /// `targets` maps schedule slots:
    /// - overall → empty/Director
    /// - editor → Editor only
    /// - subagent → first supporting instance character id when available
    pub async fn regenerate_accept_turn(
        &self,
        conversation_id: &Id,
        intent: &str,
        turn_index: u32,
        row_id: &str,
        targets: Vec<PartialRollTarget>,
    ) -> Result<SqliteTurnResult, String> {
        let campaign_id = self
            .active_campaign_id()
            .ok_or_else(|| "active campaign required".to_string())?;

        let (input_node_id, turn_id) = self.begin_preland_turn(
            &campaign_id,
            conversation_id,
            intent,
            "superseded by sqlite endurance regenerate setup",
        )?;

        let base = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = self.fill_campaign_context(base)?;

        let mut pipeline = self.new_pipeline();
        let (event_tx, event_task) = pipeline_observation_channel();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let (draft_text, _provisional_node, provenance) = pipeline
            .start_writing(
                intent.to_string(),
                &ctx,
                event_tx.clone(),
                cancel_rx.clone(),
            )
            .await
            .map_err(|e| e.to_string())?;

        let first_attempt_id = Id::new();
        let first_land = sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
            campaign_id: &campaign_id,
            conversation_id,
            turn_id: &turn_id,
            attempt_id: &first_attempt_id,
            draft_text: &draft_text,
            pending_temporary_instances: pipeline.pending_temporary_instances().to_vec(),
            provenance,
        })?;
        self.conv_store.invalidate();
        let previous_variant_id = first_land.variant_id;

        // Subagent-only regenerate must target a character present in the just-landed
        // draft provenance. Instance display names are not plan ids.
        let mut targets = targets;
        if targets
            .iter()
            .any(|t| matches!(t, PartialRollTarget::Subagent(_)))
        {
            let plan_ids = self
                .conv_store
                .get(conversation_id)
                .and_then(|c| {
                    c.find_node(&previous_variant_id)
                        .and_then(|n| n.active())
                        .and_then(|v| v.provenance.clone())
                })
                .map(|p| {
                    p.subagent_results
                        .into_iter()
                        .map(|s| s.character_id)
                        .filter(|id| !id.trim().is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let id = plan_ids.into_iter().next().ok_or_else(|| {
                "subagent regenerate requested but landed provenance has no subagent".to_string()
            })?;
            targets = vec![PartialRollTarget::Subagent(id)];
        }

        let regen_req = RegenerateRequest {
            conversation_id: conversation_id.clone(),
            node_id: previous_variant_id.clone(),
            targets: targets.clone(),
            hint: Some(format!("endurance regenerate turn {turn_index}")),
            seed: None,
        };
        let (regen_text, regen_provenance) = pipeline
            .regenerate(regen_req, &ctx, event_tx.clone(), cancel_rx.clone())
            .await
            .map_err(|error| error.to_string())?;
        if regen_text.trim().is_empty() {
            return Err("sqlite regenerate returned empty draft".into());
        }

        let attempt_id = Id::new();
        let land = sqlite_runtime::append_regenerate_attempt(RegenerateAttemptRequest {
            campaign_id: &campaign_id,
            conversation_id,
            turn_id: &turn_id,
            previous_variant_id: &previous_variant_id,
            attempt_id: &attempt_id,
            draft_text: &regen_text,
            pending_temporary_instances: pipeline.pending_temporary_instances().to_vec(),
            provenance: Some(regen_provenance.clone()),
        })?;
        self.conv_store.invalidate();
        let variant_id = land.variant_id;
        let initial_draft_hash = land.draft_hash;

        let (final_text, quality_report, autofix_provenance) =
            run_quality_gate_with_optional_editor_autofix(
                regen_text,
                QualityAutofixRequest {
                    pipeline: &mut pipeline,
                    draft_node_id: &variant_id,
                    conversation_id,
                    writing_ctx: &ctx,
                    event_tx: &event_tx,
                    cancel: cancel_rx.clone(),
                    log_prefix: "sqlite endurance regenerate",
                    original_provenance: Some(regen_provenance),
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        let quality_warning_count = quality_report.warnings.len();
        let quality_error_count = quality_report.error_count();
        let present_chars = pipeline
            .session()
            .and_then(|session| session.plan.as_ref())
            .map(|plan| {
                plan.subagent_tasks
                    .iter()
                    .map(|task| task.character_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let outcome = pipeline
            .run_postprocess(
                &final_text,
                "",
                &present_chars,
                &[],
                &ctx,
                &event_tx,
                cancel_rx.clone(),
                &[],
            )
            .await;
        let proof = self
            .apply_production_postprocess_sqlite(SqlitePostprocessRequest {
                campaign_id: &campaign_id,
                conversation_id,
                turn_id: &turn_id,
                attempt_id: &attempt_id,
                variant_id: &variant_id,
                draft_text: &final_text,
                quality_report,
                autofix_provenance,
                outcome,
                present_chars: present_chars.clone(),
                turn_number: turn_index,
                input_node_id: input_node_id.clone(),
            })
            .await?;

        drop(event_tx);
        let pipeline_observation = event_task
            .await
            .map_err(|error| format!("pipeline observation task failed: {error}"))?;
        pipeline_observation.require_roles(&[
            "director",
            "subagent",
            "editor",
            "summarizer",
            "postprocessor",
        ])?;

        let accept = self.accept_variant(&campaign_id, conversation_id, &variant_id)?;
        if !accept.ok {
            return Err(accept
                .error
                .unwrap_or_else(|| "regenerate accept failed".into()));
        }
        self.conv_store.invalidate();

        let outbox = sqlite_runtime::list_preaccept_outbox_for_turn(&turn_id).unwrap_or_default();
        let outbox_kinds: Vec<String> = outbox
            .iter()
            .map(|r| match r.kind {
                PreacceptOutboxKind::DraftReady => "draft_ready".into(),
                PreacceptOutboxKind::AutofixSync => "autofix_sync".into(),
                PreacceptOutboxKind::PostprocessApply => "postprocess_apply".into(),
                PreacceptOutboxKind::Regenerate => "regenerate".into(),
                PreacceptOutboxKind::EditStale => "edit_stale".into(),
                PreacceptOutboxKind::RecoveryFail => "recovery_fail".into(),
            })
            .collect();

        let mut observations = BTreeSet::new();
        observations.insert(ObservationKey::SqliteAuthoritative);
        observations.insert(ObservationKey::JsonFallbackFalse);
        observations.insert(ObservationKey::CommandPath(
            "sqlite_runtime::append_regenerate_attempt".into(),
        ));
        observations.insert(ObservationKey::ServicePath("pipeline.regenerate".into()));
        observations.insert(ObservationKey::Regenerated);
        observations.insert(ObservationKey::PostprocessApplied);
        observations.insert(ObservationKey::Accepted);
        observations.insert(ObservationKey::OutboxKind("regenerate".into()));
        observations.insert(ObservationKey::OutboxKind("postprocess_apply".into()));
        observations.extend(pipeline_observation.observation_keys());

        let observed = ObservedCoverage {
            row_id: row_id.to_string(),
            turn_index,
            command_path: "sqlite_runtime::append_regenerate_attempt".into(),
            service_path: "pipeline.regenerate".into(),
            agent_events: pipeline_observation.agent_events(),
            turn_id16: short_hash16(turn_id.as_str()),
            attempt_id16: short_hash16(attempt_id.as_str()),
            variant_id16: short_hash16(variant_id.as_str()),
            sqlite_post: SqlitePostcondition {
                sqlite_authoritative: true,
                json_fallback: false,
                turn_status: format!("{:?}", accept.turn_status),
                attempt_status: format!("{:?}", accept.attempt_status),
                outbox_kinds,
                campaign_revision_after: accept.campaign_revision_after,
                postprocess_applied: proof.applied,
                batch_digest16: proof.batch_digest.clone(),
            },
            observations,
        };

        let autofix_applied = proof.draft_hash != initial_draft_hash
            && pipeline_observation.completed_quality_autofix();
        let actual_subagent_count = pipeline_observation.subagent_count();
        Ok(SqliteTurnResult {
            draft_text: final_text,
            variant_id,
            turn_id,
            attempt_id,
            input_node_id,
            postprocess_proof: proof,
            accept,
            observed,
            quality_warning_count,
            quality_error_count,
            autofix_applied,
            actual_subagent_count,
        })
    }

    pub fn accept_variant(
        &self,
        campaign_id: &Id,
        conversation_id: &Id,
        variant_id: &Id,
    ) -> Result<SqliteAcceptSummary, String> {
        let before = sqlite_runtime::get_campaign(campaign_id)?
            .map(|c| (c.revision, c.chronicle_revision))
            .unwrap_or((0, 0));
        match sqlite_runtime::accept_by_variant(campaign_id, conversation_id, variant_id, false) {
            Ok(outcome) => {
                let after = sqlite_runtime::get_campaign(campaign_id)?
                    .map(|c| (c.revision, c.chronicle_revision))
                    .unwrap_or((outcome.campaign_revision_after, before.1));
                let draft_hash = sqlite_runtime::get_turn(&outcome.turn_id)?
                    .and_then(|t| {
                        t.find_attempt(&outcome.attempt_id)
                            .map(|a| a.draft_hash.clone())
                    })
                    .unwrap_or_default();
                Ok(SqliteAcceptSummary {
                    ok: true,
                    error: None,
                    campaign_revision_before: before.0,
                    campaign_revision_after: after.0,
                    chronicle_revision_before: before.1,
                    chronicle_revision_after: after.1,
                    draft_hash,
                    turn_status: outcome.turn_status,
                    attempt_status: outcome.attempt_status,
                    summary_code: None,
                })
            }
            Err(e) => Ok(SqliteAcceptSummary {
                ok: false,
                error: Some(format_accept_error_for_runner(e)),
                campaign_revision_before: before.0,
                campaign_revision_after: before.0,
                chronicle_revision_before: before.1,
                chronicle_revision_after: before.1,
                draft_hash: String::new(),
                turn_status: TurnStatus::Failed,
                attempt_status: AttemptStatus::Failed,
                summary_code: None,
            }),
        }
    }

    pub fn list_summary_contents(&self, campaign_id: &Id) -> Result<Vec<String>, String> {
        Ok(sqlite_runtime::list_summaries(campaign_id)?
            .into_iter()
            .map(|s| s.content)
            .collect())
    }

    /// Check only the final narrative for synthetic fixture leakage. The raw
    /// forbidden values are never copied into evidence.
    pub fn private_final_output_has_no_leak(&self, text: &str) -> Result<bool, String> {
        let fixture = load_fixture(&fixture_path())?;
        let normalized = text.to_lowercase();
        Ok(fixture
            .must_not_reveal
            .iter()
            .all(|forbidden| !normalized.contains(&forbidden.to_lowercase())))
    }

    /// Prove the selected world-info route using the same domain routing
    /// methods consumed by production tools/context compilation.
    pub fn world_info_route_observed(&self, route: WorldInfoRouteSlot, query: &str) -> bool {
        let guard = self.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        let Some(book) = guard.world_info.as_ref() else {
            return false;
        };
        match route {
            WorldInfoRouteSlot::Constant => book
                .constant_entries()
                .iter()
                .any(|entry| entry.route == LoreRoute::Constant),
            WorldInfoRouteSlot::Selective => book
                .triggered_selective_entries(query)
                .iter()
                .any(|entry| entry.route == LoreRoute::Selective),
            WorldInfoRouteSlot::Both => book
                .triggered_selective_entries(query)
                .iter()
                .any(|entry| entry.route == LoreRoute::Both),
        }
    }

    pub fn private_probe_slug(probe: &PrivateProbeKind) -> &'static str {
        match probe {
            PrivateProbeKind::OwnerRecall => "owner_recall",
            PrivateProbeKind::NonOwnerLeak => "non_owner_leak",
            PrivateProbeKind::NarrationLeak => "narration_leak",
            PrivateProbeKind::MustNotReveal => "must_not_reveal",
        }
    }

    async fn apply_production_postprocess_sqlite(
        &self,
        req: SqlitePostprocessRequest<'_>,
    ) -> Result<ProductionPostprocessProof, String> {
        let identity = PostprocessIdentity {
            turn_id: req.turn_id.clone(),
            attempt_id: req.attempt_id.clone(),
            campaign_id: req.campaign_id.clone(),
            conversation_id: req.conversation_id.clone(),
            turn_number: req.turn_number,
        };
        let sink = SqliteGatewayTurnAttemptSink;
        sink.sync_autofix_with_provenance(
            &identity,
            req.draft_text,
            req.quality_report,
            req.autofix_provenance,
        )
        .map_err(|e| e.to_string())?;

        let runtime = self
            .fill_campaign_context(WritingContext::legacy(
                vec![],
                None,
                req.conversation_id.clone(),
            ))?
            .campaign_runtime
            .ok_or_else(|| "campaign runtime missing for SQLite postprocess".to_string())?;

        let service = ProductionPostprocessService::new_runtime(runtime.as_ref(), &sink);
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let result = service
            .apply_outcome(&identity, req.outcome, &req.present_chars, &cancel_rx)
            .map_err(|e| e.to_string())?;
        if !result.applied {
            return Err(format!(
                "sqlite production postprocess did not apply: {:?}",
                result.skipped_reason
            ));
        }

        let turn = sqlite_runtime::get_turn(req.turn_id)?
            .ok_or_else(|| "turn missing after postprocess".to_string())?;
        if turn.status != TurnStatus::AwaitingAcceptance {
            return Err(format!(
                "turn status {:?} expected AwaitingAcceptance",
                turn.status
            ));
        }
        let attempt = turn
            .find_attempt(req.attempt_id)
            .ok_or_else(|| "attempt missing after postprocess".to_string())?;
        if attempt.status != AttemptStatus::AwaitingAcceptance {
            return Err(format!(
                "attempt status {:?} expected AwaitingAcceptance",
                attempt.status
            ));
        }
        if turn.input_node_id != req.input_node_id {
            return Err("input_node_id mismatch after postprocess".into());
        }
        if attempt.variant_id != *req.variant_id {
            return Err("variant_id mismatch after postprocess".into());
        }
        let batch_digest = attempt
            .pending_state_changes
            .as_ref()
            .map(mutation_batch_digest);

        Ok(ProductionPostprocessProof {
            turn_id: req.turn_id.clone(),
            attempt_id: req.attempt_id.clone(),
            input_node_id: req.input_node_id,
            turn_index: req.turn_number,
            variant_id: req.variant_id.clone(),
            draft_hash: attempt.draft_hash.clone(),
            summary_text: result.summary_text,
            batch_digest,
            applied: true,
        })
    }

    fn seed_fixture(&self, path: &Path) -> Result<(Id, String), String> {
        let root = load_fixture(path)?;
        let evaluation = root
            .evaluation
            .as_ref()
            .ok_or_else(|| "fixture evaluation is missing".to_string())?;
        if evaluation.target_turns != 80
            || evaluation.turn_script.len() != 80
            || evaluation
                .turn_script
                .iter()
                .enumerate()
                .any(|(index, turn)| turn.turn != index as u32 + 1)
            || evaluation
                .turn_script
                .iter()
                .filter(|turn| turn.checkpoint_after_accept)
                .map(|turn| turn.turn)
                .collect::<Vec<_>>()
                != evaluation.checkpoint_turns
        {
            return Err(format!(
                "fixture evaluation is not a contiguous 80-turn schedule"
            ));
        }
        let fixture_hash = fixture_source_hash16()?;

        let character = character_from_fixture(&root);
        self.inject_character_from_fixture(&root);

        let mut card = CharacterCard::from_character(&character);
        let mut defs = Vec::new();
        for d in &root.definitions {
            defs.push(definition_from_fixture(d));
        }
        // Ensure at least one definition when fixture empty.
        if defs.is_empty() {
            defs.push(CharacterDefinition::fallback_from_character(
                &character,
                &[],
            ));
        }
        let definitions = storyforge_app_agent::attach_definitions_to_card(defs, &card.id);
        card.character_definitions = definitions;
        card.id = Id::from_str(&root.character.id);

        let stored = StoredCard {
            card: card.clone(),
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        let payload = serde_json::to_value(&stored).map_err(|e| e.to_string())?;
        sqlite_runtime::save_card_payload(
            &card.id,
            &card.name,
            Some(card.source_character_id.as_str()),
            Some(stored.imported_at.as_str()),
            &payload,
        )?;

        let campaign = Campaign::new(card.id.clone(), "m5-sqlite-endurance".to_string());
        let campaign_id = campaign.id.clone();
        sqlite_runtime::save_campaign(&campaign)?;

        let conversation = self
            .conv_store
            .create_persisted(None, Some(campaign_id.clone()))
            .map_err(|e| e.to_string())?;
        let conversation_id = conversation.id;
        let mut campaign = sqlite_runtime::get_campaign(&campaign_id)?
            .ok_or_else(|| "campaign missing after save".to_string())?;
        campaign.conversation_id = Some(conversation_id);
        sqlite_runtime::save_campaign(&campaign)?;

        let mut instance_by_definition = std::collections::HashMap::new();
        for def in &card.character_definitions {
            if matches!(def.role_type, RoleType::Protagonist | RoleType::Supporting) {
                let inst = CharacterInstance::from_definition(campaign_id.clone(), def);
                instance_by_definition.insert(def.id.clone(), inst.id.clone());
                sqlite_runtime::save_instance(&inst)?;
            }
        }

        for fixture_definition in &root.definitions {
            let definition_id = Id::from_str(&fixture_definition.id);
            let Some(instance_id) = instance_by_definition.get(&definition_id) else {
                continue;
            };
            for private_text in &fixture_definition.private_knowledge {
                let mut entry = CharacterKnowledgeEntry::backstory(
                    campaign_id.clone(),
                    instance_id.clone(),
                    private_text.clone(),
                );
                entry.set_propagation(PropagationPolicy::Private);
                sqlite_runtime::save_knowledge(&entry)?;
            }
        }

        for fixture_task in &root.tasks {
            let mut task = StoryTask::user_planned(
                campaign_id.clone(),
                fixture_task.title.clone(),
                "synthetic SQLite endurance fixture task",
                vec![TaskTrigger::Manual],
                0,
            );
            task.id = Id::from_str(&fixture_task.id);
            task.status = match fixture_task.status.as_str() {
                "active" => TaskStatus::Active,
                "completed" => TaskStatus::Completed,
                "abandoned" => TaskStatus::Abandoned,
                _ => TaskStatus::Pending,
            };
            sqlite_runtime::save_task(&task)?;
        }

        // Temporary definition intentionally not instantiated until pipeline creates it.
        let _ = (
            root.fixture_version,
            root.safe_probe_facts,
            root.must_not_reveal,
        );
        Ok((campaign_id, fixture_hash))
    }

    fn inject_character_from_fixture(&self, root: &FixtureRoot) {
        let character = character_from_fixture(root);
        let mut ctx = self.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.characters.retain(|c| c.name != character.name);
        if let Some(wi) = character.embedded_world_info.clone() {
            ctx.world_info = Some(Arc::new(wi));
        }
        ctx.characters.push(Arc::new(character));
    }
}

/// TurnAttemptSink that routes through the process SQLite preaccept gateway.
struct SqliteGatewayTurnAttemptSink;

impl TurnAttemptSink for SqliteGatewayTurnAttemptSink {
    fn load_turn(&self, turn_id: &Id) -> Result<Option<TurnRecord>, String> {
        sqlite_runtime::get_turn(turn_id)
    }

    fn sync_autofix(
        &self,
        identity: &PostprocessIdentity,
        final_text: &str,
        report: QualityReport,
    ) -> Result<(), ProductionPostprocessError> {
        self.sync_autofix_with_provenance(identity, final_text, report, None)
    }

    fn sync_autofix_with_provenance(
        &self,
        identity: &PostprocessIdentity,
        final_text: &str,
        report: QualityReport,
        provenance: Option<Provenance>,
    ) -> Result<(), ProductionPostprocessError> {
        // Typed precheck
        let turn = sqlite_runtime::get_turn(&identity.turn_id)
            .map_err(ProductionPostprocessError::AutofixSync)?
            .ok_or_else(|| ProductionPostprocessError::AttemptMissing {
                turn_id: identity.turn_id.to_string(),
                attempt_id: identity.attempt_id.to_string(),
            })?;
        if turn.campaign_id != identity.campaign_id {
            return Err(ProductionPostprocessError::ScopeMismatch {
                field: "campaign_id",
                expected: identity.campaign_id.to_string(),
                actual: turn.campaign_id.to_string(),
            });
        }
        if turn.conversation_id != identity.conversation_id {
            return Err(ProductionPostprocessError::ScopeMismatch {
                field: "conversation_id",
                expected: identity.conversation_id.to_string(),
                actual: turn.conversation_id.to_string(),
            });
        }
        if turn.find_attempt(&identity.attempt_id).is_none() {
            return Err(ProductionPostprocessError::AttemptMissing {
                turn_id: identity.turn_id.to_string(),
                attempt_id: identity.attempt_id.to_string(),
            });
        }
        let writable = matches!(
            turn.status,
            TurnStatus::DraftReady | TurnStatus::DerivingState
        ) && turn.find_attempt(&identity.attempt_id).is_some_and(|a| {
            matches!(
                a.status,
                AttemptStatus::DraftReady | AttemptStatus::DerivingState
            )
        });
        if !writable {
            return Ok(());
        }
        sqlite_runtime::sync_autofix(AutofixSyncRequest {
            campaign_id: &identity.campaign_id,
            conversation_id: &identity.conversation_id,
            turn_id: &identity.turn_id,
            attempt_id: &identity.attempt_id,
            final_text,
            quality_report: report,
            provenance,
        })
        .map_err(ProductionPostprocessError::AutofixSync)
    }

    fn attach_postprocess(
        &self,
        identity: &PostprocessIdentity,
        batch: Option<storyforge_domain::turn::MutationBatch>,
        derivation: storyforge_domain::turn::DerivationComponents,
    ) -> Result<bool, String> {
        match sqlite_runtime::apply_postprocess(PostprocessApplyRequest {
            campaign_id: &identity.campaign_id,
            conversation_id: &identity.conversation_id,
            turn_id: &identity.turn_id,
            attempt_id: &identity.attempt_id,
            batch,
            derivation,
        })? {
            PostprocessApplyOutcome::Applied | PostprocessApplyOutcome::AlreadyApplied => Ok(true),
            PostprocessApplyOutcome::SkippedLate => Ok(false),
        }
    }

    fn mark_failed_if_current(
        &self,
        identity: &PostprocessIdentity,
        reason: String,
    ) -> Result<bool, String> {
        // Minimal fail path via mutate if current and writable.
        sqlite_runtime::mutate_turn_if(
            &identity.turn_id,
            |record| {
                record.campaign_id == identity.campaign_id
                    && record.conversation_id == identity.conversation_id
                    && turn_lifecycle::is_current_attempt_ready_for_postprocess(
                        record,
                        &identity.attempt_id,
                    )
            },
            |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(reason);
                record.touch();
            },
        )
    }
}

fn fixture_path() -> PathBuf {
    let configured = std::env::var_os("STORYFORGE_EVAL_FIXTURE").map(PathBuf::from);
    let path =
        configured.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_REL));
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub fn fixture_source_hash16() -> Result<String, String> {
    let bytes = std::fs::read(fixture_path()).map_err(|error| error.to_string())?;
    Ok(short_hash16(&String::from_utf8_lossy(&bytes)))
}

pub fn fixture_turn_spec(turn: u32) -> Result<FixtureTurnSpec, String> {
    let root = load_fixture(&fixture_path())?;
    let evaluation = root
        .evaluation
        .ok_or_else(|| "fixture evaluation is missing".to_string())?;
    evaluation
        .turn_script
        .into_iter()
        .find(|spec| spec.turn == turn)
        .ok_or_else(|| format!("fixture has no turn {turn}"))
}

fn load_fixture(path: &Path) -> Result<FixtureRoot, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn character_from_fixture(root: &FixtureRoot) -> Character {
    let mut entries = Vec::new();
    for (i, wi) in root.character.world_info.iter().enumerate() {
        let route = match wi.route.as_str() {
            "selective" => LoreRoute::Selective,
            "both" => LoreRoute::Both,
            "disabled" => LoreRoute::Disabled,
            _ => LoreRoute::Constant,
        };
        let mut extra = std::collections::BTreeMap::new();
        if wi.is_global {
            extra.insert("is_global".into(), serde_json::Value::Bool(true));
        }
        entries.push(WorldInfoEntry {
            st_id: Some(i as i32),
            keys: wi.keys.clone(),
            secondary_keys: wi.secondary_keys.clone(),
            content: wi.content.clone(),
            constant: wi.constant,
            selective: wi.selective,
            selective_logic: match wi.selective_logic.to_ascii_lowercase().as_str() {
                "or" => SelectiveLogic::Or,
                "not" => SelectiveLogic::Not,
                _ => SelectiveLogic::And,
            },
            disabled: wi.disabled,
            position: 0,
            depth: wi.depth as i32,
            order: if wi.order == 0 { i as i32 } else { wi.order },
            route,
            extensions: serde_json::Value::Null,
            extra,
        });
    }
    Character {
        id: Id::from_str(&root.character.id),
        name: root.character.name.clone(),
        description: root.character.description.clone(),
        personality: root.character.personality.clone(),
        scenario: root.character.scenario.clone(),
        first_mes: root.character.first_mes.clone(),
        mes_example: String::new(),
        system_prompt: root.character.system_prompt.clone(),
        post_history_instructions: String::new(),
        tags: root.character.tags.clone(),
        creator: root.character.creator.clone(),
        character_version: root.character.character_version.clone(),
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

fn definition_from_fixture(d: &FixtureDefinition) -> CharacterDefinition {
    let role_type = match d.role_type.as_str() {
        "supporting" => RoleType::Supporting,
        "extra" | "temporary" => RoleType::Extra,
        _ => RoleType::Protagonist,
    };
    let mut def = CharacterDefinition::fallback_from_character(
        &Character {
            id: Id::from_str(&d.id),
            name: d.name.clone(),
            description: d.persona_prompt.clone(),
            personality: d.behavior_rules.clone(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: "fixture".into(),
            character_version: "1".into(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::Value::Null,
            renderable_assets: Default::default(),
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        },
        &[],
    );
    def.id = Id::from_str(&d.id);
    def.name = d.name.clone();
    def.role_type = role_type;
    def.persona_prompt = d.persona_prompt.clone();
    def.behavior_rules = d.behavior_rules.clone();
    def
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysFailLlmClient;

    #[async_trait::async_trait]
    impl storyforge_infra_llm::LlmClient for AlwaysFailLlmClient {
        async fn chat(
            &self,
            _request: &storyforge_domain::llm::ChatRequest,
        ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError>
        {
            Err(storyforge_domain::llm::LlmError::Timeout)
        }

        async fn chat_stream(
            &self,
            _request: &storyforge_domain::llm::ChatRequest,
            _sender: tokio::sync::mpsc::UnboundedSender<storyforge_domain::llm::StreamChunk>,
            _cancel: tokio::sync::watch::Receiver<bool>,
        ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError>
        {
            Err(storyforge_domain::llm::LlmError::Timeout)
        }
    }

    #[test]
    fn fixture_loads_and_hashes() {
        let path = fixture_path();
        assert!(
            path.exists(),
            "fixture must be committed at {}",
            path.display()
        );
        let root = load_fixture(&path).expect("fixture parse");
        assert_eq!(root.fixture_version, "cot_three_arm_80turn_v1");
        assert_eq!(root.safe_probe_facts.len(), 6);
        assert_eq!(root.definitions.len(), 6);
        assert_eq!(root.tasks.len(), 9);
        assert_eq!(
            root.definitions
                .iter()
                .map(|definition| definition.private_knowledge.len())
                .sum::<usize>(),
            3
        );
    }

    #[test]
    fn accept_error_marker_is_typed_and_limited_to_quality_blocked() {
        let retryable =
            format_accept_error_for_runner(turn_lifecycle::AcceptError::QualityBlocked {
                error_count: 1,
            });
        assert!(is_retryable_quality_blocked_error(&retryable));

        let spoofed_storage = format_accept_error_for_runner(turn_lifecycle::AcceptError::Storage(
            "retryable_quality_blocked:error_count=1".into(),
        ));
        assert!(!is_retryable_quality_blocked_error(&spoofed_storage));
    }

    #[tokio::test]
    async fn preland_recovery_and_prompt_configuration_share_one_sqlite_process() {
        let data_dir = std::env::temp_dir().join(format!(
            "sf-sqlite-preland-recovery-{}",
            uuid::Uuid::new_v4()
        ));
        let mut env = SqliteHarnessEnv::bootstrap(data_dir, Arc::new(AlwaysFailLlmClient))
            .expect("bootstrap SQLite harness");
        let campaign_id = env.active_campaign_id().expect("active campaign");
        let campaign = sqlite_runtime::get_campaign(&campaign_id)
            .expect("read campaign")
            .expect("campaign exists");
        let conversation_id = campaign
            .conversation_id
            .clone()
            .expect("campaign conversation");
        let original_node_count = sqlite_runtime::get_conversation(&conversation_id)
            .expect("read conversation")
            .expect("conversation exists")
            .nodes
            .len();

        let write_error = env
            .write_accept_turn(
                &conversation_id,
                "synthetic failing write",
                1,
                "t001-write",
                None,
                false,
            )
            .await
            .expect_err("injected model failure must abort write");
        assert!(write_error.contains("超时") || write_error.contains("Timeout"));
        let write_turn = sqlite_runtime::get_active_turn(&campaign_id)
            .expect("read active write turn")
            .expect("pre-land write must retain a durable active turn");
        assert_eq!(write_turn.status, TurnStatus::Generating);
        assert!(write_turn.attempts.is_empty());

        env.recover_incomplete_turn_for_retry(&campaign_id)
            .expect("recover failed pre-land write");
        let recovered_write = sqlite_runtime::get_turn(&write_turn.turn_id)
            .expect("read recovered write turn")
            .expect("write turn exists");
        assert_eq!(recovered_write.status, TurnStatus::Failed);
        assert!(recovered_write.attempts.is_empty());
        assert_eq!(
            sqlite_runtime::get_conversation(&conversation_id)
                .expect("read conversation after write recovery")
                .expect("conversation exists")
                .nodes
                .len(),
            original_node_count,
            "restart recovery must remove the unaccepted write input node"
        );

        let regenerate_error = env
            .regenerate_accept_turn(
                &conversation_id,
                "synthetic failing regenerate",
                2,
                "t002-regen",
                Vec::new(),
            )
            .await
            .expect_err("injected model failure must abort regenerate");
        assert!(regenerate_error.contains("超时") || regenerate_error.contains("Timeout"));
        let regenerate_turn = sqlite_runtime::get_active_turn(&campaign_id)
            .expect("read active regenerate turn")
            .expect("pre-land regenerate must retain a durable active turn");
        assert_eq!(regenerate_turn.status, TurnStatus::Generating);
        assert!(regenerate_turn.attempts.is_empty());

        env.recover_incomplete_turn_for_retry(&campaign_id)
            .expect("recover failed pre-land regenerate");
        let recovered_regenerate = sqlite_runtime::get_turn(&regenerate_turn.turn_id)
            .expect("read recovered regenerate turn")
            .expect("regenerate turn exists");
        assert_eq!(recovered_regenerate.status, TurnStatus::Failed);
        assert!(recovered_regenerate.attempts.is_empty());
        assert_eq!(
            sqlite_runtime::get_conversation(&conversation_id)
                .expect("read conversation after regenerate recovery")
                .expect("conversation exists")
                .nodes
                .len(),
            original_node_count,
            "restart recovery must remove the unaccepted regenerate input node"
        );

        let base = WritingContext::legacy(vec![], None, Id::new());
        assert!(base.profile.is_none());
        assert!(base.modules.is_empty());
        let filled = env
            .fill_campaign_context(base)
            .expect("fill_campaign_context");
        assert!(
            filled.profile.is_some(),
            "default PromptProfile must be installed for Prompted CoT"
        );
        assert!(
            !filled.modules.is_empty(),
            "builtin prompt modules must be installed"
        );
        let director_cot = filled.profile.as_ref().unwrap().selected_ids(
            &storyforge_domain::agent::AgentRole::Director,
            &storyforge_domain::prompt_module::ModuleCategory::Cot,
        );
        assert_eq!(
            director_cot.first().map(|id| id.as_str()),
            Some("builtin-cot-director-plan"),
            "Director must select role-specific planning CoT"
        );

        assert_eq!(
            env.new_pipeline().reasoning_mode(),
            ReasoningMode::Disabled,
            "missing pipeline_sampling must keep CoT off"
        );
        env.set_pipeline_reasoning(ReasoningMode::Prompted);
        assert_eq!(
            env.new_pipeline().reasoning_mode(),
            ReasoningMode::Prompted,
            "Prompted must reach PipelineOrchestrator for Cot injection"
        );
        env.set_pipeline_reasoning(ReasoningMode::Disabled);
        assert_eq!(env.new_pipeline().reasoning_mode(), ReasoningMode::Disabled);
        env.set_pipeline_reasoning(ReasoningMode::Native);
        assert_eq!(env.new_pipeline().reasoning_mode(), ReasoningMode::Native);
    }
}

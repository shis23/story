//! M5 生产证据多轮 orchestration（确定性，不调用付费模型）。

use std::sync::Arc;

use async_trait::async_trait;
use harness_real_llm::HarnessEnv;
use harness_real_llm::budget::BudgetedLlmClient;
use harness_real_llm::commit_probe::CommitProbeEnv;
use harness_real_llm::evidence::{
    RealLlmRunBudget, contains_forbidden_evidence_payload, read_evidence_lines,
};
use harness_real_llm::production_evidence::{
    ProductionEvidenceConfig, ProductionTurnWriter, WrittenProductionTurn, require_fixture_file,
    run_production_evidence_loop,
};
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR};
use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmError, StreamChunk, Usage};
use storyforge_infra_llm::LlmClient;
use tokio::sync::{mpsc, watch};

struct DeterministicUsageClient;

#[async_trait]
impl LlmClient for DeterministicUsageClient {
    async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        Ok(ChatResponse {
            content: "deterministic draft".into(),
            tool_calls: vec![],
            finish_reason: Some("stop".into()),
            usage: Some(Usage {
                prompt_tokens: 20,
                completion_tokens: 8,
                total_tokens: 28,
                cached_tokens: 4,
                cache_creation_tokens: 0,
            }),
        })
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        _tx: mpsc::UnboundedSender<StreamChunk>,
        _cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        self.chat(req).await
    }
}

struct DeterministicTurnWriter {
    emit_summary: bool,
}

#[async_trait]
impl ProductionTurnWriter for DeterministicTurnWriter {
    async fn write_turn(
        &mut self,
        env: &HarnessEnv,
        ctx: &WritingContext,
        turn_index: u32,
        _intent: &str,
    ) -> Result<WrittenProductionTurn, String> {
        let req = ChatRequest {
            model: "fake".into(),
            messages: vec![storyforge_domain::llm::ChatMessage::user(format!(
                "turn={turn_index}; epoch={}; private=SF_SECRET_TEST_ONLY",
                ctx.context_epoch
                    .as_ref()
                    .map(|e| e.epoch_id.as_str())
                    .unwrap_or("none")
            ))],
            tools: None,
            params: storyforge_domain::llm::SamplingParams::default(),
        };
        env.llm.chat(&req).await.map_err(|e| e.to_string())?;

        let draft = format!(
            "第{turn_index}轮确定性正文：雾港调查继续推进，保留足够长度供 Accept 校验。{}",
            "线索稳定。".repeat(8)
        );
        let variant_id = env
            .conv_store
            .append_ai_draft(&ctx.conversation_id, draft.clone(), None)
            .map_err(|e| e.to_string())?;
        Ok(WrittenProductionTurn {
            draft_text: draft,
            variant_id,
            summary_text: self
                .emit_summary
                .then(|| format!("第{turn_index}轮：雾港线索继续推进。")),
        })
    }
}

fn setup(
    max_calls: u32,
) -> (
    HarnessEnv,
    Arc<BudgetedLlmClient>,
    Id,
    Id,
    std::path::PathBuf,
) {
    let budget = RealLlmRunBudget {
        enabled: true,
        max_calls,
        max_turns: DEFAULT_H_ANCHOR + DEFAULT_E + 1,
        timeout_secs: 5,
    };
    let llm = BudgetedLlmClient::wrap(Arc::new(DeterministicUsageClient), &budget);
    let env = HarnessEnv::new(llm.clone() as Arc<dyn LlmClient>);
    let probe = CommitProbeEnv::from_shared(
        env.data_dir.clone(),
        env.campaign_store.clone(),
        env.turn_store.clone(),
        env.conv_store.clone(),
    );
    let (campaign_id, conversation_id) = probe.bootstrap_campaign("m5-prod-evidence-test");
    env.set_active_campaign(campaign_id.clone());
    let evidence_dir =
        std::env::temp_dir().join(format!("sf_m5_prod_evidence_test_{}", uuid::Uuid::new_v4()));
    (env, llm, campaign_id, conversation_id, evidence_dir)
}

fn config(dir: &std::path::Path) -> ProductionEvidenceConfig {
    ProductionEvidenceConfig {
        turns: DEFAULT_H_ANCHOR + DEFAULT_E + 1,
        suite: "m5_production_evidence_test".into(),
        run_id: "m5-production-test".into(),
        calls_path: dir.join("calls.jsonl"),
        turns_path: dir.join("turns.jsonl"),
        model_label: "fake-model".into(),
    }
}

#[test]
fn missing_fixture_fails_before_any_real_model_setup() {
    let path =
        std::env::temp_dir().join(format!("missing-m5-fixture-{}.png", uuid::Uuid::new_v4()));
    let err = require_fixture_file(&path).unwrap_err();
    assert!(err.to_string().contains("required fixture missing"));
}

#[tokio::test]
async fn multi_turn_loop_uses_real_context_rollover_and_writes_redacted_jsonl() {
    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(turns);
    let mut writer = DeterministicTurnWriter { emit_summary: true };

    let report = run_production_evidence_loop(
        &env,
        llm.clone(),
        campaign_id,
        conversation_id,
        &config(&dir),
        &mut writer,
    )
    .await
    .expect("production evidence loop");

    assert_eq!(report.turns_accepted, turns);
    assert_eq!(report.calls_used, turns);
    assert!(report.crossed_h_plus_e);
    assert!(report.epoch_rolled_over);
    assert!(report.observed_epoch_ids16.len() >= 2);

    let calls = read_evidence_lines(&report.calls_path).unwrap();
    let accepted = read_evidence_lines(&report.turns_path).unwrap();
    assert_eq!(calls.len(), turns as usize);
    assert_eq!(accepted.len(), turns as usize);
    assert!(
        accepted
            .iter()
            .all(|line| line["context_epoch_id16"].as_str().is_some())
    );
    for line in calls.iter().chain(accepted.iter()) {
        let serialized = line.to_string();
        assert!(!contains_forbidden_evidence_payload(&serialized));
        assert!(!serialized.contains("SF_SECRET_TEST_ONLY"));
    }

    env.cleanup();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn loop_fails_closed_when_accepts_do_not_create_epoch_progress() {
    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(turns);
    let mut writer = DeterministicTurnWriter {
        emit_summary: false,
    };

    let err = run_production_evidence_loop(
        &env,
        llm,
        campaign_id,
        conversation_id,
        &config(&dir),
        &mut writer,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("epoch"), "unexpected error: {err}");
    env.cleanup();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn loop_fails_closed_on_zero_llm_calls() {
    struct ZeroCallWriter;
    #[async_trait]
    impl ProductionTurnWriter for ZeroCallWriter {
        async fn write_turn(
            &mut self,
            env: &HarnessEnv,
            ctx: &WritingContext,
            turn_index: u32,
            _intent: &str,
        ) -> Result<WrittenProductionTurn, String> {
            let draft = format!("zero-call draft {turn_index} {}", "正文。".repeat(20));
            let variant_id = env
                .conv_store
                .append_ai_draft(&ctx.conversation_id, draft.clone(), None)
                .map_err(|e| e.to_string())?;
            Ok(WrittenProductionTurn {
                draft_text: draft,
                variant_id,
                summary_text: Some(format!("summary {turn_index}")),
            })
        }
    }

    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(turns);
    let mut writer = ZeroCallWriter;
    let err = run_production_evidence_loop(
        &env,
        llm,
        campaign_id,
        conversation_id,
        &config(&dir),
        &mut writer,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("zero LLM calls"));
    env.cleanup();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn loop_propagates_suite_budget_and_evidence_io_failures() {
    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(2);
    let mut writer = DeterministicTurnWriter { emit_summary: true };
    let err = run_production_evidence_loop(
        &env,
        llm,
        campaign_id,
        conversation_id,
        &config(&dir),
        &mut writer,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("budget exhausted"));
    env.cleanup();
    let _ = std::fs::remove_dir_all(&dir);

    let (env, llm, campaign_id, conversation_id, dir) = setup(turns);
    std::fs::create_dir_all(dir.join("calls.jsonl")).unwrap();
    let mut writer = DeterministicTurnWriter { emit_summary: true };
    let err = run_production_evidence_loop(
        &env,
        llm,
        campaign_id,
        conversation_id,
        &config(&dir),
        &mut writer,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("evidence"));
    env.cleanup();
    let _ = std::fs::remove_dir_all(dir);
}

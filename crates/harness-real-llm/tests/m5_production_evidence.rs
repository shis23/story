//! M5 生产证据多轮 orchestration（确定性，不调用付费模型）。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use harness_real_llm::HarnessEnv;
use harness_real_llm::budget::BudgetedLlmClient;
use harness_real_llm::commit_probe::CommitProbeEnv;
use harness_real_llm::evidence::{
    RealLlmRunBudget, contains_forbidden_evidence_payload, read_evidence_lines,
};
use harness_real_llm::production_evidence::{
    ChronicleCandidateSource, FixedProductionPostprocessWriter, ProductionEvidenceConfig,
    ProductionEvidenceStage, ProductionEvidenceStageHook, ProductionPostprocessProof,
    ProductionTurnWriter, WrittenProductionTurn, require_explicit_fixture_path,
    require_fixture_file, run_production_evidence_loop, run_production_evidence_loop_with_hook,
    verify_production_postprocess_claim,
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
    fn write_path(&self) -> &'static str {
        "deterministic_writer_fixture"
    }

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
            chronicle_source: self
                .emit_summary
                .then_some(ChronicleCandidateSource::SyntheticChronicleFixture),
            postprocess_proof: None,
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
        max_tokens: None,
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
        hard_deadline: None,
    }
}

async fn issue_setup_call(env: &HarnessEnv, llm: &BudgetedLlmClient) {
    llm.set_tag("setup_extract");
    llm.set_role("setup");
    let req = ChatRequest {
        model: "fake".into(),
        messages: vec![storyforge_domain::llm::ChatMessage::user(
            "setup extraction call",
        )],
        tools: None,
        params: storyforge_domain::llm::SamplingParams::default(),
    };
    env.llm.chat(&req).await.expect("setup call");
}

#[test]
fn missing_fixture_fails_before_any_real_model_setup() {
    let path =
        std::env::temp_dir().join(format!("missing-m5-fixture-{}.png", uuid::Uuid::new_v4()));
    let err = require_fixture_file(&path).unwrap_err();
    assert!(err.to_string().contains("required fixture missing"));
}

#[test]
fn real_eval_requires_explicit_fixture_override() {
    let err = require_explicit_fixture_path(None).unwrap_err();
    assert!(err.to_string().contains("STORYFORGE_EVAL_FIXTURE_CARD"));

    let dir = std::env::temp_dir().join(format!("sf-m5-explicit-fixture-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.png");
    std::fs::write(&path, b"fixture-marker").unwrap();
    assert_eq!(
        require_explicit_fixture_path(Some(path.clone())).unwrap(),
        path
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn forged_production_postprocess_claim_without_proof_is_rejected() {
    let (env, _llm, campaign_id, conversation_id, _dir) = setup(2);
    let forged = WrittenProductionTurn {
        draft_text: "forged".into(),
        variant_id: Id::new(),
        summary_text: Some("forged summary".into()),
        chronicle_source: Some(ChronicleCandidateSource::ProductionPostprocessService),
        postprocess_proof: None,
    };
    let err = verify_production_postprocess_claim(&env, &campaign_id, &conversation_id, &forged)
        .expect_err("forged claim must fail closed");
    assert!(
        err.to_string().contains("without verified proof"),
        "unexpected: {err}"
    );

    // postprocess_applied-style inconsistency: service source + proof.applied=false
    let variant_id = Id::new();
    let not_applied = WrittenProductionTurn {
        draft_text: "x".into(),
        variant_id: variant_id.clone(),
        summary_text: Some("s".into()),
        chronicle_source: Some(ChronicleCandidateSource::ProductionPostprocessService),
        postprocess_proof: Some(ProductionPostprocessProof {
            turn_id: Id::new(),
            attempt_id: Id::new(),
            input_node_id: Id::new(),
            turn_index: 1,
            variant_id: variant_id.clone(),
            draft_hash: "deadbeef".into(),
            summary_text: Some("s".into()),
            batch_digest: None,
            applied: false,
        }),
    };
    let err =
        verify_production_postprocess_claim(&env, &campaign_id, &conversation_id, &not_applied)
            .expect_err("applied=false must fail");
    assert!(err.to_string().contains("not applied"), "unexpected: {err}");

    // synthetic + proof is inconsistent
    let mixed = WrittenProductionTurn {
        draft_text: "x".into(),
        variant_id: variant_id.clone(),
        summary_text: Some("s".into()),
        chronicle_source: Some(ChronicleCandidateSource::SyntheticChronicleFixture),
        postprocess_proof: Some(ProductionPostprocessProof {
            turn_id: Id::new(),
            attempt_id: Id::new(),
            input_node_id: Id::new(),
            turn_index: 1,
            variant_id,
            draft_hash: "deadbeef".into(),
            summary_text: Some("s".into()),
            batch_digest: None,
            applied: true,
        }),
    };
    let err = verify_production_postprocess_claim(&env, &campaign_id, &conversation_id, &mixed)
        .expect_err("synthetic+proof must fail");
    assert!(
        err.to_string().contains("synthetic_chronicle_fixture"),
        "unexpected: {err}"
    );
    env.cleanup();
}

#[tokio::test]
async fn multi_turn_loop_uses_shared_production_postprocess_service() {
    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(turns + 1);
    issue_setup_call(&env, &llm).await;
    let writer = FixedProductionPostprocessWriter {
        summary_template: "第{turn}轮：生产后处理摘要。".into(),
    };

    // FixedProductionPostprocessWriter itself does not call LLM; issue one call per turn
    // so the evidence loop's zero-call guard stays green while postprocess is production-shared.
    struct CountingWriter {
        inner: FixedProductionPostprocessWriter,
    }
    #[async_trait]
    impl ProductionTurnWriter for CountingWriter {
        fn write_path(&self) -> &'static str {
            "production_postprocess_service"
        }
        async fn write_turn(
            &mut self,
            env: &HarnessEnv,
            ctx: &WritingContext,
            turn_index: u32,
            intent: &str,
        ) -> Result<WrittenProductionTurn, String> {
            let req = ChatRequest {
                model: "fake".into(),
                messages: vec![storyforge_domain::llm::ChatMessage::user(format!(
                    "turn={turn_index}"
                ))],
                tools: None,
                params: storyforge_domain::llm::SamplingParams::default(),
            };
            env.llm.chat(&req).await.map_err(|e| e.to_string())?;
            self.inner.write_turn(env, ctx, turn_index, intent).await
        }
    }

    let mut writer = CountingWriter { inner: writer };
    let report = run_production_evidence_loop(
        &env,
        llm.clone(),
        campaign_id,
        conversation_id,
        &config(&dir),
        &mut writer,
    )
    .await
    .expect("production postprocess evidence loop");

    assert_eq!(report.turns_accepted, turns);
    assert!(report.production_postprocess_complete);
    assert_eq!(report.chronicle_path, "production_postprocess_service");
    let accepted = read_evidence_lines(&report.turns_path).unwrap();
    assert!(accepted.iter().all(|line| {
        line["chronicle_path"] == "production_postprocess_service"
            && line["production_postprocess_complete"] == true
            && line["kind"] == "pipeline_write_production_postprocess_accept"
    }));

    env.cleanup();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn multi_turn_loop_uses_real_context_rollover_and_writes_redacted_jsonl() {
    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(turns + 1);
    issue_setup_call(&env, &llm).await;
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
    assert!(!report.production_postprocess_complete);

    let calls = read_evidence_lines(&report.calls_path).unwrap();
    let accepted = read_evidence_lines(&report.turns_path).unwrap();
    assert_eq!(calls.len(), turns as usize);
    assert_eq!(calls[0]["tag"], "turn1");
    assert!(calls.iter().all(|line| line["tag"] != "setup_extract"));
    assert_eq!(accepted.len(), turns as usize);
    assert!(accepted.iter().all(|line| {
        line["write_path"] == "deterministic_writer_fixture"
            && line["chronicle_path"] == "synthetic_chronicle_fixture"
            && line["accept_path"] == "production_faithful_commit_probe"
            && line["production_postprocess_complete"] == false
            && line["kind"] == "pipeline_write_synthetic_chronicle_accept"
    }));
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
                chronicle_source: Some(ChronicleCandidateSource::SyntheticChronicleFixture),
                postprocess_proof: None,
            })
        }
    }

    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(turns + 1);
    issue_setup_call(&env, &llm).await;
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
    assert!(
        err.to_string().contains("turn 1 produced zero LLM calls"),
        "setup samples must not let turn1 pass: {err}"
    );
    env.cleanup();
    let _ = std::fs::remove_dir_all(dir);
}

#[derive(Clone)]
struct DelayAtStage {
    stage: ProductionEvidenceStage,
    delay: Duration,
}

#[async_trait]
impl ProductionEvidenceStageHook for DelayAtStage {
    async fn on_stage(&self, stage: ProductionEvidenceStage, _turn_index: u32) {
        if stage == self.stage {
            tokio::time::sleep(self.delay).await;
        }
    }
}

#[tokio::test]
async fn hard_deadline_flushes_completed_samples_and_covers_accept_evidence_boundaries() {
    for stage in [
        ProductionEvidenceStage::AfterWriteBeforeCallEvidence,
        ProductionEvidenceStage::AfterCallEvidence,
        ProductionEvidenceStage::AfterAccept,
    ] {
        let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
        let (env, llm, campaign_id, conversation_id, dir) = setup(turns);
        let mut cfg = config(&dir);
        cfg.hard_deadline = Some(Duration::from_millis(100));
        let hook = DelayAtStage {
            stage,
            delay: Duration::from_millis(250),
        };
        let mut writer = DeterministicTurnWriter { emit_summary: true };
        let err = run_production_evidence_loop_with_hook(
            &env,
            llm,
            campaign_id,
            conversation_id,
            &cfg,
            &mut writer,
            &hook,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("suite timeout"),
            "stage={stage:?}: {err}"
        );
        let calls = read_evidence_lines(&cfg.calls_path).unwrap();
        assert_eq!(
            calls.len(),
            1,
            "stage={stage:?} must preserve completed usage"
        );
        assert_eq!(calls[0]["turn_index"], 1);
        env.cleanup();
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tokio::test]
async fn hard_deadline_covers_final_context_fill_boundary() {
    let turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    let (env, llm, campaign_id, conversation_id, dir) = setup(turns);
    let mut cfg = config(&dir);
    cfg.hard_deadline = Some(Duration::from_secs(1));
    let hook = DelayAtStage {
        stage: ProductionEvidenceStage::BeforeFinalFill,
        delay: Duration::from_secs(2),
    };
    let mut writer = DeterministicTurnWriter { emit_summary: true };
    let err = run_production_evidence_loop_with_hook(
        &env,
        llm,
        campaign_id,
        conversation_id,
        &cfg,
        &mut writer,
        &hook,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("suite timeout"));
    assert_eq!(
        read_evidence_lines(&cfg.calls_path).unwrap().len(),
        turns as usize
    );
    assert_eq!(
        read_evidence_lines(&cfg.turns_path).unwrap().len(),
        turns as usize
    );
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

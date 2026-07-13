//! M5 生产证据真实模型入口（默认 ignored，需显式付费授权）。
//!
//! 该 suite 运行生产 Context 装配与多轮 `write → CommitTurn/Accept`，目标轮数必须
//! 严格跨越 `H_anchor + E`。模型调用全程共享一个 `BudgetedLlmClient`，并分别写
//! calls/turns 脱敏 JSONL。fixture 缺失、零调用、预算耗尽、证据写失败、未跨 epoch
//! 都会失败，不会输出假 PASS。
//!
//! ```text
//! $env:STORYFORGE_EVAL_REAL_LLM='1'
//! $env:LLM_BASE_URL='...'
//! $env:LLM_API_KEY='...'
//! $env:LLM_MODEL='...'
//! $env:STORYFORGE_EVAL_MAX_CALLS='96'
//! $env:STORYFORGE_EVAL_MAX_TURNS='16'
//! $env:STORYFORGE_EVAL_TIMEOUT_SECS='180'
//! cargo test -p harness-real-llm --test eval_m5_phaseb_real_llm -- --ignored --nocapture
//! ```

use std::sync::Arc;

use harness_real_llm::budget::BudgetedLlmClient;
use harness_real_llm::evidence::RealLlmRunBudget;
use harness_real_llm::production_evidence::{
    PipelineProductionTurnWriter, ProductionEvidenceConfig, require_fixture_file,
    run_production_evidence_loop,
};
use harness_real_llm::{HarnessEnv, require_real_llm};
use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR};
use storyforge_infra_llm::LlmClient;

fn require_eval_budget() -> RealLlmRunBudget {
    let budget = RealLlmRunBudget::from_env();
    if !budget.enabled {
        panic!(
            "STORYFORGE_EVAL_REAL_LLM not enabled; refusing real model calls. \
             Set STORYFORGE_EVAL_REAL_LLM=1 and LLM_BASE_URL/API_KEY/MODEL."
        );
    }
    let minimum_turns = DEFAULT_H_ANCHOR + DEFAULT_E + 1;
    if budget.max_turns < minimum_turns {
        panic!(
            "STORYFORGE_EVAL_MAX_TURNS must be >= {minimum_turns} to cross H_anchor+E; got {}",
            budget.max_turns
        );
    }
    if budget.max_calls == 0 {
        panic!("STORYFORGE_EVAL_MAX_CALLS must be >= 1 for real eval");
    }
    budget
}

fn evidence_dir() -> std::path::PathBuf {
    std::env::var("STORYFORGE_EVAL_EVIDENCE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join(format!(
                "storyforge_m5_production_evidence_{}",
                uuid::Uuid::new_v4()
            ))
        })
}

#[tokio::test]
#[ignore = "需要 STORYFORGE_EVAL_REAL_LLM=1 与 LLM 凭证；默认不跑"]
async fn eval_real_llm_production_write_commit_accept_across_epoch() {
    let budget = require_eval_budget();

    // fixture 必须在构造真实 client 之前验证，缺失时确保零付费调用并 fail closed。
    let card_path = find_fixture("test-card-seraphina.png");
    require_fixture_file(&card_path).unwrap_or_else(|e| panic!("INCONCLUSIVE (not pass): {e}"));

    let llm = BudgetedLlmClient::wrap(require_real_llm(), &budget);
    let env = HarnessEnv::new(llm.clone() as Arc<dyn LlmClient>);
    let bytes = std::fs::read(&card_path).expect("read fixture");
    let character = storyforge_infra_import::import_character(&bytes).expect("import fixture");
    let source_id = character.id.clone();
    env.inject_character(character);
    let card = env.extract_characters(source_id.as_str()).await;
    let campaign_id = env.create_campaign(&card, "m5-production-evidence-real");
    let conversation_id = env.conv_store.create(None, Some(campaign_id.clone())).id;
    let mut campaign = env
        .campaign_store
        .get_campaign(&campaign_id)
        .expect("campaign after create");
    campaign.conversation_id = Some(conversation_id.clone());
    env.campaign_store
        .update_campaign(campaign)
        .expect("bind campaign conversation");

    let dir = evidence_dir();
    let cfg = ProductionEvidenceConfig {
        turns: budget.max_turns,
        suite: "eval_real_m5_production_evidence".into(),
        run_id: format!("m5-real-{}", uuid::Uuid::new_v4()),
        calls_path: dir.join("calls.jsonl"),
        turns_path: dir.join("turns.jsonl"),
        model_label: std::env::var("LLM_MODEL").unwrap_or_else(|_| "unknown".into()),
    };
    let mut writer = PipelineProductionTurnWriter;
    let result = run_production_evidence_loop(
        &env,
        llm.clone(),
        campaign_id,
        conversation_id,
        &cfg,
        &mut writer,
    )
    .await;

    match result {
        Ok(report) => {
            eprintln!(
                "M5 production evidence PASS: accepts={}/{} calls={}/{} epoch_ids={} \
                 elapsed_ms={} calls_jsonl={} turns_jsonl={}",
                report.turns_accepted,
                report.turns_requested,
                report.calls_used,
                report.max_calls,
                report.observed_epoch_ids16.len(),
                report.elapsed_ms,
                report.calls_path.display(),
                report.turns_path.display()
            );
            assert!(report.crossed_h_plus_e);
            assert!(report.epoch_rolled_over);
            assert!(report.calls_used >= report.turns_accepted);
        }
        Err(err) => {
            env.cleanup();
            panic!("M5 production evidence failed closed: {err}");
        }
    }
    env.cleanup();
}

fn find_fixture(name: &str) -> std::path::PathBuf {
    if let Ok(p) = std::env::var("STORYFORGE_EVAL_FIXTURE_CARD") {
        return std::path::PathBuf::from(p);
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    loop {
        let candidate = dir.join(name);
        if candidate.exists() {
            return candidate;
        }
        let alt = dir.join("fixtures").join(name);
        if alt.exists() {
            return alt;
        }
        if !dir.pop() {
            return std::path::PathBuf::from(name);
        }
    }
}

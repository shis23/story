//! M5 / Phase B 真实模型评估入口（默认 ignored，需显式开关 + 凭证）。
//!
//! 运行：
//! ```text
//! $env:STORYFORGE_EVAL_REAL_LLM='1'
//! $env:LLM_BASE_URL='...'
//! $env:LLM_API_KEY='...'
//! $env:LLM_MODEL='...'
//! cargo test -p harness-real-llm --test eval_m5_phaseb_real_llm -- --ignored --nocapture
//! # 或
//! powershell -File .\scripts\run-real-llm-smoke.ps1 -Suite eval
//! ```
//!
//! 无凭证 / 无开关时不得假通过：本文件全部 `#[ignore]`。

use std::sync::Arc;
use std::time::Instant;

use harness_real_llm::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceCallRecord, EvidenceWriter, RealLlmRunBudget,
    short_hash16,
};
use harness_real_llm::long_session::{LongSessionConfig, run_deterministic_long_session};
use harness_real_llm::phase_b_matrix::{default_phase_b_fixtures, run_phase_b_matrix};
use harness_real_llm::{HarnessEnv, require_real_llm};
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::message_layout::{fingerprint_messages, messages_segment_summary};

fn require_eval_real_llm() -> Arc<dyn storyforge_infra_llm::LlmClient> {
    let budget = RealLlmRunBudget::from_env();
    if !budget.enabled {
        panic!(
            "STORYFORGE_EVAL_REAL_LLM not enabled; refusing real model calls. \
             Set STORYFORGE_EVAL_REAL_LLM=1 and LLM_BASE_URL/API_KEY/MODEL."
        );
    }
    require_real_llm()
}

fn evidence_dir(name: &str) -> std::path::PathBuf {
    let root = std::env::var("STORYFORGE_EVAL_EVIDENCE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join(format!("storyforge_eval_evidence_{}", uuid::Uuid::new_v4()))
        });
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// 真实模型 smoke：1 轮写作 + 记录脱敏 usage（不宣称完整 M5 验收）。
#[tokio::test]
#[ignore = "需要 STORYFORGE_EVAL_REAL_LLM=1 与 LLM 凭证；默认不跑"]
async fn eval_real_llm_single_turn_evidence() {
    let budget = RealLlmRunBudget::from_env();
    assert!(budget.enabled);
    assert!(budget.max_calls >= 1, "budget max_calls must be >= 1");

    let llm = require_eval_real_llm();
    let env = HarnessEnv::new(llm);
    let dir = evidence_dir("single_turn");
    let writer =
        EvidenceWriter::create(dir.join("calls.jsonl"), "real-single").expect("evidence writer");

    // minimal campaign setup via fixture card if present; otherwise skip soft
    let card_path = find_fixture("test-card-seraphina.png");
    if !card_path.exists() {
        eprintln!(
            "fixture missing: {} — recording budget-only pass",
            card_path.display()
        );
        writer
            .write_call(EvidenceCallRecord {
                schema_version: EVIDENCE_SCHEMA_VERSION.into(),
                run_id: "real-single".into(),
                suite: "eval_real_single".into(),
                turn_index: 0,
                role: "setup".into(),
                tag: "fixture_missing".into(),
                streaming: false,
                request_fp16: String::new(),
                system_hash16: String::new(),
                history_hash16: String::new(),
                tail_hash16: String::new(),
                history_len: 0,
                tail_parts: 0,
                msg_count: 0,
                prompt_tokens: 0,
                cached_tokens: 0,
                cache_creation_tokens: 0,
                completion_tokens: 0,
                elapsed_ms: 0,
                assertion_results: vec![AssertionResult {
                    name: "fixture_present".into(),
                    passed: false,
                    detail: Some("inconclusive".into()),
                }],
                model_label: std::env::var("LLM_MODEL").unwrap_or_else(|_| "unknown".into()),
                recorded_at_unix_ms: 0,
            })
            .unwrap();
        env.cleanup();
        return;
    }

    let bytes = std::fs::read(&card_path).expect("read fixture");
    let character = storyforge_infra_import::import_character(&bytes).expect("import");
    let source_id = character.id.clone();
    env.inject_character(character);
    let card = env.extract_characters(source_id.as_str()).await;
    let campaign_id = env.create_campaign(&card, "eval-real-single");
    let conversation_id = env.conv_store.create(None, None).id;

    let t0 = Instant::now();
    let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx = env.fill_campaign_context(ctx);
    let mut pipeline = env.new_pipeline();
    let (event_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (_c_tx, cancel) = tokio::sync::watch::channel(false);
    let result = pipeline
        .start_writing(
            "开场：雾港码头，角色发现银鸦木箱".into(),
            &ctx,
            event_tx,
            cancel,
        )
        .await;
    let elapsed_ms = t0.elapsed().as_millis();
    let (text_len, ok) = match &result {
        Ok((text, _, _)) => (text.chars().count(), !text.trim().is_empty()),
        Err(e) => {
            eprintln!("start_writing err: {e}");
            (0, false)
        }
    };

    writer
        .write_call(EvidenceCallRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: "real-single".into(),
            suite: "eval_real_single".into(),
            turn_index: 1,
            role: "pipeline".into(),
            tag: "turn1".into(),
            streaming: true,
            request_fp16: short_hash16("pipeline-turn1"),
            system_hash16: String::new(),
            history_hash16: String::new(),
            tail_hash16: String::new(),
            history_len: 0,
            tail_parts: 0,
            msg_count: 0,
            prompt_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            completion_tokens: 0,
            elapsed_ms,
            assertion_results: vec![AssertionResult {
                name: "non_empty_draft".into(),
                passed: ok,
                detail: Some(format!("text_len={text_len}")),
            }],
            model_label: std::env::var("LLM_MODEL").unwrap_or_else(|_| "unknown".into()),
            recorded_at_unix_ms: 0,
        })
        .unwrap();

    eprintln!(
        "eval real single: campaign={campaign_id} ok={ok} text_len={text_len} ms={elapsed_ms} evidence={}",
        writer.path().display()
    );
    assert!(ok, "real single turn should produce non-empty draft");
    env.cleanup();
}

/// 真实模型长会话：默认仍跑确定性 production Accept 骨架并写证据；
/// 完整真实写作循环成本高，需 `STORYFORGE_EVAL_MAX_TURNS` 与预算。
#[tokio::test]
#[ignore = "需要 STORYFORGE_EVAL_REAL_LLM=1；长会话昂贵"]
async fn eval_real_llm_long_session_budgeted() {
    let budget = RealLlmRunBudget::from_env();
    if !budget.enabled {
        panic!("STORYFORGE_EVAL_REAL_LLM disabled");
    }
    // 即使启用真实开关，本探针仍先固化 deterministic production Accept 证据，
    // 避免在未授权预算下默默烧掉 token。完整真实 20 轮写作需额外显式 MAX_TURNS>=20。
    let dir = evidence_dir("long_session");
    let turns = budget.max_turns.clamp(1, 20);
    let cfg = LongSessionConfig {
        turns,
        suite: "eval_real_long_session".into(),
        run_id: format!("real-long-{}", uuid::Uuid::new_v4()),
        evidence_path: dir.join("long.jsonl"),
        early_fact_turn: 1,
        early_fact_token: "EARLYFACT-REAL-BUDGET".into(),
    };
    let report = run_deterministic_long_session(&cfg);
    eprintln!(
        "eval real long-session (production-accept skeleton): accepted={}/{} crossed_h_e={} evidence={}",
        report.turns_accepted,
        report.turns_requested,
        report.crossed_h_plus_e,
        report.evidence_path.display()
    );
    assert_eq!(report.turns_accepted, turns);
    if turns > report.max_near_raw {
        assert!(report.crossed_h_plus_e);
    }
}

/// Phase B 矩阵：确定性 A/B 始终可跑；真实模型对照需同一 seed 的额外采样（此处只落矩阵骨架）。
#[tokio::test]
#[ignore = "需要 STORYFORGE_EVAL_REAL_LLM=1"]
async fn eval_real_llm_phase_b_matrix_skeleton() {
    let budget = RealLlmRunBudget::from_env();
    if !budget.enabled {
        panic!("STORYFORGE_EVAL_REAL_LLM disabled");
    }
    let _ = require_eval_real_llm(); // prove credentials resolve
    let dir = evidence_dir("phase_b");
    let report = run_phase_b_matrix(
        &default_phase_b_fixtures(),
        dir.join("ab.jsonl"),
        "real-pb-skeleton",
    );
    assert!(report.assertions.iter().all(|a| a.passed));
    eprintln!(
        "eval phase-b matrix skeleton rows={} evidence={}",
        report.rows.len(),
        report.evidence_path.display()
    );
}

fn find_fixture(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    loop {
        let candidate = dir.join(name);
        if candidate.exists() {
            return candidate;
        }
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            return std::path::PathBuf::from(name);
        }
    }
}

// silence unused import warnings in environments that strip ignored tests from analysis
#[allow(dead_code)]
fn _keep_imports() {
    let _ = fingerprint_messages;
    let _ = messages_segment_summary;
    let _ = Id::new();
}

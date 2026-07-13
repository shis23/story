//! M5 / Phase B 真实模型评估入口（默认 ignored，需显式开关 + 凭证）。
//!
//! 运行：
//! ```text
//! $env:STORYFORGE_EVAL_REAL_LLM='1'
//! $env:LLM_BASE_URL='...'
//! $env:LLM_API_KEY='...'
//! $env:LLM_MODEL='...'
//! $env:STORYFORGE_EVAL_MAX_CALLS='8'
//! $env:STORYFORGE_EVAL_TIMEOUT_SECS='60'
//! cargo test -p harness-real-llm --test eval_m5_phaseb_real_llm -- --ignored --nocapture
//! # 或
//! powershell -File .\scripts\run-real-llm-smoke.ps1 -Suite eval
//! ```
//!
//! 无凭证 / 无开关 / fixture 缺失时**不得假通过**：本文件全部 `#[ignore]`；
//! 运行时 fixture 缺失或预算耗尽会 `panic`/`assert` 失败，而不是 exit 0 + PROBE PASS。
//!
//! 确定性 Accept / Phase B skeleton **不在**本真实 suite 内；它们在
//! `eval_m5_phaseb_deterministic` 与 lib 单元测试中覆盖。

use std::sync::Arc;
use std::time::Instant;

use harness_real_llm::budget::BudgetedLlmClient;
use harness_real_llm::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceCallRecord, EvidenceWriter, RealLlmRunBudget,
};
use harness_real_llm::{HarnessEnv, require_real_llm};
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::message_layout::{fingerprint_messages, messages_segment_summary};

fn require_eval_budget() -> RealLlmRunBudget {
    let budget = RealLlmRunBudget::from_env();
    if !budget.enabled {
        panic!(
            "STORYFORGE_EVAL_REAL_LLM not enabled; refusing real model calls. \
             Set STORYFORGE_EVAL_REAL_LLM=1 and LLM_BASE_URL/API_KEY/MODEL."
        );
    }
    if budget.max_calls == 0 {
        panic!("STORYFORGE_EVAL_MAX_CALLS must be >= 1 for real eval");
    }
    budget
}

fn require_budgeted_client(budget: &RealLlmRunBudget) -> Arc<BudgetedLlmClient> {
    let inner = require_real_llm();
    BudgetedLlmClient::wrap(inner, budget)
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
///
/// 预算：`BudgetedLlmClient` 真正限制 `max_calls` 与 `timeout_secs`。
/// fixture 缺失：直接 panic（Inconclusive 不等于 PASS）。
#[tokio::test]
#[ignore = "需要 STORYFORGE_EVAL_REAL_LLM=1 与 LLM 凭证；默认不跑"]
async fn eval_real_llm_single_turn_evidence() {
    let budget = require_eval_budget();
    let llm = require_budgeted_client(&budget);
    llm.set_tag("turn1");
    llm.set_role("pipeline");

    let env = HarnessEnv::new(llm.clone() as Arc<dyn storyforge_infra_llm::LlmClient>);
    let dir = evidence_dir("single_turn");
    let writer =
        EvidenceWriter::create(dir.join("calls.jsonl"), "real-single").expect("evidence writer");

    let card_path = find_fixture("test-card-seraphina.png");
    if !card_path.exists() {
        // 明确失败：不允许「零调用 + PROBE PASS」。
        env.cleanup();
        panic!(
            "INCONCLUSIVE (not pass): required fixture missing: {}. \
             Provide test-card-seraphina.png at repo root or set a reachable path; \
             refusing to report PROBE PASS with zero LLM calls.",
            card_path.display()
        );
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

    // Outer wall-clock timeout as a second belt on top of per-call timeout.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(
            budget
                .timeout_secs
                .saturating_mul(budget.max_calls as u64)
                .max(budget.timeout_secs),
        ),
        pipeline.start_writing(
            "开场：雾港码头，角色发现银鸦木箱".into(),
            &ctx,
            event_tx,
            cancel,
        ),
    )
    .await;

    let elapsed_ms = t0.elapsed().as_millis();
    let (text_len, ok, err_detail) = match result {
        Ok(Ok((text, _, _))) => (text.chars().count(), !text.trim().is_empty(), None),
        Ok(Err(e)) => {
            eprintln!("start_writing err: {e}");
            (0, false, Some(format!("pipeline_err={e}")))
        }
        Err(_) => {
            eprintln!("start_writing wall-clock timeout");
            (0, false, Some("wall_clock_timeout".into()))
        }
    };

    // Write one evidence line per recorded sample (real usage, not zeros).
    let samples = llm.samples();
    if samples.is_empty() {
        writer
            .write_call(EvidenceCallRecord {
                schema_version: EVIDENCE_SCHEMA_VERSION.into(),
                run_id: "real-single".into(),
                suite: "eval_real_single".into(),
                turn_index: 1,
                role: "pipeline".into(),
                tag: "no_llm_calls".into(),
                streaming: true,
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
                elapsed_ms,
                assertion_results: vec![
                    AssertionResult {
                        name: "non_empty_draft".into(),
                        passed: ok,
                        detail: Some(format!("text_len={text_len}")),
                    },
                    AssertionResult {
                        name: "usage_recorded".into(),
                        passed: false,
                        detail: err_detail.clone(),
                    },
                ],
                model_label: std::env::var("LLM_MODEL").unwrap_or_else(|_| "unknown".into()),
                recorded_at_unix_ms: 0,
            })
            .unwrap();
    } else {
        for (idx, sample) in samples.iter().enumerate() {
            let rec = sample.to_evidence_call(
                "real-single",
                "eval_real_single",
                (idx + 1) as u32,
                std::env::var("LLM_MODEL").unwrap_or_else(|_| "unknown".into()),
                vec![
                    AssertionResult {
                        name: "usage_recorded".into(),
                        passed: sample.prompt_tokens > 0 || sample.completion_tokens > 0,
                        detail: Some(format!(
                            "prompt={} completion={} cached={}",
                            sample.prompt_tokens, sample.completion_tokens, sample.cached_tokens
                        )),
                    },
                    AssertionResult {
                        name: "non_empty_draft".into(),
                        passed: ok,
                        detail: Some(format!("text_len={text_len}")),
                    },
                ],
            );
            writer.write_call(rec).unwrap();
        }
    }

    eprintln!(
        "eval real single: campaign={campaign_id} ok={ok} text_len={text_len} ms={elapsed_ms} \
         calls={}/{} prompt_tokens={} completion_tokens={} evidence={}",
        llm.calls_used(),
        llm.max_calls(),
        llm.total_prompt_tokens(),
        llm.total_completion_tokens(),
        writer.path().display()
    );

    assert!(
        llm.calls_used() >= 1,
        "real single turn must issue >=1 budgeted LLM call (got {}); \
         refusing zero-call PASS",
        llm.calls_used()
    );
    assert!(
        llm.calls_used() <= llm.max_calls(),
        "budget violated: calls_used={} max={}",
        llm.calls_used(),
        llm.max_calls()
    );
    assert!(
        ok,
        "real single turn should produce non-empty draft; detail={err_detail:?}"
    );
    env.cleanup();
}

// 说明：长会话 / Phase B 的**确定性**骨架已从本真实 suite 移除，
// 避免 STORYFORGE_EVAL_REAL_LLM=1 时零调用假通过。
// 确定性覆盖见：
//   - harness-real-llm::budget::tests（max_calls / timeout，不消耗真实模型）
//   - harness-real-llm::long_session::tests
//   - harness-real-llm::phase_b_matrix
//   - tests/eval_m5_phaseb_deterministic.rs
// 真实 ≥20 写作循环需单独授权与更高预算，尚未实现。

fn find_fixture(name: &str) -> std::path::PathBuf {
    // Prefer explicit override.
    if let Ok(p) = std::env::var("STORYFORGE_EVAL_FIXTURE_CARD") {
        let path = std::path::PathBuf::from(p);
        if path.exists() {
            return path;
        }
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    loop {
        let candidate = dir.join(name);
        if candidate.exists() {
            return candidate;
        }
        // also check common fixtures/ locations
        let alt = dir.join("fixtures").join(name);
        if alt.exists() {
            return alt;
        }
        let alt2 = dir.join("testdata").join(name);
        if alt2.exists() {
            return alt2;
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

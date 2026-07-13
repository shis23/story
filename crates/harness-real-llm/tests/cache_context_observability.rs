//! Cache / context observability 专项门禁（确定性，无网络）。
//!
//! 覆盖 PLAN：
//! - usage parser 矩阵已在 `storyforge-infra-llm` unit 中
//! - boot/setup 与 turn 样本分离
//! - 模拟前缀 cache 稳定/失效
//! - 100+ Context 编译基准 + 预算断言
//! - 证据脱敏
//! - 生产默认 200/4/H/E 未改

use harness_real_llm::budget::UsageSample;
use harness_real_llm::context_compile_bench::{
    ContextCompileBenchConfig, run_context_compile_benchmark,
};
use harness_real_llm::observability::{
    SampleKind, SimulatedCacheEvent, SimulatedPrefixCache, classify_sample_kind, partition_samples,
};
use storyforge_domain::chronicle::{
    ContextWindowParams, DEFAULT_COMPRESS_ACTIVE_A_THRESHOLD, DEFAULT_COMPRESS_GROUP_SIZE,
    DEFAULT_E, DEFAULT_H_ANCHOR, DEFAULT_OVERVIEW_MAX_ENTRIES,
};
use storyforge_domain::llm::ChatMessage;
use storyforge_domain::message_layout::{
    estimate_reusable_prefix_tokens, longest_common_message_prefix_len,
};
use storyforge_infra_llm::usage_parse::parse_provider_usage;

#[test]
fn usage_parser_matrix_smoke_from_harness_gate() {
    let openai = serde_json::json!({
        "prompt_tokens": 200,
        "completion_tokens": 10,
        "total_tokens": 210,
        "prompt_tokens_details": {"cached_tokens": 80}
    });
    let u = parse_provider_usage(&openai).expect("openai nested");
    assert_eq!(u.cached_tokens, 80);
    assert!(u.prompt_tokens > 0);
    // 禁止 0>=0 式 vacuous cache success
    assert!(!(u.cached_tokens == 0 && u.prompt_tokens == 0));
}

#[test]
fn boot_samples_do_not_cross_attribute_to_turns() {
    let samples = vec![
        sample("boot", "pipeline", 5000, 4000),
        sample("setup-extract", "bootstrap", 800, 100),
        sample("turn1", "editor", 300, 50),
        sample("turn2", "director", 320, 60),
    ];
    let (non_turn, turns) = partition_samples(&samples);
    assert_eq!(non_turn.len(), 2);
    assert_eq!(turns.len(), 2);
    assert!(turns.iter().all(|s| s.tag.starts_with("turn")));
    assert_eq!(classify_sample_kind("boot", "pipeline"), SampleKind::Boot);
    // boot 的高 cached 不进入 turn 集合，避免假阳性 cache 成功
    let turn_cached: u32 = turns.iter().map(|s| s.cached_tokens).sum();
    assert!(turn_cached < 4000);
}

#[test]
fn simulated_prefix_cache_stable_then_invalidates() {
    let mut cache = SimulatedPrefixCache::new("e1");
    let m1 = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("h"),
        ChatMessage::user("t1"),
    ];
    assert_eq!(
        cache.observe("e1", "pfx-a", &m1).event,
        SimulatedCacheEvent::MissCold
    );
    let m2 = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("h"),
        ChatMessage::assistant("a"),
        ChatMessage::user("t2"),
    ];
    assert_eq!(
        cache.observe("e1", "pfx-a", &m2).event,
        SimulatedCacheEvent::HitStablePrefix
    );
    assert_eq!(
        cache.observe("e1", "pfx-hook", &m2).event,
        SimulatedCacheEvent::InvalidateHookChange
    );
    assert_eq!(
        cache.observe("e2", "pfx-b", &m2).event,
        SimulatedCacheEvent::InvalidateEpochRollover
    );
    assert_eq!(longest_common_message_prefix_len(&m1, &m2), 2);
    assert!(estimate_reusable_prefix_tokens(&m1, &m2) > 0);
}

#[test]
fn context_compile_benchmark_and_budget_assertions() {
    let dir = std::env::temp_dir().join(format!("sf_obs_gate_{}", uuid::Uuid::new_v4()));
    let cfg = ContextCompileBenchConfig {
        turns: 120,
        params: ContextWindowParams::default(),
        report_path: dir.join("report.json"),
        budgets: Default::default(),
    };
    let report = run_context_compile_benchmark(&cfg);
    assert!(report.turns >= 100);
    assert!(report.observed_max_near_raw <= report.max_near_raw);
    assert!(report.assertions.iter().all(|a| a.passed));
    let body = std::fs::read_to_string(&cfg.report_path).unwrap();
    assert!(!body.contains("api_key"));
    assert!(!body.contains("SF_SECRET_"));
    assert!(!body.contains("\"messages\""));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn production_defaults_unchanged_200_4_h_e() {
    assert_eq!(DEFAULT_H_ANCHOR, 5);
    assert_eq!(DEFAULT_E, 10);
    assert_eq!(DEFAULT_OVERVIEW_MAX_ENTRIES, 200);
    assert_eq!(DEFAULT_COMPRESS_ACTIVE_A_THRESHOLD, 200);
    assert_eq!(DEFAULT_COMPRESS_GROUP_SIZE, 4);
    let p = ContextWindowParams::default();
    assert_eq!(p.max_near_raw_turns(), 15);
}

fn sample(tag: &str, role: &str, prompt: u32, cached: u32) -> UsageSample {
    UsageSample {
        tag: tag.into(),
        role: role.into(),
        streaming: false,
        prompt_tokens: prompt,
        cached_tokens: cached,
        cache_creation_tokens: 0,
        completion_tokens: 1,
        request_fp16: "0".into(),
        system_hash16: "1".into(),
        history_hash16: "2".into(),
        tail_hash16: "3".into(),
        history_len: 0,
        tail_parts: 0,
        msg_count: 1,
        elapsed_ms: 1,
        outcome: "ok".into(),
    }
}

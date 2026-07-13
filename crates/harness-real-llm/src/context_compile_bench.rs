//! 确定性 100+ Accept-turn Context 编译基准（无网络）。
//!
//! 使用合成 draft / Chronicle 条目驱动 domain 纯函数：
//! `refresh_context_epoch` + `compile_history_blocks`，记录 token 估计、
//! near/band/overview 成员、延迟与脱敏报告体积。

use std::path::PathBuf;
use std::time::Instant;

use storyforge_domain::Id;
use storyforge_domain::chronicle::{
    ChronicleCode, ChronicleLevel, CommittedTurnRef, ContextCompileCapture, ContextCompileInput,
    ContextEpochSnapshot, ContextWindowParams, DEFAULT_E, DEFAULT_H_ANCHOR,
    DEFAULT_OVERVIEW_MAX_ENTRIES, OverviewCandidate, committed_turn_id, compile_history_blocks,
    refresh_context_epoch,
};
use storyforge_domain::llm::ChatMessage;
use storyforge_domain::message_layout::estimate_tokens_approx;

use crate::observability::{
    BenchmarkAssertion, BudgetAssertions, ContextCompileBenchmarkReport, percentile_ms,
    prompt_growth_slope, schema_version, write_observability_report,
};

/// 基准配置。
#[derive(Debug, Clone)]
pub struct ContextCompileBenchConfig {
    pub turns: u32,
    pub params: ContextWindowParams,
    pub report_path: PathBuf,
    pub budgets: BudgetAssertions,
}

impl Default for ContextCompileBenchConfig {
    fn default() -> Self {
        Self {
            turns: 120,
            params: ContextWindowParams::default(),
            report_path: std::env::temp_dir().join("storyforge_cache_context_compile_bench.json"),
            budgets: BudgetAssertions::default(),
        }
    }
}

/// 运行确定性编译基准并写报告。
pub fn run_context_compile_benchmark(
    cfg: &ContextCompileBenchConfig,
) -> ContextCompileBenchmarkReport {
    assert!(
        cfg.turns > cfg.params.max_near_raw_turns(),
        "benchmark turns must exceed H_anchor+E"
    );

    let mut committed: Vec<CommittedTurnRef> = Vec::with_capacity(cfg.turns as usize);
    let mut overview_candidates: Vec<OverviewCandidate> = Vec::with_capacity(cfg.turns as usize);
    let mut epoch: Option<ContextEpochSnapshot> = None;
    let mut token_estimates: Vec<u32> = Vec::with_capacity(cfg.turns as usize);
    let mut latencies: Vec<u128> = Vec::with_capacity(cfg.turns as usize);
    let mut observed_max_near = 0u32;
    let mut observed_max_anchor = 0u32;
    let mut final_near = 0usize;
    let mut final_anchor = 0usize;
    let mut final_live = 0usize;
    let mut final_band = 0usize;
    let mut final_overview = 0usize;
    let mut chronicle_revision = 0u64;
    let mut latency_us: Vec<u128> = Vec::with_capacity(cfg.turns as usize);

    for i in 1..=cfg.turns {
        let t0 = Instant::now();
        let turn_id = committed_turn_id(i);
        committed.push(CommittedTurnRef {
            turn_id: turn_id.clone(),
            sequence: i,
        });
        overview_candidates.push(OverviewCandidate {
            code: ChronicleCode::new(ChronicleLevel::A, i),
            level: ChronicleLevel::A,
            turn_start: i,
            covered_by: None,
        });
        chronicle_revision = chronicle_revision.saturating_add(1);

        let band_code_for_turn = |tid: &Id| -> Option<ChronicleCode> {
            let seq = committed.iter().find(|c| &c.turn_id == tid)?.sequence;
            Some(ChronicleCode::new(ChronicleLevel::A, seq))
        };

        let refreshed = refresh_context_epoch(
            epoch.as_ref(),
            &committed,
            &overview_candidates,
            &band_code_for_turn,
            cfg.params,
            chronicle_revision,
        );
        epoch = Some(refreshed.snapshot.clone());

        // 合成 near 正文：严格按 membership 拆分 H_anchor 与 live_suffix
        let body_for = |tid: &Id| -> Vec<ChatMessage> {
            let seq = committed
                .iter()
                .find(|c| &c.turn_id == tid)
                .map(|c| c.sequence)
                .unwrap_or(0);
            vec![
                ChatMessage::user(format!(
                    "intent-{seq}: synthetic user intent for turn {seq}"
                )),
                ChatMessage::assistant(format!(
                    "draft-{seq}: synthetic accepted draft body for turn {seq}, padded for token slope."
                )),
            ]
        };
        let mut anchor_body = Vec::new();
        for tid in &refreshed.membership.anchor_turn_ids {
            anchor_body.extend(body_for(tid));
        }
        let mut live_suffix_body = Vec::new();
        for tid in &refreshed.membership.live_suffix_turn_ids {
            live_suffix_body.extend(body_for(tid));
        }

        let overview_lines: Vec<(ChronicleCode, String)> = refreshed
            .snapshot
            .overview_codes
            .iter()
            .map(|c| (c.clone(), format!("headline for {}", c.as_str())))
            .collect();
        let band_lines: Vec<(ChronicleCode, String)> = refreshed
            .snapshot
            .band_codes
            .iter()
            .map(|c| (c.clone(), format!("band summary {}", c.as_str())))
            .collect();

        let input = ContextCompileInput {
            snapshot: refreshed.snapshot.clone(),
            capture: ContextCompileCapture {
                campaign_revision: chronicle_revision,
                chronicle_revision,
                epoch_id: refreshed.snapshot.epoch_id.clone(),
            },
            live_suffix_body,
            anchor_body,
            optional_checkpoint: None,
            overview_lines,
            band_lines,
        };
        let compiled = compile_history_blocks(&input);

        let mut est = 0u32;
        for block in &compiled.history_blocks {
            match block {
                storyforge_domain::chronicle::HistoryBlock::Checkpoint(s) => {
                    est = est.saturating_add(estimate_tokens_approx(s));
                }
                storyforge_domain::chronicle::HistoryBlock::Overview { lines }
                | storyforge_domain::chronicle::HistoryBlock::Band { lines } => {
                    for line in lines {
                        est = est.saturating_add(estimate_tokens_approx(line));
                    }
                }
                storyforge_domain::chronicle::HistoryBlock::NearRaw { messages } => {
                    for m in messages {
                        est = est.saturating_add(estimate_tokens_approx(&m.content));
                    }
                }
            }
        }
        token_estimates.push(est);
        // 用微秒避免亚毫秒轮次全被截成 0ms 导致“恒真”延迟断言
        let elapsed_us = t0.elapsed().as_micros();
        latency_us.push(elapsed_us);
        latencies.push(elapsed_us / 1000);

        let near_count = refreshed.membership.near_raw_turn_ids.len() as u32;
        let anchor_count = refreshed.membership.anchor_turn_ids.len() as u32;
        observed_max_near = observed_max_near.max(near_count);
        observed_max_anchor = observed_max_anchor.max(anchor_count);
        final_near = refreshed.membership.near_raw_turn_ids.len();
        final_anchor = refreshed.membership.anchor_turn_ids.len();
        final_live = refreshed.membership.live_suffix_turn_ids.len();
        final_band = refreshed.membership.band_turn_ids.len();
        final_overview = refreshed.snapshot.overview_codes.len();
    }

    let slope = prompt_growth_slope(&token_estimates);
    let mut sorted_lat_us = latency_us.clone();
    sorted_lat_us.sort_unstable();
    let latency_us_total: u128 = latency_us.iter().sum();
    let latency_ms_total = latency_us_total / 1000;
    let latency_ms_p50 = percentile_ms(&sorted_lat_us, 0.50) / 1000;
    let latency_ms_p95 = percentile_ms(&sorted_lat_us, 0.95) / 1000;
    let latency_samples = latency_us.len() as u32;

    let max_near_raw = cfg.params.max_near_raw_turns();
    let mut assertions = vec![
        BenchmarkAssertion {
            name: "turns_ge_100".into(),
            passed: cfg.turns >= 100,
            detail: format!("turns={}", cfg.turns),
        },
        BenchmarkAssertion {
            name: "near_raw_capped".into(),
            passed: observed_max_near <= max_near_raw,
            detail: format!("observed_max={observed_max_near} cap={max_near_raw}"),
        },
        BenchmarkAssertion {
            name: "max_near_window_budget".into(),
            passed: observed_max_near <= cfg.budgets.max_near_window,
            detail: format!(
                "observed_max={observed_max_near} budget={}",
                cfg.budgets.max_near_window
            ),
        },
        BenchmarkAssertion {
            name: "prompt_growth_slope_budget".into(),
            passed: slope <= cfg.budgets.max_prompt_growth_slope,
            detail: format!(
                "slope={slope:.4} budget={}",
                cfg.budgets.max_prompt_growth_slope
            ),
        },
        BenchmarkAssertion {
            name: "defaults_unchanged".into(),
            passed: cfg.params.h_anchor == DEFAULT_H_ANCHOR
                && cfg.params.e == DEFAULT_E
                && cfg.params.overview_max_entries == DEFAULT_OVERVIEW_MAX_ENTRIES,
            detail: format!(
                "h={} e={} overview_max={}",
                cfg.params.h_anchor, cfg.params.e, cfg.params.overview_max_entries
            ),
        },
        BenchmarkAssertion {
            name: "membership_non_empty_final".into(),
            passed: final_near > 0,
            detail: format!(
                "near={final_near} anchor={final_anchor} live={final_live} band={final_band} overview={final_overview}"
            ),
        },
        BenchmarkAssertion {
            name: "stable_h_anchor_membership".into(),
            // 越过 H+E 后 membership 的 anchor 应稳定为 H_anchor（不是把全部 near 塞进 live）
            passed: observed_max_anchor == cfg.params.h_anchor
                && final_anchor == cfg.params.h_anchor as usize,
            detail: format!(
                "observed_max_anchor={observed_max_anchor} final_anchor={final_anchor} h={}",
                cfg.params.h_anchor
            ),
        },
        BenchmarkAssertion {
            name: "latency_samples_complete".into(),
            // 不要求 wall-clock >0ms（机器可能亚毫秒），但必须每轮采样且可汇总
            passed: latency_samples == cfg.turns && latency_us_total > 0,
            detail: format!(
                "samples={latency_samples}/{} total_us={latency_us_total} p50_ms={latency_ms_p50} p95_ms={latency_ms_p95}",
                cfg.turns
            ),
        },
    ];

    let mut report = ContextCompileBenchmarkReport {
        schema_version: schema_version().into(),
        turns: cfg.turns,
        h_anchor: cfg.params.h_anchor,
        e: cfg.params.e,
        overview_max_entries: cfg.params.overview_max_entries,
        max_near_raw,
        observed_max_near_raw: observed_max_near,
        final_near_raw: final_near,
        final_band,
        final_overview,
        token_estimates: token_estimates.clone(),
        prompt_growth_slope: slope,
        latency_ms_total,
        latency_ms_p50,
        latency_ms_p95,
        evidence_bytes: 0,
        assertions: assertions.clone(),
    };

    write_observability_report(&cfg.report_path, &report).expect("write benchmark report");
    let evidence_bytes = std::fs::metadata(&cfg.report_path)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    report.evidence_bytes = evidence_bytes;

    let size_ok = evidence_bytes <= cfg.budgets.max_evidence_bytes;
    assertions.push(BenchmarkAssertion {
        name: "evidence_size_budget".into(),
        passed: size_ok,
        detail: format!(
            "bytes={evidence_bytes} budget={}",
            cfg.budgets.max_evidence_bytes
        ),
    });
    // 报告正文不得含密钥/完整 messages
    let body = std::fs::read_to_string(&cfg.report_path).unwrap_or_default();
    let redaction_ok = !crate::evidence::contains_forbidden_evidence_payload(&body)
        && !body.contains("synthetic accepted draft body");
    assertions.push(BenchmarkAssertion {
        name: "evidence_redaction".into(),
        passed: redaction_ok,
        detail: if redaction_ok {
            "no secrets or full draft text".into()
        } else {
            "forbidden payload detected".into()
        },
    });
    report.assertions = assertions;
    // 回写含最终断言的报告
    write_observability_report(&cfg.report_path, &report).expect("rewrite benchmark report");
    report.evidence_bytes = std::fs::metadata(&cfg.report_path)
        .map(|m| m.len() as usize)
        .unwrap_or(evidence_bytes);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_compile_benchmark_100_plus_turns_meets_budgets() {
        let dir = std::env::temp_dir().join(format!("sf_ctx_bench_{}", uuid::Uuid::new_v4()));
        let cfg = ContextCompileBenchConfig {
            turns: 120,
            params: ContextWindowParams::default(),
            report_path: dir.join("bench.json"),
            budgets: BudgetAssertions::default(),
        };
        let report = run_context_compile_benchmark(&cfg);
        assert!(report.turns >= 100);
        assert!(report.observed_max_near_raw <= report.max_near_raw);
        assert_eq!(report.h_anchor, DEFAULT_H_ANCHOR);
        assert_eq!(report.e, DEFAULT_E);
        assert_eq!(report.overview_max_entries, DEFAULT_OVERVIEW_MAX_ENTRIES);
        for a in &report.assertions {
            assert!(a.passed, "assertion {} failed: {}", a.name, a.detail);
        }
        assert!(report.evidence_bytes > 0);
        assert!(
            report
                .assertions
                .iter()
                .any(|a| a.name == "stable_h_anchor_membership" && a.passed)
        );
        assert!(
            report
                .assertions
                .iter()
                .any(|a| a.name == "latency_samples_complete" && a.passed)
        );
        let body = std::fs::read_to_string(&cfg.report_path).unwrap();
        assert!(body.contains("prompt_growth_slope"));
        assert!(!body.contains("api_key"));
        let _ = std::fs::remove_dir_all(dir);
    }
}

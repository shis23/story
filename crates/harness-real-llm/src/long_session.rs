//! 长会话 Accept runner（≥20 轮，跨越 H_anchor + E）。
//!
//! 确定性路径：合成正文 + production-faithful CommitTurn。
//! 真实模型路径：由测试在显式开关下驱动，本模块只负责编排与指标。

use std::path::PathBuf;

use storyforge_domain::Id;
use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR};
use storyforge_domain::turn::QualityReport;

use crate::commit_probe::{CommitProbeEnv, ProductionAcceptInput, ProductionAcceptResult};
use crate::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceTurnRecord, EvidenceWriter, short_hash16,
};

/// 长会话配置。
#[derive(Debug, Clone)]
pub struct LongSessionConfig {
    pub turns: u32,
    pub suite: String,
    pub run_id: String,
    pub evidence_path: PathBuf,
    /// 在第 `early_fact_turn` 写入唯一 token，并在最后一轮检查 catalog/store 可达性。
    pub early_fact_turn: u32,
    pub early_fact_token: String,
}

impl Default for LongSessionConfig {
    fn default() -> Self {
        Self {
            turns: 20,
            suite: "long_session".into(),
            run_id: format!("long-{}", uuid::Uuid::new_v4()),
            evidence_path: std::env::temp_dir().join("storyforge_eval_long_session.jsonl"),
            early_fact_turn: 1,
            early_fact_token: "EARLYFACT-ZXQ-7719".into(),
        }
    }
}

/// 单轮指标。
#[derive(Debug, Clone)]
pub struct LongSessionTurnMetric {
    pub turn_index: u32,
    pub accept: ProductionAcceptResult,
    pub elapsed_ms: u128,
    pub early_fact_in_summary: bool,
}

/// 长会话汇总。
#[derive(Debug, Clone)]
pub struct LongSessionReport {
    pub campaign_id: Id,
    pub conversation_id: Id,
    pub turns_requested: u32,
    pub turns_accepted: u32,
    pub h_anchor: u32,
    pub e: u32,
    pub max_near_raw: u32,
    pub crossed_h_plus_e: bool,
    pub early_fact_token: String,
    pub early_fact_present_in_store: bool,
    pub final_summary_count: usize,
    pub final_campaign_revision: u64,
    pub final_chronicle_revision: u64,
    pub metrics: Vec<LongSessionTurnMetric>,
    pub evidence_path: PathBuf,
    pub assertions: Vec<AssertionResult>,
}

/// 运行确定性长会话（合成 draft/summary，生产 Accept）。
pub fn run_deterministic_long_session(cfg: &LongSessionConfig) -> LongSessionReport {
    let env = CommitProbeEnv::new();
    let (campaign_id, conversation_id) = env.bootstrap_campaign("eval-long-session");
    let writer = EvidenceWriter::create(&cfg.evidence_path, &cfg.run_id)
        .expect("create long-session evidence writer");

    let max_near_raw = DEFAULT_H_ANCHOR + DEFAULT_E;
    let mut metrics = Vec::new();
    let mut turns_accepted = 0u32;

    for i in 1..=cfg.turns {
        let t0 = std::time::Instant::now();
        let fact_mark = if i == cfg.early_fact_turn {
            format!("关键早期事实：{}", cfg.early_fact_token)
        } else {
            format!("近轮填充-{i}")
        };
        let draft = format!(
            "第{i}幕正文。{fact_mark}。角色在雨夜继续推进调查，保持目标连贯，并留下可接续的互动停点。额外填充以确保正文长度足够通过质量门禁字数下限。"
        );
        let summary = format!("第{i}轮纪要。{fact_mark}。调查继续。");
        let variant_id = env.append_ai_draft(&conversation_id, &draft);
        let input = ProductionAcceptInput {
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            variant_id,
            draft_text: draft.clone(),
            summary_text: Some(summary.clone()),
            turn_number: i,
            quality_report: Some(QualityReport { warnings: vec![] }),
            force_accept: false,
        };
        env.prepare_awaiting_accept(&input);
        let accept = env.accept_production(&input);
        let elapsed_ms = t0.elapsed().as_millis();
        let early_fact_in_summary = summary.contains(&cfg.early_fact_token);
        if accept.ok {
            turns_accepted += 1;
        }

        let turn_rec = EvidenceTurnRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: cfg.run_id.clone(),
            suite: cfg.suite.clone(),
            turn_index: i,
            kind: "production_commit_turn".into(),
            draft_accepted: accept.ok,
            force_accept: accept.force_accept,
            quality_error_count: 0,
            quality_warning_count: 0,
            autofix_attempts: 0,
            campaign_revision_before: accept.campaign_revision_before,
            campaign_revision_after: accept.campaign_revision_after,
            chronicle_revision_before: accept.chronicle_revision_before,
            chronicle_revision_after: accept.chronicle_revision_after,
            summary_code: accept.summary_code.clone(),
            attempt_status: format!("{:?}", accept.attempt_status),
            turn_status: format!("{:?}", accept.turn_status),
            draft_hash16: short_hash16(&accept.draft_hash),
            text_len: draft.chars().count(),
            text_sha16: short_hash16(&draft),
            early_fact_reachable: None,
            assertion_results: accept.assertions.clone(),
            elapsed_ms,
            recorded_at_unix_ms: 0,
        };
        writer.write_turn(turn_rec).expect("write turn evidence");

        metrics.push(LongSessionTurnMetric {
            turn_index: i,
            accept,
            elapsed_ms,
            early_fact_in_summary,
        });
    }

    let summaries = env.campaign_store.list_summaries(&campaign_id);
    let early_fact_present_in_store = summaries
        .iter()
        .any(|s| s.content.contains(&cfg.early_fact_token));
    let camp = env
        .campaign_store
        .get_campaign(&campaign_id)
        .expect("campaign final");

    let crossed = turns_accepted > max_near_raw;
    let mut assertions = vec![
        AssertionResult {
            name: "accept_count".into(),
            passed: turns_accepted == cfg.turns,
            detail: Some(format!("{turns_accepted}/{}", cfg.turns)),
        },
        AssertionResult {
            name: "crossed_h_anchor_plus_e".into(),
            passed: crossed,
            detail: Some(format!(
                "accepted={turns_accepted} max_near_raw={max_near_raw} (H={DEFAULT_H_ANCHOR} E={DEFAULT_E})"
            )),
        },
        AssertionResult {
            name: "early_fact_present_in_store".into(),
            passed: early_fact_present_in_store,
            detail: Some(short_hash16(&cfg.early_fact_token)),
        },
        AssertionResult {
            name: "summary_count".into(),
            passed: summaries.len() as u32 == cfg.turns,
            detail: Some(format!("{}", summaries.len())),
        },
    ];

    // final evidence line with early-fact reachability
    let final_turn = EvidenceTurnRecord {
        schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        run_id: cfg.run_id.clone(),
        suite: cfg.suite.clone(),
        turn_index: cfg.turns,
        kind: "long_session_summary".into(),
        draft_accepted: turns_accepted == cfg.turns,
        force_accept: false,
        quality_error_count: 0,
        quality_warning_count: 0,
        autofix_attempts: 0,
        campaign_revision_before: 0,
        campaign_revision_after: camp.revision,
        chronicle_revision_before: 0,
        chronicle_revision_after: camp.chronicle_revision,
        summary_code: None,
        attempt_status: "n/a".into(),
        turn_status: "n/a".into(),
        draft_hash16: String::new(),
        text_len: 0,
        text_sha16: String::new(),
        early_fact_reachable: Some(early_fact_present_in_store),
        assertion_results: assertions.clone(),
        elapsed_ms: metrics.iter().map(|m| m.elapsed_ms).sum(),
        recorded_at_unix_ms: 0,
    };
    writer
        .write_turn(final_turn)
        .expect("write summary evidence");

    // keep env data for caller inspection via report; cleanup is caller's choice
    // We drop env without cleanup so tests can re-open if needed; explicit cleanup below.
    let report = LongSessionReport {
        campaign_id,
        conversation_id,
        turns_requested: cfg.turns,
        turns_accepted,
        h_anchor: DEFAULT_H_ANCHOR,
        e: DEFAULT_E,
        max_near_raw,
        crossed_h_plus_e: crossed,
        early_fact_token: cfg.early_fact_token.clone(),
        early_fact_present_in_store,
        final_summary_count: summaries.len(),
        final_campaign_revision: camp.revision,
        final_chronicle_revision: camp.chronicle_revision,
        metrics,
        evidence_path: cfg.evidence_path.clone(),
        assertions: {
            // ensure all assertions recorded
            let _ = &mut assertions;
            assertions
        },
    };
    env.cleanup();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_long_session_crosses_h_plus_e() {
        let dir = std::env::temp_dir().join(format!("sf_eval_long_{}", uuid::Uuid::new_v4()));
        let cfg = LongSessionConfig {
            turns: 20,
            suite: "long_session_unit".into(),
            run_id: "unit-long".into(),
            evidence_path: dir.join("long.jsonl"),
            early_fact_turn: 1,
            early_fact_token: "EARLYFACT-UNIT-42".into(),
        };
        let report = run_deterministic_long_session(&cfg);
        assert_eq!(report.turns_accepted, 20);
        assert!(
            report.crossed_h_plus_e,
            "20 should exceed H+E={}",
            report.max_near_raw
        );
        assert_eq!(report.max_near_raw, DEFAULT_H_ANCHOR + DEFAULT_E);
        assert!(report.early_fact_present_in_store);
        assert!(report.assertions.iter().all(|a| a.passed));
        assert!(report.evidence_path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

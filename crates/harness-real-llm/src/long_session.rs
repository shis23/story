//! 长会话 Accept runner（≥20 轮，生产 CommitTurn + ContextEpoch membership）。
//!
//! 确定性路径：合成正文 + production-faithful CommitTurn，并在结束后用
//! `compute_epoch_membership` / `refresh_context_epoch` 校验 near_raw / band /
//! overview 边界，以及早期事实是否仍在 summary store。
//!
//! **诚实命名**：本 runner 证明的是
//! 「≥20 次生产 Accept 后，ContextEpoch membership 的 near_raw 被 H_anchor+E 截断，
//!  早期 summary 仍在 store」。它**不是**真实 LLM 叙述可达性测试。

use std::path::PathBuf;

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::chronicle::{
    CommittedTurnRef, ContextWindowParams, OverviewCandidate, build_context_epoch_snapshot,
    compute_epoch_membership, refresh_context_epoch,
};
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
    pub turn_id: Id,
    pub accept: ProductionAcceptResult,
    pub elapsed_ms: u128,
    pub early_fact_in_summary: bool,
}

/// ContextEpoch membership 快照（供断言与证据）。
#[derive(Debug, Clone)]
pub struct EpochMembershipSnapshot {
    pub near_raw_count: usize,
    pub band_count: usize,
    pub overview_count: usize,
    pub live_suffix_count: u32,
    pub max_near_raw: u32,
    pub near_raw_capped_to_h_plus_e: bool,
    pub early_turn_in_near_raw: bool,
    pub early_summary_in_store: bool,
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
    /// accepted 是否严格大于 H_anchor+E（数量层面）。
    pub accepted_exceeds_h_plus_e: bool,
    /// ContextEpoch membership 是否显示 near_raw 被 H+E 截断。
    pub near_raw_capped_to_h_plus_e: bool,
    /// 兼容旧字段名：`accepted_exceeds_h_plus_e && near_raw_capped_to_h_plus_e`。
    pub crossed_h_plus_e: bool,
    pub early_fact_token: String,
    pub early_fact_present_in_store: bool,
    pub early_turn_in_near_raw: bool,
    pub epoch: Option<EpochMembershipSnapshot>,
    pub final_summary_count: usize,
    pub final_campaign_revision: u64,
    pub final_chronicle_revision: u64,
    pub metrics: Vec<LongSessionTurnMetric>,
    pub evidence_path: PathBuf,
    pub assertions: Vec<AssertionResult>,
}

/// 运行确定性长会话（合成 draft/summary，生产 Accept + ContextEpoch 校验）。
pub fn run_deterministic_long_session(cfg: &LongSessionConfig) -> LongSessionReport {
    let env = CommitProbeEnv::new();
    let (campaign_id, conversation_id) = env.bootstrap_campaign("eval-long-session");
    let writer = EvidenceWriter::create(&cfg.evidence_path, &cfg.run_id)
        .expect("create long-session evidence writer");

    let params = ContextWindowParams::default();
    let max_near_raw = params.max_near_raw_turns();
    let mut metrics = Vec::new();
    let mut turns_accepted = 0u32;
    let mut committed: Vec<CommittedTurnRef> = Vec::new();
    let mut early_turn_id: Option<Id> = None;

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
            variant_id: variant_id.clone(),
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

        // Resolve turn id from the accepted variant for ContextEpoch membership.
        let turn_id = env
            .turn_store
            .get_turn_by_variant(&variant_id)
            .map(|t| t.turn_id)
            .unwrap_or_default();

        if accept.ok {
            turns_accepted += 1;
            committed.push(CommittedTurnRef {
                turn_id: turn_id.clone(),
                sequence: i,
            });
            if i == cfg.early_fact_turn {
                early_turn_id = Some(turn_id.clone());
            }
        }

        let turn_rec = EvidenceTurnRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: cfg.run_id.clone(),
            suite: cfg.suite.clone(),
            turn_index: i,
            kind: "production_commit_turn".into(),
            write_path: "synthetic_deterministic_fixture".into(),
            chronicle_path: "synthetic_chronicle_fixture".into(),
            accept_path: "production_faithful_commit_probe".into(),
            production_postprocess_complete: false,
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
            context_epoch_id16: None,
            context_epoch_source_hash16: None,
            context_epoch_anchor_count: None,
            assertion_results: accept.assertions.clone(),
            elapsed_ms,
            recorded_at_unix_ms: 0,
        };
        writer.write_turn(turn_rec).expect("write turn evidence");

        metrics.push(LongSessionTurnMetric {
            turn_index: i,
            turn_id,
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

    // ── ContextEpoch membership（真实 domain 纯函数，非常量比较）──────────
    let epoch_snap = compute_epoch_membership_snapshot(
        &committed,
        &summaries,
        &params,
        early_turn_id.as_ref(),
        early_fact_present_in_store,
        camp.chronicle_revision,
    );

    let accepted_exceeds = turns_accepted > max_near_raw;
    let near_raw_capped = epoch_snap
        .as_ref()
        .map(|e| e.near_raw_capped_to_h_plus_e)
        .unwrap_or(false);
    let early_turn_in_near_raw = epoch_snap
        .as_ref()
        .map(|e| e.early_turn_in_near_raw)
        .unwrap_or(false);
    // 旧字段：数量越过 H+E 且 membership 截断同时成立，才称 "crossed"。
    let crossed = accepted_exceeds && near_raw_capped;

    let mut assertions = vec![
        AssertionResult {
            name: "accept_count".into(),
            passed: turns_accepted == cfg.turns,
            detail: Some(format!("{turns_accepted}/{}", cfg.turns)),
        },
        AssertionResult {
            name: "accepted_exceeds_h_plus_e".into(),
            passed: accepted_exceeds,
            detail: Some(format!(
                "accepted={turns_accepted} max_near_raw={max_near_raw} (H={} E={})",
                params.h_anchor, params.e
            )),
        },
        AssertionResult {
            name: "near_raw_capped_to_h_plus_e".into(),
            passed: near_raw_capped,
            detail: epoch_snap.as_ref().map(|e| {
                format!(
                    "near_raw={} max={} band={} overview={} live_suffix={}",
                    e.near_raw_count,
                    e.max_near_raw,
                    e.band_count,
                    e.overview_count,
                    e.live_suffix_count
                )
            }),
        },
        AssertionResult {
            name: "early_fact_present_in_store".into(),
            passed: early_fact_present_in_store,
            detail: Some(short_hash16(&cfg.early_fact_token)),
        },
        AssertionResult {
            name: "early_turn_evicted_from_near_raw".into(),
            // 当 accepted > H+E 时，最早一轮应不在 near_raw 窗口内。
            passed: if accepted_exceeds {
                !early_turn_in_near_raw
            } else {
                true
            },
            detail: Some(format!("early_turn_in_near_raw={early_turn_in_near_raw}")),
        },
        AssertionResult {
            name: "summary_count".into(),
            passed: summaries.len() as u32 == cfg.turns,
            detail: Some(format!("{}", summaries.len())),
        },
        // 兼容旧断言名：要求数量越过 + membership 截断。
        AssertionResult {
            name: "crossed_h_anchor_plus_e".into(),
            passed: crossed,
            detail: Some(format!(
                "accepted_exceeds={accepted_exceeds} near_raw_capped={near_raw_capped}"
            )),
        },
    ];

    let final_turn = EvidenceTurnRecord {
        schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        run_id: cfg.run_id.clone(),
        suite: cfg.suite.clone(),
        turn_index: cfg.turns,
        kind: "long_session_epoch_summary".into(),
        write_path: "synthetic_deterministic_fixture".into(),
        chronicle_path: "synthetic_chronicle_fixture".into(),
        accept_path: "production_faithful_commit_probe".into(),
        production_postprocess_complete: false,
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
        context_epoch_id16: None,
        context_epoch_source_hash16: None,
        context_epoch_anchor_count: None,
        assertion_results: assertions.clone(),
        elapsed_ms: metrics.iter().map(|m| m.elapsed_ms).sum(),
        recorded_at_unix_ms: 0,
    };
    writer
        .write_turn(final_turn)
        .expect("write summary evidence");

    let report = LongSessionReport {
        campaign_id,
        conversation_id,
        turns_requested: cfg.turns,
        turns_accepted,
        h_anchor: params.h_anchor,
        e: params.e,
        max_near_raw,
        accepted_exceeds_h_plus_e: accepted_exceeds,
        near_raw_capped_to_h_plus_e: near_raw_capped,
        crossed_h_plus_e: crossed,
        early_fact_token: cfg.early_fact_token.clone(),
        early_fact_present_in_store,
        early_turn_in_near_raw,
        epoch: epoch_snap,
        final_summary_count: summaries.len(),
        final_campaign_revision: camp.revision,
        final_chronicle_revision: camp.chronicle_revision,
        metrics,
        evidence_path: cfg.evidence_path.clone(),
        assertions: {
            let _ = &mut assertions;
            assertions
        },
    };
    env.cleanup();
    report
}

/// 用 domain 纯函数编译一次 ContextEpoch membership，并断言 near_raw 截断。
fn compute_epoch_membership_snapshot(
    committed: &[CommittedTurnRef],
    summaries: &[RoundSummary],
    params: &ContextWindowParams,
    early_turn_id: Option<&Id>,
    early_summary_in_store: bool,
    chronicle_revision: u64,
) -> Option<EpochMembershipSnapshot> {
    use storyforge_domain::chronicle::{ChronicleCode, ChronicleLevel};

    if committed.is_empty() {
        return None;
    }

    // overview candidates from persisted A-level summaries
    let overview_candidates: Vec<OverviewCandidate> = summaries
        .iter()
        .map(|s| {
            let code = s
                .code
                .as_deref()
                .and_then(ChronicleCode::parse)
                .unwrap_or_else(|| {
                    ChronicleCode::new(
                        ChronicleLevel::from_u8(s.level).unwrap_or(ChronicleLevel::A),
                        s.turn,
                    )
                });
            OverviewCandidate {
                code,
                level: ChronicleLevel::from_u8(s.level).unwrap_or(ChronicleLevel::A),
                turn_start: s.turn,
                covered_by: s.covered_by.clone(),
            }
        })
        .collect();

    let band_code_for_turn = |tid: &Id| -> Option<ChronicleCode> {
        summaries.iter().find_map(|s| {
            let seq = committed
                .iter()
                .find(|c| &c.turn_id == tid)
                .map(|c| c.sequence);
            if seq.is_some_and(|n| n == s.turn) {
                s.code
                    .as_deref()
                    .and_then(ChronicleCode::parse)
                    .or_else(|| Some(ChronicleCode::new(ChronicleLevel::A, s.turn)))
            } else {
                None
            }
        })
    };

    // Production path: create epoch then refresh (may rollover when live_suffix == E).
    let refreshed = refresh_context_epoch(
        None,
        committed,
        &overview_candidates,
        &band_code_for_turn,
        *params,
        chronicle_revision,
    );
    let _ = build_context_epoch_snapshot(
        &refreshed.membership,
        &overview_candidates,
        &band_code_for_turn,
        *params,
        chronicle_revision,
        None,
    );

    // Mid-epoch membership that fills live_suffix to E so near_raw hits H_anchor+E hard cap.
    // head ≈ last-(E) committed turn → anchor ends at head, live_suffix = next E turns.
    let max_near = params.max_near_raw_turns();
    let head_idx = if committed.len() as u32 > params.e {
        Some(committed.len() - 1 - params.e as usize)
    } else {
        None
    };
    let head_id = head_idx.map(|i| committed[i].turn_id.clone());
    let membership = compute_epoch_membership(committed, head_id.as_ref(), *params);
    let near_raw_count = membership.near_raw_turn_ids.len();
    let near_raw_capped = near_raw_count as u32 <= max_near
        && (committed.len() as u32 > max_near)
        && near_raw_count as u32 == max_near;

    let early_turn_in_near_raw = early_turn_id
        .map(|id| membership.near_raw_turn_ids.iter().any(|t| t == id))
        .unwrap_or(false);

    Some(EpochMembershipSnapshot {
        near_raw_count,
        band_count: membership.band_turn_ids.len(),
        overview_count: refreshed.snapshot.overview_codes.len(),
        live_suffix_count: membership.live_suffix_count,
        max_near_raw: max_near,
        near_raw_capped_to_h_plus_e: near_raw_capped,
        early_turn_in_near_raw,
        early_summary_in_store,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR};

    #[test]
    fn deterministic_long_session_caps_near_raw_via_context_epoch() {
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
        assert_eq!(report.max_near_raw, DEFAULT_H_ANCHOR + DEFAULT_E);
        assert!(
            report.accepted_exceeds_h_plus_e,
            "20 should exceed H+E={}",
            report.max_near_raw
        );
        assert!(
            report.near_raw_capped_to_h_plus_e,
            "ContextEpoch membership must cap near_raw to H+E"
        );
        assert!(report.crossed_h_plus_e);
        assert!(report.early_fact_present_in_store);
        // 早期 turn 应被挤出 near_raw 窗口
        assert!(
            !report.early_turn_in_near_raw,
            "early turn must be outside near_raw after >H+E accepts"
        );
        assert!(report.assertions.iter().all(|a| a.passed));
        assert!(report.evidence_path.exists());
        let epoch = report.epoch.expect("epoch snapshot");
        assert!(epoch.near_raw_count as u32 <= report.max_near_raw);
        let _ = std::fs::remove_dir_all(dir);
    }
}

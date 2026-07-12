//! M5 / Phase B 评估线 — 确定性门禁（无付费模型）。
//!
//! 覆盖：
//! 1. 生产忠实 CommitTurn Accept 闭环
//! 2. ≥20 Accept 跨越 H_anchor+E
//! 3. Phase B A/B 对照矩阵
//! 4. 脱敏 JSONL 证据

use harness_real_llm::commit_probe::CommitProbeEnv;
use harness_real_llm::evidence::{
    EVIDENCE_SCHEMA_VERSION, EvidenceWriter, contains_forbidden_evidence_payload,
    read_evidence_lines,
};
use harness_real_llm::long_session::{LongSessionConfig, run_deterministic_long_session};
use harness_real_llm::phase_b_matrix::{default_phase_b_fixtures, run_phase_b_matrix};
use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR};
use storyforge_domain::turn::{
    QualityReport, QualitySeverity, QualityWarning, QualityWarningCode, TurnStatus,
};

#[test]
fn eval_production_commit_turn_accept_loop() {
    use harness_real_llm::commit_probe::ProductionAcceptInput;

    let env = CommitProbeEnv::new();
    let (campaign_id, conversation_id) = env.bootstrap_campaign("eval-prod-accept");
    let draft = "生产 Accept 探针：角色在雾港确立银鸦标记，并留下可接续互动。".repeat(3);
    let variant_id = env.append_ai_draft(&conversation_id, &draft);
    let input = ProductionAcceptInput {
        campaign_id: campaign_id.clone(),
        conversation_id: conversation_id.clone(),
        variant_id: variant_id.clone(),
        draft_text: draft,
        summary_text: Some("A 轮：银鸦标记确立。".into()),
        turn_number: 1,
        quality_report: Some(QualityReport { warnings: vec![] }),
        force_accept: false,
    };
    env.prepare_awaiting_accept(&input);
    let result = env.accept_production(&input);
    assert!(result.ok, "production accept failed: {:?}", result.error);
    assert_eq!(result.turn_status, Some(TurnStatus::Committed));
    assert_eq!(
        result.campaign_revision_after,
        result.campaign_revision_before + 1
    );
    let summaries = env.campaign_store.list_summaries(&campaign_id);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].code.as_deref(), Some("A0001"));
    assert!(result.chronicle_revision_after > result.chronicle_revision_before);

    // active variant is Final
    let conv = env.conv_store.get(&conversation_id).unwrap();
    let node = conv.nodes.iter().find(|n| n.id == variant_id).unwrap();
    assert_eq!(
        node.active().unwrap().status,
        storyforge_domain::conversation::VariantStatus::Final
    );
    env.cleanup();
}

#[test]
fn eval_production_accept_blocks_quality_error() {
    use harness_real_llm::commit_probe::ProductionAcceptInput;

    let env = CommitProbeEnv::new();
    let (campaign_id, conversation_id) = env.bootstrap_campaign("eval-block-q");
    let draft = "质量拦截：正文足够长，但带 Error 级质量报告时应拒绝 Accept。".repeat(2);
    let variant_id = env.append_ai_draft(&conversation_id, &draft);
    let input = ProductionAcceptInput {
        campaign_id,
        conversation_id,
        variant_id,
        draft_text: draft,
        summary_text: Some("should not land".into()),
        turn_number: 1,
        quality_report: Some(QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::PrivateKnowledgeLeak {
                    secret_fingerprint: "abcd".into(),
                    owner_id: Some("inst-x".into()),
                },
                message: "leak".into(),
                severity: QualitySeverity::Error,
            }],
        }),
        force_accept: false,
    };
    env.prepare_awaiting_accept(&input);
    let result = env.accept_production(&input);
    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("quality gate blocked")
    );
    env.cleanup();
}

#[test]
fn eval_long_session_20_accepts_cross_h_plus_e() {
    let dir = std::env::temp_dir().join(format!("sf_eval_ls_{}", uuid::Uuid::new_v4()));
    let cfg = LongSessionConfig {
        turns: 20,
        suite: "eval_long_session".into(),
        run_id: "eval-long-1".into(),
        evidence_path: dir.join("long.jsonl"),
        early_fact_turn: 1,
        early_fact_token: "EARLYFACT-EVAL-20".into(),
    };
    let report = run_deterministic_long_session(&cfg);
    assert_eq!(report.turns_accepted, 20);
    assert_eq!(report.max_near_raw, DEFAULT_H_ANCHOR + DEFAULT_E);
    assert!(
        report.crossed_h_plus_e,
        "expected 20 > H+E={}",
        report.max_near_raw
    );
    assert!(report.early_fact_present_in_store);
    assert_eq!(report.final_summary_count, 20);
    assert!(report.assertions.iter().all(|a| a.passed));

    let lines = read_evidence_lines(&report.evidence_path).unwrap();
    assert!(lines.len() >= 21, "20 turns + summary");
    for line in &lines {
        let s = line.to_string();
        assert!(
            !contains_forbidden_evidence_payload(&s),
            "leaked secret: {s}"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn eval_phase_b_ab_matrix_same_seed() {
    let dir = std::env::temp_dir().join(format!("sf_eval_pb_{}", uuid::Uuid::new_v4()));
    let path = dir.join("ab.jsonl");
    let fixtures = default_phase_b_fixtures();
    let report = run_phase_b_matrix(&fixtures, path.clone(), "eval-pb-1");
    assert!(
        report.assertions.iter().all(|a| a.passed),
        "failed: {:?}",
        report
            .assertions
            .iter()
            .filter(|a| !a.passed)
            .collect::<Vec<_>>()
    );
    let lines = read_evidence_lines(&path).unwrap();
    assert_eq!(lines.len(), fixtures.len() * 2);
    for line in &lines {
        assert_eq!(line["schema_version"], EVIDENCE_SCHEMA_VERSION);
        assert!(!line.to_string().contains("SF_SECRET_"));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn eval_evidence_writer_rejects_secret_payload() {
    let dir = std::env::temp_dir().join(format!("sf_eval_ev2_{}", uuid::Uuid::new_v4()));
    let path = dir.join("bad.jsonl");
    let writer = EvidenceWriter::create(&path, "r").unwrap();
    let bad = serde_json::json!({
        "api_key": "sk-test-should-not-write",
        "role": "editor"
    });
    let err = writer.write_json_line(&bad).unwrap_err();
    assert!(err.to_string().contains("refusing") || err.to_string().contains("secret"));
    let _ = std::fs::remove_dir_all(dir);
}

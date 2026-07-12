//! Phase B 前后对照矩阵（A/B 同输入 / 同参数 / 同种子）。
//!
//! A 臂：无 NarrativeContract 私密扫描（baseline）。
//! B 臂：启用 attribution-aware QualityGate + 统计 auto-fix 次数。
//!
//! 确定性 fixture 使用稳定探针 `SF_SECRET_*`；证据只写 fingerprint，不写原文。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use storyforge_app_pipeline::quality_gate::{
    build_quality_fix_hint, run_quality_gate, run_quality_gate_with_contract,
};
use storyforge_domain::narrative_contract::{NarrativeContract, PrivateBinding};
use storyforge_domain::turn::{QualityReport, QualitySeverity, QualityWarningCode};

use crate::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceAbRow, EvidenceWriter, short_hash16,
};

/// A/B 臂标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseBArm {
    /// baseline：无 contract / 无 private leak 扫描
    BaselineA,
    /// Phase B：contract + attribution gate
    PhaseBB,
}

impl PhaseBArm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BaselineA => "baseline_a",
            Self::PhaseBB => "phase_b_b",
        }
    }
}

/// 单条对照 fixture（A/B 共用同一输入与种子）。
#[derive(Debug, Clone)]
pub struct PhaseBFixture {
    pub fixture_id: String,
    pub seed: u64,
    pub draft_text: String,
    pub owner_id: String,
    pub secret_probe: String,
    /// 若 true，正文故意包含 secret（应被 B 臂检出）
    pub inject_leak: bool,
}

/// 单臂运行结果。
#[derive(Debug, Clone, Serialize)]
pub struct PhaseBArmResult {
    pub arm: String,
    pub fixture_id: String,
    pub seed: u64,
    pub leak_detected: bool,
    pub quality_error_count: usize,
    pub quality_warning_count: usize,
    pub autofix_count: u32,
    pub latency_ms: u128,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    pub secret_fingerprint16: String,
    pub passed_gate: bool,
}

/// 对照矩阵汇总。
#[derive(Debug, Clone)]
pub struct PhaseBMatrixReport {
    pub rows: Vec<PhaseBArmResult>,
    pub evidence_path: PathBuf,
    pub assertions: Vec<AssertionResult>,
}

/// 内置确定性 fixture 集（A/B 同输入）。
pub fn default_phase_b_fixtures() -> Vec<PhaseBFixture> {
    vec![
        PhaseBFixture {
            fixture_id: "pb-leak-owner-probe".into(),
            seed: 42,
            draft_text: "雨夜码头，林沉默地看着陈。对话停在未决处，等待下一轮互动。".into(),
            owner_id: "inst-chen".into(),
            secret_probe: "SF_SECRET_CHEN_BADGE_X91".into(),
            inject_leak: true,
        },
        PhaseBFixture {
            fixture_id: "pb-clean-no-leak".into(),
            seed: 42,
            draft_text: "港口雾气散去，两人核对旧海图坐标，没有透露任何私密徽章编号。".into(),
            owner_id: "inst-chen".into(),
            secret_probe: "SF_SECRET_CHEN_BADGE_X91".into(),
            inject_leak: false,
        },
        PhaseBFixture {
            fixture_id: "pb-leak-short-token".into(),
            seed: 7,
            draft_text: "追兵逼近时，有人脱口而出不该说的话。".into(),
            owner_id: "inst-lin".into(),
            secret_probe: "SF_SECRET_LIN_CODE_7X9".into(),
            inject_leak: true,
        },
    ]
}

fn contract_for(fixture: &PhaseBFixture) -> NarrativeContract {
    NarrativeContract {
        private_bindings: vec![PrivateBinding {
            owner_id: fixture.owner_id.clone(),
            owner_name: Some("eval-owner".into()),
            secret: fixture.secret_probe.clone(),
        }],
        must_not_reveal: vec![fixture.secret_probe.clone()],
        ..NarrativeContract::default()
    }
}

fn render_draft(fixture: &PhaseBFixture) -> String {
    if fixture.inject_leak {
        format!(
            "{} 不该出现的私密探针：{}。正文继续推进，保持可接续。",
            fixture.draft_text, fixture.secret_probe
        )
    } else {
        format!(
            "{} 场景收束在互动中段，未暴露任何私密编号。",
            fixture.draft_text
        )
    }
}

fn report_metrics(report: &QualityReport) -> (usize, usize, bool) {
    let errors = report.error_count();
    let warnings = report.warnings.len();
    let leak = report.warnings.iter().any(|w| {
        matches!(w.code, QualityWarningCode::PrivateKnowledgeLeak { .. })
            && w.severity == QualitySeverity::Error
    });
    (errors, warnings, leak)
}

/// 运行单臂（确定性，无网络）。
pub fn run_arm(fixture: &PhaseBFixture, arm: PhaseBArm) -> PhaseBArmResult {
    let t0 = std::time::Instant::now();
    let draft = render_draft(fixture);
    let (report, autofix_count) = match arm {
        PhaseBArm::BaselineA => {
            // baseline：不传 contract
            (run_quality_gate(&draft), 0)
        }
        PhaseBArm::PhaseBB => {
            let contract = contract_for(fixture);
            let mut report = run_quality_gate_with_contract(&draft, Some(&contract));
            let mut autofix = 0u32;
            if report.has_errors() {
                // 模拟有界 1× auto-fix：去掉 leak 行再 gate（确定性修复稿）
                autofix = 1;
                let _hint = build_quality_fix_hint(&report);
                let fixed = draft
                    .lines()
                    .filter(|line| !line.contains(&fixture.secret_probe))
                    .collect::<Vec<_>>()
                    .join("\n");
                // 若仍含探针（同一行混写），做字符串擦除
                let fixed = fixed.replace(&fixture.secret_probe, "[REDACTED_PRIVATE]");
                report = run_quality_gate_with_contract(&fixed, Some(&contract));
            }
            (report, autofix)
        }
    };
    let (errors, warnings, leak) = report_metrics(&report);
    // token 成本在确定性路径记 0；真实模型路径由调用方覆写
    PhaseBArmResult {
        arm: arm.as_str().into(),
        fixture_id: fixture.fixture_id.clone(),
        seed: fixture.seed,
        leak_detected: leak,
        quality_error_count: errors,
        quality_warning_count: warnings,
        autofix_count,
        latency_ms: t0.elapsed().as_millis(),
        prompt_tokens: 0,
        completion_tokens: 0,
        cached_tokens: 0,
        secret_fingerprint16: short_hash16(&fixture.secret_probe),
        passed_gate: report.passed(),
    }
}

/// 跑完整 A/B 矩阵并写脱敏 JSONL。
pub fn run_phase_b_matrix(
    fixtures: &[PhaseBFixture],
    evidence_path: PathBuf,
    run_id: &str,
) -> PhaseBMatrixReport {
    let writer = EvidenceWriter::create(&evidence_path, run_id).expect("phase-b evidence writer");
    let mut rows = Vec::new();

    for fixture in fixtures {
        for arm in [PhaseBArm::BaselineA, PhaseBArm::PhaseBB] {
            let result = run_arm(fixture, arm);
            let ab = EvidenceAbRow {
                schema_version: EVIDENCE_SCHEMA_VERSION.into(),
                run_id: run_id.into(),
                suite: "phase_b_matrix".into(),
                fixture_id: result.fixture_id.clone(),
                arm: result.arm.clone(),
                seed: result.seed,
                model_label: "deterministic".into(),
                leak_detected: result.leak_detected,
                quality_error_count: result.quality_error_count,
                quality_warning_count: result.quality_warning_count,
                autofix_count: result.autofix_count,
                latency_ms: result.latency_ms,
                prompt_tokens: result.prompt_tokens,
                completion_tokens: result.completion_tokens,
                cached_tokens: result.cached_tokens,
                assertion_results: vec![],
                recorded_at_unix_ms: 0,
            };
            writer.write_ab_row(ab).expect("write ab row");
            rows.push(result);
        }
    }

    let mut assertions = Vec::new();

    // same seed across arms for each fixture
    for fixture in fixtures {
        let arms: Vec<_> = rows
            .iter()
            .filter(|r| r.fixture_id == fixture.fixture_id)
            .collect();
        assert_eq!(arms.len(), 2);
        assertions.push(AssertionResult {
            name: format!("same_seed:{}", fixture.fixture_id),
            passed: arms[0].seed == arms[1].seed && arms[0].seed == fixture.seed,
            detail: Some(format!("{}", fixture.seed)),
        });
    }

    // leak fixtures: baseline may miss (no contract), phase B must detect before fix
    // After autofix, phase B may pass; we assert leak_detected on the pre-fix semantics:
    // run_arm for Phase B sets leak_detected from final report. For inject_leak, we also
    // require autofix_count==1 and baseline leak_detected==false with errors possibly 0.
    for fixture in fixtures.iter().filter(|f| f.inject_leak) {
        let a = rows
            .iter()
            .find(|r| r.fixture_id == fixture.fixture_id && r.arm == PhaseBArm::BaselineA.as_str())
            .unwrap();
        let b = rows
            .iter()
            .find(|r| r.fixture_id == fixture.fixture_id && r.arm == PhaseBArm::PhaseBB.as_str())
            .unwrap();
        assertions.push(AssertionResult {
            name: format!("baseline_misses_or_weak:{}", fixture.fixture_id),
            passed: !a.leak_detected,
            detail: Some(format!("baseline_errors={}", a.quality_error_count)),
        });
        assertions.push(AssertionResult {
            name: format!("phase_b_autofix_used:{}", fixture.fixture_id),
            passed: b.autofix_count == 1,
            detail: Some(format!("autofix={}", b.autofix_count)),
        });
        // after autofix, gate should not still hard-fail on private leak
        assertions.push(AssertionResult {
            name: format!("phase_b_fixed_or_clean:{}", fixture.fixture_id),
            passed: !b.leak_detected,
            detail: Some(format!("errors={}", b.quality_error_count)),
        });
    }

    for fixture in fixtures.iter().filter(|f| !f.inject_leak) {
        let b = rows
            .iter()
            .find(|r| r.fixture_id == fixture.fixture_id && r.arm == PhaseBArm::PhaseBB.as_str())
            .unwrap();
        assertions.push(AssertionResult {
            name: format!("clean_no_false_leak:{}", fixture.fixture_id),
            passed: !b.leak_detected && b.autofix_count == 0,
            detail: None,
        });
    }

    PhaseBMatrixReport {
        rows,
        evidence_path,
        assertions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_b_matrix_detects_leak_and_autofixes() {
        let dir = std::env::temp_dir().join(format!("sf_eval_pb_{}", uuid::Uuid::new_v4()));
        let path = dir.join("phase_b.jsonl");
        let fixtures = default_phase_b_fixtures();
        let report = run_phase_b_matrix(&fixtures, path.clone(), "unit-pb");
        assert!(
            report.assertions.iter().all(|a| a.passed),
            "assertions failed: {:?}",
            report
                .assertions
                .iter()
                .filter(|a| !a.passed)
                .collect::<Vec<_>>()
        );
        // ensure evidence has 2 * fixtures rows
        let lines = crate::evidence::read_evidence_lines(&path).unwrap();
        assert_eq!(lines.len(), fixtures.len() * 2);
        // no raw secret in evidence
        for line in &lines {
            let s = line.to_string();
            assert!(!s.contains("SF_SECRET_"));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn arms_share_seed_and_fixture_input() {
        let f = &default_phase_b_fixtures()[0];
        let a = run_arm(f, PhaseBArm::BaselineA);
        let b = run_arm(f, PhaseBArm::PhaseBB);
        assert_eq!(a.seed, b.seed);
        assert_eq!(a.fixture_id, b.fixture_id);
        assert_eq!(a.secret_fingerprint16, b.secret_fingerprint16);
        assert!(!a.leak_detected);
        assert_eq!(b.autofix_count, 1);
    }
}

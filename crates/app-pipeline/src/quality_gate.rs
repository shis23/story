/// B3 DraftQualityGate（架构文档 §9）
///
/// 纯确定性规则门禁，不跑 LLM。在 Editor 产出 `final_text` 后、`run_postprocess` 前执行。
/// Gate 失败不硬阻断——只标记警告，用户仍可手动 accept（符合 Phase A 草稿有身份但不自动变事实的哲学）。
///
/// 检查项：
/// 1. n-gram 重复检测：UTF-8 字符级 8-gram 连续出现 ≥ 3 次
/// 2. 元描述检测：草稿含 LLM 自述/指令残留
/// 3. 字数下限：< 50 字
/// 4. 视角/破壁：对读者说话或指令式旁白
/// 5. 格式泄漏：代码块 / think 标签 / HTML
/// 6. 连续性：相邻句子完全重复
use storyforge_domain::narrative_contract::NarrativeContract;
use storyforge_domain::turn::{QualityReport, QualitySeverity, QualityWarning, QualityWarningCode};

/// 运行草稿质量门禁（无契约：不做 private leak 扫描）
pub fn run_quality_gate(text: &str) -> QualityReport {
    run_quality_gate_with_contract(text, None)
}

/// 运行草稿质量门禁；传入 NarrativeContract 时扫描 must_not_reveal / private_bindings。
pub fn run_quality_gate_with_contract(
    text: &str,
    contract: Option<&NarrativeContract>,
) -> QualityReport {
    let mut warnings: Vec<QualityWarning> = vec![
        check_ngram_repetition(text),
        check_meta_description(text),
        check_too_short(text),
        check_perspective_leak(text),
        check_format_leak(text),
        check_consecutive_repeat(text),
        check_em_dash(text, contract),
        check_negation_then_affirmation(text, contract),
    ]
    .into_iter()
    .flatten()
    .collect();

    if let Some(c) = contract {
        warnings.extend(check_private_knowledge_leak(text, c));
    }

    QualityReport { warnings }
}

/// n-gram 重复检测：UTF-8 字符级滑窗，统计 8-gram 出现次数。
///
/// 8-gram 在中文约 4 个词。同一 8-gram 出现 ≥ 3 次 → 说明有明显的模式重复。
fn check_ngram_repetition(text: &str) -> Option<QualityWarning> {
    const N: usize = 8;
    const MIN_REPEAT: usize = 3;

    let chars: Vec<char> = text.chars().collect();
    if chars.len() < N {
        return None; // 文本太短，不可能重复
    }

    let mut counts: std::collections::HashMap<&[char], usize> = std::collections::HashMap::new();

    for window in chars.windows(N) {
        *counts.entry(window).or_insert(0) += 1;
    }

    // 找出现次数最多的
    let max = counts
        .iter()
        .filter(|(_, c)| **c >= MIN_REPEAT)
        .max_by_key(|(_, c)| **c);

    if let Some((sample_chars, count)) = max {
        let sample: String = sample_chars.iter().collect();
        Some(QualityWarning {
            code: QualityWarningCode::NgramRepetition {
                n: N,
                count: *count,
                sample: sample.clone(),
            },
            message: format!("{N}-gram「{sample}」重复出现 {count} 次"),
            severity: QualitySeverity::Warning,
        })
    } else {
        None
    }
}

/// 元描述检测：草稿含 LLM 自述/指令残留。
///
/// 匹配常见泄漏模式。用的是包含匹配，不是正则——保持零依赖。
fn check_meta_description(text: &str) -> Option<QualityWarning> {
    const PATTERNS: &[&str] = &[
        "作为AI",
        "作为 AI",
        "作为人工智能",
        "我来写",
        "以下是为您创作",
        "以下是故事",
        "让我来",
        "现在开始创作",
        "好的，我",
        "没问题，我",
        "根据你的要求",
        "按照你的要求",
        "我将为你",
        "我来为你",
    ];

    for pat in PATTERNS {
        if text.contains(pat) {
            return Some(QualityWarning {
                code: QualityWarningCode::MetaDescription {
                    snippet: pat.to_string(),
                },
                message: format!("草稿含元描述泄漏：「{pat}」"),
                severity: QualitySeverity::Error,
            });
        }
    }
    None
}

/// 字数过短检测
fn check_too_short(text: &str) -> Option<QualityWarning> {
    const MIN_CHARS: usize = 50;
    let char_count = text.chars().count();
    if char_count < MIN_CHARS {
        Some(QualityWarning {
            code: QualityWarningCode::TooShort { char_count },
            message: format!("草稿仅 {char_count} 字，低于 {MIN_CHARS} 字下限"),
            severity: QualitySeverity::Warning,
        })
    } else {
        None
    }
}

/// 视角/破壁：直接对读者说话或指令式旁白。
fn check_perspective_leak(text: &str) -> Option<QualityWarning> {
    const PATTERNS: &[&str] = &[
        "亲爱的读者",
        "各位读者",
        "如果你想",
        "如果你希望",
        "请告诉我",
        "请继续输入",
        "下一章见",
        "未完待续，请",
        "（作者注",
        "(作者注",
        "OOC：",
        "OOC:",
        "【系统】",
        "[系统]",
    ];

    for pat in PATTERNS {
        if text.contains(pat) {
            return Some(QualityWarning {
                code: QualityWarningCode::PerspectiveLeak {
                    snippet: pat.to_string(),
                },
                message: format!("草稿含视角/破壁内容：「{pat}」"),
                severity: QualitySeverity::Error,
            });
        }
    }
    None
}

/// 格式泄漏：代码块、think 标签、HTML 等非正文残留。
fn check_format_leak(text: &str) -> Option<QualityWarning> {
    const PATTERNS: &[&str] = &[
        "```", "<think>", "</think>", "<div", "</div>", "<span", "</span>", "<p>", "</p>",
        "```json", "```xml",
    ];

    for pat in PATTERNS {
        if text.contains(pat) {
            return Some(QualityWarning {
                code: QualityWarningCode::FormatLeak {
                    snippet: pat.to_string(),
                },
                message: format!("草稿含格式泄漏：「{pat}」"),
                severity: QualitySeverity::Error,
            });
        }
    }
    None
}

/// 连续性：相邻句子完全重复（按 。！？.!? 粗分句）。
fn check_consecutive_repeat(text: &str) -> Option<QualityWarning> {
    let sentences: Vec<&str> = text
        .split(|c: char| "。！？.!?；;\n".contains(c))
        .map(str::trim)
        .filter(|s| s.chars().count() >= 6)
        .collect();

    for window in sentences.windows(2) {
        if window[0] == window[1] {
            let sample = truncate_sample(window[0], 40);
            return Some(QualityWarning {
                code: QualityWarningCode::ConsecutiveRepeat {
                    sample: sample.clone(),
                },
                message: format!("相邻句子完全重复：「{sample}」"),
                severity: QualitySeverity::Warning,
            });
        }
    }
    None
}

fn check_em_dash(text: &str, contract: Option<&NarrativeContract>) -> Option<QualityWarning> {
    let ban = contract
        .map(|c| c.style_constraints.ban_em_dash)
        .unwrap_or(true);
    if !ban {
        return None;
    }
    let count_double = text.matches("——").count();
    let count_em = text.matches('—').count().saturating_sub(count_double * 2);
    let count = count_double + count_em;
    if count == 0 {
        return None;
    }
    let severity = if count >= 3 {
        QualitySeverity::Error
    } else {
        QualitySeverity::Warning
    };
    Some(QualityWarning {
        code: QualityWarningCode::EmDashDensity { count },
        message: format!("草稿含破折号 {count} 处（建议改用逗号/句号/省略号）"),
        severity,
    })
}

fn check_negation_then_affirmation(
    text: &str,
    contract: Option<&NarrativeContract>,
) -> Option<QualityWarning> {
    let ban = contract
        .map(|c| c.style_constraints.ban_negation_affirmation)
        .unwrap_or(true);
    if !ban {
        return None;
    }
    // find 返回字节索引；从 &text[i..] 取字符窗口，避免把字节偏移当 char 计数。
    if let Some(i) = text.find("不是") {
        let tail: String = text[i..].chars().take(24).collect();
        if tail.contains("而是") || tail.contains("就是") {
            let sample = truncate_sample(&tail, 32);
            return Some(QualityWarning {
                code: QualityWarningCode::NegationThenAffirmation {
                    sample: sample.clone(),
                },
                message: format!("草稿含否后肯结构：「{sample}」"),
                severity: QualitySeverity::Warning,
            });
        }
    }
    None
}

fn check_private_knowledge_leak(text: &str, contract: &NarrativeContract) -> Vec<QualityWarning> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // v1：仅扫描显式 must_not_reveal 探针 → Error。
    // private_bindings 是归属信息，无说话者归因时不自动 Error（避免拥有者合法回忆被拦）。
    for token in &contract.must_not_reveal {
        let token = token.trim();
        if token.chars().count() < 4 {
            continue;
        }
        if !seen.insert(token.to_string()) {
            continue;
        }
        if text.contains(token) {
            let owner = contract.owner_of_secret(token).map(str::to_string);
            // 日志/报告不落完整 secret 原文：只给截断 SHA-256 + owner。
            let fingerprint = secret_fingerprint(token);
            out.push(QualityWarning {
                code: QualityWarningCode::PrivateKnowledgeLeak {
                    secret_fingerprint: fingerprint.clone(),
                    owner_id: owner.clone(),
                },
                message: match owner {
                    Some(o) => {
                        format!("正文出现硬禁探针（fp={fingerprint}，归属 {o}），可能越权全知")
                    }
                    None => format!("正文出现硬禁探针（fp={fingerprint}），可能越权全知"),
                },
                severity: QualitySeverity::Error,
            });
        }
    }
    out
}

/// 私密探针短指纹（截断 SHA-256，跨版本稳定；不落全文）
fn secret_fingerprint(secret: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(secret.as_bytes());
    // 16 hex chars = 64-bit 截断，足够报告去重/审计对照，且不回放原文
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn truncate_sample(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_chars).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::turn::QualityWarningCode;

    #[test]
    fn test_normal_text_passes() {
        let text = "夜风吹过窗棂，林秋坐在桌前，看着杯中残茶泛起的涟漪。他想起那年冬天，也是这样安静的夜晚。窗外有猫叫，声音远处传来，断断续续的。";
        let report = run_quality_gate(text);
        assert!(
            report.passed(),
            "正常叙事应通过门禁，实际: {:?}",
            report.warnings
        );
    }

    #[test]
    fn test_ngram_repetition_detected() {
        // 构造明显重复的 8-gram
        let unit = "abcdefgh";
        let text = format!("{unit}{unit}{unit} 夜风拂过窗前，远处灯火微明。");
        let report = run_quality_gate(&text);
        assert!(!report.passed(), "含重复 8-gram 应被检出");
        let has_ngram = report.warnings.iter().any(|w| {
            matches!(
                &w.code,
                QualityWarningCode::NgramRepetition { count: c, .. } if *c >= 3
            )
        });
        assert!(has_ngram, "应包含 NgramRepetition 警告");
    }

    #[test]
    fn test_meta_description_detected() {
        let text = "好的，我来为你写一个精彩的场景。夜风吹过窗棂，林秋坐在桌前，看着杯中残茶泛起的涟漪。他想起那年冬天，也是这样安静的夜晚。窗外有猫叫，声音远处传来。";
        let report = run_quality_gate(text);
        let has_meta = report
            .warnings
            .iter()
            .any(|w| matches!(&w.code, QualityWarningCode::MetaDescription { .. }));
        assert!(has_meta, "含「我来」应被检出元描述泄漏");
        assert!(
            report
                .warnings
                .iter()
                .filter(|w| matches!(w.severity, QualitySeverity::Error))
                .count()
                >= 1,
            "元描述应为 Error 级别"
        );
    }

    #[test]
    fn test_too_short_detected() {
        let text = "林秋推开门。";
        let report = run_quality_gate(text);
        let has_short = report.warnings.iter().any(|w| {
            matches!(
                &w.code,
                QualityWarningCode::TooShort { char_count: c } if *c < 50
            )
        });
        assert!(has_short, "短文本应被检出字数过短");
    }

    #[test]
    fn test_empty_text_detected() {
        let report = run_quality_gate("");
        assert!(!report.passed());
        // 空文本应至少检出 TooShort
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::TooShort { .. }))
        );
    }

    #[test]
    fn test_no_ngram_repetition_for_unique_text() {
        let text = "夜风拂面，林秋静坐窗前。茶香袅袅升起，远处传来猫叫声声。他望向那片星空，回忆如潮水般涌来。冬天的夜晚总是这样安静而寒冷。";
        let report = run_quality_gate(text);
        let has_ngram = report
            .warnings
            .iter()
            .any(|w| matches!(&w.code, QualityWarningCode::NgramRepetition { .. }));
        assert!(!has_ngram, "无重复的文本不应检出 n-gram 重复");
    }

    #[test]
    fn test_perspective_leak_detected() {
        let text = "亲爱的读者，接下来请继续输入你的选择。夜风吹过窗棂，林秋坐在桌前看着残茶。他想起那年冬天，也是这样安静的夜晚。";
        let report = run_quality_gate(text);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::PerspectiveLeak { .. })),
            "应检出视角/破壁: {:?}",
            report.warnings
        );
    }

    #[test]
    fn test_format_leak_detected() {
        let text = "夜风吹过窗棂。\n```json\n{\"ok\":true}\n```\n林秋坐在桌前，看着杯中残茶泛起的涟漪。他想起那年冬天。";
        let report = run_quality_gate(text);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::FormatLeak { .. })),
            "应检出格式泄漏: {:?}",
            report.warnings
        );
    }

    #[test]
    fn test_consecutive_repeat_detected() {
        let text = "林秋推开诊所的门。林秋推开诊所的门。窗外的雨还在下，灯火把地面映成浅金。";
        let report = run_quality_gate(text);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::ConsecutiveRepeat { .. })),
            "应检出相邻句子重复: {:?}",
            report.warnings
        );
    }
    #[test]
    fn test_em_dash_detected() {
        let text = "夜风——吹过窗棂，林秋坐在桌前，看着杯中残茶泛起的涟漪。他想起那年冬天，也是这样安静的夜晚。";
        let report = run_quality_gate(text);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::EmDashDensity { .. })),
            "应检出破折号: {:?}",
            report.warnings
        );
    }

    #[test]
    fn test_negation_then_affirmation_detected() {
        let text = "林秋不是在害怕，而是在盘算下一步。窗外雨声淅沥，急诊灯把走廊照得惨白。他抬起头看着陈警官。";
        let report = run_quality_gate(text);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::NegationThenAffirmation { .. })),
            "应检出否后肯: {:?}",
            report.warnings
        );
    }

    #[test]
    fn test_private_knowledge_leak_error() {
        use storyforge_domain::narrative_contract::{NarrativeContract, PrivateBinding};
        let contract = NarrativeContract {
            private_bindings: vec![PrivateBinding {
                owner_id: "inst-chen".into(),
                secret: "SF_SECRET_CHEN_BADGE_X91".into(),
            }],
            // 仅显式探针触发 Error
            must_not_reveal: vec!["SF_SECRET_CHEN_BADGE_X91".into()],
            ..Default::default()
        };
        let text = "林秋低声说：我知道 SF_SECRET_CHEN_BADGE_X91 这件事。窗外雨还在下，急诊灯闪着白光，空气里有消毒水味。";
        let report = run_quality_gate_with_contract(text, Some(&contract));
        assert!(
            report.warnings.iter().any(|w| matches!(
                &w.code,
                QualityWarningCode::PrivateKnowledgeLeak { .. }
            ) && w.severity == QualitySeverity::Error),
            "应检出硬禁探针 Error: {:?}",
            report.warnings
        );
        // 报告不得包含完整 secret 原文
        assert!(
            report
                .warnings
                .iter()
                .all(|w| !w.message.contains("SF_SECRET_CHEN_BADGE_X91")),
            "warning message must not contain raw secret: {:?}",
            report.warnings
        );
        assert!(report.has_errors());
    }

    #[test]
    fn test_private_binding_alone_does_not_error_without_must_not_reveal() {
        use storyforge_domain::narrative_contract::{NarrativeContract, PrivateBinding};
        // 仅 private_bindings、无 must_not_reveal：拥有者/正文出现 secret 不自动 Error
        let contract = NarrativeContract {
            private_bindings: vec![PrivateBinding {
                owner_id: "inst-chen".into(),
                secret: "SF_SECRET_CHEN_BADGE_X91".into(),
            }],
            must_not_reveal: vec![],
            ..Default::default()
        };
        let text = "陈警官在心里默念 SF_SECRET_CHEN_BADGE_X91。窗外雨还在下，急诊灯闪着白光，空气里有消毒水味。";
        let report = run_quality_gate_with_contract(text, Some(&contract));
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::PrivateKnowledgeLeak { .. })),
            "private_bindings alone must not Error: {:?}",
            report.warnings
        );
    }

    #[test]
    fn test_private_knowledge_absent_passes_contract() {
        use storyforge_domain::narrative_contract::{NarrativeContract, PrivateBinding};
        let contract = NarrativeContract {
            private_bindings: vec![PrivateBinding {
                owner_id: "inst-chen".into(),
                secret: "SF_SECRET_CHEN_BADGE_X91".into(),
            }],
            must_not_reveal: vec!["SF_SECRET_CHEN_BADGE_X91".into()],
            ..Default::default()
        };
        let text = "林秋坐在桌前，看着杯中残茶泛起的涟漪。他想起那年冬天，也是这样安静的夜晚。窗外有猫叫，声音远处传来。";
        let report = run_quality_gate_with_contract(text, Some(&contract));
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::PrivateKnowledgeLeak { .. })),
            "无探针不应泄漏: {:?}",
            report.warnings
        );
    }

    #[test]
    fn test_negation_then_affirmation_with_chinese_prefix() {
        // 前缀含多字节中文：find 返回字节偏移，必须从 &text[i..] 扫描
        let text = "夜色渐深，急诊室灯光惨白，林秋不是在害怕，而是在盘算下一步。窗外雨声淅沥，他把报告推到陈警官面前。";
        let report = run_quality_gate(text);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(&w.code, QualityWarningCode::NegationThenAffirmation { .. })),
            "中文前缀下仍应检出否后肯: {:?}",
            report.warnings
        );
    }
}

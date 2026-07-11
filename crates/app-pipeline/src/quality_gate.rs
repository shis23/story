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
use storyforge_domain::turn::{QualityReport, QualitySeverity, QualityWarning, QualityWarningCode};

/// 运行草稿质量门禁
pub fn run_quality_gate(text: &str) -> QualityReport {
    let warnings: Vec<QualityWarning> = vec![
        check_ngram_repetition(text),
        check_meta_description(text),
        check_too_short(text),
        check_perspective_leak(text),
        check_format_leak(text),
        check_consecutive_repeat(text),
    ]
    .into_iter()
    .flatten()
    .collect();

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
}

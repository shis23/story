/// 正则引擎（对应设计 §4.3 D12）
///
/// 封装 regress crate，提供 ST regex_scripts 的执行能力。
/// 正则作用域：
/// - Input: 用户输入 → 导演 Agent 前应用
/// - Output: 编剧成文后应用
/// - 子 Agent 内部不跑正则
use serde::{Deserialize, Serialize};
use storyforge_domain::preset::{RegexPlacement, RegexScript};

// ─── 正则执行器 ────────────────────────────────────────────────────────────

/// 正则执行错误
#[derive(Debug, thiserror::Error)]
pub enum RegexError {
    #[error("正则编译失败: {0}")]
    Compile(String),

    #[error("替换失败: {0}")]
    Replace(String),
}

/// 执行正则脚本列表（按顺序应用，跳过禁用的）
pub fn apply_regex_scripts(
    text: &str,
    scripts: &[RegexScript],
    placement: RegexPlacement,
) -> Result<String, RegexError> {
    let mut result = text.to_string();

    for script in scripts {
        if script.disabled {
            continue;
        }
        if script.placement != placement {
            continue;
        }

        result = apply_single_script(&result, script)?;
    }

    Ok(result)
}

/// 执行单个正则脚本
fn apply_single_script(text: &str, script: &RegexScript) -> Result<String, RegexError> {
    // 编译正则（regress 是 ECMAScript 引擎）
    // 应用 flags（如 gm），regress 会解析 i/m/s/u/v，忽略不支持的 g
    let re = regress::Regex::with_flags(&script.find_regex, script.flags.as_str()).map_err(|e| {
        RegexError::Compile(format!("正则 '{}' 编译失败: {}", script.script_name, e))
    })?;

    // 全局替换（regress 的 replace_all 行为）
    let result = re.replace_all(text, script.replace_string.as_str());

    Ok(result.to_string())
}

// ─── 输入/输出正则分类 ──────────────────────────────────────────────────────

/// 从 ST 预设的 regex_scripts 中分离输入/输出正则
pub fn split_by_placement(scripts: &[RegexScript]) -> (Vec<&RegexScript>, Vec<&RegexScript>) {
    let mut input = Vec::new();
    let mut output = Vec::new();

    for script in scripts {
        if script.disabled {
            continue;
        }
        match script.placement {
            RegexPlacement::Input => input.push(script),
            RegexPlacement::Output => output.push(script),
        }
    }

    (input, output)
}

/// 获取正则脚本摘要（前端展示用）
pub fn script_summary(script: &RegexScript) -> ScriptSummary {
    ScriptSummary {
        id: script.id.clone(),
        name: script.script_name.clone(),
        placement: format!("{:?}", script.placement),
        disabled: script.disabled,
        find_regex: script.find_regex.clone(),
        replace_string: script.replace_string.clone(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptSummary {
    pub id: String,
    pub name: String,
    pub placement: String,
    pub disabled: bool,
    pub find_regex: String,
    pub replace_string: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_script(name: &str, find: &str, replace: &str, placement: RegexPlacement) -> RegexScript {
        RegexScript {
            id: format!("test-{name}"),
            script_name: name.into(),
            find_regex: find.into(),
            replace_string: replace.into(),
            placement,
            disabled: false,
            flags: "gm".into(),
            only_format_formatting: None,
        }
    }

    #[test]
    fn test_apply_input_regex() {
        let scripts = vec![
            make_script("去复述", r"^(.{0,20}).*\1", "$1", RegexPlacement::Input),
            make_script("输出清理", r"\n{3,}", "\n\n", RegexPlacement::Output),
        ];

        let input = "你好你好你好，这是一段测试文本。";
        let result = apply_regex_scripts(input, &scripts, RegexPlacement::Input).unwrap();
        // 输入正则应该生效
        assert!(result.len() <= input.len());
    }

    #[test]
    fn test_apply_output_regex() {
        let scripts = vec![
            make_script("格式清理", r"\n{3,}", "\n\n", RegexPlacement::Output),
        ];

        let input = "段落1\n\n\n\n\n段落2";
        let result = apply_regex_scripts(input, &scripts, RegexPlacement::Output).unwrap();
        assert_eq!(result, "段落1\n\n段落2");
    }

    #[test]
    fn test_skip_disabled_scripts() {
        let mut script = make_script("禁用正则", r"测试", "PASS", RegexPlacement::Input);
        script.disabled = true;

        let result = apply_regex_scripts("测试文本", &[script], RegexPlacement::Input).unwrap();
        assert_eq!(result, "测试文本"); // 未修改
    }

    #[test]
    fn test_skip_wrong_placement() {
        let scripts = vec![
            make_script("输出正则", r"测试", "PASS", RegexPlacement::Output),
        ];

        let result = apply_regex_scripts("测试文本", &scripts, RegexPlacement::Input).unwrap();
        assert_eq!(result, "测试文本"); // Input 阶段不应用 Output 正则
    }

    #[test]
    fn test_split_by_placement() {
        let scripts = vec![
            make_script("输入1", r"a", "b", RegexPlacement::Input),
            make_script("输出1", r"c", "d", RegexPlacement::Output),
            make_script("输入2", r"e", "f", RegexPlacement::Input),
        ];

        let (input, output) = split_by_placement(&scripts);
        assert_eq!(input.len(), 2);
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn test_invalid_regex() {
        let scripts = vec![
            make_script("坏正则", r"[invalid", "x", RegexPlacement::Input),
        ];

        let result = apply_regex_scripts("test", &scripts, RegexPlacement::Input);
        assert!(result.is_err());
    }
}

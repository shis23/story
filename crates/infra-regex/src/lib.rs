/// Regex engine (design section 4.3 D12).
///
/// Wraps the regress crate to execute ST regex_scripts.
/// Scope:
/// - Input: applied before the director agent
/// - Output: applied after the writer finalizes
/// - sub-agents do not run regex
use serde::{Deserialize, Serialize};
use storyforge_domain::preset::{RegexPlacement, RegexScript};

// --- regex executor --------------------------------------------------------

/// Regex execution error
#[derive(Debug, thiserror::Error)]
pub enum RegexError {
    #[error("正则编译失败: {0}")]
    Compile(String),

    #[error("替换失败: {0}")]
    Replace(String),
}

/// Apply a list of regex scripts in order (skipping disabled ones)
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
        if !script_applies_to_placement(script, &placement) {
            continue;
        }

        result = apply_single_script(&result, script)?;
    }

    Ok(result)
}

/// Apply a single regex script.
///
/// H-8 ReDoS mitigation: regress is a backtracking engine; regexes imported from ST
/// presets can be catastrophic (e.g. `^(a+)+$`). We cap input length to bound the
/// worst-case backtracking cost (which scales with input length).
fn apply_single_script(text: &str, script: &RegexScript) -> Result<String, RegexError> {
    const MAX_REGEX_INPUT_LEN: usize = 1024 * 1024; // 1MB
    if text.len() > MAX_REGEX_INPUT_LEN {
        return Err(RegexError::Compile(format!(
            "regex input too long: {} bytes (max {}), possible ReDoS",
            text.len(),
            MAX_REGEX_INPUT_LEN
        )));
    }
    let regex_spec = parse_st_regex_spec(&script.find_regex, &script.flags);
    // Compile (regress is an ECMAScript engine).
    // Apply flags (e.g. gm); regress parses i/m/s/u/v and ignores unsupported g.
    let re = regress::Regex::with_flags(regex_spec.pattern.as_str(), regex_spec.flags.as_str())
        .map_err(|e| {
            RegexError::Compile(format!("正则 '{}' 编译失败: {}", script.script_name, e))
        })?;

    // Global replace (regress replace_all semantics)
    let result = re.replace_all(text, script.replace_string.as_str());

    Ok(result.to_string())
}

#[derive(Debug, PartialEq, Eq)]
struct RegexSpec {
    pattern: String,
    flags: String,
}

fn parse_st_regex_spec(find_regex: &str, flags: &str) -> RegexSpec {
    if let Some((pattern, inline_flags)) = split_st_regex_literal(find_regex) {
        return RegexSpec {
            pattern,
            flags: merge_flags(&inline_flags, flags),
        };
    }

    RegexSpec {
        pattern: find_regex.to_string(),
        flags: flags.to_string(),
    }
}

fn split_st_regex_literal(find_regex: &str) -> Option<(String, String)> {
    if !find_regex.starts_with('/') {
        return None;
    }

    let mut escaped = false;
    let mut in_class = false;

    for (idx, ch) in find_regex.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                let flags = &find_regex[idx + ch.len_utf8()..];
                if !flags.chars().all(|c| c.is_ascii_alphabetic()) {
                    return None;
                }
                return Some((find_regex[1..idx].to_string(), flags.to_string()));
            }
            _ => {}
        }
    }

    None
}

fn merge_flags(inline_flags: &str, field_flags: &str) -> String {
    let mut merged = String::new();
    for flag in inline_flags.chars().chain(field_flags.chars()) {
        if !merged.contains(flag) {
            merged.push(flag);
        }
    }
    merged
}

fn script_applies_to_placement(script: &RegexScript, placement: &RegexPlacement) -> bool {
    if script.placement_codes.is_empty() {
        return script.placement == *placement;
    }

    match placement {
        RegexPlacement::Input => script.placement_codes.contains(&0),
        RegexPlacement::Output => script.placement_codes.contains(&2),
    }
}

// --- input/output regex split ----------------------------------------------

/// Split ST regex_scripts into input/output scripts
pub fn split_by_placement(scripts: &[RegexScript]) -> (Vec<&RegexScript>, Vec<&RegexScript>) {
    let mut input = Vec::new();
    let mut output = Vec::new();

    for script in scripts {
        if script.disabled {
            continue;
        }
        if script_applies_to_placement(script, &RegexPlacement::Input) {
            input.push(script);
        }
        if script_applies_to_placement(script, &RegexPlacement::Output) {
            output.push(script);
        }
    }

    (input, output)
}

/// Build a script summary for frontend display
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

    fn make_script(
        name: &str,
        find: &str,
        replace: &str,
        placement: RegexPlacement,
    ) -> RegexScript {
        RegexScript {
            id: format!("test-{name}"),
            script_name: name.into(),
            find_regex: find.into(),
            replace_string: replace.into(),
            placement,
            placement_codes: vec![],
            source: storyforge_domain::preset::RegexScriptSource::Preset,
            disabled: false,
            flags: "gm".into(),
            only_format_formatting: None,
            markdown_only: None,
            prompt_only: None,
            run_on_edit: None,
            substitute_regex: None,
            trim_strings: vec![],
            min_depth: None,
            max_depth: None,
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
        assert!(result.len() <= input.len());
    }

    #[test]
    fn test_apply_output_regex() {
        let scripts = vec![make_script(
            "格式清理",
            r"\n{3,}",
            "\n\n",
            RegexPlacement::Output,
        )];

        let input = "段落1\n\n\n\n\n段落2";
        let result = apply_regex_scripts(input, &scripts, RegexPlacement::Output).unwrap();
        assert_eq!(result, "段落1\n\n段落2");
    }

    #[test]
    fn test_apply_slash_delimited_st_regex_uses_inline_flags() {
        let mut script = make_script("st-literal", r"/^foo/gm", "bar", RegexPlacement::Output);
        script.flags.clear();

        let result = apply_regex_scripts("foo\nnope\nfoo", &[script], RegexPlacement::Output)
            .expect("slash-delimited ST regex should compile");

        assert_eq!(result, "bar\nnope\nbar");
    }

    #[test]
    fn test_apply_raw_regex_still_uses_flags_field() {
        let script = make_script("raw", r"^foo", "bar", RegexPlacement::Output);

        let result = apply_regex_scripts("foo\nnope\nfoo", &[script], RegexPlacement::Output)
            .expect("raw regex should compile");

        assert_eq!(result, "bar\nnope\nbar");
    }

    #[test]
    fn test_parse_st_regex_literal_preserves_escaped_slash_and_merges_flags() {
        let spec = parse_st_regex_spec(r"/https:\/\/example\.com/gi", "im");

        assert_eq!(spec.pattern, r"https:\/\/example\.com");
        assert_eq!(spec.flags, "gim");
    }

    #[test]
    fn test_skip_disabled_scripts() {
        let mut script = make_script("禁用正则", r"测试", "PASS", RegexPlacement::Input);
        script.disabled = true;

        let result = apply_regex_scripts("测试文本", &[script], RegexPlacement::Input).unwrap();
        assert_eq!(result, "测试文本");
    }

    #[test]
    fn test_skip_wrong_placement() {
        let scripts = vec![make_script(
            "输出正则",
            r"测试",
            "PASS",
            RegexPlacement::Output,
        )];

        let result = apply_regex_scripts("测试文本", &scripts, RegexPlacement::Input).unwrap();
        assert_eq!(result, "测试文本");
    }

    #[test]
    fn test_st_multi_placement_codes_apply_to_input_and_output() {
        let mut script = make_script("both", r"foo", "bar", RegexPlacement::Input);
        script.placement_codes = vec![0, 2];

        let input = apply_regex_scripts("foo", &[script.clone()], RegexPlacement::Input).unwrap();
        let output = apply_regex_scripts("foo", &[script], RegexPlacement::Output).unwrap();

        assert_eq!(input, "bar");
        assert_eq!(output, "bar");
    }

    #[test]
    fn test_unsupported_st_placement_code_does_not_fall_back_to_input() {
        let mut script = make_script("world-info", r"foo", "bar", RegexPlacement::Input);
        script.placement_codes = vec![1];

        let result = apply_regex_scripts("foo", &[script], RegexPlacement::Input).unwrap();

        assert_eq!(result, "foo");
    }

    #[test]
    fn test_split_by_placement() {
        let mut both = make_script("both", r"x", "y", RegexPlacement::Input);
        both.placement_codes = vec![0, 2];
        let scripts = vec![
            make_script("输入1", r"a", "b", RegexPlacement::Input),
            make_script("输出1", r"c", "d", RegexPlacement::Output),
            make_script("输入2", r"e", "f", RegexPlacement::Input),
            both,
        ];

        let (input, output) = split_by_placement(&scripts);
        assert_eq!(input.len(), 3);
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn test_invalid_regex() {
        let scripts = vec![make_script(
            "坏正则",
            r"[invalid",
            "x",
            RegexPlacement::Input,
        )];

        let result = apply_regex_scripts("test", &scripts, RegexPlacement::Input);
        assert!(result.is_err());
    }

    /// H-8: oversize input must be rejected to bound ReDoS backtracking cost.
    #[test]
    fn test_oversize_input_rejected() {
        let scripts = vec![make_script("大输入", r"a", "b", RegexPlacement::Input)];
        let huge = "a".repeat(2 * 1024 * 1024); // 2MB > 1MB limit
        let result = apply_regex_scripts(&huge, &scripts, RegexPlacement::Input);
        assert!(result.is_err());
    }
}

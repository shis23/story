/// Regex engine (design section 4.3 D12).
///
/// Wraps the regress crate to execute ST regex_scripts.
/// Scope:
/// - Input: applied before the director agent
/// - Output: applied after the writer finalizes
/// - sub-agents do not run regex
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use storyforge_domain::preset::{
    RegexPlacement, RegexScript, ST_REGEX_PLACEMENT_AI_OUTPUT, ST_REGEX_PLACEMENT_REASONING,
    ST_REGEX_PLACEMENT_SLASH_COMMAND, ST_REGEX_PLACEMENT_USER_INPUT, ST_REGEX_PLACEMENT_WORLD_INFO,
};

// --- regex executor --------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegexExecutionTarget {
    Prompt,
    Persisted,
    Display,
}

/// Regex execution error
#[derive(Debug, thiserror::Error)]
pub enum RegexError {
    #[error("正则编译失败: {0}")]
    Compile(String),

    #[error("替换失败: {0}")]
    Replace(String),

    /// H-1: regress exceeded the wall-clock budget (catastrophic backtracking).
    #[error("正则执行超时（{timeout}s）：可能为灾难性回溯，script={script}")]
    Timeout { script: String, timeout: u64 },
}

/// H-1: regress (backtracking engine) wall-clock budget per `apply_single_script`.
///
/// 1MB input cap (see `MAX_REGEX_INPUT_LEN`) bounds backtracking cost linearly in
/// input length but a malicious/buggy regex (e.g. `^(a+)+$`) can still be polynomial
/// or exponential in pattern structure. This timeout is the hard backstop that turns
/// a multi-minute UI freeze into a 5s error.
pub const REGEX_TIMEOUT_SECS: u64 = 5;

/// H-1 补强（2026-09-01 全量审查）：进程级「已超时 (pattern, flags)」集合。
/// 超时后被 detach 的 worker 线程仍在 CPU 上跑指数回溯（无法杀死），同一
/// 灾难性脚本每条消息再 apply 一次就再泄漏一个满载线程。首次超时后同 spec
/// 直接短路返回 Timeout；进程重启即复位，给用户修复卡/preset 的机会。
static TIMED_OUT_SPECS: LazyLock<Mutex<HashSet<(String, String)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// H-1: trial-compile an ST regex script's `find_regex` + `flags`.
///
/// Imported presets/character cards/ST settings JSON can carry a malformed or
/// catastrophic regex. Compiling at import time rejects unparseable patterns early
/// (before they freeze the writing pipeline) and gives the user an actionable error
/// naming the offending script. Note: compilation success does NOT prove the regex
/// is non-catastrophic at runtime — `apply_single_script`'s timeout covers that case.
pub fn validate_regex(find_regex: &str, flags: &str) -> Result<(), RegexError> {
    let spec = parse_st_regex_spec(find_regex, flags);
    regress::Regex::with_flags(spec.pattern.as_str(), spec.flags.as_str())
        .map_err(|e| RegexError::Compile(format!("正则编译失败: {e}")))?;
    Ok(())
}

/// Apply a list of regex scripts in order (skipping disabled ones)
pub fn apply_regex_scripts(
    text: &str,
    scripts: &[RegexScript],
    placement: RegexPlacement,
) -> Result<String, RegexError> {
    apply_regex_scripts_for_target(text, scripts, placement, RegexExecutionTarget::Persisted)
}

/// Apply regex scripts for a concrete execution target.
///
/// ST exposes ephemerality switches such as `promptOnly` and `markdownOnly`.
/// StoryForge currently has prompt and persisted storage paths; display-only
/// rendering can call this with `Display` once a renderer hook exists.
pub fn apply_regex_scripts_for_target(
    text: &str,
    scripts: &[RegexScript],
    placement: RegexPlacement,
    target: RegexExecutionTarget,
) -> Result<String, RegexError> {
    apply_regex_scripts_for_target_at_depth(text, scripts, placement, target, 0)
}

/// Apply regex scripts for a concrete execution target and chat depth.
///
/// ST depth is counted from the newest chat message: depth 0 is the current /
/// latest message, depth 1 is one message older, and so on.
pub fn apply_regex_scripts_for_target_at_depth(
    text: &str,
    scripts: &[RegexScript],
    placement: RegexPlacement,
    target: RegexExecutionTarget,
    depth: usize,
) -> Result<String, RegexError> {
    let mut result = text.to_string();

    for script in scripts {
        if script.disabled {
            continue;
        }
        if !script_applies_to_placement(script, &placement) {
            continue;
        }
        if !script_applies_to_target(script, target) {
            continue;
        }
        if !script_applies_to_depth(script, depth) {
            continue;
        }

        result = apply_single_script(&result, script)?;
    }

    Ok(result)
}

pub fn apply_reasoning_regex_to_think_blocks_at_depth(
    text: &str,
    scripts: &[RegexScript],
    target: RegexExecutionTarget,
    depth: usize,
) -> Result<String, RegexError> {
    apply_regex_to_tagged_blocks(
        text,
        scripts,
        RegexPlacement::Reasoning,
        target,
        depth,
        &[
            ("think", "<think>", "</think>"),
            ("thinking", "<thinking>", "</thinking>"),
        ],
    )
}

/// Apply a single regex script.
///
/// H-1 ReDoS mitigation: regress is a backtracking engine; regexes imported from ST
/// presets can be catastrophic (e.g. `^(a+)+$`). Two layered defenses:
/// 1. `MAX_REGEX_INPUT_LEN` (1MB) bounds the worst-case backtracking cost, which
///    scales with input length.
/// 2. `REGEX_TIMEOUT_SECS` wall-clock budget via a blocking worker thread + timed
///    `join`. regress is sync and CPU-bound, so a thread is the only way to bound
///    runtime without an async runtime. On timeout the worker is detached (its
///    result discarded) and we return `RegexError::Timeout`.
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
    let spec_key = (regex_spec.pattern.clone(), regex_spec.flags.clone());
    let script_name = script.script_name.clone();
    {
        let timed_out = TIMED_OUT_SPECS.lock().unwrap_or_else(|p| p.into_inner());
        if timed_out.contains(&spec_key) {
            return Err(RegexError::Timeout {
                script: script_name,
                timeout: REGEX_TIMEOUT_SECS,
            });
        }
    }

    // The worker thread requires `'static` inputs, so clone the owned pieces once
    // and move them in. `text` is capped at 1MB so this clone is bounded.
    let thread_name = script_name.clone();
    let text_owned = text.to_string();
    let pattern_owned = regex_spec.pattern.clone();
    let flags_owned = regex_spec.flags.clone();
    let replace_owned = script.replace_string.clone();
    let is_global = regex_spec.flags.contains('g');

    let (result_tx, result_rx) = std::sync::mpsc::channel::<Result<String, RegexError>>();
    let worker = move || {
        let outcome = (|| -> Result<String, RegexError> {
            let re = regress::Regex::with_flags(pattern_owned.as_str(), flags_owned.as_str())
                .map_err(|e| {
                    RegexError::Compile(format!("正则 '{}' 编译失败: {}", script_name, e))
                })?;
            let result = if is_global {
                re.replace_all(&text_owned, &replace_owned)
            } else {
                re.replace(&text_owned, &replace_owned)
            };
            Ok(result.to_string())
        })();
        // Ignore send error: parent timed out and dropped the receiver.
        let _ = result_tx.send(outcome);
    };

    // Spawn the worker; on timeout recv_timeout returns Timeout and we detach the
    // worker (its result is dropped via the ignored send). The OS thread keeps
    // running regress until it finishes or the process exits — bounded by the 1MB
    // input cap so even pathological cases complete in bounded time/space.
    std::thread::Builder::new()
        .name(format!("sf-regex-{thread_name}"))
        .spawn(worker)
        .map_err(|e| RegexError::Compile(format!("正则工作线程启动失败: {e}")))?;

    match result_rx.recv_timeout(Duration::from_secs(REGEX_TIMEOUT_SECS)) {
        Ok(inner) => inner,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            TIMED_OUT_SPECS
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(spec_key);
            Err(RegexError::Timeout {
                script: thread_name,
                timeout: REGEX_TIMEOUT_SECS,
            })
        }
        // Disconnected without a value = worker panicked before sending. Map to a
        // Compile error so callers see a normal regex failure, not a crash.
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(RegexError::Compile(format!(
            "正则 '{}' 执行线程异常结束（panic 或断连）",
            thread_name
        ))),
    }
}

fn apply_regex_to_tagged_blocks(
    text: &str,
    scripts: &[RegexScript],
    placement: RegexPlacement,
    target: RegexExecutionTarget,
    depth: usize,
    tags: &[(&str, &str, &str)],
) -> Result<String, RegexError> {
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some((open_start, _tag_name, open_tag, close_tag)) =
        find_next_open_tag(text, cursor, tags)
    {
        let inner_start = open_start + open_tag.len();
        let Some(close_start) = find_ascii_case_insensitive(text, close_tag, inner_start) else {
            break;
        };
        let close_end = close_start + close_tag.len();

        result.push_str(&text[cursor..inner_start]);
        let inner = &text[inner_start..close_start];
        result.push_str(&apply_regex_scripts_for_target_at_depth(
            inner,
            scripts,
            placement.clone(),
            target,
            depth,
        )?);
        result.push_str(&text[close_start..close_end]);
        cursor = close_end;
    }

    result.push_str(&text[cursor..]);
    Ok(result)
}

fn find_next_open_tag<'a>(
    text: &str,
    start: usize,
    tags: &'a [(&'a str, &'a str, &'a str)],
) -> Option<(usize, &'a str, &'a str, &'a str)> {
    tags.iter()
        .filter_map(|(name, open, close)| {
            find_ascii_case_insensitive(text, open, start).map(|idx| (idx, *name, *open, *close))
        })
        .min_by_key(|(idx, _, _, _)| *idx)
}

fn find_ascii_case_insensitive(text: &str, needle: &str, start: usize) -> Option<usize> {
    if start >= text.len() {
        return None;
    }

    let haystack = text[start..].to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    haystack.find(&needle).map(|idx| start + idx)
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
        RegexPlacement::Input => script
            .placement_codes
            .contains(&ST_REGEX_PLACEMENT_USER_INPUT),
        RegexPlacement::Output => script
            .placement_codes
            .contains(&ST_REGEX_PLACEMENT_AI_OUTPUT),
        RegexPlacement::SlashCommand => script
            .placement_codes
            .contains(&ST_REGEX_PLACEMENT_SLASH_COMMAND),
        RegexPlacement::WorldInfo => script
            .placement_codes
            .contains(&ST_REGEX_PLACEMENT_WORLD_INFO),
        RegexPlacement::Reasoning => script
            .placement_codes
            .contains(&ST_REGEX_PLACEMENT_REASONING),
    }
}

fn script_applies_to_target(script: &RegexScript, target: RegexExecutionTarget) -> bool {
    if script.prompt_only.unwrap_or(false) {
        return target == RegexExecutionTarget::Prompt;
    }
    if script.markdown_only.unwrap_or(false) {
        return target == RegexExecutionTarget::Display;
    }
    true
}

fn script_applies_to_depth(script: &RegexScript, depth: usize) -> bool {
    let depth = i32::try_from(depth).unwrap_or(i32::MAX);
    let min = script.min_depth.filter(|value| *value >= 0);
    let max = script.max_depth.filter(|value| *value >= 0);

    if let (Some(min), Some(max)) = (min, max)
        && max < min
    {
        return false;
    }
    if let Some(min) = min
        && depth < min
    {
        return false;
    }
    if let Some(max) = max
        && depth > max
    {
        return false;
    }

    true
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
    fn test_non_global_regex_replaces_only_first_match() {
        let mut script = make_script("single", r"foo", "bar", RegexPlacement::Output);
        script.flags.clear();

        let result = apply_regex_scripts("foo foo", &[script], RegexPlacement::Output)
            .expect("non-global regex should compile");

        assert_eq!(result, "bar foo");
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
        script.placement_codes = vec![ST_REGEX_PLACEMENT_USER_INPUT, ST_REGEX_PLACEMENT_AI_OUTPUT];

        let input = apply_regex_scripts("foo", &[script.clone()], RegexPlacement::Input).unwrap();
        let output = apply_regex_scripts("foo", &[script], RegexPlacement::Output).unwrap();

        assert_eq!(input, "bar");
        assert_eq!(output, "bar");
    }

    #[test]
    fn test_current_st_user_input_placement_code_applies_to_input() {
        let mut script = make_script(
            "user-input",
            r"/(.*)/s",
            "<reader-response>$1</reader-response>",
            RegexPlacement::Input,
        );
        script.flags.clear();
        script.placement_codes = vec![ST_REGEX_PLACEMENT_USER_INPUT];
        script.prompt_only = Some(true);

        let input = apply_regex_scripts_for_target(
            "go north",
            &[script.clone()],
            RegexPlacement::Input,
            RegexExecutionTarget::Prompt,
        )
        .unwrap();
        let output = apply_regex_scripts_for_target(
            "go north",
            &[script],
            RegexPlacement::Output,
            RegexExecutionTarget::Prompt,
        )
        .unwrap();

        assert_eq!(input, "<reader-response>go north</reader-response>");
        assert_eq!(output, "go north");
    }

    #[test]
    fn test_reasoning_regex_applies_only_inside_think_blocks() {
        let mut script = make_script("reasoning", r"secret", "hidden", RegexPlacement::Reasoning);
        script.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];
        script.flags = "g".into();

        let result = apply_reasoning_regex_to_think_blocks_at_depth(
            "before secret <think>secret plan</think> after secret",
            &[script],
            RegexExecutionTarget::Persisted,
            0,
        )
        .expect("reasoning regex should compile");

        assert_eq!(
            result,
            "before secret <think>hidden plan</think> after secret"
        );
    }

    #[test]
    fn test_reasoning_regex_supports_markdown_only_display_target() {
        let mut script = make_script(
            "reasoning-display",
            r"raw",
            "pretty",
            RegexPlacement::Reasoning,
        );
        script.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];
        script.markdown_only = Some(true);
        script.flags = "g".into();

        let persisted = apply_reasoning_regex_to_think_blocks_at_depth(
            "<think>raw chain</think>",
            &[script.clone()],
            RegexExecutionTarget::Persisted,
            0,
        )
        .expect("persisted reasoning regex should compile");
        let display = apply_reasoning_regex_to_think_blocks_at_depth(
            "<think>raw chain</think>",
            &[script],
            RegexExecutionTarget::Display,
            0,
        )
        .expect("display reasoning regex should compile");

        assert_eq!(persisted, "<think>raw chain</think>");
        assert_eq!(display, "<think>pretty chain</think>");
    }

    #[test]
    fn test_unclosed_think_block_is_left_unchanged() {
        let mut script = make_script("reasoning", r"secret", "hidden", RegexPlacement::Reasoning);
        script.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];

        let result = apply_reasoning_regex_to_think_blocks_at_depth(
            "before <think>secret",
            &[script],
            RegexExecutionTarget::Persisted,
            0,
        )
        .expect("reasoning regex should compile");

        assert_eq!(result, "before <think>secret");
    }

    #[test]
    fn test_prompt_only_script_applies_only_to_prompt_target() {
        let mut script = make_script("prompt-only", r"secret", "hint", RegexPlacement::Input);
        script.prompt_only = Some(true);

        let prompt = apply_regex_scripts_for_target(
            "secret",
            &[script.clone()],
            RegexPlacement::Input,
            RegexExecutionTarget::Prompt,
        )
        .unwrap();
        let persisted = apply_regex_scripts_for_target(
            "secret",
            &[script],
            RegexPlacement::Input,
            RegexExecutionTarget::Persisted,
        )
        .unwrap();

        assert_eq!(prompt, "hint");
        assert_eq!(persisted, "secret");
    }

    #[test]
    fn test_markdown_only_script_does_not_mutate_prompt_or_persisted_text() {
        let mut script = make_script(
            "display-only",
            r"<data_block>.*</data_block>",
            "STATUS",
            RegexPlacement::Output,
        );
        script.markdown_only = Some(true);

        let display = apply_regex_scripts_for_target(
            "<data_block>hp=5</data_block>",
            &[script.clone()],
            RegexPlacement::Output,
            RegexExecutionTarget::Display,
        )
        .unwrap();
        let prompt = apply_regex_scripts_for_target(
            "<data_block>hp=5</data_block>",
            &[script.clone()],
            RegexPlacement::Output,
            RegexExecutionTarget::Prompt,
        )
        .unwrap();
        let persisted = apply_regex_scripts_for_target(
            "<data_block>hp=5</data_block>",
            &[script],
            RegexPlacement::Output,
            RegexExecutionTarget::Persisted,
        )
        .unwrap();

        assert_eq!(display, "STATUS");
        assert_eq!(prompt, "<data_block>hp=5</data_block>");
        assert_eq!(persisted, "<data_block>hp=5</data_block>");
    }

    #[test]
    fn test_depth_limited_script_applies_only_inside_inclusive_range() {
        let mut script = make_script("depth-limited", r"foo", "bar", RegexPlacement::Output);
        script.min_depth = Some(1);
        script.max_depth = Some(2);

        let recent = apply_regex_scripts_for_target_at_depth(
            "foo",
            &[script.clone()],
            RegexPlacement::Output,
            RegexExecutionTarget::Persisted,
            0,
        )
        .unwrap();
        let min = apply_regex_scripts_for_target_at_depth(
            "foo",
            &[script.clone()],
            RegexPlacement::Output,
            RegexExecutionTarget::Persisted,
            1,
        )
        .unwrap();
        let max = apply_regex_scripts_for_target_at_depth(
            "foo",
            &[script.clone()],
            RegexPlacement::Output,
            RegexExecutionTarget::Persisted,
            2,
        )
        .unwrap();
        let older = apply_regex_scripts_for_target_at_depth(
            "foo",
            &[script],
            RegexPlacement::Output,
            RegexExecutionTarget::Persisted,
            3,
        )
        .unwrap();

        assert_eq!(recent, "foo");
        assert_eq!(min, "bar");
        assert_eq!(max, "bar");
        assert_eq!(older, "foo");
    }

    #[test]
    fn test_negative_depth_bounds_are_unlimited() {
        let mut script = make_script("unlimited-depth", r"foo", "bar", RegexPlacement::Output);
        script.min_depth = Some(-1);
        script.max_depth = Some(-1);

        let result = apply_regex_scripts_for_target_at_depth(
            "foo",
            &[script],
            RegexPlacement::Output,
            RegexExecutionTarget::Persisted,
            99,
        )
        .unwrap();

        assert_eq!(result, "bar");
    }

    #[test]
    fn test_slash_command_placement_code_does_not_fall_back_to_input() {
        let mut script = make_script("slash", r"foo", "bar", RegexPlacement::SlashCommand);
        script.placement_codes = vec![ST_REGEX_PLACEMENT_SLASH_COMMAND];

        let result = apply_regex_scripts("foo", &[script], RegexPlacement::Input).unwrap();

        assert_eq!(result, "foo");
    }

    #[test]
    fn test_world_info_placement_code_applies_only_to_world_info_target() {
        let mut script = make_script("world-info", r"foo", "bar", RegexPlacement::Input);
        script.placement_codes = vec![ST_REGEX_PLACEMENT_WORLD_INFO];

        let input = apply_regex_scripts_for_target_at_depth(
            "foo",
            &[script.clone()],
            RegexPlacement::Input,
            RegexExecutionTarget::Prompt,
            0,
        )
        .unwrap();
        let world_info = apply_regex_scripts_for_target_at_depth(
            "foo",
            &[script],
            RegexPlacement::WorldInfo,
            RegexExecutionTarget::Prompt,
            0,
        )
        .unwrap();

        assert_eq!(input, "foo");
        assert_eq!(world_info, "bar");
    }

    #[test]
    fn test_slash_and_reasoning_placement_codes_are_distinct_targets() {
        let mut slash = make_script("slash", r"foo", "slash", RegexPlacement::SlashCommand);
        slash.placement_codes = vec![ST_REGEX_PLACEMENT_SLASH_COMMAND];
        let mut reasoning = make_script("reasoning", r"foo", "reason", RegexPlacement::Reasoning);
        reasoning.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];

        let scripts = vec![slash.clone(), reasoning.clone()];

        assert_eq!(
            apply_regex_scripts("foo", &scripts, RegexPlacement::SlashCommand).unwrap(),
            "slash"
        );
        assert_eq!(
            apply_regex_scripts("foo", &scripts, RegexPlacement::Reasoning).unwrap(),
            "reason"
        );
        assert_eq!(
            apply_regex_scripts("foo", &scripts, RegexPlacement::WorldInfo).unwrap(),
            "foo"
        );
    }

    #[test]
    fn test_split_by_placement() {
        let mut both = make_script("both", r"x", "y", RegexPlacement::Input);
        both.placement_codes = vec![ST_REGEX_PLACEMENT_USER_INPUT, ST_REGEX_PLACEMENT_AI_OUTPUT];
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

    /// H-1: `validate_regex` accepts a normal pattern and rejects an unparseable one.
    #[test]
    fn test_validate_regex_accepts_and_rejects() {
        assert!(validate_regex(r"\n{3,}", "gm").is_ok());
        assert!(validate_regex(r"[invalid", "gm").is_err());
    }

    /// H-1: a catastrophic regex (`^(a+)+$`) against a long all-'a' input must hit
    /// the wall-clock budget and return `RegexError::Timeout`, not freeze.
    ///
    /// We deliberately shorten the budget for this test by calling the inner logic
    /// indirectly through `apply_single_script` is not feasible (constant is fixed),
    /// so we instead verify the timeout *path* via a regex that genuinely
    /// catastrophically backtracks on a modest input within `REGEX_TIMEOUT_SECS`.
    /// The 1MB cap means even a runaway worker exits eventually; the timeout turns
    /// a multi-minute freeze into at most `REGEX_TIMEOUT_SECS` seconds.
    #[test]
    fn test_redos_catastrophic_regex_times_out() {
        // `^(a+)+$` is the textbook catastrophic backtracking pattern. On a 40KB
        // run of 'a' followed by a non-matching suffix, regress explores an
        // exponential number of partitions and cannot finish within 5s.
        let script = RegexScript {
            id: "redos".into(),
            script_name: "灾难性回溯".into(),
            find_regex: r"^(a+)+$".into(),
            replace_string: String::new(),
            placement: RegexPlacement::Input,
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
        };
        // 40_000 'a' + a 'b' forces non-match → exponential backtracking.
        let bomb = format!("{}b", "a".repeat(40_000));
        let start = std::time::Instant::now();
        let result = apply_single_script(&bomb, &script);
        let elapsed = start.elapsed();
        assert!(
            matches!(result, Err(RegexError::Timeout { .. })),
            "expected Timeout, got {result:?}"
        );
        // Sanity: we returned near the budget, not after minutes.
        assert!(
            elapsed.as_secs() < REGEX_TIMEOUT_SECS + 5,
            "timeout returned too slowly: {elapsed:?}"
        );
    }
}

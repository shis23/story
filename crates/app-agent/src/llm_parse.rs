//! LLM 输出 JSON 提取公共工具（消除 4 处重复实现）。
//!
//! 历史：`character_extractor` / `postprocess` / `mvu_import` / `app-pipeline`
//! 各自实现了逐字节相同的 `match_braces` + 代码块提取 + 5 层兜底编排。本模块
//! 统一这些机械逻辑；各调用方保留自己的 DTO 解析与特殊后处理（如 mvu_import
//! 的 wrapper 解包、character_extractor 的数组形态）。
//!
//! 5 层兜底（应对真实模型不可控的输出风格）：
//! 1. tool_calls 中指定工具调用的 arguments
//! 2. 整个 content 是合法 JSON
//! 3. ```json ... ``` 代码块
//! 4. ``` ... ``` 裸代码块
//! 5. 大括号配平：从 content 中逐个 `{` 尝试配平，返回首个能解析成功的块

use storyforge_domain::llm::ChatResponse;

/// 从 pos 位置的 `{` 开始，找配平的 `}` byte index（处理字符串转义）。
///
/// 返回 `}` 的 byte index。若中途括号不匹配（如未闭合），返回 None。
/// 比 regress 正则的贪心匹配更可靠（regress 对多字节字符的 range 可能有坑）。
pub fn match_braces(content: &str, pos: usize) -> Option<usize> {
    let chars: Vec<char> = content[pos..].chars().collect();
    if chars.is_empty() || chars[0] != '{' {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut byte_offset = pos;

    for ch in chars {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(byte_offset);
                    }
                }
                _ => {}
            }
        }
        byte_offset += ch.len_utf8();
    }
    None // 未闭合
}

/// 提取指定语言代码块的内容（第一个匹配的 ````{lang} ... ````）。
///
/// `lang` 为空表示无语言标签的裸代码块（```` ... ````）。
/// 返回 trim 后的代码块文本。
pub fn extract_codeblock(content: &str, lang: &str) -> Option<String> {
    let fence = if lang.is_empty() {
        "```".to_string()
    } else {
        format!("```{lang}")
    };
    let start = content.find(&fence)?;
    let after_fence = &content[start + fence.len()..];
    let end = after_fence.find("```")?;
    Some(after_fence[..end].trim().to_string())
}

/// 从 content 逐个 `{` 尝试配平，返回首个配平成功的 `{...}` 子串。
///
/// 比"只试第一个 `{`"更健壮：模型输出「好的，结果：{错误对象}{正确对象}」时，
/// 能跳过第一个无法解析的块继续尝试。`retry=true` 遍历每个 `{` 起点；
/// `retry=false` 仅尝试第一个。
pub fn extract_first_braces(content: &str, retry: bool) -> Option<String> {
    let mut start_idx = 0;
    while start_idx < content.len() {
        let rel = content[start_idx..].find('{')?;
        let brace_start = start_idx + rel;
        if let Some(end) = match_braces(content, brace_start) {
            return Some(content[brace_start..=end].to_string());
        }
        if !retry {
            return None;
        }
        start_idx = brace_start + 1;
    }
    None
}

/// 从 content 逐个 `{` 尝试配平，对每个候选块调用 `accept`，返回首个成功解析的 T。
///
/// 用于"层 5"：从每个 `{` 开始配平，把候选 `{...}` 字符串交给调用方解析。
/// 比单纯提取字符串更灵活（调用方可对每个候选做 serde 解析 + 后处理）。
pub fn try_each_braces<T>(content: &str, accept: impl Fn(&str) -> Option<T>) -> Option<T> {
    let mut start_idx = 0;
    while start_idx < content.len() {
        let rel = content[start_idx..].find('{')?;
        let brace_start = start_idx + rel;
        if let Some(end) = match_braces(content, brace_start) {
            let candidate = &content[brace_start..=end];
            if let Some(t) = accept(candidate) {
                return Some(t);
            }
        }
        start_idx = brace_start + 1;
    }
    None
}

/// 5 层兜底编排（层 1 由调用方自行处理 tool_call 名差异，这里负责层 2-5）。
///
/// `parse` 把提取出的 JSON 文本转成目标类型；返回首个成功的结果。
///
/// 典型用法（调用方先做层 1 的 tool_call 匹配，剩余走本函数）：
/// ```ignore
/// let content = resp.content.trim();
/// if !content.is_empty() {
///     if let Some(t) = parse_from_content(content, |s| serde_json::from_str(s).ok().map(...)) {
///         return Ok(t);
///     }
/// }
/// ```
pub fn parse_from_content<T>(content: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    // 层 2：整个 content 是 JSON
    if let Some(t) = parse(content) {
        return Some(t);
    }
    // 层 3：```json 代码块
    if let Some(extracted) = extract_codeblock(content, "json") {
        if let Some(t) = parse(&extracted) {
            return Some(t);
        }
    }
    // 层 4：裸代码块
    if let Some(extracted) = extract_codeblock(content, "") {
        if let Some(t) = parse(&extracted) {
            return Some(t);
        }
    }
    // 层 5：括号配平（逐个 `{` 尝试，调用方解析每个候选）
    try_each_braces(content, |candidate| parse(candidate))
}

/// 从 LLM 响应的 tool_calls 里找指定工具，把其 arguments 解析为目标类型。
///
/// 层 1 辅助：遍历 `resp.tool_calls`，对每个名为 `tool_name` 的调用，
/// 把 `arguments`（JSON 字符串）交给 `parse`。
pub fn from_tool_call<T>(
    resp: &ChatResponse,
    tool_name: &str,
    parse: impl Fn(&serde_json::Value) -> Option<T>,
) -> Option<T> {
    for tc in &resp.tool_calls {
        if tc.function.name == tool_name {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
                if let Some(t) = parse(&args) {
                    return Some(t);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_braces_balanced() {
        let s = "prefix {\"a\": {\"b\": 1}} suffix";
        let open = s.find('{').unwrap();
        let close = match_braces(s, open).unwrap();
        assert_eq!(&s[open..=close], "{\"a\": {\"b\": 1}}");
    }

    #[test]
    fn match_braces_string_with_brace() {
        // 字符串内的 { 不应影响配平
        let s = "{\"a\": \"}\"}";
        let open = s.find('{').unwrap();
        let close = match_braces(s, open).unwrap();
        assert_eq!(&s[open..=close], s);
    }

    #[test]
    fn match_braces_escaped_quote() {
        let s = "{\"a\": \"\\\"\"}";
        let open = s.find('{').unwrap();
        let close = match_braces(s, open).unwrap();
        assert_eq!(&s[open..=close], s);
    }

    #[test]
    fn match_braces_unclosed() {
        let s = "{\"a\": 1";
        let open = s.find('{').unwrap();
        assert!(match_braces(s, open).is_none());
    }

    #[test]
    fn extract_codeblock_json() {
        let s = "结果：\n```json\n{\"x\": 1}\n```\n结束";
        assert_eq!(extract_codeblock(s, "json").unwrap(), "{\"x\": 1}");
    }

    #[test]
    fn extract_codeblock_plain() {
        let s = "```\n{\"x\": 1}\n```";
        assert_eq!(extract_codeblock(s, "").unwrap(), "{\"x\": 1}");
    }

    #[test]
    fn extract_first_braces_skips_bad() {
        // 第一个 { 配平失败（对象内含未闭合），应跳过找下一个
        let s = "垃圾 { 不完整 {\"ok\": true}";
        let got = extract_first_braces(s, true).unwrap();
        assert_eq!(got, "{\"ok\": true}");
    }

    #[test]
    fn extract_first_braces_no_retry() {
        // retry=false：只试第一个 {，配平失败就返回 None（不跳到下一个 {）
        // 这个字符串只有 1 个 }，第一个 { 配平失败（depth 卡在 1）
        let s = "{bad {\"ok\": true}";
        assert!(extract_first_braces(s, false).is_none());
        // 配平成功的简单情况
        let s2 = "{\"ok\": true} trailing {\"ignored\": 1}";
        let got = extract_first_braces(s2, false).unwrap();
        assert_eq!(got, "{\"ok\": true}");
    }

    #[test]
    fn try_each_braces_returns_first_parsable() {
        let s = "前缀 {\"bad\": 1, } {\"good\": 2}";
        let result = try_each_braces(s, |candidate| {
            serde_json::from_str::<serde_json::Value>(candidate)
                .ok()
                .and_then(|v| v.get("good").and_then(|g| g.as_i64()))
        });
        assert_eq!(result, Some(2));
    }

    #[test]
    fn parse_from_content_layer2_whole() {
        let s = "{\"k\": 5}";
        let r = parse_from_content(s, |c| serde_json::from_str::<serde_json::Value>(c).ok());
        assert!(r.is_some());
    }

    #[test]
    fn parse_from_content_layer5_braces() {
        let s = "好的，结果如下：前缀文字 {\"k\": 9} 后缀";
        let r = parse_from_content(s, |c| {
            serde_json::from_str::<serde_json::Value>(c)
                .ok()
                .and_then(|v| v.get("k").and_then(|k| k.as_i64()))
        });
        assert_eq!(r, Some(9));
    }
}

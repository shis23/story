/// XML/JSON 降级工具协议（兼容不原生支持 function calling 的模型）
///
/// 工作方式：
/// 1. 注入：把工具定义格式化为文本描述，追加到 system prompt 末尾
/// 2. 提取：从模型输出中用正则匹配 <tool_call>...</tool_call> 或 ```json ... ``` 格式
use std::sync::LazyLock;

use storyforge_domain::llm::{ChatMessage, FunctionCall, ToolCall, ToolSpec};

/// 编译一次的正则（避免每次调用重新编译）
static RE_XML: LazyLock<regress::Regex> = LazyLock::new(|| {
    regress::Regex::new(r"<tool_call>([\s\S]*?)</tool_call>").expect("正则编译失败")
});
static RE_JSON: LazyLock<regress::Regex> =
    LazyLock::new(|| regress::Regex::new(r"```json([\s\S]*?)```").expect("正则编译失败"));

/// 将工具定义注入到 system prompt（在最后一条 system 消息末尾追加）
pub fn inject_tool_prompt(messages: &mut [ChatMessage], tools: &[ToolSpec]) {
    if tools.is_empty() {
        return;
    }

    let tool_text = format_tool_descriptions(tools);

    // 找最后一条 system 消息追加
    if let Some(sys_msg) = messages
        .iter_mut()
        .rfind(|m| m.role == storyforge_domain::llm::ChatRole::System)
    {
        sys_msg.content.push_str("\n\n");
        sys_msg.content.push_str(&tool_text);
    }
}

/// 格式化工具定义为自然语言描述（给模型看的）
fn format_tool_descriptions(tools: &[ToolSpec]) -> String {
    let mut out =
        String::from("## 可用工具\n\n你可以调用以下工具。调用时请严格使用 <tool_call> 格式：\n\n");

    for tool in tools {
        out.push_str(&format!("### {}\n", tool.function.name));
        out.push_str(&format!("{}\n", tool.function.description));

        // 参数 schema（简化展示）
        if let Some(props) = tool.function.parameters.get("properties") {
            out.push_str("参数：\n");
            if let Some(obj) = props.as_object() {
                for (name, schema) in obj {
                    let desc = schema
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let typ = schema
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("string");
                    out.push_str(&format!("  - `{name}` ({typ}): {desc}\n"));
                }
            }
        }
        out.push('\n');
    }

    out.push_str("## 调用格式\n\n当你需要调用工具时，请输出以下格式（不要输出其他内容）：\n\n");
    out.push_str(
        "<tool_call>\n{\"name\": \"工具名\", \"arguments\": {参数JSON}}\n</tool_call>\n\n",
    );
    out.push_str("可以一次调用多个工具：\n\n");
    out.push_str("<tool_call>\n{\"name\": \"工具1\", \"arguments\": {}}\n</tool_call>\n<tool_call>\n{\"name\": \"工具2\", \"arguments\": {}}\n</tool_call>\n");

    out
}

/// 从模型输出文本中提取工具调用
///
/// 支持两种格式：
/// 1. <tool_call>{"name":"x","arguments":{...}}</tool_call>
/// 2. ```json\n{"name":"x","arguments":{...}}\n```
pub fn parse_tool_calls_from_text(text: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();

    // 格式 1: <tool_call>...</tool_call>（捕获标签内全部内容，直接 JSON 解析）
    for m in RE_XML.find_iter(text) {
        let json_str = m.group(1).and_then(|g| text.get(g)).unwrap_or("").trim();
        if let Some(call) = parse_single_tool_call(json_str) {
            calls.push(call);
        }
    }

    // 格式 2: ```json ... ```（只在没匹配到格式 1 时尝试）
    if calls.is_empty() {
        for m in RE_JSON.find_iter(text) {
            let json_str = m.group(1).and_then(|g| text.get(g)).unwrap_or("").trim();
            if let Some(call) = parse_single_tool_call(json_str) {
                calls.push(call);
            }
        }
    }

    calls
}

/// 解析单个工具调用 JSON
fn parse_single_tool_call(json_str: &str) -> Option<ToolCall> {
    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let arguments = v
        .get("arguments")
        .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "{}".into()))
        .unwrap_or_else(|| "{}".into());

    Some(ToolCall {
        id: format!("call_text_{}", uuid::Uuid::new_v4()),
        call_type: "function".into(),
        function: FunctionCall { name, arguments },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_tool() -> ToolSpec {
        ToolSpec::function(
            "get_character",
            "获取角色卡详情",
            json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "角色卡 ID"}
                }
            }),
        )
    }

    #[test]
    fn test_inject_tool_prompt() {
        let mut messages = vec![
            ChatMessage::system("你是导演"),
            ChatMessage::user("写一场戏"),
        ];
        inject_tool_prompt(&mut messages, &[make_tool()]);

        assert!(messages[0].content.contains("可用工具"));
        assert!(messages[0].content.contains("get_character"));
        assert!(messages[0].content.contains("<tool_call>"));
        // user 消息不变
        assert_eq!(messages[1].content, "写一场戏");
    }

    #[test]
    fn test_parse_xml_tool_call() {
        let text = r#"我来查一下角色。

<tool_call>
{"name": "get_character", "arguments": {"id": "abc123"}}
</tool_call>"#;

        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_character");
        assert!(calls[0].function.arguments.contains("abc123"));
    }

    #[test]
    fn test_parse_multiple_xml_tool_calls() {
        let text = r#"<tool_call>
{"name": "search_world_info", "arguments": {"query": "龙"}}
</tool_call><tool_call>
{"name": "get_character", "arguments": {"id": "xyz"}}
</tool_call>"#;

        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "search_world_info");
        assert_eq!(calls[1].function.name, "get_character");
    }

    #[test]
    fn test_parse_json_codeblock_tool_call() {
        let text = r#"好的，让我查一下。

```json
{"name": "emit_plan", "arguments": {"scene_brief": "雨中告别", "subagent_tasks": []}}
```"#;

        let calls = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "emit_plan");
    }

    #[test]
    fn test_no_tool_call() {
        let text = "这是一段普通文本，没有工具调用。";
        let calls = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
    }
}

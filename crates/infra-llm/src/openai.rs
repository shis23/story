/// OpenAI 兼容协议层
///
/// 负责将内部 ChatRequest 转为 OpenAI JSON 格式，以及将 OpenAI 响应转回 ChatResponse。
/// DeepSeek、Groq、Moonshot、OpenRouter 等都走此路径。
use storyforge_domain::llm::{ChatMessage, ChatRequest, ChatResponse, ToolCall, Usage};

/// 构建 OpenAI 兼容的请求 JSON
pub fn build_request_body(req: &ChatRequest) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": req.model,
        "messages": build_messages(&req.messages),
        "stream": false,
    });

    // 采样参数
    if let Some(temp) = req.params.temperature {
        body["temperature"] = serde_json::json!(temp);
    }
    if let Some(top_p) = req.params.top_p {
        body["top_p"] = serde_json::json!(top_p);
    }
    if let Some(max_tokens) = req.params.max_tokens {
        body["max_tokens"] = serde_json::json!(max_tokens);
    }

    // P3-3：厂商扩展参数透传到请求体顶层（thinking/reasoning_effort 等）
    if let Some(extra) = &req.params.extra {
        for (key, value) in extra {
            body[key] = value.clone();
        }
    }

    // 工具定义
    if let Some(tools) = &req.tools
        && !tools.is_empty()
    {
        body["tools"] = serde_json::json!(tools);
    }

    body
}

/// 构建流式请求 JSON（stream=true）
///
/// 设置 stream_options.include_usage=true，让服务端在最后一个 chunk 返回真实 token 统计
/// （OpenAI 标准协议）。历史版本注释"流式时无法精确获取 usage"是因为未设此项。
pub fn build_stream_request_body(req: &ChatRequest) -> serde_json::Value {
    let mut body = build_request_body(req);
    body["stream"] = serde_json::json!(true);
    body["stream_options"] = serde_json::json!({ "include_usage": true });
    body
}

/// 将内部消息转为 OpenAI 格式
fn build_messages(messages: &[ChatMessage]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut obj = serde_json::json!({
                "role": serde_json::to_value(&m.role).expect("chat role should serialize to JSON"),
                "content": m.content,
            });

            // 工具调用
            if let Some(tool_calls) = &m.tool_calls {
                obj["tool_calls"] = serde_json::json!(tool_calls);
            }
            // 工具结果
            if let Some(tool_call_id) = &m.tool_call_id {
                obj["tool_call_id"] = serde_json::json!(tool_call_id);
            }

            obj
        })
        .collect();

    serde_json::json!(arr)
}

/// 解析 OpenAI 兼容的完整响应 JSON
pub fn parse_response(body: &serde_json::Value) -> Result<ChatResponse, String> {
    let choices = body
        .get("choices")
        .and_then(|v| v.as_array())
        .ok_or("响应缺少 choices 数组")?;

    let first = choices.first().ok_or("choices 为空")?;

    let message = first.get("message").ok_or("choice 缺少 message")?;

    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let tool_calls: Vec<ToolCall> = match message.get("tool_calls") {
        Some(v) => serde_json::from_value(v.clone()).unwrap_or_else(|e| {
            tracing::warn!(target: "openai", "tool_calls 反序列化失败，已忽略: {e}");
            Vec::new()
        }),
        None => Vec::new(),
    };

    let finish_reason = first
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .map(String::from);

    let usage = body.get("usage").and_then(|v| {
        Some(Usage {
            prompt_tokens: v.get("prompt_tokens")?.as_u64()? as u32,
            completion_tokens: v.get("completion_tokens")?.as_u64()? as u32,
            total_tokens: v.get("total_tokens")?.as_u64()? as u32,
        })
    });

    Ok(ChatResponse {
        content,
        tool_calls,
        finish_reason,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::{ChatMessage, SamplingParams, ToolSpec};

    #[test]
    fn test_build_request_body_basic() {
        let req = ChatRequest {
            messages: vec![
                ChatMessage::system("你是导演"),
                ChatMessage::user("写一场戏"),
            ],
            tools: None,
            params: SamplingParams::default(),
            model: "deepseek-chat".into(),
        };

        let body = build_request_body(&req);
        assert_eq!(body["model"], "deepseek-chat");
        assert_eq!(body["stream"], false);
        assert!(body["messages"].as_array().unwrap().len() == 2);
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("查角色")],
            tools: Some(vec![ToolSpec::function(
                "get_character",
                "获取角色卡详情",
                serde_json::json!({"type": "object", "properties": {}}),
            )]),
            params: SamplingParams::default(),
            model: "deepseek-chat".into(),
        };

        let body = build_request_body(&req);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], "get_character");
    }

    #[test]
    fn test_build_request_body_passes_extra_params() {
        // P3-3：extra 扩展参数（thinking/reasoning_effort 等）应透传到请求体顶层
        let mut extra = serde_json::Map::new();
        extra.insert(
            "thinking".into(),
            serde_json::json!({"type": "enabled"}),
        );
        extra.insert("reasoning_effort".into(), serde_json::json!("max"));
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                temperature: Some(0.7),
                top_p: None,
                max_tokens: Some(2048),
                extra: Some(extra),
            },
            model: "minimax-m3".into(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["model"], "minimax-m3");
        // temperature 走 f32→JSON,有浮点精度差异,用近似比较
        let temp = body["temperature"].as_f64().unwrap();
        assert!((temp - 0.7f64).abs() < 1e-5, "temperature 近似 0.7, 实际 {temp}");
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "max");
    }

    #[test]
    fn test_parse_response() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "你好世界"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        let resp = parse_response(&body).unwrap();
        assert_eq!(resp.content, "你好世界");
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
        assert_eq!(resp.usage.unwrap().total_tokens, 15);
    }

    #[test]
    fn test_parse_response_with_tool_calls() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_character",
                            "arguments": "{\"id\":\"abc\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let resp = parse_response(&body).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].function.name, "get_character");
    }
}

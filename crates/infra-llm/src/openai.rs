/// OpenAI 兼容协议层
///
/// 负责将内部 ChatRequest 转为 OpenAI JSON 格式，以及将 OpenAI 响应转回 ChatResponse。
/// DeepSeek、Groq、Moonshot、OpenRouter 等都走此路径。
use storyforge_domain::llm::{ChatMessage, ChatRequest, ChatResponse, ToolCall};

fn quantize_sampling_float(value: f32) -> f64 {
    (f64::from(value) * 100.0).round() / 100.0
}

/// 构建 OpenAI 兼容的请求 JSON
pub fn build_request_body(req: &ChatRequest) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": req.model,
        "messages": build_messages(&req.messages),
        "stream": false,
    });

    // 采样参数
    if let Some(temp) = req.params.temperature {
        body["temperature"] = serde_json::json!(quantize_sampling_float(temp));
    }
    if let Some(top_p) = req.params.top_p {
        body["top_p"] = serde_json::json!(quantize_sampling_float(top_p));
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

    // thinking/reasoning_effort 不是 OpenAI-compatible 的通用字段。
    // 需要它们的供应商由连接 `extra` 明确提供；ReasoningMode 只控制提示模块
    // 与响应捕获要求，避免 gpt-4o / 普通 Qwen 等模型因未知字段直接 400。

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

use crate::usage_parse::parse_provider_usage;

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

    // OpenAI-compatible providers use several names for surfaced reasoning.
    // Preserve the exact returned string; never reconstruct reasoning from content.
    let reasoning_content = ["reasoning_content", "reasoning", "thinking"]
        .iter()
        .find_map(|key| message.get(*key).and_then(|value| value.as_str()))
        .filter(|text| !text.trim().is_empty())
        .map(String::from);

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

    // A2：流式/非流式共用 usage 解析（OpenAI / DeepSeek / Anthropic）
    let usage = body.get("usage").and_then(parse_provider_usage);

    Ok(ChatResponse {
        content,
        reasoning_content,
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
        assert!(body.get("max_tokens").is_none());
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
        extra.insert("thinking".into(), serde_json::json!({"type": "enabled"}));
        extra.insert("reasoning_effort".into(), serde_json::json!("max"));
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                temperature: Some(0.7),
                top_p: None,
                max_tokens: Some(2048),
                max_tokens_explicit: true,
                reasoning: storyforge_domain::llm::ReasoningMode::default(),
                extra: Some(extra),
            },
            model: "minimax-m3".into(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["model"], "minimax-m3");
        // temperature 走 f32→JSON,有浮点精度差异,用近似比较
        let temp = body["temperature"].as_f64().unwrap();
        assert!(
            (temp - 0.7f64).abs() < 1e-5,
            "temperature 近似 0.7, 实际 {temp}"
        );
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "max");
    }

    #[test]
    fn sampling_floats_are_quantized_for_strict_openai_compatible_providers() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                temperature: Some(0.7_f32),
                top_p: Some(0.95_f32),
                ..SamplingParams::default()
            },
            model: "glm-5.2".into(),
        };

        let encoded = serde_json::to_string(&build_request_body(&req)).unwrap();
        assert!(encoded.contains("\"temperature\":0.7"), "{encoded}");
        assert!(encoded.contains("\"top_p\":0.95"), "{encoded}");
        assert!(!encoded.contains("0.949999"), "{encoded}");
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
        assert!(resp.reasoning_content.is_none());
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
        assert_eq!(resp.usage.unwrap().total_tokens, 15);
    }

    #[test]
    fn test_parse_response_captures_provider_reasoning_content() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "reasoning_content": "先核对叙事契约，再按单一焦点合并。",
                    "content": "正文"
                },
                "finish_reason": "stop"
            }]
        });

        let resp = parse_response(&body).unwrap();
        assert_eq!(resp.content, "正文");
        assert_eq!(
            resp.reasoning_content.as_deref(),
            Some("先核对叙事契约，再按单一焦点合并。")
        );
    }

    #[test]
    fn test_parse_response_accepts_reasoning_alias() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "reasoning": "provider alias reasoning",
                    "content": "answer"
                },
                "finish_reason": "stop"
            }]
        });

        let resp = parse_response(&body).unwrap();
        assert_eq!(
            resp.reasoning_content.as_deref(),
            Some("provider alias reasoning")
        );
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

    // ─── A1：ReasoningMode 注入测试 ─────────────────────────────────────

    #[test]
    fn test_native_reasoning_does_not_invent_provider_thinking_parameter() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Native,
                ..Default::default()
            },
            model: "deepseek-reasoner".into(),
        };
        let body = build_request_body(&req);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_prompted_reasoning_does_not_invent_provider_thinking_parameter() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Prompted,
                ..Default::default()
            },
            model: "deepseek-v4-flash".into(),
        };
        let body = build_request_body(&req);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_prompted_reasoning_preserves_explicit_provider_thinking_parameter() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Prompted,
                extra: Some(serde_json::Map::from_iter([(
                    "thinking".into(),
                    serde_json::json!({ "type": "enabled" }),
                )])),
                ..Default::default()
            },
            model: "deepseek-v4-flash".into(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["thinking"]["type"], "enabled");
    }

    #[test]
    fn test_disabled_reasoning_no_thinking() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Disabled,
                ..Default::default()
            },
            model: "deepseek-chat".into(),
        };
        let body = build_request_body(&req);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_native_reasoning_extra_overrides_default() {
        // 用户通过 extra 显式设置 thinking，Native 应原样保留
        let mut extra = serde_json::Map::new();
        extra.insert("thinking".into(), serde_json::json!({"type": "disabled"}));
        let req = ChatRequest {
            messages: vec![ChatMessage::user("test")],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Native,
                extra: Some(extra),
                ..Default::default()
            },
            model: "test".into(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["thinking"]["type"], "disabled");
    }

    // ─── A2：缓存 token 解析测试 ─────────────────────────────────────────

    #[test]
    fn test_parse_cached_tokens_deepseek() {
        let usage = serde_json::json!({"prompt_cache_hit_tokens": 500});
        assert_eq!(crate::usage_parse::parse_cached_tokens(&usage), 500);
    }

    #[test]
    fn test_parse_cached_tokens_openai_nested() {
        let usage = serde_json::json!({"prompt_tokens_details": {"cached_tokens": 300}});
        assert_eq!(crate::usage_parse::parse_cached_tokens(&usage), 300);
    }

    #[test]
    fn test_parse_cached_tokens_anthropic() {
        let usage = serde_json::json!({"cache_read_input_tokens": 200});
        assert_eq!(crate::usage_parse::parse_cached_tokens(&usage), 200);
    }

    #[test]
    fn test_parse_cached_tokens_missing() {
        let usage = serde_json::json!({"prompt_tokens": 100});
        assert_eq!(crate::usage_parse::parse_cached_tokens(&usage), 0);
    }

    #[test]
    fn test_parse_cache_creation_tokens_deepseek() {
        let usage = serde_json::json!({"prompt_cache_miss_tokens": 400});
        assert_eq!(crate::usage_parse::parse_cache_creation_tokens(&usage), 400);
    }

    #[test]
    fn test_parse_response_with_deepseek_cache() {
        let body = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "total_tokens": 1050,
                "prompt_cache_hit_tokens": 800,
                "prompt_cache_miss_tokens": 200
            }
        });
        let resp = parse_response(&body).unwrap();
        let usage = resp.usage.unwrap();
        assert_eq!(usage.cached_tokens, 800);
        assert_eq!(usage.cache_creation_tokens, 200);
    }

    #[test]
    fn test_parse_response_with_openai_cache() {
        let body = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "total_tokens": 1050,
                "prompt_tokens_details": {"cached_tokens": 600}
            }
        });
        let resp = parse_response(&body).unwrap();
        let usage = resp.usage.unwrap();
        assert_eq!(usage.cached_tokens, 600);
    }
}

//! `DshLlmClient`：把 StoryForge 的每次模型调用反向交给 DSH 插件执行。
//!
//! sidecar 发 `llm.stream` 请求（中性格式，见 [`call_params`]），插件用用户在
//! StoryForge 面板里选的底层模型调 `ctx.llm.stream`，边收边回 `llm.chunk`
//! 通知，最后以响应返回整次结果。取消时发 `llm.cancel {id}`，插件中止该调用。
//!
//! 重试不在这里做：外层套与生产相同的 `RetryingClient`，错误按 HTTP 状态码
//! 映射到 `LlmError` 的可重试分类。
//!
//! 工具协议：默认原生 function calling；插件可经 `bridge.configure` 切到文本降级
//! （`ToolMode::TextFallback`），与生产 `HttpLlmClient::with_text_fallback` 同一套
//! `text_tools`：工具说明注入 system、请求不带 tools、从正文解析 `<tool_call>`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use storyforge_domain::llm::{
    ChatRequest, ChatResponse, ChatRole, FunctionCall, LlmError, ReasoningMode, StreamChunk,
    ToolCall, Usage,
};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_llm::text_tools::{inject_tool_prompt, parse_tool_calls_from_text};

use crate::rpc::{Peer, RpcError};

/// 底层调用的累计用量（计数互不重叠，与 DSH 的 usage 口径一致：
/// `input_tokens` 不含缓存读写）。每次实际发出的调用都计一次，包括重试。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct UsageTotals {
    pub calls: u64,
    pub failed_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl UsageTotals {
    /// 从 `earlier` 快照到本快照之间的增量。
    pub fn since(&self, earlier: &UsageTotals) -> UsageTotals {
        UsageTotals {
            calls: self.calls.saturating_sub(earlier.calls),
            failed_calls: self.failed_calls.saturating_sub(earlier.failed_calls),
            input_tokens: self.input_tokens.saturating_sub(earlier.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(earlier.output_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_sub(earlier.cache_read_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_sub(earlier.cache_write_tokens),
        }
    }

    fn record(&mut self, result: &Result<ChatResponse, LlmError>) {
        self.calls += 1;
        match result {
            Ok(response) => {
                if let Some(usage) = &response.usage {
                    let cached = u64::from(usage.cached_tokens);
                    let cache_write = u64::from(usage.cache_creation_tokens);
                    self.input_tokens += u64::from(usage.prompt_tokens)
                        .saturating_sub(cached)
                        .saturating_sub(cache_write);
                    self.output_tokens += u64::from(usage.completion_tokens);
                    self.cache_read_tokens += cached;
                    self.cache_write_tokens += cache_write;
                }
            }
            Err(_) => self.failed_calls += 1,
        }
    }
}

/// 桥接侧的模型调用设置与用量计数，sidecar 内共享。
#[derive(Default)]
pub struct LlmBridgeState {
    text_fallback: AtomicBool,
    usage: Mutex<UsageTotals>,
}

impl LlmBridgeState {
    pub fn text_fallback(&self) -> bool {
        self.text_fallback.load(Ordering::Relaxed)
    }

    pub fn set_text_fallback(&self, enabled: bool) {
        self.text_fallback.store(enabled, Ordering::Relaxed);
    }

    pub fn usage(&self) -> UsageTotals {
        *self.usage.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn record(&self, result: &Result<ChatResponse, LlmError>) {
        self.usage
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .record(result);
    }
}

pub struct DshLlmClient {
    peer: Arc<Peer>,
    state: Arc<LlmBridgeState>,
}

impl DshLlmClient {
    pub fn new(peer: Arc<Peer>, state: Arc<LlmBridgeState>) -> Self {
        Self { peer, state }
    }

    async fn call(
        &self,
        req: &ChatRequest,
        tx: Option<&mpsc::UnboundedSender<StreamChunk>>,
        cancel: Option<watch::Receiver<bool>>,
    ) -> Result<ChatResponse, LlmError> {
        let text_fallback =
            self.state.text_fallback() && req.tools.as_ref().is_some_and(|t| !t.is_empty());
        let result = if text_fallback {
            let mut req = req.clone();
            let tools = req.tools.take().unwrap_or_default();
            inject_tool_prompt(&mut req.messages, &tools);
            self.send(&req, tx, cancel).await.map(|mut response| {
                if response.tool_calls.is_empty() && !response.content.is_empty() {
                    response.tool_calls = parse_tool_calls_from_text(&response.content);
                }
                response
            })
        } else {
            self.send(req, tx, cancel).await
        };
        self.state.record(&result);
        result
    }

    async fn send(
        &self,
        req: &ChatRequest,
        tx: Option<&mpsc::UnboundedSender<StreamChunk>>,
        mut cancel: Option<watch::Receiver<bool>>,
    ) -> Result<ChatResponse, LlmError> {
        if cancel.as_ref().is_some_and(|c| *c.borrow()) {
            return Err(LlmError::Cancelled);
        }
        let keep_reasoning = req.params.reasoning != ReasoningMode::Disabled;
        let mut call = self.peer.call("llm.stream", call_params(req));
        let forward = |chunk: Value| {
            if let Some(tx) = tx
                && let Some(chunk) = stream_chunk(&chunk, keep_reasoning)
            {
                let _ = tx.send(chunk);
            }
        };

        let outcome = loop {
            tokio::select! {
                // 块先于响应到达（同一条有序管道）；biased 保证先排空块再看结果。
                biased;
                Some(chunk) = call.chunks.recv() => forward(chunk),
                result = &mut call.done => break result,
                _ = wait_cancel(&mut cancel) => {
                    self.peer.notify("llm.cancel", json!({ "id": call.id }));
                    self.peer.forget(&call.id);
                    return Err(LlmError::Cancelled);
                }
            }
        };
        while let Ok(chunk) = call.chunks.try_recv() {
            forward(chunk);
        }
        match outcome {
            Ok(Ok(result)) => parse_response(&result, keep_reasoning),
            Ok(Err(error)) => Err(map_error(error)),
            Err(_) => Err(map_error(RpcError::disconnected())),
        }
    }
}

/// 取消信号；没有取消通道时永不完成。
async fn wait_cancel(cancel: &mut Option<watch::Receiver<bool>>) {
    match cancel {
        Some(cancel) => {
            if cancel.wait_for(|cancelled| *cancelled).await.is_err() {
                // 发送端已释放：不会再取消。
                std::future::pending::<()>().await;
            }
        }
        None => std::future::pending::<()>().await,
    }
}

#[async_trait]
impl LlmClient for DshLlmClient {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        self.call(req, None, None).await
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        self.call(req, Some(&tx), Some(cancel)).await
    }
}

/// `llm.stream` 的参数（中性格式，插件负责转成 DSH 的消息与工具结构）。
///
/// `model_hint` 只是 StoryForge 侧角色配置里的模型名（通常是占位值），插件
/// 一律使用用户选定的底层模型；采样参数里 DSH 不支持的字段（top_p、厂商扩展）
/// 不传。
pub fn call_params(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                ChatRole::System => "system",
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
                ChatRole::Tool => "tool",
            };
            let mut value = json!({ "role": role, "content": message.content });
            if let Some(calls) = message
                .tool_calls
                .as_ref()
                .filter(|calls| !calls.is_empty())
            {
                value["tool_calls"] = calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "name": call.function.name,
                            "arguments": call.function.arguments,
                        })
                    })
                    .collect();
            }
            if let Some(id) = &message.tool_call_id {
                value["tool_call_id"] = json!(id);
            }
            value
        })
        .collect();
    let tools: Vec<Value> = req
        .tools
        .iter()
        .flatten()
        .map(|tool| {
            json!({
                "name": tool.function.name,
                "description": tool.function.description,
                "parameters": tool.function.parameters,
            })
        })
        .collect();
    let reasoning = match req.params.reasoning {
        ReasoningMode::Disabled => "disabled",
        ReasoningMode::Native => "native",
        ReasoningMode::Prompted => "prompted",
    };
    json!({
        "messages": messages,
        "tools": tools,
        "temperature": req.params.temperature,
        "max_tokens": req.params.max_tokens,
        "reasoning": reasoning,
        "model_hint": req.model,
    })
}

/// `llm.chunk` 的块：`{type: "text" | "reasoning", text}`。
fn stream_chunk(chunk: &Value, keep_reasoning: bool) -> Option<StreamChunk> {
    let text = chunk.get("text")?.as_str()?.to_string();
    let mut out = StreamChunk {
        delta_content: None,
        delta_reasoning_content: None,
        delta_tool_calls: None,
        finish_reason: None,
    };
    match chunk.get("type")?.as_str()? {
        "text" => out.delta_content = Some(text),
        "reasoning" if keep_reasoning => out.delta_reasoning_content = Some(text),
        _ => return None,
    }
    Some(out)
}

/// 整次结果：`{content, reasoning?, tool_calls[], finish_reason?, usage?}`。
///
/// 连接未开推理（`ReasoningMode::Disabled`）时丢弃模型返回的推理正文：
/// HTTP 客户端在这种情况下直接报错，桥接这边选模型的是用户，按"不要推理"处理。
pub fn parse_response(result: &Value, keep_reasoning: bool) -> Result<ChatResponse, LlmError> {
    let text = |key: &str| result.get(key).and_then(Value::as_str).map(str::to_owned);
    let tool_calls = result
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    let field = |key: &str| {
                        call.get(key)
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                LlmError::StreamParse(format!("tool_call 缺少 {key}: {call}"))
                            })
                    };
                    Ok(ToolCall {
                        id: field("id")?,
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: field("name")?,
                            arguments: call
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or("{}")
                                .to_string(),
                        },
                    })
                })
                .collect::<Result<Vec<_>, LlmError>>()
        })
        .transpose()?
        .unwrap_or_default();
    let usage = result.get("usage").filter(|u| u.is_object()).map(|usage| {
        let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
        let cached = count("cache_read_tokens");
        let cache_write = count("cache_write_tokens");
        // DSH 的计数互不重叠（input 不含缓存），StoryForge 的 prompt_tokens 是总输入。
        let prompt = count("input_tokens") + cached + cache_write;
        let completion = count("output_tokens");
        let total = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .map(|total| total as u32)
            .unwrap_or(prompt + completion);
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            cached_tokens: cached,
            cache_creation_tokens: cache_write,
        }
    });
    Ok(ChatResponse {
        content: text("content").unwrap_or_default(),
        reasoning_content: text("reasoning")
            .filter(|reasoning| keep_reasoning && !reasoning.trim().is_empty()),
        tool_calls,
        finish_reason: text("finish_reason"),
        usage,
    })
}

/// 插件侧失败（DSH `LlmFailure` 的 code/status）→ `LlmError` 分类。
pub fn map_error(error: RpcError) -> LlmError {
    let message = error.message.clone();
    match (error.code.as_str(), error.status) {
        ("aborted", _) => LlmError::Cancelled,
        ("timeout", _) => LlmError::Timeout,
        (_, Some(401 | 403)) => LlmError::Auth(message),
        (_, Some(429)) => match error.retry_after_ms {
            // RetryingClient 从消息尾部解析 `retry-after: Ns`。
            Some(ms) => {
                LlmError::RateLimited(format!("{message} retry-after: {}s", ms.div_ceil(1000)))
            }
            None => LlmError::RateLimited(message),
        },
        (_, Some(400)) => LlmError::BadRequest(message),
        (_, Some(status)) if status >= 500 => LlmError::ServerError(message),
        ("disconnected", _) => LlmError::Http(format!("DSH 插件已断开: {message}")),
        ("no_model", _) => LlmError::BadRequest(message),
        _ => LlmError::Http(format!("{}: {message}", error.code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::{ChatMessage, SamplingParams, ToolSpec};

    fn request(reasoning: ReasoningMode) -> ChatRequest {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = Some(vec![ToolCall {
            id: "call-1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "get_character".into(),
                arguments: r#"{"id":"a"}"#.into(),
            },
        }]);
        ChatRequest {
            messages: vec![
                ChatMessage::system("sys"),
                ChatMessage::user("hi"),
                assistant,
                ChatMessage::tool_result("call-1", "result"),
            ],
            tools: Some(vec![ToolSpec::function(
                "get_character",
                "look up",
                json!({ "type": "object" }),
            )]),
            params: SamplingParams {
                reasoning,
                ..SamplingParams::default()
            },
            model: "director".into(),
        }
    }

    #[test]
    fn call_params_carry_roles_tool_calls_and_tools() {
        let params = call_params(&request(ReasoningMode::Disabled));
        let messages = params["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[2]["tool_calls"][0]["name"], "get_character");
        assert_eq!(messages[2]["tool_calls"][0]["arguments"], r#"{"id":"a"}"#);
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call-1");
        assert!(messages[1].get("tool_calls").is_none());
        assert_eq!(params["tools"][0]["parameters"]["type"], "object");
        assert_eq!(params["reasoning"], "disabled");
        assert_eq!(params["model_hint"], "director");
    }

    #[test]
    fn parse_response_maps_tool_calls_usage_and_drops_unrequested_reasoning() {
        let result = json!({
            "content": "",
            "reasoning": "thinking",
            "tool_calls": [{ "id": "c1", "name": "search", "arguments": "{}" }],
            "finish_reason": "tool_calls",
            "usage": { "input_tokens": 10, "output_tokens": 5, "cache_read_tokens": 90 },
        });
        let response = parse_response(&result, false).unwrap();
        assert_eq!(response.tool_calls[0].function.name, "search");
        assert_eq!(response.reasoning_content, None);
        let usage = response.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.cached_tokens, 90);
        assert_eq!(usage.total_tokens, 105);

        let kept = parse_response(&result, true).unwrap();
        assert_eq!(kept.reasoning_content.as_deref(), Some("thinking"));

        let broken = json!({ "tool_calls": [{ "name": "x" }] });
        assert!(matches!(
            parse_response(&broken, false),
            Err(LlmError::StreamParse(_))
        ));
    }

    #[test]
    fn errors_map_to_retryable_categories() {
        let with_status = |status| RpcError {
            status: Some(status),
            ..RpcError::new("http", "boom")
        };
        assert!(map_error(with_status(503)).is_retryable());
        assert!(matches!(map_error(with_status(401)), LlmError::Auth(_)));
        assert!(matches!(
            map_error(with_status(400)),
            LlmError::BadRequest(_)
        ));
        let limited = RpcError {
            retry_after_ms: Some(1500),
            ..with_status(429)
        };
        match map_error(limited) {
            LlmError::RateLimited(message) => assert!(message.ends_with("retry-after: 2s")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            map_error(RpcError::new("aborted", "x")),
            LlmError::Cancelled
        ));
        assert!(!map_error(RpcError::disconnected()).is_retryable());
    }

    #[tokio::test]
    async fn chat_stream_forwards_chunks_and_cancels_through_the_peer() {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let peer = Arc::new(Peer::new(out_tx));
        let client = DshLlmClient::new(peer.clone(), Arc::default());

        // 假插件：收到 llm.stream 后回两个块和结果。
        let plugin_peer = peer.clone();
        let plugin = tokio::spawn(async move {
            let line = out_rx.recv().await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let id = request["id"].as_str().unwrap().to_string();
            for (kind, text) in [("reasoning", "hmm"), ("text", "Hello")] {
                let chunk = json!({ "method": "llm.chunk", "params": { "id": id, "chunk": { "type": kind, "text": text } } });
                plugin_peer.accept_line(&chunk.to_string());
            }
            let done =
                json!({ "id": id, "result": { "content": "Hello", "finish_reason": "stop" } });
            plugin_peer.accept_line(&done.to_string());
            out_rx
        });

        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let response = client
            .chat_stream(&request(ReasoningMode::Disabled), tx, cancel_rx)
            .await
            .unwrap();
        assert_eq!(response.content, "Hello");
        // 推理块在未开推理时不转发。
        assert_eq!(
            rx.recv().await.unwrap().delta_content.as_deref(),
            Some("Hello")
        );
        assert!(rx.try_recv().is_err());

        // 取消：发 llm.cancel 并放弃挂起请求。
        let mut out_rx = plugin.await.unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let pending = tokio::spawn({
            let client = DshLlmClient::new(peer.clone(), Arc::default());
            async move {
                client
                    .chat_stream(&request(ReasoningMode::Disabled), tx, cancel_rx)
                    .await
            }
        });
        let request_line: Value = serde_json::from_str(&out_rx.recv().await.unwrap()).unwrap();
        cancel_tx.send(true).unwrap();
        assert!(matches!(pending.await.unwrap(), Err(LlmError::Cancelled)));
        let cancel_line: Value = serde_json::from_str(&out_rx.recv().await.unwrap()).unwrap();
        assert_eq!(cancel_line["method"], "llm.cancel");
        assert_eq!(cancel_line["params"]["id"], request_line["id"]);
        assert_eq!(peer.pending_count(), 0);
    }

    #[tokio::test]
    async fn text_fallback_injects_tool_prompt_parses_calls_and_meters_usage() {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let peer = Arc::new(Peer::new(out_tx));
        let state = Arc::new(LlmBridgeState::default());
        state.set_text_fallback(true);
        let client = DshLlmClient::new(peer.clone(), state.clone());

        let plugin_peer = peer.clone();
        let plugin = tokio::spawn(async move {
            let line = out_rx.recv().await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let params = &request["params"];
            // 降级模式：不带原生 tools，工具说明进了 system。
            assert_eq!(params["tools"], json!([]));
            let system = params["messages"][0]["content"].as_str().unwrap();
            assert!(system.contains("<tool_call>") && system.contains("get_character"));
            let content = "<tool_call>
{\"name\": \"get_character\", \"arguments\": {\"id\": \"b\"}}
</tool_call>";
            let done = json!({
                "id": request["id"],
                "result": {
                    "content": content,
                    "finish_reason": "stop",
                    "usage": { "input_tokens": 30, "output_tokens": 7, "cache_read_tokens": 70 },
                },
            });
            plugin_peer.accept_line(&done.to_string());
            out_rx
        });

        let response = client
            .chat(&request(ReasoningMode::Disabled))
            .await
            .unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].function.name, "get_character");
        assert_eq!(response.tool_calls[0].function.arguments, r#"{"id":"b"}"#);

        let before = UsageTotals::default();
        let usage = state.usage();
        assert_eq!(
            usage.since(&before),
            UsageTotals {
                calls: 1,
                failed_calls: 0,
                input_tokens: 30,
                output_tokens: 7,
                cache_read_tokens: 70,
                cache_write_tokens: 0,
            }
        );

        // 失败也计一次调用。
        let mut out_rx = plugin.await.unwrap();
        state.set_text_fallback(false);
        let failing = tokio::spawn({
            let client = DshLlmClient::new(peer.clone(), state.clone());
            async move { client.chat(&request(ReasoningMode::Disabled)).await }
        });
        let request_line: Value = serde_json::from_str(&out_rx.recv().await.unwrap()).unwrap();
        assert_eq!(request_line["params"]["tools"][0]["name"], "get_character");
        let error = json!({ "id": request_line["id"], "error": { "code": "http", "message": "boom", "status": 400 } });
        peer.accept_line(&error.to_string());
        assert!(matches!(
            failing.await.unwrap(),
            Err(LlmError::BadRequest(_))
        ));
        let delta = state.usage().since(&usage);
        assert_eq!((delta.calls, delta.failed_calls), (1, 1));
    }
}

/// LLM 调用拦截器（包装 LlmClient，记录每次调用的 payload/响应/token/延迟）
///
/// 设计来源：TT 的 LLM 拦截层 hook。
/// LlmInterceptor 实现 LlmClient trait，在每次调用前后记录日志。
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmError, StreamChunk};
use storyforge_infra_llm::LlmClient;

use crate::{LlmCallDetail, LlmToolCallRecord, LogEntry, LogKind, LogLevel, LogStore};

/// LLM 调用拦截器（记录每次调用到 LogStore）
pub struct LlmInterceptor {
    inner: Arc<dyn LlmClient>,
    store: Arc<LogStore>,
    connection_name: String,
}

impl LlmInterceptor {
    pub fn new(inner: Arc<dyn LlmClient>, store: Arc<LogStore>, connection_name: String) -> Self {
        Self {
            inner,
            store,
            connection_name,
        }
    }

    /// 构造 LLM 调用日志条目
    fn make_llm_entry(
        &self,
        req: &ChatRequest,
        resp: Option<&ChatResponse>,
        error: Option<&str>,
        latency_ms: u64,
    ) -> LogEntry {
        let request_payload =
            serde_json::to_string(&req.messages).unwrap_or_else(|_| "<serialization error>".into());

        let (response_text, prompt_tokens, completion_tokens, cached_tokens, cache_creation_tokens) =
            if let Some(r) = resp {
                (
                    r.content.clone(),
                    r.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                    r.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                    r.usage.as_ref().map(|u| u.cached_tokens).unwrap_or(0),
                    r.usage
                        .as_ref()
                        .map(|u| u.cache_creation_tokens)
                        .unwrap_or(0),
                )
            } else {
                (String::new(), 0, 0, 0, 0)
            };

        let response_tool_calls = resp
            .map(|r| r.tool_calls.iter().map(LlmToolCallRecord::from).collect())
            .unwrap_or_default();

        let detail = LlmCallDetail {
            connection_name: self.connection_name.clone(),
            model: req.model.clone(),
            profile_id: None,
            agent_role: None,
            request_payload,
            response_text,
            response_tool_calls,
            prompt_tokens,
            completion_tokens,
            cached_tokens,
            cache_creation_tokens,
            latency_ms,
            error: error.map(String::from),
        };

        let level = if error.is_some() {
            LogLevel::Error
        } else {
            LogLevel::Info
        };

        LogEntry {
            id: storyforge_domain::Id::new(),
            kind: LogKind::LlmCall,
            level,
            timestamp: Utc::now(),
            message: format!(
                "LLM {} {} ({}ms, {}+{} tokens{})",
                self.connection_name,
                if error.is_some() { "失败" } else { "完成" },
                latency_ms,
                detail.prompt_tokens,
                detail.completion_tokens,
                if detail.cached_tokens > 0 {
                    format!(", cache命中{}", detail.cached_tokens)
                } else {
                    String::new()
                },
            ),
            fields: Default::default(),
            llm_detail: Some(detail),
        }
    }
}

#[async_trait]
impl LlmClient for LlmInterceptor {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        debug!(target: "app-logging", "LLM chat 拦截: model={}", req.model);
        let start = Instant::now();

        let result = self.inner.chat(req).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match &result {
            Ok(resp) => {
                let entry = self.make_llm_entry(req, Some(resp), None, latency_ms);
                self.store.push(entry);
            }
            Err(e) => {
                let entry = self.make_llm_entry(req, None, Some(&e.to_string()), latency_ms);
                self.store.push(entry);
                warn!(target: "app-logging", "LLM chat 失败: {e}");
            }
        }

        result
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        debug!(target: "app-logging", "LLM chat_stream 拦截: model={}", req.model);
        let start = Instant::now();

        let result = self.inner.chat_stream(req, tx, cancel).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match &result {
            Ok(resp) => {
                let entry = self.make_llm_entry(req, Some(resp), None, latency_ms);
                self.store.push(entry);
            }
            Err(e) => {
                let entry = self.make_llm_entry(req, None, Some(&e.to_string()), latency_ms);
                self.store.push(entry);
                warn!(target: "app-logging", "LLM chat_stream 失败: {e}");
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::{ChatMessage, FunctionCall, SamplingParams, ToolCall};

    use crate::LogFilter;

    /// 只回原生 tool call、正文为空的客户端（模拟后处理 `emit_postprocess` 轮次）
    struct ToolCallOnlyClient {
        arguments: String,
    }

    impl ToolCallOnlyClient {
        fn response(&self) -> ChatResponse {
            ChatResponse {
                content: String::new(),
                reasoning_content: None,
                tool_calls: vec![ToolCall {
                    id: "call_pp_1".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "emit_postprocess".into(),
                        arguments: self.arguments.clone(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                usage: None,
            }
        }
    }

    #[async_trait]
    impl LlmClient for ToolCallOnlyClient {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            Ok(self.response())
        }

        async fn chat_stream(
            &self,
            _req: &ChatRequest,
            _tx: mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            Ok(self.response())
        }
    }

    #[tokio::test]
    async fn test_native_tool_calls_are_logged_and_persisted() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_interceptor_tool_calls_{}",
            uuid::Uuid::new_v4()
        ));
        let store = Arc::new(LogStore::new(dir.clone()));
        // 模型漏了 description 的那种 trigger：原样记录，不做解析
        let arguments = r#"{"kind":"event","desc":"雨夜来客"}"#.to_string();
        let interceptor = LlmInterceptor::new(
            Arc::new(ToolCallOnlyClient {
                arguments: arguments.clone(),
            }),
            Arc::clone(&store),
            "test-conn".into(),
        );
        let req = ChatRequest {
            messages: vec![ChatMessage::user("hi")],
            tools: None,
            params: SamplingParams::default(),
            model: "gemini-3.8-flash-high".into(),
        };

        interceptor.chat(&req).await.unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        interceptor.chat_stream(&req, tx, cancel_rx).await.unwrap();

        let expected = vec![LlmToolCallRecord {
            id: "call_pp_1".into(),
            name: "emit_postprocess".into(),
            arguments: arguments.clone(),
        }];
        let entries = store.query(&LogFilter {
            kind: Some(LogKind::LlmCall),
            ..Default::default()
        });
        assert_eq!(entries.len(), 2, "chat 与 chat_stream 各记一条");
        for entry in &entries {
            let detail = entry.llm_detail.as_ref().unwrap();
            assert!(detail.response_text.is_empty());
            assert_eq!(detail.response_tool_calls, expected);
        }

        // 落盘的 jsonl 也要带上参数原文，跨会话回填后仍可查
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let content = std::fs::read_to_string(dir.join(format!("{date}.jsonl"))).unwrap();
        let persisted: Vec<LogEntry> = content
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(persisted.len(), 2);
        for entry in &persisted {
            assert_eq!(
                entry.llm_detail.as_ref().unwrap().response_tool_calls,
                expected
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

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

use crate::{LlmCallDetail, LogEntry, LogKind, LogLevel, LogStore};

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

        let (response_text, prompt_tokens, completion_tokens) = if let Some(r) = resp {
            (
                r.content.clone(),
                r.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                r.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
            )
        } else {
            (String::new(), 0, 0)
        };

        let detail = LlmCallDetail {
            connection_name: self.connection_name.clone(),
            model: req.model.clone(),
            profile_id: None,
            agent_role: None,
            request_payload,
            response_text,
            prompt_tokens,
            completion_tokens,
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
                "LLM {} {} ({}ms, {}+{} tokens)",
                self.connection_name,
                if error.is_some() { "失败" } else { "完成" },
                latency_ms,
                detail.prompt_tokens,
                detail.completion_tokens,
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

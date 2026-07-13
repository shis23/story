//! 真实 LLM 调用预算与 usage 录制包装。
//!
//! - `max_calls`：共享原子计数，超过后返回 `LlmError::Internal`（非假成功）
//! - `timeout_secs`：对 `chat` / `chat_stream` 施加 `tokio::timeout`
//! - 录制脱敏 usage + segment hash，供 JSONL 证据写入

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmError, StreamChunk, Usage};
use storyforge_domain::message_layout::{fingerprint_messages, messages_segment_summary};
use storyforge_infra_llm::LlmClient;
use tokio::sync::{mpsc, watch};

use crate::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceCallRecord, RealLlmRunBudget, short_hash16,
};

/// 单次调用的脱敏 usage 样本（不落全文）。
#[derive(Debug, Clone)]
pub struct UsageSample {
    pub tag: String,
    pub role: String,
    pub streaming: bool,
    pub prompt_tokens: u32,
    pub cached_tokens: u32,
    pub cache_creation_tokens: u32,
    pub completion_tokens: u32,
    pub request_fp16: String,
    pub system_hash16: String,
    pub history_hash16: String,
    pub tail_hash16: String,
    pub history_len: usize,
    pub tail_parts: usize,
    pub msg_count: usize,
    pub elapsed_ms: u128,
}

impl UsageSample {
    pub fn to_evidence_call(
        &self,
        run_id: impl Into<String>,
        suite: impl Into<String>,
        turn_index: u32,
        model_label: impl Into<String>,
        assertions: Vec<AssertionResult>,
    ) -> EvidenceCallRecord {
        EvidenceCallRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: run_id.into(),
            suite: suite.into(),
            turn_index,
            role: self.role.clone(),
            tag: self.tag.clone(),
            streaming: self.streaming,
            request_fp16: self.request_fp16.clone(),
            system_hash16: self.system_hash16.clone(),
            history_hash16: self.history_hash16.clone(),
            tail_hash16: self.tail_hash16.clone(),
            history_len: self.history_len,
            tail_parts: self.tail_parts,
            msg_count: self.msg_count,
            prompt_tokens: self.prompt_tokens,
            cached_tokens: self.cached_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            completion_tokens: self.completion_tokens,
            elapsed_ms: self.elapsed_ms,
            assertion_results: assertions,
            model_label: model_label.into(),
            recorded_at_unix_ms: 0,
        }
    }
}

/// 带预算上限与 usage 录制的 LLM 客户端包装。
pub struct BudgetedLlmClient {
    inner: Arc<dyn LlmClient>,
    calls: AtomicU32,
    max_calls: u32,
    timeout_secs: u64,
    samples: Mutex<Vec<UsageSample>>,
    turn_tag: Mutex<String>,
    role_label: Mutex<String>,
}

impl BudgetedLlmClient {
    pub fn wrap(inner: Arc<dyn LlmClient>, budget: &RealLlmRunBudget) -> Arc<Self> {
        Arc::new(Self {
            inner,
            calls: AtomicU32::new(0),
            max_calls: budget.max_calls,
            timeout_secs: budget.timeout_secs.max(1),
            samples: Mutex::new(Vec::new()),
            turn_tag: Mutex::new("boot".into()),
            role_label: Mutex::new("pipeline".into()),
        })
    }

    pub fn set_tag(&self, tag: impl Into<String>) {
        *self.turn_tag.lock().expect("turn_tag lock") = tag.into();
    }

    pub fn set_role(&self, role: impl Into<String>) {
        *self.role_label.lock().expect("role lock") = role.into();
    }

    pub fn calls_used(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn max_calls(&self) -> u32 {
        self.max_calls
    }

    pub fn samples(&self) -> Vec<UsageSample> {
        self.samples.lock().expect("samples lock").clone()
    }

    pub fn total_prompt_tokens(&self) -> u32 {
        self.samples().iter().map(|s| s.prompt_tokens).sum()
    }

    pub fn total_completion_tokens(&self) -> u32 {
        self.samples().iter().map(|s| s.completion_tokens).sum()
    }

    fn reserve_call(&self) -> Result<(), LlmError> {
        // 先占位再调用，避免并发超支
        let prev = self.calls.fetch_add(1, Ordering::SeqCst);
        if prev >= self.max_calls {
            self.calls.fetch_sub(1, Ordering::SeqCst);
            return Err(LlmError::Internal(format!(
                "eval budget exhausted: max_calls={} already used={}",
                self.max_calls, prev
            )));
        }
        Ok(())
    }

    fn record(&self, req: &ChatRequest, resp: &ChatResponse, elapsed_ms: u128, streaming: bool) {
        let segs = messages_segment_summary(&req.messages);
        let fp = fingerprint_messages(&req.messages);
        let usage = resp.usage.clone().unwrap_or(Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let sample = UsageSample {
            tag: self.turn_tag.lock().expect("turn_tag lock").clone(),
            role: self.role_label.lock().expect("role lock").clone(),
            streaming,
            prompt_tokens: usage.prompt_tokens,
            cached_tokens: usage.cached_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            completion_tokens: usage.completion_tokens,
            request_fp16: short_hash16(&fp),
            system_hash16: segs.system_hash.chars().take(16).collect(),
            history_hash16: segs.history_hash.chars().take(16).collect(),
            tail_hash16: segs.tail_hash.chars().take(16).collect(),
            history_len: segs.history_len,
            tail_parts: segs.tail_parts,
            msg_count: req.messages.len(),
            elapsed_ms,
        };
        eprintln!(
            "[eval-budget] call={} tag={} stream={} prompt={} cached={} completion={} ms={}",
            self.calls_used(),
            sample.tag,
            sample.streaming,
            sample.prompt_tokens,
            sample.cached_tokens,
            sample.completion_tokens,
            sample.elapsed_ms
        );
        self.samples.lock().expect("samples lock").push(sample);
    }
}

#[async_trait]
impl LlmClient for BudgetedLlmClient {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        self.reserve_call()?;
        let t0 = Instant::now();
        let fut = self.inner.chat(req);
        let resp = match tokio::time::timeout(Duration::from_secs(self.timeout_secs), fut).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(LlmError::Timeout);
            }
        };
        self.record(req, &resp, t0.elapsed().as_millis(), false);
        Ok(resp)
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        self.reserve_call()?;
        let t0 = Instant::now();
        let fut = self.inner.chat_stream(req, tx, cancel);
        let resp = match tokio::time::timeout(Duration::from_secs(self.timeout_secs), fut).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(LlmError::Timeout);
            }
        };
        self.record(req, &resp, t0.elapsed().as_millis(), true);
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::ChatMessage;
    use storyforge_infra_llm::mock_client::MockLlmClient;

    fn dummy_req() -> ChatRequest {
        ChatRequest {
            model: "mock".into(),
            messages: vec![ChatMessage::user("hi")],
            tools: None,
            params: storyforge_domain::llm::SamplingParams::default(),
        }
    }

    #[tokio::test]
    async fn budget_blocks_after_max_calls() {
        let inner: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let budget = RealLlmRunBudget {
            enabled: true,
            max_calls: 1,
            max_turns: 1,
            timeout_secs: 30,
        };
        let client = BudgetedLlmClient::wrap(inner, &budget);
        let req = dummy_req();
        let first = client.chat(&req).await;
        assert!(first.is_ok(), "first call should succeed");
        let second = client.chat(&req).await;
        assert!(
            matches!(second, Err(LlmError::Internal(ref s)) if s.contains("budget exhausted")),
            "second call must be blocked: {second:?}"
        );
        assert_eq!(client.calls_used(), 1);
        assert_eq!(client.samples().len(), 1);
    }

    #[tokio::test]
    async fn budget_timeout_returns_timeout_error() {
        struct SlowClient;
        #[async_trait]
        impl LlmClient for SlowClient {
            async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(ChatResponse {
                    content: "late".into(),
                    tool_calls: vec![],
                    finish_reason: None,
                    usage: None,
                })
            }
            async fn chat_stream(
                &self,
                req: &ChatRequest,
                _tx: mpsc::UnboundedSender<StreamChunk>,
                _cancel: watch::Receiver<bool>,
            ) -> Result<ChatResponse, LlmError> {
                self.chat(req).await
            }
        }

        let client = Arc::new(BudgetedLlmClient {
            inner: Arc::new(SlowClient),
            calls: AtomicU32::new(0),
            max_calls: 5,
            timeout_secs: 1,
            samples: Mutex::new(Vec::new()),
            turn_tag: Mutex::new("t".into()),
            role_label: Mutex::new("r".into()),
        });
        let req = dummy_req();
        let result = client.chat(&req).await;
        assert!(
            matches!(result, Err(LlmError::Timeout)),
            "expected Timeout, got {result:?}"
        );
        assert_eq!(client.calls_used(), 1);
        assert!(client.samples().is_empty(), "timeout must not record usage");
    }
}

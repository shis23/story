//! 真实 LLM 调用预算与 usage 录制包装。
//!
//! - `max_calls`：共享原子计数，超过后返回 `LlmError::Internal`（非假成功）
//! - `timeout_secs`：对 `chat` / `chat_stream` 施加 `tokio::timeout`
//! - 录制脱敏 usage + segment hash，供 JSONL 证据写入

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use storyforge_domain::llm::{
    ChatRequest, ChatResponse, LlmError, ReasoningMode, StreamChunk, Usage,
};
use storyforge_domain::message_layout::{fingerprint_chat_request, messages_segment_summary};
use storyforge_infra_llm::LlmClient;
use tokio::sync::{mpsc, watch};

use crate::evidence::{
    AssertionResult, EVIDENCE_SCHEMA_VERSION, EvidenceCallRecord, EvidenceToolStep,
    RealLlmRunBudget, now_unix_ms, short_hash16,
};
use storyforge_domain::llm::ChatRole;

/// 单次调用的脱敏 usage 样本（不落全文）。
#[derive(Debug, Clone)]
pub struct UsageSample {
    pub call_index: u32,
    pub evidence_turn_index: u32,
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
    pub outcome: String,
    pub tools_offered: Vec<String>,
    pub tool_steps: Vec<EvidenceToolStep>,
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
            call_index: self.call_index,
            suite: suite.into(),
            turn_index: if self.evidence_turn_index == 0 {
                turn_index
            } else {
                self.evidence_turn_index
            },
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
            outcome: self.outcome.clone(),
            tools_offered: self.tools_offered.clone(),
            tool_steps: self.tool_steps.clone(),
            assertion_results: assertions,
            model_label: model_label.into(),
            recorded_at_unix_ms: 0,
        }
    }
}

fn redact_tool_args_detail(tool_name: &str, args_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let obj = v.as_object()?;
    let mut parts = Vec::new();
    match tool_name {
        "get_recent_summary" => {
            if let Some(limit) = obj.get("limit").and_then(|x| x.as_u64()) {
                parts.push(format!("limit={limit}"));
            }
        }
        "search_chronicle" | "search_vectors" | "search_world_info" => {
            if let Some(q) = obj.get("query").and_then(|x| x.as_str()) {
                parts.push(format!("query_len={}", q.chars().count()));
                parts.push(format!("query_hash16={}", short_hash16(q)));
            }
            if let Some(limit) = obj.get("limit").and_then(|x| x.as_u64()) {
                parts.push(format!("limit={limit}"));
            }
            if let Some(level) = obj.get("level").and_then(|x| x.as_str()) {
                parts.push(format!("level={level}"));
            }
        }
        "get_chronicle" => {
            if let Some(code) = obj.get("code").and_then(|x| x.as_str()) {
                parts.push(format!("code_hash16={}", short_hash16(code)));
            }
        }
        "get_character" => {
            if let Some(name) = obj
                .get("name")
                .or_else(|| obj.get("character_name"))
                .and_then(|x| x.as_str())
            {
                parts.push(format!("name_hash16={}", short_hash16(name)));
            }
            if let Some(id) = obj
                .get("id")
                .or_else(|| obj.get("character_id"))
                .and_then(|x| x.as_str())
            {
                parts.push(format!("id_hash16={}", short_hash16(id)));
            }
        }
        _ => {
            parts.push(format!("keys={}", obj.len()));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(";"))
    }
}

fn redact_tool_result_detail(tool_name: &str, result_json: &str) -> (bool, Option<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(result_json) else {
        return (true, Some(format!("result_len={}", result_json.len())));
    };
    if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
        return (
            false,
            Some(format!(
                "error_class={};error_len={}",
                if err.contains("budget") {
                    "budget"
                } else if err.contains("Invalid JSON") {
                    "bad_args"
                } else {
                    "tool_error"
                },
                err.chars().count()
            )),
        );
    }
    let detail = match tool_name {
        "get_recent_summary" => {
            let count = v
                .get("summaries_count")
                .and_then(|x| x.as_u64())
                .or_else(|| {
                    v.get("summaries")
                        .and_then(|x| x.as_array())
                        .map(|a| a.len() as u64)
                })
                .unwrap_or(0);
            Some(format!("summaries_count={count}"))
        }
        "search_chronicle" | "search_vectors" | "search_world_info" => {
            let count = v
                .get("results")
                .and_then(|x| x.as_array())
                .map(|a| a.len())
                .or_else(|| v.get("count").and_then(|x| x.as_u64()).map(|n| n as usize))
                .unwrap_or(0);
            Some(format!("results_count={count}"))
        }
        "get_chronicle" => {
            let has =
                v.get("summary").is_some() || v.get("content").is_some() || v.get("code").is_some();
            Some(format!("hit={}", has))
        }
        _ => {
            let keys = v.as_object().map(|o| o.len()).unwrap_or(0);
            Some(format!("keys={keys}"))
        }
    };
    (true, detail)
}

fn extract_tool_trace(
    req: &ChatRequest,
    resp: Option<&ChatResponse>,
) -> (Vec<String>, Vec<EvidenceToolStep>) {
    let tools_offered = req
        .tools
        .as_ref()
        .map(|tools| {
            tools
                .iter()
                .map(|t| t.function.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut steps = Vec::new();
    // History already contains prior assistant tool_calls + tool results from the loop.
    for msg in &req.messages {
        match msg.role {
            ChatRole::Assistant => {
                if let Some(tcs) = msg.tool_calls.as_ref() {
                    for tc in tcs {
                        steps.push(EvidenceToolStep {
                            kind: "call".into(),
                            tool_name: tc.function.name.clone(),
                            call_id16: Some(short_hash16(&tc.id)),
                            args_hash16: Some(short_hash16(&tc.function.arguments)),
                            args_len: Some(tc.function.arguments.len()),
                            result_hash16: None,
                            result_len: None,
                            ok: None,
                            detail: redact_tool_args_detail(
                                &tc.function.name,
                                &tc.function.arguments,
                            ),
                        });
                    }
                }
            }
            ChatRole::Tool => {
                let id16 = msg.tool_call_id.as_ref().map(|id| short_hash16(id));
                // Try to pair with last unmatched call for tool_name.
                let tool_name = steps
                    .iter()
                    .rev()
                    .find(|s| s.kind == "call" && s.call_id16.as_deref() == id16.as_deref())
                    .map(|s| s.tool_name.clone())
                    .unwrap_or_else(|| "unknown".into());
                let (ok, detail) = redact_tool_result_detail(&tool_name, &msg.content);
                steps.push(EvidenceToolStep {
                    kind: "result".into(),
                    tool_name,
                    call_id16: id16,
                    args_hash16: None,
                    args_len: None,
                    result_hash16: Some(short_hash16(&msg.content)),
                    result_len: Some(msg.content.len()),
                    ok: Some(ok),
                    detail,
                });
            }
            _ => {}
        }
    }
    if let Some(resp) = resp {
        for tc in &resp.tool_calls {
            steps.push(EvidenceToolStep {
                kind: "call".into(),
                tool_name: tc.function.name.clone(),
                call_id16: Some(short_hash16(&tc.id)),
                args_hash16: Some(short_hash16(&tc.function.arguments)),
                args_len: Some(tc.function.arguments.len()),
                result_hash16: None,
                result_len: None,
                ok: None,
                detail: redact_tool_args_detail(&tc.function.name, &tc.function.arguments),
            });
        }
    }
    (tools_offered, steps)
}

/// 带预算上限与 usage 录制的 LLM 客户端包装。
pub const CALL_RESERVATION_SCHEMA_VERSION: &str = "m5-call-reservation-v1";

/// Privacy-safe write-ahead record persisted before a real provider dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallReservationRecord {
    pub schema_version: String,
    pub run_id: String,
    pub call_index: u32,
    pub turn_index: u32,
    pub tag: String,
    pub role: String,
    pub streaming: bool,
    pub request_fp16: String,
    pub recorded_at_unix_ms: u128,
}

/// Synchronous fail-closed sink. Implementations must durably persist the
/// reservation before returning success; the provider is called only afterward.
pub trait DurableCallReservationSink: Send + Sync {
    fn reserve(&self, record: &CallReservationRecord) -> Result<(), String>;
}

#[derive(Debug, Clone)]
struct ReservedCall {
    call_index: u32,
    turn_index: u32,
    tag: String,
    role: String,
    streaming: bool,
    request_fp16: String,
}

pub struct BudgetedLlmClient {
    inner: Arc<dyn LlmClient>,
    calls: AtomicU32,
    max_calls: u32,
    max_turns: u32,
    timeout_secs: u64,
    max_tokens: Option<u32>,
    reasoning_override: Option<ReasoningMode>,
    samples: Mutex<Vec<UsageSample>>,
    turn_tag: Mutex<String>,
    role_label: Mutex<String>,
    evidence_turn_index: AtomicU32,
    reservation_gate: Mutex<()>,
    reservation_run_id: Option<String>,
    reservation_sink: Option<Arc<dyn DurableCallReservationSink>>,
}

impl BudgetedLlmClient {
    pub fn wrap(inner: Arc<dyn LlmClient>, budget: &RealLlmRunBudget) -> Arc<Self> {
        Self::wrap_with_reasoning(inner, budget, None)
    }

    pub fn wrap_with_reasoning(
        inner: Arc<dyn LlmClient>,
        budget: &RealLlmRunBudget,
        reasoning_override: Option<ReasoningMode>,
    ) -> Arc<Self> {
        Self::build(inner, budget, reasoning_override, 0, None, None)
    }

    pub fn wrap_with_reasoning_and_reservations(
        inner: Arc<dyn LlmClient>,
        budget: &RealLlmRunBudget,
        reasoning_override: Option<ReasoningMode>,
        initial_calls: u32,
        run_id: impl Into<String>,
        reservation_sink: Arc<dyn DurableCallReservationSink>,
    ) -> Result<Arc<Self>, String> {
        if initial_calls > budget.max_calls {
            return Err("initial durable calls exceed the configured global call budget".into());
        }
        let run_id = run_id.into();
        if run_id.is_empty() {
            return Err("durable reservation run id is empty".into());
        }
        Ok(Self::build(
            inner,
            budget,
            reasoning_override,
            initial_calls,
            Some(run_id),
            Some(reservation_sink),
        ))
    }

    fn build(
        inner: Arc<dyn LlmClient>,
        budget: &RealLlmRunBudget,
        reasoning_override: Option<ReasoningMode>,
        initial_calls: u32,
        reservation_run_id: Option<String>,
        reservation_sink: Option<Arc<dyn DurableCallReservationSink>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner,
            calls: AtomicU32::new(initial_calls),
            max_calls: budget.max_calls,
            max_turns: budget.max_turns,
            timeout_secs: budget.timeout_secs.max(1),
            max_tokens: budget.max_tokens,
            reasoning_override,
            samples: Mutex::new(Vec::new()),
            turn_tag: Mutex::new("boot".into()),
            role_label: Mutex::new("pipeline".into()),
            evidence_turn_index: AtomicU32::new(0),
            reservation_gate: Mutex::new(()),
            reservation_run_id,
            reservation_sink,
        })
    }

    pub fn reasoning_override(&self) -> Option<ReasoningMode> {
        self.reasoning_override.clone()
    }

    pub fn set_tag(&self, tag: impl Into<String>) {
        *self.turn_tag.lock().expect("turn_tag lock") = tag.into();
    }

    pub fn set_role(&self, role: impl Into<String>) {
        *self.role_label.lock().expect("role lock") = role.into();
    }

    pub fn set_evidence_turn_index(&self, turn_index: u32) {
        self.evidence_turn_index.store(turn_index, Ordering::SeqCst);
    }

    pub fn calls_used(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn max_calls(&self) -> u32 {
        self.max_calls
    }

    pub fn max_turns(&self) -> u32 {
        self.max_turns
    }

    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs
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

    fn reserve_call(&self, req: &ChatRequest, streaming: bool) -> Result<ReservedCall, LlmError> {
        let _gate = self
            .reservation_gate
            .lock()
            .map_err(|_| LlmError::Internal("eval reservation gate poisoned".into()))?;
        // 先占位再调用，避免并发超支
        let prev = self.calls.load(Ordering::SeqCst);
        if prev >= self.max_calls {
            return Err(LlmError::Internal(format!(
                "eval budget exhausted: max_calls={} already used={}",
                self.max_calls, prev
            )));
        }
        let call_index = prev.saturating_add(1);
        let reserved = ReservedCall {
            call_index,
            turn_index: self.evidence_turn_index.load(Ordering::SeqCst),
            tag: self.turn_tag.lock().expect("turn_tag lock").clone(),
            role: self.role_label.lock().expect("role lock").clone(),
            streaming,
            request_fp16: short_hash16(&fingerprint_chat_request(req)),
        };
        if let Some(sink) = self.reservation_sink.as_ref() {
            let run_id = self.reservation_run_id.as_ref().ok_or_else(|| {
                LlmError::Internal("durable reservation run identity is missing".into())
            })?;
            let record = CallReservationRecord {
                schema_version: CALL_RESERVATION_SCHEMA_VERSION.into(),
                run_id: run_id.clone(),
                call_index,
                turn_index: reserved.turn_index,
                tag: reserved.tag.clone(),
                role: reserved.role.clone(),
                streaming: reserved.streaming,
                request_fp16: reserved.request_fp16.clone(),
                recorded_at_unix_ms: now_unix_ms(),
            };
            sink.reserve(&record).map_err(|_| {
                LlmError::Internal("durable eval call reservation failed closed".into())
            })?;
        }
        self.calls.store(call_index, Ordering::SeqCst);
        Ok(reserved)
    }

    fn effective_request(&self, req: &ChatRequest) -> ChatRequest {
        let mut effective = req.clone();
        if let Some(max_tokens) = self.max_tokens {
            effective.params.max_tokens = Some(max_tokens);
            effective.params.max_tokens_explicit = true;
        } else {
            // Endurance defaults to provider-managed output sizing. Clear any
            // legacy profile cap (notably 4096) unless the operator explicitly
            // opted into STORYFORGE_EVAL_MAX_TOKENS.
            effective.params.max_tokens = None;
            effective.params.max_tokens_explicit = false;
        }
        if let Some(reasoning) = &self.reasoning_override {
            effective.params.reasoning = reasoning.clone();
        }
        effective
    }

    fn record(
        &self,
        reserved: &ReservedCall,
        req: &ChatRequest,
        resp: Option<&ChatResponse>,
        usage: Option<Usage>,
        elapsed_ms: u128,
        outcome: &str,
    ) {
        let segs = messages_segment_summary(&req.messages);
        // 含 model / tools / sampling，避免仅 message 正文导致假稳定
        let usage = usage.unwrap_or(Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let (tools_offered, tool_steps) = extract_tool_trace(req, resp);
        let sample = UsageSample {
            call_index: reserved.call_index,
            evidence_turn_index: reserved.turn_index,
            tag: reserved.tag.clone(),
            role: reserved.role.clone(),
            streaming: reserved.streaming,
            prompt_tokens: usage.prompt_tokens,
            cached_tokens: usage.cached_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            completion_tokens: usage.completion_tokens,
            request_fp16: reserved.request_fp16.clone(),
            system_hash16: segs.system_hash.chars().take(16).collect(),
            history_hash16: segs.history_hash.chars().take(16).collect(),
            tail_hash16: segs.tail_hash.chars().take(16).collect(),
            history_len: segs.history_len,
            tail_parts: segs.tail_parts,
            msg_count: req.messages.len(),
            elapsed_ms,
            outcome: outcome.into(),
            tools_offered,
            tool_steps,
        };
        let tool_summary = if sample.tools_offered.is_empty() && sample.tool_steps.is_empty() {
            String::new()
        } else {
            format!(
                " tools_offered={} tool_steps={}",
                sample.tools_offered.len(),
                sample.tool_steps.len()
            )
        };
        eprintln!(
            "[eval-budget] call={} tag={} stream={} outcome={} prompt={} cached={} completion={} ms={}{}",
            self.calls_used(),
            sample.tag,
            sample.streaming,
            sample.outcome,
            sample.prompt_tokens,
            sample.cached_tokens,
            sample.completion_tokens,
            sample.elapsed_ms,
            tool_summary
        );
        self.samples.lock().expect("samples lock").push(sample);
    }
}

#[async_trait]
impl LlmClient for BudgetedLlmClient {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let req = self.effective_request(req);
        let reserved = self.reserve_call(&req, false)?;
        let t0 = Instant::now();
        let fut = self.inner.chat(&req);
        let resp = match tokio::time::timeout(Duration::from_secs(self.timeout_secs), fut).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                self.record(
                    &reserved,
                    &req,
                    None,
                    None,
                    t0.elapsed().as_millis(),
                    "client_error",
                );
                return Err(e);
            }
            Err(_) => {
                self.record(
                    &reserved,
                    &req,
                    None,
                    None,
                    t0.elapsed().as_millis(),
                    "timeout",
                );
                return Err(LlmError::Timeout);
            }
        };
        self.record(
            &reserved,
            &req,
            Some(&resp),
            resp.usage.clone(),
            t0.elapsed().as_millis(),
            "ok",
        );
        Ok(resp)
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        let req = self.effective_request(req);
        let reserved = self.reserve_call(&req, true)?;
        let t0 = Instant::now();
        let fut = self.inner.chat_stream(&req, tx, cancel);
        let resp = match tokio::time::timeout(Duration::from_secs(self.timeout_secs), fut).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                self.record(
                    &reserved,
                    &req,
                    None,
                    None,
                    t0.elapsed().as_millis(),
                    "client_error",
                );
                return Err(e);
            }
            Err(_) => {
                self.record(
                    &reserved,
                    &req,
                    None,
                    None,
                    t0.elapsed().as_millis(),
                    "timeout",
                );
                return Err(LlmError::Timeout);
            }
        };
        self.record(
            &reserved,
            &req,
            Some(&resp),
            resp.usage.clone(),
            t0.elapsed().as_millis(),
            "ok",
        );
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::ChatMessage;
    use storyforge_infra_llm::mock_client::MockLlmClient;

    #[derive(Default)]
    struct RecordingReservationSink {
        records: Mutex<Vec<CallReservationRecord>>,
        fail: bool,
    }

    impl DurableCallReservationSink for RecordingReservationSink {
        fn reserve(&self, record: &CallReservationRecord) -> Result<(), String> {
            if self.fail {
                return Err("injected".into());
            }
            self.records.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

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
            max_tokens: None,
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
    async fn durable_reservation_precedes_dispatch_and_survives_dropped_future() {
        struct HangingClient {
            invoked: Arc<AtomicU32>,
        }
        #[async_trait]
        impl LlmClient for HangingClient {
            async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
                self.invoked.fetch_add(1, Ordering::SeqCst);
                std::future::pending().await
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

        let invoked = Arc::new(AtomicU32::new(0));
        let sink = Arc::new(RecordingReservationSink::default());
        let budget = RealLlmRunBudget {
            enabled: true,
            max_calls: 5,
            max_turns: 1,
            timeout_secs: 30,
            max_tokens: None,
        };
        let client = BudgetedLlmClient::wrap_with_reasoning_and_reservations(
            Arc::new(HangingClient {
                invoked: invoked.clone(),
            }),
            &budget,
            None,
            3,
            "run-canary-00000000-0000-0000-0000-000000000001",
            sink.clone(),
        )
        .unwrap();
        client.set_evidence_turn_index(1);
        let result =
            tokio::time::timeout(Duration::from_millis(20), client.chat(&dummy_req())).await;
        assert!(
            result.is_err(),
            "outer timeout must drop the hanging future"
        );
        assert_eq!(invoked.load(Ordering::SeqCst), 1);
        assert_eq!(client.calls_used(), 4);
        assert!(client.samples().is_empty());
        let records = sink.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].call_index, 4);
        assert_eq!(records[0].turn_index, 1);
    }

    #[tokio::test]
    async fn reservation_failure_prevents_provider_dispatch() {
        struct CountingClient(Arc<AtomicU32>);
        #[async_trait]
        impl LlmClient for CountingClient {
            async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(ChatResponse {
                    content: "should-not-run".into(),
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
        let invoked = Arc::new(AtomicU32::new(0));
        let sink = Arc::new(RecordingReservationSink {
            records: Mutex::new(Vec::new()),
            fail: true,
        });
        let budget = RealLlmRunBudget {
            enabled: true,
            max_calls: 1,
            max_turns: 1,
            timeout_secs: 30,
            max_tokens: None,
        };
        let client = BudgetedLlmClient::wrap_with_reasoning_and_reservations(
            Arc::new(CountingClient(invoked.clone())),
            &budget,
            None,
            0,
            "run-canary-00000000-0000-0000-0000-000000000002",
            sink,
        )
        .unwrap();
        assert!(client.chat(&dummy_req()).await.is_err());
        assert_eq!(invoked.load(Ordering::SeqCst), 0);
        assert_eq!(client.calls_used(), 0);
    }

    #[test]
    fn budget_applies_explicit_max_tokens_to_effective_request_only() {
        let mut source = dummy_req();
        source.params.max_tokens = None;
        let budget = RealLlmRunBudget {
            max_tokens: Some(384_000),
            ..RealLlmRunBudget::default()
        };
        let client = BudgetedLlmClient::wrap(Arc::new(MockLlmClient::with_defaults()), &budget);

        let effective = client.effective_request(&source);
        assert_eq!(effective.params.max_tokens, Some(384_000));
        assert_eq!(source.params.max_tokens, None);
    }

    #[test]
    fn omitted_eval_limit_clears_legacy_small_request_cap() {
        let mut source = dummy_req();
        source.params.max_tokens = Some(4096);
        source.params.max_tokens_explicit = false;
        let client = BudgetedLlmClient::wrap(
            Arc::new(MockLlmClient::with_defaults()),
            &RealLlmRunBudget::default(),
        );

        let effective = client.effective_request(&source);
        assert_eq!(effective.params.max_tokens, None);
        assert!(!effective.params.max_tokens_explicit);
        assert_eq!(source.params.max_tokens, Some(4096));
    }

    #[test]
    fn eval_reasoning_override_changes_effective_request() {
        let source = dummy_req();
        let client = BudgetedLlmClient::wrap_with_reasoning(
            Arc::new(MockLlmClient::with_defaults()),
            &RealLlmRunBudget::default(),
            Some(ReasoningMode::Native),
        );

        let effective = client.effective_request(&source);
        assert_eq!(effective.params.reasoning, ReasoningMode::Native);
        // SamplingParams::default 现为 Prompted；override 只改 effective 请求
        assert_eq!(source.params.reasoning, ReasoningMode::Prompted);
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
            max_turns: 1,
            timeout_secs: 1,
            max_tokens: None,
            reasoning_override: None,
            samples: Mutex::new(Vec::new()),
            turn_tag: Mutex::new("t".into()),
            role_label: Mutex::new("r".into()),
            evidence_turn_index: AtomicU32::new(0),
            reservation_gate: Mutex::new(()),
            reservation_run_id: None,
            reservation_sink: None,
        });
        let req = dummy_req();
        let result = client.chat(&req).await;
        assert!(
            matches!(result, Err(LlmError::Timeout)),
            "expected Timeout, got {result:?}"
        );
        assert_eq!(client.calls_used(), 1);
        let samples = client.samples();
        assert_eq!(samples.len(), 1, "timeout invocation must be recorded");
        assert_eq!(samples[0].outcome, "timeout");
    }
}

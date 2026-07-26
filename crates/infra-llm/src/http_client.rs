/// 真实 HTTP LLM 客户端（OpenAI 兼容协议 + SSE 流式）
///
/// 设计来源：
/// - TT 的 HttpChatCompletionRepository（reqwest + 自研 SSE 解析，streaming client 无 request timeout）
/// - TT 的 HttpClientPool（按用途分桶，我们简化为单 client）
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

use storyforge_domain::llm::{
    ChatRequest, ChatResponse, LlmConnection, LlmError, MAX_LLM_RESPONSE_BYTES,
    MAX_REASONING_BYTES, StreamChunk,
};

use crate::sse::{SseEventAccumulator, forward_sse_events};
use crate::text_tools::{inject_tool_prompt, parse_tool_calls_from_text};

/// Prompted/Native are auditable modes: the provider must surface reasoning.
/// Disabled remains compatible with providers that return answer text only.
fn validate_reasoning_capture(req: &ChatRequest, resp: &ChatResponse) -> Result<(), LlmError> {
    if let Some(reasoning) = resp.reasoning_content.as_deref()
        && reasoning.len() > MAX_REASONING_BYTES
    {
        return Err(LlmError::ReasoningTooLarge(format!(
            "mode={:?}, model={}, bytes={}, limit={MAX_REASONING_BYTES}",
            req.params.reasoning,
            req.model,
            reasoning.len()
        )));
    }
    if req.params.reasoning == storyforge_domain::llm::ReasoningMode::Disabled {
        let provider_disabled = req
            .params
            .extra
            .as_ref()
            .and_then(|extra| extra.get("thinking"))
            .and_then(|thinking| thinking.get("type"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("disabled"));
        if provider_disabled
            && resp
                .reasoning_content
                .as_deref()
                .is_some_and(|reasoning| !reasoning.trim().is_empty())
        {
            return Err(LlmError::UnexpectedReasoning(format!(
                "mode={:?}, model={}; provider thinking=disabled",
                req.params.reasoning, req.model
            )));
        }
        return Ok(());
    }
    if resp
        .reasoning_content
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
    {
        return Ok(());
    }
    Err(LlmError::MissingReasoning(format!(
        "mode={:?}, model={}；供应商响应未返回 reasoning_content/reasoning/thinking",
        req.params.reasoning, req.model
    )))
}

async fn read_response_body_limited(resp: reqwest::Response) -> Result<Vec<u8>, LlmError> {
    if resp
        .content_length()
        .is_some_and(|length| length > MAX_LLM_RESPONSE_BYTES as u64)
    {
        return Err(LlmError::ResponseTooLarge(format!(
            "content-length exceeds limit={MAX_LLM_RESPONSE_BYTES}"
        )));
    }

    let mut stream = resp.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| classify_reqwest_error(&error))?;
        extend_response_body_limited(&mut body, &chunk)?;
    }
    Ok(body)
}

fn extend_response_body_limited(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), LlmError> {
    let next = body.len().saturating_add(chunk.len());
    if next > MAX_LLM_RESPONSE_BYTES {
        return Err(LlmError::ResponseTooLarge(format!(
            "response bytes={next}, limit={MAX_LLM_RESPONSE_BYTES}"
        )));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn account_raw_response_bytes(total: &mut usize, additional: usize) -> Result<(), LlmError> {
    let next = total.saturating_add(additional);
    if next > MAX_LLM_RESPONSE_BYTES {
        return Err(LlmError::ResponseTooLarge(format!(
            "raw stream bytes={next}, limit={MAX_LLM_RESPONSE_BYTES}"
        )));
    }
    *total = next;
    Ok(())
}

/// HTTP LLM 客户端（OpenAI 兼容协议）
pub struct HttpLlmClient {
    /// 同步请求用的 client（有 request timeout）
    client: reqwest::Client,
    /// 流式请求用的 client（无 request timeout，仅 connect timeout）
    stream_client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    /// Provider-specific connection defaults applied at the final outbound boundary.
    default_extra: Option<serde_json::Map<String, serde_json::Value>>,
    /// 是否使用 text_tools 降级（提示词注入 + XML/JSON 解析）
    text_fallback: bool,
}

impl HttpLlmClient {
    /// 获取连接的默认模型名
    pub fn default_model(&self) -> &str {
        &self.model
    }

    /// 拉取服务商可用模型列表（GET /v1/models）
    ///
    /// 复用同步 client（带 timeout）。失败时返回错误（调用方可回退模板兜底）。
    pub async fn fetch_models(&self, raw_base_url: &str) -> Result<Vec<String>, LlmError> {
        let models_url = normalize_models_url(raw_base_url);
        debug!(target: "infra-llm", "fetch_models: GET {models_url}");

        let resp = self
            .client
            .get(&models_url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| classify_reqwest_error(&e))?;

        let status = resp.status();
        let response_body = read_response_body_limited(resp).await?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&response_body);
            return Err(classify_http_error(status.as_u16(), &text));
        }
        let json: serde_json::Value = serde_json::from_slice(&response_body)
            .map_err(|e| LlmError::Internal(format!("models JSON 解析失败: {e}")))?;

        // OpenAI 格式：{data: [{id: "model-name", ...}]}
        let models: Vec<String> = json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        info!(target: "infra-llm", "fetch_models: 拉取到 {} 个模型", models.len());
        Ok(models)
    }

    /// 从 LlmConnection 构造（M1 只支持 OpenAi 协议，其他协议预留）
    ///
    /// 返回 Result 而非 panic——连接配置错误（如构造 reqwest client 失败）
    /// 时应优雅返回错误，让用户看到提示而非崩溃。
    pub fn new(conn: &LlmConnection) -> Result<Self, LlmError> {
        // 部分兼容网关（Cloudflare）会拦默认 reqwest UA；统一浏览器 UA。
        const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 StoryForge/0.1";
        // 同步 client：connect timeout 30s + request timeout 默认 120s。
        // 慢中继/深推理模型（一次请求 2-3 分钟）可用 STORYFORGE_LLM_TIMEOUT_SECS 放宽，
        // 解析失败或未设置时保持 120s 不变。
        let request_timeout_secs = std::env::var("STORYFORGE_LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(120);
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(request_timeout_secs))
            .build()
            .map_err(|e| LlmError::Internal(format!("构建 reqwest client 失败: {e}")))?;

        // 流式 client：connect timeout 30s，无 request timeout（SSE 可能持续很久）
        let stream_client = reqwest::Client::builder()
            .user_agent(UA)
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Internal(format!("构建 stream client 失败: {e}")))?;

        // 确保 base_url 以 /v1/chat/completions 结尾
        let base_url = normalize_base_url(&conn.base_url);

        Ok(Self {
            client,
            stream_client,
            base_url,
            api_key: conn.api_key.clone(),
            model: conn.model.clone(),
            default_extra: conn.params.extra.clone(),
            text_fallback: false,
        })
    }

    /// 切换到 text_tools 降级模式
    pub fn with_text_fallback(mut self) -> Self {
        self.text_fallback = true;
        self
    }

    /// 确定实际使用的模型名
    ///
    /// 若 `req.model` 为空或等于占位默认值，则回退到连接配置的 `self.model`；
    /// 否则保留 `req.model`（来自 AgentProfileConfig 的用户 override）。
    fn effective_model<'a>(&'a self, req_model: &'a str) -> &'a str {
        const PLACEHOLDER_MODELS: &[&str] = &["deepseek-chat", "mock"];
        if req_model.is_empty() || PLACEHOLDER_MODELS.contains(&req_model) {
            &self.model
        } else {
            req_model
        }
    }

    fn apply_connection_defaults(&self, req: &mut ChatRequest) {
        let Some(defaults) = self.default_extra.as_ref() else {
            return;
        };
        let mut merged = defaults.clone();
        if let Some(request_extra) = req.params.extra.take() {
            merged.extend(request_extra);
        }
        req.params.extra = Some(merged);
    }

    /// 构建 Authorization header
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }
}

#[async_trait]
impl crate::LlmClient for HttpLlmClient {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let mut req = req.clone();
        // model 回退：连接配置的 model 优先于占位默认值
        req.model = self.effective_model(&req.model).to_string();
        self.apply_connection_defaults(&mut req);
        // text_tools 降级：注入工具提示到 system prompt，移除 tools 字段
        if self.text_fallback
            && let Some(tools) = &req.tools
        {
            inject_tool_prompt(&mut req.messages, tools);
            req.tools = None;
        }

        let body = crate::openai::build_request_body(&req);
        debug!(target: "infra-llm", "chat request: model={}, tools={}", req.model, req.tools.is_some());

        let resp = self
            .client
            .post(&self.base_url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| classify_reqwest_error(&e))?;

        let status = resp.status();
        let response_body = read_response_body_limited(resp).await?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&response_body);
            return Err(classify_http_error(status.as_u16(), &text));
        }

        let json: serde_json::Value = serde_json::from_slice(&response_body)
            .map_err(|e| LlmError::Internal(format!("JSON 解析失败: {e}")))?;

        let mut chat_resp = crate::openai::parse_response(&json)
            .map_err(|e| LlmError::Internal(format!("响应解析失败: {e}")))?;

        // text_tools 降级：从 content 中提取工具调用
        if self.text_fallback && chat_resp.tool_calls.is_empty() && !chat_resp.content.is_empty() {
            chat_resp.tool_calls = parse_tool_calls_from_text(&chat_resp.content);
        }

        validate_reasoning_capture(&req, &chat_resp)?;

        info!(target: "infra-llm", "chat done: content_len={}, tool_calls={}", chat_resp.content.len(), chat_resp.tool_calls.len());
        Ok(chat_resp)
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        let mut req = req.clone();
        // model 回退：连接配置的 model 优先于占位默认值
        req.model = self.effective_model(&req.model).to_string();
        self.apply_connection_defaults(&mut req);
        if self.text_fallback
            && let Some(tools) = &req.tools
        {
            inject_tool_prompt(&mut req.messages, tools);
            req.tools = None;
        }

        let body = crate::openai::build_stream_request_body(&req);
        debug!(target: "infra-llm", "chat_stream request: model={}", req.model);

        let resp = self
            .stream_client
            .post(&self.base_url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| classify_reqwest_error(&e))?;

        let status = resp.status();
        if !status.is_success() {
            let response_body = read_response_body_limited(resp).await?;
            let text = String::from_utf8_lossy(&response_body);
            return Err(classify_http_error(status.as_u16(), &text));
        }
        if resp
            .content_length()
            .is_some_and(|length| length > MAX_LLM_RESPONSE_BYTES as u64)
        {
            return Err(LlmError::ResponseTooLarge(format!(
                "stream content-length exceeds limit={MAX_LLM_RESPONSE_BYTES}"
            )));
        }

        // 流式读取：chunk → SSE 解析 → channel 推送
        // SseEventAccumulator 同时累积 full_content 和 tool_calls
        let mut stream = resp.bytes_stream();
        let mut buffer = Vec::new();
        let reasoning_required =
            req.params.reasoning != storyforge_domain::llm::ReasoningMode::Disabled;
        let mut accumulator = if reasoning_required {
            SseEventAccumulator::new_deferred()
        } else {
            SseEventAccumulator::new()
        };
        let mut cancel = cancel;
        let mut raw_response_bytes = 0usize;

        // H-5 流式空闲超时：正常流式 chunk 间隔远小于此（通常 <5s），
        // 仅当服务端建连后挂起不发数据（网络黑洞/代理挂起）才触发，判定死流。
        const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
        let mut last_chunk_at = std::time::Instant::now();

        // 进入循环前先检查初始取消状态（changed() 只在值变化后才触发）
        if *cancel.borrow() {
            warn!(target: "infra-llm", "stream cancelled before start");
            return Err(LlmError::Cancelled);
        }

        loop {
            // 计算到下次空闲超时的剩余时间
            let next_deadline = last_chunk_at + STREAM_IDLE_TIMEOUT;
            tokio::select! {
                // 取消信号
                _ = cancel.changed() => {
                    if *cancel.borrow() {
                        warn!(target: "infra-llm", "stream cancelled");
                        return Err(LlmError::Cancelled);
                    }
                }
                // 空闲超时：90s 无 chunk 判死流，防止连接永久泄漏
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(next_deadline)), if next_deadline > std::time::Instant::now() => {
                    warn!(target: "infra-llm", "stream 空闲超时（{}s 无数据），判定死流", STREAM_IDLE_TIMEOUT.as_secs());
                    return Err(LlmError::Http(format!("流式空闲超时（{}s 无 chunk）", STREAM_IDLE_TIMEOUT.as_secs())));
                }
                // 下一个 chunk
                chunk = stream.next() => {
                    // 收到任何 chunk（含网络分片）即重置空闲计时
                    last_chunk_at = std::time::Instant::now();
                    match chunk {
                        Some(Ok(bytes)) => {
                            account_raw_response_bytes(&mut raw_response_bytes, bytes.len())?;
                            forward_sse_events(&bytes, &mut buffer, &mut accumulator, &tx)
                                .map_err(|e| {
                                    error!(target: "infra-llm", "SSE 解析错误: {e}");
                                    e
                                })?;
                        }
                        Some(Err(e)) => {
                            error!(target: "infra-llm", "stream 读取错误: {e}");
                            return Err(LlmError::Http(format!("流读取失败: {e}")));
                        }
                        None => {
                            // 流结束，flush 剩余
                            accumulator.finish(&tx)?;
                            break;
                        }
                    }
                }
            }
        }

        // 用累积器的结果构造完整响应（不需要二次请求）
        let final_content = std::mem::take(&mut accumulator.full_content);
        let full_reasoning_content = std::mem::take(&mut accumulator.full_reasoning_content);
        let final_reasoning_content = if full_reasoning_content.trim().is_empty() {
            None
        } else {
            Some(full_reasoning_content)
        };
        let mut final_tool_calls: Vec<storyforge_domain::llm::ToolCall> = accumulator
            .all_tool_calls
            .drain(..)
            .map(|c| storyforge_domain::llm::ToolCall {
                id: c.id,
                call_type: "function".into(),
                function: storyforge_domain::llm::FunctionCall {
                    name: c.function.name,
                    arguments: c.function.arguments,
                },
            })
            .collect();

        // text_tools 降级：从 content 中提取工具调用
        if self.text_fallback && final_tool_calls.is_empty() && !final_content.is_empty() {
            final_tool_calls = parse_tool_calls_from_text(&final_content);
        }

        // 流式 usage：末 chunk 携带（需 stream_options.include_usage=true，已在 openai.rs 设置）
        let usage = accumulator.usage.take().unwrap_or_default();

        info!(target: "infra-llm", "chat_stream done: content_len={}, tool_calls={}", final_content.len(), final_tool_calls.len());

        let response = ChatResponse {
            content: final_content,
            reasoning_content: final_reasoning_content,
            tool_calls: final_tool_calls,
            finish_reason: accumulator.finish_reason.take(),
            usage: Some(usage),
        };
        validate_reasoning_capture(&req, &response)?;
        if reasoning_required {
            accumulator.flush_deferred(&tx);
        }
        Ok(response)
    }
}

/// 正规化 base_url 为完整的 chat completions 端点
fn normalize_base_url(base_url: &str) -> String {
    let url = base_url.trim_end_matches('/');
    if url.ends_with("/v1/chat/completions") {
        url.to_string()
    } else if url.ends_with("/v1") {
        format!("{url}/chat/completions")
    } else {
        format!("{url}/v1/chat/completions")
    }
}

/// 正规化 base_url 为 models 端点（GET /v1/models）
fn normalize_models_url(base_url: &str) -> String {
    let url = base_url.trim_end_matches('/');
    if url.ends_with("/v1/models") {
        url.to_string()
    } else if url.ends_with("/v1") {
        format!("{url}/models")
    } else if url.ends_with("/v1/chat/completions") {
        url.trim_end_matches("/chat/completions").to_string() + "/models"
    } else {
        format!("{url}/v1/models")
    }
}

/// 分类 reqwest 传输错误
fn classify_reqwest_error(e: &reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout
    } else if e.is_connect() {
        LlmError::Http(format!("连接失败: {e}"))
    } else {
        LlmError::Http(format!("请求失败: {e}"))
    }
}

/// 分类 HTTP 状态码错误
fn classify_http_error(status: u16, body: &str) -> LlmError {
    let msg = extract_error_message(body);
    match status {
        401 | 403 => LlmError::Auth(msg),
        400 => LlmError::BadRequest(msg),
        429 => LlmError::RateLimited(msg),
        500..=599 => LlmError::ServerError(msg),
        _ => LlmError::Internal(format!("HTTP {status}: {msg}")),
    }
}

/// 从错误响应 body 中提取 error.message
fn extract_error_message(body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        // OpenAI 格式: {"error": {"message": "..."}}
        if let Some(msg) = json
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return msg.to_string();
        }
        // 通用格式: {"message": "..."}
        if let Some(msg) = json.get("message").and_then(|m| m.as_str()) {
            return msg.to_string();
        }
    }
    body.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Id;
    use storyforge_domain::llm::{LlmProtocol, SamplingParams, ToolMode};

    fn make_client(model: &str) -> HttpLlmClient {
        make_client_with_params(model, SamplingParams::default())
    }

    fn make_client_with_params(model: &str, params: SamplingParams) -> HttpLlmClient {
        let conn = LlmConnection {
            id: Id::from_str("test-conn"),
            name: "test".into(),
            base_url: "http://localhost/v1/chat/completions".into(),
            api_key: "sk-test".into(),
            model: model.into(),
            protocol: LlmProtocol::OpenAi,
            params,
            tool_mode: ToolMode::Native,
        };
        HttpLlmClient::new(&conn).unwrap()
    }

    #[test]
    fn connection_provider_extra_applies_to_requests_without_extra() {
        let mut params = SamplingParams::default();
        params.extra = Some(serde_json::Map::from_iter([(
            "thinking".into(),
            serde_json::json!({"type": "disabled"}),
        )]));
        let client = make_client_with_params("deepseek-v4-pro", params);
        let mut req = ChatRequest {
            messages: vec![],
            tools: None,
            params: SamplingParams::default(),
            model: "deepseek-v4-pro".into(),
        };

        client.apply_connection_defaults(&mut req);

        assert_eq!(req.params.extra.unwrap()["thinking"]["type"], "disabled");
    }

    #[test]
    fn request_provider_extra_overrides_connection_key_and_preserves_other_defaults() {
        let mut params = SamplingParams::default();
        params.extra = Some(serde_json::Map::from_iter([
            ("thinking".into(), serde_json::json!({"type": "disabled"})),
            ("vendor_flag".into(), serde_json::json!(true)),
        ]));
        let client = make_client_with_params("deepseek-v4-pro", params);
        let mut req = ChatRequest {
            messages: vec![],
            tools: None,
            params: SamplingParams {
                extra: Some(serde_json::Map::from_iter([(
                    "thinking".into(),
                    serde_json::json!({"type": "enabled"}),
                )])),
                ..SamplingParams::default()
            },
            model: "deepseek-v4-pro".into(),
        };

        client.apply_connection_defaults(&mut req);

        let extra = req.params.extra.unwrap();
        assert_eq!(extra["thinking"]["type"], "enabled");
        assert_eq!(extra["vendor_flag"], true);
    }

    #[test]
    fn effective_model_placeholder_deepseek_chat_falls_back() {
        let client = make_client("deepseek-v4-flash");
        assert_eq!(client.effective_model("deepseek-chat"), "deepseek-v4-flash");
    }

    #[test]
    fn effective_model_placeholder_mock_falls_back() {
        let client = make_client("gpt-4o");
        assert_eq!(client.effective_model("mock"), "gpt-4o");
    }

    #[test]
    fn effective_model_empty_falls_back() {
        let client = make_client("deepseek-v4-flash");
        assert_eq!(client.effective_model(""), "deepseek-v4-flash");
    }

    #[test]
    fn effective_model_non_placeholder_preserved() {
        let client = make_client("deepseek-v4-flash");
        // AgentProfileConfig override — non-placeholder should be preserved
        assert_eq!(client.effective_model("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn effective_model_same_as_connection_preserved() {
        let client = make_client("deepseek-v4-flash");
        // req.model == self.model, not a placeholder — preserved as-is
        assert_eq!(
            client.effective_model("deepseek-v4-flash"),
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn prompted_and_native_modes_fail_closed_without_reasoning() {
        for reasoning in [
            storyforge_domain::llm::ReasoningMode::Prompted,
            storyforge_domain::llm::ReasoningMode::Native,
        ] {
            let req = ChatRequest {
                messages: vec![],
                tools: None,
                params: SamplingParams {
                    reasoning,
                    ..Default::default()
                },
                model: "test".into(),
            };
            let resp = ChatResponse {
                content: "answer".into(),
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: Some("stop".into()),
                usage: None,
            };
            assert!(matches!(
                validate_reasoning_capture(&req, &resp),
                Err(LlmError::MissingReasoning(_))
            ));
        }
    }

    #[test]
    fn disabled_mode_allows_absent_reasoning_and_prompted_accepts_captured_reasoning() {
        let mut req = ChatRequest {
            messages: vec![],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Disabled,
                ..Default::default()
            },
            model: "test".into(),
        };
        let mut resp = ChatResponse {
            content: "answer".into(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: Some("stop".into()),
            usage: None,
        };
        assert!(validate_reasoning_capture(&req, &resp).is_ok());

        req.params.reasoning = storyforge_domain::llm::ReasoningMode::Prompted;
        resp.reasoning_content = Some("captured".into());
        assert!(validate_reasoning_capture(&req, &resp).is_ok());
    }

    #[test]
    fn explicit_provider_thinking_disabled_rejects_returned_reasoning() {
        let req = ChatRequest {
            messages: vec![],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Disabled,
                extra: Some(serde_json::Map::from_iter([(
                    "thinking".into(),
                    serde_json::json!({"type": "disabled"}),
                )])),
                ..Default::default()
            },
            model: "test".into(),
        };
        let resp = ChatResponse {
            content: "answer".into(),
            reasoning_content: Some("must not be returned".into()),
            tool_calls: vec![],
            finish_reason: Some("stop".into()),
            usage: None,
        };

        assert!(matches!(
            validate_reasoning_capture(&req, &resp),
            Err(LlmError::UnexpectedReasoning(_))
        ));
    }

    #[test]
    fn reasoning_over_limit_fails_closed_in_all_modes() {
        let req = ChatRequest {
            messages: vec![],
            tools: None,
            params: SamplingParams::default(),
            model: "test".into(),
        };
        let resp = ChatResponse {
            content: "answer".into(),
            reasoning_content: Some("x".repeat(MAX_REASONING_BYTES + 1)),
            tool_calls: vec![],
            finish_reason: Some("stop".into()),
            usage: None,
        };
        assert!(matches!(
            validate_reasoning_capture(&req, &resp),
            Err(LlmError::ReasoningTooLarge(_))
        ));
    }

    #[test]
    fn required_reasoning_missing_keeps_stream_channel_empty() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut accumulator = SseEventAccumulator::new_deferred();
        let mut buffer = Vec::new();
        let event = r#"data: {"choices":[{"delta":{"content":"must stay hidden"},"finish_reason":"stop"}]}

"#;
        forward_sse_events(event.as_bytes(), &mut buffer, &mut accumulator, &tx).unwrap();

        let req = ChatRequest {
            messages: vec![],
            tools: None,
            params: SamplingParams {
                reasoning: storyforge_domain::llm::ReasoningMode::Prompted,
                ..Default::default()
            },
            model: "test".into(),
        };
        let response = ChatResponse {
            content: std::mem::take(&mut accumulator.full_content),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: accumulator.finish_reason.take(),
            usage: None,
        };

        assert!(matches!(
            validate_reasoning_capture(&req, &response),
            Err(LlmError::MissingReasoning(_))
        ));
        assert!(
            rx.try_recv().is_err(),
            "rejected stream leaked visible content"
        );
    }

    #[test]
    fn non_stream_body_limit_is_checked_before_append() {
        let mut body = vec![0; MAX_LLM_RESPONSE_BYTES];
        let error = extend_response_body_limited(&mut body, &[1])
            .expect_err("oversized body must fail before append");
        assert!(matches!(error, LlmError::ResponseTooLarge(_)));
        assert_eq!(body.len(), MAX_LLM_RESPONSE_BYTES);
    }

    #[test]
    fn raw_stream_limit_is_checked_before_sse_buffering() {
        let mut total = MAX_LLM_RESPONSE_BYTES;
        let error = account_raw_response_bytes(&mut total, 1)
            .expect_err("oversized raw stream must fail before parser buffering");
        assert!(matches!(error, LlmError::ResponseTooLarge(_)));
        assert_eq!(total, MAX_LLM_RESPONSE_BYTES);
    }
}

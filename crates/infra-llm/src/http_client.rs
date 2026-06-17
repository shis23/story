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

use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmConnection, LlmError, StreamChunk};

use crate::sse::{SseEventAccumulator, forward_sse_events};
use crate::text_tools::{inject_tool_prompt, parse_tool_calls_from_text};

/// HTTP LLM 客户端（OpenAI 兼容协议）
pub struct HttpLlmClient {
    /// 同步请求用的 client（有 request timeout）
    client: reqwest::Client,
    /// 流式请求用的 client（无 request timeout，仅 connect timeout）
    stream_client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
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
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_http_error(status.as_u16(), &text));
        }

        let json: serde_json::Value = resp
            .json()
            .await
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
        // 同步 client：connect timeout 30s + request timeout 120s
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::Internal(format!("构建 reqwest client 失败: {e}")))?;

        // 流式 client：connect timeout 30s，无 request timeout（SSE 可能持续很久）
        let stream_client = reqwest::Client::builder()
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
            text_fallback: false,
        })
    }

    /// 切换到 text_tools 降级模式
    pub fn with_text_fallback(mut self) -> Self {
        self.text_fallback = true;
        self
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
        // text_tools 降级：注入工具提示到 system prompt，移除 tools 字段
        if self.text_fallback {
            if let Some(tools) = &req.tools {
                inject_tool_prompt(&mut req.messages, tools);
                req.tools = None;
            }
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
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_http_error(status.as_u16(), &text));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| LlmError::Internal(format!("JSON 解析失败: {e}")))?;

        let mut chat_resp = crate::openai::parse_response(&json)
            .map_err(|e| LlmError::Internal(format!("响应解析失败: {e}")))?;

        // text_tools 降级：从 content 中提取工具调用
        if self.text_fallback && chat_resp.tool_calls.is_empty() && !chat_resp.content.is_empty() {
            chat_resp.tool_calls = parse_tool_calls_from_text(&chat_resp.content);
        }

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
        if self.text_fallback {
            if let Some(tools) = &req.tools {
                inject_tool_prompt(&mut req.messages, tools);
                req.tools = None;
            }
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
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_http_error(status.as_u16(), &text));
        }

        // 流式读取：chunk → SSE 解析 → channel 推送
        // SseEventAccumulator 同时累积 full_content 和 tool_calls
        let mut stream = resp.bytes_stream();
        let mut buffer = Vec::new();
        let mut accumulator = SseEventAccumulator::new();
        let mut cancel = cancel;

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
        let final_content = accumulator.full_content;
        let mut final_tool_calls: Vec<storyforge_domain::llm::ToolCall> = accumulator
            .all_tool_calls
            .into_iter()
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
        let usage = accumulator.usage.unwrap_or_default();

        info!(target: "infra-llm", "chat_stream done: content_len={}, tool_calls={}", final_content.len(), final_tool_calls.len());

        Ok(ChatResponse {
            content: final_content,
            tool_calls: final_tool_calls,
            finish_reason: accumulator.finish_reason,
            usage: Some(usage),
        })
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

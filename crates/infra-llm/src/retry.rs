/// LLM 调用自动重试层
///
/// 对 RateLimited / ServerError / Timeout 三类可重试错误执行指数退避重试。
/// 其他错误（Auth / BadRequest / Cancelled / StreamParse / Http / Internal）立即返回。
///
/// 设计要点：
/// - 实现 `LlmClient` trait，对调用方完全透明
/// - RateLimited 错误优先尊重响应头 Retry-After（以秒为单位）
/// - 流式调用（chat_stream）只在建连阶段重试，已开始推送 chunk 后不重试
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use tracing::warn;

use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmError, RetryConfig, StreamChunk};

use crate::LlmClient;

/// 带重试能力的 LLM 客户端包装器
pub struct RetryingClient {
    inner: Arc<dyn LlmClient>,
    config: RetryConfig,
}

/// Retry-After / 指数退避的等待上限（Gate 8 审查 P2-B2）：服务端异常值或
/// 恶意大值（如 `retry-after: 3600`）不得让请求挂起一小时；取消也无法打断
/// 一次极长 sleep。
const MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

impl RetryingClient {
    pub fn new(inner: Arc<dyn LlmClient>, config: RetryConfig) -> Self {
        Self { inner, config }
    }

    /// 从错误中提取 Retry-After 秒数（若存在），上限 60s。
    fn parse_retry_after(error: &LlmError) -> Option<u64> {
        match error {
            LlmError::RateLimited(msg) => {
                // 尝试在消息末尾找 "retry-after: Ns" 或纯数字
                if let Some(idx) = msg.rfind("retry-after:") {
                    let rest = &msg[idx + "retry-after:".len()..];
                    let num_str = rest.trim().trim_end_matches('s');
                    if let Ok(secs) = num_str.parse::<u64>() {
                        return Some(secs.min(MAX_WAIT.as_secs()));
                    }
                }
                // 尝试整个消息是纯数字
                if let Ok(secs) = msg.trim().parse::<u64>() {
                    return Some(secs.min(MAX_WAIT.as_secs()));
                }
                None
            }
            _ => None,
        }
    }

    /// 计算第 `attempt` 次重试的退避时间（0-indexed），上限 60s。
    fn backoff_duration(&self, attempt: u32) -> std::time::Duration {
        let ms = self.config.base_backoff_ms.saturating_mul(1u64 << attempt);
        std::time::Duration::from_millis(ms).min(MAX_WAIT)
    }

    /// 退避等待，期间以 ≤100ms 分片轮询 cancel 信号。
    ///
    /// 长退避（Retry-After / 指数）不得阻塞取消：cancel 置位后尽快返回
    /// `LlmError::Cancelled`（Gate 8 审查 P2-B2）。
    async fn backoff_wait(
        wait: std::time::Duration,
        cancel: &watch::Receiver<bool>,
    ) -> Result<(), LlmError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if *cancel.borrow() {
                return Err(LlmError::Cancelled);
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let remaining = (deadline - now).min(std::time::Duration::from_millis(100));
            tokio::time::sleep(remaining).await;
        }
    }
}

#[async_trait]
impl LlmClient for RetryingClient {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for attempt in 0..=self.config.max_retries {
            // 非首次调用时退避等待
            if attempt > 0 {
                let err_ref = last_error.as_ref().unwrap();

                // RateLimited 优先尊重 Retry-After
                let wait = Self::parse_retry_after(err_ref)
                    .map(std::time::Duration::from_secs)
                    .unwrap_or_else(|| self.backoff_duration(attempt - 1));

                warn!(
                    target: "infra-llm",
                    "chat 重试 {attempt}/{}，等待 {:?}（错误: {}）",
                    self.config.max_retries,
                    wait,
                    err_ref,
                );
                tokio::time::sleep(wait).await;
            }

            match self.inner.chat(req).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if !e.is_retryable() || attempt == self.config.max_retries {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
            }
        }

        // 无法到达（loop 覆盖了所有分支），但编译器需要
        Err(last_error.unwrap())
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                let err_ref = last_error.as_ref().unwrap();
                let wait = Self::parse_retry_after(err_ref)
                    .map(std::time::Duration::from_secs)
                    .unwrap_or_else(|| self.backoff_duration(attempt - 1));

                warn!(
                    target: "infra-llm",
                    "chat_stream 重试 {attempt}/{}，等待 {:?}（错误: {}）",
                    self.config.max_retries,
                    wait,
                    err_ref,
                );
                // 退避期间感知取消（分片轮询），取消置位立即返回 Cancelled。
                Self::backoff_wait(wait, &cancel).await?;
            }

            // 每次重试都需要新的 cancel receiver（clone）
            let cancel_clone = cancel.clone();
            match self.inner.chat_stream(req, tx.clone(), cancel_clone).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if !e.is_retryable() || attempt == self.config.max_retries {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap())
    }
}

/// 将 `Box<dyn LlmClient>` 转换为带重试的 `Arc<dyn LlmClient>`
pub fn with_retry(client: Box<dyn LlmClient>, config: RetryConfig) -> Arc<dyn LlmClient> {
    Arc::new(RetryingClient::new(Arc::from(client), config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 模拟会在前 N 次返回错误、之后成功的客户端
    struct FailNTimesClient {
        fail_count: AtomicU32,
        fail_with: LlmError,
    }

    impl FailNTimesClient {
        fn new(fail_count: u32, fail_with: LlmError) -> Self {
            Self {
                fail_count: AtomicU32::new(fail_count),
                fail_with,
            }
        }
    }

    #[async_trait]
    impl LlmClient for FailNTimesClient {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            let remaining = self.fail_count.fetch_sub(1, Ordering::SeqCst);
            if remaining > 0 {
                Err(self.fail_with.clone_error())
            } else {
                Ok(ChatResponse {
                    content: "ok".into(),
                    reasoning_content: None,
                    tool_calls: vec![],
                    finish_reason: Some("stop".into()),
                    usage: None,
                })
            }
        }

        async fn chat_stream(
            &self,
            req: &ChatRequest,
            tx: mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            // 简化：直接委托 chat 逻辑
            let result = self.chat(req).await;
            if let Ok(ref resp) = result {
                let _ = tx.send(StreamChunk {
                    delta_content: Some(resp.content.clone()),
                    delta_reasoning_content: resp.reasoning_content.clone(),
                    delta_tool_calls: None,
                    finish_reason: Some("stop".into()),
                });
            }
            result
        }
    }

    // LlmError 没有 Clone，需要手动复制
    trait CloneError {
        fn clone_error(&self) -> LlmError;
    }

    impl CloneError for LlmError {
        fn clone_error(&self) -> LlmError {
            match self {
                LlmError::Http(s) => LlmError::Http(s.clone()),
                LlmError::Auth(s) => LlmError::Auth(s.clone()),
                LlmError::BadRequest(s) => LlmError::BadRequest(s.clone()),
                LlmError::RateLimited(s) => LlmError::RateLimited(s.clone()),
                LlmError::ServerError(s) => LlmError::ServerError(s.clone()),
                LlmError::StreamParse(s) => LlmError::StreamParse(s.clone()),
                LlmError::MissingReasoning(s) => LlmError::MissingReasoning(s.clone()),
                LlmError::UnexpectedReasoning(s) => LlmError::UnexpectedReasoning(s.clone()),
                LlmError::ReasoningTooLarge(s) => LlmError::ReasoningTooLarge(s.clone()),
                LlmError::ResponseTooLarge(s) => LlmError::ResponseTooLarge(s.clone()),
                LlmError::Cancelled => LlmError::Cancelled,
                LlmError::Timeout => LlmError::Timeout,
                LlmError::Internal(s) => LlmError::Internal(s.clone()),
            }
        }
    }

    fn make_request() -> ChatRequest {
        ChatRequest {
            messages: vec![],
            tools: None,
            params: Default::default(),
            model: "test".into(),
        }
    }

    #[tokio::test]
    async fn retry_succeeds_after_failures() {
        // 2 次失败后成功
        let inner = Arc::new(FailNTimesClient::new(2, LlmError::Timeout));
        let client = RetryingClient::new(
            inner,
            RetryConfig {
                max_retries: 3,
                base_backoff_ms: 10, // 极短退避，加速测试
            },
        );

        let req = make_request();
        let result = client.chat(&req).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content, "ok");
    }

    #[tokio::test]
    async fn retry_exhausted_returns_error() {
        // 4 次失败，但 max_retries=2（共 3 次尝试）
        let inner = Arc::new(FailNTimesClient::new(
            4,
            LlmError::ServerError("500".into()),
        ));
        let client = RetryingClient::new(
            inner,
            RetryConfig {
                max_retries: 2,
                base_backoff_ms: 10,
            },
        );

        let req = make_request();
        let err = client.chat(&req).await.unwrap_err();
        match err {
            LlmError::ServerError(_) => {}
            other => panic!("expected ServerError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_retryable_error_not_retried() {
        let inner = Arc::new(FailNTimesClient::new(3, LlmError::Auth("401".into())));
        let client = RetryingClient::new(
            inner,
            RetryConfig {
                max_retries: 3,
                base_backoff_ms: 10,
            },
        );

        let req = make_request();
        let err = client.chat(&req).await.unwrap_err();
        // 应该立即返回，不重试
        match err {
            LlmError::Auth(_) => {}
            other => panic!("expected Auth, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn retry_zero_means_no_retry() {
        let inner = Arc::new(FailNTimesClient::new(1, LlmError::Timeout));
        let client = RetryingClient::new(
            inner,
            RetryConfig {
                max_retries: 0,
                base_backoff_ms: 10,
            },
        );

        let req = make_request();
        let err = client.chat(&req).await.unwrap_err();
        assert!(matches!(err, LlmError::Timeout));
    }

    #[test]
    fn parse_retry_after_seconds() {
        let err = LlmError::RateLimited("rate limited retry-after: 30s".into());
        assert_eq!(RetryingClient::parse_retry_after(&err), Some(30));
    }

    #[test]
    fn parse_retry_after_plain_number() {
        let err = LlmError::RateLimited("429".into());
        // 429s > 60s 上限（Gate 8 审查 P2-B2），被 cap。
        assert_eq!(RetryingClient::parse_retry_after(&err), Some(60));
    }

    #[test]
    fn parse_retry_after_capped_at_60s() {
        // 服务端异常/恶意大值不得让请求挂起一小时。
        let err = LlmError::RateLimited("rate limited retry-after: 3600s".into());
        assert_eq!(RetryingClient::parse_retry_after(&err), Some(60));
    }

    #[test]
    fn parse_retry_after_none() {
        let err = LlmError::RateLimited("rate limited".into());
        assert_eq!(RetryingClient::parse_retry_after(&err), None);
    }

    #[test]
    fn backoff_capped_at_60s() {
        let client = RetryingClient::new(
            Arc::new(FailNTimesClient::new(0, LlmError::Timeout)),
            RetryConfig {
                max_retries: 3,
                base_backoff_ms: 30_000,
            },
        );
        // 30s → 60s（指数翻倍被上限截断，而非 120s）。
        assert_eq!(
            client.backoff_duration(1),
            std::time::Duration::from_secs(60)
        );
        assert_eq!(
            client.backoff_duration(2),
            std::time::Duration::from_secs(60)
        );
    }

    #[test]
    fn backoff_exponential() {
        let client = RetryingClient::new(
            Arc::new(FailNTimesClient::new(0, LlmError::Timeout)),
            RetryConfig {
                max_retries: 3,
                base_backoff_ms: 1000,
            },
        );

        assert_eq!(
            client.backoff_duration(0),
            std::time::Duration::from_millis(1000)
        );
        assert_eq!(
            client.backoff_duration(1),
            std::time::Duration::from_millis(2000)
        );
        assert_eq!(
            client.backoff_duration(2),
            std::time::Duration::from_millis(4000)
        );
    }

    #[tokio::test]
    async fn chat_stream_cancel_interrupts_backoff() {
        // 前 2 次失败触发退避（base 5s → 退避 5s + 10s），但 cancel 在退避
        // 期间置位：必须快速返回 Cancelled，而不是等完整个退避。
        let inner = Arc::new(FailNTimesClient::new(2, LlmError::Timeout));
        let client = RetryingClient::new(
            inner,
            RetryConfig {
                max_retries: 2,
                base_backoff_ms: 5_000,
            },
        );

        let (tx, _rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = watch::channel(false);

        let start = std::time::Instant::now();
        cancel_tx.send(true).unwrap();
        let result = client.chat_stream(&make_request(), tx, cancel_rx).await;
        let elapsed = start.elapsed();

        assert!(matches!(result, Err(LlmError::Cancelled)));
        // 远小于第一个退避（5s）：取消立即生效，而非等退避结束。
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "cancel should interrupt backoff promptly, took {elapsed:?}"
        );
    }
}

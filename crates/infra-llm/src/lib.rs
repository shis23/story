pub mod embedder;
pub mod http_client;
pub mod mock_client;
pub mod openai;
pub mod sse;
pub mod text_tools;

// 重新导出嵌入客户端
pub use embedder::{EmbedConfig, Embedder, default_embed_config};

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};

use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmError, StreamChunk};

/// LLM 客户端 trait（统一接口，HttpLlmClient 和 MockLlmClient 都实现它）
///
/// 设计来源：TT 的 ChatCompletionRepository trait，简化为两个方法。
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// 同步调用（等待完整响应）
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError>;

    /// 流式调用（通过 channel 逐块推送，支持取消）
    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError>;
}

// ─── 便捷构造 ─────────────────────────────────────────────────────────────

/// 从 LlmConnection 构造对应的 LlmClient 实现
///
/// 返回 Result：HttpLlmClient 构造（reqwest client build）可能失败，
/// 不应 panic——让调用方把错误返回给用户。
pub fn create_client(
    conn: &storyforge_domain::llm::LlmConnection,
) -> Result<Box<dyn LlmClient>, storyforge_domain::llm::LlmError> {
    match conn.tool_mode {
        storyforge_domain::llm::ToolMode::Native => {
            Ok(Box::new(http_client::HttpLlmClient::new(conn)?))
        }
        storyforge_domain::llm::ToolMode::TextFallback => Ok(Box::new(
            http_client::HttpLlmClient::new(conn)?.with_text_fallback(),
        )),
    }
}

/// 拉取服务商可用模型列表（GET /v1/models）
///
/// 用临时构造的 HttpLlmClient 调 fetch_models。
/// 失败时返回错误字符串（调用方回退模板兜底）。
pub async fn fetch_models(
    conn: &storyforge_domain::llm::LlmConnection,
) -> Result<Vec<String>, String> {
    let client =
        http_client::HttpLlmClient::new(conn).map_err(|e| format!("构造客户端失败: {e}"))?;
    client
        .fetch_models(&conn.base_url)
        .await
        .map_err(|e| format!("{e}"))
}

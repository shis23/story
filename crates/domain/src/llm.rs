use serde::{Deserialize, Serialize};

use crate::Id;

// ─── 连接配置（对应设计 §5 LlmConnection，M1 简化版）──────────────────

/// LLM 连接配置
///
/// H-4：手写 Debug 打码 api_key。
/// 展示给前端用 LlmConnectionSummary（已剥离 key）。
#[derive(Clone, Serialize, Deserialize)]
pub struct LlmConnection {
    pub id: Id,
    pub name: String,
    pub base_url: String,
    /// 运行时为真实 API key；持久化时可为 SecretRef，由上层 store 解析。
    pub api_key: String,
    pub model: String,
    pub protocol: LlmProtocol,
    pub params: SamplingParams,
    pub tool_mode: ToolMode,
}

impl std::fmt::Debug for LlmConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConnection")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &"***")
            .field("model", &self.model)
            .field("protocol", &self.protocol)
            .field("params", &self.params)
            .field("tool_mode", &self.tool_mode)
            .finish()
    }
}

/// LLM 协议
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmProtocol {
    OpenAi,
    Anthropic,
    Gemini,
    Custom(String),
}

/// 采样参数
///
/// `extra` 承载厂商扩展参数（P3-3），透传到请求 JSON 顶层。用于 thinking、
/// reasoning_effort 等非标准字段，避免每加一个扩展就改结构体。值为 JSON，
/// 支持 `{"type":"enabled"}`（thinking）或 `"max"`（reasoning_effort）等任意形态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingParams {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    /// 厂商扩展参数（透传到请求体顶层），key=字段名，value=任意 JSON。
    /// 例：`{"thinking": {"type":"enabled"}, "reasoning_effort": "max"}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: Some(1.0),
            top_p: Some(0.95),
            max_tokens: Some(4096),
            extra: None,
        }
    }
}

/// 工具协议模式
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolMode {
    /// 原生 function calling（OpenAI 兼容 API 的 tools 字段）
    #[default]
    Native,
    /// XML/JSON 降级（提示词注入工具说明，模型输出 XML/JSON 指令）
    TextFallback,
}

// ─── 连接模板（对应设计 §5 ConnectionTemplate）──────────────────────────

/// 连接模板（用户选"DeepSeek"就自动填好 URL/协议/默认模型，只需填 key）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTemplate {
    /// 模板标识（如 "deepseek" / "siliconflow" / "openai" / "custom"）
    pub id: String,
    /// 展示名
    pub name: String,
    /// base_url（不含 /v1/chat/completions，HttpLlmClient 会自动补全）
    pub base_url: String,
    /// 协议（M1 只支持 OpenAi 兼容）
    pub protocol: LlmProtocol,
    /// 默认模型
    pub default_model: String,
    /// 可选模型列表（供前端下拉）
    pub models: Vec<String>,
    /// 申请 key 的提示
    pub get_key_hint: String,
    /// 默认工具模式
    pub tool_mode: ToolMode,
}

/// 内置连接模板（M1 只含 OpenAI 兼容协议的；Anthropic/Gemini 原生留 TODO）
///
/// 注：仅 OpenAi 协议的模板可用——HttpLlmClient 当前只实现了 OpenAI 兼容。
/// Anthropic/Gemini 原生协议需各自的请求格式转换，留待后续。
pub fn builtin_connection_templates() -> Vec<ConnectionTemplate> {
    vec![
        ConnectionTemplate {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com".into(),
            protocol: LlmProtocol::OpenAi,
            default_model: "deepseek-chat".into(),
            models: vec!["deepseek-chat".into(), "deepseek-reasoner".into()],
            get_key_hint: "到 platform.deepseek.com 申请".into(),
            tool_mode: ToolMode::Native,
        },
        ConnectionTemplate {
            id: "siliconflow".into(),
            name: "SiliconFlow（便宜）".into(),
            base_url: "https://api.siliconflow.cn".into(),
            protocol: LlmProtocol::OpenAi,
            default_model: "Qwen/Qwen2.5-72B-Instruct".into(),
            models: vec![
                "Qwen/Qwen2.5-72B-Instruct".into(),
                "Qwen/Qwen2.5-7B-Instruct".into(),
                "deepseek-ai/DeepSeek-V3".into(),
                "deepseek-ai/DeepSeek-R1".into(),
            ],
            get_key_hint: "到 cloud.siliconflow.cn 申请".into(),
            tool_mode: ToolMode::Native,
        },
        ConnectionTemplate {
            id: "openai".into(),
            name: "OpenAI".into(),
            base_url: "https://api.openai.com".into(),
            protocol: LlmProtocol::OpenAi,
            default_model: "gpt-4o-mini".into(),
            models: vec!["gpt-4o-mini".into(), "gpt-4o".into()],
            get_key_hint: "到 platform.openai.com 申请".into(),
            tool_mode: ToolMode::Native,
        },
        ConnectionTemplate {
            id: "custom".into(),
            name: "自定义（OpenAI 兼容）".into(),
            base_url: String::new(),
            protocol: LlmProtocol::OpenAi,
            default_model: String::new(),
            models: vec![],
            get_key_hint: "填入兼容 OpenAI 协议的服务地址".into(),
            tool_mode: ToolMode::Native,
        },
    ]
}

/// 连接摘要（给前端 list 用，不含 api_key）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConnectionSummary {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub protocol: LlmProtocol,
    pub tool_mode: ToolMode,
    /// 是否为当前活跃连接
    pub active: bool,
}

// ─── 聊天消息（发送给 LLM 的格式）───────────────────────────────────────

/// 聊天角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

/// 发送给 LLM 的单条消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// 原生 function calling 时：助手发出的工具调用
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// 工具角色消息：关联的工具调用 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// 工具调用（LLM 输出的）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// 函数调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String, // JSON 字符串
}

impl FunctionCall {
    /// 解析 arguments JSON 为具体类型
    pub fn parse_args<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.arguments)
    }
}

// ─── Agent 工具定义（发给 LLM 的 tools 数组）─────────────────────────────

/// Agent 可用的工具定义（对应 OpenAI function calling 的 tools 格式）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    pub tool_type: String, // "function"
    pub function: FunctionSpec,
}

/// 函数规格
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value, // JSON Schema
}

impl ToolSpec {
    pub fn function(
        name: impl Into<String>,
        desc: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        Self {
            tool_type: "function".into(),
            function: FunctionSpec {
                name: name.into(),
                description: desc.into(),
                parameters: params,
            },
        }
    }
}

// ─── 请求/响应（内部统一格式）────────────────────────────────────────────

/// LLM 请求（统一格式，infra-llm 层按协议转换）
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Option<Vec<ToolSpec>>,
    pub params: SamplingParams,
    pub model: String,
}

/// LLM 响应（统一格式）
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

/// Token 用量
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// 流式 SSE 块
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub delta_content: Option<String>,
    pub delta_tool_calls: Option<Vec<ToolCall>>,
    pub finish_reason: Option<String>,
}

/// LLM 调用错误
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("HTTP 请求失败: {0}")]
    Http(String),

    #[error("认证失败 (401/403): {0}")]
    Auth(String),

    #[error("请求无效 (400): {0}")]
    BadRequest(String),

    #[error("速率限制 (429): {0}")]
    RateLimited(String),

    #[error("服务端错误 (5xx): {0}")]
    ServerError(String),

    #[error("流式解析错误: {0}")]
    StreamParse(String),

    #[error("取消")]
    Cancelled,

    #[error("超时")]
    Timeout,

    #[error("内部错误: {0}")]
    Internal(String),
}

impl LlmError {
    /// 判断此错误是否值得重试（速率限制 / 服务端错误 / 超时）
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited(_) | Self::ServerError(_) | Self::Timeout
        )
    }
}

/// LLM 调用重试配置
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// 最大重试次数（不含首次调用；0 = 不重试）
    pub max_retries: u32,
    /// 初始退避时间（毫秒），每次翻倍：base, 2*base, 4*base ...
    pub base_backoff_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_backoff_ms: 1000,
        }
    }
}

impl RetryConfig {
    pub fn new(max_retries: u32, base_backoff_ms: u64) -> Self {
        Self {
            max_retries,
            base_backoff_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_connection_templates() {
        let templates = builtin_connection_templates();
        assert!(templates.len() >= 4, "至少 4 个模板");

        // DeepSeek 模板正确
        let ds = templates.iter().find(|t| t.id == "deepseek").unwrap();
        assert_eq!(ds.base_url, "https://api.deepseek.com");
        assert_eq!(ds.protocol, LlmProtocol::OpenAi);
        assert_eq!(ds.default_model, "deepseek-chat");
        assert_eq!(ds.tool_mode, ToolMode::Native);
        assert!(ds.models.contains(&"deepseek-chat".to_string()));

        // 所有内置模板都用 OpenAi 协议（M1 只实现这个）
        for t in &templates {
            assert_eq!(
                t.protocol,
                LlmProtocol::OpenAi,
                "模板 {} 应为 OpenAi 协议",
                t.id
            );
        }

        // 含 custom 兜底模板
        assert!(templates.iter().any(|t| t.id == "custom"));
    }

    #[test]
    fn test_tool_mode_default() {
        assert_eq!(ToolMode::default(), ToolMode::Native);
    }

    #[test]
    fn is_retryable_rate_limited() {
        assert!(LlmError::RateLimited("slow down".into()).is_retryable());
    }

    #[test]
    fn is_retryable_server_error() {
        assert!(LlmError::ServerError("502 bad gateway".into()).is_retryable());
    }

    #[test]
    fn is_retryable_timeout() {
        assert!(LlmError::Timeout.is_retryable());
    }

    #[test]
    fn is_not_retryable_auth() {
        assert!(!LlmError::Auth("401".into()).is_retryable());
    }

    #[test]
    fn is_not_retryable_bad_request() {
        assert!(!LlmError::BadRequest("invalid".into()).is_retryable());
    }

    #[test]
    fn is_not_retryable_cancelled() {
        assert!(!LlmError::Cancelled.is_retryable());
    }

    #[test]
    fn is_not_retryable_stream_parse() {
        assert!(!LlmError::StreamParse("bad json".into()).is_retryable());
    }

    #[test]
    fn is_not_retryable_http() {
        assert!(!LlmError::Http("connect failed".into()).is_retryable());
    }

    #[test]
    fn is_not_retryable_internal() {
        assert!(!LlmError::Internal("bug".into()).is_retryable());
    }

    #[test]
    fn retry_config_default() {
        let cfg = RetryConfig::default();
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.base_backoff_ms, 1000);
    }

    #[test]
    fn retry_config_custom() {
        let cfg = RetryConfig::new(5, 500);
        assert_eq!(cfg.max_retries, 5);
        assert_eq!(cfg.base_backoff_ms, 500);
    }
}

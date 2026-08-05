//! Structured error DTO for Tauri IPC commands.
//!
//! Preserves error type information through the Tauri IPC boundary so the
//! frontend can distinguish retryable from non-retryable errors.
//!
//! Frontend receives JSON like:
//! ```json
//! {"type":"llm","message":"Rate limited","retryable":true}
//! ```

use serde::Serialize;
use storyforge_domain::llm::LlmError;

// ─── Structured error DTO ─────────────────────────────────────────────────

/// Structured error returned from all Tauri commands.
///
/// The `type` field allows the frontend to handle different error categories
/// (e.g., retry on `retryable: true`, show auth dialog on `llm` without retry).
///
/// Serializes to JSON via `#[serde(tag = "type")]` so the variant name is
/// included as the `"type"` field.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TauriCommandError {
    /// Writing pipeline errors (AgentError, PipelineError).
    Pipeline { message: String, retryable: bool },
    /// LLM-specific errors (rate limited = retryable, auth = not).
    Llm { message: String, retryable: bool },
    /// Storage / IO errors.
    Storage { message: String },
    /// Input validation errors (bad args, parse failures).
    Validation { message: String },
    /// Resource not found.
    NotFound { message: String },
    /// User cancellation.
    Cancelled,
    /// Unexpected internal errors.
    Internal { message: String },
}

impl std::fmt::Display for TauriCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipeline { message, .. } => write!(f, "Pipeline error: {message}"),
            Self::Llm { message, .. } => write!(f, "LLM error: {message}"),
            Self::Storage { message } => write!(f, "Storage error: {message}"),
            Self::Validation { message } => write!(f, "Validation error: {message}"),
            Self::NotFound { message } => write!(f, "Not found: {message}"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Internal { message } => write!(f, "Internal error: {message}"),
        }
    }
}

impl std::error::Error for TauriCommandError {}

// ─── From<LlmError> ──────────────────────────────────────────────────────

impl From<LlmError> for TauriCommandError {
    fn from(e: LlmError) -> Self {
        match e {
            LlmError::RateLimited(msg) => Self::Llm {
                message: msg,
                retryable: true,
            },
            LlmError::Cancelled => Self::Cancelled,
            LlmError::Timeout => Self::Llm {
                message: "Request timed out".into(),
                retryable: true,
            },
            LlmError::Auth(msg) => Self::Llm {
                message: msg,
                retryable: false,
            },
            LlmError::ServerError(msg) => Self::Llm {
                message: msg,
                retryable: true,
            },
            other => Self::Llm {
                message: other.to_string(),
                retryable: false,
            },
        }
    }
}

// ─── From<AgentError> ────────────────────────────────────────────────────

impl From<storyforge_app_agent::AgentError> for TauriCommandError {
    fn from(e: storyforge_app_agent::AgentError) -> Self {
        match e {
            storyforge_app_agent::AgentError::Llm(llm_err) => Self::from(llm_err),
            storyforge_app_agent::AgentError::Cancelled => Self::Cancelled,
            storyforge_app_agent::AgentError::MaxRoundsExceeded => Self::Pipeline {
                message: "Agent exceeded maximum tool rounds".into(),
                retryable: true,
            },
            storyforge_app_agent::AgentError::PlanParse(msg) => Self::Pipeline {
                message: msg,
                retryable: false,
            },
            storyforge_app_agent::AgentError::SubagentFailed(msg) => Self::Pipeline {
                message: msg,
                retryable: true,
            },
        }
    }
}

// ─── From<PipelineError> ─────────────────────────────────────────────────

impl From<storyforge_app_pipeline::PipelineError> for TauriCommandError {
    fn from(e: storyforge_app_pipeline::PipelineError) -> Self {
        match e {
            storyforge_app_pipeline::PipelineError::Agent(agent_err) => Self::from(agent_err),
            storyforge_app_pipeline::PipelineError::Conversation(conv_err) => Self::Storage {
                message: conv_err.to_string(),
            },
            storyforge_app_pipeline::PipelineError::Llm(llm_err) => Self::from(llm_err),
            storyforge_app_pipeline::PipelineError::PlanParse(msg) => Self::Pipeline {
                message: msg,
                retryable: false,
            },
            storyforge_app_pipeline::PipelineError::Regex(msg) => Self::Pipeline {
                message: msg,
                retryable: false,
            },
            storyforge_app_pipeline::PipelineError::Regenerate(msg) => Self::Pipeline {
                message: msg,
                retryable: true,
            },
            storyforge_app_pipeline::PipelineError::Cancelled => Self::Cancelled,
            storyforge_app_pipeline::PipelineError::InvalidState(msg) => Self::Pipeline {
                message: msg,
                retryable: false,
            },
        }
    }
}

// ─── From<ImportError> ───────────────────────────────────────────────────

impl From<storyforge_infra_import::ImportError> for TauriCommandError {
    fn from(e: storyforge_infra_import::ImportError) -> Self {
        Self::Validation {
            message: e.to_string(),
        }
    }
}

// ─── From<ConversationError> ─────────────────────────────────────────────

impl From<storyforge_app_conversation::ConversationError> for TauriCommandError {
    fn from(e: storyforge_app_conversation::ConversationError) -> Self {
        match &e {
            storyforge_app_conversation::ConversationError::NotFound(_)
            | storyforge_app_conversation::ConversationError::NodeNotFound(_) => Self::NotFound {
                message: e.to_string(),
            },
            storyforge_app_conversation::ConversationError::Io(_) => Self::Storage {
                message: e.to_string(),
            },
            _ => Self::Validation {
                message: e.to_string(),
            },
        }
    }
}

// ─── From<MvuApplyError> ─────────────────────────────────────────────────

impl From<storyforge_app_meta::MvuApplyError> for TauriCommandError {
    fn from(e: storyforge_app_meta::MvuApplyError) -> Self {
        match &e {
            storyforge_app_meta::MvuApplyError::TranslationNotFound(_)
            | storyforge_app_meta::MvuApplyError::DefinitionNotFound(_) => Self::NotFound {
                message: e.to_string(),
            },
            storyforge_app_meta::MvuApplyError::NoChanges => Self::Validation {
                message: e.to_string(),
            },
        }
    }
}

// ─── From<String> (backward compatibility for existing .map_err(|e| format!(...))) ──

impl From<String> for TauriCommandError {
    fn from(s: String) -> Self {
        Self::Internal { message: s }
    }
}

impl From<&str> for TauriCommandError {
    fn from(s: &str) -> Self {
        Self::Internal {
            message: s.to_string(),
        }
    }
}

// ─── Convenience constructors ─────────────────────────────────────────────

impl TauriCommandError {
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound {
            message: msg.into(),
        }
    }

    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Storage {
            message: msg.into(),
        }
    }

    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation {
            message: msg.into(),
        }
    }

    pub fn pipeline(msg: impl Into<String>, retryable: bool) -> Self {
        Self::Pipeline {
            message: msg.into(),
            retryable,
        }
    }

    pub fn llm(msg: impl Into<String>, retryable: bool) -> Self {
        Self::Llm {
            message: msg.into(),
            retryable,
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal {
            message: msg.into(),
        }
    }
}

// ─── H-2 IPC input size limits ───────────────────────────────────────────
//
// Commands that accept a raw `String` JSON payload from the frontend (or a
// hijacked plugin iframe) must bound the size BEFORE `serde_json::from_str`,
// otherwise a super-large payload allocates heavily during IPC deserialization
// and can OOM the process. `import_character` (Vec<u8>) is bounded inside
// `infra-import` (100 MiB); these are the lower limits appropriate for
// structured config payloads that never legitimately approach those sizes.

/// Upper bound for a Campaign bundle JSON (cards + conversations + summaries).
pub const MAX_BUNDLE_JSON_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

/// Upper bound for structured config JSON (profiles, regex settings).
pub const MAX_CONFIG_JSON_BYTES: usize = 4 * 1024 * 1024; // 4 MiB

/// Reject an IPC payload exceeding `max_bytes`.
///
/// `label` names the command/kind in the error so users can identify which
/// import was rejected.
pub fn require_ipc_size(
    input: &str,
    max_bytes: usize,
    label: &str,
) -> Result<(), TauriCommandError> {
    if input.len() > max_bytes {
        return Err(TauriCommandError::validation(format!(
            "{label} 输入过大（{} 字节，上限 {} 字节）",
            input.len(),
            max_bytes
        )));
    }
    Ok(())
}

/// M-1：把绝对数据目录路径脱敏成 `<data_dir>/<filename>`，避免把
/// `C:\Users\<用户名>\...` 这样的绝对路径泄漏到前端错误信息里。
///
/// 服务器端日志（`tracing::error!`）仍可保留完整路径，但跨 IPC 返回的错误
/// 字符串只应包含文件名（帮助用户定位是哪个文件，但不暴露用户名/安装目录）。
pub fn sanitize_path_for_ipc(path: &std::path::Path) -> String {
    match path.file_name() {
        Some(name) => format!("<data_dir>/{}", name.to_string_lossy()),
        None => "<data_dir>".to_string(),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_pipeline_error() {
        let err = TauriCommandError::Pipeline {
            message: "Agent failed".into(),
            retryable: true,
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "pipeline");
        assert_eq!(json["message"], "Agent failed");
        assert_eq!(json["retryable"], true);
    }

    #[test]
    fn serialize_cancelled_error() {
        let err = TauriCommandError::Cancelled;
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "cancelled");
        assert!(json.get("message").is_none());
    }

    #[test]
    fn serialize_not_found_error() {
        let err = TauriCommandError::not_found("角色不存在");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "not_found");
        assert_eq!(json["message"], "角色不存在");
    }

    #[test]
    fn serialize_llm_rate_limited() {
        let llm_err = LlmError::RateLimited("429 Too Many Requests".into());
        let err = TauriCommandError::from(llm_err);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "llm");
        assert_eq!(json["retryable"], true);
    }

    #[test]
    fn serialize_llm_auth_error() {
        let llm_err = LlmError::Auth("Invalid API key".into());
        let err = TauriCommandError::from(llm_err);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "llm");
        assert_eq!(json["retryable"], false);
    }

    /// H-2: oversize IPC payloads must be rejected with a validation error.
    #[test]
    fn require_ipc_size_rejects_oversize_and_allows_within_limit() {
        assert!(require_ipc_size("{}", 16, "bundle").is_ok());
        let err = require_ipc_size(&"x".repeat(17), 16, "bundle").unwrap_err();
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "validation");
        assert!(json["message"].as_str().unwrap().contains("bundle"));
        assert!(json["message"].as_str().unwrap().contains("17"));
    }

    /// M-1: absolute paths must be redacted to `<data_dir>/<filename>` for IPC.
    #[test]
    fn sanitize_path_for_ipc_redacts_absolute_path() {
        let p = std::path::Path::new("C:/Users/Alice/AppData/storyforge/conversations/c-1.json");
        assert_eq!(sanitize_path_for_ipc(p), "<data_dir>/c-1.json");
        // Directory without file_name (root-ish) → just the placeholder.
        assert_eq!(
            sanitize_path_for_ipc(std::path::Path::new("/")),
            "<data_dir>"
        );
    }

    #[test]
    fn serialize_llm_cancelled() {
        let llm_err = LlmError::Cancelled;
        let err = TauriCommandError::from(llm_err);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "cancelled");
    }

    #[test]
    fn from_string_becomes_internal() {
        let err = TauriCommandError::from("something broke".to_string());
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "internal");
        assert_eq!(json["message"], "something broke");
    }

    #[test]
    fn display_trait() {
        let err = TauriCommandError::storage("disk full");
        assert_eq!(err.to_string(), "Storage error: disk full");
    }

    #[test]
    fn pipeline_cancelled() {
        let pipeline_err = storyforge_app_pipeline::PipelineError::Cancelled;
        let err = TauriCommandError::from(pipeline_err);
        assert!(matches!(err, TauriCommandError::Cancelled));
    }

    #[test]
    fn pipeline_agent_error() {
        let agent_err = storyforge_app_agent::AgentError::MaxRoundsExceeded;
        let pipeline_err = storyforge_app_pipeline::PipelineError::Agent(agent_err);
        let err = TauriCommandError::from(pipeline_err);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "pipeline");
        assert_eq!(json["retryable"], true);
    }

    #[test]
    fn pipeline_regex_error_is_not_retryable() {
        let pipeline_err = storyforge_app_pipeline::PipelineError::Regex("bad regex".into());
        let err = TauriCommandError::from(pipeline_err);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], "pipeline");
        assert_eq!(json["message"], "bad regex");
        assert_eq!(json["retryable"], false);
    }
}

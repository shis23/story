//! MVU 兜底执行运行时（对应设计 §19.6 / §8.4，AGENT_INTERFACES §8.4）
//!
//! 本 crate 只定义纯 trait、DTO、错误和事件协议；WebView runtime 位于 tauri-app。
//!
//! 设计意图：MVU 卡里 Meta Agent 翻译不了的 JS 片段（`fallback_fragments` 非空 +
//! `routing = Hybrid`）需要在一个执行环境里跑原 JS。
//!
//! 通信链路：
//!   Rust Tauri adapter `execute_fragment`
//!     → Tauri event `mvu:execute` → 前端 `MvuJsRuntime.vue` 监听
//!     → postMessage → iframe shim 执行 JS
//!     → postMessage 回传 → 前端 invoke `mvu_execute_result`
//!     → Tauri command handler → oneshot channel → Rust 拿到 `MvuExecResult`
//!
//! 安全约束：
//! - 只跑卡内本地 JS，不执行远程 JS（网络默认禁用）
//! - JS 输出经 preview/patch 确认回写，不直接写 store
//! - JS 失败不影响主写作（catch 后降级）

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// MVU 运行时执行结果（变量键 → 新值，由 JS 计算后回写）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuExecResult {
    /// JS 执行后产出的变量更新（键 → 值）
    pub variable_updates: HashMap<String, serde_json::Value>,
    /// 是否有副作用需注入下一轮（如触发事件）
    pub side_effects: Vec<String>,
}

/// MVU 运行时错误
#[derive(Debug, thiserror::Error)]
pub enum MvuRuntimeError {
    /// 当前未实现（桩状态）
    #[error("MVU 运行时未实现（共享 WebView 兜底为 W8 工作）")]
    NotImplemented,

    /// JS 执行错误
    #[error("JS 执行错误: {0}")]
    ExecutionFailed(String),

    /// 卡资产缺失
    #[error("卡资产缺失: {0}")]
    MissingAssets(String),

    /// 执行超时
    #[error("JS 执行超时 ({0}ms)")]
    Timeout(String),

    /// 通信通道错误
    #[error("MVU 通信错误: {0}")]
    ChannelError(String),
}

/// MVU 兜底执行运行时 trait（async）
///
/// 上层（写作流水线 / 后处理）依赖本 trait，实际实现可在运行时切换：
///   - `StubMvuRuntime`：桩，全部返回 NotImplemented（降级用）
///   - Tauri/WebView adapter：共享 WebView 执行（W8 实现，位于 tauri-app）
#[async_trait]
pub trait MvuRuntime: Send + Sync {
    /// 执行单个 fallback JS 片段，输入当前变量，输出变量更新
    ///
    /// `fragment_js` 是 [`FallbackFragment::js_snippet`]，
    /// `current_variables` 是当前角色实例 + Campaign 的变量快照。
    async fn execute_fragment(
        &self,
        fragment_js: &str,
        current_variables: &HashMap<String, serde_json::Value>,
    ) -> Result<MvuExecResult, MvuRuntimeError>;

    /// 加载一张卡的完整 JS 资产到共享 WebView（O(1) 内存，加载一次）
    ///
    /// 重 DOM 卡（如缄默之秋）的 18 万字符 JS 应只加载一次，
    /// 后续每轮只推 stat_data + 成文，让已加载的 JS 计算状态栏。
    async fn load_card_assets(
        &self,
        html: Option<&str>,
        css: Option<&str>,
        js: Option<&str>,
    ) -> Result<(), MvuRuntimeError>;

    /// 卸载某卡的资产（切卡时释放）
    async fn unload_card(&self) -> Result<(), MvuRuntimeError>;

    /// 是否已实现（桩返回 false，让上层据此决定是否提示用户）
    fn is_available(&self) -> bool;
}

// ─── 桩实现 ─────────────────────────────────────────────────────────────

/// 桩实现：全部返回 NotImplemented，`is_available` 返回 false
///
/// 上层默认用这个；等 WebView 实现就绪后替换。
pub struct StubMvuRuntime;

impl StubMvuRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubMvuRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MvuRuntime for StubMvuRuntime {
    async fn execute_fragment(
        &self,
        _fragment_js: &str,
        _current_variables: &HashMap<String, serde_json::Value>,
    ) -> Result<MvuExecResult, MvuRuntimeError> {
        Err(MvuRuntimeError::NotImplemented)
    }

    async fn load_card_assets(
        &self,
        _html: Option<&str>,
        _css: Option<&str>,
        _js: Option<&str>,
    ) -> Result<(), MvuRuntimeError> {
        Err(MvuRuntimeError::NotImplemented)
    }

    async fn unload_card(&self) -> Result<(), MvuRuntimeError> {
        Err(MvuRuntimeError::NotImplemented)
    }

    fn is_available(&self) -> bool {
        false
    }
}

// ─── WebView 实现 ──────────────────────────────────────────────────────

/// 前端 execute 响应（由 Tauri command `mvu_execute_result` 回传）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MvuExecuteResponse {
    pub request_id: String,
    #[serde(default)]
    pub variable_updates: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub side_effects: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// 前端 load ack 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuLoadAck {
    #[serde(default)]
    pub error: Option<String>,
}

/// Tauri event 名称常量
pub const EVENT_MVU_LOAD_CARD_ASSETS: &str = "mvu:load_card_assets";
pub const EVENT_MVU_UNLOAD_CARD: &str = "mvu:unload_card";
pub const EVENT_MVU_EXECUTE: &str = "mvu:execute";

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_returns_not_implemented() {
        let rt = StubMvuRuntime::new();
        assert!(!rt.is_available());

        let vars = HashMap::new();
        let result = rt.execute_fragment("_.set('hp', 50);", &vars).await;
        assert!(matches!(result, Err(MvuRuntimeError::NotImplemented)));

        let load = rt.load_card_assets(Some("<div>"), None, Some("x")).await;
        assert!(matches!(load, Err(MvuRuntimeError::NotImplemented)));

        let unload = rt.unload_card().await;
        assert!(matches!(unload, Err(MvuRuntimeError::NotImplemented)));
    }

    #[tokio::test]
    async fn test_mvu_exec_result_serde() {
        let mut vars = HashMap::new();
        vars.insert("hp".into(), serde_json::json!(80));
        let result = MvuExecResult {
            variable_updates: vars,
            side_effects: vec!["触发战斗结束".into()],
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: MvuExecResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.variable_updates.len(), 1);
        assert_eq!(back.side_effects.len(), 1);
    }

    #[tokio::test]
    async fn test_mvu_execute_response_serde() {
        let resp = MvuExecuteResponse {
            request_id: "test-123".into(),
            variable_updates: {
                let mut m = HashMap::new();
                m.insert("hp".into(), serde_json::json!(50));
                m
            },
            side_effects: vec!["slash_tag_1".into()],
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: MvuExecuteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.request_id, "test-123");
        assert_eq!(back.variable_updates.len(), 1);
        assert!(back.error.is_none());
    }

    #[tokio::test]
    async fn test_mvu_execute_response_with_error() {
        let resp = MvuExecuteResponse {
            request_id: "test-err".into(),
            variable_updates: HashMap::new(),
            side_effects: vec![],
            error: Some("JS 执行错误".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: MvuExecuteResponse = serde_json::from_str(&json).unwrap();
        assert!(back.error.is_some());
    }
}

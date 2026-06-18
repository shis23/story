//! MVU 兜底执行运行时（对应设计 §19.6 / §8.4，AGENT_INTERFACES §8.4）
//!
//! 实现方式：WebView runtime（隐藏 iframe + JSR/ST API shim）。
//!
//! 设计意图：MVU 卡里 Meta Agent 翻译不了的 JS 片段（`fallback_fragments` 非空 +
//! `routing = Hybrid`）需要在一个执行环境里跑原 JS。
//!
//! 通信链路：
//!   Rust `WebViewMvuRuntime::execute_fragment`
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
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::{Mutex, oneshot};

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
///   - `WebViewMvuRuntime`：共享 WebView 执行（W8 实现）
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

/// 默认执行超时（ms）
const DEFAULT_TIMEOUT_MS: u64 = 15_000;

/// WebView MVU 运行时
///
/// 通过 Tauri event 与前端 MvuJsRuntime.vue 通信。
/// `execute_fragment` 发送请求并等待前端通过 `mvu_execute_result` 命令回传结果。
pub struct WebViewMvuRuntime {
    app_handle: tauri::AppHandle,
    /// 等待中的 execute 请求（request_id → oneshot sender）
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<MvuExecuteResponse>>>>,
}

impl WebViewMvuRuntime {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self {
            app_handle,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 使用外部共享的 pending map（Tauri command handler 也用同一个 map）
    pub fn with_shared_pending(
        app_handle: tauri::AppHandle,
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<MvuExecuteResponse>>>>,
    ) -> Self {
        Self {
            app_handle,
            pending,
        }
    }

    /// 返回 pending map 引用（供外部合并使用）
    pub fn pending_ref(&self) -> Arc<Mutex<HashMap<String, oneshot::Sender<MvuExecuteResponse>>>> {
        self.pending.clone()
    }

    /// 由 Tauri command handler 调用：收到前端 execute 结果后完成对应 oneshot
    pub async fn complete_execute(&self, response: MvuExecuteResponse) {
        let mut map = self.pending.lock().await;
        if let Some(tx) = map.remove(&response.request_id) {
            let _ = tx.send(response);
        } else {
            tracing::warn!(
                "[MVU] 收到未知 request_id 的 execute_result: {}",
                response.request_id
            );
        }
    }
}

#[async_trait]
impl MvuRuntime for WebViewMvuRuntime {
    async fn execute_fragment(
        &self,
        fragment_js: &str,
        current_variables: &HashMap<String, serde_json::Value>,
    ) -> Result<MvuExecResult, MvuRuntimeError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<MvuExecuteResponse>();

        // 注册 pending
        {
            let mut map = self.pending.lock().await;
            map.insert(request_id.clone(), tx);
        }

        // 发送 execute 请求到前端
        let payload = serde_json::json!({
            "request_id": request_id,
            "fragment_js": fragment_js,
            "variables": current_variables,
            "timeout_ms": DEFAULT_TIMEOUT_MS,
        });
        self.app_handle
            .emit(EVENT_MVU_EXECUTE, &payload)
            .map_err(|e| MvuRuntimeError::ChannelError(format!("emit failed: {e}")))?;

        // 等待前端回传结果（带超时）
        let response = tokio::time::timeout(
            std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS),
            rx,
        )
        .await
        .map_err(|_| {
            // 超时清理 pending
            let pending = self.pending.clone();
            let rid = request_id.clone();
            tokio::spawn(async move {
                pending.lock().await.remove(&rid);
            });
            MvuRuntimeError::Timeout(format!("{DEFAULT_TIMEOUT_MS}ms"))
        })?
        .map_err(|e| MvuRuntimeError::ChannelError(format!("oneshot recv: {e}")))?;

        // 检查前端报告的 JS 错误
        if let Some(err) = &response.error {
            return Err(MvuRuntimeError::ExecutionFailed(err.clone()));
        }

        Ok(MvuExecResult {
            variable_updates: response.variable_updates,
            side_effects: response.side_effects,
        })
    }

    async fn load_card_assets(
        &self,
        html: Option<&str>,
        css: Option<&str>,
        js: Option<&str>,
    ) -> Result<(), MvuRuntimeError> {
        let payload = serde_json::json!({
            "html": html.unwrap_or(""),
            "css": css.unwrap_or(""),
            "js": js.unwrap_or(""),
        });
        self.app_handle
            .emit(EVENT_MVU_LOAD_CARD_ASSETS, &payload)
            .map_err(|e| MvuRuntimeError::ChannelError(format!("emit failed: {e}")))?;
        Ok(())
    }

    async fn unload_card(&self) -> Result<(), MvuRuntimeError> {
        self.app_handle
            .emit(EVENT_MVU_UNLOAD_CARD, &serde_json::json!({}))
            .map_err(|e| MvuRuntimeError::ChannelError(format!("emit failed: {e}")))?;
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

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

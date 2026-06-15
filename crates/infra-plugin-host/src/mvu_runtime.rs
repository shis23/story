//! MVU 兜底执行运行时（对应设计 §19.6 / §8.4，AGENT_INTERFACES §8.4）
//!
//! **本次实现状态：桩（stub）**。所有方法返回 `NotImplemented`。
//!
//! 设计意图：MVU 卡里 Meta Agent 翻译不了的 JS 片段（`fallback_fragments` 非空 +
//! `routing = Hybrid`）需要在一个执行环境里跑原 JS。两种方案：
//!   - QuickJS（嵌入式 JS runtime）：零星片段
//!   - 共享 WebView（全局一个常驻）：大量 JS + DOM 依赖（如缄默之秋1.4 类卡）
//!
//! 本次（P3 第一轮）只做 Meta Agent 五合一分析 + 原生渲染路径，**不实现兜底执行**。
//! 等下一轮手边有真实重 DOM 卡时再实现，避免对真卡水土不服。
//!
//! 本文件定义 `MvuRuntime` trait + `StubMvuRuntime`，让上层代码可以先依赖 trait 编译，
//! 实际执行留空。前端检测到 `fallback_fragments` 非空时显示「需共享 WebView 支持」提示。

use std::collections::HashMap;

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
    #[error("MVU 运行时未实现（共享 WebView 兜底为 P3 下一轮工作）")]
    NotImplemented,

    /// JS 执行错误
    #[error("JS 执行错误: {0}")]
    ExecutionFailed(String),

    /// 卡资产缺失
    #[error("卡资产缺失: {0}")]
    MissingAssets(String),
}

/// MVU 兜底执行运行时 trait
///
/// 上层（写作流水线 / 后处理）依赖本 trait，实际实现可在运行时切换：
///   - `StubMvuRuntime`：桩，全部返回 NotImplemented（本次）
///   - `WebViewMvuRuntime`：共享 WebView 执行（下一轮）
///   - `QuickJsMvuRuntime`：嵌入式 QuickJS 执行（未来可选）
pub trait MvuRuntime: Send + Sync {
    /// 执行单个 fallback JS 片段，输入当前变量，输出变量更新
    ///
    /// `fragment_js` 是 [`FallbackFragment::js_snippet`]，
    /// `current_variables` 是当前角色实例 + Campaign 的变量快照。
    fn execute_fragment(
        &self,
        fragment_js: &str,
        current_variables: &HashMap<String, serde_json::Value>,
    ) -> Result<MvuExecResult, MvuRuntimeError>;

    /// 加载一张卡的完整 JS 资产到共享 WebView（O(1) 内存，加载一次）
    ///
    /// 重 DOM 卡（如缄默之秋）的 18 万字符 JS 应只加载一次，
    /// 后续每轮只推 stat_data + 成文，让已加载的 JS 计算状态栏。
    fn load_card_assets(
        &self,
        html: Option<&str>,
        css: Option<&str>,
        js: Option<&str>,
    ) -> Result<(), MvuRuntimeError>;

    /// 卸载某卡的资产（切卡时释放）
    fn unload_card(&self) -> Result<(), MvuRuntimeError>;

    /// 是否已实现（桩返回 false，让上层据此决定是否提示用户）
    fn is_available(&self) -> bool;
}

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

impl MvuRuntime for StubMvuRuntime {
    fn execute_fragment(
        &self,
        _fragment_js: &str,
        _current_variables: &HashMap<String, serde_json::Value>,
    ) -> Result<MvuExecResult, MvuRuntimeError> {
        Err(MvuRuntimeError::NotImplemented)
    }

    fn load_card_assets(
        &self,
        _html: Option<&str>,
        _css: Option<&str>,
        _js: Option<&str>,
    ) -> Result<(), MvuRuntimeError> {
        Err(MvuRuntimeError::NotImplemented)
    }

    fn unload_card(&self) -> Result<(), MvuRuntimeError> {
        Err(MvuRuntimeError::NotImplemented)
    }

    fn is_available(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stub_returns_not_implemented() {
        let rt = StubMvuRuntime::new();
        assert!(!rt.is_available());

        let vars = HashMap::new();
        let result = rt.execute_fragment("_.set('hp', 50);", &vars);
        assert!(matches!(result, Err(MvuRuntimeError::NotImplemented)));

        let load = rt.load_card_assets(Some("<div>"), None, Some("x"));
        assert!(matches!(load, Err(MvuRuntimeError::NotImplemented)));

        let unload = rt.unload_card();
        assert!(matches!(unload, Err(MvuRuntimeError::NotImplemented)));
    }

    #[test]
    fn test_mvu_exec_result_serde() {
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
}

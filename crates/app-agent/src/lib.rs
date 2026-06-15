/// Agent 运行时（对应设计 §3.2 + §3.4）
///
/// 核心能力：
/// - 工具循环（run_tool_loop：max_rounds + drift recovery + 取消）
/// - 委派（spawn_subagents：tokio::spawn + watch 取消 + 并发上限 4）
/// - 工具注册（search_world_info / get_character / emit_plan / compose）
pub mod runtime;
pub mod tools;

// 重新导出核心类型
pub use runtime::{
    AgentConfig, AgentError, AgentRuntime, spawn_subagents,
    inject_hint_into_editor, inject_hint_into_subagent,
    EDITOR_HINT_MARKER, SUBAGENT_HINT_MARKER,
};
pub use tools::{ToolContext, ToolError, ToolRegistry};

/// Agent 运行时（对应设计 §3.2 + §3.4）
///
/// 核心能力：
/// - 工具循环（run_tool_loop：max_rounds + drift recovery + 取消）
/// - 委派（spawn_subagents：tokio::spawn + watch 取消 + 并发上限 4）
/// - 工具注册（search_world_info / get_character / emit_plan / compose）
pub mod runtime;
pub mod tools;

pub mod character_extractor;
pub mod pipeline_postprocess;
pub mod postprocess;
pub mod prompts;
pub mod summarizer;

// 重新导出核心类型
pub use runtime::{
    AgentConfig, AgentError, AgentRuntime, spawn_subagents,
    inject_hint_into_editor, inject_hint_into_subagent,
    EDITOR_HINT_MARKER, SUBAGENT_HINT_MARKER,
};
pub use tools::{ToolContext, ToolError, ToolRegistry};
pub use character_extractor::{extract_characters, attach_definitions_to_card, ExtractError};
pub use pipeline_postprocess::{run_postprocess_pipeline, PostProcessOutcome};
pub use prompts::character_extractor::{
    build_character_extractor_user_msg, make_character_extractor_config,
    register_character_extractor_tools, CHARACTER_EXTRACTOR_SYSTEM_PROMPT,
};

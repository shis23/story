pub mod llm_parse;
/// Agent 运行时（对应设计 §3.2 + §3.4）
///
/// 核心能力：
/// - 工具循环（run_tool_loop：max_rounds + drift recovery + 取消）
/// - 委派（spawn_subagents：tokio::spawn + watch 取消 + 并发上限 4）
/// - 工具注册（search_world_info / get_character / emit_plan / compose）
pub mod runtime;
pub mod tool_center;
pub mod tools;

pub mod character_extractor;
pub mod chronicle_compressor;
pub mod pipeline_postprocess;
pub mod postprocess;
pub mod prompts;
pub mod summarizer;

// 重新导出核心类型
pub use character_extractor::{ExtractError, attach_definitions_to_card, extract_characters};
pub use chronicle_compressor::{
    ChronicleCompressorError, CompressRunOutcome, compress_groups_with_llm,
    parse_compress_group_texts, plan_level_batch, publish_with_deterministic_texts,
    run_compress_if_needed,
};
pub use pipeline_postprocess::{PostProcessOutcome, run_postprocess_pipeline};
pub use prompts::character_extractor::{
    CHARACTER_EXTRACTOR_SYSTEM_PROMPT, build_character_extractor_user_msg,
    make_character_extractor_config, register_character_extractor_tools,
};
pub use runtime::{
    AgentConfig, AgentError, AgentRuntime, DEFAULT_MAX_CONCURRENT_SUBAGENTS, EDITOR_HINT_MARKER,
    SUBAGENT_HINT_MARKER, inject_hint_into_editor, inject_hint_into_subagent, spawn_subagents,
};
pub use tool_center::{ToolCenter, ToolScope, ToolSummary, role_matches};
pub use tools::{
    ChronicleToolBudget, ToolContext, ToolError, ToolRegistry, filter_registry_by_whitelist,
};

//! Agent 提示词集中模块（对应 AGENT_INTERFACES §6）
//!
//! 每个新增 Agent 的 system prompt / config 构造 / 用户消息拼装都放这里。
//! 改 prompt 只看本模块 + AGENT_INTERFACES.md。

pub mod character_extractor;
pub mod postprocess;
pub mod summarizer;

pub use character_extractor::{
    CHARACTER_EXTRACTOR_SYSTEM_PROMPT, build_character_extractor_user_msg,
    make_character_extractor_config, register_character_extractor_tools,
};
pub use postprocess::{
    POSTPROCESS_SYSTEM_PROMPT, build_postprocess_user_msg, build_postprocess_user_msg_with_summary,
    make_postprocess_config, register_postprocess_tools,
};
pub use summarizer::{SUMMARIZER_SYSTEM_PROMPT, build_summarizer_user_msg, make_summarizer_config};

//! Agent 提示词集中模块（对应 AGENT_INTERFACES §6）
//!
//! 每个新增 Agent 的 system prompt / config 构造 / 用户消息拼装都放这里。
//! 改 prompt 只看本模块 + AGENT_INTERFACES.md。

pub mod character_extractor;

pub use character_extractor::{
    build_character_extractor_user_msg, make_character_extractor_config,
    register_character_extractor_tools, CHARACTER_EXTRACTOR_SYSTEM_PROMPT,
};

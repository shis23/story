//! Meta Agent / MVU 分析 提示词集中模块（对应 AGENT_INTERFACES §8.3 / §9）
//!
//! 改 prompt 只看本模块 + AGENT_INTERFACES.md。

pub mod meta_agent;
pub mod mvu_analyzer;

pub use meta_agent::{
    build_meta_user_msg, make_meta_agent_config, register_meta_tools,
    META_AGENT_SYSTEM_PROMPT,
};
pub use mvu_analyzer::{
    build_mvu_analyzer_user_msg, make_mvu_analyzer_config, register_mvu_tools,
    MVU_ANALYZER_SYSTEM_PROMPT,
};

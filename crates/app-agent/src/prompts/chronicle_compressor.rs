//! ChronicleCompressor 提示词（记忆规格 §7.3）
//!
//! 系统做确定性分组；本 Agent 仅为每组生成 headline + summary。

use storyforge_domain::agent::AgentRole;
use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::chronicle::{ChronicleLevel, CompressGroup};

use crate::AgentConfig;

pub const CHRONICLE_COMPRESSOR_SYSTEM_PROMPT: &str = r#"你是剧情纪要压缩助手（chronicle compressor）。

系统已经把多条下层纪要切成连续不重叠的组。你只为每一组写：
1) headline：≤40 字，一句话导航标题
2) summary：80～200 字高密度阶段摘要

【硬性约束】
- 只根据给定条目压缩，不编造未出现的人物/事件
- 不输出 covers / code / id（系统填写）
- 不写「本轮」「综上所述」等元叙述

【输出格式】严格 JSON 数组，长度必须等于组数，顺序与输入组一致：
[
  {"headline":"...","summary":"..."},
  ...
]
只输出 JSON，不要 markdown 围栏。"#;

pub fn make_chronicle_compressor_config(
    agent_profile_config: Option<&AgentProfileConfig>,
) -> AgentConfig {
    // 复用 Summarizer 的 model/rounds 覆盖（无独立 role 时）
    let (model_override, rounds_override) = if let Some(apc) = agent_profile_config {
        let run = apc.run_config_for(&AgentRole::Summarizer);
        (run.model_override.clone(), run.max_tool_rounds)
    } else {
        (None, None)
    };
    AgentConfig {
        role: AgentRole::Summarizer,
        system_prompt: CHRONICLE_COMPRESSOR_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: rounds_override.unwrap_or(2),
        model: model_override.unwrap_or_else(|| "deepseek-chat".to_string()),
        tools: vec![],
        terminal_tools: vec![],
    }
}

/// 构造压缩 user 消息。
///
/// `members_per_group[i]` 与 `groups[i]` 对齐，每项为 (code, headline, summary)。
pub fn build_chronicle_compressor_user_msg(
    output_level: ChronicleLevel,
    groups: &[CompressGroup],
    members_per_group: &[Vec<(String, String, String)>],
) -> String {
    let level_name = match output_level {
        ChronicleLevel::B => "B（阶段纪要）",
        ChronicleLevel::C => "C（更高阶段纪要）",
        ChronicleLevel::A => "A",
    };
    let mut parts = Vec::new();
    parts.push(format!(
        "请将下列 {} 组下层纪要分别压缩为 {level_name}。输出 JSON 数组，长度 = {}。\n",
        groups.len(),
        groups.len()
    ));
    for (i, g) in groups.iter().enumerate() {
        parts.push(format!(
            "### 组 {}（turn {}–{}，{} 条）",
            i + 1,
            g.turn_start,
            g.turn_end,
            g.member_ids.len()
        ));
        if let Some(members) = members_per_group.get(i) {
            for (j, (code, headline, summary)) in members.iter().enumerate() {
                parts.push(format!(
                    "{}. [{}] {}\n{}",
                    j + 1,
                    code,
                    headline,
                    summary
                ));
            }
        }
        parts.push(String::new());
    }
    parts.join("\n")
}

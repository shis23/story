//! 剧情总结 Agent 提示词 / 配置（对应 AGENT_INTERFACES §6.3）
//!
//! 编剧成文后并行跑（与后处理 Agent 并行）。每轮产出一条「本轮摘要」
//! （200-500 字原子单位，记本轮发生的事），独立于 archiver 的批量归档。
//!
//! 改 prompt 只改本文件的常量。

use storyforge_domain::agent::AgentRole;

use crate::AgentConfig;

/// 剧情总结 Agent 系统提示词
pub const SUMMARIZER_SYSTEM_PROMPT: &str = r#"你是本轮剧情总结助手（round summary）。给你一轮成文，你要产出一段高密度的本轮摘要。

【目标】
生成一段可供长期召回的本轮剧情摘要。要求信息密度高、不水字数。

【硬性约束】
- 长度 200-500 字
- 只总结本轮发生的事，不要展望未来、不要复述前情
- 信息过多优先压缩，不要扩写

【内容优先级】（从高到低）
1. 人物关系变化（谁和谁的关系发生了转折）
2. 关键事件（发生了什么重要的事）
3. 目标变化（角色的目标/动机是否有变化）
4. 冲突（出现了什么矛盾/对抗）
5. 重要道具/地点/时间线
6. 未解决的伏笔（本轮埋下或推进了什么悬念）

【输出要求】
只输出最终摘要正文，不要标题、不要解释、不要 markdown 格式标记。直接开始写摘要。"#;

/// 构造剧情总结 Agent 的运行配置
pub fn make_summarizer_config() -> AgentConfig {
    AgentConfig {
        role: AgentRole::Summarizer,
        system_prompt: SUMMARIZER_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: 3,
        model: "deepseek-chat".to_string(),
        tools: vec![],
    }
}

/// 构造剧情总结 Agent 的用户消息
pub fn build_summarizer_user_msg(final_text: &str, scene_brief: &str, turn: u32) -> String {
    let mut parts = Vec::new();
    parts.push(format!("【当前轮次】第 {turn} 轮"));
    if !scene_brief.is_empty() {
        parts.push(format!("【导演规划的场景】{scene_brief}"));
    }
    parts.push(format!("【本轮成文】\n{final_text}"));
    parts.push("请输出本轮剧情摘要。".to_string());
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_has_correct_role() {
        let cfg = make_summarizer_config();
        assert_eq!(cfg.role, AgentRole::Summarizer);
        assert!(cfg.max_tool_rounds <= 5);
    }

    #[test]
    fn test_prompt_mentions_priority() {
        // Mock 脚本靠 "本轮剧情总结" 关键词命中
        assert!(SUMMARIZER_SYSTEM_PROMPT.contains("本轮剧情总结"));
        assert!(SUMMARIZER_SYSTEM_PROMPT.contains("人物关系"));
    }

    #[test]
    fn test_build_user_msg() {
        let msg = build_summarizer_user_msg("林医生走进急诊室", "雨中告别", 5);
        assert!(msg.contains("第 5 轮"));
        assert!(msg.contains("雨中告别"));
        assert!(msg.contains("林医生走进急诊室"));
    }
}

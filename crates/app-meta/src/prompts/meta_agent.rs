//! Meta 配置调试 Agent 提示词 / 配置 / 工具注册（对应设计 §9，AGENT_INTERFACES §9）
//!
//! 多轮对话形态：用户问"帮我看看世界书有没有冲突" → Agent 调 inspect 工具 →
//! 产出诊断结论 + 可选 Patch 提议（用户采纳才执行）。
//!
//! 工具 handler 在 `meta_conversation::register_meta_runtime_tools` 里挂接实际的
//! inspect_world_info / inspect_character / propose_patch 实现（需要 PatchStore + 数据源），
//! 这里只注册工具 spec + system prompt。

use storyforge_domain::agent::AgentRole;
use storyforge_domain::llm::ToolSpec;

use storyforge_app_agent::tools::ToolRegistry;
use storyforge_app_agent::AgentConfig;

/// Meta 配置调试 Agent 系统提示词
///
/// 定位：独立于写作流水线的配置/调试助手。三大能力：
///   ① 诊断（世界书冲突 / 角色卡字段缺失 / 预设问题）
///   ② Patch 提议（提议修改 → 用户采纳才执行）
///   ③ ST 预设分类（把 ST prompt 归入我们的 6 大模块组）
///
/// 安全约束（设计 §9.5）：
///   - 不参与写作流水线
///   - Patch 必须用户点"采纳"才应用
///   - 不调 LLM 生成工具（防 prompt injection 提权）
///   - API key 永不经过 Meta Agent
pub const META_AGENT_SYSTEM_PROMPT: &str = r#"你是 StoryForge 的配置调试助手（Meta Agent）。你不参与写作流水线，专门帮用户诊断配置问题、整理 ST 预设、提议修复 Patch。

【三大能力】
1. 诊断：调 meta_inspect_world_info / meta_inspect_character 工具，检查世界书冲突、角色卡字段缺失
2. ST 预设分类：调 meta_classify_st_preset 工具，把 ST 预设里的 prompt 归入 6 大模块组（perspective/cot/style/quality/output/待确认）
3. Patch 提议：发现问题后，调 meta_propose_patch 提议修复（不直接改，用户点采纳才执行）

【工作模式】
- 用户提问 → 你判断要调哪个诊断工具 → 看结果 → 给结论 + 提议 Patch（如有）
- 回答用中文，简洁，先给结论再给依据
- 提议 Patch 时，描述要清楚改什么、为什么改

【安全约束】
- 你不能直接修改任何配置，只能提议 Patch
- 不接触 API key、不调写作流水线
- 不确定的事就说"建议人工确认"，不要瞎猜

【工具调用规范】
- 调诊断工具拿报告，再基于报告回答（不要凭空说"发现冲突"）
- 一次只调必要的工具，不要过度调用
- 诊断报告里的字段：total_entries / conflicts / issues 等都是计数或列表，直接用"#;

/// 构造 Meta Agent 的运行配置（多轮对话）
pub fn make_meta_agent_config() -> AgentConfig {
    AgentConfig {
        role: AgentRole::Meta,
        system_prompt: META_AGENT_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: 8,
        model: "deepseek-chat".to_string(),
        tools: vec![],
    }
}

/// 构造 Meta Agent 多轮对话的用户消息（拼对话历史 + 当前用户输入）
pub fn build_meta_user_msg(history: &[String], user_input: &str) -> String {
    if history.is_empty() {
        return user_input.to_string();
    }
    let mut s = String::from("【之前的对话】\n");
    for (i, h) in history.iter().enumerate() {
        s.push_str(&format!("{}. {}\n", i + 1, h));
    }
    s.push_str("\n【用户最新问题】\n");
    s.push_str(user_input);
    s
}

/// 注册 Meta Agent 的工具 spec（handler 在 meta_conversation 里挂接实际实现）
///
/// 注意：这里只注册 spec，handler 是占位（直接返回 args）。
/// 真正的 handler 挂接在 `meta_conversation::register_meta_runtime_tools`，
/// 因为 inspect/propose 需要访问 PatchStore + 数据源，不能在这里硬编码。
///
/// 如果调用方不需要诊断工具（只想跑纯 MVU 分析），可以不调本函数。
pub fn register_meta_tools(registry: &mut ToolRegistry) {
    // meta_inspect_world_info
    registry.register(
        ToolSpec::function(
            "meta_inspect_world_info",
            "诊断世界书：检查蓝灯关键词冲突、孤立条目。返回 WorldInfoReport。",
            serde_json::json!({"type": "object", "properties": {}}),
        ),
        |args, _ctx| Box::pin(async move { Ok(args) }),
    );

    // meta_inspect_character
    registry.register(
        ToolSpec::function(
            "meta_inspect_character",
            "诊断当前角色卡：检查描述/性格/开场白是否为空、开场白是否为占位符。",
            serde_json::json!({"type": "object", "properties": {}}),
        ),
        |args, _ctx| Box::pin(async move { Ok(args) }),
    );

    // meta_propose_patch
    registry.register(
        ToolSpec::function(
            "meta_propose_patch",
            "提议一个修复 Patch（不直接执行，用户采纳后才应用）。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {"type": "string"},
                    "actions": {"type": "array"}
                },
                "required": ["description", "actions"]
            }),
        ),
        |args, _ctx| Box::pin(async move { Ok(args) }),
    );

    // meta_classify_st_preset
    registry.register(
        ToolSpec::function(
            "meta_classify_st_preset",
            "把 ST 预设里的 prompt 归入 6 大模块组（perspective/cot/style/quality/output/pending）。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "classifications": {"type": "array"}
                },
                "required": ["classifications"]
            }),
        ),
        |args, _ctx| Box::pin(async move { Ok(args) }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_config_has_meta_role() {
        let cfg = make_meta_agent_config();
        assert_eq!(cfg.role, AgentRole::Meta);
        assert!(cfg.max_tool_rounds > 0);
    }

    #[test]
    fn test_meta_prompt_contains_keyword_and_constraints() {
        // Mock 脚本靠 "配置调试助手" 关键词命中
        assert!(META_AGENT_SYSTEM_PROMPT.contains("配置调试助手"));
        assert!(META_AGENT_SYSTEM_PROMPT.contains("不参与写作流水线"));
        assert!(META_AGENT_SYSTEM_PROMPT.contains("不能直接修改"));
    }

    #[test]
    fn test_build_meta_user_msg_with_history() {
        let history = vec!["用户：看看世界书".into(), "助手：发现2处冲突".into()];
        let msg = build_meta_user_msg(&history, "帮我修一下");
        assert!(msg.contains("之前的对话"));
        assert!(msg.contains("看看世界书"));
        assert!(msg.contains("帮我修一下"));
    }

    #[test]
    fn test_build_meta_user_msg_empty_history() {
        let msg = build_meta_user_msg(&[], "你好");
        assert_eq!(msg, "你好");
    }
}

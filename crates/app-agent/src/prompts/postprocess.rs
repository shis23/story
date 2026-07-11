//! 后处理 Agent 提示词 / 配置 / 工具注册（对应 AGENT_INTERFACES §6.4，D40-D41/D45）
//!
//! 编剧成文后并行跑（与剧情总结 Agent 并行）。一次调用产出三件套：
//! 角色知识更新 + 变量更新 + 任务更新。
//!
//! 改 prompt 只改本文件的常量；改输出格式同步改 `postprocess::parse_*`。

use storyforge_domain::agent::AgentRole;
use storyforge_domain::agent_profile_config::AgentProfileConfig;

use crate::AgentConfig;
use crate::tools::ToolRegistry;

/// 后处理 Agent 系统提示词（含 JSON 输出格式示例）
///
/// 输出格式里字段名必须和 domain::agent::PostProcessResult 对齐。
pub const POSTPROCESS_SYSTEM_PROMPT: &str = r#"你是后处理助手（postprocess）。给你一轮成文，你要抽取三件事，一次输出：

【任务一：角色知识更新】
为每个在场角色判断「这轮它新获知了什么」。注意信息来源分类：
- witnessed（亲眼所见）：角色在场时发生的事
- told_by_other（被他人告知）：别人明确告诉它的话，需记 source_character（告知者的名字）
- inferred（自己推断）：角色根据观察推理出的结论
- 不要给 backstory（背景设定只在导入时建立，不在这里）
信息用该角色第一人称视角表述（"我看到了……" / "X 告诉我……"）。只抽「新的」信息，已有的不重复。如果某角色这轮没获知新信息，就不要给它写条目。

【定向告知规则】
told_by_other 时，character_id 是**被告知者**（谁收到了信息），source_character_id 是**告知者**（谁说的）。
告知可以面向不在场的角色（写信、传话、留信息、派人通知），系统会放行。
示例："林医生写信告诉陈警官地下室有尸体" → character_id=陈警官, source_character_id=林医生, source=told_by_other。

【广播规则】
如果成文里有**公开宣告或世界级事件**（公告、爆炸、天气突变、全城警报等所有角色都该知道的事），
在 knowledge_updates 里输出一条带 broadcast 字段的条目：
- broadcast: "all" 表示广播给 Campaign 内所有角色（character_id 填公告发起者或任意角色名，系统会忽略并分发给所有人）
- broadcast: "组名"（如"守卫"）表示广播给该身份组的所有角色
- 普通在场角色知识不需要 broadcast 字段（不填或 null = 单角色定向）
示例：城主宣告戒严 → { "character_id": "城主", "knowledge_text": "城主宣告全城戒严", "source": "witnessed", "broadcast": "all" }

【秘密/封口规则】
如果成文明确说明某条信息是秘密、保密、只有某人知道、不得外传，给该知识条目加：
- propagation: "private"
不要为 private 知识同时输出 broadcast；如果后续成文里有人试图传播这条 private 知识，系统写回层会做门禁。

【任务二：变量更新】
根据成文里发生的事，更新角色变量（hp/state/location/mood 等）或全局变量（story_clock/weather/world_state）。只输出真正发生了变化的字段。全局变量（无 instance_id）用于 story_clock 推进、天气变化、大势扭转等。

【任务三：任务/伏笔更新】
- 如果成文里新埋了伏笔或新出现了长期目标（如"老王说要三个月后复仇"），抽成新任务，trigger 用 event（事件描述）。
- 如果某个已存在的任务可能已完成，输出置信度（0-1），不要直接标 completed——由系统提示用户确认。
- 不要重复抽已有的任务。

【输出格式】
调用 emit_postprocess 工具，或直接输出 JSON。**只输出一个 JSON 对象，第一个字符必须是 `{`，最后一个字符必须是 `}`。严禁输出任何解释、说明、前导语、结语、Markdown、代码围栏（```）或自然语言。**冗长输出会被截断导致解析失败：
{
  "knowledge_updates": [
    {
      "character_id": "林医生",
      "knowledge_text": "我看到陈警官在地下室发现了那具尸体",
      "source": "witnessed",
      "source_character_id": null,
      "pinned": false,
      "propagation": "open"
    },
    {
      "character_id": "陈警官",
      "knowledge_text": "林医生告诉我地下室有尸体",
      "source": "told_by_other",
      "source_character_id": "林医生",
      "pinned": false,
      "propagation": "open"
    },
    {
      "character_id": "城主",
      "knowledge_text": "城主宣告全城戒严",
      "source": "witnessed",
      "source_character_id": null,
      "pinned": false,
      "broadcast": "all",
      "propagation": "open"
    }
  ],
  "variable_updates": [
    {"instance_id": "林医生", "key": "state", "value": "受伤"},
    {"instance_id": null, "key": "story_clock", "value": "第2天"}
  ],
  "task_updates": [
    {
      "task_id": null,
      "new_status": "pending",
      "new_task": {
        "title": "老王三个月后复仇",
        "description": "老王被陷害后发誓三个月后报复",
        "triggers": [{"kind": "event", "description": "三个月期限到达"}],
        "related_characters": ["老王"]
      }
    }
  ]
}

字段说明：
- character_id / instance_id / source_character_id：传角色名（不是 ID），系统会解析匹配
- source 取值：witnessed / told_by_other / inferred（小写）
- new_status 取值：pending / active / likely_completed / completed / abandoned（小写）
- broadcast：可选，"all"=广播全体，"组名"=广播该身份组，不填=单角色定向
- propagation：可选，"open"=可正常传播，"private"=秘密/禁止外传；不填按 open
- 如果某一项没有更新，输出空数组

【重要】抽取范围严格限制在「提供的在场角色列表」内，不要给不在场的角色抽知识。
例外：told_by_other 的被告知者可以不在场（写信/传话/留信息）；broadcast 条目会被系统分发给目标角色；private 知识不得广播或外传。"#;

/// 构造后处理 Agent 的运行配置
///
/// 若提供 `agent_profile_config`，从中读取 PostProcessor 的 `model_override` 和
/// `max_tool_rounds` 覆盖硬编码默认值（无 config = 当前硬编码值，向后兼容）。
pub fn make_postprocess_config(agent_profile_config: Option<&AgentProfileConfig>) -> AgentConfig {
    let (model_override, rounds_override) = if let Some(apc) = agent_profile_config {
        let run = apc.run_config_for(&AgentRole::PostProcessor);
        (run.model_override.clone(), run.max_tool_rounds)
    } else {
        (None, None)
    };
    AgentConfig {
        role: AgentRole::PostProcessor,
        system_prompt: POSTPROCESS_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: rounds_override.unwrap_or(5),
        model: model_override.unwrap_or_else(|| "deepseek-chat".to_string()),
        tools: vec![],
        // emit_postprocess 是"声明产出完成"的终止信号，必须列入 terminal_tools，
        // 否则 runtime（run_tool_loop）在 LLM 调用 emit_postprocess 后不会终止，
        // 循环到 max_tool_rounds 抛 MaxRoundsExceeded，整个 postprocess 失败。
        // 对齐 character_extractor 的 terminal_tools: ["emit_characters"] 模式。
        terminal_tools: vec!["emit_postprocess".into()],
    }
}

/// 构造后处理 Agent 的用户消息
///
/// - `final_text`：本轮成文
/// - `present_characters`：在场角色名列表（来自导演 Plan）
/// - `variable_keys`：可更新的变量键（提示 Agent 可改哪些字段）
/// - `turn` / `story_clock`：当前轮次和故事时钟（给 Agent 判断任务触发用）
pub fn build_postprocess_user_msg(
    final_text: &str,
    present_characters: &[String],
    variable_keys: &[String],
    turn: u32,
    story_clock: &str,
) -> String {
    let mut parts = Vec::new();
    parts.push(format!(
        "【当前轮次】第 {turn} 轮（故事时间：{story_clock}）"
    ));
    parts.push(format!(
        "【在场角色】{}",
        if present_characters.is_empty() {
            "（无）".to_string()
        } else {
            present_characters.join("、")
        }
    ));
    if !variable_keys.is_empty() {
        parts.push(format!("【可更新变量】{}", variable_keys.join(", ")));
    }
    parts.push(format!("【本轮成文】\n{final_text}"));
    parts.push("请按指定 JSON 格式输出后处理结果。".to_string());
    parts.join("\n\n")
}

/// 注册后处理 Agent 的工具（emit_postprocess：声明产出）
pub fn register_postprocess_tools(registry: &mut ToolRegistry) {
    registry.register(
        storyforge_domain::llm::ToolSpec::function(
            "emit_postprocess",
            "输出后处理三合一结果（角色知识 + 变量 + 任务）。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "knowledge_updates": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "character_id": {"type": "string"},
                                "knowledge_text": {"type": "string"},
                                "source": {"type": "string", "enum": ["witnessed", "told_by_other", "inferred"]},
                                "source_character_id": {"type": "string"},
                                "pinned": {"type": "boolean"},
                                "broadcast": {"type": "string", "description": "广播目标: 'all'=全体, '组名'=身份组, 不填=单角色"},
                                "propagation": {"type": "string", "enum": ["open", "private"], "description": "传播策略: open=可传播, private=秘密/禁止外传"}
                            },
                            "required": ["character_id", "knowledge_text", "source"]
                        }
                    },
                    "variable_updates": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "instance_id": {"type": "string"},
                                "key": {"type": "string"},
                                "value": {}
                            },
                            "required": ["key", "value"]
                        }
                    },
                    "task_updates": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "task_id": {"type": "string"},
                                "new_status": {"type": "string"},
                                "new_task": {
                                    "type": "object",
                                    "properties": {
                                        "title": {"type": "string"},
                                        "description": {"type": "string"},
                                        "triggers": {"type": "array"},
                                        "related_characters": {"type": "array", "items": {"type": "string"}}
                                    }
                                }
                            }
                        }
                    }
                }
            }),
        ),
        |args, _ctx| Box::pin(async move { Ok(args) }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_has_correct_role() {
        let cfg = make_postprocess_config(None);
        assert_eq!(cfg.role, AgentRole::PostProcessor);
    }

    #[test]
    fn test_config_marks_emit_postprocess_as_terminal() {
        // emit_postprocess 必须是终止工具，否则 run_tool_loop 在 LLM 调用后不终止，
        // 循环到 max_tool_rounds 抛 MaxRoundsExceeded，postprocess 整体失败（B1 阻塞根因）。
        let cfg = make_postprocess_config(None);
        assert!(
            cfg.terminal_tools.iter().any(|t| t == "emit_postprocess"),
            "terminal_tools 必须含 emit_postprocess，实际为 {:?}",
            cfg.terminal_tools
        );
    }

    #[test]
    fn test_prompt_mentions_three_outputs() {
        // Mock 脚本靠 "后处理" 关键词命中，prompt 必须含此词
        assert!(POSTPROCESS_SYSTEM_PROMPT.contains("后处理"));
        assert!(POSTPROCESS_SYSTEM_PROMPT.contains("knowledge_updates"));
        assert!(POSTPROCESS_SYSTEM_PROMPT.contains("variable_updates"));
        assert!(POSTPROCESS_SYSTEM_PROMPT.contains("task_updates"));
    }

    #[test]
    fn test_prompt_forbids_verbose_output() {
        // P2-6：prompt 必须强制「纯 JSON 无解释」,否则冗长输出被截断导致解析失败。
        assert!(
            POSTPROCESS_SYSTEM_PROMPT.contains("严禁")
                && POSTPROCESS_SYSTEM_PROMPT.contains("解释"),
            "prompt 必须明确禁止解释性输出"
        );
        assert!(
            POSTPROCESS_SYSTEM_PROMPT.contains("第一个字符必须是"),
            "prompt 必须要求首字符即 JSON 开头"
        );
    }

    #[test]
    fn test_build_user_msg_includes_essentials() {
        let msg = build_postprocess_user_msg(
            "林医生走进急诊室",
            &["林医生".to_string(), "陈警官".to_string()],
            &["hp".to_string(), "state".to_string()],
            3,
            "第2天",
        );
        assert!(msg.contains("第 3 轮"));
        assert!(msg.contains("第2天"));
        assert!(msg.contains("林医生、陈警官"));
        assert!(msg.contains("hp, state"));
        assert!(msg.contains("林医生走进急诊室"));
    }
}

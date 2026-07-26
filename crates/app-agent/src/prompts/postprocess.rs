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
如果用户消息里提供了【卡片变量更新规则】，那是这张卡翻译出的玩法规则（好感度增减条件、状态切换门槛等）。生成 variable_updates 时逐条对照：成文中出现了某条规则描述的触发情形，就按该规则计算对应变量的新值。规则与成文事实冲突时以成文为准；成文没有触发的规则不要凭空执行。

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
    build_postprocess_user_msg_with_summary(
        final_text,
        present_characters,
        variable_keys,
        turn,
        story_clock,
        None,
    )
}

/// 同 `build_postprocess_user_msg`，可附加近期剧情摘要（ContextCompiler 最小版）。
pub fn build_postprocess_user_msg_with_summary(
    final_text: &str,
    present_characters: &[String],
    variable_keys: &[String],
    turn: u32,
    story_clock: &str,
    recent_summary_block: Option<&str>,
) -> String {
    build_postprocess_user_msg_with_context(
        final_text,
        present_characters,
        variable_keys,
        turn,
        story_clock,
        recent_summary_block,
        &[],
    )
}

/// MVU 规则注入预算：条数 / 单条字符 / 总字符上限。
/// 卿卿级别的卡翻译出 40+ 条规则，预算按其 2 倍余量设定；超限截断并附说明。
const MVU_RULES_MAX_COUNT: usize = 80;
const MVU_RULES_MAX_CHARS_PER_RULE: usize = 400;
const MVU_RULES_MAX_TOTAL_CHARS: usize = 12_000;

/// 渲染【卡片变量更新规则】区块（去重 + 预算截断）。空输入返回 None。
fn render_mvu_update_rules_block(rules: &[String]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let mut lines: Vec<String> = Vec::new();
    let mut total_chars = 0usize;
    let mut dropped = 0usize;
    for rule in rules {
        let trimmed = rule.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        if lines.len() >= MVU_RULES_MAX_COUNT || total_chars >= MVU_RULES_MAX_TOTAL_CHARS {
            dropped += 1;
            continue;
        }
        let clipped: String = if trimmed.chars().count() > MVU_RULES_MAX_CHARS_PER_RULE {
            let mut s: String = trimmed.chars().take(MVU_RULES_MAX_CHARS_PER_RULE).collect();
            s.push_str("……（截断）");
            s
        } else {
            trimmed.to_string()
        };
        total_chars += clipped.chars().count();
        lines.push(format!("- {clipped}"));
    }
    if lines.is_empty() {
        return None;
    }
    if dropped > 0 {
        lines.push(format!("（另有 {dropped} 条规则因预算截断未列出）"));
    }
    Some(format!(
        "【卡片变量更新规则】\n{}\n（以上是这张卡翻译出的玩法规则。生成 variable_updates 时逐条对照，成文触发了哪条就按哪条计算变量新值。）",
        lines.join("\n")
    ))
}

/// 同 `build_postprocess_user_msg_with_summary`，可附加卡片翻译出的 MVU 变量更新规则
/// （`MvuTranslation.update_rules`，由 tauri-app 从 CampaignStore 按在场角色收集）。
#[allow(clippy::too_many_arguments)]
pub fn build_postprocess_user_msg_with_context(
    final_text: &str,
    present_characters: &[String],
    variable_keys: &[String],
    turn: u32,
    story_clock: &str,
    recent_summary_block: Option<&str>,
    mvu_update_rules: &[String],
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
    if let Some(block) = render_mvu_update_rules_block(mvu_update_rules) {
        parts.push(block);
    }
    if let Some(block) = recent_summary_block
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        parts.push(format!(
            "【近期剧情摘要】\n{block}\n（请在提取知识/变量/任务时与上述摘要保持一致，勿捏造已否决事实。）"
        ));
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

    #[test]
    fn test_build_user_msg_with_context_includes_mvu_rules_block() {
        let msg = build_postprocess_user_msg_with_context(
            "江离递来一杯茶",
            &["江离".to_string()],
            &["好感度".to_string()],
            3,
            "第2天",
            None,
            &[
                "角色赠送礼物或表达关心时，该角色好感度 +5".to_string(),
                "  角色赠送礼物或表达关心时，该角色好感度 +5  ".to_string(), // 去重（含空白差异）
                "主角受到攻击时 hp 按伤害值扣减".to_string(),
            ],
        );
        assert!(msg.contains("【卡片变量更新规则】"));
        assert!(msg.contains("- 角色赠送礼物或表达关心时，该角色好感度 +5"));
        assert!(msg.contains("- 主角受到攻击时 hp 按伤害值扣减"));
        assert_eq!(
            msg.matches("好感度 +5").count(),
            1,
            "重复规则应去重只出现一次"
        );
        // 规则块应出现在成文之前（规则是读成文时的对照表）
        let rules_pos = msg.find("【卡片变量更新规则】").unwrap();
        let text_pos = msg.find("【本轮成文】").unwrap();
        assert!(rules_pos < text_pos);
    }

    #[test]
    fn test_build_user_msg_without_rules_has_no_rules_block() {
        let msg = build_postprocess_user_msg_with_context(
            "成文",
            &["角色A".to_string()],
            &["hp".to_string()],
            1,
            "第1天",
            None,
            &[],
        );
        assert!(!msg.contains("【卡片变量更新规则】"));
        // 与旧签名保持字节级一致（回归保护：无规则时输出不变）
        let legacy = build_postprocess_user_msg_with_summary(
            "成文",
            &["角色A".to_string()],
            &["hp".to_string()],
            1,
            "第1天",
            None,
        );
        assert_eq!(msg, legacy);
    }

    #[test]
    fn test_mvu_rules_block_budget_truncation() {
        // 超出条数预算：只保留前 MVU_RULES_MAX_COUNT 条并附截断说明
        let rules: Vec<String> = (0..100).map(|i| format!("规则编号{i}：某条件触发")).collect();
        let block = render_mvu_update_rules_block(&rules).expect("非空规则应产出区块");
        assert!(block.contains("规则编号0"));
        assert!(block.contains("规则编号79"));
        assert!(!block.contains("规则编号80："));
        assert!(block.contains("因预算截断未列出"));

        // 单条超长：按字符截断
        let long_rule = vec!["长".repeat(500)];
        let block = render_mvu_update_rules_block(&long_rule).unwrap();
        assert!(block.contains("……（截断）"));

        // 全空白输入：无区块
        assert!(render_mvu_update_rules_block(&["  ".to_string()]).is_none());
    }

    #[test]
    fn test_system_prompt_mentions_mvu_rules() {
        // 系统提示词必须告诉 Agent 如何对待【卡片变量更新规则】区块
        assert!(POSTPROCESS_SYSTEM_PROMPT.contains("【卡片变量更新规则】"));
    }

    #[test]
    fn test_build_postprocess_user_msg_with_summary_includes_block() {
        let msg = build_postprocess_user_msg_with_summary(
            "林医生走进急诊室",
            &["林医生".to_string()],
            &["hp".to_string()],
            3,
            "第2天",
            Some(
                "近期剧情摘要（按轮次，供规划参考，勿直接复述）：
- T1: 昨夜有人潜入",
            ),
        );
        assert!(msg.contains("【近期剧情摘要】"));
        assert!(msg.contains("昨夜有人潜入"));
        assert!(msg.contains("林医生走进急诊室"));
    }
}

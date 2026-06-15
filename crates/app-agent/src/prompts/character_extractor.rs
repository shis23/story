//! 角色识别 Agent 提示词 / 配置 / 工具注册（对应 AGENT_INTERFACES §6.2，D33）
//!
//! 导入角色卡时跑一次：语义级分析卡内容，识别卡内角色，为每个角色生成
//! CharacterDefinition（persona/behavior/backstory/role_type）。
//!
//! 改 prompt 只改本文件的常量；改输出格式同步改 `character_extractor::parse_*`。

use storyforge_domain::agent::AgentRole;
use storyforge_domain::character::Character;

use crate::tools::ToolRegistry;
use crate::AgentConfig;

/// 角色识别 Agent 系统提示词（含 JSON 输出格式示例）
///
/// 注意：示例里 `name/persona_prompt/behavior_rules/base_backstory/role_type/group`
/// 必须和 domain::character::CharacterDefinition 的字段名对齐
/// （parse_character_definitions_from_response 反序列化时按字段名解析）。
pub const CHARACTER_EXTRACTOR_SYSTEM_PROMPT: &str = r#"你是卡内角色识别助手（character recognition）。给你一张 SillyTavern 角色卡的全文（描述、性格、开场白、备选开场、世界书条目），你的任务是语义级识别「这张卡里有哪些角色」，并为每个角色生成结构化定义。

【任务】
1. 通读全部输入，识别所有「有独立人格、会说话/会行动」的角色（不只是卡主，还包括 NPC、配角、对立角色）
2. 对每个角色提炼：人设（性格/说话风格/口头禅/外貌）、行为规则（决策倾向/禁忌）、游戏开始前就知道的事（3-5 条背景知识）
3. 判断角色类型：protagonist（主角，通常就是卡主）/ supporting（配角，常驻出场）/ extra（临场龙套）
4. 按需要打分组标签（如 "主角团"/"反派"/"路人"，无明显分组则留空）

【识别要点】
- 角色名要具体（"林医生" 而非 "医生"），从描述/对白/世界书里找
- persona_prompt 用第二人称写给子 Agent 看（"你是一位冷静的外科医生，说话简短精确……"）
- behavior_rules 写成可执行的约束（"绝不主动透露病人隐私"、"面对质问时先沉默三秒"）
- base_backstory 只写「游戏开始前该角色已经知道的事」，不要写未来剧情
- 宁少勿错：把握不准是不是独立角色，就归并到主角设定里，不要硬拆

【输出格式】
调用 emit_characters 工具，或直接输出 JSON 数组（不要多余解释）：
[
  {
    "name": "林医生",
    "persona_prompt": "你是一位三十出头的外科医生……",
    "behavior_rules": "面对危重病人时先评估再行动；绝不……",
    "base_backstory": ["你是本市三甲医院急诊科主治", "三年前经历过一次失败的手术"],
    "role_type": "protagonist",
    "group": "主角团"
  },
  {
    "name": "陈警官",
    "persona_prompt": "你是一名老刑警……",
    "behavior_rules": "……",
    "base_backstory": ["……"],
    "role_type": "supporting",
    "group": "主角团"
  }
]

role_type 取值：protagonist / supporting / extra（小写）。"#;

/// 构造角色识别 Agent 的运行配置
pub fn make_character_extractor_config() -> AgentConfig {
    AgentConfig {
        role: AgentRole::CharacterExtractor,
        system_prompt: CHARACTER_EXTRACTOR_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: 8,
        model: "deepseek-chat".to_string(),
        tools: vec![],
    }
}

/// 构造角色识别 Agent 的用户消息（喂卡的全文给它）
pub fn build_character_extractor_user_msg(character: &Character) -> String {
    let mut parts = Vec::new();

    parts.push(format!("【卡名】{}", character.name));

    if !character.description.is_empty() {
        parts.push(format!("【描述】\n{}", character.description));
    }
    if !character.personality.is_empty() {
        parts.push(format!("【性格】\n{}", character.personality));
    }
    if !character.scenario.is_empty() {
        parts.push(format!("【场景设定】\n{}", character.scenario));
    }
    if !character.system_prompt.is_empty() {
        parts.push(format!("【系统提示词】\n{}", character.system_prompt));
    }
    if !character.first_mes.is_empty() {
        parts.push(format!("【开场白】\n{}", character.first_mes));
    }
    for (i, greeting) in character.alternate_greetings.iter().enumerate() {
        if !greeting.is_empty() {
            parts.push(format!("【备选开场 {}】\n{}", i + 1, greeting));
        }
    }
    if !character.mes_example.is_empty() {
        parts.push(format!("【对话示例】\n{}", character.mes_example));
    }

    // 世界书条目全文（识别 NPC 的主要来源）
    if let Some(book) = &character.embedded_world_info {
        if !book.entries.is_empty() {
            let mut wi = String::from("【世界书条目】\n");
            for (i, entry) in book.entries.iter().enumerate() {
                wi.push_str(&format!(
                    "--- 条目 {}（keys: {}）---\n{}\n",
                    i + 1,
                    entry.keys.join(", "),
                    entry.content
                ));
            }
            parts.push(wi);
        }
    }

    parts.push("请识别这张卡里的所有角色并按指定 JSON 格式输出。".to_string());

    parts.join("\n\n")
}

/// 注册角色识别 Agent 的工具
///
/// 只注册 `emit_characters`：让模型声明「我已产出角色定义」。handler 原样返回 args，
/// 真正解析在 `character_extractor::parse_character_definitions_from_response`。
pub fn register_character_extractor_tools(registry: &mut ToolRegistry) {
    registry.register(
        storyforge_domain::llm::ToolSpec::function(
            "emit_characters",
            "输出识别出的卡内角色定义数组。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "characters": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "persona_prompt": {"type": "string"},
                                "behavior_rules": {"type": "string"},
                                "base_backstory": {
                                    "type": "array",
                                    "items": {"type": "string"}
                                },
                                "role_type": {
                                    "type": "string",
                                    "enum": ["protagonist", "supporting", "extra"]
                                },
                                "group": {"type": "string"}
                            },
                            "required": ["name", "persona_prompt"]
                        }
                    }
                },
                "required": ["characters"]
            }),
        ),
        |args, _ctx| {
            Box::pin(async move { Ok(args) })
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Id;

    #[test]
    fn test_make_config_has_unique_role() {
        let cfg = make_character_extractor_config();
        assert_eq!(cfg.role, AgentRole::CharacterExtractor);
        assert!(cfg.max_tool_rounds > 0);
    }

    #[test]
    fn test_system_prompt_mentions_character_recognition() {
        // Mock 脚本靠 "卡内角色识别" 关键词命中，prompt 必须含此词
        assert!(CHARACTER_EXTRACTOR_SYSTEM_PROMPT.contains("卡内角色识别"));
    }

    #[test]
    fn test_build_user_msg_includes_card_fields() {
        let ch = Character {
            id: Id::new(),
            name: "测试卡".into(),
            description: "一个测试角色".into(),
            personality: "冷静".into(),
            scenario: String::new(),
            first_mes: "你好".into(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec!["嗨".into()],
            embedded_world_info: None,
            extensions: serde_json::Value::Null,
            renderable_assets: None,
            source: storyforge_domain::Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        };
        let msg = build_character_extractor_user_msg(&ch);
        assert!(msg.contains("测试卡"));
        assert!(msg.contains("一个测试角色"));
        assert!(msg.contains("冷静"));
        assert!(msg.contains("你好"));
        assert!(msg.contains("嗨"));
    }
}

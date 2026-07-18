//! 角色识别 Agent 编排与输出解析（对应 AGENT_INTERFACES §6.2，D33）
//!
//! 流程：
//! 1. `extract_characters` 调 AgentRuntime::run_tool_loop 跑识别 Agent
//! 2. `parse_character_definitions_from_response` 5 层兜底解析输出
//! 3. 每个 definition 回填 card_id，合并 MVU schema 进 variable_schema
//!
//! 调用方（tauri-app）负责：构造 AgentRuntime（注入 LLM client + 空 tool_ctx）、
//! 调本模块、处理失败降级（fallback_from_character）。

use tokio::sync::watch;
use tracing::{info, warn};

use storyforge_domain::Id;
use storyforge_domain::character::{Character, CharacterDefinition};
use storyforge_domain::llm::ChatResponse;
use storyforge_domain::variables::{VariableField, default_character_variables, merge_schema};

use crate::prompts::{
    build_character_extractor_user_msg, make_character_extractor_config,
    register_character_extractor_tools,
};
use crate::runtime::AgentRuntime;
use crate::tools::ToolRegistry;
use crate::{AgentConfig, AgentError};

/// 角色识别输出解析错误
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("LLM 调用失败: {0}")]
    Agent(#[from] AgentError),

    #[error("输出解析失败（5 层兜底全miss）: {0}")]
    Parse(String),
}

/// 跑角色识别 Agent，产出卡内角色定义列表
///
/// `mvu_schema` 是从卡 extensions 探测出的 MVU stat_data 字段（可为空），
/// 会合并进每个 definition 的 variable_schema。
pub async fn extract_characters(
    runtime: &AgentRuntime,
    character: &Character,
    mvu_schema: &[VariableField],
    cancel: watch::Receiver<bool>,
) -> Result<Vec<CharacterDefinition>, ExtractError> {
    let config: AgentConfig = make_character_extractor_config();
    let user_msg = build_character_extractor_user_msg(character);

    let mut registry = ToolRegistry::new();
    register_character_extractor_tools(&mut registry);

    info!(target: "character-extractor", "开始识别卡「{}」的角色", character.name);

    let resp = runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
        .await?;

    let mut defs = parse_character_definitions_from_response(&resp)?;
    if defs.is_empty() {
        return Err(ExtractError::Parse("识别结果为空".into()));
    }

    // 回填 card_id 占位（调用方在挂到 CharacterCard 时覆盖）
    // + 合并 MVU schema
    let pending_card_id = Id::from_str("__pending__");
    for def in &mut defs {
        def.card_id = pending_card_id.clone();
        def.variable_schema = merge_schema(&default_character_variables(), mvu_schema);
    }

    info!(target: "character-extractor", "识别出 {} 个角色", defs.len());
    for d in &defs {
        info!(target: "character-extractor", "  - {}（{:?}）", d.name, d.role_type);
    }
    Ok(defs)
}

/// 把识别出的定义回填到指定 card_id（导入完成后调用）
pub fn attach_definitions_to_card(
    mut definitions: Vec<CharacterDefinition>,
    card_id: &Id,
) -> Vec<CharacterDefinition> {
    for def in &mut definitions {
        def.card_id = card_id.clone();
    }
    definitions
}

// ─── 5 层兜底解析（参考 app-pipeline::parse_plan_from_response）──────────

#[derive(Debug, serde::Deserialize)]
struct CharacterDefDto {
    name: String,
    #[serde(default)]
    persona_prompt: String,
    #[serde(default)]
    behavior_rules: String,
    #[serde(default)]
    base_backstory: Vec<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default = "default_role_type_str")]
    role_type: String,
}

fn default_role_type_str() -> String {
    "supporting".to_string()
}

fn parse_role_type(s: &str) -> storyforge_domain::character::RoleType {
    use storyforge_domain::character::RoleType;
    match s.to_lowercase().as_str() {
        "protagonist" | "主角" => RoleType::Protagonist,
        "extra" | "临场" | "龙套" => RoleType::Extra,
        _ => RoleType::Supporting,
    }
}

fn dto_to_definition(dto: CharacterDefDto) -> CharacterDefinition {
    CharacterDefinition {
        id: Id::new(),
        card_id: Id::from_str("__pending__"),
        name: dto.name,
        persona_prompt: dto.persona_prompt,
        behavior_rules: dto.behavior_rules,
        base_backstory: dto.base_backstory,
        group: dto.group,
        role_type: parse_role_type(&dto.role_type),
        variable_schema: default_character_variables(),
    }
}

/// 从 LLM 响应解析角色定义数组（5 层兜底）
pub fn parse_character_definitions_from_response(
    resp: &ChatResponse,
) -> Result<Vec<CharacterDefinition>, ExtractError> {
    // 层 1：emit_characters 工具调用（arguments 是 JSON 字符串）
    for tc in &resp.tool_calls {
        if tc.function.name == "emit_characters"
            && let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
            && let Some(arr) = args.get("characters")
            && let Ok(defs) = serde_json::from_value::<Vec<CharacterDefDto>>(arr.clone())
        {
            let parsed: Vec<CharacterDefinition> =
                defs.into_iter().map(dto_to_definition).collect();
            if !parsed.is_empty() {
                return Ok(parsed);
            }
        }
    }

    // 层 2-5：从 content 提取 JSON 数组
    let content = resp.content.trim();
    if !content.is_empty()
        && let Some(defs) = parse_definitions_from_content(content)
    {
        return Ok(defs);
    }

    Err(ExtractError::Parse(format!(
        "5 层兜底全miss；content 前 200 字: {}",
        content.chars().take(200).collect::<String>()
    )))
}

/// 从 content 文本解析角色定义数组（层 2-5 共用）
fn parse_definitions_from_content(content: &str) -> Option<Vec<CharacterDefinition>> {
    // 层 2：整个 content 是 JSON 数组
    if let Ok(defs) = serde_json::from_str::<Vec<CharacterDefDto>>(content) {
        let parsed: Vec<CharacterDefinition> = defs.into_iter().map(dto_to_definition).collect();
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }

    // 层 3：```json 代码块
    if let Some(extracted) = try_extract_codeblock(content, "json")
        && let Ok(defs) = serde_json::from_str::<Vec<CharacterDefDto>>(&extracted)
    {
        let parsed: Vec<CharacterDefinition> = defs.into_iter().map(dto_to_definition).collect();
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }

    // 层 4：裸代码块
    if let Some(extracted) = try_extract_codeblock(content, "")
        && let Ok(defs) = serde_json::from_str::<Vec<CharacterDefDto>>(&extracted)
    {
        let parsed: Vec<CharacterDefinition> = defs.into_iter().map(dto_to_definition).collect();
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }

    // 层 5：手写括号配平（找 [...] 数组里的多个 {...}）
    if let Some(defs) = try_extract_bracket_array(content) {
        return Some(defs);
    }

    None
}

/// 从 content 提取指定语言的代码块内容
fn try_extract_codeblock(content: &str, lang: &str) -> Option<String> {
    crate::llm_parse::extract_codeblock(content, lang)
}

/// 手写括号配平：找 `[` ... `]` 数组，逐个提取 `{...}` 对象解析
///
/// 模型有时会输出 "好的，识别结果如下：[{...},{...}]"，整体不是合法 JSON，
/// 但逐个对象是合法的。
fn try_extract_bracket_array(content: &str) -> Option<Vec<CharacterDefinition>> {
    // 找第一个 `[`
    let arr_start = content.find('[')?;
    let after_bracket = &content[arr_start + 1..];

    // 逐个找 `{` 并配平
    let mut defs = Vec::new();
    let mut search_from = 0;
    while search_from < after_bracket.len() {
        let rel = match after_bracket[search_from..].find('{') {
            Some(r) => r,
            None => break,
        };
        let brace_start = search_from + rel;
        if let Some(end) = crate::llm_parse::match_braces(after_bracket, brace_start) {
            let candidate = &after_bracket[brace_start..=end];
            if let Ok(dto) = serde_json::from_str::<CharacterDefDto>(candidate) {
                defs.push(dto_to_definition(dto));
            } else {
                warn!(target: "character-extractor", "括号配平段解析失败，跳过");
            }
            search_from = end + 1;
        } else {
            break; // 未闭合，放弃
        }
    }

    if defs.is_empty() { None } else { Some(defs) }
}

// 抑制未用警告：attach_definitions_to_card / merge_schema 等是公开 API
#[allow(unused_imports)]
use crate::AgentRuntime as _AgentRuntimeReexport;

// ─── 测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;
    use storyforge_domain::character::RoleType;
    use storyforge_domain::llm::{ChatResponse, FunctionCall, ToolCall, Usage};

    fn make_resp(content: &str, tool_calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            content: content.into(),
            reasoning_content: None,
            tool_calls,
            finish_reason: Some("stop".into()),
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
                ..Default::default()
            }),
        }
    }

    fn two_char_json() -> String {
        r#"[
          {
            "name": "林医生",
            "persona_prompt": "你是外科医生",
            "behavior_rules": "面对病人先评估",
            "base_backstory": ["三年前失败手术"],
            "role_type": "protagonist",
            "group": "主角团"
          },
          {
            "name": "陈警官",
            "persona_prompt": "你是老刑警",
            "behavior_rules": "",
            "base_backstory": [],
            "role_type": "supporting"
          }
        ]"#
        .to_string()
    }

    #[test]
    fn test_parse_layer1_tool_call() {
        let args = serde_json::json!({
            "characters": serde_json::from_str::<serde_json::Value>(&two_char_json()).unwrap()
        })
        .to_string();
        let resp = make_resp(
            "",
            vec![ToolCall {
                id: "call_1".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "emit_characters".into(),
                    arguments: args,
                },
            }],
        );
        let defs = parse_character_definitions_from_response(&resp).unwrap();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "林医生");
        assert_eq!(defs[0].role_type, RoleType::Protagonist);
        assert_eq!(defs[1].role_type, RoleType::Supporting);
    }

    #[test]
    fn test_parse_layer2_whole_json() {
        let resp = make_resp(&two_char_json(), vec![]);
        let defs = parse_character_definitions_from_response(&resp).unwrap();
        assert_eq!(defs.len(), 2);
    }

    #[test]
    fn test_parse_layer3_json_codeblock() {
        let content = format!("识别结果：\n```json\n{}\n```", two_char_json());
        let resp = make_resp(&content, vec![]);
        let defs = parse_character_definitions_from_response(&resp).unwrap();
        assert_eq!(defs.len(), 2);
    }

    #[test]
    fn test_parse_layer5_braces_among_text() {
        // 中文文字夹杂的 JSON 数组（整体非法 JSON，但逐对象合法）
        let content =
            "好的，识别如下：[{\"name\":\"A\",\"persona_prompt\":\"pa\"},{\"name\":\"B\",\"persona_prompt\":\"pb\"}] 完成"
                .to_string();
        let resp = make_resp(&content, vec![]);
        let defs = parse_character_definitions_from_response(&resp).unwrap();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "A");
    }

    #[test]
    fn test_parse_role_type_chinese() {
        let dto = CharacterDefDto {
            name: "x".into(),
            persona_prompt: "p".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: "主角".into(),
        };
        let def = dto_to_definition(dto);
        assert_eq!(def.role_type, RoleType::Protagonist);
    }

    #[test]
    fn test_parse_failure_returns_error() {
        let resp = make_resp("完全不是 JSON 的乱七八糟文本", vec![]);
        assert!(parse_character_definitions_from_response(&resp).is_err());
    }

    #[test]
    fn test_attach_definitions_sets_card_id() {
        let mut def = CharacterDefinition::fallback_from_character(&make_dummy_character(), &[]);
        def.card_id = Id::from_str("__pending__");
        let card_id = Id::from_str("card-xyz");
        let defs = attach_definitions_to_card(vec![def], &card_id);
        assert_eq!(defs[0].card_id, card_id);
    }

    fn make_dummy_character() -> Character {
        use storyforge_domain::Source;
        Character {
            id: Id::new(),
            name: "dummy".into(),
            description: "x".into(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::Value::Null,
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        }
    }

    /// 端到端：MockLlmClient 跑识别 Agent 闭环
    #[tokio::test]
    async fn test_extract_characters_with_mock_llm() {
        use storyforge_domain::llm::ChatMessage;
        use storyforge_infra_llm::LlmClient;
        use storyforge_infra_llm::mock_client::MockLlmClient;

        // MockLlmClient 的识别脚本匹配 "卡内角色识别"
        // 验证它能命中
        let client = MockLlmClient::with_defaults();
        let req = storyforge_domain::llm::ChatRequest {
            messages: vec![
                ChatMessage::system("你是卡内角色识别助手"),
                ChatMessage::user("分析这张卡"),
            ],
            tools: None,
            params: Default::default(),
            model: "mock".into(),
        };
        let resp = client.chat(&req).await.unwrap();
        // 应能解析出至少一个角色
        let defs = parse_character_definitions_from_response(&resp).unwrap();
        assert!(!defs.is_empty());
    }
}

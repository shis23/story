//! 后处理 Agent 编排与输出解析（对应 AGENT_INTERFACES §6.4，D40-D41/D45）
//!
//! 流程：
//! 1. `run_postprocess` 调 AgentRuntime::run_tool_loop 跑后处理 Agent
//! 2. `parse_postprocess_from_response` 5 层兜底解析输出
//! 3. 调用方（app-pipeline）把 PostProcessResult 写进 CampaignStore
//!
//! 输出是 best-effort：解析失败返回空 PostProcessResult，不报错（不阻断成文）。

use tokio::sync::watch;
use tracing::{info, warn};

use storyforge_domain::Id;
use storyforge_domain::agent::{PostProcessResult, VariableUpdate};
use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};
use storyforge_domain::llm::ChatResponse;
use storyforge_domain::story_task::{NewTaskSpec, TaskStatus, TaskTrigger, TaskUpdate};

use crate::prompts::{
    build_postprocess_user_msg, make_postprocess_config, register_postprocess_tools,
};
use crate::runtime::AgentRuntime;
use crate::tools::ToolRegistry;
use crate::{AgentConfig, AgentError};

#[derive(Debug, thiserror::Error)]
pub enum PostProcessError {
    #[error("LLM 调用失败: {0}")]
    Agent(#[from] AgentError),
}

/// 跑后处理 Agent，产出三件套
pub async fn run_postprocess(
    runtime: &AgentRuntime,
    final_text: &str,
    present_characters: &[String],
    variable_keys: &[String],
    turn: u32,
    story_clock: &str,
    cancel: watch::Receiver<bool>,
) -> Result<PostProcessResult, PostProcessError> {
    let config: AgentConfig = make_postprocess_config();
    let user_msg = build_postprocess_user_msg(
        final_text,
        present_characters,
        variable_keys,
        turn,
        story_clock,
    );

    let mut registry = ToolRegistry::new();
    register_postprocess_tools(&mut registry);

    info!(target: "postprocess", "开始后处理（在场 {} 角色）", present_characters.len());

    let resp = runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
        .await?;

    let result = parse_postprocess_from_response(&resp);
    info!(
        target: "postprocess",
        "后处理完成：知识 {} / 变量 {} / 任务 {}",
        result.knowledge_updates.len(),
        result.variable_updates.len(),
        result.task_updates.len()
    );
    Ok(result)
}

// ─── 5 层兜底解析 ─────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct PostProcessDto {
    #[serde(default)]
    knowledge_updates: Vec<KnowledgeUpdateDto>,
    #[serde(default)]
    variable_updates: Vec<VariableUpdateDto>,
    #[serde(default)]
    task_updates: Vec<TaskUpdateDto>,
}

#[derive(Debug, serde::Deserialize)]
struct KnowledgeUpdateDto {
    character_id: String,
    knowledge_text: String,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default)]
    source_character_id: Option<String>,
    #[serde(default)]
    pinned: bool,
}
fn default_source() -> String {
    "witnessed".to_string()
}

#[derive(Debug, serde::Deserialize)]
struct VariableUpdateDto {
    #[serde(default)]
    instance_id: Option<String>,
    key: String,
    value: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
struct TaskUpdateDto {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default = "default_task_status")]
    new_status: String,
    #[serde(default)]
    new_task: Option<NewTaskDto>,
}
fn default_task_status() -> String {
    "pending".to_string()
}

#[derive(Debug, serde::Deserialize)]
struct NewTaskDto {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    triggers: Vec<TaskTrigger>,
    #[serde(default)]
    related_characters: Vec<String>,
}

fn parse_source(s: &str) -> KnowledgeSource {
    match s.to_lowercase().as_str() {
        "told_by_other" | "被告知" => KnowledgeSource::ToldByOther,
        "inferred" | "推断" => KnowledgeSource::Inferred,
        _ => KnowledgeSource::Witnessed,
    }
}

fn parse_status(s: &str) -> TaskStatus {
    // LikelyCompleted 需要置信度，解析时无 confidence 字段，统一给 0.5
    match s.to_lowercase().as_str() {
        "active" => TaskStatus::Active,
        "likely_completed" | "likelycompleted" => TaskStatus::LikelyCompleted { confidence: 0.5 },
        "completed" => TaskStatus::Completed,
        "abandoned" => TaskStatus::Abandoned,
        _ => TaskStatus::Pending,
    }
}

fn dto_to_result(dto: PostProcessDto) -> PostProcessResult {
    let knowledge_updates = dto
        .knowledge_updates
        .into_iter()
        .map(|k| CharacterKnowledgeUpdate {
            character_id: Id::from_str(&k.character_id),
            knowledge_text: k.knowledge_text,
            source: parse_source(&k.source),
            source_character_id: k.source_character_id.map(|s| Id::from_str(&s)),
            pinned: k.pinned,
        })
        .collect();

    let variable_updates = dto
        .variable_updates
        .into_iter()
        .map(|v| VariableUpdate {
            instance_id: v.instance_id.map(|s| Id::from_str(&s)),
            key: v.key,
            value: v.value,
        })
        .collect();

    let task_updates = dto
        .task_updates
        .into_iter()
        .filter_map(|t| {
            let task_id = t.task_id.map(|s| Id::from_str(&s));
            let new_status = parse_status(&t.new_status);
            let new_task = t.new_task.map(|n| NewTaskSpec {
                title: n.title,
                description: n.description,
                triggers: n.triggers,
                related_characters: n
                    .related_characters
                    .into_iter()
                    .map(|s| Id::from_str(&s))
                    .collect(),
            });
            // task_id=None 时必须有 new_task
            if task_id.is_none() && new_task.is_none() {
                warn!(target: "postprocess", "task_update 无 task_id 也无 new_task，跳过");
                return None;
            }
            Some(TaskUpdate {
                task_id,
                new_status,
                new_task,
            })
        })
        .collect();

    PostProcessResult {
        knowledge_updates,
        variable_updates,
        task_updates,
        parse_succeeded: true,
    }
}

/// 从 LLM 响应解析后处理结果（5 层兜底，best-effort：失败返回空）
pub fn parse_postprocess_from_response(resp: &ChatResponse) -> PostProcessResult {
    // 层 1：emit_postprocess 工具调用
    for tc in &resp.tool_calls {
        if tc.function.name == "emit_postprocess" {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
                if let Ok(dto) = serde_json::from_value::<PostProcessDto>(args) {
                    return dto_to_result(dto);
                }
            }
        }
    }

    // 层 2-5：从 content 提取 JSON
    let content = resp.content.trim();
    if !content.is_empty() {
        if let Some(mut result) = parse_from_content(content) {
            result.parse_succeeded = true;
            return result;
        }
    }

    warn!(target: "postprocess", "5 层兜底全miss，返回空 result（best-effort）");
    PostProcessResult {
        parse_succeeded: false,
        ..Default::default()
    }
}

fn parse_from_content(content: &str) -> Option<PostProcessResult> {
    // 层 2：整体 JSON
    if let Ok(dto) = serde_json::from_str::<PostProcessDto>(content) {
        return Some(dto_to_result(dto));
    }
    // 层 3：```json 块
    if let Some(extracted) = extract_codeblock(content, "json") {
        if let Ok(dto) = serde_json::from_str::<PostProcessDto>(&extracted) {
            return Some(dto_to_result(dto));
        }
    }
    // 层 4：裸代码块
    if let Some(extracted) = extract_codeblock(content, "") {
        if let Ok(dto) = serde_json::from_str::<PostProcessDto>(&extracted) {
            return Some(dto_to_result(dto));
        }
    }
    // 层 5：手写括号配平（找第一个 {...}）
    if let Some(json_str) = extract_first_braces(content) {
        if let Ok(dto) = serde_json::from_str::<PostProcessDto>(&json_str) {
            return Some(dto_to_result(dto));
        }
    }
    None
}

fn extract_codeblock(content: &str, lang: &str) -> Option<String> {
    crate::llm_parse::extract_codeblock(content, lang)
}

/// 从 content 找第一个配平的 {...}（委托公共模块，retry=true 更健壮）
fn extract_first_braces(content: &str) -> Option<String> {
    crate::llm_parse::extract_first_braces(content, true)
}

// ─── 测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::{ChatResponse, FunctionCall, ToolCall, Usage};

    fn make_resp(content: &str, tool_calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            content: content.into(),
            tool_calls,
            finish_reason: Some("stop".into()),
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            }),
        }
    }

    fn sample_json() -> String {
        r#"{
          "knowledge_updates": [
            {"character_id": "林医生", "knowledge_text": "我看到尸体", "source": "witnessed"}
          ],
          "variable_updates": [
            {"instance_id": "林医生", "key": "state", "value": "受伤"},
            {"instance_id": null, "key": "story_clock", "value": "第2天"}
          ],
          "task_updates": [
            {"task_id": null, "new_status": "pending", "new_task": {"title": "复仇", "description": "老王复仇", "triggers": [{"kind": "event", "description": "期限到达"}], "related_characters": ["老王"]}}
          ]
        }"#
        .to_string()
    }

    #[test]
    fn test_parse_layer1_tool_call() {
        let args = sample_json();
        let resp = make_resp(
            "",
            vec![ToolCall {
                id: "c1".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "emit_postprocess".into(),
                    arguments: args,
                },
            }],
        );
        let r = parse_postprocess_from_response(&resp);
        assert_eq!(r.knowledge_updates.len(), 1);
        assert_eq!(r.variable_updates.len(), 2);
        assert_eq!(r.task_updates.len(), 1);
    }

    #[test]
    fn test_parse_layer2_whole_json() {
        let resp = make_resp(&sample_json(), vec![]);
        let r = parse_postprocess_from_response(&resp);
        assert_eq!(r.knowledge_updates.len(), 1);
        assert_eq!(r.variable_updates[1].instance_id, None);
        assert_eq!(r.variable_updates[1].key, "story_clock");
    }

    #[test]
    fn test_parse_layer3_codeblock() {
        let content = format!("结果：\n```json\n{}\n```", sample_json());
        let resp = make_resp(&content, vec![]);
        let r = parse_postprocess_from_response(&resp);
        assert_eq!(r.knowledge_updates.len(), 1);
    }

    #[test]
    fn test_parse_layer5_braces_among_text() {
        let content = format!("好的：{} 完成", sample_json());
        let resp = make_resp(&content, vec![]);
        let r = parse_postprocess_from_response(&resp);
        assert_eq!(r.knowledge_updates.len(), 1);
    }

    #[test]
    fn test_parse_failure_returns_empty() {
        let resp = make_resp("完全不是 JSON", vec![]);
        let r = parse_postprocess_from_response(&resp);
        assert!(r.is_empty());
    }

    #[test]
    fn test_parse_source_classes() {
        assert!(matches!(
            parse_source("witnessed"),
            KnowledgeSource::Witnessed
        ));
        assert!(matches!(
            parse_source("told_by_other"),
            KnowledgeSource::ToldByOther
        ));
        assert!(matches!(
            parse_source("inferred"),
            KnowledgeSource::Inferred
        ));
    }

    #[tokio::test]
    async fn test_end_to_end_with_mock_llm() {
        use storyforge_domain::llm::ChatMessage;
        use storyforge_infra_llm::LlmClient;
        use storyforge_infra_llm::mock_client::MockLlmClient;

        let client = MockLlmClient::with_defaults();
        let req = storyforge_domain::llm::ChatRequest {
            messages: vec![
                ChatMessage::system("你是后处理助手"),
                ChatMessage::user("分析"),
            ],
            tools: None,
            params: Default::default(),
            model: "mock".into(),
        };
        let resp = client.chat(&req).await.unwrap();
        let r = parse_postprocess_from_response(&resp);
        // mock 脚本应产出非空结果
        assert!(!r.is_empty(), "mock 后处理脚本应产出非空结果");
    }
}

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
use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::character_knowledge::{
    BroadcastTarget, CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
};
use storyforge_domain::llm::{ChatMessage, ChatRequest, ChatResponse};
use storyforge_domain::story_task::{NewTaskSpec, TaskStatus, TaskTrigger, TaskUpdate};

use crate::prompts::{
    build_postprocess_user_msg_with_summary, make_postprocess_config, register_postprocess_tools,
};
use crate::runtime::AgentRuntime;
use crate::tools::{ToolRegistry, filter_registry_by_whitelist};
use crate::{AgentConfig, AgentError};

#[derive(Debug, thiserror::Error)]
pub enum PostProcessError {
    #[error("LLM 调用失败: {0}")]
    Agent(#[from] AgentError),
}

/// 跑后处理 Agent，产出三件套
///
/// `agent_profile_config`（可选）用于覆盖 PostProcessor 的 model/rounds 并过滤 tool_whitelist。
/// 传 None = 当前硬编码默认值，向后兼容。
#[allow(clippy::too_many_arguments)]
pub async fn run_postprocess(
    runtime: &AgentRuntime,
    final_text: &str,
    present_characters: &[String],
    variable_keys: &[String],
    turn: u32,
    story_clock: &str,
    cancel: watch::Receiver<bool>,
    agent_profile_config: Option<&AgentProfileConfig>,
    recent_summary_block: Option<&str>,
) -> Result<PostProcessResult, PostProcessError> {
    let config: AgentConfig = make_postprocess_config(agent_profile_config);
    let user_msg = build_postprocess_user_msg_with_summary(
        final_text,
        present_characters,
        variable_keys,
        turn,
        story_clock,
        recent_summary_block,
    );

    let mut registry = ToolRegistry::new();
    register_postprocess_tools(&mut registry);
    // 应用 PostProcessor tool_whitelist（None=默认，Some=过滤/清空）
    let wl = agent_profile_config.and_then(|apc| {
        apc.run_config_for(&storyforge_domain::agent::AgentRole::PostProcessor)
            .tool_whitelist
            .as_deref()
    });
    filter_registry_by_whitelist(&mut registry, wl, "PostProcessor");

    info!(target: "postprocess", "开始后处理（在场 {} 角色）", present_characters.len());

    let fallback_user_msg = user_msg.clone();
    let fallback_cancel = cancel.clone();
    let resp = runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
        .await?;

    let result = parse_postprocess_from_response(&resp);
    let result = if result.parse_succeeded && !result.is_empty() {
        result
    } else {
        run_direct_json_fallback(runtime, &config, &fallback_user_msg, fallback_cancel)
            .await
            .map_err(PostProcessError::Agent)?
            .unwrap_or(result)
    };
    info!(
        target: "postprocess",
        "后处理完成：知识 {} / 变量 {} / 任务 {}",
        result.knowledge_updates.len(),
        result.variable_updates.len(),
        result.task_updates.len()
    );
    Ok(result)
}

async fn run_direct_json_fallback(
    runtime: &AgentRuntime,
    base_config: &AgentConfig,
    base_user_msg: &str,
    cancel: watch::Receiver<bool>,
) -> Result<Option<PostProcessResult>, AgentError> {
    warn!(
        target: "postprocess",
        "emit_postprocess/JSON parse missed or returned empty; retrying postprocess once without tools"
    );

    if *cancel.borrow() {
        return Err(AgentError::Cancelled);
    }

    let user_msg = format!(
        "{base_user_msg}\n\nReturn exactly one JSON object with keys knowledge_updates, variable_updates, and task_updates. Do not include markdown, code fences, prose, or tool calls."
    );
    let req = ChatRequest {
        messages: vec![
            ChatMessage::system(&base_config.system_prompt),
            ChatMessage::user(&user_msg),
        ],
        tools: None,
        // P2-6：knowledge_updates JSON 在多角色场景下较大，默认 max_tokens=4096
        // 会被截断导致解析失败（knowledge.json 不生成）。fallback 专门做 JSON 抽取,
        // 无工具调用开销,放宽到 8192 降低截断概率。
        params: storyforge_domain::llm::SamplingParams {
            temperature: Some(0.3),
            top_p: Some(0.95),
            max_tokens: Some(8192),
            max_tokens_explicit: false,
            reasoning: storyforge_domain::llm::ReasoningMode::default(),
            extra: None,
        },
        model: base_config.model.clone(),
    };
    let llm = runtime.llm();
    let cancel_fut = {
        let mut cancel = cancel.clone();
        async move {
            let _ = cancel.wait_for(|&c| c).await;
        }
    };

    let resp = tokio::select! {
        result = llm.chat(&req) => result.map_err(AgentError::Llm),
        _ = cancel_fut => Err(AgentError::Cancelled),
    };

    match resp {
        Ok(resp) => {
            let result = parse_postprocess_from_response(&resp);
            if result.parse_succeeded {
                Ok(Some(result))
            } else {
                warn!(
                    target: "postprocess",
                    "direct JSON postprocess fallback also missed; keeping empty best-effort result"
                );
                Ok(None)
            }
        }
        Err(AgentError::Cancelled) => Err(AgentError::Cancelled),
        Err(err) => {
            warn!(
                target: "postprocess",
                "direct JSON postprocess fallback failed: {err}"
            );
            Ok(None)
        }
    }
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
    /// 广播目标："all" = 全体; 其他字符串 = 身份组名; null/缺失 = 不广播
    #[serde(default)]
    broadcast: Option<String>,
    /// 传播策略："open" = 默认; "private"/"secret"/"sealed" = 禁止外传
    #[serde(default)]
    propagation: Option<String>,
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

fn parse_propagation(s: Option<&str>) -> PropagationPolicy {
    let Some(raw) = s else {
        return PropagationPolicy::Open;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PropagationPolicy::Open;
    }
    match trimmed.to_lowercase().as_str() {
        "open" => PropagationPolicy::Open,
        "private" | "secret" | "sealed" | "no_share" | "no-share" | "禁止外传" | "秘密"
        | "封口" => PropagationPolicy::Private,
        lower if lower.starts_with("group:") => {
            PropagationPolicy::GroupRestricted(trimmed[6..].trim().to_string())
        }
        _ => PropagationPolicy::Open,
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
            broadcast: k.broadcast.and_then(|s| match s.as_str() {
                "all" => Some(BroadcastTarget::All),
                "" => None,
                group => Some(BroadcastTarget::Group(group.to_string())),
            }),
            propagation: parse_propagation(k.propagation.as_deref()),
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
        if tc.function.name == "emit_postprocess"
            && let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
            && let Ok(dto) = serde_json::from_value::<PostProcessDto>(args)
        {
            return dto_to_result(dto);
        }
    }

    // 层 2-5：从 content 提取 JSON
    let content = resp.content.trim();
    if !content.is_empty()
        && let Some(mut result) = parse_from_content(content)
    {
        result.parse_succeeded = true;
        return result;
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
    if let Some(extracted) = extract_codeblock(content, "json")
        && let Ok(dto) = serde_json::from_str::<PostProcessDto>(&extracted)
    {
        return Some(dto_to_result(dto));
    }
    // 层 4：裸代码块
    if let Some(extracted) = extract_codeblock(content, "")
        && let Ok(dto) = serde_json::from_str::<PostProcessDto>(&extracted)
    {
        return Some(dto_to_result(dto));
    }
    // 层 5：手写括号配平（找第一个 {...}）
    if let Some(json_str) = extract_first_braces(content)
        && let Ok(dto) = serde_json::from_str::<PostProcessDto>(&json_str)
    {
        return Some(dto_to_result(dto));
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use storyforge_domain::llm::{
        ChatRequest, ChatResponse, FunctionCall, LlmError, StreamChunk, ToolCall, Usage,
    };
    use tokio::sync::Notify;

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

    fn empty_tool_context() -> Arc<crate::tools::ToolContext> {
        Arc::new(crate::tools::ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        })
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

    #[test]
    fn test_parse_broadcast_all_from_json() {
        let content = r#"{
          "knowledge_updates": [
            {
              "character_id": "城主",
              "knowledge_text": "城主宣告全城戒严",
              "source": "witnessed",
              "source_character_id": "城主",
              "broadcast": "all"
            }
          ],
          "variable_updates": [],
          "task_updates": []
        }"#;

        let resp = make_resp(content, vec![]);
        let r = parse_postprocess_from_response(&resp);

        assert_eq!(r.knowledge_updates.len(), 1);
        assert_eq!(r.knowledge_updates[0].broadcast, Some(BroadcastTarget::All));
    }

    #[test]
    fn test_parse_broadcast_group_from_json() {
        let content = r#"{
          "knowledge_updates": [
            {
              "character_id": "队长",
              "knowledge_text": "所有守卫都收到戒严令",
              "source": "told_by_other",
              "source_character_id": "队长",
              "broadcast": "守卫"
            }
          ],
          "variable_updates": [],
          "task_updates": []
        }"#;

        let resp = make_resp(content, vec![]);
        let r = parse_postprocess_from_response(&resp);

        assert_eq!(r.knowledge_updates.len(), 1);
        assert_eq!(
            r.knowledge_updates[0].broadcast,
            Some(BroadcastTarget::Group("守卫".to_string()))
        );
    }

    #[test]
    fn test_parse_private_propagation_from_json() {
        use storyforge_domain::character_knowledge::PropagationPolicy;

        let content = r#"{
          "knowledge_updates": [
            {
              "character_id": "林医生",
              "knowledge_text": "保险柜密码是 0427",
              "source": "witnessed",
              "propagation": "private"
            }
          ],
          "variable_updates": [],
          "task_updates": []
        }"#;

        let resp = make_resp(content, vec![]);
        let r = parse_postprocess_from_response(&resp);

        assert_eq!(r.knowledge_updates.len(), 1);
        assert_eq!(
            r.knowledge_updates[0].propagation,
            PropagationPolicy::Private
        );
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

    struct ToolDriftThenJsonClient {
        calls: Arc<AtomicUsize>,
        fallback_json: String,
    }

    #[async_trait]
    impl storyforge_infra_llm::LlmClient for ToolDriftThenJsonClient {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if req.tools.as_ref().is_some_and(|tools| !tools.is_empty()) {
                return Ok(make_resp(
                    "I have analyzed it, but this is not JSON.",
                    vec![],
                ));
            }
            Ok(make_resp(&self.fallback_json, vec![]))
        }

        async fn chat_stream(
            &self,
            req: &ChatRequest,
            tx: tokio::sync::mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            let resp = self.chat(req).await?;
            let _ = tx.send(StreamChunk {
                delta_content: Some(resp.content.clone()),
                delta_reasoning_content: resp.reasoning_content.clone(),
                delta_tool_calls: None,
                finish_reason: resp.finish_reason.clone(),
            });
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn test_run_postprocess_retries_direct_json_when_tool_path_drifts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(ToolDriftThenJsonClient {
            calls: calls.clone(),
            fallback_json: sample_json(),
        });
        let runtime = AgentRuntime::new(llm, empty_tool_context());
        let (_cancel_tx, cancel) = watch::channel(false);

        let result = run_postprocess(
            &runtime,
            "林医生告诉陈警官地下室有尸体。",
            &["林医生".into(), "陈警官".into()],
            &["story_clock".into()],
            7,
            "第 7 轮",
            cancel,
            None,
            None,
        )
        .await
        .expect("postprocess should recover via direct JSON fallback");

        assert!(result.parse_succeeded);
        assert_eq!(result.knowledge_updates.len(), 1);
        assert!(
            calls.load(Ordering::SeqCst) > 1,
            "first tool path should miss before direct JSON fallback runs"
        );
    }

    struct EmptyJsonThenUsefulJsonClient {
        calls: Arc<AtomicUsize>,
        fallback_json: String,
    }

    #[async_trait]
    impl storyforge_infra_llm::LlmClient for EmptyJsonThenUsefulJsonClient {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if req.tools.as_ref().is_some_and(|tools| !tools.is_empty()) {
                return Ok(make_resp(
                    r#"{"knowledge_updates":[],"variable_updates":[],"task_updates":[]}"#,
                    vec![],
                ));
            }
            Ok(make_resp(&self.fallback_json, vec![]))
        }

        async fn chat_stream(
            &self,
            req: &ChatRequest,
            tx: tokio::sync::mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            let resp = self.chat(req).await?;
            let _ = tx.send(StreamChunk {
                delta_content: Some(resp.content.clone()),
                delta_reasoning_content: resp.reasoning_content.clone(),
                delta_tool_calls: None,
                finish_reason: resp.finish_reason.clone(),
            });
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn test_run_postprocess_retries_direct_json_when_tool_path_returns_empty_json() {
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(EmptyJsonThenUsefulJsonClient {
            calls: calls.clone(),
            fallback_json: sample_json(),
        });
        let runtime = AgentRuntime::new(llm, empty_tool_context());
        let (_cancel_tx, cancel) = watch::channel(false);

        let result = run_postprocess(
            &runtime,
            "林医生告诉陈警官地下室有尸体。",
            &["林医生".into(), "陈警官".into()],
            &["story_clock".into()],
            7,
            "第 7 轮",
            cancel,
            None,
            None,
        )
        .await
        .expect("postprocess should recover when primary tool path returns an empty JSON result");

        assert!(result.parse_succeeded);
        assert_eq!(result.knowledge_updates.len(), 1);
        assert!(
            calls.load(Ordering::SeqCst) > 1,
            "valid-but-empty primary JSON should trigger direct JSON fallback"
        );
    }

    struct CancelBeforeFallbackClient {
        calls: Arc<AtomicUsize>,
        cancel_tx: watch::Sender<bool>,
    }

    #[async_trait]
    impl storyforge_infra_llm::LlmClient for CancelBeforeFallbackClient {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if req.tools.as_ref().is_some_and(|tools| !tools.is_empty()) {
                let _ = self.cancel_tx.send(true);
                return Ok(make_resp("not json", vec![]));
            }
            Ok(make_resp(&sample_json(), vec![]))
        }

        async fn chat_stream(
            &self,
            req: &ChatRequest,
            tx: tokio::sync::mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            let resp = self.chat(req).await?;
            let _ = tx.send(StreamChunk {
                delta_content: Some(resp.content.clone()),
                delta_reasoning_content: resp.reasoning_content.clone(),
                delta_tool_calls: None,
                finish_reason: resp.finish_reason.clone(),
            });
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn test_run_postprocess_respects_cancel_before_direct_json_fallback() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (cancel_tx, cancel) = watch::channel(false);
        let llm = Arc::new(CancelBeforeFallbackClient {
            calls: calls.clone(),
            cancel_tx,
        });
        let runtime = AgentRuntime::new(llm, empty_tool_context());

        let result = run_postprocess(
            &runtime,
            "林医生告诉陈警官地下室有尸体。",
            &["林医生".into(), "陈警官".into()],
            &["story_clock".into()],
            7,
            "第 7 轮",
            cancel,
            None,
            None,
        )
        .await;

        assert!(matches!(
            result,
            Err(PostProcessError::Agent(AgentError::Cancelled))
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "fallback LLM call should not start after cancellation"
        );
    }

    struct PendingFallbackClient {
        calls: Arc<AtomicUsize>,
        fallback_started: Arc<Notify>,
    }

    #[async_trait]
    impl storyforge_infra_llm::LlmClient for PendingFallbackClient {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if req.tools.as_ref().is_some_and(|tools| !tools.is_empty()) {
                return Ok(make_resp("not json", vec![]));
            }
            self.fallback_started.notify_one();
            std::future::pending::<Result<ChatResponse, LlmError>>().await
        }

        async fn chat_stream(
            &self,
            req: &ChatRequest,
            tx: tokio::sync::mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            let resp = self.chat(req).await?;
            let _ = tx.send(StreamChunk {
                delta_content: Some(resp.content.clone()),
                delta_reasoning_content: resp.reasoning_content.clone(),
                delta_tool_calls: None,
                finish_reason: resp.finish_reason.clone(),
            });
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn test_run_postprocess_respects_cancel_during_direct_json_fallback() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fallback_started = Arc::new(Notify::new());
        let (cancel_tx, cancel) = watch::channel(false);
        let llm = Arc::new(PendingFallbackClient {
            calls: calls.clone(),
            fallback_started: fallback_started.clone(),
        });
        let handle = tokio::spawn(async move {
            let runtime = AgentRuntime::new(llm, empty_tool_context());
            run_postprocess(
                &runtime,
                "林医生告诉陈警官地下室有尸体。",
                &["林医生".into(), "陈警官".into()],
                &["story_clock".into()],
                7,
                "第 7 轮",
                cancel,
                None,
                None,
            )
            .await
        });

        fallback_started.notified().await;
        cancel_tx.send(true).unwrap();
        let result = handle.await.unwrap();

        assert!(matches!(
            result,
            Err(PostProcessError::Agent(AgentError::Cancelled))
        ));
        assert!(
            calls.load(Ordering::SeqCst) >= 2,
            "primary miss and fallback call should both have been attempted"
        );
    }
}

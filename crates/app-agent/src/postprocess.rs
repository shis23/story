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
    build_postprocess_user_msg_with_context, make_postprocess_config, register_postprocess_tools,
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
///
/// `mvu_update_rules`：卡翻译产物 `MvuTranslation.update_rules`（自然语言规则），
/// 注入用户消息的【卡片变量更新规则】区块。空切片 = 无规则卡，输出与旧版一致。
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
    mvu_update_rules: &[String],
) -> Result<PostProcessResult, PostProcessError> {
    let config: AgentConfig = make_postprocess_config(agent_profile_config);
    let user_msg = build_postprocess_user_msg_with_context(
        final_text,
        present_characters,
        variable_keys,
        turn,
        story_clock,
        recent_summary_block,
        mvu_update_rules,
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
    // W-09：空结果是"LLM 正常返回但本轮无更新"，不再触发第二次完整 LLM 调用。
    let mut result = if result.parse_succeeded {
        result
    } else {
        run_direct_json_fallback(runtime, &config, &fallback_user_msg, fallback_cancel)
            .await
            .map_err(PostProcessError::Agent)?
            .unwrap_or(result)
    };
    // W-11：把名字解析为 CharacterInstance.id（持久化必须用实例 id）。
    normalize_postprocess_identities(&mut result, runtime);
    info!(
        target: "postprocess",
        "后处理完成：知识 {} / 变量 {} / 任务 {}",
        result.knowledge_updates.len(),
        result.variable_updates.len(),
        result.task_updates.len()
    );
    Ok(result)
}

/// W-11：把后处理输出里的名字/ID 归一为已存在的 `CharacterInstance.id`。
///
/// 只做"唯一命中才替换"：id 精确命中保持原样；名称（trim + 大小写不敏感）
/// 唯一命中 → 替换为实例 id；未命中/歧义（含本轮临时实例，不在快照里）→ 保留原值，
/// 由下游 `persist_postprocess_outcome` 的 extras/temps 解析兜底。
fn normalize_postprocess_identities(result: &mut PostProcessResult, runtime: &AgentRuntime) {
    let tool_ctx = runtime.tool_ctx();
    let Some(campaign_runtime) = tool_ctx.campaign_runtime.as_deref() else {
        return;
    };

    let resolve = |raw: &Id| -> Option<Id> {
        let value = raw.as_str().trim();
        if value.is_empty() {
            return None;
        }
        if let Some(inst) = campaign_runtime
            .instances
            .iter()
            .find(|inst| inst.id.as_str() == value)
        {
            return Some(inst.id.clone());
        }
        let mut by_name = campaign_runtime
            .instances
            .iter()
            .filter(|inst| crate::runtime::instance_name_matches(&inst.name, value));
        let first = by_name.next()?;
        // 同名多实例 → 不猜，保留原值让下游诊断
        if by_name.next().is_some() {
            return None;
        }
        Some(first.id.clone())
    };

    for update in &mut result.knowledge_updates {
        if let Some(resolved) = resolve(&update.character_id) {
            update.character_id = resolved;
        }
        if let Some(source) = update.source_character_id.as_ref()
            && let Some(resolved) = resolve(source)
        {
            update.source_character_id = Some(resolved);
        }
    }
    for update in &mut result.variable_updates {
        if let Some(instance_id) = update.instance_id.as_ref()
            && let Some(resolved) = resolve(instance_id)
        {
            update.instance_id = Some(resolved);
        }
    }
    for update in &mut result.task_updates {
        if let Some(task) = update.new_task.as_mut() {
            for related in &mut task.related_characters {
                if let Some(resolved) = resolve(related) {
                    *related = resolved;
                }
            }
        }
    }
}

async fn run_direct_json_fallback(
    runtime: &AgentRuntime,
    base_config: &AgentConfig,
    base_user_msg: &str,
    cancel: watch::Receiver<bool>,
) -> Result<Option<PostProcessResult>, AgentError> {
    warn!(
        target: "postprocess",
        "emit_postprocess/JSON parse missed; retrying postprocess once without tools"
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
    knowledge_updates: Vec<serde_json::Value>,
    #[serde(default)]
    variable_updates: Vec<serde_json::Value>,
    #[serde(default)]
    task_updates: Vec<serde_json::Value>,
}

/// W-05：逐条容错解析。单条畸形只丢该条（warn），不影响其他条目/其他类别。
fn parse_entries<T: serde::de::DeserializeOwned>(
    entries: Vec<serde_json::Value>,
    kind: &str,
) -> Vec<T> {
    let mut out = Vec::with_capacity(entries.len());
    for (index, value) in entries.into_iter().enumerate() {
        match serde_json::from_value::<T>(value) {
            Ok(item) => out.push(item),
            Err(error) => warn!(
                target: "postprocess",
                "postprocess {kind}[{index}] 解析失败，已跳过该条: {error}"
            ),
        }
    }
    out
}

/// W-05：从任意 JSON object 构造 DTO；数组字段非数组时忽略并 warn，
/// 至少含一个已知键才认作后处理结果（否则返回 None 走后续兜底层）。
fn postprocess_dto_from_value(value: &serde_json::Value) -> Option<PostProcessDto> {
    let object = value.as_object()?;
    let known = ["knowledge_updates", "variable_updates", "task_updates"];
    if !known.iter().any(|key| object.contains_key(*key)) {
        return None;
    }
    let array_field = |key: &str| -> Vec<serde_json::Value> {
        match object.get(key) {
            None | Some(serde_json::Value::Null) => vec![],
            Some(serde_json::Value::Array(items)) => items.clone(),
            Some(other) => {
                warn!(target: "postprocess", "postprocess 字段 {key} 不是数组，已忽略: {other}");
                vec![]
            }
        }
    };
    Some(PostProcessDto {
        knowledge_updates: array_field("knowledge_updates"),
        variable_updates: array_field("variable_updates"),
        task_updates: array_field("task_updates"),
    })
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

/// W-10：广播目标归一（trim + 大小写不敏感）；"all"/"全体"/"所有人" → All。
fn parse_broadcast(raw: &str) -> Option<BroadcastTarget> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.to_lowercase().as_str() {
        "all" | "全体" | "所有人" | "全部" => Some(BroadcastTarget::All),
        _ => Some(BroadcastTarget::Group(trimmed.to_string())),
    }
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
    let knowledge_updates =
        parse_entries::<KnowledgeUpdateDto>(dto.knowledge_updates, "knowledge_updates")
            .into_iter()
            .map(|k| CharacterKnowledgeUpdate {
                character_id: Id::from_str(&k.character_id),
                knowledge_text: k.knowledge_text,
                source: parse_source(&k.source),
                source_character_id: k.source_character_id.map(|s| Id::from_str(&s)),
                pinned: k.pinned,
                broadcast: k.broadcast.as_deref().and_then(parse_broadcast),
                propagation: parse_propagation(k.propagation.as_deref()),
            })
            .collect();

    let variable_updates =
        parse_entries::<VariableUpdateDto>(dto.variable_updates, "variable_updates")
            .into_iter()
            .map(|v| VariableUpdate {
                instance_id: v.instance_id.map(|s| Id::from_str(&s)),
                key: v.key,
                value: v.value,
            })
            .collect();

    let task_updates = parse_entries::<TaskUpdateDto>(dto.task_updates, "task_updates")
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
            && let Some(dto) = postprocess_dto_from_value(&args)
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
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content)
        && let Some(dto) = postprocess_dto_from_value(&value)
    {
        return Some(dto_to_result(dto));
    }
    // 层 3：```json 块
    if let Some(extracted) = extract_codeblock(content, "json")
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&extracted)
        && let Some(dto) = postprocess_dto_from_value(&value)
    {
        return Some(dto_to_result(dto));
    }
    // 层 4：裸代码块
    if let Some(extracted) = extract_codeblock(content, "")
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&extracted)
        && let Some(dto) = postprocess_dto_from_value(&value)
    {
        return Some(dto_to_result(dto));
    }
    // 层 5：手写括号配平（找第一个 {...}）
    if let Some(json_str) = extract_first_braces(content)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str)
        && let Some(dto) = postprocess_dto_from_value(&value)
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

    /// W-05：单条畸形条目只跳过该条，不影响其他类别（旧实现整批丢弃）。
    #[test]
    fn malformed_single_entry_does_not_drop_other_classes() {
        let content = r#"{
          "knowledge_updates": [ "不是对象", {"character_id": "城主", "knowledge_text": "戒严", "source": "witnessed"} ],
          "variable_updates": [ {"instance_id": null, "key": "story_clock", "value": "第2天"} ],
          "task_updates": [ {"task_id": null, "new_status": "pending", "new_task": {"title": "复仇", "description": "老王复仇", "triggers": [], "related_characters": []}} ]
        }"#;
        let r = parse_postprocess_from_response(&make_resp(content, vec![]));
        assert!(r.parse_succeeded, "其余类别仍应解析成功");
        assert_eq!(r.knowledge_updates.len(), 1, "畸形条目应被跳过而非整批丢弃");
        assert_eq!(r.knowledge_updates[0].character_id.as_str(), "城主");
        assert_eq!(r.variable_updates.len(), 1);
        assert_eq!(r.task_updates.len(), 1);
    }

    /// W-05：已知键存在但类型不是数组（模型偶发输出）→ 该类别空 + 不算解析失败。
    #[test]
    fn non_array_field_is_empty_and_still_parses() {
        let content = r#"{
          "knowledge_updates": {"character_id": "城主"},
          "variable_updates": [],
          "task_updates": []
        }"#;
        let r = parse_postprocess_from_response(&make_resp(content, vec![]));
        assert!(r.parse_succeeded);
        assert!(r.knowledge_updates.is_empty());
    }

    /// W-10：broadcast 的 all 变体要大小写/空白归一，Group 名字要 trim。
    #[test]
    fn broadcast_normalizes_case_and_whitespace() {
        let all = r#"{
          "knowledge_updates": [{"character_id": "城主", "knowledge_text": "戒严", "source": "witnessed", "broadcast": "  ALL "}],
          "variable_updates": [],
          "task_updates": []
        }"#;
        let r = parse_postprocess_from_response(&make_resp(all, vec![]));
        assert_eq!(r.knowledge_updates[0].broadcast, Some(BroadcastTarget::All));

        let group = r#"{
          "knowledge_updates": [{"character_id": "城主", "knowledge_text": "密令", "source": "witnessed", "broadcast": "  守卫  "}],
          "variable_updates": [],
          "task_updates": []
        }"#;
        let r = parse_postprocess_from_response(&make_resp(group, vec![]));
        assert_eq!(
            r.knowledge_updates[0].broadcast,
            Some(BroadcastTarget::Group("守卫".into()))
        );

        let blank = r#"{
          "knowledge_updates": [{"character_id": "城主", "knowledge_text": "私语", "source": "witnessed", "broadcast": "   "}],
          "variable_updates": [],
          "task_updates": []
        }"#;
        let r = parse_postprocess_from_response(&make_resp(blank, vec![]));
        assert_eq!(r.knowledge_updates[0].broadcast, None);
    }

    /// W-11：后处理输出里的**名字**必须归一为已存在的 `CharacterInstance.id`。
    #[tokio::test]
    async fn normalize_identities_resolves_unique_name_to_instance_id() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;

        let campaign = Campaign::new(Id::new(), "测试战役");
        let instance = storyforge_domain::campaign::CharacterInstance::temporary_with_overrides(
            campaign.id.clone(),
            "林医生",
            None,
            None,
        );
        let instance_id = instance.id.clone();
        let runtime_ctx = CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id: std::collections::HashMap::new(),
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        };
        let tool_ctx = Arc::new(crate::tools::ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: Some(Arc::new(runtime_ctx)),
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let runtime = AgentRuntime::new(
            Arc::new(storyforge_infra_llm::mock_client::MockLlmClient::new(
                vec![],
            )),
            tool_ctx,
        );

        let content = r#"{
          "knowledge_updates": [{"character_id": " 林医生 ", "knowledge_text": "看到尸体", "source": "witnessed"}],
          "variable_updates": [{"instance_id": "林医生", "key": "state", "value": "受伤"}],
          "task_updates": []
        }"#;
        let mut result = parse_postprocess_from_response(&make_resp(content, vec![]));
        assert_eq!(
            result.knowledge_updates[0].character_id.as_str(),
            " 林医生 "
        );

        normalize_postprocess_identities(&mut result, &runtime);

        assert_eq!(
            result.knowledge_updates[0].character_id, instance_id,
            "名字（含空白）应归一为实例 id"
        );
        assert_eq!(
            result.variable_updates[0].instance_id.as_ref(),
            Some(&instance_id),
            "变量更新的 instance_id 应归一为实例 id"
        );
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

    /// 记录发出的用户消息并返回合法 JSON，验证注入内容确实进入请求
    struct CaptureUserMsgClient {
        seen_user_msgs: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl storyforge_infra_llm::LlmClient for CaptureUserMsgClient {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            let user_msg = req
                .messages
                .iter()
                .filter(|m| matches!(m.role, storyforge_domain::llm::ChatRole::User))
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            self.seen_user_msgs.lock().unwrap().push(user_msg);
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
    async fn test_run_postprocess_injects_mvu_update_rules_into_request() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let llm = Arc::new(CaptureUserMsgClient {
            seen_user_msgs: seen.clone(),
        });
        let runtime = AgentRuntime::new(llm, empty_tool_context());
        let (_cancel_tx, cancel) = watch::channel(false);

        let result = run_postprocess(
            &runtime,
            "江离笑着递来一杯茶。",
            &["江离".into()],
            &["好感度".into()],
            3,
            "第3天",
            cancel,
            None,
            None,
            &["角色赠送礼物或表达关心时，该角色好感度 +5".to_string()],
        )
        .await
        .expect("postprocess should succeed");

        assert!(result.parse_succeeded);
        let msgs = seen.lock().unwrap();
        assert!(!msgs.is_empty());
        assert!(
            msgs[0].contains("【卡片变量更新规则】") && msgs[0].contains("好感度 +5"),
            "发给 LLM 的用户消息必须包含卡片规则区块，实际: {}",
            msgs[0].chars().take(200).collect::<String>()
        );
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
            &[],
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
        fallback_calls: Arc<AtomicUsize>,
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
            self.fallback_calls.fetch_add(1, Ordering::SeqCst);
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
    async fn test_run_postprocess_empty_json_is_success_without_second_call() {
        // W-09：合法但空的结果 = "本轮无更新"，是成功；不再触发第二次完整 LLM 调用
        // （旧行为会白付一次调用与延迟）。真·解析失败仍走 fallback，见
        // test_run_postprocess_retries_direct_json_when_tool_path_drifts。
        let calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(EmptyJsonThenUsefulJsonClient {
            calls: calls.clone(),
            fallback_calls: fallback_calls.clone(),
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
            &[],
        )
        .await
        .expect("空 JSON 结果应作为成功返回");

        assert!(result.parse_succeeded, "合法空 JSON 是成功解析");
        assert!(result.knowledge_updates.is_empty());
        assert!(result.variable_updates.is_empty());
        assert!(result.task_updates.is_empty());
        assert_eq!(
            fallback_calls.load(Ordering::SeqCst),
            0,
            "合法空结果不得再发 direct-JSON fallback（W-09）；calls={}",
            calls.load(Ordering::SeqCst)
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
            &[],
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
                &[],
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

//! Meta Agent 多轮对话框架（对应设计 §9.1）
//!
//! 形态：用户问"帮我看看世界书有没有冲突" → Agent 调 inspect 工具 → 产出诊断结论 +
//! 可选 Patch 提议（用户采纳才执行）。
//!
//! 工具 handler 挂接实际的 inspect_world_info / inspect_character / propose_patch，
//! 通过 [`MetaSession`] 共享会话状态（PatchStore + 数据源）。

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tracing::info;

use storyforge_app_agent::runtime::AgentRuntime;
use storyforge_app_agent::tools::ToolRegistry;
use storyforge_domain::character::Character;
use storyforge_domain::llm::ToolSpec;
use storyforge_domain::world_info::WorldInfoBook;

use storyforge_app_agent::AgentConfig;

use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;

use crate::prompts::meta_agent::{build_meta_user_msg, make_meta_agent_config};
use crate::typed_patch::{PreviewInput, TypedPatch, build_patch_from_action};
use crate::{
    CardReport, GenerationExplanation, Patch, PatchAction, PatchStore, WorldInfoReport,
    inspect_character, inspect_world_info,
};

/// 生成溯源数据源（由 tauri-app 层注入，避免 app-meta 依赖 tauri-app）
///
/// tauri-app 层实现此 trait，从 conv_store 查 Provenance 并调 `explain_generation`。
/// 返回 future，让实现方可以把同步 store I/O offload 到 blocking pool。
/// MetaSession 存 `Option<Arc<dyn GenerationExplainer>>`，None 表示未配置。
pub type GenerationExplainFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Option<GenerationExplanation>> + Send>>;

pub trait GenerationExplainer: Send + Sync {
    fn explain(&self, conversation_id: String, node_id: String) -> GenerationExplainFuture;
}

/// Meta Agent 会话状态（跨工具调用共享）
///
/// 工具 handler 通过 Arc<MetaSession> 访问世界书/角色卡 + PatchStore。
/// 用 Mutex 保护内部可变状态（pending patches 列表）。
pub struct MetaSession {
    /// 当前角色卡（可选，诊断 inspect_character 用）
    pub character: Mutex<Option<Arc<Character>>>,
    /// 当前世界书（可选，诊断 inspect_world_info 用）
    pub world_info: Mutex<Option<Arc<WorldInfoBook>>>,
    /// Patch 存储（提议的 Patch 进这里，用户采纳才执行）
    pub patches: PatchStore,
    /// 生成溯源数据源（由 tauri-app 层注入，None = 未配置）
    pub explainer: Option<Arc<dyn GenerationExplainer>>,
    /// active Campaign 运行时快照（inspect_* 工具读它，None = 无 active campaign）
    pub campaign_runtime: Mutex<Option<Arc<CampaignRuntimeContext>>>,
    /// Agent 通过 propose_campaign_patch 工具提议的 typed patch（chat 返回后 drain 到 AppState）
    pub typed_patches: Mutex<Vec<TypedPatch>>,
}

impl MetaSession {
    pub fn new() -> Self {
        Self {
            character: Mutex::new(None),
            world_info: Mutex::new(None),
            patches: PatchStore::new(),
            explainer: None,
            campaign_runtime: Mutex::new(None),
            typed_patches: Mutex::new(Vec::new()),
        }
    }

    pub fn set_character(&self, character: Arc<Character>) {
        *self.character.lock().unwrap_or_else(|p| p.into_inner()) = Some(character);
    }

    pub fn set_world_info(&self, book: Arc<WorldInfoBook>) {
        *self.world_info.lock().unwrap_or_else(|p| p.into_inner()) = Some(book);
    }

    pub fn set_explainer(&mut self, explainer: Arc<dyn GenerationExplainer>) {
        self.explainer = Some(explainer);
    }

    pub fn set_campaign_runtime(&self, ctx: Arc<CampaignRuntimeContext>) {
        *self
            .campaign_runtime
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(ctx);
    }
}

impl Default for MetaSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Meta 对话消息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum MetaMessage {
    /// 用户消息
    User { content: String },
    /// Agent 回复
    Agent {
        content: String,
        /// 工具调用结果（诊断报告 / patch 提议摘要），前端可结构化渲染
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_result: Option<ToolResultDisplay>,
    },
}

/// 工具结果的前端展示（结构化）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResultDisplay {
    /// 世界书诊断报告
    WorldInfoReport(WorldInfoReport),
    /// 角色卡诊断报告
    CardReport(CardReport),
    /// Patch 提议（用户可采纳/忽略）
    PatchProposed {
        patch_id: String,
        description: String,
        action_count: usize,
    },
    /// 无工具调用，纯文本回复
    None,
}

/// Meta 对话（多轮历史 + session）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaConversation {
    /// 对话 ID（前端标识）
    pub id: String,
    /// 消息历史
    pub messages: Vec<MetaMessage>,
    /// 历史摘要（给下一轮 LLM 用，避免 history 过长）
    pub history_summary: Vec<String>,
}

impl MetaConversation {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: vec![],
            history_summary: vec![],
        }
    }
}

impl Default for MetaConversation {
    fn default() -> Self {
        Self::new()
    }
}

/// 单轮对话的产出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaTurn {
    /// Agent 的回复消息（已追加到 conversation.messages）
    pub agent_message: MetaMessage,
    /// 本轮新增的 pending patch（如果有）
    pub new_patch: Option<Patch>,
    /// 本轮 Agent 通过 propose_campaign_patch 提议的 typed patch
    pub new_typed_patches: Vec<TypedPatch>,
}

/// 跑一轮 Meta 对话（用户输入 → Agent 回复）
///
/// 内部：
/// 1. 用 history_summary + 用户输入拼成 user_msg
/// 2. 注册诊断工具（handler 挂接 MetaSession）
/// 3. 跑 run_tool_loop_streaming（流式，token 经 progress_tx 推给上层）
/// 4. 把回复 + 工具结果结构化进 MetaMessage
/// 5. 更新 history_summary
///
/// `progress_tx`：流式 token 增量推送通道。Meta Agent 的工具执行（propose_patch 等）
/// 副作用在 run_tool_loop_streaming 内完成，与非流式一致。
pub async fn chat(
    runtime: &AgentRuntime,
    conversation: &mut MetaConversation,
    session: Arc<MetaSession>,
    user_input: &str,
    cancel: watch::Receiver<bool>,
    progress_tx: mpsc::UnboundedSender<String>,
) -> Result<MetaTurn, crate::MetaError> {
    // 记录用户消息
    conversation.messages.push(MetaMessage::User {
        content: user_input.to_string(),
    });

    let config: AgentConfig = make_meta_agent_config();
    let user_msg = build_meta_user_msg(&conversation.history_summary, user_input);

    let mut registry = ToolRegistry::new();
    register_meta_runtime_tools(&mut registry, session.clone());

    info!(target: "meta-conversation", "Meta 对话：用户输入 {} 字", user_input.len());

    // 记录进入前 typed_patches 长度，以便 drain 本轮新增
    let typed_patches_before = session
        .typed_patches
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .len();

    let resp = runtime
        .run_tool_loop_streaming(&config, user_msg, &registry, cancel, progress_tx, None)
        .await
        .map_err(|e| crate::MetaError::ExecutionFailed(format!("Meta Agent 运行失败: {e}")))?;

    // 解析本轮工具调用结果用于展示。
    //
    // 注意：meta_propose_patch 的实际 propose 副作用已在工具循环内由 handler 完成
    // （register_meta_runtime_tools 注册的 meta_propose_patch handler 调 session.patches.propose）。
    // 这里只读已提议的 patch 用于展示，**不再重复 propose**，否则会产生 2 个相同 patch。
    let mut new_patch: Option<Patch> = None;
    let mut tool_result = ToolResultDisplay::None;

    for tc in &resp.tool_calls {
        match tc.function.name.as_str() {
            "meta_inspect_world_info" => {
                if let Some(book) = session
                    .world_info
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()
                {
                    let report = inspect_world_info(&book);
                    tool_result = ToolResultDisplay::WorldInfoReport(report);
                }
            }
            "meta_inspect_character" => {
                if let Some(card) = session
                    .character
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()
                {
                    let report = inspect_character(&card);
                    tool_result = ToolResultDisplay::CardReport(report);
                }
            }
            "meta_propose_patch" => {
                // handler 已 propose，取最近一条未采纳 patch 用于展示
                if let Some(patch) = session.patches.pending().into_iter().last() {
                    tool_result = ToolResultDisplay::PatchProposed {
                        patch_id: patch.id.clone(),
                        description: patch.description.clone(),
                        action_count: patch.actions.len(),
                    };
                    new_patch = Some(patch);
                }
            }
            _ => {}
        }
    }

    let agent_message = MetaMessage::Agent {
        content: resp.content.clone(),
        tool_result: if matches!(tool_result, ToolResultDisplay::None) {
            None
        } else {
            Some(tool_result)
        },
    };

    // 更新 history_summary（截断到最近 6 轮，避免过长）
    conversation.history_summary.push(format!(
        "用户：{}\n助手：{}",
        user_input.chars().take(200).collect::<String>(),
        resp.content.chars().take(300).collect::<String>()
    ));
    if conversation.history_summary.len() > 6 {
        let drop_n = conversation.history_summary.len() - 6;
        conversation.history_summary.drain(0..drop_n);
    }

    conversation.messages.push(agent_message.clone());

    // drain 本轮新增的 typed patches
    let new_typed_patches = {
        let mut typed = session
            .typed_patches
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        typed.drain(typed_patches_before..).collect::<Vec<_>>()
    };

    Ok(MetaTurn {
        agent_message,
        new_patch,
        new_typed_patches,
    })
}

/// 注册 Meta 诊断工具（handler 挂接 MetaSession，实际执行 inspect/propose）
///
/// 注意：handler 是静态闭包，但通过 Arc<MetaSession> 捕获共享状态。
fn register_meta_runtime_tools(registry: &mut ToolRegistry, session: Arc<MetaSession>) {
    // meta_inspect_world_info
    registry.register(
        ToolSpec::function(
            "meta_inspect_world_info",
            "诊断世界书：检查蓝灯关键词冲突、孤立条目。返回诊断报告摘要。",
            serde_json::json!({"type": "object", "properties": {}}),
        ),
        {
            let session = session.clone();
            move |_args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let book = session.world_info.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    match book {
                        Some(b) => {
                            let report = inspect_world_info(&b);
                            let summary = format!(
                                "世界书诊断：共 {} 条，{} 个蓝灯，{} 个绿灯，发现 {} 处冲突，{} 个孤立条目",
                                report.total_entries,
                                report.constant_count,
                                report.selective_count,
                                report.conflicts.len(),
                                report.orphan_entries.len()
                            );
                            Ok(serde_json::json!({"summary": summary, "report": report}))
                        }
                        None => Ok(serde_json::json!({"error": "当前没有世界书，请先导入角色卡"})),
                    }
                })
            }
        },
    );

    // meta_inspect_character
    registry.register(
        ToolSpec::function(
            "meta_inspect_character",
            "诊断当前角色卡：检查描述/性格/开场白是否为空、开场白是否为占位符。",
            serde_json::json!({"type": "object", "properties": {}}),
        ),
        {
            let session = session.clone();
            move |_args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let card = session
                        .character
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    match card {
                        Some(c) => {
                            let report = inspect_character(&c);
                            let summary = if report.issues.is_empty() {
                                format!("角色卡「{}」诊断：未发现问题", report.name)
                            } else {
                                format!(
                                    "角色卡「{}」诊断：发现 {} 个问题：{}",
                                    report.name,
                                    report.issues.len(),
                                    report.issues.join("；")
                                )
                            };
                            Ok(serde_json::json!({"summary": summary, "report": report}))
                        }
                        None => Ok(serde_json::json!({"error": "当前没有角色卡，请先导入"})),
                    }
                })
            }
        },
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
        {
            let session = session.clone();
            move |args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let description = args
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("无描述")
                        .to_string();
                    let actions_val = args
                        .get("actions")
                        .cloned()
                        .unwrap_or(serde_json::Value::Array(vec![]));
                    let actions: Vec<PatchAction> =
                        serde_json::from_value(actions_val).map_err(|e| {
                            storyforge_app_agent::tools::ToolError::BadArgs(format!(
                                "actions 解析失败: {e}"
                            ))
                        })?;
                    let patch = session.patches.propose(description, actions);
                    Ok(serde_json::json!({
                        "patch_id": patch.id,
                        "description": patch.description,
                        "action_count": patch.actions.len(),
                        "status": "已提议，等待用户采纳"
                    }))
                })
            }
        },
    );

    // inspect_generation
    registry.register(
        ToolSpec::function(
            "inspect_generation",
            "解释某条消息的生成溯源。返回该轮的场景简述、各子 Agent 的角色/任务/输出摘要、最后的编剧提示、所用 Agent Profile 和随机种子。当用户问「为什么这么写」「这轮是怎么生成的」时调用。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "conversation_id": {"type": "string", "description": "对话 ID"},
                    "node_id": {"type": "string", "description": "消息节点 ID"}
                },
                "required": ["conversation_id", "node_id"]
            }),
        ),
        {
            let session = session.clone();
            move |args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let conversation_id = args
                        .get("conversation_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let node_id = args
                        .get("node_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match &session.explainer {
                        Some(explainer) => {
                            match explainer
                                .explain(conversation_id.to_string(), node_id.to_string())
                                .await
                            {
                                Some(explanation) => Ok(serde_json::json!({
                                    "explanation": explanation
                                })),
                                None => Ok(serde_json::json!({
                                    "error": "找不到该消息的生成溯源（可能无 provenance）"
                                })),
                            }
                        }
                        None => Ok(serde_json::json!({
                            "error": "未配置溯源数据源（GenerationExplainer 未注入）"
                        })),
                    }
                })
            }
        },
    );

    // ─── Campaign-aware 工具 ────────────────────────────────────────────────

    // inspect_campaign
    registry.register(
        ToolSpec::function(
            "inspect_campaign",
            "查看当前 active Campaign 概览：实例数/知识数/任务数/变量。",
            serde_json::json!({"type": "object", "properties": {}}),
        ),
        {
            let session = session.clone();
            move |_args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let rt = session
                        .campaign_runtime
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    match rt.as_ref() {
                        Some(ctx) => Ok(serde_json::json!({
                            "campaign_id": ctx.campaign.id.to_string(),
                            "name": ctx.campaign.name,
                            "turn": ctx.turn,
                            "instance_count": ctx.instances.len(),
                            "knowledge_count": ctx.knowledge.len(),
                            "campaign_variables": ctx.campaign.variables,
                        })),
                        None => Ok(serde_json::json!({"error": "当前没有 active Campaign"})),
                    }
                })
            }
        },
    );

    // inspect_instance
    registry.register(
        ToolSpec::function(
            "inspect_instance",
            "查看某角色实例详情：persona/behavior/变量。传 instance_id 或 name。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "instance_id_or_name": {"type": "string", "description": "实例 ID 或名称"}
                },
                "required": ["instance_id_or_name"]
            }),
        ),
        {
            let session = session.clone();
            move |args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let id_or_name = args
                        .get("instance_id_or_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let rt = session.campaign_runtime.lock().unwrap_or_else(|p| p.into_inner());
                    match rt.as_ref() {
                        Some(ctx) => {
                            match ctx.find_instance_by_id_or_name(id_or_name) {
                                Some(inst) => {
                                    let _def = ctx.definition_for_instance(inst);
                                    let persona = ctx.resolved_persona_for(inst);
                                    let behavior = ctx.resolved_behavior_for(inst);
                                    Ok(serde_json::json!({
                                        "id": inst.id.to_string(),
                                        "name": inst.name,
                                        "definition_id": inst.definition_id.as_ref().map(|d| d.to_string()),
                                        "is_temporary": inst.is_temporary,
                                        "resolved_persona": persona,
                                        "resolved_behavior": behavior,
                                        "variables": inst.variables,
                                    }))
                                }
                                None => Ok(serde_json::json!({"error": format!("找不到实例: {}", id_or_name)})),
                            }
                        }
                        None => Ok(serde_json::json!({"error": "当前没有 active Campaign"})),
                    }
                })
            }
        },
    );

    // inspect_variables
    registry.register(
        ToolSpec::function(
            "inspect_variables",
            "查看变量：scope=campaign 返回 Campaign 级变量，scope=instance 返回某实例变量。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["campaign", "instance"]},
                    "instance_id_or_name": {"type": "string", "description": "scope=instance 时必填"}
                },
                "required": ["scope"]
            }),
        ),
        {
            let session = session.clone();
            move |args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("campaign");
                    let rt = session.campaign_runtime.lock().unwrap_or_else(|p| p.into_inner());
                    match rt.as_ref() {
                        Some(ctx) => {
                            match scope {
                                "campaign" => {
                                    Ok(serde_json::json!({
                                        "scope": "campaign",
                                        "variables": ctx.campaign.variables,
                                    }))
                                }
                                "instance" => {
                                    let id_or_name = args
                                        .get("instance_id_or_name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    match ctx.find_instance_by_id_or_name(id_or_name) {
                                        Some(inst) => {
                                            Ok(serde_json::json!({
                                                "scope": "instance",
                                                "instance_id": inst.id.to_string(),
                                                "instance_name": inst.name,
                                                "variables": inst.variables,
                                            }))
                                        }
                                        None => Ok(serde_json::json!({"error": format!("找不到实例: {}", id_or_name)})),
                                    }
                                }
                                _ => Ok(serde_json::json!({"error": format!("未知 scope: {}（应为 campaign 或 instance)", scope)})),
                            }
                        }
                        None => Ok(serde_json::json!({"error": "当前没有 active Campaign"})),
                    }
                })
            }
        },
    );

    // inspect_knowledge
    registry.register(
        ToolSpec::function(
            "inspect_knowledge",
            "查看角色可见知识。无参返回全部，有 instance_id_or_name 则过滤。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "instance_id_or_name": {"type": "string", "description": "可选：按实例过滤"}
                }
            }),
        ),
        {
            let session = session.clone();
            move |args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let rt = session.campaign_runtime.lock().unwrap_or_else(|p| p.into_inner());
                    match rt.as_ref() {
                        Some(ctx) => {
                            let id_or_name = args
                                .get("instance_id_or_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let entries: Vec<&CharacterKnowledgeEntry> = if id_or_name.is_empty() {
                                ctx.knowledge.iter().collect()
                            } else {
                                match ctx.find_instance_by_id_or_name(id_or_name) {
                                    Some(inst) => ctx.knowledge_for_instance(inst),
                                    None => return Ok(serde_json::json!({"error": format!("找不到实例: {}", id_or_name)})),
                                }
                            };
                            let summary: Vec<serde_json::Value> = entries
                                .iter()
                                .map(|k| {
                                    serde_json::json!({
                                        "id": k.id.to_string(),
                                        "character_id": k.character_id.to_string(),
                                        "knowledge_text": truncate_str(&k.knowledge_text, 80),
                                        "source": k.source,
                                    })
                                })
                                .collect();
                            Ok(serde_json::json!({
                                "count": summary.len(),
                                "knowledge": summary,
                            }))
                        }
                        None => Ok(serde_json::json!({"error": "当前没有 active Campaign"})),
                    }
                })
            }
        },
    );

    // inspect_tasks
    registry.register(
        ToolSpec::function(
            "inspect_tasks",
            "查看任务列表。status=pending（默认）返回待办，status=all 返回全部。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ["pending", "all"], "default": "pending"}
                }
            }),
        ),
        {
            let session = session.clone();
            move |args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let status_filter = args
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending");
                    let rt = session.campaign_runtime.lock().unwrap_or_else(|p| p.into_inner());
                    match rt.as_ref() {
                        Some(ctx) => {
                            // 过滤：pending 返回 Pending/Active（可注入的），all 返回全部
                            let tasks: Vec<&storyforge_domain::story_task::StoryTask> = match status_filter {
                                "all" => ctx.tasks.iter().collect(),
                                _ => ctx.tasks.iter().filter(|t| t.status.is_injectable()).collect(),
                            };
                            let summary: Vec<serde_json::Value> = tasks
                                .iter()
                                .map(|t| {
                                    serde_json::json!({
                                        "id": t.id.to_string(),
                                        "title": t.title,
                                        "description": t.description,
                                        "status": t.status,
                                        "created_turn": t.created_turn,
                                        "related_characters": t.related_characters.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                                    })
                                })
                                .collect();
                            Ok(serde_json::json!({
                                "count": summary.len(),
                                "status_filter": status_filter,
                                "tasks": summary,
                            }))
                        }
                        None => Ok(serde_json::json!({"error": "当前没有 active Campaign"})),
                    }
                })
            }
        },
    );

    // propose_campaign_patch
    registry.register(
        ToolSpec::function(
            "propose_campaign_patch",
            "提议一个类型化 Campaign 修复（变量/知识/任务状态）。用户预览后才写盘。action 必须包含正确 target id（先 inspect 拿到真实 id）。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {"type": "string", "description": "修复描述"},
                    "action": {
                        "type": "object",
                        "description": "TypedPatchAction JSON（kind + 参数）",
                        "properties": {
                            "kind": {"type": "string"}
                        },
                        "required": ["kind"]
                    }
                },
                "required": ["description", "action"]
            }),
        ),
        {
            let session = session.clone();
            move |args, _ctx| {
                let session = session.clone();
                Box::pin(async move {
                    let description = args
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("无描述")
                        .to_string();
                    let action_val = args
                        .get("action")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);

                    let action: crate::typed_patch::TypedPatchAction =
                        serde_json::from_value(action_val).map_err(|e| {
                            storyforge_app_agent::tools::ToolError::BadArgs(format!(
                                "action 解析失败: {e}"
                            ))
                        })?;

                    // 从 campaign_runtime 快照构建 PreviewInput
                    let rt = session.campaign_runtime.lock().unwrap_or_else(|p| p.into_inner());
                    let ctx = match rt.as_ref() {
                        Some(ctx) => ctx,
                        None => return Ok(serde_json::json!({"error": "当前没有 active Campaign"})),
                    };

                    let definitions: Vec<_> = ctx.definitions_by_id.values().cloned().collect();
                    let input = PreviewInput {
                        instances: &ctx.instances,
                        definitions: &definitions,
                        knowledge: &ctx.knowledge,
                        tasks: &ctx.tasks,
                        campaign: Some(&ctx.campaign),
                    };

                    match build_patch_from_action(description, action, &input) {
                        Ok(patch) => {
                            let patch_id = patch.id.clone();
                            let desc = patch.description.clone();
                            session.typed_patches.lock().unwrap_or_else(|p| p.into_inner()).push(patch);
                            Ok(serde_json::json!({
                                "patch_id": patch_id,
                                "description": desc,
                                "status": "已提议，等待用户预览/接受"
                            }))
                        }
                        Err(e) => Ok(serde_json::json!({"error": format!("{e}")})),
                    }
                })
            }
        },
    );
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", chars[..max_chars].iter().collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explain::{GenerationExplanation, SubagentExplain};
    use storyforge_domain::world_info::{LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry};

    /// 测试用 mock GenerationExplainer
    struct MockExplainer {
        explanation: Option<GenerationExplanation>,
    }

    impl MockExplainer {
        fn with_explanation(explanation: GenerationExplanation) -> Self {
            Self {
                explanation: Some(explanation),
            }
        }

        fn returning_none() -> Self {
            Self { explanation: None }
        }
    }

    impl GenerationExplainer for MockExplainer {
        fn explain(&self, _conversation_id: String, _node_id: String) -> GenerationExplainFuture {
            let explanation = self.explanation.clone();
            Box::pin(async move { explanation })
        }
    }

    fn make_test_explanation() -> GenerationExplanation {
        GenerationExplanation {
            scene_brief: Some("雨夜告别场景".into()),
            subagents: vec![SubagentExplain {
                character_id: "alice".into(),
                display_name: "Alice".into(),
                task_brief: Some("扮演 Alice，表达离别的不舍".into()),
                output_preview: "Alice 望着窗外的雨。".into(),
                fallback_reason: None,
            }],
            last_hint: None,
            profile_id: Some("default".into()),
            seed: 42,
        }
    }

    fn make_world_info() -> Arc<WorldInfoBook> {
        Arc::new(WorldInfoBook {
            entries: vec![
                WorldInfoEntry {
                    st_id: Some(1),
                    keys: vec!["龙".into(), "冲突".into()],
                    secondary_keys: vec![],
                    content: "内容1".into(),
                    constant: true,
                    selective: false,
                    selective_logic: SelectiveLogic::And,
                    disabled: false,
                    position: 0,
                    depth: 2,
                    order: 100,
                    route: LoreRoute::Constant,
                    extensions: serde_json::json!({}),
                },
                WorldInfoEntry {
                    st_id: Some(2),
                    keys: vec!["龙".into()],
                    secondary_keys: vec![],
                    content: "内容2".into(),
                    constant: true,
                    selective: false,
                    selective_logic: SelectiveLogic::And,
                    disabled: false,
                    position: 0,
                    depth: 2,
                    order: 100,
                    route: LoreRoute::Constant,
                    extensions: serde_json::json!({}),
                },
            ],
            source: storyforge_domain::Source::Native,
        })
    }

    #[test]
    fn test_meta_session_set_and_get() {
        let session = Arc::new(MetaSession::new());
        let book = make_world_info();
        session.set_world_info(book);
        assert!(
            session
                .world_info
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
        );
    }

    #[test]
    fn test_meta_conversation_new_is_empty() {
        let conv = MetaConversation::new();
        assert!(conv.messages.is_empty());
        assert!(conv.history_summary.is_empty());
        assert!(!conv.id.is_empty());
    }

    #[tokio::test]
    async fn test_chat_single_turn_diagnostic_with_mock() {
        use storyforge_app_agent::runtime::AgentRuntime;
        use storyforge_app_agent::tools::ToolContext;
        use storyforge_infra_llm::mock_client::{MockLlmClient, MockScript};

        // Mock：匹配 "配置调试助手"，返回纯文本（不带工具调用，避免工具循环死循环）
        let mock = Arc::new(MockLlmClient::new(vec![MockScript {
            match_keyword: "配置调试助手".into(),
            response_content: "我先看看当前配置。世界书共有 2 条蓝灯条目，其中 1 处关键词冲突。"
                .into(),
            tool_calls: vec![],
            stream: false,
        }]));

        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let runtime = AgentRuntime::new(mock, tool_ctx);

        let session = Arc::new(MetaSession::new());
        session.set_world_info(make_world_info());

        let mut conv = MetaConversation::new();
        let (_tx, cancel) = watch::channel(false);

        let turn = chat(
            &runtime,
            &mut conv,
            session,
            "看看世界书",
            cancel,
            mpsc::unbounded_channel::<String>().0, // 测试不消费流式 token
        )
        .await
        .unwrap();

        // 纯文本回复，无工具结果
        match &turn.agent_message {
            MetaMessage::Agent {
                content,
                tool_result,
            } => {
                assert!(content.contains("世界书"));
                assert!(tool_result.is_none()); // 无工具调用
            }
            _ => panic!("应该是 Agent 消息"),
        }

        // 对话历史应该有 2 条（用户 + Agent）
        assert_eq!(conv.messages.len(), 2);
        // history_summary 更新了
        assert_eq!(conv.history_summary.len(), 1);
    }

    #[test]
    fn test_chat_patch_proposal_recorded_in_session() {
        // 直接测 PatchStore propose（chat 的工具循环逻辑单测见上面）
        let session = Arc::new(MetaSession::new());
        let patch = session.patches.propose(
            "合并重复关键词".into(),
            vec![crate::PatchAction::Update {
                target: "world_info[1]".into(),
                field: "keys".into(),
                value: serde_json::json!(["龙"]),
            }],
        );
        assert!(!patch.applied);
        assert_eq!(session.patches.pending().len(), 1);
        assert_eq!(patch.actions.len(), 1);
    }

    // ─── GenerationExplainer / inspect_generation 测试 ────────────────────

    #[test]
    fn test_inspect_generation_tool_registered() {
        let session = Arc::new(MetaSession::new());
        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session);

        let specs = registry.tool_specs();
        let has_inspect = specs
            .iter()
            .any(|s| s.function.name == "inspect_generation");
        assert!(has_inspect, "inspect_generation 工具应已注册");
    }

    #[tokio::test]
    async fn test_inspect_generation_explainer_returns_explanation() {
        let explanation = make_test_explanation();
        let mock_explainer = Arc::new(MockExplainer::with_explanation(explanation));

        let mut session = MetaSession::new();
        session.set_explainer(mock_explainer);
        let session = Arc::new(session);

        // 直接调用 handler 逻辑（通过注册 + 调用）
        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let args = serde_json::json!({
            "conversation_id": "conv-1",
            "node_id": "node-1"
        });
        let tool_ctx = Arc::new(storyforge_app_agent::tools::ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let result = registry
            .dispatch("inspect_generation", args, tool_ctx)
            .await
            .unwrap();
        assert!(
            result.get("explanation").is_some(),
            "应返回 explanation 字段"
        );
        assert_eq!(
            result["explanation"]["scene_brief"].as_str(),
            Some("雨夜告别场景")
        );
    }

    #[tokio::test]
    async fn test_inspect_generation_explainer_returns_none() {
        let mock_explainer = Arc::new(MockExplainer::returning_none());

        let mut session = MetaSession::new();
        session.set_explainer(mock_explainer);
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let args = serde_json::json!({
            "conversation_id": "conv-1",
            "node_id": "node-missing"
        });
        let tool_ctx = Arc::new(storyforge_app_agent::tools::ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let result = registry
            .dispatch("inspect_generation", args, tool_ctx)
            .await
            .unwrap();
        let error = result["error"].as_str().unwrap();
        assert!(
            error.contains("找不到"),
            "应返回'找不到'错误，实际: {error}"
        );
    }

    #[tokio::test]
    async fn test_inspect_generation_no_explainer_configured() {
        let session = Arc::new(MetaSession::new()); // explainer = None

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let args = serde_json::json!({
            "conversation_id": "conv-1",
            "node_id": "node-1"
        });
        let tool_ctx = Arc::new(storyforge_app_agent::tools::ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let result = registry
            .dispatch("inspect_generation", args, tool_ctx)
            .await
            .unwrap();
        let error = result["error"].as_str().unwrap();
        assert!(
            error.contains("未配置"),
            "应返回'未配置'错误，实际: {error}"
        );
    }

    #[test]
    fn test_meta_session_default_explainer_is_none() {
        let session = MetaSession::new();
        assert!(
            session.explainer.is_none(),
            "默认 MetaSession 的 explainer 应为 None"
        );
    }

    // ─── Campaign 工具注册测试 ─────────────────────────────────────────────

    #[test]
    fn test_campaign_tools_registered() {
        let session = Arc::new(MetaSession::new());
        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session);

        let specs = registry.tool_specs();
        let names: Vec<&str> = specs.iter().map(|s| s.function.name.as_str()).collect();
        assert!(
            names.contains(&"inspect_campaign"),
            "inspect_campaign 应已注册"
        );
        assert!(
            names.contains(&"inspect_instance"),
            "inspect_instance 应已注册"
        );
        assert!(
            names.contains(&"inspect_variables"),
            "inspect_variables 应已注册"
        );
        assert!(
            names.contains(&"inspect_knowledge"),
            "inspect_knowledge 应已注册"
        );
        assert!(names.contains(&"inspect_tasks"), "inspect_tasks 应已注册");
        assert!(
            names.contains(&"propose_campaign_patch"),
            "propose_campaign_patch 应已注册"
        );
    }

    // ─── Campaign handler 测试 helpers ─────────────────────────────────────

    use std::collections::HashMap;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
    use storyforge_domain::variables::{self};

    fn make_test_campaign_runtime() -> Arc<CampaignRuntimeContext> {
        use storyforge_domain::Id;

        let campaign = Campaign::new(Id::from_str("card-1"), "测试 Campaign");
        let def = CharacterDefinition {
            id: Id::from_str("def-1"),
            card_id: Id::from_str("card-1"),
            name: "Alice".into(),
            persona_prompt: "勇敢的冒险者".into(),
            behavior_rules: "不要放弃".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: variables::default_character_variables(),
        };
        let inst = CharacterInstance::from_definition(Id::from_str("camp-1"), &def);
        let knowledge = vec![CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            inst.id.clone(),
            "看到了龙",
            1,
        )];
        let tasks = vec![storyforge_domain::story_task::StoryTask::user_planned(
            Id::from_str("camp-1"),
            "复仇",
            "老王复仇",
            vec![storyforge_domain::story_task::TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        )];
        let mut defs = HashMap::new();
        defs.insert(def.id.clone(), def);

        Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![inst],
            definitions_by_id: defs,
            knowledge,
            tasks,
            turn: 3,
        })
    }

    fn make_tool_ctx() -> Arc<storyforge_app_agent::tools::ToolContext> {
        Arc::new(storyforge_app_agent::tools::ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        })
    }

    // ─── inspect_campaign 测试 ────────────────────────────────────────────

    #[tokio::test]
    async fn test_inspect_campaign_handler_returns_overview() {
        let session = MetaSession::new();
        session.set_campaign_runtime(make_test_campaign_runtime());
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let result = registry
            .dispatch("inspect_campaign", serde_json::json!({}), make_tool_ctx())
            .await
            .unwrap();
        assert!(result.get("campaign_id").is_some(), "应返回 campaign_id");
        assert_eq!(result["name"].as_str(), Some("测试 Campaign"));
        assert_eq!(result["turn"].as_u64(), Some(3));
        assert_eq!(result["instance_count"].as_u64(), Some(1));
        assert_eq!(result["knowledge_count"].as_u64(), Some(1));
        assert!(result.get("campaign_variables").is_some());
    }

    #[tokio::test]
    async fn test_inspect_campaign_handler_no_campaign() {
        let session = Arc::new(MetaSession::new());

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let result = registry
            .dispatch("inspect_campaign", serde_json::json!({}), make_tool_ctx())
            .await
            .unwrap();
        assert!(result.get("error").is_some(), "无 campaign 时应返回 error");
    }

    // ─── inspect_instance 测试 ────────────────────────────────────────────

    #[tokio::test]
    async fn test_inspect_instance_by_id() {
        let rt = make_test_campaign_runtime();
        let inst_id = rt.instances[0].id.to_string();
        let session = MetaSession::new();
        session.set_campaign_runtime(rt);
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let result = registry
            .dispatch(
                "inspect_instance",
                serde_json::json!({"instance_id_or_name": inst_id}),
                make_tool_ctx(),
            )
            .await
            .unwrap();
        assert_eq!(result["name"].as_str(), Some("Alice"));
        assert_eq!(result["is_temporary"].as_bool(), Some(false));
    }

    #[tokio::test]
    async fn test_inspect_instance_by_name() {
        let session = MetaSession::new();
        session.set_campaign_runtime(make_test_campaign_runtime());
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let result = registry
            .dispatch(
                "inspect_instance",
                serde_json::json!({"instance_id_or_name": "Alice"}),
                make_tool_ctx(),
            )
            .await
            .unwrap();
        assert_eq!(result["name"].as_str(), Some("Alice"));
    }

    #[tokio::test]
    async fn test_inspect_instance_not_found() {
        let session = MetaSession::new();
        session.set_campaign_runtime(make_test_campaign_runtime());
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let result = registry
            .dispatch(
                "inspect_instance",
                serde_json::json!({"instance_id_or_name": "不存在"}),
                make_tool_ctx(),
            )
            .await
            .unwrap();
        assert!(result.get("error").is_some(), "找不到实例应返回 error");
    }

    // ─── propose_campaign_patch 测试 ──────────────────────────────────────

    #[tokio::test]
    async fn test_propose_campaign_patch_valid_action() {
        let rt = make_test_campaign_runtime();
        let inst_id = rt.instances[0].id.to_string();
        let session = MetaSession::new();
        session.set_campaign_runtime(rt);
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let args = serde_json::json!({
            "description": "修改 HP",
            "action": {
                "kind": "update_instance_variable",
                "instance_id": inst_id,
                "key": "hp",
                "value": 80
            }
        });
        let result = registry
            .dispatch("propose_campaign_patch", args, make_tool_ctx())
            .await
            .unwrap();
        assert!(result.get("patch_id").is_some(), "应返回 patch_id");
        assert_eq!(result["status"].as_str(), Some("已提议，等待用户预览/接受"));

        // session.typed_patches 应增长 1
        let typed = session.typed_patches.lock().unwrap();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].source_issue_category, "agent_proposed");
    }

    #[tokio::test]
    async fn test_propose_campaign_patch_target_missing() {
        let session = MetaSession::new();
        session.set_campaign_runtime(make_test_campaign_runtime());
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let args = serde_json::json!({
            "description": "修改不存在的实例",
            "action": {
                "kind": "update_instance_variable",
                "instance_id": "nonexistent",
                "key": "hp",
                "value": 80
            }
        });
        let result = registry
            .dispatch("propose_campaign_patch", args, make_tool_ctx())
            .await
            .unwrap();
        assert!(result.get("error").is_some(), "target 缺失应返回 error");

        // session.typed_patches 不应增长
        let typed = session.typed_patches.lock().unwrap();
        assert_eq!(typed.len(), 0);
    }

    // ─── inspect_tasks 真实返回测试（修复数据源缺口后） ───────────────────

    #[tokio::test]
    async fn test_inspect_tasks_returns_real_tasks_pending() {
        let session = MetaSession::new();
        session.set_campaign_runtime(make_test_campaign_runtime());
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        // 默认 pending：helper 里的 task 是 Pending（user_planned 初始状态），可注入
        let result = registry
            .dispatch("inspect_tasks", serde_json::json!({}), make_tool_ctx())
            .await
            .unwrap();
        assert_eq!(result["count"].as_u64(), Some(1), "pending 应返回 1 条任务");
        assert_eq!(result["tasks"][0]["title"].as_str(), Some("复仇"));
        assert!(
            result.get("note").is_none(),
            "不应再返回 note 提示（已接真实数据源）"
        );
    }

    #[tokio::test]
    async fn test_inspect_tasks_all_filter() {
        let session = MetaSession::new();
        session.set_campaign_runtime(make_test_campaign_runtime());
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let result = registry
            .dispatch(
                "inspect_tasks",
                serde_json::json!({"status": "all"}),
                make_tool_ctx(),
            )
            .await
            .unwrap();
        assert_eq!(result["count"].as_u64(), Some(1), "all 也应返回 1 条");
    }

    // ─── propose_campaign_patch: UpdateTaskStatus 提议成功（修复数据源缺口后） ─

    #[tokio::test]
    async fn test_propose_campaign_patch_update_task_status() {
        let rt = make_test_campaign_runtime();
        // helper 里 task 是 user_planned 生成，id 是随机的，取出来用
        let task_id = rt.tasks[0].id.to_string();
        let session = MetaSession::new();
        session.set_campaign_runtime(rt);
        let session = Arc::new(session);

        let mut registry = storyforge_app_agent::tools::ToolRegistry::new();
        register_meta_runtime_tools(&mut registry, session.clone());

        let args = serde_json::json!({
            "description": "完成任务",
            "action": {
                "kind": "update_task_status",
                "task_id": task_id,
                "new_status": "completed"
            }
        });
        let result = registry
            .dispatch("propose_campaign_patch", args, make_tool_ctx())
            .await
            .unwrap();
        assert!(
            result.get("patch_id").is_some(),
            "UpdateTaskStatus 应提议成功，实际: {result}"
        );

        // session.typed_patches 应增长 1
        let typed = session.typed_patches.lock().unwrap();
        assert_eq!(typed.len(), 1);
    }

    // ─── MetaTurn.new_typed_patches drain 测试 ─────────────────────────────

    #[test]
    fn test_meta_turn_drains_typed_patches() {
        // 模拟：进入前有 0 条，工具 handler 添加 1 条，drain 后 new_typed_patches 非空
        let session = Arc::new(MetaSession::new());
        let typed_before = session.typed_patches.lock().unwrap().len();

        // 模拟工具 handler 添加 patch
        session.typed_patches.lock().unwrap().push(TypedPatch {
            id: "test-1".into(),
            description: "测试".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: None,
            actions: vec![],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: crate::typed_patch::TypedPatchStatus::Pending,
        });

        let new_typed = {
            let mut typed = session.typed_patches.lock().unwrap();
            typed.drain(typed_before..).collect::<Vec<_>>()
        };

        assert_eq!(new_typed.len(), 1);
        assert_eq!(new_typed[0].id, "test-1");
        // session.typed_patches 应被 drain 清空
        assert_eq!(session.typed_patches.lock().unwrap().len(), 0);
    }
}

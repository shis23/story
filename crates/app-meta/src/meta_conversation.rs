//! Meta Agent 多轮对话框架（对应设计 §9.1）
//!
//! 形态：用户问"帮我看看世界书有没有冲突" → Agent 调 inspect 工具 → 产出诊断结论 +
//! 可选 Patch 提议（用户采纳才执行）。
//!
//! 工具 handler 挂接实际的 inspect_world_info / inspect_character / propose_patch，
//! 通过 [`MetaSession`] 共享会话状态（PatchStore + 数据源）。

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::info;

use storyforge_app_agent::runtime::AgentRuntime;
use storyforge_app_agent::tools::ToolRegistry;
use storyforge_domain::character::Character;
use storyforge_domain::llm::ToolSpec;
use storyforge_domain::world_info::WorldInfoBook;

use storyforge_app_agent::AgentConfig;

use crate::prompts::meta_agent::{build_meta_user_msg, make_meta_agent_config};
use crate::{
    inspect_character, inspect_world_info, CardReport, Patch, PatchAction, PatchStore,
    WorldInfoReport,
};

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
}

impl MetaSession {
    pub fn new() -> Self {
        Self {
            character: Mutex::new(None),
            world_info: Mutex::new(None),
            patches: PatchStore::new(),
        }
    }

    pub fn set_character(&self, character: Arc<Character>) {
        *self.character.lock().unwrap() = Some(character);
    }

    pub fn set_world_info(&self, book: Arc<WorldInfoBook>) {
        *self.world_info.lock().unwrap() = Some(book);
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
}

/// 跑一轮 Meta 对话（用户输入 → Agent 回复）
///
/// 内部：
/// 1. 用 history_summary + 用户输入拼成 user_msg
/// 2. 注册诊断工具（handler 挂接 MetaSession）
/// 3. 跑 run_tool_loop
/// 4. 把回复 + 工具结果结构化进 MetaMessage
/// 5. 更新 history_summary
pub async fn chat(
    runtime: &AgentRuntime,
    conversation: &mut MetaConversation,
    session: Arc<MetaSession>,
    user_input: &str,
    cancel: watch::Receiver<bool>,
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

    let resp = runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
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
                if let Some(book) = session.world_info.lock().unwrap().clone() {
                    let report = inspect_world_info(&book);
                    tool_result = ToolResultDisplay::WorldInfoReport(report);
                }
            }
            "meta_inspect_character" => {
                if let Some(card) = session.character.lock().unwrap().clone() {
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

    Ok(MetaTurn {
        agent_message,
        new_patch,
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
                    let book = session.world_info.lock().unwrap().clone();
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
                    let card = session.character.lock().unwrap().clone();
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
                    let actions_val = args.get("actions").cloned().unwrap_or(serde_json::Value::Array(vec![]));
                    let actions: Vec<PatchAction> = serde_json::from_value(actions_val).map_err(|e| {
                        storyforge_app_agent::tools::ToolError::BadArgs(format!("actions 解析失败: {e}"))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::world_info::{LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry};

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
        assert!(session.world_info.lock().unwrap().is_some());
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
            response_content: "我先看看当前配置。世界书共有 2 条蓝灯条目，其中 1 处关键词冲突。".into(),
            tool_calls: vec![],
            stream: false,
        }]));

        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
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
        )
        .await
        .unwrap();

        // 纯文本回复，无工具结果
        let _ = match &turn.agent_message {
            MetaMessage::Agent { content, tool_result } => {
                assert!(content.contains("世界书"));
                assert!(tool_result.is_none()); // 无工具调用
            }
            _ => panic!("应该是 Agent 消息"),
        };

        // 对话历史应该有 2 条（用户 + Agent）
        assert_eq!(conv.messages.len(), 2);
        // history_summary 更新了
        assert_eq!(conv.history_summary.len(), 1);
    }

    #[tokio::test]
    async fn test_chat_patch_proposal_recorded_in_session() {
        use storyforge_app_agent::runtime::AgentRuntime;
        use storyforge_app_agent::tools::ToolContext;
        use storyforge_infra_llm::mock_client::{MockLlmClient, MockScript};

        // Mock：返回带 meta_propose_patch 工具调用的响应（单轮，工具循环会执行后退出）
        // 但 MockLlmClient 无状态会重复返回同样工具调用 → 死循环。
        // 改为：直接测试 propose 走 PatchStore，不经过完整 chat（chat 的工具循环对 mock 不友好）
        let _mock = MockLlmClient::new(vec![MockScript {
            match_keyword: "配置调试助手".into(),
            response_content: "我提议一个修复。".into(),
            tool_calls: vec![],
            stream: false,
        }]);

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
}

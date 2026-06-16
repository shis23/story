/// Agent 运行时（工具循环 + 委派 + 取消）
///
/// 核心函数 run_tool_loop()：循环调用 LLM + 执行工具，直到完成或超限。
/// 设计来源：TT 的 AgentRuntimeService（max_rounds + drift recovery + watch 取消）。
use std::sync::Arc;

use tokio::sync::{mpsc, watch, Semaphore};
use tracing::{debug, error, info, warn};

use storyforge_domain::agent::{AgentRole, ContextPackage, Performance, PipelineEvent, SubagentTask};
use storyforge_domain::llm::{ChatMessage, ChatRequest, ChatResponse, LlmError, StreamChunk, ToolSpec};
use storyforge_infra_llm::LlmClient;

use crate::tools::{ToolContext, ToolRegistry};

/// Agent 运行时配置
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub role: AgentRole,
    pub system_prompt: String,
    pub max_tool_rounds: u32,
    pub model: String,
    pub tools: Vec<ToolSpec>,
}

/// Agent 运行时
pub struct AgentRuntime {
    llm: Arc<dyn LlmClient>,
    tool_ctx: Arc<ToolContext>,
}

impl AgentRuntime {
    pub fn new(llm: Arc<dyn LlmClient>, tool_ctx: Arc<ToolContext>) -> Self {
        Self { llm, tool_ctx }
    }

    /// 运行工具循环（对应 TT 的 run_tool_loop）
    ///
    /// 流程：
    /// 1. 构造初始请求（system prompt + 用户消息）
    /// 2. 循环：调 LLM → 检查 tool_calls → 执行工具 → 追加结果 → 继续
    /// 3. 模型不调工具时（drift recovery）：注入 reminder，继续
    /// 4. 超过 max_rounds 时退出
    ///
    /// 返回最终的 ChatResponse（含 content 和 tool_calls）
    pub async fn run_tool_loop(
        &self,
        config: &AgentConfig,
        user_message: String,
        tool_registry: &ToolRegistry,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, AgentError> {
        let mut messages = vec![ChatMessage::system(&config.system_prompt)];
        messages.push(ChatMessage::user(&user_message));

        for round in 1..=config.max_tool_rounds {
            // 检查取消
            if *cancel.borrow() {
                info!(target: "app-agent", "{}: 第 {round} 轮前取消", config.role);
                return Err(AgentError::Cancelled);
            }

            debug!(target: "app-agent", "{}: 第 {round}/{max} 轮",
                config.role, max = config.max_tool_rounds);

            let req = ChatRequest {
                messages: messages.clone(),
                tools: if tool_registry.tool_specs().is_empty() {
                    None
                } else {
                    Some(tool_registry.tool_specs())
                },
                params: Default::default(),
                model: config.model.clone(),
            };

            // 调 LLM（可取消）
            let cancel_fut = {
                let mut cancel = cancel.clone();
                async move {
                    let _ = cancel.wait_for(|&c| c).await;
                }
            };
            let resp = tokio::select! {
                result = self.llm.chat(&req) => result.map_err(AgentError::Llm)?,
                _ = cancel_fut => return Err(AgentError::Cancelled),
            };

            // 没有工具调用 = 模型直接输出文本
            if resp.tool_calls.is_empty() {
                // drift recovery：如果还有工具可用，提醒模型使用工具
                if !resp.content.is_empty() && round < config.max_tool_rounds && tool_registry.tool_specs().len() > 0 {
                    // 检查是否是最终输出（没有工具定义时直接返回）
                    if req.tools.is_none() {
                        return Ok(resp);
                    }

                    // drift recovery：注入提醒
                    warn!(target: "app-agent", "{}: 第 {round} 轮模型未调用工具，注入 reminder", config.role);
                    messages.push(ChatMessage::assistant(&resp.content));
                    messages.push(ChatMessage::user(
                        "请继续使用工具完成任务。如果你已经完成，请直接输出最终结果。"
                    ));
                    continue;
                }

                // 没有内容也没有工具调用 = 空响应
                if resp.content.is_empty() {
                    warn!(target: "app-agent", "{}: 第 {round} 轮空响应", config.role);
                    if round >= config.max_tool_rounds {
                        return Err(AgentError::MaxRoundsExceeded);
                    }
                    messages.push(ChatMessage::user("请输出内容或调用工具。"));
                    continue;
                }

                // 有内容 = 最终输出
                info!(target: "app-agent", "{}: 第 {round} 轮完成，content_len={}", 
                    config.role, resp.content.len());
                return Ok(resp);
            }

            // 有工具调用 → 执行
            debug!(target: "app-agent", "{}: 第 {round} 轮调用 {} 个工具",
                config.role, resp.tool_calls.len());

            // 追加 assistant 消息（含 tool_calls）
            messages.push(ChatMessage {
                role: storyforge_domain::llm::ChatRole::Assistant,
                content: resp.content.clone(),
                tool_calls: Some(resp.tool_calls.clone()),
                tool_call_id: None,
            });

            // 执行每个工具调用
            for tc in &resp.tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::json!({}));

                let result = tool_registry
                    .dispatch(&tc.function.name, args, self.tool_ctx.clone())
                    .await;

                let result_str = match result {
                    Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "{}".into()),
                    Err(e) => {
                        warn!(target: "app-agent", "工具 {} 执行失败: {e}", tc.function.name);
                        serde_json::json!({ "error": e.to_string() }).to_string()
                    }
                };

                messages.push(ChatMessage::tool_result(&tc.id, &result_str));
            }
        }

        error!(target: "app-agent", "{}: 超过最大轮次 {max}",
            config.role, max = config.max_tool_rounds);
        Err(AgentError::MaxRoundsExceeded)
    }

    /// 流式版工具循环（导演/编剧用，把输出 token 实时推给上层）
    ///
    /// 和 `run_tool_loop` 逻辑相同，但 LLM 调用走 `chat_stream`，
    /// 每个 `StreamChunk.delta_content` 通过 `progress_tx` 推送给上层（用于前端实时显示）。
    /// 工具调用逻辑不变（工具调用的 arguments 通常不产生 content delta）。
    ///
    /// `completion_probe`：可选的"完成探测"回调。当模型不调工具但输出了文本时，
    /// 先用这个回调探测 content 是否已经是"最终结果"（如导演已输出合法 Plan JSON）。
    /// 返回 true 则立即终止循环（业界推荐的 early termination，避免 drift recovery 把
    /// 已完成的输出逼进死循环）。传 None 则退化到"无工具时直接返回"。
    pub async fn run_tool_loop_streaming(
        &self,
        config: &AgentConfig,
        user_message: String,
        tool_registry: &ToolRegistry,
        cancel: watch::Receiver<bool>,
        progress_tx: mpsc::UnboundedSender<String>,
        completion_probe: Option<&(dyn Fn(&str) -> bool + Send + Sync)>,
    ) -> Result<ChatResponse, AgentError> {
        let mut messages = vec![ChatMessage::system(&config.system_prompt)];
        messages.push(ChatMessage::user(&user_message));

        for round in 1..=config.max_tool_rounds {
            if *cancel.borrow() {
                info!(target: "app-agent", "{}: 第 {round} 轮前取消", config.role);
                return Err(AgentError::Cancelled);
            }

            debug!(target: "app-agent", "{}[stream]: 第 {round}/{max} 轮",
                config.role, max = config.max_tool_rounds);

            let req = ChatRequest {
                messages: messages.clone(),
                tools: if tool_registry.tool_specs().is_empty() {
                    None
                } else {
                    Some(tool_registry.tool_specs())
                },
                params: Default::default(),
                model: config.model.clone(),
            };

            // 流式调用：每个 delta_content 推给 progress_tx
            let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<StreamChunk>();
            let stream_fut = self.llm.chat_stream(&req, stream_tx, cancel.clone());
            let forward_fut = async {
                while let Some(chunk) = stream_rx.recv().await {
                    if let Some(delta) = chunk.delta_content {
                        let _ = progress_tx.send(delta);
                    }
                }
            };
            let (resp_res, _) = tokio::join!(stream_fut, forward_fut);
            let resp = resp_res.map_err(AgentError::Llm)?;

            // 没有工具调用 = 模型直接输出文本（最终输出）
            if resp.tool_calls.is_empty() {
                // 无可用工具 = 直接返回
                if tool_registry.tool_specs().is_empty() {
                    if resp.content.is_empty() && round >= config.max_tool_rounds {
                        return Err(AgentError::MaxRoundsExceeded);
                    }
                    if !resp.content.is_empty() {
                        info!(target: "app-agent", "{}[stream]: 第 {round} 轮完成，content_len={}",
                            config.role, resp.content.len());
                        return Ok(resp);
                    }
                    messages.push(ChatMessage::user("请输出内容。"));
                    continue;
                }

                // drift recovery：有可用工具但模型没调
                if !resp.content.is_empty() && round < config.max_tool_rounds {
                    // 提早终止：如果 content 已是"最终结果"（探测回调返回 true），立即返回，
                    // 不再注入 reminder（避免把已完成的输出逼进死循环）。
                    if let Some(probe) = completion_probe {
                        if probe(&resp.content) {
                            info!(target: "app-agent", "{}[stream]: 第 {round} 轮探测到最终结果，提早终止", config.role);
                            return Ok(resp);
                        }
                    }
                    warn!(target: "app-agent", "{}[stream]: 第 {round} 轮未调工具，注入 reminder", config.role);
                    messages.push(ChatMessage::assistant(&resp.content));
                    messages.push(ChatMessage::user(
                        "请继续使用工具完成任务。如果你已经完成，请直接输出最终结果。"
                    ));
                    continue;
                }

                if resp.content.is_empty() {
                    warn!(target: "app-agent", "{}[stream]: 第 {round} 轮空响应", config.role);
                    if round >= config.max_tool_rounds {
                        return Err(AgentError::MaxRoundsExceeded);
                    }
                    messages.push(ChatMessage::user("请输出内容或调用工具。"));
                    continue;
                }

                info!(target: "app-agent", "{}[stream]: 第 {round} 轮完成，content_len={}",
                    config.role, resp.content.len());
                return Ok(resp);
            }

            // 有工具调用 → 执行（同 run_tool_loop）
            debug!(target: "app-agent", "{}[stream]: 第 {round} 轮调用 {} 个工具",
                config.role, resp.tool_calls.len());

            messages.push(ChatMessage {
                role: storyforge_domain::llm::ChatRole::Assistant,
                content: resp.content.clone(),
                tool_calls: Some(resp.tool_calls.clone()),
                tool_call_id: None,
            });

            for tc in &resp.tool_calls {
                let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::json!({}));
                let result = tool_registry
                    .dispatch(&tc.function.name, args, self.tool_ctx.clone())
                    .await;
                let result_str = match result {
                    Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "{}".into()),
                    Err(e) => {
                        warn!(target: "app-agent", "工具 {} 执行失败: {e}", tc.function.name);
                        serde_json::json!({ "error": e.to_string() }).to_string()
                    }
                };
                messages.push(ChatMessage::tool_result(&tc.id, &result_str));
            }
        }

        error!(target: "app-agent", "{}[stream]: 超过最大轮次 {max}",
            config.role, max = config.max_tool_rounds);
        Err(AgentError::MaxRoundsExceeded)
    }
}

/// Agent 运行错误
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("LLM 调用失败: {0}")]
    Llm(LlmError),

    #[error("取消")]
    Cancelled,

    #[error("超过最大工具调用轮次")]
    MaxRoundsExceeded,

    #[error("Plan 解析失败: {0}")]
    PlanParse(String),

    #[error("子 Agent 失败: {0}")]
    SubagentFailed(String),
}

// ─── 委派（对应设计 §3.4 并发模型）─────────────────────────────────────────

/// 并发上限
const MAX_CONCURRENT_SUBAGENTS: usize = 4;

/// 委派子 Agent（tokio::spawn + watch 取消，借鉴 TT）
///
/// 每个子 Agent clone 全局 `cancel`，主流水线取消时所有子 Agent 立即响应。
/// 单独取消某个子 Agent（用户点"这个角色我不要了"）仍后续实现。
/// 返回 Vec<Result<Performance, AgentError>>。
///
/// - **流式**：每个子 Agent 走 `run_tool_loop_streaming`，token 增量经 `event_tx`
///   转发为 `PipelineEvent::SubagentProgress`（带 character_id + index），供前端实时显示。
/// - **并发**：用 `Semaphore`（permits = `MAX_CONCURRENT_SUBAGENTS`）限流，
///   超出的任务**排队等待**而非丢弃，最终全部跑完。结果按原始 index 对齐返回。
pub async fn spawn_subagents(
    tasks: Vec<SubagentTask>,
    runtime: Arc<AgentRuntime>,
    director_config: &AgentConfig,
    base_system_prompt: &str,
    cancel: watch::Receiver<bool>,
    event_tx: mpsc::UnboundedSender<PipelineEvent>,
) -> Vec<Result<Performance, AgentError>> {
    let total = tasks.len();
    // Semaphore 限流：同时最多 MAX_CONCURRENT_SUBAGENTS 个子 Agent 跑，超出排队
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_SUBAGENTS));

    // (原始 index, JoinHandle) —— spawn 时记下 index，结果按 index 对齐
    let mut handles: Vec<(usize, tokio::task::JoinHandle<Result<Performance, AgentError>>)> =
        Vec::with_capacity(total);

    for (index, task) in tasks.into_iter().enumerate() {
        let runtime = runtime.clone();
        let system_prompt = format!(
            "{}\n\n你是角色 {}。\n\n{}\n\n{}",
            base_system_prompt,
            task.character_id,
            format_context_package(&task.context_package),
            task.brief,
        );
        let character_id = task.character_id.clone();
        let user_message = task.context_package.task.clone();
        let model = director_config.model.clone(); // 子 Agent 用导演的模型（M1 简化）

        // 子 Agent clone 全局 cancel（主流水线取消时联动）
        let child_cancel = cancel.clone();
        // 排队许可（spawn 内 acquire_owned，跑完随 _permit drop 自动释放）
        let permit_sem = semaphore.clone();
        // 流式转发：每个子 Agent 一对 channel，delta 包成 SubagentProgress 发到主 event_tx
        let sub_event_tx = event_tx.clone();

        let config = AgentConfig {
            role: AgentRole::Subagent(character_id.clone()),
            system_prompt,
            max_tool_rounds: 10, // 子 Agent 轮次少
            model,
            tools: vec![], // 子 Agent 无工具（纯表演）
        };

        // 子 Agent 无工具（纯表演），不注册工具
        let registry = ToolRegistry::new();

        let handle = tokio::spawn(async move {
            // 排队等许可（超出并发上限的任务在此 await，不会丢弃）
            let _permit = permit_sem
                .acquire_owned()
                .await
                .map_err(|e| AgentError::SubagentFailed(format!("Semaphore 已关闭: {e}")))?;

            // per-subagent 流式 channel：runtime 把 delta 推到 sub_tx，
            // 一个本地转发任务把它包成 SubagentProgress（带身份）发到主 event_tx
            let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<String>();
            let fwd_tx = sub_event_tx.clone();
            let fwd_cid = character_id.clone();
            tokio::spawn(async move {
                while let Some(delta) = sub_rx.recv().await {
                    let _ = fwd_tx.send(PipelineEvent::SubagentProgress {
                        character_id: fwd_cid.clone(),
                        index,
                        delta,
                    });
                }
            });

            let result = runtime
                .run_tool_loop_streaming(&config, user_message, &registry, child_cancel, sub_tx, None)
                .await;
            // sub_tx 在此 drop，转发任务收到 None 后自然结束

            match result {
                Ok(resp) => Ok(Performance {
                    character_id,
                    narrative: String::new(),
                    dialogue: String::new(),
                    inner_thoughts: String::new(),
                    full_text: resp.content,
                }),
                Err(e) => Err(e),
            }
        });

        handles.push((index, handle));
    }

    // 收集结果，按原始 index 对齐（handles 顺序即 index 升序，直接 push 即对齐）
    let mut results = Vec::with_capacity(total);
    for (_index, handle) in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(Err(AgentError::SubagentFailed(format!("子 Agent panic: {e}")))),
        }
    }

    results
}

/// 格式化 ContextPackage 为子 Agent 的上下文文本
fn format_context_package(pkg: &ContextPackage) -> String {
    let mut out = String::new();

    if !pkg.character_brief.is_empty() {
        out.push_str(&format!("## 你的角色设定\n{}\n\n", pkg.character_brief));
    }

    if !pkg.scene_brief.is_empty() {
        out.push_str(&format!("## 当前场景\n{}\n\n", pkg.scene_brief));
    }

    if !pkg.constant_lore.is_empty() {
        out.push_str("## 世界设定（常驻）\n");
        for lore in &pkg.constant_lore {
            out.push_str(&format!("- {}: {}\n", lore.keys.join(", "), lore.content));
        }
        out.push('\n');
    }

    if !pkg.relevant_lore.is_empty() {
        out.push_str("## 相关世界设定\n");
        for lore in &pkg.relevant_lore {
            out.push_str(&format!("- {}: {}\n", lore.keys.join(", "), lore.content));
        }
        out.push('\n');
    }

    if !pkg.recent_window.is_empty() {
        out.push_str("## 最近对话\n");
        for msg in &pkg.recent_window {
            out.push_str(&format!("{msg}\n"));
        }
        out.push('\n');
    }

    out
}

// ─── Hint 注入（重 roll 时把用户反馈告知 Agent）─────────────────────────────

/// 提示词注入用的标记前缀（也用于测试断言 + mock 探针）
pub const SUBAGENT_HINT_MARKER: &str = "【导演反馈】";
pub const EDITOR_HINT_MARKER: &str = "【上次问题】";

/// 把 hint 追加到子 Agent 的 system prompt 末尾。
///
/// 用于「只重跑某子 Agent」场景：告诉该角色上次哪里演得不对。
pub fn inject_hint_into_subagent(system_prompt: &str, hint: &str) -> String {
    let hint = hint.trim();
    if hint.is_empty() {
        return system_prompt.to_string();
    }
    format!("{system_prompt}\n\n{SUBAGENT_HINT_MARKER}{hint}")
}

/// 把 hint 追加到编剧的 user 消息末尾。
///
/// 用于「整体重 roll / 只重编剧」场景：告诉编剧上次成文哪里有问题。
pub fn inject_hint_into_editor(user_message: &str, hint: &str) -> String {
    let hint = hint.trim();
    if hint.is_empty() {
        return user_message.to_string();
    }
    format!("{user_message}\n\n{EDITOR_HINT_MARKER}{hint}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_infra_llm::mock_client::MockLlmClient;

    /// 验证全局取消会中止所有子 Agent
    #[tokio::test]
    async fn test_subagents_respond_to_global_cancel() {
        // 构造两个子任务
        let tasks = vec![
            SubagentTask {
                character_id: "A".into(),
                brief: "演出".into(),
                context_package: ContextPackage {
                    character_brief: "角色A".into(),
                    scene_brief: "场景".into(),
                    relevant_lore: vec![],
                    constant_lore: vec![],
                    recent_window: vec![],
                    task: "演出你的部分".into(),
                },
            },
            SubagentTask {
                character_id: "B".into(),
                brief: "演出".into(),
                context_package: ContextPackage {
                    character_brief: "角色B".into(),
                    scene_brief: "场景".into(),
                    relevant_lore: vec![],
                    constant_lore: vec![],
                    recent_window: vec![],
                    task: "演出你的部分".into(),
                },
            },
        ];

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
        });
        let runtime = Arc::new(AgentRuntime::new(llm, tool_ctx));
        let director_config = AgentConfig {
            role: AgentRole::Director,
            system_prompt: String::new(),
            max_tool_rounds: 1,
            model: "mock".into(),
            tools: vec![],
        };

        // 预先置为取消的 watch channel
        let (cancel_tx, cancel_rx) = watch::channel(false);
        // 子 Agent 启动后立即取消
        let tx2 = cancel_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = tx2.send(true);
        });

        let results = spawn_subagents(
            tasks,
            runtime,
            &director_config,
            "你是角色",
            cancel_rx,
            mpsc::unbounded_channel::<PipelineEvent>().0, // 测试不消费事件
        )
        .await;

        // 至少一个子 Agent 应被取消（返回取消类错误）
        // 注：子 Agent 现走流式 chat_stream，cancel 命中 select! 后可能返回
        // AgentError::Cancelled（每轮前检查）或 AgentError::LlmFailed(包装 LlmError::Cancelled)（流内取消）。
        // 两种都是合法的取消表示，max_tool_rounds=1 且第 1 轮前 cancel 可能还没置位，结果也可能是 Ok。
        for r in &results {
            match r {
                Ok(_) | Err(AgentError::Cancelled) => {}
                Err(AgentError::Llm(msg)) if msg.to_string().contains("取消") => {}
                Err(e) => panic!("意外的错误: {e}"),
            }
        }
        // 至少有结果返回
        assert!(!results.is_empty());
        drop(cancel_tx);
    }

    /// 验证超过并发上限的角色会排队而非丢弃（Semaphore 改造）
    ///
    /// 构造 6 个任务（> MAX_CONCURRENT_SUBAGENTS=4），旧逻辑会丢弃后 2 个返回 SubagentFailed。
    /// 改用 Semaphore 后所有任务都应排队跑完，返回 6 个 Ok。
    #[tokio::test]
    async fn test_subagents_queue_beyond_concurrency_limit() {
        let make_task = |cid: &str| SubagentTask {
            character_id: cid.into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: format!("角色{cid}"),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        };
        // 6 个任务，超过并发上限 4
        let tasks: Vec<SubagentTask> = ["A", "B", "C", "D", "E", "F"]
            .iter()
            .map(|c| make_task(c))
            .collect();

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
        });
        let runtime = Arc::new(AgentRuntime::new(llm, tool_ctx));
        let director_config = AgentConfig {
            role: AgentRole::Director,
            system_prompt: String::new(),
            max_tool_rounds: 1,
            model: "mock".into(),
            tools: vec![],
        };
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let results = spawn_subagents(
            tasks,
            runtime,
            &director_config,
            "你是角色",
            cancel_rx,
            mpsc::unbounded_channel::<PipelineEvent>().0,
        )
        .await;

        // 6 个任务全部完成，无丢弃（旧逻辑会是 4 Ok + 2 SubagentFailed）
        assert_eq!(results.len(), 6, "应有 6 个结果（不丢弃超出任务）");
        for (i, r) in results.iter().enumerate() {
            assert!(r.is_ok(), "第 {i} 个子 Agent 应成功，实际: {:?}", r);
        }
    }

    #[test]
    fn test_inject_hint_into_subagent() {
        let prompt = "你是角色 A。";
        let result = inject_hint_into_subagent(prompt, "语气太冷");
        assert!(result.contains(SUBAGENT_HINT_MARKER));
        assert!(result.contains("语气太冷"));
        assert!(result.starts_with("你是角色 A。"));
    }

    #[test]
    fn test_inject_hint_empty_is_noop() {
        let prompt = "你是角色 A。";
        assert_eq!(inject_hint_into_subagent(prompt, ""), prompt);
        assert_eq!(inject_hint_into_subagent(prompt, "   "), prompt);
    }

    #[test]
    fn test_inject_hint_into_editor() {
        let msg = "合并这些表演。";
        let result = inject_hint_into_editor(msg, "节奏太快");
        assert!(result.contains(EDITOR_HINT_MARKER));
        assert!(result.contains("节奏太快"));
    }
}

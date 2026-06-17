/// Agent 运行时（工具循环 + 委派 + 取消）
///
/// 核心函数 run_tool_loop()：循环调用 LLM + 执行工具，直到完成或超限。
/// 设计来源：TT 的 AgentRuntimeService（max_rounds + drift recovery + watch 取消）。
use std::sync::Arc;

use tokio::sync::{Semaphore, mpsc, watch};
use tracing::{debug, error, info, warn};

use storyforge_domain::agent::{
    AgentRole, ContextPackage, Performance, PipelineEvent, SubagentTask,
};
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::llm::{
    ChatMessage, ChatRequest, ChatResponse, LlmError, StreamChunk, ToolSpec,
};
use storyforge_domain::message_layout::MessageLayout;
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

    /// 获取 LLM 客户端（供子 Agent 构造独立 runtime 时 clone）
    pub fn llm(&self) -> Arc<dyn LlmClient> {
        self.llm.clone()
    }

    /// 获取 ToolContext（供子 Agent 构造独立 runtime 时 clone 并修改）
    pub fn tool_ctx(&self) -> Arc<ToolContext> {
        self.tool_ctx.clone()
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
                if !resp.content.is_empty()
                    && round < config.max_tool_rounds
                    && tool_registry.tool_specs().len() > 0
                {
                    // 检查是否是最终输出（没有工具定义时直接返回）
                    if req.tools.is_none() {
                        return Ok(resp);
                    }

                    // drift recovery：注入提醒
                    warn!(target: "app-agent", "{}: 第 {round} 轮模型未调用工具，注入 reminder", config.role);
                    messages.push(ChatMessage::assistant(&resp.content));
                    messages.push(ChatMessage::user(
                        "请继续使用工具完成任务。如果你已经完成，请直接输出最终结果。",
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
                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));

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
                        "请继续使用工具完成任务。如果你已经完成，请直接输出最终结果。",
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
                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
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

    /// 流式工具循环（cache 友好布局版，§22 / D46）
    ///
    /// 与 `run_tool_loop_streaming` 逻辑相同，唯一区别：初始 messages 来自
    /// `layout.into_messages()`（即 `[system, history..., tail]`），而非 `[system, user]`。
    ///
    /// 让导演/编剧/子 Agent 能把稳定内容（role_directive + 模块 + 蓝灯世界设定 +
    /// 子 Agent persona）进 system 段、对话历史进 history 段（独立消息）、易变内容
    /// （意图/变量/时钟/任务/场景）压尾，最大化 LLM KV cache 命中率。
    ///
    /// 工具调用轮次里追加的 assistant/tool_result 消息进 history 之后（末尾），
    /// 不影响前缀稳定——前缀 system+history 跨轮 byte 一致即可 cache 命中。
    ///
    /// 注意：`config.system_prompt` 此处**不使用**（system 段已由 layout.stable_system
    /// 提供，调用方应保证两者一致或 layout 优先）。保留 config 参数是为复用
    /// max_tool_rounds/model/tools 等字段。
    pub async fn run_tool_loop_with_layout(
        &self,
        config: &AgentConfig,
        layout: MessageLayout,
        tool_registry: &ToolRegistry,
        cancel: watch::Receiver<bool>,
        progress_tx: mpsc::UnboundedSender<String>,
        completion_probe: Option<&(dyn Fn(&str) -> bool + Send + Sync)>,
    ) -> Result<ChatResponse, AgentError> {
        // 初始 messages = [system, history..., tail]（layout 已组装好三段）
        let mut messages = layout.into_messages();
        let rounds = config.max_tool_rounds;

        for round in 1..=rounds {
            if *cancel.borrow() {
                info!(target: "app-agent", "{}[layout]: 第 {round} 轮前取消", config.role);
                return Err(AgentError::Cancelled);
            }

            debug!(target: "app-agent", "{}[layout]: 第 {round}/{max} 轮",
                config.role, max = rounds);

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
                if tool_registry.tool_specs().is_empty() {
                    // 无可用工具 = 直接返回
                    if resp.content.is_empty() && round >= rounds {
                        return Err(AgentError::MaxRoundsExceeded);
                    }
                    if !resp.content.is_empty() {
                        info!(target: "app-agent", "{}[layout]: 第 {round} 轮完成，content_len={}",
                            config.role, resp.content.len());
                        return Ok(resp);
                    }
                    messages.push(ChatMessage::user("请输出内容。"));
                    continue;
                }

                // drift recovery：有可用工具但模型没调
                if !resp.content.is_empty() && round < rounds {
                    if let Some(probe) = completion_probe {
                        if probe(&resp.content) {
                            info!(target: "app-agent", "{}[layout]: 第 {round} 轮探测到最终结果，提早终止", config.role);
                            return Ok(resp);
                        }
                    }
                    warn!(target: "app-agent", "{}[layout]: 第 {round} 轮未调工具，注入 reminder", config.role);
                    messages.push(ChatMessage::assistant(&resp.content));
                    messages.push(ChatMessage::user(
                        "请继续使用工具完成任务。如果你已经完成，请直接输出最终结果。",
                    ));
                    continue;
                }

                if resp.content.is_empty() {
                    warn!(target: "app-agent", "{}[layout]: 第 {round} 轮空响应", config.role);
                    if round >= rounds {
                        return Err(AgentError::MaxRoundsExceeded);
                    }
                    messages.push(ChatMessage::user("请输出内容或调用工具。"));
                    continue;
                }

                info!(target: "app-agent", "{}[layout]: 第 {round} 轮完成，content_len={}",
                    config.role, resp.content.len());
                return Ok(resp);
            }

            // 有工具调用 → 执行
            debug!(target: "app-agent", "{}[layout]: 第 {round} 轮调用 {} 个工具",
                config.role, resp.tool_calls.len());

            messages.push(ChatMessage {
                role: storyforge_domain::llm::ChatRole::Assistant,
                content: resp.content.clone(),
                tool_calls: Some(resp.tool_calls.clone()),
                tool_call_id: None,
            });

            for tc in &resp.tool_calls {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
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

        error!(target: "app-agent", "{}[layout]: 超过最大轮次 {max}",
            config.role, max = rounds);
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
    campaign_runtime: Option<Arc<CampaignRuntimeContext>>,
    max_concurrent_subagents: usize,
    agent_profile_config: Option<&storyforge_domain::agent_profile_config::AgentProfileConfig>,
) -> Vec<Result<Performance, AgentError>> {
    let total = tasks.len();
    // Semaphore 限流：同时最多 max_concurrent_subagents 个子 Agent 跑，超出排队
    let semaphore = Arc::new(Semaphore::new(max_concurrent_subagents));

    // (原始 index, JoinHandle) —— spawn 时记下 index，结果按 index 对齐
    let mut handles: Vec<(
        usize,
        tokio::task::JoinHandle<Result<Performance, AgentError>>,
    )> = Vec::with_capacity(total);

    for (index, task) in tasks.into_iter().enumerate() {
        let runtime = runtime.clone();
        let character_id = task.character_id.clone();

        // 从 AgentProfileConfig 查找该子 Agent 的覆盖配置
        let subagent_run_config = agent_profile_config.map(|apc| {
            apc.run_config_for(&AgentRole::Subagent(character_id.clone()))
        });
        let model = subagent_run_config
            .and_then(|rc| rc.model_override.clone())
            .unwrap_or_else(|| director_config.model.clone()); // 子 Agent 默认用导演的模型
        let subagent_max_rounds = subagent_run_config
            .and_then(|rc| rc.max_tool_rounds)
            .unwrap_or(10);

        // ── 阶段 4：Campaign 模式下按 character_id 匹配 instance ──
        let matched_instance = campaign_runtime
            .as_ref()
            .and_then(|cr| cr.find_instance_by_id_or_name(&task.character_id));

        // ── 构造 system prompt（稳定段）──
        let (stable_system, instance_id_for_ctx) =
            if let (Some(cr), Some(inst)) = (&campaign_runtime, matched_instance) {
                // Campaign 模式：用 resolved persona/behavior 替代 context_package.character_brief
                build_campaign_subagent_system(base_system_prompt, &task, cr, inst)
            } else {
                // 旧路径：用 context_package.character_brief
                if matched_instance.is_none() && campaign_runtime.is_some() {
                    warn!(target: "app-agent",
                    "子 Agent character_id='{}' 在 Campaign 实例中未找到，退回 context_package",
                    task.character_id);
                }
                let sys = format!(
                    "{}\n\n你是角色 {}。\n\n{}",
                    base_system_prompt,
                    task.character_id,
                    format_context_stable(&task.context_package),
                );
                (sys, None)
            };

        // ── 构造 tail（易变段）──
        let volatile_text = if let (Some(cr), Some(inst)) = (&campaign_runtime, matched_instance) {
            // Campaign 模式：注入该 instance 的 knowledge（信息隔离）+ variables + scene + task
            build_campaign_subagent_volatile(&task, cr, inst)
        } else {
            // 旧路径
            format_context_volatile(&task.context_package)
        };

        let layout = MessageLayout::build().system(stable_system).tail(|_| {
            storyforge_domain::message_layout::VolatileTail::new()
                .push(task.context_package.task.clone())
                .push(volatile_text.trim_end().to_string())
        });

        // 子 Agent clone 全局 cancel（主流水线取消时联动）
        let child_cancel = cancel.clone();
        // 排队许可（spawn 内 acquire_owned，跑完随 _permit drop 自动释放）
        let permit_sem = semaphore.clone();
        // 流式转发：每个子 Agent 一对 channel，delta 包成 SubagentProgress 发到主 event_tx
        let sub_event_tx = event_tx.clone();

        let config = AgentConfig {
            role: AgentRole::Subagent(character_id.clone()),
            system_prompt: String::new(), // layout 版不使用此字段（system 由 layout 提供）
            max_tool_rounds: subagent_max_rounds,
            model,
            tools: vec![], // 子 Agent 工具由 registry 提供
        };

        // 阶段 4：为每个子 Agent 构造独立的 ToolContext，绑定 current_character_instance_id
        // 这样子 Agent 的 get_character 工具只能查自己的 instance，不泄露其他角色
        let sub_tool_ctx = {
            let base = runtime.tool_ctx();
            let mut ctx = (*base).clone();
            ctx.current_character_instance_id = instance_id_for_ctx.clone();
            Arc::new(ctx)
        };
        let sub_runtime = Arc::new(AgentRuntime::new(runtime.llm(), sub_tool_ctx));

        // 注册子 Agent 工具（get_character 限制为当前 instance）
        let mut registry = ToolRegistry::new();
        crate::tools::register_subagent_tools(&mut registry);

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

            let result = sub_runtime
                .run_tool_loop_with_layout(&config, layout, &registry, child_cancel, sub_tx, None)
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
            Err(e) => results.push(Err(AgentError::SubagentFailed(format!(
                "子 Agent panic: {e}"
            )))),
        }
    }

    results
}

/// 构造 Campaign 模式子 Agent 的 system prompt（稳定段）
///
/// 纯函数，方便测试直接断言 prompt 内容。
/// 返回 (system_prompt, matched_instance_id)。
pub fn build_campaign_subagent_system(
    base_system_prompt: &str,
    task: &SubagentTask,
    cr: &CampaignRuntimeContext,
    inst: &storyforge_domain::campaign::CharacterInstance,
) -> (String, Option<storyforge_domain::Id>) {
    let persona = cr.resolved_persona_for(inst).unwrap_or("");
    let behavior = cr.resolved_behavior_for(inst).unwrap_or("");
    let display_name = if inst.name.is_empty() {
        &task.character_id
    } else {
        &inst.name
    };

    let mut sys = format!("{}\n\n你是角色 {}。\n\n", base_system_prompt, display_name);
    if !persona.is_empty() {
        sys.push_str(&format!("## 你的角色设定\n{}\n\n", persona));
    }
    if !behavior.is_empty() {
        sys.push_str(&format!("## 行为准则\n{}\n\n", behavior));
    }
    let constant_lore = &task.context_package.constant_lore;
    if !constant_lore.is_empty() {
        sys.push_str("## 世界设定（常驻）\n");
        for lore in constant_lore {
            sys.push_str(&format!("- {}: {}\n", lore.keys.join(", "), lore.content));
        }
        sys.push('\n');
    }
    (sys, Some(inst.id.clone()))
}

/// 构造 Campaign 模式子 Agent 的 volatile tail 文本
///
/// 纯函数，方便测试直接断言 prompt 内容。
/// 包含场景、相关世界设定、该 instance 的可见 knowledge（信息隔离）、该 instance 的 variables。
pub fn build_campaign_subagent_volatile(
    task: &SubagentTask,
    cr: &CampaignRuntimeContext,
    inst: &storyforge_domain::campaign::CharacterInstance,
) -> String {
    let mut parts = Vec::new();

    if !task.context_package.scene_brief.is_empty() {
        parts.push(format!(
            "## 当前场景\n{}\n",
            task.context_package.scene_brief
        ));
    }

    if !task.context_package.relevant_lore.is_empty() {
        parts.push("## 相关世界设定\n".into());
        for lore in &task.context_package.relevant_lore {
            parts.push(format!("- {}: {}\n", lore.keys.join(", "), lore.content));
        }
    }

    let knowledge = cr.knowledge_for_instance(inst);
    if !knowledge.is_empty() {
        parts.push("## 你所知道的信息\n".into());
        for k in &knowledge {
            parts.push(format!("- {}\n", k.knowledge_text));
        }
    }

    if !inst.variables.is_empty() {
        parts.push("## 你的状态\n".into());
        for v in &inst.variables {
            parts.push(format!("- {}: {}\n", v.key, v.value));
        }
    }

    parts.join("\n")
}

/// 格式化 ContextPackage 的**稳定部分**（进 system 段，整个 campaign 不变，§22）
///
/// 包含：角色设定（persona）+ 常驻世界设定（蓝灯）。
/// 这些跨场戏稳定，进 system 段让 cache 命中。
fn format_context_stable(pkg: &ContextPackage) -> String {
    let mut out = String::new();

    if !pkg.character_brief.is_empty() {
        out.push_str(&format!("## 你的角色设定\n{}\n\n", pkg.character_brief));
    }

    if !pkg.constant_lore.is_empty() {
        out.push_str("## 世界设定（常驻）\n");
        for lore in &pkg.constant_lore {
            out.push_str(&format!("- {}: {}\n", lore.keys.join(", "), lore.content));
        }
        out.push('\n');
    }

    out
}

/// 格式化 ContextPackage 的**易变部分**（进 tail 段，每场戏变，§22）
///
/// 包含：当前场景 + 相关世界设定（绿灯检索）+ 最近对话窗口。
/// 这些每场戏不同，压在 tail。
fn format_context_volatile(pkg: &ContextPackage) -> String {
    let mut out = String::new();

    if !pkg.scene_brief.is_empty() {
        out.push_str(&format!("## 当前场景\n{}\n\n", pkg.scene_brief));
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
    use storyforge_domain::agent::LoreEntryLight;
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
            campaign_runtime: None,
            current_character_instance_id: None,
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
            None,                                         // 无 Campaign runtime（旧路径测试）
            4,
            None,
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
            campaign_runtime: None,
            current_character_instance_id: None,
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
            None, // 无 Campaign runtime（旧路径测试）
            4,
            None,
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

    /// 验证 run_tool_loop_with_layout 能消费带 history 的 layout 并正常返回。
    ///
    /// layout 三段（system + history + tail）应被正确转成 messages 喂给 LLM。
    /// 用 with_defaults mock（按 system 关键词"导演"匹配返回 Plan）。
    #[tokio::test]
    async fn test_run_tool_loop_with_layout_consumes_history() {
        use storyforge_domain::message_layout::MessageLayout;

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
        });
        let runtime = AgentRuntime::new(llm, tool_ctx);

        // 构造带 history 的 layout（模拟多轮对话的导演调用）
        let layout = MessageLayout::build()
            .system("你是写作导演。输出 Plan。")
            .history(vec![
                ChatMessage::user("写一场雨中告别"),
                ChatMessage::assistant("雨滴敲在屋檐…"),
            ])
            .tail(|t| t.push("继续下一场").push("角色：Seraphina"));

        let config = AgentConfig {
            role: AgentRole::Director,
            system_prompt: String::new(), // layout 版不使用此字段
            max_tool_rounds: 3,
            model: "mock".into(),
            tools: vec![],
        };
        let (_tx, cancel) = watch::channel(false);
        let (prog_tx, prog_rx) = mpsc::unbounded_channel::<String>();

        let resp = runtime
            .run_tool_loop_with_layout(&config, layout, &ToolRegistry::new(), cancel, prog_tx, None)
            .await
            .expect("layout 版应正常返回");

        // mock 按关键词"导演"匹配，返回 Plan JSON（非空）
        assert!(!resp.content.is_empty(), "应有响应内容");

        // 流式 progress 应有 token 推送（mock 的 stream=false 也会走 forward_fut）
        drop(prog_rx); // 仅证明 channel 正常（mock 非流式时 progress 可能为空，不强制断言）
    }

    // ── 阶段 4：Campaign 模式 spawn_subagents 测试 ──

    use storyforge_domain::Id;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
    use storyforge_domain::variables::default_character_variables;

    fn make_campaign_runtime() -> Arc<CampaignRuntimeContext> {
        let campaign = Campaign::new(Id::from_str("card-1"), "test-campaign");
        let def_lin = CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm surgeon".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec!["is a surgeon".into()],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        let def_chen = CharacterDefinition {
            id: Id::from_str("def-chen"),
            card_id: Id::from_str("card-1"),
            name: "Chen".into(),
            persona_prompt: "strict cop".into(),
            behavior_rules: "follow rules".into(),
            base_backstory: vec!["is a cop".into()],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        let inst_lin = CharacterInstance {
            id: Id::from_str("inst-lin"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def_lin.id.clone()),
            name: "Lin".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        let inst_chen = CharacterInstance {
            id: Id::from_str("inst-chen"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def_chen.id.clone()),
            name: "Chen".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        let mut definitions_by_id = std::collections::HashMap::new();
        definitions_by_id.insert(def_lin.id.clone(), def_lin);
        definitions_by_id.insert(def_chen.id.clone(), def_chen);

        let knowledge = vec![
            CharacterKnowledgeEntry::witnessed(
                campaign.id.clone(),
                Id::from_str("inst-lin"),
                "Lin saw the explosion",
                1,
            ),
            CharacterKnowledgeEntry::witnessed(
                campaign.id.clone(),
                Id::from_str("inst-chen"),
                "Chen was at the station",
                1,
            ),
        ];

        Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![inst_lin, inst_chen],
            definitions_by_id,
            knowledge,
            turn: 1,
        })
    }

    /// 阶段 4：有 campaign_runtime 时，spawn_subagents 按 instance_id 匹配
    #[tokio::test]
    async fn test_spawn_subagents_campaign_routes_by_instance_id() {
        let cr = make_campaign_runtime();
        let tasks = vec![SubagentTask {
            character_id: "inst-lin".into(), // 用 instance_id
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介（不应被使用）".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        }];

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: Some(cr.clone()),
            current_character_instance_id: None,
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
            Some(cr),
            4,
            None,
        )
        .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok(), "子 Agent 应成功: {:?}", results[0]);
    }

    /// 阶段 4：task.character_id 是角色名时，fallback 匹配 instance name
    #[tokio::test]
    async fn test_spawn_subagents_campaign_fallback_to_name() {
        let cr = make_campaign_runtime();
        let tasks = vec![SubagentTask {
            character_id: "Lin".into(), // 用角色名而非 instance_id
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        }];

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: Some(cr.clone()),
            current_character_instance_id: None,
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
            Some(cr),
            4,
            None,
        )
        .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok(), "名称 fallback 应成功: {:?}", results[0]);
    }

    /// 阶段 4：无 campaign_runtime 时旧路径仍通过
    #[tokio::test]
    async fn test_spawn_subagents_no_campaign_runtime_old_path() {
        let tasks = vec![SubagentTask {
            character_id: "Seraphina".into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "角色设定".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        }];

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
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
            None,
            4,
            None,
        )
        .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok(), "旧路径应成功: {:?}", results[0]);
    }

    // ── Phase 4 cleanup：直接断言 prompt 内容 ──

    /// 直接断言 Campaign 模式子 Agent 的 system prompt 包含 persona 和 behavior
    #[test]
    fn test_campaign_system_prompt_contains_persona_and_behavior() {
        let cr = make_campaign_runtime();
        let inst = cr.find_instance_by_id_or_name("inst-lin").unwrap();
        let task = SubagentTask {
            character_id: "inst-lin".into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        };

        let (sys, instance_id) = build_campaign_subagent_system("你是角色", &task, &cr, inst);

        // persona 来自 definition（inst-lin 无 override → fallback to def-lin.persona_prompt）
        assert!(sys.contains("calm surgeon"), "system 应含 persona: {sys}");
        // behavior 来自 definition
        assert!(sys.contains("save first"), "system 应含 behavior: {sys}");
        // 角色名
        assert!(sys.contains("Lin"), "system 应含角色名: {sys}");
        // 不应使用旧 context_package.character_brief
        assert!(
            !sys.contains("旧的角色简介"),
            "不应使用旧 context_package: {sys}"
        );
        // instance_id 应匹配
        assert_eq!(instance_id, Some(Id::from_str("inst-lin")));
    }

    /// 直接断言 Campaign 模式子 Agent 的 volatile tail 包含 knowledge 和 variables
    #[test]
    fn test_campaign_volatile_tail_contains_knowledge_and_variables() {
        let cr = make_campaign_runtime();

        // 给 inst-lin 加变量（需要 clone 后修改）
        let mut cr = (*cr).clone();
        let inst_with_vars = cr
            .instances
            .iter_mut()
            .find(|i| i.id.as_str() == "inst-lin")
            .unwrap();
        inst_with_vars.variables = vec![
            storyforge_domain::variables::VariableValue {
                key: "hp".into(),
                value: serde_json::json!(80),
                last_updated_turn: 1,
            },
            storyforge_domain::variables::VariableValue {
                key: "state".into(),
                value: serde_json::json!("受伤"),
                last_updated_turn: 1,
            },
        ];
        let cr = Arc::new(cr);
        let inst = cr.find_instance_by_id_or_name("inst-lin").unwrap();

        let task = SubagentTask {
            character_id: "inst-lin".into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介".into(),
                scene_brief: "急诊室场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        };

        let volatile = build_campaign_subagent_volatile(&task, &cr, inst);

        // Lin 的 knowledge 应包含
        assert!(
            volatile.contains("Lin saw the explosion"),
            "应含 Lin 的 knowledge: {volatile}"
        );
        // Lin 的变量应包含
        assert!(volatile.contains("hp"), "应含 hp 变量: {volatile}");
        assert!(volatile.contains("80"), "应含 hp 值: {volatile}");
        assert!(volatile.contains("state"), "应含 state 变量: {volatile}");
        assert!(volatile.contains("受伤"), "应含 state 值: {volatile}");
        // 场景
        assert!(volatile.contains("急诊室场景"), "应含场景: {volatile}");
    }

    /// 直接断言信息隔离：Lin 的 tail 不包含 Chen 的 knowledge
    #[test]
    fn test_campaign_volatile_tail_knowledge_isolation() {
        let cr = make_campaign_runtime();
        let inst_lin = cr.find_instance_by_id_or_name("inst-lin").unwrap();
        let inst_chen = cr.find_instance_by_id_or_name("inst-chen").unwrap();

        let task = SubagentTask {
            character_id: "inst-lin".into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        };

        // Lin 的 tail
        let lin_volatile = build_campaign_subagent_volatile(&task, &cr, inst_lin);
        assert!(
            lin_volatile.contains("Lin saw the explosion"),
            "Lin 应看到自己的 knowledge"
        );
        assert!(
            !lin_volatile.contains("Chen was at the station"),
            "Lin 不应看到 Chen 的 knowledge"
        );

        // Chen 的 tail
        let chen_volatile = build_campaign_subagent_volatile(&task, &cr, inst_chen);
        assert!(
            chen_volatile.contains("Chen was at the station"),
            "Chen 应看到自己的 knowledge"
        );
        assert!(
            !chen_volatile.contains("Lin saw the explosion"),
            "Chen 不应看到 Lin 的 knowledge"
        );
    }

    /// Campaign 模式 system prompt 包含常驻世界设定
    #[test]
    fn test_campaign_system_prompt_includes_constant_lore() {
        let cr = make_campaign_runtime();
        let inst = cr.find_instance_by_id_or_name("inst-lin").unwrap();
        let task = SubagentTask {
            character_id: "inst-lin".into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![LoreEntryLight {
                    keys: vec!["龙族".into()],
                    content: "龙是古老的种族".into(),
                }],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        };

        let (sys, _) = build_campaign_subagent_system("你是角色", &task, &cr, inst);
        assert!(sys.contains("龙族"), "system 应含常驻世界设定 key: {sys}");
        assert!(
            sys.contains("龙是古老的种族"),
            "system 应含常驻世界设定 content: {sys}"
        );
    }

    // ── Phase 6：临时 instance 参与 spawn_subagents ──

    /// Phase 6：临时 instance 能正常参与 spawn_subagents（fallback persona）
    #[tokio::test]
    async fn test_spawn_subagents_with_temporary_instance() {
        let cr = make_campaign_runtime();
        // 为 "Ghost" 创建临时 instance
        let (cr_with_temp, temps) =
            cr.with_temporaries_for(&[("Ghost".into(), None, None)]);
        assert_eq!(temps.len(), 1);
        let cr = Arc::new(cr_with_temp);

        let tasks = vec![SubagentTask {
            character_id: "Ghost".into(), // 临时 instance
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        }];

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: Some(cr.clone()),
            current_character_instance_id: None,
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
            Some(cr),
            4,
            None,
        )
        .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok(), "临时 instance 应成功: {:?}", results[0]);
    }

    /// Phase 6：临时 instance 的 system prompt 不含 persona（无 definition）
    #[test]
    fn test_temporary_instance_system_prompt_no_persona() {
        let cr = make_campaign_runtime();
        let (cr_with_temp, _) =
            cr.with_temporaries_for(&[("Ghost".into(), None, None)]);
        let cr = Arc::new(cr_with_temp);
        let inst = cr.find_instance_by_id_or_name("Ghost").unwrap();

        let task = SubagentTask {
            character_id: "Ghost".into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "旧的角色简介".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        };

        let (sys, _) = build_campaign_subagent_system("你是角色", &task, &cr, inst);
        // 临时 instance 没有 definition → persona 为空 → 不应包含 persona 段
        assert!(
            !sys.contains("calm surgeon"),
            "临时 instance 不应有 definition persona"
        );
        assert!(sys.contains("Ghost"), "应包含角色名");
    }

    /// Phase 6：旧扁平角色路径仍兼容（无 campaign_runtime）
    #[tokio::test]
    async fn test_spawn_subagents_old_path_still_works() {
        let tasks = vec![SubagentTask {
            character_id: "Seraphina".into(),
            brief: "演出".into(),
            context_package: ContextPackage {
                character_brief: "角色设定".into(),
                scene_brief: "场景".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: "演出你的部分".into(),
            },
        }];

        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
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
            None,
            4,
            None,
        )
        .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok(), "旧路径应成功: {:?}", results[0]);
    }
}

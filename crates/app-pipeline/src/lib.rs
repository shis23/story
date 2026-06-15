/// 写作流水线编排（对应设计 §3.1 状态机）
///
/// PipelineOrchestrator 连接 app-agent（工具循环）和 app-conversation（对话树），
/// 实现完整写作流程：用户意图 → 导演 Plan → 子 Agent 并行 → 编剧成文 → 写入对话树。
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tracing::{error, info};

use storyforge_domain::agent::{
    AgentRole, ContextPackage, Draft, LoreEntryLight, PipelineEvent,
    PipelineState, Plan, SubagentTask, WritingSession,
};
use storyforge_domain::conversation::Provenance;
use storyforge_domain::Id;

use storyforge_app_agent::{
    AgentConfig, AgentError, AgentRuntime, ToolContext, ToolRegistry, spawn_subagents,
    inject_hint_into_editor, inject_hint_into_subagent,
    tools::register_director_tools,
};
use storyforge_app_conversation::{ConversationStore, PartialRollTarget, build_provenance};
use storyforge_infra_llm::LlmClient;

// ─── 错误类型 ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("Agent 错误: {0}")]
    Agent(#[from] AgentError),

    #[error("对话错误: {0}")]
    Conversation(#[from] storyforge_app_conversation::ConversationError),

    #[error("LLM 错误: {0}")]
    Llm(storyforge_domain::llm::LlmError),

    #[error("Plan 解析失败: {0}")]
    PlanParse(String),

    #[error("重 roll 失败: {0}")]
    Regenerate(String),

    #[error("流水线已取消")]
    Cancelled,

    #[error("流水线状态错误: {0}")]
    InvalidState(String),
}

// ─── 编剧提示词（硬编码骨架，M3 再接模块系统）────────────────────────────────

const DIRECTOR_SYSTEM_PROMPT: &str = r#"你是写作导演。用户给你写作意图，你要：

1. 调用 search_world_info / get_character 了解可用素材
2. 判断本场戏：核心冲突是什么？引入哪些角色？
3. 决定出场角色，为每个角色分配任务
4. 为每个角色构造专属上下文包
5. 输出结构化 Plan

不要自己写正文。

【输出格式（必须严格遵守）】
调用 emit_plan 工具输出 Plan；如果无法调用工具，则直接输出如下 JSON（前后不要有任何其他文字、解释或 markdown 标记）：

{"scene_brief": "本场戏的一句话场景简述", "subagent_tasks": [{"character_id": "出场角色名（必须来自可用角色列表）", "brief": "该角色在本场戏的任务简述"}]}

示例（用户意图「写一场雨中告别」，可用角色 Seraphina）：
{"scene_brief": "雨中告别，屋檐下两人对话", "subagent_tasks": [{"character_id": "Seraphina", "brief": "演出告别时的温柔与不舍"}]}"#;

const EDITOR_SYSTEM_PROMPT: &str = r#"你是编剧。收集所有子 Agent 的表演，合并成连贯成文：

1. 节奏把控、视角切换、过渡衔接
2. 输出最终成文（Markdown）
3. 标注哪些子表演被你裁剪/改动了

直接输出成文，不需要调用工具。"#;

const SUBAGENT_SYSTEM_PROMPT_TEMPLATE: &str = r#"你是角色 {name}。根据导演给你的任务和专属上下文，演出你这个角色在这场戏的行为/对白/心理。只演你自己，不要替别人说话。输出纯表演，不要解释。"#;

// ─── 重 roll 请求（应用层 DTO，对应设计 §3.7.3）────────────────────────────

/// 重 roll 请求
///
/// `targets` 为空 = 整体重 roll；含 Director = 整体重 roll；
/// 仅 Editor = 只重编剧；仅 Subagent(id) = 只重该子 Agent。
#[derive(Debug, Clone)]
pub struct RegenerateRequest {
    pub conversation_id: Id,
    pub node_id: Id,
    pub targets: Vec<PartialRollTarget>,
    /// 用户附加提示词（可选），告知 Agent 上次哪里有问题
    pub hint: Option<String>,
    /// 随机种子（None = 新生成一个）
    pub seed: Option<u64>,
}

// ─── 流水线编排器 ──────────────────────────────────────────────────────────

/// 写作上下文（提供给流水线的数据源）
pub struct WritingContext {
    pub characters: Vec<Arc<storyforge_domain::character::Character>>,
    pub world_info: Option<Arc<storyforge_domain::world_info::WorldInfoBook>>,
    pub conversation_id: Id,
}

/// 流水线编排器
pub struct PipelineOrchestrator {
    runtime: Arc<AgentRuntime>,
    conv_store: Arc<ConversationStore>,
    /// 当前流水线状态
    state: PipelineState,
    /// 最近一次会话
    session: Option<WritingSession>,
}

impl PipelineOrchestrator {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        conv_store: Arc<ConversationStore>,
        tool_ctx: Arc<ToolContext>,
    ) -> Self {
        let runtime = Arc::new(AgentRuntime::new(llm, tool_ctx));
        Self {
            runtime,
            conv_store,
            state: PipelineState::Idle,
            session: None,
        }
    }

    /// 获取当前状态
    pub fn state(&self) -> &PipelineState {
        &self.state
    }

    /// 获取最近会话
    pub fn session(&self) -> Option<&WritingSession> {
        self.session.as_ref()
    }

    /// 标记流水线为中止状态并推送事件
    fn set_aborted(&mut self, event_tx: &mpsc::UnboundedSender<PipelineEvent>) {
        self.state = PipelineState::Aborted;
        let _ = event_tx.send(PipelineEvent::StateChanged {
            state: self.state.clone(),
        });
    }

    /// 标记 Aborted 并返回错误（方便用 `?` 链式传播）
    fn abort_with<E>(&mut self, event_tx: &mpsc::UnboundedSender<PipelineEvent>, err: E) -> E {
        self.set_aborted(event_tx);
        err
    }

    /// 运行完整写作流水线（对应设计 §3.1 状态机）
    ///
    /// 流程：Idle → Directing(导演) → Delegating(子×N) → Editing(编剧) → Review → Committed
    /// 通过 event_tx 推送流式事件到前端。
    pub async fn start_writing(
        &mut self,
        intent: String,
        ctx: &WritingContext,
        event_tx: mpsc::UnboundedSender<PipelineEvent>,
        cancel: watch::Receiver<bool>,
    ) -> Result<(String, Id, Option<Provenance>), PipelineError> {
        let session_id = Id::new();
        let seed = rand_seed();

        info!(target: "app-pipeline", "流水线启动: session={session_id}");

        // 推送事件
        let _ = event_tx.send(PipelineEvent::Started {
            session_id: session_id.to_string(),
        });
        self.state = PipelineState::Directing;
        let _ = event_tx.send(PipelineEvent::StateChanged {
            state: self.state.clone(),
        });

        // ─── 阶段 1：导演 Agent（流式）──────────────────────────────────
        let _ = event_tx.send(PipelineEvent::DirectorStarted);

        // 前置校验：没有可用角色卡时，导演无法分配子 Agent，提前返回友好错误
        // （避免导演陷入"搜不到角色 → 输出空 Plan → drift recovery 死循环"）
        if ctx.characters.is_empty() {
            let msg = "没有可用的角色卡。请先导入一张角色卡（角色详情 → 导入），导演才能分配子 Agent 并规划本场戏。";
            let _ = event_tx.send(PipelineEvent::Error { message: msg.into() });
            return Err(self.abort_with(&event_tx, PipelineError::InvalidState(msg.into())));
        }

        let director_config = make_director_config();
        let mut director_registry = ToolRegistry::new();
        register_director_tools(&mut director_registry);

        // 构造导演的用户消息（含蓝灯常驻世界书条目，按 depth 排序）
        let director_user_msg = build_director_user_msg(&intent, ctx);

        // 流式：导演的输出 token 实时转成 DirectorProgress 事件
        let (director_prog_tx, mut director_prog_rx) = mpsc::unbounded_channel::<String>();
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(delta) = director_prog_rx.recv().await {
                let _ = event_tx_clone.send(PipelineEvent::DirectorProgress { delta });
            }
        });

        let director_resp = match self
            .runtime
            .run_tool_loop_streaming(
                &director_config,
                director_user_msg,
                &director_registry,
                cancel.clone(),
                director_prog_tx,
                // 完成探测：导演 content 里若已含合法 Plan JSON，立即终止（避免 drift recovery 死循环）
                Some(&|content: &str| {
                    let fake_resp = storyforge_domain::llm::ChatResponse {
                        content: content.to_string(),
                        tool_calls: vec![],
                        finish_reason: None,
                        usage: None,
                    };
                    parse_plan_from_response(&fake_resp).is_ok()
                }),
            )
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                error!(target: "app-pipeline", "导演失败: {e}");
                return Err(self.abort_with(&event_tx, PipelineError::Agent(e)));
            }
        };

        // 解析 Plan
        let plan = match parse_plan_from_response(&director_resp) {
            Ok(plan) => plan,
            Err(e) => return Err(self.abort_with(&event_tx, e)),
        };

        let _ = event_tx.send(PipelineEvent::DirectorDone {
            scene_brief: plan.scene_brief.clone(),
            subagent_count: plan.subagent_tasks.len(),
        });

        info!(target: "app-pipeline", "导演完成: {} 个子任务", plan.subagent_tasks.len());

        // ─── 阶段 2：子 Agent 并行 ───────────────────────────────────────
        self.state = PipelineState::Delegating;
        let _ = event_tx.send(PipelineEvent::StateChanged {
            state: self.state.clone(),
        });

        // 推送子 Agent 开始事件
        for (i, task) in plan.subagent_tasks.iter().enumerate() {
            let _ = event_tx.send(PipelineEvent::SubagentStarted {
                character_id: task.character_id.clone(),
                index: i,
                total: plan.subagent_tasks.len(),
            });
        }

        let subagent_results = spawn_subagents(
            plan.subagent_tasks.clone(),
            self.runtime.clone(),
            &director_config,
            SUBAGENT_SYSTEM_PROMPT_TEMPLATE,
            cancel.clone(),
        )
        .await;

        // 处理子 Agent 结果
        let mut performances = Vec::new();
        for (i, result) in subagent_results.into_iter().enumerate() {
            match result {
                Ok(perf) => {
                    let _ = event_tx.send(PipelineEvent::SubagentDone {
                        character_id: perf.character_id.clone(),
                        index: i,
                        full_text: perf.full_text.clone(),
                    });
                    performances.push(perf);
                }
                Err(e) => {
                    let char_id = plan.subagent_tasks[i].character_id.clone();
                    let _ = event_tx.send(PipelineEvent::SubagentCancelled {
                        character_id: char_id.clone(),
                        index: i,
                    });
                    error!(target: "app-pipeline", "子 Agent {char_id} 失败: {e}");
                    // 不中断，继续处理其他子 Agent
                }
            }
        }

        info!(target: "app-pipeline", "子 Agent 完成: {}/{}",
            performances.len(), plan.subagent_tasks.len());

        // 全部子 Agent 失败时，无法产出有效成文，提前中止
        if performances.is_empty() && !plan.subagent_tasks.is_empty() {
            let msg = "所有子 Agent 均失败，无法生成成文";
            error!(target: "app-pipeline", "{msg}");
            let _ = event_tx.send(PipelineEvent::Error { message: msg.into() });
            return Err(self.abort_with(&event_tx, PipelineError::InvalidState(msg.into())));
        }

        // ─── 阶段 3：编剧 Agent ─────────────────────────────────────────
        // 子 Agent 完成后、编剧开始前，检查取消
        if *cancel.borrow() {
            info!(target: "app-pipeline", "子 Agent 完成后取消，跳过编剧");
            return Err(self.abort_with(&event_tx, PipelineError::Cancelled));
        }

        self.state = PipelineState::Editing;
        let _ = event_tx.send(PipelineEvent::StateChanged {
            state: self.state.clone(),
        });
        let _ = event_tx.send(PipelineEvent::EditorStarted);

        let editor_config = make_editor_config();

        // 构造编剧的用户消息（子 Agent 产出）
        let performances_text: String = performances
            .iter()
            .map(|p| format!("### {}\n{}", p.character_id, p.full_text))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let editor_user_msg = format!(
            "场景：{}\n\n子 Agent 表演：\n\n{}\n\n请合并成连贯成文。",
            plan.scene_brief, performances_text
        );

        // 流式：编剧的输出 token 实时转成 EditorProgress 事件
        let (editor_prog_tx, mut editor_prog_rx) = mpsc::unbounded_channel::<String>();
        let event_tx_clone2 = event_tx.clone();
        tokio::spawn(async move {
            while let Some(delta) = editor_prog_rx.recv().await {
                let _ = event_tx_clone2.send(PipelineEvent::EditorProgress { delta });
            }
        });

        let editor_resp = match self
            .runtime
            .run_tool_loop_streaming(
                &editor_config,
                editor_user_msg,
                &ToolRegistry::new(), // 编剧无工具，直接输出
                cancel.clone(),
                editor_prog_tx,
                None, // 编剧无完成探测（无工具，直接返回）
            )
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                error!(target: "app-pipeline", "编剧失败: {e}");
                return Err(self.abort_with(&event_tx, PipelineError::Agent(e)));
            }
        };

        let final_text = editor_resp.content;

        let _ = event_tx.send(PipelineEvent::DraftReady {
            text: final_text.clone(),
        });

        info!(target: "app-pipeline", "编剧完成: {} 字", final_text.len());

        // ─── 阶段 4：写入对话树 ─────────────────────────────────────────
        self.state = PipelineState::Review;

        let provenance = build_provenance(
            session_id.clone(),
            Some(plan.clone()),
            &performances,
            None, // profile_id
            seed,
            None, // last_hint（首次写作无 hint）
        );

        // 写入对话树
        let node_id = match self
            .conv_store
            .append_ai_draft(&ctx.conversation_id, final_text.clone(), Some(provenance.clone()))
        {
            Ok(id) => id,
            Err(e) => return Err(self.abort_with(&event_tx, PipelineError::Conversation(e))),
        };

        self.state = PipelineState::Committed;
        let _ = event_tx.send(PipelineEvent::StateChanged {
            state: self.state.clone(),
        });

        // 保存会话
        self.session = Some(WritingSession {
            id: session_id.clone(),
            intent,
            state: self.state.clone(),
            plan: Some(plan),
            subagent_results: performances,
            draft: Some(Draft {
                text: final_text.clone(),
                attribution: vec![],
            }),
            seed,
        });

        info!(target: "app-pipeline", "流水线完成: session={session_id}");

        Ok((final_text, node_id, Some(provenance)))
    }

    /// 整体重 roll（重新跑完整流水线）
    pub async fn regenerate_all(
        &mut self,
        intent: String,
        ctx: &WritingContext,
        event_tx: mpsc::UnboundedSender<PipelineEvent>,
        cancel: watch::Receiver<bool>,
    ) -> Result<(String, Id, Option<Provenance>), PipelineError> {
        self.state = PipelineState::Idle;
        self.start_writing(intent, ctx, event_tx, cancel).await
    }

    /// 重 roll：整体 / 只重编剧 / 只重某子 Agent，可附带 hint（对应设计 §3.7.3）
    ///
    /// `req.targets` 决定重跑粒度：
    /// - 空 或 含 Director → 整体重 roll（重新导演+子+编剧）
    /// - 仅 Editor → 只重编剧（复用旧子产出）
    /// - 仅 Subagent(id) → 只重该子 Agent（复用其他子产出 + 旧 Plan）
    ///
    /// `req.hint` 会被注入到被重跑的 Agent（见 inject_hint_*）。
    /// 产出的新 variant 会加到 `req.node_id` 同一个 node（分支，不删旧版）。
    pub async fn regenerate(
        &mut self,
        req: RegenerateRequest,
        ctx: &WritingContext,
        event_tx: mpsc::UnboundedSender<PipelineEvent>,
        cancel: watch::Receiver<bool>,
    ) -> Result<(String, Provenance), PipelineError> {
        let hint = req.hint.clone();
        let session_id = Id::new();
        let seed = req.seed.unwrap_or_else(rand_seed);

        info!(target: "app-pipeline", "重 roll 启动: session={session_id}, targets={:?}, hint={}",
            req.targets, hint.as_deref().unwrap_or("(无)"));

        let _ = event_tx.send(PipelineEvent::Started {
            session_id: session_id.to_string(),
        });

        // 先校验合法性（复用 app-conversation 的约束检查）
        self.conv_store
            .validate_partial_roll(&req.conversation_id, &req.node_id, &req.targets)
            .map_err(|e| self.abort_with(&event_tx, PipelineError::Regenerate(e.to_string())))?;

        // 读旧 variant 的 Provenance
        let provenance_old = {
            let conv = self
                .conv_store
                .get(&req.conversation_id)
                .ok_or_else(|| PipelineError::Regenerate("对话不存在".into()))?;
            let node = conv
                .find_node(&req.node_id)
                .ok_or_else(|| PipelineError::Regenerate("节点不存在".into()))?;
            node.active()
                .and_then(|v| v.provenance.clone())
                .ok_or_else(|| PipelineError::Regenerate("旧 variant 无溯源信息".into()))?
        };

        // 判断重 roll 粒度
        let rerun_director = req
            .targets
            .iter()
            .any(|t| matches!(t, PartialRollTarget::Director));
        let rerun_editor_only = !req.targets.is_empty()
            && req
                .targets
                .iter()
                .all(|t| matches!(t, PartialRollTarget::Editor));
        let rerun_subagent = req.targets.iter().find_map(|t| {
            if let PartialRollTarget::Subagent(id) = t {
                Some(id.clone())
            } else {
                None
            }
        });

        // ─── 路径 A：整体重 roll（含 Director 或 targets 为空）──────────────
        if rerun_director || req.targets.is_empty() {
            self.state = PipelineState::Directing;
            let _ = event_tx.send(PipelineEvent::StateChanged {
                state: self.state.clone(),
            });
            let _ = event_tx.send(PipelineEvent::DirectorStarted);

            // 前置校验：没有可用角色卡时提前返回友好错误（同 start_writing）
            if ctx.characters.is_empty() {
                let msg = "没有可用的角色卡。请先导入一张角色卡再重 roll。";
                let _ = event_tx.send(PipelineEvent::Error { message: msg.into() });
                return Err(self.abort_with(&event_tx, PipelineError::InvalidState(msg.into())));
            }

            let director_config = make_director_config();
            let mut director_registry = ToolRegistry::new();
            register_director_tools(&mut director_registry);

            // 导演 user 消息：拼接旧 Plan 的场景 + hint（含蓝灯常驻条目）
            let intent_text = provenance_old
                .plan
                .as_ref()
                .map(|p| p.scene_brief.clone())
                .unwrap_or_else(|| "重新创作".into());
            let mut director_user_msg = build_director_user_msg(&intent_text, ctx);
            if let Some(h) = &hint {
                director_user_msg =
                    inject_hint_into_editor(&director_user_msg, h);
            }

            // 流式：导演输出实时推 DirectorProgress
            let (director_prog_tx, mut director_prog_rx) = mpsc::unbounded_channel::<String>();
            let event_tx_clone = event_tx.clone();
            tokio::spawn(async move {
                while let Some(delta) = director_prog_rx.recv().await {
                    let _ = event_tx_clone.send(PipelineEvent::DirectorProgress { delta });
                }
            });

            let director_resp = match self
                .runtime
                .run_tool_loop_streaming(
                    &director_config,
                    director_user_msg,
                    &director_registry,
                    cancel.clone(),
                    director_prog_tx,
                    Some(&|content: &str| {
                        let fake_resp = storyforge_domain::llm::ChatResponse {
                            content: content.to_string(),
                            tool_calls: vec![],
                            finish_reason: None,
                            usage: None,
                        };
                        parse_plan_from_response(&fake_resp).is_ok()
                    }),
                )
                .await
            {
                Ok(resp) => resp,
                Err(e) => return Err(self.abort_with(&event_tx, PipelineError::Agent(e))),
            };

            let plan = match parse_plan_from_response(&director_resp) {
                Ok(plan) => plan,
                Err(e) => return Err(self.abort_with(&event_tx, e)),
            };
            let _ = event_tx.send(PipelineEvent::DirectorDone {
                scene_brief: plan.scene_brief.clone(),
                subagent_count: plan.subagent_tasks.len(),
            });

            // 子 Agent
            self.state = PipelineState::Delegating;
            let _ = event_tx.send(PipelineEvent::StateChanged {
                state: self.state.clone(),
            });
            for (i, task) in plan.subagent_tasks.iter().enumerate() {
                let _ = event_tx.send(PipelineEvent::SubagentStarted {
                    character_id: task.character_id.clone(),
                    index: i,
                    total: plan.subagent_tasks.len(),
                });
            }

            let subagent_results = spawn_subagents(
                plan.subagent_tasks.clone(),
                self.runtime.clone(),
                &director_config,
                SUBAGENT_SYSTEM_PROMPT_TEMPLATE,
                cancel.clone(),
            )
            .await;

            let mut performances = Vec::new();
            for (i, result) in subagent_results.into_iter().enumerate() {
                match result {
                    Ok(perf) => {
                    let _ = event_tx.send(PipelineEvent::SubagentDone {
                        character_id: perf.character_id.clone(),
                        index: i,
                        full_text: perf.full_text.clone(),
                    });
                        performances.push(perf);
                    }
                    Err(e) => {
                        let _ = event_tx.send(PipelineEvent::SubagentCancelled {
                            character_id: plan.subagent_tasks[i].character_id.clone(),
                            index: i,
                        });
                        error!(target: "app-pipeline", "重 roll 子 Agent 失败: {e}");
                    }
                }
            }

            // 全部子 Agent 失败时，无法产出有效成文，提前中止
            if performances.is_empty() && !plan.subagent_tasks.is_empty() {
                let msg = "所有子 Agent 均失败，无法生成成文";
                error!(target: "app-pipeline", "{msg}");
                let _ = event_tx.send(PipelineEvent::Error { message: msg.into() });
                return Err(self.abort_with(&event_tx, PipelineError::InvalidState(msg.into())));
            }

            // 编剧（注入 hint）
            let (final_text, provenance) = self
                .run_editor_and_commit(
                    &plan,
                    &performances,
                    &session_id,
                    seed,
                    hint.as_deref(),
                    &req,
                    event_tx,
                    cancel,
                )
                .await?;
            return Ok((final_text, provenance));
        }

        // ─── 路径 B：只重编剧 ──────────────────────────────────────────────
        if rerun_editor_only {
            // 把旧子产出快照还原为 Performance 列表
            let performances: Vec<storyforge_domain::agent::Performance> = provenance_old
                .subagent_results
                .iter()
                .map(|s| storyforge_domain::agent::Performance {
                    character_id: s.character_id.clone(),
                    narrative: String::new(),
                    dialogue: String::new(),
                    inner_thoughts: String::new(),
                    full_text: s.full_text.clone(),
                })
                .collect();

            let plan = provenance_old
                .plan
                .clone()
                .ok_or_else(|| PipelineError::Regenerate("旧 Provenance 无 Plan".into()))?;

            self.state = PipelineState::Editing;
            let _ = event_tx.send(PipelineEvent::StateChanged {
                state: self.state.clone(),
            });
            let _ = event_tx.send(PipelineEvent::EditorStarted);

            let (final_text, provenance) = self
                .run_editor_and_commit(
                    &plan,
                    &performances,
                    &session_id,
                    seed,
                    hint.as_deref(),
                    &req,
                    event_tx,
                    cancel,
                )
                .await?;
            return Ok((final_text, provenance));
        }

        // ─── 路径 C：只重某子 Agent ────────────────────────────────────────
        if let Some(target_id) = rerun_subagent {
            let plan = provenance_old
                .plan
                .clone()
                .ok_or_else(|| PipelineError::Regenerate("旧 Provenance 无 Plan".into()))?;

            // 复用旧子产出，仅替换目标角色
            let mut performances: Vec<storyforge_domain::agent::Performance> = Vec::new();
            // 找到目标角色在 plan 里的 task
            let target_task = plan
                .subagent_tasks
                .iter()
                .find(|t| t.character_id == target_id)
                .ok_or_else(|| {
                    PipelineError::Regenerate(format!(
                        "目标子 Agent '{target_id}' 不在旧 Plan 中"
                    ))
                })?
                .clone();

            self.state = PipelineState::Delegating;
            let _ = event_tx.send(PipelineEvent::StateChanged {
                state: self.state.clone(),
            });
            let _ = event_tx.send(PipelineEvent::SubagentStarted {
                character_id: target_id.clone(),
                index: 0,
                total: 1,
            });

            // 重跑该子 Agent（单任务，注入 hint 到 system prompt）
            let director_config = make_director_config();
            let new_perf = {
                let mut sys = format!(
                    "{}\n\n你是角色 {}。\n\n{}\n\n{}",
                    SUBAGENT_SYSTEM_PROMPT_TEMPLATE,
                    target_task.character_id,
                    format_subagent_context(&target_task.context_package),
                    target_task.brief,
                );
                if let Some(h) = &hint {
                    sys = inject_hint_into_subagent(&sys, h);
                }
                let config = AgentConfig {
                    role: AgentRole::Subagent(target_id.clone()),
                    system_prompt: sys,
                    max_tool_rounds: 10,
                    model: director_config.model.clone(),
                    tools: vec![],
                };
                // M1 子 Agent 无工具（纯表演）
                let registry = ToolRegistry::new();

                match self
                    .runtime
                    .run_tool_loop(&config, target_task.context_package.task.clone(), &registry, cancel.clone())
                    .await
                {
                    Ok(resp) => Ok(storyforge_domain::agent::Performance {
                        character_id: target_id.clone(),
                        narrative: String::new(),
                        dialogue: String::new(),
                        inner_thoughts: String::new(),
                        full_text: resp.content,
                    }),
                    Err(e) => Err(e),
                }
            };

            let new_perf = match new_perf {
                Ok(perf) => {
                    let _ = event_tx.send(PipelineEvent::SubagentDone {
                        character_id: perf.character_id.clone(),
                        index: 0,
                        full_text: perf.full_text.clone(),
                    });
                    perf
                }
                Err(e) => {
                    let _ = event_tx.send(PipelineEvent::SubagentCancelled {
                        character_id: target_id.clone(),
                        index: 0,
                    });
                    return Err(self.abort_with(&event_tx, PipelineError::Agent(e)));
                }
            };

            // 按原顺序重建 performances，仅替换目标子 Agent
            for snap in &provenance_old.subagent_results {
                if snap.character_id == target_id {
                    performances.push(new_perf.clone());
                } else {
                    performances.push(storyforge_domain::agent::Performance {
                        character_id: snap.character_id.clone(),
                        narrative: String::new(),
                        dialogue: String::new(),
                        inner_thoughts: String::new(),
                        full_text: snap.full_text.clone(),
                    });
                }
            }

            self.state = PipelineState::Editing;
            let _ = event_tx.send(PipelineEvent::StateChanged {
                state: self.state.clone(),
            });
            let _ = event_tx.send(PipelineEvent::EditorStarted);

            let (final_text, provenance) = self
                .run_editor_and_commit(
                    &plan,
                    &performances,
                    &session_id,
                    seed,
                    hint.as_deref(),
                    &req,
                    event_tx,
                    cancel,
                )
                .await?;
            return Ok((final_text, provenance));
        }

        // 其他不支持的 target 组合
        Err(PipelineError::Regenerate(format!(
            "不支持的重 roll 目标组合: {:?}",
            req.targets
        )))
    }

    /// 内部：跑编剧 + 写入对话树（新 variant），返回 (成文, Provenance)
    ///
    /// 被 start_writing 和 regenerate 的各路径复用。`hint` 注入到编剧 user 消息。
    async fn run_editor_and_commit(
        &mut self,
        plan: &Plan,
        performances: &[storyforge_domain::agent::Performance],
        session_id: &Id,
        seed: u64,
        hint: Option<&str>,
        req: &RegenerateRequest,
        event_tx: mpsc::UnboundedSender<PipelineEvent>,
        cancel: watch::Receiver<bool>,
    ) -> Result<(String, Provenance), PipelineError> {
        // 编剧开始前，检查取消
        if *cancel.borrow() {
            info!(target: "app-pipeline", "编剧开始前取消");
            return Err(self.abort_with(&event_tx, PipelineError::Cancelled));
        }

        self.state = PipelineState::Editing;
        let _ = event_tx.send(PipelineEvent::EditorStarted);

        let editor_config = make_editor_config();

        let performances_text: String = performances
            .iter()
            .map(|p| format!("### {}\n{}", p.character_id, p.full_text))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let mut editor_user_msg = format!(
            "场景：{}\n\n子 Agent 表演：\n\n{}\n\n请合并成连贯成文。",
            plan.scene_brief, performances_text
        );
        if let Some(h) = hint {
            editor_user_msg = inject_hint_into_editor(&editor_user_msg, h);
        }

        // 流式：编剧输出实时推 EditorProgress
        let (editor_prog_tx, mut editor_prog_rx) = mpsc::unbounded_channel::<String>();
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(delta) = editor_prog_rx.recv().await {
                let _ = event_tx_clone.send(PipelineEvent::EditorProgress { delta });
            }
        });

        let editor_resp = match self
            .runtime
            .run_tool_loop_streaming(
                &editor_config,
                editor_user_msg,
                &ToolRegistry::new(),
                cancel,
                editor_prog_tx,
                None,
            )
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                error!(target: "app-pipeline", "编剧失败: {e}");
                return Err(self.abort_with(&event_tx, PipelineError::Agent(e)));
            }
        };

        let final_text = editor_resp.content;
        let _ = event_tx.send(PipelineEvent::DraftReady {
            text: final_text.clone(),
        });

        let provenance = build_provenance(
            session_id.clone(),
            Some(plan.clone()),
            performances,
            None,
            seed,
            hint.map(String::from),
        );

        // 写入对话树：作为同 node 的新 variant（分支，不删旧版）
        if let Err(e) = self.conv_store
            .add_variant(&req.conversation_id, &req.node_id, final_text.clone(), Some(provenance.clone()))
        {
            return Err(self.abort_with(&event_tx, PipelineError::Conversation(e)));
        }

        self.state = PipelineState::Review;
        let _ = event_tx.send(PipelineEvent::StateChanged {
            state: self.state.clone(),
        });

        info!(target: "app-pipeline", "重 roll 完成: {} 字", final_text.len());
        Ok((final_text, provenance))
    }
}

// ─── 辅助函数 ──────────────────────────────────────────────────────────────

/// 构造导演的用户消息：意图 + 可用角色 + 蓝灯常驻世界书（按 depth 排序）
///
/// 蓝灯常驻条目（LoreRoute::Constant/Both）自动注入导演上下文，
/// 按 depth 升序排列（depth 小的靠后=更受重视，对齐 ST 近因效应语义）。
fn build_director_user_msg(intent: &str, ctx: &WritingContext) -> String {
    let char_names = ctx
        .characters
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>()
        .join("、");

    let mut msg = format!("用户的写作意图：{intent}\n\n可用角色：{char_names}\n\n");

    // 蓝灯常驻条目注入（按 depth 升序，depth 相同按 order）
    if let Some(book) = &ctx.world_info {
        let mut constants: Vec<_> = book
            .entries
            .iter()
            .filter(|e| {
                e.route == storyforge_domain::world_info::LoreRoute::Constant
                    || e.route == storyforge_domain::world_info::LoreRoute::Both
            })
            .collect();
        // depth 小的排后面（更重要）；depth 相同 order 小的排后面
        constants.sort_by(|a, b| {
            b.depth
                .cmp(&a.depth)
                .then_with(|| b.order.cmp(&a.order))
        });

        if !constants.is_empty() {
            msg.push_str("【世界设定（常驻）】\n");
            for e in &constants {
                msg.push_str(&format!("- {}：{}\n", e.keys.join(", "), e.content));
            }
            msg.push_str("\n（以上常驻设定始终生效。绿灯条目可通过 search_world_info / search_vectors 工具检索。）\n\n");
        }
    }

    msg.push_str("请分析意图并输出 Plan。");
    msg
}

/// 构造导演 Agent 配置
fn make_director_config() -> AgentConfig {
    AgentConfig {
        role: AgentRole::Director,
        system_prompt: DIRECTOR_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: 15,
        model: "deepseek-chat".to_string(), // 默认模型，后续从连接配置读取
        tools: vec![],
    }
}

/// 构造编剧 Agent 配置
fn make_editor_config() -> AgentConfig {
    AgentConfig {
        role: AgentRole::Editor,
        system_prompt: EDITOR_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: 5,
        model: "deepseek-chat".to_string(),
        tools: vec![],
    }
}

/// 从导演响应中解析 Plan
///
/// 导演可能通过 emit_plan 工具调用输出 Plan，也可能直接在 content 中输出 JSON。
/// 解析顺序（层层兜底，应对真实模型不可控的输出风格）：
/// 1. tool_calls 中的 emit_plan 工具调用
/// 2. 整个 content 是合法 JSON
/// 3. ```json ... ``` 代码块
/// 4. ``` ... ``` 代码块（无 json 标签）
/// 5. 大括号提取：从 content 中贪心抓最大的 {...} 块
fn parse_plan_from_response(resp: &storyforge_domain::llm::ChatResponse) -> Result<Plan, PipelineError> {
    // ① tool_calls 中的 emit_plan
    for tc in &resp.tool_calls {
        if tc.function.name == "emit_plan" {
            let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                .map_err(|e| PipelineError::PlanParse(format!("emit_plan 参数解析失败: {e}")))?;
            return parse_plan_json(&args);
        }
    }

    let content = resp.content.trim();
    if !content.is_empty() {
        // ② 整个 content 是 JSON
        if let Ok(args) = serde_json::from_str::<serde_json::Value>(content) {
            if let Ok(plan) = parse_plan_json(&args) {
                return Ok(plan);
            }
        }

        // ③ ```json ... ``` 代码块
        if let Some(plan) = try_extract_codeblock(content, "json") {
            return Ok(plan);
        }

        // ④ ``` ... ``` 代码块（无语言标签）
        if let Some(plan) = try_extract_codeblock(content, "") {
            return Ok(plan);
        }

        // ⑤ 大括号提取：贪心抓最大的 {...} 块
        if let Some(plan) = try_extract_braces(content) {
            return Ok(plan);
        }
    }

    Err(PipelineError::PlanParse(format!(
        "导演响应中未找到有效 Plan。导演原始输出（前 500 字）：{}",
        resp.content.chars().take(500).collect::<String>()
    )))
}

/// 从 ```lang ... ``` 代码块中提取并解析 Plan
fn try_extract_codeblock(content: &str, lang: &str) -> Option<Plan> {
    let pattern = if lang.is_empty() {
        r"```([\s\S]*?)```"
    } else {
        r"```[a-zA-Z]*([\s\S]*?)```"
    };
    let re = regress::Regex::new(pattern).ok()?;
    for m in re.find_iter(content) {
        if let Some(json_str) = m.group(1).and_then(|g| content.get(g)) {
            let trimmed = json_str.trim();
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Ok(plan) = parse_plan_json(&args) {
                    return Some(plan);
                }
            }
        }
    }
    None
}

/// 从 content 中提取第一个**括号配平**的 {...} 块（手写，不依赖正则）
///
/// 从第一个 `{` 开始，计数 `{` 和 `}`（考虑字符串内的转义），到配平为止。
/// 这样能正确处理中文、嵌套对象、代码块外的文字。
/// 比正则贪心匹配更可靠（regress 对多字节字符的 range 可能有坑）。
fn try_extract_braces(content: &str) -> Option<Plan> {
    // 找所有候选：从每个 `{` 开始尝试配平
    let bytes = content.as_bytes();
    let mut start_idx = 0;
    while start_idx < content.len() {
        // 找下一个 `{`
        let rel = content[start_idx..].find('{')?;
        let brace_start = start_idx + rel;
        // 从这个 `{` 开始配平
        if let Some(end) = match_braces(content, brace_start) {
            let candidate = &content[brace_start..=end];
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(candidate) {
                if let Ok(plan) = parse_plan_json(&args) {
                    return Some(plan);
                }
            }
        }
        start_idx = brace_start + 1;
        let _ = bytes; // 避免未用警告
    }
    None
}

/// 从 pos 位置的 `{` 开始，找配平的 `}` 位置（处理字符串内的引号转义）
///
/// 返回 `}` 的 byte index。如果中途括号不匹配（如未闭合），返回 None。
fn match_braces(content: &str, pos: usize) -> Option<usize> {
    let chars: Vec<char> = content[pos..].chars().collect();
    if chars.is_empty() || chars[0] != '{' {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut byte_offset = pos;

    for ch in chars {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(byte_offset);
                    }
                }
                _ => {}
            }
        }
        byte_offset += ch.len_utf8();
    }
    None // 未闭合
}

/// 解析 Plan JSON（宽松：允许 subagent_tasks 缺失或为空）
fn parse_plan_json(v: &serde_json::Value) -> Result<Plan, PipelineError> {
    // 必须包含 scene_brief 或 subagent_tasks 之一
    let has_scene = v.get("scene_brief").and_then(|v| v.as_str()).is_some();
    let tasks_arr = v.get("subagent_tasks").and_then(|v| v.as_array());
    if !has_scene && tasks_arr.is_none() {
        return Err(PipelineError::PlanParse(
            "JSON 既无 scene_brief 也无 subagent_tasks".into(),
        ));
    }

    let scene_brief = v
        .get("scene_brief")
        .and_then(|v| v.as_str())
        .unwrap_or("未命名场景")
        .to_string();

    let empty_tasks = vec![];
    let tasks = tasks_arr.unwrap_or(&empty_tasks);

    let subagent_tasks: Vec<SubagentTask> = tasks
        .iter()
        .map(|t| {
            let character_id = t
                .get("character_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let brief = t
                .get("brief")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // 解析 context_package（如果有）
            let context_package = if let Some(pkg) = t.get("context_package") {
                parse_context_package(pkg)
            } else {
                ContextPackage {
                    character_brief: String::new(),
                    scene_brief: scene_brief.clone(),
                    relevant_lore: vec![],
                    constant_lore: vec![],
                    recent_window: vec![],
                    task: brief.clone(),
                }
            };

            SubagentTask {
                character_id,
                brief,
                context_package,
            }
        })
        .collect();

    Ok(Plan {
        scene_brief,
        subagent_tasks,
    })
}

/// 解析 ContextPackage JSON
fn parse_context_package(v: &serde_json::Value) -> ContextPackage {
    ContextPackage {
        character_brief: v
            .get("character_brief")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        scene_brief: v
            .get("scene_brief")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        relevant_lore: v
            .get("relevant_lore")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|e| LoreEntryLight {
                        keys: e
                            .get("keys")
                            .and_then(|v| v.as_array())
                            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                            .unwrap_or_default(),
                        content: e
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        constant_lore: vec![],
        recent_window: v
            .get("recent_window")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        task: v
            .get("task")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// 生成随机种子
fn rand_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

/// 格式化 ContextPackage 为子 Agent 的上下文文本（重 roll 子 Agent 时用）
fn format_subagent_context(pkg: &ContextPackage) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use storyforge_infra_llm::mock_client::MockLlmClient;
    use storyforge_domain::character::Character;
    use storyforge_domain::Source;

    /// 构造最小 mock 角色卡（满足 characters 非空校验）
    fn mock_character(name: &str) -> Arc<Character> {
        Arc::new(Character {
            id: Id::from_str(name),
            name: name.to_string(),
            description: format!("{name} 的描述"),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: "test".into(),
            character_version: "1.0".into(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::json!({}),
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::json!({}),
        })
    }

    /// 集成测试：用 MockLlmClient 跑完整 Director → Subagent → Editor 闭环
    #[tokio::test]
    async fn test_full_pipeline_with_mock() {
        // 构造 mock LLM client
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let conv_dir = std::env::temp_dir().join(format!(
            "storyforge_test_pipeline_{}",
            uuid::Uuid::new_v4()
        ));
        let conv_store = Arc::new(ConversationStore::new(conv_dir.clone()));

        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
        });

        let mut orchestrator = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx);

        let ctx = WritingContext {
            characters: vec![mock_character("Seraphina")],
            world_info: None,
            conversation_id: conv_store.create(None).id,
        };

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        // 注意：_cancel_tx 必须保活到 start_writing 结束，否则 watch sender drop 后
        // 子 Agent 的 cancel.wait_for() 会立即 ready，误触发 Cancelled。

        // 运行流水线
        let result = orchestrator
            .start_writing("写一场戏".into(), &ctx, event_tx, cancel_rx)
            .await;

        assert!(result.is_ok(), "流水线应成功: {:?}", result.err());
        let (text, _node_id, provenance) = result.unwrap();

        // 验证成文
        assert!(!text.is_empty(), "成文不应为空");
        assert!(text.contains("雨中告别") || text.contains("Seraphina") || text.len() > 50);

        // 验证 Provenance
        assert!(provenance.is_some(), "应有 Provenance");

        // 验证事件序列（drain 所有已发送的事件）
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }
        let event_types: Vec<String> = events
            .iter()
            .map(|e| match e {
                PipelineEvent::Started { .. } => "started".into(),
                PipelineEvent::DirectorStarted => "director_started".into(),
                PipelineEvent::DirectorDone { .. } => "director_done".into(),
                PipelineEvent::SubagentStarted { .. } => "subagent_started".into(),
                PipelineEvent::SubagentDone { .. } => "subagent_done".into(),
                PipelineEvent::EditorStarted => "editor_started".into(),
                PipelineEvent::DraftReady { .. } => "draft_ready".into(),
                PipelineEvent::StateChanged { .. } => "state_changed".into(),
                PipelineEvent::Committed { .. } => "committed".into(),
                _ => "other".into(),
            })
            .collect();

        // 验证关键事件都出现了
        assert!(event_types.contains(&"started".into()), "应有 started 事件");
        assert!(event_types.contains(&"director_started".into()), "应有 director_started");
        assert!(event_types.contains(&"director_done".into()), "应有 director_done");
        assert!(event_types.contains(&"editor_started".into()), "应有 editor_started");
        assert!(event_types.contains(&"draft_ready".into()), "应有 draft_ready");

        // 验证对话树中有 AI 成文
        let conv = conv_store.get(&ctx.conversation_id).unwrap();
        assert!(!conv.nodes.is_empty(), "对话树不应为空");
        let last_node = conv.nodes.last().unwrap();
        assert_eq!(
            last_node.active().unwrap().role,
            storyforge_domain::conversation::Role::Assistant
        );
        assert_eq!(
            last_node.active().unwrap().status,
            storyforge_domain::conversation::VariantStatus::Draft
        );

        // 清理
        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    /// 辅助：构造一个带 mock 的 orchestrator + 对话，并先跑一次 start_writing
    async fn setup_with_first_draft() -> (
        PipelineOrchestrator,
        Arc<ConversationStore>,
        Id, // conversation_id
        Id, // node_id
        PathBuf, // conv_dir（清理用）
    ) {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let conv_dir = std::env::temp_dir().join(format!(
            "storyforge_test_regen_{}",
            uuid::Uuid::new_v4()
        ));
        let conv_store = Arc::new(ConversationStore::new(conv_dir.clone()));
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
        });
        let mut orchestrator = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx);

        let conv = conv_store.create(None);
        let ctx = WritingContext {
            characters: vec![mock_character("Seraphina")],
            world_info: None,
            conversation_id: conv.id.clone(),
        };
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);  // sender 保活，避免误触发取消
        let (text, node_id, _prov) = orchestrator
            .start_writing("写一场戏".into(), &ctx, event_tx, cancel_rx)
            .await
            .expect("首次写作应成功");
        assert!(!text.is_empty());

        (orchestrator, conv_store, conv.id, node_id, conv_dir)
    }

    /// 重 roll：只重编剧（复用旧子产出），验证产生新 variant
    #[tokio::test]
    async fn test_regenerate_editor_only() {
        let (mut orchestrator, conv_store, conv_id, node_id, conv_dir) =
            setup_with_first_draft().await;

        // 重 roll 前：该 node 有 1 个 variant
        let conv_before = conv_store.get(&conv_id).unwrap();
        let node_before = conv_before.find_node(&node_id).unwrap();
        let variants_before = node_before.variants.len();

        let req = RegenerateRequest {
            conversation_id: conv_id.clone(),
            node_id: node_id.clone(),
            targets: vec![PartialRollTarget::Editor],
            hint: Some("节奏太快".into()),
            seed: None,
        };
        let ctx = WritingContext {
            characters: vec![mock_character("Seraphina")],
            world_info: None,
            conversation_id: conv_id.clone(),
        };
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);  // sender 保活，避免误触发取消
        let result = orchestrator.regenerate(req, &ctx, event_tx, cancel_rx).await;

        assert!(result.is_ok(), "重 roll 编剧应成功: {:?}", result.err());
        let (text, provenance) = result.unwrap();
        assert!(!text.is_empty());
        // hint 应记录进 Provenance
        assert_eq!(provenance.last_hint.as_deref(), Some("节奏太快"));

        // 重 roll 后：该 node 多 1 个 variant（分支）
        let conv_after = conv_store.get(&conv_id).unwrap();
        let node_after = conv_after.find_node(&node_id).unwrap();
        assert_eq!(node_after.variants.len(), variants_before + 1);
        // active 切到新 variant
        assert_eq!(node_after.active_variant, node_after.variants.len() - 1);

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    /// 重 roll：整体重 roll（含 hint）
    #[tokio::test]
    async fn test_regenerate_full_with_hint() {
        let (mut orchestrator, conv_store, conv_id, node_id, conv_dir) =
            setup_with_first_draft().await;

        let conv_before = conv_store.get(&conv_id).unwrap();
        let variants_before = conv_before.find_node(&node_id).unwrap().variants.len();

        let req = RegenerateRequest {
            conversation_id: conv_id.clone(),
            node_id: node_id.clone(),
            targets: vec![], // 空 = 整体重 roll
            hint: Some("角色 B 语气太冷".into()),
            seed: Some(42),
        };
        let ctx = WritingContext {
            characters: vec![mock_character("Seraphina")],
            world_info: None,
            conversation_id: conv_id.clone(),
        };
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);  // sender 保活，避免误触发取消
        let result = orchestrator.regenerate(req, &ctx, event_tx, cancel_rx).await;

        assert!(result.is_ok(), "整体重 roll 应成功: {:?}", result.err());
        let (_text, provenance) = result.unwrap();
        assert_eq!(provenance.last_hint.as_deref(), Some("角色 B 语气太冷"));
        assert_eq!(provenance.seed, 42);

        // 新 variant 增加
        let conv_after = conv_store.get(&conv_id).unwrap();
        let variants_after = conv_after.find_node(&node_id).unwrap().variants.len();
        assert_eq!(variants_after, variants_before + 1);

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    /// 重 roll：约束违反应被拒绝（只重导演却保留旧子产出）
    #[tokio::test]
    async fn test_regenerate_violation_rejected() {
        let (mut orchestrator, _conv_store, conv_id, node_id, conv_dir) =
            setup_with_first_draft().await;

        let req = RegenerateRequest {
            conversation_id: conv_id.clone(),
            node_id: node_id.clone(),
            targets: vec![PartialRollTarget::Director], // 只重导演
            hint: None,
            seed: None,
        };
        let ctx = WritingContext {
            characters: vec![mock_character("Seraphina")],
            world_info: None,
            conversation_id: conv_id.clone(),
        };
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);  // sender 保活，避免误触发取消
        let result = orchestrator.regenerate(req, &ctx, event_tx, cancel_rx).await;

        assert!(result.is_err(), "只重导演却留旧子产出应被拒绝");
        match result.unwrap_err() {
            PipelineError::Regenerate(msg) => {
                assert!(msg.contains("不匹配") || msg.contains("保留"), "错误信息应说明原因: {msg}");
            }
            other => panic!("应是 Regenerate 错误，实际: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    /// 重 roll：只重某子 Agent
    #[tokio::test]
    async fn test_regenerate_subagent_only() {
        let (mut orchestrator, conv_store, conv_id, node_id, conv_dir) =
            setup_with_first_draft().await;

        // 从首次产出的 Provenance 拿到子 Agent 角色名
        let conv = conv_store.get(&conv_id).unwrap();
        let node = conv.find_node(&node_id).unwrap();
        let prov = node.active().unwrap().provenance.as_ref().unwrap();
        let target_char = prov
            .subagent_results
            .first()
            .map(|s| s.character_id.clone())
            .expect("应有子 Agent 产出");
        let variants_before = node.variants.len();

        let req = RegenerateRequest {
            conversation_id: conv_id.clone(),
            node_id: node_id.clone(),
            targets: vec![PartialRollTarget::Subagent(target_char.clone())],
            hint: Some("语气太冷".into()),
            seed: None,
        };
        let ctx = WritingContext {
            characters: vec![mock_character("Seraphina")],
            world_info: None,
            conversation_id: conv_id.clone(),
        };
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);  // sender 保活，避免误触发取消
        let result = orchestrator.regenerate(req, &ctx, event_tx, cancel_rx).await;

        assert!(result.is_ok(), "重 roll 子 Agent 应成功: {:?}", result.err());
        let (_text, provenance) = result.unwrap();
        assert_eq!(provenance.last_hint.as_deref(), Some("语气太冷"));

        // 新 variant 增加
        let conv_after = conv_store.get(&conv_id).unwrap();
        let variants_after = conv_after.find_node(&node_id).unwrap().variants.len();
        assert_eq!(variants_after, variants_before + 1);

        let _ = std::fs::remove_dir_all(&conv_dir);
    }
}

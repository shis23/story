//! `BridgeService`：传输无关的桥接核心。
//!
//! 每个方法直接调用 `storyforge-app-service` 的命令本体（`*_for_host` / `accept_variant_async`），
//! 跑的是与 Tauri 命令同一份生产代码；本层只负责编排：
//! - 一次只跑一轮。上一轮成文后，后处理 + 自动接受在后台继续（"收尾"），
//!   下一轮（或开新档）先等收尾结束，与生产的活动 Turn 屏障一致。
//! - 多个 DSH 会话各自对应一个 Campaign；轮到哪个会话就把它设为活跃 Campaign。
//! - 自动接受（PoC 决策）：后处理落定后按变体接受；质量门有 Error 被拦时
//!   强制接受（Turn 标 Degraded），保证 DSH 里显示的正文与 Campaign 一致。
//! - 用量归属：同一时刻只有一个操作持锁，按锁内前后两次计数快照的差值
//!   把底层调用记到开档（角色识别）、本轮正文、收尾（后处理）上。
//! - 与 DSH 历史对齐：调用方可带上 DSH 历史里已完成的轮数，与存档里已采纳的
//!   轮数不一致时拒绝执行（已采纳的轮次不能重写，DSH 的重新生成 / 编辑历史会
//!   造成两边错位）。分叉只支持从最新一轮（与生产 `fork_campaign` 的约束一致）。
//! - 取消只作用于前台：还在排队等锁的写作请求直接放弃（不执行），正在成文的一轮
//!   中止。收尾不受 DSH 停止按钮影响——正文已经显示在 DSH 里，半途停下后处理会让
//!   这一轮无法接受，会话与存档从此错位。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{OwnedMutexGuard, mpsc, oneshot};
use tokio::task::JoinHandle;

use storyforge_app_service::error::TauriCommandError;
use storyforge_app_service::services::conversations::{
    collect_conversation_regex_scripts, node_display_content,
};
use storyforge_app_service::{
    AppState, StartWritingRequest, accept_variant_async, create_campaign_for_backend,
    extract_characters_for_host, fork_campaign_for_host, import_character_for_host,
    route_generation_for_host, set_active_campaign_for_host, start_writing_for_host,
};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::conversation::{Conversation, Role, VariantStatus};
use storyforge_domain::generation::{GenerationMode, GenerationRouteDecision};
use storyforge_domain::preset::RegexScript;

use crate::llm::{LlmBridgeState, UsageTotals};

pub mod card_shell;
pub mod mvu;

/// 本层的事件出口（stdio 外壳把它们变成通知）。
pub trait TurnObserver: Send + Sync {
    /// 管线事件，包括成文之后后台后处理的事件。
    fn pipeline_event(&self, campaign_id: &str, event: &PipelineEvent);
    /// 一轮彻底结束（后处理 + 自动接受）。
    fn settled(&self, report: &SettleReport);
}

#[derive(Debug, Clone, Serialize)]
pub struct CampaignCreated {
    pub campaign_id: String,
    pub conversation_id: String,
    pub card_id: String,
    pub card_name: String,
    pub instance_count: usize,
    pub extraction_status: String,
    /// 存进对话的开场白（卡的第一条问候）；没有则为空串。
    pub opening: String,
    /// 开场白跑过展示正则之后的文本（与桌面会话视图同口径）；没有开场白为空串。
    pub opening_display: String,
    /// 开场白所在节点（插件记进 DSH 回放数据，之后按它取展示文本）。
    pub opening_node_id: Option<String>,
    pub opening_node_index: Option<usize>,
    /// 开档期间的底层调用（角色识别）。
    pub usage: UsageTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct CampaignForked {
    pub campaign_id: String,
    pub conversation_id: String,
    pub source_campaign_id: String,
    /// 分叉点之前已采纳的轮数（新存档从这里继续）。
    pub turns: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnOutput {
    pub campaign_id: String,
    pub conversation_id: String,
    pub node_id: String,
    /// 节点在对话里的位置（分叉换了节点 id 时按它兜底）。
    pub node_index: Option<usize>,
    /// 质量门 / autofix 之后的最终正文（与对话里存的一致）。
    pub text: String,
    /// 正文跑过展示正则之后的文本（与桌面会话视图同口径）；取不到时同 `text`。
    pub display: String,
    pub generation_mode: Value,
    /// 成文之前的底层调用（导演、角色、执笔、编剧、autofix）。
    pub usage: UsageTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettleReport {
    pub campaign_id: String,
    pub node_id: String,
    /// 后处理的终态事件类型：`PostProcessDone` / `PostProcessFailed` /
    /// `PostProcessSkipped`；没收到终态事件为 None。
    pub postprocess: Option<String>,
    pub postprocess_detail: Option<String>,
    /// `accepted` / `accepted_degraded` / `failed`
    pub accept: String,
    pub accept_error: Option<String>,
    /// 成文之后的底层调用（摘要、后处理）。
    pub usage: UsageTotals,
}

/// `conversation.display` 的一条查询：DSH 一个 Turn 里记下的存档位置（插件写进回放数据）。
#[derive(Debug, Clone, Deserialize)]
pub struct DisplayRef {
    /// 调用方自己的键，结果按它返回。
    pub key: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub node_index: Option<usize>,
}

/// 一个助手节点当前的展示文本。
#[derive(Debug, Clone, Serialize)]
pub struct NodeDisplay {
    pub conversation_id: String,
    pub node_id: String,
    pub node_index: usize,
    /// 展示正则之后的文本（深度按节点在对话里的当前位置算，每轮都可能变）。
    pub display_content: String,
    /// 展示正则之前的原文。内联卡片界面要逐个对上卡正则在原文里的命中才自动挂载
    /// （桌面 `trustedInlineShellDocStarts` 同口径）。
    pub content: String,
    /// 回放数据记的是别的对话（分叉继承的轮次），按位置从会话当前存档的对话里取的。
    pub by_position: bool,
    /// 这条回复所在对话现有的节点数。只有开场白一个节点 = 还在开场态，开场界面可以
    /// 改写开场白（`campaign.apply_opening`）。
    pub node_count: usize,
}

/// 一次 `conversation.display` 最多查的节点数。
pub const MAX_DISPLAY_REFS: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct BridgeStatus {
    pub data_dir: String,
    pub active_campaign: Option<String>,
    /// 写作锁被占用（前台成文或后台收尾）。
    pub busy: bool,
    /// 正在前台成文的存档；收尾阶段为 None。
    pub writing: Option<String>,
    /// 排队等写作锁的存档（按到达顺序，可重复）。
    pub queued: Vec<String>,
    pub last_settle: Option<SettleReport>,
    /// `native` / `text_fallback`
    pub tool_mode: &'static str,
    /// sidecar 启动以来的底层调用累计。
    pub usage: UsageTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
    /// 给插件程序化处理的附带信息（JSON-RPC 错误的 `data`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl BridgeError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: None,
        }
    }

    fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl From<TauriCommandError> for BridgeError {
    fn from(error: TauriCommandError) -> Self {
        let code = match &error {
            TauriCommandError::Pipeline { .. } => "pipeline",
            TauriCommandError::Llm { .. } => "llm",
            TauriCommandError::Storage { .. } => "storage",
            TauriCommandError::Validation { .. } => "validation",
            TauriCommandError::NotFound { .. } => "not_found",
            TauriCommandError::Cancelled => "cancelled",
            TauriCommandError::Internal { .. } => "internal",
        };
        Self::new(code, error.to_string())
    }
}

pub struct BridgeService {
    app: Arc<AppState>,
    data_dir: PathBuf,
    llm: Arc<LlmBridgeState>,
    /// 串行化开档与写作；收尾任务持有它直到自动接受完成。
    turn_lock: Arc<tokio::sync::Mutex<()>>,
    last_settle: Arc<Mutex<Option<SettleReport>>>,
    settling: Mutex<Option<JoinHandle<()>>>,
    foreground: Arc<Mutex<Foreground>>,
}

/// 可被取消的前台写作：排队等锁的请求与正在成文的一轮。
#[derive(Default)]
struct Foreground {
    next_ticket: u64,
    /// 排队中的写作请求：(票号, 存档, 放弃信号)。
    waiting: Vec<(u64, String, oneshot::Sender<()>)>,
    /// 正在成文的一轮所属存档；收尾阶段为 None。
    writing: Option<String>,
}

/// 成文阶段结束（或提前返回）时清掉前台标记。
struct WritingPhase(Arc<Mutex<Foreground>>);

impl Drop for WritingPhase {
    fn drop(&mut self) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).writing = None;
    }
}

fn targets(scope: Option<&str>, campaign_id: &str) -> bool {
    scope.is_none_or(|scope| scope == campaign_id)
}

impl BridgeService {
    pub fn new(app: Arc<AppState>, data_dir: PathBuf, llm: Arc<LlmBridgeState>) -> Self {
        Self {
            app,
            data_dir,
            llm,
            turn_lock: Arc::new(tokio::sync::Mutex::new(())),
            last_settle: Arc::new(Mutex::new(None)),
            settling: Mutex::new(None),
            foreground: Arc::new(Mutex::new(Foreground::default())),
        }
    }

    pub fn status(&self) -> BridgeStatus {
        let (writing, queued) = {
            let foreground = self.foreground.lock().unwrap_or_else(|p| p.into_inner());
            let queued = foreground
                .waiting
                .iter()
                .map(|(_, campaign, _)| campaign.clone())
                .collect();
            (foreground.writing.clone(), queued)
        };
        BridgeStatus {
            data_dir: self.data_dir.display().to_string(),
            active_campaign: self.active_campaign().map(|id| id.to_string()),
            busy: self.turn_lock.try_lock().is_err(),
            writing,
            queued,
            last_settle: self
                .last_settle
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            tool_mode: tool_mode_name(self.llm.text_fallback()),
            usage: self.llm.usage(),
        }
    }

    /// 修改底层调用设置（目前只有工具协议），返回生效后的值。
    pub fn configure(&self, tool_mode: Option<&str>) -> Result<&'static str, BridgeError> {
        match tool_mode {
            None => {}
            Some("native") => self.llm.set_text_fallback(false),
            Some("text_fallback") => self.llm.set_text_fallback(true),
            Some(other) => {
                return Err(BridgeError::new(
                    "invalid_params",
                    format!("未知 tool_mode {other}（应为 native / text_fallback）"),
                ));
            }
        }
        Ok(tool_mode_name(self.llm.text_fallback()))
    }

    fn active_campaign(&self) -> Option<Id> {
        self.app
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 取消前台写作（DSH 停止按钮）：`campaign_id` 所属的排队请求直接放弃，
    /// 正在成文的那一轮中止；收尾不受影响。`campaign_id` 为 None 时不区分存档。
    /// 返回是否取消了什么。
    pub fn cancel(&self, campaign_id: Option<&str>) -> bool {
        let mut foreground = self.foreground.lock().unwrap_or_else(|p| p.into_inner());
        let (abandoned, kept): (Vec<_>, Vec<_>) = foreground
            .waiting
            .drain(..)
            .partition(|(_, campaign, _)| targets(campaign_id, campaign));
        foreground.waiting = kept;
        let mut cancelled = false;
        for (_, _, abandon) in abandoned {
            cancelled |= abandon.send(()).is_ok();
        }
        if foreground
            .writing
            .as_deref()
            .is_some_and(|campaign| targets(campaign_id, campaign))
        {
            cancelled |= self.cancel_current();
        }
        cancelled
    }

    /// 退出时用：连同收尾一起中止。
    pub fn cancel_all(&self) -> bool {
        let abandoned = self.cancel(None);
        self.cancel_current() || abandoned
    }

    fn cancel_current(&self) -> bool {
        let slot = self
            .app
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match slot.as_ref() {
            Some(handle) => handle.cancel_tx.send(true).is_ok(),
            None => false,
        }
    }

    /// 写作请求排队等锁；排队期间被取消则放弃，不执行。拿到锁后标记为前台成文。
    async fn acquire_turn(
        &self,
        campaign_id: &str,
    ) -> Result<(OwnedMutexGuard<()>, WritingPhase), BridgeError> {
        let (abandon_tx, mut abandon_rx) = oneshot::channel();
        let ticket = {
            let mut foreground = self.foreground.lock().unwrap_or_else(|p| p.into_inner());
            foreground.next_ticket += 1;
            let ticket = foreground.next_ticket;
            foreground
                .waiting
                .push((ticket, campaign_id.to_string(), abandon_tx));
            ticket
        };
        let abandoned = || BridgeError::new("cancelled", "已取消：这一轮还在排队，没有执行");
        let guard = tokio::select! {
            guard = self.acquire() => guard,
            _ = &mut abandon_rx => return Err(abandoned()),
        };
        let mut foreground = self.foreground.lock().unwrap_or_else(|p| p.into_inner());
        foreground
            .waiting
            .retain(|(waiting, _, _)| *waiting != ticket);
        // 拿锁与取消同时发生：以取消为准。
        if abandon_rx.try_recv().is_ok() {
            return Err(abandoned());
        }
        foreground.writing = Some(campaign_id.to_string());
        drop(foreground);
        Ok((guard, WritingPhase(self.foreground.clone())))
    }

    /// 等上一轮收尾结束后拿到串行锁。
    async fn acquire(&self) -> OwnedMutexGuard<()> {
        let guard = self.turn_lock.clone().lock_owned().await;
        // 收尾任务结束前一直持锁，拿到锁时它已结束；这里只回收句柄。
        let finished = self
            .settling
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(handle) = finished {
            let _ = handle.await;
        }
        guard
    }

    /// 导入卡 → 角色识别 → 开档（建对话、存开场白）→ 设为活跃。
    pub async fn create_campaign(
        &self,
        card_path: PathBuf,
        name: Option<String>,
    ) -> Result<CampaignCreated, BridgeError> {
        let _guard = self.acquire().await;
        let usage_before = self.llm.usage();
        let bytes = tokio::fs::read(&card_path).await.map_err(|error| {
            BridgeError::new(
                "card_unreadable",
                format!("读不到角色卡 {}: {error}", card_path.display()),
            )
        })?;
        let imported = import_character_for_host(&self.app, bytes)?;
        let card = extract_characters_for_host(&self.app, imported.id.clone(), None).await?;
        let name = name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| card.name.clone());
        let campaign = create_campaign_for_backend(
            self.app.storage(),
            self.app.conv_store.as_ref(),
            card.id.clone(),
            name,
            None,
        )?;
        set_active_campaign_for_host(&self.app, campaign.id.clone())?;
        let conversation_id = campaign
            .conversation_id
            .clone()
            .ok_or_else(|| BridgeError::new("internal", "新建的 Campaign 没有绑定对话"))?;
        let opening = self
            .app
            .conv_store
            .get(&Id::from_str(&conversation_id))
            .and_then(|conversation| {
                let index = conversation.nodes.iter().position(|node| {
                    node.active()
                        .is_some_and(|variant| variant.role == Role::Assistant)
                })?;
                let content = conversation.nodes[index].active()?.content.clone();
                let display = self
                    .node_display(&conversation, index)
                    .unwrap_or_else(|| content.clone());
                Some((
                    conversation.nodes[index].id.to_string(),
                    index,
                    content,
                    display,
                ))
            });
        let (opening_node_id, opening_node_index, opening, opening_display) = match opening {
            Some((node_id, index, content, display)) => {
                (Some(node_id), Some(index), content, display)
            }
            None => (None, None, String::new(), String::new()),
        };
        Ok(CampaignCreated {
            campaign_id: campaign.id,
            conversation_id,
            card_id: card.id,
            card_name: card.name,
            instance_count: campaign.instance_count,
            extraction_status: card.extraction_status,
            opening,
            opening_display,
            opening_node_id,
            opening_node_index,
            usage: self.llm.usage().since(&usage_before),
        })
    }

    /// 存档绑定的对话。
    fn campaign_conversation(&self, campaign_id: &Id) -> Result<Conversation, BridgeError> {
        let record = self
            .app
            .storage()
            .get_campaign(campaign_id)
            .map_err(|error| BridgeError::new("storage", error))?
            .ok_or_else(|| BridgeError::new("not_found", format!("找不到存档 {campaign_id}")))?;
        let conversation_id = record
            .campaign
            .conversation_id
            .ok_or_else(|| BridgeError::new("not_found", format!("存档 {campaign_id} 没有对话")))?;
        self.app.conv_store.get(&conversation_id).ok_or_else(|| {
            BridgeError::new("not_found", format!("找不到存档对话 {conversation_id}"))
        })
    }

    /// 从存档的最新一轮分叉出新存档（DSH 会话分叉时调用）。
    ///
    /// `history_turns` 是分叉点之前 DSH 历史里已完成的轮数；只有等于存档已采纳的
    /// 轮数（即从最新一轮分叉）才执行。
    pub async fn fork_campaign(
        &self,
        source_campaign_id: &str,
        history_turns: usize,
        name: Option<String>,
    ) -> Result<CampaignForked, BridgeError> {
        let _guard = self.acquire().await;
        let source = Id::from_str(source_campaign_id);
        let conversation = self.campaign_conversation(&source)?;
        let committed = committed_turns(&conversation);
        if history_turns != committed {
            return Err(BridgeError::new(
                "out_of_sync",
                format!(
                    "StoryForge 目前只能从最新一轮分叉：分叉点之前有 {history_turns} 轮，\
                     存档已有 {committed} 轮。请从原会话最后一条回复处分叉。"
                ),
            ));
        }
        let head = conversation
            .nodes
            .iter()
            .rev()
            .find(|node| {
                node.active()
                    .is_some_and(|variant| variant.status != VariantStatus::Discarded)
            })
            .map(|node| node.id.to_string())
            .ok_or_else(|| BridgeError::new("validation", "存档对话为空，无法分叉"))?;
        let name = match name.filter(|name| !name.trim().is_empty()) {
            Some(name) => name,
            None => {
                let record = self
                    .app
                    .storage()
                    .get_campaign(&source)
                    .map_err(|error| BridgeError::new("storage", error))?;
                format!(
                    "{}（分支）",
                    record.map(|r| r.campaign.name).unwrap_or_default()
                )
            }
        };
        let forked = fork_campaign_for_host(&self.app, source_campaign_id.to_string(), head, name)?;
        Ok(CampaignForked {
            conversation_id: forked
                .conversation_id
                .clone()
                .ok_or_else(|| BridgeError::new("internal", "分支存档没有绑定对话"))?,
            campaign_id: forked.id,
            source_campaign_id: source_campaign_id.to_string(),
            turns: committed,
        })
    }

    /// 跑一轮写作，成文即返回；后处理与自动接受在后台收尾。
    ///
    /// `generation_mode` 为 None 时走生产的自动路由（自动升到高成本档会报
    /// validation 错误，要求显式选择）；显式模式不再二次确认。
    /// `history_turns`：DSH 历史里本轮之前已完成的轮数，与存档不一致时报
    /// `out_of_sync` 并且不执行。
    pub async fn run_turn(
        &self,
        campaign_id: &str,
        intent: String,
        generation_mode: Option<GenerationMode>,
        history_turns: Option<usize>,
        observer: Arc<dyn TurnObserver>,
    ) -> Result<TurnOutput, BridgeError> {
        let (guard, writing_phase) = self.acquire_turn(campaign_id).await?;
        let usage_before = self.llm.usage();
        if intent.trim().is_empty() {
            return Err(BridgeError::new("validation", "本轮输入为空"));
        }
        if let Some(history_turns) = history_turns {
            let committed =
                committed_turns(&self.campaign_conversation(&Id::from_str(campaign_id))?);
            if history_turns != committed {
                return Err(out_of_sync(history_turns, committed));
            }
        }
        if self.active_campaign().as_ref().map(Id::as_str) != Some(campaign_id) {
            set_active_campaign_for_host(&self.app, campaign_id.to_string())?;
        }
        if generation_mode.is_none() {
            // 自动路由要升到高成本档时先回给插件问用户（start_writing 自己也会拒绝，
            // 但那是一句话的 validation 错误，插件没法据此弹选项）。
            let decision = route_generation_for_host(&self.app, &intent, None, None).await?;
            if decision.requires_cost_confirmation {
                return Err(confirm_generation_mode(&decision));
            }
        }

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let forward_observer = observer.clone();
        let forward_campaign = campaign_id.to_string();
        // 所有 event_tx 克隆释放（含后台后处理里的）= 本轮全部结束。
        let events = tokio::spawn(async move {
            let mut terminal: Option<(String, Option<String>)> = None;
            while let Some(event) = event_rx.recv().await {
                forward_observer.pipeline_event(&forward_campaign, &event);
                match &event {
                    PipelineEvent::PostProcessDone { .. } => {
                        terminal = Some(("PostProcessDone".into(), None));
                    }
                    PipelineEvent::PostProcessFailed { reason } => {
                        terminal = Some(("PostProcessFailed".into(), Some(reason.clone())));
                    }
                    PipelineEvent::PostProcessSkipped { reason } => {
                        terminal = Some(("PostProcessSkipped".into(), Some(reason.clone())));
                    }
                    _ => {}
                }
            }
            terminal
        });

        let result = start_writing_for_host(
            self.app.clone(),
            StartWritingRequest {
                intent,
                generation_mode,
                ..StartWritingRequest::default()
            },
            event_tx,
            None,
        )
        .await;
        // 成文结束：之后的收尾不再响应取消。
        drop(writing_phase);
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = events.await;
                return Err(error.into());
            }
        };
        let field = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    BridgeError::new("internal", format!("start_writing 结果缺少 {key}"))
                })
        };
        // 成文时刻的计数：之前记到本轮正文，之后记到收尾。
        let usage_at_text = self.llm.usage();
        let conversation_id = field("conversation_id")?;
        let node_id = field("node_id")?;
        let text = field("text")?;
        let placed = self
            .app
            .conv_store
            .get(&Id::from_str(&conversation_id))
            .and_then(|conversation| {
                let index = conversation
                    .nodes
                    .iter()
                    .position(|node| node.id.as_str() == node_id)?;
                Some((index, self.node_display(&conversation, index)))
            });
        let output = TurnOutput {
            campaign_id: campaign_id.to_string(),
            node_index: placed.as_ref().map(|(index, _)| *index),
            display: placed
                .and_then(|(_, display)| display)
                .unwrap_or_else(|| text.clone()),
            conversation_id,
            node_id,
            text,
            generation_mode: value.get("generation_mode").cloned().unwrap_or(Value::Null),
            usage: usage_at_text.since(&usage_before),
        };

        let app = self.app.clone();
        let llm = self.llm.clone();
        let last_settle = self.last_settle.clone();
        let settle_output = output.clone();
        let handle = tokio::spawn(async move {
            let _guard = guard;
            let terminal = events.await.ok().flatten();
            let mut report = settle(&app, &settle_output, terminal).await;
            report.usage = llm.usage().since(&usage_at_text);
            observer.settled(&report);
            *last_settle.lock().unwrap_or_else(|p| p.into_inner()) = Some(report);
        });
        *self.settling.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
        Ok(output)
    }

    /// 节点的展示文本（展示正则按该对话所属存档 / 角色卡收集）。
    fn node_display(&self, conversation: &Conversation, index: usize) -> Option<String> {
        let scripts = collect_conversation_regex_scripts(conversation, &self.app);
        node_display_content(conversation, index, &scripts)
    }

    /// 按 DSH 回放数据里记下的存档位置批量取助手节点的当前展示文本（只读，不占写作锁）。
    ///
    /// `campaign_id` 是 DSH 会话当前对应的存档。回放数据记的就是它的对话时按节点 id 取；
    /// 记的是别的对话（分叉继承的轮次）时按节点位置到当前对话里取——分叉复制前缀时顺序
    /// 与正文不变、只换节点 id，展示正则的深度也该按用户正在看的这条对话算。记下的来源
    /// 对话还在时核对两边正文一致，不一致不返回（宁缺勿错）。没有当前存档时按记下的
    /// 对话取。找不到或不是助手消息的返回 None。
    pub fn display_nodes(
        &self,
        refs: &[DisplayRef],
        campaign_id: Option<&str>,
    ) -> Result<BTreeMap<String, Option<NodeDisplay>>, BridgeError> {
        if refs.len() > MAX_DISPLAY_REFS {
            return Err(BridgeError::new(
                "invalid_params",
                format!("一次最多查 {MAX_DISPLAY_REFS} 个节点，收到 {}", refs.len()),
            ));
        }
        let home = campaign_id
            .and_then(|id| self.campaign_conversation(&Id::from_str(id)).ok())
            .map(|conversation| {
                let scripts = collect_conversation_regex_scripts(&conversation, &self.app);
                (conversation, scripts)
            });
        let mut recorded = HashMap::new();
        for id in refs.iter().filter_map(|r| r.conversation_id.as_deref()) {
            let is_home = home
                .as_ref()
                .is_some_and(|(conversation, _)| conversation.id.as_str() == id);
            if is_home || recorded.contains_key(id) {
                continue;
            }
            let loaded = self
                .app
                .conv_store
                .get(&Id::from_str(id))
                .map(|conversation| {
                    // 有当前存档时，记下的对话只用来核对正文，用不上展示正则。
                    let scripts = match home {
                        Some(_) => Vec::new(),
                        None => collect_conversation_regex_scripts(&conversation, &self.app),
                    };
                    (conversation, scripts)
                });
            recorded.insert(id, loaded);
        }
        Ok(refs
            .iter()
            .map(|display_ref| {
                let item = locate_display(display_ref, home.as_ref(), &recorded);
                (display_ref.key.clone(), item)
            })
            .collect())
    }
}

type LoadedConversation = (Conversation, Vec<RegexScript>);

/// 回放数据里记的节点在对话里的位置：有节点 id 按 id 找，没有按记下的位置。
fn recorded_index(conversation: &Conversation, display_ref: &DisplayRef) -> Option<usize> {
    match display_ref.node_id.as_deref() {
        Some(node_id) => conversation
            .nodes
            .iter()
            .position(|node| node.id.as_str() == node_id),
        None => display_ref.node_index,
    }
}

/// `display_nodes` 的单条解析，规则见其文档。
fn locate_display(
    display_ref: &DisplayRef,
    home: Option<&LoadedConversation>,
    recorded: &HashMap<&str, Option<LoadedConversation>>,
) -> Option<NodeDisplay> {
    let recorded_id = display_ref.conversation_id.as_deref();
    let source = recorded_id.and_then(|id| recorded.get(id));
    let ((conversation, scripts), index, by_position) = match home {
        Some(home) if recorded_id.is_none_or(|id| id == home.0.id.as_str()) => {
            (home, recorded_index(&home.0, display_ref)?, false)
        }
        Some(home) => {
            let index = display_ref.node_index?;
            if let Some(Some((source, _))) = source {
                let original = source.nodes.get(recorded_index(source, display_ref)?)?;
                if original.active()?.content != home.0.nodes.get(index)?.active()?.content {
                    return None;
                }
            }
            (home, index, true)
        }
        None => {
            let loaded = source?.as_ref()?;
            (loaded, recorded_index(&loaded.0, display_ref)?, false)
        }
    };
    let node = conversation.nodes.get(index)?;
    let active = node.active()?;
    if active.role != Role::Assistant {
        return None;
    }
    Some(NodeDisplay {
        conversation_id: conversation.id.to_string(),
        node_id: node.id.to_string(),
        node_index: index,
        display_content: node_display_content(conversation, index, scripts)?,
        content: active.content.clone(),
        by_position,
        node_count: conversation.nodes.len(),
    })
}

/// 后处理落定之后自动接受。
async fn settle(
    app: &Arc<AppState>,
    output: &TurnOutput,
    terminal: Option<(String, Option<String>)>,
) -> SettleReport {
    let (postprocess, postprocess_detail) = match terminal {
        Some((kind, detail)) => (Some(kind), detail),
        None => (None, None),
    };
    let conversation_id = Id::from_str(&output.conversation_id);
    let node_id = Id::from_str(&output.node_id);
    let mut accept = "accepted";
    let mut result = accept_variant_async(
        app.clone(),
        conversation_id.clone(),
        node_id.clone(),
        false,
        None,
    )
    .await;
    if matches!(result, Err(TauriCommandError::Validation { .. })) {
        tracing::warn!(
            "自动接受被拦（{}），强制接受并标记 Degraded",
            result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default()
        );
        accept = "accepted_degraded";
        result = accept_variant_async(app.clone(), conversation_id, node_id, true, None).await;
    }
    let accept_error = result.err().map(|error| error.to_string());
    if let Some(error) = &accept_error {
        tracing::error!("自动接受失败: {error}");
        accept = "failed";
    }
    SettleReport {
        campaign_id: output.campaign_id.clone(),
        node_id: output.node_id.clone(),
        postprocess,
        postprocess_detail,
        accept: accept.into(),
        accept_error,
        usage: UsageTotals::default(),
    }
}

/// 对话里已采纳的写作轮数：一条用户输入紧跟一条已采纳（Final）的回复算一轮；
/// 开场白（没有用户输入）不算。
pub fn committed_turns(conversation: &Conversation) -> usize {
    let visible: Vec<_> = conversation
        .nodes
        .iter()
        .filter_map(|node| node.active())
        .filter(|variant| variant.status != VariantStatus::Discarded)
        .collect();
    visible
        .windows(2)
        .filter(|pair| {
            pair[0].role == Role::User
                && pair[1].role == Role::Assistant
                && pair[1].status == VariantStatus::Final
        })
        .count()
}

/// 自动路由建议升到高成本档：本轮没开始（没调模型、没写存档），`data` 带建议模式与
/// 预计调用量，插件问过用户后带显式 `generation_mode` 重发。
fn confirm_generation_mode(decision: &GenerationRouteDecision) -> BridgeError {
    let mode = serde_json::to_value(decision.mode).unwrap_or(Value::Null);
    BridgeError::new(
        "confirm_generation_mode",
        format!(
            "自动路由建议本轮用 {}（预计 {}），本轮还没开始；请确认后带 generation_mode 重发。",
            mode.as_str().unwrap_or("?"),
            decision.mode.estimated_call_label()
        ),
    )
    .with_data(serde_json::json!({
        "mode": mode,
        "reason": decision.reason,
        "estimate": decision.mode.estimated_call_label(),
        "continuation_estimate": GenerationMode::Continuation.estimated_call_label(),
    }))
}

fn out_of_sync(history_turns: usize, committed: usize) -> BridgeError {
    let message = if history_turns < committed {
        format!(
            "DSH 会话里本轮之前有 {history_turns} 轮，存档已有 {committed} 轮。StoryForge 自动接受的\
             轮次不能重新生成或改写（DSH 的重试 / 重新生成 / 编辑历史消息暂不支持）。请在最新\
             一轮之后继续写；想换个走向，可以从最后一条回复处分叉。"
        )
    } else {
        format!(
            "DSH 会话里本轮之前有 {history_turns} 轮，存档只有 {committed} 轮（有一轮没写进存档）。\
             为避免两边错位，本轮没有执行。"
        )
    };
    BridgeError::new("out_of_sync", message)
}

fn tool_mode_name(text_fallback: bool) -> &'static str {
    if text_fallback {
        "text_fallback"
    } else {
        "native"
    }
}

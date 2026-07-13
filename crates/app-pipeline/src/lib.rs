/// 写作流水线编排（对应设计 §3.1 状态机）
///
/// PipelineOrchestrator 连接 app-agent（工具循环）和 app-conversation（对话树），
/// 实现完整写作流程：用户意图 → 导演 Plan → 子 Agent 并行 → 编剧成文 → 写入对话树。
use std::sync::Arc;

pub mod quality_gate;

use tokio::sync::{mpsc, watch};
use tracing::{error, info};

use storyforge_domain::Id;
use storyforge_domain::agent::{
    AgentRole, ContextPackage, Draft, LoreEntryLight, PipelineEvent, PipelineState, Plan,
    SubagentTask, WritingSession,
};
use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::campaign::CharacterInstance;
use storyforge_domain::conversation::Provenance;
use storyforge_domain::mvu_translation::FallbackFragment;
use storyforge_domain::preset::{RegexPlacement, RegexScript};

use storyforge_app_agent::runtime::PromptHook;
use storyforge_app_agent::{
    AgentConfig, AgentError, AgentRuntime, DEFAULT_MAX_CONCURRENT_SUBAGENTS, EDITOR_HINT_MARKER,
    SUBAGENT_HINT_MARKER, ToolContext, ToolRegistry, filter_registry_by_whitelist, spawn_subagents,
    tools::register_director_tools,
};
use storyforge_app_conversation::{
    ConversationStore, PartialRollTarget, build_provenance_with_campaign,
};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_plugin_host::mvu_runtime::MvuRuntime;
use storyforge_infra_regex::{
    RegexExecutionTarget, apply_reasoning_regex_to_think_blocks_at_depth,
    apply_regex_scripts_for_target_at_depth,
};

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

    #[error("正则执行失败: {0}")]
    Regex(String),

    #[error("重 roll 失败: {0}")]
    Regenerate(String),

    #[error("流水线已取消")]
    Cancelled,

    #[error("流水线状态错误: {0}")]
    InvalidState(String),
}

// ─── 编剧提示词（role_directive，模块系统通过 assemble_system_prompt 增强）───

const DIRECTOR_SYSTEM_PROMPT: &str = r#"你是写作导演。用户给你写作意图，你要：

1. 调用 search_world_info / get_character 了解可用素材
2. 判断本场戏：核心冲突、对立目标、赌注、节拍；哪些线本场不得一次解决
3. 决定出场角色，为每个角色分配任务，并给出与用户输入无关的当前欲望/手头动作（可选 emotion_stage 1-6，禁止正文直说阶段名）
4. 为每个角色构造专属上下文包（注意知识隔离：角色只能知道自己的信息）
5. 输出结构化 Plan（含可选 scene_plan）

不要自己写正文。

【输出格式（必须严格遵守）】
调用 emit_plan 工具输出 Plan；如果无法调用工具，则直接输出如下 JSON（前后不要有任何其他文字、解释或 markdown 标记）：

{"scene_brief":"本场戏的一句话场景简述","scene_plan":{"conflict":"核心冲突","opposing_goals":["A想…","B想…"],"stakes":"失败代价","beats":["开场","升级","复杂化"],"complication":"搅局","must_not_resolve":"本场不得解决的问题","exit_hook":"留给下轮的钩子"},"subagent_tasks":[{"character_id":"角色标识","brief":"该角色任务","current_desire":"与用户输入无关的当前欲望","ongoing_action":"进场前正在做的事","emotion_stage":3}]}

scene_plan 与 current_desire/ongoing_action/emotion_stage 均可选；旧格式仅 scene_brief+brief 仍可接受。

character_id 规则：
- 如果可用角色列表显示为「Campaign 实例」，character_id 必须使用括号内的 instance_id（如 inst-xxx），不要使用角色名。
- 如果可用角色列表为普通角色名，character_id 使用角色名。

示例（用户意图「写一场雨中告别」，可用角色 Seraphina）：
{"scene_brief":"雨中告别，屋檐下两人对话","scene_plan":{"conflict":"想留与必须走","stakes":"错过最后一面","must_not_resolve":"两人关系终局","exit_hook":"雨势未停"},"subagent_tasks":[{"character_id":"Seraphina","brief":"演出告别时的温柔与不舍","current_desire":"想再听对方说一句留下来","ongoing_action":"撑伞站在檐下"}]}"#;

const EDITOR_SYSTEM_PROMPT: &str = r#"你是编剧。收集所有子 Agent 的表演，合并成连贯成文：

1. 节奏把控、视角切换、过渡衔接；角色对话要有目的，纹理随人设变化
2. 遵守 tail 中的 ScenePlan 与 NarrativeContract：限知叙述、不替角色全知他人私密；本场不得解决的问题保持未决；尾部停在互动中段便于接续
3. 输出最终成文（Markdown）

只输出正文本身，严禁输出任何说明、注释、改动标注、总结性文字、开场白或结语（例如「以下是合并后的成文」「我对某段做了裁剪」等）。第一行就必须是正文的开始。不需要调用工具。"#;

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
    /// 当前 Campaign（P2 新增）。None = 无 campaign，跳过后处理流水线（向后兼容）
    pub campaign_id: Option<Id>,
    /// 当前轮次（P2 新增，用于后处理摘要 turn + 任务触发比对）
    pub turn: u32,
    /// 待注入导演的任务/伏笔（P2 新增，确定性查表，零 LLM）
    pub pending_tasks: Vec<storyforge_domain::story_task::StoryTask>,
    /// 故事时钟（P2 新增，用于任务 StoryTime 触发比对 + 后处理上下文）
    pub story_clock: String,
    /// 预设 Profile（模块选择配置，用于组装 system prompt）
    pub profile: Option<storyforge_domain::prompt_module::PromptProfile>,
    /// 可用模块列表（Profile 引用的模块定义）
    pub modules: Vec<storyforge_domain::prompt_module::PromptModule>,
    /// ST regex scripts collected for this writing run.
    pub regex_scripts: Vec<RegexScript>,
    /// Campaign 运行时快照（阶段 2 新增）。None = 未开 Campaign，走旧路径。
    pub campaign_runtime: Option<Arc<storyforge_domain::campaign_runtime::CampaignRuntimeContext>>,
    /// Agent Profile 配置（可选）。None = 使用硬编码默认值。
    pub agent_profile_config: Option<AgentProfileConfig>,
    /// 近期 RoundSummary（按 turn 升序）。注入 Director volatile tail，
    /// 也同步到 ToolContext.archived_summaries 供 get_recent_summary 使用。
    /// load-side last-K + inject budgets 已落地；完整 token budget / 段 volatility 仍可后续扩展。
    pub recent_summaries: Vec<storyforge_domain::agent::RoundSummary>,
    /// 渲染冻结 overview/band 的完整 catalog（可含远 A/B/C；不受 last-K 截断）。
    pub chronicle_prompt_catalog: Vec<storyforge_domain::agent::RoundSummary>,
    /// 远记忆自动召回命中（带 id/score 溯源）。
    /// 由 Tauri 层在 start_writing 时按意图检索后填入；无向量库/无命中则为空。
    pub far_memory_hits: Vec<FarMemoryHit>,
    /// A2：本轮模板宏 `{{random}}` / `{{roll}}` 的确定性种子。
    /// Pipeline 在 start_writing / regenerate 时用本轮 seed 写入 TemplateVarContext；
    /// 外部也可预置（例如重放）。None = 渲染层回退时间种子（旧行为）。
    pub template_random_seed: Option<u64>,
    /// 本轮编译冻结的 ContextEpochSnapshot（fill_campaign 时刷新并落盘）。
    pub context_epoch: Option<storyforge_domain::chronicle::ContextEpochSnapshot>,
    /// 本轮捕获的 chronicle_revision（与 snapshot 一致）。
    pub chronicle_revision: u64,
}

// ─── ContextCompiler named budgets（注入侧；load-side last-K 在 tauri-app）──────
/// Director/Editor tail：近期 RoundSummary 最多注入条数。
pub const RECENT_SUMMARIES_INJECT_LIMIT: usize = 5;
/// Director/Editor tail：远记忆最多注入条数。
pub const FAR_MEMORY_INJECT_LIMIT: usize = 3;
/// 单条近期摘要 content 截断（字符）。
pub const RECENT_SUMMARY_ITEM_MAX_CHARS: usize = 240;
/// 单条远记忆 content 截断（字符）。
pub const FAR_MEMORY_ITEM_MAX_CHARS: usize = 200;

/// M2 过渡：Director history 前缀使用的近正文/纪要带默认（对齐 chronicle 实验默认）。
pub const M2_H_ANCHOR_TURNS: u32 = storyforge_domain::chronicle::DEFAULT_H_ANCHOR;
pub const M2_BAND_TURNS: u32 = storyforge_domain::chronicle::DEFAULT_S;
pub const M2_OVERVIEW_MAX: usize = storyforge_domain::chronicle::DEFAULT_OVERVIEW_MAX_ENTRIES;

/// 将 RoundSummary 划分为：概览 / 纪要带 / 近正文 turn 集合（按 turn 序号）。
#[derive(Debug, Clone)]
pub struct ChroniclePromptPartition {
    pub overview_lines: Vec<String>,
    pub band_lines: Vec<String>,
    pub near_turns: Vec<u32>,
    pub band_turns: Vec<u32>,
}

pub fn partition_summaries_for_prompt(
    summaries: &[storyforge_domain::agent::RoundSummary],
    h_anchor: u32,
    band_s: u32,
    overview_max: usize,
) -> ChroniclePromptPartition {
    let mut sorted = summaries.to_vec();
    sorted.sort_by_key(|s| s.turn);
    if sorted.is_empty() {
        return ChroniclePromptPartition {
            overview_lines: vec![],
            band_lines: vec![],
            near_turns: vec![],
            band_turns: vec![],
        };
    }
    let max_turn = sorted.last().map(|s| s.turn).unwrap_or(0);
    let near_start = if h_anchor == 0 {
        max_turn.saturating_add(1)
    } else {
        max_turn.saturating_sub(h_anchor - 1)
    };
    let near_turns: Vec<u32> = sorted
        .iter()
        .map(|s| s.turn)
        .filter(|t| *t >= near_start)
        .collect();
    let band_end = near_start.saturating_sub(1);
    let band_start = if band_s == 0 || band_end == 0 {
        0
    } else {
        band_end.saturating_sub(band_s - 1).max(1)
    };
    let mut band_lines = Vec::new();
    let mut band_turns = Vec::new();
    let mut overview_cands = Vec::new();
    for s in &sorted {
        if s.covered_by.is_some() {
            continue;
        }
        if s.turn >= near_start {
            continue;
        }
        if band_s > 0 && s.turn >= band_start && s.turn <= band_end {
            band_turns.push(s.turn);
            let code = s.code.as_deref().unwrap_or("");
            let body = if let Some(h) = s.headline.as_ref().filter(|h| !h.trim().is_empty()) {
                h.trim().to_string()
            } else {
                truncate_chars_pub(&s.content, RECENT_SUMMARY_ITEM_MAX_CHARS)
            };
            if code.is_empty() {
                band_lines.push(format!("T{}: {}", s.turn, body));
            } else {
                band_lines.push(format!("{code} T{}: {}", s.turn, body));
            }
        } else if s.turn < band_start || band_s == 0 {
            overview_cands.push(s.clone());
        }
    }
    if overview_cands.len() > overview_max {
        let drop_n = overview_cands.len() - overview_max;
        overview_cands = overview_cands[drop_n..].to_vec();
    }
    let overview_lines: Vec<String> = overview_cands
        .iter()
        .map(|s| {
            let code = s.code.as_deref().unwrap_or("");
            let h = s.overview_headline(40);
            if code.is_empty() {
                format!("T{} {}", s.turn, h)
            } else {
                format!("{code} {h}")
            }
        })
        .collect();
    ChroniclePromptPartition {
        overview_lines,
        band_lines,
        near_turns,
        band_turns,
    }
}

fn truncate_chars_pub(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

/// 把概览/纪要带作为 history 稳定前缀（旧于对话正文）。
/// 优先使用冻结的 ContextEpochSnapshot 构造 history 前缀分区；无快照则回退动态划分。
pub fn chronicle_partition_for_context(
    summaries: &[storyforge_domain::agent::RoundSummary],
    frozen: Option<&storyforge_domain::chronicle::ContextEpochSnapshot>,
) -> ChroniclePromptPartition {
    let fallback = partition_summaries_for_prompt(
        summaries,
        M2_H_ANCHOR_TURNS,
        M2_BAND_TURNS,
        M2_OVERVIEW_MAX,
    );
    let Some(snap) = frozen else {
        return fallback;
    };
    use storyforge_domain::chronicle::sequence_from_committed_turn_id;
    let by_code: std::collections::HashMap<String, &storyforge_domain::agent::RoundSummary> =
        summaries
            .iter()
            .filter_map(|s| s.code.as_ref().map(|c| (c.clone(), s)))
            .collect();
    let by_turn: std::collections::HashMap<u32, &storyforge_domain::agent::RoundSummary> =
        summaries.iter().map(|s| (s.turn, s)).collect();

    let overview_lines: Vec<String> = snap
        .overview_codes
        .iter()
        .map(|code| {
            if let Some(s) = by_code.get(code.as_str()) {
                format!("{} {}", code.as_str(), s.overview_headline(40))
            } else {
                code.as_str().to_string()
            }
        })
        .collect();
    let mut band_lines = Vec::new();
    let mut band_turns = Vec::new();
    for code in &snap.band_codes {
        if let Some(s) = by_code.get(code.as_str()) {
            band_turns.push(s.turn);
            let body = if let Some(h) = s.headline.as_ref().filter(|h| !h.trim().is_empty()) {
                h.trim().to_string()
            } else {
                truncate_chars_pub(&s.content, RECENT_SUMMARY_ITEM_MAX_CHARS)
            };
            band_lines.push(format!("{} T{}: {}", code.as_str(), s.turn, body));
        }
    }
    // near turns: anchor + approximate live by max turn after head
    let mut near_turns: Vec<u32> = snap
        .raw_anchor_turn_ids
        .iter()
        .filter_map(sequence_from_committed_turn_id)
        .collect();
    if let Some(head) = snap
        .source_head_turn_id
        .as_ref()
        .and_then(sequence_from_committed_turn_id)
    {
        for s in summaries {
            if s.turn > head {
                near_turns.push(s.turn);
            }
        }
    }
    near_turns.sort_unstable();
    near_turns.dedup();
    // if snapshot codes empty (fresh campaign), fall back
    if overview_lines.is_empty() && band_lines.is_empty() && near_turns.is_empty() {
        return fallback;
    }
    let _ = by_turn;
    ChroniclePromptPartition {
        overview_lines,
        band_lines,
        near_turns,
        band_turns,
    }
}

/// 将对话 history 收敛到 near_raw turns（若已知）。
///
/// 规格：history 中正文应对齐 `near_raw_turns`，避免与 overview/band 重叠的旧正文仍占窗。
/// 启发式：保留 checkpoint/非对话前缀；对话 user/assistant 对按出现顺序映射到 turn 序列后过滤。
/// 若 near_turns 为空则原样返回。
pub fn filter_history_to_near_raw_turns(
    history: Vec<storyforge_domain::llm::ChatMessage>,
    near_turns: &[u32],
) -> Vec<storyforge_domain::llm::ChatMessage> {
    use storyforge_domain::llm::{ChatMessage, ChatRole};
    if near_turns.is_empty() || history.is_empty() {
        return history;
    }
    let near: std::collections::HashSet<u32> = near_turns.iter().copied().collect();

    let mut prefix = Vec::new();
    let mut rest = Vec::new();
    for m in history {
        let is_special_prefix = matches!(m.role, ChatRole::User)
            && (m.content.starts_with("【历史纪要】")
                || m.content.starts_with("【事件概览】")
                || m.content.starts_with("【中距纪要带】")
                || m.content.starts_with("【事件概览")
                || m.content.starts_with("【中距纪要"));
        if rest.is_empty() && is_special_prefix {
            prefix.push(m);
        } else {
            rest.push(m);
        }
    }

    let mut pairs: Vec<Vec<ChatMessage>> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if matches!(rest[i].role, ChatRole::User) {
            let mut pair = vec![rest[i].clone()];
            if i + 1 < rest.len() && matches!(rest[i + 1].role, ChatRole::Assistant) {
                pair.push(rest[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
            pairs.push(pair);
        } else if let Some(last) = pairs.last_mut() {
            last.push(rest[i].clone());
            i += 1;
        } else {
            pairs.push(vec![rest[i].clone()]);
            i += 1;
        }
    }
    if pairs.is_empty() {
        prefix.extend(rest);
        return prefix;
    }

    let mut sorted_near = near_turns.to_vec();
    sorted_near.sort_unstable();
    sorted_near.dedup();
    let keep_from = pairs.len().saturating_sub(sorted_near.len());
    let mut kept = prefix;
    let mut kept_any_pair = false;
    for (idx, pair) in pairs.into_iter().enumerate() {
        if idx < keep_from {
            continue;
        }
        let turn_idx = idx - keep_from;
        let keep = turn_idx >= sorted_near.len()
            || (turn_idx < sorted_near.len() && near.contains(&sorted_near[turn_idx]));
        if keep {
            kept.extend(pair);
            kept_any_pair = true;
        }
    }
    if !kept_any_pair {
        // fail-open：过滤异常时保留前缀 + 全部 rest
        kept.extend(rest);
    }
    kept
}

pub fn prepend_chronicle_history_prefix(
    history: Vec<storyforge_domain::llm::ChatMessage>,
    part: &ChroniclePromptPartition,
) -> Vec<storyforge_domain::llm::ChatMessage> {
    let mut out = Vec::new();
    if !part.overview_lines.is_empty() {
        out.push(storyforge_domain::llm::ChatMessage::user(format!(
            "【事件概览】（code/headline，导航用；与状态/原文冲突时以状态与正文为准）\n{}",
            part.overview_lines.join("\n")
        )));
    }
    if !part.band_lines.is_empty() {
        out.push(storyforge_domain::llm::ChatMessage::user(format!(
            "【中距纪要带】（短摘要；同轮正文不在此重复）\n{}",
            part.band_lines.join("\n")
        )));
    }
    out.extend(history);
    out
}

/// 远记忆命中（ContextCompiler 注入用，可追溯向量库 id / 分数）。
#[derive(Debug, Clone, PartialEq)]
pub struct FarMemoryHit {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub kind: String,
}

impl FarMemoryHit {
    pub fn new(
        id: impl Into<String>,
        content: impl Into<String>,
        score: f32,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            score,
            kind: kind.into(),
        }
    }

    /// 测试/兼容：仅 content 的命中（无溯源）
    pub fn from_content(content: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            content: content.into(),
            score: 0.0,
            kind: "ArchivedSummary".into(),
        }
    }
}

impl WritingContext {
    /// 向后兼容的构造（无 campaign，跳过后处理）
    pub fn legacy(
        characters: Vec<Arc<storyforge_domain::character::Character>>,
        world_info: Option<Arc<storyforge_domain::world_info::WorldInfoBook>>,
        conversation_id: Id,
    ) -> Self {
        Self {
            characters,
            world_info,
            conversation_id,
            campaign_id: None,
            turn: 0,
            pending_tasks: vec![],
            story_clock: String::new(),
            profile: None,
            modules: vec![],
            regex_scripts: vec![],
            campaign_runtime: None,
            agent_profile_config: None,
            recent_summaries: vec![],
            chronicle_prompt_catalog: vec![],
            far_memory_hits: vec![],
            template_random_seed: None,
            context_epoch: None,
            chronicle_revision: 0,
        }
    }
}

/// 流水线编排器
pub struct PipelineOrchestrator {
    runtime: Arc<AgentRuntime>,
    conv_store: Arc<ConversationStore>,
    /// 当前流水线状态
    state: PipelineState,
    /// 最近一次会话
    session: Option<WritingSession>,
    /// Phase 6: 本轮创建的临时 instance（供 Tauri 层落盘）
    pending_temporary_instances: Vec<CharacterInstance>,
    /// MVU JS fallback 运行时（None=不支持 JS fallback，降级）
    mvu_runtime: Option<Arc<dyn MvuRuntime + Send + Sync>>,
    /// A1：连接级采样参数（含 reasoning 模式），从 active connection 注入。
    /// None = 用 Default（reasoning=Disabled）。
    sampling: Option<storyforge_domain::llm::SamplingParams>,
}

impl PipelineOrchestrator {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        conv_store: Arc<ConversationStore>,
        tool_ctx: Arc<ToolContext>,
        mvu_runtime: Option<Arc<dyn MvuRuntime + Send + Sync>>,
    ) -> Self {
        Self::new_with_sampling(llm, conv_store, tool_ctx, mvu_runtime, None, None)
    }

    /// A1：带连接级采样参数的构造函数。
    /// `sampling` 从 active connection 注入，让 runtime 构建 ChatRequest 时
    /// 携带 reasoning 模式；同时保存到 orchestrator 供 prompt 组装层读取。
    pub fn new_with_sampling(
        llm: Arc<dyn LlmClient>,
        conv_store: Arc<ConversationStore>,
        tool_ctx: Arc<ToolContext>,
        mvu_runtime: Option<Arc<dyn MvuRuntime + Send + Sync>>,
        prompt_hook: Option<PromptHook>,
        sampling: Option<storyforge_domain::llm::SamplingParams>,
    ) -> Self {
        let runtime = if let Some(hook) = prompt_hook {
            let mut rt = AgentRuntime::with_prompt_hook(llm, tool_ctx, hook);
            if let Some(ref sp) = sampling {
                rt = rt.with_sampling(sp.clone());
            }
            Arc::new(rt)
        } else {
            let mut rt = AgentRuntime::new(llm, tool_ctx);
            if let Some(ref sp) = sampling {
                rt = rt.with_sampling(sp.clone());
            }
            Arc::new(rt)
        };
        Self {
            runtime,
            conv_store,
            state: PipelineState::Idle,
            session: None,
            pending_temporary_instances: Vec::new(),
            mvu_runtime,
            sampling,
        }
    }

    pub fn new_with_prompt_hook(
        llm: Arc<dyn LlmClient>,
        conv_store: Arc<ConversationStore>,
        tool_ctx: Arc<ToolContext>,
        mvu_runtime: Option<Arc<dyn MvuRuntime + Send + Sync>>,
        prompt_hook: PromptHook,
    ) -> Self {
        Self::new_with_sampling(
            llm,
            conv_store,
            tool_ctx,
            mvu_runtime,
            Some(prompt_hook),
            None,
        )
    }

    /// A1：获取当前 reasoning 模式（供 prompt 组装层判断 CoT 互斥）。
    pub fn reasoning_mode(&self) -> storyforge_domain::llm::ReasoningMode {
        self.sampling
            .as_ref()
            .map(|s| s.reasoning.clone())
            .unwrap_or_default()
    }

    /// 获取当前状态
    pub fn state(&self) -> &PipelineState {
        &self.state
    }

    /// 获取最近会话
    pub fn session(&self) -> Option<&WritingSession> {
        self.session.as_ref()
    }

    /// Phase 6: 获取本轮创建的临时 instance（供 Tauri 层落盘到 CampaignStore）
    pub fn pending_temporary_instances(&self) -> &[CharacterInstance] {
        &self.pending_temporary_instances
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
        self.pending_temporary_instances.clear();
        // A2：本轮 seed 同时驱动 provenance 与模板 random/roll 宏
        let template_seed = ctx.template_random_seed.unwrap_or(seed);

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

        // 前置校验：没有可用角色时，导演无法分配子 Agent，提前返回友好错误
        // （避免导演陷入"搜不到角色 → 输出空 Plan → drift recovery 死循环"）
        // 兼容 Campaign 主线：campaign_runtime.instances 非空也可通过
        if !has_available_characters(ctx) {
            let msg = "没有可用角色。请先导入角色卡或在 Campaign 中添加角色实例。";
            let _ = event_tx.send(PipelineEvent::Error {
                message: msg.into(),
            });
            return Err(self.abort_with(&event_tx, PipelineError::InvalidState(msg.into())));
        }

        let director_intent = match apply_director_intent_regex(&intent, &ctx.regex_scripts) {
            Ok(text) => text,
            Err(e) => return Err(self.abort_with(&event_tx, e)),
        };
        let template_context = prompt_template_context_for_writing(ctx, Some(template_seed));

        let director_config = make_director_config(
            ctx.profile.as_ref(),
            &ctx.modules,
            &build_director_system_extra(ctx),
            ctx.agent_profile_config.as_ref(),
            template_context.as_ref(),
            &self.reasoning_mode(),
        );
        let mut director_registry = ToolRegistry::new();
        register_director_tools(&mut director_registry);
        if let Some(apc) = ctx.agent_profile_config.as_ref() {
            let wl = apc
                .run_config_for(&AgentRole::Director)
                .tool_whitelist
                .as_deref();
            filter_registry_by_whitelist(&mut director_registry, wl, "Director");
        }

        // §22 cache 友好布局：system（role_directive + 模块 + 蓝灯）+ history（对话历史）+ tail（意图/角色/任务）
        // 对话历史作为独立消息段（而非塞进 user 文本），保证 system+history 前缀稳定、cache 命中。
        // 超窗时 history 首条为确定性 epoch checkpoint（同 epoch 内稳定）。
        let (history, epoch_info) = self.conv_store.recent_history_with_epoch(
            &ctx.conversation_id,
            storyforge_domain::conversation::DEFAULT_HISTORY_WINDOW_SIZE,
            None,
        );
        if let Some(info) = &epoch_info {
            tracing::debug!(
                target: "context_compiler",
                epoch_id = %info.epoch_id,
                start = info.start,
                len = info.len,
                dropped = info.dropped,
                has_checkpoint = info.has_checkpoint,
                "Director history epoch"
            );
        }
        // M2：概览 + 纪要带进 history 前缀；近窗摘要从 tail 剔除（硬去重）
        let chronicle_src = if ctx.chronicle_prompt_catalog.is_empty() {
            &ctx.recent_summaries
        } else {
            &ctx.chronicle_prompt_catalog
        };
        let chronicle_part =
            chronicle_partition_for_context(chronicle_src, ctx.context_epoch.as_ref());
        tracing::debug!(
            target: "context_compiler",
            overview = chronicle_part.overview_lines.len(),
            band = chronicle_part.band_lines.len(),
            near_turns = ?chronicle_part.near_turns,
            "Director chronicle history prefix"
        );
        let history = prepend_chronicle_history_prefix(history, &chronicle_part);
        let history = filter_history_to_near_raw_turns(history, &chronicle_part.near_turns);
        let director_layout = storyforge_domain::message_layout::MessageLayout::build()
            .system(director_config.system_prompt.clone())
            .history(history)
            .tail(|_| build_director_tail(&director_intent, ctx));

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
            .run_tool_loop_with_layout(
                &director_config,
                director_layout,
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

        // Phase 6：为未匹配的角色创建临时 instance（临场角色）
        // 从 Director 的 context_package.character_brief 提取 persona 作为 override
        let char_specs: Vec<(String, Option<String>, Option<String>)> = plan
            .subagent_tasks
            .iter()
            .map(|t| {
                let persona = if t.context_package.character_brief.is_empty() {
                    None
                } else {
                    Some(t.context_package.character_brief.clone())
                };
                (t.character_id.clone(), persona, None)
            })
            .collect();
        let effective_runtime = if let Some(cr) = &ctx.campaign_runtime {
            let (updated, temps) = cr.with_temporaries_for(&char_specs);
            if !temps.is_empty() {
                info!(target: "app-pipeline", "创建 {} 个临时 instance: {:?}",
                    temps.len(), temps.iter().map(|t| t.id.as_str()).collect::<Vec<_>>());
            }
            self.pending_temporary_instances = temps;
            Some(Arc::new(updated))
        } else {
            ctx.campaign_runtime.clone()
        };

        let effective_runtime_for_prov = effective_runtime.clone();
        let max_concurrent = ctx
            .agent_profile_config
            .as_ref()
            .map(|c| c.effective_max_concurrent_subagents())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_SUBAGENTS);
        let summary_block = render_recent_summaries_for_injection(
            &ctx.recent_summaries,
            RECENT_SUMMARIES_INJECT_LIMIT,
        );
        let recent_texts: Vec<String> = ctx
            .recent_summaries
            .iter()
            .map(|s| s.content.clone())
            .collect();
        let far_block = render_far_memory_for_injection_excluding(
            &ctx.far_memory_hits,
            FAR_MEMORY_INJECT_LIMIT,
            &recent_texts,
        );
        let subagent_results = spawn_subagents(
            plan.subagent_tasks.clone(),
            self.runtime.clone(),
            &director_config,
            SUBAGENT_SYSTEM_PROMPT_TEMPLATE,
            cancel.clone(),
            event_tx.clone(),
            effective_runtime,
            max_concurrent,
            ctx.agent_profile_config.as_ref(),
            summary_block.as_deref(),
            far_block.as_deref(),
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
            let _ = event_tx.send(PipelineEvent::Error {
                message: msg.into(),
            });
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

        let editor_config = make_editor_config(
            ctx.profile.as_ref(),
            &ctx.modules,
            ctx.agent_profile_config.as_ref(),
            template_context.as_ref(),
            &self.reasoning_mode(),
        );

        let editor_contract =
            storyforge_domain::narrative_contract::NarrativeContract::from_plan_and_runtime(
                &plan,
                ctx.campaign_runtime.as_deref(),
            );
        // 构造编剧的用户消息（子 Agent 产出）：对异己 private 探针做硬 redaction。
        let performances_text =
            redact_performances_for_editor(&performances, Some(&editor_contract));

        // §22 cache 友好布局：system（role_directive + 模块）+ history + tail（场景/子产出/摘要/hint）
        let (editor_history, editor_epoch) = self.conv_store.recent_history_with_epoch(
            &ctx.conversation_id,
            storyforge_domain::conversation::DEFAULT_HISTORY_WINDOW_SIZE,
            None,
        );
        if let Some(info) = &editor_epoch {
            tracing::debug!(
                target: "context_compiler",
                epoch_id = %info.epoch_id,
                start = info.start,
                len = info.len,
                dropped = info.dropped,
                has_checkpoint = info.has_checkpoint,
                "Editor history epoch"
            );
        }
        let editor_layout = storyforge_domain::message_layout::MessageLayout::build()
            .system(editor_config.system_prompt.clone())
            .history(editor_history)
            .tail(|_| {
                build_editor_tail(
                    &plan.scene_brief,
                    &performances_text,
                    None,
                    &ctx.recent_summaries,
                    &ctx.far_memory_hits,
                    plan.scene_plan.as_ref(),
                    Some(&editor_contract),
                )
            });

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
            .run_tool_loop_with_layout(
                &editor_config,
                editor_layout,
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

        let final_text = match apply_editor_output_regex(&editor_resp.content, &ctx.regex_scripts) {
            Ok(text) => text,
            Err(e) => return Err(self.abort_with(&event_tx, e)),
        };

        let _ = event_tx.send(PipelineEvent::DraftReady {
            text: final_text.clone(),
        });

        info!(target: "app-pipeline", "编剧完成: {} 字", final_text.len());

        // ─── 阶段 4：写入对话树 ─────────────────────────────────────────
        self.state = PipelineState::Review;

        let provenance = build_provenance_with_campaign(
            session_id.clone(),
            Some(plan.clone()),
            &performances,
            None, // profile_id
            seed,
            None, // last_hint（首次写作无 hint）
            effective_runtime_for_prov.as_deref(),
        );

        // 写入对话树
        let node_id = match self.conv_store.append_ai_draft(
            &ctx.conversation_id,
            final_text.clone(),
            Some(provenance.clone()),
        ) {
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

    /// 后处理流水线（P2 新增，对应 D40-D41/D45）
    ///
    /// 编剧成文（DraftReady）后并行跑：
    /// - 剧情总结 Agent：产出本轮摘要
    /// - 后处理 Agent：三合一产出角色知识 + 变量更新 + 任务更新
    /// - **W10**：JS fallback 片段执行（`fallback_fragments` 非空 + `mvu_runtime` 可用时）
    ///
    /// **best-effort + 向后兼容**：
    /// - `ctx.campaign_id` 为 None 时跳过（旧用法无 campaign），返回 None
    /// - 任一 Agent 失败不影响另一个，失败只 warn 不阻断
    /// - 推 PostProcessStarted / PostProcessDone / PostProcessFailed / SummaryDone 事件
    ///
    /// `fallback_fragments`：由调用方（tauri-app）从 CampaignStore 查 MvuTranslation 后
    /// 拆出的 JS 片段。pipeline 不依赖 CampaignStore，只接收已拆好的片段。
    ///
    /// 返回 `Option<PostProcessOutcome>`：None 表示跳过，Some 表示跑过（产出可能为空）。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_postprocess(
        &self,
        final_text: &str,
        scene_brief: &str,
        present_characters: &[String],
        variable_keys: &[String],
        ctx: &WritingContext,
        event_tx: &mpsc::UnboundedSender<PipelineEvent>,
        cancel: watch::Receiver<bool>,
        fallback_fragments: &[FallbackFragment],
    ) -> Option<storyforge_app_agent::PostProcessOutcome> {
        let campaign_id = ctx.campaign_id.clone()?;

        // 从 AgentProfileConfig 读取开关和配置覆盖（无 config = 全开 + 硬编码默认，向后兼容）
        let (enable_postprocess, enable_summarizer) = ctx
            .agent_profile_config
            .as_ref()
            .map(|c| (c.enable_postprocess, c.enable_summarizer))
            .unwrap_or((true, true));

        // 两者都关：安静跳过，不发 PostProcessStarted，只发明确的 Skipped，不当失败处理
        if !enable_postprocess && !enable_summarizer {
            info!(target: "app-pipeline", "后处理被 AgentProfileConfig 全部关闭，跳过（campaign={campaign_id}）");
            let _ = event_tx.send(PipelineEvent::PostProcessSkipped {
                reason: "enable_postprocess=false 且 enable_summarizer=false（已按配置跳过）"
                    .into(),
            });
            return None;
        }

        let _ = event_tx.send(PipelineEvent::PostProcessStarted);
        info!(
            target: "app-pipeline",
            "后处理流水线启动: campaign={campaign_id} turn={} postprocess={} summarizer={}",
            ctx.turn, enable_postprocess, enable_summarizer
        );

        let summary_block = render_recent_summaries_for_injection(
            &ctx.recent_summaries,
            RECENT_SUMMARIES_INJECT_LIMIT,
        );
        let mut outcome = storyforge_app_agent::run_postprocess_pipeline(
            &self.runtime,
            final_text,
            scene_brief,
            present_characters,
            variable_keys,
            ctx.turn,
            &ctx.story_clock,
            cancel,
            enable_postprocess,
            enable_summarizer,
            ctx.agent_profile_config.as_ref(),
            summary_block.as_deref(),
        )
        .await;

        // ─── W10: JS fallback 执行 ─────────────────────────────────────────
        // 在正常 postprocess 之后执行。结果走现有 variable_updates 落盘路径（经 preview/patch）。
        // JS 失败只 warn，不影响主写作。
        if !fallback_fragments.is_empty() {
            match &self.mvu_runtime {
                Some(rt) if rt.is_available() => {
                    // 构造当前变量快照：campaign 内所有 CharacterInstance 的 variables + campaign 变量
                    let current_variables = build_current_variables(ctx);

                    for frag in fallback_fragments {
                        if frag.js_snippet.is_empty() {
                            continue;
                        }
                        info!(
                            target: "app-pipeline",
                            "[MVU JS] 执行 fallback 片段: desc='{}' snippet_len={}",
                            frag.description,
                            frag.js_snippet.len()
                        );
                        match rt
                            .execute_fragment(&frag.js_snippet, &current_variables)
                            .await
                        {
                            Ok(exec_result) => {
                                // side_effects 只记录日志（暂不自动执行）
                                for se in &exec_result.side_effects {
                                    info!(
                                        target: "app-pipeline",
                                        "[MVU JS] side_effect: {se}"
                                    );
                                }
                                // variable_updates 追加到 outcome（走现有落盘路径）
                                let js_var_count = exec_result.variable_updates.len();
                                if js_var_count > 0 {
                                    let pp =
                                        outcome.post_process.get_or_insert_with(Default::default);
                                    for (key, value) in exec_result.variable_updates {
                                        pp.variable_updates.push(
                                            storyforge_domain::agent::VariableUpdate {
                                                instance_id: None, // JS 产出默认为 campaign 级变量
                                                key,
                                                value,
                                            },
                                        );
                                    }
                                    info!(
                                        target: "app-pipeline",
                                        "[MVU JS] 追加 {js_var_count} 条变量更新到后处理产出"
                                    );
                                }
                            }
                            Err(e) => {
                                // JS 失败：warn + 跳过，不影响主写作（红线）
                                tracing::warn!(
                                    target: "app-pipeline",
                                    "[MVU JS] fallback 片段执行失败（跳过，不影响主写作）: desc='{}' err={e}",
                                    frag.description
                                );
                            }
                        }
                    }
                }
                Some(_) => {
                    // runtime 存在但不可用（WebView 未初始化等）
                    tracing::warn!(
                        target: "app-pipeline",
                        "[MVU JS] 有 {} 个 fallback 片段但 runtime 不可用，跳过 JS 执行",
                        fallback_fragments.len()
                    );
                }
                None => {
                    // mvu_runtime 为 None（降级：harness/无 WebView 环境）
                    if !fallback_fragments.is_empty() {
                        info!(
                            target: "app-pipeline",
                            "[MVU JS] mvu_runtime=None，{} 个 fallback 片段跳过（降级）",
                            fallback_fragments.len()
                        );
                    }
                }
            }
        }

        // 摘要完成事件：仅在 summarizer 开启且有产出时发
        if enable_summarizer && let Some(s) = &outcome.summary {
            let _ = event_tx.send(PipelineEvent::SummaryDone {
                char_count: s.chars().count(),
            });
        }

        // 后处理完成/失败/跳过事件
        match &outcome.post_process {
            Some(r) => {
                let _ = event_tx.send(PipelineEvent::PostProcessDone {
                    knowledge_count: r.knowledge_updates.len(),
                    variable_count: r.variable_updates.len(),
                    task_count: r.task_updates.len(),
                });
            }
            None => {
                if enable_postprocess {
                    // 开了但失败（best-effort）：发 Failed
                    let _ = event_tx.send(PipelineEvent::PostProcessFailed {
                        reason: "后处理 Agent 调用失败或被取消（best-effort，不阻断成文）".into(),
                    });
                } else {
                    // 明确关闭：发 Skipped，不发误导性的 Failed
                    let _ = event_tx.send(PipelineEvent::PostProcessSkipped {
                        reason: "enable_postprocess=false（已按配置跳过后处理 Agent）".into(),
                    });
                }
            }
        }

        Some(outcome)
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
        self.pending_temporary_instances.clear();
        // A2：regenerate 的 seed（含用户指定）驱动模板 random/roll
        let template_seed = ctx.template_random_seed.unwrap_or(seed);

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
        let rerun_subagents: Vec<String> = req
            .targets
            .iter()
            .filter_map(|t| {
                if let PartialRollTarget::Subagent(id) = t {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        let template_context = prompt_template_context_for_writing(ctx, Some(template_seed));

        // ─── 路径 A：整体重 roll（含 Director 或 targets 为空）──────────────
        if rerun_director || req.targets.is_empty() {
            self.state = PipelineState::Directing;
            let _ = event_tx.send(PipelineEvent::StateChanged {
                state: self.state.clone(),
            });
            let _ = event_tx.send(PipelineEvent::DirectorStarted);

            // 前置校验：没有可用角色时提前返回友好错误（同 start_writing，兼容 Campaign）
            if !has_available_characters(ctx) {
                let msg = "没有可用角色。请先导入角色卡或在 Campaign 中添加角色实例。";
                let _ = event_tx.send(PipelineEvent::Error {
                    message: msg.into(),
                });
                return Err(self.abort_with(&event_tx, PipelineError::InvalidState(msg.into())));
            }

            let director_config = make_director_config(
                ctx.profile.as_ref(),
                &ctx.modules,
                &build_director_system_extra(ctx),
                ctx.agent_profile_config.as_ref(),
                template_context.as_ref(),
                &self.reasoning_mode(),
            );
            let mut director_registry = ToolRegistry::new();
            register_director_tools(&mut director_registry);
            // 应用 AgentProfileConfig 的 tool_whitelist（None=默认全部，Some=只保留指定工具）
            if let Some(apc) = ctx.agent_profile_config.as_ref() {
                let wl = apc
                    .run_config_for(&AgentRole::Director)
                    .tool_whitelist
                    .as_deref();
                filter_registry_by_whitelist(&mut director_registry, wl, "Director");
            }

            // 导演 intent：复用旧 Plan 的场景作 intent + 可选 hint
            let intent_text = provenance_old
                .plan
                .as_ref()
                .map(|p| p.scene_brief.clone())
                .unwrap_or_else(|| "重新创作".into());
            let director_intent =
                match apply_director_intent_regex(&intent_text, &ctx.regex_scripts) {
                    Ok(text) => text,
                    Err(e) => return Err(self.abort_with(&event_tx, e)),
                };

            // §22 cache 友好布局：history 排除重 roll 目标节点及之后
            let (director_history, director_epoch) = self.conv_store.recent_history_with_epoch(
                &req.conversation_id,
                storyforge_domain::conversation::DEFAULT_HISTORY_WINDOW_SIZE,
                Some(&req.node_id),
            );
            if let Some(info) = &director_epoch {
                tracing::debug!(
                    target: "context_compiler",
                    epoch_id = %info.epoch_id,
                    start = info.start,
                    len = info.len,
                    dropped = info.dropped,
                    has_checkpoint = info.has_checkpoint,
                    "Director regenerate history epoch"
                );
            }
            let chronicle_src = if ctx.chronicle_prompt_catalog.is_empty() {
                &ctx.recent_summaries
            } else {
                &ctx.chronicle_prompt_catalog
            };
            let chronicle_part =
                chronicle_partition_for_context(chronicle_src, ctx.context_epoch.as_ref());
            let director_history =
                prepend_chronicle_history_prefix(director_history, &chronicle_part);
            let director_history =
                filter_history_to_near_raw_turns(director_history, &chronicle_part.near_turns);
            let director_layout = storyforge_domain::message_layout::MessageLayout::build()
                .system(director_config.system_prompt.clone())
                .history(director_history)
                .tail(|_| {
                    let mut t = build_director_tail(&director_intent, ctx);
                    if let Some(h) = hint.as_deref() {
                        let h = h.trim();
                        if !h.is_empty() {
                            t = t.push(format!("{EDITOR_HINT_MARKER}{h}"));
                        }
                    }
                    t
                });

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
                .run_tool_loop_with_layout(
                    &director_config,
                    director_layout,
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

            // Phase 6：为未匹配的角色创建临时 instance（临场角色）
            // 从 Director 的 context_package.character_brief 提取 persona 作为 override
            let char_specs: Vec<(String, Option<String>, Option<String>)> = plan
                .subagent_tasks
                .iter()
                .map(|t| {
                    let persona = if t.context_package.character_brief.is_empty() {
                        None
                    } else {
                        Some(t.context_package.character_brief.clone())
                    };
                    (t.character_id.clone(), persona, None)
                })
                .collect();
            let effective_runtime = if let Some(cr) = &ctx.campaign_runtime {
                let (updated, temps) = cr.with_temporaries_for(&char_specs);
                if !temps.is_empty() {
                    info!(target: "app-pipeline", "创建 {} 个临时 instance（重 roll）: {:?}",
                        temps.len(), temps.iter().map(|t| t.id.as_str()).collect::<Vec<_>>());
                }
                self.pending_temporary_instances = temps;
                Some(Arc::new(updated))
            } else {
                ctx.campaign_runtime.clone()
            };

            let effective_runtime_for_prov = effective_runtime.clone();
            let max_concurrent = ctx
                .agent_profile_config
                .as_ref()
                .map(|c| c.effective_max_concurrent_subagents())
                .unwrap_or(DEFAULT_MAX_CONCURRENT_SUBAGENTS);
            let summary_block = render_recent_summaries_for_injection(
                &ctx.recent_summaries,
                RECENT_SUMMARIES_INJECT_LIMIT,
            );
            let recent_texts: Vec<String> = ctx
                .recent_summaries
                .iter()
                .map(|s| s.content.clone())
                .collect();
            let far_block = render_far_memory_for_injection_excluding(
                &ctx.far_memory_hits,
                FAR_MEMORY_INJECT_LIMIT,
                &recent_texts,
            );
            let subagent_results = spawn_subagents(
                plan.subagent_tasks.clone(),
                self.runtime.clone(),
                &director_config,
                SUBAGENT_SYSTEM_PROMPT_TEMPLATE,
                cancel.clone(),
                event_tx.clone(),
                effective_runtime,
                max_concurrent,
                ctx.agent_profile_config.as_ref(),
                summary_block.as_deref(),
                far_block.as_deref(),
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
                let _ = event_tx.send(PipelineEvent::Error {
                    message: msg.into(),
                });
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
                    ctx.profile.as_ref(),
                    &ctx.modules,
                    effective_runtime_for_prov.as_deref(),
                    ctx.agent_profile_config.as_ref(),
                    &ctx.regex_scripts,
                    template_context.as_ref(),
                    &ctx.recent_summaries,
                    &ctx.far_memory_hits,
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
                    ctx.profile.as_ref(),
                    &ctx.modules,
                    ctx.campaign_runtime.as_deref(),
                    ctx.agent_profile_config.as_ref(),
                    &ctx.regex_scripts,
                    template_context.as_ref(),
                    &ctx.recent_summaries,
                    &ctx.far_memory_hits,
                )
                .await?;
            return Ok((final_text, provenance));
        }

        // ─── 路径 C：只重某子 Agent ────────────────────────────────────────
        if !rerun_subagents.is_empty() {
            let plan = provenance_old
                .plan
                .clone()
                .ok_or_else(|| PipelineError::Regenerate("旧 Provenance 无 Plan".into()))?;

            let director_config = make_director_config(
                ctx.profile.as_ref(),
                &ctx.modules,
                &build_director_system_extra(ctx),
                ctx.agent_profile_config.as_ref(),
                template_context.as_ref(),
                &self.reasoning_mode(),
            );

            self.state = PipelineState::Delegating;
            let _ = event_tx.send(PipelineEvent::StateChanged {
                state: self.state.clone(),
            });

            // 逐个重跑目标子 Agent
            let mut rerun_perfs: Vec<storyforge_domain::agent::Performance> = Vec::new();
            for (idx, target_id) in rerun_subagents.iter().enumerate() {
                // 找到目标角色在 plan 里的 task
                let target_task = plan
                    .subagent_tasks
                    .iter()
                    .find(|t| t.character_id == *target_id)
                    .ok_or_else(|| {
                        PipelineError::Regenerate(format!(
                            "目标子 Agent '{target_id}' 不在旧 Plan 中"
                        ))
                    })?
                    .clone();

                let _ = event_tx.send(PipelineEvent::SubagentStarted {
                    character_id: target_id.clone(),
                    index: idx,
                    total: rerun_subagents.len(),
                });

                // 重跑该子 Agent（单任务，注入 hint 到 system prompt）
                let new_perf = {
                    // §22 cache 友好布局：persona + 常驻世界设定进 system，场景/相关设定/最近对话 + 任务 + hint 进 tail
                    let stable_system = format!(
                        "{}\n\n你是角色 {}。\n\n{}",
                        SUBAGENT_SYSTEM_PROMPT_TEMPLATE,
                        target_task.character_id,
                        format_subagent_context_stable(&target_task.context_package),
                    );
                    let mut volatile_text = format!(
                        "{}\n\n{}",
                        format_subagent_context_volatile(&target_task.context_package),
                        target_task.brief,
                    );
                    // ContextCompiler 最小版：regenerate 单子 Agent 也注入近期摘要 + 远记忆
                    if let Some(block) = render_recent_summaries_for_injection(
                        &ctx.recent_summaries,
                        RECENT_SUMMARIES_INJECT_LIMIT,
                    ) {
                        volatile_text.push_str("\n\n");
                        volatile_text.push_str(&block);
                        volatile_text.push_str(
                            "\n（以上为近期剧情摘要，仅供保持连续性；勿泄露你角色不该知道的信息。）",
                        );
                    }
                    let recent_texts: Vec<String> = ctx
                        .recent_summaries
                        .iter()
                        .map(|s| s.content.clone())
                        .collect();
                    if let Some(block) = render_far_memory_for_injection_excluding(
                        &ctx.far_memory_hits,
                        FAR_MEMORY_INJECT_LIMIT,
                        &recent_texts,
                    ) {
                        volatile_text.push_str("\n\n");
                        volatile_text.push_str(&block);
                        volatile_text.push_str(
                            "\n（以上为与当前意图相关的远记忆，仅作背景；勿泄露你角色不该知道的信息，勿整段复述。）",
                        );
                    }
                    let hint_for_tail = hint.clone();
                    let sub_layout = storyforge_domain::message_layout::MessageLayout::build()
                        .system(stable_system)
                        .tail(|_| {
                            let mut t = storyforge_domain::message_layout::VolatileTail::new()
                                .push(target_task.context_package.task.clone())
                                .push(volatile_text.trim_end().to_string());
                            if let Some(h) = hint_for_tail.as_deref() {
                                let h = h.trim();
                                if !h.is_empty() {
                                    t = t.push(format!("{SUBAGENT_HINT_MARKER}{h}"));
                                }
                            }
                            t
                        });
                    let config = AgentConfig {
                        role: AgentRole::Subagent(target_id.clone()),
                        system_prompt: String::new(), // layout 版不使用此字段
                        max_tool_rounds: 10,
                        model: director_config.model.clone(),
                        tools: vec![],
                        terminal_tools: vec![],
                    };
                    // M1 子 Agent 无工具（纯表演）
                    let registry = ToolRegistry::new();

                    // 单子 Agent 流式：建 channel 转发 token 到 SubagentProgress
                    let (sub_tx, mut sub_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                    let pp_tx = event_tx.clone();
                    let pp_cid = target_id.clone();
                    tokio::spawn(async move {
                        while let Some(delta) = sub_rx.recv().await {
                            let _ = pp_tx.send(PipelineEvent::SubagentProgress {
                                character_id: pp_cid.clone(),
                                index: idx,
                                delta,
                            });
                        }
                    });

                    match self
                        .runtime
                        .run_tool_loop_with_layout(
                            &config,
                            sub_layout,
                            &registry,
                            cancel.clone(),
                            sub_tx,
                            None,
                        )
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
                            index: idx,
                            full_text: perf.full_text.clone(),
                        });
                        perf
                    }
                    Err(e) => {
                        let _ = event_tx.send(PipelineEvent::SubagentCancelled {
                            character_id: target_id.clone(),
                            index: idx,
                        });
                        return Err(self.abort_with(&event_tx, PipelineError::Agent(e)));
                    }
                };

                rerun_perfs.push(new_perf);
            }

            let rerun_by_character: std::collections::HashMap<
                String,
                storyforge_domain::agent::Performance,
            > = rerun_perfs
                .into_iter()
                .map(|perf| (perf.character_id.clone(), perf))
                .collect();

            // 按原顺序重建 performances，替换所有重跑的目标子 Agent
            let mut performances: Vec<storyforge_domain::agent::Performance> = Vec::new();
            for snap in &provenance_old.subagent_results {
                if let Some(perf) = rerun_by_character.get(&snap.character_id) {
                    performances.push(perf.clone());
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
                    ctx.profile.as_ref(),
                    &ctx.modules,
                    ctx.campaign_runtime.as_deref(),
                    ctx.agent_profile_config.as_ref(),
                    &ctx.regex_scripts,
                    template_context.as_ref(),
                    &ctx.recent_summaries,
                    &ctx.far_memory_hits,
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
    /// 被 regenerate 的各路径复用。`hint` 注入到编剧 tail。
    /// `before_node_id`：取对话历史时排除该节点及之后（重 roll 时排除目标消息）。
    #[allow(clippy::too_many_arguments)]
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
        profile: Option<&storyforge_domain::prompt_module::PromptProfile>,
        modules: &[storyforge_domain::prompt_module::PromptModule],
        campaign_runtime: Option<&storyforge_domain::campaign_runtime::CampaignRuntimeContext>,
        agent_profile_config: Option<&AgentProfileConfig>,
        regex_scripts: &[RegexScript],
        template_context: Option<&storyforge_domain::prompt_module::TemplateVarContext>,
        recent_summaries: &[storyforge_domain::agent::RoundSummary],
        far_memory_hits: &[FarMemoryHit],
    ) -> Result<(String, Provenance), PipelineError> {
        // 编剧开始前，检查取消
        if *cancel.borrow() {
            info!(target: "app-pipeline", "编剧开始前取消");
            return Err(self.abort_with(&event_tx, PipelineError::Cancelled));
        }

        self.state = PipelineState::Editing;
        let _ = event_tx.send(PipelineEvent::EditorStarted);

        let editor_config = make_editor_config(
            profile,
            modules,
            agent_profile_config,
            template_context,
            &self.reasoning_mode(),
        );

        let editor_contract =
            storyforge_domain::narrative_contract::NarrativeContract::from_plan_and_runtime(
                plan,
                campaign_runtime,
            );
        let performances_text =
            redact_performances_for_editor(performances, Some(&editor_contract));

        // §22 cache 友好布局：system（role_directive + 模块）+ history + tail（场景/子产出/摘要/hint）
        let (editor_history, editor_epoch) = self.conv_store.recent_history_with_epoch(
            &req.conversation_id,
            storyforge_domain::conversation::DEFAULT_HISTORY_WINDOW_SIZE,
            Some(&req.node_id),
        );
        if let Some(info) = &editor_epoch {
            tracing::debug!(
                target: "context_compiler",
                epoch_id = %info.epoch_id,
                start = info.start,
                len = info.len,
                dropped = info.dropped,
                has_checkpoint = info.has_checkpoint,
                "Editor regenerate history epoch"
            );
        }
        let editor_layout = storyforge_domain::message_layout::MessageLayout::build()
            .system(editor_config.system_prompt.clone())
            .history(editor_history)
            .tail(|_| {
                build_editor_tail(
                    &plan.scene_brief,
                    &performances_text,
                    hint,
                    recent_summaries,
                    far_memory_hits,
                    plan.scene_plan.as_ref(),
                    Some(&editor_contract),
                )
            });

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
            .run_tool_loop_with_layout(
                &editor_config,
                editor_layout,
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

        let final_text = match apply_editor_output_regex(&editor_resp.content, regex_scripts) {
            Ok(text) => text,
            Err(e) => return Err(self.abort_with(&event_tx, e)),
        };
        let _ = event_tx.send(PipelineEvent::DraftReady {
            text: final_text.clone(),
        });

        let provenance = build_provenance_with_campaign(
            session_id.clone(),
            Some(plan.clone()),
            performances,
            None,
            seed,
            hint.map(String::from),
            campaign_runtime,
        );

        // 写入对话树（variant 保留语义）：
        // - 重 roll **最后一条** AI 消息 → replace_active_variant（旧 active 降级
        //   Discarded，可切换切回查看；新 variant 设 active 为 Draft）
        // - 重 roll **中间**消息 → add_variant（保留旧版，开分支）
        // 后端按 nodes.last() 实时判定，避免前端 isLast 标志脏数据。
        // 用户可继续重 roll 产生多个版本，用 variant 切换按钮对比，满意后 accept。
        let is_last_ai = self
            .conv_store
            .is_last_assistant_node(&req.conversation_id, &req.node_id)
            .map_err(|e| self.abort_with(&event_tx, PipelineError::Conversation(e)))?;
        let land = if is_last_ai {
            self.conv_store.replace_active_variant(
                &req.conversation_id,
                &req.node_id,
                final_text.clone(),
                Some(provenance.clone()),
            )
        } else {
            self.conv_store.add_variant(
                &req.conversation_id,
                &req.node_id,
                final_text.clone(),
                Some(provenance.clone()),
            )
        };
        if let Err(e) = land {
            return Err(self.abort_with(&event_tx, PipelineError::Conversation(e)));
        }

        self.state = PipelineState::Committed;
        let _ = event_tx.send(PipelineEvent::StateChanged {
            state: self.state.clone(),
        });

        info!(target: "app-pipeline", "重 roll 完成: {} 字", final_text.len());
        Ok((final_text, provenance))
    }
}

// ─── 辅助函数 ──────────────────────────────────────────────────────────────

/// 构造导演 system 段的「稳定附加」部分：蓝灯常驻世界设定（§22 cache 友好布局）
///
/// 这部分内容整个会话稳定（不随每轮变化），移进 system 段以最大化 cache 命中。
/// 蓝灯常驻条目（LoreRoute::Constant/Both）按 depth 升序排列
///（depth 小的靠后=更受重视，对齐 ST 近因效应语义）。
fn apply_context_regex(
    text: &str,
    scripts: &[RegexScript],
    placement: RegexPlacement,
) -> Result<String, PipelineError> {
    let target = match placement {
        RegexPlacement::Input => RegexExecutionTarget::Prompt,
        RegexPlacement::Output => RegexExecutionTarget::Persisted,
        RegexPlacement::SlashCommand => RegexExecutionTarget::Prompt,
        RegexPlacement::WorldInfo => RegexExecutionTarget::Prompt,
        RegexPlacement::Reasoning => RegexExecutionTarget::Prompt,
    };
    apply_regex_scripts_for_target_at_depth(text, scripts, placement, target, 0)
        .map_err(|e| PipelineError::Regex(e.to_string()))
}

fn apply_director_intent_regex(
    text: &str,
    scripts: &[RegexScript],
) -> Result<String, PipelineError> {
    let slash_applied = if text.starts_with('/') {
        apply_context_regex(text, scripts, RegexPlacement::SlashCommand)?
    } else {
        text.to_string()
    };
    apply_context_regex(&slash_applied, scripts, RegexPlacement::Input)
}

fn apply_editor_output_regex(text: &str, scripts: &[RegexScript]) -> Result<String, PipelineError> {
    let reasoning_applied = apply_reasoning_regex_to_think_blocks_at_depth(
        text,
        scripts,
        RegexExecutionTarget::Persisted,
        0,
    )
    .map_err(|e| PipelineError::Regex(e.to_string()))?;

    apply_context_regex(&reasoning_applied, scripts, RegexPlacement::Output)
}

fn apply_world_info_regex(content: &str, ctx: &WritingContext) -> String {
    if ctx.regex_scripts.is_empty() {
        return content.to_string();
    }

    apply_context_regex(content, &ctx.regex_scripts, RegexPlacement::WorldInfo).unwrap_or_else(
        |e| {
            tracing::warn!("世界书正则执行失败，使用原始世界书内容: {e}");
            content.to_string()
        },
    )
}

fn build_director_system_extra(ctx: &WritingContext) -> String {
    let Some(book) = &ctx.world_info else {
        return String::new();
    };
    let mut constants = book.constant_entries();
    // depth 小的排后面（更重要）；depth 相同 order 小的排后面
    constants.sort_by(|a, b| b.depth.cmp(&a.depth).then_with(|| b.order.cmp(&a.order)));

    if constants.is_empty() {
        return String::new();
    }

    let mut out = String::from("【世界设定（常驻）】\n");
    for e in &constants {
        let content = apply_world_info_regex(&e.content, ctx);
        out.push_str(&format!("- {}：{}\n", e.keys.join(", "), content));
    }
    out.push_str(
        "\n（以上常驻设定始终生效。绿灯条目可通过 search_world_info / search_vectors 工具检索。）",
    );
    out
}

fn build_triggered_selective_lore(intent: &str, ctx: &WritingContext) -> String {
    let Some(book) = &ctx.world_info else {
        return String::new();
    };
    let mut entries = book.triggered_selective_entries(intent);
    // depth 小的排后面（更重要）；depth 相同 order 小的排后面
    entries.sort_by(|a, b| b.depth.cmp(&a.depth).then_with(|| b.order.cmp(&a.order)));

    if entries.is_empty() {
        return String::new();
    }

    let mut out = String::from("【世界设定（关键词触发）】\n");
    for e in &entries {
        let content = apply_world_info_regex(&e.content, ctx);
        out.push_str(&format!("- {}：{}\n", e.keys.join(", "), content));
    }
    out.push_str("\n（以上设定由本轮写作意图关键词触发，请优先参考。）");
    out
}

/// 检查是否有可用角色（兼容 Campaign 和旧扁平 Character 两条路径）
///
/// Campaign 主线：campaign_runtime.instances 非空即可通过。
/// 旧路径：ctx.characters 非空。
fn has_available_characters(ctx: &WritingContext) -> bool {
    if let Some(runtime) = &ctx.campaign_runtime {
        !runtime.instances.is_empty()
    } else {
        !ctx.characters.is_empty()
    }
}

/// A2：构造模板上下文并注入 random_seed（仅影响 random/roll，不固定 now）。
///
/// `random_seed` 优先用调用方本轮 seed；为 None 时回退 `ctx.template_random_seed`。
fn prompt_template_context_for_writing(
    ctx: &WritingContext,
    random_seed: Option<u64>,
) -> Option<storyforge_domain::prompt_module::TemplateVarContext> {
    let mut template = if let Some(runtime) = &ctx.campaign_runtime {
        prompt_template_context_from_campaign_runtime(runtime)?
    } else {
        if ctx.characters.len() != 1 {
            return None;
        }
        ctx.characters.first().map(|character| {
            storyforge_domain::prompt_module::TemplateVarContext::from_character(
                character.as_ref(),
                "玩家",
            )
        })?
    };
    let seed = random_seed.or(ctx.template_random_seed);
    if seed.is_some() {
        template.random_seed = seed;
    }
    Some(template)
}

fn prompt_template_context_from_campaign_runtime(
    runtime: &storyforge_domain::campaign_runtime::CampaignRuntimeContext,
) -> Option<storyforge_domain::prompt_module::TemplateVarContext> {
    let mut variables = std::collections::BTreeMap::new();
    variables.insert(
        "campaign.id".into(),
        runtime.campaign.id.as_str().to_string(),
    );
    variables.insert("campaign.name".into(), runtime.campaign.name.clone());
    variables.insert("turn".into(), runtime.turn.to_string());
    for variable in &runtime.campaign.variables {
        insert_template_variable(&mut variables, &variable.key, &variable.value);
        insert_template_variable(
            &mut variables,
            &format!("campaign.{}", variable.key),
            &variable.value,
        );
    }

    let mut instance_name_counts = std::collections::BTreeMap::<String, usize>::new();
    for instance in &runtime.instances {
        *instance_name_counts
            .entry(instance.name.clone())
            .or_default() += 1;
    }

    for instance in &runtime.instances {
        insert_instance_template_scope(
            &mut variables,
            runtime,
            instance,
            &format!("instance.{}", instance.id.as_str()),
        );

        if instance_name_counts.get(&instance.name) == Some(&1) {
            insert_instance_template_scope(
                &mut variables,
                runtime,
                instance,
                &format!("instance.{}", instance.name),
            );
        }
    }

    let Some(instance) = runtime
        .instances
        .first()
        .filter(|_| runtime.instances.len() == 1)
    else {
        return Some(storyforge_domain::prompt_module::TemplateVarContext {
            user_name: "玩家".into(),
            variables,
            render_character_macros: false,
            ..Default::default()
        });
    };

    let definition = runtime.definition_for_instance(instance);
    variables.insert("instance.id".into(), instance.id.as_str().to_string());
    variables.insert("instance.name".into(), instance.name.clone());
    for variable in &instance.variables {
        insert_template_variable(&mut variables, &variable.key, &variable.value);
        insert_template_variable(
            &mut variables,
            &format!("instance.{}", variable.key),
            &variable.value,
        );
    }

    Some(storyforge_domain::prompt_module::TemplateVarContext {
        char_name: instance.name.clone(),
        user_name: "玩家".into(),
        description: runtime
            .resolved_persona_for(instance)
            .map(str::to_string)
            .unwrap_or_default(),
        personality: runtime
            .resolved_behavior_for(instance)
            .map(str::to_string)
            .unwrap_or_default(),
        tags: definition
            .and_then(|d| d.group.clone())
            .into_iter()
            .collect(),
        variables,
        ..Default::default()
    })
}

fn insert_instance_template_scope(
    variables: &mut std::collections::BTreeMap<String, String>,
    runtime: &storyforge_domain::campaign_runtime::CampaignRuntimeContext,
    instance: &CharacterInstance,
    scope: &str,
) {
    variables.insert(format!("{scope}.id"), instance.id.as_str().to_string());
    variables.insert(format!("{scope}.name"), instance.name.clone());
    if let Some(persona) = runtime.resolved_persona_for(instance) {
        variables.insert(format!("{scope}.description"), persona.to_string());
    }
    if let Some(behavior) = runtime.resolved_behavior_for(instance) {
        variables.insert(format!("{scope}.personality"), behavior.to_string());
    }
    for variable in &instance.variables {
        insert_template_variable(
            variables,
            &format!("{scope}.{}", variable.key),
            &variable.value,
        );
    }
}

fn insert_template_variable(
    variables: &mut std::collections::BTreeMap<String, String>,
    key: &str,
    value: &serde_json::Value,
) {
    variables.insert(key.to_string(), template_variable_value(value));
}

fn template_variable_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

/// 构造 MVU JS 执行所需的当前变量快照
///
/// 从 CampaignRuntimeContext 的所有 CharacterInstance.variables 收集，
/// key 格式保持 VariableValue.key 原样。无 campaign_runtime 时返回空 map。
fn build_current_variables(
    ctx: &WritingContext,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut vars = std::collections::HashMap::new();
    if let Some(runtime) = &ctx.campaign_runtime {
        for inst in &runtime.instances {
            for vv in &inst.variables {
                vars.insert(vv.key.clone(), vv.value.clone());
            }
        }
    }
    vars
}

/// 构造导演的易变末尾（§22 volatile tail）：意图 + 可用角色 + 任务/伏笔
///
/// 这些内容每轮可能变化（意图变、任务触发变），压在 user tail 段，保证
/// system + history 前缀稳定、cache 命中。
fn build_director_tail(
    intent: &str,
    ctx: &WritingContext,
) -> storyforge_domain::message_layout::VolatileTail {
    use storyforge_domain::message_layout::VolatileTail;

    // 阶段 3：有 campaign_runtime 时，从 instances 渲染可用角色（含 id/role/persona/variables）
    // 无 campaign_runtime 时，退回旧逻辑（扁平 Character 名称列表）
    let char_block = if let Some(runtime) = &ctx.campaign_runtime {
        let mut lines = Vec::new();
        for inst in &runtime.instances {
            let def = runtime.definition_for_instance(inst);
            let role_str = def
                .map(|d| format!("{:?}", d.role_type))
                .unwrap_or_else(|| "unknown".into());
            let persona_summary = runtime
                .resolved_persona_for(inst)
                .map(|p| truncate_chars(p, 80))
                .unwrap_or_else(|| "(无 persona)".into());
            // 阶段 3：附加该 instance 的 variables 摘要（hp/state/location/mood）
            let var_summary = format_instance_variables(&inst.variables);
            let var_part = if var_summary.is_empty() {
                String::new()
            } else {
                format!(" | {}", var_summary)
            };
            lines.push(format!(
                "- {}（{}）[{}] {}{}",
                inst.name,
                inst.id.as_str(),
                role_str,
                persona_summary,
                var_part
            ));
        }
        lines.join("\n")
    } else {
        ctx.characters
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join("、")
    };

    let mut tail = VolatileTail::new();

    // 阶段 3：campaign_runtime 时用结构化角色列表，否则用旧的纯名称
    if ctx.campaign_runtime.is_some() {
        tail = tail.push(format!(
            "用户的写作意图：{intent}\n\n可用角色（Campaign 实例）：\n{char_block}"
        ));
    } else {
        tail = tail.push(format!(
            "用户的写作意图：{intent}\n\n可用角色：{char_block}"
        ));
    }

    let selective_lore = build_triggered_selective_lore(intent, ctx);
    if !selective_lore.is_empty() {
        tail = tail.push(selective_lore);
    }

    // 阶段 3：Campaign 全局变量注入（story_clock/weather/world_state 等，压在 volatile tail）
    if let Some(runtime) = &ctx.campaign_runtime {
        let vars_text = storyforge_domain::variables::render_variables_for_injection(
            &runtime.campaign.variables,
            &[],
        );
        if !vars_text.trim().is_empty() {
            tail = tail.push(vars_text);
        }
    }

    // M2：近窗/纪要带已进 history 时不再在 tail 双税；仅注入更远且未进前缀的摘要（兜底）
    let part = chronicle_partition_for_context(&ctx.recent_summaries, ctx.context_epoch.as_ref());
    let mut exclude_turns = part.near_turns.clone();
    exclude_turns.extend(part.band_turns.iter().copied());
    let tail_summaries = filter_summaries_excluding_turns(&ctx.recent_summaries, &exclude_turns);
    if let Some(summary_block) =
        render_recent_summaries_for_injection(&tail_summaries, RECENT_SUMMARIES_INJECT_LIMIT)
    {
        tail = tail.push(summary_block);
    }

    // 远记忆自动召回；与近期摘要去重，避免重复占预算
    let recent_texts: Vec<String> = ctx
        .recent_summaries
        .iter()
        .map(|s| s.content.clone())
        .collect();
    if let Some(far_block) = render_far_memory_for_injection_excluding(
        &ctx.far_memory_hits,
        FAR_MEMORY_INJECT_LIMIT,
        &recent_texts,
    ) {
        tail = tail.push(far_block);
    }

    // 任务/伏笔注入（P2，确定性查表，零 LLM）：只注入 Pending/Active 且触发满足的任务
    if !ctx.pending_tasks.is_empty() {
        let task_block = storyforge_domain::story_task::render_tasks_for_injection(
            &ctx.pending_tasks,
            ctx.turn,
            &ctx.story_clock,
        );
        if !task_block.is_empty() {
            tail = tail.push(format!(
                "{task_block}\n（请在规划本场戏时考虑以上即将触发或正在推进的任务/伏笔。）"
            ));
        }
    }

    tail = tail.push("请分析意图并输出 Plan。");
    tail
}

/// 将远记忆召回结果渲染为导演 volatile tail 文本。
///
/// - 取前 `limit` 条；单条截断 200 字。
/// - 注入文本仍只含 content（不把 id/score 塞给模型）；溯源字段供日志/调试。
/// - 空列表返回 None。
pub fn render_far_memory_for_injection(hits: &[FarMemoryHit], limit: usize) -> Option<String> {
    render_far_memory_for_injection_excluding(hits, limit, &[])
}

/// 同 `render_far_memory_for_injection`，但跳过与 `exclude`（通常是 recent_summaries）
/// 内容高度重叠的命中，避免近期摘要与远记忆重复占预算。
pub fn render_far_memory_for_injection_excluding(
    hits: &[FarMemoryHit],
    limit: usize,
    exclude: &[String],
) -> Option<String> {
    if hits.is_empty() || limit == 0 {
        return None;
    }
    let exclude_norms: Vec<String> = exclude
        .iter()
        .map(|s| normalize_summary_key(s))
        .filter(|s| !s.is_empty())
        .collect();
    let mut lines = Vec::new();
    for hit in hits {
        if lines.len() >= limit {
            break;
        }
        let content = hit.content.trim();
        if content.is_empty() {
            continue;
        }
        let key = normalize_summary_key(content);
        if exclude_norms.iter().any(|ex| summaries_overlap(ex, &key)) {
            continue;
        }
        lines.push(format!(
            "{}. {}",
            lines.len() + 1,
            truncate_chars(content, FAR_MEMORY_ITEM_MAX_CHARS)
        ));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "远记忆召回（与当前意图相关的归档摘要，供规划参考，勿整段复述）：\n{}",
        lines.join("\n")
    ))
}

fn normalize_summary_key(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .collect::<String>()
        .to_lowercase()
}

/// 粗粒度重叠：一方包含另一方的前缀/全文（≥12 字），或完全相等。
fn summaries_overlap(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let min_len = 12usize;
    if a.chars().count() >= min_len && b.contains(a) {
        return true;
    }
    if b.chars().count() >= min_len && a.contains(b) {
        return true;
    }
    false
}

/// 将近期 RoundSummary 渲染为导演 volatile tail 文本。
///
/// - 按 turn 升序输入；取最近 `limit` 条（生产默认 `RECENT_SUMMARIES_INJECT_LIMIT`）。
/// - 单条 content 截断到 `RECENT_SUMMARY_ITEM_MAX_CHARS`，避免吞掉当前意图预算。
/// - 跳过 `covered_by` 已折叠条目（记忆规格 §5.1）。
/// - 有 `code`/`headline` 时优先展示 Chronicle 导航行；否则回退 content 截断。
/// - **M2 完整态**：概览/纪要带进 history、近正文硬隔离；当前仍为 tail 注入兼容路径。
/// - 空列表返回 None（调用方不 push 空段）。
pub fn render_recent_summaries_for_injection(
    summaries: &[storyforge_domain::agent::RoundSummary],
    limit: usize,
) -> Option<String> {
    if summaries.is_empty() || limit == 0 {
        return None;
    }
    let start = summaries.len().saturating_sub(limit);
    let mut lines = Vec::new();
    for s in &summaries[start..] {
        if s.covered_by.is_some() {
            continue;
        }
        let content = s.content.trim();
        if content.is_empty() && s.headline.as_ref().is_none_or(|h| h.trim().is_empty()) {
            continue;
        }
        let body = if let Some(h) = s.headline.as_ref().filter(|h| !h.trim().is_empty()) {
            truncate_chars(h.trim(), RECENT_SUMMARY_ITEM_MAX_CHARS)
        } else {
            truncate_chars(content, RECENT_SUMMARY_ITEM_MAX_CHARS)
        };
        let label = match s.code.as_deref() {
            Some(code) if !code.is_empty() => format!("{code} T{}", s.turn),
            _ => format!("T{}", s.turn),
        };
        lines.push(format!("- {label}: {body}"));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "近期剧情摘要（按轮次/Chronicle code，供规划参考，勿直接复述）：\n{}",
        lines.join("\n")
    ))
}

/// 硬去重：去掉与 `exclude_turns` 同 turn 的摘要（近正文窗内禁止 A 双税）。
pub fn filter_summaries_excluding_turns(
    summaries: &[storyforge_domain::agent::RoundSummary],
    exclude_turns: &[u32],
) -> Vec<storyforge_domain::agent::RoundSummary> {
    summaries
        .iter()
        .filter(|s| !exclude_turns.contains(&s.turn))
        .filter(|s| s.covered_by.is_none())
        .cloned()
        .collect()
}

/// UTF-8 安全的字符截断（按 char 而非 byte 截断，避免中文 panic）
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

/// 格式化 instance 的 variables 为紧凑摘要（hp/state/location/mood）
fn format_instance_variables(variables: &[storyforge_domain::variables::VariableValue]) -> String {
    let keys = ["hp", "state", "location", "mood"];
    let mut parts = Vec::new();
    for key in &keys {
        if let Some(v) = variables.iter().find(|v| v.key == *key) {
            let val = match &v.value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            parts.push(format!("{}:{}", key, val));
        }
    }
    parts.join("/")
}

/// 构造导演 Agent 配置（通过 assemble_system_prompt 增强 role_directive + 蓝灯进 system）
///
/// 如果提供了 `agent_profile_config`，从中读取 Director 的 `model_override` 和 `max_tool_rounds` 覆盖默认值。
fn make_director_config(
    profile: Option<&storyforge_domain::prompt_module::PromptProfile>,
    modules: &[storyforge_domain::prompt_module::PromptModule],
    system_extra: &str,
    agent_profile_config: Option<&AgentProfileConfig>,
    template_context: Option<&storyforge_domain::prompt_module::TemplateVarContext>,
    reasoning: &storyforge_domain::llm::ReasoningMode,
) -> AgentConfig {
    let mut system_prompt = storyforge_domain::prompt_module::assemble_system_prompt(
        &AgentRole::Director,
        DIRECTOR_SYSTEM_PROMPT,
        profile,
        modules,
        "",
        reasoning,
    );
    // 蓝灯世界设定拼进 system 末尾（稳定段，§22.4）
    if !system_extra.is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(system_extra);
    }
    if let Some(template_context) = template_context {
        system_prompt = storyforge_domain::prompt_module::replace_template_vars_with_context(
            &system_prompt,
            template_context,
        );
    }

    // 从 AgentProfileConfig 读取覆盖
    let (model_override, rounds_override) = if let Some(apc) = agent_profile_config {
        let run = apc.run_config_for(&AgentRole::Director);
        (run.model_override.clone(), run.max_tool_rounds)
    } else {
        (None, None)
    };

    AgentConfig {
        role: AgentRole::Director,
        system_prompt,
        max_tool_rounds: rounds_override.unwrap_or(15),
        model: model_override.unwrap_or_else(|| "deepseek-chat".to_string()),
        tools: vec![],
        // emit_plan 是"声明 Plan 产出完成"的终止信号，必须列入 terminal_tools。
        // 否则 LLM 调用 emit_plan 后 run_tool_loop（runtime.rs:276）不终止，继续催
        // "请继续使用工具完成任务"，导致 Director 在工具循环里空转耗尽 max_tool_rounds
        // (15) 后失败。content 里的 JSON 完成探测兜不住 tool_call 路径（LLM 走
        // emit_plan 时 content 是自然语言）。对齐 postprocess 的 terminal_tools 修复。
        terminal_tools: vec!["emit_plan".into()],
    }
}

/// 构造编剧的易变末尾（§22 volatile tail）：场景 + 子产出 + 近期摘要 + 可选 hint
///
/// Editor performance 硬 redaction：按 NarrativeContract 裁掉异己 private 探针。
///
/// 规则：
/// - 某 performance 的 `character_id` 若不是 secret 拥有者，则 full_text 中的
///   稳定探针替换为 `[REDACTED_PRIVATE]`；
/// - 拥有者 performance 保留原文；
/// - 无 contract 时不做裁剪（向后兼容）。
fn redact_performances_for_editor(
    performances: &[storyforge_domain::agent::Performance],
    contract: Option<&storyforge_domain::narrative_contract::NarrativeContract>,
) -> String {
    use storyforge_domain::narrative_contract::extract_gate_probes;

    let Some(c) = contract else {
        return performances
            .iter()
            .map(|p| format!("### {}\n{}", p.character_id, p.full_text))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
    };

    // (probe, owner_id)
    let mut probes: Vec<(String, String)> = Vec::new();
    for b in &c.private_bindings {
        for p in extract_gate_probes(&b.secret) {
            probes.push((p, b.owner_id.clone()));
        }
    }
    for token in &c.must_not_reveal {
        let token = token.trim();
        if token.chars().count() < 4 {
            continue;
        }
        let owner = c.owner_of_secret(token).unwrap_or("").to_string();
        if !probes.iter().any(|(p, _)| p == token) {
            probes.push((token.to_string(), owner));
        }
    }

    performances
        .iter()
        .map(|p| {
            let mut text = p.full_text.clone();
            let speaker = p.character_id.as_str();
            for (probe, owner) in &probes {
                if owner.is_empty() {
                    // 无归属：保守 redact 全部 performance
                    if text.contains(probe) {
                        text = text.replace(probe, "[REDACTED_PRIVATE]");
                    }
                    continue;
                }
                // speaker 是 owner，或 speaker 名等于 owner binding 的 name → 保留
                let is_owner = speaker == owner
                    || c.private_bindings
                        .iter()
                        .any(|b| b.owner_id == *owner && b.owner_labels().contains(&speaker));
                if !is_owner && text.contains(probe) {
                    text = text.replace(probe, "[REDACTED_PRIVATE]");
                }
            }
            format!("### {speaker}\n{text}")
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// 编剧 system 段只有 role_directive + 模块（不含蓝灯，编剧不需要）。
/// 场景简述和子 Agent 产出每场戏都变，压在 tail。
fn build_editor_tail(
    scene_brief: &str,
    performances_text: &str,
    hint: Option<&str>,
    recent_summaries: &[storyforge_domain::agent::RoundSummary],
    far_memory_hits: &[FarMemoryHit],
    scene_plan: Option<&storyforge_domain::agent::ScenePlan>,
    narrative_contract: Option<&storyforge_domain::narrative_contract::NarrativeContract>,
) -> storyforge_domain::message_layout::VolatileTail {
    use storyforge_domain::message_layout::VolatileTail;

    let mut head = format!(
        "场景：{scene_brief}\n\n子 Agent 表演：\n\n{performances_text}\n\n请合并成连贯成文。"
    );
    if let Some(sp) = scene_plan {
        let rendered = sp.render_for_prompt();
        if !rendered.is_empty() {
            head.push_str("\n\n");
            head.push_str(&rendered);
        }
    }
    if let Some(nc) = narrative_contract {
        let rendered = nc.render_for_prompt();
        if !rendered.is_empty() {
            head.push_str("\n\n");
            head.push_str(&rendered);
        }
    }
    let mut tail = VolatileTail::new().push(head);
    // ContextCompiler 最小版：编剧也看到近期事实，减少跨轮设定漂移
    if let Some(summary_block) =
        render_recent_summaries_for_injection(recent_summaries, RECENT_SUMMARIES_INJECT_LIMIT)
    {
        tail = tail.push(format!(
            "{summary_block}\n（合并成文时保持与上述摘要一致，勿改写已发生事实。）"
        ));
    }
    let recent_texts: Vec<String> = recent_summaries.iter().map(|s| s.content.clone()).collect();
    if let Some(far_block) = render_far_memory_for_injection_excluding(
        far_memory_hits,
        FAR_MEMORY_INJECT_LIMIT,
        &recent_texts,
    ) {
        tail = tail.push(format!(
            "{far_block}\n（合稿时仅作背景约束，勿整段复述远记忆。）"
        ));
    }
    if let Some(h) = hint {
        let h = h.trim();
        if !h.is_empty() {
            tail = tail.push(format!("{}{}", EDITOR_HINT_MARKER, h));
        }
    }
    tail
}

/// 构造编剧 Agent 配置（通过 assemble_system_prompt 增强 role_directive）
///
/// 如果提供了 `agent_profile_config`，从中读取 Editor 的 `model_override` 和 `max_tool_rounds` 覆盖默认值。
fn make_editor_config(
    profile: Option<&storyforge_domain::prompt_module::PromptProfile>,
    modules: &[storyforge_domain::prompt_module::PromptModule],
    agent_profile_config: Option<&AgentProfileConfig>,
    template_context: Option<&storyforge_domain::prompt_module::TemplateVarContext>,
    reasoning: &storyforge_domain::llm::ReasoningMode,
) -> AgentConfig {
    let mut system_prompt = storyforge_domain::prompt_module::assemble_system_prompt(
        &AgentRole::Editor,
        EDITOR_SYSTEM_PROMPT,
        profile,
        modules,
        "",
        reasoning,
    );
    if let Some(template_context) = template_context {
        system_prompt = storyforge_domain::prompt_module::replace_template_vars_with_context(
            &system_prompt,
            template_context,
        );
    }

    // 从 AgentProfileConfig 读取覆盖
    let (model_override, rounds_override) = if let Some(apc) = agent_profile_config {
        let run = apc.run_config_for(&AgentRole::Editor);
        (run.model_override.clone(), run.max_tool_rounds)
    } else {
        (None, None)
    };

    AgentConfig {
        role: AgentRole::Editor,
        system_prompt,
        max_tool_rounds: rounds_override.unwrap_or(5),
        model: model_override.unwrap_or_else(|| "deepseek-chat".to_string()),
        tools: vec![],
        terminal_tools: vec![],
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
fn parse_plan_from_response(
    resp: &storyforge_domain::llm::ChatResponse,
) -> Result<Plan, PipelineError> {
    let parse = |v: &serde_json::Value| -> Option<Plan> {
        serde_json::from_value::<serde_json::Value>(v.clone())
            .ok()
            .and_then(|val| parse_plan_json(&val).ok())
    };

    // ① tool_calls 中的 emit_plan（层 1）
    if let Some(plan) = storyforge_app_agent::llm_parse::from_tool_call(resp, "emit_plan", parse) {
        return Ok(plan);
    }

    let content = resp.content.trim();
    if !content.is_empty() {
        // ②-⑤：整体 JSON / ```json / 裸代码块 / 括号配平（层 2-5）
        let parse_text = |s: &str| -> Option<Plan> {
            serde_json::from_str::<serde_json::Value>(s)
                .ok()
                .and_then(|val| parse_plan_json(&val).ok())
        };
        if let Some(plan) = storyforge_app_agent::llm_parse::parse_from_content(content, parse_text)
        {
            return Ok(plan);
        }
    }

    Err(PipelineError::PlanParse(format!(
        "导演响应中未找到有效 Plan。导演原始输出（前 500 字）：{}",
        resp.content.chars().take(500).collect::<String>()
    )))
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

            let current_desire = t
                .get("current_desire")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let ongoing_action = t
                .get("ongoing_action")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let emotion_stage = t.get("emotion_stage").and_then(|v| {
                let n = if let Some(n) = v.as_u64() {
                    u8::try_from(n).ok()?
                } else {
                    v.as_str()?.parse::<u8>().ok()?
                };
                (1..=6).contains(&n).then_some(n)
            });

            SubagentTask {
                character_id,
                brief,
                context_package,
                current_desire,
                ongoing_action,
                emotion_stage,
            }
        })
        .collect();

    let scene_plan = v.get("scene_plan").and_then(parse_scene_plan);

    Ok(Plan {
        scene_brief,
        subagent_tasks,
        scene_plan,
    })
}

/// 解析可选 ScenePlan 对象；全空则返回 None
fn parse_scene_plan(v: &serde_json::Value) -> Option<storyforge_domain::agent::ScenePlan> {
    if !v.is_object() {
        return None;
    }
    let str_field = |key: &str| -> Option<String> {
        v.get(key)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let list_field = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let plan = storyforge_domain::agent::ScenePlan {
        conflict: str_field("conflict"),
        opposing_goals: list_field("opposing_goals"),
        stakes: str_field("stakes"),
        beats: list_field("beats"),
        complication: str_field("complication"),
        must_not_resolve: str_field("must_not_resolve"),
        exit_hook: str_field("exit_hook"),
    };
    if plan.is_empty() { None } else { Some(plan) }
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
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
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
        constant_lore: v
            .get("constant_lore")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|e| LoreEntryLight {
                        keys: e
                            .get("keys")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
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
        recent_window: v
            .get("recent_window")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
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

/// 格式化 ContextPackage 的**稳定部分**（子 Agent system 段，§22，重 roll 子 Agent 时用）
///
/// 与 `app-agent/src/runtime.rs::format_context_stable` 保持同步（同算法，独立实现避免跨 crate 耦合）。
fn format_subagent_context_stable(pkg: &ContextPackage) -> String {
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

/// 格式化 ContextPackage 的**易变部分**（子 Agent tail 段，§22，重 roll 子 Agent 时用）
///
/// 与 `app-agent/src/runtime.rs::format_context_volatile` 保持同步。
fn format_subagent_context_volatile(pkg: &ContextPackage) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use storyforge_domain::Source;
    use storyforge_domain::character::Character;
    use storyforge_domain::preset::{
        RegexPlacement, RegexScript, RegexScriptSource, ST_REGEX_PLACEMENT_AI_OUTPUT,
        ST_REGEX_PLACEMENT_REASONING, ST_REGEX_PLACEMENT_SLASH_COMMAND,
        ST_REGEX_PLACEMENT_USER_INPUT, ST_REGEX_PLACEMENT_WORLD_INFO,
    };
    use storyforge_infra_llm::mock_client::{MockLlmClient, MockScript};

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

    /// 构造最小 mock regex script（测试流水线执行顺序用）
    fn mock_regex_script(
        name: &str,
        find_regex: &str,
        replace_string: &str,
        placement: RegexPlacement,
    ) -> RegexScript {
        let placement_codes = match placement {
            RegexPlacement::Input => vec![ST_REGEX_PLACEMENT_USER_INPUT],
            RegexPlacement::Output => vec![ST_REGEX_PLACEMENT_AI_OUTPUT],
            RegexPlacement::SlashCommand => vec![ST_REGEX_PLACEMENT_SLASH_COMMAND],
            RegexPlacement::WorldInfo => vec![ST_REGEX_PLACEMENT_WORLD_INFO],
            RegexPlacement::Reasoning => vec![ST_REGEX_PLACEMENT_REASONING],
        };
        RegexScript {
            id: format!("test-{name}"),
            script_name: name.to_string(),
            find_regex: find_regex.to_string(),
            replace_string: replace_string.to_string(),
            placement,
            placement_codes,
            source: RegexScriptSource::Scoped,
            disabled: false,
            flags: String::new(),
            only_format_formatting: None,
            markdown_only: None,
            prompt_only: None,
            run_on_edit: None,
            substitute_regex: None,
            trim_strings: vec![],
            min_depth: None,
            max_depth: None,
        }
    }

    /// 集成测试：用 MockLlmClient 跑完整 Director → Subagent → Editor 闭环
    #[tokio::test]
    async fn test_full_pipeline_with_mock() {
        // 构造 mock LLM client
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let conv_dir =
            std::env::temp_dir().join(format!("storyforge_test_pipeline_{}", uuid::Uuid::new_v4()));
        let conv_store = Arc::new(ConversationStore::new(conv_dir.clone()));

        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });

        let mut orchestrator = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx, None);

        let ctx = WritingContext::legacy(
            vec![mock_character("Seraphina")],
            None,
            conv_store.create(None, None).id,
        );

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
        assert!(
            event_types.contains(&"director_started".into()),
            "应有 director_started"
        );
        assert!(
            event_types.contains(&"director_done".into()),
            "应有 director_done"
        );
        assert!(
            event_types.contains(&"editor_started".into()),
            "应有 editor_started"
        );
        assert!(
            event_types.contains(&"draft_ready".into()),
            "应有 draft_ready"
        );

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
    #[tokio::test]
    async fn test_output_regex_applies_before_commit() {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let conv_dir = std::env::temp_dir().join(format!(
            "storyforge_test_output_regex_{}",
            uuid::Uuid::new_v4()
        ));
        let conv_store = Arc::new(ConversationStore::new(conv_dir.clone()));

        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });

        let mut orchestrator = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx, None);
        let mut ctx = WritingContext::legacy(
            vec![mock_character("Seraphina")],
            None,
            conv_store.create(None, None).id,
        );
        ctx.regex_scripts = vec![mock_regex_script(
            "replace-all-output",
            r"[\s\S]+",
            "REGEX_FILTERED_DRAFT",
            RegexPlacement::Output,
        )];

        let (event_tx, _event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let (text, node_id, _provenance) = orchestrator
            .start_writing("write a scene".into(), &ctx, event_tx, cancel_rx)
            .await
            .expect("pipeline should succeed with output regex");

        assert_eq!(text, "REGEX_FILTERED_DRAFT");
        let conv = conv_store.get(&ctx.conversation_id).unwrap();
        let node = conv.find_node(&node_id).unwrap();
        assert_eq!(node.active().unwrap().content, "REGEX_FILTERED_DRAFT");

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    #[tokio::test]
    async fn test_reasoning_regex_applies_before_commit() {
        let plan = serde_json::json!({
            "scene_brief": "test scene",
            "subagent_tasks": [{
                "character_id": "Seraphina",
                "brief": "perform",
                "context_package": {
                    "character_brief": "Seraphina",
                    "scene_brief": "test scene",
                    "relevant_lore": [],
                    "constant_lore": [],
                    "recent_window": [],
                    "task": "perform"
                }
            }]
        })
        .to_string();
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::new(vec![
            MockScript {
                match_keyword: DIRECTOR_SYSTEM_PROMPT.chars().take(8).collect(),
                response_content: plan,
                tool_calls: vec![],
                stream: false,
            },
            MockScript {
                match_keyword: SUBAGENT_SYSTEM_PROMPT_TEMPLATE
                    .split("{name}")
                    .next()
                    .unwrap()
                    .to_string(),
                response_content: "subagent performance".into(),
                tool_calls: vec![],
                stream: false,
            },
            MockScript {
                match_keyword: EDITOR_SYSTEM_PROMPT.chars().take(8).collect(),
                response_content: "<think>secret plan</think> final secret".into(),
                tool_calls: vec![],
                stream: false,
            },
        ]));
        let conv_dir = std::env::temp_dir().join(format!(
            "storyforge_test_reasoning_regex_{}",
            uuid::Uuid::new_v4()
        ));
        let conv_store = Arc::new(ConversationStore::new(conv_dir.clone()));
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let mut orchestrator = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx, None);

        let conv = conv_store.create(None, None);
        let mut ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, conv.id);
        let mut script = mock_regex_script(
            "reasoning-redact",
            r"secret",
            "hidden",
            RegexPlacement::Reasoning,
        );
        script.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];
        script.flags = "g".into();
        ctx.regex_scripts = vec![script];
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let (text, node_id, _provenance) = orchestrator
            .start_writing("test intent".into(), &ctx, event_tx, cancel_rx)
            .await
            .expect("pipeline should commit reasoning-filtered draft");

        assert_eq!(text, "<think>hidden plan</think> final secret");
        let conv = conv_store.get(&ctx.conversation_id).unwrap();
        let node = conv.find_node(&node_id).unwrap();
        assert_eq!(
            node.active().unwrap().content,
            "<think>hidden plan</think> final secret"
        );

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    #[test]
    fn test_context_input_regex_uses_prompt_target() {
        let mut script = mock_regex_script(
            "prompt-only-input",
            r"raw intent",
            "rewritten intent",
            RegexPlacement::Input,
        );
        script.prompt_only = Some(true);

        let result = apply_context_regex("raw intent", &[script], RegexPlacement::Input)
            .expect("prompt-only input regex should apply to prompt target");

        assert_eq!(result, "rewritten intent");
    }

    #[test]
    fn test_context_input_regex_uses_current_st_user_input_code() {
        let mut script = mock_regex_script(
            "reader-input-wrapper",
            r"(.*)",
            "<reader-response>$1</reader-response>",
            RegexPlacement::Input,
        );
        script.prompt_only = Some(true);
        script.placement_codes = vec![ST_REGEX_PLACEMENT_USER_INPUT];

        let result = apply_context_regex("go north", &[script], RegexPlacement::Input)
            .expect("current ST user input placement should apply before prompting");

        assert_eq!(result, "<reader-response>go north</reader-response>");
    }

    #[test]
    fn test_director_intent_slash_regex_applies_to_slash_input() {
        let script = mock_regex_script(
            "slash-roll",
            r"^/roll$",
            "ROLL_INTENT",
            RegexPlacement::SlashCommand,
        );

        let result = apply_director_intent_regex("/roll", &[script])
            .expect("slash command regex should apply to slash input");

        assert_eq!(result, "ROLL_INTENT");
    }

    #[test]
    fn test_director_intent_slash_regex_ignores_non_slash_input() {
        let script = mock_regex_script(
            "slash-roll",
            r"^roll$",
            "SHOULD_NOT_RUN",
            RegexPlacement::SlashCommand,
        );

        let result = apply_director_intent_regex("roll", &[script])
            .expect("non-slash input should skip slash command regex");

        assert_eq!(result, "roll");
    }

    #[test]
    fn test_director_intent_slash_regex_uses_prompt_target() {
        let mut script = mock_regex_script(
            "prompt-only-slash",
            r"^/roll$",
            "ROLL_INTENT",
            RegexPlacement::SlashCommand,
        );
        script.prompt_only = Some(true);

        let result = apply_director_intent_regex("/roll", &[script])
            .expect("prompt-only slash regex should apply to prompt target");

        assert_eq!(result, "ROLL_INTENT");
    }

    #[test]
    fn test_director_intent_non_slash_skips_invalid_slash_regex() {
        let script = mock_regex_script(
            "invalid-slash",
            "(",
            "SHOULD_NOT_RUN",
            RegexPlacement::SlashCommand,
        );

        let result = apply_director_intent_regex("roll", &[script])
            .expect("non-slash input should not compile slash command regex");

        assert_eq!(result, "roll");
    }

    #[test]
    fn test_director_intent_non_slash_still_runs_input_regex() {
        let input = mock_regex_script(
            "reader-input-wrapper",
            r"^hello$",
            "<reader-response>hello</reader-response>",
            RegexPlacement::Input,
        );

        let result = apply_director_intent_regex("hello", &[input])
            .expect("non-slash input should still run input regex");

        assert_eq!(result, "<reader-response>hello</reader-response>");
    }

    #[test]
    fn test_director_intent_input_regex_does_not_reenter_slash_regex() {
        let slash = mock_regex_script(
            "slash-foo",
            r"^/foo$",
            "SHOULD_NOT_RUN",
            RegexPlacement::SlashCommand,
        );
        let input = mock_regex_script("input-to-slash", r"^hello$", "/foo", RegexPlacement::Input);

        let result = apply_director_intent_regex("hello", &[slash, input])
            .expect("input regex result should not re-run slash command regex");

        assert_eq!(result, "/foo");
    }

    #[test]
    fn test_director_intent_slash_regex_requires_leading_slash() {
        let script = mock_regex_script(
            "slash-with-space",
            r"^\s*/foo$",
            "SHOULD_NOT_RUN",
            RegexPlacement::SlashCommand,
        );

        let result = apply_director_intent_regex(" /foo", &[script])
            .expect("leading whitespace should not trigger slash command regex");

        assert_eq!(result, " /foo");
    }

    #[test]
    fn test_director_intent_input_regex_runs_after_slash_regex() {
        let slash = mock_regex_script("slash-foo", r"^/foo$", "foo", RegexPlacement::SlashCommand);
        let input = mock_regex_script(
            "reader-input-wrapper",
            r"^foo$",
            "<reader-response>foo</reader-response>",
            RegexPlacement::Input,
        );

        let result = apply_director_intent_regex("/foo", &[slash, input])
            .expect("input regex should run after slash command regex");

        assert_eq!(result, "<reader-response>foo</reader-response>");
    }

    #[test]
    fn test_editor_output_regex_applies_reasoning_inside_think_blocks() {
        let mut script = mock_regex_script(
            "reasoning-redact",
            r"secret",
            "hidden",
            RegexPlacement::Reasoning,
        );
        script.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];
        script.flags = "g".into();

        let result = apply_editor_output_regex(
            "before secret <think>secret plan</think> after secret",
            &[script],
        )
        .expect("reasoning regex should apply before output commit");

        assert_eq!(
            result,
            "before secret <think>hidden plan</think> after secret"
        );
    }

    #[test]
    fn test_editor_output_regex_runs_reasoning_before_output_regex() {
        let mut reasoning = mock_regex_script(
            "reasoning-first",
            r"secret",
            "hidden",
            RegexPlacement::Reasoning,
        );
        reasoning.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];
        reasoning.flags = "g".into();
        let mut output = mock_regex_script(
            "output-second",
            r"hidden plan",
            "visible summary",
            RegexPlacement::Output,
        );
        output.placement_codes = vec![ST_REGEX_PLACEMENT_AI_OUTPUT];

        let result = apply_editor_output_regex(
            "<think>secret plan</think> final secret",
            &[reasoning, output],
        )
        .expect("reasoning and output regex should compose");

        assert_eq!(result, "<think>visible summary</think> final secret");
    }

    #[test]
    fn test_context_output_regex_uses_persisted_target() {
        let mut script = mock_regex_script(
            "display-only-output",
            r"<data_block>.*</data_block>",
            "DISPLAY",
            RegexPlacement::Output,
        );
        script.markdown_only = Some(true);

        let result = apply_context_regex(
            "<data_block>hp=5</data_block>",
            &[script],
            RegexPlacement::Output,
        )
        .expect("display-only output regex should be skipped for persisted target");

        assert_eq!(result, "<data_block>hp=5</data_block>");
    }

    #[test]
    fn test_context_regex_treats_current_message_as_depth_zero() {
        let mut script = mock_regex_script(
            "older-only-output",
            r"draft",
            "rewritten",
            RegexPlacement::Output,
        );
        script.min_depth = Some(1);

        let result = apply_context_regex("draft", &[script], RegexPlacement::Output)
            .expect("current output regex should execute with depth zero");

        assert_eq!(result, "draft");
    }

    async fn setup_with_first_draft() -> (
        PipelineOrchestrator,
        Arc<ConversationStore>,
        Id,      // conversation_id
        Id,      // node_id
        PathBuf, // conv_dir（清理用）
    ) {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
        let conv_dir =
            std::env::temp_dir().join(format!("storyforge_test_regen_{}", uuid::Uuid::new_v4()));
        let conv_store = Arc::new(ConversationStore::new(conv_dir.clone()));
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let mut orchestrator = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx, None);

        let conv = conv_store.create(None, None);
        let ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, conv.id.clone());
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false); // sender 保活，避免误触发取消
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
        let ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, conv_id.clone());
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false); // sender 保活，避免误触发取消
        let result = orchestrator
            .regenerate(req, &ctx, event_tx, cancel_rx)
            .await;

        assert!(result.is_ok(), "重 roll 编剧应成功: {:?}", result.err());
        let (text, provenance) = result.unwrap();
        assert!(!text.is_empty());
        // hint 应记录进 Provenance
        assert_eq!(provenance.last_hint.as_deref(), Some("节奏太快"));

        // variant 保留语义：重 roll 后该 node 多 1 个 variant（分支），
        // 旧 variant 降级 Discarded（可切回），新 variant 设 active。
        let conv_after = conv_store.get(&conv_id).unwrap();
        let node_after = conv_after.find_node(&node_id).unwrap();
        assert_eq!(
            node_after.variants.len(),
            variants_before + 1,
            "重 roll 后应多 1 个 variant"
        );
        // active 切到新 variant
        assert_eq!(node_after.active_variant, node_after.variants.len() - 1);

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    #[tokio::test]
    async fn test_regenerate_output_regex_applies_before_variant() {
        let (mut orchestrator, conv_store, conv_id, node_id, conv_dir) =
            setup_with_first_draft().await;

        let req = RegenerateRequest {
            conversation_id: conv_id.clone(),
            node_id: node_id.clone(),
            targets: vec![PartialRollTarget::Editor],
            hint: None,
            seed: None,
        };
        let mut ctx =
            WritingContext::legacy(vec![mock_character("Seraphina")], None, conv_id.clone());
        ctx.regex_scripts = vec![mock_regex_script(
            "replace-regenerated-output",
            r"[\s\S]+",
            "REGEX_FILTERED_REGEN",
            RegexPlacement::Output,
        )];
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let (text, _provenance) = orchestrator
            .regenerate(req, &ctx, event_tx, cancel_rx)
            .await
            .expect("regenerate should succeed with output regex");

        assert_eq!(text, "REGEX_FILTERED_REGEN");
        let conv_after = conv_store.get(&conv_id).unwrap();
        // variant 保留语义：旧 node 仍在，新 variant 是 active
        let node_after = conv_after.find_node(&node_id).unwrap();
        assert_eq!(node_after.active().unwrap().content, "REGEX_FILTERED_REGEN");

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
        let ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, conv_id.clone());
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false); // sender 保活，避免误触发取消
        let result = orchestrator
            .regenerate(req, &ctx, event_tx, cancel_rx)
            .await;

        assert!(result.is_ok(), "整体重 roll 应成功: {:?}", result.err());
        let (_text, provenance) = result.unwrap();
        assert_eq!(provenance.last_hint.as_deref(), Some("角色 B 语气太冷"));
        assert_eq!(provenance.seed, 42);

        // variant 保留语义：旧 node 仍在，多 1 个 variant，active 切到新的
        let conv_after = conv_store.get(&conv_id).unwrap();
        let node_after = conv_after.find_node(&node_id).unwrap();
        assert_eq!(
            node_after.variants.len(),
            variants_before + 1,
            "重 roll 后应多 1 个 variant"
        );
        assert_eq!(node_after.active_variant, node_after.variants.len() - 1);

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
        let ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, conv_id.clone());
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false); // sender 保活，避免误触发取消
        let result = orchestrator
            .regenerate(req, &ctx, event_tx, cancel_rx)
            .await;

        assert!(result.is_err(), "只重导演却留旧子产出应被拒绝");
        match result.unwrap_err() {
            PipelineError::Regenerate(msg) => {
                assert!(
                    msg.contains("不匹配") || msg.contains("保留"),
                    "错误信息应说明原因: {msg}"
                );
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
        let ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, conv_id.clone());
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false); // sender 保活，避免误触发取消
        let result = orchestrator
            .regenerate(req, &ctx, event_tx, cancel_rx)
            .await;

        assert!(
            result.is_ok(),
            "重 roll 子 Agent 应成功: {:?}",
            result.err()
        );
        let (_text, provenance) = result.unwrap();
        assert_eq!(provenance.last_hint.as_deref(), Some("语气太冷"));

        // variant 保留语义：旧 node 仍在，多 1 个 variant，active 切到新的
        let conv_after = conv_store.get(&conv_id).unwrap();
        let node_after = conv_after.find_node(&node_id).unwrap();
        assert_eq!(
            node_after.variants.len(),
            variants_before + 1,
            "重 roll 后应多 1 个 variant"
        );
        assert_eq!(node_after.active_variant, node_after.variants.len() - 1);

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    // ─── P2 后处理流水线接入测试 ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_regenerate_multiple_subagents_preserves_character_mapping() {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::new(vec![
            MockScript {
                match_keyword: "Alpha".into(),
                response_content: "new-alpha".into(),
                tool_calls: vec![],
                stream: false,
            },
            MockScript {
                match_keyword: "Beta".into(),
                response_content: "new-beta".into(),
                tool_calls: vec![],
                stream: false,
            },
            MockScript {
                match_keyword: EDITOR_SYSTEM_PROMPT.lines().next().unwrap().into(),
                response_content: "edited multi target".into(),
                tool_calls: vec![],
                stream: false,
            },
        ]));
        let conv_dir = std::env::temp_dir().join(format!(
            "storyforge_test_multi_regen_{}",
            uuid::Uuid::new_v4()
        ));
        let conv_store = Arc::new(ConversationStore::new(conv_dir.clone()));
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let mut orchestrator = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx, None);

        let plan = Plan {
            scene_brief: "three-character scene".into(),
            subagent_tasks: ["Alpha", "Beta", "Gamma"]
                .into_iter()
                .map(|character_id| SubagentTask {
                    character_id: character_id.into(),
                    brief: format!("{character_id} task"),
                    context_package: ContextPackage {
                        character_brief: format!("{character_id} brief"),
                        scene_brief: "three-character scene".into(),
                        relevant_lore: vec![],
                        constant_lore: vec![],
                        recent_window: vec![],
                        task: format!("{character_id} task"),
                    },
                    current_desire: None,
                    ongoing_action: None,
                    emotion_stage: None,
                })
                .collect(),
            scene_plan: None,
        };
        let old_snapshot = |character_id: &str, full_text: &str| {
            storyforge_domain::conversation::SubagentSnapshot {
                character_id: character_id.into(),
                full_text: full_text.into(),
                character_instance_id: None,
                display_name: None,
                fallback_reason: None,
            }
        };
        let provenance = Provenance {
            session_id: Id::new(),
            plan: Some(plan),
            subagent_results: vec![
                old_snapshot("Alpha", "old-alpha"),
                old_snapshot("Beta", "old-beta"),
                old_snapshot("Gamma", "old-gamma"),
            ],
            profile_id: None,
            seed: 7,
            last_hint: None,
        };

        let conv = conv_store.create(None, None);
        let node_id = conv_store
            .append_ai_draft(&conv.id, "old draft".into(), Some(provenance))
            .unwrap();
        let req = RegenerateRequest {
            conversation_id: conv.id.clone(),
            node_id: node_id.clone(),
            targets: vec![
                PartialRollTarget::Subagent("Beta".into()),
                PartialRollTarget::Subagent("Alpha".into()),
            ],
            hint: None,
            seed: None,
        };
        let ctx = WritingContext::legacy(vec![mock_character("Alpha")], None, conv.id.clone());
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let (_text, provenance) = orchestrator
            .regenerate(req, &ctx, event_tx, cancel_rx)
            .await
            .expect("multi-subagent regenerate should succeed");

        let subagents = &provenance.subagent_results;
        assert_eq!(subagents[0].character_id, "Alpha");
        assert_eq!(subagents[0].full_text, "new-alpha");
        assert_eq!(subagents[1].character_id, "Beta");
        assert_eq!(subagents[1].full_text, "new-beta");
        assert_eq!(subagents[2].character_id, "Gamma");
        assert_eq!(subagents[2].full_text, "old-gamma");

        let _ = std::fs::remove_dir_all(&conv_dir);
    }

    fn make_orchestrator() -> (PipelineOrchestrator, Arc<ConversationStore>) {
        let llm = Arc::new(MockLlmClient::with_defaults()) as Arc<dyn LlmClient>;
        let conv_dir = std::env::temp_dir().join(format!("sf_postproc_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&conv_dir).unwrap();
        let conv_store = Arc::new(ConversationStore::new(conv_dir));
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let orch = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx, None);
        (orch, conv_store)
    }

    struct FakeMvuRuntime {
        calls:
            std::sync::Mutex<Vec<(String, std::collections::HashMap<String, serde_json::Value>)>>,
        variable_updates: std::collections::HashMap<String, serde_json::Value>,
    }

    impl FakeMvuRuntime {
        fn new(variable_updates: std::collections::HashMap<String, serde_json::Value>) -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
                variable_updates,
            }
        }

        fn calls(&self) -> Vec<(String, std::collections::HashMap<String, serde_json::Value>)> {
            self.calls.lock().unwrap_or_else(|p| p.into_inner()).clone()
        }
    }

    #[async_trait::async_trait]
    impl MvuRuntime for FakeMvuRuntime {
        async fn execute_fragment(
            &self,
            fragment_js: &str,
            current_variables: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<
            storyforge_infra_plugin_host::mvu_runtime::MvuExecResult,
            storyforge_infra_plugin_host::mvu_runtime::MvuRuntimeError,
        > {
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((fragment_js.to_string(), current_variables.clone()));
            Ok(storyforge_infra_plugin_host::mvu_runtime::MvuExecResult {
                variable_updates: self.variable_updates.clone(),
                side_effects: vec!["fake-side-effect".into()],
            })
        }

        async fn load_card_assets(
            &self,
            _html: Option<&str>,
            _css: Option<&str>,
            _js: Option<&str>,
        ) -> Result<(), storyforge_infra_plugin_host::mvu_runtime::MvuRuntimeError> {
            Ok(())
        }

        async fn unload_card(
            &self,
        ) -> Result<(), storyforge_infra_plugin_host::mvu_runtime::MvuRuntimeError> {
            Ok(())
        }

        fn is_available(&self) -> bool {
            true
        }
    }

    fn make_orchestrator_with_mvu_runtime(
        mvu_runtime: Arc<dyn MvuRuntime + Send + Sync>,
    ) -> (PipelineOrchestrator, Arc<ConversationStore>) {
        let llm = Arc::new(MockLlmClient::with_defaults()) as Arc<dyn LlmClient>;
        let conv_dir =
            std::env::temp_dir().join(format!("sf_postproc_mvu_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&conv_dir).unwrap();
        let conv_store = Arc::new(ConversationStore::new(conv_dir));
        let tool_ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let orch = PipelineOrchestrator::new(llm, conv_store.clone(), tool_ctx, Some(mvu_runtime));
        (orch, conv_store)
    }

    /// 无 campaign（campaign_id=None）→ run_postprocess 返回 None（向后兼容，跳过后处理）
    #[tokio::test]
    async fn test_postprocess_skipped_without_campaign() {
        let (orch, conv_store) = make_orchestrator();
        let ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_tx, cancel) = watch::channel(false);

        let outcome = orch
            .run_postprocess(
                "成文内容",
                "场景简述",
                &["林医生".into()],
                &["hp".into()],
                &ctx,
                &event_tx,
                cancel,
                &[],
            )
            .await;
        assert!(outcome.is_none(), "无 campaign 应跳过后处理");
    }

    #[tokio::test]
    async fn test_postprocess_executes_mvu_fallback_fragments_into_variable_updates() {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::variables::VariableValue;

        let mut js_updates = std::collections::HashMap::new();
        js_updates.insert("mvu_hp".into(), serde_json::json!(41));
        let fake_runtime = Arc::new(FakeMvuRuntime::new(js_updates));
        let (orch, conv_store) = make_orchestrator_with_mvu_runtime(
            fake_runtime.clone() as Arc<dyn MvuRuntime + Send + Sync>
        );

        let card_id = Id::new();
        let def = CharacterDefinition {
            id: Id::new(),
            card_id: card_id.clone(),
            name: "Seraphina".into(),
            persona_prompt: String::new(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        };
        let mut inst = CharacterInstance::from_definition(Id::new(), &def);
        inst.variables = vec![VariableValue::new("hp", serde_json::json!(80), 1)];

        let campaign = Campaign::new(card_id, "MVU fallback test");
        let campaign_id = campaign.id.clone();
        let runtime = Arc::new(
            storyforge_domain::campaign_runtime::CampaignRuntimeContext {
                campaign,
                instances: vec![inst],
                definitions_by_id: std::collections::HashMap::new(),
                knowledge: vec![],
                tasks: vec![],
                turn: 1,
            },
        );
        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.campaign_id = Some(campaign_id);
        ctx.campaign_runtime = Some(runtime);
        ctx.turn = 1;

        let fragments = vec![FallbackFragment {
            description: "fake status math".into(),
            js_snippet: "variables.mvu_hp = variables.hp - 39;".into(),
            reason: "exercise runtime bridge".into(),
        }];
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_tx, cancel) = watch::channel(false);

        let outcome = orch
            .run_postprocess(
                "final text",
                "scene",
                &["Seraphina".into()],
                &["hp".into()],
                &ctx,
                &event_tx,
                cancel,
                &fragments,
            )
            .await
            .expect("campaign postprocess should run");

        let calls = fake_runtime.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, fragments[0].js_snippet);
        assert_eq!(calls[0].1.get("hp"), Some(&serde_json::json!(80)));

        let pp = outcome
            .post_process
            .expect("MVU fallback updates should create postprocess output");
        assert!(
            pp.variable_updates
                .iter()
                .any(|update| update.instance_id.is_none()
                    && update.key == "mvu_hp"
                    && update.value == serde_json::json!(41)),
            "MVU fallback variable update should be appended to postprocess outcome"
        );
    }

    /// 有 campaign → run_postprocess 并行跑总结 + 后处理，返回 Some(outcome)，两件产出非空
    #[tokio::test]
    async fn test_postprocess_runs_with_campaign() {
        let (orch, conv_store) = make_orchestrator();
        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.campaign_id = Some(Id::new());
        ctx.turn = 3;
        ctx.story_clock = "第2天".into();

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_tx, cancel) = watch::channel(false);

        let outcome = orch
            .run_postprocess(
                "林医生走进急诊室，看到陈警官带来一具尸体。",
                "急诊室",
                &["林医生".into(), "陈警官".into()],
                &["hp".into(), "state".into()],
                &ctx,
                &event_tx,
                cancel,
                &[],
            )
            .await;

        let outcome = outcome.expect("有 campaign 应跑后处理");
        // mock 脚本：summary 应非空，post_process 三件套应非空
        assert!(outcome.summary.is_some(), "mock 总结应产出非空");
        let pp = outcome
            .post_process
            .as_ref()
            .expect("mock 后处理应产出非空");
        assert!(
            !pp.is_empty(),
            "mock 后处理三件套应非空，实际 知识{} 变量{} 任务{}",
            pp.knowledge_updates.len(),
            pp.variable_updates.len(),
            pp.task_updates.len()
        );

        // 验证事件序列：PostProcessStarted 在前
        let mut events = vec![];
        while let Ok(e) = event_rx.try_recv() {
            events.push(e);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PipelineEvent::PostProcessStarted)),
            "应有 PostProcessStarted"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PipelineEvent::PostProcessDone { .. })),
            "应有 PostProcessDone"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PipelineEvent::SummaryDone { .. })),
            "应有 SummaryDone"
        );
    }

    /// AgentProfileConfig 把 enable_postprocess/enable_summarizer 都关 → 不调 LLM，
    /// 不发 PostProcessStarted，发 PostProcessSkipped，不发误导性的 PostProcessFailed。
    #[tokio::test]
    async fn test_postprocess_both_disabled_emits_skipped_not_failed() {
        use std::collections::HashMap;
        use storyforge_domain::agent_profile_config::AgentProfileConfig;
        use storyforge_domain::prompt_module::ProfileSource;

        let (orch, conv_store) = make_orchestrator();
        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.campaign_id = Some(Id::new());
        ctx.agent_profile_config = Some(AgentProfileConfig::new(
            Id::from_str("both-disabled-test"),
            "test".into(),
            String::new(),
            HashMap::new(),
            4,
            false, // enable_postprocess
            false, // enable_summarizer
            ProfileSource::UserCreated,
            1,
        ));

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_tx, cancel) = watch::channel(false);

        let outcome = orch
            .run_postprocess(
                "成文",
                "场景",
                &["林医生".into()],
                &["hp".into()],
                &ctx,
                &event_tx,
                cancel,
                &[],
            )
            .await;

        // 两者都关 → 安静跳过（注意：run_postprocess 的 None 语义是「无 campaign 或全关跳过」）
        assert!(outcome.is_none(), "两者都关应返回 None");

        let mut events = vec![];
        while let Ok(e) = event_rx.try_recv() {
            events.push(e);
        }
        // 不应发 PostProcessStarted
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, PipelineEvent::PostProcessStarted)),
            "全关时不应发 PostProcessStarted"
        );
        // 不应发 PostProcessFailed（区别于真失败）
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, PipelineEvent::PostProcessFailed { .. })),
            "全关时不应发 PostProcessFailed（应发 Skipped）"
        );
        // 应发 PostProcessSkipped
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PipelineEvent::PostProcessSkipped { .. })),
            "全关时应发 PostProcessSkipped，实际事件: {events:?}"
        );
    }

    /// 有 campaign + 任务待注入 → build_director_tail 含任务块（§22：任务压在 volatile tail）
    #[test]
    fn test_director_tail_includes_pending_tasks() {
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::story_task::{StoryTask, TaskTrigger};
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_task_msg_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.turn = 10; // 故意设大，确保 TurnReminder(at_turn=1) 触发
        ctx.pending_tasks = vec![StoryTask::user_planned(
            Id::new(),
            "老王复仇",
            "三个月期限到了",
            vec![TaskTrigger::TurnReminder { at_turn: 1 }],
            0,
        )];

        // 构造完整 layout，取 tail（最后一条 user 消息）检查
        let layout = MessageLayout::build()
            .system("你是导演")
            .tail(|_| build_director_tail("写一场戏", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();
        assert!(
            tail_content.contains("老王复仇"),
            "导演 tail 应含待注入任务: {tail_content}"
        );
        assert!(tail_content.contains("即将触发"), "应有任务注入块标题");

        // §22：任务应在 tail（易变段），不在 system（稳定段）
        let system_content = msgs.first().unwrap().content.as_str();
        assert!(!system_content.contains("老王复仇"), "任务不应进 system 段");
    }

    /// 蓝灯世界设定进 system（稳定段），不进 tail（§22.4）
    #[test]
    fn test_director_lore_in_system_not_tail() {
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_lore_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let book = Arc::new(WorldInfoBook {
            source: storyforge_domain::Source::Native,
            entries: vec![WorldInfoEntry {
                st_id: Some(1),
                keys: vec!["龙".into()],
                secondary_keys: vec![],
                content: "龙族设定详情".into(),
                constant: true,
                selective: false,
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 2,
                order: 100,
                route: LoreRoute::Constant,
                extensions: serde_json::json!({}),
            }],
            metadata: Default::default(),
        });
        let ctx = WritingContext::legacy(vec![], Some(book), conv_store.create(None, None).id);

        let system_extra = build_director_system_extra(&ctx);
        assert!(
            system_extra.contains("龙族设定详情"),
            "蓝灯应进 system_extra"
        );

        // tail 不应含蓝灯内容
        let layout = MessageLayout::build()
            .system(&system_extra)
            .tail(|_| build_director_tail("写一场戏", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();
        assert!(!tail_content.contains("龙族设定详情"), "蓝灯不应进 tail");
    }

    #[test]
    fn test_director_system_applies_world_info_regex_without_mutating_book() {
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        let book = Arc::new(WorldInfoBook {
            source: storyforge_domain::Source::Native,
            entries: vec![WorldInfoEntry {
                st_id: Some(1),
                keys: vec!["dragon".into()],
                secondary_keys: vec![],
                content: "Dragon note: {{DRAGON}}".into(),
                constant: true,
                selective: false,
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 2,
                order: 100,
                route: LoreRoute::Constant,
                extensions: serde_json::json!({}),
            }],
            metadata: Default::default(),
        });
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_lore_regex_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let mut ctx =
            WritingContext::legacy(vec![], Some(book.clone()), conv_store.create(None, None).id);
        ctx.regex_scripts = vec![mock_regex_script(
            "world-info-format",
            r"\{\{DRAGON\}\}",
            "Aurelion",
            RegexPlacement::WorldInfo,
        )];

        let system_extra = build_director_system_extra(&ctx);

        assert!(system_extra.contains("Dragon note: Aurelion"));
        assert!(!system_extra.contains("{{DRAGON}}"));
        assert_eq!(book.entries[0].content, "Dragon note: {{DRAGON}}");
    }

    #[test]
    fn test_director_config_marks_emit_plan_as_terminal() {
        // emit_plan 必须是终止工具，否则 LLM 调用 emit_plan 后 run_tool_loop 不终止，
        // 循环到 max_tool_rounds(15) 失败。content 的 JSON 完成探测兜不住 tool_call 路径
        // （LLM 走 emit_plan 时 content 是自然语言）。对齐 postprocess 的同源修复。
        let cfg = make_director_config(None, &[], "", None, None, &Default::default());
        assert!(
            cfg.terminal_tools.iter().any(|t| t == "emit_plan"),
            "terminal_tools 必须含 emit_plan，实际为 {:?}",
            cfg.terminal_tools
        );
    }

    #[test]
    fn test_editor_prompt_forbids_meta_commentary() {
        // Editor prompt 历史上含「3. 标注哪些子表演被你裁剪/改动了」，主动要求 LLM 输出元描述，
        // 导致元描述混入正文（P2-4）。必须改为显式禁止元描述，且不再要求标注改动。
        // 防止日后误改回旧文本。
        let cfg = make_editor_config(None, &[], None, None, &Default::default());
        assert!(
            !cfg.system_prompt.contains("标注哪些子表演被你裁剪/改动了"),
            "editor prompt 不应再要求标注改动，实际为:\n{}",
            cfg.system_prompt
        );
        assert!(
            cfg.system_prompt.contains("严禁") && cfg.system_prompt.contains("说明"),
            "editor prompt 必须显式禁止输出说明性文字，实际为:\n{}",
            cfg.system_prompt
        );
        assert!(
            cfg.system_prompt.contains("第一行"),
            "editor prompt 必须要求第一行就是正文，实际为:\n{}",
            cfg.system_prompt
        );
    }

    /// §22 cache 命中验证：两轮调用（不同 intent/turn）但相同蓝灯世界设定 →
    /// system 段指纹一致（cache 命中），只有 tail 变。
    #[test]
    fn test_director_system_stable_across_rounds() {
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_cache_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let book = Arc::new(WorldInfoBook {
            source: storyforge_domain::Source::Native,
            entries: vec![WorldInfoEntry {
                st_id: Some(1),
                keys: vec!["龙".into()],
                secondary_keys: vec![],
                content: "稳定的世界设定".into(),
                constant: true,
                selective: false,
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 2,
                order: 100,
                route: LoreRoute::Constant,
                extensions: serde_json::json!({}),
            }],
            metadata: Default::default(),
        });

        // 第 1 轮：intent=A，turn=1
        let mut ctx1 =
            WritingContext::legacy(vec![], Some(book.clone()), conv_store.create(None, None).id);
        ctx1.turn = 1;
        let layout1 = MessageLayout::build()
            .system(build_director_system_extra(&ctx1))
            .tail(|_| build_director_tail("意图A", &ctx1));

        // 第 2 轮：intent=B（完全不同），turn=5
        let mut ctx2 =
            WritingContext::legacy(vec![], Some(book.clone()), conv_store.create(None, None).id);
        ctx2.turn = 5;
        let layout2 = MessageLayout::build()
            .system(build_director_system_extra(&ctx2))
            .tail(|_| build_director_tail("完全不同的意图B", &ctx2));

        // 两轮 system 指纹必须一致（蓝灯相同 → cache 命中）
        assert_eq!(
            layout1.prefix_fingerprint(),
            layout2.prefix_fingerprint(),
            "相同蓝灯 → system 指纹应一致（cache 友好）"
        );
    }

    #[test]
    fn test_director_tail_injects_triggered_selective_world_info() {
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        let book = Arc::new(WorldInfoBook {
            source: storyforge_domain::Source::Native,
            entries: vec![
                WorldInfoEntry {
                    st_id: None,
                    keys: vec!["always".into()],
                    secondary_keys: vec![],
                    content: "CONSTANT_SYSTEM_LORE".into(),
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
                    st_id: None,
                    keys: vec!["moon vault".into()],
                    secondary_keys: vec![],
                    content: "LUNAR_VAULT_LORE".into(),
                    constant: false,
                    selective: true,
                    selective_logic: SelectiveLogic::And,
                    disabled: false,
                    position: 0,
                    depth: 4,
                    order: 200,
                    route: LoreRoute::Selective,
                    extensions: serde_json::json!({}),
                },
                WorldInfoEntry {
                    st_id: None,
                    keys: vec!["sun gate".into()],
                    secondary_keys: vec![],
                    content: "SUN_GATE_LORE".into(),
                    constant: false,
                    selective: true,
                    selective_logic: SelectiveLogic::And,
                    disabled: false,
                    position: 0,
                    depth: 4,
                    order: 100,
                    route: LoreRoute::Selective,
                    extensions: serde_json::json!({}),
                },
            ],
            metadata: Default::default(),
        });

        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_selective_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(vec![], Some(book), conv_store.create(None, None).id);

        let system_extra = build_director_system_extra(&ctx);
        let layout = MessageLayout::build()
            .system(system_extra.clone())
            .tail(|_| build_director_tail("open the moon vault", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(system_extra.contains("CONSTANT_SYSTEM_LORE"));
        assert!(!system_extra.contains("LUNAR_VAULT_LORE"));
        assert!(tail_content.contains("LUNAR_VAULT_LORE"));
        assert!(!tail_content.contains("SUN_GATE_LORE"));
    }

    #[test]
    fn test_director_tail_applies_world_info_regex_to_triggered_selective_lore() {
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        let book = Arc::new(WorldInfoBook {
            source: storyforge_domain::Source::Native,
            entries: vec![WorldInfoEntry {
                st_id: None,
                keys: vec!["moon vault".into()],
                secondary_keys: vec![],
                content: "Triggered: {{VAULT}}".into(),
                constant: false,
                selective: true,
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 4,
                order: 100,
                route: LoreRoute::Selective,
                extensions: serde_json::json!({}),
            }],
            metadata: Default::default(),
        });
        let conv_store = {
            let dir =
                std::env::temp_dir().join(format!("sf_selective_regex_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let mut ctx = WritingContext::legacy(vec![], Some(book), conv_store.create(None, None).id);
        ctx.regex_scripts = vec![mock_regex_script(
            "world-info-format",
            r"\{\{VAULT\}\}",
            "Lunar Vault",
            RegexPlacement::WorldInfo,
        )];

        let layout = MessageLayout::build()
            .system(build_director_system_extra(&ctx))
            .tail(|_| build_director_tail("open the moon vault", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(tail_content.contains("Triggered: Lunar Vault"));
        assert!(!tail_content.contains("{{VAULT}}"));
    }

    #[test]
    fn test_director_tail_omits_untriggered_selective_world_info() {
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        let book = Arc::new(WorldInfoBook {
            source: storyforge_domain::Source::Native,
            entries: vec![WorldInfoEntry {
                st_id: None,
                keys: vec!["moon vault".into()],
                secondary_keys: vec![],
                content: "LUNAR_VAULT_LORE".into(),
                constant: false,
                selective: true,
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 2,
                order: 100,
                route: LoreRoute::Selective,
                extensions: serde_json::json!({}),
            }],
            metadata: Default::default(),
        });

        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_selective_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(vec![], Some(book), conv_store.create(None, None).id);

        let layout = MessageLayout::build()
            .system(build_director_system_extra(&ctx))
            .tail(|_| build_director_tail("write a quiet market scene", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(!tail_content.contains("LUNAR_VAULT_LORE"));
    }

    #[test]
    fn test_director_lore_both_route_enters_system_and_triggered_tail() {
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        let book = Arc::new(WorldInfoBook {
            source: storyforge_domain::Source::Native,
            entries: vec![WorldInfoEntry {
                st_id: None,
                keys: vec!["harbor".into()],
                secondary_keys: vec![],
                content: "BOTH_ROUTE_LORE".into(),
                constant: true,
                selective: true,
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 2,
                order: 100,
                route: LoreRoute::Both,
                extensions: serde_json::json!({}),
            }],
            metadata: Default::default(),
        });

        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_both_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(vec![], Some(book), conv_store.create(None, None).id);

        let system_extra = build_director_system_extra(&ctx);
        let layout = MessageLayout::build()
            .system(system_extra.as_str())
            .tail(|_| build_director_tail("sail to the harbor", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(
            system_extra.contains("BOTH_ROUTE_LORE"),
            "Both route lore should enter stable Director system context: {system_extra}"
        );
        assert!(
            tail_content.contains("BOTH_ROUTE_LORE"),
            "Both route lore should also enter triggered Director tail: {tail_content}"
        );
    }

    // ─── 阶段 3：Director tail 消费 campaign_runtime 测试 ────────────────────

    /// 无 campaign_runtime 时，build_director_tail 仍用旧的扁平角色名
    #[test]
    fn test_director_tail_uses_flat_characters_when_no_runtime() {
        use storyforge_domain::message_layout::MessageLayout;
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_flat_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(
            vec![mock_character("Seraphina"), mock_character("Lin")],
            None,
            conv_store.create(None, None).id,
        );

        let layout = MessageLayout::build()
            .system("你是导演")
            .tail(|_| build_director_tail("写一场戏", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(
            tail_content.contains("Seraphina"),
            "应含角色名: {tail_content}"
        );
        assert!(tail_content.contains("Lin"), "应含角色名: {tail_content}");
        // 无 campaign_runtime 时不应出现 instance_id 格式
        assert!(
            !tail_content.contains("Campaign 实例"),
            "不应出现 Campaign 实例标题: {tail_content}"
        );
    }

    /// 有 campaign_runtime 时，build_director_tail 从 instances 渲染（含 id/role/persona）
    #[test]
    fn test_director_tail_uses_campaign_instances() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign::CharacterInstance;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::variables::default_character_variables;

        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_camp_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };

        let campaign = Campaign::new(Id::from_str("card-1"), "test-campaign");
        let def = CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm surgeon who saves lives".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec!["is a surgeon".into()],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        let instance = CharacterInstance {
            id: Id::from_str("inst-lin"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def.id.clone()),
            name: "Lin".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        let mut definitions_by_id = std::collections::HashMap::new();
        definitions_by_id.insert(def.id.clone(), def);

        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id,
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        });

        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.campaign_runtime = Some(runtime);
        ctx.recent_summaries = vec![
            storyforge_domain::agent::RoundSummary::new(
                Id::from_str("camp-1"),
                Id::from_str("conv-1"),
                1,
                "林秋在雨夜诊所发现了未署名的病历。".into(),
            ),
            storyforge_domain::agent::RoundSummary::new(
                Id::from_str("camp-1"),
                Id::from_str("conv-1"),
                2,
                "陈警官上门问询，林秋隐瞒了部分线索。".into(),
            ),
        ];

        let layout = MessageLayout::build()
            .system("你是导演")
            .tail(|_| build_director_tail("写一场戏", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(
            tail_content.contains("Campaign 实例"),
            "应出现 Campaign 实例标题: {tail_content}"
        );
        assert!(
            tail_content.contains("inst-lin"),
            "应含 instance id: {tail_content}"
        );
        // M2：近 H 轮摘要进 history 前缀，不在 tail 双税
        assert!(
            !tail_content.contains("近期剧情摘要"),
            "近窗摘要不应再进 tail: {tail_content}"
        );
        assert!(
            !tail_content.contains("陈警官上门问询"),
            "近窗摘要正文不应出现在 tail: {tail_content}"
        );
        let part = partition_summaries_for_prompt(&ctx.recent_summaries, 5, 10, 200);
        assert!(part.near_turns.contains(&1) && part.near_turns.contains(&2));
        assert!(tail_content.contains("Lin"), "应含角色名: {tail_content}");
        assert!(
            tail_content.contains("Protagonist"),
            "应含 role_type: {tail_content}"
        );
        assert!(
            tail_content.contains("calm surgeon"),
            "应含 persona 摘要: {tail_content}"
        );
    }

    /// 阶段 C 契约：原始 history 在 stable_history；远记忆只进 volatile tail，
    /// 不挤占 history 窗口（MessageLayout 物理顺序）。
    #[test]
    fn test_history_not_displaced_by_far_memory() {
        use storyforge_domain::llm::{ChatMessage, ChatRole};
        use storyforge_domain::message_layout::MessageLayout;

        let history = vec![
            ChatMessage {
                role: ChatRole::User,
                content: "原始用户消息".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: ChatRole::Assistant,
                content: "原始 AI 消息".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let far = vec![FarMemoryHit::new(
            "fm1",
            "远记忆不应出现在 history",
            0.99,
            "ArchivedSummary",
        )];
        let far_block = render_far_memory_for_injection(&far, FAR_MEMORY_INJECT_LIMIT).unwrap();
        let layout = MessageLayout::build()
            .system("stable system")
            .history(history.clone())
            .tail(|_| {
                storyforge_domain::message_layout::VolatileTail::new()
                    .push("当前意图")
                    .push(far_block)
            });
        let msgs = layout.into_messages();
        // system + 2 history + 1 tail
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, ChatRole::System);
        assert_eq!(msgs[1].content, "原始用户消息");
        assert_eq!(msgs[2].content, "原始 AI 消息");
        assert_eq!(msgs[3].role, ChatRole::User);
        assert!(msgs[3].content.contains("远记忆"));
        assert!(!msgs[1].content.contains("远记忆"));
        assert!(!msgs[2].content.contains("远记忆"));
        // 前缀指纹只看 system+history，tail 含远记忆不影响 prefix
        let layout_a = MessageLayout::build()
            .system("stable system")
            .history(history.clone())
            .tail(|_| storyforge_domain::message_layout::VolatileTail::new().push("意图A"));
        let layout_b = MessageLayout::build()
            .system("stable system")
            .history(history)
            .tail(|_| {
                storyforge_domain::message_layout::VolatileTail::new()
                    .push("意图B")
                    .push(render_far_memory_for_injection(&far, FAR_MEMORY_INJECT_LIMIT).unwrap())
            });
        assert_eq!(
            layout_a.prefix_fingerprint(),
            layout_b.prefix_fingerprint(),
            "far memory in tail must not change prefix fingerprint"
        );
    }

    // ─── ContextCompiler 最小版：RoundSummary 注入 ───────────────────────────

    #[test]
    fn test_render_recent_summaries_for_injection_takes_last_n() {
        let summaries = vec![
            storyforge_domain::agent::RoundSummary::new(
                Id::from_str("c"),
                Id::from_str("conv"),
                1,
                "第一轮".into(),
            ),
            storyforge_domain::agent::RoundSummary::new(
                Id::from_str("c"),
                Id::from_str("conv"),
                2,
                "第二轮".into(),
            ),
            storyforge_domain::agent::RoundSummary::new(
                Id::from_str("c"),
                Id::from_str("conv"),
                3,
                "第三轮".into(),
            ),
        ];
        let text = render_recent_summaries_for_injection(&summaries, 2).expect("should render");
        assert!(text.contains("T2:") && text.contains("第二轮"));
        assert!(text.contains("T3:") && text.contains("第三轮"));
        assert!(!text.contains("T1:"));
    }

    #[test]
    fn test_render_recent_summaries_empty_returns_none() {
        assert!(render_recent_summaries_for_injection(&[], 5).is_none());
    }

    #[test]
    fn test_partition_summaries_near_band_overview() {
        let mut summaries = Vec::new();
        for t in 1..=20u32 {
            summaries.push(
                storyforge_domain::agent::RoundSummary::new(
                    Id::from_str("c"),
                    Id::from_str("v"),
                    t,
                    format!("事件{t}"),
                )
                .with_code(format!("A{t:04}"))
                .with_headline(format!("头{t}")),
            );
        }
        let p = partition_summaries_for_prompt(&summaries, 5, 10, 200);
        assert_eq!(p.near_turns, vec![16, 17, 18, 19, 20]);
        assert_eq!(p.band_turns.first(), Some(&6));
        assert_eq!(p.band_turns.last(), Some(&15));
        assert_eq!(p.band_lines.len(), 10);
        assert_eq!(p.overview_lines.len(), 5);
        assert!(p.overview_lines[0].contains("A0001") || p.overview_lines[0].contains("头1"));
        let hist = prepend_chronicle_history_prefix(vec![], &p);
        assert_eq!(hist.len(), 2);
        assert!(hist[0].content.contains("事件概览"));
        assert!(hist[1].content.contains("中距纪要"));
    }

    #[test]
    fn test_filter_summaries_excluding_near_turns() {
        let summaries = vec![
            storyforge_domain::agent::RoundSummary::new(
                Id::from_str("c"),
                Id::from_str("v"),
                1,
                "a".into(),
            ),
            storyforge_domain::agent::RoundSummary::new(
                Id::from_str("c"),
                Id::from_str("v"),
                2,
                "b".into(),
            ),
        ];
        let f = filter_summaries_excluding_turns(&summaries, &[2]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].turn, 1);
    }

    #[test]
    fn test_render_far_memory_for_injection() {
        assert!(render_far_memory_for_injection(&[], 3).is_none());
        let hits = vec![
            FarMemoryHit::new("a1", "昨夜潜入诊所", 0.9, "ArchivedSummary"),
            FarMemoryHit::new("a2", "陈警官上门", 0.8, "ArchivedSummary"),
        ];
        let text = render_far_memory_for_injection(&hits, 3).expect("hits");
        assert!(text.contains("远记忆召回"));
        assert!(text.contains("昨夜潜入诊所"));
        assert!(text.contains("陈警官上门"));
        // 注入文本不含溯源 id，避免污染模型上下文
        assert!(!text.contains("a1"));
    }

    #[test]
    fn test_render_far_memory_excludes_recent_overlap() {
        let hits = vec![
            FarMemoryHit::from_content("昨夜有人潜入诊所，林秋藏起病历。"),
            FarMemoryHit::from_content("陈警官次日上门调查。"),
        ];
        let exclude = vec!["昨夜有人潜入诊所，林秋藏起病历。".into()];
        let text =
            render_far_memory_for_injection_excluding(&hits, 3, &exclude).expect("should keep one");
        assert!(!text.contains("林秋藏起病历"), "重叠摘要应被剔除: {text}");
        assert!(text.contains("陈警官次日上门"), "非重叠应保留: {text}");
    }

    #[test]
    fn test_editor_tail_includes_recent_summaries() {
        use storyforge_domain::message_layout::MessageLayout;

        let summaries = vec![storyforge_domain::agent::RoundSummary::new(
            Id::from_str("c"),
            Id::from_str("conv"),
            3,
            "林秋把病历藏进抽屉。".into(),
        )];
        let layout = MessageLayout::build().system("你是编剧").tail(|_| {
            build_editor_tail(
                "雨夜诊所",
                "### Lin\n林秋沉默。",
                None,
                &summaries,
                &[],
                None,
                None,
            )
        });
        let msgs = layout.into_messages();
        let tail = msgs.last().unwrap().content.as_str();
        assert!(tail.contains("场景：雨夜诊所"), "应含场景: {tail}");
        assert!(tail.contains("近期剧情摘要"), "应含摘要段: {tail}");
        assert!(tail.contains("林秋把病历藏进抽屉"), "应含摘要正文: {tail}");
        assert!(
            tail.contains("保持与上述摘要一致"),
            "应提示保持一致性: {tail}"
        );
    }

    // ─── 阶段 3 cleanup：has_available_characters + UTF-8 截断 + variables 注入 ──

    /// has_available_characters：旧路径 - characters 非空 → true

    #[test]
    fn test_parse_plan_json_scene_plan_and_agency_fields() {
        let v = serde_json::json!({
            "scene_brief": "雨夜诊所",
            "scene_plan": {
                "conflict": "是否透露真相",
                "opposing_goals": ["林想隐瞒", "陈想追问"],
                "stakes": "信任破裂",
                "beats": ["对峙", "犹豫"],
                "must_not_resolve": "主线谜底",
                "exit_hook": "雨未停"
            },
            "subagent_tasks": [{
                "character_id": "inst-lin",
                "brief": "诊治",
                "current_desire": "想快点结束问话",
                "ongoing_action": "擦手",
                "emotion_stage": 3
            }]
        });
        let plan = parse_plan_json(&v).expect("plan");
        assert_eq!(plan.scene_brief, "雨夜诊所");
        let sp = plan.scene_plan.expect("scene_plan");
        assert_eq!(sp.conflict.as_deref(), Some("是否透露真相"));
        assert_eq!(sp.must_not_resolve.as_deref(), Some("主线谜底"));
        assert_eq!(
            plan.subagent_tasks[0].current_desire.as_deref(),
            Some("想快点结束问话")
        );
        assert_eq!(
            plan.subagent_tasks[0].ongoing_action.as_deref(),
            Some("擦手")
        );
        assert_eq!(plan.subagent_tasks[0].emotion_stage, Some(3));
    }

    #[test]
    fn test_parse_plan_json_old_shape_still_works() {
        let v = serde_json::json!({
            "scene_brief": "旧场景",
            "subagent_tasks": [{"character_id": "A", "brief": "演"}]
        });
        let plan = parse_plan_json(&v).expect("old plan");
        assert!(plan.scene_plan.is_none());
        assert!(plan.subagent_tasks[0].current_desire.is_none());
    }

    #[test]
    fn test_parse_plan_json_rejects_out_of_range_emotion_stage() {
        let v = serde_json::json!({
            "scene_brief": "场景",
            "subagent_tasks": [{
                "character_id": "A",
                "brief": "演",
                "emotion_stage": 7
            }]
        });
        let plan = parse_plan_json(&v).expect("plan");
        assert!(plan.subagent_tasks[0].emotion_stage.is_none());

        let v2 = serde_json::json!({
            "scene_brief": "场景",
            "subagent_tasks": [{
                "character_id": "A",
                "brief": "演",
                "emotion_stage": 256
            }]
        });
        let plan2 = parse_plan_json(&v2).expect("plan2");
        assert!(plan2.subagent_tasks[0].emotion_stage.is_none());

        // 257 as u8 会截断成 1；必须先 try_from 再校验范围
        let v3 = serde_json::json!({
            "scene_brief": "场景",
            "subagent_tasks": [{
                "character_id": "A",
                "brief": "演",
                "emotion_stage": 257
            }]
        });
        let plan3 = parse_plan_json(&v3).expect("plan3");
        assert!(
            plan3.subagent_tasks[0].emotion_stage.is_none(),
            "257 must not truncate to stage 1"
        );
    }

    #[test]
    fn test_editor_tail_includes_scene_plan_and_contract() {
        let sp = storyforge_domain::agent::ScenePlan {
            conflict: Some("对峙".into()),
            must_not_resolve: Some("主线谜底".into()),
            ..Default::default()
        };
        let mut contract = storyforge_domain::narrative_contract::NarrativeContract::default();
        contract
            .must_not_reveal
            .push("SF_SECRET_CHEN_BADGE_X91".into());
        contract.focalizers.push("inst-lin".into());
        let summaries: Vec<storyforge_domain::agent::RoundSummary> = vec![];
        let tail = build_editor_tail(
            "雨夜诊所",
            "### Lin\n沉默。",
            None,
            &summaries,
            &[],
            Some(&sp),
            Some(&contract),
        );
        let rendered = tail.joined_content();
        assert!(
            rendered.contains("【场景规划 ScenePlan】") && rendered.contains("对峙"),
            "ScenePlan must be injected into editor tail: {rendered}"
        );
        assert!(
            rendered.contains("主线谜底"),
            "must_not_resolve must appear in editor tail: {rendered}"
        );
        assert!(
            rendered.contains("【叙事契约 NarrativeContract】"),
            "NarrativeContract header missing: {rendered}"
        );
        assert!(
            rendered.contains("限知第三人称") || rendered.contains("硬禁探针"),
            "contract body missing: {rendered}"
        );
        // Editor tail 不应倾倒完整自然语言 secret
        assert!(
            !rendered.contains("SF_SECRET_CHEN_BADGE_X91"),
            "editor tail must not dump raw secret: {rendered}"
        );
    }

    #[test]
    fn test_redact_performances_strips_non_owner_private_probe() {
        use storyforge_domain::narrative_contract::{NarrativeContract, PrivateBinding};
        let perfs = vec![
            storyforge_domain::agent::Performance {
                character_id: "inst-lin".into(),
                narrative: String::new(),
                dialogue: String::new(),
                inner_thoughts: String::new(),
                full_text: "我知道 SF_SECRET_CHEN_BADGE_X91".into(),
            },
            storyforge_domain::agent::Performance {
                character_id: "inst-chen".into(),
                narrative: String::new(),
                dialogue: String::new(),
                inner_thoughts: String::new(),
                full_text: "我的秘密是 SF_SECRET_CHEN_BADGE_X91".into(),
            },
        ];
        let contract = NarrativeContract {
            private_bindings: vec![PrivateBinding {
                owner_id: "inst-chen".into(),
                owner_name: Some("陈警官".into()),
                secret: "SF_SECRET_CHEN_BADGE_X91".into(),
            }],
            must_not_reveal: vec!["SF_SECRET_CHEN_BADGE_X91".into()],
            ..Default::default()
        };
        let redacted = redact_performances_for_editor(&perfs, Some(&contract));
        assert!(
            redacted.contains("[REDACTED_PRIVATE]"),
            "non-owner probe must redact: {redacted}"
        );
        // owner keeps raw
        assert!(
            redacted.contains("### inst-chen\n我的秘密是 SF_SECRET_CHEN_BADGE_X91"),
            "owner must keep secret: {redacted}"
        );
        // non-owner loses raw
        assert!(
            !redacted.contains("### inst-lin\n我知道 SF_SECRET_CHEN_BADGE_X91"),
            "non-owner must not keep raw: {redacted}"
        );
    }

    #[test]
    fn test_has_available_characters_flat_true() {
        let ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, Id::new());
        assert!(has_available_characters(&ctx));
    }

    /// has_available_characters：旧路径 - characters 空 → false
    #[test]
    fn test_has_available_characters_flat_false() {
        let ctx = WritingContext::legacy(vec![], None, Id::new());
        assert!(!has_available_characters(&ctx));
    }

    /// has_available_characters：Campaign 路径 - instances 非空 → true（即使 characters 空）
    #[test]
    fn test_has_available_characters_campaign_true() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign::CharacterInstance;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;

        let campaign = Campaign::new(Id::from_str("card-1"), "test");
        let instance = CharacterInstance {
            id: Id::from_str("inst-1"),
            campaign_id: campaign.id.clone(),
            definition_id: None,
            name: "Lin".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id: std::collections::HashMap::new(),
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        });

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_runtime = Some(runtime);
        assert!(
            has_available_characters(&ctx),
            "Campaign instances 非空应通过"
        );
    }

    /// has_available_characters：Campaign 路径 - instances 空 → false
    #[test]
    fn test_has_available_characters_campaign_empty() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;

        let campaign = Campaign::new(Id::from_str("card-1"), "test");
        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![],
            definitions_by_id: std::collections::HashMap::new(),
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        });

        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_runtime = Some(runtime);
        assert!(
            !has_available_characters(&ctx),
            "Campaign instances 空应不通过"
        );
    }

    /// 中文 persona 超过 80 字符时 build_director_tail 不 panic
    #[test]
    fn test_director_tail_chinese_persona_no_panic() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign::CharacterInstance;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::variables::default_character_variables;

        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_zh_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };

        // 构造一个超过 80 个中文字符的 persona
        let long_persona = "她是一位经验丰富的外科医生，性格冷静理性，面对紧急情况总能保持镇定。她相信医学的力量，但也深知生命的脆弱。在手术台上她是最可靠的搭档，在生活中她是最值得信赖的朋友。".to_string();
        assert!(
            long_persona.chars().count() > 80,
            "测试前提：persona 超过 80 字符"
        );

        let campaign = Campaign::new(Id::from_str("card-1"), "test");
        let def = CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: Id::from_str("card-1"),
            name: "林医生".into(),
            persona_prompt: long_persona.clone(),
            behavior_rules: "save first".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        let instance = CharacterInstance {
            id: Id::from_str("inst-lin"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def.id.clone()),
            name: "林医生".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        let mut definitions_by_id = std::collections::HashMap::new();
        definitions_by_id.insert(def.id.clone(), def);

        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id,
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        });

        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.campaign_runtime = Some(runtime);

        // 这里之前会 panic（按字节截断中文），现在应安全
        let layout = MessageLayout::build()
            .system("你是导演")
            .tail(|_| build_director_tail("写一场戏", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        // 验证截断后包含省略号和部分中文
        assert!(tail_content.contains("…"), "应有省略号截断: {tail_content}");
        assert!(
            tail_content.contains("林医生"),
            "应含角色名: {tail_content}"
        );
        // 验证不会包含完整 persona（因为被截断了）
        assert!(
            !tail_content.contains(&long_persona),
            "不应包含完整 persona"
        );
    }

    /// 有 campaign_runtime 时 director tail 包含 instance variables
    #[test]
    fn test_director_tail_includes_instance_variables() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::campaign::CharacterInstance;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
        use storyforge_domain::character::{CharacterDefinition, RoleType};
        use storyforge_domain::message_layout::MessageLayout;
        use storyforge_domain::variables::{VariableValue, default_character_variables};

        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_vars_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };

        let campaign = Campaign::new(Id::from_str("card-1"), "test");
        let def = CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm surgeon".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        let instance = CharacterInstance {
            id: Id::from_str("inst-lin"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def.id.clone()),
            name: "Lin".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![
                VariableValue {
                    key: "hp".into(),
                    value: serde_json::json!(80),
                    last_updated_turn: 1,
                },
                VariableValue {
                    key: "state".into(),
                    value: serde_json::json!("受伤"),
                    last_updated_turn: 1,
                },
                VariableValue {
                    key: "location".into(),
                    value: serde_json::json!("急诊室"),
                    last_updated_turn: 1,
                },
            ],
            is_temporary: false,
        };
        let mut definitions_by_id = std::collections::HashMap::new();
        definitions_by_id.insert(def.id.clone(), def);

        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id,
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        });

        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.campaign_runtime = Some(runtime);

        let layout = MessageLayout::build()
            .system("你是导演")
            .tail(|_| build_director_tail("写一场戏", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(
            tail_content.contains("hp:80"),
            "应含 hp 变量: {tail_content}"
        );
        assert!(
            tail_content.contains("state:受伤"),
            "应含 state 变量: {tail_content}"
        );
        assert!(
            tail_content.contains("location:急诊室"),
            "应含 location 变量: {tail_content}"
        );
    }

    /// 无 campaign_runtime 时旧扁平角色列表仍然可用
    #[test]
    fn test_director_tail_flat_characters_still_works() {
        use storyforge_domain::message_layout::MessageLayout;
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_flat2_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(
            vec![mock_character("Seraphina"), mock_character("Lin")],
            None,
            conv_store.create(None, None).id,
        );

        let layout = MessageLayout::build()
            .system("你是导演")
            .tail(|_| build_director_tail("写一场戏", &ctx));
        let msgs = layout.into_messages();
        let tail_content = msgs.last().unwrap().content.as_str();

        assert!(
            tail_content.contains("Seraphina"),
            "应含角色名: {tail_content}"
        );
        assert!(tail_content.contains("Lin"), "应含角色名: {tail_content}");
        assert!(
            !tail_content.contains("Campaign 实例"),
            "不应出现 Campaign 实例标题"
        );
        assert!(!tail_content.contains("hp:"), "旧路径不应含变量摘要");
    }

    #[test]
    fn test_prompt_template_context_uses_single_legacy_character() {
        let mut character = (*mock_character("Seraphina")).clone();
        character.scenario = "雨夜驿站".into();
        let ctx = WritingContext::legacy(vec![Arc::new(character)], None, Id::new());
        let template = prompt_template_context_for_writing(&ctx, None).expect("legacy single card");

        let module_id = Id::from_str("template-module");
        let module = storyforge_domain::prompt_module::PromptModule {
            id: module_id.clone(),
            name: "ST template".into(),
            category: storyforge_domain::prompt_module::ModuleCategory::Quality,
            content: "角色 {{char}} 在 {{scenario}} 与 <user> 对话".into(),
            exclusivity: storyforge_domain::prompt_module::Exclusivity::Multiple,
            source: storyforge_domain::prompt_module::ModuleSource::ImportedFromST,
            applicable_roles: vec![AgentRole::Director],
            tags: vec![],
        };
        let mut selections = std::collections::HashMap::new();
        let mut cats = std::collections::HashMap::new();
        cats.insert(
            storyforge_domain::prompt_module::ModuleCategory::Quality,
            vec![module_id],
        );
        selections.insert(AgentRole::Director, cats);
        let profile = storyforge_domain::prompt_module::PromptProfile {
            id: Id::from_str("template-profile"),
            name: "Template Profile".into(),
            selections,
            overrides: std::collections::HashMap::new(),
            source: storyforge_domain::prompt_module::ProfileSource::ImportedFromST,
        };

        let config = make_director_config(
            Some(&profile),
            &[module],
            "",
            None,
            Some(&template),
            &Default::default(),
        );

        assert!(
            config
                .system_prompt
                .contains("角色 Seraphina 在 雨夜驿站 与 玩家 对话")
        );
        assert!(!config.system_prompt.contains("{{char}}"));
    }

    #[test]
    fn test_prompt_template_context_uses_single_campaign_instance_variables() {
        use storyforge_domain::campaign::CharacterInstance;
        use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
        use storyforge_domain::variables::VariableValue;

        let mut campaign =
            storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "第一周目");
        campaign.set_variable("story_clock", serde_json::json!("Day 9 夜"), 3);
        campaign.set_variable("weather", serde_json::json!("雨"), 3);

        let mut instance = CharacterInstance::temporary(campaign.id.clone(), "林医生");
        instance.is_temporary = false;
        instance.persona_override = Some("谨慎的外科医生".into());
        instance.variables = vec![
            VariableValue::new("hp", serde_json::json!(72), 3),
            VariableValue::new("location", serde_json::json!("旧医院"), 3),
        ];

        let runtime = Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id: std::collections::HashMap::new(),
            knowledge: vec![],
            tasks: vec![],
            turn: 3,
        });
        let mut ctx = WritingContext::legacy(vec![mock_character("Legacy")], None, Id::new());
        ctx.campaign_runtime = Some(runtime);

        let template =
            prompt_template_context_for_writing(&ctx, None).expect("single campaign instance");
        let rendered = storyforge_domain::prompt_module::replace_template_vars_with_context(
            "{{char}} {{description}} hp={{getvar::hp}} loc={{getvar::location}} clock={{getvar::story_clock}} weather={{getvar::campaign.weather}}",
            &template,
        );

        assert_eq!(
            rendered,
            "林医生 谨慎的外科医生 hp=72 loc=旧医院 clock=Day 9 夜 weather=雨"
        );
    }

    #[test]
    fn test_prompt_template_context_skips_ambiguous_character_contexts() {
        let multi = WritingContext::legacy(
            vec![mock_character("Seraphina"), mock_character("Lin")],
            None,
            Id::new(),
        );
        assert!(prompt_template_context_for_writing(&multi, None).is_none());

        let mut campaign =
            storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "第一周目");
        campaign.set_variable("weather", serde_json::json!("雨"), 2);
        let mut lin =
            storyforge_domain::campaign::CharacterInstance::temporary(campaign.id.clone(), "Lin");
        lin.id = Id::from_str("inst-lin");
        lin.is_temporary = false;
        lin.variables = vec![storyforge_domain::variables::VariableValue::new(
            "hp",
            serde_json::json!(71),
            2,
        )];
        let mut mei =
            storyforge_domain::campaign::CharacterInstance::temporary(campaign.id.clone(), "Mei");
        mei.id = Id::from_str("inst-mei");
        mei.is_temporary = false;
        mei.variables = vec![storyforge_domain::variables::VariableValue::new(
            "location",
            serde_json::json!("天台"),
            2,
        )];
        let runtime = Arc::new(
            storyforge_domain::campaign_runtime::CampaignRuntimeContext {
                campaign: {
                    campaign.name = "第一周目".into();
                    campaign
                },
                instances: vec![lin, mei],
                definitions_by_id: std::collections::HashMap::new(),
                knowledge: vec![],
                tasks: vec![],
                turn: 0,
            },
        );
        let mut campaign_ctx =
            WritingContext::legacy(vec![mock_character("Legacy")], None, Id::new());
        campaign_ctx.campaign_runtime = Some(runtime);

        let template = prompt_template_context_for_writing(&campaign_ctx, None)
            .expect("multi-instance campaign should still expose scoped variables");
        let rendered = storyforge_domain::prompt_module::replace_template_vars_with_context(
            "{{char}} {{description}} <bot> user=<user> campaign={{getvar::campaign.name}} weather={{getvar::weather}} lin={{getvar::instance.inst-lin.name}} hp={{getvar::instance.inst-lin.hp}} mei_loc={{getvar::instance.Mei.location}}",
            &template,
        );

        assert_eq!(
            rendered,
            "{{char}} {{description}} <bot> user=玩家 campaign=第一周目 weather=雨 lin=Lin hp=71 mei_loc=天台"
        );
    }

    #[test]
    fn test_prompt_template_context_uses_id_scope_for_duplicate_instance_names() {
        let campaign = storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "同名档");
        let mut first =
            storyforge_domain::campaign::CharacterInstance::temporary(campaign.id.clone(), "影");
        first.id = Id::from_str("inst-shadow-a");
        first.is_temporary = false;
        first.variables = vec![storyforge_domain::variables::VariableValue::new(
            "stance",
            serde_json::json!("guard"),
            1,
        )];
        let mut second =
            storyforge_domain::campaign::CharacterInstance::temporary(campaign.id.clone(), "影");
        second.id = Id::from_str("inst-shadow-b");
        second.is_temporary = false;
        second.variables = vec![storyforge_domain::variables::VariableValue::new(
            "stance",
            serde_json::json!("attack"),
            1,
        )];

        let runtime = Arc::new(
            storyforge_domain::campaign_runtime::CampaignRuntimeContext {
                campaign,
                instances: vec![first, second],
                definitions_by_id: std::collections::HashMap::new(),
                knowledge: vec![],
                tasks: vec![],
                turn: 1,
            },
        );
        let mut ctx = WritingContext::legacy(vec![], None, Id::new());
        ctx.campaign_runtime = Some(runtime);

        let template = prompt_template_context_for_writing(&ctx, None)
            .expect("duplicate-name campaign should expose id-scoped variables");
        let rendered = storyforge_domain::prompt_module::replace_template_vars_with_context(
            "a={{getvar::instance.inst-shadow-a.stance}} b={{getvar::instance.inst-shadow-b.stance}} by_name={{getvar::instance.影.stance}}",
            &template,
        );

        assert_eq!(rendered, "a=guard b=attack by_name=");
    }

    /// truncate_chars 基本功能验证
    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello", 5), "hello");
        assert_eq!(truncate_chars("hello world", 5), "hello…");
        // 中文
        assert_eq!(truncate_chars("你好世界", 3), "你好世…");
        assert_eq!(truncate_chars("你好世界", 4), "你好世界");
        assert_eq!(truncate_chars("你好世界", 10), "你好世界");
    }

    /// A2：写作 seed 注入模板 random_seed 后，同 seed 渲染可复现
    #[test]
    fn test_template_random_seed_is_deterministic() {
        let mut ctx = WritingContext::legacy(vec![mock_character("Seraphina")], None, Id::new());
        ctx.template_random_seed = Some(42);
        let template = prompt_template_context_for_writing(&ctx, None).expect("single character");
        assert_eq!(template.random_seed, Some(42));
        let a = storyforge_domain::prompt_module::replace_template_vars_with_context(
            "{{random::alpha::beta::gamma}}",
            &template,
        );
        let b = storyforge_domain::prompt_module::replace_template_vars_with_context(
            "{{random::alpha::beta::gamma}}",
            &template,
        );
        assert_eq!(a, b, "fixed seed must make random macro deterministic");
        assert!(
            matches!(a.as_str(), "alpha" | "beta" | "gamma"),
            "random macro should pick one of the options, got {a}"
        );

        let other = prompt_template_context_for_writing(&ctx, Some(99)).unwrap();
        let c = storyforge_domain::prompt_module::replace_template_vars_with_context(
            "{{random::alpha::beta::gamma}}",
            &other,
        );
        // 不同 seed 允许相同结果，但至少 random_seed 字段必须写入
        assert_eq!(other.random_seed, Some(99));
        let _ = c;
    }

    /// W10: mvu_runtime=None + 空 fragments → run_postprocess 无 campaign 时返回 None（正常跳过）
    #[tokio::test]
    async fn test_postprocess_no_campaign_skips_even_with_fragments() {
        let (orch, _conv_store) = make_orchestrator();
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_mvu1_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_tx, cancel) = watch::channel(false);

        // 传非空 fragments + mvu_runtime=None → 无 campaign 时直接返回 None，不碰 JS 路径
        let fragments = vec![FallbackFragment {
            description: "test".into(),
            js_snippet: "_.set('hp', 1);".into(),
            reason: "test".into(),
        }];
        let outcome = orch
            .run_postprocess("text", "", &[], &[], &ctx, &event_tx, cancel, &fragments)
            .await;
        assert!(
            outcome.is_none(),
            "无 campaign 应跳过后处理（不管 fragments）"
        );
    }

    /// W10: mvu_runtime=None + 空 fragments → 无 campaign 时安静跳过（无额外日志噪声）
    #[tokio::test]
    async fn test_postprocess_empty_fragments_noop() {
        let (orch, _conv_store) = make_orchestrator();
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_mvu2_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        let (event_tx, _rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_tx, cancel) = watch::channel(false);

        let outcome = orch
            .run_postprocess("text", "", &[], &[], &ctx, &event_tx, cancel, &[])
            .await;
        assert!(outcome.is_none(), "空 fragments + 无 campaign → 跳过");
    }

    /// W10: build_current_variables 在无 campaign_runtime 时返回空 map
    #[test]
    fn test_build_current_variables_no_campaign() {
        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_mvu3_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };
        let ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        let vars = build_current_variables(&ctx);
        assert!(vars.is_empty(), "无 campaign_runtime 时变量快照应为空");
    }

    /// W10: build_current_variables 从 campaign instances 收集变量
    #[test]
    fn test_build_current_variables_with_instances() {
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::character::RoleType;
        use storyforge_domain::variables::VariableValue;

        let conv_store = {
            let dir = std::env::temp_dir().join(format!("sf_mvu4_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Arc::new(ConversationStore::new(dir))
        };

        let card_id = Id::new();
        let def = storyforge_domain::character::CharacterDefinition {
            id: Id::new(),
            card_id: card_id.clone(),
            name: "TestChar".into(),
            persona_prompt: "".into(),
            behavior_rules: "".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        };
        let mut inst = CharacterInstance::from_definition(Id::new(), &def);
        inst.variables = vec![
            VariableValue::new("hp", serde_json::json!(80), 1),
            VariableValue::new("state", serde_json::json!("calm"), 1),
        ];

        let campaign = Campaign::new(card_id, "test-camp");
        let runtime = Arc::new(
            storyforge_domain::campaign_runtime::CampaignRuntimeContext {
                campaign,
                instances: vec![inst],
                definitions_by_id: std::collections::HashMap::new(),
                knowledge: vec![],
                tasks: vec![],
                turn: 1,
            },
        );

        let mut ctx = WritingContext::legacy(vec![], None, conv_store.create(None, None).id);
        ctx.campaign_runtime = Some(runtime);

        let vars = build_current_variables(&ctx);
        assert_eq!(vars.len(), 2, "应收集到 2 个变量");
        assert_eq!(vars.get("hp").unwrap(), &serde_json::json!(80));
        assert_eq!(vars.get("state").unwrap(), &serde_json::json!("calm"));
    }

    #[test]
    fn filter_history_to_near_raw_turns_keeps_prefix_and_near() {
        use storyforge_domain::llm::ChatMessage;
        let history = vec![
            ChatMessage::user(
                "【事件概览】
A0001 far",
            ),
            ChatMessage::user("u1"),
            ChatMessage::assistant("a1"),
            ChatMessage::user("u2"),
            ChatMessage::assistant("a2"),
            ChatMessage::user("u3"),
            ChatMessage::assistant("a3"),
        ];
        // near only turn 2 and 3 mapped to last two pairs
        let filtered = filter_history_to_near_raw_turns(history, &[2, 3]);
        let texts: Vec<_> = filtered.iter().map(|m| m.content.as_str()).collect();
        assert!(texts[0].starts_with("【事件概览】"));
        assert!(texts.contains(&"u2"));
        assert!(texts.contains(&"u3"));
        assert!(!texts.contains(&"u1"));
    }
}

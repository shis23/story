use serde::{Deserialize, Serialize};

use crate::Id;
use crate::llm::ToolSpec;
use crate::world_info::WorldInfoEntry;

// ─── Agent 角色 ──────────────────────────────────────────────────────────

/// Agent 角色（对应设计 §3.2 的三个 Agent + Meta）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentRole {
    /// 导演：解析意图、查资料、分配任务
    Director,
    /// 子 Agent：按角色表演（附带角色 ID）
    Subagent(String),
    /// 编剧：收集子产出、合并润色
    Editor,
    /// Meta：配置调试助手
    Meta,
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Director => write!(f, "导演"),
            Self::Subagent(id) => write!(f, "子Agent({id})"),
            Self::Editor => write!(f, "编剧"),
            Self::Meta => write!(f, "Meta"),
        }
    }
}

// ─── Agent 配置（对应设计 §5 AgentProfile，M1 简化版）───────────────────

/// Agent 运行配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub role: AgentRole,
    /// 系统提示词（由 assemble_system_prompt 组装）
    pub system_prompt: String,
    pub max_tool_rounds: u32,
    pub tools: Vec<ToolSpec>,
    /// 该 Agent 要使用的模型名（从 LlmConnection 读取，允许 Agent 覆盖）
    pub model_override: Option<String>,
}

// ─── 专属上下文包（对应设计 §3.3 ContextPackage）────────────────────────

/// 子 Agent 的专属上下文包（导演构造，不互相污染）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPackage {
    /// 该角色设定（从角色卡提取）
    pub character_brief: String,
    /// 当前场景目标（导演写的）
    pub scene_brief: String,
    /// 检索到的相关世界书条目
    pub relevant_lore: Vec<LoreEntryLight>,
    /// 蓝灯常驻条目（所有人共享，但各取所需）
    pub constant_lore: Vec<LoreEntryLight>,
    /// 最近 N 条原文（共享窗口）
    pub recent_window: Vec<String>,
    /// 导演分配的具体任务
    pub task: String,
}

/// 轻量世界书条目（上下文包用，不暴露完整结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoreEntryLight {
    pub keys: Vec<String>,
    pub content: String,
}

impl From<&WorldInfoEntry> for LoreEntryLight {
    fn from(e: &WorldInfoEntry) -> Self {
        Self {
            keys: e.keys.clone(),
            content: e.content.clone(),
        }
    }
}

// ─── Plan（对应设计 §3.2 Plan）───────────────────────────────────────────

/// 导演输出的 Plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// 场景简述
    pub scene_brief: String,
    /// 分配给每个子 Agent 的任务
    pub subagent_tasks: Vec<SubagentTask>,
}

/// 单个子 Agent 的任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTask {
    /// 角色 ID（对应 Character.name 或 CharacterId）
    pub character_id: String,
    /// 简述（给子 Agent 的任务说明）
    pub brief: String,
    /// 专属上下文包
    pub context_package: ContextPackage,
}

// ─── 子 Agent 产出（Performance）与编剧产出（Draft）──────────────────────

/// 子 Agent 的表演产出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Performance {
    pub character_id: String,
    pub narrative: String,      // 叙事段落
    pub dialogue: String,       // 对白
    pub inner_thoughts: String, // 内心独白
    /// 完整输出（合并 narrative/dialogue/inner_thoughts）
    pub full_text: String,
}

/// 编剧的成文产出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    /// 最终成文（Markdown）
    pub text: String,
    /// 各子表演的裁剪/保留记录
    pub attribution: Vec<Attribution>,
}

/// 子表演的归因记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribution {
    pub character_id: String,
    /// 该子表演是否被编剧保留（未大幅改动）
    pub kept: bool,
    /// 编剧的备注
    pub note: Option<String>,
}

// ─── 流水线状态（对应设计 §3.1 PipelineState）───────────────────────────

/// 流水线状态机
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineState {
    /// 空闲
    Idle,
    /// 导演 Agent 运行中
    Directing,
    /// 子 Agent 并行派发中
    Delegating,
    /// 编剧 Agent 运行中
    Editing,
    /// 用户预览/编辑/采纳
    Review,
    /// 已写入对话历史
    Committed,
    /// 失败/取消
    Aborted,
}

// ─── 流水线事件（前端订阅）────────────────────────────────────────────────

/// 流水线推送到前端的事件（Tauri Channel 标签化枚举）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PipelineEvent {
    /// 流水线启动
    Started { session_id: String },
    /// 导演开始
    DirectorStarted,
    /// 导演流式进度（思维链/输出的文字片段）
    DirectorProgress {
        delta: String,
    },
    /// 导演完成，产出 Plan
    DirectorDone {
        scene_brief: String,
        subagent_count: usize,
    },
    /// 子 Agent 开始
    SubagentStarted {
        character_id: String,
        index: usize,
        total: usize,
    },
    /// 子 Agent 流式进度（文字片段）
    SubagentProgress {
        character_id: String,
        index: usize,
        delta: String,
    },
    /// 子 Agent 完成
    SubagentDone {
        character_id: String,
        index: usize,
        /// 子 Agent 的完整产出文本（供前端展示）
        full_text: String,
    },
    /// 子 Agent 取消
    SubagentCancelled {
        character_id: String,
        index: usize,
    },
    /// 编剧开始
    EditorStarted,
    /// 编剧流式进度
    EditorProgress {
        delta: String,
    },
    /// 成文就绪
    DraftReady {
        text: String,
    },
    /// 完成（已写入树）
    Committed {
        session_id: String,
        variant_id: String,
    },
    /// 错误
    Error {
        message: String,
    },
    /// 状态变化通知
    StateChanged {
        state: PipelineState,
    },
}

// ─── 写作会话（运行时状态）───────────────────────────────────────────────

/// 写作会话（对应设计 §5 WritingSession，M1 简化版）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WritingSession {
    pub id: Id,
    /// 用户输入的写作意图
    pub intent: String,
    pub state: PipelineState,
    pub plan: Option<Plan>,
    pub subagent_results: Vec<Performance>,
    pub draft: Option<Draft>,
    /// 溯源信息（用于部分重 roll）
    pub seed: u64,
}

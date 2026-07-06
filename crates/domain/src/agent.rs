use serde::{Deserialize, Serialize};

use crate::Id;
use crate::character_knowledge::CharacterKnowledgeUpdate;
use crate::llm::ToolSpec;
use crate::story_task::TaskUpdate;
use crate::world_info::WorldInfoEntry;

// ─── Agent 角色 ──────────────────────────────────────────────────────────

/// Agent 角色（对应设计 §3.2 的三个 Agent + Meta + 导入/后处理 Agent）
///
/// 序列化为**扁平字符串**：`Director` → `"Director"`，`Subagent("*")` → `"Subagent:*"`。
/// 原因：`PromptProfile` 用 `HashMap<AgentRole, ...>`，JSON 的 map key 必须是字符串，
/// serde 默认对带数据的 enum 变体（`Subagent(String)`）会生成 `"Subagent("*")"`
/// 这种格式，反序列化时无法 round-trip（报 unknown variant）。扁平字符串可作 map key。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentRole {
    /// 导演：解析意图、查资料、分配任务
    Director,
    /// 子 Agent：按角色表演（附带角色 ID，"*" 表示通配符，对所有子 Agent 生效）
    Subagent(String),
    /// 编剧：收集子产出、合并润色
    Editor,
    /// Meta：配置调试助手
    Meta,
    /// 角色识别：导入卡时分析卡内容、拆分多角色定义（D33，AGENT_INTERFACES §6.2）
    CharacterExtractor,
    /// 剧情总结：编剧后并行，产出本轮摘要（AGENT_INTERFACES §6.3）
    Summarizer,
    /// 后处理：编剧后并行，三合一产出知识/变量/任务（AGENT_INTERFACES §6.4）
    PostProcessor,
}

// ─── AgentRole 自定义 serde：扁平字符串 ────────────────────────────────────
//
// 序列化：Director → "Director"；Subagent("*") → "Subagent:*"（':' 分隔，可 round-trip）
// 反序列化：先按 ':' 切，前缀匹配变体名；无 ':' 的当 unit 变体。
// 兼容旧格式：纯 "Subagent" 当 Subagent("")（理论上不会出现，旧数据是 "Subagent(\"*\")" 无法兼容，
//   但 profiles.json 此前因本 bug 从未成功保存，无旧数据需迁移）。
impl Serialize for AgentRole {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            AgentRole::Subagent(id) => serializer.serialize_str(&format!("Subagent:{id}")),
            other => serializer.serialize_str(other.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for AgentRole {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.split_once(':') {
            // 新格式："Subagent:*" → Subagent("*")
            Some(("Subagent", payload)) => Ok(AgentRole::Subagent(payload.to_string())),
            _ => {
                // 兼容旧格式：serde 默认对元组变体生成的 "Subagent(\"*\")" / `Subagent("xxx")`
                // 用正则太重，手写解析：匹配 Subagent("...")
                if let Some(rest) = s.strip_prefix("Subagent(\"")
                    && let Some(id) = rest.strip_suffix("\")")
                {
                    return Ok(AgentRole::Subagent(id.to_string()));
                }
                AgentRole::from_str(&s).ok_or_else(|| {
                    serde::de::Error::custom(format!("unknown AgentRole variant: {s}"))
                })
            }
        }
    }
}

impl AgentRole {
    /// unit 变体的字符串名（不含 Subagent，它走 Subagent:id 格式）
    fn as_str(&self) -> &'static str {
        match self {
            AgentRole::Director => "Director",
            AgentRole::Editor => "Editor",
            AgentRole::Meta => "Meta",
            AgentRole::CharacterExtractor => "CharacterExtractor",
            AgentRole::Summarizer => "Summarizer",
            AgentRole::PostProcessor => "PostProcessor",
            AgentRole::Subagent(_) => "Subagent", // 序列化走 Subagent:id，这里只作 fallback
        }
    }

    /// 从字符串解析 unit 变体（Subagent 不在此列，它带 :payload）
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "Director" => Some(AgentRole::Director),
            "Editor" => Some(AgentRole::Editor),
            "Meta" => Some(AgentRole::Meta),
            "CharacterExtractor" => Some(AgentRole::CharacterExtractor),
            "Summarizer" => Some(AgentRole::Summarizer),
            "PostProcessor" => Some(AgentRole::PostProcessor),
            _ => None,
        }
    }
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Director => write!(f, "导演"),
            Self::Subagent(id) => write!(f, "子Agent({id})"),
            Self::Editor => write!(f, "编剧"),
            Self::Meta => write!(f, "Meta"),
            Self::CharacterExtractor => write!(f, "角色识别"),
            Self::Summarizer => write!(f, "剧情总结"),
            Self::PostProcessor => write!(f, "后处理"),
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
    DirectorProgress { delta: String },
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
    SubagentCancelled { character_id: String, index: usize },
    /// 编剧开始
    EditorStarted,
    /// 编剧流式进度
    EditorProgress { delta: String },
    /// 成文就绪
    DraftReady { text: String },
    /// 后处理流水线启动（总结 + 后处理并行）
    PostProcessStarted,
    /// 后处理完成（三件套产出计数）
    PostProcessDone {
        knowledge_count: usize,
        variable_count: usize,
        task_count: usize,
    },
    /// 后处理失败（best-effort，不阻断成文）
    PostProcessFailed { reason: String },
    /// 后处理被 AgentProfileConfig 关闭跳过（区别于失败：明确是配置关闭，非 LLM 出错）
    PostProcessSkipped { reason: String },
    /// 本轮剧情总结完成
    SummaryDone { char_count: usize },
    /// 完成（已写入树）
    Committed {
        session_id: String,
        variant_id: String,
    },
    /// 错误
    Error { message: String },
    /// 状态变化通知
    StateChanged { state: PipelineState },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：Subagent("*") 必须能 serde round-trip（曾因元组变体导致 unknown variant）
    #[test]
    fn agent_role_subagent_wildcard_roundtrips() {
        let role = AgentRole::Subagent("*".into());
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, "\"Subagent:*\"");
        let back: AgentRole = serde_json::from_str(&json).unwrap();
        assert_eq!(back, role);
    }

    /// 回归：unit 变体 round-trip
    #[test]
    fn agent_role_unit_variants_roundtrip() {
        for role in [
            AgentRole::Director,
            AgentRole::Editor,
            AgentRole::Meta,
            AgentRole::CharacterExtractor,
            AgentRole::Summarizer,
            AgentRole::PostProcessor,
        ] {
            let json = serde_json::to_string(&role).unwrap();
            let back: AgentRole = serde_json::from_str(&json).unwrap();
            assert_eq!(back, role, "{role:?} round-trip failed");
        }
    }

    /// 回归：作为 HashMap key 能 round-trip（PromptProfile 的实际触发场景）
    #[test]
    fn agent_role_as_hashmap_key_roundtrips() {
        let mut map = std::collections::HashMap::new();
        map.insert(AgentRole::Director, 1);
        map.insert(AgentRole::Subagent("*".into()), 2);
        map.insert(AgentRole::Editor, 3);

        let json = serde_json::to_string(&map).unwrap();
        let back: std::collections::HashMap<AgentRole, i32> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back.get(&AgentRole::Subagent("*".into())), Some(&2));
    }

    /// 兼容旧格式：serde 默认生成的 `Subagent("xxx")` 也应能反序列化
    #[test]
    fn agent_role_legacy_format_compat() {
        // 旧格式 "Subagent(\"*\")"
        let role: AgentRole = serde_json::from_str("\"Subagent(\\\"*\\\")\"").unwrap();
        assert_eq!(role, AgentRole::Subagent("*".into()));
        // 旧格式带具体 id
        let role: AgentRole = serde_json::from_str("\"Subagent(\\\"林医生\\\")\"").unwrap();
        assert_eq!(role, AgentRole::Subagent("林医生".into()));
        // 新格式仍然正常
        let role: AgentRole = serde_json::from_str("\"Subagent:*\"").unwrap();
        assert_eq!(role, AgentRole::Subagent("*".into()));
    }
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

// ─── 后处理产出（P2 新增，对应 AGENT_INTERFACES §6.4 / §9.3）──────────────

/// 后处理 Agent 一次调用产出的三件套
///
/// 编剧成文后并行跑（与剧情总结 Agent 并行），产出角色知识更新 + 变量更新 + 任务更新。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PostProcessResult {
    /// 角色知识更新（各角色获知的新信息，带 source 分类）
    #[serde(default)]
    pub knowledge_updates: Vec<CharacterKnowledgeUpdate>,
    /// 变量更新（角色级 + 全局级，stat_data 的 _.set 解析结果）
    #[serde(default)]
    pub variable_updates: Vec<VariableUpdate>,
    /// 任务更新（新建伏笔 / 触发状态变化 / 完成检测置信度）
    #[serde(default)]
    pub task_updates: Vec<TaskUpdate>,
    /// 5 层兜底解析是否成功（区分「LLM 正常返回但无更新」与「解析全 miss」）。
    ///
    /// 历史 bug：失败返回空 PostProcessResult，与"成功但无更新"无法区分，
    /// 解析失败被静默吞掉。现标记 parse_succeeded=false 让上层可观测。
    /// 旧数据反序列化时缺此字段，默认 true（假定历史数据是成功解析的）。
    #[serde(default = "default_true")]
    pub parse_succeeded: bool,
}

fn default_true() -> bool {
    true
}

/// 单条变量更新（角色级或全局级）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableUpdate {
    /// 目标角色实例 ID（None = 全局 Campaign 变量，如 story_clock）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<Id>,
    /// 变量键名（如 "hp" / "story_clock"）
    pub key: String,
    /// 新值
    pub value: serde_json::Value,
}

impl PostProcessResult {
    pub fn is_empty(&self) -> bool {
        self.knowledge_updates.is_empty()
            && self.variable_updates.is_empty()
            && self.task_updates.is_empty()
    }
}

// ─── 本轮剧情摘要（P2 新增，每轮一条原子单位）─────────────────────────────

/// 一轮写作的剧情摘要（剧情总结 Agent 产出，独立于 archiver 的批量归档）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundSummary {
    pub id: Id,
    pub campaign_id: Id,
    pub conversation_id: Id,
    /// 第几轮（与对话树节点对应）
    pub turn: u32,
    /// 摘要正文（200-500 字高密度总结）
    pub content: String,
    pub created_at: String,
}

impl RoundSummary {
    pub fn new(campaign_id: Id, conversation_id: Id, turn: u32, content: String) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            conversation_id,
            turn,
            content,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

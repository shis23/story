use serde::{Deserialize, Serialize};

use crate::Id;
use crate::character_knowledge::CharacterKnowledgeUpdate;
use crate::llm::{ChatMessage, ToolSpec};
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

/// 扩展场景规划（阶段 B / ScenePlan）。
///
/// 全部可选；旧 Plan JSON / Provenance 无此字段时按空处理。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePlan {
    /// 本场核心冲突
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
    /// 对立目标（各方想要什么）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opposing_goals: Vec<String>,
    /// 赌注 / 失败代价
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stakes: Option<String>,
    /// 本场节拍（短句列表）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beats: Vec<String>,
    /// 本场复杂化 / 搅局
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complication: Option<String>,
    /// 本场**不得**一次性解决的问题
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_not_resolve: Option<String>,
    /// 收束时留下的出口钩子
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_hook: Option<String>,
}

impl ScenePlan {
    /// 是否完全为空（无任何有效内容）
    pub fn is_empty(&self) -> bool {
        self.conflict.as_ref().is_none_or(|s| s.trim().is_empty())
            && self.opposing_goals.iter().all(|s| s.trim().is_empty())
            && self.stakes.as_ref().is_none_or(|s| s.trim().is_empty())
            && self.beats.iter().all(|s| s.trim().is_empty())
            && self
                .complication
                .as_ref()
                .is_none_or(|s| s.trim().is_empty())
            && self
                .must_not_resolve
                .as_ref()
                .is_none_or(|s| s.trim().is_empty())
            && self.exit_hook.as_ref().is_none_or(|s| s.trim().is_empty())
    }

    /// 渲染为短文本，供 Editor / Director tail 注入。
    pub fn render_for_prompt(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut lines = vec!["【场景规划 ScenePlan】".to_string()];
        if let Some(c) = self
            .conflict
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("- 冲突：{c}"));
        }
        let goals: Vec<&str> = self
            .opposing_goals
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !goals.is_empty() {
            lines.push(format!("- 对立目标：{}", goals.join("；")));
        }
        if let Some(s) = self
            .stakes
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("- 赌注：{s}"));
        }
        let beats: Vec<&str> = self
            .beats
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !beats.is_empty() {
            lines.push(format!("- 节拍：{}", beats.join(" → ")));
        }
        if let Some(c) = self
            .complication
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("- 复杂化：{c}"));
        }
        if let Some(m) = self
            .must_not_resolve
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("- 本场不得解决：{m}"));
        }
        if let Some(e) = self
            .exit_hook
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("- 出口钩子：{e}"));
        }
        lines.join("\n")
    }
}

/// 导演输出的 Plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// 场景简述
    pub scene_brief: String,
    /// 分配给每个子 Agent 的任务
    pub subagent_tasks: Vec<SubagentTask>,
    /// 扩展场景规划（阶段 B）；旧数据缺省为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_plan: Option<ScenePlan>,
}

impl Plan {
    /// 最小构造（无 ScenePlan）
    pub fn new(scene_brief: impl Into<String>, subagent_tasks: Vec<SubagentTask>) -> Self {
        Self {
            scene_brief: scene_brief.into(),
            subagent_tasks,
            scene_plan: None,
        }
    }
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
    /// 与用户输入无关的当前欲望（梁元 CharacterAgency 结构化吸收）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_desire: Option<String>,
    /// 进场前已在做的事
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ongoing_action: Option<String>,
    /// 情绪阶段 1..=6；仅幕后约束，禁止正文直说阶段名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion_stage: Option<u8>,
}

impl SubagentTask {
    /// 最小构造（无 agency 字段）
    pub fn new(
        character_id: impl Into<String>,
        brief: impl Into<String>,
        context_package: ContextPackage,
    ) -> Self {
        Self {
            character_id: character_id.into(),
            brief: brief.into(),
            context_package,
            current_desire: None,
            ongoing_action: None,
            emotion_stage: None,
        }
    }
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
    /// B3：草稿质量门禁检查完成（warn-only，不阻断流程）
    QualityChecked {
        passed: bool,
        warning_count: usize,
        /// Error 级问题数（与 warning 区分；旧事件缺省 0）
        #[serde(default)]
        error_count: usize,
        /// 警告摘要（message 列表，便于前端展示；无则空）
        #[serde(default)]
        warnings: Vec<String>,
    },
    /// 最终 LLM messages prompt hook 请求（前端插件可异步改写 messages）
    PromptHookRequest {
        request_id: String,
        role: AgentRole,
        round: u32,
        model: String,
        messages: Vec<ChatMessage>,
    },
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

/// 一轮写作的剧情摘要（剧情总结 Agent 产出，独立于 archiver 的批量归档）。
///
/// **演进方向（记忆规格 v1.0 M1）**：本结构是 Chronicle A 的兼容存储形态，
/// 禁止再维护第二套完整 RoundSummary 副本。新增字段全部 `default`，旧 JSON 可反序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundSummary {
    pub id: Id,
    pub campaign_id: Id,
    pub conversation_id: Id,
    /// 第几轮（与对话树节点对应）
    pub turn: u32,
    /// 摘要正文（200-500 字高密度总结）— 对齐 Chronicle.summary
    pub content: String,
    pub created_at: String,
    /// Chronicle 可读 code（如 A0001）；空 = 尚未分配
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 概览短标题；空则注入侧可从 content 截断
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    /// 记忆线；缺省时由运行时用 conversation 主线 lineage 回填
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<Id>,
    /// 被上层 B/C 折叠时填写 parent entry id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered_by: Option<Id>,
    /// Chronicle 层级：0=A leaf，1=B，2=C。缺省 0（旧 JSON）。
    #[serde(default)]
    pub level: u8,
    /// 跨轮 span 的结束 turn（含）；缺省 0 表示 = `turn`（单轮 A）。
    #[serde(default)]
    pub turn_end: u32,
    /// B/C 覆盖的子 entry id（系统填写）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<Id>,
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
            code: None,
            headline: None,
            lineage_id: None,
            covered_by: None,
            level: 0,
            turn_end: turn,
            covers: vec![],
        }
    }

    pub fn effective_turn_end(&self) -> u32 {
        if self.turn_end == 0 {
            self.turn
        } else {
            self.turn_end.max(self.turn)
        }
    }

    pub fn chronicle_level(&self) -> crate::chronicle::ChronicleLevel {
        crate::chronicle::ChronicleLevel::from_u8(self.level)
            .unwrap_or(crate::chronicle::ChronicleLevel::A)
    }

    pub fn is_leaf_a(&self) -> bool {
        self.level == 0
    }

    /// 分配 leaf code（系统侧；Summarizer 不写 code）。
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_headline(mut self, headline: impl Into<String>) -> Self {
        self.headline = Some(headline.into());
        self
    }

    pub fn with_lineage(mut self, lineage_id: Id) -> Self {
        self.lineage_id = Some(lineage_id);
        self
    }

    /// 概览行：优先 headline，否则 content 截断。
    pub fn overview_headline(&self, max_chars: usize) -> String {
        if let Some(h) = &self.headline
            && !h.trim().is_empty()
        {
            return crate::chronicle::truncate_headline(h, max_chars);
        }
        crate::chronicle::truncate_headline(&self.content, max_chars)
    }

    /// 兼容视图 → Chronicle A 条目（M1 纯转换，不落盘）。
    pub fn to_chronicle_a(&self, lineage_fallback: &Id) -> crate::chronicle::ChronicleEntry {
        use crate::chronicle::{ChronicleCode, ChronicleEntry, ChronicleLevel};
        let lineage = self
            .lineage_id
            .clone()
            .unwrap_or_else(|| lineage_fallback.clone());
        let code = self
            .code
            .as_deref()
            .and_then(ChronicleCode::parse)
            .unwrap_or_else(|| ChronicleCode::new(ChronicleLevel::A, self.turn));
        let level = self.chronicle_level();
        ChronicleEntry {
            id: self.id.clone(),
            code,
            level,
            campaign_id: self.campaign_id.clone(),
            lineage_id: lineage,
            headline: self.overview_headline(40),
            summary: self.content.clone(),
            full: None,
            turn_start: self.turn,
            turn_end: self.effective_turn_end(),
            covers: self.covers.clone(),
            covered_by: self.covered_by.clone(),
            source_turn_ids: vec![],
            source_variant_hashes: vec![],
            source_campaign_revision: None,
            origin_campaign_id: None,
            origin_chronicle_id: None,
            origin_code: None,
            invalidated_at: None,
            created_at: self.created_at.clone(),
        }
    }

    /// ChronicleEntry（含 B/C）→ 存储形态 RoundSummary。
    pub fn from_chronicle_entry(
        entry: &crate::chronicle::ChronicleEntry,
        conversation_id: Id,
    ) -> Self {
        Self {
            id: entry.id.clone(),
            campaign_id: entry.campaign_id.clone(),
            conversation_id,
            turn: entry.turn_start,
            content: entry.summary.clone(),
            created_at: entry.created_at.clone(),
            code: Some(entry.code.as_str().to_string()),
            headline: Some(entry.headline.clone()),
            lineage_id: Some(entry.lineage_id.clone()),
            covered_by: entry.covered_by.clone(),
            level: entry.level.as_u8(),
            turn_end: entry.turn_end,
            covers: entry.covers.clone(),
        }
    }
}

#[cfg(test)]
mod round_summary_chronicle_tests {
    use super::*;
    use crate::chronicle::ChronicleLevel;

    #[test]
    fn legacy_json_deserializes_without_chronicle_fields() {
        let raw = r#"{
            "id":"s1","campaign_id":"c1","conversation_id":"v1",
            "turn":3,"content":"发生了重要转折。","created_at":"t"
        }"#;
        let s: RoundSummary = serde_json::from_str(raw).unwrap();
        assert_eq!(s.turn, 3);
        assert!(s.code.is_none());
        assert!(s.headline.is_none());
    }

    #[test]
    fn to_chronicle_a_fills_code_and_headline() {
        let s = RoundSummary::new(Id::from_str("c"), Id::from_str("v"), 7, "长摘要正文".into())
            .with_code("A0007")
            .with_headline("关系破裂")
            .with_lineage(Id::from_str("lin-1"));
        let a = s.to_chronicle_a(&Id::from_str("fallback"));
        assert_eq!(a.level, ChronicleLevel::A);
        assert_eq!(a.code.as_str(), "A0007");
        assert_eq!(a.headline, "关系破裂");
        assert_eq!(a.lineage_id.as_str(), "lin-1");
        assert_eq!(a.turn_start, 7);
        assert_eq!(a.summary, "长摘要正文");
    }
}

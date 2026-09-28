//! 叙事计划系统（任务追踪 / 长程一致性）
//!
//! 对应设计 §21 / INTENT D45。
//!
//! 解决"导演忘记三个月后的伏笔"：用户规划或叙事伏笔 → 每轮比对触发 →
//! 接近时注入导演提示词 → 完成走软状态 + 用户确认。
//! 比对/抽取并入后处理 Agent（零额外调用），注入是确定性查表（零 LLM）。

use crate::Id;
use serde::{Deserialize, Serialize};

// ─── 触发条件（D45：三种都要）───────────────────────────────────────────────

/// 任务触发条件
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskTrigger {
    /// 事件驱动（主）："角色X得知真相" —— 后处理 Agent 语义判断
    Event { description: String },
    /// 轮次兜底：第 N 轮提醒（确定性比对 current_turn >= N）
    TurnReminder { at_turn: u32 },
    /// 故事时钟："到第2年6月触发"（参考 MVU 日期卡片，确定性比对）
    StoryTime { target: String },
    /// 只手动激活（用户点"现在触发"）
    Manual,
}

// ─── 状态（软状态 + 用户确认）──────────────────────────────────────────────

/// 任务状态（LikelyCompleted 是软状态，需用户确认）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 待激活（已录入但触发条件未满足）
    Pending,
    /// 已激活（触发条件满足，正在注入导演提示词）
    Active,
    /// 可能完成（后处理 Agent 判断，带置信度，待用户确认）
    LikelyCompleted { confidence: f32 },
    /// 已完成（用户确认或后处理高置信度）
    Completed,
    /// 已放弃（用户手动或剧情线断裂）
    Abandoned,
}

impl TaskStatus {
    pub fn is_injectable(&self) -> bool {
        matches!(self, Self::Pending | Self::Active)
    }
}

// ─── 任务来源 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    /// 用户显式规划（前端 UI 建）
    UserPlanned,
    /// 叙事中自然产生（后处理 Agent 抽取伏笔）
    ExtractedFromNarrative,
}

// ─── 任务结构 ──────────────────────────────────────────────────────────────

/// 一条叙事计划任务（quest / 伏笔追踪）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryTask {
    pub id: Id,
    pub campaign_id: Id,
    pub title: String,
    pub description: String,
    /// 触发条件（多个为 OR，任一满足即提醒）
    pub triggers: Vec<TaskTrigger>,
    pub status: TaskStatus,
    /// 创建它的轮次
    pub created_turn: u32,
    /// 相关角色（用于上下文聚焦）
    #[serde(default)]
    pub related_characters: Vec<Id>,
    pub source: TaskSource,
    /// 已在哪些轮注入过（防重复 + 统计）
    #[serde(default)]
    pub injected_turns: Vec<u32>,
}

impl StoryTask {
    /// 用户新建任务
    pub fn user_planned(
        campaign_id: Id,
        title: impl Into<String>,
        description: impl Into<String>,
        triggers: Vec<TaskTrigger>,
        created_turn: u32,
    ) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            title: title.into(),
            description: description.into(),
            triggers,
            status: TaskStatus::Pending,
            created_turn,
            related_characters: vec![],
            source: TaskSource::UserPlanned,
            injected_turns: vec![],
        }
    }

    /// 后处理 Agent 抽取伏笔时建任务
    pub fn from_narrative(
        campaign_id: Id,
        title: impl Into<String>,
        description: impl Into<String>,
        triggers: Vec<TaskTrigger>,
        created_turn: u32,
    ) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            title: title.into(),
            description: description.into(),
            triggers,
            status: TaskStatus::Pending,
            created_turn,
            related_characters: vec![],
            source: TaskSource::ExtractedFromNarrative,
            injected_turns: vec![],
        }
    }

    /// 确定性判断触发条件是否满足（Event 类型需后处理 Agent 判断，这里返回 None）
    ///
    /// - TurnReminder：current_turn >= at_turn → Some(true)
    /// - StoryTime：字符串比对（归一化后相等即触发）→ Some(bool)
    /// - Event：语义判断，返回 None（调用方应查询后处理 Agent 的判断结果）
    /// - Manual：永远不自动触发 → Some(false)
    ///
    /// D-04：StoryTime 只有"等于"是确定性的，因此
    /// - 时钟缺失（空/全空白，例如调用方没接故事时钟）→ 不能确定性判否，
    ///   该任务转为 [`TriggerCheck::NeedsAgentJudgment`]，而不是静默判否消失；
    /// - 任务已经注入过（`injected_turns` 非空或状态已 Active）→ 时钟越过
    ///   target 之后仍交 Agent 判断，避免"长程伏笔静默蒸发"。
    ///   （StoryTime 一旦越过目标，严格相等永远不会再命中，这是本模块要解决的
    ///   核心失效场景；时钟可比较大小之前，用"交由 Agent 判断"兜住。）
    pub fn check_trigger(&self, current_turn: u32, story_clock: &str) -> TriggerCheck {
        // 第一遍：扫描确定性触发器（Turn / StoryTime）。
        // 必须先于 Event 判断，否则任务同时配了 [Event, TurnReminder{at_turn:1}]
        // 且当前 turn=1 时，本应确定性 Satisfied 却因 Event 排前面而浪费一次 Agent 调用。
        let clock = normalize_story_clock(story_clock);
        let mut has_event = false;
        let mut story_time_unresolved = false;
        for trigger in &self.triggers {
            match trigger {
                TaskTrigger::TurnReminder { at_turn } => {
                    if current_turn >= *at_turn {
                        return TriggerCheck::Satisfied;
                    }
                }
                TaskTrigger::StoryTime { target } => {
                    // 归一化比较（去空白 + 大小写不敏感，M-30/D-04）
                    let target = normalize_story_clock(target);
                    if target.is_empty() {
                        // 空 target = 无意义触发器：忽略，不参与判定
                        continue;
                    }
                    if !clock.is_empty() && clock == target {
                        return TriggerCheck::Satisfied;
                    }
                    if clock.is_empty() || self.has_been_injected() {
                        story_time_unresolved = true;
                    }
                }
                TaskTrigger::Event { .. } => {
                    has_event = true;
                }
                TaskTrigger::Manual => {
                    // 手动触发，不自动激活
                }
            }
        }
        // 第二遍：没有确定性触发器命中，但存在 Event 触发器或无法判定的
        // StoryTime 触发器 → 需 Agent 语义判断
        if has_event || story_time_unresolved {
            return TriggerCheck::NeedsAgentJudgment;
        }
        TriggerCheck::NotSatisfied
    }

    /// 该任务是否已进入过注入状态（`mark_injected` 或后处理直接置 Active）。
    fn has_been_injected(&self) -> bool {
        !self.injected_turns.is_empty() || self.status == TaskStatus::Active
    }

    /// 标记为在某轮注入过
    pub fn mark_injected(&mut self, turn: u32) {
        if !self.injected_turns.contains(&turn) {
            self.injected_turns.push(turn);
        }
        if self.status == TaskStatus::Pending {
            self.status = TaskStatus::Active;
        }
    }

    /// 用户确认完成
    pub fn complete(&mut self) {
        self.status = TaskStatus::Completed;
    }

    /// 放弃
    pub fn abandon(&mut self) {
        self.status = TaskStatus::Abandoned;
    }
}

/// 触发条件检查结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerCheck {
    /// 确定性满足（轮次/时钟命中）
    Satisfied,
    /// 确定性不满足
    NotSatisfied,
    /// 含 Event 触发，需后处理 Agent 语义判断
    NeedsAgentJudgment,
}

// ─── 后处理 Agent 产出的任务更新 ─────────────────────────────────────────────

/// 后处理 Agent 每轮产出的任务变更
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskUpdate {
    /// None = 新建任务；Some = 改已有任务
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Id>,
    /// 新状态（Pending/Active/LikelyCompleted/Completed/Abandoned）
    pub new_status: TaskStatus,
    /// 新建任务时填
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_task: Option<NewTaskSpec>,
}

/// 新建任务的规格（后处理 Agent 产出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTaskSpec {
    pub title: String,
    pub description: String,
    pub triggers: Vec<TaskTrigger>,
    /// 抽取自叙事
    pub related_characters: Vec<Id>,
}

// ─── 注入导演提示词（确定性查表，零 LLM）──────────────────────────────────

/// 故事时钟归一化（比较用）：去掉所有空白 + Unicode 小写。
///
/// "第 47 天" / "第47天" / "Day 1" / "day1" 在归一化后可比（D-04）。
fn normalize_story_clock(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 把待注入的任务渲染成文本，拼进导演 user message 末尾
///
/// 只注入 Pending/Active 的任务（`TaskStatus::is_injectable`），并且触发条件
/// 为 Satisfied 或 NeedsAgentJudgment。`LikelyCompleted` / `Completed` /
/// `Abandoned` 一律不注入——无论置信度高低，软状态只在前端提示用户确认，
/// 不改变注入集合（D-20：原文案容易被读成"高置信度会自动注入"）。
///
/// D-04（Lead 裁定）：两类结果**必须分组渲染且指令不同**——
/// 把 `NeedsAgentJudgment` 混在「即将触发」标题下等于暗示模型"现在就写出来"，
/// 会造成提前揭示后续剧情（时钟缺失的老数据全部落进这一类）。
/// - `Satisfied` → 「【已满足条件的任务/伏笔】」，可以推进；
/// - `NeedsAgentJudgment` → 「【待判断的任务/伏笔】」，明确"不适用则忽略、
///   不要提前揭示后续剧情"。
///
/// 两组都为空时返回**空字符串**（不输出空标题），保持既有字节级契约。
pub fn render_tasks_for_injection(
    tasks: &[StoryTask],
    current_turn: u32,
    story_clock: &str,
) -> String {
    let mut satisfied: Vec<&StoryTask> = Vec::new();
    let mut needs_judgment: Vec<&StoryTask> = Vec::new();
    for t in tasks.iter().filter(|t| t.status.is_injectable()) {
        match t.check_trigger(current_turn, story_clock) {
            TriggerCheck::Satisfied => satisfied.push(t),
            TriggerCheck::NeedsAgentJudgment => needs_judgment.push(t),
            TriggerCheck::NotSatisfied => {}
        }
    }

    if satisfied.is_empty() && needs_judgment.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    if !satisfied.is_empty() {
        out.push_str("【已满足条件的任务/伏笔】\n");
        for t in &satisfied {
            out.push_str(&format!("- {}：{}\n", t.title, t.description));
        }
    }
    if !needs_judgment.is_empty() {
        if !satisfied.is_empty() {
            out.push('\n');
        }
        out.push_str("【待判断的任务/伏笔】\n");
        out.push_str(
            "以下任务/伏笔的条件无法确定（故事时钟缺失或已越过目标）。\
             请判断本轮是否适用；不适用则忽略，不要提前揭示后续剧情。\n",
        );
        for t in &needs_judgment {
            out.push_str(&format!("- {}：{}\n", t.title, t.description));
        }
    }
    out
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_reminder_triggers() {
        let task = StoryTask::user_planned(
            Id::new(),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::TurnReminder { at_turn: 30 }],
            5,
        );
        assert_eq!(task.check_trigger(29, "第1天"), TriggerCheck::NotSatisfied);
        assert_eq!(task.check_trigger(30, "第1天"), TriggerCheck::Satisfied);
    }

    #[test]
    fn test_story_time_trigger() {
        let task = StoryTask::user_planned(
            Id::new(),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::StoryTime {
                target: "第2年6月".into(),
            }],
            5,
        );
        assert_eq!(
            task.check_trigger(100, "第2年5月"),
            TriggerCheck::NotSatisfied
        );
        assert_eq!(task.check_trigger(100, "第2年6月"), TriggerCheck::Satisfied);
    }

    #[test]
    fn test_event_needs_agent() {
        let task = StoryTask::user_planned(
            Id::new(),
            "真相",
            "角色X得知真相",
            vec![TaskTrigger::Event {
                description: "角色X得知真相".into(),
            }],
            5,
        );
        assert_eq!(
            task.check_trigger(100, "第1天"),
            TriggerCheck::NeedsAgentJudgment
        );
    }

    /// 回归：确定性触发器（Turn/StoryTime）应优先于 Event，避免短路浪费 Agent 调用。
    /// Bug M-4：旧实现遇 Event 立即 return NeedsAgentJudgment，
    /// 任务配 [Event, TurnReminder{at_turn:1}] 且 turn=1 时本应 Satisfied 却返回 NeedsAgentJudgment。
    #[test]
    fn test_deterministic_trigger_takes_priority_over_event() {
        // Event 在前 + TurnReminder{at_turn:1} 在后，当前 turn=1 应确定性 Satisfied
        let task = StoryTask::user_planned(
            Id::new(),
            "混合",
            "测试",
            vec![
                TaskTrigger::Event {
                    description: "某事件".into(),
                },
                TaskTrigger::TurnReminder { at_turn: 1 },
            ],
            5,
        );
        assert_eq!(task.check_trigger(1, "第1天"), TriggerCheck::Satisfied);

        // 都没命中时，有 Event 才返回 NeedsAgentJudgment
        assert_eq!(
            task.check_trigger(0, "第99天"),
            TriggerCheck::NeedsAgentJudgment
        );
    }

    #[test]
    fn test_manual_never_auto_triggers() {
        let task =
            StoryTask::user_planned(Id::new(), "手动", "手动任务", vec![TaskTrigger::Manual], 5);
        assert_eq!(
            task.check_trigger(1000, "任何时候"),
            TriggerCheck::NotSatisfied
        );
    }

    #[test]
    fn test_mark_injected_promotes_to_active() {
        let mut task = StoryTask::user_planned(
            Id::new(),
            "t",
            "d",
            vec![TaskTrigger::TurnReminder { at_turn: 1 }],
            0,
        );
        assert_eq!(task.status, TaskStatus::Pending);
        task.mark_injected(5);
        assert_eq!(task.status, TaskStatus::Active);
        assert!(task.injected_turns.contains(&5));
    }

    #[test]
    fn test_render_injection_filters_by_status() {
        let campaign = Id::new();
        let mut pending = StoryTask::user_planned(
            campaign.clone(),
            "伏笔1",
            "描述",
            vec![TaskTrigger::TurnReminder { at_turn: 1 }],
            0,
        );
        let mut completed = StoryTask::user_planned(
            campaign.clone(),
            "伏笔2",
            "已完成",
            vec![TaskTrigger::TurnReminder { at_turn: 1 }],
            0,
        );
        completed.complete();

        let out = render_tasks_for_injection(&[pending.clone(), completed], 5, "第1天");
        assert!(out.contains("伏笔1"));
        assert!(!out.contains("伏笔2"));

        // pending 触发后注入会标 active
        pending.mark_injected(5);
    }

    #[test]
    fn test_render_empty_when_no_match() {
        let task = StoryTask::user_planned(
            Id::new(),
            "未来",
            "远期任务",
            vec![TaskTrigger::TurnReminder { at_turn: 100 }],
            0,
        );
        let out = render_tasks_for_injection(&[task], 5, "第1天");
        assert!(out.is_empty());
    }

    // ─── D-04：StoryTime 在时钟缺失/越过目标时不得静默消失 ─────────────────

    #[test]
    fn test_story_time_missing_clock_needs_agent_judgment() {
        let task = StoryTask::user_planned(
            Id::new(),
            "复仇",
            "到第2年6月触发复仇",
            vec![TaskTrigger::StoryTime {
                target: "第2年6月".into(),
            }],
            5,
        );
        // 时钟缺失 → 不能确定性判否（D-04）
        assert_eq!(task.check_trigger(10, ""), TriggerCheck::NeedsAgentJudgment);
        assert_eq!(
            task.check_trigger(10, "   "),
            TriggerCheck::NeedsAgentJudgment
        );
        // 注入渲染必须包含它（此前 "" 会让它彻底消失）
        let out = render_tasks_for_injection(std::slice::from_ref(&task), 10, "");
        assert!(
            out.contains("复仇"),
            "时钟缺失时 StoryTime 伏笔仍须注入：{out}"
        );
    }

    #[test]
    fn test_story_time_comparison_normalizes_whitespace_and_case() {
        let task = StoryTask::user_planned(
            Id::new(),
            "约定",
            "Day 1 的约定",
            vec![TaskTrigger::StoryTime {
                target: "Day 1".into(),
            }],
            5,
        );
        assert_eq!(task.check_trigger(5, "day1"), TriggerCheck::Satisfied);
        assert_eq!(task.check_trigger(5, " Day 1 "), TriggerCheck::Satisfied);
        assert_eq!(task.check_trigger(5, "day 1"), TriggerCheck::Satisfied);
        // 中文时钟的空格差异同样归一
        let cn = StoryTask::user_planned(
            Id::new(),
            "中文",
            "第 47 天",
            vec![TaskTrigger::StoryTime {
                target: "第47天".into(),
            }],
            5,
        );
        assert_eq!(cn.check_trigger(5, "第 47 天"), TriggerCheck::Satisfied);
        // 不同时钟仍不满足（未注入过、时钟非空）
        assert_eq!(task.check_trigger(5, "Day 2"), TriggerCheck::NotSatisfied);
    }

    #[test]
    fn test_story_time_already_injected_survives_clock_advance() {
        let mut task = StoryTask::user_planned(
            Id::new(),
            "伏笔",
            "第2年6月的伏笔",
            vec![TaskTrigger::StoryTime {
                target: "第2年6月".into(),
            }],
            5,
        );
        assert_eq!(task.check_trigger(10, "第2年6月"), TriggerCheck::Satisfied);
        task.mark_injected(10);
        // 时钟越过目标：严格相等不再命中，但任务不得静默消失（D-04）
        assert_eq!(
            task.check_trigger(20, "第2年7月"),
            TriggerCheck::NeedsAgentJudgment
        );
        let out = render_tasks_for_injection(std::slice::from_ref(&task), 20, "第2年7月");
        assert!(out.contains("伏笔"));
    }

    #[test]
    fn test_story_time_blank_target_is_ignored() {
        let task = StoryTask::user_planned(
            Id::new(),
            "空目标",
            "无意义触发器",
            vec![TaskTrigger::StoryTime {
                target: "   ".into(),
            }],
            5,
        );
        // 空 target 不参与判定：既不算命中，也不产生未决判断
        assert_eq!(task.check_trigger(5, ""), TriggerCheck::NotSatisfied);
        assert_eq!(task.check_trigger(5, "第1天"), TriggerCheck::NotSatisfied);
    }

    #[test]
    fn render_splits_satisfied_and_pending_judgment_groups() {
        // Lead 裁定 D-04：两类结果必须分组 + 指令不同，不得混在同一标题下
        let campaign = Id::new();
        let satisfied = StoryTask::user_planned(
            campaign.clone(),
            "已满足伏笔",
            "第2天的约定已到期",
            vec![TaskTrigger::StoryTime {
                target: "第2天".into(),
            }],
            1,
        );
        let mut need_judgment = StoryTask::user_planned(
            campaign.clone(),
            "待判断伏笔",
            "第2年6月触发复仇",
            vec![TaskTrigger::StoryTime {
                target: "第1天".into(),
            }],
            1,
        );
        // 已注入过 + 时钟已越过目标 → 严格相等不再命中，交 Agent 判断
        need_judgment.mark_injected(9);
        assert_eq!(
            need_judgment.check_trigger(10, "第2天"),
            TriggerCheck::NeedsAgentJudgment
        );
        let out = render_tasks_for_injection(&[satisfied, need_judgment], 10, "第2天");

        assert!(out.contains("【已满足条件的任务/伏笔】"), "{out}");
        assert!(out.contains("【待判断的任务/伏笔】"), "{out}");
        // 旧标题（两类混用）不得再出现
        assert!(!out.contains("【即将触发的任务/伏笔】"), "{out}");
        // 两类各归其组：已满足在前、待判断在后，且不交叉
        let idx_satisfied = out.find("已满足伏笔").expect("已满足项必须在输出里");
        let idx_pending = out.find("待判断伏笔").expect("待判断项必须在输出里");
        let idx_pending_title = out.find("【待判断的任务/伏笔】").unwrap();
        assert!(
            idx_satisfied < idx_pending_title,
            "已满足项必须落在『待判断』标题之前：{out}"
        );
        assert!(idx_pending > idx_pending_title, "{out}");
    }

    #[test]
    fn render_pending_judgment_group_contains_no_spoiler_instruction() {
        let task = StoryTask::user_planned(
            Id::new(),
            "待判断",
            "远期伏笔",
            vec![TaskTrigger::StoryTime {
                target: "第2年6月".into(),
            }],
            1,
        );
        // 时钟缺失 → NeedsAgentJudgment
        let out = render_tasks_for_injection(std::slice::from_ref(&task), 10, "");
        assert!(out.contains("【待判断的任务/伏笔】"), "{out}");
        assert!(
            !out.contains("【已满足条件的任务/伏笔】"),
            "只有待判断项时不得输出『已满足』标题：{out}"
        );
        assert!(
            out.contains("不适用则忽略") && out.contains("不要提前揭示"),
            "待判断组必须带防剧透指令：{out}"
        );
    }

    #[test]
    fn render_missing_clock_story_time_lands_in_pending_group() {
        // 第③条：时钟缺失 + StoryTime → 「待判断」组，而不是「已满足」组
        let task = StoryTask::user_planned(
            Id::new(),
            "复仇",
            "到第2年6月触发复仇",
            vec![TaskTrigger::StoryTime {
                target: "第2年6月".into(),
            }],
            5,
        );
        let out = render_tasks_for_injection(std::slice::from_ref(&task), 10, "");
        assert!(out.contains("【待判断的任务/伏笔】"), "{out}");
        assert!(!out.contains("【已满足条件的任务/伏笔】"), "{out}");
        assert!(out.contains("复仇"), "{out}");
    }

    #[test]
    fn render_returns_empty_string_when_nothing_injectable() {
        // 第④条：空集合 → ""（不得输出空标题，保持既有字节级契约）
        assert_eq!(render_tasks_for_injection(&[], 5, "第1天"), "");
        let far = StoryTask::user_planned(
            Id::new(),
            "未来",
            "远期",
            vec![TaskTrigger::TurnReminder { at_turn: 100 }],
            0,
        );
        assert_eq!(render_tasks_for_injection(&[far], 5, "第1天"), "");
        // 只有 Satisfied 时不得出现「待判断」标题
        let done = StoryTask::user_planned(
            Id::new(),
            "到期",
            "第1天到期",
            vec![TaskTrigger::StoryTime {
                target: "第1天".into(),
            }],
            1,
        );
        let out = render_tasks_for_injection(&[done], 5, "第1天");
        assert!(out.contains("【已满足条件的任务/伏笔】"), "{out}");
        assert!(!out.contains("【待判断的任务/伏笔】"), "{out}");
    }

    #[test]
    fn test_active_status_alone_counts_as_injected() {
        // 后处理 Agent 可直接把状态置 Active（无 injected_turns）：
        // 该任务同样受"越过目标不消失"保护
        let mut task = StoryTask::user_planned(
            Id::new(),
            "外部激活",
            "由后处理置 Active",
            vec![TaskTrigger::StoryTime {
                target: "第3天".into(),
            }],
            5,
        );
        task.status = TaskStatus::Active;
        assert_eq!(
            task.check_trigger(9, "第9天"),
            TriggerCheck::NeedsAgentJudgment
        );
    }
}

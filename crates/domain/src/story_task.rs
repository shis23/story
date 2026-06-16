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
    /// - StoryTime：需字符串比对（简化实现：相等即触发）→ Some(bool)
    /// - Event：语义判断，返回 None（调用方应查询后处理 Agent 的判断结果）
    /// - Manual：永远不自动触发 → Some(false)
    pub fn check_trigger(&self, current_turn: u32, story_clock: &str) -> TriggerCheck {
        // 第一遍：扫描确定性触发器（Turn / StoryTime）。
        // 必须先于 Event 判断，否则任务同时配了 [Event, TurnReminder{at_turn:1}]
        // 且当前 turn=1 时，本应确定性 Satisfied 却因 Event 排前面而浪费一次 Agent 调用。
        let mut has_event = false;
        for trigger in &self.triggers {
            match trigger {
                TaskTrigger::TurnReminder { at_turn } => {
                    if current_turn >= *at_turn {
                        return TriggerCheck::Satisfied;
                    }
                }
                TaskTrigger::StoryTime { target } => {
                    // 大小写不敏感比较（M-30），并去除首尾空白
                    if story_clock.trim().eq_ignore_ascii_case(target.trim()) {
                        return TriggerCheck::Satisfied;
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
        // 第二遍：没有确定性触发器命中，但存在 Event 触发器 → 需 Agent 语义判断
        if has_event {
            return TriggerCheck::NeedsAgentJudgment;
        }
        TriggerCheck::NotSatisfied
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

/// 把待注入的任务渲染成文本，拼进导演 user message 末尾
///
/// 只注入 Pending/Active 且触发条件满足的任务。
/// LikelyCompleted(>0.8) 的不自动注入（避免误判消失），由前端提示用户确认。
pub fn render_tasks_for_injection(
    tasks: &[StoryTask],
    current_turn: u32,
    story_clock: &str,
) -> String {
    let to_inject: Vec<&StoryTask> = tasks
        .iter()
        .filter(|t| t.status.is_injectable())
        .filter(|t| {
            matches!(
                t.check_trigger(current_turn, story_clock),
                TriggerCheck::Satisfied | TriggerCheck::NeedsAgentJudgment
            )
        })
        .collect();

    if to_inject.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("【即将触发的任务/伏笔】\n");
    for t in to_inject {
        out.push_str(&format!("- {}：{}\n", t.title, t.description));
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
        assert_eq!(
            task.check_trigger(29, "第1天"),
            TriggerCheck::NotSatisfied
        );
        assert_eq!(
            task.check_trigger(30, "第1天"),
            TriggerCheck::Satisfied
        );
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
        assert_eq!(
            task.check_trigger(100, "第2年6月"),
            TriggerCheck::Satisfied
        );
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
                TaskTrigger::Event { description: "某事件".into() },
                TaskTrigger::TurnReminder { at_turn: 1 },
            ],
            5,
        );
        assert_eq!(task.check_trigger(1, "第1天"), TriggerCheck::Satisfied);

        // 都没命中时，有 Event 才返回 NeedsAgentJudgment
        assert_eq!(task.check_trigger(0, "第99天"), TriggerCheck::NeedsAgentJudgment);
    }

    #[test]
    fn test_manual_never_auto_triggers() {
        let task = StoryTask::user_planned(
            Id::new(),
            "手动",
            "手动任务",
            vec![TaskTrigger::Manual],
            5,
        );
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
}

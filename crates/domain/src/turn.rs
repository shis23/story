//! Turn 提交屏障领域模型（Phase A）
//!
//! 对应 `docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md` P0：
//! 一轮故事缺少明确的提交事务。
//!
//! 核心思想：
//! - `PipelineState` 描述 Agent 执行进度（Directing / Editing / Review）。
//! - `TurnStatus` 描述本轮数据的一致性（草稿 / 状态推导 / Campaign 变更是否已提交）。
//!
//! 两者是正交维度，不在同一枚举上扩展。
//!
//! 一个 `TurnRecord` 代表一次用户意图触发的完整轮次，包含多个 `TurnAttempt`
//! （regenerate/swipe 变体）。每个 Attempt 持有自己的候选状态变更
//! （`MutationBatch`），accept 只应用 `variant_id + draft_hash` 精确匹配的那个。

use crate::Id;
use crate::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource, PropagationPolicy};
use crate::conversation::Provenance;
use crate::story_task::{StoryTask, TaskStatus};
use serde::{Deserialize, Serialize};

// ─── Turn 级状态 ───────────────────────────────────────────────────────────

/// 一轮故事的提交一致性状态。
///
/// 与 `PipelineState` 正交：PipelineState 跟踪 Agent 执行进度，
/// TurnStatus 跟踪本轮数据是否已完整提交到 Campaign。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    /// 生成中（pipeline 运行中）
    Generating,
    /// 草稿已就绪（Editor 成文，DraftReady）
    DraftReady,
    /// 正在推导候选状态（postprocess 运行中）
    DerivingState,
    /// 等待用户采纳（候选 diff 已暂存到 TurnAttempt）
    AwaitingAcceptance,
    /// 正在提交（原子写入 CampaignStore + revision bump）
    Committing,
    /// 已提交（正文 Final + 状态变更 + revision 已更新）
    Committed,
    /// 降级提交（用户明确接受正文，但状态推导失败或被配置关闭）
    Degraded,
    /// 失败（生成或提交失败，无副作用）
    Failed,
    /// 放弃（用户主动终止，user 消息和 AI 草稿都排除）
    Abandoned,
}

impl TurnStatus {
    /// 终态：不需要再继续处理。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Committed | Self::Degraded | Self::Failed | Self::Abandoned
        )
    }

    /// 活动：非终态，下一轮 start_writing 必须等待。
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }

    /// 是否已有副作用开始（Committing 态幂等重放恢复）。
    pub fn has_side_effects_started(&self) -> bool {
        matches!(self, Self::Committing)
    }
}

// ─── Attempt 级状态 ─────────────────────────────────────────────────────────

/// 单个变体（regenerate/swipe）的提交状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    /// 生成中
    Generating,
    /// 草稿就绪
    DraftReady,
    /// 正在推导状态
    DerivingState,
    /// 等待采纳
    AwaitingAcceptance,
    /// 正在提交
    Committing,
    /// 已提交（被 accept）
    Committed,
    /// 已过期（draft_hash 因编辑而变化，候选 diff 不再匹配）
    Stale,
    /// 已丢弃（单个 AI 变体 discard，Turn 仍开放）
    Discarded,
    /// 已被后续 Attempt 取代（regenerate 后旧 Attempt）
    Superseded,
    /// 失败
    Failed,
}

impl AttemptStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Committed | Self::Stale | Self::Discarded | Self::Superseded | Self::Failed
        )
    }

    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

// ─── 状态推导结果 ───────────────────────────────────────────────────────────

/// postprocess 的推导结果（独立于用户是否 accept）。
///
/// 区分"配置主动关闭"与"实际推导失败"：
/// - `SkippedByPolicy` → 正常 Committed（空 diff）。
/// - `Failed` → 允许 Degraded（用户显式接受不完整状态）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DerivationOutcome {
    /// 推导成功，产出候选 diff
    Succeeded,
    /// 被配置关闭（enable_postprocess=false / enable_summarizer=false）
    SkippedByPolicy,
    /// 推导尝试了但失败（LLM 超时、解析失败等）
    Failed(String),
}

/// 单个推导组件（summary / state）的状态，分别追踪。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivationStatus {
    /// 被配置关闭
    Disabled,
    /// 成功
    Succeeded,
    /// 失败
    Failed,
}

impl DerivationStatus {
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// summary 和 state 推导的分别结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivationComponents {
    /// 剧情总结 Agent 的推导状态
    pub summary_derivation: DerivationStatus,
    /// 后处理三合一 Agent 的推导状态
    pub state_derivation: DerivationStatus,
}

impl DerivationComponents {
    /// 两者都被配置关闭 → 正常 Committed（SkippedByPolicy）。
    pub fn is_all_disabled(&self) -> bool {
        self.summary_derivation.is_disabled() && self.state_derivation.is_disabled()
    }

    /// 任一实际失败 → 允许 Degraded。
    pub fn has_failure(&self) -> bool {
        self.summary_derivation.is_failed() || self.state_derivation.is_failed()
    }
}

// ─── MutationBatch（幂等可重放操作集）──────────────────────────────────────

/// 一条可幂等重放的 Campaign 变更操作。
///
/// 设计原则（收敛决策步骤 16-18）：
/// - 变量写**绝对值**（不是 `+1` 增量），重放安全。
/// - 已有任务更新**绝对状态**（`TaskStatus` 枚举），天然幂等。
/// - 新建知识/任务使用 Prepare 阶段预分配的稳定 ID，重放复用。
/// - upsert 三态：ID 不存在→插入；ID 存在 payload 一致→no-op；不一致→Conflict。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mutation {
    /// 设置变量为绝对值（角色级 instance_id 或全局 None）
    SetVariable {
        instance_id: Option<Id>,
        key: String,
        value: serde_json::Value,
        /// 写入时的轮次（last_updated_turn）
        turn: u32,
    },
    /// upsert 知识条目（预分配 entry_id）
    UpsertKnowledge(Box<KnowledgeMutation>),
    /// 设置已有任务的绝对状态
    SetTaskStatus { task_id: Id, status: TaskStatus },
    /// upsert 新建任务（预分配 task_id）
    UpsertNewTask(Box<StoryTask>),
    /// upsert 本轮摘要（campaign_id + turn 幂等键）
    UpsertSummary(Box<crate::agent::RoundSummary>),
    /// 将 AI 草稿 Draft → Final
    FinalizeVariant { variant_id: Id },
}

/// 知识 mutation 的完整载荷（对应 `CharacterKnowledgeEntry`，预分配 ID）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeMutation {
    pub entry_id: Id,
    pub campaign_id: Id,
    pub character_id: Id,
    pub knowledge_text: String,
    pub source: KnowledgeSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_character_id: Option<Id>,
    pub turn_number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<Id>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub propagation: PropagationPolicy,
}

impl KnowledgeMutation {
    /// 转成持久化 entry（使用预分配的 entry_id，不重新生成）
    pub fn to_entry(&self) -> CharacterKnowledgeEntry {
        CharacterKnowledgeEntry {
            id: self.entry_id.clone(),
            campaign_id: self.campaign_id.clone(),
            character_id: self.character_id.clone(),
            knowledge_text: self.knowledge_text.clone(),
            source: self.source.clone(),
            source_character_id: self.source_character_id.clone(),
            source_knowledge_id: None,
            turn_number: self.turn_number,
            event_id: self.event_id.clone(),
            pinned: self.pinned,
            propagation: self.propagation.clone(),
        }
    }
}

/// MutationBatch 的执行状态（崩溃恢复用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationBatchStatus {
    /// 已构建但未开始执行（无副作用）
    Prepared,
    /// 正在执行（可能有部分写入）
    Applying,
    /// 已完成
    Committed,
}

/// 一轮提交的完整变更集。
///
/// 在任何副作用发生前，包含全部预分配 ID 的 Prepared MutationBatch 必须已原子落盘。
/// 崩溃后重放读取同一 MutationBatch，自然复用同一批 ID。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationBatch {
    /// 本次提交的唯一 ID
    pub commit_id: Id,
    /// 提交前期望的 Campaign revision（CAS 校验）
    pub expected_revision: u64,
    /// 提交后的目标 revision（expected + 1）
    pub target_revision: u64,
    /// 执行状态
    pub status: MutationBatchStatus,
    /// 全部变更操作（顺序敏感）
    pub mutations: Vec<Mutation>,
}

impl MutationBatch {
    pub fn new(commit_id: Id, expected_revision: u64) -> Self {
        Self {
            commit_id,
            expected_revision,
            target_revision: expected_revision + 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }
}

// ─── TurnAttempt（单个变体的提交上下文）──────────────────────────────────

/// 一个 Turn 中的单次生成尝试（regenerate/swipe 创建新 Attempt）。
///
/// 每个 Attempt 持有自己的 `pending_state_changes`（候选 diff），
/// 通过 `variant_id + draft_hash` 绑定到具体的变体内容。
/// edit → draft_hash 改变 → 原候选 diff 标 Stale。
/// discard → 只丢弃这个 Attempt，不影响 Campaign。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnAttempt {
    pub attempt_id: Id,
    /// 对话树中的 AI Draft variant 节点 ID
    pub variant_id: Id,
    /// 草稿内容的 hash（SHA-256），用于检测编辑后 diff 失效
    pub draft_hash: String,
    pub status: AttemptStatus,
    /// 候选状态变更（postprocess 推导产出），None = 未推导
    pub pending_state_changes: Option<MutationBatch>,
    /// 推导状态追踪（summary / state 分别记录）
    pub derivation: Option<DerivationComponents>,
    /// 溯源信息（用于部分重 roll）
    pub provenance: Option<Provenance>,
    pub created_at: String,
}

// ─── TurnRecord（完整轮次的提交上下文）──────────────────────────────────

/// 一次用户意图触发的完整轮次。
///
/// 包含多个 `TurnAttempt`（regenerate/swipe）。
/// `accepted_attempt_id` 指向最终被采纳的 Attempt。
/// `input_node_id` 是触发本轮的 user 消息节点——Abandon Turn 时需要同时排除它。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_id: Id,
    pub campaign_id: Id,
    pub conversation_id: Id,
    /// 触发本轮的 user 消息节点（Abandon Turn 时一起排除）
    pub input_node_id: Id,
    /// 本轮开始时的 Campaign revision
    pub base_campaign_revision: u64,
    pub status: TurnStatus,
    pub attempts: Vec<TurnAttempt>,
    /// 最终被采纳的 Attempt（None = 未 accept）
    pub accepted_attempt_id: Option<Id>,
    /// 失败原因（Failed/Degraded 时）
    pub failure_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl TurnRecord {
    /// 创建新 TurnRecord（status = Generating）
    pub fn new(
        campaign_id: Id,
        conversation_id: Id,
        input_node_id: Id,
        base_campaign_revision: u64,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            turn_id: Id::new(),
            campaign_id,
            conversation_id,
            input_node_id,
            base_campaign_revision,
            status: TurnStatus::Generating,
            attempts: vec![],
            accepted_attempt_id: None,
            failure_reason: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// 获取当前活动 Attempt（status 非 terminal 的最新 Attempt）
    pub fn active_attempt(&self) -> Option<&TurnAttempt> {
        self.attempts.iter().rev().find(|a| a.status.is_active())
    }

    /// 获取活动 Attempt 的可变引用
    pub fn active_attempt_mut(&mut self) -> Option<&mut TurnAttempt> {
        self.attempts
            .iter_mut()
            .rev()
            .find(|a| a.status.is_active())
    }

    /// 按 attempt_id 查找
    pub fn find_attempt(&self, attempt_id: &Id) -> Option<&TurnAttempt> {
        self.attempts.iter().find(|a| &a.attempt_id == attempt_id)
    }

    /// 按 variant_id 查找——只返回活动（非 terminal）Attempt。
    ///
    /// P0-1 修复：regenerate 在同一 node 上创建新 Attempt 后，旧 Attempt 已 Superseded。
    /// 如果返回第一个匹配（旧的），accept 会命中旧 Attempt。改为跳过 terminal 态。
    pub fn find_attempt_by_variant(&self, variant_id: &Id) -> Option<&TurnAttempt> {
        self.attempts
            .iter()
            .find(|a| &a.variant_id == variant_id && a.status.is_active())
    }

    /// 按 attempt_id 查找（可变）
    pub fn find_attempt_mut(&mut self, attempt_id: &Id) -> Option<&mut TurnAttempt> {
        self.attempts
            .iter_mut()
            .find(|a| &a.attempt_id == attempt_id)
    }

    /// 按 variant_id 查找（可变）——只返回活动（非 terminal）Attempt（同 P0-1 修复）
    pub fn find_attempt_by_variant_mut(&mut self, variant_id: &Id) -> Option<&mut TurnAttempt> {
        self.attempts
            .iter_mut()
            .find(|a| &a.variant_id == variant_id && a.status.is_active())
    }

    /// 更新 updated_at 时间戳
    pub fn touch(&mut self) {
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

// ─── B3：DraftQualityGate 质量报告（架构文档 §9）───────────────────────────

/// 质量警告严重级别
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualitySeverity {
    /// 轻微问题，不阻塞后续流程
    Warning,
    /// 严重问题，建议人工审查
    Error,
}

/// 质量警告编码（稳定错误码，便于前端区分展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QualityWarningCode {
    /// n-gram 重复：同一段 N 字连续出现 ≥ K 次
    NgramRepetition {
        n: usize,
        count: usize,
        sample: String,
    },
    /// 元描述泄漏：草稿含 LLM 自述/指令残留
    MetaDescription { snippet: String },
    /// 字数过短
    TooShort { char_count: usize },
}

/// 单条质量警告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityWarning {
    pub code: QualityWarningCode,
    pub message: String,
    pub severity: QualitySeverity,
}

/// 草稿质量报告（DraftQualityGate 输出）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityReport {
    pub warnings: Vec<QualityWarning>,
}

impl QualityReport {
    /// 门禁通过 = 无任何警告
    pub fn passed(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Error 级别警告数
    pub fn error_count(&self) -> usize {
        self.warnings
            .iter()
            .filter(|w| w.severity == QualitySeverity::Error)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_report_passed_when_empty() {
        let report = QualityReport::default();
        assert!(report.passed());
        assert_eq!(report.error_count(), 0);
    }

    #[test]
    fn quality_report_failed_with_warnings() {
        let report = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::TooShort { char_count: 10 },
                message: "过短".into(),
                severity: QualitySeverity::Warning,
            }],
        };
        assert!(!report.passed());
        assert_eq!(report.error_count(), 0); // Warning 不是 Error
    }

    #[test]
    fn quality_report_error_count() {
        let report = QualityReport {
            warnings: vec![
                QualityWarning {
                    code: QualityWarningCode::TooShort { char_count: 10 },
                    message: "过短".into(),
                    severity: QualitySeverity::Warning,
                },
                QualityWarning {
                    code: QualityWarningCode::MetaDescription {
                        snippet: "作为AI".into(),
                    },
                    message: "元描述".into(),
                    severity: QualitySeverity::Error,
                },
            ],
        };
        assert!(!report.passed());
        assert_eq!(report.error_count(), 1);
    }

    #[test]
    fn turn_status_terminal_states() {
        assert!(TurnStatus::Committed.is_terminal());
        assert!(TurnStatus::Degraded.is_terminal());
        assert!(TurnStatus::Failed.is_terminal());
        assert!(TurnStatus::Abandoned.is_terminal());

        assert!(!TurnStatus::Generating.is_terminal());
        assert!(!TurnStatus::AwaitingAcceptance.is_terminal());
        assert!(!TurnStatus::Committing.is_terminal());
    }

    #[test]
    fn turn_status_active_is_not_terminal() {
        for s in [
            TurnStatus::Generating,
            TurnStatus::DraftReady,
            TurnStatus::DerivingState,
            TurnStatus::AwaitingAcceptance,
            TurnStatus::Committing,
        ] {
            assert!(s.is_active(), "{:?} should be active", s);
        }
        for s in [
            TurnStatus::Committed,
            TurnStatus::Degraded,
            TurnStatus::Failed,
            TurnStatus::Abandoned,
        ] {
            assert!(!s.is_active(), "{:?} should not be active", s);
        }
    }

    #[test]
    fn turn_status_committing_has_side_effects() {
        assert!(TurnStatus::Committing.has_side_effects_started());
        assert!(!TurnStatus::AwaitingAcceptance.has_side_effects_started());
        assert!(!TurnStatus::Committed.has_side_effects_started());
    }

    #[test]
    fn attempt_status_terminal_states() {
        assert!(AttemptStatus::Committed.is_terminal());
        assert!(AttemptStatus::Stale.is_terminal());
        assert!(AttemptStatus::Discarded.is_terminal());
        assert!(AttemptStatus::Superseded.is_terminal());
        assert!(AttemptStatus::Failed.is_terminal());

        assert!(!AttemptStatus::Generating.is_terminal());
        assert!(!AttemptStatus::AwaitingAcceptance.is_terminal());
    }

    #[test]
    fn derivation_status_disabled_and_failed() {
        assert!(DerivationStatus::Disabled.is_disabled());
        assert!(!DerivationStatus::Succeeded.is_disabled());

        assert!(DerivationStatus::Failed.is_failed());
        assert!(!DerivationStatus::Succeeded.is_failed());
    }

    #[test]
    fn derivation_components_all_disabled() {
        let both_disabled = DerivationComponents {
            summary_derivation: DerivationStatus::Disabled,
            state_derivation: DerivationStatus::Disabled,
        };
        assert!(both_disabled.is_all_disabled());
        assert!(!both_disabled.has_failure());
    }

    #[test]
    fn derivation_components_has_failure() {
        let one_failed = DerivationComponents {
            summary_derivation: DerivationStatus::Succeeded,
            state_derivation: DerivationStatus::Failed,
        };
        assert!(!one_failed.is_all_disabled());
        assert!(one_failed.has_failure());
    }

    #[test]
    fn mutation_batch_new_sets_target_revision() {
        let batch = MutationBatch::new(Id::from_str("c1"), 42);
        assert_eq!(batch.expected_revision, 42);
        assert_eq!(batch.target_revision, 43);
        assert_eq!(batch.status, MutationBatchStatus::Prepared);
        assert!(batch.is_empty());
    }

    #[test]
    fn turn_record_new_starts_generating() {
        let record = TurnRecord::new(
            Id::from_str("camp-1"),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            5,
        );
        assert_eq!(record.status, TurnStatus::Generating);
        assert_eq!(record.base_campaign_revision, 5);
        assert!(record.attempts.is_empty());
        assert!(record.accepted_attempt_id.is_none());
        assert!(record.active_attempt().is_none());
    }

    #[test]
    fn turn_record_find_attempt_by_variant() {
        let mut record = TurnRecord::new(
            Id::from_str("camp-1"),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        );
        let variant_id = Id::from_str("var-1");
        record.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("att-1"),
            variant_id: variant_id.clone(),
            draft_hash: "abc".into(),
            status: AttemptStatus::AwaitingAcceptance,
            pending_state_changes: None,
            derivation: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });

        assert!(record.find_attempt_by_variant(&variant_id).is_some());
        assert!(record.active_attempt().is_some());
        assert_eq!(
            record.active_attempt().unwrap().attempt_id,
            Id::from_str("att-1")
        );
    }

    #[test]
    fn turn_record_active_attempt_skips_terminal() {
        let mut record = TurnRecord::new(
            Id::from_str("camp-1"),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        );
        record.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("att-1"),
            variant_id: Id::from_str("var-1"),
            draft_hash: "abc".into(),
            status: AttemptStatus::Superseded,
            pending_state_changes: None,
            derivation: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        record.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("att-2"),
            variant_id: Id::from_str("var-2"),
            draft_hash: "def".into(),
            status: AttemptStatus::AwaitingAcceptance,
            pending_state_changes: None,
            derivation: None,
            provenance: None,
            created_at: "2026-01-01T00:01:00Z".into(),
        });

        let active = record.active_attempt().unwrap();
        assert_eq!(active.attempt_id, Id::from_str("att-2"));
    }
}

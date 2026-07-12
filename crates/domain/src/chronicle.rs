//! Chronicle / ContextEpoch pure model (Memory Spec v1.0 M0).
//!
//! 规范：`docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`
//!
//! 本模块只放**身份、epoch 成员公式、revision 规则、确定性分组校验、Compiler IO 草图**。
//! 不接线 Director prompt、不写盘、不调 LLM。

use serde::{Deserialize, Serialize};

use crate::Id;

// ─── 实验默认参数（可配置；改参数 ≠ 改架构）────────────────────────────────

/// epoch 开始时近正文锚点轮数。
pub const DEFAULT_H_ANCHOR: u32 = 5;
/// 一个 context epoch 内允许追加的新提交 Turn 个数上限。
pub const DEFAULT_E: u32 = 10;
/// 纪要带宽度；默认与 E 相同。
pub const DEFAULT_S: u32 = DEFAULT_E;
/// 概览最大行数（entries 帽；token 帽另配）。
pub const DEFAULT_OVERVIEW_MAX_ENTRIES: usize = 200;
/// active 未覆盖 A 达阈值后批压。
pub const DEFAULT_COMPRESS_ACTIVE_A_THRESHOLD: usize = 200;
/// 每组连续 A 数 → 1 个 B（200/4=50）。
pub const DEFAULT_COMPRESS_GROUP_SIZE: usize = 4;
/// active 未覆盖 B 达阈值后批压为 C。
pub const DEFAULT_COMPRESS_ACTIVE_B_THRESHOLD: usize = 200;
/// 自动 hybrid 兜底条数上限（实现可取 1..=k）。
pub const DEFAULT_AUTO_RECALL_K: usize = 2;
/// 每轮 get_chronicle(summary) 上限。
pub const DEFAULT_TOOL_SUMMARY_MAX: u32 = 6;
/// 每轮 get_chronicle(full) 上限。
pub const DEFAULT_TOOL_FULL_MAX: u32 = 2;
/// 每轮 search_chronicle 上限。
pub const DEFAULT_SEARCH_MAX: u32 = 3;
/// Compiler / snapshot 算法版本（防漂移假稳定）。
pub const CONTEXT_COMPILER_VERSION: &str = "memory-spec-v1.0-m4.2.1";

/// 压缩发布意图（磁盘侧半提交恢复用）。
///
/// 流程：写 summaries 前/后写入 campaign.pending；metadata 成功后清空。
/// heal 只依赖本 marker，不依赖 context_epoch 是否仍存在。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCompressPublication {
    pub publication_id: Id,
    /// 发布前的 chronicle_revision（成功后应 > 该值）
    pub base_chronicle_revision: u64,
    /// 本批 parent stage summary ids
    pub parent_ids: Vec<Id>,
    /// child_id → parent_id 覆盖关系（完成前必须在 summaries 中成立）
    #[serde(default)]
    pub child_covered_by: Vec<(Id, Id)>,
    /// 旧字段兼容（仅反序列化；新写入不再使用）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_ids: Vec<Id>,
    pub created_at: String,
}

impl PendingCompressPublication {
    pub fn new(
        base_chronicle_revision: u64,
        parent_ids: Vec<Id>,
        child_covered_by: Vec<(Id, Id)>,
    ) -> Self {
        Self {
            publication_id: Id::new(),
            base_chronicle_revision,
            parent_ids,
            child_covered_by,
            child_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

// ─── 身份与层级 ────────────────────────────────────────────────────────────

/// Chronicle 层级：A=leaf, B/C=stage。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChronicleLevel {
    A = 0,
    B = 1,
    C = 2,
}

impl ChronicleLevel {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn code_prefix(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::A),
            1 => Some(Self::B),
            2 => Some(Self::C),
            _ => None,
        }
    }
}

/// 人类/模型可读 code：`A0123` / `B0042` / `C0007`。
///
/// **非**全局主键；唯一域为 `(campaign_id, lineage_id)`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChronicleCode(String);

impl ChronicleCode {
    pub fn new(level: ChronicleLevel, seq: u32) -> Self {
        Self(format!("{}{:04}", level.code_prefix(), seq))
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.len() < 2 {
            return None;
        }
        let mut chars = raw.chars();
        let prefix = chars.next()?;
        let level = match prefix {
            'A' | 'a' => ChronicleLevel::A,
            'B' | 'b' => ChronicleLevel::B,
            'C' | 'c' => ChronicleLevel::C,
            _ => return None,
        };
        let digits: String = chars.collect();
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let seq: u32 = digits.parse().ok()?;
        Some(Self::new(level, seq))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn level(&self) -> Option<ChronicleLevel> {
        match self.0.chars().next()? {
            'A' => Some(ChronicleLevel::A),
            'B' => Some(ChronicleLevel::B),
            'C' => Some(ChronicleLevel::C),
            _ => None,
        }
    }
}

impl std::fmt::Display for ChronicleCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 向量 / 检索平面的来源种类（M1 起写入索引）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceKind {
    ChronicleA,
    ChronicleB,
    ChronicleC,
    RoundSummaryLegacy,
    ArchivedMessage,
}

impl MemorySourceKind {
    pub fn from_level(level: ChronicleLevel) -> Self {
        match level {
            ChronicleLevel::A => Self::ChronicleA,
            ChronicleLevel::B => Self::ChronicleB,
            ChronicleLevel::C => Self::ChronicleC,
        }
    }
}

/// Chronicle 条目（规范叙事纪要；A 由 RoundSummary 演进）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChronicleEntry {
    /// 唯一主键
    pub id: Id,
    pub code: ChronicleCode,
    pub level: ChronicleLevel,
    pub campaign_id: Id,
    /// 当前对话分支可用的记忆线
    pub lineage_id: Id,
    pub headline: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
    /// 含端点闭区间，按已提交 Turn 序号
    pub turn_start: u32,
    pub turn_end: u32,
    /// 系统填写：B/C 覆盖的子 **entry id**（非 LLM 自由决定）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered_by: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_turn_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_variant_hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_campaign_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_campaign_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_chronicle_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_code: Option<ChronicleCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidated_at: Option<String>,
    pub created_at: String,
}

impl ChronicleEntry {
    pub fn is_active_for(&self, campaign_id: &Id, lineage_id: &Id) -> bool {
        &self.campaign_id == campaign_id
            && &self.lineage_id == lineage_id
            && self.invalidated_at.is_none()
    }

    pub fn is_uncovered(&self) -> bool {
        self.covered_by.is_none()
    }

    pub fn is_leaf(&self) -> bool {
        self.level == ChronicleLevel::A
    }
}

// ─── lineage ───────────────────────────────────────────────────────────────

/// 新建写作线时分配 lineage_id。
pub fn new_lineage_id() -> Id {
    Id::new()
}

/// 条目是否适用于当前分支（规格 §3.3）。
pub fn entry_applies_to_lineage(entry: &ChronicleEntry, campaign_id: &Id, lineage_id: &Id) -> bool {
    entry.is_active_for(campaign_id, lineage_id)
}

// ─── revision 规则 ─────────────────────────────────────────────────────────

/// 导致 `chronicle_revision` 必须递增的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChronicleRevisionBumpReason {
    AcceptChronicleA,
    EpochRollover,
    CompressPublish,
    ForkOrImportOrMigration,
    LineageInvalidation,
}

/// 是否应 bump chronicle_revision（规则表，供调用方决策）。
pub fn should_bump_chronicle_revision(reason: ChronicleRevisionBumpReason) -> bool {
    // 规格 §3.4：所列情况全部必须递增
    matches!(
        reason,
        ChronicleRevisionBumpReason::AcceptChronicleA
            | ChronicleRevisionBumpReason::EpochRollover
            | ChronicleRevisionBumpReason::CompressPublish
            | ChronicleRevisionBumpReason::ForkOrImportOrMigration
            | ChronicleRevisionBumpReason::LineageInvalidation
    )
}

// ─── Context epoch 成员公式 ────────────────────────────────────────────────

/// 窗口 / epoch 可配置参数（实验默认见常量）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextWindowParams {
    pub h_anchor: u32,
    pub e: u32,
    pub s: u32,
    pub overview_max_entries: usize,
}

impl Default for ContextWindowParams {
    fn default() -> Self {
        Self {
            h_anchor: DEFAULT_H_ANCHOR,
            e: DEFAULT_E,
            s: DEFAULT_S,
            overview_max_entries: DEFAULT_OVERVIEW_MAX_ENTRIES,
        }
    }
}

impl ContextWindowParams {
    /// 近正文最大轮数 = H_anchor + E（规格写死）。
    pub fn max_near_raw_turns(&self) -> u32 {
        self.h_anchor.saturating_add(self.e)
    }
}

/// 已提交 Turn 在 lineage 上的有序序号视图（纯数据；调用方映射 conversation）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedTurnRef {
    pub turn_id: Id,
    /// 1-based 或 0-based 均可，只要**同一 lineage 内单调且稳定**；本模块只做序列下标运算。
    pub sequence: u32,
}

/// 一次编译捕获的上下文快照（Turn 全程冻结）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextCompileCapture {
    pub campaign_revision: u64,
    pub chronicle_revision: u64,
    pub epoch_id: String,
}

/// ContextEpochSnapshot 最小持久化字段（规格 §3.5）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextEpochSnapshot {
    pub epoch_id: String,
    /// 创建快照时 lineage 上最后一个已提交 Turn；尚无则为 None
    pub source_head_turn_id: Option<Id>,
    /// 有序；编译时再取 headline
    pub overview_codes: Vec<ChronicleCode>,
    pub band_codes: Vec<ChronicleCode>,
    /// 近正文锚点 Turn 列表（有序，旧→新）
    pub raw_anchor_turn_ids: Vec<Id>,
    pub compiler_version: String,
    pub chronicle_revision: u64,
    /// 对成员 + 关键正文/摘要 hash 的摘要（调用方填；空串表示未算）
    pub source_hash: String,
}

impl ContextEpochSnapshot {
    pub fn new_empty(epoch_id: impl Into<String>, chronicle_revision: u64) -> Self {
        Self {
            epoch_id: epoch_id.into(),
            source_head_turn_id: None,
            overview_codes: Vec::new(),
            band_codes: Vec::new(),
            raw_anchor_turn_ids: Vec::new(),
            compiler_version: CONTEXT_COMPILER_VERSION.to_string(),
            chronicle_revision,
            source_hash: String::new(),
        }
    }
}

/// epoch 成员计算结果（纯函数输出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochMembership {
    pub epoch_start_head: Option<Id>,
    pub anchor_turn_ids: Vec<Id>,
    pub live_suffix_turn_ids: Vec<Id>,
    pub near_raw_turn_ids: Vec<Id>,
    pub band_turn_ids: Vec<Id>,
    pub live_suffix_count: u32,
    pub needs_rollover_before_next_compile: bool,
}

/// 在已提交序列中定位 turn_id 的下标。
pub fn index_of_turn(committed: &[CommittedTurnRef], turn_id: &Id) -> Option<usize> {
    committed.iter().position(|t| &t.turn_id == turn_id)
}

/// 以 `head` 为末元素，向前取 `min(h_anchor, 已有)` 个 Turn（闭区间，时间升序）。
pub fn select_anchor_turns(
    committed: &[CommittedTurnRef],
    epoch_start_head: Option<&Id>,
    h_anchor: u32,
) -> Vec<Id> {
    let Some(head) = epoch_start_head else {
        return Vec::new();
    };
    let Some(head_idx) = index_of_turn(committed, head) else {
        return Vec::new();
    };
    let take = (h_anchor as usize).min(head_idx + 1);
    let start = head_idx + 1 - take;
    committed[start..=head_idx]
        .iter()
        .map(|t| t.turn_id.clone())
        .collect()
}

/// 严格晚于 epoch_start_head 的新提交 Turn（升序）。
pub fn select_live_suffix(
    committed: &[CommittedTurnRef],
    epoch_start_head: Option<&Id>,
) -> Vec<Id> {
    match epoch_start_head {
        None => committed.iter().map(|t| t.turn_id.clone()).collect(),
        Some(head) => {
            let Some(head_idx) = index_of_turn(committed, head) else {
                return Vec::new();
            };
            committed[head_idx + 1..]
                .iter()
                .map(|t| t.turn_id.clone())
                .collect()
        }
    }
}

/// 紧邻 anchor 之前、长度最多 S 的已提交 Turn。
pub fn select_band_turns(
    committed: &[CommittedTurnRef],
    anchor_turn_ids: &[Id],
    s: u32,
) -> Vec<Id> {
    if s == 0 || committed.is_empty() {
        return Vec::new();
    }
    let first_anchor = match anchor_turn_ids.first() {
        Some(id) => id,
        None => {
            // 无锚点：取全序列末尾最多 S（尚无 near raw 时纪要带可覆盖最近历史）
            let take = (s as usize).min(committed.len());
            let start = committed.len() - take;
            return committed[start..]
                .iter()
                .map(|t| t.turn_id.clone())
                .collect();
        }
    };
    let Some(first_idx) = index_of_turn(committed, first_anchor) else {
        return Vec::new();
    };
    if first_idx == 0 {
        return Vec::new();
    }
    let take = (s as usize).min(first_idx);
    let start = first_idx - take;
    committed[start..first_idx]
        .iter()
        .map(|t| t.turn_id.clone())
        .collect()
}

/// 计算当前 epoch 成员（规格 §4.1）。
///
/// `live_suffix_count == E` 时：第 E 个新 Turn **仍属本 epoch**；
/// `needs_rollover_before_next_compile = true`，在**下一次** Context 编译前 rollover。
pub fn compute_epoch_membership(
    committed: &[CommittedTurnRef],
    epoch_start_head: Option<&Id>,
    params: ContextWindowParams,
) -> EpochMembership {
    let anchor_turn_ids = select_anchor_turns(committed, epoch_start_head, params.h_anchor);
    let live_suffix_turn_ids = select_live_suffix(committed, epoch_start_head);
    // 防御：live_suffix 理论最多 E；若存储漂移多出，截到 E（仍标记 rollover）
    let capped_suffix: Vec<Id> = live_suffix_turn_ids
        .iter()
        .take(params.e as usize)
        .cloned()
        .collect();
    let live_suffix_count = capped_suffix.len() as u32;
    let mut near_raw = anchor_turn_ids.clone();
    near_raw.extend(capped_suffix.iter().cloned());
    // 硬上限 H_anchor + E
    let max_near = params.max_near_raw_turns() as usize;
    if near_raw.len() > max_near {
        near_raw.truncate(max_near);
    }
    let band_turn_ids = select_band_turns(committed, &anchor_turn_ids, params.s);
    let needs_rollover = live_suffix_count >= params.e && params.e > 0;
    EpochMembership {
        epoch_start_head: epoch_start_head.cloned(),
        anchor_turn_ids,
        live_suffix_turn_ids: capped_suffix,
        near_raw_turn_ids: near_raw,
        band_turn_ids,
        live_suffix_count,
        needs_rollover_before_next_compile: needs_rollover,
    }
}

/// 执行 rollover：以当前最后一个已提交 Turn 为新 head，清空 live_suffix。
pub fn rollover_epoch_head(
    committed: &[CommittedTurnRef],
    params: ContextWindowParams,
) -> EpochMembership {
    let new_head = committed.last().map(|t| t.turn_id.clone());
    // 新 epoch：live_suffix 为空；anchor = 以新 head 为末的 H_anchor
    compute_epoch_membership(committed, new_head.as_ref(), params)
}

// ─── 概览选择（纯函数草图）────────────────────────────────────────────────

/// 候选概览条目的最小输入。
#[derive(Debug, Clone)]
pub struct OverviewCandidate {
    pub code: ChronicleCode,
    pub level: ChronicleLevel,
    pub turn_start: u32,
    pub covered_by: Option<Id>,
}

/// 选择 overview_codes（先选后排，严格 turn_start 升序）。
///
/// - 仅 uncovered
/// - B/C 优先占位；超 cap 从最旧 B/C 丢弃
/// - 再补最近远 A（turn_start 严格早于 band 最早一轮）直到 cap
pub fn select_overview_codes(
    candidates: &[OverviewCandidate],
    band_earliest_turn: Option<u32>,
    max_entries: usize,
) -> Vec<ChronicleCode> {
    if max_entries == 0 {
        return Vec::new();
    }
    let uncovered: Vec<&OverviewCandidate> = candidates
        .iter()
        .filter(|c| c.covered_by.is_none())
        .collect();

    let mut stages: Vec<&OverviewCandidate> = uncovered
        .iter()
        .copied()
        .filter(|c| c.level == ChronicleLevel::B || c.level == ChronicleLevel::C)
        .collect();
    stages.sort_by_key(|c| c.turn_start);

    // 超 cap：丢最旧 stage，保留较新
    if stages.len() > max_entries {
        let drop_n = stages.len() - max_entries;
        stages = stages[drop_n..].to_vec();
    }

    let mut selected: Vec<&OverviewCandidate> = stages;
    let remaining = max_entries.saturating_sub(selected.len());
    if remaining > 0 {
        let mut far_a: Vec<&OverviewCandidate> = uncovered
            .iter()
            .copied()
            .filter(|c| c.level == ChronicleLevel::A)
            .filter(|c| match band_earliest_turn {
                Some(earliest) => c.turn_start < earliest,
                None => true,
            })
            .collect();
        // 最近的远 A 优先：按 turn_start 降序取 remaining，再升序输出
        far_a.sort_by_key(|c| std::cmp::Reverse(c.turn_start));
        far_a.truncate(remaining);
        selected.extend(far_a);
    }

    selected.sort_by_key(|c| c.turn_start);
    selected.into_iter().map(|c| c.code.clone()).collect()
}

// ─── Epoch 刷新 / 创建 / rollover（编译入口纯函数）────────────────────────

/// 将已提交轮次序号映射为稳定 turn_id（无 TurnRecord UUID 时的确定性 id）。
pub fn committed_turn_id(sequence: u32) -> Id {
    Id::from_str(format!("committed-turn-{sequence}"))
}

/// 从 1..=n 的已提交轮次构造 CommittedTurnRef 序列。
pub fn committed_turns_from_count(n: u32) -> Vec<CommittedTurnRef> {
    (1..=n)
        .map(|sequence| CommittedTurnRef {
            turn_id: committed_turn_id(sequence),
            sequence,
        })
        .collect()
}

/// 从 turn_id 解析序号（仅识别 `committed-turn-{n}`）。
pub fn sequence_from_committed_turn_id(turn_id: &Id) -> Option<u32> {
    let s = turn_id.as_str();
    s.strip_prefix("committed-turn-")?.parse().ok()
}

/// Epoch 刷新结果。
#[derive(Debug, Clone)]
pub struct EpochRefreshResult {
    pub snapshot: ContextEpochSnapshot,
    pub membership: EpochMembership,
    /// 是否因 live_suffix 满 E 而 rollover
    pub rolled_over: bool,
    /// 是否新建（原先无 snapshot）
    pub created: bool,
}

impl EpochRefreshResult {
    pub fn should_bump_chronicle_revision(&self) -> bool {
        self.rolled_over || self.created
    }
}

/// 根据现有 snapshot + 已提交序列，在「下一次 Context 编译前」刷新 epoch。
///
/// 规则（规格 §4.1）：
/// - 无 snapshot → 以当前最后一个已提交为 head 创建 epoch（live_suffix 空）
/// - 有 snapshot 且 membership.needs_rollover → rollover 后写新 snapshot
/// - 否则沿用 snapshot 的 overview/band/anchor（仅 membership 反映 live_suffix 增长）
pub fn refresh_context_epoch(
    existing: Option<&ContextEpochSnapshot>,
    committed: &[CommittedTurnRef],
    overview_candidates: &[OverviewCandidate],
    // band_turn_id → ChronicleCode（leaf A）
    band_code_for_turn: &dyn Fn(&Id) -> Option<ChronicleCode>,
    params: ContextWindowParams,
    chronicle_revision: u64,
) -> EpochRefreshResult {
    match existing {
        None => {
            let membership = rollover_epoch_head(committed, params);
            let snapshot = build_context_epoch_snapshot(
                &membership,
                overview_candidates,
                band_code_for_turn,
                params,
                chronicle_revision,
                None,
            );
            EpochRefreshResult {
                snapshot,
                membership,
                rolled_over: false,
                created: true,
            }
        }
        Some(prev) => {
            let membership =
                compute_epoch_membership(committed, prev.source_head_turn_id.as_ref(), params);
            if membership.needs_rollover_before_next_compile {
                let membership = rollover_epoch_head(committed, params);
                let snapshot = build_context_epoch_snapshot(
                    &membership,
                    overview_candidates,
                    band_code_for_turn,
                    params,
                    chronicle_revision,
                    Some(prev),
                );
                EpochRefreshResult {
                    snapshot,
                    membership,
                    rolled_over: true,
                    created: false,
                }
            } else {
                // 同 epoch：冻结 overview/band/anchor；membership 带 live_suffix
                EpochRefreshResult {
                    snapshot: prev.clone(),
                    membership,
                    rolled_over: false,
                    created: false,
                }
            }
        }
    }
}

/// 从 membership 构造可持久化 ContextEpochSnapshot。
pub fn build_context_epoch_snapshot(
    membership: &EpochMembership,
    overview_candidates: &[OverviewCandidate],
    band_code_for_turn: &dyn Fn(&Id) -> Option<ChronicleCode>,
    params: ContextWindowParams,
    chronicle_revision: u64,
    previous: Option<&ContextEpochSnapshot>,
) -> ContextEpochSnapshot {
    let band_codes: Vec<ChronicleCode> = membership
        .band_turn_ids
        .iter()
        .filter_map(band_code_for_turn)
        .collect();
    let band_earliest = membership
        .band_turn_ids
        .first()
        .and_then(sequence_from_committed_turn_id)
        .or_else(|| {
            membership
                .anchor_turn_ids
                .first()
                .and_then(sequence_from_committed_turn_id)
        });
    let overview_codes = select_overview_codes(
        overview_candidates,
        band_earliest,
        params.overview_max_entries,
    );

    let epoch_id = if let Some(prev) = previous {
        if prev.source_head_turn_id == membership.epoch_start_head
            && prev.raw_anchor_turn_ids == membership.anchor_turn_ids
        {
            prev.epoch_id.clone()
        } else {
            format!(
                "ctx-epoch-{}",
                membership
                    .epoch_start_head
                    .as_ref()
                    .map(|id| id.as_str())
                    .unwrap_or("empty")
            )
        }
    } else {
        format!(
            "ctx-epoch-{}",
            membership
                .epoch_start_head
                .as_ref()
                .map(|id| id.as_str())
                .unwrap_or("empty")
        )
    };

    let mut source_hasher = sha2::Sha256::new();
    use sha2::Digest;
    source_hasher.update(epoch_id.as_bytes());
    source_hasher.update(b"|");
    for c in &overview_codes {
        source_hasher.update(c.as_str().as_bytes());
        source_hasher.update(b",");
    }
    source_hasher.update(b"|");
    for c in &band_codes {
        source_hasher.update(c.as_str().as_bytes());
        source_hasher.update(b",");
    }
    source_hasher.update(b"|");
    for id in &membership.anchor_turn_ids {
        source_hasher.update(id.as_str().as_bytes());
        source_hasher.update(b",");
    }
    let source_hash = format!("{:x}", source_hasher.finalize());

    ContextEpochSnapshot {
        epoch_id,
        source_head_turn_id: membership.epoch_start_head.clone(),
        overview_codes,
        band_codes,
        raw_anchor_turn_ids: membership.anchor_turn_ids.clone(),
        compiler_version: CONTEXT_COMPILER_VERSION.to_string(),
        chronicle_revision,
        source_hash,
    }
}

// ─── 硬去重 ────────────────────────────────────────────────────────────────

/// 装配期硬去重判定（规格 §5.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnInjectMode {
    /// 只发正文
    NearRawBodyOnly,
    /// 只发短纪要
    BandSummaryOnly,
    /// 默认可进概览/召回（由 covered 等再过滤）
    FarEligible,
}

pub fn turn_inject_mode(turn_id: &Id, near_raw: &[Id], band: &[Id]) -> TurnInjectMode {
    if near_raw.iter().any(|id| id == turn_id) {
        TurnInjectMode::NearRawBodyOnly
    } else if band.iter().any(|id| id == turn_id) {
        TurnInjectMode::BandSummaryOnly
    } else {
        TurnInjectMode::FarEligible
    }
}

// ─── ChronicleCompressor 确定性分组 ────────────────────────────────────────

/// 一组待压缩的连续 leaf/stage。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressGroup {
    /// 组内 entry id（时间升序）
    pub member_ids: Vec<Id>,
    pub turn_start: u32,
    pub turn_end: u32,
}

/// 按 turn_start 排序后切成连续不重叠组，每组 `group_size` 个（末组可短）。
pub fn partition_compress_groups(
    sorted_ids: &[Id],
    turn_spans: &[(u32, u32)],
    group_size: usize,
) -> Result<Vec<CompressGroup>, CompressGroupError> {
    if group_size == 0 {
        return Err(CompressGroupError::InvalidGroupSize);
    }
    if sorted_ids.len() != turn_spans.len() {
        return Err(CompressGroupError::LenMismatch);
    }
    // 校验 turn_start 非降
    for w in turn_spans.windows(2) {
        if w[0].0 > w[1].0 {
            return Err(CompressGroupError::NotTimeOrdered);
        }
    }
    let mut groups = Vec::new();
    let mut i = 0;
    while i < sorted_ids.len() {
        let end = (i + group_size).min(sorted_ids.len());
        let members = sorted_ids[i..end].to_vec();
        let turn_start = turn_spans[i].0;
        let turn_end = turn_spans[end - 1].1;
        groups.push(CompressGroup {
            member_ids: members,
            turn_start,
            turn_end,
        });
        i = end;
    }
    Ok(groups)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompressGroupError {
    InvalidGroupSize,
    LenMismatch,
    NotTimeOrdered,
}

/// 系统填写的 covers 发布前校验（失败则整批不发布）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoversValidationError {
    MissingMember(Id),
    DuplicateMember(Id),
    UnexpectedMember(Id),
    EmptyGroup,
    NonContiguousTurns { group_index: usize },
    TurnSpanMismatch { group_index: usize },
}

/// 校验压缩 covers：
/// - 每个输入恰好出现在一个 covers 中
/// - covers 两两不相交（由「恰好一次」蕴含）
/// - 每组 covers 时间连续（按输入顺序的连续切片）
/// - turn_span 与子项一致
pub fn validate_compress_covers(
    input_ids_in_time_order: &[Id],
    input_turn_spans: &[(u32, u32)],
    groups: &[CompressGroup],
) -> Result<(), CoversValidationError> {
    if input_ids_in_time_order.len() != input_turn_spans.len() {
        return Err(CoversValidationError::TurnSpanMismatch { group_index: 0 });
    }
    let mut seen = std::collections::HashMap::<&Id, usize>::new();
    for (gi, g) in groups.iter().enumerate() {
        if g.member_ids.is_empty() {
            return Err(CoversValidationError::EmptyGroup);
        }
        for id in &g.member_ids {
            if let Some(_prev) = seen.insert(id, gi) {
                return Err(CoversValidationError::DuplicateMember(id.clone()));
            }
            if !input_ids_in_time_order.iter().any(|x| x == id) {
                return Err(CoversValidationError::UnexpectedMember(id.clone()));
            }
        }
        // 时间连续：members 必须是 input 中的连续切片
        let positions: Vec<usize> = g
            .member_ids
            .iter()
            .map(|id| {
                input_ids_in_time_order
                    .iter()
                    .position(|x| x == id)
                    .expect("checked above")
            })
            .collect();
        for w in positions.windows(2) {
            if w[1] != w[0] + 1 {
                return Err(CoversValidationError::NonContiguousTurns { group_index: gi });
            }
        }
        let first = *positions.first().unwrap();
        let last = *positions.last().unwrap();
        let expect_start = input_turn_spans[first].0;
        let expect_end = input_turn_spans[last].1;
        if g.turn_start != expect_start || g.turn_end != expect_end {
            return Err(CoversValidationError::TurnSpanMismatch { group_index: gi });
        }
    }
    for id in input_ids_in_time_order {
        if !seen.contains_key(id) {
            return Err(CoversValidationError::MissingMember(id.clone()));
        }
    }
    Ok(())
}

// ─── ChronicleCompressor 触发判定（M4 最小）──────────────────────────────────

/// 未覆盖 active 条目是否达到批压阈值。
pub fn should_enqueue_compress(uncovered_active_count: usize, threshold: usize) -> bool {
    threshold > 0 && uncovered_active_count >= threshold
}

/// 统计 uncovered leaf（covered_by is None）数量。
pub fn count_uncovered_active(covered_flags: impl IntoIterator<Item = bool>) -> usize {
    // true = covered (skip); false = uncovered
    covered_flags
        .into_iter()
        .filter(|covered| !*covered)
        .count()
}

/// 为未覆盖 A 规划压缩组（仅确定性切分；LLM 文案与发布在后台任务）。
pub fn plan_compress_batch_for_uncovered(
    uncovered_ids_in_time_order: &[Id],
    turn_spans: &[(u32, u32)],
    threshold: usize,
    group_size: usize,
) -> Result<Option<Vec<CompressGroup>>, CompressGroupError> {
    if !should_enqueue_compress(uncovered_ids_in_time_order.len(), threshold) {
        return Ok(None);
    }
    let groups = partition_compress_groups(uncovered_ids_in_time_order, turn_spans, group_size)?;
    Ok(Some(groups))
}

/// 一组 LLM 产出的压缩文案（系统填 covers/span）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressGroupText {
    pub headline: String,
    pub summary: String,
}

/// 压缩发布结果（纯函数；调用方落盘）。
#[derive(Debug, Clone)]
pub struct CompressPublishResult {
    /// 新 B 或 C 条目
    pub parents: Vec<ChronicleEntry>,
    /// 子条目 id → parent id（写 covered_by）
    pub child_covered_by: Vec<(Id, Id)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompressPublishError {
    GroupTextLenMismatch { groups: usize, texts: usize },
    EmptyInput,
    Validation(CoversValidationError),
    Group(CompressGroupError),
}

/// 系统侧发布压缩：校验 covers、生成 parent ChronicleEntry、计算 covered_by 映射。
///
/// `output_level`：B 或 C；`next_seq`：该 level 下一个序号（从 1 起）。
#[allow(clippy::too_many_arguments)]
pub fn publish_compress_batch(
    campaign_id: &Id,
    lineage_id: &Id,
    input_ids_in_time_order: &[Id],
    input_turn_spans: &[(u32, u32)],
    groups: &[CompressGroup],
    texts: &[CompressGroupText],
    output_level: ChronicleLevel,
    next_seq: u32,
) -> Result<CompressPublishResult, CompressPublishError> {
    if input_ids_in_time_order.is_empty() {
        return Err(CompressPublishError::EmptyInput);
    }
    if groups.len() != texts.len() {
        return Err(CompressPublishError::GroupTextLenMismatch {
            groups: groups.len(),
            texts: texts.len(),
        });
    }
    validate_compress_covers(input_ids_in_time_order, input_turn_spans, groups)
        .map_err(CompressPublishError::Validation)?;

    let mut parents = Vec::with_capacity(groups.len());
    let mut child_covered_by = Vec::new();
    let now = chrono::Utc::now().to_rfc3339();
    for (i, (g, text)) in groups.iter().zip(texts.iter()).enumerate() {
        let seq = next_seq.saturating_add(i as u32);
        let parent_id = Id::new();
        let code = ChronicleCode::new(output_level, seq);
        let headline = truncate_headline(text.headline.trim(), 40);
        let summary = text.summary.trim().to_string();
        for child in &g.member_ids {
            child_covered_by.push((child.clone(), parent_id.clone()));
        }
        parents.push(ChronicleEntry {
            id: parent_id,
            code,
            level: output_level,
            campaign_id: campaign_id.clone(),
            lineage_id: lineage_id.clone(),
            headline,
            summary,
            full: None,
            turn_start: g.turn_start,
            turn_end: g.turn_end,
            covers: g.member_ids.clone(),
            covered_by: None,
            source_turn_ids: vec![],
            source_variant_hashes: vec![],
            source_campaign_revision: None,
            origin_campaign_id: None,
            origin_chronicle_id: None,
            origin_code: None,
            invalidated_at: None,
            created_at: now.clone(),
        });
    }
    Ok(CompressPublishResult {
        parents,
        child_covered_by,
    })
}

/// 从已有 code 列表推算下一序号（同 level 前缀）。
pub fn next_code_seq(existing_codes: &[ChronicleCode], level: ChronicleLevel) -> u32 {
    let mut max = 0u32;
    for c in existing_codes {
        if c.level() == Some(level)
            && let Ok(n) = c.as_str()[1..].parse::<u32>()
        {
            max = max.max(n);
        }
    }
    max.saturating_add(1).max(1)
}

// ─── Compiler 纯函数 IO 草图 ───────────────────────────────────────────────

/// 编译输入（不持有 LLM / IO）。
#[derive(Debug, Clone)]
pub struct ContextCompileInput {
    pub snapshot: ContextEpochSnapshot,
    pub capture: ContextCompileCapture,
    /// live_suffix 对应正文消息（旧→新）；由调用方从 conversation 取出
    pub live_suffix_body: Vec<crate::llm::ChatMessage>,
    /// anchor 正文（snapshot.raw_anchor_turn_ids 对应）
    pub anchor_body: Vec<crate::llm::ChatMessage>,
    /// 可选：早于概览的确定性 checkpoint
    pub optional_checkpoint: Option<String>,
    /// overview 行：code + headline（与 snapshot.overview_codes 对齐）
    pub overview_lines: Vec<(ChronicleCode, String)>,
    /// band 行：code + short summary
    pub band_lines: Vec<(ChronicleCode, String)>,
}

/// 编译输出：history 段有序块 + 元数据（尚未拼 MessageLayout）。
#[derive(Debug, Clone)]
pub struct ContextCompileOutput {
    pub history_blocks: Vec<HistoryBlock>,
    pub epoch_id: String,
    pub chronicle_revision: u64,
    pub near_raw_count: usize,
    pub overview_codes: Vec<ChronicleCode>,
    pub band_codes: Vec<ChronicleCode>,
}

#[derive(Debug, Clone)]
pub enum HistoryBlock {
    Checkpoint(String),
    Overview {
        lines: Vec<String>,
    },
    Band {
        lines: Vec<String>,
    },
    NearRaw {
        messages: Vec<crate::llm::ChatMessage>,
    },
}

/// 纯函数：由 snapshot + 正文材料生成 history 块（规格 §5 物理序）。
pub fn compile_history_blocks(input: &ContextCompileInput) -> ContextCompileOutput {
    let mut blocks = Vec::new();
    if let Some(cp) = &input.optional_checkpoint
        && !cp.is_empty()
    {
        blocks.push(HistoryBlock::Checkpoint(cp.clone()));
    }
    if !input.overview_lines.is_empty() {
        let lines = input
            .overview_lines
            .iter()
            .map(|(code, headline)| format!("{} {}", code.as_str(), headline))
            .collect();
        blocks.push(HistoryBlock::Overview { lines });
    }
    if !input.band_lines.is_empty() {
        let lines = input
            .band_lines
            .iter()
            .map(|(code, summary)| format!("{} {}", code.as_str(), summary))
            .collect();
        blocks.push(HistoryBlock::Band { lines });
    }
    let mut near = input.anchor_body.clone();
    near.extend(input.live_suffix_body.iter().cloned());
    let near_raw_count = near.len();
    if !near.is_empty() {
        blocks.push(HistoryBlock::NearRaw { messages: near });
    }
    ContextCompileOutput {
        history_blocks: blocks,
        epoch_id: input.snapshot.epoch_id.clone(),
        chronicle_revision: input.capture.chronicle_revision,
        near_raw_count,
        overview_codes: input.snapshot.overview_codes.clone(),
        band_codes: input.snapshot.band_codes.clone(),
    }
}

// ─── headline 截断 ─────────────────────────────────────────────────────────

/// 按字符上限截断 headline（禁止假设 1 字=1 token）。
pub fn truncate_headline(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

// ─── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;

    fn tid(n: u32) -> Id {
        Id::from_str(format!("t{n}"))
    }

    fn committed_seq(n: u32) -> Vec<CommittedTurnRef> {
        (1..=n)
            .map(|i| CommittedTurnRef {
                turn_id: tid(i),
                sequence: i,
            })
            .collect()
    }

    #[test]
    fn max_near_raw_is_h_plus_e() {
        let p = ContextWindowParams::default();
        assert_eq!(p.h_anchor, 5);
        assert_eq!(p.e, 10);
        assert_eq!(p.max_near_raw_turns(), 15);
    }

    #[test]
    fn empty_committed_no_anchor() {
        let p = ContextWindowParams::default();
        let m = compute_epoch_membership(&[], None, p);
        assert!(m.anchor_turn_ids.is_empty());
        assert!(m.live_suffix_turn_ids.is_empty());
        assert!(m.near_raw_turn_ids.is_empty());
        assert!(!m.needs_rollover_before_next_compile);
    }

    #[test]
    fn anchor_takes_h_ending_at_head() {
        let committed = committed_seq(20);
        let p = ContextWindowParams {
            h_anchor: 5,
            e: 10,
            s: 10,
            overview_max_entries: 200,
        };
        // head = t10 → anchor = t6..t10
        let m = compute_epoch_membership(&committed, Some(&tid(10)), p);
        assert_eq!(
            m.anchor_turn_ids,
            vec![tid(6), tid(7), tid(8), tid(9), tid(10)]
        );
        // live = t11..t20 but capped to E=10 → t11..t20
        assert_eq!(m.live_suffix_count, 10);
        assert_eq!(m.live_suffix_turn_ids.first(), Some(&tid(11)));
        assert_eq!(m.live_suffix_turn_ids.last(), Some(&tid(20)));
        assert_eq!(m.near_raw_turn_ids.len(), 15); // 5+10
        assert!(m.needs_rollover_before_next_compile);
        // band = 紧邻 anchor 前 S=10 → t1..t5? first anchor t6, before = t1..t5 (only 5)
        assert_eq!(
            m.band_turn_ids,
            vec![tid(1), tid(2), tid(3), tid(4), tid(5)]
        );
    }

    #[test]
    fn live_suffix_e_still_in_epoch_then_rollover() {
        let committed = committed_seq(15);
        let p = ContextWindowParams {
            h_anchor: 5,
            e: 10,
            s: 10,
            overview_max_entries: 200,
        };
        // head t5, suffix t6..t15 = 10 == E
        let m = compute_epoch_membership(&committed, Some(&tid(5)), p);
        assert_eq!(m.live_suffix_count, 10);
        assert!(m.needs_rollover_before_next_compile);
        assert_eq!(m.near_raw_turn_ids.len(), 15);

        let after = rollover_epoch_head(&committed, p);
        assert_eq!(after.epoch_start_head, Some(tid(15)));
        assert_eq!(after.live_suffix_count, 0);
        assert!(!after.needs_rollover_before_next_compile);
        assert_eq!(
            after.anchor_turn_ids,
            vec![tid(11), tid(12), tid(13), tid(14), tid(15)]
        );
        // band: before t11, S=10 → t1..t10
        assert_eq!(after.band_turn_ids.len(), 10);
        assert_eq!(after.band_turn_ids.first(), Some(&tid(1)));
        assert_eq!(after.band_turn_ids.last(), Some(&tid(10)));
    }

    #[test]
    fn same_epoch_append_only_grows_suffix() {
        let p = ContextWindowParams {
            h_anchor: 3,
            e: 4,
            s: 4,
            overview_max_entries: 50,
        };
        let c5 = committed_seq(5);
        let m5 = compute_epoch_membership(&c5, Some(&tid(5)), p);
        assert_eq!(m5.live_suffix_count, 0);
        assert_eq!(m5.anchor_turn_ids, vec![tid(3), tid(4), tid(5)]);

        let c7 = committed_seq(7);
        let m7 = compute_epoch_membership(&c7, Some(&tid(5)), p);
        assert_eq!(m7.anchor_turn_ids, m5.anchor_turn_ids); // anchor frozen
        assert_eq!(m7.live_suffix_turn_ids, vec![tid(6), tid(7)]);
        assert_eq!(m7.near_raw_turn_ids.len(), 5); // 3+2
        assert!(!m7.needs_rollover_before_next_compile);
    }

    #[test]
    fn chronicle_code_roundtrip() {
        let c = ChronicleCode::new(ChronicleLevel::A, 123);
        assert_eq!(c.as_str(), "A0123");
        assert_eq!(ChronicleCode::parse("A0123").unwrap().as_str(), "A0123");
        assert_eq!(ChronicleCode::parse("b42").unwrap().as_str(), "B0042");
        assert!(ChronicleCode::parse("X001").is_none());
        assert!(ChronicleCode::parse("A").is_none());
    }

    #[test]
    fn lineage_applicability() {
        let camp = Id::from_str("c1");
        let lin = Id::from_str("l1");
        let other = Id::from_str("l2");
        let mut e = ChronicleEntry {
            id: Id::new(),
            code: ChronicleCode::new(ChronicleLevel::A, 1),
            level: ChronicleLevel::A,
            campaign_id: camp.clone(),
            lineage_id: lin.clone(),
            headline: "h".into(),
            summary: "s".into(),
            full: None,
            turn_start: 1,
            turn_end: 1,
            covers: vec![],
            covered_by: None,
            source_turn_ids: vec![],
            source_variant_hashes: vec![],
            source_campaign_revision: None,
            origin_campaign_id: None,
            origin_chronicle_id: None,
            origin_code: None,
            invalidated_at: None,
            created_at: "t".into(),
        };
        assert!(entry_applies_to_lineage(&e, &camp, &lin));
        assert!(!entry_applies_to_lineage(&e, &camp, &other));
        e.invalidated_at = Some("now".into());
        assert!(!entry_applies_to_lineage(&e, &camp, &lin));
    }

    #[test]
    fn revision_bump_reasons_all_true() {
        for r in [
            ChronicleRevisionBumpReason::AcceptChronicleA,
            ChronicleRevisionBumpReason::EpochRollover,
            ChronicleRevisionBumpReason::CompressPublish,
            ChronicleRevisionBumpReason::ForkOrImportOrMigration,
            ChronicleRevisionBumpReason::LineageInvalidation,
        ] {
            assert!(should_bump_chronicle_revision(r));
        }
    }

    #[test]
    fn compress_partition_and_covers_ok() {
        let ids: Vec<Id> = (1..=8).map(|i| Id::from_str(format!("a{i}"))).collect();
        let spans: Vec<(u32, u32)> = (1..=8).map(|i| (i, i)).collect();
        let groups = partition_compress_groups(&ids, &spans, 4).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].member_ids.len(), 4);
        assert_eq!(groups[0].turn_start, 1);
        assert_eq!(groups[0].turn_end, 4);
        assert_eq!(groups[1].turn_start, 5);
        assert_eq!(groups[1].turn_end, 8);
        validate_compress_covers(&ids, &spans, &groups).unwrap();
    }

    #[test]
    fn compress_covers_rejects_duplicate_and_gap() {
        let ids: Vec<Id> = (1..=4).map(|i| Id::from_str(format!("a{i}"))).collect();
        let spans: Vec<(u32, u32)> = (1..=4).map(|i| (i, i)).collect();
        let bad = vec![CompressGroup {
            member_ids: vec![ids[0].clone(), ids[2].clone()], // 非连续
            turn_start: 1,
            turn_end: 3,
        }];
        assert!(matches!(
            validate_compress_covers(&ids, &spans, &bad),
            Err(CoversValidationError::NonContiguousTurns { .. })
        ));

        let dup = vec![
            CompressGroup {
                member_ids: vec![ids[0].clone(), ids[1].clone()],
                turn_start: 1,
                turn_end: 2,
            },
            CompressGroup {
                member_ids: vec![ids[1].clone(), ids[2].clone(), ids[3].clone()],
                turn_start: 2,
                turn_end: 4,
            },
        ];
        assert!(matches!(
            validate_compress_covers(&ids, &spans, &dup),
            Err(CoversValidationError::DuplicateMember(_))
        ));
    }

    #[test]
    fn compress_covers_rejects_missing_member() {
        let ids: Vec<Id> = (1..=4).map(|i| Id::from_str(format!("a{i}"))).collect();
        let spans: Vec<(u32, u32)> = (1..=4).map(|i| (i, i)).collect();
        let partial = vec![CompressGroup {
            member_ids: vec![ids[0].clone(), ids[1].clone()],
            turn_start: 1,
            turn_end: 2,
        }];
        assert!(matches!(
            validate_compress_covers(&ids, &spans, &partial),
            Err(CoversValidationError::MissingMember(_))
        ));
    }

    #[test]
    fn overview_prefers_bc_then_far_a_time_order() {
        let cands = vec![
            OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, 1),
                level: ChronicleLevel::A,
                turn_start: 1,
                covered_by: None,
            },
            OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, 2),
                level: ChronicleLevel::A,
                turn_start: 5,
                covered_by: None,
            },
            OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::B, 1),
                level: ChronicleLevel::B,
                turn_start: 3,
                covered_by: None,
            },
            OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, 9),
                level: ChronicleLevel::A,
                turn_start: 50, // in/after band → 不进 overview far-A
                covered_by: None,
            },
            OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, 3),
                level: ChronicleLevel::A,
                turn_start: 2,
                covered_by: Some(Id::new()), // covered
            },
        ];
        // band earliest = 10 → only turn_start < 10 far A
        let codes = select_overview_codes(&cands, Some(10), 10);
        let s: Vec<&str> = codes.iter().map(|c| c.as_str()).collect();
        // time order: A0001(1), A0002(5), B0001(3) → sorted: A1, B1, A2
        assert_eq!(s, vec!["A0001", "B0001", "A0002"]);
        assert!(!s.contains(&"A0009"));
        assert!(!s.contains(&"A0003"));
    }

    #[test]
    fn turn_inject_hard_dedup() {
        let near = vec![tid(8), tid(9)];
        let band = vec![tid(5), tid(6), tid(7)];
        assert_eq!(
            turn_inject_mode(&tid(9), &near, &band),
            TurnInjectMode::NearRawBodyOnly
        );
        assert_eq!(
            turn_inject_mode(&tid(6), &near, &band),
            TurnInjectMode::BandSummaryOnly
        );
        assert_eq!(
            turn_inject_mode(&tid(1), &near, &band),
            TurnInjectMode::FarEligible
        );
    }

    #[test]
    fn compile_history_order_checkpoint_overview_band_near() {
        let snap = ContextEpochSnapshot {
            epoch_id: "e1".into(),
            source_head_turn_id: Some(tid(5)),
            overview_codes: vec![ChronicleCode::new(ChronicleLevel::B, 1)],
            band_codes: vec![ChronicleCode::new(ChronicleLevel::A, 3)],
            raw_anchor_turn_ids: vec![tid(4), tid(5)],
            compiler_version: CONTEXT_COMPILER_VERSION.into(),
            chronicle_revision: 7,
            source_hash: "h".into(),
        };
        let input = ContextCompileInput {
            snapshot: snap,
            capture: ContextCompileCapture {
                campaign_revision: 3,
                chronicle_revision: 7,
                epoch_id: "e1".into(),
            },
            live_suffix_body: vec![ChatMessage::user("new")],
            anchor_body: vec![ChatMessage::assistant("old")],
            optional_checkpoint: Some("【历史纪要】…".into()),
            overview_lines: vec![(ChronicleCode::new(ChronicleLevel::B, 1), "阶段".into())],
            band_lines: vec![(ChronicleCode::new(ChronicleLevel::A, 3), "短纪要".into())],
        };
        let out = compile_history_blocks(&input);
        assert_eq!(out.history_blocks.len(), 4);
        assert!(matches!(out.history_blocks[0], HistoryBlock::Checkpoint(_)));
        assert!(matches!(
            out.history_blocks[1],
            HistoryBlock::Overview { .. }
        ));
        assert!(matches!(out.history_blocks[2], HistoryBlock::Band { .. }));
        if let HistoryBlock::NearRaw { messages } = &out.history_blocks[3] {
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[0].content, "old");
            assert_eq!(messages[1].content, "new");
        } else {
            panic!("expected NearRaw");
        }
        assert_eq!(out.chronicle_revision, 7);
    }

    #[test]
    fn publish_compress_batch_sets_covers_and_covered_by() {
        let ids: Vec<Id> = (1..=4).map(|i| Id::from_str(format!("a{i}"))).collect();
        let spans: Vec<(u32, u32)> = (1..=4).map(|i| (i, i)).collect();
        let groups = partition_compress_groups(&ids, &spans, 2).unwrap();
        let texts = vec![
            CompressGroupText {
                headline: "阶段一".into(),
                summary: "前两轮合并".into(),
            },
            CompressGroupText {
                headline: "阶段二".into(),
                summary: "后两轮合并".into(),
            },
        ];
        let camp = Id::from_str("c");
        let lin = Id::from_str("l");
        let pubr = publish_compress_batch(
            &camp,
            &lin,
            &ids,
            &spans,
            &groups,
            &texts,
            ChronicleLevel::B,
            1,
        )
        .unwrap();
        assert_eq!(pubr.parents.len(), 2);
        assert_eq!(pubr.parents[0].code.as_str(), "B0001");
        assert_eq!(pubr.parents[0].covers, groups[0].member_ids);
        assert_eq!(pubr.parents[0].turn_start, 1);
        assert_eq!(pubr.parents[0].turn_end, 2);
        assert_eq!(pubr.child_covered_by.len(), 4);
        assert_eq!(pubr.child_covered_by[0].1, pubr.parents[0].id);
        assert_eq!(
            next_code_seq(
                &[ChronicleCode::new(ChronicleLevel::B, 3)],
                ChronicleLevel::B
            ),
            4
        );
    }

    #[test]
    fn plan_compress_batch_threshold() {
        let ids: Vec<Id> = (1..=8).map(|i| Id::from_str(format!("a{i}"))).collect();
        let spans: Vec<(u32, u32)> = (1..=8).map(|i| (i, i)).collect();
        assert!(
            plan_compress_batch_for_uncovered(&ids, &spans, 200, 4)
                .unwrap()
                .is_none()
        );
        let groups = plan_compress_batch_for_uncovered(&ids, &spans, 8, 4)
            .unwrap()
            .expect("should plan");
        assert_eq!(groups.len(), 2);
        assert!(should_enqueue_compress(200, 200));
        assert!(!should_enqueue_compress(199, 200));
        assert_eq!(count_uncovered_active([false, true, false]), 2);
    }

    #[test]
    fn truncate_headline_by_chars() {
        assert_eq!(truncate_headline("你好世界", 2), "你好");
        assert_eq!(truncate_headline("ab", 10), "ab");
    }

    #[test]
    fn refresh_creates_epoch_then_grows_suffix_then_rollover() {
        let params = ContextWindowParams {
            h_anchor: 2,
            e: 2,
            s: 2,
            overview_max_entries: 50,
        };
        // 3 committed turns
        let c3 = committed_turns_from_count(3);
        let cands: Vec<OverviewCandidate> = (1..=3)
            .map(|t| OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, t),
                level: ChronicleLevel::A,
                turn_start: t,
                covered_by: None,
            })
            .collect();
        let band_lookup = |id: &Id| {
            sequence_from_committed_turn_id(id).map(|t| ChronicleCode::new(ChronicleLevel::A, t))
        };
        let r1 = refresh_context_epoch(None, &c3, &cands, &band_lookup, params, 0);
        assert!(r1.created);
        assert!(!r1.rolled_over);
        assert_eq!(r1.membership.live_suffix_count, 0);
        assert_eq!(r1.snapshot.source_head_turn_id, Some(committed_turn_id(3)));
        assert_eq!(
            r1.membership.anchor_turn_ids,
            vec![committed_turn_id(2), committed_turn_id(3)]
        );

        // +1 turn → live_suffix=1, same epoch
        let c4 = committed_turns_from_count(4);
        let cands4: Vec<OverviewCandidate> = (1..=4)
            .map(|t| OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, t),
                level: ChronicleLevel::A,
                turn_start: t,
                covered_by: None,
            })
            .collect();
        let r2 = refresh_context_epoch(Some(&r1.snapshot), &c4, &cands4, &band_lookup, params, 0);
        assert!(!r2.created && !r2.rolled_over);
        assert_eq!(r2.membership.live_suffix_count, 1);
        assert_eq!(r2.snapshot.epoch_id, r1.snapshot.epoch_id);
        assert_eq!(r2.snapshot.overview_codes, r1.snapshot.overview_codes);
        assert_eq!(r2.snapshot.band_codes, r1.snapshot.band_codes);

        // +2 turns total suffix=2 == E → next compile rollover
        let c5 = committed_turns_from_count(5);
        let cands5: Vec<OverviewCandidate> = (1..=5)
            .map(|t| OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, t),
                level: ChronicleLevel::A,
                turn_start: t,
                covered_by: None,
            })
            .collect();
        let r3 = refresh_context_epoch(Some(&r2.snapshot), &c5, &cands5, &band_lookup, params, 1);
        // live_suffix would be 2 (>=E) → rolled over
        assert!(r3.rolled_over);
        assert_eq!(r3.snapshot.source_head_turn_id, Some(committed_turn_id(5)));
        assert_eq!(r3.membership.live_suffix_count, 0);
        assert_ne!(r3.snapshot.epoch_id, r1.snapshot.epoch_id);
        assert!(r3.should_bump_chronicle_revision());
    }

    #[test]
    fn same_epoch_two_refreshes_stable_codes() {
        let params = ContextWindowParams {
            h_anchor: 3,
            e: 5,
            s: 3,
            overview_max_entries: 20,
        };
        let c10 = committed_turns_from_count(10);
        let cands: Vec<OverviewCandidate> = (1..=10)
            .map(|t| OverviewCandidate {
                code: ChronicleCode::new(ChronicleLevel::A, t),
                level: ChronicleLevel::A,
                turn_start: t,
                covered_by: None,
            })
            .collect();
        let band_lookup = |id: &Id| {
            sequence_from_committed_turn_id(id).map(|t| ChronicleCode::new(ChronicleLevel::A, t))
        };
        let a = refresh_context_epoch(None, &c10, &cands, &band_lookup, params, 0);
        let b = refresh_context_epoch(Some(&a.snapshot), &c10, &cands, &band_lookup, params, 0);
        assert_eq!(a.snapshot.overview_codes, b.snapshot.overview_codes);
        assert_eq!(a.snapshot.band_codes, b.snapshot.band_codes);
        assert_eq!(a.snapshot.source_hash, b.snapshot.source_hash);
        assert_eq!(a.snapshot.epoch_id, b.snapshot.epoch_id);
    }
}

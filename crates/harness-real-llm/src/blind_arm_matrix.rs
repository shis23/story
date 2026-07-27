//! 四臂盲测骨架（流水线重设计落地顺序第 1 步，设计定稿见
//! ARCHITECTURE-REVIEW-2026-07-26 补记三 §11）。
//!
//! 四臂（同一 Campaign 状态与意图，盲评成文质量与对话交互感）：
//! 1. `SoloWriter`   — 单笔者 + 全量状态注入（续写档原型）
//! 2. `ParallelCrew` — 当前生产平行流水线（Director→Subagent→Editor）
//! 3. `SequentialCrew` — 顺序可见流水线（后者可见前者公开产出）
//! 4. `DuetMerge`    — 场记 + 按拍交替续演合并
//!
//! 本模块只提供确定性骨架：臂枚举、样本/裁决数据形态、双盲随机化配对、
//! 多数票判定、汇总统计。LLM 驱动（各臂生成 + 裁判调用）由 #[ignore]
//! 集成测试接入（evidence 落盘沿用 evidence.rs 脱敏纪律）。
//!
//! 预注册判读口径（BLIND-AB-PIPELINE-PLAN，防事后挑数）：
//! 对样本多数臂胜率 ≥75% → 结构显著有效；40-60% → 简化优先；
//! ≤35% → 结构负收益重审方向。同时记录墙钟与 token 成本比。

use serde::{Deserialize, Serialize};

/// 生成臂。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlindArm {
    /// 单笔者 + 全量状态注入（续写档原型）
    SoloWriter,
    /// 当前生产平行流水线
    ParallelCrew,
    /// 顺序可见流水线
    SequentialCrew,
    /// 对手戏：场记 + 按拍交替续演
    DuetMerge,
}

impl BlindArm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SoloWriter => "solo_writer",
            Self::ParallelCrew => "parallel_crew",
            Self::SequentialCrew => "sequential_crew",
            Self::DuetMerge => "duet_merge",
        }
    }

    /// 当前产品实现是否可用于真实评测。
    pub fn implemented(self) -> bool {
        true
    }

    pub fn all() -> [BlindArm; 4] {
        [
            Self::SoloWriter,
            Self::ParallelCrew,
            Self::SequentialCrew,
            Self::DuetMerge,
        ]
    }
}

/// 单臂单意图的生成样本（正文不入证据文件——脱敏纪律）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmSample {
    pub arm: BlindArm,
    pub intent_id: String,
    pub text_chars: usize,
    pub latency_ms: u128,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// 正文 sha256 前 16 hex（对账/去重用，不可逆）
    pub text_fingerprint16: String,
}

/// 裁判单次评审（一对臂样本，双盲乱序呈现）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeVerdict {
    pub intent_id: String,
    /// 呈现顺序里"文本甲"实际是哪个臂（随机化记录，复算用）
    pub first_arm: BlindArm,
    pub second_arm: BlindArm,
    /// 裁判宣布的胜者
    pub winner: BlindArm,
    /// 维度分 1-10：角色一致性/情节推进/文风/约束遵守
    pub scores_first: [u8; 4],
    pub scores_second: [u8; 4],
    pub judge_round: u8,
}

/// 双盲呈现顺序：由种子决定的确定性"抛硬币"（评审可复算，无系统偏置）。
///
/// 用 (seed, intent_id, round) 的 FNV-1a 折叠决定 A/B 先后。
pub fn presentation_order(
    seed: u64,
    intent_id: &str,
    round: u8,
    left: BlindArm,
    right: BlindArm,
) -> (BlindArm, BlindArm) {
    let mut hash: u64 = 0xcbf29ce484222325 ^ seed;
    for byte in intent_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= u64::from(round);
    hash = hash.wrapping_mul(0x100000001b3);
    if hash & 1 == 0 {
        (left, right)
    } else {
        (right, left)
    }
}

/// Four-arm Latin-square presentation. Across four rounds every arm appears
/// exactly once in every position, so a judge that merely prefers the first
/// text cannot create a systematic winner.
pub fn balanced_four_arm_order(seed: u64, intent_id: &str, round: u8) -> [BlindArm; 4] {
    let mut hash: u64 = 0xcbf29ce484222325 ^ seed;
    for byte in intent_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let mut arms = BlindArm::all();
    if hash & 1 != 0 {
        arms.swap(0, 1);
    }
    if hash & 2 != 0 {
        arms.swap(2, 3);
    }
    if hash & 4 != 0 {
        arms.reverse();
    }
    let shift = (round as usize) % arms.len();
    arms.rotate_left(shift);
    arms
}

/// 一对臂在一个意图上的多数票结果。
#[derive(Debug, Clone, Serialize)]
pub struct PairMajority {
    pub intent_id: String,
    pub left: BlindArm,
    pub right: BlindArm,
    pub left_votes: usize,
    pub right_votes: usize,
    /// 多数胜者；平票 None（计入 undecided）
    pub winner: Option<BlindArm>,
}

/// 多数票判定（每意图 N 轮裁决 → 胜者；平票如实报 None）。
pub fn majority_for_pair(
    intent_id: &str,
    left: BlindArm,
    right: BlindArm,
    verdicts: &[JudgeVerdict],
) -> PairMajority {
    let mut left_votes = 0usize;
    let mut right_votes = 0usize;
    for v in verdicts {
        if v.intent_id != intent_id {
            continue;
        }
        if v.winner == left {
            left_votes += 1;
        } else if v.winner == right {
            right_votes += 1;
        }
    }
    let winner = match left_votes.cmp(&right_votes) {
        std::cmp::Ordering::Greater => Some(left),
        std::cmp::Ordering::Less => Some(right),
        std::cmp::Ordering::Equal => None,
    };
    PairMajority {
        intent_id: intent_id.to_string(),
        left,
        right,
        left_votes,
        right_votes,
        winner,
    }
}

/// 矩阵汇总（含本轮未生成臂的显式清单）。
#[derive(Debug, Clone, Serialize)]
pub struct BlindMatrixSummary {
    pub intents: usize,
    pub judge_rounds_per_pair: u8,
    /// 各臂多数票胜场数
    pub wins: Vec<(String, usize)>,
    pub undecided: usize,
    /// 本轮没有生成样本的臂（no silent caps）
    pub absent_arms: Vec<String>,
    /// 各臂总耗时/补全 token（成本比判读用）
    pub cost: Vec<(String, u128, u64)>,
}

/// 汇总一组 pair 多数票 + 样本成本。
pub fn summarize_matrix(
    pairs: &[PairMajority],
    samples: &[ArmSample],
    judge_rounds_per_pair: u8,
) -> BlindMatrixSummary {
    let mut wins: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let mut undecided = 0usize;
    let mut intents: std::collections::BTreeSet<&str> = Default::default();
    for pair in pairs {
        intents.insert(&pair.intent_id);
        match pair.winner {
            Some(arm) => *wins.entry(arm.as_str()).or_default() += 1,
            None => undecided += 1,
        }
    }
    let mut cost: std::collections::BTreeMap<&'static str, (u128, u64)> = Default::default();
    let mut present: std::collections::BTreeSet<&'static str> = Default::default();
    for s in samples {
        intents.insert(&s.intent_id);
        present.insert(s.arm.as_str());
        let entry = cost.entry(s.arm.as_str()).or_default();
        entry.0 += s.latency_ms;
        entry.1 += s.completion_tokens;
    }
    let absent_arms = BlindArm::all()
        .into_iter()
        .filter(|arm| !present.contains(arm.as_str()))
        .map(|arm| arm.as_str().to_string())
        .collect();
    BlindMatrixSummary {
        intents: intents.len(),
        judge_rounds_per_pair,
        wins: wins.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        undecided,
        absent_arms,
        cost: cost
            .into_iter()
            .map(|(k, (ms, tok))| (k.to_string(), ms, tok))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(intent: &str, winner: BlindArm, round: u8) -> JudgeVerdict {
        JudgeVerdict {
            intent_id: intent.into(),
            first_arm: BlindArm::SoloWriter,
            second_arm: BlindArm::ParallelCrew,
            winner,
            scores_first: [7, 7, 7, 7],
            scores_second: [6, 6, 6, 6],
            judge_round: round,
        }
    }

    #[test]
    fn presentation_order_is_deterministic_and_seed_sensitive() {
        let a = presentation_order(
            1,
            "intent-1",
            0,
            BlindArm::SoloWriter,
            BlindArm::ParallelCrew,
        );
        let b = presentation_order(
            1,
            "intent-1",
            0,
            BlindArm::SoloWriter,
            BlindArm::ParallelCrew,
        );
        assert_eq!(a, b, "同种子同输入必须可复算");
        // 不同 (intent, round) 组合应产生两种顺序（随机化确实在起作用）
        let mut seen = std::collections::BTreeSet::new();
        for round in 0..8u8 {
            for intent in ["i1", "i2", "i3", "i4"] {
                let (first, _) = presentation_order(
                    7,
                    intent,
                    round,
                    BlindArm::SoloWriter,
                    BlindArm::ParallelCrew,
                );
                seen.insert(first.as_str());
            }
        }
        assert_eq!(seen.len(), 2, "呈现顺序应双向都出现: {seen:?}");
    }

    #[test]
    fn four_arm_order_is_position_balanced_across_four_rounds() {
        let mut positions = std::collections::BTreeMap::<&'static str, Vec<usize>>::new();
        for round in 0..4 {
            for (position, arm) in balanced_four_arm_order(7, "intent-1", round)
                .into_iter()
                .enumerate()
            {
                positions.entry(arm.as_str()).or_default().push(position);
            }
        }

        for arm in BlindArm::all() {
            let mut seen = positions.remove(arm.as_str()).unwrap_or_default();
            seen.sort_unstable();
            assert_eq!(seen, vec![0, 1, 2, 3], "{} position bias", arm.as_str());
        }
    }

    #[test]
    fn every_product_arm_is_now_implemented() {
        assert!(BlindArm::all().into_iter().all(BlindArm::implemented));
    }

    #[test]
    fn majority_vote_and_tie_are_reported_honestly() {
        let verdicts = vec![
            verdict("i1", BlindArm::ParallelCrew, 0),
            verdict("i1", BlindArm::SoloWriter, 1),
            verdict("i1", BlindArm::ParallelCrew, 2),
            // 其他意图的裁决不得串台
            verdict("i2", BlindArm::SoloWriter, 0),
        ];
        let pair = majority_for_pair(
            "i1",
            BlindArm::SoloWriter,
            BlindArm::ParallelCrew,
            &verdicts,
        );
        assert_eq!(pair.left_votes, 1);
        assert_eq!(pair.right_votes, 2);
        assert_eq!(pair.winner, Some(BlindArm::ParallelCrew));

        let tie = majority_for_pair(
            "i2",
            BlindArm::SoloWriter,
            BlindArm::ParallelCrew,
            &[
                verdict("i2", BlindArm::SoloWriter, 0),
                verdict("i2", BlindArm::ParallelCrew, 1),
            ],
        );
        assert_eq!(tie.winner, None, "平票必须如实报 None");
    }

    #[test]
    fn summary_lists_absent_arms_explicitly() {
        let samples = vec![
            ArmSample {
                arm: BlindArm::SoloWriter,
                intent_id: "i1".into(),
                text_chars: 800,
                latency_ms: 30_000,
                prompt_tokens: 4_000,
                completion_tokens: 900,
                text_fingerprint16: "aa".into(),
            },
            ArmSample {
                arm: BlindArm::ParallelCrew,
                intent_id: "i1".into(),
                text_chars: 900,
                latency_ms: 90_000,
                prompt_tokens: 12_000,
                completion_tokens: 2_400,
                text_fingerprint16: "bb".into(),
            },
        ];
        let pairs = vec![majority_for_pair(
            "i1",
            BlindArm::SoloWriter,
            BlindArm::ParallelCrew,
            &[verdict("i1", BlindArm::ParallelCrew, 0)],
        )];
        let summary = summarize_matrix(&pairs, &samples, 3);
        assert_eq!(summary.intents, 1);
        assert_eq!(
            summary.absent_arms,
            vec!["sequential_crew".to_string(), "duet_merge".to_string()],
            "本轮未生成的臂必须显式列缺席"
        );
        assert_eq!(summary.wins, vec![("parallel_crew".to_string(), 1)]);
    }

    #[test]
    fn generated_intents_are_counted_without_in_process_judging() {
        let samples = vec![ArmSample {
            arm: BlindArm::SequentialCrew,
            intent_id: "i1".into(),
            text_chars: 1_000,
            latency_ms: 1,
            prompt_tokens: 1,
            completion_tokens: 1,
            text_fingerprint16: "cc".into(),
        }];

        let summary = summarize_matrix(&[], &samples, 0);

        assert_eq!(summary.intents, 1);
        assert!(summary.wins.is_empty());
    }
}

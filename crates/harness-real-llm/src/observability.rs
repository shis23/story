//! Cache / context 可观测性：样本分类、前缀稳定性与脱敏报告。
//!
//! 不调用真实供应商；不宣称 provider cache hits。

use std::path::Path;

use serde::{Deserialize, Serialize};
use storyforge_domain::llm::ChatMessage;
use storyforge_domain::message_layout::{
    estimate_reusable_prefix_tokens, estimate_tokens_approx, fingerprint_messages,
    longest_common_message_prefix_len, messages_segment_summary,
};

use crate::budget::UsageSample;
use crate::evidence::{EVIDENCE_SCHEMA_VERSION, contains_forbidden_evidence_payload, short_hash16};

/// 样本分类：boot/setup 与 turn 必须分离，防止跨轮归因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleKind {
    Boot,
    Setup,
    Turn,
    Unknown,
}

/// 从 tag / role 判定样本种类（确定性规则，不依赖 usage 数值）。
pub fn classify_sample_kind(tag: &str, role: &str) -> SampleKind {
    let tag_l = tag.trim().to_ascii_lowercase();
    let role_l = role.trim().to_ascii_lowercase();

    if tag_l.is_empty() || tag_l == "boot" || tag_l.starts_with("boot") {
        return SampleKind::Boot;
    }
    if tag_l.starts_with("setup")
        || tag_l.contains("extract")
        || tag_l.contains("bootstrap")
        || role_l == "bootstrap"
        || role_l == "setup"
    {
        return SampleKind::Setup;
    }
    if tag_l.starts_with("turn")
        || (tag_l.starts_with('t') && tag_l.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        || role_l == "director"
        || role_l == "editor"
        || role_l == "subagent"
        || role_l == "summarizer"
        || role_l == "postprocess"
    {
        return SampleKind::Turn;
    }
    SampleKind::Unknown
}

/// 是否允许把该样本计入跨轮 cache / prompt 斜率（仅 turn）。
pub fn is_turn_attributed(kind: SampleKind) -> bool {
    matches!(kind, SampleKind::Turn)
}

/// 单次请求的可观测摘要（无 prompt 正文）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestObservability {
    pub role: String,
    pub streaming: bool,
    pub sample_kind: SampleKind,
    pub request_fp16: String,
    pub system_hash16: String,
    pub history_hash16: String,
    pub tail_hash16: String,
    pub history_len: usize,
    pub tail_parts: usize,
    pub msg_count: usize,
    pub estimated_prompt_tokens: u32,
    pub longest_common_prefix_msgs: usize,
    pub reusable_token_estimate: u32,
}

/// 相对上一轮 turn 样本构建可观测摘要。
pub fn observe_request(
    role: &str,
    streaming: bool,
    tag: &str,
    messages: &[ChatMessage],
    previous_turn_messages: Option<&[ChatMessage]>,
) -> RequestObservability {
    let segs = messages_segment_summary(messages);
    let fp = fingerprint_messages(messages);
    let kind = classify_sample_kind(tag, role);
    let (lcp, reusable) = match previous_turn_messages {
        Some(prev) if is_turn_attributed(kind) => (
            longest_common_message_prefix_len(prev, messages),
            estimate_reusable_prefix_tokens(prev, messages),
        ),
        _ => (0, 0),
    };
    let estimated_prompt_tokens = messages
        .iter()
        .map(|m| estimate_tokens_approx(&m.content))
        .sum();
    RequestObservability {
        role: role.into(),
        streaming,
        sample_kind: kind,
        request_fp16: short_hash16(&fp),
        system_hash16: segs.system_hash.chars().take(16).collect(),
        history_hash16: segs.history_hash.chars().take(16).collect(),
        tail_hash16: segs.tail_hash.chars().take(16).collect(),
        history_len: segs.history_len,
        tail_parts: segs.tail_parts,
        msg_count: messages.len(),
        estimated_prompt_tokens,
        longest_common_prefix_msgs: lcp,
        reusable_token_estimate: reusable,
    }
}

/// 模拟前缀缓存：同 epoch 稳定前缀命中，hook 变更 / epoch 滚动时失效。
#[derive(Debug, Clone, Default)]
pub struct SimulatedPrefixCache {
    epoch_id: String,
    prefix_fp: Option<String>,
    last_messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulatedCacheEvent {
    MissCold,
    HitStablePrefix,
    InvalidateHookChange,
    InvalidateEpochRollover,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SimulatedCacheObservation {
    pub event: SimulatedCacheEvent,
    pub epoch_id: String,
    pub common_prefix_msgs: usize,
    pub reusable_token_estimate: u32,
    pub system_hash16: String,
}

impl SimulatedPrefixCache {
    pub fn new(epoch_id: impl Into<String>) -> Self {
        Self {
            epoch_id: epoch_id.into(),
            prefix_fp: None,
            last_messages: Vec::new(),
        }
    }

    pub fn epoch_id(&self) -> &str {
        &self.epoch_id
    }

    /// 观察一次请求前缀。`stable_prefix_fp` 通常为 system+history 指纹。
    pub fn observe(
        &mut self,
        epoch_id: &str,
        stable_prefix_fp: &str,
        messages: &[ChatMessage],
    ) -> SimulatedCacheObservation {
        let segs = messages_segment_summary(messages);
        let system_hash16: String = segs.system_hash.chars().take(16).collect();
        let common = longest_common_message_prefix_len(&self.last_messages, messages);
        let reusable = estimate_reusable_prefix_tokens(&self.last_messages, messages);

        let event = if self.epoch_id != epoch_id {
            self.epoch_id = epoch_id.to_string();
            self.prefix_fp = Some(stable_prefix_fp.to_string());
            SimulatedCacheEvent::InvalidateEpochRollover
        } else if self.prefix_fp.is_none() {
            self.prefix_fp = Some(stable_prefix_fp.to_string());
            SimulatedCacheEvent::MissCold
        } else if self.prefix_fp.as_deref() == Some(stable_prefix_fp) {
            SimulatedCacheEvent::HitStablePrefix
        } else {
            self.prefix_fp = Some(stable_prefix_fp.to_string());
            SimulatedCacheEvent::InvalidateHookChange
        };

        self.last_messages = messages.to_vec();
        SimulatedCacheObservation {
            event,
            epoch_id: self.epoch_id.clone(),
            common_prefix_msgs: common,
            reusable_token_estimate: reusable,
            system_hash16,
        }
    }
}

/// 从 usage 样本中分离 boot/setup 与 turn，防止跨轮归因。
pub fn partition_samples(samples: &[UsageSample]) -> (Vec<&UsageSample>, Vec<&UsageSample>) {
    let mut non_turn = Vec::new();
    let mut turns = Vec::new();
    for s in samples {
        match classify_sample_kind(&s.tag, &s.role) {
            SampleKind::Turn => turns.push(s),
            _ => non_turn.push(s),
        }
    }
    (non_turn, turns)
}

/// 100+ 轮 Context 编译基准结果（确定性、无网络）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextCompileBenchmarkReport {
    pub schema_version: String,
    pub turns: u32,
    pub h_anchor: u32,
    pub e: u32,
    pub overview_max_entries: usize,
    pub max_near_raw: u32,
    pub observed_max_near_raw: u32,
    pub final_near_raw: usize,
    pub final_band: usize,
    pub final_overview: usize,
    pub token_estimates: Vec<u32>,
    pub prompt_growth_slope: f64,
    pub latency_ms_total: u128,
    pub latency_ms_p50: u128,
    pub latency_ms_p95: u128,
    pub evidence_bytes: usize,
    pub assertions: Vec<BenchmarkAssertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkAssertion {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// 预算门禁：斜率、near 窗口、证据脱敏。
#[derive(Debug, Clone)]
pub struct BudgetAssertions {
    pub max_prompt_growth_slope: f64,
    pub max_near_window: u32,
    pub max_evidence_bytes: usize,
}

impl Default for BudgetAssertions {
    fn default() -> Self {
        Self {
            // 每轮估计 token 增量上界（粗粒度 /4 估算，允许近窗填充）
            max_prompt_growth_slope: 120.0,
            max_near_window: storyforge_domain::chronicle::DEFAULT_H_ANCHOR
                + storyforge_domain::chronicle::DEFAULT_E,
            max_evidence_bytes: 256 * 1024,
        }
    }
}

/// 写机器可读报告；拒绝密钥/全文。
pub fn write_observability_report(
    path: &Path,
    report: &ContextCompileBenchmarkReport,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(report)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if contains_forbidden_evidence_payload(&body) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "refusing to write observability report containing secrets or full prompts",
        ));
    }
    std::fs::write(path, body)
}

/// 线性回归斜率（x=0..n-1, y=token_estimates）。
pub fn prompt_growth_slope(token_estimates: &[u32]) -> f64 {
    let n = token_estimates.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_xy = 0.0;
    for (i, y) in token_estimates.iter().enumerate() {
        let x = i as f64;
        let y = f64::from(*y);
        sum_x += x;
        sum_y += y;
        sum_xx += x * x;
        sum_xy += x * y;
    }
    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < f64::EPSILON {
        return 0.0;
    }
    (n * sum_xy - sum_x * sum_y) / denom
}

pub fn percentile_ms(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn schema_version() -> &'static str {
    EVIDENCE_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::ChatMessage;

    #[test]
    fn classifies_boot_setup_and_turn_without_cross_attribution() {
        assert_eq!(classify_sample_kind("boot", "pipeline"), SampleKind::Boot);
        assert_eq!(
            classify_sample_kind("setup-extract", "bootstrap"),
            SampleKind::Setup
        );
        assert_eq!(classify_sample_kind("turn3", "editor"), SampleKind::Turn);
        assert_eq!(
            classify_sample_kind("t12-write", "director"),
            SampleKind::Turn
        );

        let samples = vec![
            UsageSample {
                call_index: 1,
                evidence_turn_index: 0,
                tag: "boot".into(),
                role: "pipeline".into(),
                streaming: false,
                prompt_tokens: 999,
                cached_tokens: 900,
                cache_creation_tokens: 0,
                completion_tokens: 1,
                request_fp16: "a".into(),
                system_hash16: "b".into(),
                history_hash16: "c".into(),
                tail_hash16: "d".into(),
                history_len: 0,
                tail_parts: 0,
                msg_count: 1,
                elapsed_ms: 1,
                outcome: "ok".into(),
                tools_offered: vec![],
                tool_steps: vec![],
            },
            UsageSample {
                call_index: 2,
                evidence_turn_index: 0,
                tag: "turn1".into(),
                role: "editor".into(),
                streaming: true,
                prompt_tokens: 100,
                cached_tokens: 10,
                cache_creation_tokens: 0,
                completion_tokens: 20,
                request_fp16: "e".into(),
                system_hash16: "f".into(),
                history_hash16: "g".into(),
                tail_hash16: "h".into(),
                history_len: 2,
                tail_parts: 1,
                msg_count: 4,
                elapsed_ms: 2,
                outcome: "ok".into(),
                tools_offered: vec![],
                tool_steps: vec![],
            },
        ];
        let (non_turn, turns) = partition_samples(&samples);
        assert_eq!(non_turn.len(), 1);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].tag, "turn1");
        // boot 的高 cached 不得混入 turn 归因集合
        assert!(non_turn[0].cached_tokens > turns[0].cached_tokens);
    }

    #[test]
    fn observe_request_records_hashes_without_prompt_text() {
        let prev = vec![
            ChatMessage::system("stable-system"),
            ChatMessage::user("h1"),
            ChatMessage::assistant("a1"),
        ];
        let curr = vec![
            ChatMessage::system("stable-system"),
            ChatMessage::user("h1"),
            ChatMessage::assistant("a1"),
            ChatMessage::user("new-tail"),
        ];
        let obs = observe_request("editor", true, "turn2", &curr, Some(&prev));
        assert_eq!(obs.sample_kind, SampleKind::Turn);
        assert_eq!(obs.longest_common_prefix_msgs, 3);
        assert!(obs.reusable_token_estimate > 0);
        assert_eq!(obs.system_hash16.len(), 16);
        let json = serde_json::to_string(&obs).unwrap();
        assert!(!json.contains("stable-system"));
        assert!(!json.contains("new-tail"));
        assert!(!contains_forbidden_evidence_payload(&json));
    }

    #[test]
    fn simulated_cache_hits_within_epoch_and_invalidates_on_hook_and_rollover() {
        let mut cache = SimulatedPrefixCache::new("epoch-1");
        let sys = "You are director";
        let round1 = vec![
            ChatMessage::system(sys),
            ChatMessage::user("intent-1"),
            ChatMessage::user("tail-1"),
        ];
        let prefix1 = "fp-system-history-v1";
        let o1 = cache.observe("epoch-1", prefix1, &round1);
        assert_eq!(o1.event, SimulatedCacheEvent::MissCold);

        let round2 = vec![
            ChatMessage::system(sys),
            ChatMessage::user("intent-1"),
            ChatMessage::assistant("draft-1"),
            ChatMessage::user("tail-2"),
        ];
        // 同 epoch 且 stable prefix 指纹不变（history append 在真实系统会改 fp；
        // 此处用固定 fp 模拟“仅 tail 变、system 稳定”的段级稳定前缀）
        let o2 = cache.observe("epoch-1", prefix1, &round2);
        assert_eq!(o2.event, SimulatedCacheEvent::HitStablePrefix);
        assert!(o2.common_prefix_msgs >= 1);

        // hook 改写 system → 前缀失效
        let round3 = vec![
            ChatMessage::system("You are director\nHOOK"),
            ChatMessage::user("intent-1"),
            ChatMessage::assistant("draft-1"),
            ChatMessage::user("tail-3"),
        ];
        let o3 = cache.observe("epoch-1", "fp-system-history-v2-hook", &round3);
        assert_eq!(o3.event, SimulatedCacheEvent::InvalidateHookChange);

        // epoch 滚动
        let o4 = cache.observe("epoch-2", "fp-system-history-v3", &round3);
        assert_eq!(o4.event, SimulatedCacheEvent::InvalidateEpochRollover);
    }

    #[test]
    fn report_writer_rejects_secret_payload() {
        let mut report = ContextCompileBenchmarkReport {
            schema_version: schema_version().into(),
            turns: 1,
            h_anchor: 5,
            e: 10,
            overview_max_entries: 200,
            max_near_raw: 15,
            observed_max_near_raw: 1,
            final_near_raw: 1,
            final_band: 0,
            final_overview: 0,
            token_estimates: vec![10],
            prompt_growth_slope: 0.0,
            latency_ms_total: 1,
            latency_ms_p50: 1,
            latency_ms_p95: 1,
            evidence_bytes: 10,
            assertions: vec![BenchmarkAssertion {
                name: "x".into(),
                passed: true,
                detail: "api_key=sk-secret".into(),
            }],
        };
        // detail 含 api_key 标记应被拒绝
        let path = std::env::temp_dir().join(format!("sf_obs_{}.json", uuid::Uuid::new_v4()));
        let err = write_observability_report(&path, &report).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        report.assertions[0].detail = "ok".into();
        write_observability_report(&path, &report).unwrap();
        let _ = std::fs::remove_file(path);
    }
}

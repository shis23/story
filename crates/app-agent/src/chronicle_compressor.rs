#![allow(clippy::too_many_arguments, clippy::type_complexity)]
//! ChronicleCompressor：A→B / B→C 批压（记忆规格 §7.3）
//!
//! - 系统确定性分组 + covers 校验
//! - LLM 仅写 headline/summary
//! - 纯函数 `publish_compress_batch` 生成 parent + covered_by 映射
//! - 落盘由调用方（CampaignStore）完成

use tokio::sync::watch;
use tracing::{info, warn};

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::chronicle::{
    ChronicleCode, ChronicleLevel, CompressGroup, CompressGroupText, CompressPublishResult,
    DEFAULT_COMPRESS_ACTIVE_A_THRESHOLD, DEFAULT_COMPRESS_ACTIVE_B_THRESHOLD,
    DEFAULT_COMPRESS_GROUP_SIZE, next_code_seq, plan_compress_batch_for_uncovered,
    publish_compress_batch,
};
use storyforge_domain::llm::ChatResponse;

use crate::AgentError;
use crate::prompts::chronicle_compressor::{
    build_chronicle_compressor_user_msg, make_chronicle_compressor_config,
};
use crate::runtime::AgentRuntime;
use crate::tools::ToolRegistry;

#[derive(Debug, thiserror::Error)]
pub enum ChronicleCompressorError {
    #[error("LLM 调用失败: {0}")]
    Agent(#[from] AgentError),
    #[error("解析压缩 JSON 失败: {0}")]
    Parse(String),
    #[error("发布校验失败: {0:?}")]
    Publish(String),
    #[error("无待压缩条目")]
    NothingToCompress,
}

/// 一次压缩运行结果（尚未落盘或已由调用方落盘）。
#[derive(Debug, Clone)]
pub struct CompressRunOutcome {
    pub output_level: ChronicleLevel,
    pub groups: Vec<CompressGroup>,
    pub publish: CompressPublishResult,
    /// 可直接 insert 的 parent RoundSummary
    pub parent_summaries: Vec<RoundSummary>,
}

/// 从 uncovered 条目中按 level 过滤并规划组。
pub fn plan_level_batch(
    entries: &[RoundSummary],
    input_level: ChronicleLevel,
    threshold: usize,
    group_size: usize,
) -> Result<Option<(Vec<Id>, Vec<(u32, u32)>, Vec<CompressGroup>)>, ChronicleCompressorError> {
    let mut filtered: Vec<&RoundSummary> = entries
        .iter()
        .filter(|s| s.covered_by.is_none() && s.chronicle_level() == input_level)
        .collect();
    filtered.sort_by_key(|s| s.turn);
    let ids: Vec<Id> = filtered.iter().map(|s| s.id.clone()).collect();
    let spans: Vec<(u32, u32)> = filtered
        .iter()
        .map(|s| (s.turn, s.effective_turn_end()))
        .collect();
    match plan_compress_batch_for_uncovered(&ids, &spans, threshold, group_size) {
        Ok(None) => Ok(None),
        Ok(Some(groups)) => Ok(Some((ids, spans, groups))),
        Err(e) => Err(ChronicleCompressorError::Publish(format!("{e:?}"))),
    }
}

/// 解析 LLM JSON 数组 → CompressGroupText 列表。
pub fn parse_compress_group_texts(
    raw: &str,
    expected_len: usize,
) -> Result<Vec<CompressGroupText>, ChronicleCompressorError> {
    let trimmed = raw.trim();
    let json_str = extract_json_array(trimmed)
        .ok_or_else(|| ChronicleCompressorError::Parse("未找到 JSON 数组".into()))?;
    let value: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| ChronicleCompressorError::Parse(format!("JSON 解析: {e}")))?;
    let arr = value
        .as_array()
        .ok_or_else(|| ChronicleCompressorError::Parse("根不是数组".into()))?;
    if arr.len() != expected_len {
        return Err(ChronicleCompressorError::Parse(format!(
            "组数不匹配: got {} want {expected_len}",
            arr.len()
        )));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let headline = item
            .get("headline")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let summary = item
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if headline.is_empty() && summary.is_empty() {
            return Err(ChronicleCompressorError::Parse(format!(
                "第 {} 组 headline/summary 皆空",
                i + 1
            )));
        }
        let headline_final = if headline.is_empty() {
            summary.chars().take(40).collect::<String>()
        } else {
            headline
        };
        let summary_final = if summary.is_empty() {
            headline_final.clone()
        } else {
            summary
        };
        out.push(CompressGroupText {
            headline: headline_final,
            summary: summary_final,
        });
    }
    Ok(out)
}

fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end >= start {
        Some(&s[start..=end])
    } else {
        None
    }
}

/// 无 LLM：用确定性拼接文案发布（测试 / fail-open 降级）。
pub fn publish_with_deterministic_texts(
    campaign_id: &Id,
    lineage_id: &Id,
    conversation_id: &Id,
    entries: &[RoundSummary],
    ids: &[Id],
    spans: &[(u32, u32)],
    groups: &[CompressGroup],
    output_level: ChronicleLevel,
) -> Result<CompressRunOutcome, ChronicleCompressorError> {
    let by_id: std::collections::HashMap<&Id, &RoundSummary> =
        entries.iter().map(|s| (&s.id, s)).collect();
    let texts: Vec<CompressGroupText> = groups
        .iter()
        .map(|g| {
            let mut headlines = Vec::new();
            let mut bodies = Vec::new();
            for id in &g.member_ids {
                if let Some(s) = by_id.get(id) {
                    headlines.push(s.overview_headline(40));
                    bodies.push(s.content.clone());
                }
            }
            CompressGroupText {
                headline: headlines.join(" / "),
                summary: bodies.join(" "),
            }
        })
        .collect();
    finish_publish(
        campaign_id,
        lineage_id,
        conversation_id,
        entries,
        ids,
        spans,
        groups,
        &texts,
        output_level,
    )
}

fn finish_publish(
    campaign_id: &Id,
    lineage_id: &Id,
    conversation_id: &Id,
    entries: &[RoundSummary],
    ids: &[Id],
    spans: &[(u32, u32)],
    groups: &[CompressGroup],
    texts: &[CompressGroupText],
    output_level: ChronicleLevel,
) -> Result<CompressRunOutcome, ChronicleCompressorError> {
    let existing_codes: Vec<ChronicleCode> = entries
        .iter()
        .filter_map(|s| s.code.as_deref().and_then(ChronicleCode::parse))
        .collect();
    let next_seq = next_code_seq(&existing_codes, output_level);
    let publish = publish_compress_batch(
        campaign_id,
        lineage_id,
        ids,
        spans,
        groups,
        texts,
        output_level,
        next_seq,
    )
    .map_err(|e| ChronicleCompressorError::Publish(format!("{e:?}")))?;

    let parent_summaries: Vec<RoundSummary> = publish
        .parents
        .iter()
        .map(|e| RoundSummary::from_chronicle_entry(e, conversation_id.clone()))
        .collect();

    Ok(CompressRunOutcome {
        output_level,
        groups: groups.to_vec(),
        publish,
        parent_summaries,
    })
}

/// 跑 LLM 压缩一组已规划的 groups。
pub async fn compress_groups_with_llm(
    runtime: &AgentRuntime,
    campaign_id: &Id,
    lineage_id: &Id,
    conversation_id: &Id,
    entries: &[RoundSummary],
    ids: &[Id],
    spans: &[(u32, u32)],
    groups: &[CompressGroup],
    output_level: ChronicleLevel,
    cancel: watch::Receiver<bool>,
    agent_profile_config: Option<&AgentProfileConfig>,
) -> Result<CompressRunOutcome, ChronicleCompressorError> {
    let by_id: std::collections::HashMap<&Id, &RoundSummary> =
        entries.iter().map(|s| (&s.id, s)).collect();
    let members_per_group: Vec<Vec<(String, String, String)>> = groups
        .iter()
        .map(|g| {
            g.member_ids
                .iter()
                .filter_map(|id| {
                    let s = by_id.get(id)?;
                    Some((
                        s.code.clone().unwrap_or_default(),
                        s.overview_headline(40),
                        s.content.clone(),
                    ))
                })
                .collect()
        })
        .collect();

    let config = make_chronicle_compressor_config(agent_profile_config);
    let user_msg = build_chronicle_compressor_user_msg(output_level, groups, &members_per_group);
    let registry = ToolRegistry::new();
    info!(
        target: "chronicle_compressor",
        level = ?output_level,
        groups = groups.len(),
        "开始 LLM 压缩"
    );
    let resp: ChatResponse = runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
        .await?;
    let texts = match parse_compress_group_texts(&resp.content, groups.len()) {
        Ok(t) => t,
        Err(e) => {
            warn!(
                target: "chronicle_compressor",
                "LLM JSON 解析失败，降级确定性文案: {e}"
            );
            return publish_with_deterministic_texts(
                campaign_id,
                lineage_id,
                conversation_id,
                entries,
                ids,
                spans,
                groups,
                output_level,
            );
        }
    };
    finish_publish(
        campaign_id,
        lineage_id,
        conversation_id,
        entries,
        ids,
        spans,
        groups,
        &texts,
        output_level,
    )
}

/// 若 A 达阈值则压 B；随后若 B 达阈值则压 C。返回本轮产生的 outcomes。
pub async fn run_compress_if_needed(
    runtime: &AgentRuntime,
    campaign_id: &Id,
    lineage_id: &Id,
    conversation_id: &Id,
    mut entries: Vec<RoundSummary>,
    cancel: watch::Receiver<bool>,
    agent_profile_config: Option<&AgentProfileConfig>,
    // 测试可注入更低阈值
    a_threshold: Option<usize>,
    b_threshold: Option<usize>,
) -> Result<Vec<CompressRunOutcome>, ChronicleCompressorError> {
    let a_th = a_threshold.unwrap_or(DEFAULT_COMPRESS_ACTIVE_A_THRESHOLD);
    let b_th = b_threshold.unwrap_or(DEFAULT_COMPRESS_ACTIVE_B_THRESHOLD);
    let mut outcomes = Vec::new();

    if let Some((ids, spans, groups)) = plan_level_batch(
        &entries,
        ChronicleLevel::A,
        a_th,
        DEFAULT_COMPRESS_GROUP_SIZE,
    )? {
        let out = compress_groups_with_llm(
            runtime,
            campaign_id,
            lineage_id,
            conversation_id,
            &entries,
            &ids,
            &spans,
            &groups,
            ChronicleLevel::B,
            cancel.clone(),
            agent_profile_config,
        )
        .await?;
        // 内存侧应用 covered_by + parents，供可能的 B→C
        apply_outcome_in_memory(&mut entries, &out);
        outcomes.push(out);
    }

    if let Some((ids, spans, groups)) = plan_level_batch(
        &entries,
        ChronicleLevel::B,
        b_th,
        DEFAULT_COMPRESS_GROUP_SIZE,
    )? {
        let out = compress_groups_with_llm(
            runtime,
            campaign_id,
            lineage_id,
            conversation_id,
            &entries,
            &ids,
            &spans,
            &groups,
            ChronicleLevel::C,
            cancel,
            agent_profile_config,
        )
        .await?;
        outcomes.push(out);
    }

    if outcomes.is_empty() {
        return Err(ChronicleCompressorError::NothingToCompress);
    }
    Ok(outcomes)
}

fn apply_outcome_in_memory(entries: &mut Vec<RoundSummary>, out: &CompressRunOutcome) {
    for (child, parent) in &out.publish.child_covered_by {
        if let Some(s) = entries.iter_mut().find(|s| s.id == *child) {
            s.covered_by = Some(parent.clone());
        }
    }
    entries.extend(out.parent_summaries.iter().cloned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::agent::RoundSummary;
    use storyforge_infra_llm::LlmClient;
    use storyforge_infra_llm::mock_client::MockLlmClient;

    use crate::AgentRuntime;
    use crate::tools::ToolContext;

    fn leaf(turn: u32) -> RoundSummary {
        RoundSummary::new(
            Id::from_str("c"),
            Id::from_str("v"),
            turn,
            format!("事件{turn}的摘要正文"),
        )
        .with_code(format!("A{turn:04}"))
        .with_headline(format!("头{turn}"))
        .with_lineage(Id::from_str("lin"))
    }

    #[test]
    fn parse_compress_json_array() {
        let raw = r#"[{"headline":"h1","summary":"s1"},{"headline":"h2","summary":"s2"}]"#;
        let t = parse_compress_group_texts(raw, 2).unwrap();
        assert_eq!(t[0].headline, "h1");
        assert_eq!(t[1].summary, "s2");
    }

    #[test]
    fn deterministic_publish_a_to_b() {
        let entries: Vec<_> = (1..=4).map(leaf).collect();
        let (ids, spans, groups) = plan_level_batch(&entries, ChronicleLevel::A, 4, 2)
            .unwrap()
            .expect("plan");
        let out = publish_with_deterministic_texts(
            &Id::from_str("c"),
            &Id::from_str("lin"),
            &Id::from_str("v"),
            &entries,
            &ids,
            &spans,
            &groups,
            ChronicleLevel::B,
        )
        .unwrap();
        assert_eq!(out.parent_summaries.len(), 2);
        assert_eq!(out.parent_summaries[0].level, 1);
        assert!(
            out.parent_summaries[0]
                .code
                .as_deref()
                .unwrap()
                .starts_with('B')
        );
        assert_eq!(out.publish.child_covered_by.len(), 4);
    }

    #[tokio::test]
    async fn llm_compress_with_mock() {
        let client =
            std::sync::Arc::new(MockLlmClient::with_defaults()) as std::sync::Arc<dyn LlmClient>;
        let ctx = std::sync::Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let runtime = AgentRuntime::new(client, ctx);
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let entries: Vec<_> = (1..=4).map(leaf).collect();
        // mock 可能吐非 JSON → 走确定性降级，仍应成功
        let outs = run_compress_if_needed(
            &runtime,
            &Id::from_str("c"),
            &Id::from_str("lin"),
            &Id::from_str("v"),
            entries,
            rx,
            None,
            Some(4),
            Some(999), // 不触发 B→C
        )
        .await
        .unwrap();
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].output_level, ChronicleLevel::B);
    }
}

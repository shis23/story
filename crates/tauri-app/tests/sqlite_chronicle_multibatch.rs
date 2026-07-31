//! SQLite Chronicle multi-batch publication (Gate 4 review regression, P1-1).
//!
//! A single compress job may produce two outcome batches (A→B then B→C).
//! Uniqueness must be per (job_id, batch_index): publishing batch 0 must NOT
//! make batch 1 look like a duplicate/late result, and the job must be able
//! to reach Succeeded.
//!
//! `sqlite_runtime::activate` is process-global, so this binary contains one
//! test and must run in its own process (same rule as
//! `sqlite_chronicle_compressor.rs`).

use storyforge_app_agent::chronicle_compressor::{
    plan_level_batch, publish_with_deterministic_texts,
};
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::chronicle::ChronicleLevel;
use storyforge_domain::conversation::Conversation;
use storyforge_infra_sqlite::publication::PublishOutcome;
use storyforge_lib::sqlite_compress_jobs::CompressJobStatus;
use storyforge_lib::sqlite_runtime;

fn leaf(campaign_id: &Id, conversation_id: &Id, lineage_id: &Id, turn: u32) -> RoundSummary {
    RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        turn,
        format!("事件{turn}的摘要正文"),
    )
    .with_code(format!("A{turn:04}"))
    .with_headline(format!("头{turn}"))
    .with_lineage(lineage_id.clone())
}

#[test]
fn sqlite_compress_job_publishes_two_batches_without_dedup_confusion() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    let card_id = Id::from_str("mb-card-1");
    let campaign_id = Id::from_str("mb-camp-1");
    let conversation_id = Id::from_str("mb-conv-1");
    let lineage_id = Id::from_str("mb-lin-1");

    let mut campaign = Campaign::new(card_id.clone(), "MultiBatch Camp");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    campaign.lineage_id = Some(lineage_id.clone());
    sqlite_runtime::save_campaign(&campaign).expect("save campaign");
    sqlite_runtime::save_card_payload(
        &card_id,
        "MultiBatch Card",
        Some("mb-source-1"),
        Some("2026-07-16T00:00:00Z"),
        &serde_json::json!({"card": {"id": card_id.as_str(), "name": "MultiBatch Card"}}),
    )
    .expect("save card");
    let mut conversation = Conversation::new(None, Some(campaign_id.clone()));
    conversation.id = conversation_id.clone();
    sqlite_runtime::save_conversation(&conversation).expect("save conversation");

    let leaves: Vec<RoundSummary> = (1..=5)
        .map(|turn| leaf(&campaign_id, &conversation_id, &lineage_id, turn))
        .collect();
    for summary in &leaves {
        sqlite_runtime::seed_summary(summary).expect("seed summary");
    }

    let (uncovered_a, _) = sqlite_runtime::compress_count_uncovered(&campaign_id).expect("count");
    let (job, created) = sqlite_runtime::compress_enqueue_or_get_open(
        &campaign_id,
        Some(conversation_id.clone()),
        Some(lineage_id.clone()),
        uncovered_a as u32,
        0,
    )
    .expect("enqueue");
    assert!(created);
    assert!(sqlite_runtime::compress_try_claim_pending(&job.id).expect("claim"));

    // ── Batch 0: A→B ─────────────────────────────────────────────────────
    let entries_a = sqlite_runtime::list_summaries(&campaign_id).expect("list A entries");
    let (ids, spans, groups) = plan_level_batch(&entries_a, ChronicleLevel::A, 4, 2)
        .expect("plan A")
        .expect("A groups");
    let out_b = publish_with_deterministic_texts(
        &campaign_id,
        &lineage_id,
        &conversation_id,
        &entries_a,
        &ids,
        &spans,
        &groups,
        ChronicleLevel::B,
    )
    .expect("deterministic A→B");
    assert_eq!(out_b.parent_summaries.len(), 3);

    let result = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &Id::new(),
        &out_b.parent_summaries,
        &out_b.publish.child_covered_by,
        Some(job.id.as_str()),
        0,
    )
    .expect("publish batch 0");
    assert!(matches!(result, PublishOutcome::Applied));

    // ── Batch 1: B→C（同一 job、下一批次键）────────────────────────────
    // 从 DB 重读：A 已被 batch 0 覆盖，B summaries 已落库。
    let entries_b = sqlite_runtime::list_summaries(&campaign_id).expect("list B entries");
    let (ids, spans, groups) = plan_level_batch(&entries_b, ChronicleLevel::B, 2, 2)
        .expect("plan B")
        .expect("B groups");
    let out_c = publish_with_deterministic_texts(
        &campaign_id,
        &lineage_id,
        &conversation_id,
        &entries_b,
        &ids,
        &spans,
        &groups,
        ChronicleLevel::C,
    )
    .expect("deterministic B→C");
    assert!(!out_c.parent_summaries.is_empty());

    // 核心回归断言：batch 1 必须被 Applied，绝不能因 batch 0 已占用 job_id
    // 而被误判为重复/迟到结果丢弃。
    let result_c = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &Id::new(),
        &out_c.parent_summaries,
        &out_c.publish.child_covered_by,
        Some(job.id.as_str()),
        1,
    )
    .expect("publish batch 1 must not be treated as a duplicate");
    assert!(matches!(result_c, PublishOutcome::Applied));

    // 同批次重复（真迟到）仍被拒绝——判重维度没有放宽。
    let late = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &Id::new(),
        &out_c.parent_summaries,
        &out_c.publish.child_covered_by,
        Some(job.id.as_str()),
        1,
    );
    let late_err = late.expect_err("duplicate (job_id, batch) must be rejected");
    assert!(
        late_err.contains("job_id") && late_err.contains("already used"),
        "{late_err}"
    );

    // job 可以正常终态化，不卡在 Running。
    assert!(sqlite_runtime::compress_mark_succeeded(&job.id).expect("succeed"));
    let job_final = sqlite_runtime::compress_list_all()
        .expect("list jobs")
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("job row");
    assert_eq!(job_final.status, CompressJobStatus::Succeeded);

    // 发布结果完整：C 级 summary 存在、A/B 覆盖关系落库。
    let final_entries = sqlite_runtime::list_summaries(&campaign_id).expect("final summaries");
    assert!(
        final_entries
            .iter()
            .any(|s| s.chronicle_level() == ChronicleLevel::C),
        "C summary must exist after B→C publish"
    );
}

//! SQLite Chronicle crash-recovery batch key (Gate 4 三审 P1).
//!
//! Scenario: the first batch (A→B) is published under job key 1 (ChronicleLevel
//! B). The process then "crashes". A recovered run that only computes B→C must
//! publish under key 2 (ChronicleLevel C) — never renumbered to 0 — or the
//! batch is misread as a duplicate and the job retries until attempts exhaust.
//!
//! `sqlite_runtime::activate` is process-global, so this binary contains one
//! test and must run in its own process.

use storyforge_app_agent::chronicle_compressor::{
    plan_level_batch, publish_with_deterministic_texts,
};
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::chronicle::ChronicleLevel;
use storyforge_domain::conversation::Conversation;
use storyforge_infra_sqlite::publication::PublishOutcome;
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
fn crash_after_first_batch_recovered_run_publishes_under_stable_level_key() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    let card_id = Id::from_str("crash-card-1");
    let campaign_id = Id::from_str("crash-camp-1");
    let conversation_id = Id::from_str("crash-conv-1");
    let lineage_id = Id::from_str("crash-lin-1");

    let mut campaign = Campaign::new(card_id.clone(), "Crash Camp");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    campaign.lineage_id = Some(lineage_id.clone());
    sqlite_runtime::save_campaign(&campaign).expect("save campaign");
    sqlite_runtime::save_card_payload(
        &card_id,
        "Crash Card",
        Some("crash-source-1"),
        Some("2026-07-16T00:00:00Z"),
        &serde_json::json!({"card": {"id": card_id.as_str(), "name": "Crash Card"}}),
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

    // ── 第一段：A→B 发布成功（键 = output_level 派生，B=1）─────────────
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
    let b_key = out_b.output_level.as_u8() as u32;
    assert_eq!(b_key, 1, "B→ key must be 1");
    let result = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &Id::new(),
        &out_b.parent_summaries,
        &out_b.publish.child_covered_by,
        Some(job.id.as_str()),
        b_key,
    )
    .expect("publish A→B");
    assert!(matches!(result, PublishOutcome::Applied));

    // ── "崩溃"：进程状态丢失，但 A→B 的发布已持久化（job 仍 Running，
    //    崩溃恢复会把它重置为 Pending 后重跑）。─────────────────────────
    sqlite_runtime::compress_reset_running_to_pending().expect("crash recovery reset");

    // ── 恢复段：重跑。此时只有 B→C 可达（A 已被上一段覆盖）。
    let recovered = sqlite_runtime::compress_try_claim_pending(&job.id).expect("re-claim");
    assert!(recovered);
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
    // 断言：输出层级（C=2），绝不重编号为 0。
    assert_eq!(out_c.output_level, ChronicleLevel::C);
    let c_key = out_c.output_level.as_u8() as u32;
    assert_eq!(c_key, 2, "C→ key must be 2, never renumbered to 0");

    // 核心回归：恢复段的 B→C 用稳定键发布，必须 Applied——若 worker 用数组
    // 序号会把它编成 0，与已发布的 A→B（键 1）不冲突但更危险：恢复重跑若只
    // 算出 B→C 却编号 0，而实际发布 A→B 用的是 1…… 关键在于键稳定不与
    // 已发布批次撞车。此处断言已发布的键恰好是 2。
    let result_c = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &Id::new(),
        &out_c.parent_summaries,
        &out_c.publish.child_covered_by,
        Some(job.id.as_str()),
        c_key,
    )
    .expect("recovered B→C publish must not collide with published A→B");
    assert!(matches!(result_c, PublishOutcome::Applied));

    // 同键重发（真迟到）仍被拒绝。
    let late = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &Id::new(),
        &out_c.parent_summaries,
        &out_c.publish.child_covered_by,
        Some(job.id.as_str()),
        c_key,
    );
    let late_err = late.expect_err("duplicate (job_id, level key) must be rejected");
    assert!(
        late_err.contains("job_id") && late_err.contains("already used"),
        "{late_err}"
    );

    // 恢复后的 job 可以正常终态化。
    assert!(sqlite_runtime::compress_mark_succeeded(&job.id).expect("succeed"));
    let job_final = sqlite_runtime::compress_list_all()
        .expect("list jobs")
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("job row");
    assert_eq!(
        job_final.status,
        storyforge_lib::sqlite_compress_jobs::CompressJobStatus::Succeeded
    );

    // 两条发布都在：B 与 C 各至少一条，覆盖链完整。
    let final_entries = sqlite_runtime::list_summaries(&campaign_id).expect("final summaries");
    assert!(
        final_entries
            .iter()
            .any(|s| s.chronicle_level() == ChronicleLevel::C),
        "C summary must exist after recovered publish"
    );
    assert!(
        final_entries
            .iter()
            .any(|s| s.chronicle_level() == ChronicleLevel::B),
        "B summary must exist after first publish"
    );
}

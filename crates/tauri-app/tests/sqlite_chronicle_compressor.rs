//! SQLite-native Chronicle compressor lifecycle (Gate 4).
//!
//! `sqlite_runtime::activate` is process-global, so this integration binary
//! intentionally contains one test that walks the full queue lifecycle:
//! threshold count → enqueue (dedup) → claim (idempotent) → deterministic
//! A→B publish → late-result rejection → fault-injection rollback with job
//! retry → crash recovery (Running→Pending).

use storyforge_app_agent::chronicle_compressor::{
    plan_level_batch, publish_with_deterministic_texts,
};
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::chronicle::ChronicleLevel;
use storyforge_domain::conversation::Conversation;
use storyforge_infra_sqlite::publication::PublishFault;
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
fn sqlite_compress_queue_threshold_claim_publish_rollback_and_recovery() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    let card_id = Id::from_str("compress-card-1");
    let campaign_id = Id::from_str("compress-camp-1");
    let conversation_id = Id::from_str("compress-conv-1");
    let lineage_id = Id::from_str("compress-lin-1");

    // 1. 种子：campaign + conversation + 5 条未覆盖 A。
    let mut campaign = Campaign::new(card_id.clone(), "Compress Camp");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    campaign.lineage_id = Some(lineage_id.clone());
    sqlite_runtime::save_campaign(&campaign).expect("save campaign");
    sqlite_runtime::save_card_payload(
        &card_id,
        "Compress Card",
        Some("compress-source-1"),
        Some("2026-07-16T00:00:00Z"),
        &serde_json::json!({"card": {"id": card_id.as_str(), "name": "Compress Card"}}),
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

    // 2. 阈值判定：SQLite 权威计数。
    let (uncovered_a, uncovered_b) =
        sqlite_runtime::compress_count_uncovered(&campaign_id).expect("count uncovered");
    assert_eq!(uncovered_a, 5);
    assert_eq!(uncovered_b, 0);

    // 3. 入队 + campaign 级去重。
    let (job, created) = sqlite_runtime::compress_enqueue_or_get_open(
        &campaign_id,
        Some(conversation_id.clone()),
        Some(lineage_id.clone()),
        uncovered_a as u32,
        uncovered_b as u32,
    )
    .expect("enqueue");
    assert!(created);
    let (job2, created2) = sqlite_runtime::compress_enqueue_or_get_open(
        &campaign_id,
        Some(conversation_id.clone()),
        Some(lineage_id.clone()),
        uncovered_a as u32,
        uncovered_b as u32,
    )
    .expect("enqueue again");
    assert!(!created2);
    assert_eq!(job.id, job2.id);

    // 4. claim 幂等：重复启动的 worker 只有一个能领到。
    assert!(sqlite_runtime::compress_try_claim_pending(&job.id).expect("claim"));
    assert!(!sqlite_runtime::compress_try_claim_pending(&job.id).expect("second claim"));

    // 5. 确定性 A→B 压缩 + 原子发布（无 LLM）。
    let entries = sqlite_runtime::list_summaries(&campaign_id).expect("list summaries");
    let (ids, spans, groups) = plan_level_batch(&entries, ChronicleLevel::A, 4, 2)
        .expect("plan")
        .expect("groups");
    let outcome = publish_with_deterministic_texts(
        &campaign_id,
        &lineage_id,
        &conversation_id,
        &entries,
        &ids,
        &spans,
        &groups,
        ChronicleLevel::B,
    )
    .expect("deterministic publish");
    assert_eq!(outcome.parent_summaries.len(), 3);
    assert_eq!(outcome.publish.child_covered_by.len(), 5);

    let publication_id = Id::new();
    let result = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &publication_id,
        &outcome.parent_summaries,
        &outcome.publish.child_covered_by,
        Some(job.id.as_str()),
        0,
    )
    .expect("publish");
    assert!(matches!(
        result,
        storyforge_infra_sqlite::publication::PublishOutcome::Applied
    ));

    // 发布后：A 全部被覆盖，出现 2 条 B；计数归零（阈值不再触发）。
    let after = sqlite_runtime::list_summaries(&campaign_id).expect("list after publish");
    assert!(
        after
            .iter()
            .filter(|s| s.is_leaf_a() && s.covered_by.is_none())
            .count()
            == 0
    );
    assert!(
        after
            .iter()
            .filter(|s| s.chronicle_level() == ChronicleLevel::B)
            .count()
            == 3
    );
    let (uncovered_a_after, _) =
        sqlite_runtime::compress_count_uncovered(&campaign_id).expect("count after");
    assert_eq!(uncovered_a_after, 0);

    // 6. 迟到结果：同一 (job_id, batch=0) 再次发布 → 唯一索引拒绝（批次键内判重）。
    let late_publication_id = Id::new();
    let late_result = sqlite_runtime::publish_chronicle_compress(
        &campaign_id,
        &late_publication_id,
        &outcome.parent_summaries,
        &outcome.publish.child_covered_by,
        Some(job.id.as_str()),
        0,
    );
    assert!(
        late_result.is_err(),
        "duplicate (job_id, batch) publication must be rejected"
    );
    let late_err = late_result.unwrap_err();
    assert!(late_err.contains("job_id"), "got: {late_err}");

    // 7. 故障注入回滚：publish 在 parent 插入后失败 → 事务回滚 + job 回队。
    // 用 batch=1（未占用的批次键）确保注入路径真正执行，而非被迟到拒绝抢先。
    sqlite_runtime::fail_chronicle_publish_for_test(PublishFault::AfterParentInsert);
    let fault_pub_id = Id::new();
    let fault_result = sqlite_runtime::publish_chronicle_compress_with_fault_flag(
        &campaign_id,
        &fault_pub_id,
        &outcome.parent_summaries,
        &outcome.publish.child_covered_by,
        Some(job.id.as_str()),
        1,
    );
    assert!(fault_result.is_err());
    let fault_err = fault_result.unwrap_err();
    assert!(
        fault_err.contains("injected failure after parent insert"),
        "fault injection must be the failure, got: {fault_err}"
    );
    sqlite_runtime::fail_chronicle_publish_for_test(PublishFault::None);

    // 回滚证据：没有新的 B 条目、covered_by 未变、ledger 无 fault 记录。
    let after_fault = sqlite_runtime::list_summaries(&campaign_id).expect("list after fault");
    assert_eq!(
        after_fault
            .iter()
            .filter(|s| s.chronicle_level() == ChronicleLevel::B)
            .count(),
        3,
        "faulted publication parents must be rolled back"
    );
    assert!(
        after_fault
            .iter()
            .filter(|s| s.covered_by.is_none() && s.is_leaf_a())
            .count()
            == 0
    );

    // job 回队语义：mark_failed_or_retry 只在 Running 时生效（模拟 worker 失败处理）。
    assert!(
        sqlite_runtime::compress_mark_failed_or_retry(&job.id, "injected publish failure")
            .expect("mark failed")
    );
    let job_after = sqlite_runtime::compress_list_all()
        .expect("list all")
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("job row");
    assert_eq!(
        job_after.status,
        storyforge_lib::sqlite_compress_jobs::CompressJobStatus::Pending
    );
    assert!(job_after.last_error.as_deref() == Some("injected publish failure"));

    // 8. 崩溃恢复：Running → Pending 可重放。
    assert!(sqlite_runtime::compress_try_claim_pending(&job.id).expect("re-claim"));
    let reset = sqlite_runtime::compress_reset_running_to_pending().expect("reset");
    assert_eq!(reset, 1);
    let job_after_reset = sqlite_runtime::compress_list_all()
        .expect("list all")
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("job row");
    assert_eq!(
        job_after_reset.status,
        storyforge_lib::sqlite_compress_jobs::CompressJobStatus::Pending
    );

    // 9. 迟到 worker 终态化被拒：先 succeeded，再尝试 failed → 不生效。
    assert!(sqlite_runtime::compress_try_claim_pending(&job.id).expect("claim again"));
    assert!(sqlite_runtime::compress_mark_succeeded(&job.id).expect("succeed"));
    assert!(!sqlite_runtime::compress_mark_failed_or_retry(&job.id, "late").expect("late fail"));
    let job_final = sqlite_runtime::compress_list_all()
        .expect("list all")
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("job row");
    assert_eq!(
        job_final.status,
        storyforge_lib::sqlite_compress_jobs::CompressJobStatus::Succeeded
    );
    assert_eq!(job_final.last_error, None);
}

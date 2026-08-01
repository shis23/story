//! SQLite-native Chronicle compressor lifecycle (Gate 4).
//!
//! `sqlite_runtime::activate` is process-global, so this integration binary
//! intentionally contains one test that walks the full queue lifecycle:
//! threshold count → enqueue (dedup) → claim (idempotent) → deterministic
//! A→B publish → late-result rejection → fault-injection rollback with job
//! retry → crash recovery (Running→Pending).

use std::sync::Arc;

use storyforge_app_agent::chronicle_compressor::{
    plan_level_batch, publish_with_deterministic_texts,
};
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::chronicle::ChronicleLevel;
use storyforge_domain::conversation::Conversation;
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_infra_sqlite::publication::PublishFault;
use storyforge_lib::AppState;
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::StorageFacade;

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

#[tokio::test]
async fn sqlite_compress_queue_threshold_claim_publish_rollback_and_recovery() {
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

    // ── 10. 三.1：真实 worker 的发布步骤重确认 Running ─────────────────
    // 注入 compress：执行期间把 job 经 facade 终态化为 Succeeded（模拟并发
    // worker/恢复流程），再返回真实批次。worker 发布前必须重确认 Running 并
    // 丢弃迟到批次——终态 job 的 summary/coverage/revision 不得被改写。
    let wcamp_id = Id::from_str("worker-camp-2");
    let wconv_id = Id::from_str("worker-conv-2");
    let wlin_id = Id::from_str("worker-lin-2");
    let mut wcampaign = Campaign::new(Id::from_str("worker-card-2"), "Worker Camp 2");
    wcampaign.id = wcamp_id.clone();
    wcampaign.conversation_id = Some(wconv_id.clone());
    wcampaign.lineage_id = Some(wlin_id.clone());
    sqlite_runtime::save_campaign(&wcampaign).expect("save worker campaign");
    let mut wconversation = Conversation::new(None, Some(wcamp_id.clone()));
    wconversation.id = wconv_id.clone();
    sqlite_runtime::save_conversation(&wconversation).expect("save worker conversation");
    let wleaves: Vec<RoundSummary> = (1..=5)
        .map(|turn| leaf(&wcamp_id, &wconv_id, &wlin_id, turn))
        .collect();
    for summary in &wleaves {
        sqlite_runtime::seed_summary(summary).expect("seed worker summary");
    }
    let (wjob, _) = sqlite_runtime::compress_enqueue_or_get_open(
        &wcamp_id,
        Some(wconv_id.clone()),
        Some(wlin_id.clone()),
        wleaves.len() as u32,
        0,
    )
    .expect("enqueue worker job");
    let wjob_id = wjob.id.clone();

    let data_dir = temp.path().to_path_buf();
    let state = Arc::new(
        AppState::new_with_backend(
            data_dir,
            Arc::new(StorageFacade::new(
                temp.path().to_path_buf(),
                PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
            )),
        )
        .expect("SQLite AppState for worker"),
    );
    let entries = sqlite_runtime::list_summaries(&wcamp_id).expect("list worker entries");
    let (ids, spans, groups) = plan_level_batch(&entries, ChronicleLevel::A, 4, 2)
        .expect("plan worker")
        .expect("worker groups");
    let outcome = publish_with_deterministic_texts(
        &wcamp_id,
        &wlin_id,
        &wconv_id,
        &entries,
        &ids,
        &spans,
        &groups,
        ChronicleLevel::B,
    )
    .expect("deterministic worker publish");

    let state_for_compress = state.clone();
    let wjob_for_compress = wjob_id.clone();
    let wcamp_for_compress = wcamp_id.clone();
    storyforge_lib::backend_workflows::spawn_compress_job_worker_with(
        state.clone(),
        wjob_id.clone(),
        move |_st, _campaign_id, _lineage, _conv, _entries, _cancel| {
            let outcome = outcome.clone();
            let job_id = wjob_for_compress.clone();
            let state = state_for_compress.clone();
            let campaign_id = wcamp_for_compress.clone();
            Box::pin(async move {
                // 压缩执行期间被并发终态化。
                assert!(
                    state
                        .storage()
                        .succeed_compress_job(&job_id)
                        .expect("succeed mid-run"),
                    "worker 已 claim，job 必须 Running"
                );
                let _ = campaign_id;
                Ok(vec![outcome])
            })
        },
    );

    // 等待 worker 收敛：job 终态且发布步骤已跑完。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let done = sqlite_runtime::compress_list_all()
            .expect("list jobs")
            .into_iter()
            .find(|j| j.id == wjob_id)
            .map(|j| j.status == storyforge_lib::sqlite_compress_jobs::CompressJobStatus::Succeeded)
            .unwrap_or(false);
        if done || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // 断言：迟到批次被丢弃——没有新增 B、covered_by 未变、revision 未推进。
    let after = sqlite_runtime::list_summaries(&wcamp_id).expect("list after worker");
    assert_eq!(
        after.len(),
        wleaves.len(),
        "迟到批次不得新增 summary（worker 发布被丢弃）"
    );
    assert!(
        after
            .iter()
            .filter(|s| s.chronicle_level() == ChronicleLevel::B)
            .count()
            == 0,
        "迟到批次不得产生 B 级 summary"
    );
    assert!(
        after
            .iter()
            .filter(|s| s.is_leaf_a() && s.covered_by.is_none())
            .count()
            == wleaves.len(),
        "迟到批次不得改写 covered_by"
    );
    let wcampaign_after = sqlite_runtime::get_campaign(&wcamp_id)
        .expect("read campaign")
        .expect("campaign exists");
    assert_eq!(
        wcampaign_after.chronicle_revision, 0,
        "迟到批次不得推进 chronicle_revision"
    );
    let wjob_after = sqlite_runtime::compress_list_all()
        .expect("list jobs")
        .into_iter()
        .find(|j| j.id == wjob_id)
        .expect("job row");
    assert_eq!(
        wjob_after.status,
        storyforge_lib::sqlite_compress_jobs::CompressJobStatus::Succeeded,
        "job 保持并发终态化的 Succeeded"
    );
    assert_eq!(wjob_after.last_error, None);
    drop(state);
}

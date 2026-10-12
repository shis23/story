//! 取消端到端：DSH 停止按钮只取消前台。排队等收尾的下一轮被放弃且不执行，上一轮的
//! 后处理与自动接受照常完成（否则该轮无法接受，会话与存档从此错位）；正在成文的一轮
//! 照常中止，之后还能接着写。

mod support;

use serde_json::{Value, json};

use support::{FakePlugin, write_card};

async fn create_campaign(plugin: &mut FakePlugin, card: &std::path::Path) -> String {
    plugin.wait_notification("bridge.ready").await;
    let created = plugin
        .request("campaign.create", json!({ "card_path": card }))
        .await
        .expect("campaign.create");
    created["campaign_id"].as_str().unwrap().to_string()
}

fn turn_params(campaign_id: &str, intent: &str, history_turns: usize) -> Value {
    json!({ "campaign_id": campaign_id, "intent": intent, "history_turns": history_turns })
}

fn assert_accepted(report: &Value) {
    assert!(
        matches!(
            report["accept"].as_str(),
            Some("accepted" | "accepted_degraded")
        ),
        "未被接受: {report}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_queued_turn_leaves_the_previous_settle_alone() {
    let temp = tempfile::tempdir().unwrap();
    let card = write_card(temp.path());
    let mut plugin = FakePlugin::spawn(&temp.path().join("bridge-data"));
    let campaign_id = create_campaign(&mut plugin, &card).await;

    plugin
        .request("turn.run", turn_params(&campaign_id, "我推开木屋的门。", 0))
        .await
        .expect("第一轮");
    // 第一轮的收尾（摘要 / 后处理）调用扣住不答，下一轮只能排队。
    plugin.hold_llm = true;
    let queued = plugin
        .send_request(
            "turn.run",
            turn_params(&campaign_id, "我向 Seraphina 道谢。", 1),
        )
        .await;
    while plugin.held.is_empty() {
        plugin.pump().await;
    }

    let cancel = plugin
        .send_request("turn.cancel", json!({ "campaign_id": campaign_id }))
        .await;
    let cancelled = plugin.response(&cancel).await.expect("turn.cancel");
    assert_eq!(cancelled["cancelled"], true);
    let error = plugin
        .response(&queued)
        .await
        .expect_err("排队中的一轮应被放弃");
    assert_eq!(error["code"], "cancelled", "{error}");

    plugin.release_held().await;
    let report = plugin.wait_settled(1).await;
    assert_accepted(&report);
    assert_eq!(report["postprocess"], "PostProcessDone", "{report}");

    // 被放弃的一轮没有写进存档：存档仍是 1 轮，按 1 轮接着写。
    plugin
        .request(
            "turn.run",
            turn_params(&campaign_id, "我再次向 Seraphina 道谢。", 1),
        )
        .await
        .expect("取消后的下一轮");
    assert_accepted(&plugin.wait_settled(2).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_writing_aborts_the_turn_and_writing_continues() {
    let temp = tempfile::tempdir().unwrap();
    let card = write_card(temp.path());
    let mut plugin = FakePlugin::spawn(&temp.path().join("bridge-data"));
    let campaign_id = create_campaign(&mut plugin, &card).await;

    plugin.hold_llm = true;
    let running = plugin
        .send_request("turn.run", turn_params(&campaign_id, "我推开木屋的门。", 0))
        .await;
    while plugin.held.is_empty() {
        plugin.pump().await;
    }
    // 其它存档的取消不影响这一轮。
    let other = plugin
        .send_request("turn.cancel", json!({ "campaign_id": "another-campaign" }))
        .await;
    assert_eq!(
        plugin.response(&other).await.expect("turn.cancel")["cancelled"],
        false
    );
    let cancel = plugin
        .send_request("turn.cancel", json!({ "campaign_id": campaign_id }))
        .await;
    assert_eq!(
        plugin.response(&cancel).await.expect("turn.cancel")["cancelled"],
        true
    );
    plugin.release_held().await;
    let error = plugin
        .response(&running)
        .await
        .expect_err("成文中的一轮应被中止");
    assert_eq!(error["code"], "cancelled", "{error}");

    let output = plugin
        .request(
            "turn.run",
            turn_params(&campaign_id, "我再次推开木屋的门。", 0),
        )
        .await
        .expect("中止后重新写这一轮");
    assert!(!output["text"].as_str().unwrap().trim().is_empty());
    assert_accepted(&plugin.wait_settled(1).await);
}

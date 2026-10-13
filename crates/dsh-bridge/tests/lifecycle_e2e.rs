//! 生命周期端到端（PoC P3）：sidecar 被杀后在同一数据目录重启继续写；只能从最新
//! 一轮分叉，分叉后两个存档各自继续；DSH 历史与存档错位（重新生成 / 编辑历史）时
//! 拒绝执行且不发任何模型调用。

mod support;

use std::path::Path;

use serde_json::{Value, json};

use support::{FakePlugin, write_card};

async fn create_campaign(plugin: &mut FakePlugin, card: &Path) -> Value {
    plugin.wait_notification("bridge.ready").await;
    plugin
        .request("campaign.create", json!({ "card_path": card }))
        .await
        .expect("campaign.create")
}

async fn run_turn(
    plugin: &mut FakePlugin,
    campaign_id: &str,
    intent: &str,
    history_turns: usize,
) -> Result<Value, Value> {
    plugin
        .request(
            "turn.run",
            json!({ "campaign_id": campaign_id, "intent": intent, "history_turns": history_turns }),
        )
        .await
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
async fn a_killed_sidecar_restarts_on_the_same_data_and_keeps_writing() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("bridge-data");
    let card = write_card(temp.path());

    let mut first = FakePlugin::spawn(&data_dir);
    let created = create_campaign(&mut first, &card).await;
    let campaign_id = created["campaign_id"].as_str().unwrap().to_string();

    // 第一次模型调用进行中时把 sidecar 杀掉（模拟崩溃）。
    first.hold_llm = true;
    first
        .send(json!({
            "id": "p-crash",
            "method": "turn.run",
            "params": { "campaign_id": campaign_id, "intent": "我推开木屋的门。", "history_turns": 0 },
        }))
        .await;
    while first.held.is_empty() {
        first.pump().await;
    }
    first.child.start_kill().unwrap();
    first.child.wait().await.unwrap();

    // 同一数据目录重启：启动恢复清掉中断的活动 Turn，存档仍是 0 轮。
    let mut second = FakePlugin::spawn(&data_dir);
    second.wait_notification("bridge.ready").await;
    let output = run_turn(&mut second, &campaign_id, "我再次推开木屋的门。", 0)
        .await
        .expect("重启后第一轮");
    assert!(!output["text"].as_str().unwrap().trim().is_empty());
    assert_accepted(&second.wait_settled(1).await);

    run_turn(&mut second, &campaign_id, "我向 Seraphina 道谢。", 1)
        .await
        .expect("重启后第二轮");
    assert_accepted(&second.wait_settled(2).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn forks_only_from_the_latest_turn_and_both_branches_keep_writing() {
    let temp = tempfile::tempdir().unwrap();
    let card = write_card(temp.path());
    let mut plugin = FakePlugin::spawn(&temp.path().join("bridge-data"));
    let created = create_campaign(&mut plugin, &card).await;
    let source = created["campaign_id"].as_str().unwrap().to_string();
    let source_conversation = created["conversation_id"].as_str().unwrap().to_string();

    run_turn(&mut plugin, &source, "我推开木屋的门。", 0)
        .await
        .expect("第一轮");
    assert_accepted(&plugin.wait_settled(1).await);

    // 从开场（0 轮）分叉：存档已有 1 轮，不是最新一轮 → 拒绝。
    let refused = plugin
        .request(
            "campaign.fork",
            json!({ "campaign_id": source, "history_turns": 0 }),
        )
        .await
        .unwrap_err();
    assert_eq!(refused["code"], "out_of_sync", "{refused}");

    let forked = plugin
        .request(
            "campaign.fork",
            json!({ "campaign_id": source, "history_turns": 1, "name": "分支" }),
        )
        .await
        .expect("campaign.fork");
    let branch = forked["campaign_id"].as_str().unwrap().to_string();
    let branch_conversation = forked["conversation_id"].as_str().unwrap().to_string();
    assert_ne!(branch, source);
    assert_ne!(branch_conversation, source_conversation);
    assert_eq!(forked["turns"], 1);

    // 两个存档各写一轮，互不影响：各自的轮数独立推进。
    let on_branch = run_turn(&mut plugin, &branch, "我在分支里收拾行囊。", 1)
        .await
        .expect("分支第二轮");
    assert_eq!(on_branch["conversation_id"], branch_conversation.as_str());
    assert_accepted(&plugin.wait_settled(2).await);
    let on_source = run_turn(&mut plugin, &source, "我在原线里留下来过夜。", 1)
        .await
        .expect("原线第二轮");
    assert_eq!(on_source["conversation_id"], source_conversation.as_str());
    assert_accepted(&plugin.wait_settled(3).await);

    // 重新生成原线第二轮（DSH 历史只有 1 轮）：拒绝，且不发任何模型调用。
    let calls = plugin.llm_calls;
    let regenerate = run_turn(&mut plugin, &source, "我在原线里留下来过夜。", 1)
        .await
        .unwrap_err();
    assert_eq!(regenerate["code"], "out_of_sync", "{regenerate}");
    assert!(
        regenerate["message"]
            .as_str()
            .unwrap()
            .contains("不能重新生成")
    );
    assert_eq!(plugin.llm_calls, calls);

    run_turn(&mut plugin, &branch, "我和 Seraphina 一起出发。", 2)
        .await
        .expect("分支第三轮");
    assert_accepted(&plugin.wait_settled(4).await);
}

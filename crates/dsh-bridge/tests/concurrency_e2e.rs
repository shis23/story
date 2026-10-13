//! 并发端到端（PoC P2 验收第 4 项）：群像档里导演派出两个子 Agent，两次 `llm.stream`
//! 必须同时在途（插件侧屏障攒满两个才应答，串行执行会卡死超时）；分块交错到达、
//! 结果倒序返回时，每个子 Agent 的流式片段与产出都只含自己的内容。

mod support;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use support::{FakePlugin, subagent_part, write_card};

#[tokio::test(flavor = "multi_thread")]
async fn big_scene_subagents_stream_in_parallel_without_mixing() {
    let temp = tempfile::tempdir().unwrap();
    let card = write_card(temp.path());
    let mut plugin = FakePlugin::spawn(&temp.path().join("bridge-data"));
    plugin.director_plan = Some(
        json!({
            "scene_brief": "木屋里来了一位旅店老板，三人围着炉火商量下山的路",
            "subagent_tasks": [
                {
                    "character_id": "Seraphina",
                    "brief": "提醒旅人山路上的危险",
                    "context_package": { "character_brief": "守护森林的女法师", "task": "劝旅人等雨停" }
                },
                {
                    "character_id": "旅店老板",
                    "brief": "推销自家的马车",
                    "context_package": { "character_brief": "山脚旅店的老板，精明健谈", "task": "说服旅人坐马车下山" }
                }
            ]
        })
        .to_string(),
    );
    plugin.subagent_barrier = 2;

    plugin.wait_notification("bridge.ready").await;
    let created = plugin
        .request("campaign.create", json!({ "card_path": card }))
        .await
        .expect("campaign.create");
    let output = plugin
        .request(
            "turn.run",
            json!({
                "campaign_id": created["campaign_id"],
                "intent": "我问他们下山走哪条路。",
                "generation_mode": "big_scene",
                "history_turns": 0,
            }),
        )
        .await
        .expect("群像档一轮");
    assert!(!output["text"].as_str().unwrap().trim().is_empty());
    let report = plugin.wait_settled(1).await;
    assert!(
        matches!(
            report["accept"].as_str(),
            Some("accepted" | "accepted_degraded")
        ),
        "未被接受: {report}"
    );

    assert_eq!(
        plugin.max_in_flight_subagents, 2,
        "两个子 Agent 调用应同时在途"
    );
    assert_eq!(plugin.subagent_batches, 1, "每个子 Agent 应只调用一次");

    // 每个子 Agent（按 character_id）的流式片段与最终产出只含自己的标记。
    let mut progress: BTreeMap<String, String> = BTreeMap::new();
    let mut done: BTreeMap<String, String> = BTreeMap::new();
    for event in &plugin.event_log {
        let character = event["character_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match event["type"].as_str() {
            Some("SubagentProgress") => progress
                .entry(character)
                .or_default()
                .push_str(event["delta"].as_str().unwrap_or_default()),
            Some("SubagentDone") => {
                done.insert(character, event["full_text"].as_str().unwrap().to_string());
            }
            _ => {}
        }
    }
    assert_eq!(done.len(), 2, "两个子 Agent 都应完成: {:?}", plugin.events);
    let mut owners = BTreeSet::new();
    for (character, full_text) in &done {
        let owner = ["Seraphina", "旅店老板"]
            .into_iter()
            .find(|name| full_text.contains(&subagent_part(name, 1)))
            .unwrap_or_else(|| panic!("{character} 的产出没有任何角色标记: {full_text}"));
        assert_eq!(
            full_text,
            &format!("{}{}", subagent_part(owner, 1), subagent_part(owner, 2)),
            "{character} 的产出混入了别的流"
        );
        if let Some(streamed) = progress.get(character) {
            assert_eq!(streamed, full_text, "{character} 的流式片段与产出不一致");
        }
        owners.insert(owner);
    }
    assert_eq!(owners.len(), 2, "两个子 Agent 的产出应各自归属不同角色");
}

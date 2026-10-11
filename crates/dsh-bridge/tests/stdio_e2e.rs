//! 端到端：真的拉起 sidecar 进程，测试扮演 DSH 插件（见 `support`）。覆盖：数据
//! 目录隔离、导卡开档、两轮自动路由写作、显式群像档（导演 + 子 Agent）、工具协议
//! 文本降级、用量归属、后处理后自动接受、轮次串行、状态查询与退出。

mod support;

use serde_json::json;

use support::{FakePlugin, OPENING, STEP_TIMEOUT, write_card};

#[tokio::test(flavor = "multi_thread")]
async fn sidecar_runs_production_turns_with_models_served_by_the_plugin() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("bridge-data");
    let card_path = write_card(temp.path());
    let mut plugin = FakePlugin::spawn(&data_dir);

    let ready = plugin.wait_notification("bridge.ready").await;
    assert_eq!(ready["data_dir"], data_dir.display().to_string());

    // 导卡 → 角色识别（走插件模型）→ 开档，开场白来自卡的 first_mes。
    let created = plugin
        .request("campaign.create", json!({ "card_path": card_path }))
        .await
        .expect("campaign.create");
    let campaign_id = created["campaign_id"].as_str().unwrap().to_string();
    assert_eq!(created["card_name"], "Seraphina");
    assert_eq!(created["opening"], OPENING);
    let calls_after_create = plugin.llm_calls;
    assert!(calls_after_create >= 1, "角色识别应经插件调用模型");
    assert_eq!(created["usage"]["calls"], calls_after_create as u64);

    for (turn, intent) in ["我推开木屋的门。", "我向 Seraphina 道谢。"]
        .into_iter()
        .enumerate()
    {
        let output = plugin
            .request(
                "turn.run",
                json!({ "campaign_id": campaign_id, "intent": intent }),
            )
            .await
            .unwrap_or_else(|error| panic!("turn.run #{turn} failed: {error}"));
        assert!(!output["text"].as_str().unwrap().trim().is_empty());
        assert_eq!(output["campaign_id"], campaign_id.as_str());

        // 成文后后台后处理，落定后自动接受。
        let report = plugin.wait_settled(turn + 1).await;
        assert_eq!(report["node_id"], output["node_id"]);
        assert!(
            matches!(
                report["accept"].as_str(),
                Some("accepted" | "accepted_degraded")
            ),
            "turn #{turn} 未被接受: {report}"
        );
        assert!(
            report["postprocess"].is_string(),
            "缺少后处理终态: {report}"
        );
        assert!(output["usage"]["calls"].as_u64().unwrap() >= 1);
        assert!(output["usage"]["output_tokens"].as_u64().unwrap() > 0);
        assert!(report["usage"]["calls"].as_u64().unwrap() >= 1);
    }
    assert!(plugin.llm_calls > calls_after_create);
    // 生成模式走自动路由（单角色短输入会选续写档，不一定经过导演）。
    for expected in ["Started", "DraftReady", "QualityChecked", "PostProcessDone"] {
        assert!(
            plugin.events.iter().any(|kind| kind == expected),
            "缺少事件 {expected}: {:?}",
            plugin.events
        );
    }

    assert_eq!(plugin.fallback_prompt_requests, 0);

    // 无效的生成模式在参数层被拒。
    let invalid = plugin
        .request(
            "turn.run",
            json!({ "campaign_id": campaign_id, "intent": "x", "generation_mode": "huge" }),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid["code"], "invalid_params");

    // 文本降级 + 显式群像档：导演、子 Agent 都走插件；请求不带原生 tools。
    let configured = plugin
        .request("bridge.configure", json!({ "tool_mode": "text_fallback" }))
        .await
        .expect("configure");
    assert_eq!(configured["tool_mode"], "text_fallback");
    let native_before = plugin.native_tool_requests;
    let events_before = plugin.events.len();
    let calls_before = plugin.llm_calls;
    let output = plugin
        .request(
            "turn.run",
            json!({
                "campaign_id": campaign_id,
                "intent": "我把信交给 Seraphina，问她要不要一起出发。",
                "generation_mode": "big_scene",
            }),
        )
        .await
        .expect("big_scene turn");
    assert_eq!(output["generation_mode"], "big_scene");
    let turn_events = &plugin.events[events_before..];
    for expected in [
        "DirectorStarted",
        "DirectorDone",
        "SubagentStarted",
        "SubagentDone",
    ] {
        assert!(
            turn_events.iter().any(|kind| kind == expected),
            "群像档缺少事件 {expected}: {turn_events:?}"
        );
    }
    // 后处理在成文前一刻已在后台起步，它的请求可能先于 turn.run 的响应到达。
    let turn_calls = output["usage"]["calls"].as_u64().unwrap();
    assert!(
        (3..=(plugin.llm_calls - calls_before) as u64).contains(&turn_calls),
        "本轮正文用量 {turn_calls} 应覆盖导演 + 子 Agent + 成文"
    );
    let report = plugin.wait_settled(3).await;
    assert!(matches!(
        report["accept"].as_str(),
        Some("accepted" | "accepted_degraded")
    ));
    assert_eq!(plugin.native_tool_requests, native_before);
    assert!(
        plugin.fallback_prompt_requests >= 1,
        "文本降级应注入工具说明"
    );

    let status = plugin
        .request("bridge.status", json!({}))
        .await
        .expect("status");
    assert_eq!(status["active_campaign"], campaign_id.as_str());
    assert_eq!(status["busy"], false);
    assert_eq!(status["tool_mode"], "text_fallback");
    assert_eq!(status["usage"]["calls"], plugin.llm_calls as u64);
    assert_eq!(
        status["last_settle"]["node_id"],
        plugin.settled[2]["node_id"]
    );

    // 隔离：调用日志与存储都落在 sidecar 自己的数据目录。
    assert!(data_dir.join("logs").is_dir());
    assert!(data_dir.join("conversations").is_dir());

    let unknown = plugin.request("nope", json!({})).await.unwrap_err();
    assert_eq!(unknown["code"], "unknown_method");

    plugin
        .request("bridge.shutdown", json!({}))
        .await
        .expect("shutdown");
    let status = tokio::time::timeout(STEP_TIMEOUT, plugin.child.wait())
        .await
        .expect("sidecar 未退出")
        .unwrap();
    assert!(status.success());
}

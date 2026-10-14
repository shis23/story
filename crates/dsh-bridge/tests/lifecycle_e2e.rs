//! 生命周期端到端（PoC P3）：sidecar 被杀后在同一数据目录重启继续写；只能从最新
//! 一轮分叉，分叉后两个存档各自继续；DSH 历史与存档错位（重新生成 / 编辑历史）时
//! 拒绝执行且不发任何模型调用。

mod support;

use std::path::Path;

use serde_json::{Value, json};

use support::{
    FakePlugin, MVU_TRANSLATION, SHELL_INLINE_HTML, SHELL_OPENING, SHELL_STATUS_URL,
    STATUS_OPENING, write_card, write_mvu_card, write_shell_card, write_status_card,
};

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

async fn display(plugin: &mut FakePlugin, params: Value) -> Value {
    plugin
        .request("conversation.display", params)
        .await
        .expect("conversation.display")["items"]
        .clone()
}

/// 展示文本按 DSH 回放数据里记下的节点取：开场与每轮都带节点位置；展示正则的深度
/// 按节点当前位置算；分叉换了节点 id，继承的轮次按位置到分支对话里取。
#[tokio::test(flavor = "multi_thread")]
async fn display_follows_recorded_nodes_across_turns_and_forks() {
    let temp = tempfile::tempdir().unwrap();
    let card = write_status_card(temp.path());
    let mut plugin = FakePlugin::spawn(&temp.path().join("bridge-data"));
    let created = create_campaign(&mut plugin, &card).await;
    let campaign = created["campaign_id"].as_str().unwrap().to_string();
    let conversation = created["conversation_id"].as_str().unwrap().to_string();
    let opening_node = created["opening_node_id"].as_str().unwrap().to_string();
    assert_eq!(created["opening"], STATUS_OPENING);
    assert_eq!(created["opening_node_index"], 0);
    // 开档时开场白是最新一条消息，展示正则生效。
    assert_eq!(
        created["opening_display"],
        "林间的光斑落在木屋门前。\n<div class=\"sf-bar\">体力 80</div>"
    );

    let output = run_turn(&mut plugin, &campaign, "我推开木屋的门。", 0)
        .await
        .expect("第一轮");
    let turn_node = output["node_id"].as_str().unwrap().to_string();
    assert_eq!(output["node_index"], 2, "开场 0、用户 1、回复 2");
    assert_eq!(output["display"], output["text"], "正文里没有状态栏标签");
    assert_accepted(&plugin.wait_settled(1).await);

    let items = display(
        &mut plugin,
        json!({ "campaign_id": campaign, "refs": [
            { "key": "opening", "conversation_id": conversation, "node_id": opening_node, "node_index": 0 },
            { "key": "turn", "conversation_id": conversation, "node_id": turn_node, "node_index": 2 },
            { "key": "user", "conversation_id": conversation, "node_index": 1 },
            { "key": "stale", "conversation_id": conversation, "node_id": "no-such-node", "node_index": 0 },
        ] }),
    )
    .await;
    // 开场白已不是最新一条（深度 2 > maxDepth 0）：展示正则不再作用。
    assert_eq!(items["opening"]["display_content"], STATUS_OPENING);
    assert_eq!(items["opening"]["by_position"], false);
    assert_eq!(items["turn"]["node_id"], turn_node.as_str());
    assert_eq!(items["turn"]["display_content"], output["text"]);
    assert!(items["user"].is_null(), "用户消息不返回: {items}");
    assert!(
        items["stale"].is_null(),
        "对话还在而节点对不上时不按位置猜: {items}"
    );

    let forked = plugin
        .request(
            "campaign.fork",
            json!({ "campaign_id": campaign, "history_turns": 1, "name": "分支" }),
        )
        .await
        .expect("campaign.fork");
    let branch = forked["campaign_id"].as_str().unwrap().to_string();

    let branch_conversation = forked["conversation_id"].as_str().unwrap().to_string();

    // 分叉继承的轮次：回放数据记的是来源对话，按位置到会话当前（分支）对话里取，
    // 节点 id 已换新；来源对话还在时核对正文一致。
    let inherited = json!([
        { "key": "opening", "conversation_id": conversation, "node_id": opening_node, "node_index": 0 },
        { "key": "turn", "conversation_id": conversation, "node_id": turn_node, "node_index": 2 },
        { "key": "shifted", "conversation_id": conversation, "node_id": turn_node, "node_index": 0 },
    ]);
    let items = display(
        &mut plugin,
        json!({ "campaign_id": branch, "refs": inherited }),
    )
    .await;
    assert_eq!(items["opening"]["by_position"], true);
    assert_eq!(
        items["opening"]["conversation_id"],
        branch_conversation.as_str()
    );
    assert_ne!(items["opening"]["node_id"], opening_node.as_str());
    assert_eq!(items["opening"]["display_content"], STATUS_OPENING);
    assert_eq!(items["turn"]["display_content"], output["text"]);
    assert!(
        items["shifted"].is_null(),
        "位置与来源正文对不上时不返回: {items}"
    );

    // 来源对话已不在（模拟来源存档被删）：照样按位置取。
    let orphaned = json!([
        { "key": "opening", "conversation_id": "deleted-conversation", "node_id": opening_node, "node_index": 0 },
    ]);
    let items = display(
        &mut plugin,
        json!({ "campaign_id": branch, "refs": orphaned }),
    )
    .await;
    assert_eq!(items["opening"]["by_position"], true);
    assert_eq!(items["opening"]["display_content"], STATUS_OPENING);
    // 没有当前存档：按记下的对话取，对话不在就没有。
    let items = display(&mut plugin, json!({ "refs": inherited })).await;
    assert_eq!(items["opening"]["conversation_id"], conversation.as_str());
    assert_eq!(items["opening"]["by_position"], false);
    assert!(display(&mut plugin, json!({ "refs": orphaned })).await["opening"].is_null());

    let too_many: Vec<Value> = (0..=200)
        .map(|i| json!({ "key": i.to_string(), "conversation_id": conversation, "node_index": 0 }))
        .collect();
    let refused = plugin
        .request("conversation.display", json!({ "refs": too_many }))
        .await
        .unwrap_err();
    assert_eq!(refused["code"], "invalid_params", "{refused}");
}

async fn mvu(plugin: &mut FakePlugin, method: &str, params: Value) -> Value {
    plugin.request(method, params).await.expect(method)
}

fn variable<'a>(variables: &'a Value, key: &str) -> Option<&'a Value> {
    variables
        .as_array()?
        .iter()
        .find(|entry| entry["key"] == key)
        .map(|entry| &entry["value"])
}

#[tokio::test(flavor = "multi_thread")]
async fn mvu_panel_analyzes_applies_and_writes_variables_behind_the_settle() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("bridge-data");
    let mut plugin = FakePlugin::spawn(&data_dir);
    plugin.mvu_translation = Some(MVU_TRANSLATION.to_string());

    // 纯数据卡（没有 JS、没有 MVU 数据）：分析器直接降级，不调用模型，结果标为空。
    let plain = create_campaign(&mut plugin, &write_card(temp.path())).await;
    let plain_id = plain["campaign_id"].as_str().unwrap();
    let before = plugin.llm_calls;
    let analyzed = mvu(
        &mut plugin,
        "mvu.analyze",
        json!({ "campaign_id": plain_id }),
    )
    .await;
    assert_eq!(analyzed["empty"], true, "{analyzed}");
    assert_eq!(plugin.llm_calls, before, "纯数据卡不该调用模型");
    assert_eq!(plugin.mvu_requests, 0);

    // MVU 卡：分析前没有翻译。
    let created = plugin
        .request(
            "campaign.create",
            json!({ "card_path": write_mvu_card(temp.path()) }),
        )
        .await
        .expect("campaign.create");
    let campaign_id = created["campaign_id"].as_str().unwrap().to_string();
    let target = json!({ "campaign_id": campaign_id });
    let status = mvu(&mut plugin, "mvu.status", target.clone()).await;
    assert!(status["translation"].is_null(), "{status}");
    assert_eq!(status["pending_apply"], json!([]));
    assert_eq!(status["card_name"], "Seraphina");
    let instances = status["instances"].as_array().unwrap();
    assert!(!instances.is_empty(), "{status}");
    assert!(!status["definitions"].as_array().unwrap().is_empty());
    let instance_id = instances[0]["id"].as_str().unwrap().to_string();

    // 手动分析：调用一次模型，翻译入库；兜底片段不带原始 JS；变量定义待应用。
    let analyzed = mvu(&mut plugin, "mvu.analyze", target.clone()).await;
    assert_eq!(plugin.mvu_requests, 1);
    assert_eq!(analyzed["empty"], false, "{analyzed}");
    assert!(
        analyzed["usage"]["calls"].as_u64().unwrap() >= 1,
        "{analyzed}"
    );
    let translation = &analyzed["status"]["translation"];
    assert_eq!(translation["ui_bindings"][0]["variable_key"], "hp");
    assert_eq!(translation["interactions"][0]["element_label"], "休息");
    assert_eq!(
        translation["fallback_fragments"],
        json!([{ "description": "开场动画", "reason": "需要 DOM" }])
    );
    assert!(!analyzed.to_string().contains("playIntro"));
    assert!(analyzed["status"]["analyzed_at"].is_string());
    let pending = analyzed["status"]["pending_apply"].as_array().unwrap();
    assert!(
        pending
            .iter()
            .any(|entry| entry["added"].as_array().unwrap().contains(&json!("金币"))),
        "{pending:?}"
    );

    // 应用：变量定义进卡内角色，已有实例补上默认值。
    let applied = mvu(&mut plugin, "mvu.apply", target.clone()).await;
    assert_eq!(applied["pending_apply"], json!([]), "{applied}");
    let backfilled = applied["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| instance["id"] == instance_id.as_str())
        .unwrap();
    assert_eq!(
        variable(&backfilled["variables"], "金币"),
        Some(&json!(5)),
        "{backfilled}"
    );

    // 写一轮；成文后立即写变量——写入排在后台收尾之后执行，不撞活动 Turn 屏障。
    let output = run_turn(&mut plugin, &campaign_id, "我推开木屋的门。", 0)
        .await
        .expect("turn.run");
    assert_eq!(output["campaign_id"], campaign_id.as_str());
    let settled_before = plugin.settled.len();
    let written = mvu(
        &mut plugin,
        "variables.set",
        json!({
            "campaign_id": campaign_id,
            "writes": [
                { "scope": "instance", "instance_id": instance_id, "key": "hp", "value": 90 },
                { "scope": "campaign", "key": "story_clock", "value": "第3天" },
                { "scope": "campaign", "key": "__storyforge_card_shell_variables", "value": 1 },
                { "scope": "instance", "instance_id": "no-such-instance", "key": "hp", "value": 1 },
                { "scope": "instance", "key": "hp", "value": 1 },
            ],
        }),
    )
    .await;
    assert_eq!(
        plugin.settled.len(),
        settled_before + 1,
        "写变量应在收尾之后返回"
    );
    assert_accepted(plugin.settled.last().unwrap());
    let results = written["results"].as_array().unwrap();
    let oks: Vec<bool> = results
        .iter()
        .map(|result| result["ok"].as_bool().unwrap())
        .collect();
    assert_eq!(oks, [true, true, false, false, false], "{results:?}");
    assert!(
        results[2]["error"].as_str().unwrap().contains("__ 内部段"),
        "{results:?}"
    );
    assert!(
        results[3]["error"]
            .as_str()
            .unwrap()
            .contains("no-such-instance"),
        "{results:?}"
    );
    let status = &written["status"];
    let instance = status["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| instance["id"] == instance_id.as_str())
        .unwrap();
    assert_eq!(variable(&instance["variables"], "hp"), Some(&json!(90)));
    assert_eq!(
        variable(&status["campaign_variables"], "story_clock"),
        Some(&json!("第3天"))
    );

    // 写入数量上限与参数校验。
    let too_many: Vec<Value> = (0..33)
        .map(|i| json!({ "scope": "campaign", "key": format!("k{i}"), "value": i }))
        .collect();
    let refused = plugin
        .request(
            "variables.set",
            json!({ "campaign_id": campaign_id, "writes": too_many }),
        )
        .await
        .unwrap_err();
    assert_eq!(refused["code"], "invalid_params", "{refused}");
    let refused = plugin
        .request(
            "variables.set",
            json!({ "campaign_id": campaign_id, "writes": [{ "scope": "global", "key": "k", "value": 1 }] }),
        )
        .await
        .unwrap_err();
    assert_eq!(refused["code"], "invalid_params", "{refused}");
    let missing = plugin
        .request("mvu.status", json!({ "campaign_id": "no-such-campaign" }))
        .await
        .unwrap_err();
    assert_eq!(missing["code"], "not_found", "{missing}");
}

/// 卡片界面清单：`.load(url)` 状态栏壳带 URL，内联壳只带触发正则（不带正文）；主机白名单
/// 与桌面同一份。展示数据同时给原文，插件按原文里的命中给内联文档背书。
#[tokio::test(flavor = "multi_thread")]
async fn card_shell_manifest_lists_the_card_shells_and_display_keeps_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let mut plugin = FakePlugin::spawn(&temp.path().join("bridge-data"));
    let created = create_campaign(&mut plugin, &write_shell_card(temp.path())).await;
    let campaign = created["campaign_id"].as_str().unwrap().to_string();

    let manifest = plugin
        .request("card_shell.manifest", json!({ "campaign_id": campaign }))
        .await
        .expect("card_shell.manifest");
    assert_eq!(manifest["campaign_id"], campaign);
    let shells = manifest["shells"].as_array().unwrap();
    let status = shells
        .iter()
        .find(|shell| shell["source"] == "remote_url")
        .unwrap_or_else(|| panic!("{manifest}"));
    assert_eq!(status["kind"], "status_bar");
    assert_eq!(status["url"], SHELL_STATUS_URL);
    assert_eq!(status["label"], "状态栏界面");
    let inline = shells
        .iter()
        .find(|shell| shell["source"] == "inline_html")
        .unwrap_or_else(|| panic!("{manifest}"));
    assert_eq!(inline["kind"], "message_html");
    assert_eq!(inline["trigger"], "<sf-inline/>");
    assert!(inline.get("url").is_none(), "{inline}");
    assert!(
        !manifest.to_string().contains("内联界面已运行"),
        "清单不带内联正文"
    );
    assert!(
        manifest["remote_urls"]
            .as_array()
            .unwrap()
            .contains(&json!(SHELL_STATUS_URL))
    );
    let hosts = manifest["allowed_hosts"].as_array().unwrap();
    assert!(
        hosts.contains(&json!("testingcf.jsdelivr.net")),
        "{manifest}"
    );
    assert!(hosts.contains(&json!("i.postimg.cc")), "{manifest}");

    let items = display(
        &mut plugin,
        json!({ "campaign_id": campaign, "refs": [{
            "key": "opening",
            "conversation_id": created["conversation_id"],
            "node_id": created["opening_node_id"],
            "node_index": 0
        }] }),
    )
    .await;
    let opening = &items["opening"];
    assert_eq!(opening["content"], SHELL_OPENING);
    let shown = opening["display_content"].as_str().unwrap();
    assert!(
        shown.contains(&format!("$('body').load('{SHELL_STATUS_URL}')")),
        "{shown}"
    );
    assert!(shown.contains(SHELL_INLINE_HTML), "{shown}");

    let missing = plugin
        .request(
            "card_shell.manifest",
            json!({ "campaign_id": "no-such-campaign" }),
        )
        .await
        .expect_err("不存在的存档");
    assert!(missing.to_string().contains("not_found"), "{missing}");
}

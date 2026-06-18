//! Tier 1 — 真实 LLM 全链路。
//!
//! T1：导入卡 → 角色识别 → 建 Campaign → 激活 → 组装 campaign-mode 上下文 →
//! `start_writing` 一轮。验证接缝，确认 campaign 驱动写作端到端跑得通。
//!
//! `#[ignore]` + `require_real_llm()` 保护：默认 `cargo test` 零网络。
//! 显式 `cargo test -p harness-real-llm -- --ignored` 跑真实 LLM。

use std::sync::Arc;

use storyforge_app_pipeline::WritingContext;
use storyforge_domain::character::CharacterCard;
use storyforge_infra_llm::mock_client::MockLlmClient;
use storyforge_infra_llm::LlmClient;
use tokio::sync::{mpsc, watch};

use harness_real_llm::{HarnessEnv, require_real_llm};

/// 定位仓库根的 fixture 文件。
/// 优先 `STORYFORGE_FIXTURE_<NAME>`（去扩展名大写）环境变量；
/// 否则从 cwd 向上逐级找，直到命中文件（仓库根有 test-card-seraphina.png）。
fn find_fixture(name: &str) -> std::path::PathBuf {
    let env_key = format!(
        "STORYFORGE_FIXTURE_{}",
        name.trim_end_matches(".png").to_uppercase().replace('-', "_")
    );
    if let Ok(p) = std::env::var(&env_key) {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return p;
        }
    }
    // 从 cwd 向上逐级找，命中即返回。仓库根（C:\...\storyforge）含 fixture。
    let mut dir = std::env::current_dir().expect("无法获取 cwd");
    loop {
        let candidate = dir.join(name);
        if candidate.exists() {
            return candidate;
        }
        dir = match dir.parent() {
            Some(p) => p.to_path_buf(),
            None => return std::path::PathBuf::from(name),
        };
    }
}

/// T1：真实 LLM 跑通首轮 campaign 写作全链路。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL 或 data/connections.json）"]
async fn t1_first_turn_campaign_writing() {
    let llm: Arc<dyn LlmClient> = require_real_llm();
    let env = HarnessEnv::new(llm);

    // 1. 导入 seraphina 卡（仓库根的 fixture）
    let card_path = find_fixture("test-card-seraphina.png");
    let bytes = std::fs::read(&card_path).unwrap_or_else(|e| {
        panic!("读不到 fixture {}: {e}", card_path.display())
    });
    let character = storyforge_infra_import::import_character(&bytes)
        .expect("导入 seraphina 卡失败");
    let source_id = character.id.clone();
    env.inject_character(character);
    eprintln!("导入成功，source id = {source_id}");

    // 2. 角色识别（真实 LLM）
    let card = env.extract_characters(source_id.as_str()).await;
    eprintln!(
        "识别完成：card id={}，{} 个定义",
        card.id,
        card.character_definitions.len()
    );

    // 3. 建 Campaign + 激活
    let campaign_id = env.create_campaign(&card, "harness-campaign");
    eprintln!("Campaign 已激活: {campaign_id}");

    // 4. 组装 campaign-mode 上下文
    let conversation_id = env.conv_store.create(None).id;
    let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx = env.fill_campaign_context(ctx);
    assert!(
        ctx.campaign_id.is_some(),
        "fill_campaign_context 后 campaign_id 应非 None"
    );
    assert!(
        ctx.campaign_runtime.is_some(),
        "campaign_runtime 应已组装"
    );
    eprintln!(
        "上下文就绪：turn={}，instances={}",
        ctx.turn,
        ctx.campaign_runtime
            .as_ref()
            .map(|c| c.instances.len())
            .unwrap_or(0)
    );

    // 5. 跑写作流水线
    let mut pipeline = env.new_pipeline();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    // cancel sender 必须保活到 start_writing 结束（否则 sender drop 误触发 Cancelled）
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let result = pipeline
        .start_writing("开场：角色登场".into(), &ctx, event_tx, cancel_rx)
        .await;
    let (text, _node_id, provenance) = result.expect("start_writing 失败");
    eprintln!("成文长度 {} 字，provenance={}", text.len(), provenance.is_some());

    // 6. 断言成文非空
    assert!(!text.trim().is_empty(), "成文不应为空");

    // 7. 断言事件序列
    let mut events = Vec::new();
    while let Ok(ev) = event_rx.try_recv() {
        events.push(ev);
    }
    let event_names: Vec<&str> = events
        .iter()
        .map(|e| match e {
            storyforge_domain::agent::PipelineEvent::Started { .. } => "started",
            storyforge_domain::agent::PipelineEvent::DirectorStarted => "director_started",
            storyforge_domain::agent::PipelineEvent::DirectorDone { .. } => "director_done",
            storyforge_domain::agent::PipelineEvent::SubagentStarted { .. } => "subagent_started",
            storyforge_domain::agent::PipelineEvent::SubagentDone { .. } => "subagent_done",
            storyforge_domain::agent::PipelineEvent::EditorStarted => "editor_started",
            storyforge_domain::agent::PipelineEvent::DraftReady { .. } => "draft_ready",
            storyforge_domain::agent::PipelineEvent::Committed { .. } => "committed",
            _ => "other",
        })
        .collect();
    eprintln!("事件序列: {:?}", event_names);
    assert!(event_names.contains(&"started"), "缺 started");
    assert!(event_names.contains(&"director_done"), "缺 director_done");
    assert!(event_names.contains(&"draft_ready"), "缺 draft_ready");

    // 8. 断言会话落盘
    let conv = env
        .conv_store
        .get(&conversation_id)
        .expect("会话应已落盘");
    assert!(!conv.nodes.is_empty(), "对话树不应为空");
    eprintln!("T1 通过：{} 个节点落盘", conv.nodes.len());

    env.cleanup();
}

/// 烟雾测试（不 ignore，不需要 LLM）：验证 HarnessEnv 的 campaign 组装
/// 在 mock 下能跑通接缝——证明 harness 脚手架本身无误，与真实 LLM 解耦。
#[test]
fn harness_seam_smoke_with_mock() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    // 手工建最小 Character + 用 fallback_from_character 产 1 个 protagonist 定义
    use storyforge_domain::character::{Character, CharacterDefinition};
    use storyforge_domain::Source;
    let ch = Character {
        id: storyforge_domain::Id::from_str("smoke-src"),
        name: "smoke-protagonist".into(),
        description: "测试主角".into(),
        personality: "冷静".into(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        alternate_greetings: vec![],
        embedded_world_info: None,
        extensions: serde_json::Value::Null,
        renderable_assets: None,
        source: Source::Native,
        spec_version: "3.0".into(),
        raw_card_json: serde_json::Value::Null,
    };
    let mut card = CharacterCard::from_character(&ch);
    let def = CharacterDefinition::fallback_from_character(&ch, &[]);
    card.character_definitions = vec![def];
    let stored = env.campaign_store.save_card(card);
    let card = stored.card;

    let campaign_id = env.create_campaign(&card, "smoke-campaign");
    assert_eq!(env.active_campaign_id(), Some(campaign_id.clone()));

    let conv_id = env.conv_store.create(None).id;
    let ctx = WritingContext::legacy(vec![], None, conv_id);
    let ctx = env.fill_campaign_context(ctx);
    assert_eq!(ctx.campaign_id, Some(campaign_id));
    assert!(ctx.campaign_runtime.is_some());
    assert_eq!(ctx.turn, 1, "无历史摘要时 turn 应为 1");
    assert_eq!(
        ctx.campaign_runtime.as_ref().unwrap().instances.len(),
        1,
        "应实例化 1 个 protagonist"
    );
    eprintln!("smoke 通过：campaign 组装接缝正常");

    env.cleanup();
}

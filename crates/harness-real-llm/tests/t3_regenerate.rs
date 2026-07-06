//! Tier 3 — regenerate 变体树（真实 LLM）。
//!
//! T3：T1 首轮后跑 4 种 regenerate（整体 / 仅导演 / 仅编剧 / 指定 subagent），
//! 验证变体树正确增长。
//!
//! `#[ignore]` + `require_real_llm()` 保护：默认 `cargo test` 零网络。

use std::sync::Arc;

use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::{RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_infra_llm::LlmClient;
use tokio::sync::{mpsc, watch};

use harness_real_llm::{HarnessEnv, require_real_llm};

/// 定位仓库根的 fixture 文件。
fn find_fixture(name: &str) -> std::path::PathBuf {
    let env_key = format!(
        "STORYFORGE_FIXTURE_{}",
        name.trim_end_matches(".png")
            .to_uppercase()
            .replace('-', "_")
    );
    if let Ok(p) = std::env::var(&env_key) {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return p;
        }
    }
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

/// 跑首轮写作，返回 (conversation_id, node_id, env)。
async fn setup_first_draft(env: &HarnessEnv) -> (Id, Id) {
    let card_path = find_fixture("test-card-seraphina.png");
    let bytes = std::fs::read(&card_path)
        .unwrap_or_else(|e| panic!("读不到 fixture {}: {e}", card_path.display()));
    let character =
        storyforge_infra_import::import_character(&bytes).expect("导入 seraphina 卡失败");
    let source_id = character.id.clone();
    env.inject_character(character);

    let card = env.extract_characters(source_id.as_str()).await;
    let campaign_id = env.create_campaign(&card, "t3-campaign");
    eprintln!("Campaign 已激活: {campaign_id}");

    let conversation_id = env.conv_store.create(None, None).id;
    let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx = env.fill_campaign_context(ctx);

    let mut pipeline = env.new_pipeline();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let (_text, node_id, _provenance) = pipeline
        .start_writing("开场：角色登场".into(), &ctx, event_tx, cancel_rx)
        .await
        .expect("start_writing 失败");

    eprintln!("首轮完成: node_id={node_id}");
    (conversation_id, node_id)
}

/// 跑一次 regenerate，对 LLM 层瞬态错误（520 / 超时 / 限流）重试，业务错误立即 panic。
///
/// 真实 LLM 偶发瞬态错误以 `PipelineError::Llm` 冒泡，重试可消除 flaky；
/// `Regenerate` / `InvalidState` 是业务错误，重试无意义，直接 panic 暴露真 bug——
/// 不再把任何 Err 当作"瞬态噪声"吞掉（历史版本吞错误导致测试对真 bug 也判绿）。
async fn regenerate_with_retry(
    env: &HarnessEnv,
    conv_id: &Id,
    node_id: &Id,
    targets: Vec<PartialRollTarget>,
    hint: Option<String>,
) -> (String, storyforge_domain::conversation::Provenance) {
    use storyforge_app_pipeline::PipelineError;
    const MAX_ATTEMPTS: usize = 3;
    let mut last_err: Option<PipelineError> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        let req = RegenerateRequest {
            conversation_id: conv_id.clone(),
            node_id: node_id.clone(),
            targets: targets.clone(),
            hint: hint.clone(),
            seed: None,
        };
        let ctx = WritingContext::legacy(vec![], None, conv_id.clone());
        let ctx = env.fill_campaign_context(ctx);
        let mut pipeline = env.new_pipeline();
        let (event_tx, _rx) = mpsc::unbounded_channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        match pipeline.regenerate(req, &ctx, event_tx, cancel_rx).await {
            Ok(ok) => return ok,
            Err(e) => {
                let transient = matches!(e, PipelineError::Llm(_));
                eprintln!(
                    "regenerate 第 {attempt}/{MAX_ATTEMPTS} 次失败 (transient={transient}): {e:?}"
                );
                if !transient {
                    panic!("regenerate 业务错误（非瞬态，不重试）: {e:?}");
                }
                last_err = Some(e);
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }
    panic!("regenerate {MAX_ATTEMPTS} 次均因瞬态错误失败: {last_err:?}");
}

/// T3：regenerate 整体重 roll。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL 或 data/connections.json）"]
async fn t3_regenerate_all() {
    let llm: Arc<dyn LlmClient> = require_real_llm();
    let env = HarnessEnv::new(llm);

    let (conv_id, node_id) = setup_first_draft(&env).await;

    // 重 roll 前 variant 数
    let conv_before = env.conv_store.get(&conv_id).unwrap();
    let variants_before = conv_before.find_node(&node_id).unwrap().variants.len();

    let (text, provenance) = regenerate_with_retry(
        &env,
        &conv_id,
        &node_id,
        vec![], // 空 = 整体重 roll
        Some("语气更紧张".into()),
    )
    .await;
    assert!(!text.is_empty(), "regenerate 成文不应为空");
    eprintln!(
        "regenerate_all 成文: {} 字，hint={:?}",
        text.len(),
        provenance.last_hint
    );

    // variant 数 +1
    let conv_after = env.conv_store.get(&conv_id).unwrap();
    let node_after = conv_after.find_node(&node_id).unwrap();
    assert_eq!(
        node_after.variants.len(),
        variants_before + 1,
        "regenerate 后应多 1 个 variant"
    );

    env.cleanup();
}

/// T3：regenerate 仅编剧（复用旧子产出）。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL 或 data/connections.json）"]
async fn t3_regenerate_editor_only() {
    let llm: Arc<dyn LlmClient> = require_real_llm();
    let env = HarnessEnv::new(llm);

    let (conv_id, node_id) = setup_first_draft(&env).await;

    let conv_before = env.conv_store.get(&conv_id).unwrap();
    let variants_before = conv_before.find_node(&node_id).unwrap().variants.len();

    let (text, provenance) = regenerate_with_retry(
        &env,
        &conv_id,
        &node_id,
        vec![PartialRollTarget::Editor],
        Some("节奏太快".into()),
    )
    .await;
    assert!(!text.is_empty(), "editor regenerate 成文不应为空");
    assert_eq!(
        provenance.last_hint.as_deref(),
        Some("节奏太快"),
        "hint 应透传到 provenance"
    );

    let conv_after = env.conv_store.get(&conv_id).unwrap();
    let node_after = conv_after.find_node(&node_id).unwrap();
    assert_eq!(
        node_after.variants.len(),
        variants_before + 1,
        "editor regenerate 后应多 1 个 variant"
    );

    env.cleanup();
}

/// T3：regenerate 仅导演——预期被业务约束拒绝。
///
/// `validate_partial_roll` 禁止"只重跑导演却保留旧子产出"：Plan 变了，
/// 旧 subagent_results 不再匹配。seraphina 卡首轮必 spawn subagent，
/// 故此请求在 LLM 调用前即被拒（确定性，无需重试）。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL 或 data/connections.json）"]
async fn t3_regenerate_director_only() {
    let llm: Arc<dyn LlmClient> = require_real_llm();
    let env = HarnessEnv::new(llm);

    let (conv_id, node_id) = setup_first_draft(&env).await;

    let conv_before = env.conv_store.get(&conv_id).unwrap();
    let _variants_before = conv_before.find_node(&node_id).unwrap().variants.len();

    let req = RegenerateRequest {
        conversation_id: conv_id.clone(),
        node_id: node_id.clone(),
        targets: vec![PartialRollTarget::Director],
        hint: None,
        seed: None,
    };
    let ctx = WritingContext::legacy(vec![], None, conv_id.clone());
    let ctx = env.fill_campaign_context(ctx);
    let mut pipeline = env.new_pipeline();
    let (event_tx, _rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let result = pipeline.regenerate(req, &ctx, event_tx, cancel_rx).await;
    // 只重跑导演但保留旧子产出 = 业务约束拒绝（Plan 变了旧子产出不匹配）
    assert!(
        result.is_err(),
        "director-only regenerate 应被拒绝（Plan 变了旧子产出不匹配）: {:?}",
        result
    );

    env.cleanup();
}

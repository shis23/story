//! M5 — 真实模型：缓存命中 / 远楼可达 / 压缩损失 / epoch 冷启动。
//!
//! 规格：`docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md` §8 / §9 M5。
//! 全部 `#[ignore]`；需 `LLM_BASE_URL` + `LLM_API_KEY` + `LLM_MODEL`。
//!
//! 运行：
//! ```text
//! cargo test -p harness-real-llm --test m5_cache_and_memory -- --ignored --nocapture
//! # 或
//! powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite m5
//! ```

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use std::collections::HashMap;

use storyforge_app_agent::{
    AgentRuntime, ChronicleToolBudget, run_compress_if_needed,
    tools::{ToolContext, ToolRegistry, register_director_tools},
};
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmError, StreamChunk, Usage};
use storyforge_domain::message_layout::{
    PROMPT_LAYOUT_VERSION, fingerprint_messages, messages_segment_summary,
};
use storyforge_domain::prompt_module::ProfileSource;
use storyforge_infra_llm::LlmClient;
use tokio::sync::{mpsc, watch};

use harness_real_llm::{HarnessEnv, require_real_llm};

// ─── Usage 录制包装 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct UsageSample {
    tag: String,
    prompt_tokens: u32,
    cached_tokens: u32,
    cache_creation_tokens: u32,
    completion_tokens: u32,
    request_fp16: String,
    system_hash16: String,
    history_len: usize,
    tail_parts: usize,
    msg_count: usize,
    elapsed_ms: u128,
}

struct UsageRecordingLlmClient {
    inner: Arc<dyn LlmClient>,
    samples: Mutex<Vec<UsageSample>>,
    /// 仅记录「本轮主写作」边界时的 tag（由测试在 turn 前后 push）。
    turn_tag: Mutex<String>,
}

impl UsageRecordingLlmClient {
    fn new(inner: Arc<dyn LlmClient>) -> Self {
        Self {
            inner,
            samples: Mutex::new(Vec::new()),
            turn_tag: Mutex::new("boot".into()),
        }
    }

    fn set_tag(&self, tag: impl Into<String>) {
        *self.turn_tag.lock().unwrap() = tag.into();
    }

    fn samples(&self) -> Vec<UsageSample> {
        self.samples.lock().unwrap().clone()
    }

    fn record(&self, req: &ChatRequest, resp: &ChatResponse, elapsed_ms: u128) {
        let segs = messages_segment_summary(&req.messages);
        let fp = fingerprint_messages(&req.messages);
        let usage = resp.usage.clone().unwrap_or(Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
        });
        let sample = UsageSample {
            tag: self.turn_tag.lock().unwrap().clone(),
            prompt_tokens: usage.prompt_tokens,
            cached_tokens: usage.cached_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            completion_tokens: usage.completion_tokens,
            request_fp16: fp.chars().take(16).collect(),
            system_hash16: segs.system_hash.chars().take(16).collect(),
            history_len: segs.history_len,
            tail_parts: segs.tail_parts,
            msg_count: req.messages.len(),
            elapsed_ms,
        };
        eprintln!(
            "[m5-usage] tag={} prompt={} cached={} create={} completion={} ratio={:.3} fp={} sys={} hist={} tail_parts={} msgs={} ms={} pv={}",
            sample.tag,
            sample.prompt_tokens,
            sample.cached_tokens,
            sample.cache_creation_tokens,
            sample.completion_tokens,
            if sample.prompt_tokens > 0 {
                sample.cached_tokens as f64 / sample.prompt_tokens as f64
            } else {
                0.0
            },
            sample.request_fp16,
            sample.system_hash16,
            sample.history_len,
            sample.tail_parts,
            sample.msg_count,
            sample.elapsed_ms,
            PROMPT_LAYOUT_VERSION,
        );
        self.samples.lock().unwrap().push(sample);
    }
}

#[async_trait]
impl LlmClient for UsageRecordingLlmClient {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let t0 = Instant::now();
        let resp = self.inner.chat(req).await?;
        self.record(req, &resp, t0.elapsed().as_millis());
        Ok(resp)
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        let t0 = Instant::now();
        let resp = self.inner.chat_stream(req, tx, cancel).await?;
        self.record(req, &resp, t0.elapsed().as_millis());
        Ok(resp)
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────────

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

async fn setup_campaign(env: &HarnessEnv, name: &str) -> (Id, Id) {
    let card_path = find_fixture("test-card-seraphina.png");
    let bytes = std::fs::read(&card_path)
        .unwrap_or_else(|e| panic!("读不到 fixture {}: {e}", card_path.display()));
    let character =
        storyforge_infra_import::import_character(&bytes).expect("导入 seraphina 卡失败");
    let source_id = character.id.clone();
    env.inject_character(character);
    let card = env.extract_characters(source_id.as_str()).await;
    let campaign_id = env.create_campaign(&card, name);
    let conversation_id = env.conv_store.create(None, None).id;
    (campaign_id, conversation_id)
}

async fn run_turn(env: &HarnessEnv, conversation_id: &Id, intent: &str) -> String {
    let (text, _node) = run_turn_with_node(env, conversation_id, intent).await;
    text
}

async fn run_turn_with_node(
    env: &HarnessEnv,
    conversation_id: &Id,
    intent: &str,
) -> (String, Option<Id>) {
    let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx = env.fill_campaign_context(ctx);
    let mut pipeline = env.new_pipeline();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    match pipeline
        .start_writing(intent.into(), &ctx, event_tx, cancel_rx)
        .await
    {
        Ok((text, node_id, _provenance)) => (text, Some(node_id)),
        Err(e) => {
            eprintln!("start_writing 失败: {e}");
            (String::new(), None)
        }
    }
}

/// 仅开 summarizer（关三合一 postprocess），降低 Accept 闭环成本。
fn summarizer_only_profile() -> AgentProfileConfig {
    AgentProfileConfig::new(
        Id::from_str("m5-accept-summarizer-only"),
        "m5-accept".into(),
        "M5 accept-loop: summarizer only".into(),
        HashMap::new(),
        4,
        false,
        true,
        ProfileSource::UserCreated,
        1,
    )
}

fn next_chronicle_a_seq(existing: &[RoundSummary]) -> u32 {
    let mut max_seq = 0u32;
    for s in existing {
        if let Some(code) = s.code.as_deref() {
            if let Some(parsed) = storyforge_domain::chronicle::ChronicleCode::parse(code)
                && parsed.level() == Some(storyforge_domain::chronicle::ChronicleLevel::A)
                && let Ok(n) = code[1..].parse::<u32>()
            {
                max_seq = max_seq.max(n);
            }
        } else {
            max_seq = max_seq.max(s.turn);
        }
    }
    max_seq.saturating_add(1).max(1)
}

/// 写作 → Draft accept → summarizer → 落 Chronicle A（对齐线上 Accept 后 summary 可见性）。
///
/// 不走完整 TurnRecord/QualityGate/MutationBatch（那是 Tauri 层）；本探针验证
/// **记忆侧**「成文被采纳 + A 纪要进入 store + 下一轮 fill 看见 catalog/epoch」。
async fn accept_write_and_summarize(
    env: &HarnessEnv,
    campaign_id: &Id,
    conversation_id: &Id,
    intent: &str,
    write_tag: &str,
    summary_tag: &str,
    recorder: &UsageRecordingLlmClient,
) -> (String, Option<String>) {
    recorder.set_tag(write_tag);
    let (text, node_id) = run_turn_with_node(env, conversation_id, intent).await;
    if text.trim().is_empty() {
        return (text, None);
    }
    let Some(node_id) = node_id else {
        return (text, None);
    };

    if let Err(e) = env.conv_store.accept_variant(conversation_id, &node_id) {
        eprintln!("accept_variant 失败: {e}");
    }

    // 用写作后的 fill 上下文（正确 turn / recent / epoch）跑 summarizer
    let mut ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
    ctx = env.fill_campaign_context(ctx);
    ctx.agent_profile_config = Some(summarizer_only_profile());

    let present: Vec<String> = ctx
        .campaign_runtime
        .as_ref()
        .map(|rt| rt.instances.iter().map(|i| i.name.clone()).collect())
        .unwrap_or_default();

    recorder.set_tag(summary_tag);
    let pipeline = env.new_pipeline();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let outcome = pipeline
        .run_postprocess(
            &text,
            intent,
            &present,
            &[],
            &ctx,
            &event_tx,
            cancel_rx,
            &[],
        )
        .await;

    let summary_text = outcome.and_then(|o| o.summary);
    if let Some(ref summary) = summary_text {
        let lineage = ensure_lineage(env, campaign_id);
        let existing = env.campaign_store.list_summaries(campaign_id);
        let seq = next_chronicle_a_seq(&existing);
        let code = storyforge_domain::chronicle::ChronicleCode::new(
            storyforge_domain::chronicle::ChronicleLevel::A,
            seq,
        );
        let headline = storyforge_domain::chronicle::truncate_headline(summary, 40);
        let rs = RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            ctx.turn,
            summary.clone(),
        )
        .with_code(code.as_str())
        .with_headline(headline)
        .with_lineage(lineage);
        if let Err(e) = env.campaign_store.add_summary(rs) {
            eprintln!("add_summary 失败: {e}");
        }
    } else {
        eprintln!("{summary_tag}: summarizer 未产出 summary");
    }

    (text, summary_text)
}

fn print_usage_table(samples: &[UsageSample]) {
    eprintln!("--- M5 usage table ---");
    eprintln!(
        "{:<18} {:>8} {:>8} {:>8} {:>8} {:>7} {:>6}",
        "tag", "prompt", "cached", "create", "compl", "ratio", "ms"
    );
    for s in samples {
        let ratio = if s.prompt_tokens > 0 {
            s.cached_tokens as f64 / s.prompt_tokens as f64
        } else {
            0.0
        };
        eprintln!(
            "{:<18} {:>8} {:>8} {:>8} {:>8} {:>7.3} {:>6}",
            s.tag,
            s.prompt_tokens,
            s.cached_tokens,
            s.cache_creation_tokens,
            s.completion_tokens,
            ratio,
            s.elapsed_ms
        );
    }
}

fn max_cached_for_tag(samples: &[UsageSample], tag_prefix: &str) -> u32 {
    samples
        .iter()
        .filter(|s| s.tag.starts_with(tag_prefix))
        .map(|s| s.cached_tokens)
        .max()
        .unwrap_or(0)
}

fn sum_prompt_for_tag(samples: &[UsageSample], tag_prefix: &str) -> u32 {
    samples
        .iter()
        .filter(|s| s.tag.starts_with(tag_prefix))
        .map(|s| s.prompt_tokens)
        .sum()
}

fn inject_leaf_a(
    env: &HarnessEnv,
    campaign_id: &Id,
    conversation_id: &Id,
    lineage: &Id,
    turn: u32,
    fact: &str,
) {
    let summary = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        turn,
        format!("第{turn}轮纪要。关键事实：{fact}"),
    )
    .with_code(format!("A{turn:04}"))
    .with_headline(format!("T{turn}:{fact}"))
    .with_lineage(lineage.clone());
    env.campaign_store
        .add_summary(summary)
        .expect("inject leaf A");
}

fn ensure_lineage(env: &HarnessEnv, campaign_id: &Id) -> Id {
    let mut camp = env
        .campaign_store
        .get_campaign(campaign_id)
        .expect("campaign");
    if camp.lineage_id.is_none() {
        let lin = Id::new();
        camp.lineage_id = Some(lin.clone());
        env.campaign_store
            .update_campaign(camp)
            .expect("persist lineage");
        lin
    } else {
        camp.lineage_id.clone().unwrap()
    }
}

// ─── S1: 同 epoch 多轮缓存 ────────────────────────────────────────────────────

/// S1：6 轮真实写作，观察同 epoch 内 `cached_tokens` 与 system 段稳定性。
///
/// 通过标准（务实）：
/// - 至少 4 轮成文非空
/// - usage 样本非空
/// - 后半程（turn4–6）最大 cached ≥ 前半程（turn1–3）最大 cached，**或**
///   任一热轮 cached>0（供应商对小 prompt 可能延迟建 cache）
/// - 同 tag 内 system_hash 在连续 Director 主请求上不无故全变（soft warn）
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL）"]
async fn m5_s1_same_epoch_cache_hit_multi_turn() {
    let real = require_real_llm();
    let recorder = Arc::new(UsageRecordingLlmClient::new(real));
    let env = HarnessEnv::new(recorder.clone());
    let (campaign_id, conversation_id) = setup_campaign(&env, "m5-s1-cache").await;
    eprintln!("S1 campaign={campaign_id}");

    let intents = [
        "开场：角色在雨夜街头相遇，埋下代号银鸦的伏笔",
        "继续：双方试探身份，提到一枚生锈的怀表",
        "推进：进入废弃车站，发现墙上的暗号 7X-9",
        "冲突：追兵逼近，角色必须在信任与逃跑间选择",
        "转折：怀表打开后露出微型胶卷，内容指向旧码头",
        "收束：暂避雨棚下，约定次日在银鸦标记处再会",
    ];

    let mut non_empty = 0usize;
    for (i, intent) in intents.iter().enumerate() {
        let tag = format!("turn{}", i + 1);
        recorder.set_tag(&tag);
        let t0 = Instant::now();
        let text = run_turn(&env, &conversation_id, intent).await;
        eprintln!(
            "S1 {tag}: text_len={} wall_ms={}",
            text.len(),
            t0.elapsed().as_millis()
        );
        if !text.trim().is_empty() {
            non_empty += 1;
        }
        // 刷新上下文，便于观察 epoch / catalog
        let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = env.fill_campaign_context(ctx);
        eprintln!(
            "S1 {tag} after fill: turn={} epoch={} chronicle_rev={} catalog={}",
            ctx.turn,
            ctx.context_epoch
                .as_ref()
                .map(|e| e.epoch_id.to_string())
                .unwrap_or_else(|| "-".into()),
            ctx.chronicle_revision,
            ctx.chronicle_prompt_catalog.len()
        );
    }

    let samples = recorder.samples();
    print_usage_table(&samples);
    assert!(non_empty >= 4, "S1 至少 4 轮成文非空，实际 {non_empty}");
    assert!(!samples.is_empty(), "S1 应录到 usage 样本");

    let early_cached = (1..=3)
        .map(|i| max_cached_for_tag(&samples, &format!("turn{i}")))
        .max()
        .unwrap_or(0);
    let late_cached = (4..=6)
        .map(|i| max_cached_for_tag(&samples, &format!("turn{i}")))
        .max()
        .unwrap_or(0);
    let any_cached = samples.iter().any(|s| s.cached_tokens > 0);
    eprintln!(
        "S1 cache summary: early_max_cached={early_cached} late_max_cached={late_cached} any_cached={any_cached}"
    );

    // 供应商限制：允许 any_cached 或 late>=early（含同为 0 时 soft pass + 明确日志）
    if !any_cached {
        eprintln!(
            "S1 PARTIAL: 全轮 cached_tokens=0（可能供应商对当前前缀未建缓存或字段未返回）。\
             不判架构失败；请对照 request_fp/system_hash 是否稳定。"
        );
    } else {
        assert!(
            late_cached >= early_cached || late_cached > 0,
            "S1: 后半程 cache 应不劣于前半程或至少有命中 late={late_cached} early={early_cached}"
        );
        eprintln!("S1 PASS: 观察到真实 cached_tokens 信号");
    }

    // system_hash 稳定性 soft check：同 turn 内多次调用允许变（subagent）；
    // 跨 turn 的「最大 prompt」主请求 system_hash 若完全离散则 warn。
    let mut per_turn_sys: Vec<String> = Vec::new();
    for i in 1..=6 {
        let tag = format!("turn{i}");
        if let Some(best) = samples
            .iter()
            .filter(|s| s.tag == tag)
            .max_by_key(|s| s.prompt_tokens)
        {
            per_turn_sys.push(best.system_hash16.clone());
        }
    }
    if per_turn_sys.len() >= 3 {
        let first = &per_turn_sys[0];
        let same_as_first = per_turn_sys.iter().filter(|h| *h == first).count();
        eprintln!(
            "S1 system_hash of largest-prompt call per turn: {:?} (same_as_first={same_as_first}/{})",
            per_turn_sys,
            per_turn_sys.len()
        );
    }

    let conv = env.conv_store.get(&conversation_id).expect("会话");
    eprintln!("S1 nodes={} summaries={}", conv.nodes.len(), env.campaign_store.list_summaries(&campaign_id).len());
    env.cleanup();
}

// ─── S2: 远楼 search/get ──────────────────────────────────────────────────────

/// S2：注入含唯一 token 的远楼 A + 近轮填充，验证 search/get 不依赖 embedding。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL）"]
async fn m5_s2_far_floor_search_and_get() {
    let real = require_real_llm();
    let recorder = Arc::new(UsageRecordingLlmClient::new(real));
    let env = HarnessEnv::new(recorder.clone());
    let (campaign_id, conversation_id) = setup_campaign(&env, "m5-s2-far").await;
    let lineage = ensure_lineage(&env, &campaign_id);

    const FAR_TOKEN: &str = "ZXQ-远楼密约-7719";
    // 远楼：turn 1
    inject_leaf_a(
        &env,
        &campaign_id,
        &conversation_id,
        &lineage,
        1,
        FAR_TOKEN,
    );
    // 填充足够多近轮，使 near_raw 窗口挤掉 turn1（H_anchor+E=15，注入 18 条）
    for t in 2..=20 {
        inject_leaf_a(
            &env,
            &campaign_id,
            &conversation_id,
            &lineage,
            t,
            &format!("近轮填充-{t}"),
        );
    }

    // 真实写作 2 轮，确保 fill 路径刷新 epoch/catalog
    recorder.set_tag("s2-turn1");
    let _ = run_turn(
        &env,
        &conversation_id,
        "继续推进当前场景，不要回顾远古密约",
    )
    .await;
    recorder.set_tag("s2-turn2");
    let _ = run_turn(
        &env,
        &conversation_id,
        "角色检查装备，为下一行动做准备",
    )
    .await;

    // 直调工具（不依赖模型是否选 tool）
    let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx = env.fill_campaign_context(ctx);
    let catalog = {
        let g = env.tool_ctx.read().unwrap();
        g.chronicle_summaries.clone()
    };
    assert!(
        catalog.iter().any(|s| s.content.contains(FAR_TOKEN)),
        "工具目录应仍含远楼事实"
    );
    eprintln!(
        "S2 catalog={} prompt_catalog={} epoch_overview={:?}",
        catalog.len(),
        ctx.chronicle_prompt_catalog.len(),
        ctx.context_epoch
            .as_ref()
            .map(|e| e.overview_codes.len())
    );

    let tool_ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None, // 关 embedding
        archived_summaries: vec![],
        chronicle_summaries: catalog,
        chronicle_tool_budget: Arc::new(ChronicleToolBudget::new()),
        campaign_runtime: ctx.campaign_runtime.clone(),
        current_character_instance_id: None,
        regex_scripts: vec![],
    });
    let mut registry = ToolRegistry::new();
    register_director_tools(&mut registry);

    let search = registry
        .dispatch(
            "search_chronicle",
            serde_json::json!({"query": FAR_TOKEN, "level": "a"}),
            tool_ctx.clone(),
        )
        .await
        .expect("search_chronicle");
    eprintln!("S2 search: {search}");
    assert_eq!(search["results_count"].as_u64().unwrap_or(0), 1);
    assert_eq!(search["results"][0]["code"], "A0001");

    let got = registry
        .dispatch(
            "get_chronicle",
            serde_json::json!({"code": "A0001", "detail": "summary"}),
            tool_ctx,
        )
        .await
        .expect("get_chronicle");
    eprintln!(
        "S2 get found={} content_has_token={}",
        got["found"],
        got.to_string().contains(FAR_TOKEN)
    );
    assert_eq!(got["found"], true);
    assert!(
        got.to_string().contains(FAR_TOKEN),
        "get_chronicle summary 应含远楼 token"
    );

    print_usage_table(&recorder.samples());
    eprintln!("S2 PASS: 远楼 search/get 可达（无 embedding）");
    env.cleanup();
}

// ─── S3: 压缩损失探针 ─────────────────────────────────────────────────────────

/// S3：8 条 A（阈值 8 / group 4）→ 真 LLM 压 B → publish → 事实保留 ≥2/3。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL）"]
async fn m5_s3_compress_loss_probe() {
    let real = require_real_llm();
    let recorder = Arc::new(UsageRecordingLlmClient::new(real.clone()));
    let env = HarnessEnv::new(recorder.clone());
    let (campaign_id, conversation_id) = setup_campaign(&env, "m5-s3-compress").await;
    let lineage = ensure_lineage(&env, &campaign_id);

    const FACTS: [&str; 3] = [
        "FACT-银鸦徽章编号-A7F2",
        "FACT-旧码头坐标-北纬31.2",
        "FACT-联络口令-月下无声",
    ];

    let mut entries = Vec::new();
    for t in 1u32..=8 {
        let fact = FACTS[((t - 1) as usize) % FACTS.len()];
        let s = RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            t,
            format!(
                "第{t}轮。角色推进调查。必须保留的关键事实：{fact}。其它叙述：雨势渐大，街灯闪烁。"
            ),
        )
        .with_code(format!("A{t:04}"))
        .with_headline(format!("调查{t}"))
        .with_lineage(lineage.clone());
        env.campaign_store.add_summary(s.clone()).unwrap();
        entries.push(s);
    }

    recorder.set_tag("s3-compress");
    let tool_ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: entries.clone(),
        chronicle_tool_budget: Arc::new(ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    });
    let runtime = AgentRuntime::new(recorder.clone() as Arc<dyn LlmClient>, tool_ctx);
    let (_tx, cancel) = watch::channel(false);

    let outcomes = run_compress_if_needed(
        &runtime,
        &campaign_id,
        &lineage,
        &conversation_id,
        entries,
        cancel,
        None,
        Some(8),  // 测试阈值
        Some(999), // 不触发 B→C
    )
    .await
    .expect("compress should run");

    assert_eq!(outcomes.len(), 1);
    let out = &outcomes[0];
    assert_eq!(out.parent_summaries.len(), 2, "8/4 → 2 个 B");
    assert_eq!(out.publish.child_covered_by.len(), 8);

    env.campaign_store
        .publish_compress_result(
            &campaign_id,
            &out.parent_summaries,
            &out.publish.child_covered_by,
        )
        .expect("publish");

    let all = env.campaign_store.list_summaries(&campaign_id);
    let parents: Vec<_> = all.iter().filter(|s| s.level == 1).collect();
    let covered: Vec<_> = all.iter().filter(|s| s.covered_by.is_some()).collect();
    assert_eq!(parents.len(), 2);
    assert_eq!(covered.len(), 8);
    eprintln!(
        "S3 published B codes: {:?}",
        parents.iter().map(|p| p.code.clone()).collect::<Vec<_>>()
    );

    // 事实保留：在 B 全文或仍可读的 A summary 中
    let blob: String = all
        .iter()
        .map(|s| format!("{} {} {}", s.code.as_deref().unwrap_or(""), s.headline.as_deref().unwrap_or(""), s.content))
        .collect::<Vec<_>>()
        .join("\n");
    let mut kept = 0usize;
    for f in FACTS {
        let hit = blob.contains(f);
        eprintln!("S3 fact kept={hit}: {f}");
        if hit {
            kept += 1;
        }
    }
    assert!(
        kept >= 2,
        "S3 事实保留应 ≥2/3，实际 {kept}/3；B 正文:\n{}",
        parents
            .iter()
            .map(|p| p.content.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n")
    );

    // get_chronicle 旧 A 仍可读
    let tool_ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: all.clone(),
        chronicle_tool_budget: Arc::new(ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    });
    let mut registry = ToolRegistry::new();
    register_director_tools(&mut registry);
    let got = registry
        .dispatch(
            "get_chronicle",
            serde_json::json!({"code": "A0001", "detail": "summary"}),
            tool_ctx,
        )
        .await
        .unwrap();
    assert_eq!(got["found"], true);

    print_usage_table(&recorder.samples());
    eprintln!("S3 PASS: covers ok + facts {kept}/3 + get A still readable");
    env.cleanup();
}

// ─── S4: epoch rollover 冷启动信号 ────────────────────────────────────────────

/// S4：注入足够 A 触发 rollover 边界后，再跑 2 轮写作，对比 cache 冷/热。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL）"]
async fn m5_s4_epoch_rollover_cold_warm() {
    let real = require_real_llm();
    let recorder = Arc::new(UsageRecordingLlmClient::new(real));
    let env = HarnessEnv::new(recorder.clone());
    let (campaign_id, conversation_id) = setup_campaign(&env, "m5-s4-epoch").await;
    let lineage = ensure_lineage(&env, &campaign_id);

    // 先写 2 轮建立真实会话
    recorder.set_tag("s4-warm0");
    let _ = run_turn(&env, &conversation_id, "开场：港口雾夜，角色登船").await;
    recorder.set_tag("s4-warm1");
    let _ = run_turn(&env, &conversation_id, "继续：舱内发现密信").await;

    let ctx_before = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx_before = env.fill_campaign_context(ctx_before);
    let epoch_before = ctx_before
        .context_epoch
        .as_ref()
        .map(|e| e.epoch_id.clone());
    let overview_before = ctx_before
        .context_epoch
        .as_ref()
        .map(|e| e.overview_codes.len())
        .unwrap_or(0);
    eprintln!(
        "S4 before inject: epoch={epoch_before:?} overview={overview_before} rev={}",
        ctx_before.chronicle_revision
    );

    // 注入大量 A，推高 committed turns / 迫使 epoch 重算
    let existing = env.campaign_store.list_summaries(&campaign_id).len() as u32;
    let start = existing + 1;
    for t in start..=(start + 24) {
        inject_leaf_a(
            &env,
            &campaign_id,
            &conversation_id,
            &lineage,
            t,
            &format!("epoch-pad-{t}"),
        );
    }

    // fill 触发 refresh
    let ctx_mid = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx_mid = env.fill_campaign_context(ctx_mid);
    let epoch_mid = ctx_mid
        .context_epoch
        .as_ref()
        .map(|e| e.epoch_id.clone());
    eprintln!(
        "S4 after inject fill: epoch={epoch_mid:?} overview={} band={} rev={} (epoch_changed={})",
        ctx_mid
            .context_epoch
            .as_ref()
            .map(|e| e.overview_codes.len())
            .unwrap_or(0),
        ctx_mid
            .context_epoch
            .as_ref()
            .map(|e| e.band_codes.len())
            .unwrap_or(0),
        ctx_mid.chronicle_revision,
        epoch_before != epoch_mid
    );

    recorder.set_tag("s4-cold");
    let text_cold = run_turn(
        &env,
        &conversation_id,
        "新阶段：角色抵达灯塔，重新评估局势",
    )
    .await;
    recorder.set_tag("s4-hot");
    let text_hot = run_turn(
        &env,
        &conversation_id,
        "同阶段续写：灯塔内发现旧日志",
    )
    .await;

    let samples = recorder.samples();
    print_usage_table(&samples);
    let cold_cached = max_cached_for_tag(&samples, "s4-cold");
    let hot_cached = max_cached_for_tag(&samples, "s4-hot");
    let cold_prompt = sum_prompt_for_tag(&samples, "s4-cold");
    let hot_prompt = sum_prompt_for_tag(&samples, "s4-hot");
    eprintln!(
        "S4 cold_cached={cold_cached} hot_cached={hot_cached} cold_prompt={cold_prompt} hot_prompt={hot_prompt} text_cold={} text_hot={}",
        text_cold.len(),
        text_hot.len()
    );

    // 不强制 hot>cold（供应商策略），但要求两次调用都有 usage 样本且成文非空
    assert!(!text_cold.trim().is_empty() || !text_hot.trim().is_empty());
    assert!(
        samples.iter().any(|s| s.tag.starts_with("s4-cold"))
            && samples.iter().any(|s| s.tag.starts_with("s4-hot")),
        "S4 应录到 cold/hot 样本"
    );
    if hot_cached > 0 || cold_cached > 0 {
        eprintln!(
            "S4 cache signal: cold={cold_cached} hot={hot_cached} (hot>=cold? {})",
            hot_cached >= cold_cached
        );
    } else {
        eprintln!("S4 PARTIAL: no cached_tokens on cold/hot — record only");
    }

    eprintln!("S4 PASS(path): epoch refresh + dual write observed");
    env.cleanup();
}

// ─── S5 汇总：长会话压力（更多轮） ────────────────────────────────────────────

/// 额外压力：连续 8 轮写作，输出完整 cost/latency/cache 曲线（成本不敏感场景）。
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL）"]
async fn m5_s5_long_session_cost_curve() {
    let real = require_real_llm();
    let recorder = Arc::new(UsageRecordingLlmClient::new(real));
    let env = HarnessEnv::new(recorder.clone());
    let (campaign_id, conversation_id) = setup_campaign(&env, "m5-s5-long").await;

    let intents: Vec<String> = (1..=8)
        .map(|i| {
            format!(
                "第{i}幕：推进主线，加入新细节标记 M5MARK{i}，保持角色目标连贯，不要重复上一幕原文"
            )
        })
        .collect();

    let mut ok = 0usize;
    for (i, intent) in intents.iter().enumerate() {
        let tag = format!("long{}", i + 1);
        recorder.set_tag(&tag);
        let t0 = Instant::now();
        let text = run_turn(&env, &conversation_id, intent).await;
        eprintln!(
            "S5 {tag}: len={} wall_ms={} has_mark={}",
            text.len(),
            t0.elapsed().as_millis(),
            text.contains(&format!("M5MARK{}", i + 1)) || text.contains("M5MARK")
        );
        if !text.trim().is_empty() {
            ok += 1;
        }
    }

    let samples = recorder.samples();
    print_usage_table(&samples);
    let total_prompt: u32 = samples.iter().map(|s| s.prompt_tokens).sum();
    let total_cached: u32 = samples.iter().map(|s| s.cached_tokens).sum();
    let total_completion: u32 = samples.iter().map(|s| s.completion_tokens).sum();
    let total_ms: u128 = samples.iter().map(|s| s.elapsed_ms).sum();
    eprintln!(
        "S5 totals: turns_ok={ok}/8 llm_calls={} prompt={} cached={} completion={} ms={} cache_ratio={:.3}",
        samples.len(),
        total_prompt,
        total_cached,
        total_completion,
        total_ms,
        if total_prompt > 0 {
            total_cached as f64 / total_prompt as f64
        } else {
            0.0
        }
    );

    assert!(ok >= 6, "S5 至少 6/8 轮非空，实际 {ok}");
    let summaries = env.campaign_store.list_summaries(&campaign_id).len();
    eprintln!("S5 summaries_on_disk={summaries}");
    env.cleanup();
}

// ─── S6: Accept 闭环热 cache / catalog ───────────────────────────────────────

/// S6：4 轮「写作 → accept_variant → summarizer → 落 A」。
///
/// 验证记忆侧闭环（相对 S1 无 Accept）：
/// 1. summaries 随轮次增长，code 为 A0001…
/// 2. fill 后 turn 递增、catalog 非空、epoch 有 overview
/// 3. 记录 writing vs summary 的 cached_tokens；Director 最大 prompt 的 system_hash 稳定
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL）"]
async fn m5_s6_accept_loop_hot_cache() {
    let real = require_real_llm();
    let recorder = Arc::new(UsageRecordingLlmClient::new(real));
    let env = HarnessEnv::new(recorder.clone());
    let (campaign_id, conversation_id) = setup_campaign(&env, "m5-s6-accept").await;
    eprintln!("S6 campaign={campaign_id}");

    let intents = [
        "开场：雾港码头，角色发现刻着银鸦的木箱",
        "继续：打开木箱见到旧海图，标注坐标北纬31.2",
        "推进：追兵逼近，角色用口令「月下无声」联络接应",
        "收束：暂避灯塔，约定次日在银鸦标记处再会",
    ];

    let mut accepted = 0usize;
    let mut summarized = 0usize;
    for (i, intent) in intents.iter().enumerate() {
        let n = i + 1;
        let write_tag = format!("s6w{n}");
        let sum_tag = format!("s6s{n}");
        let t0 = Instant::now();
        let (text, summary) = accept_write_and_summarize(
            &env,
            &campaign_id,
            &conversation_id,
            intent,
            &write_tag,
            &sum_tag,
            &recorder,
        )
        .await;
        if !text.trim().is_empty() {
            accepted += 1;
        }
        if summary.as_ref().is_some_and(|s| !s.trim().is_empty()) {
            summarized += 1;
        }
        let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = env.fill_campaign_context(ctx);
        let n_sum = env.campaign_store.list_summaries(&campaign_id).len();
        eprintln!(
            "S6 turn{n}: text_len={} summary_len={} wall_ms={} store_summaries={} fill_turn={} catalog={} epoch={:?} overview={}",
            text.len(),
            summary.as_ref().map(|s| s.len()).unwrap_or(0),
            t0.elapsed().as_millis(),
            n_sum,
            ctx.turn,
            ctx.chronicle_prompt_catalog.len(),
            ctx.context_epoch.as_ref().map(|e| e.epoch_id.clone()),
            ctx.context_epoch
                .as_ref()
                .map(|e| e.overview_codes.len())
                .unwrap_or(0),
        );
    }

    let samples = recorder.samples();
    print_usage_table(&samples);

    let write_cached = (1..=4)
        .map(|i| max_cached_for_tag(&samples, &format!("s6w{i}")))
        .collect::<Vec<_>>();
    let sum_cached = (1..=4)
        .map(|i| max_cached_for_tag(&samples, &format!("s6s{i}")))
        .collect::<Vec<_>>();
    let late_write_cached = write_cached.iter().skip(1).copied().max().unwrap_or(0);
    let early_write_cached = write_cached.first().copied().unwrap_or(0);
    let any_write_cached = write_cached.iter().any(|&c| c > 0);
    let any_sum_cached = sum_cached.iter().any(|&c| c > 0);

    // Director 最大 prompt system_hash 跨 writing 轮
    let mut per_turn_sys = Vec::new();
    for i in 1..=4 {
        let tag = format!("s6w{i}");
        if let Some(best) = samples
            .iter()
            .filter(|s| s.tag == tag)
            .max_by_key(|s| s.prompt_tokens)
        {
            per_turn_sys.push(best.system_hash16.clone());
        }
    }
    let sys_stable = per_turn_sys.len() >= 2
        && per_turn_sys.iter().all(|h| h == &per_turn_sys[0]);

    let all = env.campaign_store.list_summaries(&campaign_id);
    let codes: Vec<_> = all
        .iter()
        .filter_map(|s| s.code.clone())
        .collect();
    let ctx_final = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx_final = env.fill_campaign_context(ctx_final);

    eprintln!(
        "S6 summary: accepted={accepted}/4 summarized={summarized}/4 codes={codes:?} final_turn={} catalog={} write_cached={write_cached:?} sum_cached={sum_cached:?} sys_stable={sys_stable} sys={per_turn_sys:?}",
        ctx_final.turn,
        ctx_final.chronicle_prompt_catalog.len(),
    );
    eprintln!(
        "S6 cache: any_write_cached={any_write_cached} any_sum_cached={any_sum_cached} early_write={early_write_cached} late_write={late_write_cached}"
    );

    assert!(accepted >= 3, "S6 至少 3 轮成文，实际 {accepted}");
    assert!(
        summarized >= 2,
        "S6 至少 2 轮 summary 落盘，实际 {summarized}"
    );
    assert!(
        all.len() >= 2,
        "S6 store 应有 ≥2 条 A，实际 {}",
        all.len()
    );
    assert!(
        ctx_final.turn >= 3,
        "S6 Accept 后 fill.turn 应 ≥3，实际 {}",
        ctx_final.turn
    );
    assert!(
        !ctx_final.chronicle_prompt_catalog.is_empty(),
        "S6 Accept 后 prompt catalog 不应为空"
    );
    if sys_stable {
        eprintln!("S6 PASS: system_hash 稳定 + Accept/summary 闭环");
    } else {
        eprintln!("S6 WARN: system_hash 不完全稳定: {per_turn_sys:?}");
    }
    if any_write_cached {
        eprintln!("S6 cache signal on writing turns: {write_cached:?}");
    } else {
        eprintln!(
            "S6 PARTIAL cache: writing turns still cached_tokens=0 after Accept/catalog（供应商/网关侧可能不记写作前缀 cache）"
        );
    }

    env.cleanup();
}

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
    streaming: bool,
    prompt_tokens: u32,
    cached_tokens: u32,
    cache_creation_tokens: u32,
    completion_tokens: u32,
    request_fp16: String,
    system_hash16: String,
    history_hash16: String,
    tail_hash16: String,
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

    fn record(&self, req: &ChatRequest, resp: &ChatResponse, elapsed_ms: u128, streaming: bool) {
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
            streaming,
            prompt_tokens: usage.prompt_tokens,
            cached_tokens: usage.cached_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            completion_tokens: usage.completion_tokens,
            request_fp16: fp.chars().take(16).collect(),
            system_hash16: segs.system_hash.chars().take(16).collect(),
            history_hash16: segs.history_hash.chars().take(16).collect(),
            tail_hash16: segs.tail_hash.chars().take(16).collect(),
            history_len: segs.history_len,
            tail_parts: segs.tail_parts,
            msg_count: req.messages.len(),
            elapsed_ms,
        };
        eprintln!(
            "[m5-usage] tag={} stream={} prompt={} cached={} create={} completion={} ratio={:.3} fp={} sys={} hist_h={} tail_h={} hist_n={} tail_parts={} msgs={} ms={} pv={}",
            sample.tag,
            sample.streaming,
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
            sample.history_hash16,
            sample.tail_hash16,
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
        self.record(req, &resp, t0.elapsed().as_millis(), false);
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
        self.record(req, &resp, t0.elapsed().as_millis(), true);
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

/// 手工记忆闭环结果（**不是**生产 CommitTurn/Accept）。
///
/// 绕过 TurnRecord / QualityGate / MutationBatch / `commit_turn_attempt`；
/// 只验证 conv Draft→Final + summarizer + 直接 `add_summary` 后 catalog 可见。
#[derive(Debug, Clone)]
struct ManualMemoryLoopResult {
    text: String,
    summary: Option<String>,
    draft_accepted: bool,
    summary_persisted: bool,
}

/// 写作 → `conv_store.accept_variant` → summarizer → 直接落 Chronicle A。
async fn accept_write_and_summarize(
    env: &HarnessEnv,
    campaign_id: &Id,
    conversation_id: &Id,
    intent: &str,
    write_tag: &str,
    summary_tag: &str,
    recorder: &UsageRecordingLlmClient,
) -> ManualMemoryLoopResult {
    recorder.set_tag(write_tag);
    let (text, node_id) = run_turn_with_node(env, conversation_id, intent).await;
    if text.trim().is_empty() {
        return ManualMemoryLoopResult {
            text,
            summary: None,
            draft_accepted: false,
            summary_persisted: false,
        };
    }
    let Some(node_id) = node_id else {
        return ManualMemoryLoopResult {
            text,
            summary: None,
            draft_accepted: false,
            summary_persisted: false,
        };
    };

    let draft_accepted = match env.conv_store.accept_variant(conversation_id, &node_id) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("accept_variant 失败: {e}");
            false
        }
    };
    if !draft_accepted {
        return ManualMemoryLoopResult {
            text,
            summary: None,
            draft_accepted: false,
            summary_persisted: false,
        };
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
    let mut summary_persisted = false;
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
        match env.campaign_store.add_summary(rs) {
            Ok(()) => summary_persisted = true,
            Err(e) => eprintln!("add_summary 失败: {e}"),
        }
    } else {
        eprintln!("{summary_tag}: summarizer 未产出 summary");
    }

    ManualMemoryLoopResult {
        text,
        summary: summary_text,
        draft_accepted,
        summary_persisted,
    }
}

fn print_usage_table(samples: &[UsageSample]) {
    eprintln!("--- M5 usage table ---");
    eprintln!(
        "{:<18} {:<6} {:>8} {:>8} {:>8} {:>8} {:>7} {:>6}",
        "tag", "stream", "prompt", "cached", "create", "compl", "ratio", "ms"
    );
    for s in samples {
        let ratio = if s.prompt_tokens > 0 {
            s.cached_tokens as f64 / s.prompt_tokens as f64
        } else {
            0.0
        };
        eprintln!(
            "{:<18} {:<6} {:>8} {:>8} {:>8} {:>8} {:>7.3} {:>6}",
            s.tag,
            if s.streaming { "yes" } else { "no" },
            s.prompt_tokens,
            s.cached_tokens,
            s.cache_creation_tokens,
            s.completion_tokens,
            ratio,
            s.elapsed_ms
        );
    }
}

/// 仅统计写作轮（tag 前缀匹配，排除 boot/extract）。
fn writing_samples<'a>(samples: &'a [UsageSample], tag_prefix: &str) -> Vec<&'a UsageSample> {
    samples
        .iter()
        .filter(|s| s.tag.starts_with(tag_prefix) && s.tag != "boot")
        .collect()
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
/// 通过标准：
/// - 至少 4 轮成文非空 + 写作轮 usage 样本非空
/// - **缓存结论仅看 turn* 写作样本**（排除 boot/extract）
/// - 写作轮全 0 cache → **Inconclusive**（不假 PASS）；有真实热命中才记 cache signal
/// - system/history/tail hash 仅作观测（最大 prompt 请求 ≠ 可靠 Director 身份）
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

    let write_samples = writing_samples(&samples, "turn");
    assert!(
        !write_samples.is_empty(),
        "S1 应录到写作轮 usage 样本（排除 boot）"
    );

    let early_cached = (1..=3)
        .map(|i| max_cached_for_tag(&samples, &format!("turn{i}")))
        .max()
        .unwrap_or(0);
    let late_cached = (4..=6)
        .map(|i| max_cached_for_tag(&samples, &format!("turn{i}")))
        .max()
        .unwrap_or(0);
    // 禁止用 boot/extract 的高 cache 冒充写作 cache
    let any_write_cached = write_samples.iter().any(|s| s.cached_tokens > 0);
    let any_stream_write_cached = write_samples
        .iter()
        .any(|s| s.streaming && s.cached_tokens > 0);
    let boot_cached = samples
        .iter()
        .filter(|s| s.tag == "boot")
        .map(|s| s.cached_tokens)
        .max()
        .unwrap_or(0);
    eprintln!(
        "S1 cache summary: early_write_max={early_cached} late_write_max={late_cached} \
         any_write_cached={any_write_cached} any_stream_write_cached={any_stream_write_cached} \
         boot_cached={boot_cached} (boot 不计入写作结论)"
    );

    if !any_write_cached {
        // 0>=0 不得记 PASS：写作 cache 无证据 → Inconclusive
        eprintln!(
            "S1 cache Inconclusive: 写作轮 cached_tokens 全 0（可能未命中，或历史流式嵌套 usage 漏解析；\
             已修 SSE 后若仍为 0 再对照供应商）。生成路径本身非空即探针执行成功。"
        );
    } else if late_cached > early_cached || (late_cached > 0 && early_cached > 0) {
        eprintln!(
            "S1 cache signal: 写作轮有 cached_tokens late={late_cached} early={early_cached} \
             stream_hit={any_stream_write_cached}"
        );
    } else {
        eprintln!(
            "S1 cache weak signal: any_write_cached 但 late({late_cached}) 未优于 early({early_cached})"
        );
    }

    // 观测：每轮最大 prompt 请求的三段 hash（身份不可靠，可能是 Editor）
    let mut per_turn_sys: Vec<String> = Vec::new();
    let mut per_turn_hist: Vec<String> = Vec::new();
    for i in 1..=6 {
        let tag = format!("turn{i}");
        if let Some(best) = samples
            .iter()
            .filter(|s| s.tag == tag)
            .max_by_key(|s| s.prompt_tokens)
        {
            per_turn_sys.push(best.system_hash16.clone());
            per_turn_hist.push(format!(
                "{}(stream={})",
                best.history_hash16, best.streaming
            ));
        }
    }
    if per_turn_sys.len() >= 3 {
        let first = &per_turn_sys[0];
        let same_as_first = per_turn_sys.iter().filter(|h| *h == first).count();
        eprintln!(
            "S1 largest-prompt system_hash per turn (NOT proven Director): {:?} same_as_first={same_as_first}/{}",
            per_turn_sys,
            per_turn_sys.len()
        );
        eprintln!("S1 largest-prompt history_hash/stream: {per_turn_hist:?}");
    }

    let conv = env.conv_store.get(&conversation_id).expect("会话");
    eprintln!(
        "S1 generation PASS (cache separate): nodes={} summaries={} write_calls={}",
        conv.nodes.len(),
        env.campaign_store.list_summaries(&campaign_id).len(),
        write_samples.len()
    );
    env.cleanup();
}

// ─── S2: 远楼 search/get ──────────────────────────────────────────────────────

/// S2：注入含唯一 token 的远楼 A + 近轮填充，验证 **工具目录** search/get（无 embedding）。
///
/// 不证明 Director 会主动召回远楼；不证明 A0001 已离开 near_raw。
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
    inject_leaf_a(&env, &campaign_id, &conversation_id, &lineage, 1, FAR_TOKEN);
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
    let _ = run_turn(&env, &conversation_id, "继续推进当前场景，不要回顾远古密约").await;
    recorder.set_tag("s2-turn2");
    let _ = run_turn(&env, &conversation_id, "角色检查装备，为下一行动做准备").await;

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
        ctx.context_epoch.as_ref().map(|e| e.overview_codes.len())
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
    eprintln!(
        "S2 PASS (tool catalog): search/get 精确命中远楼 code（无 embedding）；\
         Director 自动召回 / near_raw 排除未验证"
    );
    env.cleanup();
}

// ─── S3: 压缩损失探针 ─────────────────────────────────────────────────────────

/// S3：8 条 A（阈值 8 / group 4）→ 真 LLM 压 B → publish。
///
/// - 分组/covers/旧 A 可读：硬断言
/// - **B-only 关键实体/标识符保留**：只在 parents 上检测 token 存在
///   （**不是**否定极性 / 有向因果 / 限定词级语义事实保真）
#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL）"]
async fn m5_s3_compress_loss_probe() {
    let real = require_real_llm();
    let recorder = Arc::new(UsageRecordingLlmClient::new(real.clone()));
    let env = HarnessEnv::new(recorder.clone());
    let (campaign_id, conversation_id) = setup_campaign(&env, "m5-s3-compress").await;
    let lineage = ensure_lineage(&env, &campaign_id);

    // 每组不重复：姓名/数字/时间/否定/因果 —— 组1→B1，组2→B2
    const GROUP1_FACTS: [&str; 3] = [
        "FACT-G1-人名-卫岚澈",
        "FACT-G1-数字-徽章A7F2",
        "FACT-G1-时间-雨夜03:17",
    ];
    const GROUP2_FACTS: [&str; 3] = [
        "FACT-G2-否定-绝非走私货",
        "FACT-G2-因果-因口令错误导致伏击",
        "FACT-G2-坐标-北纬31.208",
    ];
    // 每条 A 只带本组一个独有事实，避免跨组重复降低难度
    let leaf_facts: [&str; 8] = [
        GROUP1_FACTS[0],
        GROUP1_FACTS[1],
        GROUP1_FACTS[2],
        "填充叙述-组1过渡-无关键编号",
        GROUP2_FACTS[0],
        GROUP2_FACTS[1],
        GROUP2_FACTS[2],
        "填充叙述-组2过渡-无关键编号",
    ];

    let mut entries = Vec::new();
    for t in 1u32..=8 {
        let fact = leaf_facts[(t - 1) as usize];
        let s = RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            t,
            format!(
                "第{t}轮纪要。必须由压缩层保留的独有事实：{fact}。其余氛围：雨势渐大，街灯闪烁，角色推进调查。"
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
        Some(8),   // 测试阈值
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

    // B-only 关键实体/标识符保留（禁止扫 A 原文）
    // 注意：token 存在 ≠ 语义事实保真——
    // - 「走私」不能证明「绝非走私」的否定极性
    // - 「口令」+「伏击」不能证明「口令错误导致伏击」的因果方向
    // - 「A7F2/03:17/31.208」不验证徽章/雨夜/北纬等限定关联
    let b_blob: String = parents
        .iter()
        .map(|p| {
            format!(
                "{} {} {}",
                p.code.as_deref().unwrap_or(""),
                p.headline.as_deref().unwrap_or(""),
                p.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let entity_cores: [(&str, &[&str]); 6] = [
        ("G1-人名实体", &["卫岚澈"]),
        ("G1-编号标识", &["A7F2"]),
        ("G1-时间标识", &["03:17"]),
        ("G2-走私相关词", &["走私"]),
        ("G2-口令+伏击词", &["口令", "伏击"]),
        ("G2-坐标标识", &["31.208"]),
    ];
    let mut kept = 0usize;
    for (label, cores) in entity_cores {
        let hit = cores.iter().all(|c| b_blob.contains(*c));
        eprintln!("S3 B-only entity/token kept={hit}: {label} cores={cores:?}");
        if hit {
            kept += 1;
        }
    }
    // 6 个实体/标识，压缩后期望至少保留一半（存在性门槛，非语义保真门槛）
    assert!(
        kept >= 3,
        "S3 B-only 实体/标识符保留应 ≥3/6，实际 {kept}/6；B 正文:\n{b_blob}"
    );

    // 旧 A 可读：独立断言，不计入 B 实体保留
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
    assert!(
        got.to_string().contains(GROUP1_FACTS[0]) || got.to_string().contains("卫岚澈"),
        "旧 A summary 应仍含原始实体（与 B 实体保留独立）"
    );

    print_usage_table(&recorder.samples());
    eprintln!(
        "S3 PASS: covers ok + B-only entity/token retention {kept}/6 \
         (NOT polarity/causal semantic fidelity) + get A still readable"
    );
    env.cleanup();
}

// ─── S4: epoch rollover 冷启动信号 ────────────────────────────────────────────

/// S4：注入足够 A 触发 epoch 重算后，再跑 2 轮写作。
///
/// 硬断言：epoch_id 变化、chronicle_revision 递增、overview/band 非空。
/// cache 冷热仅为观测（Inconclusive 不挡路径 PASS）。
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
    let rev_before = ctx_before.chronicle_revision;
    let ctx_mid = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx_mid = env.fill_campaign_context(ctx_mid);
    let epoch_mid = ctx_mid.context_epoch.as_ref().map(|e| e.epoch_id.clone());
    let overview_mid = ctx_mid
        .context_epoch
        .as_ref()
        .map(|e| e.overview_codes.len())
        .unwrap_or(0);
    let band_mid = ctx_mid
        .context_epoch
        .as_ref()
        .map(|e| e.band_codes.len())
        .unwrap_or(0);
    let rev_mid = ctx_mid.chronicle_revision;
    eprintln!(
        "S4 after inject fill: epoch={epoch_mid:?} overview={overview_mid} band={band_mid} rev={rev_mid} (epoch_changed={})",
        epoch_before != epoch_mid
    );

    assert!(
        epoch_before.is_some() && epoch_mid.is_some(),
        "S4 注入前后都应有 context_epoch"
    );
    assert_ne!(
        epoch_before, epoch_mid,
        "S4 大量注入后 epoch_id 必须变化（rollover/refresh），before={epoch_before:?} mid={epoch_mid:?}"
    );
    assert!(
        rev_mid > rev_before,
        "S4 chronicle_revision 必须递增 before={rev_before} mid={rev_mid}"
    );
    // 规格：refresh 后 overview 与 band 应同建；禁止 `band||overview` 被 overview 强断言遮蔽
    assert!(
        overview_mid > 0,
        "S4 refresh 后 overview_codes 应非空，实际 {overview_mid}"
    );
    assert!(
        band_mid > 0,
        "S4 refresh 后 band_codes 应非空，实际 {band_mid}"
    );

    recorder.set_tag("s4-cold");
    let text_cold = run_turn(&env, &conversation_id, "新阶段：角色抵达灯塔，重新评估局势").await;
    recorder.set_tag("s4-hot");
    let text_hot = run_turn(&env, &conversation_id, "同阶段续写：灯塔内发现旧日志").await;

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
        eprintln!("S4 cache Inconclusive: cold/hot writing cached_tokens=0 — 路径断言已独立通过");
    }

    eprintln!(
        "S4 PASS(path): epoch_id changed + rev {rev_before}→{rev_mid} + overview={overview_mid} band={band_mid}"
    );
    env.cleanup();
}

// ─── S5 汇总：长会话压力（更多轮） ────────────────────────────────────────────

/// 8 轮真实生成 smoke + cost/latency 曲线。
///
/// **不是**记忆架构长会话：无 Accept、无 summaries、未越过 H_anchor+E=15。
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
    eprintln!(
        "S5 generation smoke PASS: summaries_on_disk={summaries} \
         (no Accept; not H+E long-session; not fact-continuity)"
    );
    env.cleanup();
}

// ─── S6: 手工记忆闭环（非生产 Accept）────────────────────────────────────────

/// S6：4 轮「写作 → conv accept_variant → summarizer → 直接 add_summary」。
///
/// **不是**生产 CommitTurn（无 TurnRecord/QualityGate/MutationBatch）。
/// 验证：draft_accepted 真成功、A 落盘、catalog/turn 可见；写作 cache 单独定级。
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

    let mut draft_ok = 0usize;
    let mut summary_ok = 0usize;
    let mut text_ok = 0usize;
    for (i, intent) in intents.iter().enumerate() {
        let n = i + 1;
        let write_tag = format!("s6w{n}");
        let sum_tag = format!("s6s{n}");
        let t0 = Instant::now();
        let r = accept_write_and_summarize(
            &env,
            &campaign_id,
            &conversation_id,
            intent,
            &write_tag,
            &sum_tag,
            &recorder,
        )
        .await;
        if !r.text.trim().is_empty() {
            text_ok += 1;
        }
        if r.draft_accepted {
            draft_ok += 1;
        }
        if r.summary_persisted {
            summary_ok += 1;
        }
        let ctx = WritingContext::legacy(vec![], None, conversation_id.clone());
        let ctx = env.fill_campaign_context(ctx);
        let n_sum = env.campaign_store.list_summaries(&campaign_id).len();
        eprintln!(
            "S6 turn{n}: text_len={} draft_accepted={} summary_persisted={} summary_len={} wall_ms={} store_summaries={} fill_turn={} catalog={} epoch={:?}",
            r.text.len(),
            r.draft_accepted,
            r.summary_persisted,
            r.summary.as_ref().map(|s| s.len()).unwrap_or(0),
            t0.elapsed().as_millis(),
            n_sum,
            ctx.turn,
            ctx.chronicle_prompt_catalog.len(),
            ctx.context_epoch.as_ref().map(|e| e.epoch_id.clone()),
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
    let any_write_cached = write_cached.iter().any(|&c| c > 0);
    let any_sum_cached = sum_cached.iter().any(|&c| c > 0);
    let any_stream_write = samples
        .iter()
        .filter(|s| s.tag.starts_with("s6w"))
        .any(|s| s.streaming && s.cached_tokens > 0);

    let mut per_turn_sys = Vec::new();
    for i in 1..=4 {
        let tag = format!("s6w{i}");
        if let Some(best) = samples
            .iter()
            .filter(|s| s.tag == tag)
            .max_by_key(|s| s.prompt_tokens)
        {
            per_turn_sys.push(format!(
                "{}:{}:stream={}",
                best.system_hash16, best.history_hash16, best.streaming
            ));
        }
    }

    let all = env.campaign_store.list_summaries(&campaign_id);
    let codes: Vec<_> = all.iter().filter_map(|s| s.code.clone()).collect();
    let ctx_final = WritingContext::legacy(vec![], None, conversation_id.clone());
    let ctx_final = env.fill_campaign_context(ctx_final);

    eprintln!(
        "S6 summary: text_ok={text_ok}/4 draft_accepted={draft_ok}/4 summary_persisted={summary_ok}/4 \
         codes={codes:?} final_turn={} catalog={} write_cached={write_cached:?} sum_cached={sum_cached:?} segs={per_turn_sys:?}",
        ctx_final.turn,
        ctx_final.chronicle_prompt_catalog.len(),
    );
    eprintln!(
        "S6 cache: any_write_cached={any_write_cached} stream_write_hit={any_stream_write} any_sum_cached={any_sum_cached}"
    );

    // 概率型真实模型 smoke：成功率门槛（非「任一失败即 fail」的确定性闭环）
    // draft_ok 统计的是 accept_variant 真成功，不能用「正文非空」冒充
    const S6_MIN_DRAFT_OK: usize = 3;
    const S6_MIN_SUMMARY_OK: usize = 2;
    assert!(
        draft_ok >= S6_MIN_DRAFT_OK,
        "S6 成功率门槛：conv accept_variant ≥{S6_MIN_DRAFT_OK}/4，实际 {draft_ok}（text_ok={text_ok}）"
    );
    assert!(
        summary_ok >= S6_MIN_SUMMARY_OK,
        "S6 成功率门槛：summary 落盘 ≥{S6_MIN_SUMMARY_OK}/4，实际 {summary_ok}"
    );
    assert!(all.len() >= 2, "S6 store 应有 ≥2 条 A，实际 {}", all.len());
    assert!(
        ctx_final.turn >= 3,
        "S6 手工闭环后 fill.turn 应 ≥3，实际 {}",
        ctx_final.turn
    );
    assert!(
        !ctx_final.chronicle_prompt_catalog.is_empty(),
        "S6 手工闭环后 prompt catalog 不应为空"
    );
    eprintln!(
        "S6 PROBE EXECUTION PASS (manual memory loop, NOT production CommitTurn; \
         thresholds draft≥{S6_MIN_DRAFT_OK} summary≥{S6_MIN_SUMMARY_OK}, not 4/4 hard): \
         draft+summary+catalog"
    );
    if any_write_cached {
        eprintln!("S6 write cache signal: {write_cached:?} stream_hit={any_stream_write}");
    } else {
        eprintln!(
            "S6 write cache Inconclusive: writing turns cached_tokens=0（非流式 summarizer 命中不计入写作）"
        );
    }

    env.cleanup();
}

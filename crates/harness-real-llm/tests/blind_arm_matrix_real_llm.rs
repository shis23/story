//! Four-arm production pipeline evaluation with position-balanced blind judging.
//!
//! Run with an OpenAI-compatible endpoint:
//! ```text
//! $env:LLM_BASE_URL='https://open.bigmodel.cn/api/coding/paas/v4'
//! $env:LLM_API_KEY='<process-only secret>'
//! $env:LLM_MODEL='glm-5.2'
//! $env:STORYFORGE_LLM_TIMEOUT_SECS='600'
//! cargo test -p harness-real-llm --test blind_arm_matrix_real_llm -- --ignored --nocapture
//! ```
//!
//! The tracked evidence is privacy-safe: scores, arm identities, latency, usage,
//! and hashes only. Full generated text is written below ignored `artifacts/` for
//! local human review.
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use harness_real_llm::blind_arm_matrix::{
    ArmSample, BlindArm, JudgeVerdict, balanced_four_arm_order, majority_for_pair, summarize_matrix,
};
use harness_real_llm::{HarnessEnv, require_real_llm};
use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::{RegenerateRequest, WritingContext};
use storyforge_domain::Source;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::character::{Character, CharacterCard, CharacterDefinition, RoleType};
use storyforge_domain::generation::GenerationMode;
use storyforge_domain::llm::{
    ChatMessage, ChatRequest, ChatResponse, LlmError, SamplingParams, StreamChunk,
};
use storyforge_infra_llm::LlmClient;
use tokio::sync::{Semaphore, mpsc, watch};

const SEED: u64 = 20260727;
const JUDGE_ROUNDS: u8 = 4;

fn intents() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "i1_opening",
            "开场：夜雨初歇，四人在灯塔下第一次交谈，铺陈各自来意；必须让每个人都以可辨认的方式参与。",
        ),
        (
            "i2_conflict",
            "冲突升级：沉船旧案的关键证物被摆到明面，四方立场碰撞，不许和稀泥收场，也不许任何人凭空知道他人的秘密。",
        ),
        (
            "i3_ensemble",
            "群像推进：警铃突然响起，所有人必须在同一场景里作出相互衔接的行动与反应，人物声音要立得住。",
        ),
        (
            "i4_constraint",
            "回收伏笔：灯塔停摆的原因被触及，但本轮不许彻底揭开谜底；结尾必须形成下一轮可行动的新局面。",
        ),
    ]
}

fn fixture_character(id: &str, name: &str, persona: &str) -> Character {
    Character {
        id: storyforge_domain::Id::from_str(id),
        name: name.into(),
        description: persona.into(),
        personality: persona.into(),
        scenario: "海崖上的老灯塔，守灯人世代相传；灯已停摆三夜。".into(),
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
    }
}

fn fixture_card() -> CharacterCard {
    let characters = [
        fixture_character(
            "blind-keeper",
            "沈砚",
            "老守灯人，沉默寡言，守着灯塔与三十年前的沉船旧事；绝不主动提旧案。",
        ),
        fixture_character(
            "blind-investigator",
            "闻笙",
            "年轻海事调查员，为沉船旧案而来；敏锐、执拗，不肯空手而归。",
        ),
        fixture_character(
            "blind-smuggler",
            "贺川",
            "熟悉暗礁的走私客，外表轻佻，真正目的在找回沉船上的账册。",
        ),
        fixture_character(
            "blind-doctor",
            "苏棠",
            "岛上医生，克制冷静，知道当年伤员名单有一处被人为涂改。",
        ),
    ];
    let mut card = CharacterCard::from_character(&characters[0]);
    let definitions = characters
        .iter()
        .enumerate()
        .map(|(index, character)| {
            let mut definition = CharacterDefinition::fallback_from_character(character, &[]);
            definition.role_type = if index == 0 {
                RoleType::Protagonist
            } else {
                RoleType::Supporting
            };
            definition
        })
        .collect();
    card.character_definitions =
        storyforge_app_agent::attach_definitions_to_card(definitions, &card.id);
    card
}

fn sha16(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn artifacts_dir() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("artifacts")
        .join("blind-ab");
    std::fs::create_dir_all(dir.join("texts")).expect("create blind evaluation directory");
    dir
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
struct UsageTotals {
    calls: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
}

impl UsageTotals {
    fn delta(self, before: Self) -> Self {
        Self {
            calls: self.calls.saturating_sub(before.calls),
            prompt_tokens: self.prompt_tokens.saturating_sub(before.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_sub(before.completion_tokens),
        }
    }
}

/// Records provider usage and serializes outbound dispatches. The production
/// big-scene arm still schedules subagents concurrently, but this harness avoids
/// turning a shared evaluation endpoint into an accidental load test.
struct RecordingLlmClient {
    inner: Arc<dyn LlmClient>,
    dispatch_gate: Semaphore,
    calls: AtomicU64,
    tokens: Mutex<(u64, u64)>,
}

impl RecordingLlmClient {
    fn new(inner: Arc<dyn LlmClient>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            dispatch_gate: Semaphore::new(1),
            calls: AtomicU64::new(0),
            tokens: Mutex::new((0, 0)),
        })
    }

    fn snapshot(&self) -> UsageTotals {
        let (prompt_tokens, completion_tokens) = *self.tokens.lock().expect("usage lock");
        UsageTotals {
            calls: self.calls.load(Ordering::SeqCst),
            prompt_tokens,
            completion_tokens,
        }
    }

    fn record(&self, response: &ChatResponse) {
        if let Some(usage) = response.usage.as_ref() {
            let mut totals = self.tokens.lock().expect("usage lock");
            totals.0 += u64::from(usage.prompt_tokens);
            totals.1 += u64::from(usage.completion_tokens);
        }
    }
}

#[async_trait]
impl LlmClient for RecordingLlmClient {
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let _permit = self
            .dispatch_gate
            .acquire()
            .await
            .map_err(|_| LlmError::Internal("evaluation dispatch gate closed".into()))?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.inner.chat(request).await?;
        self.record(&response);
        Ok(response)
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, LlmError> {
        let _permit = self
            .dispatch_gate
            .acquire()
            .await
            .map_err(|_| LlmError::Internal("evaluation dispatch gate closed".into()))?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.inner.chat_stream(request, tx, cancel).await?;
        self.record(&response);
        Ok(response)
    }
}

fn mode_for_arm(arm: BlindArm) -> GenerationMode {
    match arm {
        BlindArm::SoloWriter => GenerationMode::Continuation,
        BlindArm::ParallelCrew => GenerationMode::BigScene,
        BlindArm::SequentialCrew => GenerationMode::SequentialCrew,
        BlindArm::DuetMerge => GenerationMode::Duet,
    }
}

struct ArmRun {
    text: String,
    latency_ms: u128,
    usage: UsageTotals,
}

async fn run_arm(
    env: &HarnessEnv,
    recorder: &RecordingLlmClient,
    arm: BlindArm,
    intent: &str,
) -> Result<ArmRun, String> {
    let conversation = env.conv_store.create(None, None);
    let context = env.fill_campaign_context(WritingContext::legacy(vec![], None, conversation.id));
    let before = recorder.snapshot();
    let started = std::time::Instant::now();
    let mut pipeline = env.new_pipeline();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let (text, _node_id, provenance) = pipeline
        .start_writing_with_mode(
            intent.to_string(),
            &context,
            mode_for_arm(arm),
            event_tx,
            cancel_rx,
        )
        .await
        .map_err(|error| format!("{} failed: {error}", arm.as_str()))?;
    let provenance =
        provenance.ok_or_else(|| format!("{} returned no provenance", arm.as_str()))?;
    if provenance.generation_mode != Some(mode_for_arm(arm)) {
        return Err(format!("{} returned wrong provenance mode", arm.as_str()));
    }
    Ok(ArmRun {
        text,
        latency_ms: started.elapsed().as_millis(),
        usage: recorder.snapshot().delta(before),
    })
}

#[derive(Debug, serde::Deserialize)]
struct JudgeOut {
    ranking: Vec<String>,
    scores: BTreeMap<String, [u8; 4]>,
}

fn validate_judge_output(output: &JudgeOut) -> bool {
    let expected = ["A", "B", "C", "D"];
    output.ranking.len() == expected.len()
        && expected
            .iter()
            .all(|label| output.ranking.iter().any(|ranked| ranked == label))
        && output.scores.len() == expected.len()
        && expected.iter().all(|label| {
            output
                .scores
                .get(*label)
                .is_some_and(|scores| scores.iter().all(|score| (1..=10).contains(score)))
        })
}

async fn judge_once(
    env: &HarnessEnv,
    intent: &str,
    ordered_texts: &[&str; 4],
) -> Result<JudgeOut, String> {
    let system = "你是严格的中文小说评审。比较四段响应同一指令的叙事文本。按四项分别打 1-10 分：角色一致性与人物区分、情节推进、文风质量、约束遵守。篇幅更长本身不是优点。你不知道文本来源，不得猜测生成方法。只输出 JSON：{\"ranking\":[\"A\",\"B\",\"C\",\"D\"],\"scores\":{\"A\":[四个整数],\"B\":[四个整数],\"C\":[四个整数],\"D\":[四个整数]}}。ranking 从最好到最差且四个标签各出现一次。";
    let user = format!(
        "写作指令：{intent}\n\n【文本 A】\n{}\n\n【文本 B】\n{}\n\n【文本 C】\n{}\n\n【文本 D】\n{}\n\n只输出 JSON。",
        ordered_texts[0], ordered_texts[1], ordered_texts[2], ordered_texts[3]
    );
    for _attempt in 0..3 {
        let request = ChatRequest {
            messages: vec![
                ChatMessage::system(system.to_string()),
                ChatMessage::user(user.clone()),
            ],
            tools: None,
            params: SamplingParams {
                temperature: Some(0.2),
                max_tokens: Some(4096),
                max_tokens_explicit: true,
                ..Default::default()
            },
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "glm-5.2".into()),
        };
        let response = env
            .llm
            .chat(&request)
            .await
            .map_err(|error| format!("judge request failed: {error}"))?;
        let text = response.content.trim();
        let json = match (text.find('{'), text.rfind('}')) {
            (Some(start), Some(end)) if end > start => &text[start..=end],
            _ => continue,
        };
        if let Ok(output) = serde_json::from_str::<JudgeOut>(json)
            && validate_judge_output(&output)
        {
            return Ok(output);
        }
    }
    Err("judge returned invalid JSON three times".into())
}

#[derive(Debug, serde::Serialize)]
struct JudgeRoundEvidence {
    intent_id: String,
    round: u8,
    presentation_order: Vec<String>,
    ranking: Vec<String>,
    scores: BTreeMap<String, [u8; 4]>,
}

fn arm_pairs() -> Vec<(BlindArm, BlindArm)> {
    let arms = BlindArm::all();
    let mut pairs = Vec::new();
    for left_index in 0..arms.len() {
        for right_index in (left_index + 1)..arms.len() {
            pairs.push((arms[left_index], arms[right_index]));
        }
    }
    pairs
}

async fn sequential_suffix_replay_acceptance(
    env: &HarnessEnv,
    recorder: &RecordingLlmClient,
) -> Result<serde_json::Value, String> {
    let conversation = env.conv_store.create(None, None);
    let context = env.fill_campaign_context(WritingContext::legacy(
        vec![],
        None,
        conversation.id.clone(),
    ));
    let mut pipeline = env.new_pipeline();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let (_text, node_id, before_provenance) = pipeline
        .start_writing_with_mode(
            "群像验收：四人围绕失踪账册依次表态和行动，每个人都必须参与。".into(),
            &context,
            GenerationMode::SequentialCrew,
            event_tx,
            cancel_rx,
        )
        .await
        .map_err(|error| format!("sequential setup failed: {error}"))?;
    let before_provenance =
        before_provenance.ok_or_else(|| "sequential setup returned no provenance".to_string())?;
    if before_provenance.subagent_results.len() < 3 {
        return Err(format!(
            "sequential setup produced only {} actor performances",
            before_provenance.subagent_results.len()
        ));
    }
    let target_index = 1usize;
    let target = before_provenance.subagent_results[target_index]
        .character_id
        .clone();
    let prefix_hash = sha16(&before_provenance.subagent_results[0].full_text);
    let before_usage = recorder.snapshot();
    let request = RegenerateRequest {
        conversation_id: conversation.id,
        node_id,
        targets: vec![PartialRollTarget::Subagent(target.clone())],
        generation_mode: Some(GenerationMode::SequentialCrew),
        hint: Some("让目标角色的回应更尖锐，并保持后续角色接戏连贯".into()),
        seed: Some(SEED + 1),
    };
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let (replayed_text, after_provenance) = pipeline
        .regenerate(request, &context, event_tx, cancel_rx)
        .await
        .map_err(|error| format!("sequential suffix replay failed: {error}"))?;

    let mut replayed_indices = Vec::new();
    let mut director_restarted = false;
    while let Ok(event) = event_rx.try_recv() {
        match event {
            PipelineEvent::SubagentStarted { index, .. } => replayed_indices.push(index),
            PipelineEvent::DirectorStarted => director_restarted = true,
            _ => {}
        }
    }
    let prefix_preserved = after_provenance
        .subagent_results
        .first()
        .is_some_and(|snapshot| sha16(&snapshot.full_text) == prefix_hash);
    if !prefix_preserved
        || director_restarted
        || replayed_indices.first().copied() != Some(target_index)
        || after_provenance.generation_mode != Some(GenerationMode::SequentialCrew)
        || replayed_text.trim().is_empty()
    {
        return Err(format!(
            "suffix replay invariant failed: prefix={prefix_preserved}, director={director_restarted}, indices={replayed_indices:?}"
        ));
    }
    Ok(serde_json::json!({
        "target_character_id": target,
        "target_index": target_index,
        "actor_count": after_provenance.subagent_results.len(),
        "prefix_preserved": prefix_preserved,
        "director_restarted": director_restarted,
        "replayed_indices": replayed_indices,
        "result_fingerprint16": sha16(&replayed_text),
        "usage": recorder.snapshot().delta(before_usage),
    }))
}

#[tokio::test]
#[ignore = "requires a real active LLM connection"]
async fn provider_connection_and_native_tool_probe() {
    use storyforge_domain::llm::ToolSpec;

    let llm = require_real_llm();
    let request = ChatRequest {
        messages: vec![ChatMessage::user(
            "调用 echo_text 工具，参数 text 必须是 OK。不要直接回答。",
        )],
        tools: Some(vec![ToolSpec::function(
            "echo_text",
            "Echo text",
            serde_json::json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        )]),
        params: SamplingParams {
            temperature: Some(0.0),
            max_tokens: Some(1024),
            max_tokens_explicit: true,
            ..Default::default()
        },
        model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-chat".into()),
    };
    let response = llm.chat(&request).await.expect("real provider probe");
    let call = response
        .tool_calls
        .first()
        .expect("provider must return a native tool call");
    assert_eq!(call.function.name, "echo_text");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&call.function.arguments).unwrap()["text"],
        "OK"
    );
}

#[tokio::test]
#[ignore = "requires real LLM credentials"]
async fn blind_four_arm_generation_and_optional_self_judging() {
    let recorder = RecordingLlmClient::new(require_real_llm());
    let llm: Arc<dyn LlmClient> = recorder.clone();
    let env = HarnessEnv::new(llm);
    let dir = artifacts_dir();

    let stored = env.campaign_store.save_card(fixture_card()).unwrap();
    let campaign_id = env.create_campaign(&stored.card, "blind-four-arm-campaign");
    eprintln!("fixture campaign: {campaign_id}");

    let mut samples = Vec::<ArmSample>::new();
    let mut verdicts = Vec::<JudgeVerdict>::new();
    let mut judge_rounds = Vec::<JudgeRoundEvidence>::new();
    let mut pair_majorities = Vec::new();
    let self_judge = std::env::var("STORYFORGE_BLIND_SELF_JUDGE")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "yes"));

    for (intent_id, intent) in intents() {
        let mut texts = HashMap::<BlindArm, String>::new();
        for arm in BlindArm::all() {
            eprintln!("[{intent_id}] generating {}", arm.as_str());
            let run = run_arm(&env, &recorder, arm, intent)
                .await
                .unwrap_or_else(|error| panic!("{error}"));
            assert!(
                !run.text.trim().is_empty(),
                "{intent_id}: empty {}",
                arm.as_str()
            );
            std::fs::write(
                dir.join("texts")
                    .join(format!("{intent_id}_{}.txt", arm.as_str())),
                &run.text,
            )
            .unwrap();
            samples.push(ArmSample {
                arm,
                intent_id: intent_id.into(),
                text_chars: run.text.chars().count(),
                latency_ms: run.latency_ms,
                prompt_tokens: run.usage.prompt_tokens,
                completion_tokens: run.usage.completion_tokens,
                text_fingerprint16: sha16(&run.text),
            });
            texts.insert(arm, run.text);
        }

        for round in 0..if self_judge { JUDGE_ROUNDS } else { 0 } {
            let order = balanced_four_arm_order(SEED, intent_id, round);
            let ordered_texts = order.map(|arm| texts.get(&arm).unwrap().as_str());
            let output = judge_once(&env, intent, &ordered_texts)
                .await
                .unwrap_or_else(|error| panic!("{intent_id} judge round {round}: {error}"));
            let label_to_arm = ["A", "B", "C", "D"]
                .into_iter()
                .zip(order)
                .collect::<HashMap<_, _>>();
            let rank_by_arm = output
                .ranking
                .iter()
                .enumerate()
                .map(|(rank, label)| (label_to_arm[label.as_str()], rank))
                .collect::<HashMap<_, _>>();
            let score_by_arm = ["A", "B", "C", "D"]
                .into_iter()
                .map(|label| (label_to_arm[label], output.scores[label]))
                .collect::<HashMap<_, _>>();

            for (left, right) in arm_pairs() {
                let winner = if rank_by_arm[&left] < rank_by_arm[&right] {
                    left
                } else {
                    right
                };
                let left_position = order.iter().position(|arm| *arm == left).unwrap();
                let right_position = order.iter().position(|arm| *arm == right).unwrap();
                let (first_arm, second_arm) = if left_position < right_position {
                    (left, right)
                } else {
                    (right, left)
                };
                verdicts.push(JudgeVerdict {
                    intent_id: intent_id.into(),
                    first_arm,
                    second_arm,
                    winner,
                    scores_first: score_by_arm[&first_arm],
                    scores_second: score_by_arm[&second_arm],
                    judge_round: round,
                });
            }
            judge_rounds.push(JudgeRoundEvidence {
                intent_id: intent_id.into(),
                round,
                presentation_order: order
                    .into_iter()
                    .map(|arm| arm.as_str().to_string())
                    .collect(),
                ranking: output.ranking,
                scores: output.scores,
            });
        }

        if self_judge {
            for (left, right) in arm_pairs() {
                pair_majorities.push(majority_for_pair(intent_id, left, right, &verdicts));
            }
        }
    }

    let summary = summarize_matrix(&pair_majorities, &samples, JUDGE_ROUNDS);
    assert!(summary.absent_arms.is_empty(), "all four arms must run");
    let suffix_replay = sequential_suffix_replay_acceptance(&env, &recorder)
        .await
        .expect("real sequential suffix replay acceptance");
    let evidence = serde_json::json!({
        "schema_version": "blind-four-arm-v2",
        "seed": SEED,
        "model": std::env::var("LLM_MODEL").unwrap_or_default(),
        "self_judge_enabled": self_judge,
        "judge_protocol": if self_judge {
            "provider-self-judge-four-way-latin-square-position-balanced"
        } else {
            "external-independent-panel-required"
        },
        "generated_intents": intents().len(),
        "samples": samples,
        "judge_rounds": judge_rounds,
        "pair_verdicts": verdicts,
        "pair_majorities": pair_majorities,
        "summary": summary,
        "sequential_suffix_replay": suffix_replay,
        "total_provider_usage": recorder.snapshot(),
    });
    let output_path = dir.join("blind-four-arm-summary.json");
    std::fs::write(
        &output_path,
        serde_json::to_string_pretty(&evidence).unwrap(),
    )
    .unwrap();
    eprintln!(
        "four-arm summary:\n{}\nevidence: {}",
        serde_json::to_string_pretty(&summary).unwrap(),
        output_path.display()
    );

    env.cleanup();
}

#[tokio::test]
#[ignore = "requires real LLM credentials"]
async fn sequential_suffix_replay_real_llm() {
    let recorder = RecordingLlmClient::new(require_real_llm());
    let llm: Arc<dyn LlmClient> = recorder.clone();
    let env = HarnessEnv::new(llm);
    let stored = env.campaign_store.save_card(fixture_card()).unwrap();
    env.create_campaign(&stored.card, "sequential-suffix-replay-campaign");

    let evidence = sequential_suffix_replay_acceptance(&env, &recorder)
        .await
        .expect("real sequential suffix replay acceptance");
    let output_path = artifacts_dir().join("sequential-suffix-replay-summary.json");
    std::fs::write(
        &output_path,
        serde_json::to_string_pretty(&evidence).unwrap(),
    )
    .unwrap();
    eprintln!("suffix replay evidence: {}", output_path.display());
    env.cleanup();
}

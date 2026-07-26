//! 四臂盲测——首轮两臂实测（solo_writer vs parallel_crew，真实 LLM）。
//!
//! 设计与判读口径：docs/workstreams/BLIND-AB-PIPELINE-PLAN.md（预注册）；
//! 骨架：harness_real_llm::blind_arm_matrix。sequential_crew/duet_merge
//! 未实现，报告 absent_arms 显式列出。
//!
//! 运行方式：
//! ```text
//! $env:LLM_BASE_URL='https://cli.2529985.xyz/v1'
//! $env:LLM_API_KEY='<key>'                 # 只从环境读取
//! $env:LLM_MODEL='deepseek-v4-pro'
//! $env:STORYFORGE_LLM_TIMEOUT_SECS='600'
//! cargo test -p harness-real-llm --test blind_arm_matrix_real_llm -- --ignored --nocapture
//! ```
//! 证据（脱敏：分数/胜负/成本/指纹，无正文）写 `artifacts/blind-ab/`；
//! 正文样本写同目录 `texts/`（目录整体 gitignore，人工复核用）。

use harness_real_llm::blind_arm_matrix::{
    ArmSample, BlindArm, JudgeVerdict, majority_for_pair, presentation_order, summarize_matrix,
};
use harness_real_llm::{HarnessEnv, require_real_llm};
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Source;
use storyforge_domain::character::{Character, CharacterCard, CharacterDefinition, RoleType};
use storyforge_domain::llm::{ChatMessage, ChatRequest, SamplingParams};
use tokio::sync::{mpsc, watch};

const SEED: u64 = 20260727;
const JUDGE_ROUNDS: u8 = 3;

fn intents() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "i1_opening",
            "开场：夜雨初歇，两人在灯塔下初次交谈，铺陈各自来意。",
        ),
        (
            "i2_conflict",
            "冲突升级：矛盾摊到明面，双方立场碰撞，不许和稀泥收场。",
        ),
        (
            "i3_ensemble",
            "两人同场：一段有来有回的长对话，各自的声音要立得住。",
        ),
        (
            "i4_constraint",
            "回收伏笔：灯塔停摆的原因被触及，但本轮不许彻底揭开谜底。",
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

/// 双角色 fixture 卡：守灯人 + 来查旧案的访客（对手戏张力面）。
fn fixture_card() -> CharacterCard {
    let keeper = fixture_character(
        "blind-keeper",
        "沈磐",
        "老守灯人，沉默寡言，守着灯塔与一桩三十年前的沉船旧事；绝不主动提旧案。",
    );
    let visitor = fixture_character(
        "blind-visitor",
        "闻笛",
        "年轻的海事调查员，为三十年前沉船旧案而来，敏锐、执拗、不肯空手而归。",
    );
    let mut card = CharacterCard::from_character(&keeper);
    let mut def_a = CharacterDefinition::fallback_from_character(&keeper, &[]);
    def_a.role_type = RoleType::Protagonist;
    let mut def_b = CharacterDefinition::fallback_from_character(&visitor, &[]);
    def_b.name = "闻笛".into();
    def_b.persona_prompt = visitor.personality.clone();
    def_b.role_type = RoleType::Supporting;
    card.character_definitions =
        storyforge_app_agent::attach_definitions_to_card(vec![def_a, def_b], &card.id);
    card
}

fn sha16(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let out = hasher.finalize();
    out.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn artifacts_dir() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("artifacts")
        .join("blind-ab");
    std::fs::create_dir_all(dir.join("texts")).expect("建证据目录失败");
    dir
}

/// solo_writer 臂：同源 campaign 状态拼单条提示直写（续写档原型的最简形态）。
async fn run_solo_writer(
    env: &HarnessEnv,
    ctx: &WritingContext,
    intent: &str,
) -> Result<(String, u128, u64, u64), String> {
    let runtime = ctx
        .campaign_runtime
        .as_ref()
        .ok_or("solo 臂需要 campaign_runtime")?;
    let mut roster = String::new();
    for inst in &runtime.instances {
        let def = runtime.definition_for_instance(inst);
        roster.push_str(&format!(
            "### {}\n人设：{}\n行为准则：{}\n",
            inst.name,
            inst.resolved_persona(def).unwrap_or("（无）"),
            inst.resolved_behavior(def).unwrap_or("（无）"),
        ));
    }
    let system = format!(
        "你是一位单人执笔的小说家，独立完成整段叙事。\n\
         场景：{}\n\n## 在场角色\n{roster}\n\
         ## 写作要求\n- 用中文写一段完整的小说叙事（600-1000 字）\n\
         - 每个角色的声音与知识边界要立得住；不代替用户行动\n\
         - 只输出正文，不要任何解释或标题",
        "海崖上的老灯塔，守灯人世代相传；灯已停摆三夜。"
    );
    let req = ChatRequest {
        messages: vec![
            ChatMessage::system(system),
            ChatMessage::user(intent.to_string()),
        ],
        tools: None,
        params: SamplingParams {
            temperature: Some(0.7),
            max_tokens: Some(8192),
            max_tokens_explicit: true,
            ..Default::default()
        },
        model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".into()),
    };
    let t0 = std::time::Instant::now();
    let resp = env
        .llm
        .chat(&req)
        .await
        .map_err(|e| format!("solo 臂调用失败: {e}"))?;
    let usage = resp.usage.clone().unwrap_or_default();
    Ok((
        resp.content,
        t0.elapsed().as_millis(),
        u64::from(usage.prompt_tokens),
        u64::from(usage.completion_tokens),
    ))
}

/// parallel_crew 臂：当前生产平行流水线。
async fn run_parallel_crew(
    env: &HarnessEnv,
    ctx: &WritingContext,
    intent: &str,
) -> Result<(String, u128), String> {
    let mut pipeline = env.new_pipeline();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let t0 = std::time::Instant::now();
    let (text, _node, _prov) = pipeline
        .start_writing(intent.to_string(), ctx, event_tx, cancel_rx)
        .await
        .map_err(|e| format!("parallel 臂失败: {e}"))?;
    Ok((text, t0.elapsed().as_millis()))
}

#[derive(serde::Deserialize)]
struct JudgeOut {
    winner: String,
    scores_first: [u8; 4],
    scores_second: [u8; 4],
}

/// 裁判一轮（双盲：只见"文本甲/文本乙"）。解析失败重试一次。
async fn judge_once(
    env: &HarnessEnv,
    intent: &str,
    first_text: &str,
    second_text: &str,
) -> Result<JudgeOut, String> {
    let system = "你是严格的小说评审。对比两段针对同一写作指令的叙事文本，按四个维度各打 1-10 分：\
        ①角色一致性（人设/声音/知识边界，多角色时人物区分度）②情节推进（是否响应指令、有效事件密度）\
        ③文风质量（叙事张力、重复率、AI 腔；注意：篇幅长短本身不是优点）④约束遵守（不代打用户、不出戏、格式干净）。\
        只输出 JSON：{\"winner\":\"甲\"或\"乙\",\"scores_first\":[4个整数],\"scores_second\":[4个整数]}，无其他文字。";
    let user = format!(
        "写作指令：{intent}\n\n【文本甲】\n{first_text}\n\n【文本乙】\n{second_text}\n\n请评审并输出 JSON。"
    );
    for _attempt in 0..2 {
        let req = ChatRequest {
            messages: vec![
                ChatMessage::system(system.to_string()),
                ChatMessage::user(user.clone()),
            ],
            tools: None,
            params: SamplingParams {
                temperature: Some(0.2),
                max_tokens: Some(2048),
                max_tokens_explicit: true,
                ..Default::default()
            },
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".into()),
        };
        let resp = env
            .llm
            .chat(&req)
            .await
            .map_err(|e| format!("裁判调用失败: {e}"))?;
        let text = resp.content.trim();
        let slice = match (text.find('{'), text.rfind('}')) {
            (Some(a), Some(b)) if b > a => &text[a..=b],
            _ => continue,
        };
        if let Ok(out) = serde_json::from_str::<JudgeOut>(slice)
            && (out.winner == "甲" || out.winner == "乙")
        {
            return Ok(out);
        }
    }
    Err("裁判两次输出均不可解析".into())
}

#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/LLM_API_KEY/LLM_MODEL）"]
async fn blind_two_arm_matrix_with_blind_judging() {
    let llm = require_real_llm();
    let env = HarnessEnv::new(llm);
    let dir = artifacts_dir();

    // fixture campaign（双角色，无外部卡文件依赖——异机可复现）
    let stored = env.campaign_store.save_card(fixture_card()).unwrap();
    let campaign_id = env.create_campaign(&stored.card, "blind-ab-campaign");
    eprintln!("fixture campaign: {campaign_id}");

    let mut samples: Vec<ArmSample> = Vec::new();
    let mut verdicts: Vec<JudgeVerdict> = Vec::new();
    let mut pairs = Vec::new();

    for (intent_id, intent) in intents() {
        // 每意图独立会话（同一轮初状态，不互相续写）
        let conv_solo = env.conv_store.create(None, None).id;
        let ctx_solo = env.fill_campaign_context(WritingContext::legacy(vec![], None, conv_solo));
        let (solo_text, solo_ms, solo_pt, solo_ct) = run_solo_writer(&env, &ctx_solo, intent)
            .await
            .expect("solo 臂成功");

        let conv_crew = env.conv_store.create(None, None).id;
        let ctx_crew = env.fill_campaign_context(WritingContext::legacy(vec![], None, conv_crew));
        let (crew_text, crew_ms) = run_parallel_crew(&env, &ctx_crew, intent)
            .await
            .expect("parallel 臂成功");

        assert!(!solo_text.trim().is_empty(), "{intent_id}: solo 正文为空");
        assert!(!crew_text.trim().is_empty(), "{intent_id}: crew 正文为空");

        // 正文样本（人工复核用，gitignore 目录）
        std::fs::write(
            dir.join("texts").join(format!("{intent_id}_solo.txt")),
            &solo_text,
        )
        .unwrap();
        std::fs::write(
            dir.join("texts").join(format!("{intent_id}_crew.txt")),
            &crew_text,
        )
        .unwrap();

        samples.push(ArmSample {
            arm: BlindArm::SoloWriter,
            intent_id: intent_id.into(),
            text_chars: solo_text.chars().count(),
            latency_ms: solo_ms,
            prompt_tokens: solo_pt,
            completion_tokens: solo_ct,
            text_fingerprint16: sha16(&solo_text),
        });
        samples.push(ArmSample {
            arm: BlindArm::ParallelCrew,
            intent_id: intent_id.into(),
            text_chars: crew_text.chars().count(),
            latency_ms: crew_ms,
            prompt_tokens: 0, // pipeline usage 聚合不在此路径暴露；成本以墙钟对比
            completion_tokens: 0,
            text_fingerprint16: sha16(&crew_text),
        });

        // 裁判 3 轮，双盲乱序
        for round in 0..JUDGE_ROUNDS {
            let (first_arm, second_arm) = presentation_order(
                SEED,
                intent_id,
                round,
                BlindArm::SoloWriter,
                BlindArm::ParallelCrew,
            );
            let (first_text, second_text) = if first_arm == BlindArm::SoloWriter {
                (&solo_text, &crew_text)
            } else {
                (&crew_text, &solo_text)
            };
            match judge_once(&env, intent, first_text, second_text).await {
                Ok(out) => {
                    let winner = if out.winner == "甲" {
                        first_arm
                    } else {
                        second_arm
                    };
                    eprintln!(
                        "[{intent_id} r{round}] 甲={} 乙={} → 胜者 {}",
                        first_arm.as_str(),
                        second_arm.as_str(),
                        winner.as_str()
                    );
                    verdicts.push(JudgeVerdict {
                        intent_id: intent_id.into(),
                        first_arm,
                        second_arm,
                        winner,
                        scores_first: out.scores_first,
                        scores_second: out.scores_second,
                        judge_round: round,
                    });
                }
                Err(e) => eprintln!("[{intent_id} r{round}] 裁判失败（如实弃权）: {e}"),
            }
        }
        pairs.push(majority_for_pair(
            intent_id,
            BlindArm::SoloWriter,
            BlindArm::ParallelCrew,
            &verdicts,
        ));
    }

    let summary = summarize_matrix(&pairs, &samples, JUDGE_ROUNDS);
    let evidence = serde_json::json!({
        "seed": SEED,
        "model": std::env::var("LLM_MODEL").unwrap_or_default(),
        "samples": samples,
        "verdicts": verdicts,
        "pairs": pairs,
        "summary": summary,
    });
    let out_path = dir.join("blind-two-arm-summary.json");
    std::fs::write(&out_path, serde_json::to_string_pretty(&evidence).unwrap()).unwrap();
    eprintln!(
        "── 盲测汇总 ──\n{}\n证据: {}",
        serde_json::to_string_pretty(&summary).unwrap(),
        out_path.display()
    );

    // 硬断言只防脱轨：全部意图有裁决（弃权不超 1/3），缺席臂如实登记
    let total_verdicts = pairs
        .iter()
        .map(|p| p.left_votes + p.right_votes)
        .sum::<usize>();
    assert!(
        total_verdicts * 3 >= (intents().len() * JUDGE_ROUNDS as usize) * 2,
        "裁决弃权过多：{total_verdicts}"
    );
    assert_eq!(
        summary.absent_arms,
        vec!["sequential_crew".to_string(), "duet_merge".to_string()]
    );

    env.cleanup();
}

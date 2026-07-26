//! Card Studio 实机 golden path（真实 LLM，#[ignore]）
//!
//! 目标：补 STATUS 文档唯一 High 缺口——A 从零 / C 修订各产一张卡，
//! 全程走与 `cardstudio_run_stage` / `cardstudio_run_review` 相同的
//! prompt 装配、JSON 解析、apply、阶段推进逻辑（in-memory 驱动，
//! 不落 app data 目录），最后过出卡质量闸门（JSON+PNG 双 round-trip）。
//!
//! 运行方式：
//! ```text
//! $env:LLM_API_KEY='<key>'                       # 必需，只从环境读取
//! $env:LLM_BASE_URL='https://cli.2529985.xyz/v1' # 可选，默认此值
//! $env:LLM_MODEL='deepseek-v4-pro'               # 可选，默认此值
//! cargo test -p storyforge --test card_studio_golden_path_real_llm -- --ignored --nocapture
//! ```
//!
//! 证据（脱敏，无 key）写入 `artifacts/card-studio/`（已 gitignore）。

use std::path::PathBuf;
use std::sync::Arc;

use storyforge_domain::card_studio::{
    apply_stage_json, build_review_prompt, build_stage_prompt, compile_artifacts,
    export_gate_checks, extract_json_object, merge_review_reports, run_checks, CardArtifacts,
    CardProject, CheckReport, GateCheck, StageStatus, WorldviewDraftEntry, STAGE_BASIC,
    STAGE_BRIEF, STAGE_OPENING, STAGE_PERSONALITY, STAGE_REVIEW, STAGE_WORLDVIEW,
};
use storyforge_domain::character::StCharacterCard;
use storyforge_domain::llm::{
    ChatMessage, ChatRequest, LlmConnection, LlmProtocol, SamplingParams, ToolMode,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn evidence_dir() -> PathBuf {
    let dir = repo_root().join("artifacts").join("card-studio");
    std::fs::create_dir_all(&dir).expect("创建证据目录失败");
    dir
}

fn make_llm() -> Arc<dyn storyforge_infra_llm::LlmClient> {
    let base_url = std::env::var("LLM_BASE_URL")
        .unwrap_or_else(|_| "https://cli.2529985.xyz/v1".into());
    let api_key = std::env::var("LLM_API_KEY").expect("需设 LLM_API_KEY（只从环境读取）");
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".into());
    let conn = LlmConnection {
        id: storyforge_domain::Id::new(),
        name: format!("card-studio-golden-{model}"),
        base_url,
        api_key,
        model,
        protocol: LlmProtocol::OpenAi,
        params: SamplingParams {
            // 中继不设 max_tokens 时补全上限过小，JSON 会被截断（同卡片翻译验收结论）
            max_tokens: Some(8192),
            max_tokens_explicit: true,
            ..Default::default()
        },
        tool_mode: ToolMode::Native,
    };
    let client = storyforge_infra_llm::create_client(&conn).expect("构造 LLM client 失败");
    Arc::from(client)
}

fn model_name() -> String {
    std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".into())
}

/// 镜像 `cardstudio_run_stage`：prompt → chat → extract_json → apply → 推进。
/// JSON 解析失败重试一次（对应 GUI 用户重点一次「AI 生成本阶段」）。
async fn drive_stage(
    llm: &Arc<dyn storyforge_infra_llm::LlmClient>,
    project: &mut CardProject,
    stage_id: &str,
    user_note: Option<&str>,
    log: &mut Vec<serde_json::Value>,
) -> Result<(), String> {
    let (system, user) = build_stage_prompt(stage_id, project, user_note)?;
    let mut last_err = String::new();
    for attempt in 1..=2 {
        let req = ChatRequest {
            messages: vec![
                ChatMessage::system(system.clone()),
                ChatMessage::user(user.clone()),
            ],
            tools: None,
            params: SamplingParams {
                temperature: Some(0.7),
                max_tokens: Some(8192),
                max_tokens_explicit: true,
                ..Default::default()
            },
            model: model_name(),
        };
        let t0 = std::time::Instant::now();
        let resp = llm
            .chat(&req)
            .await
            .map_err(|e| format!("阶段 {stage_id} LLM 调用失败: {e}"))?;
        let elapsed = t0.elapsed();
        project.last_stage_output = Some(resp.content.clone());
        match extract_json_object(&resp.content).and_then(|json| {
            apply_stage_json(stage_id, &mut project.artifacts, &json).map(|_| json)
        }) {
            Ok(_) => {
                if !project.artifacts.name.trim().is_empty() {
                    project.name = project.artifacts.name.trim().to_string();
                }
                project.set_stage_status(stage_id, StageStatus::Done);
                let next = match stage_id {
                    STAGE_BASIC => Some(STAGE_PERSONALITY),
                    STAGE_PERSONALITY => Some(STAGE_WORLDVIEW),
                    STAGE_WORLDVIEW => Some(STAGE_OPENING),
                    STAGE_OPENING => Some(STAGE_REVIEW),
                    _ => None,
                };
                if let Some(n) = next {
                    project.set_stage_status(n, StageStatus::Ready);
                    project.current_stage = n.to_string();
                }
                eprintln!(
                    "[stage {stage_id}] OK attempt={attempt} {}ms output={} 字符",
                    elapsed.as_millis(),
                    resp.content.chars().count()
                );
                log.push(serde_json::json!({
                    "stage": stage_id,
                    "attempt": attempt,
                    "elapsed_ms": elapsed.as_millis() as u64,
                    "output_chars": resp.content.chars().count(),
                }));
                return Ok(());
            }
            Err(e) => {
                eprintln!("[stage {stage_id}] attempt={attempt} 解析/应用失败: {e}");
                last_err = e;
            }
        }
    }
    Err(format!("阶段 {stage_id} 两次尝试均失败: {last_err}"))
}

/// 镜像 `cardstudio_run_review`（use_llm=true 分支，解析失败回退规则报告）。
async fn drive_review(
    llm: &Arc<dyn storyforge_infra_llm::LlmClient>,
    project: &CardProject,
) -> CheckReport {
    let rule_report = run_checks(&project.artifacts);
    let (system, user) = build_review_prompt(project, &rule_report, None);
    let req = ChatRequest {
        messages: vec![ChatMessage::system(system), ChatMessage::user(user)],
        tools: None,
        params: SamplingParams {
            temperature: Some(0.3),
            max_tokens: Some(8192),
            max_tokens_explicit: true,
            ..Default::default()
        },
        model: model_name(),
    };
    match llm.chat(&req).await {
        Ok(resp) => match extract_json_object(&resp.content) {
            Ok(v) => merge_review_reports(rule_report, &v),
            Err(_) => {
                let mut report = rule_report;
                report.summary = Some(format!(
                    "{}（LLM 审查输出无法解析，已回退规则检查）",
                    report.summary.unwrap_or_default()
                ));
                report.source = Some("rule".into());
                report
            }
        },
        Err(e) => {
            let mut report = rule_report;
            report.summary = Some(format!(
                "{}（LLM 审查调用失败: {e}，已回退规则检查）",
                report.summary.unwrap_or_default()
            ));
            report.source = Some("rule".into());
            report
        }
    }
}

/// 镜像 `run_export_gate`：compile → JSON/PNG 真实导入 round-trip → GateCheck。
fn drive_export_gate(
    artifacts: &CardArtifacts,
) -> Result<(Vec<GateCheck>, Vec<GateCheck>, serde_json::Value, Vec<u8>), String> {
    let compiled = compile_artifacts(artifacts)?;
    let st_card: StCharacterCard = serde_json::from_value(compiled.st_card_json.clone())
        .map_err(|e| format!("ST 卡 JSON 反序列化失败: {e}"))?;

    let json_bytes =
        serde_json::to_vec(&compiled.st_card_json).map_err(|e| format!("序列化失败: {e}"))?;
    let json_reimported = storyforge_infra_import::import_character(&json_bytes)
        .map_err(|e| format!("JSON round-trip 导入失败: {e}"))?;
    let json_checks = export_gate_checks(artifacts, &json_reimported);

    let png_bytes = storyforge_infra_import::png::write_st_card_png(&st_card, None)
        .map_err(|e| format!("PNG 导出失败: {e}"))?;
    let png_reimported = storyforge_infra_import::import_character(&png_bytes)
        .map_err(|e| format!("PNG round-trip 导入失败: {e}"))?;
    let png_checks = export_gate_checks(artifacts, &png_reimported);

    Ok((json_checks, png_checks, compiled.st_card_json, png_bytes))
}

fn assert_gate_pass(json_checks: &[GateCheck], png_checks: &[GateCheck], tag: &str) {
    for (label, checks) in [("JSON", json_checks), ("PNG", png_checks)] {
        for c in checks {
            eprintln!(
                "[{tag}][{label} {}] {}: {}",
                if c.pass { "PASS" } else { "FAIL" },
                c.name,
                c.detail
            );
        }
    }
    let failed: Vec<String> = json_checks
        .iter()
        .map(|c| ("JSON", c))
        .chain(png_checks.iter().map(|c| ("PNG", c)))
        .filter(|(_, c)| !c.pass)
        .map(|(l, c)| format!("[{l}] {}: {}", c.name, c.detail))
        .collect();
    assert!(failed.is_empty(), "「{tag}」出卡闸门未过:\n{}", failed.join("\n"));
}

fn write_evidence(name: &str, value: &serde_json::Value) -> PathBuf {
    let path = evidence_dir().join(name);
    std::fs::write(&path, serde_json::to_string_pretty(value).expect("序列化证据"))
        .expect("写证据失败");
    eprintln!("证据: {}", path.display());
    path
}

// ═══════════════════════════════════════════════════════════════════════════
// A 从零 golden path
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要真实 LLM 凭证（LLM_API_KEY）"]
async fn card_studio_a_path_golden_real_llm() {
    let llm = make_llm();
    let mut stage_log: Vec<serde_json::Value> = Vec::new();

    let mut project = CardProject::new_from_scratch(
        "golden-a-守灯人",
        "末班电车之后的城市里，有一位在雨夜车站守着最后一盏灯的年轻守灯人。\
         灯亮着，迷路的人就能找到回家的路；灯灭了，被雨困住的旧日残影会漫上站台。\
         希望是单主角旅程卡：克制、温和、话少，但每句都落在实处。\
         世界观走低魔现代都市怪谈，规模小而完整。",
    );
    // brief 阶段是手工确认（镜像 cardstudio_complete_manual_stage）
    project.set_stage_status(STAGE_BRIEF, StageStatus::Done);
    project.set_stage_status(STAGE_BASIC, StageStatus::Ready);
    project.current_stage = STAGE_BASIC.to_string();

    for stage in [STAGE_BASIC, STAGE_PERSONALITY, STAGE_WORLDVIEW, STAGE_OPENING] {
        drive_stage(&llm, &mut project, stage, None, &mut stage_log)
            .await
            .expect("阶段生成失败");
    }

    let review = drive_review(&llm, &project).await;
    eprintln!(
        "[review] ok={} score={:?} source={:?} issues={}",
        review.ok,
        review.score,
        review.source,
        review.issues.len()
    );
    assert!(
        review.ok,
        "审查未过: {:?}",
        review
            .issues
            .iter()
            .map(|i| format!("{}: {}", i.code, i.message))
            .collect::<Vec<_>>()
    );

    let (json_checks, png_checks, st_card_json, png_bytes) =
        drive_export_gate(&project.artifacts).expect("闸门运行失败");
    assert_gate_pass(&json_checks, &png_checks, "A-golden");

    // 可玩性底线（不过度断言 LLM 行为）
    assert!(!project.artifacts.name.trim().is_empty());
    assert!(!project.artifacts.first_mes.trim().is_empty());
    assert!(
        !project.artifacts.worldview_entries.is_empty(),
        "世界书不应为空"
    );

    let png_path = evidence_dir().join("golden-a-card.png");
    std::fs::write(&png_path, &png_bytes).expect("写 PNG 失败");
    eprintln!("可导入 PNG 卡: {}", png_path.display());

    write_evidence(
        "golden-a-evidence.json",
        &serde_json::json!({
            "path": "A-from-scratch",
            "model": model_name(),
            "stage_pack": project.stage_pack_id,
            "stages": stage_log,
            "review": {
                "ok": review.ok,
                "score": review.score,
                "source": review.source,
                "issue_count": review.issues.len(),
                "summary": review.summary,
            },
            "gate": {
                "json_checks": json_checks,
                "png_checks": png_checks,
            },
            "card_name": project.artifacts.name,
            "worldview_entries": project.artifacts.worldview_entries.len(),
            "st_card_json": st_card_json,
        }),
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// C 修订 golden path（reverse_parse → 局部重跑 → 另存语义）
// ═══════════════════════════════════════════════════════════════════════════

fn c_path_source_artifacts() -> CardArtifacts {
    CardArtifacts {
        name: "灯下客".into(),
        description: "总在末班车后出现在站台的青年，随身带一盏纸罩灯。".into(),
        personality: "淡漠而有礼，只谈眼前事，从不解释来历。".into(),
        scenario: "雨夜的旧站台，灯罩上凝着水珠。".into(),
        first_mes: "他把灯往你这边挪了半寸：……末班车已经走了。".into(),
        tags: vec!["都市怪谈".into()],
        creator: "golden-c-source".into(),
        worldview_entries: vec![
            WorldviewDraftEntry {
                keys: vec![],
                content: "常驻：灯亮时，站台是安全的。".into(),
                constant: true,
                order: 10,
            },
            WorldviewDraftEntry {
                keys: vec!["纸罩灯".into(), "灯".into()],
                content: "纸罩灯的火苗不怕雨，只怕谎话。".into(),
                constant: false,
                order: 20,
            },
        ],
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要真实 LLM 凭证（LLM_API_KEY）"]
async fn card_studio_c_path_golden_real_llm() {
    let llm = make_llm();
    let mut stage_log: Vec<serde_json::Value> = Vec::new();

    // 源卡：确定性编译（不耗 LLM），模拟卡库里已有的卡
    let source = compile_artifacts(&c_path_source_artifacts()).expect("源卡编译");
    let source_id = source.character.id.as_str().to_string();

    // 镜像 cardstudio_create_from_character → reverse_parse 落检查阶段
    let mut project = CardProject::new_from_existing_character(
        &source.character,
        None,
        "修订角色卡：灯下客——开场白改得更有动作感，保留原设定",
    );
    eprintln!(
        "[C] reverse_parse: name={} entries={} stage={}",
        project.artifacts.name,
        project.artifacts.worldview_entries.len(),
        project.current_stage
    );
    assert_eq!(project.artifacts.name, "灯下客", "reverse_parse 应回填基础字段");
    assert_eq!(
        project.artifacts.worldview_entries.len(),
        2,
        "reverse_parse 应回填世界书"
    );

    // 局部重跑开场白（真实 LLM，带修订指令）
    drive_stage(
        &llm,
        &mut project,
        STAGE_OPENING,
        Some("保留守灯人设定与人物关系，把开场白改写得更有动作感与画面感，长度相近"),
        &mut stage_log,
    )
    .await
    .expect("C 路开场白重跑失败");

    let review = drive_review(&llm, &project).await;
    eprintln!(
        "[C review] ok={} score={:?} issues={}",
        review.ok,
        review.score,
        review.issues.len()
    );
    assert!(
        review.ok,
        "C 修订审查未过: {:?}",
        review
            .issues
            .iter()
            .map(|i| format!("{}: {}", i.code, i.message))
            .collect::<Vec<_>>()
    );

    // 编译另存：新 Character.id，原卡不受影响
    let revised = compile_artifacts(&project.artifacts).expect("修订编译");
    assert_ne!(
        revised.character.id.as_str(),
        source_id,
        "修订编译必须产生新 domain id（另存语义）"
    );

    let (json_checks, png_checks, st_card_json, png_bytes) =
        drive_export_gate(&project.artifacts).expect("C 闸门运行失败");
    assert_gate_pass(&json_checks, &png_checks, "C-golden");

    let png_path = evidence_dir().join("golden-c-card.png");
    std::fs::write(&png_path, &png_bytes).expect("写 PNG 失败");
    eprintln!("可导入 PNG 卡: {}", png_path.display());

    write_evidence(
        "golden-c-evidence.json",
        &serde_json::json!({
            "path": "C-revise-reverse-parse",
            "model": model_name(),
            "source_character_id": source_id,
            "revised_character_id": revised.character.id.as_str(),
            "stages": stage_log,
            "review": {
                "ok": review.ok,
                "score": review.score,
                "source": review.source,
                "issue_count": review.issues.len(),
            },
            "gate": {
                "json_checks": json_checks,
                "png_checks": png_checks,
            },
            "revised_first_mes": project.artifacts.first_mes,
            "st_card_json": st_card_json,
        }),
    );
}

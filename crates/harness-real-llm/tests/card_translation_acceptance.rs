//! 新型卡片翻译验收（2026-07-26）
//!
//! 两张验收卡（仓库根目录，未入库的本机文件）：
//!   - `test-card.png`（命定之诗：引导器+远程应用形态）
//!   - `卿卿 (33).png`（单体全内嵌形态，chara_card_v3 双块）
//!
//! 确定性部分（无 LLM，卡文件缺失时跳过不 fail）：导入检查 + 组件归类零漏项。
//!
//! 真实模型部分（#[ignore]）：
//! ```text
//! $env:LLM_BASE_URL='https://cli.2529985.xyz/v1'
//! $env:LLM_API_KEY='<key>'
//! $env:LLM_MODEL='deepseek-v4-pro'          # require_real_llm 兜底用；实际模型见下
//! $env:STORYFORGE_CT_MODELS='deepseek-v4-pro,deepseek-v4-flash'
//! $env:STORYFORGE_LLM_TIMEOUT_SECS='600'
//! cargo test -p harness-real-llm --test card_translation_acceptance -- --ignored --nocapture
//! ```
//! 每个（卡 × 模型）组合写一份脱敏证据 JSON 到 artifacts/card-translation/。

use std::path::PathBuf;
use std::sync::Arc;

use harness_real_llm::card_translation::{
    CardExpectations, build_evidence, classify_components, component_checks, import_checks,
    load_card, repo_root, run_extraction, run_mvu_translation, translation_checks, write_evidence,
};

fn card_path(env_key: &str, default_name: &str) -> PathBuf {
    std::env::var(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join(default_name))
}

fn acceptance_cards() -> Vec<(PathBuf, CardExpectations)> {
    vec![
        (
            card_path("STORYFORGE_CT_CARD_DESTINY", "test-card.png"),
            CardExpectations::destiny(),
        ),
        (
            card_path("STORYFORGE_CT_CARD_QINGQING", "卿卿 (33).png"),
            CardExpectations::qingqing(),
        ),
    ]
}

// ═══════════════════════════════════════════════════════════════════════════
// 确定性验收（无 LLM）
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn deterministic_import_and_component_checks() {
    let mut ran = 0;
    for (path, exp) in acceptance_cards() {
        if !path.exists() {
            eprintln!(
                "[skip] 卡文件不存在: {}（本机验收卡，CI 无此文件属正常）",
                path.display()
            );
            continue;
        }
        ran += 1;
        let character = load_card(&path).expect("卡应可导入");
        let mut checks = import_checks(&character, &exp);
        let components = classify_components(&character);
        checks.extend(component_checks(&components, &exp));

        for c in &checks {
            eprintln!(
                "[{}] {} {}: {}",
                exp.label,
                if c.pass { "PASS" } else { "FAIL" },
                c.name,
                c.detail
            );
        }
        let failed: Vec<_> = checks.iter().filter(|c| !c.pass).collect();
        assert!(
            failed.is_empty(),
            "「{}」确定性验收未过: {:?}",
            exp.label,
            failed
        );
    }
    if ran == 0 {
        eprintln!("[skip] 两张验收卡都不在本机，确定性验收未执行");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 真实模型验收（2 卡 × 2 模型）
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/LLM_API_KEY + STORYFORGE_CT_MODELS）"]
async fn card_translation_acceptance_real_llm() {
    // 接 tracing（stderr）——analyze_mvu_card 的"解析失败，降级"等 warn 必须可见，
    // 否则空壳降级的根因无法诊断
    let log_store = Arc::new(storyforge_app_logging::LogStore::new(
        std::env::temp_dir().join("storyforge-ct-logs"),
    ));
    storyforge_app_logging::init_tracing(log_store);

    let base_url = std::env::var("LLM_BASE_URL").expect("需设 LLM_BASE_URL");
    let api_key = std::env::var("LLM_API_KEY").expect("需设 LLM_API_KEY（只从环境读取）");
    let models: Vec<String> = std::env::var("STORYFORGE_CT_MODELS")
        .unwrap_or_else(|_| "deepseek-v4-pro,deepseek-v4-flash".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(!models.is_empty(), "STORYFORGE_CT_MODELS 为空");
    // 目标是（max）档：默认给 reasoning_effort=max（端点已实测接受），可用
    // STORYFORGE_CT_EXTRA_JSON 覆盖或置 {} 关闭
    let extra: Option<serde_json::Map<String, serde_json::Value>> = {
        let raw = std::env::var("STORYFORGE_CT_EXTRA_JSON")
            .unwrap_or_else(|_| r#"{"reasoning_effort":"max"}"#.into());
        let v: serde_json::Value =
            serde_json::from_str(&raw).expect("STORYFORGE_CT_EXTRA_JSON 须为 JSON");
        let map = v
            .as_object()
            .cloned()
            .expect("STORYFORGE_CT_EXTRA_JSON 须为 JSON 对象");
        (!map.is_empty()).then_some(map)
    };

    let mut failures: Vec<String> = Vec::new();
    let mut evidence_paths: Vec<PathBuf> = Vec::new();

    // 并行验收：4 个（卡 × 模型）组合互相独立并发；组合内角色抽取与 MVU 分析
    // 也互相独立（各自空 ToolContext），tokio::join 同时跑。
    // 墙钟时间从 Σ(所有调用) 降到 ≈ 最慢一条链。
    let max_tokens: u32 = std::env::var("STORYFORGE_CT_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16384);

    struct ComboOutcome {
        label: String,
        model: String,
        checks: Vec<harness_real_llm::card_translation::Check>,
        failures: Vec<String>,
        evidence_path: Option<PathBuf>,
    }

    // 并发上限：2 个组合同时（组合内双路 → 峰值 4 个 LLM 请求）。
    // 第三轮实测 8 路并发打空中继号池（error 1101 / auth_unavailable），4 路为安全水位
    let combo_semaphore = Arc::new(tokio::sync::Semaphore::new(
        std::env::var("STORYFORGE_CT_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2usize),
    ));

    let mut handles = Vec::new();
    for (path, exp) in acceptance_cards() {
        assert!(
            path.exists(),
            "验收卡缺失: {}（真实验收要求两张卡都在）",
            path.display()
        );
        let character = Arc::new(load_card(&path).expect("卡应可导入"));
        let components = Arc::new(classify_components(&character));

        for model in &models {
            let character = character.clone();
            let components = components.clone();
            let exp = exp.clone();
            let model = model.clone();
            let base_url = base_url.clone();
            let api_key = api_key.clone();
            let extra = extra.clone();
            let semaphore = combo_semaphore.clone();

            handles.push(tokio::spawn(async move {
                let _permit = semaphore.acquire().await.expect("semaphore closed");
                let tag = format!("{}×{}", exp.label, model);
                eprintln!("===== 启动: {tag} =====");
                let conn = storyforge_domain::llm::LlmConnection {
                    id: storyforge_domain::Id::new(),
                    name: format!("card-translation-{model}"),
                    base_url,
                    api_key,
                    model: model.clone(),
                    protocol: storyforge_domain::llm::LlmProtocol::OpenAi,
                    params: storyforge_domain::llm::SamplingParams {
                        extra,
                        // 不设 max_tokens 时中继默认补全上限过小，max 推理会把
                        // 预算吃光导致 JSON 截断 → 5 层解析全 miss → 静默空壳
                        max_tokens: Some(max_tokens),
                        max_tokens_explicit: true,
                        ..Default::default()
                    },
                    tool_mode: storyforge_domain::llm::ToolMode::Native,
                };
                let client =
                    storyforge_infra_llm::create_client(&conn).expect("构造 LLM client 失败");
                let llm: Arc<dyn storyforge_infra_llm::LlmClient> = Arc::from(client);

                let mut checks = import_checks(&character, &exp);
                checks.extend(component_checks(&components, &exp));
                let mut local_failures: Vec<String> = Vec::new();
                let mut evidence_path = None;

                let t0 = std::time::Instant::now();
                let (defs_res, trans_res) = tokio::join!(
                    run_extraction(llm.clone(), &character, &tag),
                    run_mvu_translation(llm.clone(), &character, &tag)
                );
                eprintln!("[{tag}] 双路并行完成，墙钟 {:?}", t0.elapsed());

                match (&defs_res, &trans_res) {
                    (Ok(defs), Ok(translation)) => {
                        eprintln!(
                            "[{tag}] 抽取 {} 定义；翻译 schema={} rules={} confidence={:.2}",
                            defs.len(),
                            translation.variable_schema.len(),
                            translation.update_rules.len(),
                            translation.analysis_confidence
                        );
                        checks.extend(translation_checks(defs, translation, &exp));
                        let evidence = build_evidence(
                            &exp.label,
                            &model,
                            &character,
                            defs,
                            translation,
                            &components,
                            &checks,
                        );
                        match write_evidence(&evidence) {
                            Ok(p) => evidence_path = Some(p),
                            Err(e) => local_failures.push(format!("{tag}: 证据写盘失败: {e}")),
                        }
                    }
                    (Err(e), _) => local_failures.push(format!("{tag}: 角色抽取失败: {e}")),
                    (_, Err(e)) => local_failures.push(format!("{tag}: MVU 翻译失败: {e}")),
                }

                let failed: Vec<String> = checks
                    .iter()
                    .filter(|c| !c.pass)
                    .map(|c| format!("{}: {}", c.name, c.detail))
                    .collect();
                if !failed.is_empty() {
                    local_failures.push(format!("{tag} 断言未过:\n  {}", failed.join("\n  ")));
                }

                ComboOutcome {
                    label: exp.label.clone(),
                    model,
                    checks,
                    failures: local_failures,
                    evidence_path,
                }
            }));
        }
    }

    for handle in handles {
        let outcome = handle.await.expect("验收子任务 panic");
        eprintln!("\n===== 结果: {} × {} =====", outcome.label, outcome.model);
        for c in &outcome.checks {
            eprintln!(
                "[{}×{}] {} {}: {}",
                outcome.label,
                outcome.model,
                if c.pass { "PASS" } else { "FAIL" },
                c.name,
                c.detail
            );
        }
        if let Some(p) = &outcome.evidence_path {
            eprintln!(
                "[{}×{}] 证据: {}",
                outcome.label,
                outcome.model,
                p.display()
            );
            evidence_paths.push(p.clone());
        }
        failures.extend(outcome.failures);
    }

    eprintln!("\n===== 汇总: {} 份证据 =====", evidence_paths.len());
    for p in &evidence_paths {
        eprintln!("  {}", p.display());
    }
    assert!(
        failures.is_empty(),
        "真实模型验收未全绿:\n{}",
        failures.join("\n")
    );
}

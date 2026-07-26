//! 新型卡片翻译验收 harness（2026-07-26）
//!
//! 目标：把「引导器+远程应用」（命定之诗）与「单体全内嵌」（卿卿）两代 ST 卡
//! 通过 Meta Agent 翻译成本项目格式，并做确定性验收：
//!   1. 导入检查（世界书条目数 / 启用禁用位 / v3 字段语义）
//!   2. 组件兼容归类（每个 regex 界面脚本与 tavern_helper 脚本都必须有归类，零漏项）
//!   3. 真实 LLM 角色抽取（CharacterDefinition，persona 非空）
//!   4. 真实 LLM MVU 五合一翻译（variable_schema / update_rules 非空、非降级空壳）
//!   5. 证据落盘（脱敏：只记计数、字段键名、断言结果，不记正文）
//!
//! 真实模型入口见 `tests/card_translation_acceptance.rs`（#[ignore]，需
//! LLM_BASE_URL / LLM_API_KEY；模型列表 STORYFORGE_CT_MODELS）。

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use storyforge_app_agent::runtime::AgentRuntime;
use storyforge_app_agent::tools::ToolContext;
use storyforge_domain::character::{Character, CharacterDefinition};
use storyforge_domain::mvu_translation::{MvuRouting, MvuTranslation};
use storyforge_infra_llm::LlmClient;
use storyforge_domain::variables::extract_mvu_schema_from_extensions;

// ═══════════════════════════════════════════════════════════════════════════
// 验收期望与检查结构
// ═══════════════════════════════════════════════════════════════════════════

/// 每张卡的确定性验收期望（阈值取"明显达标"而非贴线，容忍卡片小版本差异）
#[derive(Debug, Clone)]
pub struct CardExpectations {
    pub label: String,
    /// 世界书条目总数下限（禁用条目保留后）
    pub min_book_entries: usize,
    pub min_enabled_entries: usize,
    /// 禁用条目下限——为 0 说明 v3 enabled:false 没被解析，导入退化
    pub min_disabled_entries: usize,
    /// extensions.regex_scripts 精确条数（组件归类零漏项检查）
    pub regex_script_count: usize,
    /// extensions.tavern_helper.scripts 精确条数
    pub tavern_helper_count: usize,
    /// 角色抽取最少产出的 CharacterDefinition 数
    pub min_definitions: usize,
    /// 期望出现的角色名（命中 `min_expected_names` 个即可；空 = 不检查）
    pub expected_names: Vec<String>,
    pub min_expected_names: usize,
    /// variable_schema 字段数下限
    pub min_schema_fields: usize,
    /// schema 键/标签里至少出现其中一个关键词（证明 schema 来自卡而非编造）
    pub schema_keyword_any: Vec<String>,
    pub min_update_rules: usize,
}

impl CardExpectations {
    pub fn destiny() -> Self {
        Self {
            label: "命定之诗".into(),
            min_book_entries: 400,
            min_enabled_entries: 280,
            min_disabled_entries: 100,
            regex_script_count: 13,
            tavern_helper_count: 6,
            // 命定之诗是单主角旅程卡（description 为空、世界书以世界/事件为主），
            // 第一轮实测 flash 抽取产出 1 个主角定义属合理产物；抽取能力的强证据
            // 由卿卿（11 定义 / 6 名字全中）承担
            min_definitions: 1,
            expected_names: vec![],
            min_expected_names: 0,
            min_schema_fields: 5,
            schema_keyword_any: vec![
                "属性".into(),
                "任务".into(),
                "等级".into(),
                "主角".into(),
                "世界".into(),
                "事件".into(),
            ],
            min_update_rules: 1,
        }
    }

    pub fn qingqing() -> Self {
        Self {
            label: "卿卿".into(),
            min_book_entries: 115,
            min_enabled_entries: 45,
            min_disabled_entries: 60,
            regex_script_count: 19,
            tavern_helper_count: 16,
            min_definitions: 4,
            expected_names: vec![
                "陆希微".into(),
                "江离".into(),
                "沈若萱".into(),
                "赵徽柔".into(),
                "东方珏".into(),
                "长孙燕".into(),
            ],
            min_expected_names: 2,
            min_schema_fields: 5,
            schema_keyword_any: vec![
                "好感度".into(),
                "在场".into(),
                "主角".into(),
                "女性角色".into(),
                "stat_data".into(),
            ],
            min_update_rules: 1,
        }
    }
}

/// 单条验收检查结果
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub pass: bool,
    pub detail: String,
}

fn check(name: &str, pass: bool, detail: String) -> Check {
    Check {
        name: name.into(),
        pass,
        detail,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 卡片加载与确定性检查（无 LLM）
// ═══════════════════════════════════════════════════════════════════════════

/// 仓库根目录（harness crate 上两级）
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 读取并导入卡片文件
pub fn load_card(path: &std::path::Path) -> Result<Character, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读卡失败 {}: {e}", path.display()))?;
    storyforge_infra_import::import_character(&bytes).map_err(|e| format!("导入失败: {e}"))
}

/// 导入层确定性检查（新语义：禁用条目保留、路由不死档）
pub fn import_checks(character: &Character, exp: &CardExpectations) -> Vec<Check> {
    let mut out = Vec::new();
    let book = character.embedded_world_info.as_ref();
    let total = book.map(|b| b.entries.len()).unwrap_or(0);
    let enabled = book
        .map(|b| b.entries.iter().filter(|e| !e.disabled).count())
        .unwrap_or(0);
    let disabled = total - enabled;

    out.push(check(
        "import.book_total",
        total >= exp.min_book_entries,
        format!("{total} 条（下限 {}）", exp.min_book_entries),
    ));
    out.push(check(
        "import.book_enabled",
        enabled >= exp.min_enabled_entries,
        format!("{enabled} 条启用（下限 {}）", exp.min_enabled_entries),
    ));
    out.push(check(
        "import.book_disabled_retained",
        disabled >= exp.min_disabled_entries,
        format!(
            "{disabled} 条禁用被保留（下限 {}；为 0 说明 v3 enabled:false 未解析）",
            exp.min_disabled_entries
        ),
    ));
    // 禁用条目绝不进常驻注入
    let leaked = book
        .map(|b| b.constant_entries().iter().any(|e| e.disabled))
        .unwrap_or(false);
    out.push(check(
        "import.disabled_never_constant",
        !leaked,
        "禁用条目不得出现在常驻注入".into(),
    ));
    // 死路由检查：启用且带主键的非常驻条目必须可触发（(false,false) 修复的回归锚点）
    let dead = book
        .map(|b| {
            b.entries
                .iter()
                .filter(|e| {
                    !e.disabled
                        && !e.constant
                        && e.keys.iter().any(|k| !k.trim().is_empty())
                        && matches!(
                            e.route,
                            storyforge_domain::world_info::LoreRoute::Disabled
                        )
                })
                .count()
        })
        .unwrap_or(0);
    out.push(check(
        "import.no_dead_keyword_routes",
        dead == 0,
        format!("{dead} 条启用关键词条目被路由成 Disabled（应为 0）"),
    ));
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// 组件兼容归类（确定性，零漏项）
// ═══════════════════════════════════════════════════════════════════════════

/// 单个卡片组件（regex 界面脚本 / TH 脚本）的兼容归类
#[derive(Debug, Clone, Serialize)]
pub struct ComponentDisposition {
    pub component: String,
    pub label: String,
    pub enabled: bool,
    /// presentation_shell_remote / presentation_shell_inline / text_regex /
    /// mvu_framework / remote_module / inline_script
    pub disposition: String,
    pub remote_url: Option<String>,
}

/// 对卡里每个 regex 脚本与 tavern_helper 脚本做归类。
/// 归类是翻译战略的兼容矩阵投影：UI 壳留表现层沙箱、MVU 框架是原生化目标、
/// 文本 regex 走 infra-regex、远程模块挂牌沙箱。
pub fn classify_components(character: &Character) -> Vec<ComponentDisposition> {
    let mut out = Vec::new();

    if let Some(scripts) = character
        .extensions
        .get("regex_scripts")
        .and_then(|v| v.as_array())
    {
        for s in scripts {
            let label = s
                .get("scriptName")
                .and_then(|v| v.as_str())
                .unwrap_or("(未命名)")
                .to_string();
            let enabled = !s.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let replace = s.get("replaceString").and_then(|v| v.as_str()).unwrap_or("");
            let load_url = replace
                .find("').load('")
                .or_else(|| replace.find("\").load(\""))
                .and_then(|_| extract_first_http_url(replace));
            let disposition = if replace.contains("').load('") || replace.contains("\").load(\"") {
                "presentation_shell_remote"
            } else if replace.contains("<script") || replace.contains("<style") {
                "presentation_shell_inline"
            } else {
                "text_regex"
            };
            out.push(ComponentDisposition {
                component: "regex_script".into(),
                label,
                enabled,
                disposition: disposition.into(),
                remote_url: load_url,
            });
        }
    }

    if let Some(scripts) = character
        .extensions
        .get("tavern_helper")
        .and_then(|v| v.get("scripts"))
        .and_then(|v| v.as_array())
    {
        for s in scripts {
            let label = s
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("(未命名)")
                .to_string();
            let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            let content = s.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let url = extract_first_http_url(content);
            let disposition = if content.contains("MagVarUpdate") {
                "mvu_framework"
            } else if content.trim_start().starts_with("import")
                && url.is_some()
                && content.chars().count() < 2000
            {
                "remote_module"
            } else {
                "inline_script"
            };
            out.push(ComponentDisposition {
                component: "tavern_helper".into(),
                label,
                enabled,
                disposition: disposition.into(),
                remote_url: url,
            });
        }
    }

    out
}

fn extract_first_http_url(text: &str) -> Option<String> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c == '\'' || c == '"' || c == ')' || c.is_whitespace() || c == '<')
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// 组件归类的确定性检查：数量零漏项 + 分布合理
pub fn component_checks(
    components: &[ComponentDisposition],
    exp: &CardExpectations,
) -> Vec<Check> {
    let regex_n = components
        .iter()
        .filter(|c| c.component == "regex_script")
        .count();
    let th_n = components
        .iter()
        .filter(|c| c.component == "tavern_helper")
        .count();
    vec![
        check(
            "compat.regex_all_classified",
            regex_n == exp.regex_script_count,
            format!("{regex_n}/{} 个 regex 脚本已归类", exp.regex_script_count),
        ),
        check(
            "compat.tavern_helper_all_classified",
            th_n == exp.tavern_helper_count,
            format!("{th_n}/{} 个 TH 脚本已归类", exp.tavern_helper_count),
        ),
        check(
            "compat.mvu_framework_recognized",
            components.iter().any(|c| c.disposition == "mvu_framework"),
            "应识别出 MagVarUpdate 框架依赖".into(),
        ),
    ]
}

// ═══════════════════════════════════════════════════════════════════════════
// 真实 LLM 翻译流程（角色抽取 + MVU 五合一）
// ═══════════════════════════════════════════════════════════════════════════

fn empty_tool_ctx() -> Arc<ToolContext> {
    Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: vec![],
        chronicle_tool_budget: Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    })
}

/// 五合一翻译是否为"降级空壳"（分析失败的静默形态，验收必须识别并拒绝）
pub fn is_empty_fallback(t: &MvuTranslation) -> bool {
    t.analysis_confidence == 0.0
        && t.ui_bindings.is_empty()
        && t.update_rules.is_empty()
        && t.interactions.is_empty()
        && t.fallback_fragments.is_empty()
}

/// 跑角色抽取（带重试）。`tag` 用于并行运行时区分日志来源。
pub async fn run_extraction(
    llm: Arc<dyn LlmClient>,
    character: &Character,
    tag: &str,
) -> Result<Vec<CharacterDefinition>, String> {
    let runtime = AgentRuntime::new(llm, empty_tool_ctx());
    let mvu_schema = extract_mvu_schema_from_extensions(&character.extensions);
    let mut last_err = String::new();
    for attempt in 1..=3 {
        let (_tx, cancel) = tokio::sync::watch::channel(false);
        match storyforge_app_agent::character_extractor::extract_characters(
            &runtime,
            character,
            &mvu_schema,
            cancel,
        )
        .await
        {
            Ok(defs) => return Ok(defs),
            Err(e) => {
                last_err = format!("attempt {attempt}: {e}");
                eprintln!("[{tag}] 角色抽取失败，重试: {last_err}");
                // 中继侧 5xx（524/1101/auth_unavailable）多为容量抖动，退避后再试
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
            }
        }
    }
    Err(format!("角色抽取 3 次均失败，最后错误: {last_err}"))
}

/// 跑 MVU 五合一翻译（带重试；空壳降级视为失败并重试）。`tag` 区分并行日志。
pub async fn run_mvu_translation(
    llm: Arc<dyn LlmClient>,
    character: &Character,
    tag: &str,
) -> Result<MvuTranslation, String> {
    let runtime = AgentRuntime::new(llm, empty_tool_ctx());
    let mut last: Option<MvuTranslation> = None;
    for attempt in 1..=3 {
        let (_tx, cancel) = tokio::sync::watch::channel(false);
        match storyforge_app_meta::analyze_mvu_card(&runtime, character, cancel).await {
            Ok(t) if !is_empty_fallback(&t) => return Ok(t),
            Ok(t) => {
                eprintln!("[{tag}] attempt {attempt}: 分析返回降级空壳（confidence=0），重试");
                last = Some(t);
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
            }
            Err(e) => {
                eprintln!("[{tag}] attempt {attempt}: 分析出错重试: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
            }
        }
    }
    match last {
        Some(t) => Ok(t), // 交给验收断言去 fail，保留产物便于诊断
        None => Err("MVU 分析 3 次均出错".into()),
    }
}

/// 翻译产物验收检查
pub fn translation_checks(
    defs: &[CharacterDefinition],
    translation: &MvuTranslation,
    exp: &CardExpectations,
) -> Vec<Check> {
    let mut out = Vec::new();

    out.push(check(
        "extract.min_definitions",
        defs.len() >= exp.min_definitions,
        format!("{} 个角色定义（下限 {}）", defs.len(), exp.min_definitions),
    ));
    let empty_persona = defs.iter().filter(|d| d.persona_prompt.trim().is_empty()).count();
    out.push(check(
        "extract.personas_nonempty",
        empty_persona == 0,
        format!("{empty_persona} 个定义 persona 为空（应为 0）"),
    ));
    if !exp.expected_names.is_empty() {
        let found: Vec<&String> = exp
            .expected_names
            .iter()
            .filter(|n| defs.iter().any(|d| d.name.contains(n.as_str())))
            .collect();
        out.push(check(
            "extract.expected_names",
            found.len() >= exp.min_expected_names,
            format!(
                "命中 {}/{}（要求 ≥{}）: {:?}",
                found.len(),
                exp.expected_names.len(),
                exp.min_expected_names,
                found
            ),
        ));
    }

    out.push(check(
        "mvu.not_empty_fallback",
        !is_empty_fallback(translation),
        format!(
            "confidence={:.2}，bindings={}，rules={}",
            translation.analysis_confidence,
            translation.ui_bindings.len(),
            translation.update_rules.len()
        ),
    ));
    out.push(check(
        "mvu.schema_fields",
        translation.variable_schema.len() >= exp.min_schema_fields,
        format!(
            "{} 个 schema 字段（下限 {}）",
            translation.variable_schema.len(),
            exp.min_schema_fields
        ),
    ));
    let schema_text: String = translation
        .variable_schema
        .iter()
        .map(|f| format!("{} {}", f.key, f.label))
        .collect::<Vec<_>>()
        .join(" ");
    let keyword_hit = exp
        .schema_keyword_any
        .iter()
        .find(|k| schema_text.contains(k.as_str()));
    out.push(check(
        "mvu.schema_from_card",
        keyword_hit.is_some() || exp.schema_keyword_any.is_empty(),
        format!(
            "schema 关键词命中: {:?}（候选 {:?}）",
            keyword_hit, exp.schema_keyword_any
        ),
    ));
    out.push(check(
        "mvu.update_rules",
        translation.update_rules.len() >= exp.min_update_rules,
        format!(
            "{} 条更新规则（下限 {}）",
            translation.update_rules.len(),
            exp.min_update_rules
        ),
    ));
    out.push(check(
        "mvu.routing_valid",
        matches!(
            translation.routing,
            MvuRouting::Native | MvuRouting::Hybrid { .. }
        ),
        format!("routing={:?}", translation.routing),
    ));
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// 证据落盘（脱敏）
// ═══════════════════════════════════════════════════════════════════════════

/// 单次（卡 × 模型）验收证据。脱敏：不含卡片正文/人设文本，只有计数、
/// 字段键名、角色名、断言结果。
#[derive(Debug, Serialize)]
pub struct AcceptanceEvidence {
    pub card_label: String,
    pub model: String,
    pub timestamp_utc: String,
    pub all_passed: bool,
    pub checks: Vec<Check>,
    pub stats: EvidenceStats,
}

#[derive(Debug, Serialize)]
pub struct EvidenceStats {
    pub book_entries: usize,
    pub book_enabled: usize,
    pub definitions: usize,
    pub definition_names: Vec<String>,
    pub schema_fields: usize,
    pub schema_keys_sample: Vec<String>,
    pub update_rules: usize,
    pub ui_bindings: usize,
    pub interactions: usize,
    pub fallback_fragments: usize,
    pub routing: String,
    pub confidence: f64,
    pub components_total: usize,
    pub components_by_disposition: std::collections::BTreeMap<String, usize>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_evidence(
    card_label: &str,
    model: &str,
    character: &Character,
    defs: &[CharacterDefinition],
    translation: &MvuTranslation,
    components: &[ComponentDisposition],
    checks: &[Check],
) -> AcceptanceEvidence {
    let book = character.embedded_world_info.as_ref();
    let mut by_disp = std::collections::BTreeMap::new();
    for c in components {
        *by_disp.entry(c.disposition.clone()).or_insert(0usize) += 1;
    }
    AcceptanceEvidence {
        card_label: card_label.into(),
        model: model.into(),
        timestamp_utc: chrono::Utc::now().to_rfc3339(),
        all_passed: checks.iter().all(|c| c.pass),
        checks: checks.to_vec(),
        stats: EvidenceStats {
            book_entries: book.map(|b| b.entries.len()).unwrap_or(0),
            book_enabled: book
                .map(|b| b.entries.iter().filter(|e| !e.disabled).count())
                .unwrap_or(0),
            definitions: defs.len(),
            definition_names: defs.iter().map(|d| d.name.clone()).collect(),
            schema_fields: translation.variable_schema.len(),
            schema_keys_sample: translation
                .variable_schema
                .iter()
                // 512：足够容纳整棵 schema 键集（forge 差分 V2-2 的覆盖率
                // 分析需要全量键；键名无正文，脱敏性质不变）
                .take(512)
                .map(|f| f.key.clone())
                .collect(),
            update_rules: translation.update_rules.len(),
            ui_bindings: translation.ui_bindings.len(),
            interactions: translation.interactions.len(),
            fallback_fragments: translation.fallback_fragments.len(),
            routing: format!("{:?}", translation.routing),
            confidence: translation.analysis_confidence,
            components_total: components.len(),
            components_by_disposition: by_disp,
        },
    }
}

/// 写证据 JSON，返回写入路径
pub fn write_evidence(evidence: &AcceptanceEvidence) -> Result<PathBuf, String> {
    let dir = std::env::var("STORYFORGE_CT_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join("artifacts").join("card-translation"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("建证据目录失败: {e}"))?;
    let safe_model = evidence.model.replace(['/', ':', ' '], "_");
    let path = dir.join(format!(
        "{}-{}.json",
        evidence.card_label.replace(' ', "_"),
        safe_model
    ));
    let json = serde_json::to_string_pretty(evidence).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写证据失败: {e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_first_http_url_stops_at_quote() {
        assert_eq!(
            extract_first_http_url("$('body').load('https://cdn.example/a.html')"),
            Some("https://cdn.example/a.html".into())
        );
        assert_eq!(extract_first_http_url("no url"), None);
    }

    #[test]
    fn empty_fallback_is_detected() {
        let t = MvuTranslation::pure_data_fallback(vec![]);
        assert!(is_empty_fallback(&t));
    }
}

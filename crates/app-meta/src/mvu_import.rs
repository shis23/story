//! MVU 五合一分析编排 + 输出解析（对应 AGENT_INTERFACES §8.3，设计 §19.4）
//!
//! 流程：
//! 1. [`analyze_mvu_card`] 调 AgentRuntime::run_tool_loop 跑 MVU 分析 Agent
//! 2. [`parse_mvu_translation_from_response`] 5 层兜底解析输出
//! 3. 解析失败 → 返回 [`MvuTranslation::pure_data_fallback`]（降级，不阻塞用户）
//!
//! 调用方（tauri-app）负责：构造 AgentRuntime、调本模块、持久化结果。
//!
//! 同时提供 ST 预设 LLM 分类（[`classify_st_preset_with_llm`]），增强现有纯启发式 bridge。

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::{info, warn};

use storyforge_app_agent::runtime::AgentRuntime;
use storyforge_app_agent::tools::ToolRegistry;

use crate::prompts::{build_mvu_analyzer_user_msg, make_mvu_analyzer_config, register_mvu_tools};
use storyforge_domain::character::Character;
use storyforge_domain::llm::ChatResponse;
use storyforge_domain::mvu_translation::{
    BindingDisplay, CardComplexityReport, FallbackFragment, InteractionAction, InteractionMapping,
    MvuRouting, MvuTranslation, UiBinding, score_card_complexity,
};
use storyforge_domain::preset::Preset;
use storyforge_domain::variables::{
    VariableField, VariableType, extract_mvu_schema_from_extensions,
};

use crate::MetaError;

// ═══════════════════════════════════════════════════════════════════════════
// MVU 五合一分析
// ═══════════════════════════════════════════════════════════════════════════

/// 跑 MVU 五合一分析 Agent，产出 [`MvuTranslation`]。
///
/// 内部先跑启发式打分（[`score_card_complexity`]），把打分报告喂给 LLM 做元素级判定。
/// 解析失败时降级返回 [`MvuTranslation::pure_data_fallback`]（不报错，不阻塞用户）。
pub async fn analyze_mvu_card(
    runtime: &AgentRuntime,
    character: &Character,
    cancel: watch::Receiver<bool>,
) -> Result<MvuTranslation, MetaError> {
    // 1. 启发式打分（纯 Rust，无 LLM）
    let complexity =
        score_card_complexity(character.renderable_assets.as_ref(), &character.extensions);
    // 2. P1 已探测的字段 schema（让 LLM 复用/扩展）
    let field_schema = extract_mvu_schema_from_extensions(&character.extensions);

    info!(
        target: "mvu-import",
        "开始分析卡「{}」的 MVU，启发式分类: {:?}",
        character.name, complexity.classification
    );

    // 3. 纯数据卡短路：无 JS 且无 extensions 数据，直接降级，省 LLM 调用
    if matches!(
        complexity.classification,
        storyforge_domain::mvu_translation::CardComplexity::PureData
    ) && field_schema.is_empty()
    {
        info!(target: "mvu-import", "卡「{}」无 JS 无 MVU 数据，跳过 LLM 分析", character.name);
        return Ok(MvuTranslation::pure_data_fallback(vec![]));
    }

    // 4. 跑 LLM 分析
    let config = make_mvu_analyzer_config();
    let user_msg = build_mvu_analyzer_user_msg(character, &complexity, &field_schema);

    let mut registry = ToolRegistry::new();
    register_mvu_tools(&mut registry);

    let resp = match runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(target: "mvu-import", "LLM 调用失败，降级: {}", e);
            return Ok(MvuTranslation::pure_data_fallback(field_schema));
        }
    };

    // 5. 解析（5 层兜底）
    match parse_mvu_translation_from_response(&resp, &field_schema) {
        Ok(translation) => {
            info!(
                target: "mvu-import",
                "卡「{}」MVU 分析成功：{} 绑定 / {} 规则 / {} 兜底片段，路由 {:?}",
                character.name,
                translation.ui_bindings.len(),
                translation.update_rules.len(),
                translation.fallback_fragments.len(),
                translation.routing
            );
            Ok(translation)
        }
        Err(e) => {
            warn!(target: "mvu-import", "解析失败，降级: {}", e);
            Ok(MvuTranslation::pure_data_fallback(field_schema))
        }
    }
}

/// 启发式打分单跑入口（给前端展示打分报告用，不调 LLM）
pub fn score_card(character: &Character) -> CardComplexityReport {
    score_card_complexity(character.renderable_assets.as_ref(), &character.extensions)
}

// ═══════════════════════════════════════════════════════════════════════════
// 5 层兜底解析
// ═══════════════════════════════════════════════════════════════════════════

/// 解析 MVU 分析输出（5 层兜底，照搬 character_extractor 模式）。
///
/// 失败返回 Err，调用方决定是否降级。
pub fn parse_mvu_translation_from_response(
    resp: &ChatResponse,
    field_schema: &[VariableField],
) -> Result<MvuTranslation, String> {
    // 层 1：emit_mvu_translation 工具调用
    for tc in &resp.tool_calls {
        if tc.function.name == "emit_mvu_translation"
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
            && let Ok(t) = parse_mvu_from_value(&val, field_schema)
        {
            return Ok(t);
        }
    }

    // 层 2-5：从 content 提取 JSON 对象
    let content = resp.content.trim();
    if !content.is_empty()
        && let Some(t) = parse_mvu_from_content(content, field_schema)
    {
        return Ok(t);
    }

    Err(format!(
        "5 层兜底全miss；content 前 200 字: {}",
        content.chars().take(200).collect::<String>()
    ))
}

fn parse_mvu_from_content(content: &str, field_schema: &[VariableField]) -> Option<MvuTranslation> {
    // 层 2：整个 content 是 JSON 对象
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(content)
        && let Ok(t) = parse_mvu_from_value(&val, field_schema)
    {
        return Some(t);
    }

    // 层 3：```json 代码块
    if let Some(extracted) = storyforge_app_agent::llm_parse::extract_codeblock(content, "json")
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&extracted)
        && let Ok(t) = parse_mvu_from_value(&val, field_schema)
    {
        return Some(t);
    }

    // 层 4：裸代码块
    if let Some(extracted) = storyforge_app_agent::llm_parse::extract_codeblock(content, "")
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&extracted)
        && let Ok(t) = parse_mvu_from_value(&val, field_schema)
    {
        return Some(t);
    }

    // 层 5：括号配平（委托公共模块，逐个 `{` 尝试，比"只试第一个"更健壮）
    if let Some(t) = storyforge_app_agent::llm_parse::try_each_braces(content, |candidate| {
        serde_json::from_str::<serde_json::Value>(candidate)
            .ok()
            .and_then(|val| parse_mvu_from_value(&val, field_schema).ok())
    }) {
        return Some(t);
    }

    None
}

/// 把 JSON Value 解析成 MvuTranslation（字段对齐 domain 结构）
fn parse_mvu_from_value(
    val: &serde_json::Value,
    field_schema: &[VariableField],
) -> Result<MvuTranslation, String> {
    // 先看是否包了一层（{result: {...}} / {mvu_translation: {...}} 等）。
    // 注意：必须在直接反序列化之前判断，因为 MvuTranslationRaw 所有字段都有 default，
    // 直接解析 {result:{...}} 会得到一个"合法但全空"的结构，吞掉真正的内容。
    for wrapper in ["result", "mvu_translation", "translation", "data"] {
        if let Some(inner) = val.get(wrapper)
            && let Ok(t) = serde_json::from_value::<MvuTranslationRaw>(inner.clone())
        {
            return Ok(t.into_translation(field_schema));
        }
    }

    // 直接反序列化（serde tag 已对齐）
    match serde_json::from_value::<MvuTranslationRaw>(val.clone()) {
        Ok(t) => Ok(t.into_translation(field_schema)),
        Err(e) => Err(format!("无法解析为 MvuTranslation: {e}")),
    }
}

/// 容错反序列化结构（字段大多 Option/default，避免 LLM 漏字段就失败）
#[derive(Debug, Deserialize)]
struct MvuTranslationRaw {
    #[serde(default)]
    variable_schema: Vec<VariableFieldRaw>,
    #[serde(default)]
    ui_bindings: Vec<UiBindingRaw>,
    #[serde(default)]
    update_rules: Vec<String>,
    #[serde(default)]
    interactions: Vec<InteractionMappingRaw>,
    #[serde(default)]
    fallback_fragments: Vec<FallbackFragmentRaw>,
    #[serde(default = "default_native_routing")]
    routing: RoutingRaw,
    #[serde(default = "default_confidence")]
    analysis_confidence: f64,
    #[serde(default)]
    notes: Vec<String>,
}

fn default_native_routing() -> RoutingRaw {
    RoutingRaw {
        kind: "native".into(),
        webview_reason: None,
    }
}

fn default_confidence() -> f64 {
    0.5
}

#[derive(Debug, Deserialize)]
struct RoutingRaw {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    webview_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VariableFieldRaw {
    #[serde(default)]
    key: String,
    #[serde(default)]
    label: String,
    #[serde(default = "default_string_type")]
    value_type: String,
    #[serde(default)]
    default: serde_json::Value,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

fn default_string_type() -> String {
    "string".into()
}

#[derive(Debug, Deserialize)]
struct UiBindingRaw {
    #[serde(default)]
    element: String,
    #[serde(default)]
    variable_key: String,
    display: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct InteractionMappingRaw {
    #[serde(default)]
    element_label: String,
    #[serde(default)]
    actions: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct FallbackFragmentRaw {
    #[serde(default)]
    description: String,
    #[serde(default)]
    js_snippet: String,
    #[serde(default)]
    reason: String,
}

impl MvuTranslationRaw {
    fn into_translation(self, field_schema: &[VariableField]) -> MvuTranslation {
        // variable_schema：LLM 输出优先，空则用 P1 已探测字段
        let variable_schema = if self.variable_schema.is_empty() {
            field_schema.to_vec()
        } else {
            self.variable_schema
                .into_iter()
                .map(|r| r.into_field())
                .collect()
        };

        let ui_bindings: Vec<UiBinding> = self
            .ui_bindings
            .into_iter()
            .filter(|b| !b.element.is_empty() && !b.variable_key.is_empty())
            .map(|b| UiBinding {
                element: b.element,
                variable_key: b.variable_key,
                display: parse_display(&b.display),
            })
            .collect();

        let interactions: Vec<InteractionMapping> = self
            .interactions
            .into_iter()
            .filter(|m| !m.element_label.is_empty())
            .map(|m| InteractionMapping {
                element_label: m.element_label,
                actions: m.actions.iter().map(parse_action).collect(),
            })
            .collect();

        let fallback_fragments: Vec<FallbackFragment> = self
            .fallback_fragments
            .into_iter()
            .map(|f| FallbackFragment {
                description: f.description,
                js_snippet: f.js_snippet,
                reason: f.reason,
            })
            .collect();

        // routing：根据 kind + 是否有 fallback 校正
        let has_fallback = !fallback_fragments.is_empty();
        let routing = match self.routing.kind.to_lowercase().as_str() {
            "hybrid" => MvuRouting::Hybrid {
                webview_reason: self
                    .routing
                    .webview_reason
                    .unwrap_or_else(|| "卡含未翻译 JS 片段".into()),
            },
            _ => {
                if has_fallback {
                    // 自相矛盾：说 native 却有 fallback，纠正为 hybrid
                    MvuRouting::Hybrid {
                        webview_reason: "存在 fallback_fragments 但 routing 标 native，已纠正"
                            .into(),
                    }
                } else {
                    MvuRouting::Native
                }
            }
        };

        MvuTranslation {
            variable_schema,
            ui_bindings,
            update_rules: self.update_rules,
            interactions,
            fallback_fragments,
            routing,
            analysis_confidence: self.analysis_confidence.clamp(0.0, 1.0),
            notes: self.notes,
        }
    }
}

impl VariableFieldRaw {
    fn into_field(self) -> VariableField {
        VariableField {
            key: self.key,
            label: self.label,
            value_type: parse_value_type(&self.value_type, &self.default),
            default: if self.default.is_null() {
                serde_json::Value::String(String::new())
            } else {
                self.default
            },
            description: self.description,
            group: self.group,
        }
    }
}

fn parse_value_type(s: &str, default: &serde_json::Value) -> VariableType {
    match s.to_lowercase().as_str() {
        "int" | "integer" | "number" => match default {
            serde_json::Value::Number(n) if n.is_i64() => VariableType::Int,
            serde_json::Value::Number(_) => VariableType::Float,
            _ => VariableType::Int,
        },
        "float" | "double" => VariableType::Float,
        "bool" | "boolean" => VariableType::Bool,
        "json" | "object" | "array" => VariableType::Json,
        _ => match default {
            serde_json::Value::Number(n) if n.is_i64() => VariableType::Int,
            serde_json::Value::Number(_) => VariableType::Float,
            serde_json::Value::Bool(_) => VariableType::Bool,
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => VariableType::Json,
            _ => VariableType::String,
        },
    }
}

fn parse_display(val: &serde_json::Value) -> BindingDisplay {
    let kind = val
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("text")
        .to_lowercase();
    match kind.as_str() {
        "bar" => {
            let max = val.get("max").and_then(|v| v.as_f64()).unwrap_or(100.0);
            BindingDisplay::Bar { max }
        }
        "tag" => BindingDisplay::Tag,
        "icon" => {
            let mapping = val
                .get("mapping")
                .and_then(|v| v.as_object())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            BindingDisplay::Icon { mapping }
        }
        _ => BindingDisplay::Text,
    }
}

fn parse_action(val: &serde_json::Value) -> InteractionAction {
    let kind = val
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("modify_variable")
        .to_lowercase();
    match kind.as_str() {
        "trigger_next_turn" => InteractionAction::TriggerNextTurn {
            hint: val
                .get("hint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "multi" => {
            let actions = val
                .get("actions")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().map(parse_action).collect())
                .unwrap_or_default();
            InteractionAction::Multi { actions }
        }
        "run_original_js" => InteractionAction::RunOriginalJs {
            js_snippet: val
                .get("js_snippet")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            description: val
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        _ => InteractionAction::ModifyVariable {
            key: val
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            value_expr: val
                .get("value_expr")
                .or_else(|| val.get("value"))
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default(),
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ST 预设 LLM 分类
// ═══════════════════════════════════════════════════════════════════════════

/// ST 预设分类结果（每条 prompt 的归类建议）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StPresetClassification {
    pub preset_name: String,
    /// 每条 prompt 的分类
    pub items: Vec<PromptClassification>,
    /// 无法自动归类的，标记待确认
    pub pending_count: usize,
    /// 建议分配到哪个 Agent
    pub agent_suggestions: Vec<AgentSuggestion>,
}

/// 单条 ST prompt 的分类
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptClassification {
    pub identifier: String,
    pub name: String,
    /// 归入的模块组：perspective / cot / style / quality / output / pending
    pub category: String,
    /// 建议挂到哪个 Agent：Director / Editor / Subagent / Pending
    pub suggested_agent: String,
    /// 置信度 0-1
    pub confidence: f64,
    /// 理由（可解释）
    pub reason: String,
}

/// Agent 分配建议汇总
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSuggestion {
    pub agent: String,
    pub module_count: usize,
    pub summary: String,
}

/// 跑 ST 预设 LLM 分类（增强现有纯启发式 bridge）
///
/// 失败时返回 Err，调用方降级到现有 `import_preset_as_modules` 的纯启发式。
pub async fn classify_st_preset_with_llm(
    runtime: &AgentRuntime,
    preset: &Preset,
    cancel: watch::Receiver<bool>,
) -> Result<StPresetClassification, MetaError> {
    use storyforge_app_agent::AgentConfig;
    use storyforge_domain::agent::AgentRole;

    let config = AgentConfig {
        role: AgentRole::Meta,
        system_prompt: ST_CLASSIFY_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: 8,
        model: "deepseek-chat".to_string(),
        tools: vec![],
        terminal_tools: vec![],
    };

    let user_msg = build_st_classify_user_msg(preset);

    let registry = ToolRegistry::new(); // 不需要工具，纯文本输出

    let resp = runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
        .await
        .map_err(|e| MetaError::ExecutionFailed(format!("LLM 调用失败: {e}")))?;

    parse_st_classification(&resp, preset)
}

fn parse_st_classification(
    resp: &ChatResponse,
    preset: &Preset,
) -> Result<StPresetClassification, MetaError> {
    // 5 层兜底（复用同样的 codeblock 提取 + 括号配平逻辑）
    let parse_from = |val: &serde_json::Value| -> Option<StPresetClassification> {
        serde_json::from_value::<StPresetClassificationDto>(val.clone())
            .ok()
            .map(|dto| dto.into_classification(&preset.name))
    };

    // 层 1-4：整体 JSON / ```json / 裸代码块 / 括号配平（委托公共模块）
    let content = resp.content.trim();
    let parse_text = |s: &str| -> Option<StPresetClassification> {
        serde_json::from_str::<serde_json::Value>(s)
            .ok()
            .and_then(|val| parse_from(&val))
    };
    if let Some(c) = storyforge_app_agent::llm_parse::parse_from_content(content, parse_text) {
        return Ok(c);
    }

    Err(MetaError::ExecutionFailed(format!(
        "ST 分类解析失败；content 前 200 字: {}",
        content.chars().take(200).collect::<String>()
    )))
}

#[derive(Debug, Deserialize)]
struct StPresetClassificationDto {
    #[serde(default)]
    items: Vec<PromptClassificationDto>,
}

#[derive(Debug, Deserialize)]
struct PromptClassificationDto {
    #[serde(default)]
    identifier: String,
    #[serde(default)]
    name: String,
    #[serde(default = "default_pending")]
    category: String,
    #[serde(default = "default_pending")]
    suggested_agent: String,
    #[serde(default = "default_half_confidence")]
    confidence: f64,
    #[serde(default)]
    reason: String,
}

fn default_pending() -> String {
    "pending".into()
}

fn default_half_confidence() -> f64 {
    0.5
}

impl StPresetClassificationDto {
    fn into_classification(self, preset_name: &str) -> StPresetClassification {
        let items: Vec<PromptClassification> = self
            .items
            .into_iter()
            .map(|d| PromptClassification {
                identifier: d.identifier,
                name: d.name,
                category: d.category,
                suggested_agent: d.suggested_agent,
                confidence: d.confidence.clamp(0.0, 1.0),
                reason: d.reason,
            })
            .collect();

        let pending_count = items.iter().filter(|i| i.category == "pending").count();

        // 汇总 Agent 建议
        let mut agent_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for it in &items {
            *agent_counts.entry(it.suggested_agent.clone()).or_default() += 1;
        }
        let agent_suggestions: Vec<AgentSuggestion> = agent_counts
            .into_iter()
            .map(|(agent, count)| AgentSuggestion {
                summary: format!("建议 {} 个模块挂到 {}", count, agent),
                agent,
                module_count: count,
            })
            .collect();

        StPresetClassification {
            preset_name: preset_name.to_string(),
            items,
            pending_count,
            agent_suggestions,
        }
    }
}

const ST_CLASSIFY_SYSTEM_PROMPT: &str = r#"你是 ST 预设分类助手（preset classifier）。给你一个 SillyTavern 预设里的所有 prompt，你的任务是逐条归类。

【6 大模块组】
- perspective：人称/视角（含"人称""视角""第一/二/三人称"）
- cot：思维链（含模型名 Gemini/Claude/GLM + 思维链指引）
- style：文风（含"文风""叙事""白描""轻小说"）
- quality：质量约束（含"杀""禁""反""抗"的负面约束）
- output：输出规范（含"字数""格式""语言""对白"）
- pending：无法判断，待人工确认

【Agent 分配规则】
- Director（导演）：推剧情 / NPC 引入 / 世界书增强（role=system 且靠前）
- Editor（编剧）：文风 / 视角 / 输出规范（role=system 且靠后，jailbreak 区）
- Subagent（子 Agent）：角色扮演基准 / {{char}} 行为指引
- Pending：无法判断

【输出格式】
直接输出 JSON（不要多余解释）：
{
  "items": [
    {
      "identifier": "main-prompt",
      "name": "主提示词",
      "category": "style",
      "suggested_agent": "Editor",
      "confidence": 0.9,
      "reason": "含文风相关内容"
    }
  ]
}

confidence 取值 0-1，低置信（<0.5）的归 pending 并在 reason 说明。"#;

fn build_st_classify_user_msg(preset: &Preset) -> String {
    let mut parts = Vec::new();
    parts.push(format!("【预设名】{}", preset.name));
    parts.push(format!("【prompt 总数】{}", preset.prompts.len()));

    let mut s = String::from("【prompt 清单】\n");
    for (i, p) in preset.prompts.iter().enumerate() {
        if p.marker {
            continue; // 跳过 marker
        }
        let content_preview: String = p.content.chars().take(300).collect();
        s.push_str(&format!(
            "--- #{} identifier={} name={} role={:?} enabled={} ---\n{}\n",
            i + 1,
            p.identifier,
            p.name,
            p.role,
            p.enabled,
            content_preview
        ));
    }
    parts.push(s);

    if !preset.regex_scripts.is_empty() {
        parts.push(format!(
            "（预设还含 {} 个正则脚本，本次不分类）",
            preset.regex_scripts.len()
        ));
    }

    parts.push("请按指定 JSON 格式输出每条 prompt 的分类。".into());
    parts.join("\n\n")
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Source;
    use storyforge_domain::character::RenderableAssets;
    use storyforge_domain::llm::{ChatResponse, ToolCall};

    fn sample_mvu_json() -> &'static str {
        r#"{
  "variable_schema": [
    {"key": "hp", "label": "生命值", "value_type": "int", "default": 100}
  ],
  "ui_bindings": [
    {"element": "hp_bar", "variable_key": "hp", "display": {"kind": "bar", "max": 100}},
    {"element": "mp_text", "variable_key": "mp", "display": {"kind": "text"}}
  ],
  "update_rules": ["受伤时 hp 减少伤害值"],
  "interactions": [
    {"element_label": "攻击", "actions": [{"kind": "modify_variable", "key": "hp", "value_expr": "hp - 10"}]}
  ],
  "fallback_fragments": [
    {"description": "战斗动画", "js_snippet": "playAnim()", "reason": "DOM 动画"}
  ],
  "routing": {"kind": "hybrid", "webview_reason": "战斗动画"},
  "analysis_confidence": 0.8,
  "notes": ["战斗部分需验证"]
}"#
    }

    fn make_response(content: &str, tool_calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            content: content.into(),
            tool_calls,
            finish_reason: Some("stop".into()),
            usage: None,
        }
    }

    // ── 5 层兜底解析 ──────────────────────────────────────────────────

    #[test]
    fn test_parse_layer1_tool_call() {
        let tc = ToolCall {
            id: "1".into(),
            call_type: "function".into(),
            function: storyforge_domain::llm::FunctionCall {
                name: "emit_mvu_translation".into(),
                arguments: sample_mvu_json().into(),
            },
        };
        let resp = make_response("", vec![tc]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert_eq!(t.ui_bindings.len(), 2);
        assert_eq!(t.fallback_fragments.len(), 1);
        assert_eq!(
            t.routing,
            MvuRouting::Hybrid {
                webview_reason: "战斗动画".into()
            }
        );
        assert!((t.analysis_confidence - 0.8).abs() < 1e-9);
    }

    #[test]
    fn test_parse_layer2_whole_content_json() {
        let resp = make_response(sample_mvu_json(), vec![]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert_eq!(t.ui_bindings.len(), 2);
        assert!(!t.update_rules.is_empty());
    }

    #[test]
    fn test_parse_layer3_json_codeblock() {
        let content = format!("好的，分析结果如下：\n```json\n{}\n```", sample_mvu_json());
        let resp = make_response(&content, vec![]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert_eq!(t.ui_bindings.len(), 2);
    }

    #[test]
    fn test_parse_layer4_bare_codeblock() {
        let content = format!("```\n{}\n```", sample_mvu_json());
        let resp = make_response(&content, vec![]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert_eq!(t.variable_schema.len(), 1);
    }

    #[test]
    fn test_parse_layer5_brace_matching() {
        // 前后带噪声文本，整体不是合法 JSON
        let content = format!("分析完成，结果：{} 以上。", sample_mvu_json());
        let resp = make_response(&content, vec![]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert_eq!(t.ui_bindings.len(), 2);
    }

    #[test]
    fn test_parse_empty_variable_schema_falls_back_to_field_schema() {
        // LLM 没给 variable_schema，应回退到 P1 字段
        let json = r#"{
          "variable_schema": [],
          "ui_bindings": [],
          "update_rules": [],
          "interactions": [],
          "fallback_fragments": [],
          "routing": {"kind": "native"}
        }"#;
        let resp = make_response(json, vec![]);
        let field = vec![VariableField {
            key: "hp".into(),
            label: "HP".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(100),
            description: None,
            group: None,
        }];
        let t = parse_mvu_translation_from_response(&resp, &field).unwrap();
        assert_eq!(t.variable_schema.len(), 1);
        assert_eq!(t.variable_schema[0].key, "hp");
    }

    #[test]
    fn test_parse_routing_native_with_fallback_corrected_to_hybrid() {
        let json = r#"{
          "variable_schema": [],
          "ui_bindings": [],
          "update_rules": [],
          "interactions": [],
          "fallback_fragments": [{"description": "x", "js_snippet": "y", "reason": "z"}],
          "routing": {"kind": "native"}
        }"#;
        let resp = make_response(json, vec![]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert!(matches!(t.routing, MvuRouting::Hybrid { .. }));
    }

    #[test]
    fn test_parse_confidence_clamped() {
        let json = r#"{
          "variable_schema": [], "ui_bindings": [], "update_rules": [],
          "interactions": [], "fallback_fragments": [],
          "routing": {"kind": "native"},
          "analysis_confidence": 1.5
        }"#;
        let resp = make_response(json, vec![]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert_eq!(t.analysis_confidence, 1.0);
    }

    #[test]
    fn test_parse_wrapper_unwrap() {
        let wrapped = format!("{{\"result\": {}}}", sample_mvu_json());
        let resp = make_response(&wrapped, vec![]);
        let t = parse_mvu_translation_from_response(&resp, &[]).unwrap();
        assert_eq!(t.ui_bindings.len(), 2);
    }

    // ── 端到端 mock LLM ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_analyze_mvu_card_with_mock() {
        use storyforge_app_agent::runtime::AgentRuntime;
        use storyforge_app_agent::tools::ToolContext;
        use storyforge_infra_llm::mock_client::{MockLlmClient, MockScript};

        let mock: std::sync::Arc<MockLlmClient> =
            std::sync::Arc::new(MockLlmClient::new(vec![MockScript {
                match_keyword: "卡内状态栏分析".into(),
                response_content: sample_mvu_json().into(),
                tool_calls: vec![],
                stream: false,
            }]));

        let tool_ctx = std::sync::Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
        });
        let runtime = AgentRuntime::new(mock, tool_ctx);

        let card = Character {
            id: storyforge_domain::Id::new(),
            name: "测试卡".into(),
            description: String::new(),
            personality: String::new(),
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
            extensions: serde_json::json!({"mvu": {"initvar": {"hp": 100}}}),
            renderable_assets: Some(RenderableAssets {
                html: None,
                css: None,
                js: Some("_.set('hp', 80);".into()),
                name: "t".into(),
            }),
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        };

        let (_tx, cancel) = watch::channel(false);
        let result = analyze_mvu_card(&runtime, &card, cancel).await.unwrap();
        assert_eq!(result.ui_bindings.len(), 2);
        assert!(matches!(result.routing, MvuRouting::Hybrid { .. }));
    }

    #[tokio::test]
    async fn test_analyze_mvu_card_pure_data_short_circuit() {
        use storyforge_app_agent::runtime::AgentRuntime;
        use storyforge_app_agent::tools::ToolContext;
        use storyforge_infra_llm::mock_client::MockLlmClient;

        // 无 JS 无 extensions 数据 → 短路，不调 LLM
        let mock: std::sync::Arc<MockLlmClient> = std::sync::Arc::new(MockLlmClient::new(vec![]));
        let tool_ctx = std::sync::Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
        });
        let runtime = AgentRuntime::new(mock, tool_ctx);

        let card = Character {
            id: storyforge_domain::Id::new(),
            name: "纯数据卡".into(),
            description: String::new(),
            personality: String::new(),
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

        let (_tx, cancel) = watch::channel(false);
        let result = analyze_mvu_card(&runtime, &card, cancel).await.unwrap();
        // 短路：应该是 pure_data_fallback（空 schema，native routing）
        assert!(matches!(result.routing, MvuRouting::Native));
        assert!(result.ui_bindings.is_empty());
    }

    // ── ST 分类 ──────────────────────────────────────────────────────

    #[test]
    fn test_parse_st_classification_whole_json() {
        use storyforge_domain::preset::Preset;
        let preset = Preset {
            name: "测试预设".into(),
            prompts: vec![],
            regex_scripts: vec![],
            source: Source::ImportedFromST,
        };
        let json = r#"{
          "items": [
            {"identifier": "main", "name": "主", "category": "style", "suggested_agent": "Editor", "confidence": 0.9, "reason": "文风"},
            {"identifier": "x", "name": "未知", "category": "pending", "suggested_agent": "Pending", "confidence": 0.3, "reason": "无法判断"}
          ]
        }"#;
        let resp = make_response(json, vec![]);
        let c = parse_st_classification(&resp, &preset).unwrap();
        assert_eq!(c.items.len(), 2);
        assert_eq!(c.pending_count, 1);
        assert!(!c.agent_suggestions.is_empty());
    }
}

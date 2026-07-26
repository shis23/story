//! MVU 翻译产物 + 卡复杂度启发式打分（对应设计 §19，AGENT_INTERFACES §8）
//!
//! 本章经过多轮推敲，核心认知三次修正：
//!   ① 翻译 = JS 逻辑 → tool-call 描述（不是 → Rust 结构）
//!   ② 渲染（数据绑定）和逻辑（tool-call）彻底分离
//!   ③ 元素级混合（能翻译的翻译，不能的保留 JS 执行）
//!
//! 本文件定义两块纯领域数据（无 IO）：
//!   - [`MvuTranslation`]：Meta Agent 导入时五合一分析的产物（§19.2 / §19.4）
//!   - [`CardComplexityReport`]：纯 Rust 启发式打分，给 Meta Agent 当判据 + 给前端路由建议（§19.5）

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::character::RenderableAssets;
use crate::variables::{VariableField, merge_schema};

// ═══════════════════════════════════════════════════════════════════════════
// Part 1：MvuTranslation（Meta Agent 五合一产物）
// ═══════════════════════════════════════════════════════════════════════════

/// Meta Agent 导入时一次性产出的 MVU 翻译（设计 §19.2 / §19.4）。
///
/// 元素级判定：每条 binding / interaction / fragment 独立决定「能翻译 / 走兜底」，
/// 不强制全卡统一。失败时 Meta Agent 不会产出本结构，调用方走
/// [`MvuTranslation::pure_data_fallback`] 回退到字段级。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuTranslation {
    /// 变量 schema（卡 initvar 解析出来，覆盖/扩展基础表）
    pub variable_schema: Vec<VariableField>,
    /// 数据绑定（UI 元素 ↔ 变量，纯声明式，前端原生画）
    pub ui_bindings: Vec<UiBinding>,
    /// 自然语言规则文本，每轮注入后处理 Agent，让它按规则调 tool 改变量
    pub update_rules: Vec<String>,
    /// 用户交互动作映射（JS 事件 → ToolCallSpec）
    pub interactions: Vec<InteractionMapping>,
    /// 翻译不了的 JS 片段（运行时执行兜底，本次留桩不执行）
    pub fallback_fragments: Vec<FallbackFragment>,
    /// 路由建议（Native 走原生，Hybrid 需共享 WebView 执行 fallback）
    pub routing: MvuRouting,
    /// 整体置信度 0.0-1.0，低值提示用户验证
    pub analysis_confidence: f64,
    /// 备注（低置信项标注、用户可改的提示等）
    #[serde(default)]
    pub notes: Vec<String>,
}

/// 数据绑定：UI 元素 ↔ 变量（声明式，前端原生渲染用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiBinding {
    /// 元素标识，如 "hp_bar" / "mp_text"
    pub element: String,
    /// 绑定的变量键，如 "hp"
    pub variable_key: String,
    /// 渲染方式
    pub display: BindingDisplay,
}

/// 绑定的渲染方式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BindingDisplay {
    /// 进度条（血条），max 是满值
    Bar { max: f64 },
    /// 纯文本
    Text,
    /// 标签（状态 buff）
    Tag,
    /// 图标映射（value → icon url/class）
    Icon {
        #[serde(default)]
        mapping: HashMap<String, String>,
    },
}

/// 用户交互动作映射
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionMapping {
    /// 元素标签，如 "攻击按钮"
    pub element_label: String,
    /// 点击后执行的动作序列
    pub actions: Vec<InteractionAction>,
}

/// 单个交互动作（翻译或兜底）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionAction {
    /// 改变量（value_expr 是表达式/字面量描述，交给前端/后处理解释）
    ModifyVariable { key: String, value_expr: String },
    /// 触发下一轮写作（hint 注入导演）
    TriggerNextTurn { hint: String },
    /// 多个动作组合
    Multi { actions: Vec<InteractionAction> },
    /// 翻译不了的 JS 片段，运行时执行（本次留桩不执行）
    RunOriginalJs {
        js_snippet: String,
        description: String,
    },
}

/// 兜底片段：翻译不了的 JS（运行时执行，本次留桩不执行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackFragment {
    /// 这段 JS 干什么的（人类可读）
    pub description: String,
    /// 原始 JS 代码
    pub js_snippet: String,
    /// 为什么翻译不了
    pub reason: String,
}

/// 路由建议
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MvuRouting {
    /// 全部能原生渲染 + tool-call，无 JS 执行
    Native,
    /// 部分元素需共享 WebView 执行 fallback
    Hybrid {
        /// 为什么需要 WebView（前端展示给用户）
        webview_reason: String,
    },
}

impl MvuTranslation {
    /// 纯降级构造：Meta Agent 分析失败时，回退到字段级 schema，所有 binding/interaction 为空，
    /// routing=Native（仍可用，只是没有状态栏）。保证最坏情况不退步。
    pub fn pure_data_fallback(field_schema: Vec<VariableField>) -> Self {
        Self {
            variable_schema: field_schema,
            ui_bindings: vec![],
            update_rules: vec![],
            interactions: vec![],
            fallback_fragments: vec![],
            routing: MvuRouting::Native,
            analysis_confidence: 0.0,
            notes: vec!["自动降级：Meta Agent 五合一分析未产出，仅保留字段级 schema".into()],
        }
    }

    /// 把本翻译的 variable_schema 与给定基础 schema 合并（extra 覆盖 base 同 key）。
    /// 给导入时把 MVU schema 并进 CharacterDefinition.variable_schema 用。
    pub fn merged_variable_schema(&self, base: &[VariableField]) -> Vec<VariableField> {
        merge_schema(base, &self.variable_schema)
    }

    /// 高玩模式可读的摘要文本（前端/日志展示）
    pub fn render_translation_for_review(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "【MVU 翻译】置信度 {:.0}%  路由: {}\n",
            self.analysis_confidence * 100.0,
            match &self.routing {
                MvuRouting::Native => "原生".into(),
                MvuRouting::Hybrid { webview_reason } =>
                    format!("混合（需 WebView：{}）", webview_reason),
            }
        ));
        out.push_str(&format!("  变量字段: {} 个\n", self.variable_schema.len()));
        out.push_str(&format!("  UI 绑定: {} 个\n", self.ui_bindings.len()));
        out.push_str(&format!("  更新规则: {} 条\n", self.update_rules.len()));
        out.push_str(&format!("  交互映射: {} 个\n", self.interactions.len()));
        out.push_str(&format!(
            "  兜底 JS 片段: {} 个\n",
            self.fallback_fragments.len()
        ));
        if !self.notes.is_empty() {
            out.push_str("  备注:\n");
            for n in &self.notes {
                out.push_str(&format!("    - {}\n", n));
            }
        }
        out
    }

    /// 给前端后处理 Agent 用的更新规则注入文本（无规则返回空串）
    pub fn render_update_rules_for_injection(&self) -> String {
        if self.update_rules.is_empty() {
            return String::new();
        }
        let mut out = String::from(
            "【状态栏更新规则（来自卡的 MVU 翻译，请按规则调 update_variable 工具）】\n",
        );
        for (i, rule) in self.update_rules.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, rule));
        }
        out
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Part 2：卡复杂度启发式打分（纯 Rust，无 LLM，给 Meta Agent 当判据）
// ═══════════════════════════════════════════════════════════════════════════

/// 卡的 JS 复杂度分类（设计 §19.5 三类卡）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CardComplexity {
    /// 纯数据驱动：无 JS 或纯占位，全部原生
    PureData,
    /// 规则驱动：有简单变量脚本（_.set / getLocalVar），可翻译成 tool-call
    RuleDriven,
    /// 重 DOM：document. / innerHTML 大量，需 WebView 兜底
    Heavy,
}

impl CardComplexity {
    /// 对应路由建议
    pub fn suggested_routing(self) -> MvuRouting {
        match self {
            CardComplexity::PureData | CardComplexity::RuleDriven => MvuRouting::Native,
            CardComplexity::Heavy => MvuRouting::Hybrid {
                webview_reason: "卡含大量 DOM 操作 JS，部分元素需共享 WebView 执行".into(),
            },
        }
    }
}

/// 启发式打分报告：统计卡的 JS 特征 + 分类 + 路由建议 + 推理
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardComplexityReport {
    /// 各特征计数
    pub counts: HashMap<String, usize>,
    /// 分类
    pub classification: CardComplexity,
    /// 路由建议（分类映射）
    pub suggested_routing: MvuRouting,
    /// 推理（前端/日志展示，可解释）
    pub reasoning: String,
}

/// 阈值（基于缄默之秋1.4 实测 document.×179 倒推，留足裕度）
const THRESHOLD_DOCUMENT_HEAVY: usize = 20;
const THRESHOLD_INNERHTML_HEAVY: usize = 10;
const THRESHOLD_SCRIPT_BYTES_HEAVY: usize = 5_000;

/// 对卡的 JS 复杂度做纯启发式打分（设计 §19.5）。
///
/// 判定规则（可解释，无 LLM）：
///   - `document.` / `getElementById` / `innerHTML` 任意超过 heavy 阈值 → `Heavy`
///   - script 总字节超过 5KB → `Heavy`
///   - 有 script 但都简单（仅 MVU API 调用） → `RuleDriven`
///   - 无 script 或纯占位 → `PureData`
///
/// 输出 [`CardComplexityReport`]，含 counts 明细，给 Meta Agent 当判据
/// （它在启发式基础上做元素级二次判定，§19.4）。
pub fn score_card_complexity(
    assets: Option<&RenderableAssets>,
    extensions: &serde_json::Value,
) -> CardComplexityReport {
    // 拼接所有 JS 源（renderable_assets.js + extensions 里可能的 depth_prompt / script 块）
    let js_blob = collect_js_blob(assets, extensions);
    let html_blob = assets.and_then(|a| a.html.clone()).unwrap_or_default();

    let document_calls = count_occurrences(&js_blob, "document.");
    let get_by_id_calls = count_occurrences(&js_blob, "getElementById");
    let innerhtml_calls = count_occurrences(&js_blob, "innerHTML");
    let script_blocks = count_script_blocks(&html_blob, &js_blob);
    let script_bytes = js_blob.len();
    let placeholder_calls = count_occurrences(&js_blob, "{{");
    let mvu_set_calls =
        count_occurrences(&js_blob, "_.set") + count_occurrences(&js_blob, "setLocalVar");
    let mvu_get_calls =
        count_occurrences(&js_blob, "_.get") + count_occurrences(&js_blob, "getLocalVar");
    let jq_calls = count_occurrences(&js_blob, "$(");

    let mut counts = HashMap::new();
    counts.insert("document_calls".into(), document_calls);
    counts.insert("get_by_id_calls".into(), get_by_id_calls);
    counts.insert("innerhtml_calls".into(), innerhtml_calls);
    counts.insert("script_blocks".into(), script_blocks);
    counts.insert("script_bytes".into(), script_bytes);
    counts.insert("placeholder_calls".into(), placeholder_calls);
    counts.insert("mvu_set_calls".into(), mvu_set_calls);
    counts.insert("mvu_get_calls".into(), mvu_get_calls);
    counts.insert("jq_calls".into(), jq_calls);

    // 分类判定
    let dom_heavy =
        document_calls >= THRESHOLD_DOCUMENT_HEAVY || innerhtml_calls >= THRESHOLD_INNERHTML_HEAVY;
    let script_heavy = script_bytes >= THRESHOLD_SCRIPT_BYTES_HEAVY;

    let (classification, reasoning) = if dom_heavy || script_heavy {
        let reasons = [
            (
                document_calls >= THRESHOLD_DOCUMENT_HEAVY,
                format!("document.×{}", document_calls),
            ),
            (
                innerhtml_calls >= THRESHOLD_INNERHTML_HEAVY,
                format!("innerHTML×{}", innerhtml_calls),
            ),
            (script_heavy, format!("script {} 字节", script_bytes)),
        ];
        let hit: Vec<String> = reasons
            .into_iter()
            .filter_map(|(hit, msg)| if hit { Some(msg) } else { None })
            .collect();
        (
            CardComplexity::Heavy,
            format!("重 DOM（{}），需共享 WebView 兜底", hit.join(" / ")),
        )
    } else if script_bytes > 0 || mvu_set_calls > 0 || mvu_get_calls > 0 {
        (
            CardComplexity::RuleDriven,
            format!(
                "有简单变量脚本（{} 字节 / _.set×{} / _.get×{}），可翻译为 tool-call",
                script_bytes, mvu_set_calls, mvu_get_calls
            ),
        )
    } else {
        (
            CardComplexity::PureData,
            "无 JS 或纯占位，走原生数据绑定".into(),
        )
    };

    let suggested_routing = classification.suggested_routing();

    CardComplexityReport {
        counts,
        classification,
        suggested_routing,
        reasoning,
    }
}

/// 把 assets.js + extensions 里可能的 JS 源拼到一起（粗略，给打分用）
fn collect_js_blob(assets: Option<&RenderableAssets>, extensions: &serde_json::Value) -> String {
    let mut blob = String::new();
    if let Some(a) = assets
        && let Some(js) = &a.js
    {
        blob.push_str(js);
        blob.push('\n');
    }
    // depth_prompt 可能内嵌 script
    if let Some(dp) = extensions
        .get("depth_prompt")
        .and_then(|v| v.get("prompt"))
        .and_then(|v| v.as_str())
    {
        blob.push_str(dp);
        blob.push('\n');
    }
    // mvu 插件可能内嵌 script 字段
    if let Some(mvu) = extensions.get("mvu")
        && let Some(script) = mvu.get("script").and_then(|v| v.as_str())
    {
        blob.push_str(script);
        blob.push('\n');
    }
    // 新一代卡（命定之诗/卿卿形态）：卡逻辑在 extensions.tavern_helper.scripts，
    // 只统计启用脚本；不统计会把 TH-only 卡误判为 PureData 并短路跳过 LLM 分析
    if let Some(scripts) = extensions
        .get("tavern_helper")
        .and_then(|v| v.get("scripts"))
        .and_then(|v| v.as_array())
    {
        for s in scripts {
            let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            if !enabled {
                continue;
            }
            if let Some(content) = s.get("content").and_then(|v| v.as_str()) {
                blob.push_str(content);
                blob.push('\n');
            }
        }
    }
    blob
}

/// 计数 needle 在 haystack 里出现的次数（非重叠，字节级，够用）
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let n = needle.len();
    let bytes = haystack.as_bytes();
    let needle_b = needle.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i + n <= bytes.len() {
        if &bytes[i..i + n] == needle_b {
            count += 1;
            i += n;
        } else {
            i += 1;
        }
    }
    count
}

/// 粗略数 script 块数（HTML 里 <script> 标签 + js_blob 非空算 1）
fn count_script_blocks(html: &str, js_blob: &str) -> usize {
    let html_count = count_occurrences(html, "<script");
    let js_count = if js_blob.trim().is_empty() { 0 } else { 1 };
    html_count.max(js_count)
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::RenderableAssets;
    use crate::variables::{VariableField, VariableType, default_character_variables};

    fn sample_variable_schema() -> Vec<VariableField> {
        vec![VariableField {
            key: "hp".into(),
            label: "生命值".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(100),
            description: None,
            group: Some("状态".into()),
        }]
    }

    // ── MvuTranslation 结构/序列化 ──────────────────────────────────────

    #[test]
    fn test_mvu_translation_serde_roundtrip() {
        let t = MvuTranslation {
            variable_schema: sample_variable_schema(),
            ui_bindings: vec![UiBinding {
                element: "hp_bar".into(),
                variable_key: "hp".into(),
                display: BindingDisplay::Bar { max: 100.0 },
            }],
            update_rules: vec!["受伤时 hp -= 伤害值".into()],
            interactions: vec![InteractionMapping {
                element_label: "攻击按钮".into(),
                actions: vec![InteractionAction::ModifyVariable {
                    key: "hp".into(),
                    value_expr: "hp - 10".into(),
                }],
            }],
            fallback_fragments: vec![FallbackFragment {
                description: "战斗动画".into(),
                js_snippet: "playAnim()".into(),
                reason: "DOM 动画无法翻译".into(),
            }],
            routing: MvuRouting::Hybrid {
                webview_reason: "战斗动画".into(),
            },
            analysis_confidence: 0.8,
            notes: vec!["低置信：战斗逻辑".into()],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: MvuTranslation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.variable_schema.len(), 1);
        assert_eq!(back.ui_bindings.len(), 1);
        assert_eq!(back.update_rules.len(), 1);
        assert_eq!(back.interactions.len(), 1);
        assert_eq!(back.fallback_fragments.len(), 1);
        assert_eq!(
            back.routing,
            MvuRouting::Hybrid {
                webview_reason: "战斗动画".into()
            }
        );
        assert!((back.analysis_confidence - 0.8).abs() < 1e-9);
    }

    #[test]
    fn test_binding_display_variants_serde() {
        let variants = vec![
            BindingDisplay::Bar { max: 50.5 },
            BindingDisplay::Text,
            BindingDisplay::Tag,
            BindingDisplay::Icon {
                mapping: HashMap::from([("happy".into(), "😀".into())]),
            },
        ];
        for v in variants {
            let b = UiBinding {
                element: "e".into(),
                variable_key: "k".into(),
                display: v.clone(),
            };
            let json = serde_json::to_string(&b).unwrap();
            let back: UiBinding = serde_json::from_str(&json).unwrap();
            assert_eq!(back.display, v);
        }
    }

    #[test]
    fn test_interaction_action_variants_serde() {
        let variants = vec![
            InteractionAction::ModifyVariable {
                key: "hp".into(),
                value_expr: "10".into(),
            },
            InteractionAction::TriggerNextTurn {
                hint: "继续战斗".into(),
            },
            InteractionAction::Multi { actions: vec![] },
            InteractionAction::RunOriginalJs {
                js_snippet: "x()".into(),
                description: "动画".into(),
            },
        ];
        for v in variants {
            let m = InteractionMapping {
                element_label: "b".into(),
                actions: vec![v.clone()],
            };
            let json = serde_json::to_string(&m).unwrap();
            let back: InteractionMapping = serde_json::from_str(&json).unwrap();
            assert_eq!(back.actions.len(), 1);
        }
    }

    #[test]
    fn test_pure_data_fallback_is_native_and_empty_bindings() {
        let t = MvuTranslation::pure_data_fallback(sample_variable_schema());
        assert_eq!(t.routing, MvuRouting::Native);
        assert!(t.ui_bindings.is_empty());
        assert!(t.interactions.is_empty());
        assert!(t.fallback_fragments.is_empty());
        assert_eq!(t.analysis_confidence, 0.0);
        assert!(!t.notes.is_empty());
    }

    #[test]
    fn test_merged_variable_schema_extra_overrides_base() {
        // base 有 hp=100；translation 的 schema 把 hp 改成 200（覆盖）
        let mut override_hp = sample_variable_schema();
        override_hp[0].default = serde_json::json!(200);
        let t = MvuTranslation {
            variable_schema: override_hp,
            ui_bindings: vec![],
            update_rules: vec![],
            interactions: vec![],
            fallback_fragments: vec![],
            routing: MvuRouting::Native,
            analysis_confidence: 1.0,
            notes: vec![],
        };
        let merged = t.merged_variable_schema(&default_character_variables());
        let hp = merged.iter().find(|f| f.key == "hp").unwrap();
        assert_eq!(hp.default, serde_json::json!(200));
        // 基础表的其他字段保留
        assert!(merged.iter().any(|f| f.key == "mp"));
    }

    #[test]
    fn test_render_translation_for_review_format() {
        let t = MvuTranslation {
            variable_schema: sample_variable_schema(),
            ui_bindings: vec![],
            update_rules: vec!["规则1".into()],
            interactions: vec![],
            fallback_fragments: vec![],
            routing: MvuRouting::Native,
            analysis_confidence: 0.9,
            notes: vec![],
        };
        let s = t.render_translation_for_review();
        assert!(s.contains("90%"));
        assert!(s.contains("原生"));
        assert!(s.contains("变量字段: 1"));
        assert!(s.contains("更新规则: 1"));

        let inject = t.render_update_rules_for_injection();
        assert!(inject.contains("规则1"));
        assert!(inject.contains("MVU 翻译"));

        // 空规则时注入文本为空
        let empty = MvuTranslation::pure_data_fallback(vec![]);
        assert!(empty.render_update_rules_for_injection().is_empty());
    }

    // ── 卡复杂度打分 ────────────────────────────────────────────────────

    fn assets_with_js(js: &str) -> RenderableAssets {
        RenderableAssets {
            html: None,
            css: None,
            js: Some(js.into()),
            name: "test".into(),
        }
    }

    #[test]
    fn test_complexity_pure_data_when_no_js() {
        let report = score_card_complexity(None, &serde_json::json!({}));
        assert_eq!(report.classification, CardComplexity::PureData);
        assert_eq!(report.suggested_routing, MvuRouting::Native);
    }

    #[test]
    fn test_complexity_rule_driven_for_simple_mvu_script() {
        // 简单的变量脚本，无 DOM 操作
        let js = "_.set('hp', 80); var x = _.get('mp'); setLocalVar('state', '受伤');";
        let report = score_card_complexity(Some(&assets_with_js(js)), &serde_json::json!({}));
        assert_eq!(report.classification, CardComplexity::RuleDriven);
        assert_eq!(report.suggested_routing, MvuRouting::Native);
        assert_eq!(report.counts.get("mvu_set_calls"), Some(&2)); // _.set + setLocalVar
    }

    #[test]
    fn test_complexity_heavy_for_lots_of_dom_calls() {
        // 模拟缄默之秋量级：大量 document. 调用
        let mut js = String::new();
        for _ in 0..30 {
            js.push_str("document.getElementById('x').innerHTML = 'y';\n");
        }
        let report = score_card_complexity(Some(&assets_with_js(&js)), &serde_json::json!({}));
        assert_eq!(report.classification, CardComplexity::Heavy);
        assert_eq!(
            report.suggested_routing,
            MvuRouting::Hybrid {
                webview_reason: "卡含大量 DOM 操作 JS，部分元素需共享 WebView 执行".into()
            }
        );
        assert!(*report.counts.get("document_calls").unwrap() >= 30);
    }

    #[test]
    fn test_complexity_heavy_for_large_script_blob() {
        // 超过 5KB 的 script
        let js = "x".repeat(6_000);
        let report = score_card_complexity(Some(&assets_with_js(&js)), &serde_json::json!({}));
        assert_eq!(report.classification, CardComplexity::Heavy);
    }

    #[test]
    fn test_complexity_collects_js_from_extensions_depth_prompt() {
        // depth_prompt 里内嵌 script 也应被统计
        let ext = serde_json::json!({
            "depth_prompt": { "prompt": "_.set('hp', 50);" }
        });
        let report = score_card_complexity(None, &ext);
        assert_eq!(report.classification, CardComplexity::RuleDriven);
        assert_eq!(report.counts.get("mvu_set_calls"), Some(&1));
    }
}

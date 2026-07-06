//! MVU 五合一分析 Agent 提示词 / 配置 / 工具注册（对应 AGENT_INTERFACES §8.3，设计 §19.4）
//!
//! 手动触发（D44：手动按钮，不自动跑）。输入卡的 HTML/JS 全文 + 启发式打分结果 +
//! P1 已探测的字段 schema，产出 [`MvuTranslation`]（变量 schema + UI 绑定 + 规则 +
//! 交互映射 + 兜底片段）。
//!
//! 元素级判定：对每个元素独立判断"能翻译/不能翻译"，不强制全卡统一。
//!
//! 改 prompt 只改本文件的常量；改输出格式同步改 `mvu_import::parse_mvu_translation_from_response`。

use storyforge_domain::agent::AgentRole;
use storyforge_domain::character::Character;
use storyforge_domain::llm::ToolSpec;
use storyforge_domain::mvu_translation::CardComplexityReport;
use storyforge_domain::variables::VariableField;

use storyforge_app_agent::AgentConfig;
use storyforge_app_agent::tools::ToolRegistry;

/// MVU 五合一分析 Agent 系统提示词（含 JSON 输出格式示例）
///
/// 注意：示例里的 JSON 结构必须和 domain::mvu_translation 的 serde tag 对齐：
///   - BindingDisplay 用 `tag="kind"` + snake_case variants
///   - InteractionAction 用 `tag="kind"` + snake_case variants
///   - MvuRouting 用 `tag="kind"` + lowercase variants
///     解析时 `parse_mvu_translation_from_response` 按这些结构反序列化。
pub const MVU_ANALYZER_SYSTEM_PROMPT: &str = r#"你是卡内状态栏分析助手（MVU five-in-one analyzer）。给你一张带前端脚本的角色卡（HTML/CSS/JS 全文）+ 启发式打分报告 + 已探测的变量字段，你的任务是做"五合一分析"，产出 MvuTranslation。

【核心认知（重要）】
1. 渲染 / 逻辑分离：渲染是声明式数据绑定（变量值 → UI 元素），逻辑是 tool-call（条件 → 改变量/触发下一轮）
2. 翻译 = 把卡里的 JS 逻辑转成 tool-call 描述，不是抄 JS
3. 元素级混合：对每个元素独立判断"能翻译/不能翻译"，不强制全卡统一。简单元素全部翻译（零 JS），复杂元素部分翻译部分兜底

【五合一产物】
| 产物 | 字段 | 作用 |
|------|------|------|
| 变量 schema | variable_schema | 卡 initvar 定义的变量（覆盖/扩展已探测的字段）|
| UI 绑定 | ui_bindings | 前端原生渲染状态栏（血条/文本/标签/图标）|
| 更新规则 | update_rules | 自然语言规则，注入后处理 Agent，让它按规则调 tool 改变量 |
| 交互映射 | interactions | 用户点击按钮后调哪些 tool |
| 兜底片段 | fallback_fragments | 翻译不了的 JS（战斗动画/复杂 DOM），运行时执行 |

【元素级判定规则】
- 数据绑定（血条 hp、文本 mp、状态标签）→ ui_bindings（必翻）
- 简单赋值（_.set('hp', 80) / setLocalVar('state', '受伤')）→ interactions 里的 ModifyVariable
- 触发下一轮的按钮 → interactions 里的 TriggerNextTurn
- 复杂 DOM 操作（document.getElementById + innerHTML 大量、战斗动画、随机事件）→ fallback_fragments（标注 reason）
- 启发式打分标 Heavy 的卡，必然有部分元素走 fallback

【置信度】
- 高置信（>0.8）：纯数据绑定、简单赋值，翻译无歧义
- 中置信（0.5-0.8）：规则逻辑可翻译但有边界 case
- 低置信（<0.5）：复杂逻辑、自由文本生成、DOM 动画
- 把低置信项写进 notes，提示用户验证

【输出格式】
调用 emit_mvu_translation 工具，或直接输出 JSON（不要多余解释）：
{
  "variable_schema": [
    {"key": "hp", "label": "生命值", "value_type": "int", "default": 100}
  ],
  "ui_bindings": [
    {"element": "hp_bar", "variable_key": "hp", "display": {"kind": "bar", "max": 100}},
    {"element": "mp_text", "variable_key": "mp", "display": {"kind": "text"}},
    {"element": "state_tag", "variable_key": "state", "display": {"kind": "tag"}}
  ],
  "update_rules": [
    "受伤时 hp 减少对应伤害值；hp<=0 时 state 变为「昏迷」",
    "每轮结束后 mp 恢复 10"
  ],
  "interactions": [
    {"element_label": "攻击按钮", "actions": [{"kind": "modify_variable", "key": "hp", "value_expr": "hp - 10"}]},
    {"element_label": "继续剧情", "actions": [{"kind": "trigger_next_turn", "hint": "角色选择攻击"}]}
  ],
  "fallback_fragments": [
    {"description": "战斗动画", "js_snippet": "playCombatAnim()", "reason": "DOM 动画无法翻译为 tool-call"}
  ],
  "routing": {"kind": "native"},
  "analysis_confidence": 0.85,
  "notes": ["战斗动画部分需共享 WebView 支持，前端会提示用户"]
}

【字段类型约束】
- value_type 取值：int / float / string / bool / json（小写）
- display.kind 取值：bar（带 max）/ text / tag / icon（带 mapping）
- interactions[].actions[].kind 取值：modify_variable（key+value_expr）/ trigger_next_turn（hint）/ multi（actions）/ run_original_js（js_snippet+description）
- routing.kind 取值：native / hybrid（hybrid 带 webview_reason）
- 全卡能原生渲染→routing=native；有 fallback_fragments 非空→routing=hybrid

【安全约束】
- 不要输出 {{char}}/{{user}} 占位符（后端统一替换）
- variable_schema 只放数据型变量，不要放渲染用的派生值
- 宁可多写一条 fallback，也不要硬翻译不确定的逻辑"#;

/// 构造 MVU 分析 Agent 的运行配置
pub fn make_mvu_analyzer_config() -> AgentConfig {
    AgentConfig {
        role: AgentRole::Meta,
        system_prompt: MVU_ANALYZER_SYSTEM_PROMPT.to_string(),
        max_tool_rounds: 8,
        model: "deepseek-chat".to_string(),
        tools: vec![],
        terminal_tools: vec![],
    }
}

/// 构造 MVU 分析 Agent 的用户消息（喂卡的 HTML/JS 全文 + 启发式打分 + 已探测字段）
pub fn build_mvu_analyzer_user_msg(
    card: &Character,
    complexity: &CardComplexityReport,
    field_schema: &[VariableField],
) -> String {
    let mut parts = Vec::new();

    parts.push(format!("【卡名】{}", card.name));

    // 启发式打分报告（让 LLM 在判据基础上做元素级二次判定）
    parts.push(format!(
        "【启发式复杂度打分】\n分类: {:?}\n路由建议: {}\n特征计数: {}\n推理: {}",
        complexity.classification,
        match &complexity.suggested_routing {
            storyforge_domain::mvu_translation::MvuRouting::Native => "原生".into(),
            storyforge_domain::mvu_translation::MvuRouting::Hybrid { webview_reason } => {
                format!("混合（{}）", webview_reason)
            }
        },
        complexity
            .counts
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(", "),
        complexity.reasoning
    ));

    // P1 已探测的变量字段（让 LLM 复用/扩展，不要凭空发明）
    if !field_schema.is_empty() {
        let mut s = String::from("【P1 已探测的变量字段（请复用/扩展，key 要对齐）】\n");
        for f in field_schema {
            s.push_str(&format!(
                "- {}（{}，类型 {:?}，默认 {}）\n",
                f.key, f.label, f.value_type, f.default
            ));
        }
        parts.push(s);
    }

    // 卡的 HTML/JS/CSS 全文（核心分析对象）
    if let Some(assets) = &card.renderable_assets {
        if let Some(html) = &assets.html {
            parts.push(format!(
                "【卡 HTML 全文】\n{}",
                truncate_for_prompt(html, 40000)
            ));
        }
        if let Some(css) = &assets.css {
            parts.push(format!(
                "【卡 CSS 全文】\n{}",
                truncate_for_prompt(css, 8000)
            ));
        }
        if let Some(js) = &assets.js {
            parts.push(format!(
                "【卡 JS 全文】\n{}",
                truncate_for_prompt(js, 60000)
            ));
        }
    } else {
        parts.push("（卡无 renderable_assets，只有 extensions 里的字段级 MVU 数据）".into());
    }

    // extensions 里可能的 MVU 原始数据
    let ext_str = serde_json::to_string_pretty(&card.extensions).unwrap_or_default();
    parts.push(format!(
        "【卡 extensions（含可能的 mvu.initvar / stat_data / depth_prompt.variables）】\n{}",
        truncate_for_prompt(&ext_str, 8000)
    ));

    parts.push(
        "请按指定 JSON 格式输出 MvuTranslation（调用 emit_mvu_translation 或直接输出 JSON）。"
            .into(),
    );

    parts.join("\n\n")
}

/// 注册 MVU 分析 Agent 的工具
///
/// 只注册 `emit_mvu_translation`：让模型声明"我已产出翻译"。handler 原样返回 args，
/// 真正解析在 `mvu_import::parse_mvu_translation_from_response`。
pub fn register_mvu_tools(registry: &mut ToolRegistry) {
    registry.register(
        ToolSpec::function(
            "emit_mvu_translation",
            "输出 MVU 五合一翻译结果（MvuTranslation JSON）。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "variable_schema": {"type": "array"},
                    "ui_bindings": {"type": "array"},
                    "update_rules": {"type": "array", "items": {"type": "string"}},
                    "interactions": {"type": "array"},
                    "fallback_fragments": {"type": "array"},
                    "routing": {"type": "object"},
                    "analysis_confidence": {"type": "number"},
                    "notes": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["variable_schema", "ui_bindings", "update_rules", "interactions", "fallback_fragments", "routing"]
            }),
        ),
        |args, _ctx| {
            Box::pin(async move { Ok(args) })
        },
    );
}

/// 截断超长文本（避免 prompt 爆炸，保留头部 + 尾部 + 省略提示）
fn truncate_for_prompt(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let head: String = s.chars().take(max_chars * 4 / 5).collect();
    let tail: String = s.chars().skip(s.chars().count() - max_chars / 5).collect();
    format!(
        "{}\n\n……（中间省略 {} 字符）……\n\n{}",
        head,
        s.chars().count() - max_chars,
        tail
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Source;
    use storyforge_domain::character::RenderableAssets;

    fn make_card() -> Character {
        Character {
            id: storyforge_domain::Id::new(),
            name: "测试MVU卡".into(),
            description: "测试".into(),
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
                html: Some("<div id='hp_bar'></div>".into()),
                css: None,
                js: Some("_.set('hp', 80);".into()),
                name: "test".into(),
            }),
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_make_mvu_config_has_meta_role() {
        let cfg = make_mvu_analyzer_config();
        assert_eq!(cfg.role, AgentRole::Meta);
        assert!(cfg.max_tool_rounds > 0);
    }

    #[test]
    fn test_mvu_prompt_contains_keyword_and_examples() {
        // Mock 脚本靠 "卡内状态栏分析" 关键词命中，prompt 必须含此词
        assert!(MVU_ANALYZER_SYSTEM_PROMPT.contains("卡内状态栏分析"));
        // 必须含 JSON 输出示例的关键字段名
        assert!(MVU_ANALYZER_SYSTEM_PROMPT.contains("emit_mvu_translation"));
        assert!(MVU_ANALYZER_SYSTEM_PROMPT.contains("ui_bindings"));
        assert!(MVU_ANALYZER_SYSTEM_PROMPT.contains("fallback_fragments"));
    }

    #[test]
    fn test_build_mvu_user_msg_includes_card_and_complexity() {
        let card = make_card();
        let report = storyforge_domain::mvu_translation::score_card_complexity(
            card.renderable_assets.as_ref(),
            &card.extensions,
        );
        let msg = build_mvu_analyzer_user_msg(&card, &report, &[]);
        assert!(msg.contains("测试MVU卡"));
        assert!(msg.contains("启发式复杂度打分"));
        assert!(msg.contains("卡 JS 全文"));
        assert!(msg.contains("_.set('hp', 80)"));
    }

    #[test]
    fn test_truncate_preserves_head_and_tail() {
        let s = "a".repeat(1000);
        let t = truncate_for_prompt(&s, 100);
        assert!(t.contains("省略"));
        assert!(t.len() < s.len());
    }
}

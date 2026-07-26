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

【新一代卡形态（重要）】
很多卡不把变量放 extensions.mvu，而是：
- 初始变量在世界书 [InitVar] 条目（YAML/JSON 树，常为禁用态当数据用，属正常）
- 更新规则在世界书 [mvu_update] 条目（自然语言，指导模型输出 <UpdateVariable>/JSONPatch）
- 在场/分阶段人设用 EJS 控制器（<% getvar('stat_data.….是否在场') %>、好感度阈值分段拉取人设条目）
- 变量引擎是远程 MagVarUpdate 框架（tavern_helper 一行 import），字段 schema 可能在远程 Zod 模块
对这类卡的翻译要求：
- variable_schema 从 [InitVar] 树 + 开场白 <UpdateVariable> 种子推导，保持原变量路径层级，统一用点记法且去掉 stat_data. 容器前缀（如 女性角色.某人.好感度，不写 stat_data.女性角色.某人.好感度，也不写 /女性角色/某人/好感度），value_type 按值形态判断
- EJS 控制器翻译为 update_rules（写明变量、阈值区间、各区间效果），不要把 EJS 源码留在产物里
- 远程框架本身不翻译（在 notes 里注明依赖）；只翻译卡专属逻辑

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
- 变量 key 一律点记法路径（a.b.c），不带 stat_data. 前缀，不用 / 分隔；同构角色子树的模板占位符段用 {角色名} 形式（不用 <角色名>）。variable_schema、ui_bindings.variable_key、interactions 的 key 三处记法保持一致
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
        // emit_mvu_translation 是"声明任务完成"型工具，必须终止循环：
        // 否则模型早期成功提交后循环继续空转，最终响应只剩一句"已完成"，
        // 解析器 5 层兜底全 miss → 静默降级空壳（2026-07-26 真实验收抓获）
        terminal_tools: vec!["emit_mvu_translation".into()],
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

    // 新一代卡（命定之诗/卿卿形态）：变量与规则藏在世界书 / tavern_helper / regex / 开场白里
    if let Some(section) = build_worldbook_variable_section(card) {
        parts.push(section);
    }
    if let Some(section) = build_worldbook_ejs_section(card) {
        parts.push(section);
    }
    if let Some(section) = build_tavern_helper_section(card) {
        parts.push(section);
    }
    if let Some(section) = build_regex_summary_section(card) {
        parts.push(section);
    }
    if let Some(section) = build_greeting_seed_section(card) {
        parts.push(section);
    }

    parts.push(
        "请按指定 JSON 格式输出 MvuTranslation（调用 emit_mvu_translation 或直接输出 JSON）。"
            .into(),
    );

    parts.join("\n\n")
}

/// 世界书条目的 comment（ST 存在 entry-level `comment` 字段，落在 extra 里）
fn entry_comment(entry: &storyforge_domain::world_info::WorldInfoEntry) -> String {
    entry
        .extra
        .get("comment")
        .and_then(|v| v.as_str())
        .unwrap_or("(无标题)")
        .to_string()
}

/// 世界书里的变量条目（[InitVar] 初始值 / [mvu_update] 规则 / stat_data 引用）。
/// 禁用条目也要收——MVU 卡的 [InitVar] 约定就是 enabled=false 当数据用。
fn build_worldbook_variable_section(card: &Character) -> Option<String> {
    let book = card.embedded_world_info.as_ref()?;
    let mut out = String::from(
        "【世界书变量条目（[InitVar]=初始变量数据（常为禁用态，属正常）；[mvu_update]=更新规则；请翻译进 variable_schema / update_rules）】\n",
    );
    // 预算教训（2026-07-27，forge 差分定位）：destiny 两个模型 schema 覆盖率
    // 恰好同为 50.9%——确定性输入截断的指纹，不是模型能力问题。[InitVar]
    // 树是 variable_schema 的唯一事实来源，中段被 truncate 就意味着中段
    // 子树永远进不了 schema，预算必须容得下整棵树。
    let mut budget = 40_000usize;
    let mut hit = false;
    for entry in &book.entries {
        let comment = entry_comment(entry);
        let c_lower = comment.to_lowercase();
        let is_var_entry = c_lower.contains("initvar")
            || c_lower.contains("mvu")
            || comment.contains("变量")
            || entry.content.contains("UpdateVariable")
            || entry.content.contains("stat_data") && comment.contains("规则");
        if !is_var_entry || budget == 0 {
            continue;
        }
        // [InitVar] 数据条目给大额度（整树不可截断），规则类条目维持小额度
        let per_entry_cap = if c_lower.contains("initvar") { 24_000 } else { 4_000 };
        let body = truncate_for_prompt(&entry.content, per_entry_cap.min(budget));
        budget = budget.saturating_sub(body.chars().count());
        out.push_str(&format!(
            "--- {}（{}{}）---\n{}\n",
            comment,
            if entry.disabled { "禁用/数据态" } else { "启用" },
            if entry.constant { "，常驻" } else { "" },
            body
        ));
        hit = true;
    }
    hit.then_some(out)
}

/// 世界书里的 EJS 控制器（<% %>：在场门控 / 好感度分阶段人设等）。
/// 这些是用提示词模板手写的"变量驱动人设路由"，请翻译成 update_rules +
/// variable_schema（如 是否在场 / 好感度 阶段阈值），不要原样保留 EJS。
fn build_worldbook_ejs_section(card: &Character) -> Option<String> {
    let book = card.embedded_world_info.as_ref()?;
    let mut out = String::from(
        "【世界书 EJS 控制器（变量驱动的在场/分阶段门控，请翻译为 variable_schema + update_rules，说明各阈值区间）】\n",
    );
    let mut budget = 12_000usize;
    let mut count = 0usize;
    for entry in &book.entries {
        if !entry.content.contains("<%") || budget == 0 || count >= 25 {
            continue;
        }
        let body = truncate_for_prompt(&entry.content, 1500.min(budget));
        budget = budget.saturating_sub(body.chars().count());
        out.push_str(&format!("--- {} ---\n{}\n", entry_comment(entry), body));
        count += 1;
    }
    (count > 0).then_some(out)
}

/// tavern_helper 脚本清单（同名多版本按"启用优先"去重；短脚本给全文，长脚本给头部）
fn build_tavern_helper_section(card: &Character) -> Option<String> {
    let scripts = card
        .extensions
        .get("tavern_helper")
        .and_then(|v| v.get("scripts"))
        .and_then(|v| v.as_array())?;
    if scripts.is_empty() {
        return None;
    }
    // 去重：同名脚本保留启用版（卡内常带历史版本）
    let mut chosen: Vec<(&serde_json::Value, bool)> = Vec::new();
    for s in scripts {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        if let Some(existing) = chosen.iter_mut().find(|(e, _)| {
            e.get("name").and_then(|v| v.as_str()).unwrap_or("") == name
        }) {
            if enabled && !existing.1 {
                *existing = (s, enabled);
            }
        } else {
            chosen.push((s, enabled));
        }
    }
    let mut out = String::from(
        "【tavern_helper 脚本（远程 import = 依赖框架，如 MagVarUpdate=MVU 变量引擎、mvu_zod=Zod schema；内联 = 卡自带逻辑）】\n",
    );
    for (s, enabled) in chosen.into_iter().take(16) {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("(未命名)");
        let content = s.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let shown = if content.chars().count() <= 600 {
            content.to_string()
        } else {
            format!(
                "{}\n……（共 {} 字符，仅展示头部）",
                content.chars().take(600).collect::<String>(),
                content.chars().count()
            )
        };
        out.push_str(&format!(
            "--- {}（{}）---\n{}\n",
            name,
            if enabled { "启用" } else { "禁用" },
            shown
        ));
    }
    Some(out)
}

/// regex 界面脚本摘要（只给元数据与标记，不给巨型 HTML 正文）
fn build_regex_summary_section(card: &Character) -> Option<String> {
    let scripts = card
        .extensions
        .get("regex_scripts")
        .and_then(|v| v.as_array())?;
    if scripts.is_empty() {
        return None;
    }
    let mut out = String::from(
        "【regex 界面脚本摘要（状态栏/开局/战斗等 UI；含 Mvu/_.set 标记的界面读写变量，供 ui_bindings/interactions 判断）】\n",
    );
    for s in scripts.iter().take(24) {
        let name = s.get("scriptName").and_then(|v| v.as_str()).unwrap_or("");
        let disabled = s.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
        let find = s.get("findRegex").and_then(|v| v.as_str()).unwrap_or("");
        let replace = s.get("replaceString").and_then(|v| v.as_str()).unwrap_or("");
        let markers: Vec<&str> = [
            "<script", "<style", "Mvu", "_.set", "getvar", "triggerSlash",
        ]
        .into_iter()
        .filter(|m| replace.contains(*m))
        .collect();
        out.push_str(&format!(
            "- {}（{}）find={} 替换体 {} 字符{}\n",
            name,
            if disabled { "禁用" } else { "启用" },
            find.chars().take(60).collect::<String>(),
            replace.chars().count(),
            if markers.is_empty() {
                String::new()
            } else {
                format!("，含标记: {}", markers.join(","))
            }
        ));
    }
    Some(out)
}

/// 开场白里的 <UpdateVariable> 初始状态种子（schema 的直接证据）
fn build_greeting_seed_section(card: &Character) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut sources: Vec<&str> = vec![card.first_mes.as_str()];
    sources.extend(card.alternate_greetings.iter().map(|s| s.as_str()));
    for text in sources {
        let mut rest = text;
        while let Some(start) = rest.find("<UpdateVariable>") {
            let Some(end_rel) = rest[start..].find("</UpdateVariable>") else {
                break;
            };
            let block = &rest[start..start + end_rel + "</UpdateVariable>".len()];
            blocks.push(truncate_for_prompt(block, 3000));
            rest = &rest[start + end_rel..];
            if blocks.len() >= 3 {
                break;
            }
        }
        if blocks.len() >= 3 {
            break;
        }
    }
    if blocks.is_empty() {
        return None;
    }
    Some(format!(
        "【开场白初始变量种子（<UpdateVariable> 块，变量树/字段名的直接证据，请对齐 variable_schema）】\n{}",
        blocks.join("\n---\n")
    ))
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

    /// 卿卿/命定之诗形态的迷你卡：变量在世界书，逻辑在 tavern_helper，UI 在 regex
    fn make_new_generation_card() -> Character {
        use storyforge_domain::world_info::WorldInfoBook;
        let st_book: storyforge_domain::character::StWorldInfoBook = serde_json::from_value(
            serde_json::json!({
                "entries": [
                    {
                        "id": 1, "keys": [], "content": "主角:\n  好感度: 0\n  是否在场: false",
                        "constant": false, "selective": true, "enabled": false,
                        "comment": "[initvar]变量初始化勿开"
                    },
                    {
                        "id": 2, "keys": [], "content": "每轮按剧情输出 <UpdateVariable> JSONPatch 更新 stat_data",
                        "constant": true, "selective": false, "enabled": true,
                        "comment": "[mvu_update]变量更新规则"
                    },
                    {
                        "id": 3, "keys": [], "content": "<%_ var g = getvar('stat_data.主角.好感度', {defaults:0}); if (g >= 76) { _%><%- await getwi(null, '阶段04') %><%_ } _%>",
                        "constant": true, "selective": false, "enabled": true,
                        "comment": "某角色_分阶段人设"
                    }
                ]
            }),
        )
        .expect("st book json");
        let mut card = make_card();
        card.embedded_world_info = Some(WorldInfoBook::from_st(st_book));
        card.renderable_assets = None;
        card.first_mes = "开场<UpdateVariable>{\"主角\":{\"好感度\":5}}</UpdateVariable>白".into();
        card.extensions = serde_json::json!({
            "tavern_helper": { "scripts": [
                {"name": "MVUbeta", "enabled": true, "content": "import 'https://cdn.example/MagVarUpdate/bundle.js'"},
                {"name": "战斗", "enabled": false, "content": "old version"},
                {"name": "战斗", "enabled": true, "content": "new version"}
            ]},
            "regex_scripts": [
                {"scriptName": "状态栏", "disabled": false, "findRegex": "<StatusPlaceHolderImpl/>",
                 "replaceString": "<div><script>Mvu.getMvuData()</script></div>", "placement": [2]}
            ]
        });
        card
    }

    #[test]
    fn test_initvar_entry_survives_budget_without_middle_truncation() {
        // 覆盖率缺口根因回归：大 [InitVar] 树的中段不得被截断。
        // 旧预算（单条 4000/总 14000）会把 20K 字的树砍掉中间——
        // 中段子树永远进不了 variable_schema，且两模型覆盖率完全一致。
        use storyforge_domain::world_info::WorldInfoBook;
        let mut lines = vec!["变量树:".to_string()];
        for i in 0..800 {
            lines.push(format!("  角色{i:03}:\n    好感度: {i}\n    等级: 1"));
        }
        // 中段哨兵：截断最先吃掉中间
        lines.insert(400, "  中段哨兵角色:\n    唯一标记: 42".into());
        let big_tree = lines.join("\n");
        assert!(big_tree.chars().count() > 14_000, "样本必须超过旧总预算");

        let st_book: storyforge_domain::character::StWorldInfoBook = serde_json::from_value(
            serde_json::json!({
                "entries": [{
                    "id": 1, "keys": [], "content": big_tree,
                    "constant": false, "selective": true, "enabled": false,
                    "comment": "[InitVar]变量初始化"
                }]
            }),
        )
        .expect("st book json");
        let mut card = make_card();
        card.embedded_world_info = Some(WorldInfoBook::from_st(st_book));

        let section = build_worldbook_variable_section(&card).expect("有变量条目");
        assert!(
            section.contains("中段哨兵角色"),
            "InitVar 中段被截断，覆盖率缺口会复现"
        );
        assert!(section.contains("角色799"), "树尾也应保留");
    }

    #[test]
    fn test_user_msg_includes_worldbook_th_regex_and_greeting_sections() {
        let card = make_new_generation_card();
        let report = storyforge_domain::mvu_translation::score_card_complexity(
            card.renderable_assets.as_ref(),
            &card.extensions,
        );
        let msg = build_mvu_analyzer_user_msg(&card, &report, &[]);

        // 世界书变量条目：禁用的 [initvar] 也要在场
        assert!(msg.contains("世界书变量条目"));
        assert!(msg.contains("[initvar]变量初始化勿开"));
        assert!(msg.contains("好感度: 0"));
        assert!(msg.contains("[mvu_update]变量更新规则"));
        // EJS 控制器
        assert!(msg.contains("世界书 EJS 控制器"));
        assert!(msg.contains("某角色_分阶段人设"));
        // tavern_helper：同名脚本按启用版去重（只查 TH 区块，extensions 原样转储区不受影响）
        let th_start = msg.find("【tavern_helper 脚本").expect("应有 TH 区块");
        let th_section = &msg[th_start..];
        let th_section = &th_section[..th_section[3..]
            .find("【")
            .map(|i| i + 3)
            .unwrap_or(th_section.len())];
        assert!(th_section.contains("MagVarUpdate"));
        assert!(th_section.contains("new version"));
        assert!(
            !th_section.contains("old version"),
            "同名禁用旧版应被启用版去重"
        );
        // regex 摘要：给标记不给正文
        assert!(msg.contains("regex 界面脚本摘要"));
        assert!(msg.contains("状态栏"));
        // 开场白种子
        assert!(msg.contains("开场白初始变量种子"));
        assert!(msg.contains("好感度\":5"));
    }

    #[test]
    fn test_th_only_card_is_not_pure_data() {
        // 卡逻辑只在 tavern_helper 时不得短路成 PureData
        let card = make_new_generation_card();
        let report = storyforge_domain::mvu_translation::score_card_complexity(
            card.renderable_assets.as_ref(),
            &card.extensions,
        );
        assert_ne!(
            report.classification,
            storyforge_domain::mvu_translation::CardComplexity::PureData,
            "启用 tavern_helper 脚本的卡不应判为 PureData"
        );
    }
}

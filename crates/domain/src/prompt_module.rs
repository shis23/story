use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::Id;
use crate::agent::AgentRole;

/// 判断模块是否适用于某个 Agent 角色，支持 `Subagent("*")` 通配符。
///
/// 与 `PromptProfile::selected_ids` / `override_text` 的通配符回退保持一致：
/// 当模块的 `applicable_roles` 含 `Subagent("*")` 时，对任意 `Subagent(id)` 角色
/// 都视为适用（避免内置子 Agent 模块因精确 `PartialEq` 比较而永远匹配不上具体角色）。
fn role_applicable(applicable: &[AgentRole], role: &AgentRole) -> bool {
    if applicable.iter().any(|r| r == role) {
        return true;
    }
    // Subagent 通配符回退：applicable 含 Subagent("*") 则匹配任意 Subagent(_)
    match role {
        AgentRole::Subagent(_) => applicable
            .iter()
            .any(|r| matches!(r, AgentRole::Subagent(w) if w == "*")),
        _ => false,
    }
}

// ─── 提示词模块（对应设计 §3.6.6 三层预设体系 Layer ①）───────────────────

/// 模块分类组（单选互斥 / 多选叠加）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleCategory {
    /// 视角（单选）
    Perspective,
    /// 思维链（单选，按模型）
    Cot,
    /// 文风（单选）
    Style,
    /// 质量约束（多选）
    Quality,
    /// 输出规范（多选）
    Output,
    /// 情感基调（单选）
    Tone,
}

/// 互斥性
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Exclusivity {
    /// 单选互斥（同组只能选一个）
    Single,
    /// 多选叠加（同组可选多个）
    Multiple,
}

/// 模块来源
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleSource {
    BuiltIn,
    ImportedFromST,
    UserCustom,
}

/// 提示词模块（最小单位，对应设计 §3.6.6 Layer ①）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptModule {
    pub id: Id,
    pub name: String,
    pub category: ModuleCategory,
    /// 模块正文（拼进 system prompt 的文本）
    pub content: String,
    pub exclusivity: Exclusivity,
    pub source: ModuleSource,
    /// 该模块可挂到哪些 Agent 角色
    pub applicable_roles: Vec<AgentRole>,
    /// 作者/版本/标签
    pub tags: Vec<String>,
}

// ─── 提示词预设 Profile（对应设计 §3.6.6 Layer ②）─────────────────────────

/// Profile 来源
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileSource {
    UserCreated,
    ImportedFromST,
    BuiltIn,
}

/// 提示词预设 Profile（可保存命名，对应设计 §3.6.6 Layer ②）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptProfile {
    pub id: Id,
    pub name: String,
    /// 每个 Agent 角色选定哪些模块（按 category 组织）
    /// key = AgentRole，value = { category -> Vec<module_id> }
    pub selections:
        std::collections::HashMap<AgentRole, std::collections::HashMap<ModuleCategory, Vec<Id>>>,
    /// 自定义覆盖（用户直接改某 Agent 的提示词，不走模块）
    pub overrides: std::collections::HashMap<AgentRole, Option<String>>,
    pub source: ProfileSource,
}

impl PromptProfile {
    /// 获取某个 Agent 在某个 category 下选中的模块 ID 列表
    ///
    /// 支持通配符：当 role 是 Subagent(id) 且找不到精确匹配时，
    /// 自动回退到 Subagent("*") 的配置。
    pub fn selected_ids(&self, role: &AgentRole, category: &ModuleCategory) -> &[Id] {
        if let Some(cats) = self.selections.get(role)
            && let Some(ids) = cats.get(category)
        {
            return ids.as_slice();
        }
        // 回退：Subagent(id) → Subagent("*")
        if let AgentRole::Subagent(_) = role {
            let wildcard = AgentRole::Subagent("*".into());
            if let Some(cats) = self.selections.get(&wildcard)
                && let Some(ids) = cats.get(category)
            {
                return ids.as_slice();
            }
        }
        &[]
    }

    /// 获取某个 Agent 的自定义覆盖文本（支持 Subagent 通配符回退）
    pub fn override_text(&self, role: &AgentRole) -> Option<&String> {
        if let Some(text) = self.overrides.get(role) {
            return text.as_ref();
        }
        if let AgentRole::Subagent(_) = role {
            let wildcard = AgentRole::Subagent("*".into());
            if let Some(text) = self.overrides.get(&wildcard) {
                return text.as_ref();
            }
        }
        None
    }
}

// ─── Agent 绑定（对应设计 §3.6.6 Layer ③）─────────────────────────────────

/// 运行时绑定：每个 Agent 角色当前用哪个 Profile + 连接
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBinding {
    pub role: AgentRole,
    pub active_profile_id: Id,
    pub active_connection_id: Id,
}

// ─── 提示词组装函数（对应设计 §3.6.7）─────────────────────────────────────

/// 按 Profile 组装系统提示词（设计 §3.6.7 的核心流程）
///
/// 按 category 优先级顺序，插入已选模块的 content，最后拼接工具说明。
///
/// A1 互斥规则（架构文档 §6.2）：只有 `Prompted` 模式注入 CoT 提示模块。
/// - `Disabled` = 不启用任何推理引导（跳过 CoT）
/// - `Native` = 使用厂商原生 thinking（跳过 CoT，避免双重推理）
/// - `Prompted` = 使用提示式 CoT（注入 CoT 模块）
pub fn assemble_system_prompt(
    role: &AgentRole,
    role_directive: &str,
    profile: Option<&PromptProfile>,
    modules: &[PromptModule],
    tool_directives: &str,
    reasoning: &crate::llm::ReasoningMode,
) -> String {
    let mut parts = vec![role_directive.to_string()];

    if let Some(profile) = profile {
        // 按 category 的固定优先级顺序
        let category_order = [
            ModuleCategory::Perspective,
            ModuleCategory::Cot,
            ModuleCategory::Style,
            ModuleCategory::Tone,
            ModuleCategory::Quality,
            ModuleCategory::Output,
        ];

        for cat in &category_order {
            // A1：只有 Prompted 模式注入 CoT；Disabled 和 Native 都跳过
            // - Disabled = 不启用任何推理引导
            // - Native = 使用厂商原生 thinking，CoT 冗余
            // - Prompted = 使用提示式 CoT
            if *cat == ModuleCategory::Cot && *reasoning != crate::llm::ReasoningMode::Prompted {
                continue;
            }
            let ids = profile.selected_ids(role, cat);
            for mid in ids {
                if let Some(m) = modules
                    .iter()
                    .find(|m| &m.id == mid && role_applicable(&m.applicable_roles, role))
                {
                    parts.push(m.content.clone());
                }
            }
        }

        // 检查自定义覆盖（支持 Subagent 通配符回退）
        if let Some(text) = profile.override_text(role) {
            parts.push(text.clone());
        }
    }

    if !tool_directives.is_empty() {
        parts.push(tool_directives.to_string());
    }

    parts.join("\n\n---\n\n")
}

/// Prompt-template/ST style macro context.
///
/// This intentionally keeps values owned so callers can build a context from a
/// card, campaign snapshot, or test fixture without lifetime plumbing.
#[derive(Debug, Clone)]
pub struct TemplateVarContext {
    pub char_name: String,
    pub user_name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_mes: String,
    pub mes_example: String,
    pub system_prompt: String,
    pub post_history_instructions: String,
    pub creator: String,
    pub character_version: String,
    pub tags: Vec<String>,
    pub variables: BTreeMap<String, String>,
    /// Whether macros that imply one active character (`{{char}}`,
    /// `{{description}}`, `<bot>`, etc.) should be rendered.
    ///
    /// Multi-instance Campaign prompts can still render scoped variables while
    /// preserving ambiguous character-card macros for later compatibility layers.
    pub render_character_macros: bool,
    /// Fixed clock for deterministic prompt-template rendering in tests or replays.
    /// When absent, the renderer captures `Utc::now()` once per render call.
    pub now: Option<DateTime<Utc>>,
    /// Optional deterministic seed for `random` / `roll` macros.
    /// When absent, a per-render seed is derived from system time.
    pub random_seed: Option<u64>,
}

impl Default for TemplateVarContext {
    fn default() -> Self {
        Self {
            char_name: String::new(),
            user_name: String::new(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            creator: String::new(),
            character_version: String::new(),
            tags: Vec::new(),
            variables: BTreeMap::new(),
            render_character_macros: true,
            now: None,
            random_seed: None,
        }
    }
}

impl TemplateVarContext {
    pub fn from_character(character: &crate::character::Character, user_name: &str) -> Self {
        Self {
            char_name: character.name.clone(),
            user_name: user_name.to_string(),
            description: character.description.clone(),
            personality: character.personality.clone(),
            scenario: character.scenario.clone(),
            first_mes: character.first_mes.clone(),
            mes_example: character.mes_example.clone(),
            system_prompt: character.system_prompt.clone(),
            post_history_instructions: character.post_history_instructions.clone(),
            creator: character.creator.clone(),
            character_version: character.character_version.clone(),
            tags: character.tags.clone(),
            variables: BTreeMap::new(),
            render_character_macros: true,
            now: None,
            random_seed: None,
        }
    }
}

#[derive(Debug, Clone)]
struct TemplateRenderState {
    variables: BTreeMap<String, String>,
    trim_output: bool,
    now: DateTime<Utc>,
    random_seed: u64,
    random_counter: u64,
}

/// ST 风格占位符替换（设计 §7.5 prompt-template 功能）
///
/// 支持的占位符：
/// - `{{char}}` → 角色名
/// - `{{user}}` → 用户名（默认 "玩家"）
/// - `{{charIfNotUser}}` → 如果不是用户则显示角色名（简化为 char）
/// - 常用角色卡字段：`{{description}}`, `{{personality}}`, `{{scenario}}`,
///   `{{first_mes}}`, `{{mes_example}}`, `{{system_prompt}}`,
///   `{{post_history_instructions}}` 等
/// - 本地 ST 变量宏：`{{setvar::key::value}}`, `{{addvar::key::value}}`,
///   `{{getvar::key}}`, `{{trim}}`, `{{// comment}}`
/// - 常用动态宏：`{{date}}`, `{{time}}`, `{{datetime}}`, `{{weekday}}`,
///   `{{isotime}}`, `{{random::A::B}}`, `{{roll::2d6+1}}`
///
/// 在组装 system prompt 后、发送给 LLM 前调用。
pub fn replace_template_vars(text: &str, char_name: &str, user_name: &str) -> String {
    let ctx = TemplateVarContext {
        char_name: char_name.to_string(),
        user_name: user_name.to_string(),
        ..Default::default()
    };
    replace_template_vars_with_context(text, &ctx)
}

pub fn replace_template_vars_with_context(text: &str, context: &TemplateVarContext) -> String {
    let mut state = TemplateRenderState {
        variables: context.variables.clone(),
        trim_output: false,
        now: context.now.unwrap_or_else(Utc::now),
        random_seed: context
            .random_seed
            .unwrap_or_else(default_template_random_seed),
        random_counter: 0,
    };
    let mut rendered = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find("{{") {
        let (before, after_start) = rest.split_at(start);
        rendered.push_str(before);
        let macro_body_start = &after_start[2..];
        if let Some(end) = macro_body_start.find("}}") {
            let (body, after_body) = macro_body_start.split_at(end);
            match render_template_macro(body.trim(), context, &mut state) {
                Some(value) => rendered.push_str(&value),
                None => {
                    rendered.push_str("{{");
                    rendered.push_str(body);
                    rendered.push_str("}}");
                }
            }
            rest = &after_body[2..];
        } else {
            rendered.push_str(after_start);
            rest = "";
        }
    }

    rendered.push_str(rest);
    let rendered = replace_angle_aliases(&rendered, context);
    if state.trim_output {
        rendered.trim().to_string()
    } else {
        rendered
    }
}

fn render_template_macro(
    body: &str,
    context: &TemplateVarContext,
    state: &mut TemplateRenderState,
) -> Option<String> {
    if body.is_empty() {
        return Some(String::new());
    }
    if body == "trim" {
        state.trim_output = true;
        return Some(String::new());
    }
    if body.starts_with("//") {
        return Some(String::new());
    }

    if let Some(rest) = body.strip_prefix("setvar::") {
        let (key, value) = split_macro_key_value(rest)?;
        let value = render_template_value(value, context, state);
        state.variables.insert(key.trim().to_string(), value);
        return Some(String::new());
    }
    if let Some(rest) = body.strip_prefix("addvar::") {
        let (key, value) = split_macro_key_value(rest)?;
        let value = render_template_value(value, context, state);
        state
            .variables
            .entry(key.trim().to_string())
            .or_default()
            .push_str(&value);
        return Some(String::new());
    }
    if let Some(key) = body
        .strip_prefix("getvar::")
        .or_else(|| body.strip_prefix("getglobalvar::"))
    {
        return Some(state.variables.get(key.trim()).cloned().unwrap_or_default());
    }

    if let Some(rest) = strip_ascii_case_prefix(body, "random::")
        .or_else(|| strip_ascii_case_prefix(body, "pick::"))
        .or_else(|| strip_ascii_case_prefix(body, "random:"))
        .or_else(|| strip_ascii_case_prefix(body, "pick:"))
    {
        return render_random_macro(rest, context, state);
    }

    if let Some(rest) = strip_ascii_case_prefix(body, "roll::")
        .or_else(|| strip_ascii_case_prefix(body, "dice::"))
        .or_else(|| strip_ascii_case_prefix(body, "roll:"))
        .or_else(|| strip_ascii_case_prefix(body, "dice:"))
    {
        return render_roll_macro(rest, state);
    }

    render_field_macro(body, context, state)
}

fn split_macro_key_value(rest: &str) -> Option<(&str, &str)> {
    rest.split_once("::")
}

fn render_template_value(
    value: &str,
    context: &TemplateVarContext,
    state: &TemplateRenderState,
) -> String {
    let mut rendered = replace_angle_aliases(value, context);
    for (key, value) in &state.variables {
        rendered = rendered.replace(&format!("{{{{getvar::{key}}}}}"), value);
    }
    rendered
}

fn render_field_macro(
    body: &str,
    context: &TemplateVarContext,
    state: &TemplateRenderState,
) -> Option<String> {
    let key = body.trim().to_ascii_lowercase();
    if !context.render_character_macros && is_character_field_macro(&key) {
        return None;
    }
    let value = match key.as_str() {
        "char" | "charname" | "char_name" | "character" | "bot" => context.char_name.clone(),
        "user" | "username" | "user_name" => context.user_name.clone(),
        "charifnotuser" | "char_if_not_user" => context.char_name.clone(),
        "description" | "char_description" | "character_description" => context.description.clone(),
        "personality" | "persona" => context.personality.clone(),
        "scenario" => context.scenario.clone(),
        "first_mes" | "first_message" | "firstmsg" | "greeting" => context.first_mes.clone(),
        "mes_example" | "example_dialogue" | "example_messages" | "examples" => {
            context.mes_example.clone()
        }
        "system_prompt" | "system" => context.system_prompt.clone(),
        "post_history_instructions" | "post_history" | "post_history_instruction" => {
            context.post_history_instructions.clone()
        }
        "creator" => context.creator.clone(),
        "character_version" | "char_version" | "version" => context.character_version.clone(),
        "tags" => context.tags.join(", "),
        "newline" => "\n".to_string(),
        "noop" => String::new(),
        "date" => state.now.format("%Y-%m-%d").to_string(),
        "time" => format!("{:02}:{:02}", state.now.hour(), state.now.minute()),
        "datetime" => state.now.format("%Y-%m-%d %H:%M").to_string(),
        "isotime" | "iso8601" => state.now.to_rfc3339(),
        "weekday" => weekday_name(state.now.weekday()).to_string(),
        _ => return None,
    };
    Some(value)
}

fn is_character_field_macro(key: &str) -> bool {
    matches!(
        key,
        "char"
            | "charname"
            | "char_name"
            | "character"
            | "bot"
            | "charifnotuser"
            | "char_if_not_user"
            | "description"
            | "char_description"
            | "character_description"
            | "personality"
            | "persona"
            | "scenario"
            | "first_mes"
            | "first_message"
            | "firstmsg"
            | "greeting"
            | "mes_example"
            | "example_dialogue"
            | "example_messages"
            | "examples"
            | "system_prompt"
            | "system"
            | "post_history_instructions"
            | "post_history"
            | "post_history_instruction"
            | "creator"
            | "character_version"
            | "char_version"
            | "version"
            | "tags"
    )
}

fn render_random_macro(
    rest: &str,
    context: &TemplateVarContext,
    state: &mut TemplateRenderState,
) -> Option<String> {
    let options: Vec<&str> = if rest.contains("::") {
        rest.split("::").collect::<Vec<_>>()
    } else {
        rest.split([',', '|']).collect::<Vec<_>>()
    }
    .into_iter()
    .map(str::trim)
    .filter(|option| !option.is_empty())
    .collect();

    if options.is_empty() {
        return None;
    }

    let idx = (next_template_random(state, rest) as usize) % options.len();
    Some(render_template_value(options[idx], context, state))
}

fn render_roll_macro(rest: &str, state: &mut TemplateRenderState) -> Option<String> {
    let spec = rest.trim().replace(' ', "");
    let (dice_spec, modifier) = split_roll_modifier(&spec)?;
    let lower = dice_spec.to_ascii_lowercase();
    let (count_str, sides_str) = lower.split_once('d')?;
    let count = if count_str.is_empty() {
        1
    } else {
        count_str.parse::<u32>().ok()?
    };
    let sides = sides_str.parse::<u32>().ok()?;
    if count == 0 || count > 100 || sides == 0 || sides > 100_000 {
        return None;
    }

    let mut total = modifier;
    for roll_idx in 0..count {
        let salt = format!("{spec}#{roll_idx}");
        total += (next_template_random(state, &salt) % u64::from(sides) + 1) as i32;
    }
    Some(total.to_string())
}

fn split_roll_modifier(spec: &str) -> Option<(&str, i32)> {
    if spec.is_empty() {
        return None;
    }

    let mut modifier_idx = None;
    for (idx, ch) in spec.char_indices().skip(1) {
        if ch == '+' || ch == '-' {
            modifier_idx = Some(idx);
        }
    }

    if let Some(idx) = modifier_idx {
        let dice_spec = &spec[..idx];
        let modifier = spec[idx..].parse::<i32>().ok()?;
        Some((dice_spec, modifier))
    } else {
        Some((spec, 0))
    }
}

fn strip_ascii_case_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn weekday_name(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}

fn next_template_random(state: &mut TemplateRenderState, salt: &str) -> u64 {
    state.random_counter = state.random_counter.wrapping_add(1);
    let mut hash = state.random_seed ^ state.random_counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for byte in salt.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01B3);
        hash ^= hash >> 32;
    }
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    hash ^ (hash >> 33)
}

fn default_template_random_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
        ^ u64::from(std::process::id())
}

fn replace_angle_aliases(text: &str, context: &TemplateVarContext) -> String {
    let rendered = text
        .replace("<USER>", &context.user_name)
        .replace("<User>", &context.user_name)
        .replace("<user>", &context.user_name);

    if !context.render_character_macros {
        return rendered;
    }

    rendered
        .replace("<BOT>", &context.char_name)
        .replace("<Bot>", &context.char_name)
        .replace("<bot>", &context.char_name)
        .replace("<CHAR>", &context.char_name)
        .replace("<Char>", &context.char_name)
        .replace("<char>", &context.char_name)
}

// ─── 预置模块（M1 内置 5 个核心模块）──────────────────────────────────────

/// 预置模块工厂（对应设计 §3.6.5：内置 5-8 个核心模块覆盖 80% 场景）
pub mod builtins {
    use super::*;
    use std::collections::HashMap;

    /// 创作三角 + 摘要（全角色化 CoT 适用面）
    fn all_roles() -> Vec<AgentRole> {
        vec![
            AgentRole::Director,
            AgentRole::Editor,
            AgentRole::Subagent("*".into()),
            AgentRole::Summarizer,
        ]
    }

    fn director_only() -> Vec<AgentRole> {
        vec![AgentRole::Director]
    }

    /// 编剧 Agent
    fn editor_only() -> Vec<AgentRole> {
        vec![AgentRole::Editor]
    }

    fn subagent_only() -> Vec<AgentRole> {
        vec![AgentRole::Subagent("*".into())]
    }

    fn summarizer_only() -> Vec<AgentRole> {
        vec![AgentRole::Summarizer]
    }

    /// 编剧 + 子 Agent
    fn editor_and_subagent() -> Vec<AgentRole> {
        vec![AgentRole::Editor, AgentRole::Subagent("*".into())]
    }

    /// 预置模块列表（角色化 CoT + 文风/输出）
    pub fn preset_modules() -> Vec<PromptModule> {
        vec![
            // 1. 视角：第三人称（最常用）
            PromptModule {
                id: Id::from_str("builtin-perspective-third"),
                name: "第三人称".into(),
                category: ModuleCategory::Perspective,
                content: "使用第三人称叙事。以全知或限知视角描写场景和角色行为。避免使用「你」称呼角色，使用角色名字或代词。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: editor_only(),
                tags: vec!["视角".into(), "第三人称".into()],
            },
            // 2. 文风：白描
            PromptModule {
                id: Id::from_str("builtin-style-baimiao"),
                name: "白描".into(),
                category: ModuleCategory::Style,
                content: "【文风：白描】\n用朴素、简洁的语言描写。少用华丽辞藻和比喻，多用短句。重在白描动作、对话和细节，让场景自然呈现。避免大段心理分析和抒情。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: editor_only(),
                tags: vec!["文风".into(), "白描".into()],
            },
            // 3. 质量约束：杀八股
            PromptModule {
                id: Id::from_str("builtin-quality-kill-bagu"),
                name: "杀八股".into(),
                category: ModuleCategory::Quality,
                content: "【质量约束：杀八股】\n禁止以下八股化写法：\n- 「仿佛」「宛如」「犹如」等比喻词连续出现超过 2 次\n- 「心中涌起一股xxx」等模板化心理描写\n- 「不禁xxx」「下意识xxx」等被动反应\n- 每段结尾的总结式感慨\n- 「xxx的光芒」「xxx的气息」等空洞渲染\n- 排比句超过 3 句\n- 连续 2 段以上纯心理独白无动作/对话推进".into(),
                exclusivity: Exclusivity::Multiple,
                source: ModuleSource::BuiltIn,
                applicable_roles: editor_only(),
                tags: vec!["约束".into(), "杀八股".into()],
            },
            // 4. CoT：通用兜底（手绑用；不进 default profile）
            PromptModule {
                id: Id::from_str("builtin-cot-generic"),
                name: "通用思维链".into(),
                category: ModuleCategory::Cot,
                content: "【思考指引】\n在输出前内部确认：目标产物是什么、有哪些硬约束、有哪些不可跨越的信息边界。想清楚后只输出目标产物，不要输出思考过程。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: all_roles(),
                tags: vec!["CoT".into(), "通用".into()],
            },
            // 4b. CoT：导演规划（梁元意图/结构/待决吸收；仅 Director）
            PromptModule {
                id: Id::from_str("builtin-cot-director-plan"),
                name: "导演规划五步".into(),
                category: ModuleCategory::Cot,
                content: "【思考指引：导演规划】（内部完成，不要输出思考过程）\n1. 意图：解析用户输入的何时/何人/何事与言外之意；本轮可承诺什么。\n2. 结构：对照近期节拍，避免同构复读；必要时换推进角度。\n3. 待决：列 2–4 个待决项，每项≥2 选项，按不降智、动机驱动、本场不一次解决选定；写清冲突、对立目标、节拍与必须留下的未决。\n4. 分派：每人知/不知/误解；子任务只给合法可知信息；character_id 用实例 id（可先 list_characters 再 get_character）；禁止跨角色私密全知。\n5. 出口：只输出 Plan（scene_brief + ScenePlan + subagent_tasks）。任务可填 desire/at_hand/move。禁止正文、总结腔、元叙述。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: director_only(),
                tags: vec!["CoT".into(), "导演".into(), "梁元结构".into()],
            },
            // 4c. 兼容旧 id：场景清单五步 → 导演规划（避免旧 profile 选中后失语）
            PromptModule {
                id: Id::from_str("builtin-cot-scene-checklist"),
                name: "场景清单五步".into(),
                category: ModuleCategory::Cot,
                content: "【思考指引：导演规划】（内部完成，不要输出思考过程）\n1. 意图：解析用户输入的何时/何人/何事与言外之意；本轮可承诺什么。\n2. 结构：对照近期节拍，避免同构复读；必要时换推进角度。\n3. 待决：列 2–4 个待决项，每项≥2 选项，按不降智、动机驱动、本场不一次解决选定；写清冲突、对立目标、节拍与必须留下的未决。\n4. 分派：每人知/不知/误解；子任务只给合法可知信息；character_id 用实例 id（可先 list_characters 再 get_character）；禁止跨角色私密全知。\n5. 出口：只输出 Plan（scene_brief + ScenePlan + subagent_tasks）。任务可填 desire/at_hand/move。禁止正文、总结腔、元叙述。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: director_only(),
                tags: vec!["CoT".into(), "场景清单".into(), "梁元结构".into(), "兼容".into()],
            },
            // 4d. CoT：编剧合并（反总结腔）
            PromptModule {
                id: Id::from_str("builtin-cot-editor-merge"),
                name: "编剧合并三步".into(),
                category: ModuleCategory::Cot,
                content: "【思考指引：编剧合并】（内部完成，不要输出思考过程）\n1. 材料：Plan 的冲突/节拍；各表演谁在场、谁推进、谁只反应。\n2. 舞台：选定唯一叙述焦点；用动作与对白推进，不旁白解释剧情功能。\n3. 出口检查后只输出正文：禁止「本轮推进了…」「人物关系上…」「场景意义在于…」等总结/说明腔；禁止作者评论、预告、盘点；不写角色不知之事；不为凑字复读上一段。不要输出思考、标题或编辑说明。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: editor_only(),
                tags: vec!["CoT".into(), "编剧".into(), "反总结腔".into()],
            },
            // 4e. CoT：子 Agent 表演（梁元活人化压缩）
            PromptModule {
                id: Id::from_str("builtin-cot-subagent-perform"),
                name: "角色表演四步".into(),
                category: ModuleCategory::Cot,
                content: "【思考指引：角色表演】（内部完成，不要输出思考过程）\n1. 我是谁：身份、当下情绪、与在场者关系；at_hand（正在做的事）。\n2. 我要什么：一个与用户指令无关的、自私而具体的即时欲望（desire）；它如何与任务角力。\n3. 我知什么：只用合法可知信息；不知则猜错、回避或追问；不替他人独白。\n4. 我怎么动：一个主动、可被看见的动作/对白（move）。只输出「我」的表演片段，不写他角完整心理、不写上帝全景、不写本轮剧情总结。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: subagent_only(),
                tags: vec!["CoT".into(), "子Agent".into(), "活人化".into()],
            },
            // 4f. CoT：摘要抽取
            PromptModule {
                id: Id::from_str("builtin-cot-summarizer-extract"),
                name: "摘要抽取".into(),
                category: ModuleCategory::Cot,
                content: "【思考指引：摘要抽取】（内部完成，不要输出思考）\n1. 只圈本轮新发生：关系转折 > 关键事件 > 目标变化 > 冲突 > 道具/地点/时间 > 未决伏笔。\n2. 丢掉气氛描写与重复信息；不展望、不复述前情、不加评论。\n3. 压缩到 200–500 字后，只输出摘要正文。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: summarizer_only(),
                tags: vec!["CoT".into(), "摘要".into()],
            },
            // 5. 输出规范：字数控制
            PromptModule {
                id: Id::from_str("builtin-output-word-count"),
                name: "字数控制".into(),
                category: ModuleCategory::Output,
                content: "【输出规范】\n- 目标字数：800-1500 字\n- 如果内容丰富可以适当超出，但不要低于 500 字\n- 对白占比控制在 30%-50%，不要全是对话或全是叙述".into(),
                exclusivity: Exclusivity::Multiple,
                source: ModuleSource::BuiltIn,
                applicable_roles: editor_and_subagent(),
                tags: vec!["输出".into(), "字数".into()],
            },
        ]
    }

    /// 创建默认 Profile（绑定所有预置模块到各 Agent）
    pub fn default_profile() -> (PromptProfile, Vec<PromptModule>) {
        let modules = preset_modules();

        // 构建 selections：每个 Agent 选中哪些模块
        let mut selections = HashMap::new();

        // 导演：角色化规划 CoT
        let mut director_cats = HashMap::new();
        director_cats.insert(
            ModuleCategory::Cot,
            vec![Id::from_str("builtin-cot-director-plan")],
        );
        selections.insert(AgentRole::Director, director_cats);

        // 编剧：第三人称 + 白描 + 杀八股 + 合并 CoT + 字数控制
        let mut editor_cats = HashMap::new();
        editor_cats.insert(
            ModuleCategory::Perspective,
            vec![Id::from_str("builtin-perspective-third")],
        );
        editor_cats.insert(
            ModuleCategory::Style,
            vec![Id::from_str("builtin-style-baimiao")],
        );
        editor_cats.insert(
            ModuleCategory::Quality,
            vec![Id::from_str("builtin-quality-kill-bagu")],
        );
        editor_cats.insert(
            ModuleCategory::Cot,
            vec![Id::from_str("builtin-cot-editor-merge")],
        );
        editor_cats.insert(
            ModuleCategory::Output,
            vec![Id::from_str("builtin-output-word-count")],
        );
        selections.insert(AgentRole::Editor, editor_cats);

        // 子 Agent：表演 CoT + 字数控制
        let mut sub_cats = HashMap::new();
        sub_cats.insert(
            ModuleCategory::Cot,
            vec![Id::from_str("builtin-cot-subagent-perform")],
        );
        sub_cats.insert(
            ModuleCategory::Output,
            vec![Id::from_str("builtin-output-word-count")],
        );
        selections.insert(AgentRole::Subagent("*".into()), sub_cats);

        // 摘要：抽取 CoT
        let mut sum_cats = HashMap::new();
        sum_cats.insert(
            ModuleCategory::Cot,
            vec![Id::from_str("builtin-cot-summarizer-extract")],
        );
        selections.insert(AgentRole::Summarizer, sum_cats);

        let profile = PromptProfile {
            id: Id::from_str("builtin-default-v1"),
            name: "默认预设 v1".into(),
            selections,
            overrides: HashMap::new(),
            source: ProfileSource::BuiltIn,
        };

        (profile, modules)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_preset_modules_count() {
            let modules = preset_modules();
            // perspective/style/quality + generic + director + scene-compat + editor + sub + summarizer + word-count
            assert_eq!(modules.len(), 10);
        }

        #[test]
        fn test_preset_modules_unique_ids() {
            let modules = preset_modules();
            let ids: Vec<&Id> = modules.iter().map(|m| &m.id).collect();
            let unique: std::collections::HashSet<_> = ids.iter().collect();
            assert_eq!(ids.len(), unique.len(), "模块 ID 应唯一");
        }

        #[test]
        fn test_default_profile_bindings() {
            let (profile, _modules) = default_profile();

            // 导演：角色化规划 CoT
            let director_cot = profile.selected_ids(&AgentRole::Director, &ModuleCategory::Cot);
            assert_eq!(director_cot.len(), 1);
            assert_eq!(director_cot[0], Id::from_str("builtin-cot-director-plan"));

            // 编剧：视角 + 文风 + 质量 + 合并 CoT + 输出
            let editor_persp =
                profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Perspective);
            assert_eq!(editor_persp.len(), 1);
            let editor_style = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Style);
            assert_eq!(editor_style.len(), 1);
            let editor_quality = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Quality);
            assert_eq!(editor_quality.len(), 1);
            let editor_cot = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Cot);
            assert_eq!(editor_cot.len(), 1);
            assert_eq!(editor_cot[0], Id::from_str("builtin-cot-editor-merge"));
            let editor_output = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Output);
            assert_eq!(editor_output.len(), 1);

            // 子 Agent：表演 CoT + 输出
            let sub_cot =
                profile.selected_ids(&AgentRole::Subagent("*".into()), &ModuleCategory::Cot);
            assert_eq!(sub_cot.len(), 1);
            assert_eq!(sub_cot[0], Id::from_str("builtin-cot-subagent-perform"));
            let sub_output =
                profile.selected_ids(&AgentRole::Subagent("*".into()), &ModuleCategory::Output);
            assert_eq!(sub_output.len(), 1);

            // 摘要：抽取 CoT
            let sum_cot = profile.selected_ids(&AgentRole::Summarizer, &ModuleCategory::Cot);
            assert_eq!(sum_cot.len(), 1);
            assert_eq!(sum_cot[0], Id::from_str("builtin-cot-summarizer-extract"));
        }

        #[test]
        fn test_assemble_with_default_profile() {
            let (profile, modules) = default_profile();

            let assembled = assemble_system_prompt(
                &AgentRole::Editor,
                "你是编剧。",
                Some(&profile),
                &modules,
                "工具说明",
                &crate::llm::ReasoningMode::default(),
            );

            assert!(assembled.contains("你是编剧"));
            assert!(assembled.contains("第三人称叙事")); // 模块 content，非 name
            assert!(assembled.contains("白描")); // 模块 content 中含"白描"
            assert!(assembled.contains("杀八股")); // 模块 content 中含"杀八股"
            assert!(assembled.contains("目标字数")); // 字数控制模块的 content
            assert!(assembled.contains("工具说明"));
        }

        #[test]
        fn test_assemble_native_reasoning_excludes_cot() {
            // A1：Native reasoning 模式下，CoT 提示模块应被跳过
            let (profile, modules) = default_profile();
            let assembled = assemble_system_prompt(
                &AgentRole::Director,
                "你是导演。",
                Some(&profile),
                &modules,
                "",
                &crate::llm::ReasoningMode::Native,
            );
            // CoT 模块 content 含"思考指引"
            assert!(
                !assembled.contains("思考指引"),
                "Native reasoning 模式不应注入 CoT 提示，实际: {assembled}"
            );
        }

        #[test]
        fn test_assemble_prompted_reasoning_includes_cot() {
            // A1：Prompted 模式下 CoT 模块正常注入
            let (profile, modules) = default_profile();
            let assembled = assemble_system_prompt(
                &AgentRole::Director,
                "你是导演。",
                Some(&profile),
                &modules,
                "",
                &crate::llm::ReasoningMode::Prompted,
            );
            assert!(
                assembled.contains("思考指引") && assembled.contains("导演规划"),
                "Prompted 模式应注入导演规划 CoT，实际: {assembled}"
            );
        }

        #[test]
        fn test_assemble_prompted_editor_and_subagent_and_summarizer_cot() {
            let (profile, modules) = default_profile();

            let editor = assemble_system_prompt(
                &AgentRole::Editor,
                "你是编剧。",
                Some(&profile),
                &modules,
                "",
                &crate::llm::ReasoningMode::Prompted,
            );
            assert!(
                editor.contains("编剧合并") && editor.contains("总结/说明腔"),
                "Editor Prompted 应注入合并 CoT: {editor}"
            );

            let sub = assemble_system_prompt(
                &AgentRole::Subagent("inst-1".into()),
                "你是角色。",
                Some(&profile),
                &modules,
                "",
                &crate::llm::ReasoningMode::Prompted,
            );
            assert!(
                sub.contains("角色表演") && sub.contains("desire"),
                "Subagent Prompted 应注入表演 CoT: {sub}"
            );

            let sum = assemble_system_prompt(
                &AgentRole::Summarizer,
                "你是摘要。",
                Some(&profile),
                &modules,
                "",
                &crate::llm::ReasoningMode::Prompted,
            );
            assert!(
                sum.contains("摘要抽取") && sum.contains("200–500"),
                "Summarizer Prompted 应注入抽取 CoT: {sum}"
            );
        }

        #[test]
        fn test_assemble_disabled_reasoning_excludes_cot() {
            // A1：Disabled 模式下不注入 CoT
            let (profile, modules) = default_profile();
            let assembled = assemble_system_prompt(
                &AgentRole::Director,
                "你是导演。",
                Some(&profile),
                &modules,
                "",
                &crate::llm::ReasoningMode::Disabled,
            );
            assert!(
                !assembled.contains("思考指引"),
                "Disabled 模式不应注入 CoT 提示，实际: {assembled}"
            );
        }

        #[test]
        fn test_replace_template_vars() {
            let text = "你是 {{char}}，正在和 {{user}} 对话。{{charIfNotUser}} 会回应。";
            let result = replace_template_vars(text, "Seraphina", "玩家");
            assert_eq!(
                result,
                "你是 Seraphina，正在和 玩家 对话。Seraphina 会回应。"
            );
        }

        #[test]
        fn test_replace_template_vars_no_placeholders() {
            let text = "没有占位符的普通文本";
            let result = replace_template_vars(text, "Seraphina", "玩家");
            assert_eq!(result, "没有占位符的普通文本");
        }

        #[test]
        fn test_replace_template_vars_with_card_fields_and_aliases() {
            let ctx = TemplateVarContext {
                char_name: "Seraphina".into(),
                user_name: "玩家".into(),
                description: "银发旅人".into(),
                personality: "温柔而敏锐".into(),
                scenario: "雨夜驿站".into(),
                first_mes: "欢迎回来。".into(),
                mes_example: "<START>\n{{char}}: 你好".into(),
                system_prompt: "保持诗意".into(),
                post_history_instructions: "延续上一轮气氛".into(),
                creator: "tester".into(),
                character_version: "1.2.3".into(),
                tags: vec!["fantasy".into(), "slow-burn".into()],
                ..Default::default()
            };

            let text = "{{char}}/{{Char}}/{{user}}/{{User}}/<BOT>/<user>\n{{description}}\n{{personality}}\n{{scenario}}\n{{first_mes}}\n{{first_message}}\n{{mes_example}}\n{{system_prompt}}\n{{post_history_instructions}}\n{{creator}}\n{{character_version}}\n{{tags}}";
            let result = replace_template_vars_with_context(text, &ctx);

            assert!(result.contains("Seraphina/Seraphina/玩家/玩家/Seraphina/玩家"));
            assert!(result.contains("银发旅人"));
            assert!(result.contains("温柔而敏锐"));
            assert!(result.contains("雨夜驿站"));
            assert!(result.contains("欢迎回来。"));
            assert!(result.contains("<START>"));
            assert!(result.contains("保持诗意"));
            assert!(result.contains("延续上一轮气氛"));
            assert!(result.contains("tester"));
            assert!(result.contains("1.2.3"));
            assert!(result.contains("fantasy, slow-burn"));
        }

        #[test]
        fn test_replace_template_vars_supports_st_state_macros() {
            let ctx = TemplateVarContext::default();
            let text = "{{// comment should disappear }}\n{{setvar::prefix::<utility>}}{{addvar::body::第一段}}{{addvar::body::第二段}}{{setvar::suffix::</utility>}}\n{{trim}}{{getvar::prefix}}{{getvar::body}}{{getvar::suffix}}{{trim}}";

            let result = replace_template_vars_with_context(text, &ctx);

            assert_eq!(result, "<utility>第一段第二段</utility>");
        }

        #[test]
        fn test_replace_template_vars_supports_time_macros_with_fixed_clock() {
            let ctx = TemplateVarContext {
                now: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-07-07T09:08:05Z")
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                ),
                ..Default::default()
            };

            let result = replace_template_vars_with_context(
                "{{date}} {{time}} {{datetime}} {{weekday}} {{isotime}}",
                &ctx,
            );

            assert_eq!(
                result,
                "2026-07-07 09:08 2026-07-07 09:08 Tuesday 2026-07-07T09:08:05+00:00"
            );
        }

        #[test]
        fn test_replace_template_vars_supports_random_and_roll_macros() {
            let ctx = TemplateVarContext {
                user_name: "玩家".into(),
                random_seed: Some(42),
                ..Default::default()
            };

            let first = replace_template_vars_with_context(
                "{{random::红色::晴::<user>}}\n{{roll::2d6+1}}",
                &ctx,
            );
            let second = replace_template_vars_with_context(
                "{{random::红色::晴::<user>}}\n{{roll::2d6+1}}",
                &ctx,
            );

            assert_eq!(first, second, "fixed random_seed should be deterministic");
            let (choice, roll_text) = first.split_once('\n').unwrap();
            assert!(
                ["红色", "晴", "玩家"].contains(&choice),
                "random choice should come from rendered options, got {choice}"
            );
            let roll: i32 = roll_text.parse().unwrap();
            assert!(
                (3..=13).contains(&roll),
                "2d6+1 should stay within dice range, got {roll}"
            );
        }

        #[test]
        fn test_replace_template_vars_preserves_unknown_macros() {
            let result = replace_template_vars("{{unknown::macro}} {{char}}", "Seraphina", "玩家");
            assert_eq!(result, "{{unknown::macro}} Seraphina");
        }

        #[test]
        fn test_role_applicable_subagent_wildcard() {
            // 模块声明 applicable_roles 含 Subagent("*")，应匹配任意 Subagent(id)
            let wildcard = vec![AgentRole::Subagent("*".into())];
            assert!(super::role_applicable(
                &wildcard,
                &AgentRole::Subagent("林医生".into())
            ));
            assert!(super::role_applicable(
                &wildcard,
                &AgentRole::Subagent("any".into())
            ));
            // 精确匹配
            let exact = vec![AgentRole::Director];
            assert!(super::role_applicable(&exact, &AgentRole::Director));
            assert!(!super::role_applicable(&exact, &AgentRole::Editor));
            // 通配符不匹配非 Subagent 角色
            assert!(!super::role_applicable(&wildcard, &AgentRole::Editor));
        }

        /// 回归：子 Agent 角色应能命中 applicable_roles 含 Subagent("*") 的模块。
        /// Bug-2：旧实现 `applicable_roles.contains(role)` 用精确 PartialEq，
        /// `Subagent("林医生") != Subagent("*")`，导致字数控制等子Agent模块永不生效。
        #[test]
        fn test_assemble_subagent_wildcard_module_applies() {
            use std::collections::HashMap;
            // 构造一个只对 Subagent("*") 生效的模块
            let module = PromptModule {
                id: Id::from_str("m-sub-control"),
                name: "子Agent字数控制".into(),
                category: ModuleCategory::Output,
                content: "[子Agent专属约束]".into(),
                exclusivity: crate::prompt_module::Exclusivity::Single,
                source: crate::prompt_module::ModuleSource::BuiltIn,
                applicable_roles: vec![AgentRole::Subagent("*".into())],
                tags: vec![],
            };
            // Profile 把 Subagent("*") 的 Output 类别指向该模块
            let mut selections = HashMap::new();
            let mut cats = HashMap::new();
            cats.insert(ModuleCategory::Output, vec![Id::from_str("m-sub-control")]);
            selections.insert(AgentRole::Subagent("*".into()), cats);
            let profile = PromptProfile {
                id: Id::from_str("p1"),
                name: "test".into(),
                selections,
                overrides: HashMap::new(),
                source: crate::prompt_module::ProfileSource::BuiltIn,
            };

            // 子 Agent 角色组装时应包含该模块
            let out = assemble_system_prompt(
                &AgentRole::Subagent("林医生".into()),
                "你是角色。",
                Some(&profile),
                &[module],
                "",
                &crate::llm::ReasoningMode::default(),
            );
            assert!(
                out.contains("[子Agent专属约束]"),
                "子Agent 应命中 Subagent(\"*\") 通配符模块，实际: {out}"
            );
        }
    }
}

use serde::{Deserialize, Serialize};

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
    pub selections: std::collections::HashMap<AgentRole, std::collections::HashMap<ModuleCategory, Vec<Id>>>,
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
        if let Some(cats) = self.selections.get(role) {
            if let Some(ids) = cats.get(category) {
                return ids.as_slice();
            }
        }
        // 回退：Subagent(id) → Subagent("*")
        if let AgentRole::Subagent(_) = role {
            let wildcard = AgentRole::Subagent("*".into());
            if let Some(cats) = self.selections.get(&wildcard) {
                if let Some(ids) = cats.get(category) {
                    return ids.as_slice();
                }
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
pub fn assemble_system_prompt(
    role: &AgentRole,
    role_directive: &str,
    profile: Option<&PromptProfile>,
    modules: &[PromptModule],
    tool_directives: &str,
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
            let ids = profile.selected_ids(role, cat);
            for mid in ids {
                if let Some(m) = modules.iter().find(|m| &m.id == mid && role_applicable(&m.applicable_roles, role)) {
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

/// ST 风格占位符替换（设计 §7.5 prompt-template 功能）
///
/// 支持的占位符：
/// - `{{char}}` → 角色名
/// - `{{user}}` → 用户名（默认 "玩家"）
/// - `{{charIfNotUser}}` → 如果不是用户则显示角色名（简化为 char）
///
/// 在组装 system prompt 后、发送给 LLM 前调用。
pub fn replace_template_vars(text: &str, char_name: &str, user_name: &str) -> String {
    text.replace("{{char}}", char_name)
        .replace("{{user}}", user_name)
        .replace("{{charIfNotUser}}", char_name)
}

// ─── 预置模块（M1 内置 5 个核心模块）──────────────────────────────────────

/// 预置模块工厂（对应设计 §3.6.5：内置 5-8 个核心模块覆盖 80% 场景）
pub mod builtins {
    use super::*;
    use std::collections::HashMap;

    /// 所有 Agent 角色
    fn all_roles() -> Vec<AgentRole> {
        vec![
            AgentRole::Director,
            AgentRole::Editor,
            AgentRole::Subagent("*".into()),
        ]
    }

    /// 编剧 Agent
    fn editor_only() -> Vec<AgentRole> {
        vec![AgentRole::Editor]
    }

    /// 编剧 + 子 Agent
    fn editor_and_subagent() -> Vec<AgentRole> {
        vec![
            AgentRole::Editor,
            AgentRole::Subagent("*".into()),
        ]
    }

    /// 预置模块列表（5 个核心模块）
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
            // 4. CoT：通用思维链（不绑定特定模型）
            PromptModule {
                id: Id::from_str("builtin-cot-generic"),
                name: "通用思维链".into(),
                category: ModuleCategory::Cot,
                content: "【思考指引】\n在输出正文前，先在内部思考：\n1. 当前场景的核心冲突是什么？\n2. 各角色此刻的情绪状态和下一步动机\n3. 有哪些伏笔可以呼应？\n4. 上一段的结尾是什么？如何自然衔接？\n思考完毕后，直接输出正文，不要输出思考过程。".into(),
                exclusivity: Exclusivity::Single,
                source: ModuleSource::BuiltIn,
                applicable_roles: all_roles(),
                tags: vec!["CoT".into(), "通用".into()],
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

        // 导演：选中 CoT
        let mut director_cats = HashMap::new();
        director_cats.insert(
            ModuleCategory::Cot,
            vec![Id::from_str("builtin-cot-generic")],
        );
        selections.insert(AgentRole::Director, director_cats);

        // 编剧：选中 第三人称 + 白描 + 杀八股 + 字数控制
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
            ModuleCategory::Output,
            vec![Id::from_str("builtin-output-word-count")],
        );
        selections.insert(AgentRole::Editor, editor_cats);

        // 子 Agent：选中 字数控制
        let mut sub_cats = HashMap::new();
        sub_cats.insert(
            ModuleCategory::Output,
            vec![Id::from_str("builtin-output-word-count")],
        );
        selections.insert(AgentRole::Subagent("*".into()), sub_cats);

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
            assert_eq!(modules.len(), 5);
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

            // 导演应有 CoT
            let director_cot = profile.selected_ids(&AgentRole::Director, &ModuleCategory::Cot);
            assert_eq!(director_cot.len(), 1);

            // 编剧应有 视角 + 文风 + 质量 + 输出
            let editor_persp = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Perspective);
            assert_eq!(editor_persp.len(), 1);
            let editor_style = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Style);
            assert_eq!(editor_style.len(), 1);
            let editor_quality = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Quality);
            assert_eq!(editor_quality.len(), 1);
            let editor_output = profile.selected_ids(&AgentRole::Editor, &ModuleCategory::Output);
            assert_eq!(editor_output.len(), 1);

            // 子 Agent 应有 输出
            let sub_output = profile.selected_ids(&AgentRole::Subagent("*".into()), &ModuleCategory::Output);
            assert_eq!(sub_output.len(), 1);
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
            );

            assert!(assembled.contains("你是编剧"));
            assert!(assembled.contains("第三人称叙事"));  // 模块 content，非 name
            assert!(assembled.contains("白描"));           // 模块 content 中含"白描"
            assert!(assembled.contains("杀八股"));         // 模块 content 中含"杀八股"
            assert!(assembled.contains("目标字数"));       // 字数控制模块的 content
            assert!(assembled.contains("工具说明"));
        }

        #[test]
        fn test_replace_template_vars() {
            let text = "你是 {{char}}，正在和 {{user}} 对话。{{charIfNotUser}} 会回应。";
            let result = replace_template_vars(text, "Seraphina", "玩家");
            assert_eq!(result, "你是 Seraphina，正在和 玩家 对话。Seraphina 会回应。");
        }

        #[test]
        fn test_replace_template_vars_no_placeholders() {
            let text = "没有占位符的普通文本";
            let result = replace_template_vars(text, "Seraphina", "玩家");
            assert_eq!(result, "没有占位符的普通文本");
        }

        #[test]
        fn test_role_applicable_subagent_wildcard() {
            // 模块声明 applicable_roles 含 Subagent("*")，应匹配任意 Subagent(id)
            let wildcard = vec![AgentRole::Subagent("*".into())];
            assert!(super::role_applicable(&wildcard, &AgentRole::Subagent("林医生".into())));
            assert!(super::role_applicable(&wildcard, &AgentRole::Subagent("any".into())));
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
            );
            assert!(out.contains("[子Agent专属约束]"),
                "子Agent 应命中 Subagent(\"*\") 通配符模块，实际: {out}");
        }
    }
}

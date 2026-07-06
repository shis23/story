use std::collections::HashMap;

use storyforge_domain::agent::AgentRole;

// ─── 工具适用角色范围 ────────────────────────────────────────────────────

/// 工具的适用角色范围
///
/// 决定该工具默认出现在哪些角色的可选池里。
/// `All` = 所有角色都默认可用；`Roles` = 仅指定角色默认可用。
/// `Subagent("*")` 作为通配，匹配任意 `Subagent(_)` 实例。
#[derive(Debug, Clone)]
pub enum ToolScope {
    /// 所有角色都默认可用
    All,
    /// 仅指定角色默认可用（Subagent("*") 匹配所有子 Agent）
    Roles(Vec<AgentRole>),
}

// ─── 工具摘要（给前端选配 UI 用）────────────────────────────────────────

/// 全局工具注册项（元信息，不含 handler）
#[derive(Debug, Clone)]
pub struct ToolSummary {
    /// 工具唯一名称（如 "search_world_info" 或 "subagent.get_character"）
    pub name: String,
    /// 人类可读说明
    pub description: String,
    /// 适用角色范围
    pub scope: ToolScope,
}

// ─── 通配匹配 ────────────────────────────────────────────────────────────

/// 判断 `declared` scope 中的角色声明是否匹配 `actual` 角色。
///
/// 规则：
/// - `Subagent("*")` 匹配任意 `Subagent(_)`（含具体 id 和 "*"
/// - 其他变体严格相等
pub fn role_matches(declared: &AgentRole, actual: &AgentRole) -> bool {
    match (declared, actual) {
        // Subagent("*") 通配：匹配任意 Subagent
        (AgentRole::Subagent(wild), AgentRole::Subagent(_)) if wild == "*" => true,
        // 其他情况严格相等
        _ => declared == actual,
    }
}

// ─── ToolCenter ──────────────────────────────────────────────────────────

/// 全局工具注册中心（纯元信息层）
///
/// **重要**：handler 是闭包（捕获 `ToolContext`），无法跨角色共享同一实例。
/// `ToolCenter` 的职责是「登记工具元信息 + 按角色筛选默认工具名」。
/// 真正构造 `ToolRegistry` 仍由调用方调对应 `register_*_tools` ——
/// 这是第一阶段，不强制改造调用点。
pub struct ToolCenter {
    /// 工具元信息（名字 → summary）
    summaries: HashMap<String, ToolSummary>,
}

impl ToolCenter {
    /// 启动时构建：登记所有已知工具的元信息
    pub fn builtin() -> Self {
        let mut c = Self {
            summaries: HashMap::new(),
        };
        // 通用只读工具（scope = All）
        c.reg("search_world_info", "按关键词搜索世界书", ToolScope::All);
        c.reg("search_vectors", "向量记忆搜索", ToolScope::All);
        c.reg("get_recent_summary", "获取远记忆摘要", ToolScope::All);
        // Director 专属
        c.reg(
            "get_character",
            "查任意角色详情（Director 版，可见全部）",
            ToolScope::Roles(vec![AgentRole::Director]),
        );
        c.reg(
            "emit_plan",
            "导演输出 Plan",
            ToolScope::Roles(vec![AgentRole::Director]),
        );
        // Subagent 专属（信息隔离版同名，scope 限 Subagent 通配）
        c.reg(
            "subagent.get_character",
            "子 Agent 查自己 instance（信息隔离）",
            ToolScope::Roles(vec![AgentRole::Subagent("*".into())]),
        );
        // Editor 专属
        c.reg(
            "compose",
            "编剧声明完成",
            ToolScope::Roles(vec![AgentRole::Editor]),
        );
        // PostProcessor 专属
        c.reg(
            "emit_postprocess",
            "后处理三合一输出",
            ToolScope::Roles(vec![AgentRole::PostProcessor]),
        );
        // CharacterExtractor 专属
        c.reg(
            "emit_characters",
            "卡导入时识别角色",
            ToolScope::Roles(vec![AgentRole::CharacterExtractor]),
        );
        c
    }

    /// 某角色的默认工具名列表（按 scope 筛选）
    pub fn default_tool_names_for(&self, role: &AgentRole) -> Vec<String> {
        self.summaries
            .iter()
            .filter(|(_, s)| match &s.scope {
                ToolScope::All => true,
                ToolScope::Roles(rs) => rs.iter().any(|r| role_matches(r, role)),
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// 所有工具摘要（给前端选配 UI 用）
    pub fn all_summaries(&self) -> Vec<ToolSummary> {
        self.summaries.values().cloned().collect()
    }

    // ── 内部 helper ──

    fn reg(&mut self, name: &str, description: &str, scope: ToolScope) {
        self.summaries.insert(
            name.to_string(),
            ToolSummary {
                name: name.to_string(),
                description: description.to_string(),
                scope,
            },
        );
    }
}

// ─── 测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn center() -> ToolCenter {
        ToolCenter::builtin()
    }

    // ── role_matches ──

    #[test]
    fn role_matches_subagent_wildcard_matches_any_subagent() {
        let wildcard = AgentRole::Subagent("*".into());
        let concrete = AgentRole::Subagent("inst-abc".into());
        assert!(role_matches(&wildcard, &concrete));
        assert!(role_matches(&wildcard, &wildcard));
    }

    #[test]
    fn role_matches_director_does_not_match_subagent() {
        let director = AgentRole::Director;
        let subagent = AgentRole::Subagent("inst-abc".into());
        assert!(!role_matches(&director, &subagent));
    }

    #[test]
    fn role_matches_exact_equality_for_unit_variants() {
        assert!(role_matches(&AgentRole::Editor, &AgentRole::Editor));
        assert!(!role_matches(&AgentRole::Editor, &AgentRole::PostProcessor));
    }

    // ── default_tool_names_for ──

    #[test]
    fn director_default_tools_include_five() {
        let c = center();
        let names = c.default_tool_names_for(&AgentRole::Director);
        // 3 个 All + 2 个 Director-only = 5
        assert!(
            names.contains(&"search_world_info".into()),
            "missing search_world_info"
        );
        assert!(
            names.contains(&"search_vectors".into()),
            "missing search_vectors"
        );
        assert!(
            names.contains(&"get_recent_summary".into()),
            "missing get_recent_summary"
        );
        assert!(
            names.contains(&"get_character".into()),
            "missing get_character"
        );
        assert!(names.contains(&"emit_plan".into()), "missing emit_plan");
        assert_eq!(
            names.len(),
            5,
            "Director should have 5 tools, got {}",
            names.len()
        );
    }

    #[test]
    fn subagent_with_concrete_id_gets_wildcard_tools() {
        let c = center();
        let names = c.default_tool_names_for(&AgentRole::Subagent("inst-xyz".into()));
        // 3 All + 1 subagent.get_character = 4
        assert!(names.contains(&"search_world_info".into()));
        assert!(
            names.contains(&"subagent.get_character".into()),
            "subagent should have subagent.get_character"
        );
        assert_eq!(
            names.len(),
            4,
            "Subagent(id) should have 4 tools, got {}",
            names.len()
        );
    }

    #[test]
    fn subagent_wildcard_gets_wildcard_tools() {
        let c = center();
        let names = c.default_tool_names_for(&AgentRole::Subagent("*".into()));
        assert!(names.contains(&"subagent.get_character".into()));
        assert_eq!(names.len(), 4);
    }

    #[test]
    fn editor_default_tools_include_compose() {
        let c = center();
        let names = c.default_tool_names_for(&AgentRole::Editor);
        assert!(
            names.contains(&"compose".into()),
            "Editor should have compose"
        );
        // 3 All + 1 Editor = 4
        assert_eq!(
            names.len(),
            4,
            "Editor should have 4 tools, got {}",
            names.len()
        );
    }

    #[test]
    fn postprocessor_default_tools_include_emit_postprocess() {
        let c = center();
        let names = c.default_tool_names_for(&AgentRole::PostProcessor);
        assert!(
            names.contains(&"emit_postprocess".into()),
            "PostProcessor should have emit_postprocess"
        );
        // 3 All + 1 PostProcessor = 4
        assert_eq!(
            names.len(),
            4,
            "PostProcessor should have 4 tools, got {}",
            names.len()
        );
    }

    #[test]
    fn common_tools_appear_in_director_and_subagent_lists() {
        let c = center();
        let director_names = c.default_tool_names_for(&AgentRole::Director);
        let subagent_names = c.default_tool_names_for(&AgentRole::Subagent("id".into()));
        for name in &["search_world_info", "search_vectors", "get_recent_summary"] {
            assert!(
                director_names.contains(&name.to_string()),
                "Director missing {name}"
            );
            assert!(
                subagent_names.contains(&name.to_string()),
                "Subagent missing {name}"
            );
        }
    }

    // ── all_summaries ──

    #[test]
    fn all_summaries_returns_all_tools() {
        let c = center();
        let summaries = c.all_summaries();
        // 9 tools total:
        //   search_world_info, search_vectors, get_recent_summary (All)
        //   get_character, emit_plan (Director)
        //   subagent.get_character (Subagent)
        //   compose (Editor)
        //   emit_postprocess (PostProcessor)
        //   emit_characters (CharacterExtractor)
        assert!(
            summaries.len() >= 9,
            "expected >= 9 tool summaries, got {}",
            summaries.len()
        );
    }
}

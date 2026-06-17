use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Id;
use crate::agent::AgentRole;
use crate::prompt_module::ProfileSource;

// ─── Default helpers ────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

fn default_max_concurrent_subagents() -> usize {
    4
}

fn default_config_version() -> u32 {
    1
}

fn default_source() -> ProfileSource {
    ProfileSource::UserCreated
}

// ─── AgentRunConfig（单角色运行时覆盖）──────────────────────────────────────

/// 单个 Agent 角色的运行时配置覆盖
///
/// 所有字段都是 `Option`：`None` 表示使用当前默认值（连接模型/硬编码常量）。
/// 旧数据反序列化时缺失字段自动填 `None`，不会崩。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentRunConfig {
    /// 模型覆盖（None = 使用连接默认模型）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,

    /// 最大工具轮次（None = 使用角色默认值）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_rounds: Option<u32>,

    /// 工具白名单（None = 使用默认工具集，Some(vec![]) = 禁用所有工具，Some(list) = 只允许列表中的工具）
    ///
    /// 注意：当前版本存储并验证此字段，但不做过滤运行时工具。
    /// 运行时过滤将在后续版本实现。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_whitelist: Option<Vec<String>>,
}

// ─── AgentProfileConfig（完整配置）────────────────────────────────────────

/// 完整的 Agent Profile 配置（持久化用）
///
/// 包含每个 Agent 角色的运行参数覆盖、并发控制、后处理开关等。
/// `agent_configs` 用 `AgentRole` 作为 key，支持 Director/Editor/Subagent 等角色的独立配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfileConfig {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 每个 Agent 角色的运行参数覆盖
    #[serde(default)]
    pub agent_configs: HashMap<AgentRole, AgentRunConfig>,
    /// 子 Agent 最大并发数
    #[serde(default = "default_max_concurrent_subagents")]
    pub max_concurrent_subagents: usize,
    /// 是否启用后处理（知识/变量/任务提取）
    #[serde(default = "default_true")]
    pub enable_postprocess: bool,
    /// 是否启用剧情总结
    #[serde(default = "default_true")]
    pub enable_summarizer: bool,
    /// 来源
    #[serde(default = "default_source")]
    pub source: ProfileSource,
    /// 版本号（用于未来迁移）
    #[serde(default = "default_config_version")]
    pub config_version: u32,
}

/// 列表/摘要 DTO（前端列表页用，不含详细 agent_configs）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfileConfigSummaryDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub is_active: bool,
    pub max_concurrent_subagents: usize,
    pub enable_postprocess: bool,
    pub enable_summarizer: bool,
}

// ─── 内置默认 Profile ──────────────────────────────────────────────────

/// 内置默认 AgentProfileConfig 的稳定 ID
pub const BUILTIN_DEFAULT_AGENT_PROFILE_ID: &str = "builtin-default-agent-v1";

/// 内置默认 AgentProfileConfig，与当前硬编码行为一致
///
/// - agent_configs 为空（所有角色使用硬编码默认值）
/// - max_concurrent_subagents = 4
/// - enable_postprocess = true
/// - enable_summarizer = true
pub fn default_agent_profile_config() -> AgentProfileConfig {
    AgentProfileConfig {
        id: Id::from_str(BUILTIN_DEFAULT_AGENT_PROFILE_ID),
        name: "默认 Agent 配置".into(),
        description: "与当前硬编码行为一致的默认配置。导演 15 轮，编剧 5 轮，子 Agent 10 轮，并发 4。".into(),
        agent_configs: HashMap::new(),
        max_concurrent_subagents: default_max_concurrent_subagents(),
        enable_postprocess: true,
        enable_summarizer: true,
        source: ProfileSource::BuiltIn,
        config_version: 1,
    }
}

// ─── 查询 helpers ─────────────────────────────────────────────────────

impl AgentProfileConfig {
    /// 查找某个角色的运行配置。支持 Subagent 通配符回退：
    /// 先查精确的 `Subagent(id)`，找不到则回退到 `Subagent("*")`。
    pub fn run_config_for(&self, role: &AgentRole) -> &AgentRunConfig {
        static EMPTY: AgentRunConfig = AgentRunConfig {
            model_override: None,
            max_tool_rounds: None,
            tool_whitelist: None,
        };

        if let Some(cfg) = self.agent_configs.get(role) {
            return cfg;
        }
        // Subagent 通配符回退
        if let AgentRole::Subagent(_) = role {
            let wildcard = AgentRole::Subagent("*".into());
            if let Some(cfg) = self.agent_configs.get(&wildcard) {
                return cfg;
            }
        }
        &EMPTY
    }

    /// 是否为内置默认配置
    pub fn is_builtin(&self) -> bool {
        self.source == ProfileSource::BuiltIn
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_current_behavior() {
        let cfg = default_agent_profile_config();
        assert_eq!(cfg.id.to_string(), BUILTIN_DEFAULT_AGENT_PROFILE_ID);
        assert_eq!(cfg.max_concurrent_subagents, 4);
        assert!(cfg.enable_postprocess);
        assert!(cfg.enable_summarizer);
        assert!(cfg.agent_configs.is_empty());
        assert!(cfg.is_builtin());
    }

    #[test]
    fn serde_round_trip() {
        let cfg = default_agent_profile_config();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let back: AgentProfileConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id.to_string(), cfg.id.to_string());
        assert_eq!(back.name, cfg.name);
        assert_eq!(back.max_concurrent_subagents, cfg.max_concurrent_subagents);
        assert_eq!(back.enable_postprocess, cfg.enable_postprocess);
        assert_eq!(back.enable_summarizer, cfg.enable_summarizer);
        assert_eq!(back.config_version, cfg.config_version);
    }

    #[test]
    fn partial_json_fills_defaults() {
        // 最小 JSON：只有 id 和 name
        let json = r#"{"id":"test-1","name":"test"}"#;
        let cfg: AgentProfileConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.id.to_string(), "test-1");
        assert_eq!(cfg.name, "test");
        assert_eq!(cfg.description, "");
        assert!(cfg.agent_configs.is_empty());
        assert_eq!(cfg.max_concurrent_subagents, 4);
        assert!(cfg.enable_postprocess);
        assert!(cfg.enable_summarizer);
        assert_eq!(cfg.config_version, 1);
    }

    #[test]
    fn empty_json_deserializes_with_defaults() {
        let json = r#"{"id":"x","name":"y"}"#;
        let cfg: AgentProfileConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.max_concurrent_subagents, 4);
        assert!(cfg.enable_postprocess);
        assert!(cfg.enable_summarizer);
    }

    #[test]
    fn role_lookup_director() {
        let mut cfg = default_agent_profile_config();
        cfg.agent_configs.insert(
            AgentRole::Director,
            AgentRunConfig {
                model_override: Some("gpt-4".into()),
                max_tool_rounds: Some(20),
                tool_whitelist: None,
            },
        );
        let run = cfg.run_config_for(&AgentRole::Director);
        assert_eq!(run.model_override.as_deref(), Some("gpt-4"));
        assert_eq!(run.max_tool_rounds, Some(20));
    }

    #[test]
    fn role_lookup_editor() {
        let mut cfg = default_agent_profile_config();
        cfg.agent_configs.insert(
            AgentRole::Editor,
            AgentRunConfig {
                model_override: None,
                max_tool_rounds: Some(8),
                tool_whitelist: None,
            },
        );
        let run = cfg.run_config_for(&AgentRole::Editor);
        assert_eq!(run.max_tool_rounds, Some(8));
    }

    #[test]
    fn role_lookup_subagent_wildcard_fallback() {
        let mut cfg = default_agent_profile_config();
        cfg.agent_configs.insert(
            AgentRole::Subagent("*".into()),
            AgentRunConfig {
                model_override: Some("claude-sonnet".into()),
                max_tool_rounds: Some(12),
                tool_whitelist: None,
            },
        );
        // 精确匹配不存在，应回退到 Subagent("*")
        let run = cfg.run_config_for(&AgentRole::Subagent("林医生".into()));
        assert_eq!(run.model_override.as_deref(), Some("claude-sonnet"));
        assert_eq!(run.max_tool_rounds, Some(12));
    }

    #[test]
    fn role_lookup_exact_subagent_overrides_wildcard() {
        let mut cfg = default_agent_profile_config();
        cfg.agent_configs.insert(
            AgentRole::Subagent("*".into()),
            AgentRunConfig {
                model_override: Some("wildcard-model".into()),
                max_tool_rounds: None,
                tool_whitelist: None,
            },
        );
        cfg.agent_configs.insert(
            AgentRole::Subagent("林医生".into()),
            AgentRunConfig {
                model_override: Some("exact-model".into()),
                max_tool_rounds: Some(5),
                tool_whitelist: None,
            },
        );
        // 精确匹配优先
        let run = cfg.run_config_for(&AgentRole::Subagent("林医生".into()));
        assert_eq!(run.model_override.as_deref(), Some("exact-model"));
        assert_eq!(run.max_tool_rounds, Some(5));

        // 其他角色走通配符
        let run2 = cfg.run_config_for(&AgentRole::Subagent("其他人".into()));
        assert_eq!(run2.model_override.as_deref(), Some("wildcard-model"));
    }

    #[test]
    fn role_lookup_missing_returns_empty() {
        let cfg = default_agent_profile_config();
        let run = cfg.run_config_for(&AgentRole::Director);
        assert!(run.model_override.is_none());
        assert!(run.max_tool_rounds.is_none());
        assert!(run.tool_whitelist.is_none());
    }

    #[test]
    fn agent_run_config_serde_round_trip() {
        let cfg = AgentRunConfig {
            model_override: Some("deepseek-chat".into()),
            max_tool_rounds: Some(10),
            tool_whitelist: Some(vec!["search_world_info".into(), "get_character".into()]),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AgentRunConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model_override, cfg.model_override);
        assert_eq!(back.max_tool_rounds, cfg.max_tool_rounds);
        assert_eq!(back.tool_whitelist, cfg.tool_whitelist);
    }

    #[test]
    fn agent_run_config_empty_json() {
        let json = r#"{}"#;
        let cfg: AgentRunConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.model_override.is_none());
        assert!(cfg.max_tool_rounds.is_none());
        assert!(cfg.tool_whitelist.is_none());
    }

    #[test]
    fn config_with_agent_configs_serde() {
        let mut agent_configs = HashMap::new();
        agent_configs.insert(
            AgentRole::Director,
            AgentRunConfig {
                model_override: Some("gpt-4o".into()),
                max_tool_rounds: Some(20),
                tool_whitelist: None,
            },
        );
        agent_configs.insert(
            AgentRole::Subagent("*".into()),
            AgentRunConfig {
                model_override: None,
                max_tool_rounds: Some(8),
                tool_whitelist: Some(vec!["get_character".into()]),
            },
        );
        let cfg = AgentProfileConfig {
            id: Id::from_str("custom-1"),
            name: "自定义配置".into(),
            description: "测试".into(),
            agent_configs,
            max_concurrent_subagents: 2,
            enable_postprocess: true,
            enable_summarizer: false,
            source: ProfileSource::UserCreated,
            config_version: 1,
        };
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let back: AgentProfileConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "自定义配置");
        assert_eq!(back.max_concurrent_subagents, 2);
        assert!(!back.enable_summarizer);
        let director = back.run_config_for(&AgentRole::Director);
        assert_eq!(director.model_override.as_deref(), Some("gpt-4o"));
        assert_eq!(director.max_tool_rounds, Some(20));
    }

    #[test]
    fn validation_max_concurrent_subagents_bounds() {
        // 0 不合理但不 panic（运行时会退化为串行）
        let mut cfg = default_agent_profile_config();
        cfg.max_concurrent_subagents = 0;
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AgentProfileConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.max_concurrent_subagents, 0);
    }

    #[test]
    fn tool_whitelist_empty_vec_vs_none() {
        let cfg_none = AgentRunConfig {
            model_override: None,
            max_tool_rounds: None,
            tool_whitelist: None,
        };
        let cfg_empty = AgentRunConfig {
            model_override: None,
            max_tool_rounds: None,
            tool_whitelist: Some(vec![]),
        };
        // None = 使用默认工具集，Some(vec![]) = 禁用所有工具
        assert!(cfg_none.tool_whitelist.is_none());
        assert!(cfg_empty.tool_whitelist.as_ref().unwrap().is_empty());
    }
}

//! Campaign health check（确定性数据校验，零 LLM）
//!
//! 扫描 Campaign 数据快照，找出孤立 instance、未解析的知识引用、孤儿任务等。
//! 对应 ROADMAP Phase 3：Meta Agent Campaign 诊断入口。

use serde::{Deserialize, Serialize};
use storyforge_domain::Id;
use storyforge_domain::campaign::CharacterInstance;
use storyforge_domain::character::CharacterDefinition;
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::story_task::StoryTask;

// ─── 数据结构 ──────────────────────────────────────────────────────────────

/// 问题严重程度
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Warning,
    Error,
}

/// 一条健康检查问题
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthIssue {
    pub severity: IssueSeverity,
    pub category: String,
    pub message: String,
    pub affected_id: Option<String>,
}

/// Campaign 数据快照（用于 health check，纯只读输入）
pub struct CampaignHealthSnapshot<'a> {
    pub instances: &'a [CharacterInstance],
    pub definitions: &'a [CharacterDefinition],
    pub knowledge: &'a [CharacterKnowledgeEntry],
    pub tasks: &'a [StoryTask],
}

// ─── 核心检查 ──────────────────────────────────────────────────────────────

/// 对 Campaign 做健康检查，返回所有发现的问题。
///
/// 确定性检查，不调 LLM，纯数据校验。
pub fn check_campaign_health(snapshot: &CampaignHealthSnapshot) -> Vec<HealthIssue> {
    let mut issues = Vec::new();

    check_orphan_instances(&mut issues, snapshot);
    check_unresolved_knowledge(&mut issues, snapshot);
    check_orphan_task_references(&mut issues, snapshot);
    check_variable_schema_mismatch(&mut issues, snapshot);

    issues
}

/// 检查孤立 instance：definition_id 指向不存在的 definition
fn check_orphan_instances(issues: &mut Vec<HealthIssue>, snapshot: &CampaignHealthSnapshot) {
    let def_ids: std::collections::HashSet<&Id> =
        snapshot.definitions.iter().map(|d| &d.id).collect();

    for inst in snapshot.instances {
        if let Some(ref def_id) = inst.definition_id {
            if !def_ids.contains(def_id) {
                issues.push(HealthIssue {
                    severity: IssueSeverity::Error,
                    category: "orphan_instance".into(),
                    message: format!(
                        "角色实例「{}」的 definition_id 指向不存在的角色定义 ({})",
                        inst.name,
                        def_id.as_str()
                    ),
                    affected_id: Some(inst.id.to_string()),
                });
            }
        }
    }
}

/// 检查未解析的知识引用：knowledge 的 character_id 找不到对应 instance
fn check_unresolved_knowledge(issues: &mut Vec<HealthIssue>, snapshot: &CampaignHealthSnapshot) {
    let instance_ids: std::collections::HashSet<&Id> =
        snapshot.instances.iter().map(|i| &i.id).collect();

    for entry in snapshot.knowledge {
        if !instance_ids.contains(&entry.character_id) {
            issues.push(HealthIssue {
                severity: IssueSeverity::Warning,
                category: "unresolved_knowledge".into(),
                message: format!(
                    "知识条目「{}」的角色 ID ({}) 在当前 Campaign 中无对应实例",
                    truncate(&entry.knowledge_text, 30),
                    entry.character_id.as_str()
                ),
                affected_id: Some(entry.id.to_string()),
            });
        }
    }
}

/// 检查孤儿任务引用：task.related_characters 引用了不存在的 instance
fn check_orphan_task_references(issues: &mut Vec<HealthIssue>, snapshot: &CampaignHealthSnapshot) {
    let instance_ids: std::collections::HashSet<&Id> =
        snapshot.instances.iter().map(|i| &i.id).collect();

    for task in snapshot.tasks {
        for char_id in &task.related_characters {
            if !instance_ids.contains(char_id) {
                issues.push(HealthIssue {
                    severity: IssueSeverity::Warning,
                    category: "orphan_task_reference".into(),
                    message: format!(
                        "任务「{}」关联的角色 ID ({}) 在当前 Campaign 中无对应实例",
                        task.title,
                        char_id.as_str()
                    ),
                    affected_id: Some(task.id.to_string()),
                });
            }
        }
    }
}

/// 检查变量 schema 不一致：非临时 instance 的 definition 有 variable_schema，
/// 但 instance 的变量键集与 schema 键集不匹配
fn check_variable_schema_mismatch(
    issues: &mut Vec<HealthIssue>,
    snapshot: &CampaignHealthSnapshot,
) {
    let def_map: std::collections::HashMap<&Id, &CharacterDefinition> =
        snapshot.definitions.iter().map(|d| (&d.id, d)).collect();

    for inst in snapshot.instances {
        if inst.is_temporary {
            continue;
        }
        if let Some(ref def_id) = inst.definition_id {
            if let Some(def) = def_map.get(def_id) {
                if def.variable_schema.is_empty() {
                    continue;
                }
                let schema_keys: std::collections::HashSet<&str> =
                    def.variable_schema.iter().map(|f| f.key.as_str()).collect();
                let instance_keys: std::collections::HashSet<&str> =
                    inst.variables.iter().map(|v| v.key.as_str()).collect();

                let missing: Vec<&str> = schema_keys.difference(&instance_keys).copied().collect();
                let extra: Vec<&str> = instance_keys.difference(&schema_keys).copied().collect();

                if !missing.is_empty() || !extra.is_empty() {
                    let mut detail = String::new();
                    if !missing.is_empty() {
                        detail.push_str(&format!("缺失: {:?} ", missing));
                    }
                    if !extra.is_empty() {
                        detail.push_str(&format!("多余: {:?}", extra));
                    }
                    issues.push(HealthIssue {
                        severity: IssueSeverity::Warning,
                        category: "variable_schema_mismatch".into(),
                        message: format!(
                            "角色实例「{}」的变量与定义 schema 不一致 ({})",
                            inst.name,
                            detail.trim()
                        ),
                        affected_id: Some(inst.id.to_string()),
                    });
                }
            }
        }
    }
}

// ─── 辅助 ──────────────────────────────────────────────────────────────────

fn truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", chars[..max_chars].iter().collect::<String>())
    }
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::campaign::CharacterInstance;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::story_task::{StoryTask, TaskTrigger};

    fn make_def(id: &str) -> CharacterDefinition {
        CharacterDefinition {
            id: Id::from_str(id),
            card_id: Id::from_str("card-1"),
            name: format!("角色-{id}"),
            persona_prompt: "测试".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: storyforge_domain::variables::default_character_variables(),
        }
    }

    fn make_def_no_schema(id: &str) -> CharacterDefinition {
        CharacterDefinition {
            id: Id::from_str(id),
            card_id: Id::from_str("card-1"),
            name: format!("角色-{id}"),
            persona_prompt: "测试".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        }
    }

    fn make_instance(inst_id: &str, def_id: Option<&str>) -> CharacterInstance {
        let def = def_id.map(|d| make_def(d));
        if let Some(ref d) = def {
            let mut inst = CharacterInstance::from_definition(Id::from_str("camp-1"), d);
            // override id for deterministic tests
            inst.id = Id::from_str(inst_id);
            inst
        } else {
            let mut inst = CharacterInstance::temporary(Id::from_str("camp-1"), "临时角色");
            inst.id = Id::from_str(inst_id);
            inst
        }
    }

    #[test]
    fn test_clean_campaign_has_no_issues() {
        let def = make_def("def-1");
        let mut inst = CharacterInstance::from_definition(Id::from_str("camp-1"), &def);
        inst.id = Id::from_str("inst-1");

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        assert!(issues.is_empty(), "clean campaign should have 0 issues");
    }

    #[test]
    fn test_orphan_instance_detected() {
        let def = make_def("def-real");
        // instance points to non-existent def
        let mut inst =
            CharacterInstance::from_definition(Id::from_str("camp-1"), &make_def("def-ghost"));
        inst.id = Id::from_str("inst-orphan");

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def], // only def-real exists
            knowledge: &[],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "orphan_instance");
        assert_eq!(issues[0].severity, IssueSeverity::Error);
        assert!(issues[0].message.contains("def-ghost"));
        assert_eq!(issues[0].affected_id, Some("inst-orphan".to_string()));
    }

    #[test]
    fn test_temporary_instance_no_definition_not_orphan() {
        let inst = make_instance("inst-tmp", None);

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        // temporary instance with definition_id=None should NOT be flagged as orphan
        assert!(
            issues.is_empty(),
            "temporary instance without definition should not be flagged"
        );
    }

    #[test]
    fn test_unresolved_knowledge_detected() {
        let def = make_def("def-1");
        let inst = make_instance("inst-1", Some("def-1"));

        // knowledge points to non-existent character
        let entry = CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            Id::from_str("ghost-char"),
            "看到了什么",
            1,
        );

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[entry],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "unresolved_knowledge");
        assert_eq!(issues[0].severity, IssueSeverity::Warning);
    }

    #[test]
    fn test_knowledge_to_existing_instance_no_issue() {
        let def = make_def("def-1");
        let inst = make_instance("inst-1", Some("def-1"));

        let entry = CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            Id::from_str("inst-1"), // matches inst
            "看到了什么",
            1,
        );

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[entry],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_orphan_task_reference_detected() {
        let def = make_def("def-1");
        let inst = make_instance("inst-1", Some("def-1"));

        let mut task = StoryTask::user_planned(
            Id::from_str("camp-1"),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        );
        task.related_characters = vec![Id::from_str("inst-ghost")]; // non-existent

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[],
            tasks: &[task],
        };
        let issues = check_campaign_health(&snapshot);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "orphan_task_reference");
        assert_eq!(issues[0].severity, IssueSeverity::Warning);
    }

    #[test]
    fn test_task_with_valid_reference_no_issue() {
        let def = make_def("def-1");
        let inst = make_instance("inst-1", Some("def-1"));

        let mut task = StoryTask::user_planned(
            Id::from_str("camp-1"),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        );
        task.related_characters = vec![Id::from_str("inst-1")]; // matches

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[],
            tasks: &[task],
        };
        let issues = check_campaign_health(&snapshot);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_variable_schema_mismatch_detected() {
        let mut def = make_def("def-1");
        // add a custom field to schema
        def.variable_schema
            .push(storyforge_domain::variables::VariableField {
                key: "custom_var".into(),
                label: "自定义变量".into(),
                value_type: storyforge_domain::variables::VariableType::Int,
                default: serde_json::json!(0),
                description: None,
                group: Some("状态".into()),
            });

        let inst = make_instance("inst-1", Some("def-1"));
        // inst was from_definition with default_character_variables(), which has hp, mana, fatigue, mood
        // but def now also has custom_var → missing

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        let var_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.category == "variable_schema_mismatch")
            .collect();
        assert_eq!(var_issues.len(), 1);
        assert_eq!(var_issues[0].severity, IssueSeverity::Warning);
        assert!(var_issues[0].message.contains("custom_var"));
    }

    #[test]
    fn test_temporary_instance_skips_schema_check() {
        let def = make_def("def-1");
        let inst = make_instance("inst-tmp", None); // temporary, no definition

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        let var_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.category == "variable_schema_mismatch")
            .collect();
        assert!(
            var_issues.is_empty(),
            "temporary instances should skip schema check"
        );
    }

    #[test]
    fn test_definition_with_empty_schema_skips_check() {
        let def = make_def_no_schema("def-1");
        let mut inst = CharacterInstance::from_definition(Id::from_str("camp-1"), &def);
        inst.id = Id::from_str("inst-1");

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[def],
            knowledge: &[],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);
        let var_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.category == "variable_schema_mismatch")
            .collect();
        assert!(var_issues.is_empty(), "empty schema should skip check");
    }

    #[test]
    fn test_multiple_issues_accumulated() {
        // orphan instance + unresolved knowledge + orphan task reference
        let def = make_def("def-real");
        let mut inst_orphan =
            CharacterInstance::from_definition(Id::from_str("camp-1"), &make_def("def-ghost"));
        inst_orphan.id = Id::from_str("inst-orphan");

        let entry = CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            Id::from_str("ghost-char"),
            "幽灵知识",
            1,
        );

        let mut task = StoryTask::user_planned(
            Id::from_str("camp-1"),
            "孤儿任务",
            "指向幽灵",
            vec![TaskTrigger::TurnReminder { at_turn: 1 }],
            1,
        );
        task.related_characters = vec![Id::from_str("inst-missing")];

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst_orphan],
            definitions: &[def],
            knowledge: &[entry],
            tasks: &[task],
        };
        let issues = check_campaign_health(&snapshot);
        assert!(issues.len() >= 3, "should find at least 3 issues");

        let categories: Vec<&str> = issues.iter().map(|i| i.category.as_str()).collect();
        assert!(categories.contains(&"orphan_instance"));
        assert!(categories.contains(&"unresolved_knowledge"));
        assert!(categories.contains(&"orphan_task_reference"));
    }

    #[test]
    fn test_severity_levels() {
        // orphan instance = Error, knowledge/task = Warning
        let mut inst =
            CharacterInstance::from_definition(Id::from_str("camp-1"), &make_def("def-ghost"));
        inst.id = Id::from_str("inst-orphan");

        let entry = CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            Id::from_str("ghost"),
            "x",
            1,
        );

        let snapshot = CampaignHealthSnapshot {
            instances: &[inst],
            definitions: &[],
            knowledge: &[entry],
            tasks: &[],
        };
        let issues = check_campaign_health(&snapshot);

        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == IssueSeverity::Error)
            .collect();
        let warnings: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == IssueSeverity::Warning)
            .collect();
        assert!(!errors.is_empty(), "should have at least one Error");
        assert!(!warnings.is_empty(), "should have at least one Warning");
    }
}

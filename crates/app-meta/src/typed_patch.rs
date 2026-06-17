//! 类型化 Patch DTO（第三轮：campaign-runtime 修复闭环）
//!
//! 本文件是 Task C 的编译桩，Task A 会用完整实现替换。
//! 签名严格对齐 ROUND-3-README.md 锁死的契约。

use serde::{Deserialize, Serialize};
use storyforge_domain::Id;

/// 类型化修复操作（每个变体 = 一种 health issue 的修复）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypedPatchAction {
    /// 修变量 schema 不一致：为 instance 补齐缺失字段 / 删除多余字段
    SyncInstanceVariables {
        instance_id: Id,
        definition_id: Id,
        add_keys: Vec<String>,
        remove_keys: Vec<String>,
    },
    /// 修孤儿任务引用：从 task.related_characters 移除不存在的 id
    PruneOrphanTaskReferences {
        task_id: Id,
        orphan_character_ids: Vec<Id>,
    },
    /// 修未解析知识引用：删除指向不存在 instance 的知识条目
    DeleteOrphanKnowledge {
        knowledge_id: Id,
    },
    /// 修孤立 instance：把 definition_id 改成现存 definition（或清空为临时角色）
    RepointInstanceDefinition {
        instance_id: Id,
        new_definition_id: Option<Id>,
    },
}

/// 单个字段变更（diff 用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDiff {
    pub path: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}

/// 一条修复建议（可含多个 action）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedPatch {
    pub id: String,
    pub description: String,
    pub source_issue_category: String,
    pub affected_id: Option<String>,
    pub actions: Vec<TypedPatchAction>,
    pub diff: Vec<FieldDiff>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub status: TypedPatchStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TypedPatchStatus {
    Pending,
    Accepted,
    Dismissed,
    Stale,
}

/// preview 的输入快照（不可变）
pub struct PreviewInput<'a> {
    pub instances: &'a [storyforge_domain::campaign::CharacterInstance],
    pub definitions: &'a [storyforge_domain::character::CharacterDefinition],
    pub knowledge: &'a [storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
    pub tasks: &'a [storyforge_domain::story_task::StoryTask],
}

/// preview 的输入快照（可变，纯函数预演用）
pub struct PreviewInputMut {
    pub instances: Vec<storyforge_domain::campaign::CharacterInstance>,
    pub definitions: Vec<storyforge_domain::character::CharacterDefinition>,
    pub knowledge: Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>,
    pub tasks: Vec<storyforge_domain::story_task::StoryTask>,
}

#[derive(Debug, thiserror::Error)]
pub enum TypedPatchError {
    #[error("patch target 不存在: {0}")]
    TargetMissing(String),
    #[error("patch 已过期")]
    Stale,
}

/// 从一个 health issue 构造对应的修复 patch
pub fn build_patch_for_issue(
    issue: &crate::HealthIssue,
    input: &PreviewInput,
) -> Option<TypedPatch> {
    use crate::health_check::check_campaign_health;

    match issue.category.as_str() {
        "unresolved_knowledge" => {
            // 找到 orphan knowledge id
            let affected_id = issue.affected_id.as_ref()?;
            let knowledge_id = Id::from_str(affected_id);
            Some(TypedPatch {
                id: format!("patch-{}-{}", issue.category, uuid::Uuid::new_v4()),
                description: issue.message.clone(),
                source_issue_category: issue.category.clone(),
                affected_id: issue.affected_id.clone(),
                actions: vec![TypedPatchAction::DeleteOrphanKnowledge { knowledge_id }],
                diff: vec![],
                created_at: chrono::Utc::now(),
                status: TypedPatchStatus::Pending,
            })
        }
        "orphan_task_references" => {
            let affected_id = issue.affected_id.as_ref()?;
            let task_id = Id::from_str(affected_id);
            // 找出 task 中不存在的 character ids
            let task = input.tasks.iter().find(|t| t.id == task_id)?;
            let instance_ids: std::collections::HashSet<&Id> =
                input.instances.iter().map(|i| &i.id).collect();
            let orphans: Vec<Id> = task
                .related_characters
                .iter()
                .filter(|id| !instance_ids.contains(id))
                .cloned()
                .collect();
            if orphans.is_empty() {
                return None;
            }
            Some(TypedPatch {
                id: format!("patch-{}-{}", issue.category, uuid::Uuid::new_v4()),
                description: issue.message.clone(),
                source_issue_category: issue.category.clone(),
                affected_id: issue.affected_id.clone(),
                actions: vec![TypedPatchAction::PruneOrphanTaskReferences {
                    task_id,
                    orphan_character_ids: orphans,
                }],
                diff: vec![],
                created_at: chrono::Utc::now(),
                status: TypedPatchStatus::Pending,
            })
        }
        "orphan_instance" => {
            let affected_id = issue.affected_id.as_ref()?;
            let instance_id = Id::from_str(affected_id);
            Some(TypedPatch {
                id: format!("patch-{}-{}", issue.category, uuid::Uuid::new_v4()),
                description: issue.message.clone(),
                source_issue_category: issue.category.clone(),
                affected_id: issue.affected_id.clone(),
                actions: vec![TypedPatchAction::RepointInstanceDefinition {
                    instance_id,
                    new_definition_id: None,
                }],
                diff: vec![],
                created_at: chrono::Utc::now(),
                status: TypedPatchStatus::Pending,
            })
        }
        "variable_schema_mismatch" => {
            let affected_id = issue.affected_id.as_ref()?;
            let instance_id = Id::from_str(affected_id);
            let instance = input.instances.iter().find(|i| i.id == instance_id)?;
            let definition = instance
                .definition_id
                .as_ref()
                .and_then(|did| input.definitions.iter().find(|d| &d.id == did))?;
            let schema_keys: std::collections::HashSet<&str> = definition
                .variable_schema
                .iter()
                .map(|f| f.key.as_str())
                .collect();
            let instance_keys: std::collections::HashSet<&str> =
                instance.variables.iter().map(|v| v.key.as_str()).collect();
            let add_keys: Vec<String> = schema_keys
                .difference(&instance_keys)
                .map(|s| s.to_string())
                .collect();
            let remove_keys: Vec<String> = instance_keys
                .difference(&schema_keys)
                .map(|s| s.to_string())
                .collect();
            if add_keys.is_empty() && remove_keys.is_empty() {
                return None;
            }
            Some(TypedPatch {
                id: format!("patch-{}-{}", issue.category, uuid::Uuid::new_v4()),
                description: issue.message.clone(),
                source_issue_category: issue.category.clone(),
                affected_id: issue.affected_id.clone(),
                actions: vec![TypedPatchAction::SyncInstanceVariables {
                    instance_id,
                    definition_id: definition.id.clone(),
                    add_keys,
                    remove_keys,
                }],
                diff: vec![],
                created_at: chrono::Utc::now(),
                status: TypedPatchStatus::Pending,
            })
        }
        _ => None,
    }
}

/// 检查 patch 是否过期：target id 是否仍存在于快照中
pub fn is_patch_stale(patch: &TypedPatch, input: &PreviewInput) -> bool {
    // 如果 affected_id 指向的 instance 不存在，视为 stale
    if let Some(ref affected) = patch.affected_id {
        let affected_id = Id::from_str(affected);
        // 检查 instance
        if input.instances.iter().any(|i| i.id == affected_id) {
            return false;
        }
        // 检查 task
        if input.tasks.iter().any(|t| t.id == affected_id) {
            return false;
        }
        // 检查 knowledge
        if input.knowledge.iter().any(|k| k.id == affected_id) {
            return false;
        }
        return true;
    }
    false
}

/// 把 patch 的 actions 应用到一份可变快照副本上（纯函数预演）
pub fn apply_to_snapshot(
    patch: &TypedPatch,
    snapshot: &mut PreviewInputMut,
) -> Result<(), TypedPatchError> {
    for action in &patch.actions {
        match action {
            TypedPatchAction::SyncInstanceVariables {
                instance_id,
                add_keys,
                remove_keys,
                ..
            } => {
                let instance = snapshot
                    .instances
                    .iter_mut()
                    .find(|i| i.id == *instance_id)
                    .ok_or_else(|| {
                        TypedPatchError::TargetMissing(format!(
                            "instance {}",
                            instance_id.as_str()
                        ))
                    })?;
                for key in add_keys {
                    if instance.get_variable(key).is_none() {
                        instance.set_variable(key, serde_json::Value::Null, 0);
                    }
                }
                instance.variables.retain(|v| !remove_keys.contains(&v.key));
            }
            TypedPatchAction::PruneOrphanTaskReferences {
                task_id,
                orphan_character_ids,
            } => {
                let task = snapshot
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == *task_id)
                    .ok_or_else(|| {
                        TypedPatchError::TargetMissing(format!("task {}", task_id.as_str()))
                    })?;
                task.related_characters
                    .retain(|id| !orphan_character_ids.contains(id));
            }
            TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
                let before = snapshot.knowledge.len();
                snapshot.knowledge.retain(|k| k.id != *knowledge_id);
                if snapshot.knowledge.len() == before {
                    return Err(TypedPatchError::TargetMissing(format!(
                        "knowledge {}",
                        knowledge_id.as_str()
                    )));
                }
            }
            TypedPatchAction::RepointInstanceDefinition {
                instance_id,
                new_definition_id,
            } => {
                let instance = snapshot
                    .instances
                    .iter_mut()
                    .find(|i| i.id == *instance_id)
                    .ok_or_else(|| {
                        TypedPatchError::TargetMissing(format!(
                            "instance {}",
                            instance_id.as_str()
                        ))
                    })?;
                instance.definition_id = new_definition_id.clone();
            }
        }
    }
    Ok(())
}

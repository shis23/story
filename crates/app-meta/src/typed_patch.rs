//! 类型化修复 Patch DTO + 纯函数 diff/preview/apply
//!
//! 对应 PLAN-META-AGENT.md 阶段 3「Typed Patch Preview」。
//! 四种 `TypedPatchAction` 变体分别覆盖 health check 发现的四类问题。
//! 所有函数均为纯函数，不读写 store 或全局状态。

use serde::{Deserialize, Serialize};
use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::CharacterDefinition;
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
use storyforge_domain::story_task::{StoryTask, TaskStatus};

use crate::HealthIssue;

// ─── DTO 类型 ────────────────────────────────────────────────────────────────

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
    DeleteOrphanKnowledge { knowledge_id: Id },
    /// 修孤立 instance：把 definition_id 改成现存 definition（或清空为临时角色）
    RepointInstanceDefinition {
        instance_id: Id,
        new_definition_id: Option<Id>,
    },
    /// 改 Campaign 级变量（如 story_clock）
    UpdateCampaignVariable {
        key: String,
        value: serde_json::Value,
    },
    /// 改某 instance 的变量（如 hp）
    UpdateInstanceVariable {
        instance_id: Id,
        key: String,
        value: serde_json::Value,
    },
    /// 给 instance 加一条知识
    AddKnowledge {
        character_id: Id,
        knowledge_text: String,
        source: KnowledgeSource,
    },
    /// 改任务状态
    UpdateTaskStatus { task_id: Id, new_status: TaskStatus },
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

// ─── Preview 输入（不可变 / 可变）────────────────────────────────────────────

/// preview 的输入快照（纯数据切片，不含 store 引用）
pub struct PreviewInput<'a> {
    pub instances: &'a [CharacterInstance],
    pub definitions: &'a [CharacterDefinition],
    pub knowledge: &'a [CharacterKnowledgeEntry],
    pub tasks: &'a [StoryTask],
    pub campaign: Option<&'a Campaign>,
}

/// apply_to_snapshot 的可变快照
pub struct PreviewInputMut<'a> {
    pub instances: &'a mut Vec<CharacterInstance>,
    pub definitions: &'a mut Vec<CharacterDefinition>,
    pub knowledge: &'a mut Vec<CharacterKnowledgeEntry>,
    pub tasks: &'a mut Vec<StoryTask>,
    pub campaign: Option<&'a mut Campaign>,
    pub turn: u32,
}

// ─── 错误类型 ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum TypedPatchError {
    #[error("patch target 不存在: {0}")]
    TargetMissing(String),
    #[error("patch 已过期")]
    Stale,
    /// 前置条件不满足（definition 不匹配、schema key 缺失等 target 存在之外的语义校验）。
    #[error("patch 前置条件失败: {0}")]
    PreconditionFailed(String),
}

// ─── 纯函数实现 ──────────────────────────────────────────────────────────────

/// 从一个 health issue 构造对应的修复 patch（preview 在构造时一并算好 diff）。
/// 无法修复的 issue（如未实现的 category）返回 None。
pub fn build_patch_for_issue(issue: &HealthIssue, input: &PreviewInput) -> Option<TypedPatch> {
    match issue.category.as_str() {
        "orphan_instance" => build_orphan_instance_patch(issue, input),
        "unresolved_knowledge" => build_unresolved_knowledge_patch(issue, input),
        "orphan_task_reference" => build_orphan_task_reference_patch(issue, input),
        "variable_schema_mismatch" => build_variable_schema_mismatch_patch(issue, input),
        _ => None,
    }
}

/// 检查 patch 是否过期：target id 是否仍存在于快照中。
/// 返回 true = 过期（不应接受）。
///
/// 本函数只判 target 存在性；更严格的前置条件（definition 匹配、schema key 存在）
/// 由 [`validate_patch_preconditions`] 承载。两者共用同一存在性判定路径。
pub fn is_patch_stale(patch: &TypedPatch, input: &PreviewInput) -> bool {
    patch
        .actions
        .iter()
        .any(|action| action_target_missing(action, input))
}

/// 判定单条 action 的 target 是否缺失（存在性判定，`is_patch_stale` 与
/// `validate_patch_preconditions` 共用，避免重复扫描）。
fn action_target_missing(action: &TypedPatchAction, input: &PreviewInput) -> bool {
    match action {
        TypedPatchAction::RepointInstanceDefinition { instance_id, .. } => {
            !input.instances.iter().any(|i| &i.id == instance_id)
        }
        TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
            !input.knowledge.iter().any(|k| &k.id == knowledge_id)
        }
        TypedPatchAction::PruneOrphanTaskReferences { task_id, .. } => {
            !input.tasks.iter().any(|t| &t.id == task_id)
        }
        TypedPatchAction::SyncInstanceVariables { instance_id, .. } => {
            !input.instances.iter().any(|i| &i.id == instance_id)
        }
        TypedPatchAction::UpdateCampaignVariable { .. } => input.campaign.is_none(),
        TypedPatchAction::UpdateInstanceVariable { instance_id, .. } => {
            !input.instances.iter().any(|i| &i.id == instance_id)
        }
        TypedPatchAction::AddKnowledge { character_id, .. } => {
            !input.instances.iter().any(|i| &i.id == character_id)
        }
        TypedPatchAction::UpdateTaskStatus { task_id, .. } => {
            !input.tasks.iter().any(|t| &t.id == task_id)
        }
    }
}

/// 校验 patch 在应用前的全部前置条件（target 存在 + definition/schema 一致）。
///
/// 这是 Preview 与 Accept **共用**的单一权威校验纯函数（Gate 2 Batch 2.4）：
/// - target 存在性复用 [`action_target_missing`]（与 [`is_patch_stale`] 同源）；
/// - `SyncInstanceVariables` 额外校验 instance.definition_id 与 action 的 definition_id
///   一致、definition 存在、add_keys 均在 schema 内（既有
///   `meta_typed::validate_typed_patch_targets` 的检查归并于此）。
///
/// 缺失 target 返回 `TypedPatchError::TargetMissing`；definition/schema 不匹配返回
/// `TypedPatchError::PreconditionFailed`。调用方（Preview/Accept）各自决定如何映射到
/// 用户可见错误或 stale 标记，但**判定逻辑只此一份**。
pub fn validate_patch_preconditions(
    patch: &TypedPatch,
    input: &PreviewInput,
) -> Result<(), TypedPatchError> {
    for action in &patch.actions {
        // 1. target 存在性（与 is_patch_stale 共用）
        if action_target_missing(action, input) {
            return Err(target_missing_error(action));
        }
        // 2. SyncInstanceVariables 的严格前置条件
        if let TypedPatchAction::SyncInstanceVariables {
            instance_id,
            definition_id,
            add_keys,
            ..
        } = action
        {
            let inst = input
                .instances
                .iter()
                .find(|i| &i.id == instance_id)
                .expect("target existence checked above");
            if inst.definition_id.as_ref() != Some(definition_id) {
                return Err(TypedPatchError::PreconditionFailed(format!(
                    "Instance {} 已不再使用 definition {}",
                    instance_id.as_str(),
                    definition_id.as_str()
                )));
            }
            let definition = input.definitions.iter().find(|d| &d.id == definition_id);
            let Some(definition) = definition else {
                return Err(TypedPatchError::TargetMissing(format!(
                    "Definition 不存在: {}",
                    definition_id.as_str()
                )));
            };
            for key in add_keys {
                if !definition
                    .variable_schema
                    .iter()
                    .any(|field| field.key == *key)
                {
                    return Err(TypedPatchError::PreconditionFailed(format!(
                        "Definition {} 缺少变量 schema: {}",
                        definition_id.as_str(),
                        key
                    )));
                }
            }
        }
        // 3. RepointInstanceDefinition：若指定新 definition_id，必须存在于快照中。
        if let TypedPatchAction::RepointInstanceDefinition {
            new_definition_id: Some(definition_id),
            ..
        } = action
            && !input.definitions.iter().any(|d| &d.id == definition_id)
        {
            return Err(TypedPatchError::TargetMissing(format!(
                "Definition 不存在: {}",
                definition_id.as_str()
            )));
        }
    }
    Ok(())
}

/// 根据缺失的 target 类型生成 `TargetMissing` 错误消息（与 apply_action 的错误同源）。
fn target_missing_error(action: &TypedPatchAction) -> TypedPatchError {
    match action {
        TypedPatchAction::RepointInstanceDefinition { instance_id, .. }
        | TypedPatchAction::SyncInstanceVariables { instance_id, .. }
        | TypedPatchAction::UpdateInstanceVariable { instance_id, .. } => {
            TypedPatchError::TargetMissing(format!("instance {}", instance_id))
        }
        TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
            TypedPatchError::TargetMissing(format!("knowledge {}", knowledge_id))
        }
        TypedPatchAction::PruneOrphanTaskReferences { task_id, .. }
        | TypedPatchAction::UpdateTaskStatus { task_id, .. } => {
            TypedPatchError::TargetMissing(format!("task {}", task_id))
        }
        TypedPatchAction::AddKnowledge { character_id, .. } => {
            TypedPatchError::TargetMissing(format!("instance {}", character_id))
        }
        TypedPatchAction::UpdateCampaignVariable { .. } => {
            TypedPatchError::TargetMissing("campaign (none)".into())
        }
    }
}

/// 把 patch 的 actions 应用到一份可变快照副本上（纯函数，验证语义正确用）。
pub fn apply_to_snapshot(
    patch: &TypedPatch,
    snapshot: &mut PreviewInputMut,
) -> Result<(), TypedPatchError> {
    for action in &patch.actions {
        apply_action(action, snapshot)?;
    }
    Ok(())
}

// ─── 内部 helpers ────────────────────────────────────────────────────────────

fn build_orphan_instance_patch(issue: &HealthIssue, input: &PreviewInput) -> Option<TypedPatch> {
    let affected_id = issue.affected_id.as_deref()?;
    let inst = input
        .instances
        .iter()
        .find(|i| i.id.as_str() == affected_id)?;

    let name = inst.name.clone();
    let old_def_id = inst.definition_id.clone();

    let actions = vec![TypedPatchAction::RepointInstanceDefinition {
        instance_id: inst.id.clone(),
        new_definition_id: None,
    }];

    let diff = vec![FieldDiff {
        path: "definition_id".into(),
        before: old_def_id
            .as_ref()
            .map(|id| serde_json::Value::String(id.to_string()))
            .unwrap_or(serde_json::Value::Null),
        after: serde_json::Value::Null,
    }];

    Some(TypedPatch {
        id: uuid::Uuid::new_v4().to_string(),
        description: format!("将孤立实例 {name} 的 definition_id 清空（降为临时角色）"),
        source_issue_category: "orphan_instance".into(),
        affected_id: Some(affected_id.into()),
        actions,
        diff,
        created_at: chrono::Utc::now(),
        status: TypedPatchStatus::Pending,
    })
}

fn build_unresolved_knowledge_patch(
    issue: &HealthIssue,
    input: &PreviewInput,
) -> Option<TypedPatch> {
    let affected_id = issue.affected_id.as_deref()?;
    let entry = input
        .knowledge
        .iter()
        .find(|k| k.id.as_str() == affected_id)?;

    let truncated = truncate(&entry.knowledge_text, 40);

    let actions = vec![TypedPatchAction::DeleteOrphanKnowledge {
        knowledge_id: entry.id.clone(),
    }];

    let before_val =
        serde_json::to_value(entry).expect("knowledge entry should serialize for typed patch diff");

    let diff = vec![FieldDiff {
        path: format!("knowledge[{}]", entry.id),
        before: before_val,
        after: serde_json::Value::Null,
    }];

    Some(TypedPatch {
        id: uuid::Uuid::new_v4().to_string(),
        description: format!("删除未解析的知识条目「{truncated}」"),
        source_issue_category: "unresolved_knowledge".into(),
        affected_id: Some(affected_id.into()),
        actions,
        diff,
        created_at: chrono::Utc::now(),
        status: TypedPatchStatus::Pending,
    })
}

fn build_orphan_task_reference_patch(
    issue: &HealthIssue,
    input: &PreviewInput,
) -> Option<TypedPatch> {
    let affected_id = issue.affected_id.as_deref()?;
    let task = input.tasks.iter().find(|t| t.id.as_str() == affected_id)?;

    let instance_ids: std::collections::HashSet<&Id> =
        input.instances.iter().map(|i| &i.id).collect();

    let orphan_ids: Vec<Id> = task
        .related_characters
        .iter()
        .filter(|cid| !instance_ids.contains(cid))
        .cloned()
        .collect();

    if orphan_ids.is_empty() {
        return None;
    }

    let filtered: Vec<Id> = task
        .related_characters
        .iter()
        .filter(|cid| instance_ids.contains(cid))
        .cloned()
        .collect();

    let actions = vec![TypedPatchAction::PruneOrphanTaskReferences {
        task_id: task.id.clone(),
        orphan_character_ids: orphan_ids,
    }];

    let before_vec: Vec<serde_json::Value> = task
        .related_characters
        .iter()
        .map(|id| serde_json::Value::String(id.to_string()))
        .collect();
    let after_vec: Vec<serde_json::Value> = filtered
        .iter()
        .map(|id| serde_json::Value::String(id.to_string()))
        .collect();

    let diff = vec![FieldDiff {
        path: "related_characters".into(),
        before: serde_json::Value::Array(before_vec),
        after: serde_json::Value::Array(after_vec),
    }];

    Some(TypedPatch {
        id: uuid::Uuid::new_v4().to_string(),
        description: format!("从任务「{}」移除孤儿角色引用", task.title),
        source_issue_category: "orphan_task_reference".into(),
        affected_id: Some(affected_id.into()),
        actions,
        diff,
        created_at: chrono::Utc::now(),
        status: TypedPatchStatus::Pending,
    })
}

fn build_variable_schema_mismatch_patch(
    issue: &HealthIssue,
    input: &PreviewInput,
) -> Option<TypedPatch> {
    let affected_id = issue.affected_id.as_deref()?;
    let inst = input
        .instances
        .iter()
        .find(|i| i.id.as_str() == affected_id)?;

    let def_id = inst.definition_id.as_ref()?;
    let def = input.definitions.iter().find(|d| &d.id == def_id)?;

    let schema_keys: std::collections::HashSet<&str> =
        def.variable_schema.iter().map(|f| f.key.as_str()).collect();
    let instance_keys: std::collections::HashSet<&str> =
        inst.variables.iter().map(|v| v.key.as_str()).collect();

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

    // Build diff entries
    let mut diff = Vec::new();

    // For add_keys: get default from schema
    for key in &add_keys {
        let default_val = def
            .variable_schema
            .iter()
            .find(|f| f.key == *key)
            .map(|f| f.default.clone())
            .unwrap_or(serde_json::Value::Null);
        diff.push(FieldDiff {
            path: format!("variables[{key}]"),
            before: serde_json::Value::Null,
            after: default_val,
        });
    }

    // For remove_keys: get current value from instance
    for key in &remove_keys {
        let old_val = inst
            .variables
            .iter()
            .find(|v| v.key == *key)
            .map(|v| v.value.clone())
            .unwrap_or(serde_json::Value::Null);
        diff.push(FieldDiff {
            path: format!("variables[{key}]"),
            before: old_val,
            after: serde_json::Value::Null,
        });
    }

    let actions = vec![TypedPatchAction::SyncInstanceVariables {
        instance_id: inst.id.clone(),
        definition_id: def_id.clone(),
        add_keys: add_keys.clone(),
        remove_keys: remove_keys.clone(),
    }];

    Some(TypedPatch {
        id: uuid::Uuid::new_v4().to_string(),
        description: format!(
            "同步实例「{}」的变量 schema（补 {} 个、删 {} 个字段）",
            inst.name,
            add_keys.len(),
            remove_keys.len()
        ),
        source_issue_category: "variable_schema_mismatch".into(),
        affected_id: Some(affected_id.into()),
        actions,
        diff,
        created_at: chrono::Utc::now(),
        status: TypedPatchStatus::Pending,
    })
}

fn apply_action(
    action: &TypedPatchAction,
    snapshot: &mut PreviewInputMut,
) -> Result<(), TypedPatchError> {
    match action {
        TypedPatchAction::SyncInstanceVariables {
            instance_id,
            definition_id,
            add_keys,
            remove_keys,
        } => {
            let inst = snapshot
                .instances
                .iter_mut()
                .find(|i| &i.id == instance_id)
                .ok_or_else(|| {
                    TypedPatchError::TargetMissing(format!("instance {}", instance_id))
                })?;

            // Look up the definition to get defaults for add_keys
            let def_defaults: Vec<(String, serde_json::Value)> = snapshot
                .definitions
                .iter()
                .find(|d| &d.id == definition_id)
                .map(|def| {
                    add_keys
                        .iter()
                        .filter_map(|key| {
                            def.variable_schema
                                .iter()
                                .find(|f| f.key == *key)
                                .map(|f| (key.clone(), f.default.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default();

            // Add missing keys using schema defaults
            for (key, default_val) in def_defaults {
                if !inst.variables.iter().any(|v| v.key == key) {
                    inst.variables
                        .push(storyforge_domain::variables::VariableValue {
                            key,
                            value: default_val,
                            last_updated_turn: 0,
                        });
                }
            }

            // Remove extra keys
            inst.variables.retain(|v| !remove_keys.contains(&v.key));

            Ok(())
        }
        TypedPatchAction::PruneOrphanTaskReferences {
            task_id,
            orphan_character_ids,
        } => {
            let task = snapshot
                .tasks
                .iter_mut()
                .find(|t| &t.id == task_id)
                .ok_or_else(|| TypedPatchError::TargetMissing(format!("task {}", task_id)))?;

            task.related_characters
                .retain(|cid| !orphan_character_ids.contains(cid));
            Ok(())
        }
        TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
            let original_len = snapshot.knowledge.len();
            snapshot.knowledge.retain(|k| &k.id != knowledge_id);
            if snapshot.knowledge.len() == original_len {
                return Err(TypedPatchError::TargetMissing(format!(
                    "knowledge {}",
                    knowledge_id
                )));
            }
            Ok(())
        }
        TypedPatchAction::RepointInstanceDefinition {
            instance_id,
            new_definition_id,
        } => {
            let inst = snapshot
                .instances
                .iter_mut()
                .find(|i| &i.id == instance_id)
                .ok_or_else(|| {
                    TypedPatchError::TargetMissing(format!("instance {}", instance_id))
                })?;

            inst.definition_id = new_definition_id.clone();
            if new_definition_id.is_none() {
                inst.is_temporary = true;
            }
            Ok(())
        }
        TypedPatchAction::UpdateCampaignVariable { key, value } => {
            let campaign = snapshot
                .campaign
                .as_deref_mut()
                .ok_or_else(|| TypedPatchError::TargetMissing("campaign (none)".into()))?;
            campaign.set_variable(key, value.clone(), snapshot.turn);
            Ok(())
        }
        TypedPatchAction::UpdateInstanceVariable {
            instance_id,
            key,
            value,
        } => {
            let inst = snapshot
                .instances
                .iter_mut()
                .find(|i| &i.id == instance_id)
                .ok_or_else(|| {
                    TypedPatchError::TargetMissing(format!("instance {}", instance_id))
                })?;
            inst.set_variable(key, value.clone(), snapshot.turn);
            Ok(())
        }
        TypedPatchAction::AddKnowledge {
            character_id,
            knowledge_text,
            source,
        } => {
            let campaign_id = snapshot
                .campaign
                .as_ref()
                .map(|c| c.id.clone())
                .unwrap_or_else(|| Id::from_str("unknown"));
            let entry = CharacterKnowledgeEntry {
                id: Id::new(),
                campaign_id,
                character_id: character_id.clone(),
                knowledge_text: knowledge_text.clone(),
                source: source.clone(),
                source_character_id: None,
                source_knowledge_id: None,
                turn_number: snapshot.turn,
                event_id: None,
                pinned: false,
                propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
            };
            snapshot.knowledge.push(entry);
            Ok(())
        }
        TypedPatchAction::UpdateTaskStatus {
            task_id,
            new_status,
        } => {
            let task = snapshot
                .tasks
                .iter_mut()
                .find(|t| &t.id == task_id)
                .ok_or_else(|| TypedPatchError::TargetMissing(format!("task {}", task_id)))?;
            task.status = new_status.clone();
            Ok(())
        }
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", chars[..max_chars].iter().collect::<String>())
    }
}

/// 从 Agent 提议的 action 构造 TypedPatch（含 target 校验 + diff 构造）。
///
/// 与 `build_patch_for_issue` 不同：本函数由 `propose_campaign_patch` 工具调用，
/// `source_issue_category` 固定为 `"agent_proposed"`。
pub fn build_patch_from_action(
    description: String,
    action: TypedPatchAction,
    input: &PreviewInput,
) -> Result<TypedPatch, TypedPatchError> {
    // 校验 target 存在
    match &action {
        TypedPatchAction::UpdateCampaignVariable { .. } => {
            if input.campaign.is_none() {
                return Err(TypedPatchError::TargetMissing("campaign (none)".into()));
            }
        }
        TypedPatchAction::UpdateInstanceVariable { instance_id, .. } => {
            if !input.instances.iter().any(|i| &i.id == instance_id) {
                return Err(TypedPatchError::TargetMissing(format!(
                    "instance {}",
                    instance_id
                )));
            }
        }
        TypedPatchAction::AddKnowledge { character_id, .. } => {
            if !input.instances.iter().any(|i| &i.id == character_id) {
                return Err(TypedPatchError::TargetMissing(format!(
                    "instance {}",
                    character_id
                )));
            }
        }
        TypedPatchAction::UpdateTaskStatus { task_id, .. } => {
            if !input.tasks.iter().any(|t| &t.id == task_id) {
                return Err(TypedPatchError::TargetMissing(format!("task {}", task_id)));
            }
        }
        // health-issue-driven 变体不通过 build_patch_from_action 构造
        _ => {
            return Err(TypedPatchError::TargetMissing(
                "不支持的 action 类型（请用 build_patch_for_issue）".into(),
            ));
        }
    }

    // 构造 diff
    let diff = build_diff_for_action(&action, input);

    // 确定 affected_id
    let affected_id = match &action {
        TypedPatchAction::UpdateCampaignVariable { key, .. } => Some(key.clone()),
        TypedPatchAction::UpdateInstanceVariable { instance_id, .. } => {
            Some(instance_id.to_string())
        }
        TypedPatchAction::AddKnowledge { character_id, .. } => Some(character_id.to_string()),
        TypedPatchAction::UpdateTaskStatus { task_id, .. } => Some(task_id.to_string()),
        _ => None,
    };

    Ok(TypedPatch {
        id: uuid::Uuid::new_v4().to_string(),
        description,
        source_issue_category: "agent_proposed".into(),
        affected_id,
        actions: vec![action],
        diff,
        created_at: chrono::Utc::now(),
        status: TypedPatchStatus::Pending,
    })
}

/// 为单个 action 构造 diff entries（纯函数，只读 input）。
fn build_diff_for_action(action: &TypedPatchAction, input: &PreviewInput) -> Vec<FieldDiff> {
    match action {
        TypedPatchAction::UpdateCampaignVariable { key, value } => {
            let before = input
                .campaign
                .and_then(|c| c.get_variable(key))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            vec![FieldDiff {
                path: format!("campaign.variables[{key}]"),
                before,
                after: value.clone(),
            }]
        }
        TypedPatchAction::UpdateInstanceVariable {
            instance_id,
            key,
            value,
        } => {
            let before = input
                .instances
                .iter()
                .find(|i| &i.id == instance_id)
                .and_then(|i| i.get_variable(key))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            vec![FieldDiff {
                path: format!("instance[{instance_id}].variables[{key}]"),
                before,
                after: value.clone(),
            }]
        }
        TypedPatchAction::AddKnowledge {
            character_id,
            knowledge_text,
            ..
        } => {
            vec![FieldDiff {
                path: format!("knowledge (new for {character_id})"),
                before: serde_json::Value::Null,
                after: serde_json::json!({"knowledge_text": knowledge_text}),
            }]
        }
        TypedPatchAction::UpdateTaskStatus {
            task_id,
            new_status,
        } => {
            let before = input
                .tasks
                .iter()
                .find(|t| &t.id == task_id)
                .map(|t| {
                    serde_json::to_value(&t.status)
                        .expect("task status should serialize for typed patch diff")
                })
                .unwrap_or(serde_json::Value::Null);
            vec![FieldDiff {
                path: format!("task[{task_id}].status"),
                before,
                after: serde_json::to_value(new_status)
                    .expect("task status should serialize for typed patch diff"),
            }]
        }
        _ => vec![],
    }
}

// ─── 测试 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::campaign::CharacterInstance;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::story_task::{StoryTask, TaskTrigger};
    use storyforge_domain::variables::{self, VariableField, VariableType, VariableValue};

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
            variable_schema: variables::default_character_variables(),
        }
    }

    fn make_def_with_custom_schema(
        id: &str,
        extra_fields: Vec<VariableField>,
    ) -> CharacterDefinition {
        let mut def = make_def(id);
        def.variable_schema.extend(extra_fields);
        def
    }

    fn make_instance(inst_id: &str, def_id: Option<&str>) -> CharacterInstance {
        if let Some(d) = def_id {
            let def = make_def(d);
            let mut inst = CharacterInstance::from_definition(Id::from_str("camp-1"), &def);
            inst.id = Id::from_str(inst_id);
            inst
        } else {
            let mut inst = CharacterInstance::temporary(Id::from_str("camp-1"), "临时角色");
            inst.id = Id::from_str(inst_id);
            inst
        }
    }

    // ── build_patch_for_issue: orphan_instance ────────────────────────────

    #[test]
    fn test_build_orphan_instance_patch() {
        let inst = make_instance("inst-1", Some("def-ghost"));
        let def_real = make_def("def-real");

        let issue = HealthIssue {
            severity: crate::IssueSeverity::Error,
            category: "orphan_instance".into(),
            message: "孤立实例".into(),
            affected_id: Some("inst-1".into()),
        };

        let input = PreviewInput {
            instances: std::slice::from_ref(&inst),
            definitions: &[def_real],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };

        let patch = build_patch_for_issue(&issue, &input).expect("should build patch");
        assert_eq!(patch.source_issue_category, "orphan_instance");
        assert_eq!(patch.actions.len(), 1);
        assert_eq!(patch.diff.len(), 1);
        assert_eq!(patch.diff[0].path, "definition_id");
        assert_eq!(
            patch.diff[0].before,
            serde_json::Value::String("def-ghost".into())
        );
        assert_eq!(patch.diff[0].after, serde_json::Value::Null);
        assert_eq!(patch.status, TypedPatchStatus::Pending);

        match &patch.actions[0] {
            TypedPatchAction::RepointInstanceDefinition {
                instance_id,
                new_definition_id,
            } => {
                assert_eq!(instance_id.as_str(), "inst-1");
                assert!(new_definition_id.is_none());
            }
            _ => panic!("expected RepointInstanceDefinition"),
        }
    }

    // ── build_patch_for_issue: unresolved_knowledge ───────────────────────

    #[test]
    fn test_build_unresolved_knowledge_patch() {
        let entry = CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            Id::from_str("ghost-char"),
            "看到了一些奇怪的事情发生在古老的城堡里",
            1,
        );

        let issue = HealthIssue {
            severity: crate::IssueSeverity::Warning,
            category: "unresolved_knowledge".into(),
            message: "未解析知识".into(),
            affected_id: Some(entry.id.to_string()),
        };

        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: std::slice::from_ref(&entry),
            tasks: &[],
            campaign: None,
        };

        let patch = build_patch_for_issue(&issue, &input).expect("should build patch");
        assert_eq!(patch.source_issue_category, "unresolved_knowledge");
        assert_eq!(patch.actions.len(), 1);
        assert_eq!(patch.diff.len(), 1);
        assert_eq!(patch.diff[0].after, serde_json::Value::Null);
        // before should be the serialized entry
        assert!(patch.diff[0].before.is_object());

        match &patch.actions[0] {
            TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
                assert_eq!(knowledge_id, &entry.id);
            }
            _ => panic!("expected DeleteOrphanKnowledge"),
        }
    }

    // ── build_patch_for_issue: orphan_task_reference ──────────────────────

    #[test]
    fn test_build_orphan_task_reference_patch() {
        let inst = make_instance("inst-1", Some("def-1"));

        let mut task = StoryTask::user_planned(
            Id::from_str("camp-1"),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        );
        task.related_characters = vec![Id::from_str("inst-1"), Id::from_str("inst-ghost")];

        let issue = HealthIssue {
            severity: crate::IssueSeverity::Warning,
            category: "orphan_task_reference".into(),
            message: "孤儿任务引用".into(),
            affected_id: Some(task.id.to_string()),
        };

        let input = PreviewInput {
            instances: &[inst],
            definitions: &[],
            knowledge: &[],
            tasks: &[task.clone()],
            campaign: None,
        };

        let patch = build_patch_for_issue(&issue, &input).expect("should build patch");
        assert_eq!(patch.source_issue_category, "orphan_task_reference");
        assert_eq!(patch.actions.len(), 1);
        assert_eq!(patch.diff.len(), 1);
        assert_eq!(patch.diff[0].path, "related_characters");

        // before should have 2 entries, after should have 1 (only inst-1 remains)
        if let serde_json::Value::Array(before) = &patch.diff[0].before {
            assert_eq!(before.len(), 2);
        } else {
            panic!("expected array");
        }
        if let serde_json::Value::Array(after) = &patch.diff[0].after {
            assert_eq!(after.len(), 1);
            assert_eq!(after[0], serde_json::Value::String("inst-1".into()));
        } else {
            panic!("expected array");
        }

        match &patch.actions[0] {
            TypedPatchAction::PruneOrphanTaskReferences {
                task_id,
                orphan_character_ids,
            } => {
                assert_eq!(task_id, &task.id);
                assert_eq!(orphan_character_ids.len(), 1);
                assert_eq!(orphan_character_ids[0].as_str(), "inst-ghost");
            }
            _ => panic!("expected PruneOrphanTaskReferences"),
        }
    }

    // ── build_patch_for_issue: variable_schema_mismatch ───────────────────

    #[test]
    fn test_build_variable_schema_mismatch_patch() {
        let extra = VariableField {
            key: "custom_var".into(),
            label: "自定义变量".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(0),
            description: None,
            group: Some("状态".into()),
        };
        let def = make_def_with_custom_schema("def-1", vec![extra]);
        let mut inst = CharacterInstance::from_definition(Id::from_str("camp-1"), &def);
        inst.id = Id::from_str("inst-1");
        // Remove custom_var from instance to create mismatch
        inst.variables.retain(|v| v.key != "custom_var");

        let issue = HealthIssue {
            severity: crate::IssueSeverity::Warning,
            category: "variable_schema_mismatch".into(),
            message: "变量不一致".into(),
            affected_id: Some("inst-1".into()),
        };

        let input = PreviewInput {
            instances: std::slice::from_ref(&inst),
            definitions: &[def],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };

        let patch = build_patch_for_issue(&issue, &input).expect("should build patch");
        assert_eq!(patch.source_issue_category, "variable_schema_mismatch");
        assert_eq!(patch.actions.len(), 1);
        // diff should have 1 entry for the missing custom_var
        assert_eq!(patch.diff.len(), 1);
        assert_eq!(patch.diff[0].path, "variables[custom_var]");
        assert_eq!(patch.diff[0].before, serde_json::Value::Null);
        assert_eq!(patch.diff[0].after, serde_json::json!(0));

        match &patch.actions[0] {
            TypedPatchAction::SyncInstanceVariables {
                instance_id,
                add_keys,
                remove_keys,
                ..
            } => {
                assert_eq!(instance_id.as_str(), "inst-1");
                assert!(add_keys.contains(&"custom_var".into()));
                assert!(remove_keys.is_empty());
            }
            _ => panic!("expected SyncInstanceVariables"),
        }
    }

    // ── build_patch_for_issue: unknown category → None ────────────────────

    #[test]
    fn test_build_patch_unknown_category_returns_none() {
        let issue = HealthIssue {
            severity: crate::IssueSeverity::Warning,
            category: "some_future_category".into(),
            message: "未知类型".into(),
            affected_id: None,
        };
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        assert!(build_patch_for_issue(&issue, &input).is_none());
    }

    // ── is_patch_stale ────────────────────────────────────────────────────

    #[test]
    fn test_is_patch_stale_target_exists() {
        let inst = make_instance("inst-1", Some("def-1"));

        let issue = HealthIssue {
            severity: crate::IssueSeverity::Error,
            category: "orphan_instance".into(),
            message: "孤立".into(),
            affected_id: Some("inst-1".into()),
        };
        let input = PreviewInput {
            instances: &[inst],
            definitions: &[make_def("def-1")],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let patch = build_patch_for_issue(&issue, &input).unwrap();
        assert!(!is_patch_stale(&patch, &input));
    }

    #[test]
    fn test_is_patch_stale_target_removed() {
        let inst = make_instance("inst-1", Some("def-ghost"));

        let issue = HealthIssue {
            severity: crate::IssueSeverity::Error,
            category: "orphan_instance".into(),
            message: "孤立".into(),
            affected_id: Some("inst-1".into()),
        };
        let input_with = PreviewInput {
            instances: std::slice::from_ref(&inst),
            definitions: &[make_def("def-1")],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let patch = build_patch_for_issue(&issue, &input_with).unwrap();

        // Now the instance is removed from the snapshot
        let input_without = PreviewInput {
            instances: &[],
            definitions: &[make_def("def-1")],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        assert!(is_patch_stale(&patch, &input_without));
    }

    // ── apply_to_snapshot: SyncInstanceVariables ──────────────────────────

    #[test]
    fn test_apply_sync_instance_variables() {
        let extra = VariableField {
            key: "custom_var".into(),
            label: "自定义变量".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(42),
            description: None,
            group: Some("状态".into()),
        };
        let def = make_def_with_custom_schema("def-1", vec![extra]);
        let mut inst = CharacterInstance::from_definition(Id::from_str("camp-1"), &def);
        inst.id = Id::from_str("inst-1");
        // Remove custom_var, and add an extra key "obsolete"
        inst.variables.retain(|v| v.key != "custom_var");
        inst.variables
            .push(VariableValue::new("obsolete", serde_json::json!("old"), 0));

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "variable_schema_mismatch".into(),
            affected_id: Some("inst-1".into()),
            actions: vec![TypedPatchAction::SyncInstanceVariables {
                instance_id: Id::from_str("inst-1"),
                definition_id: Id::from_str("def-1"),
                add_keys: vec!["custom_var".into()],
                remove_keys: vec!["obsolete".into()],
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        let mut instances = vec![inst];
        let mut defs = vec![def];
        let mut knowledge = vec![];
        let mut tasks = vec![];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: None,
            turn: 0,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");

        let updated = &snapshot.instances[0];
        // custom_var should be added with default value 42
        let cv = updated.variables.iter().find(|v| v.key == "custom_var");
        assert!(cv.is_some(), "custom_var should be added");
        assert_eq!(cv.unwrap().value, serde_json::json!(42));
        // obsolete should be removed
        assert!(
            !updated.variables.iter().any(|v| v.key == "obsolete"),
            "obsolete should be removed"
        );
        // hp should still exist (other variables untouched)
        assert!(
            updated.variables.iter().any(|v| v.key == "hp"),
            "hp should remain"
        );
    }

    // ── apply_to_snapshot: PruneOrphanTaskReferences ──────────────────────

    #[test]
    fn test_apply_prune_orphan_task_references() {
        let mut task = StoryTask::user_planned(
            Id::from_str("camp-1"),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        );
        task.related_characters = vec![
            Id::from_str("inst-valid"),
            Id::from_str("inst-orphan-1"),
            Id::from_str("inst-orphan-2"),
        ];

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "orphan_task_reference".into(),
            affected_id: Some(task.id.to_string()),
            actions: vec![TypedPatchAction::PruneOrphanTaskReferences {
                task_id: task.id.clone(),
                orphan_character_ids: vec![
                    Id::from_str("inst-orphan-1"),
                    Id::from_str("inst-orphan-2"),
                ],
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        let mut instances = vec![];
        let mut defs = vec![];
        let mut knowledge = vec![];
        let mut tasks = vec![task];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: None,
            turn: 0,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");

        let updated = &snapshot.tasks[0];
        assert_eq!(updated.related_characters.len(), 1);
        assert_eq!(updated.related_characters[0].as_str(), "inst-valid");
    }

    // ── apply_to_snapshot: DeleteOrphanKnowledge ──────────────────────────

    #[test]
    fn test_apply_delete_orphan_knowledge() {
        let entry_keep = CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            Id::from_str("char-1"),
            "保留的知识",
            1,
        );
        let entry_delete = CharacterKnowledgeEntry::witnessed(
            Id::from_str("camp-1"),
            Id::from_str("ghost"),
            "要删的知识",
            1,
        );

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "unresolved_knowledge".into(),
            affected_id: Some(entry_delete.id.to_string()),
            actions: vec![TypedPatchAction::DeleteOrphanKnowledge {
                knowledge_id: entry_delete.id.clone(),
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        let mut instances = vec![];
        let mut defs = vec![];
        let mut knowledge = vec![entry_keep.clone(), entry_delete];
        let mut tasks = vec![];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: None,
            turn: 0,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");

        assert_eq!(snapshot.knowledge.len(), 1);
        assert_eq!(snapshot.knowledge[0].id, entry_keep.id);
    }

    // ── apply_to_snapshot: RepointInstanceDefinition ──────────────────────

    #[test]
    fn test_apply_repoint_instance_definition() {
        let inst = make_instance("inst-1", Some("def-ghost"));
        assert_eq!(inst.definition_id.as_ref().unwrap().as_str(), "def-ghost");

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "orphan_instance".into(),
            affected_id: Some("inst-1".into()),
            actions: vec![TypedPatchAction::RepointInstanceDefinition {
                instance_id: Id::from_str("inst-1"),
                new_definition_id: None,
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        let mut instances = vec![inst];
        let mut defs = vec![];
        let mut knowledge = vec![];
        let mut tasks = vec![];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: None,
            turn: 0,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");

        let updated = &snapshot.instances[0];
        assert!(updated.definition_id.is_none());
        assert!(updated.is_temporary);
    }

    // ── apply_to_snapshot: target missing → TargetMissing ─────────────────

    #[test]
    fn test_apply_target_missing_returns_error() {
        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "unresolved_knowledge".into(),
            affected_id: Some("nonexistent".into()),
            actions: vec![TypedPatchAction::DeleteOrphanKnowledge {
                knowledge_id: Id::from_str("nonexistent"),
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        let mut instances = vec![];
        let mut defs = vec![];
        let mut knowledge = vec![];
        let mut tasks = vec![];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: None,
            turn: 0,
        };

        let result = apply_to_snapshot(&patch, &mut snapshot);
        assert!(result.is_err());
        match result.unwrap_err() {
            TypedPatchError::TargetMissing(msg) => {
                assert!(msg.contains("nonexistent"));
            }
            _ => panic!("expected TargetMissing"),
        }
    }

    // ── build_patch_from_action: UpdateCampaignVariable ─────────────────

    #[test]
    fn test_build_patch_from_action_update_campaign_variable() {
        let campaign = Campaign::new(Id::from_str("card-1"), "测试 Campaign");
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: Some(&campaign),
        };

        let action = TypedPatchAction::UpdateCampaignVariable {
            key: "story_clock".into(),
            value: serde_json::json!("Night 5"),
        };
        let patch = build_patch_from_action("改故事时间".into(), action, &input).unwrap();
        assert_eq!(patch.source_issue_category, "agent_proposed");
        assert_eq!(patch.status, TypedPatchStatus::Pending);
        assert_eq!(patch.diff.len(), 1);
        assert_eq!(patch.diff[0].path, "campaign.variables[story_clock]");
    }

    #[test]
    fn test_build_patch_from_action_update_campaign_variable_no_campaign() {
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let action = TypedPatchAction::UpdateCampaignVariable {
            key: "story_clock".into(),
            value: serde_json::json!("Night 5"),
        };
        let result = build_patch_from_action("改故事时间".into(), action, &input);
        assert!(result.is_err());
    }

    // ── build_patch_from_action: UpdateInstanceVariable ─────────────────

    #[test]
    fn test_build_patch_from_action_update_instance_variable() {
        let inst = make_instance("inst-1", Some("def-1"));
        let input = PreviewInput {
            instances: &[inst],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let action = TypedPatchAction::UpdateInstanceVariable {
            instance_id: Id::from_str("inst-1"),
            key: "hp".into(),
            value: serde_json::json!(80),
        };
        let patch = build_patch_from_action("改 HP".into(), action, &input).unwrap();
        assert_eq!(patch.source_issue_category, "agent_proposed");
        assert_eq!(patch.diff.len(), 1);
        assert_eq!(patch.diff[0].path, "instance[inst-1].variables[hp]");
    }

    #[test]
    fn test_build_patch_from_action_update_instance_variable_missing() {
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let action = TypedPatchAction::UpdateInstanceVariable {
            instance_id: Id::from_str("nonexistent"),
            key: "hp".into(),
            value: serde_json::json!(80),
        };
        let result = build_patch_from_action("改 HP".into(), action, &input);
        assert!(result.is_err());
    }

    // ── build_patch_from_action: AddKnowledge ───────────────────────────

    #[test]
    fn test_build_patch_from_action_add_knowledge() {
        let inst = make_instance("inst-1", Some("def-1"));
        let input = PreviewInput {
            instances: &[inst],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let action = TypedPatchAction::AddKnowledge {
            character_id: Id::from_str("inst-1"),
            knowledge_text: "看到了龙".into(),
            source: KnowledgeSource::Witnessed,
        };
        let patch = build_patch_from_action("加知识".into(), action, &input).unwrap();
        assert_eq!(patch.source_issue_category, "agent_proposed");
        assert_eq!(patch.diff.len(), 1);
        assert_eq!(patch.diff[0].path, "knowledge (new for inst-1)");
    }

    #[test]
    fn test_build_patch_from_action_add_knowledge_missing_character() {
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let action = TypedPatchAction::AddKnowledge {
            character_id: Id::from_str("nonexistent"),
            knowledge_text: "看到了龙".into(),
            source: KnowledgeSource::Witnessed,
        };
        let result = build_patch_from_action("加知识".into(), action, &input);
        assert!(result.is_err());
    }

    // ── build_patch_from_action: UpdateTaskStatus ───────────────────────

    #[test]
    fn test_build_patch_from_action_update_task_status() {
        let task = StoryTask::user_planned(
            Id::from_str("camp-1"),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        );
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[task],
            campaign: None,
        };
        let action = TypedPatchAction::UpdateTaskStatus {
            task_id: Id::from_str(input.tasks[0].id.to_string()),
            new_status: TaskStatus::Completed,
        };
        let patch = build_patch_from_action("完成任务".into(), action, &input).unwrap();
        assert_eq!(patch.source_issue_category, "agent_proposed");
        assert_eq!(patch.diff.len(), 1);
        assert!(patch.diff[0].path.starts_with("task["));
    }

    #[test]
    fn test_build_patch_from_action_update_task_status_missing() {
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        let action = TypedPatchAction::UpdateTaskStatus {
            task_id: Id::from_str("nonexistent"),
            new_status: TaskStatus::Completed,
        };
        let result = build_patch_from_action("完成任务".into(), action, &input);
        assert!(result.is_err());
    }

    // ── apply_to_snapshot: UpdateCampaignVariable ───────────────────────

    #[test]
    fn test_apply_update_campaign_variable() {
        let mut campaign = Campaign::new(Id::from_str("card-1"), "测试");
        let mut instances = vec![];
        let mut defs = vec![];
        let mut knowledge = vec![];
        let mut tasks = vec![];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: Some(&mut campaign),
            turn: 3,
        };

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: Some("story_clock".into()),
            actions: vec![TypedPatchAction::UpdateCampaignVariable {
                key: "story_clock".into(),
                value: serde_json::json!("Night 5"),
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");
        assert_eq!(campaign.story_clock, "Night 5");
        assert_eq!(
            campaign.get_variable("story_clock"),
            Some(&serde_json::json!("Night 5"))
        );
    }

    // ── apply_to_snapshot: UpdateInstanceVariable ───────────────────────

    #[test]
    fn test_apply_update_instance_variable() {
        let inst = make_instance("inst-1", Some("def-1"));
        let mut instances = vec![inst];
        let mut defs = vec![];
        let mut knowledge = vec![];
        let mut tasks = vec![];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: None,
            turn: 3,
        };

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: Some("inst-1".into()),
            actions: vec![TypedPatchAction::UpdateInstanceVariable {
                instance_id: Id::from_str("inst-1"),
                key: "hp".into(),
                value: serde_json::json!(80),
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");
        let updated = &snapshot.instances[0];
        assert_eq!(updated.get_variable("hp"), Some(&serde_json::json!(80)));
    }

    // ── apply_to_snapshot: AddKnowledge ─────────────────────────────────

    #[test]
    fn test_apply_add_knowledge() {
        let mut campaign = Campaign::new(Id::from_str("card-1"), "测试");
        let mut instances = vec![];
        let mut defs = vec![];
        let mut knowledge = vec![];
        let mut tasks = vec![];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: Some(&mut campaign),
            turn: 5,
        };

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: Some("char-1".into()),
            actions: vec![TypedPatchAction::AddKnowledge {
                character_id: Id::from_str("char-1"),
                knowledge_text: "看到了龙".into(),
                source: KnowledgeSource::Witnessed,
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");
        assert_eq!(snapshot.knowledge.len(), 1);
        assert_eq!(snapshot.knowledge[0].knowledge_text, "看到了龙");
        assert_eq!(snapshot.knowledge[0].character_id.as_str(), "char-1");
        assert_eq!(snapshot.knowledge[0].source, KnowledgeSource::Witnessed);
        assert_eq!(snapshot.knowledge[0].turn_number, 5);
    }

    // ── apply_to_snapshot: UpdateTaskStatus ─────────────────────────────

    #[test]
    fn test_apply_update_task_status() {
        let task = StoryTask::user_planned(
            Id::from_str("camp-1"),
            "复仇",
            "老王复仇",
            vec![TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        );
        let task_id = task.id.clone();
        let mut instances = vec![];
        let mut defs = vec![];
        let mut knowledge = vec![];
        let mut tasks = vec![task];
        let mut snapshot = PreviewInputMut {
            instances: &mut instances,
            definitions: &mut defs,
            knowledge: &mut knowledge,
            tasks: &mut tasks,
            campaign: None,
            turn: 0,
        };

        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: Some(task_id.to_string()),
            actions: vec![TypedPatchAction::UpdateTaskStatus {
                task_id: task_id.clone(),
                new_status: TaskStatus::Completed,
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };

        apply_to_snapshot(&patch, &mut snapshot).expect("apply should succeed");
        assert_eq!(snapshot.tasks[0].status, TaskStatus::Completed);
    }

    // ── is_patch_stale: new variants ────────────────────────────────────

    #[test]
    fn test_is_patch_stale_update_campaign_variable_no_campaign() {
        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: Some("story_clock".into()),
            actions: vec![TypedPatchAction::UpdateCampaignVariable {
                key: "story_clock".into(),
                value: serde_json::json!("Night 5"),
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        assert!(is_patch_stale(&patch, &input));
    }

    #[test]
    fn test_is_patch_stale_update_instance_variable_target_removed() {
        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: Some("inst-1".into()),
            actions: vec![TypedPatchAction::UpdateInstanceVariable {
                instance_id: Id::from_str("inst-1"),
                key: "hp".into(),
                value: serde_json::json!(80),
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        assert!(is_patch_stale(&patch, &input));
    }

    #[test]
    fn test_is_patch_stale_update_task_status_target_removed() {
        let patch = TypedPatch {
            id: "test".into(),
            description: "test".into(),
            source_issue_category: "agent_proposed".into(),
            affected_id: Some("task-1".into()),
            actions: vec![TypedPatchAction::UpdateTaskStatus {
                task_id: Id::from_str("task-1"),
                new_status: TaskStatus::Completed,
            }],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: TypedPatchStatus::Pending,
        };
        let input = PreviewInput {
            instances: &[],
            definitions: &[],
            knowledge: &[],
            tasks: &[],
            campaign: None,
        };
        assert!(is_patch_stale(&patch, &input));
    }
}

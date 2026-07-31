//! SQLite-native typed Meta patch repository (Gate 4).
//!
//! All actions of one `TypedPatch` are applied in a single SQLite transaction
//! (UnitOfWork). Every touched row is scope-validated against the patch
//! campaign; a missing or foreign target fails the whole UoW instead of
//! partially applying. Fault injection points prove rollback.
//!
//! This module intentionally stays inside the composition root (`tauri-app`):
//! `TypedPatchAction` is an app-meta type, so the pure-domain `infra-sqlite`
//! crate must not import it.

use storyforge_app_meta::TypedPatchAction;
use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::story_task::StoryTask;
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::UnitOfWork;
use storyforge_infra_sqlite::error::{Result as SqliteResult, SqliteError};
use storyforge_infra_sqlite::migrations;

use crate::campaign_store::StoredCard;

/// Injection points used to prove that a failure mid-patch rolls everything
/// back (nothing before the fault survives).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaPatchFault {
    None,
    /// Fail after the first action has been written to the transaction.
    AfterFirstAction,
    /// Fail after every action has been written but before commit.
    AfterAllActions,
}

pub struct SqliteMetaRepository;

impl SqliteMetaRepository {
    pub fn apply_typed_patch_actions(
        db: &mut Database,
        campaign_id: &Id,
        actions: &[TypedPatchAction],
    ) -> SqliteResult<()> {
        Self::apply_typed_patch_actions_with_fault(db, campaign_id, actions, MetaPatchFault::None)
    }

    #[doc(hidden)]
    pub fn apply_typed_patch_actions_with_fault(
        db: &mut Database,
        campaign_id: &Id,
        actions: &[TypedPatchAction],
        fault: MetaPatchFault,
    ) -> SqliteResult<()> {
        if actions.is_empty() {
            return Ok(());
        }
        migrations::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        // 前置：campaign 必须存在（scope 锚点）。
        let campaign = load_payload_tx::<Campaign>(
            tx,
            "SELECT payload_json FROM campaigns WHERE campaign_id = ?1",
            rusqlite::params![campaign_id.as_str()],
        )?
        .ok_or_else(|| SqliteError::RecordNotFound(format!("campaign {}", campaign_id)))?;
        let card_payload = load_payload_tx::<serde_json::Value>(
            tx,
            "SELECT payload_json FROM character_cards WHERE card_id = ?1",
            rusqlite::params![campaign.card_id.as_str()],
        )?;
        // 卡 payload 缺失只影响 SyncInstanceVariables 的 schema 默认值（降级为
        // null），不阻断其它 action；definition 引用校验在行级完成。
        let definitions: Vec<storyforge_domain::character::CharacterDefinition> = match card_payload
        {
            Some(payload) => serde_json::from_value::<StoredCard>(payload)
                .map(|stored| stored.card.character_definitions)
                .unwrap_or_default(),
            None => Vec::new(),
        };

        for (index, action) in actions.iter().enumerate() {
            apply_action(tx, &campaign, &definitions, action)?;
            if fault == MetaPatchFault::AfterFirstAction && index == 0 {
                return Err(SqliteError::Other(
                    "injected failure after first meta action".into(),
                ));
            }
        }
        if fault == MetaPatchFault::AfterAllActions {
            return Err(SqliteError::Other(
                "injected failure after all meta actions".into(),
            ));
        }
        uow.commit()?;
        Ok(())
    }
}

fn apply_action(
    tx: &rusqlite::Transaction<'_>,
    campaign: &Campaign,
    definitions: &[storyforge_domain::character::CharacterDefinition],
    action: &TypedPatchAction,
) -> SqliteResult<()> {
    match action {
        TypedPatchAction::SyncInstanceVariables {
            instance_id,
            definition_id,
            add_keys,
            remove_keys,
        } => {
            let mut instance = load_instance_scoped(tx, campaign, instance_id)?
                .ok_or_else(|| SqliteError::RecordNotFound(format!("instance {instance_id}")))?;
            // definition 绑定必须是当前有效绑定（stale/revision 校验：instance
            // 自 propose 后被 repoint 则拒绝）。
            if instance.definition_id.as_ref() != Some(definition_id) {
                return Err(SqliteError::Conflict(format!(
                    "instance {instance_id} no longer bound to definition {definition_id}"
                )));
            }
            let schema_defaults: std::collections::HashMap<String, serde_json::Value> = definitions
                .iter()
                .find(|d| &d.id == definition_id)
                .map(|d| {
                    d.variable_schema
                        .iter()
                        .map(|f| (f.key.clone(), f.default.clone()))
                        .collect()
                })
                .unwrap_or_default();
            for key in add_keys {
                if instance.get_variable(key).is_none() {
                    let default = schema_defaults
                        .get(key)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    instance.set_variable(key, default, 0);
                }
            }
            instance.variables.retain(|v| !remove_keys.contains(&v.key));
            write_instance(tx, &instance)?;
            Ok(())
        }
        TypedPatchAction::PruneOrphanTaskReferences {
            task_id,
            orphan_character_ids,
        } => {
            let mut task = load_task_scoped(tx, campaign, task_id)?
                .ok_or_else(|| SqliteError::RecordNotFound(format!("task {task_id}")))?;
            task.related_characters
                .retain(|id| !orphan_character_ids.contains(id));
            write_task(tx, &task)?;
            Ok(())
        }
        TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
            let entry = load_knowledge_scoped(tx, campaign, knowledge_id)?
                .ok_or_else(|| SqliteError::RecordNotFound(format!("knowledge {knowledge_id}")))?;
            // scope 已由加载校验；知识删除后须确属本 campaign。
            tx.execute(
                "DELETE FROM character_knowledge WHERE knowledge_id = ?1 AND campaign_id = ?2",
                rusqlite::params![entry.id.as_str(), campaign.id.as_str()],
            )?;
            Ok(())
        }
        TypedPatchAction::RepointInstanceDefinition {
            instance_id,
            new_definition_id,
        } => {
            let mut instance = load_instance_scoped(tx, campaign, instance_id)?
                .ok_or_else(|| SqliteError::RecordNotFound(format!("instance {instance_id}")))?;
            if let Some(new_def) = new_definition_id
                && !definitions.iter().any(|d| &d.id == new_def)
            {
                return Err(SqliteError::Conflict(format!(
                    "new_definition_id {new_def} not found in campaign card definitions"
                )));
            }
            instance.definition_id = new_definition_id.clone();
            write_instance(tx, &instance)?;
            Ok(())
        }
        TypedPatchAction::UpdateCampaignVariable { key, value } => {
            let mut campaign = campaign.clone();
            campaign.set_variable(key, value.clone(), 0);
            write_campaign(tx, &campaign)?;
            Ok(())
        }
        TypedPatchAction::UpdateInstanceVariable {
            instance_id,
            key,
            value,
        } => {
            let mut instance = load_instance_scoped(tx, campaign, instance_id)?
                .ok_or_else(|| SqliteError::RecordNotFound(format!("instance {instance_id}")))?;
            instance.set_variable(key, value.clone(), 0);
            write_instance(tx, &instance)?;
            Ok(())
        }
        TypedPatchAction::AddKnowledge {
            character_id,
            knowledge_text,
            source,
        } => {
            let entry = CharacterKnowledgeEntry {
                id: Id::new(),
                campaign_id: campaign.id.clone(),
                character_id: character_id.clone(),
                knowledge_text: knowledge_text.clone(),
                source: source.clone(),
                source_character_id: None,
                source_knowledge_id: None,
                turn_number: 0,
                event_id: None,
                pinned: false,
                propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
            };
            write_knowledge(tx, &entry)?;
            Ok(())
        }
        TypedPatchAction::UpdateTaskStatus {
            task_id,
            new_status,
        } => {
            let mut task = load_task_scoped(tx, campaign, task_id)?
                .ok_or_else(|| SqliteError::RecordNotFound(format!("task {task_id}")))?;
            task.status = new_status.clone();
            write_task(tx, &task)?;
            Ok(())
        }
    }
}

fn load_instance_scoped(
    tx: &rusqlite::Transaction<'_>,
    campaign: &Campaign,
    instance_id: &Id,
) -> SqliteResult<Option<CharacterInstance>> {
    let instance: Option<CharacterInstance> = load_payload_tx(
        tx,
        "SELECT payload_json FROM character_instances WHERE instance_id = ?1 AND campaign_id = ?2",
        (instance_id.as_str(), campaign.id.as_str()),
    )?;
    if let Some(instance) = &instance
        && instance.campaign_id != campaign.id
    {
        return Err(SqliteError::Conflict(format!(
            "instance {instance_id} does not belong to campaign {}",
            campaign.id
        )));
    }
    Ok(instance)
}

fn load_task_scoped(
    tx: &rusqlite::Transaction<'_>,
    campaign: &Campaign,
    task_id: &Id,
) -> SqliteResult<Option<StoryTask>> {
    let task: Option<StoryTask> = load_payload_tx(
        tx,
        "SELECT payload_json FROM story_tasks WHERE task_id = ?1 AND campaign_id = ?2",
        (task_id.as_str(), campaign.id.as_str()),
    )?;
    if let Some(task) = &task
        && task.campaign_id != campaign.id
    {
        return Err(SqliteError::Conflict(format!(
            "task {task_id} does not belong to campaign {}",
            campaign.id
        )));
    }
    Ok(task)
}

fn load_knowledge_scoped(
    tx: &rusqlite::Transaction<'_>,
    campaign: &Campaign,
    knowledge_id: &Id,
) -> SqliteResult<Option<CharacterKnowledgeEntry>> {
    let entry: Option<CharacterKnowledgeEntry> = load_payload_tx(
        tx,
        "SELECT payload_json FROM character_knowledge WHERE knowledge_id = ?1 AND campaign_id = ?2",
        (knowledge_id.as_str(), campaign.id.as_str()),
    )?;
    if let Some(entry) = &entry
        && entry.campaign_id != campaign.id
    {
        return Err(SqliteError::Conflict(format!(
            "knowledge {knowledge_id} does not belong to campaign {}",
            campaign.id
        )));
    }
    Ok(entry)
}

fn write_instance(
    tx: &rusqlite::Transaction<'_>,
    instance: &CharacterInstance,
) -> SqliteResult<()> {
    tx.execute(
        r#"
        INSERT INTO character_instances (instance_id, campaign_id, definition_id, name, is_temporary, payload_json)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(instance_id) DO UPDATE SET
            campaign_id=excluded.campaign_id, definition_id=excluded.definition_id,
            name=excluded.name, is_temporary=excluded.is_temporary,
            payload_json=excluded.payload_json
        "#,
        rusqlite::params![
            instance.id.as_str(),
            instance.campaign_id.as_str(),
            instance.definition_id.as_ref().map(Id::as_str),
            instance.name,
            instance.is_temporary as i64,
            json(instance)?,
        ],
    )?;
    Ok(())
}

fn write_task(tx: &rusqlite::Transaction<'_>, task: &StoryTask) -> SqliteResult<()> {
    tx.execute(
        r#"
        INSERT INTO story_tasks (task_id, campaign_id, payload_json)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(task_id) DO UPDATE SET
            campaign_id=excluded.campaign_id, payload_json=excluded.payload_json
        "#,
        rusqlite::params![task.id.as_str(), task.campaign_id.as_str(), json(task)?],
    )?;
    Ok(())
}

fn write_knowledge(
    tx: &rusqlite::Transaction<'_>,
    entry: &CharacterKnowledgeEntry,
) -> SqliteResult<()> {
    tx.execute(
        r#"
        INSERT INTO character_knowledge (knowledge_id, campaign_id, payload_json)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(knowledge_id) DO UPDATE SET
            campaign_id=excluded.campaign_id, payload_json=excluded.payload_json
        "#,
        rusqlite::params![entry.id.as_str(), entry.campaign_id.as_str(), json(entry)?],
    )?;
    Ok(())
}

fn write_campaign(tx: &rusqlite::Transaction<'_>, campaign: &Campaign) -> SqliteResult<()> {
    tx.execute(
        r#"
        INSERT INTO campaigns (
            campaign_id, card_id, name, conversation_id, revision, chronicle_revision,
            lineage_id, story_clock, created_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(campaign_id) DO UPDATE SET
            card_id=excluded.card_id, name=excluded.name,
            conversation_id=excluded.conversation_id, revision=excluded.revision,
            chronicle_revision=excluded.chronicle_revision, lineage_id=excluded.lineage_id,
            story_clock=excluded.story_clock, created_at=excluded.created_at,
            payload_json=excluded.payload_json
        "#,
        rusqlite::params![
            campaign.id.as_str(),
            campaign.card_id.as_str(),
            campaign.name,
            campaign.conversation_id.as_ref().map(Id::as_str),
            campaign.revision,
            campaign.chronicle_revision,
            campaign.lineage_id.as_ref().map(Id::as_str),
            campaign.current_story_clock(),
            campaign.created_at,
            json(campaign)?,
        ],
    )?;
    Ok(())
}

fn load_payload_tx<T: serde::de::DeserializeOwned>(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    params: impl rusqlite::Params,
) -> SqliteResult<Option<T>> {
    use rusqlite::OptionalExtension;
    let payload: Option<String> = tx
        .query_row(sql, params, |row| row.get(0))
        .optional()
        .map_err(SqliteError::from)?;
    payload
        .map(|value| serde_json::from_str(&value).map_err(SqliteError::from))
        .transpose()
}

fn json(value: &impl serde::Serialize) -> SqliteResult<String> {
    serde_json::to_string(value).map_err(SqliteError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_app_meta::TypedPatchAction;
    use storyforge_domain::Id;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{
        CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
    };
    use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
    use storyforge_domain::story_task::{StoryTask, TaskStatus, TaskTrigger};
    use storyforge_infra_sqlite::Database;
    use storyforge_infra_sqlite::production::SqliteProductionRepository;

    struct Fixture {
        db: Database,
        campaign_id: Id,
        definition_id: Id,
        instance_id: Id,
        task_id: Id,
    }

    fn setup() -> Fixture {
        let mut db = Database::open_in_memory().unwrap();
        let campaign_id = Id::from_str("meta-camp-1");
        let card_id = Id::from_str("meta-card-1");
        let definition_id = Id::from_str("meta-def-1");
        let instance_id = Id::from_str("meta-inst-1");
        let task_id = Id::from_str("meta-task-1");

        let definition = CharacterDefinition {
            id: definition_id.clone(),
            card_id: card_id.clone(),
            name: "Lin".into(),
            persona_prompt: "calm".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![storyforge_domain::variables::VariableField {
                key: "hp".into(),
                label: "HP".into(),
                value_type: storyforge_domain::variables::VariableType::Int,
                default: serde_json::json!(100),
                description: None,
                group: None,
            }],
        };
        let stored_card = StoredCard {
            card: CharacterCard {
                id: card_id.clone(),
                name: "Fixture".into(),
                source_character_id: Id::from_str("meta-source-1"),
                character_definitions: vec![definition],
                campaign_variable_schema: vec![],
                raw_card_json: serde_json::json!({}),
                extraction_status: CharacterExtractionStatus::Extracted,
                extraction_message: None,
            },
            imported_at: "2026-07-16T00:00:00Z".into(),
        };
        SqliteProductionRepository::save_card_payload(
            &mut db,
            &card_id,
            "Fixture",
            Some("meta-source-1"),
            Some("2026-07-16T00:00:00Z"),
            &serde_json::to_value(&stored_card).unwrap(),
        )
        .unwrap();
        let mut campaign = Campaign::new(card_id.clone(), "Meta Fixture");
        campaign.id = campaign_id.clone();
        SqliteProductionRepository::save_campaign(&mut db, &campaign).unwrap();

        let mut instance = CharacterInstance::from_definition(
            campaign_id.clone(),
            &CharacterDefinition {
                id: definition_id.clone(),
                card_id: card_id.clone(),
                name: "Lin".into(),
                persona_prompt: "calm".into(),
                behavior_rules: "save first".into(),
                base_backstory: vec![],
                group: None,
                role_type: RoleType::Protagonist,
                variable_schema: vec![],
            },
        );
        instance.id = instance_id.clone();
        instance.definition_id = Some(definition_id.clone());
        SqliteProductionRepository::save_instance(&mut db, &instance).unwrap();

        let mut task = StoryTask::user_planned(
            campaign_id.clone(),
            "Task",
            "desc",
            vec![TaskTrigger::Manual],
            0,
        );
        task.id = task_id.clone();
        task.related_characters = vec![instance_id.clone()];
        SqliteProductionRepository::save_task(&mut db, &task).unwrap();

        let knowledge =
            CharacterKnowledgeEntry::backstory(campaign_id.clone(), instance_id.clone(), "fact");
        SqliteProductionRepository::save_knowledge(&mut db, &knowledge).unwrap();

        Fixture {
            db,
            campaign_id,
            definition_id,
            instance_id,
            task_id,
        }
    }

    #[test]
    fn applies_all_actions_in_one_transaction() {
        let mut f = setup();
        let actions = vec![
            TypedPatchAction::UpdateCampaignVariable {
                key: "gate4_weather".into(),
                value: serde_json::json!("rain"),
            },
            TypedPatchAction::UpdateInstanceVariable {
                instance_id: f.instance_id.clone(),
                key: "hp".into(),
                value: serde_json::json!(42),
            },
            TypedPatchAction::UpdateTaskStatus {
                task_id: f.task_id.clone(),
                new_status: TaskStatus::Completed,
            },
        ];
        SqliteMetaRepository::apply_typed_patch_actions(&mut f.db, &f.campaign_id, &actions)
            .unwrap();

        let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            campaign.get_variable("gate4_weather"),
            Some(&serde_json::json!("rain"))
        );
        let instances = SqliteProductionRepository::list_instances(&f.db, &f.campaign_id).unwrap();
        assert_eq!(
            instances[0].get_variable("hp"),
            Some(&serde_json::json!(42))
        );
        let tasks = SqliteProductionRepository::list_tasks(&f.db, &f.campaign_id).unwrap();
        assert_eq!(tasks[0].status, TaskStatus::Completed);
    }

    #[test]
    fn sync_instance_variables_uses_schema_defaults_and_scoped_definition() {
        let mut f = setup();
        let actions = vec![TypedPatchAction::SyncInstanceVariables {
            instance_id: f.instance_id.clone(),
            definition_id: f.definition_id.clone(),
            add_keys: vec!["hp".into(), "mana".into()],
            remove_keys: vec![],
        }];
        SqliteMetaRepository::apply_typed_patch_actions(&mut f.db, &f.campaign_id, &actions)
            .unwrap();
        let instances = SqliteProductionRepository::list_instances(&f.db, &f.campaign_id).unwrap();
        assert_eq!(
            instances[0].get_variable("hp"),
            Some(&serde_json::json!(100))
        );
        assert_eq!(
            instances[0].get_variable("mana"),
            Some(&serde_json::Value::Null)
        );
    }

    #[test]
    fn sync_instance_variables_rejects_stale_definition_binding() {
        let mut f = setup();
        let actions = vec![TypedPatchAction::SyncInstanceVariables {
            instance_id: f.instance_id.clone(),
            definition_id: Id::from_str("other-def"),
            add_keys: vec![],
            remove_keys: vec![],
        }];
        let err =
            SqliteMetaRepository::apply_typed_patch_actions(&mut f.db, &f.campaign_id, &actions)
                .unwrap_err();
        assert!(err.to_string().contains("no longer bound"));
    }

    #[test]
    fn fault_after_first_action_rolls_back_everything() {
        let mut f = setup();
        let actions = vec![
            TypedPatchAction::UpdateCampaignVariable {
                key: "gate4_weather".into(),
                value: serde_json::json!("rain"),
            },
            TypedPatchAction::UpdateTaskStatus {
                task_id: f.task_id.clone(),
                new_status: TaskStatus::Completed,
            },
        ];
        let err = SqliteMetaRepository::apply_typed_patch_actions_with_fault(
            &mut f.db,
            &f.campaign_id,
            &actions,
            MetaPatchFault::AfterFirstAction,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("injected failure after first meta action")
        );

        // 事务回滚：campaign 变量与 task 状态都不能留下任何痕迹。
        let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
            .unwrap()
            .unwrap();
        assert_eq!(campaign.get_variable("gate4_weather"), None);
        let tasks = SqliteProductionRepository::list_tasks(&f.db, &f.campaign_id).unwrap();
        assert_eq!(tasks[0].status, TaskStatus::Pending);
    }

    #[test]
    fn fault_after_all_actions_rolls_back_everything() {
        let mut f = setup();
        let actions = vec![TypedPatchAction::UpdateCampaignVariable {
            key: "gate4_weather".into(),
            value: serde_json::json!("rain"),
        }];
        let err = SqliteMetaRepository::apply_typed_patch_actions_with_fault(
            &mut f.db,
            &f.campaign_id,
            &actions,
            MetaPatchFault::AfterAllActions,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("injected failure after all meta actions")
        );
        let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
            .unwrap()
            .unwrap();
        assert_eq!(campaign.get_variable("gate4_weather"), None);
    }

    #[test]
    fn missing_target_fails_closed_with_scope_validation() {
        let mut f = setup();
        // 目标 instance 属于另一 campaign → 必须拒绝且无部分写入。
        let foreign_campaign_id = Id::from_str("other-camp");
        let actions = vec![TypedPatchAction::UpdateInstanceVariable {
            instance_id: f.instance_id.clone(),
            key: "hp".into(),
            value: serde_json::json!(1),
        }];
        let err = SqliteMetaRepository::apply_typed_patch_actions(
            &mut f.db,
            &foreign_campaign_id,
            &actions,
        )
        .unwrap_err();
        assert!(err.to_string().contains("campaign other-camp"));
    }

    #[test]
    fn repoint_definition_validates_new_definition_exists() {
        let mut f = setup();
        let actions = vec![TypedPatchAction::RepointInstanceDefinition {
            instance_id: f.instance_id.clone(),
            new_definition_id: Some(Id::from_str("ghost-def")),
        }];
        let err =
            SqliteMetaRepository::apply_typed_patch_actions(&mut f.db, &f.campaign_id, &actions)
                .unwrap_err();
        assert!(
            err.to_string()
                .contains("not found in campaign card definitions")
        );
        let instances = SqliteProductionRepository::list_instances(&f.db, &f.campaign_id).unwrap();
        assert_eq!(instances[0].definition_id.as_ref(), Some(&f.definition_id));
    }

    #[test]
    fn delete_orphan_knowledge_and_prune_task_references() {
        let mut f = setup();
        let knowledge_id = {
            let entries =
                SqliteProductionRepository::list_knowledge(&f.db, &f.campaign_id).unwrap();
            entries[0].id.clone()
        };
        let actions = vec![
            TypedPatchAction::DeleteOrphanKnowledge {
                knowledge_id: knowledge_id.clone(),
            },
            TypedPatchAction::PruneOrphanTaskReferences {
                task_id: f.task_id.clone(),
                orphan_character_ids: vec![f.instance_id.clone()],
            },
        ];
        SqliteMetaRepository::apply_typed_patch_actions(&mut f.db, &f.campaign_id, &actions)
            .unwrap();
        assert!(
            SqliteProductionRepository::list_knowledge(&f.db, &f.campaign_id)
                .unwrap()
                .is_empty()
        );
        let tasks = SqliteProductionRepository::list_tasks(&f.db, &f.campaign_id).unwrap();
        assert!(tasks[0].related_characters.is_empty());
    }

    #[test]
    fn add_knowledge_scopes_to_campaign() {
        let mut f = setup();
        let actions = vec![TypedPatchAction::AddKnowledge {
            character_id: f.instance_id.clone(),
            knowledge_text: "new fact".into(),
            source: KnowledgeSource::Inferred,
        }];
        SqliteMetaRepository::apply_typed_patch_actions(&mut f.db, &f.campaign_id, &actions)
            .unwrap();
        let entries = SqliteProductionRepository::list_knowledge(&f.db, &f.campaign_id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(&entries[1].campaign_id, &f.campaign_id);
    }
}

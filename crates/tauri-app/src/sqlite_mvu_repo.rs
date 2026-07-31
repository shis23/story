//! SQLite-native MVU schema apply repository (Gate 4).
//!
//! `meta_apply_mvu_schema` semantics under one SQLite transaction:
//! translation → card (by source id) → definition (must belong to the card) →
//! merged schema preview → card payload update → default backfill for every
//! existing instance referencing the definition (across campaigns). A failure
//! at any point rolls the whole operation back; fault injection points prove
//! it.

use storyforge_app_meta::{MvuApplyError, apply_schema_to_definition, compute_apply_preview};
use storyforge_domain::Id;
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::UnitOfWork;
use storyforge_infra_sqlite::error::{Result as SqliteResult, SqliteError};
use storyforge_infra_sqlite::migrations;

use crate::campaign_store::{StoredCard, StoredMvuTranslation};

/// Injection points used to prove full rollback of the MVU apply UoW.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MvuApplyFault {
    None,
    /// Fail after the card payload has been updated inside the transaction.
    AfterCardUpdate,
    /// Fail after some instances were backfilled but before commit.
    AfterInstanceBackfill,
}

/// Result summary for the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MvuApplySummary {
    pub definition_updated: bool,
    pub instances_backfilled: usize,
}

pub struct SqliteMvuRepository;

impl SqliteMvuRepository {
    pub fn apply_schema(
        db: &mut Database,
        source_character_id: &Id,
        definition_id: &Id,
    ) -> SqliteResult<MvuApplySummary> {
        Self::apply_schema_with_fault(db, source_character_id, definition_id, MvuApplyFault::None)
    }

    #[doc(hidden)]
    pub fn apply_schema_with_fault(
        db: &mut Database,
        source_character_id: &Id,
        definition_id: &Id,
        fault: MvuApplyFault,
    ) -> SqliteResult<MvuApplySummary> {
        migrations::migrate(db)?;
        let uow = UnitOfWork::begin(db.connection_mut())?;
        let tx = uow.transaction()?;

        // 1. 翻译必须存在（source 卡唯一索引反查入口）。
        let mvu_payload: Option<String> = tx
            .query_row(
                "SELECT payload_json FROM mvu_translations WHERE source_character_id = ?1",
                [source_character_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let mvu_payload = mvu_payload.ok_or_else(|| {
            SqliteError::RecordNotFound(format!(
                "{}",
                MvuApplyError::TranslationNotFound(source_character_id.as_str().to_string())
            ))
        })?;
        let stored_mvu: StoredMvuTranslation =
            serde_json::from_str(&mvu_payload).map_err(SqliteError::from)?;

        // 2. card 必须按 source_character_id 可反查（translation ↔ card 归属）。
        let card_payload: Option<String> = tx
            .query_row(
                "SELECT payload_json FROM character_cards WHERE source_character_id = ?1",
                [source_character_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let card_payload = card_payload.ok_or_else(|| {
            SqliteError::RecordNotFound(format!(
                "找不到 source_character_id={source_character_id} 的 card"
            ))
        })?;
        let stored_card: StoredCard =
            serde_json::from_str(&card_payload).map_err(SqliteError::from)?;

        // 3. definition 必须属于该卡（card ↔ definition 归属校验）。
        let def = stored_card
            .card
            .character_definitions
            .iter()
            .find(|d| d.id == *definition_id)
            .ok_or_else(|| {
                SqliteError::RecordNotFound(format!(
                    "{}",
                    MvuApplyError::DefinitionNotFound(definition_id.as_str().to_string())
                ))
            })?;
        if def.card_id != stored_card.card.id {
            return Err(SqliteError::Conflict(format!(
                "definition {} belongs to card {}, not {}",
                def.id, def.card_id, stored_card.card.id
            )));
        }

        // 4. 预览 + 变化判定（与 JSON 路径同一纯函数）。
        let normalized_mvu_schema = storyforge_domain::variables::normalize_schema_keys(
            stored_mvu.translation.variable_schema.clone(),
        );
        let preview = compute_apply_preview(
            &def.variable_schema,
            &normalized_mvu_schema,
            def.id.as_str(),
            &def.name,
            source_character_id.as_str(),
        );
        if !preview.has_changes {
            return Err(SqliteError::RecordNotFound(format!(
                "{}",
                MvuApplyError::NoChanges
            )));
        }

        // 5. card payload 更新（definition.variable_schema = merged）。
        let mut card = stored_card.card.clone();
        if let Some(target_def) = card
            .character_definitions
            .iter_mut()
            .find(|d| d.id == *definition_id)
        {
            apply_schema_to_definition(target_def, preview.merged_schema.clone());
        }
        tx.execute(
            r#"
            INSERT INTO character_cards (card_id, source_character_id, name, imported_at, payload_json)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(card_id) DO UPDATE SET
                source_character_id = excluded.source_character_id,
                name = excluded.name,
                imported_at = excluded.imported_at,
                payload_json = excluded.payload_json
            "#,
            rusqlite::params![
                card.id.as_str(),
                card.source_character_id.as_str(),
                card.name,
                stored_card.imported_at,
                serde_json::to_string(&StoredCard {
                    card: card.clone(),
                    imported_at: stored_card.imported_at.clone(),
                })
                .map_err(SqliteError::from)?,
            ],
        )?;
        if fault == MvuApplyFault::AfterCardUpdate {
            return Err(SqliteError::Other(
                "injected failure after MVU card update".into(),
            ));
        }

        // 6. 已存在 instances 默认值回填（跨 campaign，凡引用该 definition 的）。
        let mut instances_backfilled = 0usize;
        let mut stmt = tx.prepare(
            "SELECT instance_id, payload_json FROM character_instances WHERE definition_id = ?1",
        )?;
        let rows = stmt.query_map([definition_id.as_str()], |row| {
            let id: String = row.get(0)?;
            let payload: String = row.get(1)?;
            Ok((id, payload))
        })?;
        let mut to_write: Vec<storyforge_domain::campaign::CharacterInstance> = Vec::new();
        for row in rows {
            let (instance_id, payload) = row?;
            let mut instance: storyforge_domain::campaign::CharacterInstance =
                serde_json::from_str(&payload).map_err(SqliteError::from)?;
            if instance.id.as_str() != instance_id.as_str() {
                return Err(SqliteError::Conflict(format!(
                    "instance payload id {instance_id} mismatch"
                )));
            }
            let missing_fields: Vec<_> = preview
                .merged_schema
                .iter()
                .filter(|f| !instance.variables.iter().any(|v| v.key == f.key))
                .collect();
            if missing_fields.is_empty() {
                continue;
            }
            for field in missing_fields {
                instance
                    .variables
                    .push(storyforge_domain::variables::VariableValue::new(
                        field.key.clone(),
                        field.default.clone(),
                        0,
                    ));
            }
            instances_backfilled += 1;
            to_write.push(instance);
        }
        drop(stmt);
        for instance in &to_write {
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
                    serde_json::to_string(instance).map_err(SqliteError::from)?,
                ],
            )?;
        }
        if fault == MvuApplyFault::AfterInstanceBackfill {
            return Err(SqliteError::Other(
                "injected failure after MVU instance backfill".into(),
            ));
        }

        uow.commit()?;
        Ok(MvuApplySummary {
            definition_updated: true,
            instances_backfilled,
        })
    }
}

use rusqlite::OptionalExtension;
#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::campaign::CharacterInstance;
    use storyforge_domain::character::{
        CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
    };
    use storyforge_domain::mvu_translation::MvuTranslation;
    use storyforge_domain::variables::{VariableField, VariableType};
    use storyforge_infra_sqlite::production::SqliteProductionRepository;

    fn card_fixture(card_id: &Id, source_id: &Id, definition_id: &Id) -> StoredCard {
        let definition = CharacterDefinition {
            id: definition_id.clone(),
            card_id: card_id.clone(),
            name: "Lin".into(),
            persona_prompt: "calm".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![VariableField {
                key: "hp".into(),
                label: "HP".into(),
                value_type: VariableType::Int,
                default: serde_json::json!(100),
                description: None,
                group: None,
            }],
        };
        StoredCard {
            card: CharacterCard {
                id: card_id.clone(),
                name: "MVU Fixture".into(),
                source_character_id: source_id.clone(),
                character_definitions: vec![definition],
                campaign_variable_schema: vec![],
                raw_card_json: serde_json::json!({}),
                extraction_status: CharacterExtractionStatus::Extracted,
                extraction_message: None,
            },
            imported_at: "2026-07-16T00:00:00Z".into(),
        }
    }

    fn mvu_fixture() -> StoredMvuTranslation {
        StoredMvuTranslation {
            source_character_id: Id::from_str("mvu-source-1"),
            character_name: "MVU Fixture".into(),
            analyzed_at: "2026-07-16T00:00:00Z".into(),
            translation: MvuTranslation {
                routing: storyforge_domain::mvu_translation::MvuRouting::Native,
                variable_schema: vec![
                    VariableField {
                        key: "hp".into(),
                        label: "HP".into(),
                        value_type: VariableType::Int,
                        default: serde_json::json!(120),
                        description: None,
                        group: None,
                    },
                    VariableField {
                        key: "mvu_mana".into(),
                        label: "Mana".into(),
                        value_type: VariableType::Int,
                        default: serde_json::json!(50),
                        description: None,
                        group: None,
                    },
                ],
                ui_bindings: vec![],
                update_rules: vec![],
                interactions: vec![],
                fallback_fragments: vec![],
                analysis_confidence: 0.9,
                notes: vec![],
            },
        }
    }

    fn setup() -> (Database, Id, Id, Id) {
        let mut db = Database::open_in_memory().unwrap();
        let card_id = Id::from_str("mvu-card-1");
        let source_id = Id::from_str("mvu-source-1");
        let definition_id = Id::from_str("mvu-def-1");
        let stored_card = card_fixture(&card_id, &source_id, &definition_id);
        SqliteProductionRepository::save_card_payload(
            &mut db,
            &card_id,
            "MVU Fixture",
            Some(source_id.as_str()),
            Some("2026-07-16T00:00:00Z"),
            &serde_json::to_value(&stored_card).unwrap(),
        )
        .unwrap();
        SqliteProductionRepository::save_mvu_payload(
            &mut db,
            &source_id,
            "MVU Fixture",
            &serde_json::to_value(mvu_fixture()).unwrap(),
        )
        .unwrap();
        // 两个 campaign 引用同一 definition（跨 campaign backfill 验证）。
        for (camp_index, instance_id) in [(0, "mvu-inst-a"), (1, "mvu-inst-b")].into_iter() {
            let campaign_id = Id::from_str(format!("mvu-camp-{camp_index}"));
            let mut campaign = storyforge_domain::campaign::Campaign::new(card_id.clone(), "C");
            campaign.id = campaign_id.clone();
            SqliteProductionRepository::save_campaign(&mut db, &campaign).unwrap();
            let mut instance = CharacterInstance::from_definition(
                campaign_id.clone(),
                &stored_card.card.character_definitions[0],
            );
            instance.id = Id::from_str(instance_id);
            SqliteProductionRepository::save_instance(&mut db, &instance).unwrap();
        }
        (db, source_id, definition_id, card_id)
    }

    #[test]
    fn applies_schema_and_backfills_all_instances_in_one_transaction() {
        let (mut db, source_id, definition_id, _card_id) = setup();
        let summary =
            SqliteMvuRepository::apply_schema(&mut db, &source_id, &definition_id).unwrap();
        assert!(summary.definition_updated);
        assert_eq!(summary.instances_backfilled, 2);

        // card payload 中的 definition schema 已合并（hp 默认 120 + mvu_mana）。
        let payload =
            SqliteProductionRepository::get_card_payload(&db, &Id::from_str("mvu-card-1"))
                .unwrap()
                .unwrap();
        let stored: StoredCard = serde_json::from_value(payload).unwrap();
        let def = stored
            .card
            .character_definitions
            .iter()
            .find(|d| d.id == definition_id)
            .unwrap();
        assert!(def.variable_schema.iter().any(|f| f.key == "mvu_mana"));
        let hp = def.variable_schema.iter().find(|f| f.key == "hp").unwrap();
        assert_eq!(hp.default, serde_json::json!(120));

        // 两个 campaign 的 instance 都回填默认值（已有 hp=100 不覆盖，
        // 缺失的 mvu_mana 补 50）。
        for instance_id in ["mvu-inst-a", "mvu-inst-b"] {
            let all = SqliteProductionRepository::list_all_instances(&db).unwrap();
            let instance = all.iter().find(|i| i.id.as_str() == instance_id).unwrap();
            assert_eq!(instance.get_variable("hp"), Some(&serde_json::json!(100)));
            assert_eq!(
                instance.get_variable("mvu_mana"),
                Some(&serde_json::json!(50))
            );
        }
    }

    #[test]
    fn fault_after_card_update_rolls_back_card_and_instances() {
        let (mut db, source_id, definition_id, _card_id) = setup();
        let err = SqliteMvuRepository::apply_schema_with_fault(
            &mut db,
            &source_id,
            &definition_id,
            MvuApplyFault::AfterCardUpdate,
        )
        .unwrap_err();
        assert!(err.to_string().contains("after MVU card update"));

        let payload =
            SqliteProductionRepository::get_card_payload(&db, &Id::from_str("mvu-card-1"))
                .unwrap()
                .unwrap();
        let stored: StoredCard = serde_json::from_value(payload).unwrap();
        let def = stored
            .card
            .character_definitions
            .iter()
            .find(|d| d.id == definition_id)
            .unwrap();
        assert!(
            !def.variable_schema.iter().any(|f| f.key == "mvu_mana"),
            "card update must be rolled back"
        );
        let all = SqliteProductionRepository::list_all_instances(&db).unwrap();
        for instance in &all {
            assert_eq!(
                instance.get_variable("mvu_mana"),
                None,
                "instance backfill must be rolled back"
            );
        }
    }

    #[test]
    fn fault_after_instance_backfill_rolls_back_instances() {
        let (mut db, source_id, definition_id, _card_id) = setup();
        let err = SqliteMvuRepository::apply_schema_with_fault(
            &mut db,
            &source_id,
            &definition_id,
            MvuApplyFault::AfterInstanceBackfill,
        )
        .unwrap_err();
        assert!(err.to_string().contains("after MVU instance backfill"));
        let all = SqliteProductionRepository::list_all_instances(&db).unwrap();
        for instance in &all {
            assert_eq!(instance.get_variable("mvu_mana"), None);
        }
    }

    #[test]
    fn missing_translation_or_card_fails_closed() {
        let (mut db, source_id, definition_id, _card_id) = setup();
        let err = SqliteMvuRepository::apply_schema(
            &mut db,
            &Id::from_str("ghost-source"),
            &definition_id,
        )
        .unwrap_err();
        assert!(err.to_string().contains("翻译不存在"), "got: {err}");

        let mut db2 = Database::open_in_memory().unwrap();
        let err =
            SqliteMvuRepository::apply_schema(&mut db2, &source_id, &definition_id).unwrap_err();
        assert!(err.to_string().contains("翻译不存在"), "got: {err}");
    }

    #[test]
    fn no_changes_is_rejected_without_writes() {
        let (mut db, source_id, _definition_id, _card_id) = setup();
        // schema 相同 → NoChanges。
        let same = StoredMvuTranslation {
            source_character_id: source_id.clone(),
            character_name: "MVU Fixture".into(),
            analyzed_at: "2026-07-16T00:00:00Z".into(),
            translation: MvuTranslation {
                routing: storyforge_domain::mvu_translation::MvuRouting::Native,
                variable_schema: vec![VariableField {
                    key: "hp".into(),
                    label: "HP".into(),
                    value_type: VariableType::Int,
                    default: serde_json::json!(100),
                    description: None,
                    group: None,
                }],
                ui_bindings: vec![],
                update_rules: vec![],
                interactions: vec![],
                fallback_fragments: vec![],
                analysis_confidence: 0.9,
                notes: vec![],
            },
        };
        SqliteProductionRepository::save_mvu_payload(
            &mut db,
            &source_id,
            "MVU Fixture",
            &serde_json::to_value(&same).unwrap(),
        )
        .unwrap();
        let err =
            SqliteMvuRepository::apply_schema(&mut db, &source_id, &Id::from_str("mvu-def-1"))
                .unwrap_err();
        assert!(err.to_string().contains("无变化"), "got: {err}");
    }
}

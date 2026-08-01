//! Process-owned SQLite production storage boundary.
//!
//! When the backend selector chooses SQLite, this module owns the open
//! database handle and routes Accept / recovery / campaign / conversation /
//! turn operations through `SqliteProductionRepository`. JSON stores are not
//! consulted and no dual-write occurs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rusqlite::OptionalExtension;
use storyforge_app_conversation::{ConversationError, ConversationPersistence};
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::CharacterCard;
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::conversation::Conversation;
use storyforge_domain::story_task::StoryTask;
use storyforge_domain::turn::{AttemptStatus, TurnRecord, TurnStatus};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, DraftAttemptOutcome, DraftAttemptRequest, PostprocessApplyOutcome,
    PostprocessApplyRequest, PreacceptOutboxRow, PreacceptRecoverySnapshot,
    RegenerateAttemptRequest, SqlitePreacceptRepository,
};
use storyforge_infra_sqlite::production::{
    AcceptOutcome as SqliteAcceptOutcome, AcceptTurnRequest, SqliteProductionRepository,
};
use storyforge_infra_sqlite::{current_version, migrate};

use crate::commands::characters::CharacterInfo;
use crate::storage::StoredCharacter;

/// Process-owned SQLite handle. Opened once when SQLite is selected.
static SQLITE_DB: OnceLock<Arc<Mutex<Database>>> = OnceLock::new();
static SQLITE_PATH: OnceLock<PathBuf> = OnceLock::new();
static SQLITE_ACTIVATION: Mutex<()> = Mutex::new(());

/// SQLite-backed durable authority injected into `ConversationStore` when the
/// opt-in backend is active. It makes the existing pipeline's user/draft/
/// regenerate mutations write the same database used by Accept and recovery.
#[derive(Clone)]
pub struct SqliteConversationPersistence {
    db: Arc<Mutex<Database>>,
}

impl SqliteConversationPersistence {
    fn new(db: Arc<Mutex<Database>>) -> Self {
        Self { db }
    }
}

impl ConversationPersistence for SqliteConversationPersistence {
    fn load_all(&self) -> Result<Vec<Conversation>, ConversationError> {
        let db = self.db.lock().map_err(|_| {
            ConversationError::ExternalStorage("sqlite database lock poisoned".into())
        })?;
        SqliteProductionRepository::list_conversations(&db)
            .map_err(|error| ConversationError::ExternalStorage(error.to_string()))
    }

    fn save(&self, conversation: &Conversation) -> Result<(), ConversationError> {
        let mut db = self.db.lock().map_err(|_| {
            ConversationError::ExternalStorage("sqlite database lock poisoned".into())
        })?;
        SqliteProductionRepository::save_conversation(&mut db, conversation)
            .map_err(|error| ConversationError::ExternalStorage(error.to_string()))
    }

    fn delete(&self, id: &Id) -> Result<(), ConversationError> {
        let mut db = self.db.lock().map_err(|_| {
            ConversationError::ExternalStorage("sqlite database lock poisoned".into())
        })?;
        SqliteProductionRepository::delete_conversation(&mut db, id)
            .map_err(|error| ConversationError::ExternalStorage(error.to_string()))
    }
}

/// Whether the process is running with SQLite as the authoritative backend.
pub fn is_sqlite_active() -> bool {
    SQLITE_DB.get().is_some()
}

/// Open and pin the SQLite database for this process. Fail closed if the path
/// cannot be opened or migrated.
pub fn activate(db_path: impl AsRef<Path>) -> Result<(), String> {
    let _activation = SQLITE_ACTIVATION
        .lock()
        .map_err(|_| "sqlite activation lock poisoned".to_string())?;
    let path = db_path.as_ref().to_path_buf();
    if let Some(active_path) = SQLITE_PATH.get() {
        return if authority_paths_match(active_path, &path) {
            Ok(())
        } else {
            Err("sqlite backend already active at a different path".to_string())
        };
    }
    if SQLITE_DB.get().is_some() {
        return Err("sqlite database is active without a pinned path".to_string());
    }
    let mut db = Database::open(&path).map_err(|e| format!("open sqlite: {e}"))?;
    migrate(&mut db).map_err(|e| format!("migrate sqlite: {e}"))?;
    let _ = current_version(&db).map_err(|e| format!("schema version: {e}"))?;
    SQLITE_PATH
        .set(path)
        .map_err(|_| "sqlite path already set".to_string())?;
    SQLITE_DB
        .set(Arc::new(Mutex::new(db)))
        .map_err(|_| "sqlite database already set".to_string())?;
    Ok(())
}

fn authority_paths_match(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

/// Validate that SQLite is active at the expected process authority path.
pub fn validate_active_path(expected: &Path) -> Result<(), String> {
    let active = SQLITE_PATH
        .get()
        .ok_or_else(|| "SQLite facade requires an active SQLite runtime".to_string())?;
    if authority_paths_match(active, expected) {
        Ok(())
    } else {
        Err("SQLite facade/runtime path mismatch".to_string())
    }
}

/// Build the sole durable conversation authority for an SQLite process.
pub fn conversation_persistence() -> Result<Arc<dyn ConversationPersistence>, String> {
    let db = SQLITE_DB
        .get()
        .cloned()
        .ok_or_else(|| "sqlite backend is not active".to_string())?;
    Ok(Arc::new(SqliteConversationPersistence::new(db)))
}

fn with_db_mut<T>(f: impl FnOnce(&mut Database) -> Result<T, String>) -> Result<T, String> {
    let mutex = SQLITE_DB
        .get()
        .ok_or_else(|| "sqlite backend is not active".to_string())?;
    let mut db = mutex
        .lock()
        .map_err(|_| "sqlite database lock poisoned".to_string())?;
    f(&mut db)
}

fn with_db<T>(f: impl FnOnce(&Database) -> Result<T, String>) -> Result<T, String> {
    let mutex = SQLITE_DB
        .get()
        .ok_or_else(|| "sqlite backend is not active".to_string())?;
    let db = mutex
        .lock()
        .map_err(|_| "sqlite database lock poisoned".to_string())?;
    f(&db)
}

/// Integration-test access to the process DB connection (dependent-row
/// assertions / fixture seeding). Not for production use.
#[doc(hidden)]
pub fn with_db_raw<T>(f: impl FnOnce(&Database) -> T) -> T {
    let mutex = SQLITE_DB.get().expect("sqlite backend active");
    let db = mutex.lock().map_err(|_| "poisoned").unwrap();
    f(&db)
}

pub fn get_campaign(campaign_id: &Id) -> Result<Option<Campaign>, String> {
    with_db(|db| {
        SqliteProductionRepository::get_campaign(db, campaign_id).map_err(|e| e.to_string())
    })
}

pub fn list_campaigns() -> Result<Vec<Campaign>, String> {
    with_db(|db| SqliteProductionRepository::list_campaigns(db).map_err(|e| e.to_string()))
}

pub fn get_conversation(conversation_id: &Id) -> Result<Option<Conversation>, String> {
    with_db(|db| {
        SqliteProductionRepository::get_conversation(db, conversation_id).map_err(|e| e.to_string())
    })
}

pub fn get_turn_by_variant(variant_id: &Id) -> Result<Option<TurnRecord>, String> {
    with_db(|db| {
        SqliteProductionRepository::get_turn_by_variant(db, variant_id).map_err(|e| e.to_string())
    })
}

pub fn get_turn(turn_id: &Id) -> Result<Option<TurnRecord>, String> {
    with_db(|db| SqliteProductionRepository::get_turn(db, turn_id).map_err(|e| e.to_string()))
}

pub fn get_active_turn(campaign_id: &Id) -> Result<Option<TurnRecord>, String> {
    with_db(|db| {
        SqliteProductionRepository::get_active_turn(db, campaign_id).map_err(|e| e.to_string())
    })
}

pub fn list_active_turns() -> Result<Vec<TurnRecord>, String> {
    with_db(|db| SqliteProductionRepository::list_active_turns(db).map_err(|e| e.to_string()))
}

pub fn list_instances(
    campaign_id: &Id,
) -> Result<Vec<storyforge_domain::campaign::CharacterInstance>, String> {
    with_db(|db| {
        SqliteProductionRepository::list_instances(db, campaign_id).map_err(|e| e.to_string())
    })
}

pub fn list_knowledge(
    campaign_id: &Id,
) -> Result<Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>, String> {
    with_db(|db| {
        SqliteProductionRepository::list_knowledge(db, campaign_id).map_err(|e| e.to_string())
    })
}

pub fn list_tasks(
    campaign_id: &Id,
) -> Result<Vec<storyforge_domain::story_task::StoryTask>, String> {
    with_db(|db| SqliteProductionRepository::list_tasks(db, campaign_id).map_err(|e| e.to_string()))
}

pub fn list_summaries(
    campaign_id: &Id,
) -> Result<Vec<storyforge_domain::agent::RoundSummary>, String> {
    with_db(|db| {
        SqliteProductionRepository::list_summaries(db, campaign_id).map_err(|e| e.to_string())
    })
}

pub fn get_card_payload(card_id: &Id) -> Result<Option<serde_json::Value>, String> {
    with_db(|db| {
        SqliteProductionRepository::get_card_payload(db, card_id).map_err(|e| e.to_string())
    })
}

pub fn save_turn(turn: &TurnRecord) -> Result<(), String> {
    with_db_mut(|db| SqliteProductionRepository::save_turn(db, turn).map_err(|e| e.to_string()))
}

/// Mutate a Turn under the same process-owned SQLite lock that guards all
/// other runtime operations. This replaces the JSON `TurnStore` read/modify/
/// write helpers in opt-in mode and prevents a silent fallback to JSON.
pub fn update_turn_record<F>(turn_id: &Id, f: F) -> Result<(), String>
where
    F: FnOnce(&mut TurnRecord),
{
    with_db_mut(|db| {
        let mut turn = SqliteProductionRepository::get_turn(db, turn_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("TurnRecord {turn_id} does not exist"))?;
        f(&mut turn);
        SqliteProductionRepository::save_turn(db, &turn).map_err(|e| e.to_string())
    })
}

pub fn mutate_turn_if<P, M>(turn_id: &Id, predicate: P, mutate: M) -> Result<bool, String>
where
    P: FnOnce(&TurnRecord) -> bool,
    M: FnOnce(&mut TurnRecord),
{
    with_db_mut(|db| {
        let mut turn = SqliteProductionRepository::get_turn(db, turn_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("TurnRecord {turn_id} does not exist"))?;
        if !predicate(&turn) {
            return Ok(false);
        }
        mutate(&mut turn);
        SqliteProductionRepository::save_turn(db, &turn).map_err(|e| e.to_string())?;
        Ok(true)
    })
}

pub fn save_conversation(conversation: &Conversation) -> Result<(), String> {
    with_db_mut(|db| {
        SqliteProductionRepository::save_conversation(db, conversation).map_err(|e| e.to_string())
    })
}

pub fn save_campaign(campaign: &Campaign) -> Result<(), String> {
    with_db_mut(|db| {
        SqliteProductionRepository::save_campaign(db, campaign).map_err(|e| e.to_string())
    })
}

/// Persist a card payload under the process-owned SQLite authority.
///
/// `payload` should be the Tauri `StoredCard` JSON shape (`{ card, imported_at }`)
/// so production context loaders can deserialize it unchanged.
pub fn save_card_payload(
    card_id: &Id,
    name: &str,
    source_character_id: Option<&str>,
    imported_at: Option<&str>,
    payload: &serde_json::Value,
) -> Result<(), String> {
    with_db_mut(|db| {
        SqliteProductionRepository::save_card_payload(
            db,
            card_id,
            name,
            source_character_id,
            imported_at,
            payload,
        )
        .map_err(|e| e.to_string())
    })
}

pub fn save_instance(
    instance: &storyforge_domain::campaign::CharacterInstance,
) -> Result<(), String> {
    with_db_mut(|db| {
        SqliteProductionRepository::save_instance(db, instance).map_err(|e| e.to_string())
    })
}

pub fn save_knowledge(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
) -> Result<(), String> {
    with_db_mut(|db| {
        SqliteProductionRepository::save_knowledge(db, entry).map_err(|e| e.to_string())
    })
}

pub fn save_task(task: &storyforge_domain::story_task::StoryTask) -> Result<(), String> {
    with_db_mut(|db| SqliteProductionRepository::save_task(db, task).map_err(|e| e.to_string()))
}

// ─── Gate 5: CRUD parity primitives（等价 JSON CampaignStore 语义）────────

/// JSON `CampaignStore::get_task` 等价：读取单条任务（缺失 → Ok(None)）。
pub fn get_task(task_id: &Id) -> Result<Option<StoryTask>, String> {
    with_db(|db| SqliteProductionRepository::get_task(db, task_id).map_err(|e| e.to_string()))
}

/// JSON `CampaignStore::update_task` 等价：仅当行存在时更新。
pub fn update_task(task: &StoryTask) -> Result<bool, String> {
    with_db_mut(|db| SqliteProductionRepository::update_task(db, task).map_err(|e| e.to_string()))
}

/// JSON `CampaignStore::delete_task` 等价：删除任务行，返回是否删除。
pub fn delete_task(task_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| {
        SqliteProductionRepository::delete_task(db, task_id).map_err(|e| e.to_string())
    })
}

/// JSON `CampaignStore::update_campaign` 等价：仅当行存在时更新
/// （缺失 → Ok(false)，不创建）。
pub fn update_campaign(campaign: &Campaign) -> Result<bool, String> {
    with_db_mut(|db| {
        SqliteProductionRepository::update_campaign(db, campaign).map_err(|e| e.to_string())
    })
}

/// JSON `CampaignStore::update_instance` 等价：仅当行存在时更新。
pub fn update_instance(instance: &CharacterInstance) -> Result<bool, String> {
    with_db_mut(|db| {
        SqliteProductionRepository::update_instance(db, instance).map_err(|e| e.to_string())
    })
}

/// JSON `CampaignStore::add_knowledge` 等价：单事务批量 upsert 知识条目。
pub fn add_knowledge_batch(entries: &[CharacterKnowledgeEntry]) -> Result<(), String> {
    with_db_mut(|db| {
        SqliteProductionRepository::add_knowledge_batch(db, entries).map_err(|e| e.to_string())
    })
}

/// JSON `CampaignStore::create_campaign_with_instances` 等价：从卡 payload 构建
/// Protagonist/Supporting 实例并单事务写入（清空既有实例 → Campaign → 实例）。
/// 返回 (StoredCard, Campaign, instance_count)，与 JSON 语义一致。
pub fn create_campaign_with_instances(
    campaign: &Campaign,
) -> Result<(crate::campaign_store::StoredCard, Campaign, usize), String> {
    use storyforge_domain::character::RoleType;
    with_db_mut(|db| {
        let card_payload = SqliteProductionRepository::get_card_payload(db, &campaign.card_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("card not found: {}", campaign.card_id))?;
        let stored: crate::campaign_store::StoredCard = serde_json::from_value(card_payload)
            .map_err(|e| format!("解析角色卡 payload 失败: {e}"))?;
        let mut instances: Vec<CharacterInstance> = Vec::new();
        let mut instance_count = 0usize;
        for def in &stored.card.character_definitions {
            if matches!(def.role_type, RoleType::Protagonist | RoleType::Supporting) {
                instances.push(CharacterInstance::from_definition(campaign.id.clone(), def));
                instance_count += 1;
            }
        }
        SqliteProductionRepository::create_campaign_with_instances(db, campaign, &instances)
            .map_err(|e| e.to_string())?;
        Ok((stored, campaign.clone(), instance_count))
    })
}

/// JSON `CampaignStore::delete_knowledge` 等价：删除知识条目行，返回是否删除。
pub fn delete_knowledge(knowledge_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| {
        SqliteProductionRepository::delete_knowledge(db, knowledge_id).map_err(|e| e.to_string())
    })
}

/// JSON `CampaignStore::delete_campaign` 等价：单事务级联删除一局活动
/// （含会话/任务/知识/实例/世界书/总结/台账），返回是否删除。
pub fn delete_campaign_cascade(campaign_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| {
        SqliteProductionRepository::delete_campaign_cascade(db, campaign_id)
            .map_err(|e| e.to_string())
    })
}

/// JSON `CampaignStore::save_card` / `save_card_if_no_campaigns` 等价：按
/// source_character_id 去重覆盖，事务内完成。被替换的卡（含新卡自身 id）若被
/// Campaign 引用则返回 `FORCE_RERUN_BLOCKED_BY_CAMPAIGN`——JSON `save_card`
/// 会静默移除旧卡留下孤儿引用，SQLite 的 `campaigns.card_id` FK 无法表达孤儿
/// 引用，统一 fail-closed 拒绝（差异已记录，不影响正常路径）。
pub fn save_card_with_dedupe(
    card: CharacterCard,
) -> Result<crate::campaign_store::StoredCard, String> {
    let imported_at = chrono::Utc::now().to_rfc3339();
    let stored = crate::campaign_store::StoredCard {
        card: card.clone(),
        imported_at: imported_at.clone(),
    };
    let payload = serde_json::to_value(&stored).map_err(|e| format!("序列化角色卡失败: {e}"))?;
    with_db_mut(|db| {
        let tx = db
            .connection_mut()
            .transaction()
            .map_err(|e| e.to_string())?;
        let replaced_ids: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT card_id FROM character_cards WHERE source_character_id = ?1")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([card.source_character_id.as_str()], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        if !replaced_ids.is_empty() {
            // 与 JSON `save_card_if_no_campaigns` 同口径：被替换的卡（含新卡
            // 自身 id）若被 Campaign 引用则拒绝覆盖，绝不静默孤儿化。
            let campaigns: i64 = {
                let placeholders = replaced_ids
                    .iter()
                    .map(|_| "?")
                    .collect::<Vec<_>>()
                    .join(",");
                let sql =
                    format!("SELECT COUNT(*) FROM campaigns WHERE card_id IN ({placeholders})");
                let params = rusqlite::params_from_iter(replaced_ids.iter());
                tx.query_row(&sql, params, |row| row.get::<_, i64>(0))
                    .map_err(|e| e.to_string())?
            };
            if campaigns > 0 {
                return Err(crate::campaign_store::FORCE_RERUN_BLOCKED_BY_CAMPAIGN.to_string());
            }
            for replaced_id in &replaced_ids {
                tx.execute(
                    "DELETE FROM character_cards WHERE card_id = ?1",
                    [replaced_id],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.execute(
            r#"
            INSERT INTO character_cards (card_id, source_character_id, name, imported_at, payload_json)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            rusqlite::params![
                card.id.as_str(),
                card.source_character_id.as_str(),
                card.name,
                imported_at,
                serde_json::to_string(&payload).map_err(|e| e.to_string())?,
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(stored)
    })
}

// ─── MVU 翻译缓存（V005；#22 SQLite 权威補齐）────────────────────────────

pub fn save_mvu(stored: &crate::campaign_store::StoredMvuTranslation) -> Result<(), String> {
    let payload = serde_json::to_value(stored).map_err(|e| format!("序列化 MVU 翻译失败: {e}"))?;
    with_db_mut(|db| {
        SqliteProductionRepository::save_mvu_payload(
            db,
            &stored.source_character_id,
            &stored.character_name,
            &payload,
        )
        .map_err(|e| e.to_string())
    })
}

pub fn get_mvu(
    source_character_id: &Id,
) -> Result<Option<crate::campaign_store::StoredMvuTranslation>, String> {
    let payload = with_db(|db| {
        SqliteProductionRepository::get_mvu_payload(db, source_character_id)
            .map_err(|e| e.to_string())
    })?;
    payload
        .map(|value| {
            serde_json::from_value(value).map_err(|e| format!("反序列化 MVU 翻译失败: {e}"))
        })
        .transpose()
}

pub fn list_mvu() -> Result<Vec<crate::campaign_store::StoredMvuTranslation>, String> {
    let payloads =
        with_db(|db| SqliteProductionRepository::list_mvu_payloads(db).map_err(|e| e.to_string()))?;
    payloads
        .into_iter()
        .map(|value| {
            serde_json::from_value(value).map_err(|e| format!("反序列化 MVU 翻译失败: {e}"))
        })
        .collect()
}

pub fn delete_mvu(source_character_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| {
        SqliteProductionRepository::delete_mvu_payload(db, source_character_id)
            .map_err(|e| e.to_string())
    })
}

/// definition_id → source_character_id 反查用：全部卡 payload（StoredCard JSON）。
pub fn list_card_payloads() -> Result<Vec<serde_json::Value>, String> {
    with_db(|db| SqliteProductionRepository::list_card_payloads(db).map_err(|e| e.to_string()))
}

pub fn capture_audit_snapshot() -> Result<storyforge_infra_sqlite::SqliteAuditSnapshot, String> {
    with_db_mut(|db| storyforge_infra_sqlite::capture_audit_snapshot(db).map_err(|e| e.to_string()))
}

/// Look up a card wrapper payload by its ST source character id (SQLite).
pub fn get_card_payload_by_source(
    source_character_id: &Id,
) -> Result<Option<serde_json::Value>, String> {
    with_db(|db| {
        SqliteProductionRepository::get_card_payload_by_source(db, source_character_id)
            .map_err(|e| e.to_string())
    })
}

/// One `card_id → name` snapshot from the SQLite card authority
/// (Gate 4: conversation card-name enrichment).
pub fn list_card_names() -> Result<std::collections::HashMap<Id, String>, String> {
    with_db(|db| {
        let mut stmt = db
            .connection()
            .prepare("SELECT card_id, name FROM character_cards")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                let card_id: String = row.get(0)?;
                let name: String = row.get(1)?;
                Ok((Id::from_str(&card_id), name))
            })
            .map_err(|e| e.to_string())?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (card_id, name) = row.map_err(|e| e.to_string())?;
            out.insert(card_id, name);
        }
        Ok(out)
    })
}

// ─── Gate 4: MVU schema apply (atomic single-transaction) ─────────────────

/// Test-only fault injection for the MVU schema apply UoW (rollback proof).
#[doc(hidden)]
#[cfg_attr(not(test), allow(dead_code))]
pub fn fail_mvu_apply_uow_for_test(fault: crate::sqlite_mvu_repo::MvuApplyFault) {
    FAIL_MVU_APPLY_UOW.with(|slot| slot.set(fault));
}

thread_local! {
    static FAIL_MVU_APPLY_UOW: std::cell::Cell<crate::sqlite_mvu_repo::MvuApplyFault> =
        const { std::cell::Cell::new(crate::sqlite_mvu_repo::MvuApplyFault::None) };
}

/// Apply the MVU schema for one source card + definition in a single SQLite
/// transaction (card payload update + cross-campaign instance default backfill).
pub fn mvu_apply_schema(
    source_character_id: &Id,
    definition_id: &Id,
) -> Result<crate::sqlite_mvu_repo::MvuApplySummary, String> {
    with_db_mut(|db| {
        let fault = FAIL_MVU_APPLY_UOW.with(|slot| slot.get());
        crate::sqlite_mvu_repo::SqliteMvuRepository::apply_schema_with_fault(
            db,
            source_character_id,
            definition_id,
            fault,
        )
        .map_err(|e| e.to_string())
    })
}

// ─── Gate 4: campaign world info (V006 campaign_world_info table) ──────────

/// Load the campaign world info book from the SQLite authority.
/// A missing row returns `Ok(None)` — callers decide whether to seed from the
/// card template (the JSON store treats a missing file as an empty book).
pub fn get_world_info(
    campaign_id: &Id,
) -> Result<Option<storyforge_domain::world_info::WorldInfoBook>, String> {
    with_db(|db| {
        let payload = SqliteProductionRepository::get_world_info_payload(db, campaign_id)
            .map_err(|e| e.to_string())?;
        payload
            .map(|value| {
                serde_json::from_value(value).map_err(|e| format!("反序列化世界书失败: {e}"))
            })
            .transpose()
    })
}

/// Replace the whole campaign world info book (idempotent upsert).
pub fn set_world_info(
    campaign_id: &Id,
    book: &storyforge_domain::world_info::WorldInfoBook,
) -> Result<(), String> {
    let payload = serde_json::to_value(book).map_err(|e| format!("序列化世界书失败: {e}"))?;
    with_db_mut(|db| {
        SqliteProductionRepository::save_world_info_payload(db, campaign_id, &payload)
            .map_err(|e| e.to_string())
    })
}

/// Read-modify-write a campaign world info book under the process-owned SQLite
/// lock. Mirrors the JSON `CampaignStore::mutate_world_info` semantics and
/// returns the closure's result alongside the persisted book.
pub fn mutate_world_info<T>(
    campaign_id: &Id,
    f: impl FnOnce(&mut storyforge_domain::world_info::WorldInfoBook) -> Result<T, String>,
) -> Result<T, String> {
    with_db_mut(|db| {
        let mut book: storyforge_domain::world_info::WorldInfoBook =
            match SqliteProductionRepository::get_world_info_payload(db, campaign_id)
                .map_err(|e| e.to_string())?
            {
                Some(value) => {
                    serde_json::from_value(value).map_err(|e| format!("反序列化世界书失败: {e}"))?
                }
                None => storyforge_domain::world_info::WorldInfoBook {
                    entries: Vec::new(),
                    source: storyforge_domain::Source::Native,
                    metadata: Default::default(),
                },
            };
        let result = f(&mut book)?;
        let payload = serde_json::to_value(&book).map_err(|e| format!("序列化世界书失败: {e}"))?;
        SqliteProductionRepository::save_world_info_payload(db, campaign_id, &payload)
            .map_err(|e| e.to_string())?;
        Ok(result)
    })
}

/// Seed the campaign world info from a card template when the campaign book is
/// still empty (lazy migration, same semantics as the JSON store).
pub fn ensure_world_info_from_book(
    campaign_id: &Id,
    template: &storyforge_domain::world_info::WorldInfoBook,
) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
    let book = mutate_world_info(campaign_id, |existing| {
        if existing.entries.is_empty() {
            *existing = template.clone();
        }
        Ok(existing.clone())
    })?;
    Ok(book)
}

/// Delete the campaign world info row (campaign teardown).
pub fn delete_world_info(campaign_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| {
        SqliteProductionRepository::delete_world_info_payload(db, campaign_id)
            .map_err(|e| e.to_string())
    })
}

fn reconfirm_accept_replay(
    attempt: &storyforge_domain::turn::TurnAttempt,
    outcome: crate::turn_lifecycle::AcceptOutcome,
) -> Result<crate::turn_lifecycle::AcceptOutcome, crate::turn_lifecycle::AcceptError> {
    use crate::turn_lifecycle::AcceptError;

    let batch = outcome.batch.clone();
    let terminal_status = outcome.turn_status.clone();
    let ledger_outcome = with_db_mut(|db| {
        let request = AcceptTurnRequest {
            turn_id: &outcome.turn_id,
            attempt_id: &outcome.attempt_id,
            draft_hash: &attempt.draft_hash,
            batch: &batch,
            terminal_status: terminal_status.clone(),
        };
        SqliteProductionRepository::accept_turn(db, request).map_err(|e| e.to_string())
    })
    .map_err(AcceptError::Commit)?;
    debug_assert!(matches!(
        ledger_outcome,
        SqliteAcceptOutcome::AlreadyCommitted
    ));
    Ok(outcome)
}

/// Atomic Accept through the SQLite production UoW.
///
/// The pure decision (scope, derivation, quality, draft-hash, revision, batch)
/// is delegated to [`crate::turn_lifecycle::evaluate_accept_decision`], shared
/// with the JSON path. This function owns only the SQLite store reads and the
/// atomic UoW persistence.
pub fn accept_by_variant(
    campaign_id: &Id,
    conversation_id: &Id,
    variant_id: &Id,
    force_accept: bool,
) -> Result<crate::turn_lifecycle::AcceptOutcome, crate::turn_lifecycle::AcceptError> {
    use crate::turn_lifecycle::{
        AcceptDecision, AcceptDecisionInput, AcceptError, AcceptOutcome, ensure_accept_scope,
        evaluate_accept_decision, evaluate_accept_replay,
    };

    let turn = get_turn_by_variant(variant_id)
        .map_err(AcceptError::Storage)?
        .ok_or(AcceptError::NoTurnRecord)?;

    // Preserve the permanent-guard order used by the JSON adapter. In
    // particular, a foreign requested conversation must remain a typed scope
    // mismatch even when that id does not exist in SQLite.
    ensure_accept_scope(campaign_id, conversation_id, &turn)?;

    let attempt = turn
        .attempts
        .iter()
        .rev()
        .find(|a| a.variant_id == *variant_id)
        .cloned()
        .ok_or(AcceptError::NoAttempt)?;

    // Terminal retries only need the persisted Attempt batch and SQLite
    // ledger. Do not require live conversation/campaign reads to replay them.
    if let Some(outcome) = evaluate_accept_replay(&turn, &attempt)? {
        return reconfirm_accept_replay(&attempt, outcome);
    }

    let conversation = get_conversation(conversation_id)
        .map_err(AcceptError::Storage)?
        .ok_or_else(|| AcceptError::Storage(format!("conversation {conversation_id} missing")))?;
    let current_text = conversation
        .nodes
        .iter()
        .find(|n| n.id == *variant_id)
        .and_then(|n| n.active())
        .map(|v| v.content.clone())
        .unwrap_or_default();

    let camp = get_campaign(&turn.campaign_id)
        .map_err(AcceptError::Storage)?
        .ok_or(AcceptError::CampaignMissing)?;
    let campaign_revision_before = camp.revision;

    // Shared backend-agnostic decision prologue (Gate 2).
    let decision = evaluate_accept_decision(&AcceptDecisionInput {
        campaign_id,
        conversation_id,
        variant_id,
        turn: &turn,
        attempt: &attempt,
        force_accept,
        current_draft_text: current_text,
        current_campaign_revision: campaign_revision_before,
    })?;

    match decision {
        AcceptDecision::Replay(outcome) => {
            // Re-confirm durability against the SQLite mutation ledger. A retry
            // after the UoW committed must reach `accept_turn`, which validates
            // the persisted batch and returns AlreadyCommitted.
            reconfirm_accept_replay(&attempt, outcome)
        }
        AcceptDecision::Commit {
            batch,
            terminal_status,
            commit_as_degraded,
            campaign_revision_before,
        } => {
            let turn_id = turn.turn_id.clone();
            let attempt_id = attempt.attempt_id.clone();
            let draft_hash = attempt.draft_hash.clone();

            // `SqliteProductionRepository` intentionally verifies that the exact
            // candidate batch was durable before it finalizes. Persist the batch
            // generated by the app service first; otherwise an empty postprocess
            // candidate (which still needs FinalizeVariant) is rejected and force
            // Accept can never close a normal draft.
            with_db_mut(|db| {
                let mut current = SqliteProductionRepository::get_turn(db, &turn_id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| {
                        format!("turn {turn_id} disappeared before accept preparation")
                    })?;
                if current.status != TurnStatus::AwaitingAcceptance {
                    return Err(format!(
                        "turn {turn_id} changed to {:?} before accept preparation",
                        current.status
                    ));
                }
                let stored_attempt = current.find_attempt_mut(&attempt_id).ok_or_else(|| {
                    format!("attempt {attempt_id} disappeared before accept preparation")
                })?;
                if stored_attempt.status != AttemptStatus::AwaitingAcceptance
                    || stored_attempt.draft_hash != draft_hash
                {
                    return Err(format!(
                        "attempt {attempt_id} changed before accept preparation"
                    ));
                }
                stored_attempt.pending_state_changes = Some(batch.clone());
                current.touch();
                SqliteProductionRepository::save_turn(db, &current).map_err(|e| e.to_string())
            })
            .map_err(AcceptError::Storage)?;

            // V7：typed 错误跨边界分类。旧实现 `e.contains("revision")` 会把
            // integrity 分歧（消息同样含 "revisions"）等 DB 损坏误报成"回合过期"。
            let outcome = with_db_mut(|db| {
                let request = AcceptTurnRequest {
                    turn_id: &turn_id,
                    attempt_id: &attempt_id,
                    draft_hash: &draft_hash,
                    batch: &batch,
                    terminal_status: terminal_status.clone(),
                };
                Ok(SqliteProductionRepository::accept_turn(db, request))
            })
            .map_err(AcceptError::Commit)?
            .map_err(|e| match e {
                storyforge_infra_sqlite::SqliteError::RevisionConflict {
                    campaign,
                    turn_base,
                    ..
                } => AcceptError::RevisionConflict {
                    base: turn_base,
                    current: campaign,
                },
                storyforge_infra_sqlite::SqliteError::RecordNotFound(_) => {
                    AcceptError::Storage(e.to_string())
                }
                other => AcceptError::Commit(other.to_string()),
            })?;

            // UoW 内写入的真值：`campaign.revision = batch.target_revision`
            // （UoW 校验 target == expected + 1 后才提交）。直接从已验证的
            // batch 取，避免提交后的二次读取——提交成功却因读失败误报错误，
            // 会让调用方跳过 conversation invalidate 与后续动作。
            let campaign_revision_after = batch.target_revision;

            let _ = outcome; // Applied | AlreadyCommitted — both OK for the caller.
            let _ = matches!(outcome, SqliteAcceptOutcome::AlreadyCommitted);

            Ok(AcceptOutcome {
                turn_id,
                attempt_id,
                turn_status: terminal_status,
                attempt_status: AttemptStatus::Committed,
                commit_as_degraded,
                campaign_revision_before,
                campaign_revision_after,
                batch,
            })
        }
    }
}

/// SQLite startup recovery: fail incomplete pre-accept turns (outbox-aware),
/// then fail any remaining Committing turns via production recovery.
pub fn recover_turns_on_startup() -> Result<usize, String> {
    with_db_mut(|db| {
        let preaccept =
            SqlitePreacceptRepository::fail_incomplete_preaccept(db).map_err(|e| e.to_string())?;
        let production =
            SqliteProductionRepository::fail_incomplete_turns(db).map_err(|e| e.to_string())?;
        Ok(preaccept + production)
    })
}

// ─── Pre-accept lifecycle gateway (shared by Tauri + harness) ──────────────

/// Test-only fault injection: make the first-draft preaccept UoW fail after
/// its mutations (rollback path) so callers exercise the real failure
/// write-back semantics. Exposed to integration tests via thread-local flag.
#[doc(hidden)]
#[cfg_attr(not(test), allow(dead_code))]
pub fn fail_draft_uow_for_test(fail: bool) {
    FAIL_DRAFT_UOW.with(|flag| flag.set(fail));
}

thread_local! {
    static FAIL_DRAFT_UOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Atomic first-draft land: conversation AI node + Turn/Attempt DraftReady + outbox.
pub fn create_draft_attempt(
    request: DraftAttemptRequest<'_>,
) -> Result<DraftAttemptOutcome, String> {
    with_db_mut(|db| {
        if FAIL_DRAFT_UOW.with(|flag| flag.get()) {
            SqlitePreacceptRepository::create_draft_attempt_with_fault(
                db,
                request,
                storyforge_infra_sqlite::preaccept::PreacceptFault::BeforeCommit,
            )
            .map_err(|e| e.to_string())
        } else {
            SqlitePreacceptRepository::create_draft_attempt(db, request).map_err(|e| e.to_string())
        }
    })
}

/// Atomic autofix land: conversation content + attempt draft_hash/quality_report + outbox.
pub fn sync_autofix(request: AutofixSyncRequest<'_>) -> Result<(), String> {
    with_db_mut(|db| {
        SqlitePreacceptRepository::sync_autofix(db, request).map_err(|e| e.to_string())
    })
}

/// Atomic postprocess land: MutationBatch/derivation/AwaitingAcceptance + outbox.
pub fn apply_postprocess(
    request: PostprocessApplyRequest<'_>,
) -> Result<PostprocessApplyOutcome, String> {
    with_db_mut(|db| {
        SqlitePreacceptRepository::apply_postprocess(db, request).map_err(|e| e.to_string())
    })
}

/// Atomic regenerate land: supersede old attempt, new draft variant, new Attempt, outbox.
pub fn append_regenerate_attempt(
    request: RegenerateAttemptRequest<'_>,
) -> Result<DraftAttemptOutcome, String> {
    with_db_mut(|db| {
        SqlitePreacceptRepository::append_regenerate_attempt(db, request).map_err(|e| e.to_string())
    })
}

/// Atomic edit-stale: conversation content + Attempt=Stale + outbox.
pub fn mark_stale_after_edit(
    campaign_id: &Id,
    conversation_id: &Id,
    turn_id: &Id,
    attempt_id: &Id,
    new_content: &str,
) -> Result<(), String> {
    with_db_mut(|db| {
        SqlitePreacceptRepository::mark_stale_after_edit(
            db,
            campaign_id,
            conversation_id,
            turn_id,
            attempt_id,
            new_content,
        )
        .map_err(|e| e.to_string())
    })
}

pub fn list_preaccept_outbox_for_turn(turn_id: &Id) -> Result<Vec<PreacceptOutboxRow>, String> {
    with_db(|db| {
        SqlitePreacceptRepository::list_outbox_for_turn(db, turn_id).map_err(|e| e.to_string())
    })
}

pub fn recover_active_preaccept_state(
    campaign_id: &Id,
) -> Result<PreacceptRecoverySnapshot, String> {
    with_db(|db| {
        SqlitePreacceptRepository::recover_active_state(db, campaign_id).map_err(|e| e.to_string())
    })
}

// ─── Gate 4: Chronicle compressor jobs (V006 table) ───────────────────────

pub fn compress_enqueue_or_get_open(
    campaign_id: &Id,
    conversation_id: Option<Id>,
    lineage_id: Option<Id>,
    uncovered_a: u32,
    uncovered_b: u32,
) -> Result<(crate::sqlite_compress_jobs::SqliteCompressJob, bool), String> {
    with_db_mut(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::enqueue_or_get_open(
            db,
            campaign_id,
            conversation_id,
            lineage_id,
            uncovered_a,
            uncovered_b,
        )
        .map_err(|e| e.to_string())
    })
}

pub fn compress_try_claim_pending(job_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::try_claim_pending(db, job_id)
            .map_err(|e| e.to_string())
    })
}

/// Finalize a job only while it is still Running (late/concurrent worker safe).
pub fn compress_mark_succeeded(job_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::mark_succeeded(db, job_id)
            .map_err(|e| e.to_string())
    })
}

pub fn compress_mark_failed_or_retry(job_id: &Id, err: &str) -> Result<bool, String> {
    with_db_mut(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::mark_failed_or_retry(
            db, job_id, err,
        )
        .map_err(|e| e.to_string())
    })
}

pub fn compress_reset_running_to_pending() -> Result<usize, String> {
    with_db_mut(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::reset_running_to_pending(db)
            .map_err(|e| e.to_string())
    })
}

pub fn compress_list_open() -> Result<Vec<crate::sqlite_compress_jobs::SqliteCompressJob>, String> {
    with_db(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::list_open(db)
            .map_err(|e| e.to_string())
    })
}

pub fn compress_list_all() -> Result<Vec<crate::sqlite_compress_jobs::SqliteCompressJob>, String> {
    with_db(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::list_all(db)
            .map_err(|e| e.to_string())
    })
}

pub fn compress_count_uncovered(campaign_id: &Id) -> Result<(usize, usize), String> {
    with_db(|db| {
        crate::sqlite_compress_jobs::SqliteCompressJobRepository::count_uncovered(db, campaign_id)
            .map_err(|e| e.to_string())
    })
}

/// Publish a compress batch through the typed Chronicle publication UoW
/// (atomic parents + covered_by + revision bump + job ledger).
pub fn publish_chronicle_compress(
    campaign_id: &Id,
    publication_id: &Id,
    parents: &[storyforge_domain::agent::RoundSummary],
    child_covered_by: &[(Id, Id)],
    job_id: Option<&str>,
    batch_index: u32,
) -> Result<storyforge_infra_sqlite::publication::PublishOutcome, String> {
    with_db_mut(|db| {
        let request = storyforge_infra_sqlite::publication::PublishRequest {
            campaign_id,
            publication_id,
            parents,
            child_covered_by,
            job_id,
            batch_index,
        };
        storyforge_infra_sqlite::publication::SqliteChronicleRepository::publish_compress(
            db, request,
        )
        .map_err(|e| e.to_string())
    })
}

/// Seed a leaf/stage summary row directly (test/bootstrap helper).
pub fn seed_summary(summary: &storyforge_domain::agent::RoundSummary) -> Result<(), String> {
    with_db_mut(|db| {
        storyforge_infra_sqlite::publication::SqliteChronicleRepository::seed_summary(db, summary)
            .map_err(|e| e.to_string())
    })
}

/// Test-only fault injection for the publication UoW (rollback proof).
#[doc(hidden)]
#[cfg_attr(not(test), allow(dead_code))]
pub fn fail_chronicle_publish_for_test(fault: storyforge_infra_sqlite::publication::PublishFault) {
    FAIL_CHRONICLE_PUBLISH.with(|slot| slot.set(fault));
}

thread_local! {
    static FAIL_CHRONICLE_PUBLISH: std::cell::Cell<storyforge_infra_sqlite::publication::PublishFault> =
        const { std::cell::Cell::new(storyforge_infra_sqlite::publication::PublishFault::None) };
}

/// Publish with the test fault flag applied (used by the worker when set).
pub fn publish_chronicle_compress_with_fault_flag(
    campaign_id: &Id,
    publication_id: &Id,
    parents: &[storyforge_domain::agent::RoundSummary],
    child_covered_by: &[(Id, Id)],
    job_id: Option<&str>,
    batch_index: u32,
) -> Result<storyforge_infra_sqlite::publication::PublishOutcome, String> {
    with_db_mut(|db| {
        let fault = FAIL_CHRONICLE_PUBLISH.with(|slot| slot.get());
        let request = storyforge_infra_sqlite::publication::PublishRequest {
            campaign_id,
            publication_id,
            parents,
            child_covered_by,
            job_id,
            batch_index,
        };
        storyforge_infra_sqlite::publication::SqliteChronicleRepository::publish_compress_with_fault(
            db, request, fault,
        )
        .map_err(|e| e.to_string())
    })
}

// ─── Gate 4: typed Meta patch atomic apply ─────────────────────────────────

/// Test-only fault injection for the typed Meta patch UoW (rollback proof).
#[doc(hidden)]
#[cfg_attr(not(test), allow(dead_code))]
pub fn fail_meta_patch_uow_for_test(fault: crate::sqlite_meta_repo::MetaPatchFault) {
    FAIL_META_PATCH_UOW.with(|slot| slot.set(fault));
}

thread_local! {
    static FAIL_META_PATCH_UOW: std::cell::Cell<crate::sqlite_meta_repo::MetaPatchFault> =
        const { std::cell::Cell::new(crate::sqlite_meta_repo::MetaPatchFault::None) };
}

/// Apply every action of one typed Meta patch in a single SQLite transaction.
pub fn meta_apply_typed_patch_actions(
    campaign_id: &Id,
    actions: &[storyforge_app_meta::TypedPatchAction],
    expected_revision: Option<u64>,
) -> Result<(), String> {
    with_db_mut(|db| {
        let fault = FAIL_META_PATCH_UOW.with(|slot| slot.get());
        crate::sqlite_meta_repo::SqliteMetaRepository::apply_typed_patch_actions_with_fault(
            db,
            campaign_id,
            actions,
            expected_revision,
            fault,
        )
        .map_err(|e| e.to_string())
    })
}

// ─── Gate 4 P1-4: SQLite character library (V007 `characters` table) ────────
//
// Mirrors the JSON `CharacterStore` (`characters.json`) semantics exactly:
// save generates a fresh stored id and stamps `imported_at`; list returns all
// stored characters; get/delete resolve by stored id **or** source
// `Character.id`; world-info edits are read-modify-write over the full
// `CharacterInfo` JSON with derived counters refreshed on every write.

fn character_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredCharacter> {
    let character_id: String = row.get(0)?;
    let source_character_id: Option<String> = row.get(1)?;
    let _name: String = row.get(2)?;
    let info_json: String = row.get(3)?;
    let imported_at: String = row.get(4)?;
    let info: CharacterInfo = serde_json::from_str(&info_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            format!("角色卡 info_json 解析失败: {e}").into(),
        )
    })?;
    if info.source_character_id.as_deref() != source_character_id.as_deref() {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            "角色卡 source_character_id 与 info_json 不一致"
                .to_string()
                .into(),
        ));
    }
    Ok(StoredCharacter {
        id: character_id,
        info,
        imported_at,
    })
}

const CHARACTER_SELECT: &str =
    "SELECT character_id, source_character_id, name, info_json, imported_at FROM characters";

fn character_select_by_id_or_source(
    db: &Database,
    id_or_source: &str,
) -> Result<Option<StoredCharacter>, String> {
    let mut stmt = db
        .connection()
        .prepare(
            "SELECT character_id, source_character_id, name, info_json, imported_at \
             FROM characters WHERE character_id = ?1 OR source_character_id = ?1 LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map([id_or_source], character_row)
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(row) => row.map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Persist an imported character card (fresh stored id, `imported_at` stamp).
pub fn save_character(info: &CharacterInfo) -> Result<StoredCharacter, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let imported_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let stored = StoredCharacter {
        id: id.clone(),
        info: info.clone(),
        imported_at: imported_at.clone(),
    };
    let info_json = serde_json::to_string(info).map_err(|e| format!("序列化角色卡失败: {e}"))?;
    with_db_mut(|db| {
        db.connection()
            .execute(
                "INSERT INTO characters (character_id, source_character_id, name, info_json, imported_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, info.source_character_id, info.name, info_json, imported_at],
            )
            .map_err(|e| format!("保存角色卡失败: {e}"))?;
        Ok(())
    })?;
    Ok(stored)
}

/// List every stored character (导入时间序，与 JSON 插入序语义对齐)。
pub fn list_characters() -> Result<Vec<StoredCharacter>, String> {
    with_db(|db| {
        let mut stmt = db
            .connection()
            .prepare(&format!(
                "{CHARACTER_SELECT} ORDER BY imported_at, character_id"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], character_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
}

/// Get a stored character by stored id **or** source `Character.id`.
pub fn get_character(id_or_source: &str) -> Result<Option<StoredCharacter>, String> {
    with_db(|db| character_select_by_id_or_source(db, id_or_source))
}

/// Delete a stored character by stored id **or** source `Character.id`.
/// Returns whether a row was actually removed.
pub fn delete_character(id_or_source: &str) -> Result<bool, String> {
    with_db_mut(|db| {
        let removed = db
            .connection()
            .execute(
                "DELETE FROM characters WHERE character_id = ?1 OR source_character_id = ?1",
                [id_or_source],
            )
            .map_err(|e| format!("删除角色卡失败: {e}"))?;
        Ok(removed > 0)
    })
}

/// Read-modify-write a stored character's `CharacterInfo` under the
/// process-owned SQLite lock. Mirrors the JSON `CharacterStore` world-info
/// edit semantics: derived counters are refreshed on every write and the
/// row's `name` / `source_character_id` columns stay in sync with the JSON.
pub fn mutate_character<T>(
    id_or_source: &str,
    f: impl FnOnce(&mut CharacterInfo) -> Result<T, String>,
) -> Result<T, String> {
    with_db_mut(|db| {
        let row: Option<(String, String)> = db
            .connection()
            .query_row(
                "SELECT character_id, info_json FROM characters \
                 WHERE character_id = ?1 OR source_character_id = ?1 LIMIT 1",
                [id_or_source],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (character_id, info_json) =
            row.ok_or_else(|| format!("角色卡不存在: {id_or_source}"))?;
        let mut info: CharacterInfo =
            serde_json::from_str(&info_json).map_err(|e| format!("解析角色卡失败: {e}"))?;
        let result = f(&mut info)?;
        // 同步更新计数（对齐 JSON CharacterStore 编辑路径）。
        info.world_info_count = info.world_info_entries.len();
        info.has_world_info = !info.world_info_entries.is_empty();
        let new_json =
            serde_json::to_string(&info).map_err(|e| format!("序列化角色卡失败: {e}"))?;
        db.connection()
            .execute(
                "UPDATE characters SET info_json = ?1, name = ?2, source_character_id = ?3 \
                 WHERE character_id = ?4",
                rusqlite::params![new_json, info.name, info.source_character_id, character_id],
            )
            .map_err(|e| format!("更新角色卡失败: {e}"))?;
        Ok(result)
    })
}

/// 原子批量替换多个角色的 world_info_entries（Gate 4 七审 P1）。
///
/// 无活动 Campaign 时 `meta_accept_patch` 需把 patch 后的全局世界书写回所有
/// 角色卡。旧实现逐角色 `mutate_character`（各自独立事务），第二个角色失败
/// 时第一个已永久更新——部分提交。本方法在**单个 UoW 事务**内更新全部角色，
/// 任一步失败整体回滚。
///
/// `entries`：`(id_or_source, 该角色替换后的 world_info_entries)` 对列表。
pub fn update_world_info_entries_bulk_multi(
    entries: &[(String, Vec<crate::WorldInfoEntryInfo>)],
) -> Result<(), String> {
    with_db_mut(|db| {
        let tx = db
            .connection_mut()
            .transaction()
            .map_err(|e| e.to_string())?;
        for (id_or_source, new_entries) in entries {
            let character_id: Option<String> = tx
                .query_row(
                    "SELECT character_id FROM characters \
                     WHERE character_id = ?1 OR source_character_id = ?1 LIMIT 1",
                    [id_or_source],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let character_id =
                character_id.ok_or_else(|| format!("角色卡不存在: {id_or_source}"))?;
            // 读取现有 info，替换 entries 后整体写回（与 mutate_character 同语义）。
            let info_json: String = tx
                .query_row(
                    "SELECT info_json FROM characters WHERE character_id = ?1",
                    [&character_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut info: CharacterInfo =
                serde_json::from_str(&info_json).map_err(|e| format!("解析角色卡失败: {e}"))?;
            crate::commands::characters::apply_update_world_info_entries_bulk(
                &mut info,
                new_entries.clone(),
            )
            .map_err(|e| e.to_string())?;
            info.world_info_count = info.world_info_entries.len();
            info.has_world_info = !info.world_info_entries.is_empty();
            let new_json =
                serde_json::to_string(&info).map_err(|e| format!("序列化角色卡失败: {e}"))?;
            tx.execute(
                "UPDATE characters SET info_json = ?1, name = ?2, source_character_id = ?3 \
                 WHERE character_id = ?4",
                rusqlite::params![new_json, info.name, info.source_character_id, character_id],
            )
            .map_err(|e| format!("更新角色卡失败: {e}"))?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Delete one card payload row (character delete cascade). Mirrors the JSON
/// `CampaignStore::delete_card` scope: the card's campaigns cascade into
/// instances / knowledge / tasks / summaries / world info / MVU rows.
/// Delete a card payload with its **full** campaign cascade in one transaction
/// (character delete). Dependency order respects `foreign_keys=ON`:
/// mutation_commits → chronicle jobs → preaccept_outbox → attempts → turns →
/// conversations → summaries/covers → tasks/knowledge/instances/world_info →
/// campaigns → mvu → character_cards. Any FK failure rolls everything back.
pub fn delete_card_payload(card_id: &Id) -> Result<bool, String> {
    with_db_mut(|db| delete_card_payload_inner(db, card_id))
}

fn delete_card_payload_inner(db: &mut Database, card_id: &Id) -> Result<bool, String> {
    let tx = db
        .connection_mut()
        .transaction()
        .map_err(|e| e.to_string())?;
    let source_ids: Vec<Option<String>> = {
        let mut stmt = tx
            .prepare("SELECT source_character_id FROM character_cards WHERE card_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([card_id.as_str()], |row| row.get::<_, Option<String>>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    delete_card_cascade_tx(&tx, card_id.as_str())?;
    for source_id in source_ids.iter().flatten() {
        tx.execute(
            "DELETE FROM mvu_translations WHERE source_character_id = ?1",
            [source_id],
        )
        .map_err(|e| e.to_string())?;
    }
    let removed = tx
        .execute(
            "DELETE FROM character_cards WHERE card_id = ?1",
            [card_id.as_str()],
        )
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(removed > 0)
}

/// Delete one character (by stored id **or** source id) with its entire
/// cascade (MVU translations + card/campaign + all dependent rows) in a
/// **single** SQLite transaction (Gate 4 三审 P1：真实玩过的 Campaign 的
/// FK 依赖全部清理，失败整体回滚，绝不报告成功却留下不一致数据）。
pub fn delete_character_full_cascade(id: &str, extra_source_ids: &[Id]) -> Result<bool, String> {
    with_db_mut(|db| delete_character_full_cascade_inner(db, id, extra_source_ids))
}

/// 测试 fault 注入点：删除级联的中间失败点（回滚证明）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteCascadeFault {
    None,
    /// 级联清理中途注入失败（发生在事务内、commit 前）。
    MidCascade,
}

thread_local! {
    static FAIL_DELETE_CASCADE: std::cell::Cell<DeleteCascadeFault> =
        const { std::cell::Cell::new(DeleteCascadeFault::None) };
}

#[doc(hidden)]
pub fn fail_delete_cascade_for_test(fault: DeleteCascadeFault) {
    FAIL_DELETE_CASCADE.with(|slot| slot.set(fault));
}

/// 测试专用原始写钩子（失败注入测试用）。
///
/// Gate 4 五审 P1：验证 set_active_campaign 的失败原子性——世界书读取或种子
/// 阶段故障注入后，命令返回错误但活跃指针不得改变。`execute` 让测试能直接
/// 对进程权威库写入（如破坏 campaign_world_info 的 payload 使反序列化失败），
/// 而无需在命令层新开任何 JSON/SQLite 分支。仅供 `#[cfg(test)]` 集成测试。
#[doc(hidden)]
pub fn with_db_raw_write<T>(
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    with_db_mut(|db| f(db.connection()))
}

fn delete_character_full_cascade_inner(
    db: &mut Database,
    id: &str,
    extra_source_ids: &[Id],
) -> Result<bool, String> {
    let tx = db
        .connection_mut()
        .transaction()
        .map_err(|e| e.to_string())?;
    let row: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT character_id, source_character_id FROM characters \
             WHERE character_id = ?1 OR source_character_id = ?1 LIMIT 1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((character_id, source_from_row)) = row else {
        return Ok(false);
    };
    // 候选 source id：库行自带 + 调用方（tool_ctx 域 id）提供。
    let mut candidates: Vec<String> = Vec::new();
    let mut push_source = |s: &str| {
        if !candidates.iter().any(|c| c == s) {
            candidates.push(s.to_string());
        }
    };
    if let Some(s) = source_from_row.as_deref() {
        push_source(s);
    }
    for id in extra_source_ids {
        push_source(id.as_str());
    }
    for source in &candidates {
        tx.execute(
            "DELETE FROM mvu_translations WHERE source_character_id = ?1",
            [source],
        )
        .map_err(|e| e.to_string())?;
        let card_id: Option<String> = tx
            .query_row(
                "SELECT card_id FROM character_cards WHERE source_character_id = ?1 LIMIT 1",
                [source],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(card_id) = card_id {
            delete_card_cascade_tx(&tx, &card_id)?;
            if FAIL_DELETE_CASCADE.with(|slot| slot.get()) == DeleteCascadeFault::MidCascade {
                return Err("injected failure mid delete cascade".into());
            }
            tx.execute("DELETE FROM character_cards WHERE card_id = ?1", [&card_id])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.execute(
        "DELETE FROM characters WHERE character_id = ?1",
        [&character_id],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(true)
}

/// Full dependency-ordered campaign cascade inside an open transaction.
/// `foreign_keys=ON` makes order mandatory: children before their parents.
fn delete_card_cascade_tx(tx: &rusqlite::Transaction<'_>, card_id: &str) -> Result<(), String> {
    let campaign_ids: Vec<String> = {
        let mut stmt = tx
            .prepare("SELECT campaign_id FROM campaigns WHERE card_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([card_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    for campaign_id in &campaign_ids {
        // mutation_commits → publication/compress jobs → preaccept_outbox
        tx.execute(
            "DELETE FROM mutation_commits WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM chronicle_publication_jobs WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM chronicle_compress_jobs WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM preaccept_outbox WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        // turn_attempts → turns → conversations
        tx.execute(
            "DELETE FROM turn_attempts WHERE turn_id IN \
             (SELECT turn_id FROM turns WHERE campaign_id = ?1)",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM turns WHERE campaign_id = ?1", [campaign_id])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM conversations WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        // chronicle summaries / covers / tasks / knowledge / instances / world info
        tx.execute(
            "DELETE FROM round_summary_covers WHERE parent_id IN \
             (SELECT summary_id FROM round_summaries WHERE campaign_id = ?1) \
             OR child_id IN (SELECT summary_id FROM round_summaries WHERE campaign_id = ?1)",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM round_summaries WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM story_tasks WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM character_knowledge WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM character_instances WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM campaign_world_info WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM campaigns WHERE campaign_id = ?1",
            [campaign_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── Gate 4 P1-4: atomic Campaign Bundle import (single transaction) ────────

/// Test-only fault injection for the bundle import UoW (rollback proof).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleImportFault {
    None,
    AfterCard,
    AfterCampaign,
    AfterSummaries,
}

/// Test-only fault injection setter for the bundle import UoW.
#[doc(hidden)]
#[cfg_attr(not(test), allow(dead_code))]
pub fn fail_bundle_import_for_test(fault: BundleImportFault) {
    FAIL_BUNDLE_IMPORT.with(|slot| slot.set(fault));
}

thread_local! {
    static FAIL_BUNDLE_IMPORT: std::cell::Cell<BundleImportFault> =
        const { std::cell::Cell::new(BundleImportFault::None) };
}

/// Persist one full Campaign bundle (conversation + card + campaign +
/// instances + knowledge + tasks + summaries/covers) in a **single** SQLite
/// transaction. Any failure rolls the whole import back — no partial graph
/// can survive. Insert order respects `foreign_keys=ON`: card → conversation
/// → campaign → children → summaries (parents after leaves, then covered_by /
/// covers patches).
pub fn import_campaign_bundle_into_db(
    conversation: &Conversation,
    card: &CharacterCard,
    campaign: &Campaign,
    instances: &[CharacterInstance],
    knowledge: &[CharacterKnowledgeEntry],
    tasks: &[StoryTask],
    summaries: &[RoundSummary],
) -> Result<(), String> {
    with_db_mut(|db| {
        let tx = db
            .connection_mut()
            .transaction()
            .map_err(|e| e.to_string())?;
        let fault = FAIL_BUNDLE_IMPORT.with(|slot| slot.get());

        // 1. 角色卡（campaign 的 FK 依赖）。payload 必须是 StoredCard 包装
        //    （`{ card, imported_at }`），与 `save_card_payload` / 生产读取
        //    消费者（meta 健康检查、世界书模板等）的约定一致。
        let imported_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let card_payload = serde_json::to_value(crate::campaign_store::StoredCard {
            card: card.clone(),
            imported_at: imported_at.clone(),
        })
        .map_err(|e| format!("序列化角色卡失败: {e}"))?;
        tx.execute(
            r#"
            INSERT INTO character_cards (card_id, source_character_id, name, imported_at, payload_json)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            rusqlite::params![
                card.id.as_str(),
                Some(card.source_character_id.as_str()),
                card.name,
                imported_at,
                serde_json::to_string(&card_payload).map_err(|e| format!("序列化角色卡失败: {e}"))?,
            ],
        )
        .map_err(|e| format!("导入角色卡失败: {e}"))?;
        if fault == BundleImportFault::AfterCard {
            return Err("injected failure after bundle card import".to_string());
        }

        // 2. 对话（round_summaries 的 FK 依赖）。
        tx.execute(
            r#"
            INSERT INTO conversations (
                conversation_id, campaign_id, character_id, archived_upto,
                created_at, updated_at, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            rusqlite::params![
                conversation.id.as_str(),
                conversation.campaign_id.as_ref().map(Id::as_str),
                conversation.character_id,
                conversation.archived_upto as u64,
                conversation.created_at.to_rfc3339(),
                conversation.updated_at.to_rfc3339(),
                serde_json::to_string(conversation).map_err(|e| format!("序列化对话失败: {e}"))?,
            ],
        )
        .map_err(|e| format!("导入对话失败: {e}"))?;

        // 3. Campaign。
        tx.execute(
            r#"
            INSERT INTO campaigns (
                campaign_id, card_id, name, conversation_id, revision, chronicle_revision,
                lineage_id, story_clock, created_at, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
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
                serde_json::to_string(campaign)
                    .map_err(|e| format!("序列化 Campaign 失败: {e}"))?,
            ],
        )
        .map_err(|e| format!("导入 Campaign 失败: {e}"))?;
        if fault == BundleImportFault::AfterCampaign {
            return Err("injected failure after bundle campaign import".to_string());
        }

        // 4. 实例 / 知识 / 任务。
        for instance in instances {
            tx.execute(
                r#"
                INSERT INTO character_instances (
                    instance_id, campaign_id, definition_id, name, is_temporary, payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                rusqlite::params![
                    instance.id.as_str(),
                    instance.campaign_id.as_str(),
                    instance.definition_id.as_ref().map(Id::as_str),
                    instance.name,
                    instance.is_temporary,
                    serde_json::to_string(instance)
                        .map_err(|e| format!("序列化角色实例失败: {e}"))?,
                ],
            )
            .map_err(|e| format!("导入角色实例失败: {e}"))?;
        }
        for entry in knowledge {
            tx.execute(
                "INSERT INTO character_knowledge (knowledge_id, campaign_id, payload_json) \
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    entry.id.as_str(),
                    entry.campaign_id.as_str(),
                    serde_json::to_string(entry).map_err(|e| format!("序列化知识失败: {e}"))?,
                ],
            )
            .map_err(|e| format!("导入知识失败: {e}"))?;
        }
        for task in tasks {
            tx.execute(
                "INSERT INTO story_tasks (task_id, campaign_id, payload_json) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    task.id.as_str(),
                    task.campaign_id.as_str(),
                    serde_json::to_string(task).map_err(|e| format!("序列化任务失败: {e}"))?,
                ],
            )
            .map_err(|e| format!("导入任务失败: {e}"))?;
        }

        // 5. 摘要：先全部落行（covered_by 置空），再回填 covered_by + covers
        //    （父行可能在子行之后，foreign_keys=ON 下必须两趟）。
        for summary in summaries {
            tx.execute(
                r#"
                INSERT INTO round_summaries (
                    summary_id, campaign_id, conversation_id, lineage_id, level, turn, turn_end,
                    code, headline, covered_by, content, created_at, payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12)
                "#,
                rusqlite::params![
                    summary.id.as_str(),
                    summary.campaign_id.as_str(),
                    summary.conversation_id.as_str(),
                    summary.lineage_id.as_ref().map(Id::as_str),
                    summary.level,
                    summary.turn,
                    summary.effective_turn_end(),
                    summary.code,
                    summary.headline,
                    summary.content,
                    summary.created_at,
                    serde_json::to_string(summary).map_err(|e| format!("序列化摘要失败: {e}"))?,
                ],
            )
            .map_err(|e| format!("导入摘要失败: {e}"))?;
        }
        for summary in summaries {
            if let Some(parent) = &summary.covered_by {
                tx.execute(
                    "UPDATE round_summaries SET covered_by = ?1 WHERE summary_id = ?2",
                    rusqlite::params![parent.as_str(), summary.id.as_str()],
                )
                .map_err(|e| format!("导入摘要 covered_by 失败: {e}"))?;
            }
            for child_id in &summary.covers {
                tx.execute(
                    "INSERT INTO round_summary_covers (parent_id, child_id) VALUES (?1, ?2)",
                    rusqlite::params![summary.id.as_str(), child_id.as_str()],
                )
                .map_err(|e| format!("导入摘要 covers 失败: {e}"))?;
            }
        }
        if fault == BundleImportFault::AfterSummaries {
            return Err("injected failure after bundle summaries import".to_string());
        }

        tx.commit().map_err(|e| format!("导入事务提交失败: {e}"))?;
        Ok(())
    })
}

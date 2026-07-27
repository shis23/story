//! Process-owned SQLite production storage boundary.
//!
//! When the backend selector chooses SQLite, this module owns the open
//! database handle and routes Accept / recovery / campaign / conversation /
//! turn operations through `SqliteProductionRepository`. JSON stores are not
//! consulted and no dual-write occurs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use storyforge_app_conversation::{ConversationError, ConversationPersistence};
use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::conversation::Conversation;
use storyforge_domain::turn::{AttemptStatus, TurnRecord, TurnStatus};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, DraftAttemptOutcome, DraftAttemptRequest, PostprocessApplyOutcome,
    PostprocessApplyRequest, PreacceptOutboxRow, PreacceptRecoverySnapshot,
    RegenerateAttemptRequest, SqlitePreacceptRepository,
};
use storyforge_infra_sqlite::production::{
    AcceptOutcome as SqliteAcceptOutcome, AcceptTurnRequest, SqliteProductionRepository,
    compute_draft_hash,
};
use storyforge_infra_sqlite::{current_version, migrate};

/// Process-owned SQLite handle. Opened once when SQLite is selected.
static SQLITE_DB: OnceLock<Arc<Mutex<Database>>> = OnceLock::new();
static SQLITE_PATH: OnceLock<PathBuf> = OnceLock::new();

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
    if SQLITE_DB.get().is_some() {
        return Ok(());
    }
    let path = db_path.as_ref().to_path_buf();
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

/// Atomic Accept through the SQLite production UoW.
pub fn accept_by_variant(
    campaign_id: &Id,
    conversation_id: &Id,
    variant_id: &Id,
    force_accept: bool,
) -> Result<crate::turn_lifecycle::AcceptOutcome, crate::turn_lifecycle::AcceptError> {
    use crate::turn_lifecycle::{AcceptError, AcceptOutcome, prepare_commit_batch};

    let turn = get_turn_by_variant(variant_id)
        .map_err(AcceptError::Storage)?
        .ok_or(AcceptError::NoTurnRecord)?;
    if &turn.campaign_id != campaign_id {
        return Err(AcceptError::CampaignScopeMismatch {
            turn_campaign: turn.campaign_id.to_string(),
            requested: campaign_id.to_string(),
        });
    }
    if &turn.conversation_id != conversation_id {
        return Err(AcceptError::ConversationScopeMismatch {
            turn_conversation: turn.conversation_id.to_string(),
            requested: conversation_id.to_string(),
        });
    }

    let attempt = turn
        .attempts
        .iter()
        .rev()
        .find(|a| a.variant_id == *variant_id)
        .cloned()
        .ok_or(AcceptError::NoAttempt)?;
    if attempt.status != AttemptStatus::AwaitingAcceptance
        && turn.status != TurnStatus::AwaitingAcceptance
    {
        // Allow already-committed replay through the ledger below.
        if !matches!(
            turn.status,
            TurnStatus::Committed | TurnStatus::Degraded | TurnStatus::Committing
        ) {
            return Err(AcceptError::InvalidAttemptStatus(format!(
                "{:?}",
                attempt.status
            )));
        }
    }

    // A retry after the SQLite UoW committed must reach the repository's
    // mutation ledger unchanged. Do not re-prepare the Attempt: terminal
    // records are no longer AwaitingAcceptance, but `accept_turn` can safely
    // validate the persisted batch and return AlreadyCommitted.
    if matches!(turn.status, TurnStatus::Committed | TurnStatus::Degraded) {
        let batch = attempt.pending_state_changes.clone().ok_or_else(|| {
            AcceptError::Storage(format!(
                "terminal turn {} has no persisted MutationBatch for replay",
                turn.turn_id
            ))
        })?;
        let terminal_status = turn.status.clone();
        let outcome = with_db_mut(|db| {
            let request = AcceptTurnRequest {
                turn_id: &turn.turn_id,
                attempt_id: &attempt.attempt_id,
                draft_hash: &attempt.draft_hash,
                batch: &batch,
                terminal_status: terminal_status.clone(),
            };
            SqliteProductionRepository::accept_turn(db, request).map_err(|e| e.to_string())
        })
        .map_err(AcceptError::Commit)?;
        debug_assert!(matches!(outcome, SqliteAcceptOutcome::AlreadyCommitted));

        return Ok(AcceptOutcome {
            turn_id: turn.turn_id,
            attempt_id: attempt.attempt_id,
            turn_status: terminal_status.clone(),
            attempt_status: AttemptStatus::Committed,
            commit_as_degraded: terminal_status == TurnStatus::Degraded,
            campaign_revision_before: batch.expected_revision,
            campaign_revision_after: batch.target_revision,
            batch,
        });
    }

    let derivation_failed = attempt
        .derivation
        .as_ref()
        .is_some_and(storyforge_domain::turn::DerivationComponents::has_failure);
    if derivation_failed && !force_accept {
        return Err(AcceptError::DerivationFailed);
    }

    // Quality gate（V7：与 JSON 路径共用 domain 决策函数，杜绝两处内联实现漂移）。
    let commit_as_degraded = match storyforge_domain::turn::quality_accept_decision(
        attempt.quality_report.as_ref(),
        force_accept,
    ) {
        storyforge_domain::turn::QualityAcceptDecision::AllowCommit => derivation_failed,
        storyforge_domain::turn::QualityAcceptDecision::ForceDegraded { .. } => true,
        storyforge_domain::turn::QualityAcceptDecision::Block { error_count } => {
            return Err(AcceptError::QualityBlocked { error_count });
        }
    };

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
    if compute_draft_hash(&current_text) != attempt.draft_hash {
        return Err(AcceptError::DraftHashMismatch);
    }

    let camp = get_campaign(&turn.campaign_id)
        .map_err(AcceptError::Storage)?
        .ok_or(AcceptError::CampaignMissing)?;
    let campaign_revision_before = camp.revision;
    if turn.base_campaign_revision != campaign_revision_before {
        return Err(AcceptError::RevisionConflict {
            base: turn.base_campaign_revision,
            current: campaign_revision_before,
        });
    }

    let batch = prepare_commit_batch(&attempt, variant_id, campaign_revision_before);
    let terminal_status = if commit_as_degraded {
        TurnStatus::Degraded
    } else {
        TurnStatus::Committed
    };
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
            .ok_or_else(|| format!("turn {turn_id} disappeared before accept preparation"))?;
        if current.status != TurnStatus::AwaitingAcceptance {
            return Err(format!(
                "turn {turn_id} changed to {:?} before accept preparation",
                current.status
            ));
        }
        let stored_attempt = current
            .find_attempt_mut(&attempt_id)
            .ok_or_else(|| format!("attempt {attempt_id} disappeared before accept preparation"))?;
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

    let campaign_revision_after = get_campaign(&turn.campaign_id)
        .ok()
        .flatten()
        .map(|c| c.revision)
        .unwrap_or(campaign_revision_before + 1);

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

/// Atomic first-draft land: conversation AI node + Turn/Attempt DraftReady + outbox.
pub fn create_draft_attempt(
    request: DraftAttemptRequest<'_>,
) -> Result<DraftAttemptOutcome, String> {
    with_db_mut(|db| {
        SqlitePreacceptRepository::create_draft_attempt(db, request).map_err(|e| e.to_string())
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

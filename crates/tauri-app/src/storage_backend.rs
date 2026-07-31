//! Application-level storage backend wiring.
//!
//! This module resolves the storage backend at process startup and, when
//! SQLite is explicitly selected, runs the fail-closed cutover. JSON remains
//! the production default — no dual-write, no automatic data deletion.
//!
//! After a successful cutover (or when a valid SQLite marker already exists),
//! `sqlite_runtime` is activated and becomes the sole authority for
//! Campaign / Conversation / Turn Accept and recovery. JSON stores are not
//! consulted for those operations and are never dual-written.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use storyforge_app_conversation::ConversationStore;
use storyforge_infra_sqlite::backend::{
    BackendDiagnostics, BackendSelection, PinnedBackend, StorageBackend,
};
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker, recover_or_verify,
};
use storyforge_infra_sqlite::migrations::current_version;

use crate::campaign_store::{CampaignStore, StoredCard};
use crate::compress_job_store::CompressJobStore;
use crate::error::TauriCommandError;
use crate::sqlite_runtime;
use crate::storage::CharacterStore;
use crate::stored_character_for_id_or_source_in_store;
use crate::turn_store::TurnStore;
use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;

/// Gate 4 P1-4: the character library DTO re-exported through the facade so
/// both backends expose one application-level character contract.
pub use crate::commands::characters::CharacterInfo;
/// Gate 4 P1-4: the stored-character DTO shared by the JSON and SQLite
/// character libraries.
pub use crate::storage::StoredCharacter;

/// The pinned backend for this process, resolved once at startup.
static PINNED: OnceLock<PinnedBackend> = OnceLock::new();

/// The canonical SQLite database filename in the app data directory.
pub const SQLITE_DB_FILENAME: &str = "storyforge.sqlite3";

/// Result of resolving the backend at startup.
#[derive(Debug, Clone)]
pub struct BackendResolution {
    pub pinned: PinnedBackend,
    pub db_path: Option<PathBuf>,
    pub diagnostics: BackendDiagnostics,
    pub cutover_performed: bool,
}

impl BackendResolution {
    pub fn is_sqlite(&self) -> bool {
        self.pinned.is_sqlite()
    }
}

/// Stable application-facing storage capabilities. Commands and application
/// services consume this contract instead of probing the process-global
/// SQLite handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendCapability {
    CampaignRead,
    CampaignInstanceRead,
    CampaignLifecycle,
    CardCommands,
    CharacterCommands,
    ImportExport,
    CampaignHealth,
    ConversationRead,
    TurnLifecycle,
    Postprocess,
    KnowledgeTaskRead,
    KnowledgeTaskCommands,
    VariableRead,
    VariableCommands,
    WorldInfo,
    TypedMetaPatch,
    MvuTranslation,
    MvuSchemaApply,
    ChroniclePublication,
    ChronicleCompressor,
    ActiveCampaignPersistence,
    StoryClock,
}

impl BackendCapability {
    pub const ALL: [Self; 22] = [
        Self::CampaignRead,
        Self::CampaignInstanceRead,
        Self::CampaignLifecycle,
        Self::CardCommands,
        Self::CharacterCommands,
        Self::ImportExport,
        Self::CampaignHealth,
        Self::ConversationRead,
        Self::TurnLifecycle,
        Self::Postprocess,
        Self::KnowledgeTaskRead,
        Self::KnowledgeTaskCommands,
        Self::VariableRead,
        Self::VariableCommands,
        Self::WorldInfo,
        Self::TypedMetaPatch,
        Self::MvuTranslation,
        Self::MvuSchemaApply,
        Self::ChroniclePublication,
        Self::ChronicleCompressor,
        Self::ActiveCampaignPersistence,
        Self::StoryClock,
    ];
}

/// Availability is explicit so unsupported or recovery-only behavior cannot
/// be mistaken for an empty successful result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStatus {
    Supported,
    Degraded,
    Unsupported,
    MigrationRequired,
    ReadOnlyRecovery,
}

/// Process-lifetime backend facade injected into `AppState`.
///
/// The facade owns the pinned selector and canonical data directory. Domain
/// ports are added to this type as Gate 3 migrates each vertical slice.
#[derive(Clone)]
pub struct StorageFacade {
    data_dir: PathBuf,
    pinned: PinnedBackend,
    json_campaign_store: Option<Arc<CampaignStore>>,
    json_character_store: Option<Arc<CharacterStore>>,
    json_turn_store: Option<Arc<TurnStore>>,
    json_compress_job_store: Option<Arc<CompressJobStore>>,
}

impl std::fmt::Debug for StorageFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorageFacade")
            .field("data_dir", &self.data_dir)
            .field("pinned", &self.pinned)
            .field("json_writers_constructed", &self.has_json_writers())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CampaignRecord {
    pub campaign: Campaign,
    pub instance_count: usize,
}

impl StorageFacade {
    pub fn new(data_dir: PathBuf, pinned: PinnedBackend) -> Self {
        let (json_campaign_store, json_character_store, json_turn_store, json_compress_job_store) =
            if pinned.is_sqlite() {
                (None, None, None, None)
            } else {
                (
                    Some(Arc::new(CampaignStore::new(&data_dir))),
                    Some(Arc::new(CharacterStore::new(&data_dir))),
                    Some(Arc::new(TurnStore::new(&data_dir))),
                    Some(Arc::new(CompressJobStore::new(&data_dir))),
                )
            };
        Self {
            data_dir,
            pinned,
            json_campaign_store,
            json_character_store,
            json_turn_store,
            json_compress_job_store,
        }
    }

    pub fn backend(&self) -> StorageBackend {
        self.pinned.backend()
    }

    pub fn is_sqlite(&self) -> bool {
        self.pinned.is_sqlite()
    }

    pub fn is_json(&self) -> bool {
        !self.is_sqlite()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Whether this facade owns legacy JSON writer adapters. SQLite facades
    /// never construct them.
    pub fn has_json_writers(&self) -> bool {
        self.json_campaign_store.is_some()
            || self.json_character_store.is_some()
            || self.json_turn_store.is_some()
            || self.json_compress_job_store.is_some()
    }

    pub fn capability(&self, capability: BackendCapability) -> CapabilityStatus {
        if self.is_json() {
            return CapabilityStatus::Supported;
        }
        match capability {
            BackendCapability::CampaignRead
            | BackendCapability::CampaignInstanceRead
            | BackendCapability::CampaignHealth
            | BackendCapability::ConversationRead
            | BackendCapability::TurnLifecycle
            | BackendCapability::Postprocess
            | BackendCapability::KnowledgeTaskRead
            | BackendCapability::VariableRead
            | BackendCapability::MvuTranslation
            | BackendCapability::ChroniclePublication
            | BackendCapability::WorldInfo
            | BackendCapability::TypedMetaPatch
            | BackendCapability::MvuSchemaApply
            | BackendCapability::ChronicleCompressor
            | BackendCapability::StoryClock
            | BackendCapability::CharacterCommands
            | BackendCapability::ImportExport => CapabilityStatus::Supported,
            BackendCapability::ActiveCampaignPersistence => CapabilityStatus::Degraded,
            BackendCapability::CampaignLifecycle
            | BackendCapability::CardCommands
            | BackendCapability::KnowledgeTaskCommands
            | BackendCapability::VariableCommands => CapabilityStatus::Unsupported,
        }
    }

    /// Require a fully supported capability before entering an application
    /// path that would otherwise touch a backend-specific adapter.
    pub fn require_supported(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<(), String> {
        let status = self.capability(capability);
        if status == CapabilityStatus::Supported {
            return Ok(());
        }
        Err(format!(
            "{operation} is unavailable for {:?}: capability {capability:?} is {status:?}",
            self.backend()
        ))
    }

    /// Fail closed when the injected facade and the process-owned SQLite
    /// handle do not describe the same authority.
    pub fn validate_runtime_authority(&self) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::validate_active_path(&self.data_dir.join(SQLITE_DB_FILENAME))
        } else if sqlite_runtime::is_sqlite_active() {
            Err("JSON facade cannot coexist with an active SQLite runtime".to_string())
        } else {
            Ok(())
        }
    }

    pub fn json_campaign_store(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<&CampaignStore, String> {
        self.require_supported(capability, operation)?;
        self.json_campaign_store.as_deref().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CampaignStore for {:?}",
                self.backend()
            )
        })
    }

    pub(crate) fn json_campaign_store_owned(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<Arc<CampaignStore>, String> {
        self.require_supported(capability, operation)?;
        self.json_campaign_store.clone().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CampaignStore for {:?}",
                self.backend()
            )
        })
    }

    pub fn json_character_store(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<&CharacterStore, String> {
        self.require_supported(capability, operation)?;
        self.json_character_store.as_deref().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CharacterStore for {:?}",
                self.backend()
            )
        })
    }

    pub(crate) fn json_character_store_owned(
        &self,
        capability: BackendCapability,
        operation: &str,
    ) -> Result<Arc<CharacterStore>, String> {
        self.require_supported(capability, operation)?;
        self.json_character_store.clone().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CharacterStore for {:?}",
                self.backend()
            )
        })
    }

    pub fn json_turn_store(&self, operation: &str) -> Result<&TurnStore, String> {
        self.json_turn_store.as_deref().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy TurnStore for {:?}",
                self.backend()
            )
        })
    }

    pub(crate) fn json_compress_job_store(
        &self,
        operation: &str,
    ) -> Result<Arc<CompressJobStore>, String> {
        self.require_supported(BackendCapability::ChronicleCompressor, operation)?;
        self.json_compress_job_store.clone().ok_or_else(|| {
            format!(
                "{operation} cannot use the legacy CompressJobStore for {:?}",
                self.backend()
            )
        })
    }

    pub fn get_active_turn(
        &self,
        campaign_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_active_turn(campaign_id)
        } else {
            Ok(self
                .json_turn_store("get active turn")?
                .get_active_turn(campaign_id))
        }
    }

    pub fn get_turn_by_variant(
        &self,
        variant_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_turn_by_variant(variant_id)
        } else {
            Ok(self
                .json_turn_store("get turn by variant")?
                .get_turn_by_variant(variant_id))
        }
    }

    pub fn get_turn(
        &self,
        turn_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_turn(turn_id)
        } else {
            Ok(self.json_turn_store("get turn")?.get_turn(turn_id))
        }
    }

    pub fn save_turn(&self, turn: &storyforge_domain::turn::TurnRecord) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_turn(turn)
        } else {
            self.json_turn_store("save turn")?.create_turn(turn.clone())
        }
    }

    pub fn update_turn_record<F>(&self, turn_id: &Id, mutate: F) -> Result<(), String>
    where
        F: FnOnce(&mut storyforge_domain::turn::TurnRecord),
    {
        if self.is_sqlite() {
            sqlite_runtime::update_turn_record(turn_id, mutate)
        } else {
            self.json_turn_store("update turn")?
                .with_turn_mut(turn_id, mutate)
        }
    }

    pub fn mutate_turn_if<P, M>(
        &self,
        turn_id: &Id,
        predicate: P,
        mutate: M,
    ) -> Result<bool, String>
    where
        P: FnOnce(&storyforge_domain::turn::TurnRecord) -> bool,
        M: FnOnce(&mut storyforge_domain::turn::TurnRecord),
    {
        if self.is_sqlite() {
            sqlite_runtime::mutate_turn_if(turn_id, predicate, mutate)
        } else {
            self.json_turn_store("mutate turn")?
                .mutate_if(turn_id, predicate, mutate)
        }
    }

    pub fn save_campaign(&self, campaign: &Campaign) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_campaign(campaign)
        } else {
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "save campaign")?
                .update_campaign(campaign.clone())
        }
    }

    pub fn get_card_payload(&self, card_id: &Id) -> Result<Option<serde_json::Value>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_card_payload(card_id)
        } else {
            self.json_campaign_store(BackendCapability::CardCommands, "get card payload")?
                .get_card(card_id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| format!("serialize card payload: {error}"))
        }
    }

    pub fn list_card_payloads(&self) -> Result<Vec<serde_json::Value>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_card_payloads()
        } else {
            self.json_campaign_store(BackendCapability::CardCommands, "list card payloads")?
                .list_cards()
                .into_iter()
                .map(|card| {
                    serde_json::to_value(card)
                        .map_err(|error| format!("serialize card payload: {error}"))
                })
                .collect()
        }
    }

    pub fn save_mvu(
        &self,
        stored: &crate::campaign_store::StoredMvuTranslation,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::save_mvu(stored)
        } else {
            self.json_campaign_store(BackendCapability::MvuTranslation, "save MVU translation")?
                .save_mvu(stored.clone())
        }
    }

    pub fn get_mvu(
        &self,
        source_character_id: &Id,
    ) -> Result<Option<crate::campaign_store::StoredMvuTranslation>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_mvu(source_character_id)
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::MvuTranslation, "get MVU translation")?
                .get_mvu(source_character_id))
        }
    }

    pub fn list_mvu(&self) -> Result<Vec<crate::campaign_store::StoredMvuTranslation>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_mvu()
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::MvuTranslation, "list MVU translations")?
                .list_all_mvu())
        }
    }

    /// Persist the active Campaign pointer. JSON writes `active_campaign.json`
    /// (legacy bootstrap pointer); SQLite keeps the selection in-process
    /// (`ActiveCampaignPersistence` is Degraded) and this is a no-op.
    pub fn save_active_pointer(&self, campaign_id: Option<&Id>) -> Result<(), String> {
        if !self.is_sqlite() {
            save_active_campaign(&self.data_dir, campaign_id)?;
        }
        Ok(())
    }

    /// Whether the pipeline must defer conversation land to the pre-accept
    /// UoW (SQLite) instead of landing drafts into the ConversationStore.
    pub fn defer_pipeline_conversation_land(&self) -> bool {
        self.is_sqlite()
    }

    pub fn list_campaigns(&self, card_id: Option<&Id>) -> Result<Vec<CampaignRecord>, String> {
        let campaigns = if self.is_sqlite() {
            sqlite_runtime::list_campaigns()?
        } else if let Some(card_id) = card_id {
            let json_store =
                self.json_campaign_store(BackendCapability::CampaignRead, "list campaigns")?;
            json_store.list_campaigns_of_card(card_id)
        } else {
            self.json_campaign_store(BackendCapability::CampaignRead, "list campaigns")?
                .list_campaigns()
        };

        campaigns
            .into_iter()
            .filter(|campaign| card_id.is_none_or(|card_id| campaign.card_id == *card_id))
            .map(|campaign| {
                let instance_count = if self.is_sqlite() {
                    sqlite_runtime::list_instances(&campaign.id)?.len()
                } else {
                    let json_store = self.json_campaign_store(
                        BackendCapability::CampaignRead,
                        "count campaign instances",
                    )?;
                    json_store.list_instances(&campaign.id).len()
                };
                Ok(CampaignRecord {
                    campaign,
                    instance_count,
                })
            })
            .collect()
    }

    pub fn get_campaign(&self, campaign_id: &Id) -> Result<Option<CampaignRecord>, String> {
        let campaign = if self.is_sqlite() {
            sqlite_runtime::get_campaign(campaign_id)?
        } else {
            self.json_campaign_store(BackendCapability::CampaignRead, "get campaign")?
                .get_campaign(campaign_id)
        };
        let Some(campaign) = campaign else {
            return Ok(None);
        };
        let instance_count = if self.is_sqlite() {
            sqlite_runtime::list_instances(&campaign.id)?.len()
        } else {
            let json_store = self
                .json_campaign_store(BackendCapability::CampaignRead, "count campaign instances")?;
            json_store.list_instances(&campaign.id).len()
        };
        Ok(Some(CampaignRecord {
            campaign,
            instance_count,
        }))
    }

    pub fn campaign_exists(&self, campaign_id: &Id) -> Result<bool, String> {
        self.get_campaign(campaign_id)
            .map(|record| record.is_some())
    }

    pub fn list_instances(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::campaign::CharacterInstance>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_instances(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(
                    BackendCapability::CampaignInstanceRead,
                    "list campaign instances",
                )?
                .list_instances(campaign_id))
        }
    }

    pub fn get_instance(
        &self,
        campaign_id: &Id,
        instance_id: &Id,
    ) -> Result<Option<storyforge_domain::campaign::CharacterInstance>, String> {
        Ok(self
            .list_instances(campaign_id)?
            .into_iter()
            .find(|instance| instance.id == *instance_id))
    }

    pub fn list_knowledge(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_knowledge(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(
                    BackendCapability::KnowledgeTaskRead,
                    "list character knowledge",
                )?
                .list_knowledge(campaign_id))
        }
    }

    pub fn list_tasks(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::story_task::StoryTask>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_tasks(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::KnowledgeTaskRead, "list story tasks")?
                .list_tasks(campaign_id))
        }
    }

    pub fn list_summaries(
        &self,
        campaign_id: &Id,
    ) -> Result<Vec<storyforge_domain::agent::RoundSummary>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_summaries(campaign_id)
        } else {
            Ok(self
                .json_campaign_store(
                    BackendCapability::ChroniclePublication,
                    "list round summaries",
                )?
                .list_summaries(campaign_id))
        }
    }

    // ─── Gate 4 P1-4: character library (backend-neutral facade) ──────────

    /// Persist an imported character card. JSON writes `characters.json`;
    /// SQLite writes the V007 `characters` table.
    pub fn save_character(&self, info: CharacterInfo) -> Result<StoredCharacter, String> {
        if self.is_sqlite() {
            sqlite_runtime::save_character(&info)
        } else {
            self.json_character_store(BackendCapability::CharacterCommands, "save character")?
                .save(info)
        }
    }

    /// List every stored character.
    pub fn list_characters(&self) -> Result<Vec<StoredCharacter>, String> {
        if self.is_sqlite() {
            sqlite_runtime::list_characters()
        } else {
            Ok(self
                .json_character_store(BackendCapability::CharacterCommands, "list characters")?
                .list())
        }
    }

    /// Get a stored character by stored id or source `Character.id`.
    pub fn get_character(&self, id_or_source: &str) -> Result<Option<StoredCharacter>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_character(id_or_source)
        } else {
            let store =
                self.json_character_store(BackendCapability::CharacterCommands, "get character")?;
            Ok(stored_character_for_id_or_source_in_store(
                store,
                &Id::from_str(id_or_source),
            ))
        }
    }

    /// Delete a stored character by stored id or source `Character.id`.
    pub fn delete_character(&self, id_or_source: &str) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_character(id_or_source)
        } else {
            self.json_character_store(BackendCapability::CharacterCommands, "delete character")?
                .delete(id_or_source)
        }
    }

    /// Update a character world-info entry route (world_info_entries[i].route).
    pub fn update_character_world_info_route(
        &self,
        id_or_source: &str,
        entry_index: usize,
        new_route: &str,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_world_info_route_update(
                    info,
                    entry_index,
                    new_route,
                )
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "update character world info route",
            )?
            .update_world_info_route(id_or_source, entry_index, new_route)
        }
    }

    /// Update a character world-info entry's keys/content/constant/is_global/depth/order.
    #[allow(clippy::too_many_arguments)]
    pub fn update_character_world_info_entry(
        &self,
        id_or_source: &str,
        entry_index: usize,
        keys: Vec<String>,
        content: String,
        constant: bool,
        is_global: bool,
        depth: i32,
        order: i32,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_world_info_entry_update(
                    info,
                    entry_index,
                    keys,
                    content,
                    constant,
                    is_global,
                    depth,
                    order,
                )
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "update character world info entry",
            )?
            .update_world_info_entry(
                id_or_source,
                entry_index,
                keys,
                content,
                constant,
                is_global,
                depth,
                order,
            )
        }
    }

    /// Append a character world-info entry, returning its new index.
    pub fn add_character_world_info_entry(
        &self,
        id_or_source: &str,
        keys: Vec<String>,
        content: String,
        constant: bool,
        is_global: bool,
    ) -> Result<usize, String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_add_world_info_entry(
                    info, keys, content, constant, is_global,
                )
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "add character world info entry",
            )?
            .add_world_info_entry(id_or_source, keys, content, constant, is_global)
        }
    }

    /// Remove a character world-info entry.
    pub fn delete_character_world_info_entry(
        &self,
        id_or_source: &str,
        entry_index: usize,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_delete_world_info_entry(info, entry_index)
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "delete character world info entry",
            )?
            .delete_world_info_entry(id_or_source, entry_index)
        }
    }

    /// Replace a character's whole world-info entry list (bulk patch).
    pub fn update_character_world_info_entries_bulk(
        &self,
        id_or_source: &str,
        entries: Vec<crate::WorldInfoEntryInfo>,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_character(id_or_source, |info| {
                crate::commands::characters::apply_update_world_info_entries_bulk(info, entries)
            })
        } else {
            self.json_character_store(
                BackendCapability::CharacterCommands,
                "replace character world info entries",
            )?
            .update_world_info_entries_bulk(id_or_source, entries)
        }
    }

    // ─── Gate 4 P1-4: character delete cascade (backend-neutral) ──────────

    /// MVU translation delete (character delete cascade).
    pub fn delete_mvu(&self, source_character_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_mvu(source_character_id)
        } else {
            self.json_campaign_store(BackendCapability::MvuTranslation, "delete MVU cascade")?
                .delete_mvu(source_character_id)
        }
    }

    /// Look up a card wrapper by its ST source character id (cascade bridge).
    pub fn get_card_by_source(
        &self,
        source_character_id: &Id,
    ) -> Result<Option<StoredCard>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_card_payload_by_source(source_character_id)?
                .map(|payload| {
                    serde_json::from_value(payload)
                        .map_err(|e| format!("解析角色卡 payload 失败: {e}"))
                })
                .transpose()
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::CampaignLifecycle, "get card by source")?
                .get_card_by_source(source_character_id))
        }
    }

    /// Resolve the character-library world-info template for a source
    /// character id, backend-neutral (Gate 4 四审 P1: set_active_campaign 不再
    /// 触碰 JSON store)。JSON 复用既有 CharacterStore 语义；SQLite 读角色库。
    pub fn resolve_character_world_info_template(
        &self,
        source_character_id: &Id,
    ) -> Result<Option<storyforge_domain::world_info::WorldInfoBook>, String> {
        if self.is_sqlite() {
            let Some(stored) = sqlite_runtime::get_character(source_character_id.as_str())? else {
                return Ok(None);
            };
            let info = stored.info;
            if let Some(book) = info.embedded_world_info.clone() {
                return Ok(Some(
                    crate::commands::campaigns::merge_global_entries_into_book_facade(
                        self, book, &info.name,
                    ),
                ));
            }
            if let Some(book) =
                crate::startup_support::world_info_book_from_entries(&info.world_info_entries)
            {
                return Ok(Some(
                    crate::commands::campaigns::merge_global_entries_into_book_facade(
                        self, book, &info.name,
                    ),
                ));
            }
            Ok(None)
        } else {
            let store = self.json_character_store(
                BackendCapability::CharacterCommands,
                "world info template",
            )?;
            let Some(stored) =
                stored_character_for_id_or_source_in_store(store, source_character_id)
            else {
                return Ok(None);
            };
            let info = stored.info;
            if let Some(book) = info.embedded_world_info.clone() {
                return Ok(Some(
                    crate::commands::campaigns::merge_global_entries_into_book(
                        store, book, &info.name,
                    ),
                ));
            }
            if let Some(book) =
                crate::startup_support::world_info_book_from_entries(&info.world_info_entries)
            {
                return Ok(Some(
                    crate::commands::campaigns::merge_global_entries_into_book(
                        store, book, &info.name,
                    ),
                ));
            }
            Ok(None)
        }
    }

    /// Delete a card payload with its campaign cascade (character delete).
    pub fn delete_card(&self, card_id: &Id) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_card_payload(card_id)
        } else {
            self.json_campaign_store(BackendCapability::CampaignLifecycle, "delete card")?
                .delete_card(card_id)
        }
    }

    /// Delete one character with its **entire** cascade (MVU + card + all
    /// campaign dependent rows) in a single atomic operation. SQLite: one
    /// transaction; JSON: existing `delete_character` cascade semantics.
    pub fn delete_character_full_cascade(
        &self,
        id: &str,
        extra_source_ids: &[Id],
    ) -> Result<bool, String> {
        if self.is_sqlite() {
            sqlite_runtime::delete_character_full_cascade(id, extra_source_ids)
        } else {
            // JSON 路径：沿用 CharacterStore + CampaignStore 既有级联。
            let character_store = self
                .json_character_store(BackendCapability::CharacterCommands, "delete character")?;
            let removed = character_store.delete(id)?;
            let campaign_store =
                self.json_campaign_store(BackendCapability::CampaignLifecycle, "delete character")?;
            for source_id in extra_source_ids {
                let _ = campaign_store.delete_mvu(source_id);
                if let Some(stored_card) = campaign_store.get_card_by_source(source_id) {
                    let _ = campaign_store.delete_card(&stored_card.card.id);
                }
            }
            if let Some(stored) = character_store.get(id)
                && let Some(source_id) = stored.info.source_character_id.as_ref()
            {
                let source_id = Id::from_str(source_id);
                let _ = campaign_store.delete_mvu(&source_id);
                if let Some(stored_card) = campaign_store.get_card_by_source(&source_id) {
                    let _ = campaign_store.delete_card(&stored_card.card.id);
                }
            }
            Ok(removed)
        }
    }

    // ─── Gate 4 P1-4: ImportExport (backend-neutral facade) ───────────────

    /// Get one card wrapper payload as a `StoredCard`.
    pub fn get_card(&self, card_id: &Id) -> Result<Option<StoredCard>, String> {
        if self.is_sqlite() {
            sqlite_runtime::get_card_payload(card_id)?
                .map(|payload| {
                    serde_json::from_value(payload)
                        .map_err(|e| format!("解析角色卡 payload 失败: {e}"))
                })
                .transpose()
        } else {
            Ok(self
                .json_campaign_store(BackendCapability::ImportExport, "get card")?
                .get_card(card_id))
        }
    }

    /// Export the full Campaign bundle JSON. Both backends produce the exact
    /// same `CampaignBundle` structure (format_version 2).
    pub fn export_campaign_bundle(&self, camp_id: &Id) -> Result<String, TauriCommandError> {
        if self.is_sqlite() {
            self.export_campaign_bundle_from_sqlite(camp_id)
        } else {
            let store = self
                .json_campaign_store(BackendCapability::ImportExport, "export campaign bundle")?;
            crate::commands::import_export::export_campaign_bundle_from_store(
                store,
                camp_id.clone(),
            )
        }
    }

    /// Import a Campaign bundle. JSON path keeps its snapshot-verified
    /// rollback; SQLite path lands everything in one transaction.
    pub fn import_campaign_bundle(
        &self,
        bundle: crate::commands::import_export::CampaignBundle,
        conv_store: &ConversationStore,
    ) -> Result<crate::commands::import_export::CampaignImportResult, TauriCommandError> {
        if self.is_sqlite() {
            crate::commands::import_export::import_campaign_bundle_into_sqlite(conv_store, bundle)
        } else {
            let store = self
                .json_campaign_store(BackendCapability::ImportExport, "import campaign bundle")
                .map_err(TauriCommandError::validation)?;
            crate::commands::import_export::import_campaign_bundle_into_store(
                store, conv_store, bundle,
            )
        }
    }

    fn export_campaign_bundle_from_sqlite(
        &self,
        camp_id: &Id,
    ) -> Result<String, TauriCommandError> {
        use crate::commands::import_export::{BUNDLE_FORMAT_VERSION, CampaignBundle};

        let campaign = sqlite_runtime::get_campaign(camp_id)
            .map_err(TauriCommandError::storage)?
            .ok_or_else(|| {
                TauriCommandError::not_found(format!("Campaign 不存在: {}", camp_id.as_str()))
            })?;
        let stored_card = match sqlite_runtime::get_card_payload(&campaign.card_id)
            .map_err(TauriCommandError::storage)?
        {
            Some(payload) => Some(serde_json::from_value::<StoredCard>(payload).map_err(|e| {
                TauriCommandError::storage(format!("角色卡 payload 解析失败: {e}"))
            })?),
            None => None,
        };
        let instances =
            sqlite_runtime::list_instances(camp_id).map_err(TauriCommandError::storage)?;
        let definitions = stored_card
            .as_ref()
            .map(|c| c.card.character_definitions.clone())
            .unwrap_or_default();
        let knowledge =
            sqlite_runtime::list_knowledge(camp_id).map_err(TauriCommandError::storage)?;
        let tasks = sqlite_runtime::list_tasks(camp_id).map_err(TauriCommandError::storage)?;
        let summaries =
            sqlite_runtime::list_summaries(camp_id).map_err(TauriCommandError::storage)?;

        let bundle = CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: stored_card.map(|c| c.card),
            campaign,
            instances,
            definitions,
            knowledge,
            tasks,
            summaries,
        };
        serde_json::to_string_pretty(&bundle)
            .map_err(|e| TauriCommandError::internal(format!("Bundle 序列化失败: {e}")))
    }

    // ─── Gate 4: campaign world info (backend-neutral facade) ─────────────

    /// Read the campaign world info book. Missing data yields an empty book on
    /// both backends (JSON treats a missing file as empty; SQLite a missing row).
    pub fn get_world_info(
        &self,
        campaign_id: &Id,
    ) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
        if self.is_sqlite() {
            Ok(sqlite_runtime::get_world_info(campaign_id)?.unwrap_or_else(empty_world_info_book))
        } else {
            self.json_campaign_store(BackendCapability::WorldInfo, "get campaign world info")?
                .get_world_info(campaign_id)
                .map_err(|e| e.to_string())
        }
    }

    pub fn set_world_info(
        &self,
        campaign_id: &Id,
        book: &storyforge_domain::world_info::WorldInfoBook,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::set_world_info(campaign_id, book)
        } else {
            self.json_campaign_store(BackendCapability::WorldInfo, "set campaign world info")?
                .set_world_info(campaign_id, book.clone())
        }
    }

    /// Seed the campaign book from a card template when it is still empty.
    pub fn ensure_world_info_from_book(
        &self,
        campaign_id: &Id,
        template: &storyforge_domain::world_info::WorldInfoBook,
    ) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
        if self.is_sqlite() {
            sqlite_runtime::ensure_world_info_from_book(campaign_id, template)
        } else {
            self.json_campaign_store(BackendCapability::WorldInfo, "ensure campaign world info")?
                .ensure_world_info_from_book(campaign_id, template)
        }
    }

    pub fn add_world_info_entry(
        &self,
        campaign_id: &Id,
        entry: storyforge_domain::world_info::WorldInfoEntry,
    ) -> Result<usize, String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                let mut entry = entry.clone();
                if entry.extensions.is_null() {
                    entry.extensions = serde_json::json!({ "sf_source": "user" });
                } else if let Some(obj) = entry.extensions.as_object_mut() {
                    obj.entry("sf_source")
                        .or_insert_with(|| serde_json::json!("user"));
                }
                book.entries.push(entry);
                Ok(book.entries.len() - 1)
            })
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "add campaign world info entry",
            )?
            .add_world_info_entry(campaign_id, entry)
        }
    }

    pub fn update_world_info_entry(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        entry: storyforge_domain::world_info::WorldInfoEntry,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                if entry_index >= book.entries.len() {
                    return Err(format!("世界书条目索引越界: {entry_index}"));
                }
                book.entries[entry_index] = entry;
                Ok(())
            })?;
            Ok(())
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "update campaign world info entry",
            )?
            .update_world_info_entry(campaign_id, entry_index, entry)
        }
    }

    pub fn delete_world_info_entry(
        &self,
        campaign_id: &Id,
        entry_index: usize,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                if entry_index >= book.entries.len() {
                    return Err(format!("世界书条目索引越界: {entry_index}"));
                }
                book.entries.remove(entry_index);
                Ok(())
            })?;
            Ok(())
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "delete campaign world info entry",
            )?
            .delete_world_info_entry(campaign_id, entry_index)
        }
    }

    pub fn set_world_info_route(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        route: storyforge_domain::world_info::LoreRoute,
    ) -> Result<(), String> {
        if self.is_sqlite() {
            sqlite_runtime::mutate_world_info(campaign_id, |book| {
                if entry_index >= book.entries.len() {
                    return Err(format!("世界书条目索引越界: {entry_index}"));
                }
                book.entries[entry_index].route = route;
                book.entries[entry_index].disabled = matches!(
                    book.entries[entry_index].route,
                    storyforge_domain::world_info::LoreRoute::Disabled
                );
                Ok(())
            })?;
            Ok(())
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "set campaign world info route",
            )?
            .set_world_info_route(campaign_id, entry_index, route)
        }
    }

    pub fn set_world_info_entry_enabled(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        enabled: bool,
    ) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
        if self.is_sqlite() {
            let book = sqlite_runtime::mutate_world_info(campaign_id, |book| {
                let entry = book
                    .entries
                    .get_mut(entry_index)
                    .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;
                entry.set_enabled(enabled)?;
                Ok(book.clone())
            })?;
            Ok(book)
        } else {
            self.json_campaign_store(
                BackendCapability::WorldInfo,
                "set campaign world info enabled",
            )?
            .set_world_info_entry_enabled(campaign_id, entry_index, enabled)
        }
    }

    /// Template world info resolvable from the SQLite card authority
    /// (`raw_card_json.character_book`), used when a campaign book is empty.
    pub fn template_world_info_from_card(
        &self,
        card_payload: &serde_json::Value,
    ) -> Result<Option<storyforge_domain::world_info::WorldInfoBook>, String> {
        if !self.is_sqlite() {
            return Ok(None);
        }
        let stored: crate::campaign_store::StoredCard =
            serde_json::from_value(card_payload.clone())
                .map_err(|e| format!("invalid SQLite card payload: {e}"))?;
        let template = stored
            .card
            .raw_card_json
            .get("character_book")
            .and_then(|value| {
                serde_json::from_value::<storyforge_domain::character::StWorldInfoBook>(
                    value.clone(),
                )
                .ok()
            })
            .map(storyforge_domain::world_info::WorldInfoBook::from_st);
        Ok(template)
    }
}

fn empty_world_info_book() -> storyforge_domain::world_info::WorldInfoBook {
    storyforge_domain::world_info::WorldInfoBook {
        entries: Vec::new(),
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}

/// Resolve and pin the storage backend for this process.
///
/// When SQLite is explicitly selected, this also runs the cutover (or verifies
/// it has already completed). When JSON is selected (the default), no database
/// is opened and no cutover runs.
///
/// This function is safe to call multiple times — the first call pins the
/// backend, and subsequent calls return the cached resolution.
pub fn resolve_backend(data_dir: &Path) -> Result<BackendResolution, BackendWiringError> {
    // If already pinned, return cached resolution.
    if let Some(pinned) = PINNED.get() {
        let schema_version = pinned
            .is_sqlite()
            .then(|| current_version_sqlite(&data_dir.join(SQLITE_DB_FILENAME)));
        return Ok(BackendResolution {
            pinned: pinned.clone(),
            db_path: pinned
                .is_sqlite()
                .then(|| data_dir.join(SQLITE_DB_FILENAME)),
            diagnostics: BackendDiagnostics::from_pinned(pinned, schema_version.flatten()),
            cutover_performed: false,
        });
    }

    let resolution = resolve_backend_inner(data_dir)?;
    let _ = PINNED.set(resolution.pinned.clone());
    Ok(resolution)
}

/// Inner resolution logic without the OnceLock — testable in isolation.
fn resolve_backend_inner(data_dir: &Path) -> Result<BackendResolution, BackendWiringError> {
    let selection = BackendSelection::from_env(None);
    let pinned = PinnedBackend::resolve(&selection)
        .map_err(|e| BackendWiringError::Selection(format!("{e}")))?;

    match pinned.backend() {
        StorageBackend::Json => {
            let diag = BackendDiagnostics::from_pinned(&pinned, None);
            Ok(BackendResolution {
                pinned,
                db_path: None,
                diagnostics: diag,
                cutover_performed: false,
            })
        }
        StorageBackend::Sqlite => {
            let db_path = data_dir.join(SQLITE_DB_FILENAME);
            let plan = CutoverPlan::new(data_dir, &db_path);

            // Run the cutover (or verify if already done).
            let request = CutoverRequest {
                plan: plan.clone(),
                label: "app-startup".to_string(),
            };
            let outcome = recover_or_verify(&request)
                .map_err(|e| BackendWiringError::Cutover(format!("{e}")))?;

            let cutover_performed = matches!(outcome, CutoverOutcome::Completed(_));

            let schema_version = {
                let db = storyforge_infra_sqlite::Database::open(&db_path)
                    .map_err(|e| BackendWiringError::Cutover(format!("reopen: {e}")))?;
                current_version(&db).unwrap_or(0)
            };

            let diag = BackendDiagnostics::from_pinned(&pinned, Some(schema_version));

            Ok(BackendResolution {
                pinned,
                db_path: Some(db_path),
                diagnostics: diag,
                cutover_performed,
            })
        }
    }
}

/// Check the current marker status without performing a cutover.
pub fn check_marker_status(data_dir: &Path) -> MarkerStatus {
    let plan = CutoverPlan::new(data_dir, data_dir.join(SQLITE_DB_FILENAME));
    inspect_marker(&plan)
}

/// The SQLite database path for the given data directory, regardless of
/// whether SQLite is currently selected.
pub fn sqlite_db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SQLITE_DB_FILENAME)
}

fn current_version_sqlite(db_path: &Path) -> Option<i64> {
    if !db_path.exists() {
        return None;
    }
    let db = storyforge_infra_sqlite::Database::open(db_path).ok()?;
    current_version(&db).ok()
}

/// Persist the JSON active-campaign pointer file (legacy bootstrap sidecar).
/// SQLite keeps the pointer in-process and never writes this file.
fn save_active_campaign(data_dir: &Path, id: Option<&Id>) -> Result<(), String> {
    let path = data_dir.join("active_campaign.json");
    let v = serde_json::json!({ "campaign_id": id.map(|i| i.as_str()).unwrap_or("") });
    storyforge_infra_util::atomic_write_json(&path, &v)
        .map_err(|error| format!("保存活跃 Campaign 失败: {error}"))
}

/// Errors produced during backend wiring.
#[derive(Debug, thiserror::Error)]
pub enum BackendWiringError {
    #[error("backend selection error: {0}")]
    Selection(String),
    #[error("cutover error: {0}")]
    Cutover(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_json(path: &Path, value: &serde_json::Value) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn sample_source(dir: &Path) {
        write_json(
            &dir.join("cards.json"),
            &serde_json::json!([{
                "id": "card-1", "name": "Hero", "source_character_id": null
            }]),
        );
        write_json(
            &dir.join("campaigns.json"),
            &serde_json::json!([{
                "id": "camp-1", "card_id": "card-1", "name": "Main",
                "created_at": "2026-07-13T00:00:00Z", "revision": 0,
                "chronicle_revision": 0, "conversation_id": "conv-1", "lineage_id": "lin-1"
            }]),
        );
        write_json(
            &dir.join("conversations").join("conv-1.json"),
            &serde_json::json!({
                "id": "conv-1", "campaign_id": "camp-1", "character_id": null,
                "created_at": "2026-07-13T00:00:00Z", "updated_at": "2026-07-13T00:00:00Z",
                "nodes": []
            }),
        );
        write_json(&dir.join("instances.json"), &serde_json::json!([]));
        write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
        write_json(&dir.join("tasks.json"), &serde_json::json!([]));
        write_json(
            &dir.join("round_summaries.json"),
            &serde_json::json!([{
                "id": "sum-a1", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "turn": 1, "content": "leaf", "created_at": "2026-07-13T00:00:00Z",
                "level": 0, "lineage_id": "lin-1", "code": "A0001"
            }]),
        );
        write_json(&dir.join("turns.json"), &serde_json::json!([]));
    }

    #[test]
    fn default_resolution_is_json_without_database() {
        let dir = TempDir::new().unwrap();
        // Ensure env var is not set.
        // SAFETY: test-only; no concurrent threads depend on this env var.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(!resolution.is_sqlite());
        assert!(resolution.db_path.is_none());
        assert!(!resolution.cutover_performed);
        // No SQLite file created.
        assert!(!dir.path().join(SQLITE_DB_FILENAME).exists());
    }

    #[test]
    fn sqlite_resolution_runs_cutover() {
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        // SAFETY: test-only.
        unsafe {
            std::env::set_var("STORYFORGE_STORAGE_BACKEND", "sqlite");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(resolution.is_sqlite());
        assert!(resolution.cutover_performed);
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());

        // Cleanup env for other tests.
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
    }

    #[test]
    fn marker_status_absent_when_no_marker() {
        let dir = TempDir::new().unwrap();
        let status = check_marker_status(dir.path());
        assert_eq!(status, MarkerStatus::Absent);
    }

    #[test]
    fn no_dual_write_json_stores_not_affected_by_resolution() {
        // When JSON is selected, resolve_backend must not open any database.
        let dir = TempDir::new().unwrap();
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert_eq!(resolution.pinned.backend(), StorageBackend::Json);
        // No marker, no database.
        assert!(!dir.path().join("storyforge.backend.json").exists());
        assert!(!dir.path().join(SQLITE_DB_FILENAME).exists());
    }

    #[test]
    fn facade_pins_backend_data_dir_and_sqlite_capabilities() {
        let dir = TempDir::new().unwrap();
        let facade = StorageFacade::new(
            dir.path().to_path_buf(),
            PinnedBackend::new(
                StorageBackend::Sqlite,
                storyforge_infra_sqlite::backend::BackendSource::Env,
            ),
        );

        assert_eq!(facade.backend(), StorageBackend::Sqlite);
        assert_eq!(facade.data_dir(), dir.path());
        assert!(!facade.has_json_writers());
        assert_eq!(
            facade.capability(BackendCapability::CampaignRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CampaignInstanceRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::KnowledgeTaskRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::VariableRead),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CardCommands),
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            facade.capability(BackendCapability::CharacterCommands),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::ImportExport),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::CampaignHealth),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::VariableCommands),
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            facade.capability(BackendCapability::KnowledgeTaskCommands),
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            facade.capability(BackendCapability::WorldInfo),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::TypedMetaPatch),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::ChronicleCompressor),
            CapabilityStatus::Supported
        );
        assert_eq!(
            facade.capability(BackendCapability::StoryClock),
            CapabilityStatus::Supported
        );
        facade
            .require_supported(
                BackendCapability::ChronicleCompressor,
                "chronicle compression",
            )
            .expect("SQLite chronicle compressor must be supported");
        assert_eq!(
            facade.capability(BackendCapability::ActiveCampaignPersistence),
            CapabilityStatus::Degraded
        );

        let supported = [
            BackendCapability::CampaignRead,
            BackendCapability::CampaignInstanceRead,
            BackendCapability::CampaignHealth,
            BackendCapability::ConversationRead,
            BackendCapability::TurnLifecycle,
            BackendCapability::Postprocess,
            BackendCapability::KnowledgeTaskRead,
            BackendCapability::VariableRead,
            BackendCapability::MvuTranslation,
            BackendCapability::ChroniclePublication,
            BackendCapability::WorldInfo,
            BackendCapability::TypedMetaPatch,
            BackendCapability::MvuSchemaApply,
            BackendCapability::ChronicleCompressor,
            BackendCapability::StoryClock,
            BackendCapability::CharacterCommands,
            BackendCapability::ImportExport,
        ];
        let degraded = [BackendCapability::ActiveCampaignPersistence];
        let unsupported = [
            BackendCapability::CampaignLifecycle,
            BackendCapability::CardCommands,
            BackendCapability::KnowledgeTaskCommands,
            BackendCapability::VariableCommands,
        ];
        assert_eq!(
            supported.len() + degraded.len() + unsupported.len(),
            BackendCapability::ALL.len()
        );
        for capability in supported {
            assert_eq!(facade.capability(capability), CapabilityStatus::Supported);
        }
        for capability in degraded {
            assert_eq!(facade.capability(capability), CapabilityStatus::Degraded);
        }
        for capability in unsupported {
            assert_eq!(facade.capability(capability), CapabilityStatus::Unsupported);
        }

        let character_error = match facade.json_character_store(
            BackendCapability::CharacterCommands,
            "read SQLite character",
        ) {
            Ok(_) => panic!("SQLite facade must not expose a legacy CharacterStore"),
            Err(error) => error,
        };
        assert!(
            character_error.contains("legacy CharacterStore"),
            "SQLite facade must refuse the legacy JSON CharacterStore, got: {character_error}"
        );
        assert!(!dir.path().join("characters.json").exists());
    }

    #[test]
    fn json_facade_reports_current_capabilities_as_supported() {
        let dir = TempDir::new().unwrap();
        let facade = StorageFacade::new(
            dir.path().to_path_buf(),
            PinnedBackend::new(
                StorageBackend::Json,
                storyforge_infra_sqlite::backend::BackendSource::Default,
            ),
        );

        for capability in BackendCapability::ALL {
            assert_eq!(
                facade.capability(capability),
                CapabilityStatus::Supported,
                "JSON capability {capability:?} must remain available"
            );
        }
        assert!(facade.has_json_writers());
    }

    #[test]
    fn app_state_owns_the_explicitly_injected_facade() {
        let dir = TempDir::new().unwrap();
        let facade = std::sync::Arc::new(StorageFacade::new(
            dir.path().to_path_buf(),
            PinnedBackend::new(
                StorageBackend::Json,
                storyforge_infra_sqlite::backend::BackendSource::Default,
            ),
        ));

        let state = crate::AppState::new_with_backend(dir.path().to_path_buf(), facade.clone())
            .expect("matching JSON facade constructs AppState");

        assert!(std::sync::Arc::ptr_eq(state.storage(), &facade));
        assert_eq!(state.storage().backend(), StorageBackend::Json);
        assert_eq!(state.storage().data_dir(), dir.path());
    }
}

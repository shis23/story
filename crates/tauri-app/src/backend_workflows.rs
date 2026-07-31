//! Named backend adapters for Turn / Attempt / Accept / Postprocess workflows.
//!
//! Gate 3: commands and ordinary application services never probe the storage
//! backend themselves. `AppState::new_with_backend` constructs the concrete
//! adapters once from the startup-pinned `StorageFacade`; every Turn/Attempt/
//! Accept/Postprocess mutation below dispatches JSON vs SQLite internally and
//! exposes only backend-neutral domain DTOs.
//!
//! This module is the one production place (besides `storage_backend.rs` and
//! the bootstrap code in `lib.rs`) where `.is_sqlite()` / `.is_json()` are
//! legal, pinned by the static gate tests.

use std::sync::Arc;

use storyforge_app_agent::ToolContext;
use storyforge_app_conversation::ConversationStore;
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::conversation::{Provenance, Role as ConversationRole};
use storyforge_domain::preset::RegexScript;
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, PostprocessApplyOutcome, PostprocessApplyRequest,
};

use crate::campaign_store::CampaignStore;
use crate::production_postprocess::{
    ProductionPostprocessError, ProductionPostprocessService, TurnAttemptSink,
};
use crate::runtime_support::{
    collect_campaign_scoped_regex_scripts, load_campaign_context_snapshot,
    load_sqlite_campaign_context_snapshot,
};
use crate::storage_backend::{BackendCapability, StorageFacade};
use crate::turn_lifecycle::{self, AcceptError, AcceptOutcome};
use crate::turn_store::TurnStore;

// ─── Backend-neutral workflow DTOs ────────────────────────────────────────

/// First-draft land request. Domain-only types: no infra-sqlite types leak
/// through the application layer.
pub struct DraftAttemptRequest<'a> {
    pub campaign_id: &'a Id,
    pub conversation_id: &'a Id,
    pub turn_id: &'a Id,
    pub attempt_id: &'a Id,
    /// JSON 路径的 provisional draft node id（pipeline 落盘时返回的节点）。
    /// SQLite 路径忽略它——preaccept UoW 产出权威 variant_id。
    pub provisional_variant_id: Option<&'a Id>,
    pub draft_text: &'a str,
    pub pending_temporary_instances: Vec<storyforge_domain::campaign::CharacterInstance>,
    pub provenance: Option<Provenance>,
}

/// Regenerate land request (supersede + new Attempt on the same node).
pub struct RegenerateAttemptRequest<'a> {
    pub campaign_id: &'a Id,
    pub conversation_id: &'a Id,
    pub turn_id: &'a Id,
    pub previous_variant_id: &'a Id,
    pub attempt_id: &'a Id,
    pub draft_text: &'a str,
    pub pending_temporary_instances: Vec<storyforge_domain::campaign::CharacterInstance>,
    pub provenance: Option<Provenance>,
}

/// Land outcome shared by both backends.
#[derive(Debug, Clone)]
pub struct DraftAttemptOutcome {
    pub attempt_id: Id,
    pub variant_id: Id,
}

/// Turn / Attempt / Accept lifecycle through the startup-pinned backend.
///
/// Constructed exactly once in `AppState::new_with_backend`; commands call the
/// backend-neutral methods below and never branch on the storage flag.
pub struct TurnWorkflow {
    storage: Arc<StorageFacade>,
    conv_store: Arc<ConversationStore>,
}

impl TurnWorkflow {
    pub fn new(storage: Arc<StorageFacade>, conv_store: Arc<ConversationStore>) -> Self {
        Self {
            storage,
            conv_store,
        }
    }

    /// Mark the Turn Failed only while it still owns the failed operation:
    /// scope matches the request and the status is still `Generating`.
    ///
    /// A draft/regenerate UoW that failed because the Turn already moved to
    /// `DraftReady`/`Committed` (duplicate or racing caller) must NOT be
    /// downgraded — that would destroy a successful state. Write-back errors
    /// are reported, never silently swallowed.
    fn mark_failed_if_still_generating(
        &self,
        turn_id: &Id,
        campaign_id: &Id,
        conversation_id: &Id,
        reason: String,
    ) {
        use storyforge_domain::turn::TurnStatus;
        match self.storage.mutate_turn_if(
            turn_id,
            |record| {
                record.campaign_id == *campaign_id
                    && record.conversation_id == *conversation_id
                    && record.status == TurnStatus::Generating
            },
            |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(reason.clone());
                record.touch();
            },
        ) {
            Ok(_) => {}
            Err(write_error) => {
                tracing::error!(
                    "标记 Turn {} 失败失败（原因: {reason}）: {write_error}",
                    turn_id
                );
            }
        }
    }

    /// Land the first draft Attempt.
    ///
    /// JSON: attach the Attempt (DraftReady) with soft-delete compensation when
    /// the Turn mutation fails. SQLite: atomic preaccept UoW plus conversation
    /// cache invalidate; failure marks the Turn Failed while it is still
    /// Generating (never downgrades an already-successful state).
    pub fn create_draft_attempt(
        &self,
        request: DraftAttemptRequest<'_>,
    ) -> Result<DraftAttemptOutcome, String> {
        use storyforge_domain::turn::TurnStatus;
        if self.storage.is_sqlite() {
            let outcome = crate::sqlite_runtime::create_draft_attempt(
                storyforge_infra_sqlite::preaccept::DraftAttemptRequest {
                    campaign_id: request.campaign_id,
                    conversation_id: request.conversation_id,
                    turn_id: request.turn_id,
                    attempt_id: request.attempt_id,
                    draft_text: request.draft_text,
                    pending_temporary_instances: request.pending_temporary_instances,
                    provenance: request.provenance,
                },
            )
            .map_err(|e| {
                // UoW 失败会回滚：Turn 若保持 Generating 将永远占用
                // active-turn barrier。仅当 scope 匹配且仍为 Generating 时
                // 才标 Failed 解除屏障；已推进到 DraftReady/Committed 的
                // Turn（重复/竞态请求被 UoW 拒绝）不被降级。
                self.mark_failed_if_still_generating(
                    request.turn_id,
                    request.campaign_id,
                    request.conversation_id,
                    format!("sqlite preaccept draft 失败: {e}"),
                );
                format!("sqlite preaccept draft 失败: {e}")
            })?;
            self.conv_store.invalidate();
            return Ok(DraftAttemptOutcome {
                attempt_id: outcome.attempt_id,
                variant_id: outcome.variant_id,
            });
        }
        let provisional_variant_id = request
            .provisional_variant_id
            .ok_or_else(|| "JSON draft land requires a provisional node id".to_string())?;
        let attempt = turn_lifecycle::new_draft_attempt(
            request.attempt_id.clone(),
            provisional_variant_id.clone(),
            request.draft_text,
            request.pending_temporary_instances,
        );
        if let Err(e) = self.storage.update_turn_record(request.turn_id, |record| {
            record.attempts.push(attempt);
            record.status = TurnStatus::DraftReady;
            record.touch();
        }) {
            // A.1/P0-4：Attempt 创建失败必须传播，并补偿软删无主 Draft
            if let Err(comp_e) = self
                .conv_store
                .soft_delete_variant(request.conversation_id, provisional_variant_id)
            {
                tracing::error!(
                    "P0-4 补偿失败: soft_delete 无主 Draft {} 失败: {comp_e}（原错误: {e}）",
                    provisional_variant_id
                );
            }
            let _ = self.storage.update_turn_record(request.turn_id, |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(format!("TurnAttempt 持久化失败: {e}"));
                record.touch();
            });
            return Err(format!(
                "TurnAttempt 持久化失败（已尝试软删无主 Draft）: {e}"
            ));
        }
        Ok(DraftAttemptOutcome {
            attempt_id: request.attempt_id.clone(),
            variant_id: provisional_variant_id.clone(),
        })
    }

    /// Land a regenerated Attempt (supersedes the previous Attempt).
    pub fn append_regenerate_attempt(
        &self,
        request: RegenerateAttemptRequest<'_>,
    ) -> Result<DraftAttemptOutcome, String> {
        use storyforge_domain::turn::TurnStatus;
        if self.storage.is_sqlite() {
            let outcome = crate::sqlite_runtime::append_regenerate_attempt(
                storyforge_infra_sqlite::preaccept::RegenerateAttemptRequest {
                    campaign_id: request.campaign_id,
                    conversation_id: request.conversation_id,
                    turn_id: request.turn_id,
                    previous_variant_id: request.previous_variant_id,
                    attempt_id: request.attempt_id,
                    draft_text: request.draft_text,
                    pending_temporary_instances: request.pending_temporary_instances,
                    provenance: request.provenance,
                },
            )
            .map_err(|e| {
                // 同 create_draft_attempt：仅当 scope 匹配且仍为 Generating
                // 时标 Failed；重复/竞态请求拒绝已推进状态时不降级。
                self.mark_failed_if_still_generating(
                    request.turn_id,
                    request.campaign_id,
                    request.conversation_id,
                    format!("sqlite preaccept regenerate 失败: {e}"),
                );
                format!("sqlite preaccept regenerate 失败: {e}")
            })?;
            self.conv_store.invalidate();
            return Ok(DraftAttemptOutcome {
                attempt_id: outcome.attempt_id,
                variant_id: outcome.variant_id,
            });
        }
        let new_attempt = turn_lifecycle::new_draft_attempt(
            request.attempt_id.clone(),
            request.previous_variant_id.clone(),
            request.draft_text,
            request.pending_temporary_instances,
        );
        let new_attempt_id = new_attempt.attempt_id.clone();
        // P0-4：regenerate Attempt 落盘失败不能吞掉，否则后处理会把 Turn
        // 推到 AwaitingAcceptance 却找不到 Attempt，形成无法 accept 的死锁。
        if let Err(e) = self.storage.update_turn_record(request.turn_id, |record| {
            turn_lifecycle::append_regenerate_attempt(record, new_attempt);
        }) {
            if let Err(comp_e) = self
                .conv_store
                .soft_delete_variant(request.conversation_id, request.previous_variant_id)
            {
                tracing::error!(
                    "P0-4 regenerate 补偿失败: soft_delete node {} 失败: {comp_e}（原错误: {e}）",
                    request.previous_variant_id
                );
            }
            let _ = self.storage.update_turn_record(request.turn_id, |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(format!("regenerate TurnAttempt 持久化失败: {e}"));
                record.touch();
            });
            return Err(format!(
                "regenerate TurnAttempt 持久化失败（已尝试软删变体）: {e}"
            ));
        }
        Ok(DraftAttemptOutcome {
            attempt_id: new_attempt_id,
            variant_id: request.previous_variant_id.clone(),
        })
    }

    /// Accept through the backend-owned Turn lifecycle.
    ///
    /// JSON: shared `TurnLifecycleService`. SQLite: atomic production UoW with
    /// conversation cache invalidate (the UoW owns the graph mutation).
    pub fn accept_by_variant(
        &self,
        campaign_id: &Id,
        conversation_id: &Id,
        variant_id: &Id,
        force_accept: bool,
    ) -> Result<AcceptOutcome, AcceptError> {
        if self.storage.is_sqlite() {
            let outcome = crate::sqlite_runtime::accept_by_variant(
                campaign_id,
                conversation_id,
                variant_id,
                force_accept,
            )?;
            self.conv_store.invalidate();
            return Ok(outcome);
        }
        let campaign_store = self
            .storage
            .json_campaign_store(BackendCapability::TurnLifecycle, "accept JSON turn")
            .map_err(AcceptError::Storage)?;
        let turn_store = self
            .storage
            .json_turn_store("accept JSON turn")
            .map_err(AcceptError::Storage)?;
        let service = turn_lifecycle::TurnLifecycleService::new(
            campaign_store,
            turn_store,
            self.conv_store.as_ref(),
        );
        service.accept_by_variant(campaign_id, conversation_id, variant_id, force_accept)
    }

    /// Edit a variant and mark its Attempt Stale when one is linked.
    ///
    /// SQLite: atomic mark-stale preaccept UoW when an Attempt is linked (else
    /// plain conversation edit). JSON: plain edit first, then best-effort
    /// Stale mark (P0-3 draft_hash contract).
    pub fn edit_variant_with_stale_mark(
        &self,
        conversation_id: &Id,
        node_id: &Id,
        new_content: &str,
    ) -> Result<(), String> {
        if self.storage.is_sqlite() {
            if let Some(turn) =
                crate::get_turn_by_variant_for_backend(self.storage.as_ref(), node_id)?
                && let Some(att) = turn.find_attempt_by_variant(node_id)
            {
                crate::sqlite_runtime::mark_stale_after_edit(
                    &turn.campaign_id,
                    &turn.conversation_id,
                    &turn.turn_id,
                    &att.attempt_id,
                    new_content,
                )?;
                self.conv_store.invalidate();
                return Ok(());
            }
            return self
                .conv_store
                .edit_variant(conversation_id, node_id, new_content.to_string())
                .map_err(|e| e.to_string());
        }

        self.conv_store
            .edit_variant(conversation_id, node_id, new_content.to_string())
            .map_err(|e| e.to_string())?;
        // P0-3：编辑后 draft_hash 不再匹配 → 标记关联 Attempt 为 Stale
        // commit_turn_attempt 会因 hash 不匹配拒绝 accept，Stale 是显式信号
        if let Some(turn) = crate::get_turn_by_variant_for_backend(self.storage.as_ref(), node_id)?
        {
            let turn_id = turn.turn_id.clone();
            if let Some(att) = turn.find_attempt_by_variant(node_id) {
                let attempt_id = att.attempt_id.clone();
                let _ = self.storage.update_turn_record(&turn_id, |record| {
                    if let Some(a) = record.find_attempt_mut(&attempt_id) {
                        a.status = storyforge_domain::turn::AttemptStatus::Stale;
                    }
                    record.touch();
                });
            }
        }
        Ok(())
    }
}

/// Routes Attempt/Turn persistence through the active backend (JSON or SQLite).
pub struct BackendTurnAttemptSink<'a> {
    /// Production and backend-routing tests carry the startup-pinned facade.
    /// A test that injects only an isolated JSON store remains explicitly JSON.
    storage: Option<Arc<StorageFacade>>,
    /// Production uses the process-wide store; tests can inject an isolated store so
    /// backend adapter coverage never writes to the user's real AppData directory.
    json_turn_store: Option<&'a TurnStore>,
}

impl<'a> BackendTurnAttemptSink<'a> {
    pub fn production(storage: Arc<StorageFacade>) -> Self {
        Self {
            storage: Some(storage),
            json_turn_store: None,
        }
    }

    #[cfg(test)]
    pub fn for_json_store(json_turn_store: &'a TurnStore) -> Self {
        Self {
            storage: None,
            json_turn_store: Some(json_turn_store),
        }
    }

    #[cfg(test)]
    pub fn for_backend_store(storage: Arc<StorageFacade>, json_turn_store: &'a TurnStore) -> Self {
        Self {
            storage: Some(storage),
            json_turn_store: Some(json_turn_store),
        }
    }

    fn is_sqlite(&self) -> bool {
        self.storage
            .as_ref()
            .is_some_and(|storage| storage.is_sqlite())
    }

    fn json_store(&self) -> Result<&TurnStore, String> {
        match self.json_turn_store {
            Some(store) => Ok(store),
            None => self
                .storage
                .as_deref()
                .ok_or_else(|| "JSON TurnStore was not injected".to_string())?
                .json_turn_store("postprocess Turn mutation"),
        }
    }

    fn mutate_json_if<P, M>(&self, turn_id: &Id, predicate: P, mutate: M) -> Result<bool, String>
    where
        P: FnOnce(&storyforge_domain::turn::TurnRecord) -> bool,
        M: FnOnce(&mut storyforge_domain::turn::TurnRecord),
    {
        self.json_store()?
            .mutate_if(turn_id, predicate, mutate)
            .map_err(|e| format!("条件更新 TurnRecord 失败: {e}"))
    }

    fn mutate_backend_if<P, M>(&self, turn_id: &Id, predicate: P, mutate: M) -> Result<bool, String>
    where
        P: FnOnce(&storyforge_domain::turn::TurnRecord) -> bool,
        M: FnOnce(&mut storyforge_domain::turn::TurnRecord),
    {
        if let Some(storage) = self.storage.as_deref() {
            storage.mutate_turn_if(turn_id, predicate, mutate)
        } else {
            self.mutate_json_if(turn_id, predicate, mutate)
        }
    }
}

impl TurnAttemptSink for BackendTurnAttemptSink<'_> {
    fn load_turn(
        &self,
        turn_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if let Some(storage) = self.storage.as_deref() {
            storage.get_turn(turn_id)
        } else {
            Ok(self.json_store()?.get_turn(turn_id))
        }
    }

    fn sync_autofix(
        &self,
        identity: &crate::production_postprocess::PostprocessIdentity,
        final_text: &str,
        report: storyforge_domain::turn::QualityReport,
    ) -> Result<(), ProductionPostprocessError> {
        self.sync_autofix_with_provenance(identity, final_text, report, None)
    }

    fn sync_autofix_with_provenance(
        &self,
        identity: &crate::production_postprocess::PostprocessIdentity,
        final_text: &str,
        report: storyforge_domain::turn::QualityReport,
        provenance: Option<Provenance>,
    ) -> Result<(), ProductionPostprocessError> {
        use storyforge_domain::turn::{AttemptStatus, TurnStatus};

        if self.is_sqlite() {
            // Typed pre-validation against the authoritative SQLite turn before UoW.
            let storage = self.storage.as_deref().ok_or_else(|| {
                ProductionPostprocessError::AutofixSync(
                    "SQLite postprocess storage was not injected".to_string(),
                )
            })?;
            let turn = storage
                .get_turn(&identity.turn_id)
                .map_err(ProductionPostprocessError::AutofixSync)?
                .ok_or_else(|| ProductionPostprocessError::AttemptMissing {
                    turn_id: identity.turn_id.to_string(),
                    attempt_id: identity.attempt_id.to_string(),
                })?;
            if turn.campaign_id != identity.campaign_id {
                return Err(ProductionPostprocessError::ScopeMismatch {
                    field: "campaign_id",
                    expected: identity.campaign_id.to_string(),
                    actual: turn.campaign_id.to_string(),
                });
            }
            if turn.conversation_id != identity.conversation_id {
                return Err(ProductionPostprocessError::ScopeMismatch {
                    field: "conversation_id",
                    expected: identity.conversation_id.to_string(),
                    actual: turn.conversation_id.to_string(),
                });
            }
            if turn.find_attempt(&identity.attempt_id).is_none() {
                return Err(ProductionPostprocessError::AttemptMissing {
                    turn_id: identity.turn_id.to_string(),
                    attempt_id: identity.attempt_id.to_string(),
                });
            }
            let writable = matches!(
                turn.status,
                TurnStatus::DraftReady | TurnStatus::DerivingState
            ) && turn
                .find_attempt(&identity.attempt_id)
                .is_some_and(|attempt| {
                    matches!(
                        attempt.status,
                        AttemptStatus::DraftReady | AttemptStatus::DerivingState
                    )
                });
            if !writable {
                // Concurrent supersede / late status: durable zero-write, non-fatal.
                return Ok(());
            }
            return crate::sqlite_runtime::sync_autofix(AutofixSyncRequest {
                campaign_id: &identity.campaign_id,
                conversation_id: &identity.conversation_id,
                turn_id: &identity.turn_id,
                attempt_id: &identity.attempt_id,
                final_text,
                quality_report: report,
                provenance: provenance.clone(),
            })
            .map_err(ProductionPostprocessError::AutofixSync);
        }

        // Capture typed validation under the same conditional durable mutation.
        let mut precondition: Option<ProductionPostprocessError> = None;
        let mut not_writable = false;
        let applied = self
            .mutate_json_if(
                &identity.turn_id,
                |record| {
                    if record.campaign_id != identity.campaign_id {
                        precondition = Some(ProductionPostprocessError::ScopeMismatch {
                            field: "campaign_id",
                            expected: identity.campaign_id.to_string(),
                            actual: record.campaign_id.to_string(),
                        });
                        return false;
                    }
                    if record.conversation_id != identity.conversation_id {
                        precondition = Some(ProductionPostprocessError::ScopeMismatch {
                            field: "conversation_id",
                            expected: identity.conversation_id.to_string(),
                            actual: record.conversation_id.to_string(),
                        });
                        return false;
                    }
                    if record.find_attempt(&identity.attempt_id).is_none() {
                        precondition = Some(ProductionPostprocessError::AttemptMissing {
                            turn_id: identity.turn_id.to_string(),
                            attempt_id: identity.attempt_id.to_string(),
                        });
                        return false;
                    }
                    let writable = matches!(
                        record.status,
                        TurnStatus::DraftReady | TurnStatus::DerivingState
                    ) && record.find_attempt(&identity.attempt_id).is_some_and(
                        |attempt| {
                            matches!(
                                attempt.status,
                                AttemptStatus::DraftReady | AttemptStatus::DerivingState
                            )
                        },
                    );
                    if !writable {
                        not_writable = true;
                        return false;
                    }
                    true
                },
                |record| {
                    if let Some(att) = record.find_attempt_mut(&identity.attempt_id) {
                        turn_lifecycle::sync_attempt_after_autofix(att, final_text, report);
                        if let Some(provenance) = provenance {
                            att.provenance = Some(provenance);
                        }
                    }
                    record.touch();
                },
            )
            .map_err(ProductionPostprocessError::AutofixSync)?;
        if applied {
            Ok(())
        } else if let Some(err) = precondition {
            Err(err)
        } else if not_writable {
            // Concurrent supersede / late status: durable zero-write, non-fatal.
            Ok(())
        } else {
            Ok(())
        }
    }

    fn attach_postprocess(
        &self,
        identity: &crate::production_postprocess::PostprocessIdentity,
        batch: Option<storyforge_domain::turn::MutationBatch>,
        derivation: storyforge_domain::turn::DerivationComponents,
    ) -> Result<bool, String> {
        if self.is_sqlite() {
            self.storage
                .as_deref()
                .ok_or_else(|| "SQLite postprocess storage was not injected".to_string())?;
            return match crate::sqlite_runtime::apply_postprocess(PostprocessApplyRequest {
                campaign_id: &identity.campaign_id,
                conversation_id: &identity.conversation_id,
                turn_id: &identity.turn_id,
                attempt_id: &identity.attempt_id,
                batch,
                derivation,
            })? {
                PostprocessApplyOutcome::Applied | PostprocessApplyOutcome::AlreadyApplied => {
                    Ok(true)
                }
                PostprocessApplyOutcome::SkippedLate => Ok(false),
            };
        }
        self.mutate_backend_if(
            &identity.turn_id,
            |record| {
                record.campaign_id == identity.campaign_id
                    && record.conversation_id == identity.conversation_id
                    && crate::runtime_support::is_current_attempt_ready_for_postprocess(
                        record,
                        &identity.attempt_id,
                    )
            },
            |record| {
                if let Some(att) = record.find_attempt_mut(&identity.attempt_id) {
                    turn_lifecycle::apply_postprocess_to_attempt(att, batch, derivation);
                }
                record.status = storyforge_domain::turn::TurnStatus::AwaitingAcceptance;
                record.touch();
            },
        )
    }

    fn mark_failed_if_current(
        &self,
        identity: &crate::production_postprocess::PostprocessIdentity,
        reason: String,
    ) -> Result<bool, String> {
        self.mutate_backend_if(
            &identity.turn_id,
            |record| {
                record.campaign_id == identity.campaign_id
                    && record.conversation_id == identity.conversation_id
                    && crate::runtime_support::is_current_attempt_ready_for_postprocess(
                        record,
                        &identity.attempt_id,
                    )
            },
            |record| {
                record.status = storyforge_domain::turn::TurnStatus::Failed;
                record.failure_reason = Some(reason);
                record.touch();
            },
        )
    }
}

/// Construct the production postprocess service for the pinned backend.
///
/// SQLite requires the CampaignRuntimeContext snapshot (no JSON store fallback);
/// JSON reads through its CampaignStore. Selection happens once per postprocess
/// run at the adapter boundary, never in commands.
pub fn build_postprocess_service<'a>(
    storage: &'a StorageFacade,
    runtime: Option<&'a CampaignRuntimeContext>,
    sink: &'a dyn TurnAttemptSink,
) -> Result<ProductionPostprocessService<'a>, ProductionPostprocessError> {
    if storage.is_sqlite() {
        let runtime = runtime.ok_or_else(|| {
            ProductionPostprocessError::BatchConstruction(
                "sqlite postprocess has no CampaignRuntimeContext; refusing JSON fallback".into(),
            )
        })?;
        Ok(ProductionPostprocessService::new_runtime(runtime, sink))
    } else {
        let json_store = storage
            .json_campaign_store(
                BackendCapability::Postprocess,
                "apply JSON postprocess outcome",
            )
            .map_err(ProductionPostprocessError::BatchConstruction)?;
        Ok(ProductionPostprocessService::new_json(json_store, sink))
    }
}

/// Persist the active Campaign pointer. JSON writes `active_campaign.json`;
/// SQLite keeps the pointer in-process (Degraded `ActiveCampaignPersistence`).
pub fn save_active_pointer(
    storage: &StorageFacade,
    campaign_id: Option<&Id>,
) -> Result<(), String> {
    storage.save_active_pointer(campaign_id)
}

// ─── Start-conversation preparation ───────────────────────────────────────

pub struct StartConversationTarget {
    pub conversation_id: Id,
    pub regex_character_id: Option<String>,
    /// Phase A: 追加的 user 消息节点 ID（TurnRecord.input_node_id 用）
    pub input_node_id: Option<Id>,
}

pub async fn prepare_start_conversation_async(
    state: Arc<crate::AppState>,
    campaign_store: Option<&'static CampaignStore>,
    requested_conversation_id: Option<String>,
    character_id: Option<String>,
    legacy_opening_character: Option<Arc<storyforge_domain::character::Character>>,
    opening_message: Option<String>,
    intent: String,
) -> Result<StartConversationTarget, crate::error::TauriCommandError> {
    tokio::task::spawn_blocking(move || {
        prepare_start_conversation(
            state,
            campaign_store,
            requested_conversation_id,
            character_id,
            legacy_opening_character,
            opening_message,
            intent,
        )
    })
    .await
    .map_err(|e| crate::error::TauriCommandError::internal(format!("准备写作对话任务失败: {e}")))?
}

pub fn prepare_start_conversation(
    state: Arc<crate::AppState>,
    campaign_store: Option<&CampaignStore>,
    requested_conversation_id: Option<String>,
    character_id: Option<String>,
    legacy_opening_character: Option<Arc<storyforge_domain::character::Character>>,
    opening_message: Option<String>,
    intent: String,
) -> Result<StartConversationTarget, crate::error::TauriCommandError> {
    use crate::error::TauriCommandError;
    let sqlite_backend = state.storage().is_sqlite();
    let campaign_conv_id: Option<Id> = {
        let active = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if sqlite_backend {
            match active.as_ref() {
                Some(campaign_id) => {
                    state
                        .storage()
                        .get_campaign(campaign_id)
                        .map_err(TauriCommandError::internal)?
                        .ok_or_else(|| {
                            TauriCommandError::internal(format!(
                                "sqlite campaign {} missing while preparing writing",
                                campaign_id
                            ))
                        })?
                        .campaign
                        .conversation_id
                }
                None => None,
            }
        } else {
            match campaign_store {
                Some(campaign_store) => active
                    .as_ref()
                    .and_then(|cid| campaign_store.get_campaign(cid))
                    .and_then(|campaign| campaign.conversation_id.clone()),
                None => active
                    .as_ref()
                    .and_then(|cid| {
                        state
                            .storage()
                            .json_campaign_store(
                                BackendCapability::CampaignRead,
                                "prepare start conversation",
                            )
                            .ok()?
                            .get_campaign(cid)
                    })
                    .and_then(|campaign| campaign.conversation_id.clone()),
            }
        }
    };
    let conversation_id = campaign_conv_id
        .map(|cid| cid.as_str().to_string())
        .or(requested_conversation_id);

    let (conversation_id, input_node_id) = if let Some(id_str) = conversation_id {
        let id = Id::from_str(&id_str);
        match state.conv_store.append_user_message(&id, intent.clone()) {
            Ok(node_id) => (id, Some(node_id)),
            Err(e) => {
                if sqlite_backend {
                    return Err(TauriCommandError::internal(format!(
                        "sqlite user message persistence failed: {e}"
                    )));
                }
                tracing::warn!("追加 user 消息失败: {e}");
                (id, None)
            }
        }
    } else {
        let conv = if sqlite_backend {
            state
                .conv_store
                .create_persisted(character_id.clone(), None)
                .map_err(|e| {
                    TauriCommandError::internal(format!("sqlite conversation creation failed: {e}"))
                })?
        } else {
            state.conv_store.create(character_id.clone(), None)
        };
        let id = conv.id.clone();
        let legacy_opening = crate::runtime_support::resolve_legacy_opening_message(
            legacy_opening_character.as_ref(),
            opening_message,
        );
        if let Some(opening) = legacy_opening
            && let Err(e) =
                state
                    .conv_store
                    .append_final_message(&id, ConversationRole::Assistant, opening)
        {
            if sqlite_backend {
                return Err(TauriCommandError::internal(format!(
                    "sqlite opening message persistence failed: {e}"
                )));
            }
            tracing::warn!("追加开场白失败: {e}");
        }
        let node_id = match state.conv_store.append_user_message(&id, intent.clone()) {
            Ok(node_id) => Some(node_id),
            Err(e) if sqlite_backend => {
                return Err(TauriCommandError::internal(format!(
                    "sqlite user message persistence failed: {e}"
                )));
            }
            Err(e) => {
                tracing::warn!("追加 user 消息失败: {e}");
                None
            }
        };
        (id, node_id)
    };

    let regex_character_id = character_id.or_else(|| {
        state
            .conv_store
            .get(&conversation_id)
            .and_then(|c| c.character_id)
    });

    Ok(StartConversationTarget {
        conversation_id,
        regex_character_id,
        input_node_id,
    })
}

// ─── Campaign instance DTO enrichment ─────────────────────────────────────

/// Build the display DTO for one CharacterInstance through the pinned backend.
/// Both backends enrich `role_type` from the card definitions (SQLite reads
/// the card payload from the process authority).
pub fn character_instance_dto_for_backend(
    storage: &StorageFacade,
    instance: &storyforge_domain::campaign::CharacterInstance,
) -> Result<crate::commands::campaigns::CharacterInstanceDto, String> {
    if storage.is_json() {
        let store = storage.json_campaign_store(
            BackendCapability::CampaignInstanceRead,
            "enrich campaign instance DTO",
        )?;
        return Ok(character_instance_dto_from_store(store, instance));
    }
    use crate::commands::campaigns::CharacterInstanceDto;
    let role_type = instance.definition_id.as_ref().and_then(|definition_id| {
        let campaign = crate::sqlite_runtime::get_campaign(&instance.campaign_id).ok()??;
        let payload = crate::sqlite_runtime::get_card_payload(&campaign.card_id).ok()??;
        let stored = serde_json::from_value::<crate::campaign_store::StoredCard>(payload).ok()?;
        stored
            .card
            .character_definitions
            .iter()
            .find(|definition| definition.id == *definition_id)
            .map(|definition| definition.role_type.clone())
    });
    Ok(match role_type.as_ref() {
        Some(role_type) => CharacterInstanceDto::with_role_type(instance, role_type),
        None => CharacterInstanceDto::from(instance),
    })
}

pub fn character_instance_dto_from_store(
    store: &CampaignStore,
    instance: &storyforge_domain::campaign::CharacterInstance,
) -> crate::commands::campaigns::CharacterInstanceDto {
    use crate::commands::campaigns::CharacterInstanceDto;
    let role_type = instance.definition_id.as_ref().and_then(|definition_id| {
        let campaign = store.get_campaign(&instance.campaign_id)?;
        let card = store.get_card(&campaign.card_id)?;
        card.card
            .character_definitions
            .iter()
            .find(|definition| definition.id == *definition_id)
            .map(|definition| definition.role_type.clone())
    });
    match role_type.as_ref() {
        Some(role_type) => CharacterInstanceDto::with_role_type(instance, role_type),
        None => CharacterInstanceDto::from(instance),
    }
}

// ─── Conversation card-name lookup ────────────────────────────────────────

/// Resolve card names for conversation summaries through the pinned backend.
///
/// JSON builds one `card_id → name` snapshot from the CampaignStore and
/// resolves every summary against it (by `character_id` first, then campaign
/// → card fallback) — a single `list_cards()` per call, not per conversation.
/// SQLite reads one `character_cards` snapshot from the process authority.
pub fn conversation_card_names_for_backend(
    storage: &StorageFacade,
    summaries: &[storyforge_app_conversation::ConversationSummary],
) -> Result<std::collections::HashMap<String, String>, String> {
    let card_by_id: std::collections::HashMap<Id, String> = if storage.is_sqlite() {
        crate::sqlite_runtime::list_card_names()?
    } else {
        let store =
            storage.json_campaign_store(BackendCapability::CampaignRead, "list conversations")?;
        store
            .list_cards()
            .iter()
            .map(|sc| (sc.card.id.clone(), sc.card.name.clone()))
            .collect()
    };
    let mut out = std::collections::HashMap::new();
    for summary in summaries {
        let card_name = summary
            .character_id
            .as_ref()
            .and_then(|cid| {
                // 首选:直接按 character_id(=CharacterCard.id)查卡名
                card_by_id.get(&Id::from_str(cid)).cloned()
            })
            .or_else(|| {
                // 兜底:campaign_id → campaign.card_id → card.name
                summary.campaign_id.as_ref().and_then(|camp_id| {
                    storage
                        .get_campaign(camp_id)
                        .ok()
                        .flatten()
                        .and_then(|record| card_by_id.get(&record.campaign.card_id).cloned())
                })
            });
        if let Some(name) = card_name {
            out.insert(summary.id.as_str().to_string(), name);
        }
    }
    Ok(out)
}

/// Campaign-scoped regex scripts through the pinned backend.
///
/// JSON reads the CampaignStore; SQLite reads the campaign card payload and
/// resolves the card's scoped regex scripts the same way (`scoped_regex_scripts()`).
pub fn campaign_scoped_regex_scripts_for_backend(
    storage: &StorageFacade,
    campaign_id: &Id,
) -> Result<Option<Vec<RegexScript>>, String> {
    if storage.is_json() {
        return Ok(Some(collect_campaign_scoped_regex_scripts(
            campaign_id,
            storage.json_campaign_store(
                BackendCapability::CampaignRead,
                "conversation regex context",
            )?,
        )));
    }
    let scripts = crate::sqlite_runtime::get_campaign(campaign_id)?
        .and_then(|campaign| crate::sqlite_runtime::get_card_payload(&campaign.card_id).ok()?)
        .and_then(|payload| {
            serde_json::from_value::<crate::campaign_store::StoredCard>(payload).ok()
        })
        .map(|stored| stored.card.scoped_regex_scripts())
        .unwrap_or_default();
    Ok(Some(scripts))
}

// ─── Campaign health ──────────────────────────────────────────────────────

/// Deterministic campaign health check through the pinned backend.
///
/// JSON: shared `check_campaign_health` snapshot algorithm with a
/// "Campaign 不存在" error for missing campaigns. SQLite: fail-closed typed
/// reads via `meta_backend::sqlite_campaign_health_issues`.
pub fn campaign_health_issues_for_backend(
    storage: &StorageFacade,
    campaign_id: &Id,
) -> Result<Vec<serde_json::Value>, String> {
    use storyforge_app_meta::{CampaignHealthSnapshot, check_campaign_health};
    if storage.is_sqlite() {
        return crate::meta_backend::sqlite_campaign_health_issues(campaign_id)?
            .into_iter()
            .map(|issue| {
                serde_json::to_value(&issue)
                    .map_err(|e| format!("campaign health issue 序列化失败: {e}"))
            })
            .collect();
    }
    let store =
        storage.json_campaign_store(BackendCapability::CampaignHealth, "campaign health check")?;
    // 确认 campaign 存在
    let campaign = store
        .get_campaign(campaign_id)
        .ok_or_else(|| format!("Campaign 不存在: {campaign_id}"))?;
    // 获取关联的 card → definitions
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();
    let instances = store.list_instances(campaign_id);
    let knowledge = store.list_knowledge(campaign_id);
    let tasks = store.list_tasks(campaign_id);
    let snapshot = CampaignHealthSnapshot {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
    };
    check_campaign_health(&snapshot)
        .into_iter()
        .map(|issue| {
            serde_json::to_value(&issue)
                .map_err(|e| format!("campaign health issue 序列化失败: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()
}

// ─── Campaign runtime context loading ─────────────────────────────────────

/// Load the CampaignRuntimeContext snapshot through the pinned backend.
///
/// JSON: CampaignStore snapshot loader. SQLite: typed SQLite snapshot loader.
/// The caller still resolves the active campaign id (memory + legacy pointer).
pub fn load_campaign_context_snapshot_for_backend(
    storage: &StorageFacade,
    active_id: &Id,
) -> Result<Option<crate::runtime_support::CampaignContextSnapshot>, String> {
    if storage.is_sqlite() {
        return load_sqlite_campaign_context_snapshot(storage, active_id);
    }
    let store =
        storage.json_campaign_store(BackendCapability::CampaignRead, "fill campaign context")?;
    Ok(load_campaign_context_snapshot(store, active_id))
}

/// Fill WritingContext from the process-owned SQLite authority (opt-in only).
pub fn fill_campaign_runtime_from_sqlite(
    storage: &StorageFacade,
    ctx: &mut WritingContext,
    tool_ctx: &Arc<std::sync::RwLock<ToolContext>>,
    active_id: &Id,
) -> Result<(), String> {
    if !storage.is_sqlite() {
        return Err("fill_campaign_runtime_from_sqlite requires SQLite backend".into());
    }
    match load_sqlite_campaign_context_snapshot(storage, active_id)? {
        Some(snapshot) => {
            crate::runtime_support::apply_campaign_context_snapshot(ctx, tool_ctx, snapshot);
            Ok(())
        }
        None => Ok(()),
    }
}

// ─── MVU collection dispatchers ───────────────────────────────────────────

/// Route MVU fallback reads through the startup-pinned backend. SQLite read or
/// decode failures are propagated so corrupt authority data cannot look like
/// a legitimate empty optional-snippet set.
pub fn collect_mvu_fallback_fragments_for_backend(
    storage: &StorageFacade,
    ctx: &WritingContext,
    present_chars: &[String],
) -> Result<Vec<storyforge_domain::mvu_translation::FallbackFragment>, String> {
    if storage.is_sqlite() {
        // #22：SQLite 已是 MVU 翻译权威（V005 表 + importer 迁移），直接读它。
        collect_mvu_from_sqlite(storage, ctx, present_chars, false, |stored| {
            stored
                .translation
                .fallback_fragments
                .into_iter()
                .filter(|f| !f.js_snippet.is_empty())
                .collect()
        })
    } else {
        Ok(crate::runtime_support::collect_mvu_fallback_fragments(
            ctx,
            storage.json_campaign_store(
                BackendCapability::MvuTranslation,
                "collect MVU fallback fragments",
            )?,
            present_chars,
        ))
    }
}

/// #22：SQLite 后端的 MVU 收集骨架——与 JSON 版同语义：
/// present instance → definition_id → source 卡（def→source 反查表来自
/// character_cards payload）→ mvu_translations 表取翻译，`extract` 挑字段。
/// `dedup_sources=true` 时同一 source 卡只贡献一次（规则收集用）。
pub fn collect_mvu_from_sqlite<T>(
    storage: &StorageFacade,
    ctx: &WritingContext,
    present_chars: &[String],
    dedup_sources: bool,
    mut extract: impl FnMut(crate::campaign_store::StoredMvuTranslation) -> Vec<T>,
) -> Result<Vec<T>, String> {
    let runtime = match &ctx.campaign_runtime {
        Some(rt) => rt,
        None => return Ok(vec![]),
    };
    let payloads = storage
        .list_card_payloads()
        .map_err(|error| format!("SQLite MVU card payload lookup failed: {error}"))?;
    // payload 兼容两种形态：StoredCard 包装（生产写入）/ 裸 CharacterCard（旧 cutover 源）
    let mut def_to_source = std::collections::HashMap::<Id, Id>::new();
    for value in payloads {
        let inner = value.get("card").unwrap_or(&value);
        let card =
            serde_json::from_value::<storyforge_domain::character::CharacterCard>(inner.clone())
                .map_err(|error| format!("SQLite MVU card payload decode failed: {error}"))?;
        def_to_source.extend({
            let src = card.source_character_id.clone();
            card.character_definitions
                .into_iter()
                .map(move |d| (d.id, src.clone()))
        });
    }

    let mut visited_sources = std::collections::HashSet::new();
    let mut out = Vec::new();
    for char_id_str in present_chars {
        let char_id = Id::from_str(char_id_str);
        let inst = runtime
            .instances
            .iter()
            .find(|i| i.id == char_id || i.name == *char_id_str);
        let inst = match inst {
            Some(i) => i,
            None => continue,
        };
        let def_id = match &inst.definition_id {
            Some(d) => d,
            None => continue,
        };
        let source_id = match def_to_source.get(def_id) {
            Some(s) => s,
            None => continue,
        };
        if dedup_sources && !visited_sources.insert(source_id.clone()) {
            continue;
        }
        if let Some(stored) = storage
            .get_mvu(source_id)
            .map_err(|error| format!("SQLite MVU translation lookup failed: {error}"))?
        {
            let items = extract(stored);
            if !items.is_empty() {
                tracing::info!(
                    target: "tauri-app",
                    "[MVU] 角色 '{}' 所属卡贡献 {} 条 MVU 产物（SQLite）",
                    inst.name,
                    items.len()
                );
                out.extend(items);
            }
        }
    }
    Ok(out)
}

/// 后端分流：SQLite 活跃时读 mvu_translations 表（V005 起为权威），
/// 否则读 JSON CampaignStore——两条路径同语义（按 source 卡去重）。
pub fn collect_mvu_update_rules_for_backend(
    storage: &StorageFacade,
    ctx: &WritingContext,
    present_chars: &[String],
) -> Result<Vec<String>, String> {
    if storage.is_sqlite() {
        // #22：SQLite 已是 MVU 翻译权威，规则收集不再空转（按 source 卡去重）。
        collect_mvu_from_sqlite(storage, ctx, present_chars, true, |stored| {
            stored
                .translation
                .update_rules
                .into_iter()
                .filter(|r| !r.trim().is_empty())
                .collect()
        })
    } else {
        Ok(crate::runtime_support::collect_mvu_update_rules(
            ctx,
            storage.json_campaign_store(
                BackendCapability::MvuTranslation,
                "collect MVU update rules",
            )?,
            present_chars,
        ))
    }
}

// ─── Postprocess outcome persistence ──────────────────────────────────────

pub async fn persist_postprocess_outcome_async(
    storage: Arc<StorageFacade>,
    ctx: &WritingContext,
    outcome: storyforge_app_agent::PostProcessOutcome,
    present_chars: Vec<String>,
) -> Result<(), String> {
    use crate::runtime_support::PostprocessPersistContext;
    let Some(persist_ctx) = PostprocessPersistContext::from_writing_context(ctx) else {
        return Ok(());
    };
    if storage.is_sqlite() {
        return Err(
            "refusing legacy JSON postprocess persistence while SQLite is authoritative".into(),
        );
    }

    let store = storage.json_campaign_store_owned(
        BackendCapability::Postprocess,
        "persist postprocess outcome",
    )?;

    tokio::task::spawn_blocking(move || {
        crate::runtime_support::persist_postprocess_outcome_to_store(
            store.as_ref(),
            &persist_ctx,
            &outcome,
            &present_chars,
        );
    })
    .await
    .map_err(|error| format!("保存后处理结果的阻塞任务失败: {error}"))?;
    Ok(())
}

// ─── Typed Meta patch snapshot load (Gate 4 backend-neutral reads) ────────

/// Load the pure `app-meta` snapshot for a Campaign through the pinned backend.
/// JSON reads the CampaignStore; SQLite reads the process-owned authority and
/// decodes the StoredCard payload. Both produce the same domain snapshot shape
/// consumed by `check_campaign_health` / `build_patch_for_issue` /
/// `validate_patch_preconditions`.
pub fn load_meta_snapshot_for_backend(
    storage: &StorageFacade,
    campaign_id: &Id,
) -> Result<crate::commands::meta_typed::MetaSnapshot, String> {
    if storage.is_json() {
        let store =
            storage.json_campaign_store(BackendCapability::TypedMetaPatch, "load Meta snapshot")?;
        return crate::commands::meta_typed::load_meta_snapshot_from_store(store, campaign_id)
            .map_err(|e| e.to_string());
    }
    let campaign = storage
        .get_campaign(campaign_id)?
        .map(|record| record.campaign)
        .ok_or_else(|| format!("Campaign 不存在: {campaign_id}"))?;
    let definitions = match storage.get_card_payload(&campaign.card_id)? {
        Some(payload) => serde_json::from_value::<crate::campaign_store::StoredCard>(payload)
            .map(|stored| stored.card.character_definitions)
            .map_err(|e| format!("invalid SQLite card payload: {e}"))?,
        None => Vec::new(),
    };
    let instances = storage.list_instances(campaign_id)?;
    let knowledge = storage.list_knowledge(campaign_id)?;
    let tasks = storage.list_tasks(campaign_id)?;
    Ok(crate::commands::meta_typed::MetaSnapshot {
        campaign,
        definitions,
        instances,
        knowledge,
        tasks,
    })
}

// ─── Typed Meta patch apply (Gate 4 SQLite-native atomic UoW) ─────────────

/// Apply every action of one typed Meta patch through the pinned backend.
///
/// JSON: the legacy per-action `apply_typed_action` loop under the shared
/// campaign commit lock. SQLite: a single atomic UoW
/// (`SqliteMetaRepository`) with scope/stale/definition validation and fault
/// injection points. Both paths reject when any single action fails — nothing
/// is partially applied.
pub fn apply_typed_patch_actions_for_backend(
    storage: &StorageFacade,
    campaign_id: &Id,
    actions: &[storyforge_app_meta::TypedPatchAction],
    expected_revision: Option<u64>,
) -> Result<(), String> {
    if storage.is_sqlite() {
        // SQLite：expected_revision（提案盖章）在事务内与最新 revision 比对
        // （Gate 4 评审 P1-2：revision/barrier 检查与写入原子化）。
        crate::sqlite_runtime::meta_apply_typed_patch_actions(
            campaign_id,
            actions,
            expected_revision,
        )
    } else {
        let store = storage
            .json_campaign_store(BackendCapability::TypedMetaPatch, "accept typed Meta patch")?;
        crate::turn_coordinator::with_campaign_lock(|| {
            for (index, action) in actions.iter().enumerate() {
                crate::commands::meta_typed::apply_typed_action(store, campaign_id, action)
                    .map_err(|error| {
                        crate::turn_coordinator::CommitError::Storage(format!(
                            "第 {} 个 action 失败: {error}",
                            index + 1
                        ))
                    })?;
            }
            Ok(())
        })
        .map_err(|error| error.to_string())
    }
}

// ─── MVU schema apply (Gate 4 SQLite-native atomic UoW) ───────────────────

/// Preview MVU schema merges through the pinned backend.
/// JSON reads the CampaignStore; SQLite reads the `mvu_translations` table and
/// resolves the source card from `character_cards`. Both produce the same
/// `MvuApplyPreview` list (one per definition) via the shared pure function.
pub fn preview_mvu_apply_for_backend(
    storage: &StorageFacade,
    source_character_id: &Id,
) -> Result<Vec<storyforge_app_meta::MvuApplyPreview>, String> {
    let mvu = storage.get_mvu(source_character_id)?.ok_or_else(|| {
        storyforge_app_meta::MvuApplyError::TranslationNotFound(
            source_character_id.as_str().to_string(),
        )
        .to_string()
    })?;
    let stored_card = if storage.is_sqlite() {
        let payload = crate::sqlite_runtime::get_card_payload_by_source(source_character_id)?
            .ok_or_else(|| format!("找不到 source_character_id={source_character_id} 的 card"))?;
        serde_json::from_value::<crate::campaign_store::StoredCard>(payload)
            .map_err(|e| format!("invalid SQLite card payload: {e}"))?
    } else {
        storage
            .json_campaign_store(
                BackendCapability::MvuSchemaApply,
                "MVU schema apply preview",
            )?
            .get_card_by_source(source_character_id)
            .ok_or_else(|| format!("找不到 source_character_id={source_character_id} 的 card"))?
    };
    let previews: Vec<storyforge_app_meta::MvuApplyPreview> = stored_card
        .card
        .character_definitions
        .iter()
        .map(|def| {
            storyforge_app_meta::compute_apply_preview(
                &def.variable_schema,
                &mvu.translation.variable_schema,
                def.id.as_str(),
                &def.name,
                source_character_id.as_str(),
            )
        })
        .collect();
    Ok(previews)
}

/// Apply the MVU schema through the pinned backend.
///
/// JSON: `update_card` + per-instance backfill (existing path). SQLite: one
/// atomic UoW covering the card payload update and every cross-campaign
/// instance backfill, with ownership validation and fault-injection rollback.
pub fn apply_mvu_schema_for_backend(
    storage: &StorageFacade,
    source_character_id: &Id,
    definition_id: &Id,
) -> Result<(), String> {
    if storage.is_sqlite() {
        crate::sqlite_runtime::mvu_apply_schema(source_character_id, definition_id)?;
        Ok(())
    } else {
        let store =
            storage.json_campaign_store(BackendCapability::MvuSchemaApply, "MVU schema apply")?;
        crate::commands::meta_typed::meta_apply_mvu_schema_in_store(
            store,
            source_character_id,
            definition_id,
        )
        .map_err(|e| e.to_string())
    }
}

// ─── Campaign world info (Gate 4 SQLite-native) ──────────────────────────

/// Load the campaign world info book through the pinned backend, lazily seeding
/// it from the card template when the campaign book is still empty (JSON: the
/// CharacterStore template; SQLite: `raw_card_json.character_book` in the card
/// payload). Never silently returns an empty book when a template exists.
pub fn load_campaign_world_info_for_backend(
    storage: &StorageFacade,
    state: &crate::AppState,
    campaign_id: &Id,
) -> Result<storyforge_domain::world_info::WorldInfoBook, String> {
    let mut book = storage.get_world_info(campaign_id)?;
    if !book.entries.is_empty() {
        return Ok(book);
    }
    let template = if storage.is_sqlite() {
        let card_payload = storage.get_card_payload(&campaign_card_id(storage, campaign_id)?)?;
        match card_payload {
            Some(payload) => storage
                .template_world_info_from_card(&payload)?
                .unwrap_or_else(empty_world_info_book_for_template),
            None => empty_world_info_book_for_template(),
        }
    } else {
        let store = storage.json_campaign_store(
            BackendCapability::CampaignRead,
            "seed campaign world info from card template",
        )?;
        let campaign = store
            .get_campaign(campaign_id)
            .ok_or_else(|| format!("Campaign 不存在: {campaign_id}"))?;
        let card = store
            .get_card(&campaign.card_id)
            .ok_or_else(|| format!("Campaign card 不存在: {}", campaign.card_id))?;
        let character_store = state
            .json_character_store(
                BackendCapability::CharacterCommands,
                "seed campaign world info from character template",
            )
            .map_err(|e| e.to_string())?;
        crate::commands::campaigns::resolve_template_world_info_for_card(character_store, &card)
    };
    if !template.entries.is_empty() {
        book = storage.ensure_world_info_from_book(campaign_id, &template)?;
    }
    Ok(book)
}

fn campaign_card_id(
    storage: &StorageFacade,
    campaign_id: &Id,
) -> Result<storyforge_domain::Id, String> {
    storage
        .get_campaign(campaign_id)?
        .map(|record| record.campaign.card_id)
        .ok_or_else(|| format!("Campaign 不存在: {campaign_id}"))
}

fn empty_world_info_book_for_template() -> storyforge_domain::world_info::WorldInfoBook {
    storyforge_domain::world_info::WorldInfoBook {
        entries: Vec::new(),
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}

// ─── Chronicle compressor ─────────────────────────────────────────────────

/// 统计 campaign 未覆盖 A/B 数量（JSON store 版；SQLite 版见
/// `SqliteCompressJobRepository::count_uncovered`）。
pub fn count_uncovered_chronicle_levels(store: &CampaignStore, campaign_id: &Id) -> (usize, usize) {
    let entries = store.list_summaries(campaign_id);
    let uncovered_a = entries
        .iter()
        .filter(|s| s.covered_by.is_none() && s.is_leaf_a())
        .count();
    let uncovered_b = entries
        .iter()
        .filter(|s| {
            s.covered_by.is_none()
                && s.chronicle_level() == storyforge_domain::chronicle::ChronicleLevel::B
        })
        .count();
    (uncovered_a, uncovered_b)
}

/// 未覆盖 A/B 数量（pinned backend 分派；SQLite 读权威表）。
pub fn count_uncovered_chronicle_levels_for_backend(
    storage: &StorageFacade,
    campaign_id: &Id,
) -> Result<(usize, usize), String> {
    if storage.is_sqlite() {
        crate::sqlite_runtime::compress_count_uncovered(campaign_id)
    } else {
        let store = storage.json_campaign_store(
            BackendCapability::ChronicleCompressor,
            "count uncovered chronicle levels",
        )?;
        Ok(count_uncovered_chronicle_levels(store, campaign_id))
    }
}

/// Accept 成功后：达阈值则**持久化入队**，再 spawn worker 消费 job。
/// SQLite 走 V006 `chronicle_compress_jobs` 表；JSON 走 CompressJobStore。
pub fn maybe_spawn_chronicle_compress(state: Arc<crate::AppState>, campaign_id: Id) {
    let storage = state.storage().clone();
    let (uncovered_a, uncovered_b) =
        match count_uncovered_chronicle_levels_for_backend(&storage, &campaign_id) {
            Ok(counts) => counts,
            Err(error) => {
                tracing::error!(
                    target: "chronicle_compressor",
                    campaign_id = %campaign_id,
                    "chronicle uncovered count unavailable: {error}"
                );
                return;
            }
        };
    let need_a = storyforge_domain::chronicle::should_enqueue_compress(
        uncovered_a,
        storyforge_domain::chronicle::DEFAULT_COMPRESS_ACTIVE_A_THRESHOLD,
    );
    let need_b = storyforge_domain::chronicle::should_enqueue_compress(
        uncovered_b,
        storyforge_domain::chronicle::DEFAULT_COMPRESS_ACTIVE_B_THRESHOLD,
    );
    if !need_a && !need_b {
        return;
    }
    let enqueue = || -> Result<(), String> {
        if storage.is_sqlite() {
            let campaign = crate::sqlite_runtime::get_campaign(&campaign_id)?
                .ok_or_else(|| format!("campaign {campaign_id} missing"))?;
            let (job, created) = crate::sqlite_runtime::compress_enqueue_or_get_open(
                &campaign_id,
                campaign.conversation_id.clone(),
                campaign.lineage_id.clone(),
                uncovered_a as u32,
                uncovered_b as u32,
            )?;
            tracing::info!(
                target: "chronicle_compressor",
                campaign_id = %campaign_id,
                job_id = %job.id,
                created,
                uncovered_a,
                uncovered_b,
                "compress job enqueued (SQLite)"
            );
            if created || job.status == crate::sqlite_compress_jobs::CompressJobStatus::Pending {
                spawn_compress_job_worker(state.clone(), job.id);
            }
            return Ok(());
        }
        let store = storage.json_campaign_store(
            BackendCapability::ChronicleCompressor,
            "enqueue chronicle compressor",
        )?;
        let job_store = storage.json_compress_job_store("enqueue chronicle compression")?;
        let camp = store.get_campaign(&campaign_id);
        let conversation_id = camp.as_ref().and_then(|c| c.conversation_id.clone());
        let lineage_id = camp.as_ref().and_then(|c| c.lineage_id.clone());
        let (job, created) = job_store.enqueue_or_get_open(
            &campaign_id,
            conversation_id,
            lineage_id,
            uncovered_a as u32,
            uncovered_b as u32,
        )?;
        tracing::info!(
            target: "chronicle_compressor",
            campaign_id = %campaign_id,
            job_id = %job.id,
            created,
            uncovered_a,
            uncovered_b,
            "compress job enqueued"
        );
        // Pending（含失败回队）允许再次 spawn；Running 不重复 spawn。
        if created || job.status == crate::compress_job_store::CompressJobStatus::Pending {
            spawn_compress_job_worker(state.clone(), job.id);
        }
        Ok(())
    };
    if let Err(e) = enqueue() {
        tracing::error!(target: "chronicle_compressor", "enqueue compress job failed: {e}");
    }
}

/// 启动恢复：Running→Pending，然后为所有 open job spawn worker。
/// SQLite 与 JSON 各走自己的队列权威；worker 重复启动由原子 claim 幂等。
pub fn recover_compress_jobs_on_startup(app_state: Arc<crate::AppState>) {
    let storage = app_state.storage().clone();
    if storage.is_sqlite() {
        match crate::sqlite_runtime::compress_reset_running_to_pending() {
            Ok(reset) => {
                if reset > 0 {
                    tracing::info!(
                        target: "chronicle_compressor",
                        reset,
                        "startup: reset Running compress jobs to Pending (SQLite)"
                    );
                }
            }
            Err(e) => {
                tracing::error!(target: "chronicle_compressor", "SQLite compress recovery: {e}");
                return;
            }
        }
        let open = match crate::sqlite_runtime::compress_list_open() {
            Ok(open) => open,
            Err(e) => {
                tracing::error!(target: "chronicle_compressor", "SQLite compress list: {e}");
                return;
            }
        };
        tracing::info!(
            target: "chronicle_compressor",
            count = open.len(),
            "startup: replaying open compress jobs (SQLite)"
        );
        for job in open {
            spawn_compress_job_worker(app_state.clone(), job.id);
        }
        return;
    }
    let job_store = match app_state
        .storage()
        .json_compress_job_store("recover chronicle compression")
    {
        Ok(store) => store,
        Err(error) => {
            tracing::error!(target: "chronicle_compressor", "compress job store unavailable: {error}");
            return;
        }
    };
    let reset = job_store.reset_running_to_pending();
    if reset > 0 {
        tracing::info!(
            target: "chronicle_compressor",
            reset,
            "startup: reset Running compress jobs to Pending"
        );
    }
    let open = job_store.list_open();
    if open.is_empty() {
        return;
    }
    tracing::info!(
        target: "chronicle_compressor",
        count = open.len(),
        "startup: replaying open compress jobs"
    );
    for job in open {
        spawn_compress_job_worker(app_state.clone(), job.id);
    }
}

/// 生产压缩执行器：走真实 LLM（run_compress_if_needed）。
fn production_compress(
    state: Arc<crate::AppState>,
    campaign_id: Id,
    lineage_id: Id,
    conversation_id: Id,
    entries: Vec<storyforge_domain::agent::RoundSummary>,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    Vec<storyforge_app_agent::CompressRunOutcome>,
                    storyforge_app_agent::ChronicleCompressorError,
                >,
            > + Send,
    >,
> {
    Box::pin(async move {
        let llm = match state.require_active_llm() {
            Ok(llm) => llm,
            Err(error) => {
                return Err(storyforge_app_agent::ChronicleCompressorError::Parse(
                    error.to_string(),
                ));
            }
        };
        let tool_snapshot = state.snapshot_tool_ctx();
        let runtime = storyforge_app_agent::AgentRuntime::new(llm, tool_snapshot);
        storyforge_app_agent::run_compress_if_needed(
            &runtime,
            &campaign_id,
            &lineage_id,
            &conversation_id,
            entries,
            cancel,
            None,
            None,
            None,
        )
        .await
    })
}

/// 消费单个 compress job（可崩溃重试：失败回到 Pending 或 Failed）。
/// JSON 走 CampaignStore + CompressJobStore；SQLite 走 compress_jobs 表 +
/// 原子 publication UoW。可注入 `compress` 供确定性测试（无 LLM）。
pub fn spawn_compress_job_worker(state: Arc<crate::AppState>, job_id: Id) {
    spawn_compress_job_worker_with(state, job_id, production_compress);
}

#[allow(clippy::type_complexity)]
pub fn spawn_compress_job_worker_with(
    state: Arc<crate::AppState>,
    job_id: Id,
    compress: impl Fn(
        Arc<crate::AppState>,
        Id,
        Id,
        Id,
        Vec<storyforge_domain::agent::RoundSummary>,
        tokio::sync::watch::Receiver<bool>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Vec<storyforge_app_agent::CompressRunOutcome>,
                        storyforge_app_agent::ChronicleCompressorError,
                    >,
                > + Send,
        >,
    > + Send
    + Sync
    + 'static,
) {
    let storage = state.storage().clone();
    let compress = std::sync::Arc::new(compress);
    tokio::spawn(async move {
        let campaign_store = match storage.json_campaign_store_owned(
            BackendCapability::ChronicleCompressor,
            "run chronicle compressor",
        ) {
            Ok(store) => Some(store),
            Err(_) if storage.is_sqlite() => None,
            Err(error) => {
                tracing::error!(
                    target: "chronicle_compressor",
                    job_id = %job_id,
                    "chronicle campaign store unavailable: {error}"
                );
                return;
            }
        };
        let job: Option<(Id, Id, Option<Id>, Option<Id>)> = if storage.is_sqlite() {
            match crate::sqlite_runtime::compress_list_all() {
                Ok(jobs) => jobs.into_iter().find_map(|j| {
                    (j.id == job_id
                        && j.status == crate::sqlite_compress_jobs::CompressJobStatus::Pending)
                        .then_some((j.id, j.campaign_id, j.conversation_id, j.lineage_id))
                }),
                Err(e) => {
                    tracing::error!(target: "chronicle_compressor", "SQLite compress list: {e}");
                    return;
                }
            }
        } else {
            let job_store = match storage.json_compress_job_store("run chronicle compressor") {
                Ok(store) => store,
                Err(e) => {
                    tracing::error!(target: "chronicle_compressor", "compress job store unavailable: {e}");
                    return;
                }
            };
            job_store.list_all().into_iter().find_map(|j| {
                (j.id == job_id
                    && j.status == crate::compress_job_store::CompressJobStatus::Pending)
                    .then_some((j.id, j.campaign_id, j.conversation_id, j.lineage_id))
            })
        };
        let Some((job_id, campaign_id, job_conversation_id, job_lineage_id)) = job else {
            return;
        };
        let claimed = if storage.is_sqlite() {
            match crate::sqlite_runtime::compress_try_claim_pending(&job_id) {
                Ok(claimed) => claimed,
                Err(e) => {
                    tracing::warn!(target: "chronicle_compressor", "try_claim_pending {job_id}: {e}");
                    return;
                }
            }
        } else {
            let job_store = match storage.json_compress_job_store("run chronicle compressor") {
                Ok(store) => store,
                Err(e) => {
                    tracing::warn!(target: "chronicle_compressor", "try_claim_pending {job_id}: {e}");
                    return;
                }
            };
            match job_store.try_claim_pending(&job_id) {
                Ok(claimed) => claimed,
                Err(e) => {
                    tracing::warn!(target: "chronicle_compressor", "try_claim_pending {job_id}: {e}");
                    return;
                }
            }
        };
        if !claimed {
            tracing::debug!(
                target: "chronicle_compressor",
                job_id = %job_id,
                "compress job already claimed; worker exit"
            );
            return;
        }

        let camp = match crate::sqlite_runtime::get_campaign(&campaign_id) {
            Ok(Some(c)) => c,
            _ => match campaign_store
                .as_ref()
                .and_then(|s| s.get_campaign(&campaign_id))
            {
                Some(c) => c,
                None => {
                    let _ = mark_job_failed(&storage, &job_id, "campaign missing");
                    return;
                }
            },
        };
        let lineage = job_lineage_id
            .or(camp.lineage_id.clone())
            .unwrap_or_else(Id::new);
        let conversation_id = job_conversation_id
            .or(camp.conversation_id.clone())
            .unwrap_or_else(|| Id::from_str("unknown-conv"));
        let entries = if storage.is_sqlite() {
            match crate::sqlite_runtime::list_summaries(&campaign_id) {
                Ok(entries) => entries,
                Err(e) => {
                    let _ = mark_job_failed(&storage, &job_id, &e);
                    return;
                }
            }
        } else {
            match campaign_store.as_ref() {
                Some(store) => store.list_summaries(&campaign_id),
                None => {
                    let _ = mark_job_failed(&storage, &job_id, "campaign store unavailable");
                    return;
                }
            }
        };

        let (_tx, cancel) = tokio::sync::watch::channel(false);
        match compress(
            state,
            campaign_id.clone(),
            lineage,
            conversation_id,
            entries,
            cancel,
        )
        .await
        {
            Ok(outcomes) => {
                let mut publish_err: Option<String> = None;
                for out in outcomes {
                    // 批次键用稳定语义值：output_level（A→B=0、B→C=1）。若用数组
                    // 序号，第一批发布后崩溃、恢复只剩 B→C 时会重新编号 0，与已
                    // 发布批次冲突并反复重试（三轮评审 P1-1b）。
                    let batch_index = out.output_level.as_u8() as u32;
                    let result = if storage.is_sqlite() {
                        let publication_id = Id::new();
                        crate::sqlite_runtime::publish_chronicle_compress_with_fault_flag(
                            &campaign_id,
                            &publication_id,
                            &out.parent_summaries,
                            &out.publish.child_covered_by,
                            Some(job_id.as_str()),
                            batch_index,
                        )
                    } else {
                        match campaign_store.as_ref() {
                            Some(store) => store
                                .publish_compress_result(
                                    &campaign_id,
                                    &out.parent_summaries,
                                    &out.publish.child_covered_by,
                                )
                                .map(|_| {
                                    storyforge_infra_sqlite::publication::PublishOutcome::Applied
                                }),
                            None => Err("campaign store unavailable".into()),
                        }
                    };
                    match result {
                        Ok(_) => {
                            tracing::info!(
                                target: "chronicle_compressor",
                                job_id = %job_id,
                                batch = batch_index,
                                level = ?out.output_level,
                                parents = out.parent_summaries.len(),
                                children = out.publish.child_covered_by.len(),
                                "compress batch published"
                            );
                        }
                        Err(e) => {
                            // 同一 (job_id, batch_index) 已被发布（V007 唯一索引）：
                            // - job 已终态 = 其它 worker 处理过的迟到结果 → 丢弃退出；
                            // - job 仍 open（本 worker 重试运行）→ 该 job 此前部分
                            //   发布过，本次重试无法推进，回队重试/耗尽 attempts，
                            //   绝不把任务卡在 Running。
                            if storage.is_sqlite()
                                && e.contains("job_id")
                                && e.contains("already used")
                            {
                                let job_still_open = crate::sqlite_runtime::compress_list_all()
                                    .ok()
                                    .and_then(|jobs| jobs.into_iter().find(|j| j.id == job_id))
                                    .map(|j| j.status.is_open())
                                    .unwrap_or(false);
                                if !job_still_open {
                                    tracing::debug!(
                                        target: "chronicle_compressor",
                                        job_id = %job_id,
                                        batch = batch_index,
                                        "compress publication already published by another worker; late result dropped"
                                    );
                                    return;
                                }
                            }
                            publish_err = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = publish_err {
                    if !storage.is_sqlite()
                        && let Some(store) = &campaign_store
                        && store.needs_compress_metadata_heal(&campaign_id)
                    {
                        match store.heal_compress_publication_metadata(&campaign_id) {
                            Ok(()) => {}
                            Err(he) => tracing::warn!(
                                target: "chronicle_compressor",
                                "heal after publish err (marker kept if incomplete): {he}"
                            ),
                        }
                    }
                    if let Err(me) = mark_job_failed(&storage, &job_id, &e) {
                        tracing::error!(target: "chronicle_compressor", "mark_failed_or_retry: {me}");
                    }
                } else if let Err(e) = mark_job_succeeded(&storage, &job_id) {
                    tracing::error!(target: "chronicle_compressor", "mark_succeeded: {e}");
                }
            }
            Err(storyforge_app_agent::ChronicleCompressorError::NothingToCompress) => {
                if !storage.is_sqlite()
                    && let Some(store) = &campaign_store
                    && store.needs_compress_metadata_heal(&campaign_id)
                {
                    match store.heal_compress_publication_metadata(&campaign_id) {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::warn!(
                                target: "chronicle_compressor",
                                "heal metadata on NothingToCompress failed: {e}"
                            );
                            if let Err(me) = mark_job_failed(&storage, &job_id, &e) {
                                tracing::error!(target: "chronicle_compressor", "mark_failed_or_retry: {me}");
                            }
                            return;
                        }
                    }
                }
                if let Err(e) = mark_job_succeeded(&storage, &job_id) {
                    tracing::error!(target: "chronicle_compressor", "mark_succeeded: {e}");
                } else {
                    tracing::info!(
                        target: "chronicle_compressor",
                        job_id = %job_id,
                        "compress job nothing to do → succeeded"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "chronicle_compressor",
                    job_id = %job_id,
                    "compress run failed: {e}"
                );
                if let Err(me) = mark_job_failed(&storage, &job_id, &e.to_string()) {
                    tracing::error!(target: "chronicle_compressor", "mark_failed_or_retry: {me}");
                }
            }
        }
    });
}

fn mark_job_failed(storage: &StorageFacade, job_id: &Id, err: &str) -> Result<bool, String> {
    if storage.is_sqlite() {
        crate::sqlite_runtime::compress_mark_failed_or_retry(job_id, err)
    } else {
        let job_store = storage.json_compress_job_store("mark compress job failed")?;
        job_store.mark_failed_or_retry(job_id, err).map(|_| true)
    }
}

fn mark_job_succeeded(storage: &StorageFacade, job_id: &Id) -> Result<bool, String> {
    if storage.is_sqlite() {
        crate::sqlite_runtime::compress_mark_succeeded(job_id)
    } else {
        let job_store = storage.json_compress_job_store("mark compress job succeeded")?;
        job_store.mark_succeeded(job_id).map(|_| true)
    }
}

use std::collections::HashSet;

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;

use crate::AppState;
use crate::campaign_store::CampaignStore;
use crate::error::TauriCommandError;

trait ConversationDeleter {
    fn delete(&self, id: &Id) -> Result<(), String>;
}

struct ConversationStoreDeleter<'a>(&'a ConversationStore);

impl ConversationDeleter for ConversationStoreDeleter<'_> {
    fn delete(&self, id: &Id) -> Result<(), String> {
        self.0.delete(id).map_err(|error| error.to_string())
    }
}

/// Gate 5：Campaign 删除源的 backend-neutral 端口——JSON `CampaignStore` 与
/// `StorageFacade`（SQLite 单事务级联）都实现它，删除一局活动的编排逻辑只依赖
/// 该端口，不再直连 JSON store。
pub(crate) trait CampaignDeleter {
    fn get_campaign(&self, id: &Id) -> Result<Option<Campaign>, String>;
    fn delete_campaign(&self, id: &Id) -> Result<bool, String>;
}

impl CampaignDeleter for CampaignStore {
    fn get_campaign(&self, id: &Id) -> Result<Option<Campaign>, String> {
        Ok(CampaignStore::get_campaign(self, id))
    }
    fn delete_campaign(&self, id: &Id) -> Result<bool, String> {
        CampaignStore::delete_campaign(self, id)
    }
}

impl CampaignDeleter for crate::storage_backend::StorageFacade {
    fn get_campaign(&self, id: &Id) -> Result<Option<Campaign>, String> {
        crate::storage_backend::StorageFacade::get_campaign(self, id)
            .map(|record| record.map(|record| record.campaign))
    }
    fn delete_campaign(&self, id: &Id) -> Result<bool, String> {
        crate::storage_backend::StorageFacade::delete_campaign(self, id)
    }
}

/// 删除一局活动的全部权威数据：Campaign 聚合、绑定会话和活跃指针。
/// `source` 是 backend-neutral 端口（JSON store 或 StorageFacade）。
pub(crate) fn delete_campaign_playthrough_in_store(
    source: &dyn CampaignDeleter,
    conv_store: &ConversationStore,
    state: &AppState,
    campaign_id: &Id,
) -> Result<(), TauriCommandError> {
    let mut conversation_ids = HashSet::new();
    let campaign_present = if let Some(campaign) = source
        .get_campaign(campaign_id)
        .map_err(|error| TauriCommandError::storage(format!("读取 campaign 失败: {error}")))?
    {
        // 同时检查正向绑定与反向 campaign_id，兼容历史上的半绑定数据。
        if let Some(conversation_id) = campaign.conversation_id {
            conversation_ids.insert(conversation_id);
        }
        if let Some(conversation) = conv_store.find_by_campaign(campaign_id) {
            conversation_ids.insert(conversation.id);
        }
        true
    } else if let Some(conversation) = conv_store.find_by_campaign(campaign_id) {
        // 旧版本可能已经删掉 Campaign、但在清理会话时失败；允许重试补偿。
        conversation_ids.insert(conversation.id);
        false
    } else {
        return Err(TauriCommandError::not_found(format!(
            "找不到 campaign id={}",
            campaign_id.as_str()
        )));
    };

    let deleter = ConversationStoreDeleter(conv_store);
    delete_campaign_playthrough_with_deleter(
        source,
        state,
        campaign_id,
        campaign_present,
        &conversation_ids,
        &deleter,
    )?;
    // SQLite 级联直接删除 conversations 行（不经 ConversationStore），缓存若不
    // 失效，后续 find_by_campaign 会命中已删会话 → 二次 delete 行为与 JSON 分叉。
    conv_store.invalidate();
    Ok(())
}

fn delete_campaign_playthrough_with_deleter<D: ConversationDeleter>(
    source: &dyn CampaignDeleter,
    state: &AppState,
    campaign_id: &Id,
    campaign_present: bool,
    conversation_ids: &HashSet<Id>,
    deleter: &D,
) -> Result<(), TauriCommandError> {
    // Keep pointer read, persistence/rollback, Campaign deletion and the final
    // in-memory commit in the same critical section as active Campaign changes.
    let _active_update = state
        .active_campaign_update
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let active_pointer_needs_clear = {
        let active_campaign = state
            .active_campaign
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active_campaign.as_ref() == Some(campaign_id)
    };
    // 指针持久化由 backend adapter 处理：JSON 清 active_campaign.json，
    // SQLite 仅内存指针（ActiveCampaignPersistence Degraded，不落盘）。
    let pointer_cleared = active_pointer_needs_clear;
    if pointer_cleared {
        crate::backend_workflows::save_active_pointer(state.storage(), None).map_err(|error| {
            TauriCommandError::storage(format!("清除活跃活动指针失败: {error}"))
        })?;
    }

    // 先删会话 + Turn。失败时 Campaign 仍存在，用户可以安全重试；
    // 这是跨 JSON 文件删除无法使用单一事务时的补偿顺序。SQLite 由
    // delete_campaign_cascade 单事务级联删除（facade 内 backend 策略）。
    if let Err(error) = state.storage().delete_campaign_precursors(
        campaign_id,
        conversation_ids,
        |conversation_id| deleter.delete(conversation_id),
    ) {
        let restore_error = if pointer_cleared {
            crate::backend_workflows::save_active_pointer(state.storage(), Some(campaign_id)).err()
        } else {
            None
        };
        tracing::warn!(
            "删除活动 {} 的前置数据（会话/Turn）失败: {error}",
            campaign_id.as_str()
        );
        let suffix = restore_error
            .map(|restore| format!("；恢复活跃活动指针也失败: {restore}"))
            .unwrap_or_default();
        return Err(TauriCommandError::storage(format!(
            "清理活动前置数据失败，活动仍保留可重试: {error}{suffix}"
        )));
    }

    if campaign_present {
        let deleted = match source.delete_campaign(campaign_id) {
            Ok(deleted) => deleted,
            Err(error) => {
                let restore_error = if pointer_cleared {
                    crate::backend_workflows::save_active_pointer(
                        state.storage(),
                        Some(campaign_id),
                    )
                    .err()
                } else {
                    None
                };
                let suffix = restore_error
                    .map(|restore| format!("；恢复活跃活动指针也失败: {restore}"))
                    .unwrap_or_default();
                return Err(TauriCommandError::storage(format!(
                    "删除活动失败: {error}{suffix}"
                )));
            }
        };
        if !deleted {
            let restore_error = if pointer_cleared {
                crate::backend_workflows::save_active_pointer(state.storage(), Some(campaign_id))
                    .err()
            } else {
                None
            };
            let suffix = restore_error
                .map(|restore| format!("；恢复活跃活动指针也失败: {restore}"))
                .unwrap_or_default();
            return Err(TauriCommandError::not_found(format!(
                "找不到 campaign id={}{}",
                campaign_id.as_str(),
                suffix
            )));
        }
    }

    if active_pointer_needs_clear {
        let mut active_campaign = state
            .active_campaign
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active_campaign.as_ref() == Some(campaign_id) {
            *active_campaign = None;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::commands::campaigns::set_active_campaign_in_state;
    use crate::load_active_campaign;
    use storyforge_app_conversation::{ConversationError, ConversationPersistence};
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::conversation::Conversation;

    #[derive(Default)]
    struct FailingPersistence {
        conversations: Mutex<Vec<Conversation>>,
        fail_delete: AtomicBool,
    }

    impl ConversationPersistence for FailingPersistence {
        fn load_all(&self) -> Result<Vec<Conversation>, ConversationError> {
            Ok(self
                .conversations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone())
        }

        fn save(&self, conversation: &Conversation) -> Result<(), ConversationError> {
            let mut conversations = self
                .conversations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            conversations.retain(|current| current.id != conversation.id);
            conversations.push(conversation.clone());
            Ok(())
        }

        fn delete(&self, id: &Id) -> Result<(), ConversationError> {
            if self.fail_delete.load(Ordering::Acquire) {
                return Err(ConversationError::ExternalStorage(
                    "injected conversation persistence delete failure".into(),
                ));
            }
            self.conversations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retain(|conversation| &conversation.id != id);
            Ok(())
        }
    }

    #[test]
    fn public_lifecycle_entry_retries_through_real_conversation_persistence() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-playthrough-public-delete-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("campaign-public-retry"), "public retry");
        let persistence = Arc::new(FailingPersistence {
            fail_delete: AtomicBool::new(true),
            ..Default::default()
        });
        let conv_store = ConversationStore::with_persistence(persistence.clone());
        let conversation = conv_store
            .create_persisted(None, Some(campaign.id.clone()))
            .unwrap();
        let mut campaign = campaign;
        campaign.conversation_id = Some(conversation.id.clone());
        store.save_campaign(campaign.clone()).unwrap();
        let state = AppState::new_for_test();
        *state
            .active_campaign
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(campaign.id.clone());

        let first = delete_campaign_playthrough_in_store(&store, &conv_store, &state, &campaign.id);
        assert!(first.is_err());
        assert!(store.get_campaign(&campaign.id).is_some());
        assert!(conv_store.get(&conversation.id).is_some());
        assert_eq!(
            load_active_campaign(&state.data_dir),
            Some(campaign.id.clone())
        );

        persistence.fail_delete.store(false, Ordering::Release);
        delete_campaign_playthrough_in_store(&store, &conv_store, &state, &campaign.id).unwrap();
        assert!(store.get_campaign(&campaign.id).is_none());
        assert!(conv_store.get(&conversation.id).is_none());
        assert_eq!(load_active_campaign(&state.data_dir), None);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn active_campaign_switch_and_delete_success_are_serialized() {
        for round in 0..16 {
            let dir = std::env::temp_dir().join(format!(
                "storyforge-active-update-success-{round}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let store = Arc::new(CampaignStore::new(&dir));
            let conv_store = Arc::new(ConversationStore::new(dir.join("conversations")));

            let campaign_a = Campaign::new(
                Id::from_str(format!("campaign-delete-{round}")),
                "delete me",
            );
            let campaign_b = Campaign::new(
                Id::from_str(format!("campaign-switch-{round}")),
                "switch to me",
            );
            let conversation = conv_store
                .create_persisted(None, Some(campaign_a.id.clone()))
                .unwrap();
            let mut campaign_a = campaign_a;
            campaign_a.conversation_id = Some(conversation.id.clone());
            store.save_campaign(campaign_a.clone()).unwrap();
            store.save_campaign(campaign_b.clone()).unwrap();

            let state = Arc::new(AppState::new_for_test());
            *state
                .active_campaign
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(campaign_a.id.clone());
            crate::backend_workflows::save_active_pointer(state.storage(), Some(&campaign_a.id))
                .unwrap();

            let switch_entered = Arc::new(Barrier::new(2));
            let release_switch = Arc::new(Barrier::new(2));
            let switch_store = Arc::clone(&store);
            let switch_state = Arc::clone(&state);
            let switch_id = campaign_b.id.clone();
            let switch_entered_for_thread = Arc::clone(&switch_entered);
            let release_switch_for_thread = Arc::clone(&release_switch);
            let switch = std::thread::spawn(move || {
                let id_for_validation = switch_id.clone();
                set_active_campaign_in_state(
                    &switch_state,
                    switch_id,
                    move || {
                        switch_entered_for_thread.wait();
                        release_switch_for_thread.wait();
                        if switch_store.get_campaign(&id_for_validation).is_some() {
                            Ok(())
                        } else {
                            Err(TauriCommandError::not_found(
                                "switch target disappeared during validation",
                            ))
                        }
                    },
                    |_, _| {},
                )
            });

            // The switch now owns the update lock and is paused inside its
            // validation. Deletion must wait until the whole switch commits.
            switch_entered.wait();
            let delete_start = Arc::new(Barrier::new(2));
            let delete_store = Arc::clone(&store);
            let delete_conv_store = Arc::clone(&conv_store);
            let delete_state = Arc::clone(&state);
            let delete_id = campaign_a.id.clone();
            let delete_start_for_thread = Arc::clone(&delete_start);
            let delete = std::thread::spawn(move || {
                delete_start_for_thread.wait();
                delete_campaign_playthrough_in_store(
                    delete_store.as_ref(),
                    &delete_conv_store,
                    &delete_state,
                    &delete_id,
                )
            });
            delete_start.wait();
            release_switch.wait();
            assert!(delete.join().unwrap().is_ok());
            assert!(switch.join().unwrap().is_ok());
            assert_eq!(
                state
                    .active_campaign
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref(),
                Some(&campaign_b.id)
            );
            assert_eq!(
                load_active_campaign(&state.data_dir),
                Some(campaign_b.id.clone())
            );
            assert!(store.get_campaign(&campaign_a.id).is_none());
            assert!(store.get_campaign(&campaign_b.id).is_some());

            std::fs::remove_dir_all(dir).ok();
            std::fs::remove_dir_all(&state.data_dir).ok();
        }
    }

    #[test]
    fn active_campaign_switch_and_delete_failure_are_serialized() {
        for round in 0..16 {
            let dir = std::env::temp_dir().join(format!(
                "storyforge-active-update-failure-{round}-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let store = Arc::new(CampaignStore::new(&dir));
            let conversations_dir = dir.join("conversations");
            let conv_store = Arc::new(ConversationStore::new(conversations_dir.clone()));

            let campaign_a = Campaign::new(
                Id::from_str(format!("campaign-delete-fails-{round}")),
                "delete fails",
            );
            let campaign_b = Campaign::new(
                Id::from_str(format!("campaign-switch-after-failure-{round}")),
                "switch survives",
            );
            let conversation = conv_store
                .create_persisted(None, Some(campaign_a.id.clone()))
                .unwrap();
            let conversation_path = conversations_dir.join(format!("{}.json", conversation.id));
            std::fs::remove_file(&conversation_path).unwrap();
            std::fs::create_dir(&conversation_path).unwrap();
            let sentinel = conversation_path.join("must-survive");
            std::fs::write(&sentinel, b"sentinel").unwrap();

            let mut campaign_a = campaign_a;
            campaign_a.conversation_id = Some(conversation.id.clone());
            store.save_campaign(campaign_a.clone()).unwrap();
            store.save_campaign(campaign_b.clone()).unwrap();

            let state = Arc::new(AppState::new_for_test());
            *state
                .active_campaign
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(campaign_a.id.clone());
            crate::backend_workflows::save_active_pointer(state.storage(), Some(&campaign_a.id))
                .unwrap();

            let switch_entered = Arc::new(Barrier::new(2));
            let release_switch = Arc::new(Barrier::new(2));
            let switch_store = Arc::clone(&store);
            let switch_state = Arc::clone(&state);
            let switch_id = campaign_b.id.clone();
            let switch_entered_for_thread = Arc::clone(&switch_entered);
            let release_switch_for_thread = Arc::clone(&release_switch);
            let switch = std::thread::spawn(move || {
                let id_for_validation = switch_id.clone();
                set_active_campaign_in_state(
                    &switch_state,
                    switch_id,
                    move || {
                        switch_entered_for_thread.wait();
                        release_switch_for_thread.wait();
                        if switch_store.get_campaign(&id_for_validation).is_some() {
                            Ok(())
                        } else {
                            Err(TauriCommandError::not_found(
                                "switch target disappeared during validation",
                            ))
                        }
                    },
                    |_, _| {},
                )
            });

            // Hold the update lock across the switch while deletion is
            // released, then verify the failed deletion cannot restore A.
            switch_entered.wait();
            let delete_start = Arc::new(Barrier::new(2));
            let delete_store = Arc::clone(&store);
            let delete_conv_store = Arc::clone(&conv_store);
            let delete_state = Arc::clone(&state);
            let delete_id = campaign_a.id.clone();
            let delete_start_for_thread = Arc::clone(&delete_start);
            let delete = std::thread::spawn(move || {
                delete_start_for_thread.wait();
                delete_campaign_playthrough_in_store(
                    delete_store.as_ref(),
                    &delete_conv_store,
                    &delete_state,
                    &delete_id,
                )
            });
            delete_start.wait();
            release_switch.wait();
            assert!(delete.join().unwrap().is_err());
            assert!(switch.join().unwrap().is_ok());
            assert_eq!(
                state
                    .active_campaign
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref(),
                Some(&campaign_b.id)
            );
            assert_eq!(
                load_active_campaign(&state.data_dir),
                Some(campaign_b.id.clone())
            );
            assert!(store.get_campaign(&campaign_a.id).is_some());
            assert!(sentinel.is_file());

            std::fs::remove_dir_all(dir).ok();
            std::fs::remove_dir_all(&state.data_dir).ok();
        }
    }
}

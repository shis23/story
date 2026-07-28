use std::collections::HashSet;

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;

use crate::campaign_store::CampaignStore;
use crate::error::TauriCommandError;
use crate::{AppState, save_active_campaign, sqlite_runtime};

trait ConversationDeleter {
    fn delete(&self, id: &Id) -> Result<(), String>;
}

struct ConversationStoreDeleter<'a>(&'a ConversationStore);

impl ConversationDeleter for ConversationStoreDeleter<'_> {
    fn delete(&self, id: &Id) -> Result<(), String> {
        self.0.delete(id).map_err(|error| error.to_string())
    }
}

/// 删除一局活动的全部权威数据：Campaign 聚合、绑定会话和活跃指针。
pub(crate) fn delete_campaign_playthrough_in_store(
    store: &CampaignStore,
    conv_store: &ConversationStore,
    state: &AppState,
    campaign_id: &Id,
) -> Result<(), TauriCommandError> {
    let mut conversation_ids = HashSet::new();
    let campaign_present = if let Some(campaign) = store.get_campaign(campaign_id) {
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
        store,
        state,
        campaign_id,
        campaign_present,
        &conversation_ids,
        &deleter,
    )
}

fn delete_campaign_playthrough_with_deleter<D: ConversationDeleter>(
    store: &CampaignStore,
    state: &AppState,
    campaign_id: &Id,
    campaign_present: bool,
    conversation_ids: &HashSet<Id>,
    deleter: &D,
) -> Result<(), TauriCommandError> {
    let active_pointer_needs_clear = {
        let active_campaign = state
            .active_campaign
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active_campaign.as_ref() == Some(campaign_id)
    };
    let pointer_cleared = active_pointer_needs_clear && !sqlite_runtime::is_sqlite_active();
    if pointer_cleared {
        save_active_campaign(&state.data_dir, None).map_err(|error| {
            TauriCommandError::storage(format!("清除活跃活动指针失败: {error}"))
        })?;
    }

    // 先删会话。失败时 Campaign 仍存在，用户可以安全重试；
    // 这是跨 JSON 文件删除无法使用单一事务时的补偿顺序。
    for conversation_id in conversation_ids {
        if let Err(error) = deleter.delete(conversation_id) {
            let restore_error = if pointer_cleared {
                save_active_campaign(&state.data_dir, Some(campaign_id)).err()
            } else {
                None
            };
            tracing::warn!(
                "删除活动 {} 的会话 {} 失败: {error}",
                campaign_id.as_str(),
                conversation_id.as_str()
            );
            let suffix = restore_error
                .map(|restore| format!("；恢复活跃活动指针也失败: {restore}"))
                .unwrap_or_default();
            return Err(TauriCommandError::storage(format!(
                "清理会话失败，活动仍保留可重试: {error}{suffix}"
            )));
        }
    }

    if campaign_present {
        let deleted = match store.delete_campaign(campaign_id) {
            Ok(deleted) => deleted,
            Err(error) => {
                let restore_error = if pointer_cleared {
                    save_active_campaign(&state.data_dir, Some(campaign_id)).err()
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
                save_active_campaign(&state.data_dir, Some(campaign_id)).err()
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
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
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
}

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
    // 先删会话。失败时 Campaign 仍存在，用户可以安全重试；
    // 这是跨 JSON 文件删除无法使用单一事务时的补偿顺序。
    for conversation_id in conversation_ids {
        if let Err(error) = deleter.delete(conversation_id) {
            tracing::warn!(
                "删除活动 {} 的会话 {} 失败: {error}",
                campaign_id.as_str(),
                conversation_id.as_str()
            );
            return Err(TauriCommandError::storage(format!(
                "清理会话失败，活动仍保留可重试: {error}"
            )));
        }
    }

    if campaign_present {
        let deleted = store
            .delete_campaign(campaign_id)
            .map_err(|error| TauriCommandError::storage(format!("删除活动失败: {error}")))?;
        if !deleted {
            return Err(TauriCommandError::not_found(format!(
                "找不到 campaign id={}",
                campaign_id.as_str()
            )));
        }
    }

    let mut active_campaign = state
        .active_campaign
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if active_campaign.as_ref() == Some(campaign_id) {
        *active_campaign = None;
        if !sqlite_runtime::is_sqlite_active() {
            save_active_campaign(&state.data_dir, None);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use storyforge_domain::campaign::Campaign;

    struct FailingConversationDeleter {
        fail: AtomicBool,
        deleted: Mutex<Vec<Id>>,
    }

    impl FailingConversationDeleter {
        fn new() -> Self {
            Self {
                fail: AtomicBool::new(true),
                deleted: Mutex::new(Vec::new()),
            }
        }
    }

    #[test]
    fn conversation_failure_keeps_campaign_retryable() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-playthrough-delete-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("campaign-retry"), "retry");
        store.save_campaign(campaign.clone()).unwrap();
        let state = AppState::new_for_test();
        let deleter = FailingConversationDeleter::new();
        let conversation_ids = HashSet::from([Id::from_str("conversation-retry")]);

        let first = delete_campaign_playthrough_with_deleter(
            &store,
            &state,
            &campaign.id,
            true,
            &conversation_ids,
            &deleter,
        );
        assert!(first.is_err());
        assert!(store.get_campaign(&campaign.id).is_some());

        deleter.fail.store(false, Ordering::Release);
        let second = delete_campaign_playthrough_with_deleter(
            &store,
            &state,
            &campaign.id,
            true,
            &conversation_ids,
            &deleter,
        );
        assert!(second.is_ok());
        assert!(store.get_campaign(&campaign.id).is_none());
        assert_eq!(
            deleter.deleted.lock().unwrap().as_slice(),
            &[Id::from_str("conversation-retry")]
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn orphan_conversation_cleanup_can_finish_after_campaign_is_missing() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-playthrough-orphan-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = CampaignStore::new(&dir);
        let state = AppState::new_for_test();
        let deleter = FailingConversationDeleter::new();
        deleter.fail.store(false, Ordering::Release);
        let campaign_id = Id::from_str("campaign-orphan");
        let conversation_ids = HashSet::from([Id::from_str("conversation-orphan")]);

        let result = delete_campaign_playthrough_with_deleter(
            &store,
            &state,
            &campaign_id,
            false,
            &conversation_ids,
            &deleter,
        );

        assert!(result.is_ok());
        assert_eq!(
            deleter.deleted.lock().unwrap().as_slice(),
            &[Id::from_str("conversation-orphan")]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    impl ConversationDeleter for FailingConversationDeleter {
        fn delete(&self, id: &Id) -> Result<(), String> {
            if self.fail.load(Ordering::Acquire) {
                return Err("injected conversation delete failure".into());
            }
            self.deleted.lock().unwrap().push(id.clone());
            Ok(())
        }
    }
}

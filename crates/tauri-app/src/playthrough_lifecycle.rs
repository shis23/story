use std::collections::HashSet;

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;

use crate::campaign_store::CampaignStore;
use crate::error::TauriCommandError;
use crate::{AppState, save_active_campaign, sqlite_runtime};

/// 删除一局活动的全部权威数据：Campaign 聚合、绑定会话和活跃指针。
pub(crate) fn delete_campaign_playthrough_in_store(
    store: &CampaignStore,
    conv_store: &ConversationStore,
    state: &AppState,
    campaign_id: &Id,
) -> Result<(), TauriCommandError> {
    let campaign = store.get_campaign(campaign_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("找不到 campaign id={}", campaign_id.as_str()))
    })?;

    // 同时检查正向绑定与反向 campaign_id，兼容历史上的半绑定数据。
    let mut conversation_ids = HashSet::new();
    if let Some(conversation_id) = campaign.conversation_id.clone() {
        conversation_ids.insert(conversation_id);
    }
    if let Some(conversation) = conv_store.find_by_campaign(campaign_id) {
        conversation_ids.insert(conversation.id);
    }

    let deleted = store
        .delete_campaign(campaign_id)
        .map_err(|error| TauriCommandError::storage(format!("删除活动失败: {error}")))?;
    if !deleted {
        return Err(TauriCommandError::not_found(format!(
            "找不到 campaign id={}",
            campaign_id.as_str()
        )));
    }

    for conversation_id in conversation_ids {
        if let Err(error) = conv_store.delete(&conversation_id) {
            tracing::warn!(
                "删除活动 {} 后清理会话 {} 失败: {error}",
                campaign_id.as_str(),
                conversation_id.as_str()
            );
            return Err(TauriCommandError::storage(format!(
                "活动已删除，但清理会话失败: {error}"
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

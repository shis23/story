use std::sync::Arc;

use storyforge_domain::Id;

use crate::AppState;
use crate::error::TauriCommandError;

pub use storyforge_app_service::services::conversations::*;

// 保留 API：当前无前端入口（wrapper 于 2026-09-01 移除），保留原因见 docs/review-2026-09-13/fixes/04-tauri-fixes.md
// （T-15：本命令被显式登记在 scripts/architecture/backend-baseline.mjs 的
// RETAINED_NO_FRONTEND_CALLER 中；删除它需要连带删除 archive_conversation_impl，
// 而后者是唯一实现体）。
#[tauri::command]
pub(crate) async fn archive_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    archive_conversation_impl(conversation_id, state).await
}

pub(crate) async fn archive_conversation_impl(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let llm = state.require_active_llm()?;
    let archiver = build_archiver(&state, llm);

    let count = run_archive_with_watermark(&state, &conv_id, &archiver, true)
        .await
        .map_err(TauriCommandError::internal)?;
    Ok(count)
}

#[tauri::command]
pub(crate) fn list_conversations(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<ConversationSummaryDto>, TauriCommandError> {
    // 联查角色卡名由 backend adapter 统一处理（Gate 3）：
    // JSON 一次 list_cards 快照建卡名表（character_id 直查 → campaign 兜底）；
    // SQLite 无卡名来源，card_name 恒 None（文档化 Gate 4 缺口，不静默回退）。
    let summaries = state.conv_store.list();
    let card_names =
        crate::backend_workflows::conversation_card_names_for_backend(state.storage(), &summaries)
            .map_err(TauriCommandError::internal)?;
    Ok(summaries
        .into_iter()
        .map(|c| ConversationSummaryDto {
            id: c.id.to_string(),
            character_id: c.character_id,
            campaign_id: c.campaign_id.map(|id| id.to_string()),
            card_name: card_names.get(c.id.as_str()).cloned(),
            message_count: c.message_count,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        })
        .collect())
}

/// 删除整个会话。
///
/// 一 Campaign 一对话：若该会话绑定了 Campaign（或某 Campaign 的 conversation_id 指向它），
/// 则按 **整局活动** 级联删除（实例/知识/任务/总结 + 会话），而不是只清消息树。
#[tauri::command]
pub(crate) fn delete_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);

    // 优先：会话自己记录的 campaign_id
    let campaign_id = state
        .conv_store
        .get(&conv_id)
        .and_then(|c| c.campaign_id.clone())
        // 兜底：Campaign.conversation_id 反向指向（悬空/半绑定时）
        .or_else(|| {
            state
                .storage()
                .list_campaigns(None)
                .ok()
                .and_then(|records| {
                    records
                        .into_iter()
                        .find(|record| record.campaign.conversation_id.as_ref() == Some(&conv_id))
                        .map(|record| record.campaign.id)
                })
        });

    if let Some(campaign_id) = campaign_id {
        return crate::playthrough_lifecycle::delete_campaign_playthrough_in_store(
            state.storage().as_ref(),
            state.conv_store.as_ref(),
            state.inner().as_ref(),
            &campaign_id,
        );
    }

    // 无 Campaign 的遗留/孤儿会话：只删对话
    state
        .conv_store
        .delete(&conv_id)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

#[tauri::command]
pub(crate) fn get_conversation(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let conv_id = storyforge_domain::Id::from_str(&id);
    let conversation = state
        .conv_store
        .get(&conv_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("对话不存在: {id}")))?;
    let regex_scripts = collect_conversation_regex_scripts(&conversation, state.inner().as_ref());
    Ok(
        serde_json::to_value(conversation_display_dto(&conversation, &regex_scripts))
            .unwrap_or_default(),
    )
}

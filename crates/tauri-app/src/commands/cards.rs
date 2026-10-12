use super::super::*;

pub use storyforge_app_service::services::cards::*;

#[tauri::command]
pub fn list_cards(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<CardSummaryDto>, TauriCommandError> {
    let cards = state
        .storage()
        .list_cards()
        .map_err(TauriCommandError::storage)?;
    Ok(cards.iter().map(CardSummaryDto::from).collect())
}

/// 删除角色卡（按 CharacterCard.id，级联删 campaign/instances/mvu）
///
/// Gate 5 三.7：双后端完整等价级联。JSON：会话 + Turn + 压缩任务先行清理，
/// `CampaignStore::delete_card` 快照/补偿式删除（cards/campaigns/instances/
/// knowledge/tasks/summaries/mvu + 本局世界书文件，任一文件写失败整体回滚）；
/// SQLite：`delete_card_payload` 单事务级联（mutation_commits → compress
/// jobs → outbox → turns → conversations → summaries → tasks → knowledge →
/// instances → world_info → campaigns → mvu → card）。两后端删除后都清活跃
/// 指针、失效会话缓存、重建 tool_ctx 世界书（进程内状态，in-process 断言）。
#[tauri::command]
pub fn delete_card(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let card_id = Id::from_str(&id);

    // 删除前收集受影响 Campaign / 会话 ids（删除后无法再读）。收集错误传播
    // （三.7：不得用 if let Ok / .ok() 吞掉——否则删除成功却漏清关联数据）。
    let mut per_campaign_conv_ids: Vec<(Id, std::collections::HashSet<Id>)> = Vec::new();
    {
        let campaigns = state
            .storage()
            .list_campaigns(Some(&card_id))
            .map_err(|e| {
                TauriCommandError::storage(format!("收集受影响 Campaign 失败 card={card_id}: {e}"))
            })?;
        for record in campaigns {
            let campaign_id = record.campaign.id.clone();
            let mut conv_ids = std::collections::HashSet::new();
            if let Some(conv_id) = record.campaign.conversation_id {
                conv_ids.insert(conv_id);
            }
            if let Some(conversation) = state.conv_store.find_by_campaign(&campaign_id) {
                conv_ids.insert(conversation.id);
            }
            per_campaign_conv_ids.push((campaign_id, conv_ids));
        }
    }

    // 三审5：JSON 多文件跨边界（会话/Turn/压缩任务 + CampaignStore 聚合）必须整体
    // 原子——前置删除成功、聚合写盘失败时，所有数据必须回到删除前原样（重启一致）。
    // 快照收集经 backend_workflows（backend flag 白名单文件），SQLite 返回 None。
    let snapshot = crate::backend_workflows::snapshot_delete_card_affected_files(
        state.storage(),
        &per_campaign_conv_ids,
    );

    // 1) 前置清理：会话 + Turn + 压缩任务（JSON 多文件无法单事务，先删前置、
    //    失败时卡仍在可安全重试；SQLite 由 delete_card_payload 单事务级联，
    //    此处 no-op，与 delete_campaign_playthrough 同款补偿顺序）。
    for (campaign_id, conv_ids) in &per_campaign_conv_ids {
        state
            .storage()
            .delete_campaign_precursors(campaign_id, conv_ids, |conversation_id| {
                state
                    .conv_store
                    .delete(conversation_id)
                    .map_err(|e| e.to_string())
            })
            .map_err(|e| {
                TauriCommandError::storage(format!(
                    "清理活动 {} 的前置数据（会话/Turn）失败，卡保留可重试: {e}",
                    campaign_id.as_str()
                ))
            })?;
        state
            .storage()
            .delete_compress_jobs_for_campaign(campaign_id)
            .map_err(|e| {
                TauriCommandError::storage(format!(
                    "清理活动 {} 的压缩任务失败: {e}",
                    campaign_id.as_str()
                ))
            })?;
    }

    // 2) 删卡（JSON：快照/补偿式全级联；SQLite：单事务级联）。
    //    三审5：聚合写盘失败时，用前置快照逆序恢复全部受影响文件（含会话/Turn），
    //    保证重启后所有数据保持删除前原样——绝不留「前置已删、卡还在」的半状态。
    let aggregate_result = state.storage().delete_card(&card_id);
    if let Err(e) = aggregate_result {
        if let Some(snap) = snapshot
            && let Err(restore_err) = crate::backend_workflows::restore_delete_card_snapshot(snap)
        {
            return Err(TauriCommandError::storage(format!(
                "存储写入失败: {e}; 删除回滚也失败，数据需要恢复: {restore_err}"
            )));
        }
        return Err(TauriCommandError::storage(format!("存储写入失败: {e}")));
    }
    if !aggregate_result.unwrap() {
        // 卡不存在：聚合未改任何状态，但前置可能已删——用快照恢复前置原样。
        if let Some(snap) = snapshot
            && let Err(restore_err) = crate::backend_workflows::restore_delete_card_snapshot(snap)
        {
            // 2026-10-11 审计：旧实现 `let _ =` 吞掉恢复失败，用户只看到「不存在」，
            // 前置已删的数据却没回来。
            return Err(TauriCommandError::storage(format!(
                "角色卡不存在: {id}; 恢复前置删除也失败，数据需要恢复: {restore_err}"
            )));
        }
        return Err(TauriCommandError::not_found(format!("角色卡不存在: {id}")));
    }

    // 3) 应用内状态清理（双后端一致，in-process 证明）：
    //    活跃指针（若指向被删 Campaign）+ 会话缓存失效 + tool_ctx 世界书重建。
    let _update = state
        .active_campaign_update
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut active_campaign = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(active_id) = active_campaign.as_ref()
        && per_campaign_conv_ids.iter().any(|(id, _)| id == active_id)
    {
        crate::backend_workflows::save_active_pointer(state.storage(), None)
            .map_err(|e| TauriCommandError::storage(format!("清除活跃活动指针失败: {e}")))?;
        *active_campaign = None;
    }
    drop(active_campaign);
    drop(_update);
    // SQLite 级联直接删除 conversations 行（不经 ConversationStore），缓存若
    // 不失效，后续 find_by_campaign 会命中已删会话（与 delete_character 同款）。
    state.conv_store.invalidate();
    // 被删 Campaign 的远记忆向量（派生索引，失败只告警）。
    for (campaign_id, _) in &per_campaign_conv_ids {
        crate::vector_index::purge_campaign_vectors(state.inner(), campaign_id);
    }
    // tool_ctx：被删活动若曾是当前注入源，重建世界书（无活跃活动时回退角色库）。
    super::characters::rebuild_world_info_in_tool_ctx(&state);
    Ok(())
}

#[tauri::command]
pub fn get_card(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardDetailDto, TauriCommandError> {
    let stored = state
        .storage()
        .get_card(&Id::from_str(&id))
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 card id={id}")))?;
    let source_character = state
        .storage()
        .get_character(stored.card.source_character_id.as_str())
        .map_err(TauriCommandError::storage)?;
    let (raw_first_mes, raw_alternate_greetings) = raw_card_greetings(&stored.card.raw_card_json);
    let definition_count = stored.card.character_definitions.len();
    Ok(CardDetailDto {
        id: stored.card.id.as_str().to_string(),
        name: stored.card.name.clone(),
        source_character_id: stored.card.source_character_id.as_str().to_string(),
        first_mes: source_character
            .as_ref()
            .map(|sc| sc.info.first_mes.clone())
            .unwrap_or(raw_first_mes),
        alternate_greetings: source_character
            .as_ref()
            .map(|sc| sc.info.alternate_greetings.clone())
            .unwrap_or(raw_alternate_greetings),
        character_definitions: stored
            .card
            .character_definitions
            .iter()
            .map(CharacterDefinitionDto::from)
            .collect(),
        campaign_variable_schema: stored.card.effective_campaign_variable_schema(),
        definition_count,
        character_count: definition_count,
        imported_at: stored.imported_at.clone(),
        extracted: stored.card.extraction_succeeded(),
        extraction_status: stored.card.extraction_status.as_str().to_string(),
        extraction_message: card_extraction_message(&stored.card),
    })
}

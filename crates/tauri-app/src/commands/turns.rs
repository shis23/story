use super::super::*;

pub use storyforge_app_service::services::turns::*;

// ─── M1 对话操作命令 ──────────────────────────────────────────────────────

#[tauri::command]
pub(crate) fn get_active_turn_receipt(
    campaign_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Option<ActiveTurnReceiptDto>, TauriCommandError> {
    get_active_turn_receipt_impl(state.storage(), campaign_id, node_id)
}

#[tauri::command]
pub(crate) async fn retry_active_turn_postprocess(
    campaign_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ActiveTurnReceiptDto, TauriCommandError> {
    retry_active_turn_postprocess_impl(campaign_id, node_id, state).await
}

#[tauri::command]
pub(crate) fn get_active_turn_quality(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Option<ActiveTurnQualityDto>, TauriCommandError> {
    get_active_turn_quality_impl(state.storage(), campaign_id)
}

/// 仅重跑当前草稿的 Summarizer + PostProcessor，不重写正文。
pub(crate) async fn retry_active_turn_postprocess_impl(
    campaign_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ActiveTurnReceiptDto, TauriCommandError> {
    use storyforge_domain::turn::{AttemptStatus, TurnStatus};

    let campaign_id = Id::from_str(&campaign_id);
    let variant_id = Id::from_str(&node_id);
    let active_campaign = state
        .active_campaign
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if active_campaign.as_ref() != Some(&campaign_id) {
        return Err(TauriCommandError::validation(
            "只能重试当前打开 Campaign 的记账推导",
        ));
    }

    let turn = get_active_turn_for_backend(state.storage(), &campaign_id)
        .map_err(TauriCommandError::internal)?
        .ok_or_else(|| TauriCommandError::validation("当前 Campaign 没有待采纳 Turn"))?;
    let attempt = turn
        .find_attempt_by_variant(&variant_id)
        .cloned()
        .ok_or_else(|| TauriCommandError::validation("该草稿已不是当前 Attempt"))?;
    if attempt.status != AttemptStatus::AwaitingAcceptance
        || !attempt
            .derivation
            .as_ref()
            .is_some_and(storyforge_domain::turn::DerivationComponents::has_failure)
    {
        return Err(TauriCommandError::validation(
            "只有记账推导失败的待采纳草稿可以重试",
        ));
    }

    let turn_id = turn.turn_id.clone();
    let attempt_id = attempt.attempt_id.clone();

    let conversation = state
        .conv_store
        .get(&turn.conversation_id)
        .ok_or_else(|| TauriCommandError::internal("找不到草稿所属对话"))?;
    let final_text = conversation
        .nodes
        .iter()
        .find(|node| node.id == variant_id)
        .and_then(|node| node.active())
        .map(|variant| variant.content.clone())
        .ok_or_else(|| TauriCommandError::internal("找不到待重试草稿正文"))?;

    let snapshot = state.snapshot_tool_ctx();
    let regex_character_id = conversation.character_id.clone();
    let mut writing_ctx = WritingContext {
        characters: snapshot.characters.clone(),
        world_info: snapshot.world_info.clone(),
        conversation_id: turn.conversation_id.clone(),
        campaign_id: None,
        turn: 0,
        pending_tasks: vec![],
        story_clock: String::new(),
        profile: None,
        modules: vec![],
        regex_scripts: collect_scoped_regex_scripts_for_backend(
            regex_character_id.as_deref(),
            &snapshot.characters,
            Some(state.storage()),
        )
        .map_err(TauriCommandError::storage)?,
        campaign_runtime: None,
        agent_profile_config: None,
        recent_summaries: vec![],
        chronicle_prompt_catalog: vec![],
        far_memory_hits: vec![],
        template_random_seed: None,
        context_epoch: None,
        chronicle_revision: 0,
    };
    fill_regex_context(
        &mut writing_ctx,
        get_preset_store(),
        get_global_regex_store(),
    );
    fill_profile_context(&mut writing_ctx, &state);
    fill_agent_profile_context(&mut writing_ctx, &state);
    fill_campaign_context_async(&mut writing_ctx, &state).await?;
    if writing_ctx.campaign_id.as_ref() != Some(&campaign_id) {
        return Err(TauriCommandError::validation(
            "重试期间 Campaign 已切换，请重新打开小票",
        ));
    }

    let present_characters = postprocess_present_characters(attempt.provenance.as_ref());
    let variable_keys = postprocess_variable_keys(&writing_ctx);
    let fallback_fragments = collect_mvu_fallback_fragments_for_backend(
        state.storage(),
        &writing_ctx,
        &present_characters,
    )
    .map_err(TauriCommandError::internal)?;
    let mvu_update_rules =
        collect_mvu_update_rules_for_backend(state.storage(), &writing_ctx, &present_characters)
            .map_err(TauriCommandError::internal)?;
    let pipeline = state.new_pipeline_with_regex(&writing_ctx.regex_scripts)?;
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let identity = production_postprocess::PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id: attempt_id.clone(),
        campaign_id: campaign_id.clone(),
        conversation_id: turn.conversation_id.clone(),
        turn_number: writing_ctx.turn,
    };
    let runtime = writing_ctx.campaign_runtime.clone();

    // All fallible preparation above deliberately happens while the attempt is
    // still AwaitingAcceptance. Only take the DerivingState lease immediately
    // before the shared postprocess service starts, so a missing conversation,
    // invalid Campaign context, or config load error cannot strand the Turn.
    let transitioned = update_turn_record_if(
        state.storage(),
        &turn_id,
        |record| {
            record.status == TurnStatus::AwaitingAcceptance
                && record.find_attempt(&attempt_id).is_some_and(|candidate| {
                    candidate.status == AttemptStatus::AwaitingAcceptance
                        && candidate.variant_id == variant_id
                })
        },
        |record| {
            record.status = TurnStatus::DerivingState;
            record.failure_reason = None;
            if let Some(candidate) = record.find_attempt_mut(&attempt_id) {
                candidate.status = AttemptStatus::DerivingState;
            }
            record.touch();
        },
    )
    .map_err(TauriCommandError::internal)?;
    if !transitioned {
        return Err(TauriCommandError::validation(
            "Turn 状态已变化，请刷新小票后重试",
        ));
    }

    run_shared_postprocess_background(
        state.storage().clone(),
        pipeline,
        writing_ctx,
        final_text,
        present_characters,
        variable_keys,
        fallback_fragments,
        mvu_update_rules,
        event_tx,
        cancel_rx,
        Some(identity),
        runtime,
    )
    .await
    .map_err(|error| TauriCommandError::internal(format!("重试记账失败: {error}")))?;

    let refreshed = get_active_turn_for_backend(state.storage(), &campaign_id)
        .map_err(TauriCommandError::internal)?
        .ok_or_else(|| TauriCommandError::internal("重试后找不到活动 Turn"))?;
    active_turn_receipt_from_record(&refreshed, &variant_id)
        .ok_or_else(|| TauriCommandError::internal("重试后找不到活动 Attempt"))
}

/// 编辑当前变体内容
#[tauri::command]
pub(crate) fn edit_variant(
    conversation_id: String,
    node_id: String,
    new_content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);

    // Turn/Attempt edit + Stale mark is a backend-owned workflow (Gate 3):
    // SQLite uses the atomic mark-stale preaccept UoW when an Attempt is
    // linked; JSON edits first and marks Stale best-effort (P0-3).
    state
        .turn_workflow
        .edit_variant_with_stale_mark(&conv_id, &nid, &new_content)
        .map_err(TauriCommandError::internal)?;
    Ok(())
}

/// 采纳当前变体（Draft → Final），并自动检查是否需要归档。
///
/// `force_accept`：QualityGate 存在 **Error** 时默认拦截；传 true 强制接受并标记 Turn **Degraded**。
/// Warning 不拦截。
#[tauri::command]
pub(crate) async fn accept_variant(
    conversation_id: String,
    node_id: String,
    force_accept: Option<bool>,
    selected_mutation_indices: Option<Vec<usize>>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    accept_variant_async(
        state.inner().clone(),
        conv_id,
        nid,
        force_accept.unwrap_or(false),
        selected_mutation_indices,
    )
    .await
}

/// 软删除当前变体（→ Discarded）。
///
/// Phase A: Campaign 模式下同时把对应 TurnAttempt 标 Discarded。
/// Turn 仍开放，允许 regenerate（Discard Attempt ≠ Abandon Turn）。
// 保留 API：当前无前端入口（wrapper 于 2026-09-01 移除），保留原因见 docs/review-2026-09-13/fixes/04-tauri-fixes.md
// （T-15：显式登记在 scripts/architecture/backend-baseline.mjs 的
// RETAINED_NO_FRONTEND_CALLER 中；同名 store 方法仍被 backend_workflows 的
// 补偿路径使用，本命令只是它的手动入口。）
#[tauri::command]
pub(crate) fn soft_delete_variant(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);

    // Phase A: Campaign 模式下标记 Attempt Discarded
    if let Some(turn) = get_turn_by_variant_for_backend(state.storage(), &nid)
        .map_err(TauriCommandError::internal)?
    {
        let attempt_id = turn
            .find_attempt_by_variant(&nid)
            .map(|a| a.attempt_id.clone());
        if let Some(att_id) = attempt_id {
            // 2026-10-11 审计：标记失败必须在软删变体之前中止。旧实现 `let _ =`
            // 吞掉错误后照常软删，留下「变体已删、Attempt 仍是 Draft」的不一致态。
            update_turn_record(state.storage(), &turn.turn_id, |record| {
                if let Some(att) = record.find_attempt_mut(&att_id) {
                    att.status = storyforge_domain::turn::AttemptStatus::Discarded;
                }
                record.touch();
            })
            .map_err(|e| TauriCommandError::storage(format!("标记 Attempt 丢弃失败: {e}")))?;
        }
    }

    state
        .conv_store
        .soft_delete_variant(&conv_id, &nid)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// Phase A: 放弃整个 Turn（终止本轮，排除 user 消息和所有 AI 草稿）。
///
/// 与 Discard Attempt 的区别：
/// - Discard Attempt 只丢弃单个 AI 变体，Turn 仍开放。
/// - Abandon Turn 终止整个 Turn，同时把 input user 变体和所有未 accept 的 AI 变体标记 Discarded。
// 保留 API：当前无前端入口（wrapper 于 2026-09-01 移除），保留原因见 docs/review-2026-09-13/fixes/04-tauri-fixes.md
// （T-15：显式登记在 scripts/architecture/backend-baseline.mjs 的
// RETAINED_NO_FRONTEND_CALLER 中；本命令承载 Gate 8 复评加固的 Turn CAS
// 谓词——先 CAS 置 Abandoned 再软删变体，防止并发 Accept 已提交的故事被
// 静默回退，见下方 :697 起的长注释。）
#[tauri::command]
pub(crate) async fn abandon_turn(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);

    let active_campaign = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };

    let Some(campaign_id) = active_campaign else {
        return Err(TauriCommandError::validation(
            "非 Campaign 模式不支持 abandon_turn".to_string(),
        ));
    };

    let turn = get_active_turn_for_backend(state.storage(), &campaign_id)
        .map_err(TauriCommandError::internal)?
        .ok_or_else(|| TauriCommandError::validation("没有活动 Turn 可以放弃".to_string()))?;

    // Gate 8 复评：先 CAS 把 Turn 置 Abandoned——谓词要求「仍在活动态且未开始
    // 提交副作用」（排除 Committing/已终态）。旧实现先软删变体、后无条件
    // update_turn_record 置 Abandoned：若并发 Accept 已赢下（CAS→Committing→
    // Committed），abandon 的软删会把已 Final 的正文变体标 Discarded、Turn 被
    // 回退成 Abandoned——已提交故事被静默还原。CAS 成功后 accept 的
    // AwaitingAcceptance→Committing 不可能再通过，软删才安全。
    let abandoned = state
        .storage()
        .mutate_turn_if(
            &turn.turn_id,
            |record| record.status.is_active() && !record.status.has_side_effects_started(),
            |record| {
                record.status = storyforge_domain::turn::TurnStatus::Abandoned;
                for att in &mut record.attempts {
                    if att.status.is_active() {
                        att.status = storyforge_domain::turn::AttemptStatus::Discarded;
                    }
                }
                record.touch();
            },
        )
        .map_err(TauriCommandError::internal)?;

    if !abandoned {
        return Err(TauriCommandError::validation(
            "Turn 已进入提交或终态，无法放弃（可能已被并发 Accept 接管）".to_string(),
        ));
    }

    // 把 input user 变体和所有未 accept 的 AI 变体标记 Discarded
    let conv_store = state.conv_store.clone();
    let turn_clone = turn.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        // Discard input user 消息
        conv_store
            .soft_delete_variant(&conv_id, &turn_clone.input_node_id)
            .map_err(|e| e.to_string())?;
        // Discard 所有未 accept 的 AI 变体
        for attempt in &turn_clone.attempts {
            if attempt.status.is_active() {
                conv_store
                    .soft_delete_variant(&conv_id, &attempt.variant_id)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("Abandon Turn 任务失败: {e}")))?
    .map_err(TauriCommandError::internal)?;

    Ok(())
}

/// Tauri command: 删除指定消息及其后所有消息（截断对话 = 撤销从这条开始的写作）
#[tauri::command]
pub(crate) fn delete_message_from(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .turn_workflow
        .delete_uncommitted_history(&conv_id, &nid)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// 添加新变体（分支/swipe）
///
/// `provenance` 对应前端 `addVariant(conversationId, nodeId, content, provenance)`
/// （`frontend/src/tauri-api.js`，invoke 时发 `provenance: provenance || null`）。
/// 2026-09-13 域4 修复 T-04：此前本命令**没有**该形参，而 Tauri 只按 camelCase
/// 键逐个取参（`v.get(self.key)`）、对多余的键静默忽略，因此前端传的
/// `provenance` 被丢弃、store 层被硬编码为 `None` —— API 表面承诺了"记录来源"
/// 的能力却永不生效。`Option<T>` 形参可缺省，故这是向后兼容的修正。
#[tauri::command]
pub(crate) fn add_variant(
    conversation_id: String,
    node_id: String,
    content: String,
    provenance: Option<Provenance>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .add_variant(&conv_id, &nid, content, provenance)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// 切换变体（左右滑）
#[tauri::command]
pub(crate) fn switch_variant(
    conversation_id: String,
    node_id: String,
    index: usize,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .switch_variant(&conv_id, &nid, index)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;

    // P0-3：切换变体后，当前 active variant 变了 → 旧 Attempt 标 Stale
    if let Some(turn) = get_turn_by_variant_for_backend(state.storage(), &nid)
        .map_err(TauriCommandError::internal)?
    {
        let turn_id = turn.turn_id.clone();
        if let Some(att) = turn.find_attempt_by_variant(&nid) {
            let attempt_id = att.attempt_id.clone();
            // 变体切换已落盘，这里不再回报 Err（否则前端会误以为切换失败）；
            // 但 Stale 标记失败必须留痕，不得静默。
            if let Err(e) = update_turn_record(state.storage(), &turn_id, |record| {
                if let Some(a) = record.find_attempt_mut(&attempt_id) {
                    a.status = storyforge_domain::turn::AttemptStatus::Stale;
                }
                record.touch();
            }) {
                tracing::warn!(
                    "切换变体后标记 Attempt {attempt_id} 为 Stale 失败（turn={turn_id}）: {e}"
                );
            }
        }
    }

    Ok(())
}

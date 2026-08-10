use super::super::*;
use super::conversations::auto_archive_if_needed;

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

fn mutation_is_receipt_reviewable(mutation: &storyforge_domain::turn::Mutation) -> bool {
    !matches!(
        mutation,
        storyforge_domain::turn::Mutation::FinalizeVariant { .. }
            | storyforge_domain::turn::Mutation::UpsertInstance(_)
    )
}

pub(crate) fn receipt_items_from_batch(
    batch: &storyforge_domain::turn::MutationBatch,
) -> Vec<TurnReceiptItemDto> {
    use storyforge_domain::turn::Mutation;

    batch
        .mutations
        .iter()
        .enumerate()
        .filter_map(|(mutation_index, mutation)| {
            let (kind, title, detail) = match mutation {
                Mutation::UpsertSummary(summary) => (
                    "chronicle",
                    summary
                        .headline
                        .clone()
                        .unwrap_or_else(|| "本轮纪要（Chronicle A）".into()),
                    summary.content.clone(),
                ),
                Mutation::UpsertKnowledge(knowledge) => (
                    "knowledge",
                    "角色知识更新".into(),
                    format!(
                        "{} 得知：{}",
                        knowledge.character_id, knowledge.knowledge_text
                    ),
                ),
                Mutation::SetVariable {
                    instance_id,
                    key,
                    value,
                    ..
                } => {
                    let target = instance_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "Campaign".into());
                    (
                        "variable",
                        format!("变量 · {key}"),
                        format!("{target}: {key} → {value}"),
                    )
                }
                Mutation::SetTaskStatus { task_id, status } => (
                    "task",
                    "任务状态变化".into(),
                    format!("任务 {task_id} → {status:?}"),
                ),
                Mutation::UpsertNewTask(task) => (
                    "task",
                    format!("新任务 · {}", task.title),
                    task.description.clone(),
                ),
                Mutation::FinalizeVariant { .. } | Mutation::UpsertInstance(_) => return None,
            };
            Some(TurnReceiptItemDto {
                mutation_index,
                kind: kind.into(),
                title,
                detail,
                selected_by_default: true,
            })
        })
        .collect()
}

/// 按小票勾选结果裁剪候选变更。正文 Finalize 与临时角色实例属于结构性
/// mutation，始终保留，避免引用断裂；其余未勾选项不参与 Accept。
pub(crate) fn retain_selected_receipt_mutations(
    batch: &mut storyforge_domain::turn::MutationBatch,
    selected_mutation_indices: &[usize],
) -> Result<(), String> {
    use std::collections::HashSet;

    if selected_mutation_indices
        .iter()
        .any(|index| *index >= batch.mutations.len())
    {
        return Err("小票包含已经失效的 mutation 下标，请刷新后重试".into());
    }
    let selected: HashSet<usize> = selected_mutation_indices.iter().copied().collect();
    batch.mutations = std::mem::take(&mut batch.mutations)
        .into_iter()
        .enumerate()
        .filter_map(|(index, mutation)| {
            if !mutation_is_receipt_reviewable(&mutation) || selected.contains(&index) {
                Some(mutation)
            } else {
                None
            }
        })
        .collect();
    Ok(())
}

fn active_turn_receipt_from_record(
    turn: &storyforge_domain::turn::TurnRecord,
    variant_id: &Id,
) -> Option<ActiveTurnReceiptDto> {
    use storyforge_domain::turn::AttemptStatus;

    let attempt = turn.find_attempt_by_variant(variant_id)?;
    let derivation_failed = attempt
        .derivation
        .as_ref()
        .is_some_and(storyforge_domain::turn::DerivationComponents::has_failure);
    let ready = attempt.status == AttemptStatus::AwaitingAcceptance;
    let notice = if derivation_failed {
        Some("记账推导有失败项：可重试，或明确降级采纳正文。".into())
    } else if ready && attempt.pending_state_changes.is_none() {
        Some("本轮没有可写入的纪要或状态变化。".into())
    } else if !ready {
        Some("记账仍在生成，请稍后刷新。".into())
    } else {
        None
    };
    Some(ActiveTurnReceiptDto {
        turn_id: turn.turn_id.to_string(),
        attempt_id: attempt.attempt_id.to_string(),
        variant_id: attempt.variant_id.to_string(),
        status: format!("{:?}", attempt.status),
        ready,
        derivation_failed,
        can_retry: ready && derivation_failed,
        can_degraded_accept: ready && derivation_failed,
        notice,
        items: attempt
            .pending_state_changes
            .as_ref()
            .map(receipt_items_from_batch)
            .unwrap_or_default(),
    })
}

/// 读取活动 Attempt 的 Accept-before 小票，不修改任何状态。
pub(crate) fn get_active_turn_receipt_impl(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: String,
    node_id: String,
) -> Result<Option<ActiveTurnReceiptDto>, TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    let variant_id = Id::from_str(&node_id);
    let Some(turn) =
        get_active_turn_for_backend(storage, &campaign_id).map_err(TauriCommandError::internal)?
    else {
        return Ok(None);
    };
    Ok(active_turn_receipt_from_record(&turn, &variant_id))
}

fn apply_turn_receipt_selection(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: &Id,
    variant_id: &Id,
    selected_mutation_indices: &[usize],
) -> Result<(), String> {
    use storyforge_domain::turn::{AttemptStatus, MutationBatchStatus, TurnStatus};

    let turn = get_active_turn_for_backend(storage, campaign_id)?
        .ok_or_else(|| "当前 Campaign 没有待采纳 Turn".to_string())?;
    let turn_id = turn.turn_id.clone();
    let attempt_id = turn
        .find_attempt_by_variant(variant_id)
        .ok_or_else(|| "小票对应的草稿已不是当前 Attempt".to_string())?
        .attempt_id
        .clone();
    let mut selection_error: Option<String> = None;
    let applied = update_turn_record_if(
        storage,
        &turn_id,
        |record| {
            record.campaign_id == *campaign_id
                && record.status == TurnStatus::AwaitingAcceptance
                && record.find_attempt(&attempt_id).is_some_and(|attempt| {
                    attempt.variant_id == *variant_id
                        && attempt.status == AttemptStatus::AwaitingAcceptance
                })
        },
        |record| {
            let Some(attempt) = record.find_attempt_mut(&attempt_id) else {
                selection_error = Some("小票对应的 Attempt 已消失".into());
                return;
            };
            match attempt.pending_state_changes.as_mut() {
                Some(batch) if batch.status == MutationBatchStatus::Prepared => {
                    if let Err(error) =
                        retain_selected_receipt_mutations(batch, selected_mutation_indices)
                    {
                        selection_error = Some(error);
                    }
                }
                Some(_) => selection_error = Some("候选变更已开始提交，不能再修改勾选项".into()),
                None if selected_mutation_indices.is_empty() => {}
                None => selection_error = Some("本轮没有可选择的候选变更".into()),
            }
            record.touch();
        },
    )?;
    if !applied {
        return Err("Turn 状态已变化，请刷新小票后重试".into());
    }
    if let Some(error) = selection_error {
        return Err(error);
    }
    Ok(())
}

pub(crate) fn postprocess_present_characters(provenance: Option<&Provenance>) -> Vec<String> {
    let Some(provenance) = provenance else {
        return vec![];
    };
    let candidates: Vec<String> = provenance
        .plan
        .as_ref()
        .map(|plan| {
            plan.subagent_tasks
                .iter()
                .map(|task| task.character_id.clone())
                .collect()
        })
        .filter(|characters: &Vec<String>| !characters.is_empty())
        .unwrap_or_else(|| {
            provenance
                .subagent_results
                .iter()
                .map(|snapshot| {
                    snapshot
                        .display_name
                        .clone()
                        .unwrap_or_else(|| snapshot.character_id.clone())
                })
                .collect()
        });
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
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

/// 从 TurnRecord 提取活动 Attempt 的质量 DTO（无报告 → None）。
pub(crate) fn active_turn_quality_from_record(
    turn: &storyforge_domain::turn::TurnRecord,
) -> Option<ActiveTurnQualityDto> {
    let attempt = turn.active_attempt()?;
    let report = attempt.quality_report.as_ref()?;
    let warnings: Vec<String> = report.warnings.iter().map(|w| w.message.clone()).collect();
    Some(ActiveTurnQualityDto {
        turn_id: turn.turn_id.to_string(),
        attempt_id: attempt.attempt_id.to_string(),
        status: format!("{:?}", attempt.status),
        passed: report.passed(),
        warning_count: report.warnings.len(),
        error_count: report.error_count(),
        warnings,
    })
}

/// 读取当前 Campaign 活动 Turn 上 active Attempt 的 QualityReport。
///
/// 无活动 Turn / 无质量报告 → None。不创建状态。
pub(crate) fn get_active_turn_quality_impl(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: String,
) -> Result<Option<ActiveTurnQualityDto>, TauriCommandError> {
    let camp = Id::from_str(&campaign_id);
    let turn = get_active_turn_for_backend(storage, &camp).map_err(TauriCommandError::internal)?;
    Ok(turn.as_ref().and_then(active_turn_quality_from_record))
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

pub(crate) async fn accept_variant_async(
    state: Arc<AppState>,
    conv_id: Id,
    node_id: Id,
    force_accept: bool,
    selected_mutation_indices: Option<Vec<usize>>,
) -> Result<(), TauriCommandError> {
    // Phase A: Campaign 模式分流
    let active_campaign = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };

    // 自动归档用的 conversation_id（在分支之前 clone，避免 move 问题）
    let archive_conv_id = conv_id.clone();

    if let Some(campaign_id) = active_campaign {
        // Campaign 模式 → TurnCommit
        if let Some(selected) = selected_mutation_indices.as_deref() {
            // A committed replay has no active Turn left. In that case skip the
            // already-applied receipt selection and let commit_turn_attempt
            // return its existing idempotent result.
            if get_active_turn_for_backend(state.storage(), &campaign_id)
                .map_err(TauriCommandError::internal)?
                .is_some()
            {
                apply_turn_receipt_selection(state.storage(), &campaign_id, &node_id, selected)
                    .map_err(TauriCommandError::validation)?;
            }
        }
        commit_turn_attempt(&state, &campaign_id, &conv_id, &node_id, force_accept).await?;
    } else {
        // 非 Campaign 模式 → 保持现有行为（无 Turn quality 报告）
        let conv_store = state.conv_store.clone();
        tokio::task::spawn_blocking(move || conv_store.accept_variant(&conv_id, &node_id))
            .await
            .map_err(|e| TauriCommandError::internal(format!("采纳变体任务失败: {e}")))?
            .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    }

    let state_clone = state.clone();
    tokio::spawn(async move {
        auto_archive_if_needed(&state_clone, &archive_conv_id).await;
    });

    Ok(())
}

/// Phase A: Campaign 模式下的 TurnCommit（accept → 正文 Final + 状态变更 + revision bump）。
///
/// 生产实现委托共享 `turn_lifecycle::TurnLifecycleService`，保证 Tauri 与 harness 同路径。
pub(crate) async fn commit_turn_attempt(
    state: &Arc<AppState>,
    campaign_id: &Id,
    conv_id: &Id,
    node_id: &Id,
    force_accept: bool,
) -> Result<(), TauriCommandError> {
    let state = state.clone();
    let campaign_id = campaign_id.clone();
    let conv_id = conv_id.clone();
    let node_id = node_id.clone();

    // Turn/Attempt/Accept dispatch lives in the injected TurnWorkflow adapter
    // (Gate 3): JSON uses the shared TurnLifecycleService, SQLite the atomic
    // production UoW (which also invalidates the conversation cache).
    let workflow = state.turn_workflow.clone();
    let campaign_id_for_accept = campaign_id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        workflow.accept_by_variant(&campaign_id_for_accept, &conv_id, &node_id, force_accept)
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("TurnCommit 任务失败: {e}")))?
    .map_err(|e| match e {
        turn_lifecycle::AcceptError::Storage(msg) | turn_lifecycle::AcceptError::Commit(msg) => {
            TauriCommandError::internal(msg)
        }
        turn_lifecycle::AcceptError::CampaignMissing => TauriCommandError::internal(e.to_string()),
        other => TauriCommandError::validation(other.to_string()),
    })?;

    // ContextCompiler：已接受的 RoundSummary 进入远记忆向量池（best-effort）
    {
        let state_for_index = state.clone();
        let batch_index = outcome.batch.clone();
        tokio::spawn(async move {
            index_round_summaries_async(state_for_index, batch_index).await;
        });
    }

    // M4：阈值达则后台 ChronicleCompressor（A→B / B→C）
    maybe_spawn_chronicle_compress(state, campaign_id);
    Ok(())
}

/// 已有 RoundSummary 上分配下一个 Chronicle A 序号（兼容无 code 的旧行）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn next_chronicle_a_seq(existing: &[storyforge_domain::agent::RoundSummary]) -> u32 {
    turn_lifecycle::next_chronicle_a_seq(existing)
}

/// 软删除当前变体（→ Discarded）。
///
/// Phase A: Campaign 模式下同时把对应 TurnAttempt 标 Discarded。
/// Turn 仍开放，允许 regenerate（Discard Attempt ≠ Abandon Turn）。
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
            let _ = update_turn_record(state.storage(), &turn.turn_id, |record| {
                if let Some(att) = record.find_attempt_mut(&att_id) {
                    att.status = storyforge_domain::turn::AttemptStatus::Discarded;
                }
                record.touch();
            });
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
        .conv_store
        .truncate_from(&conv_id, &nid)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// 添加新变体（分支/swipe）
#[tauri::command]
pub(crate) fn add_variant(
    conversation_id: String,
    node_id: String,
    content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .add_variant(&conv_id, &nid, content, None)
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
            let _ = update_turn_record(state.storage(), &turn_id, |record| {
                if let Some(a) = record.find_attempt_mut(&attempt_id) {
                    a.status = storyforge_domain::turn::AttemptStatus::Stale;
                }
                record.touch();
            });
        }
    }

    Ok(())
}

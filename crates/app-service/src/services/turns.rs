use super::super::*;
use super::conversations::auto_archive_if_needed;

fn mutation_is_receipt_reviewable(mutation: &storyforge_domain::turn::Mutation) -> bool {
    !matches!(
        mutation,
        storyforge_domain::turn::Mutation::FinalizeVariant { .. }
            | storyforge_domain::turn::Mutation::UpsertInstance(_)
    )
}

pub fn receipt_items_from_batch(
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
pub fn retain_selected_receipt_mutations(
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

pub fn active_turn_receipt_from_record(
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
pub fn get_active_turn_receipt_impl(
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

pub fn postprocess_present_characters(provenance: Option<&Provenance>) -> Vec<String> {
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

/// 从 TurnRecord 提取活动 Attempt 的质量 DTO（无报告 → None）。
pub fn active_turn_quality_from_record(
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
pub fn get_active_turn_quality_impl(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: String,
) -> Result<Option<ActiveTurnQualityDto>, TauriCommandError> {
    let camp = Id::from_str(&campaign_id);
    let turn = get_active_turn_for_backend(storage, &camp).map_err(TauriCommandError::internal)?;
    Ok(turn.as_ref().and_then(active_turn_quality_from_record))
}

pub async fn accept_variant_async(
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
pub async fn commit_turn_attempt(
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
pub fn next_chronicle_a_seq(existing: &[storyforge_domain::agent::RoundSummary]) -> u32 {
    turn_lifecycle::next_chronicle_a_seq(existing)
}

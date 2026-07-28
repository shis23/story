use super::super::*;

// ─── M1 对话操作命令 ──────────────────────────────────────────────────────

#[tauri::command]
pub(crate) fn get_active_turn_receipt(
    campaign_id: String,
    node_id: String,
) -> Result<Option<ActiveTurnReceiptDto>, TauriCommandError> {
    crate::get_active_turn_receipt_impl(campaign_id, node_id)
}

#[tauri::command]
pub(crate) async fn retry_active_turn_postprocess(
    campaign_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ActiveTurnReceiptDto, TauriCommandError> {
    crate::retry_active_turn_postprocess_impl(campaign_id, node_id, state).await
}

#[tauri::command]
pub(crate) fn get_active_turn_quality(campaign_id: String) -> Option<ActiveTurnQualityDto> {
    crate::get_active_turn_quality_impl(campaign_id)
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

    // SQLite opt-in: atomic edit + Stale via preaccept UoW when a live Attempt is linked.
    if sqlite_runtime::is_sqlite_active() {
        if let Some(turn) =
            get_turn_by_variant_for_backend(&nid).map_err(TauriCommandError::internal)?
            && let Some(att) = turn.find_attempt_by_variant(&nid)
        {
            sqlite_runtime::mark_stale_after_edit(
                &turn.campaign_id,
                &turn.conversation_id,
                &turn.turn_id,
                &att.attempt_id,
                &new_content,
            )
            .map_err(TauriCommandError::internal)?;
            state.conv_store.invalidate();
            return Ok(());
        }
        // No linked Attempt: plain conversation edit only (still SQLite-backed store).
        state
            .conv_store
            .edit_variant(&conv_id, &nid, new_content)
            .map_err(|e| TauriCommandError::internal(e.to_string()))?;
        return Ok(());
    }

    state
        .conv_store
        .edit_variant(&conv_id, &nid, new_content)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;

    // P0-3：编辑后 draft_hash 不再匹配 → 标记关联 Attempt 为 Stale
    // commit_turn_attempt 会因 hash 不匹配拒绝 accept，Stale 是显式信号
    if let Some(turn) =
        get_turn_by_variant_for_backend(&nid).map_err(TauriCommandError::internal)?
    {
        let turn_id = turn.turn_id.clone();
        if let Some(att) = turn.find_attempt_by_variant(&nid) {
            let attempt_id = att.attempt_id.clone();
            let _ = update_turn_record(&turn_id, |record| {
                if let Some(a) = record.find_attempt_mut(&attempt_id) {
                    a.status = storyforge_domain::turn::AttemptStatus::Stale;
                }
                record.touch();
            });
        }
    }

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
            if get_active_turn_for_backend(&campaign_id)
                .map_err(TauriCommandError::internal)?
                .is_some()
            {
                apply_turn_receipt_selection(&campaign_id, &node_id, selected)
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

/// 统计 campaign 未覆盖 A/B 数量。
pub(crate) fn count_uncovered_chronicle_levels(
    store: &campaign_store::CampaignStore,
    campaign_id: &Id,
) -> (usize, usize) {
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

/// Accept 成功后：达阈值则**持久化入队**，再 spawn worker 消费 job。
pub(crate) fn maybe_spawn_chronicle_compress(state: Arc<AppState>, campaign_id: Id) {
    if sqlite_runtime::is_sqlite_active() {
        // Chronicle publication jobs are already typed in infra-sqlite, but
        // the background worker still depends on the JSON job store. Refuse
        // that secondary authority rather than silently reading/writing it.
        tracing::debug!(
            campaign_id = %campaign_id,
            "sqlite backend skips JSON-only chronicle compressor worker"
        );
        return;
    }
    let store = get_campaign_store();
    let job_store = get_compress_job_store();
    let (uncovered_a, uncovered_b) = count_uncovered_chronicle_levels(store, &campaign_id);
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
    let camp = store.get_campaign(&campaign_id);
    let conversation_id = camp.as_ref().and_then(|c| c.conversation_id.clone());
    let lineage_id = camp.as_ref().and_then(|c| c.lineage_id.clone());
    match job_store.enqueue_or_get_open(
        &campaign_id,
        conversation_id,
        lineage_id,
        uncovered_a as u32,
        uncovered_b as u32,
    ) {
        Ok((job, created)) => {
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
            // 实际互斥靠 try_claim_pending。
            if created || job.status == compress_job_store::CompressJobStatus::Pending {
                spawn_compress_job_worker(state, job.id);
            }
        }
        Err(e) => {
            tracing::error!(
                target: "chronicle_compressor",
                "enqueue compress job failed: {e}"
            );
        }
    }
}

pub(crate) fn should_recover_json_compress_jobs(sqlite_active: bool) -> bool {
    !sqlite_active
}

/// 启动恢复：Running→Pending，然后为所有 open job spawn worker。
pub(crate) fn recover_compress_jobs_on_startup(app_state: Arc<AppState>) {
    // Chronicle compression still has a JSON job-store implementation only.
    // An opt-in SQLite process must neither resume nor mutate that secondary
    // store until the SQLite-native job path exists.
    if !should_recover_json_compress_jobs(sqlite_runtime::is_sqlite_active()) {
        tracing::info!(
            target: "chronicle_compressor",
            "startup: skipping JSON chronicle-job recovery in SQLite mode"
        );
        return;
    }
    let job_store = get_compress_job_store();
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

/// 消费单个 compress job（可崩溃重试：失败回到 Pending 或 Failed）。
pub(crate) fn spawn_compress_job_worker(state: Arc<AppState>, job_id: Id) {
    tokio::spawn(async move {
        let job_store = get_compress_job_store();
        let store = get_campaign_store();
        let job = match job_store.list_all().into_iter().find(|j| j.id == job_id) {
            Some(j) if j.status == compress_job_store::CompressJobStatus::Pending => j,
            _ => return,
        };
        match job_store.try_claim_pending(&job_id) {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    target: "chronicle_compressor",
                    job_id = %job_id,
                    "compress job already claimed; worker exit"
                );
                return;
            }
            Err(e) => {
                tracing::warn!(target: "chronicle_compressor", "try_claim_pending {job_id}: {e}");
                return;
            }
        }

        let campaign_id = job.campaign_id.clone();
        let camp = match store.get_campaign(&campaign_id) {
            Some(c) => c,
            None => {
                let _ = job_store.mark_failed_or_retry(&job_id, "campaign missing");
                return;
            }
        };
        let lineage = job
            .lineage_id
            .clone()
            .or(camp.lineage_id.clone())
            .unwrap_or_else(Id::new);
        let conversation_id = job
            .conversation_id
            .clone()
            .or(camp.conversation_id.clone())
            .unwrap_or_else(|| Id::from_str("unknown-conv"));
        let entries = store.list_summaries(&campaign_id);

        let llm = match state.require_active_llm() {
            Ok(llm) => llm,
            Err(error) => {
                let _ = job_store.mark_failed_or_retry(&job_id, error.to_string());
                return;
            }
        };
        let tool_snapshot = state.snapshot_tool_ctx();
        let runtime = storyforge_app_agent::AgentRuntime::new(llm, tool_snapshot);
        let (_tx, cancel) = tokio::sync::watch::channel(false);

        match storyforge_app_agent::run_compress_if_needed(
            &runtime,
            &campaign_id,
            &lineage,
            &conversation_id,
            entries,
            cancel,
            None,
            None,
            None,
        )
        .await
        {
            Ok(outcomes) => {
                let mut publish_err: Option<String> = None;
                for out in outcomes {
                    if let Err(e) = store.publish_compress_result(
                        &campaign_id,
                        &out.parent_summaries,
                        &out.publish.child_covered_by,
                    ) {
                        publish_err = Some(e);
                        break;
                    }
                    tracing::info!(
                        target: "chronicle_compressor",
                        job_id = %job_id,
                        level = ?out.output_level,
                        parents = out.parent_summaries.len(),
                        children = out.publish.child_covered_by.len(),
                        "compress batch published"
                    );
                }
                if let Some(e) = publish_err {
                    // 仅当 marker 仍在且 summaries 校验可通过时 heal 才能完成；
                    // 校验失败会保留 marker，并让 job 回 Pending 下次 Accept/启动再试。
                    if store.needs_compress_metadata_heal(&campaign_id) {
                        match store.heal_compress_publication_metadata(&campaign_id) {
                            Ok(()) => {}
                            Err(he) => tracing::warn!(
                                target: "chronicle_compressor",
                                "heal after publish err (marker kept if incomplete): {he}"
                            ),
                        }
                    }
                    if let Err(me) = job_store.mark_failed_or_retry(&job_id, e) {
                        tracing::error!(target: "chronicle_compressor", "mark_failed_or_retry: {me}");
                    }
                } else if let Err(e) = job_store.mark_succeeded(&job_id) {
                    tracing::error!(target: "chronicle_compressor", "mark_succeeded: {e}");
                }
            }
            Err(storyforge_app_agent::ChronicleCompressorError::NothingToCompress) => {
                // 可能是：并发已压缩完，或 summaries 已写但 metadata 未 heal
                if store.needs_compress_metadata_heal(&campaign_id) {
                    if let Err(e) = store.heal_compress_publication_metadata(&campaign_id) {
                        tracing::warn!(
                            target: "chronicle_compressor",
                            job_id = %job_id,
                            "heal metadata on NothingToCompress failed: {e}"
                        );
                        if let Err(me) = job_store.mark_failed_or_retry(&job_id, e) {
                            tracing::error!(target: "chronicle_compressor", "mark_failed_or_retry: {me}");
                        }
                        return;
                    }
                    tracing::info!(
                        target: "chronicle_compressor",
                        job_id = %job_id,
                        "healed compress metadata after NothingToCompress"
                    );
                }
                if let Err(e) = job_store.mark_succeeded(&job_id) {
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
                if let Err(me) = job_store.mark_failed_or_retry(&job_id, e.to_string()) {
                    tracing::error!(target: "chronicle_compressor", "mark_failed_or_retry: {me}");
                }
            }
        }
    });
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

    let state_for_accept = state.clone();
    let campaign_id_for_accept = campaign_id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        if sqlite_runtime::is_sqlite_active() {
            return sqlite_runtime::accept_by_variant(
                &campaign_id_for_accept,
                &conv_id,
                &node_id,
                force_accept,
            );
        }
        let service = turn_lifecycle::TurnLifecycleService::new(
            get_campaign_store(),
            get_turn_store(),
            &state_for_accept.conv_store,
        );
        service.accept_by_variant(&campaign_id_for_accept, &conv_id, &node_id, force_accept)
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

    // SQLite Accept mutates the conversation graph in its atomic UoW. Drop
    // the in-process projection so subsequent context reads reload the Final
    // variant from SQLite instead of retaining a stale Draft cache entry.
    if sqlite_runtime::is_sqlite_active() {
        state.conv_store.invalidate();
    }

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
    if let Some(turn) =
        get_turn_by_variant_for_backend(&nid).map_err(TauriCommandError::internal)?
    {
        let attempt_id = turn
            .find_attempt_by_variant(&nid)
            .map(|a| a.attempt_id.clone());
        if let Some(att_id) = attempt_id {
            let _ = update_turn_record(&turn.turn_id, |record| {
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

    let turn = get_active_turn_for_backend(&campaign_id)
        .map_err(TauriCommandError::internal)?
        .ok_or_else(|| TauriCommandError::validation("没有活动 Turn 可以放弃".to_string()))?;

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

    // 标记 Turn Abandoned
    update_turn_record(&turn.turn_id, |record| {
        record.status = storyforge_domain::turn::TurnStatus::Abandoned;
        for att in &mut record.attempts {
            if att.status.is_active() {
                att.status = storyforge_domain::turn::AttemptStatus::Discarded;
            }
        }
        record.touch();
    })
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
    if let Some(turn) =
        get_turn_by_variant_for_backend(&nid).map_err(TauriCommandError::internal)?
    {
        let turn_id = turn.turn_id.clone();
        if let Some(att) = turn.find_attempt_by_variant(&nid) {
            let attempt_id = att.attempt_id.clone();
            let _ = update_turn_record(&turn_id, |record| {
                if let Some(a) = record.find_attempt_mut(&attempt_id) {
                    a.status = storyforge_domain::turn::AttemptStatus::Stale;
                }
                record.touch();
            });
        }
    }

    Ok(())
}

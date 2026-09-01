use super::*;

/// Shared production postprocess entry used by start_writing (spawned) and regenerate (awaited).
///
/// Runner orchestration still uses `PipelineOrchestrator::run_postprocess` at the command
/// layer. Outcome writeback / guards / Chronicle candidates are owned by
/// `ProductionPostprocessService`.
///
/// Returns `Ok(applied)` or `Err` for critical consistency failures that callers must
/// surface (regenerate) / fail-closed (background start_writing).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_shared_postprocess_background(
    storage: Arc<storage_backend::StorageFacade>,
    pipeline: PipelineOrchestrator,
    writing_ctx: WritingContext,
    final_text: String,
    present_chars: Vec<String>,
    variable_keys: Vec<String>,
    fallback_fragments: Vec<storyforge_domain::mvu_translation::FallbackFragment>,
    mvu_update_rules: Vec<String>,
    event_tx: tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    cancel: watch::Receiver<bool>,
    identity: Option<production_postprocess::PostprocessIdentity>,
    runtime: Option<std::sync::Arc<CampaignRuntimeContext>>,
) -> Result<bool, production_postprocess::ProductionPostprocessError> {
    use production_postprocess::ProductionPostprocessError;

    if *cancel.borrow() {
        tracing::warn!("Phase A: postprocess 写回跳过——cancelled");
        // 三.3：runner 前取消也必须发出 Skipped（前端置 idle），经真实 helper。
        if let Some(event) = postprocess_pipeline_event(None, "", true) {
            let _ = event_tx.send(event);
        }
        return Ok(false);
    }

    let outcome = pipeline
        .run_postprocess(
            &final_text,
            "",
            &present_chars,
            &variable_keys,
            &writing_ctx,
            &event_tx,
            cancel.clone(),
            &fallback_fragments,
            &mvu_update_rules,
        )
        .await;

    if *cancel.borrow() {
        tracing::warn!("Phase A: postprocess 写回跳过——cancelled after runner");
        // 三.3：runner 后取消同样发出 Skipped（与 runner 前取消同一个事件）。
        if let Some(event) = postprocess_pipeline_event(None, "", true) {
            let _ = event_tx.send(event);
        }
        return Ok(false);
    }

    let sink = crate::backend_workflows::BackendTurnAttemptSink::production(storage.clone());
    let Some(identity) = identity else {
        // 非 Campaign 路径：保持旧行为（直接写 store）
        if let Some(outcome) = outcome {
            crate::backend_workflows::persist_postprocess_outcome_async(
                storage.clone(),
                &writing_ctx,
                outcome,
                present_chars,
            )
            .await
            .map_err(ProductionPostprocessError::BatchConstruction)?;
        }
        return Ok(false);
    };

    // Batch source / adapter selection happens in the named backend adapter
    // (Gate 3): SQLite requires the CampaignRuntimeContext snapshot and never
    // falls back to the JSON store.
    let service = crate::backend_workflows::build_postprocess_service(
        storage.as_ref(),
        runtime.as_deref(),
        &sink,
    )?;
    let result = service.apply_outcome(&identity, outcome, &present_chars, &cancel);

    match result {
        Ok(result) if result.applied => {
            let event = postprocess_pipeline_event(Some(&Ok(result)), "", false)
                .expect("applied result must derive a Done event");
            let _ = event_tx.send(event);
            Ok(true)
        }
        Ok(result) => {
            if let Some(reason) = result.skipped_reason.as_deref() {
                tracing::warn!("Phase A: postprocess 写回跳过——{reason}");
            }
            // 取消不是失败：发 PostProcessSkipped（前端置 idle），
            // 不再误发 PostProcessFailed（前端会显示「后处理失败」错误态）。
            // 其它 skipped 原因（late_or_superseded_attempt 等）不产生事件。
            if let Some(event) = postprocess_pipeline_event(Some(&Ok(result)), "", false) {
                let _ = event_tx.send(event);
            }
            Ok(false)
        }
        Err(e) => {
            tracing::error!("Phase A: postprocess 失败: {e}");
            let combined = service_fail_turn(&sink, &identity, e);
            if let Some(event) = postprocess_pipeline_event(
                Some(&Err(combined.clone())),
                &combined.to_string(),
                false,
            ) {
                let _ = event_tx.send(event);
            }
            Err(combined)
        }
    }
}

/// 三.3：从 postprocess 执行结果派生 Pipeline 事件——生产代码与测试共用的
/// **唯一**事件派生点（禁止手拼事件字符串）。
///
/// 四条路径恰好各产生一个正确事件：
/// - 正常落盘（`Ok(applied=true)`）→ `PostProcessDone{k,v,t}`；
/// - runner 前/后取消（`cancelled=true`，或结果带 `skipped_reason="cancelled"`）
///   → 恰好一个 `PostProcessSkipped`；
/// - 持久化失败（`Err`）→ 恰好一个 `PostProcessFailed{fail_reason}`；
/// - 其它 skipped 原因不产生事件。
///
/// `run_shared_postprocess_background` 的四个分支全部经此函数发送。
pub fn postprocess_pipeline_event(
    result: Option<
        &Result<
            production_postprocess::ProductionPostprocessResult,
            production_postprocess::ProductionPostprocessError,
        >,
    >,
    fail_reason: &str,
    cancelled: bool,
) -> Option<PipelineEvent> {
    if cancelled {
        return Some(PipelineEvent::PostProcessSkipped {
            reason: "postprocess cancelled".into(),
        });
    }
    match result {
        Some(Ok(result)) if result.applied => {
            let (knowledge_count, variable_count, task_count) = result
                .outcome
                .as_ref()
                .and_then(|o| o.post_process.as_ref())
                .map(|pp| {
                    (
                        pp.knowledge_updates.len(),
                        pp.variable_updates.len(),
                        pp.task_updates.len(),
                    )
                })
                .unwrap_or((0, 0, 0));
            Some(PipelineEvent::PostProcessDone {
                knowledge_count,
                variable_count,
                task_count,
            })
        }
        Some(Ok(result)) if result.skipped_reason.as_deref() == Some("cancelled") => {
            Some(PipelineEvent::PostProcessSkipped {
                reason: "postprocess cancelled".into(),
            })
        }
        Some(Ok(_)) => None,
        Some(Err(_)) => Some(PipelineEvent::PostProcessFailed {
            reason: fail_reason.to_string(),
        }),
        None => None,
    }
}

pub(crate) fn service_fail_turn(
    sink: &BackendTurnAttemptSink<'_>,
    identity: &production_postprocess::PostprocessIdentity,
    original: production_postprocess::ProductionPostprocessError,
) -> production_postprocess::ProductionPostprocessError {
    // Identity-validation errors must never mark/write the Turn.
    if matches!(
        original,
        production_postprocess::ProductionPostprocessError::ScopeMismatch { .. }
            | production_postprocess::ProductionPostprocessError::AttemptMissing { .. }
    ) {
        return original;
    }
    // Only mark Failed when this identity still owns the current writable Attempt.
    // Superseded / cancelled late errors are zero-write and keep the new Attempt intact.
    match sink.mark_failed_if_current(identity, original.to_string()) {
        Ok(_marked) => original,
        Err(mark_error) => production_postprocess::ProductionPostprocessError::MarkFailed {
            original: original.to_string(),
            mark_error,
        },
    }
}

/// 读取-修改-写回 TurnRecord 的便捷辅助。
pub(crate) fn update_turn_record<F>(
    storage: &storage_backend::StorageFacade,
    turn_id: &Id,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&mut storyforge_domain::turn::TurnRecord),
{
    storage.update_turn_record(turn_id, f)
}

/// 条件更新 TurnRecord：predicate 失败返回 Ok(false)，不改盘。
pub(crate) fn update_turn_record_if<P, M>(
    storage: &storage_backend::StorageFacade,
    turn_id: &Id,
    predicate: P,
    mutate: M,
) -> Result<bool, String>
where
    P: FnOnce(&storyforge_domain::turn::TurnRecord) -> bool,
    M: FnOnce(&mut storyforge_domain::turn::TurnRecord),
{
    storage.mutate_turn_if(turn_id, predicate, mutate)
}

/// 后处理结果只能写回仍属于当前草稿的 Attempt。
pub(crate) fn is_current_attempt_ready_for_postprocess(
    record: &storyforge_domain::turn::TurnRecord,
    attempt_id: &Id,
) -> bool {
    turn_lifecycle::is_current_attempt_ready_for_postprocess(record, attempt_id)
}

pub(crate) fn resolve_legacy_opening_message(
    character: Option<&Arc<storyforge_domain::character::Character>>,
    requested: Option<String>,
) -> Option<String> {
    let character = character?;
    resolve_opening_message_from_parts(
        &character.first_mes,
        &character.alternate_greetings,
        requested,
        "legacy",
    )
}

#[cfg(test)]
pub(crate) fn resolve_campaign_opening_message(
    store: &storage::CharacterStore,
    source_character_id: &Id,
    requested: Option<String>,
) -> Option<String> {
    let stored = stored_character_for_source_id(store, source_character_id)?;
    resolve_opening_message_from_parts(
        &stored.info.first_mes,
        &stored.info.alternate_greetings,
        requested,
        "campaign",
    )
}

pub(crate) fn stored_character_for_id_or_source_in_store(
    store: &storage::CharacterStore,
    character_id: &Id,
) -> Option<storage::StoredCharacter> {
    let id = character_id.as_str();
    store.get(id).or_else(|| {
        store
            .list()
            .into_iter()
            .find(|stored| stored.info.source_character_id.as_deref() == Some(id))
    })
}

pub(crate) fn stored_character_for_source_id(
    store: &storage::CharacterStore,
    source_character_id: &Id,
) -> Option<storage::StoredCharacter> {
    stored_character_for_id_or_source_in_store(store, source_character_id)
}

pub(crate) fn resolve_opening_message_from_parts(
    first_mes: &str,
    alternate_greetings: &[String],
    requested: Option<String>,
    source_label: &str,
) -> Option<String> {
    let mut choices = Vec::new();
    if !first_mes.trim().is_empty() {
        choices.push(first_mes.to_string());
    }
    choices.extend(
        alternate_greetings
            .iter()
            .filter(|greeting| !greeting.trim().is_empty())
            .cloned(),
    );

    let requested = requested.filter(|message| !message.trim().is_empty());
    if let Some(message) = requested {
        if choices.iter().any(|choice| choice == &message) {
            return Some(message);
        }
        tracing::warn!(
            "Ignoring {source_label} opening_message that is not present on the source character"
        );
    }

    choices.into_iter().next()
}

/// 从模块/Profile 存储加载预设配置到 WritingContext
///
/// 无 Profile 时不动 ctx（profile 保持 None → 流水线用硬编码常量兜底）。
pub(crate) fn collect_campaign_scoped_regex_scripts(
    campaign_id: &Id,
    store: &campaign_store::CampaignStore,
) -> Vec<RegexScript> {
    store
        .get_campaign(campaign_id)
        .and_then(|campaign| store.get_card(&campaign.card_id))
        .map(|stored_card| stored_card.card.scoped_regex_scripts())
        .unwrap_or_default()
}

/// 三.4 后端中立角色解析器：把 stored / source / card / name 任意 ID 映射到
/// 领域 Character 快照，**经 facade**（`get_character(id_or_source)` 命中 stored
/// 或 source id；`get_card` 命中 card id；name 走存储角色的名字匹配）——绝不
/// 触达 `json_character_store`，JSON 与 SQLite 走同一解析语义。
///
/// 解析语义（与旧 JSON-only 实现逐项对齐）：
/// source_id（存储角色 source_character_id 或直传 id）→ Character.id 精确匹配，
/// 直传 id 直接匹配 Character.id，stored_name 匹配 Character.name。
pub fn collect_scoped_regex_scripts_for_backend(
    character_id: Option<&str>,
    characters: &[std::sync::Arc<storyforge_domain::character::Character>],
    storage: Option<&crate::storage_backend::StorageFacade>,
) -> Result<Vec<RegexScript>, String> {
    let Some(character_id) = character_id else {
        return Ok(Vec::new());
    };
    let Some(facade) = storage else {
        return Ok(Vec::new());
    };

    // 1) stored / source id：经 facade 角色库解析。
    let mut stored = facade
        .get_character(character_id)
        .map_err(|e| format!("character resolver: 角色库读取失败: {e}"))?;
    // 2) name 键：角色库按名字匹配（backend-neutral，JSON/SQLite 同一语义）。
    if stored.is_none() {
        stored = facade
            .list_characters()
            .map_err(|e| format!("character resolver: 角色库列表读取失败: {e}"))?
            .into_iter()
            .find(|c| c.info.name == character_id);
    }
    // 3) card id 兜底：card.source_character_id → Character.id。
    let mut source_id = stored
        .as_ref()
        .and_then(|stored| stored.info.source_character_id.as_deref())
        .map(str::to_string);
    if source_id.is_none() {
        source_id = facade
            .get_card(&Id::from_str(character_id))
            .map_err(|e| format!("character resolver: 卡片读取失败: {e}"))?
            .map(|card| card.card.source_character_id.as_str().to_string());
    }
    let source_id = source_id.unwrap_or_else(|| character_id.to_string());
    let stored_name = stored.as_ref().map(|stored| stored.info.name.as_str());

    Ok(characters
        .iter()
        .find(|character| {
            character.id.as_str() == source_id
                || character.id.as_str() == character_id
                || stored_name.is_some_and(|name| character.name == name)
        })
        .map(|character| character.scoped_regex_scripts())
        .unwrap_or_default())
}

pub(crate) fn merge_runtime_regex_scripts(
    scoped_scripts: Vec<RegexScript>,
    preset_store: &PresetStore,
    global_regex_store: &global_regex_store::GlobalRegexStore,
) -> Vec<RegexScript> {
    let global_scripts = global_regex_store.list();
    let preset_scripts = preset_store
        .active()
        .map(|stored| stored.preset.regex_scripts)
        .unwrap_or_default();

    merge_regex_script_sources(&global_scripts, &preset_scripts, &scoped_scripts)
}

pub(crate) fn fill_regex_context(
    ctx: &mut WritingContext,
    preset_store: &PresetStore,
    global_regex_store: &global_regex_store::GlobalRegexStore,
) {
    let scoped_scripts = std::mem::take(&mut ctx.regex_scripts);
    ctx.regex_scripts =
        merge_runtime_regex_scripts(scoped_scripts, preset_store, global_regex_store);
}

pub(crate) fn fill_profile_context(ctx: &mut WritingContext, state: &Arc<AppState>) {
    // 加载活跃 Profile
    if let Some(profile) = state.profile_store.get_active() {
        // 加载所有启用的模块
        let modules: Vec<_> = state
            .module_store
            .list_all()
            .into_iter()
            .filter(|(_, enabled)| *enabled)
            .map(|(m, _)| m)
            .collect();
        ctx.profile = Some(profile);
        ctx.modules = modules;
    }
}

/// 从活跃 Agent Profile Config 加载运行时配置覆盖到 WritingContext
///
/// 无活跃配置时不动 ctx（agent_profile_config 保持 None → 流水线用硬编码默认值）。
pub(crate) fn fill_agent_profile_context(ctx: &mut WritingContext, state: &Arc<AppState>) {
    let config = state.agent_profile_config_store.get_active();
    ctx.agent_profile_config = Some(config);
}

/// 从活跃 Campaign 填充 WritingContext 的 P2 字段（campaign_id / turn / pending_tasks / story_clock）
///
/// 无活跃 Campaign 时不动 ctx（campaign_id 保持 None → 后处理跳过）。
/// 从活跃 Campaign 填充 P2 字段（任务注入导演 / 后处理需要）。
///
/// 优先读内存 `state.active_campaign`，磁盘 `load_active_campaign` 仅作 fallback。
/// 历史 bug：旧实现绕过内存直接读磁盘，若 `set_active_campaign` 先改内存后写盘
/// 但写盘失败（非原子），会用旧/空 campaign。
#[cfg(test)]
pub(crate) fn fill_campaign_context(ctx: &mut WritingContext, state: &AppState) {
    clear_campaign_runtime(ctx, &state.tool_ctx);

    let active_id = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        resolve_active_campaign_with_legacy_fallback(
            guard.clone(),
            &state.data_dir,
            state.storage(),
        )
    };
    let Some(active_id) = active_id else {
        return;
    };
    match crate::backend_workflows::load_campaign_context_snapshot_for_backend(
        state.storage(),
        &active_id,
    ) {
        Ok(Some(snapshot)) => apply_campaign_context_snapshot(ctx, &state.tool_ctx, snapshot),
        Ok(None) => {}
        Err(error) => tracing::warn!("test Campaign context load failed: {error}"),
    }
}

pub(crate) async fn fill_campaign_context_async(
    ctx: &mut WritingContext,
    state: &Arc<AppState>,
) -> Result<(), TauriCommandError> {
    clear_campaign_runtime(ctx, &state.tool_ctx);

    let memory_active_id = {
        let guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };
    let data_dir = state.data_dir.clone();
    let storage = state.storage().clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        let active_id =
            resolve_active_campaign_with_legacy_fallback(memory_active_id, &data_dir, &storage);
        let Some(active_id) = active_id else {
            return Ok(None);
        };
        crate::backend_workflows::load_campaign_context_snapshot_for_backend(
            storage.as_ref(),
            &active_id,
        )
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("加载 Campaign 快照任务失败: {e}")))?
    .map_err(TauriCommandError::internal)?;

    if let Some(snapshot) = snapshot {
        apply_campaign_context_snapshot(ctx, &state.tool_ctx, snapshot);
    }
    Ok(())
}

pub(crate) fn clear_campaign_runtime(
    ctx: &mut WritingContext,
    tool_ctx: &Arc<RwLock<ToolContext>>,
) {
    // 阶段 2 cleanup：先清空旧 runtime，避免 stale 数据残留。
    ctx.campaign_runtime = None;
    ctx.recent_summaries.clear();
    ctx.far_memory_hits.clear();
    let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    tool_guard.campaign_runtime = None;
    tool_guard.archived_summaries.clear();
    tool_guard.chronicle_summaries.clear();
}

/// 取 `before_node_id` 之前最近一条 User 消息正文（用于 regenerate 无 hint 时的远记忆查询）。
pub(crate) fn last_user_intent_before(
    conv_store: &ConversationStore,
    conv_id: &Id,
    before_node_id: &Id,
) -> Option<String> {
    let conv = conv_store.get(conv_id)?;
    let pos = conv.nodes.iter().position(|n| &n.id == before_node_id)?;
    for node in conv.nodes[..pos].iter().rev() {
        let Some(v) = node.active() else {
            continue;
        };
        if v.status == storyforge_domain::conversation::VariantStatus::Discarded {
            continue;
        }
        if v.role != storyforge_domain::conversation::Role::User {
            continue;
        }
        let content = v.content.trim();
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }
    None
}

/// 按用户意图从向量库召回 ArchivedSummary（混合：关键词 + 可选嵌入，best-effort）。
///
/// 失败只 warn，不阻断写作。无命中时 `far_memory_hits` 为空。
/// 有 campaign 时优先过滤同 campaign 记录，兼容无标签的旧归档。
/// 配置了 Embedder 时走向量+关键词合并；否则纯关键词。
pub(crate) async fn fill_far_memory_hits(ctx: &mut WritingContext, state: &AppState, intent: &str) {
    ctx.far_memory_hits.clear();
    let intent = intent.trim();
    if intent.is_empty() {
        return;
    }
    let campaign_filter = ctx.campaign_id.as_ref().map(|id| id.to_string());
    let embedder = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .and_then(|cfg| match storyforge_infra_llm::Embedder::new(cfg) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::debug!(target: "far_memory", "构建 Embedder 失败，关键词路径: {e}");
                None
            }
        });
    match storyforge_app_memory::recall_archived_hybrid(
        state.vector_store.as_ref(),
        intent,
        3,
        campaign_filter.as_deref(),
        embedder.as_ref(),
    )
    .await
    {
        Ok(hits) => {
            if !hits.is_empty() {
                tracing::info!(
                    target: "far_memory",
                    "远记忆召回 {} 条（hybrid={}）ids={:?}",
                    hits.len(),
                    embedder.is_some(),
                    hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>()
                );
            }
            // 保留 id/score/kind 溯源；注入文本仍只用 content
            ctx.far_memory_hits = hits
                .into_iter()
                .map(|h| {
                    storyforge_app_pipeline::FarMemoryHit::new(h.id, h.content, h.score, h.kind)
                })
                .collect();
        }
        Err(e) => {
            tracing::warn!(target: "far_memory", "远记忆召回失败（跳过）: {e}");
        }
    }
}

/// 把已 accept 的 RoundSummary 索引进向量库。
///
/// - 始终写关键词（无 Embedder 也能召回）
/// - 若传入 `embedder`，best-effort 嵌入真实向量供 hybrid 召回
/// - 幂等：以 summary.id 为向量记录 id upsert
pub(crate) fn index_round_summaries_to_vector(
    vector_store: &dyn VectorStore,
    batch: &storyforge_domain::turn::MutationBatch,
) {
    // 同步关键词路径（commit 关键不阻塞等嵌入）
    index_round_summaries_to_vector_with_vectors(vector_store, batch, &[]);
}

/// 同 `index_round_summaries_to_vector`，但允许预计算向量（id → vector）。
pub(crate) fn index_round_summaries_to_vector_with_vectors(
    vector_store: &dyn VectorStore,
    batch: &storyforge_domain::turn::MutationBatch,
    vectors: &[(Id, Vec<f32>)],
) {
    for mutation in &batch.mutations {
        let storyforge_domain::turn::Mutation::UpsertSummary(summary) = mutation else {
            continue;
        };
        let content = summary.content.trim();
        if content.is_empty() {
            continue;
        }
        // extract_keywords 取高频；extract_query_tokens 补全 query 侧 2-gram/词，
        // 合并后召回与 intent 分词更对齐。
        let mut keywords = storyforge_app_memory::extract_keywords(content);
        for t in storyforge_app_memory::extract_query_tokens(content) {
            if !keywords.iter().any(|k| k == &t) {
                keywords.push(t);
            }
        }
        if keywords.is_empty() {
            continue;
        }
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "campaign_id".into(),
            serde_json::Value::String(summary.campaign_id.to_string()),
        );
        metadata.insert(
            "conversation_id".into(),
            serde_json::Value::String(summary.conversation_id.to_string()),
        );
        metadata.insert(
            "turn".into(),
            serde_json::Value::Number(summary.turn.into()),
        );
        metadata.insert(
            "source".into(),
            serde_json::Value::String("round_summary".into()),
        );
        // 记忆规格 M1：统一 source_kind / code / lineage（兼容旧检索）
        metadata.insert(
            "source_kind".into(),
            serde_json::Value::String("chronicle_a".into()),
        );
        metadata.insert(
            "source_entry_id".into(),
            serde_json::Value::String(summary.id.to_string()),
        );
        if let Some(code) = &summary.code {
            metadata.insert("code".into(), serde_json::Value::String(code.clone()));
        }
        if let Some(lin) = &summary.lineage_id {
            metadata.insert(
                "lineage_id".into(),
                serde_json::Value::String(lin.to_string()),
            );
        }
        if let Some(h) = &summary.headline {
            metadata.insert("headline".into(), serde_json::Value::String(h.clone()));
        }
        let vector = vectors
            .iter()
            .find(|(id, _)| id == &summary.id)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        if let Err(e) = vector_store.upsert(VectorRecord {
            id: summary.id.clone(),
            content: content.to_string(),
            vector,
            keywords,
            kind: VectorKind::ArchivedSummary,
            metadata,
        }) {
            tracing::warn!(
                target: "far_memory",
                "RoundSummary {} 入向量库失败: {e}",
                summary.id
            );
        } else {
            tracing::debug!(
                target: "far_memory",
                "RoundSummary {} 已索引到远记忆向量库",
                summary.id
            );
        }
    }
}

/// 后台：为 batch 中的 RoundSummary 补齐嵌入向量（有 Embedder 时）。
///
/// 先关键词落盘保证可召回；嵌入成功后覆盖 upsert 写入真实向量。
pub(crate) async fn index_round_summaries_async(
    state: Arc<AppState>,
    batch: storyforge_domain::turn::MutationBatch,
) {
    // 1. 关键词立即入库
    index_round_summaries_to_vector(state.vector_store.as_ref(), &batch);

    // 2. 可选嵌入补齐
    let config = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let Some(config) = config else {
        return;
    };
    let embedder = match storyforge_infra_llm::Embedder::new(config) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(target: "far_memory", "RoundSummary 嵌入跳过（Embedder 构建失败）: {e}");
            return;
        }
    };

    let mut vectors = Vec::new();
    for mutation in &batch.mutations {
        let storyforge_domain::turn::Mutation::UpsertSummary(summary) = mutation else {
            continue;
        };
        let content = summary.content.trim();
        if content.is_empty() {
            continue;
        }
        match embedder.embed(content).await {
            Ok(v) => vectors.push((summary.id.clone(), v)),
            Err(e) => {
                tracing::warn!(
                    target: "far_memory",
                    "RoundSummary {} 嵌入失败（保留关键词索引）: {e}",
                    summary.id
                );
            }
        }
    }
    if !vectors.is_empty() {
        index_round_summaries_to_vector_with_vectors(state.vector_store.as_ref(), &batch, &vectors);
        tracing::info!(
            target: "far_memory",
            "RoundSummary 嵌入补齐 {} 条",
            vectors.len()
        );
    }
}

pub struct CampaignContextSnapshot {
    active_id: Id,
    story_clock: String,
    turn: u32,
    pending_tasks: Vec<storyforge_domain::story_task::StoryTask>,
    scoped_regex_scripts: Vec<RegexScript>,
    runtime: Arc<CampaignRuntimeContext>,
    /// 近期 RoundSummary（turn 升序），供 Director tail + get_recent_summary
    recent_summaries: Vec<storyforge_domain::agent::RoundSummary>,
    /// search/get_chronicle 目录（含 B/C + 较远 A）
    chronicle_tool_catalog: Vec<storyforge_domain::agent::RoundSummary>,
    /// 渲染 overview/band 的完整条目（不受 last-12 截断）
    chronicle_prompt_catalog: Vec<storyforge_domain::agent::RoundSummary>,
}

/// ContextCompiler load-side：写入 WritingContext 的近期摘要条数上限。
/// turn 计数仍用全量 list_summaries.len()；注入侧再取 last 5。
const RECENT_SUMMARIES_LOAD_LIMIT: usize = 12;
/// 按 turn 升序后只保留最近 `limit` 条，避免长战役全量摘要进内存/工具上下文。
pub(crate) fn take_recent_summaries_for_context(
    mut summaries: Vec<storyforge_domain::agent::RoundSummary>,
    limit: usize,
) -> Vec<storyforge_domain::agent::RoundSummary> {
    summaries.sort_by_key(|s| s.turn);
    if limit == 0 {
        return Vec::new();
    }
    if summaries.len() > limit {
        let drop_n = summaries.len() - limit;
        summaries.drain(0..drop_n);
    }
    summaries
}

/// 渲染 ContextEpochSnapshot 用的 catalog：snapshot 引用的 code + 全部 leaf A。
pub(crate) fn build_chronicle_prompt_catalog(
    all: &[storyforge_domain::agent::RoundSummary],
    frozen: Option<&storyforge_domain::chronicle::ContextEpochSnapshot>,
) -> Vec<storyforge_domain::agent::RoundSummary> {
    use std::collections::HashSet;
    let mut needed: HashSet<String> = HashSet::new();
    if let Some(snap) = frozen {
        for c in &snap.overview_codes {
            needed.insert(c.as_str().to_string());
        }
        for c in &snap.band_codes {
            needed.insert(c.as_str().to_string());
        }
    }
    let mut out: Vec<_> = all
        .iter()
        .filter(|s| s.code.as_ref().map(|c| needed.contains(c)).unwrap_or(false) || s.is_leaf_a())
        .cloned()
        .collect();
    let mut seen = HashSet::new();
    out.retain(|s| seen.insert(s.id.to_string()));
    out.sort_by_key(|s| s.effective_turn_end());
    out
}

/// 已提交 Turn 数：只计 Chronicle A（leaf），不计 B/C stage。
pub(crate) fn committed_turn_count(summaries: &[storyforge_domain::agent::RoundSummary]) -> u32 {
    summaries
        .iter()
        .filter(|s| s.is_leaf_a())
        .map(|s| s.turn)
        .max()
        .unwrap_or(0)
}

/// 下一写作轮次 = max(A.turn) + 1（无 A 时为 1）。
pub(crate) fn next_writing_turn(summaries: &[storyforge_domain::agent::RoundSummary]) -> u32 {
    committed_turn_count(summaries).saturating_add(1)
}

/// 工具侧 Chronicle 目录：优先保留全部 B/C，再保留最近的 A（按 turn_end）。
#[cfg(test)]
pub(crate) fn build_chronicle_tool_catalog(
    summaries: Vec<storyforge_domain::agent::RoundSummary>,
    limit: usize,
) -> Vec<storyforge_domain::agent::RoundSummary> {
    if limit == 0 {
        return Vec::new();
    }
    let mut stages: Vec<_> = summaries.iter().filter(|s| s.level > 0).cloned().collect();
    stages.sort_by_key(|s| s.effective_turn_end());
    let mut leaves: Vec<_> = summaries.into_iter().filter(|s| s.level == 0).collect();
    leaves.sort_by_key(|s| s.turn);

    if stages.len() >= limit {
        // 极端：B/C 已超 cap → 保留最新 stage
        let drop_n = stages.len() - limit;
        stages.drain(0..drop_n);
        return stages;
    }
    let leaf_room = limit - stages.len();
    if leaves.len() > leaf_room {
        let drop_n = leaves.len() - leaf_room;
        leaves.drain(0..drop_n);
    }
    stages.extend(leaves);
    stages.sort_by_key(|s| s.effective_turn_end());
    stages
}

/// 旧存档：缺 lineage 的 summary 回填为 campaign.lineage_id 并落盘。
pub(crate) fn backfill_summary_lineage_if_needed(
    store: &campaign_store::CampaignStore,
    lineage: &Id,
    summaries: &mut [storyforge_domain::agent::RoundSummary],
) {
    for s in summaries.iter_mut() {
        if s.lineage_id.is_none() {
            s.lineage_id = Some(lineage.clone());
            if let Err(e) = store.update_summary_by_id(s.clone()) {
                tracing::warn!(
                    target: "context_compiler",
                    "backfill summary lineage failed id={}: {e}",
                    s.id
                );
            }
        }
    }
}

/// ensure campaign.lineage_id；若新建则立即 persist。
pub(crate) fn ensure_campaign_lineage_persisted(
    store: &campaign_store::CampaignStore,
    camp: &mut storyforge_domain::campaign::Campaign,
) -> Id {
    let created = camp.lineage_id.is_none();
    let lineage = camp.ensure_lineage_id().clone();
    if created && let Err(e) = store.update_campaign(camp.clone()) {
        tracing::warn!(
            target: "context_compiler",
            "persist new lineage_id failed: {e}"
        );
    }
    lineage
}

pub(crate) fn load_campaign_context_snapshot(
    store: &campaign_store::CampaignStore,
    active_id: &Id,
) -> Option<CampaignContextSnapshot> {
    let mut camp = store.get_campaign(active_id)?;
    let lineage = ensure_campaign_lineage_persisted(store, &mut camp);
    // Gate 4：story_clock 唯一权威 = variables["story_clock"]。
    let story_clock = camp.current_story_clock().to_string();
    // turn 只计 A；注入 recent last-K；prompt catalog 覆盖 snapshot codes；工具目录含 B/C
    let mut all_summaries = store.list_summaries(active_id);
    backfill_summary_lineage_if_needed(store, &lineage, &mut all_summaries);
    let (epoch_snap, _) = refresh_and_persist_context_epoch(store, &mut camp, &all_summaries);
    let _ = epoch_snap; // applied via camp.context_epoch into runtime.campaign
    // 锁内可能已合并压缩结果：重新 list 供 catalog / turn 使用
    let all_summaries = store.list_summaries(active_id);
    let turn = next_writing_turn(&all_summaries);
    let chronicle_prompt_catalog =
        build_chronicle_prompt_catalog(&all_summaries, camp.context_epoch.as_ref());
    // 全量目录：get_chronicle 按 code 点名旧 A 不受 256 截断；search 仍有 limit/预算
    let chronicle_tool_catalog = all_summaries.clone();
    let recent_summaries =
        take_recent_summaries_for_context(all_summaries, RECENT_SUMMARIES_LOAD_LIMIT);
    let tasks = store.list_tasks(active_id);
    let instances = store.list_instances(active_id);
    let knowledge = store.list_knowledge(active_id);

    let stored_card = store.get_card(&camp.card_id);
    let scoped_regex_scripts = stored_card
        .as_ref()
        .map(|stored| stored.card.scoped_regex_scripts())
        .unwrap_or_default();
    let definitions_by_id: std::collections::HashMap<
        Id,
        storyforge_domain::character::CharacterDefinition,
    > = stored_card
        .map(|stored| {
            stored
                .card
                .character_definitions
                .into_iter()
                .map(|def| (def.id.clone(), def))
                .collect()
        })
        .unwrap_or_default();

    let runtime = Arc::new(CampaignRuntimeContext {
        campaign: camp,
        instances,
        definitions_by_id,
        knowledge,
        tasks: tasks.clone(),
        turn,
    });

    Some(CampaignContextSnapshot {
        active_id: active_id.clone(),
        story_clock,
        turn,
        pending_tasks: tasks,
        scoped_regex_scripts,
        runtime,
        recent_summaries,
        chronicle_tool_catalog,
        chronicle_prompt_catalog,
    })
}

/// SQLite opt-in version of the Campaign context compiler input. The writing
/// pipeline receives the same immutable domain snapshot as the JSON backend,
/// but every Campaign/instance/task/knowledge/summary read and every epoch
/// refresh write uses the process-owned SQLite authority.
/// Public for harness SQLite endurance: same snapshot path as production opt-in.
pub fn load_sqlite_campaign_context_snapshot(
    storage: &storage_backend::StorageFacade,
    active_id: &Id,
) -> Result<Option<CampaignContextSnapshot>, String> {
    let Some(mut camp) = storage
        .get_campaign(active_id)?
        .map(|record| record.campaign)
    else {
        return Ok(None);
    };

    let mut campaign_changed = false;
    if camp.lineage_id.is_none() {
        camp.ensure_lineage_id();
        campaign_changed = true;
    }
    let all_summaries = storage.list_summaries(active_id)?;
    let (epoch_snap, _membership, should_persist_epoch, bumped_revision) =
        compute_context_epoch_refresh_parts(&camp, &all_summaries);
    if should_persist_epoch {
        camp.chronicle_revision = bumped_revision;
        camp.context_epoch = Some(epoch_snap.clone());
        campaign_changed = true;
    }
    if campaign_changed {
        storage.save_campaign(&camp)?;
    }

    let all_summaries = storage.list_summaries(active_id)?;
    let turn = next_writing_turn(&all_summaries);
    let chronicle_prompt_catalog =
        build_chronicle_prompt_catalog(&all_summaries, camp.context_epoch.as_ref());
    let recent_summaries =
        take_recent_summaries_for_context(all_summaries.clone(), RECENT_SUMMARIES_LOAD_LIMIT);
    let tasks = storage.list_tasks(active_id)?;
    let instances = storage.list_instances(active_id)?;
    let knowledge = storage.list_knowledge(active_id)?;

    let stored_card = match storage.get_card_payload(&camp.card_id)? {
        Some(payload) => {
            match serde_json::from_value::<campaign_store::StoredCard>(payload.clone()) {
                Ok(stored) => Some(stored),
                Err(stored_error) => {
                    let card = serde_json::from_value::<
                        storyforge_domain::character::CharacterCard,
                    >(payload)
                    .map_err(|card_error| {
                        format!(
                            "SQLite Campaign card payload decode failed: stored={stored_error}; card={card_error}"
                        )
                    })?;
                    Some(campaign_store::StoredCard {
                        card,
                        imported_at: String::new(),
                    })
                }
            }
        }
        None => {
            return Err(format!(
                "SQLite Campaign {} references missing card {}",
                camp.id, camp.card_id
            ));
        }
    };
    let scoped_regex_scripts = stored_card
        .as_ref()
        .map(|stored| stored.card.scoped_regex_scripts())
        .unwrap_or_default();
    let definitions_by_id = stored_card
        .map(|stored| {
            stored
                .card
                .character_definitions
                .into_iter()
                .map(|definition| (definition.id.clone(), definition))
                .collect()
        })
        .unwrap_or_default();

    // Gate 4：story_clock 唯一权威 = variables["story_clock"]。
    let story_clock = camp.current_story_clock().to_string();
    let runtime = Arc::new(CampaignRuntimeContext {
        campaign: camp,
        instances,
        definitions_by_id,
        knowledge,
        tasks: tasks.clone(),
        turn,
    });
    Ok(Some(CampaignContextSnapshot {
        active_id: active_id.clone(),
        story_clock,
        turn,
        pending_tasks: tasks,
        scoped_regex_scripts,
        runtime,
        recent_summaries,
        chronicle_tool_catalog: all_summaries,
        chronicle_prompt_catalog,
    }))
}

/// Apply a pre-built campaign context snapshot (JSON or SQLite).
pub fn apply_campaign_context_snapshot(
    ctx: &mut WritingContext,
    tool_ctx: &Arc<RwLock<ToolContext>>,
    snapshot: CampaignContextSnapshot,
) {
    ctx.campaign_id = Some(snapshot.active_id);
    ctx.story_clock = snapshot.story_clock;
    ctx.turn = snapshot.turn;
    ctx.pending_tasks = snapshot.pending_tasks;
    append_missing_campaign_scoped_regex_scripts(ctx, snapshot.scoped_regex_scripts);
    ctx.campaign_runtime = Some(snapshot.runtime.clone());
    ctx.recent_summaries = snapshot.recent_summaries;
    ctx.chronicle_prompt_catalog = snapshot.chronicle_prompt_catalog;
    ctx.context_epoch = snapshot.runtime.campaign.context_epoch.clone();
    ctx.chronicle_revision = snapshot.runtime.campaign.chronicle_revision;

    let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    tool_guard.campaign_runtime = Some(snapshot.runtime);
    // 同步到 get_recent_summary 工具：content 列表（升序，工具内部 rev().take）
    tool_guard.archived_summaries = ctx
        .recent_summaries
        .iter()
        .map(|s| s.content.clone())
        .collect();
    // 同步到 search_chronicle / get_chronicle（A/B/C 目录，宽于 inject 窗口）
    tool_guard.chronicle_summaries = snapshot.chronicle_tool_catalog;
    tool_guard.reset_chronicle_tool_budget();
}

/// 激活时把 Campaign runtime 同步进 tool_ctx（无 WritingContext 场景）。
///
/// 2026-09-01 B5 复验：Meta 会话的 campaign_runtime 来自 tool_ctx 快照，
/// 此前只在第一次写作（fill_campaign_context）时填充——重启恢复/手动切换
/// 活动后、未写作前，Meta 工具一律回答「当前没有 active Campaign」。
/// 与 apply_campaign_context_snapshot 的 tool_ctx 侧保持同一写入集。
pub(crate) fn apply_campaign_runtime_to_tool_ctx(
    state: &AppState,
    snapshot: CampaignContextSnapshot,
) {
    let mut tool_guard = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
    tool_guard.campaign_runtime = Some(snapshot.runtime);
    tool_guard.archived_summaries = snapshot
        .recent_summaries
        .iter()
        .map(|s| s.content.clone())
        .collect();
    tool_guard.chronicle_summaries = snapshot.chronicle_tool_catalog;
    tool_guard.reset_chronicle_tool_budget();
}

/// 在 Context 编译入口刷新 ContextEpochSnapshot 并可选落盘。
///
/// - live_suffix 满 E → rollover（新 overview/band/anchor）
/// - 同 epoch 内 overview/band 冻结
/// - created/rollover 时 bump chronicle_revision
///
/// **并发边界**：与 Compressor publish/heal 共用 `with_campaign_lock`。
/// 锁内重新读取最新 Campaign + summaries 再计算并落盘，避免用过期快照回写
/// 覆盖压缩后的 chronicle_revision / epoch / pending marker。
///
/// `all_summaries` 仅作锁失败时的无落盘兜底输入；成功路径以锁内 list 为准。
pub(crate) fn refresh_and_persist_context_epoch(
    store: &campaign_store::CampaignStore,
    camp: &mut storyforge_domain::campaign::Campaign,
    all_summaries: &[storyforge_domain::agent::RoundSummary],
) -> (
    storyforge_domain::chronicle::ContextEpochSnapshot,
    storyforge_domain::chronicle::EpochMembership,
) {
    let campaign_id = camp.id.clone();
    let locked = turn_coordinator::with_campaign_lock(|| {
        let Some(mut latest) = store.get_campaign(&campaign_id) else {
            return Err(turn_coordinator::CommitError::CampaignNotFound(
                campaign_id.clone(),
            ));
        };
        let summaries = store.list_summaries(&campaign_id);
        let (snap, membership, should_persist, bumped_rev) =
            compute_context_epoch_refresh_parts(&latest, &summaries);

        if should_persist {
            latest.chronicle_revision = bumped_rev;
            latest.context_epoch = Some(snap.clone());
            // 不触碰 pending_compress_publication：压缩半提交仍由 heal 收口
            store
                .update_campaign(latest.clone())
                .map_err(turn_coordinator::CommitError::Storage)?;
            tracing::debug!(
                target: "context_compiler",
                epoch_id = %snap.epoch_id,
                chronicle_revision = latest.chronicle_revision,
                overview = snap.overview_codes.len(),
                band = snap.band_codes.len(),
                "context epoch refreshed under campaign lock"
            );
        }

        *camp = store.get_campaign(&campaign_id).unwrap_or(latest);
        Ok((snap, membership))
    });

    match locked {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "context_compiler",
                "refresh context_epoch under lock failed: {e}; in-memory compute without persist"
            );
            let (snap, membership, _, _) = compute_context_epoch_refresh_parts(camp, all_summaries);
            (snap, membership)
        }
    }
}

/// 纯计算：基于给定 camp/summaries 产出 snapshot + membership + 是否需要 persist + 新 revision。
pub(crate) fn compute_context_epoch_refresh_parts(
    camp: &storyforge_domain::campaign::Campaign,
    all_summaries: &[storyforge_domain::agent::RoundSummary],
) -> (
    storyforge_domain::chronicle::ContextEpochSnapshot,
    storyforge_domain::chronicle::EpochMembership,
    bool,
    u64,
) {
    use storyforge_domain::chronicle::{
        ChronicleCode, ChronicleLevel, ContextWindowParams, OverviewCandidate,
        committed_turns_from_count, refresh_context_epoch, sequence_from_committed_turn_id,
    };

    // 只基于 A 的 max turn（B/C 不增加 committed turn 序列长度）
    let n = committed_turn_count(all_summaries);
    let committed = committed_turns_from_count(n);

    let mut cands: Vec<OverviewCandidate> = Vec::new();
    for s in all_summaries {
        let level = s.chronicle_level();
        let code = s
            .code
            .as_deref()
            .and_then(ChronicleCode::parse)
            .unwrap_or_else(|| ChronicleCode::new(level, s.turn));
        cands.push(OverviewCandidate {
            code,
            level,
            turn_start: s.turn,
            covered_by: s.covered_by.clone(),
        });
    }

    let code_by_turn: std::collections::HashMap<u32, ChronicleCode> = all_summaries
        .iter()
        .map(|s| {
            let code = s
                .code
                .as_deref()
                .and_then(ChronicleCode::parse)
                .unwrap_or_else(|| ChronicleCode::new(ChronicleLevel::A, s.turn));
            (s.turn, code)
        })
        .collect();

    let band_lookup =
        |id: &Id| sequence_from_committed_turn_id(id).and_then(|t| code_by_turn.get(&t).cloned());

    let params = ContextWindowParams::default();
    let existing = camp.context_epoch.clone();
    let result = refresh_context_epoch(
        existing.as_ref(),
        &committed,
        &cands,
        &band_lookup,
        params,
        camp.chronicle_revision,
    );

    let mut rev = camp.chronicle_revision;
    let should_bump = result.should_bump_chronicle_revision();
    if should_bump {
        rev = rev.saturating_add(1);
    }
    let mut snap = result.snapshot;
    snap.chronicle_revision = rev;
    // 需要落盘：新建 / rollover / bump，或 camp 尚无 epoch
    let should_persist =
        should_bump || result.created || result.rolled_over || camp.context_epoch.is_none();
    (snap, result.membership, should_persist, rev)
}

/// 从指定 CampaignStore 的活跃 Campaign 组装 CampaignRuntimeContext 快照写入 ctx + tool_ctx。
///
/// 抽自 `fill_campaign_context`，供外部 harness 用独立 tempdir `CampaignStore` 复刻真实
/// campaign-mode 组装逻辑（无需 AppState / 全局 store / Tauri runtime）。行为与线上路径一致。
///
/// 调用前应已清空 `ctx.campaign_runtime` 与 `tool_ctx.campaign_runtime`（防 stale）。
/// 若 store 中找不到该 campaign，直接返回（无 campaign 模式）。
pub fn fill_campaign_runtime_from_store(
    ctx: &mut WritingContext,
    tool_ctx: &Arc<RwLock<ToolContext>>,
    store: &campaign_store::CampaignStore,
    active_id: &Id,
) {
    let mut camp = match store.get_campaign(active_id) {
        Some(c) => c,
        None => return,
    };
    let lineage = ensure_campaign_lineage_persisted(store, &mut camp);
    ctx.campaign_id = Some(active_id.clone());
    ctx.story_clock = camp.current_story_clock().to_string();
    // turn = max(A.turn)+1；B/C 不计入
    let mut all_summaries = store.list_summaries(active_id);
    backfill_summary_lineage_if_needed(store, &lineage, &mut all_summaries);
    ctx.turn = next_writing_turn(&all_summaries);
    // pending_tasks：该 Campaign 下所有任务（build_director_user_msg 内部按触发条件过滤）
    ctx.pending_tasks = store.list_tasks(active_id);

    // 阶段 2：组装 CampaignRuntimeContext 快照
    // 加载 instances、card definitions、knowledge、tasks，构建纯 domain 快照
    let instances = store.list_instances(active_id);
    let knowledge = store.list_knowledge(active_id);
    let tasks = store.list_tasks(active_id);

    let stored_card = store.get_card(&camp.card_id);
    if let Some(stored_card) = &stored_card {
        append_missing_campaign_scoped_regex_scripts(ctx, stored_card.card.scoped_regex_scripts());
    }

    // 从 card 的 character_definitions 构建 definitions_by_id
    let definitions_by_id: std::collections::HashMap<
        Id,
        storyforge_domain::character::CharacterDefinition,
    > = if let Some(stored_card) = stored_card {
        stored_card
            .card
            .character_definitions
            .into_iter()
            .map(|def| (def.id.clone(), def))
            .collect()
    } else {
        std::collections::HashMap::new()
    };

    // M2 完整：编译入口刷新 epoch 快照（满 E 则 rollover）并落盘（锁内重读）
    let (epoch_snap, _membership) =
        refresh_and_persist_context_epoch(store, &mut camp, &all_summaries);
    ctx.context_epoch = Some(epoch_snap);
    ctx.chronicle_revision = camp.chronicle_revision;
    // 刷新后重读 summaries（压缩可能已在锁窗口内发布）
    let all_summaries = store.list_summaries(active_id);
    ctx.turn = next_writing_turn(&all_summaries);

    let runtime = Arc::new(CampaignRuntimeContext {
        campaign: camp,
        instances,
        definitions_by_id,
        knowledge,
        tasks,
        turn: ctx.turn,
    });

    // 写入 WritingContext
    ctx.campaign_runtime = Some(runtime.clone());
    // ContextCompiler：RoundSummary → Director history 前缀 + tools
    // load-side 只保留最近 K 条（turn 已在上方用全量条数计算）
    // 全量目录：get_chronicle 按 code 点名旧 A 不受 256 截断；search 仍有 limit/预算
    let chronicle_tool_catalog = all_summaries.clone();
    ctx.chronicle_prompt_catalog =
        build_chronicle_prompt_catalog(&all_summaries, ctx.context_epoch.as_ref());
    ctx.recent_summaries =
        take_recent_summaries_for_context(all_summaries, RECENT_SUMMARIES_LOAD_LIMIT);

    // 同步到 ToolContext（快照，非 store 引用）
    {
        let mut tool_guard = tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        tool_guard.campaign_runtime = Some(runtime);
        tool_guard.archived_summaries = ctx
            .recent_summaries
            .iter()
            .map(|s| s.content.clone())
            .collect();
        tool_guard.chronicle_summaries = chronicle_tool_catalog;
        tool_guard.reset_chronicle_tool_budget();
    }
}

pub(crate) fn append_missing_campaign_scoped_regex_scripts(
    ctx: &mut WritingContext,
    scripts: Vec<RegexScript>,
) {
    let mut existing_scoped_ids: std::collections::HashSet<String> = ctx
        .regex_scripts
        .iter()
        .filter(|script| script.source == RegexScriptSource::Scoped)
        .map(|script| script.id.clone())
        .collect();

    for script in scripts {
        if existing_scoped_ids.insert(script.id.clone()) {
            ctx.regex_scripts.push(script);
        }
    }
}

pub(crate) fn postprocess_variable_type_name(
    value_type: &storyforge_domain::variables::VariableType,
) -> &'static str {
    use storyforge_domain::variables::VariableType;
    match value_type {
        VariableType::Int => "int",
        VariableType::Float => "float",
        VariableType::String => "string",
        VariableType::Bool => "bool",
        VariableType::Json => "json",
    }
}

pub(crate) fn postprocess_json_value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(number) if number.is_i64() => "int",
        serde_json::Value::Number(_) => "float",
        serde_json::Value::String(_) => "string",
        _ => "json",
    }
}

pub(crate) fn postprocess_variable_hint(
    key: &str,
    scope: &str,
    label: &str,
    value_type: &str,
) -> String {
    format!("{key}（{scope}/{label}/{value_type}）")
}

pub(crate) fn insert_postprocess_schema_hint(
    catalog: &mut std::collections::BTreeMap<String, String>,
    scope_key: &str,
    scope_label: &str,
    field: &storyforge_domain::variables::VariableField,
) {
    catalog.insert(
        format!("{scope_key}:{}", field.key),
        postprocess_variable_hint(
            &field.key,
            scope_label,
            &field.label,
            postprocess_variable_type_name(&field.value_type),
        ),
    );
}

/// 后处理可更新变量键。
///
/// 基础表提供常用角色/全局变量；CampaignRuntimeContext 提供当前卡自定义 schema、
/// 已存在 Campaign 变量和 instance 变量，覆盖 MVU/initvar 与高玩自定义字段。
pub(crate) fn postprocess_variable_keys(ctx: &WritingContext) -> Vec<String> {
    let mut catalog = std::collections::BTreeMap::<String, String>::new();

    for field in storyforge_domain::variables::default_character_variables() {
        insert_postprocess_schema_hint(&mut catalog, "character", "角色", &field);
    }
    for field in storyforge_domain::variables::default_campaign_variables() {
        insert_postprocess_schema_hint(&mut catalog, "campaign", "全局", &field);
    }

    let Some(runtime) = &ctx.campaign_runtime else {
        return catalog.into_values().collect();
    };

    for field in &runtime.campaign.variable_schema {
        insert_postprocess_schema_hint(&mut catalog, "campaign", "全局", field);
    }
    for variable in &runtime.campaign.variables {
        catalog
            .entry(format!("campaign:{}", variable.key))
            .or_insert_with(|| {
                postprocess_variable_hint(
                    &variable.key,
                    "全局",
                    &variable.key,
                    postprocess_json_value_type_name(&variable.value),
                )
            });
    }

    let mut definitions: Vec<_> = runtime.definitions_by_id.values().collect();
    definitions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    for definition in definitions {
        for field in &definition.variable_schema {
            insert_postprocess_schema_hint(&mut catalog, "character", "角色", field);
        }
    }

    for instance in &runtime.instances {
        for variable in &instance.variables {
            catalog
                .entry(format!("character:{}", variable.key))
                .or_insert_with(|| {
                    postprocess_variable_hint(
                        &variable.key,
                        "角色",
                        &variable.key,
                        postprocess_json_value_type_name(&variable.value),
                    )
                });
        }
    }

    catalog.into_values().collect()
}

#[derive(Debug, Clone)]
#[cfg(test)]
pub(crate) struct TemporaryInstancesPersistContext {
    campaign_id: Id,
}

#[cfg(test)]
impl TemporaryInstancesPersistContext {
    pub(crate) fn from_writing_context(ctx: &WritingContext) -> Option<Self> {
        Some(Self {
            campaign_id: ctx.campaign_id.clone()?,
        })
    }
}
#[cfg(test)]
pub(crate) fn persist_temporary_instances_to(
    store: &campaign_store::CampaignStore,
    ctx: &WritingContext,
    temporaries: &[storyforge_domain::campaign::CharacterInstance],
) {
    let Some(persist_ctx) = TemporaryInstancesPersistContext::from_writing_context(ctx) else {
        return;
    };
    persist_temporary_instances_to_store(store, &persist_ctx, temporaries);
}

#[cfg(test)]
pub(crate) fn persist_temporary_instances_to_store(
    store: &campaign_store::CampaignStore,
    persist_ctx: &TemporaryInstancesPersistContext,
    temporaries: &[storyforge_domain::campaign::CharacterInstance],
) {
    if temporaries.is_empty() {
        return;
    }
    let camp_id = &persist_ctx.campaign_id;
    let existing = store.list_instances(camp_id);
    let mut known_names: std::collections::HashSet<String> =
        existing.into_iter().map(|i| i.name).collect();

    let mut persisted_count = 0;
    for temp in temporaries {
        if temp.campaign_id != *camp_id {
            tracing::warn!(
                "跳过 campaign 不匹配的临时 instance '{}'（instance campaign: {}, current campaign: {}）",
                temp.name,
                temp.campaign_id,
                camp_id
            );
            continue;
        }
        if !known_names.insert(temp.name.clone()) {
            tracing::info!(
                "跳过已存在的同名临时 instance '{}'（campaign {}）",
                temp.name,
                camp_id
            );
            continue;
        }
        if let Err(e) = store.add_instance(temp.clone()) {
            tracing::error!("落盘临时 instance '{}' 失败: {e}", temp.name);
        } else {
            persisted_count += 1;
        }
    }
    if persisted_count > 0 {
        tracing::info!(
            "已落盘 {} 个临时 instance 到 campaign {}",
            persisted_count,
            camp_id
        );
    }
}

/// W10: 为在场角色收集 MVU fallback 片段（供 run_postprocess 执行 JS）
///
/// 查找链：present_chars → CampaignRuntimeContext.instances → definition_id →
/// CampaignStore.cards → source_character_id → MvuTranslation.fallback_fragments
pub(crate) fn collect_mvu_fallback_fragments(
    ctx: &WritingContext,
    store: &campaign_store::CampaignStore,
    present_chars: &[String],
) -> Vec<storyforge_domain::mvu_translation::FallbackFragment> {
    let runtime = match &ctx.campaign_runtime {
        Some(rt) => rt,
        None => return vec![],
    };

    // definition_id → source_character_id（从 CampaignStore cards 构建查找表）
    let def_to_source: std::collections::HashMap<Id, Id> = store
        .list_cards()
        .iter()
        .flat_map(|sc| {
            let src = sc.card.source_character_id.clone();
            sc.card
                .character_definitions
                .iter()
                .map(move |d| (d.id.clone(), src.clone()))
        })
        .collect();

    let mut fragments = Vec::new();
    for char_id_str in present_chars {
        let char_id = Id::from_str(char_id_str);
        // 在 present_characters 中匹配 instance（by id or name）
        let inst = runtime
            .instances
            .iter()
            .find(|i| i.id == char_id || i.name == *char_id_str);
        let inst = match inst {
            Some(i) => i,
            None => continue,
        };
        let def_id = match &inst.definition_id {
            Some(d) => d,
            None => continue,
        };
        let source_id = match def_to_source.get(def_id) {
            Some(s) => s,
            None => continue,
        };
        if let Some(stored) = store.get_mvu(source_id) {
            let non_empty: Vec<_> = stored
                .translation
                .fallback_fragments
                .into_iter()
                .filter(|f| !f.js_snippet.is_empty())
                .collect();
            if !non_empty.is_empty() {
                tracing::info!(
                    target: "tauri-app",
                    "[MVU] 角色 '{}' 有 {} 个 fallback 片段",
                    inst.name,
                    non_empty.len()
                );
                fragments.extend(non_empty);
            }
        }
    }
    fragments
}

/// CampaignStore.cards → source_character_id → MvuTranslation.update_rules
///
/// 与 `collect_mvu_fallback_fragments` 同型的查找链，但按 source 卡去重：
/// 同一张卡的多个在场实例只贡献一次规则（规则是卡级玩法，不随实例数翻倍）。
pub(crate) fn collect_mvu_update_rules(
    ctx: &WritingContext,
    store: &campaign_store::CampaignStore,
    present_chars: &[String],
) -> Vec<String> {
    let runtime = match &ctx.campaign_runtime {
        Some(rt) => rt,
        None => return vec![],
    };

    let def_to_source: std::collections::HashMap<Id, Id> = store
        .list_cards()
        .iter()
        .flat_map(|sc| {
            let src = sc.card.source_character_id.clone();
            sc.card
                .character_definitions
                .iter()
                .map(move |d| (d.id.clone(), src.clone()))
        })
        .collect();

    let mut visited_sources = std::collections::HashSet::new();
    let mut rules = Vec::new();
    for char_id_str in present_chars {
        let char_id = Id::from_str(char_id_str);
        let inst = runtime
            .instances
            .iter()
            .find(|i| i.id == char_id || i.name == *char_id_str);
        let inst = match inst {
            Some(i) => i,
            None => continue,
        };
        let def_id = match &inst.definition_id {
            Some(d) => d,
            None => continue,
        };
        let source_id = match def_to_source.get(def_id) {
            Some(s) => s,
            None => continue,
        };
        if !visited_sources.insert(source_id.clone()) {
            continue;
        }
        if let Some(stored) = store.get_mvu(source_id) {
            let non_empty: Vec<String> = stored
                .translation
                .update_rules
                .into_iter()
                .filter(|r| !r.trim().is_empty())
                .collect();
            if !non_empty.is_empty() {
                tracing::info!(
                    target: "tauri-app",
                    "[MVU] 角色 '{}' 所属卡贡献 {} 条变量更新规则",
                    inst.name,
                    non_empty.len()
                );
                rules.extend(non_empty);
            }
        }
    }
    rules
}

#[derive(Debug, Clone)]
pub(crate) struct PostprocessPersistContext {
    pub(crate) campaign_id: Id,
    pub(crate) conversation_id: Id,
    pub(crate) turn: u32,
}

impl PostprocessPersistContext {
    pub(crate) fn from_writing_context(ctx: &WritingContext) -> Option<Self> {
        Some(Self {
            campaign_id: ctx.campaign_id.clone()?,
            conversation_id: ctx.conversation_id.clone(),
            turn: ctx.turn,
        })
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_mutation_batch(
    store: &campaign_store::CampaignStore,
    persist_ctx: &PostprocessPersistContext,
    outcome: &storyforge_app_agent::PostProcessOutcome,
    present_chars: &[String],
) -> storyforge_domain::turn::MutationBatch {
    production_postprocess::build_json_mutation_batch(
        store,
        &production_postprocess::PostprocessPersistContext {
            campaign_id: persist_ctx.campaign_id.clone(),
            conversation_id: persist_ctx.conversation_id.clone(),
            turn: persist_ctx.turn,
        },
        outcome,
        present_chars,
        &[],
    )
}
pub(crate) fn persist_postprocess_outcome_to_store(
    store: &campaign_store::CampaignStore,
    persist_ctx: &PostprocessPersistContext,
    outcome: &storyforge_app_agent::PostProcessOutcome,
    present_chars: &[String],
) {
    let camp_id = &persist_ctx.campaign_id;

    // 本轮摘要
    if let Some(summary) = &outcome.summary
        && let Err(e) = store.add_summary(storyforge_domain::agent::RoundSummary::new(
            camp_id.clone(),
            persist_ctx.conversation_id.clone(),
            persist_ctx.turn,
            summary.clone(),
        ))
    {
        tracing::warn!("保存本轮摘要失败: {e}");
    }

    // 后处理三合一
    if let Some(pp) = &outcome.post_process {
        // 构建 present_chars 的 Id 集合（用于校验写入目标）
        let present_ids: std::collections::HashSet<String> =
            present_chars.iter().cloned().collect();

        // P4：算 name_collisions——campaign 内出现 ≥2 次的 name 集合，同名时 name 路失效逼 id
        let name_collisions: std::collections::HashSet<String> = {
            let mut name_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for inst in store.list_instances(camp_id) {
                *name_counts.entry(inst.name).or_insert(0) += 1;
            }
            name_counts
                .into_iter()
                .filter(|(_, count)| *count >= 2)
                .map(|(name, _)| name)
                .collect()
        };

        // 知识：update → entry（assign campaign_id + turn）
        // 只写入 present_chars 中的角色知识（信息隔离：不出场角色不应被后处理写入知识）
        // W6：broadcast 非空时一条 update 可能分发为多条 entry（flat_map）
        let knowledge_entries: Vec<_> = pp
            .knowledge_updates
            .iter()
            .flat_map(|u| {
                // P3: normalize 内部按 source 分流（ToldByOther/Backstory 不查在场）
                // P4: name_collisions 同名时 name 路失效
                // W6: broadcast 时分发到多个 target
                normalize_knowledge_update_for_postprocess(
                    store,
                    camp_id,
                    u,
                    persist_ctx.turn,
                    &present_ids,
                    &name_collisions,
                )
            })
            .collect();
        if !knowledge_entries.is_empty()
            && let Err(e) = store.add_knowledge(knowledge_entries)
        {
            tracing::warn!("保存后处理知识失败: {e}");
        }

        // 变量更新：角色级（按 name 匹配 instance）/ 全局级（无 instance_id）
        for vu in &pp.variable_updates {
            if let Some(inst_id) = &vu.instance_id {
                // instance_id 可能是角色名（后处理 Agent 按名字输出），尝试匹配 campaign 内 instance
                if let Some(inst) = find_instance_by_name_or_id(store, camp_id, inst_id) {
                    // 校验：该 instance 是否在 present_chars 中（P4: 同名时 name 路失效）
                    let is_present = is_postprocess_instance_present(
                        &inst,
                        inst_id,
                        &present_ids,
                        &name_collisions,
                    );
                    if is_present {
                        let mut inst = inst;
                        inst.set_variable(&vu.key, vu.value.clone(), persist_ctx.turn);
                        if let Err(e) = store.update_instance(inst) {
                            tracing::warn!("保存后处理角色变量失败: {e}");
                        }
                    } else {
                        tracing::warn!(
                            "跳过非在场角色 '{}' 的变量写入（present_chars 校验）",
                            inst.name
                        );
                    }
                }
            } else {
                // 全局 Campaign 变量（无 instance_id，不受 present_chars 约束）
                if let Some(mut camp) = store.get_campaign(camp_id) {
                    camp.set_variable(&vu.key, vu.value.clone(), persist_ctx.turn);
                    if let Err(e) = store.update_campaign(camp) {
                        tracing::warn!("保存后处理 Campaign 变量失败: {e}");
                    }
                }
            }
        }

        // 任务更新：新建 / 状态变化
        for tu in &pp.task_updates {
            if let Some(tid) = &tu.task_id {
                if let Some(task) = store.get_task(tid)
                    && let Some(task) =
                        normalize_task_update_for_postprocess(camp_id, task, tu.new_status.clone())
                    && let Err(e) = store.update_task(task)
                {
                    tracing::warn!("保存后处理任务状态失败: {e}");
                }
            } else if let Some(spec) = &tu.new_task {
                let new_task = storyforge_domain::story_task::StoryTask::from_narrative(
                    camp_id.clone(),
                    spec.title.clone(),
                    spec.description.clone(),
                    spec.triggers.clone(),
                    persist_ctx.turn,
                );
                if let Err(e) = store.add_task(new_task) {
                    tracing::warn!("保存后处理新任务失败: {e}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::production_postprocess::{ProductionPostprocessError, ProductionPostprocessResult};
    use storyforge_domain::turn::{DerivationComponents, DerivationStatus};

    fn applied_result() -> ProductionPostprocessResult {
        use storyforge_app_agent::PostProcessOutcome;
        use storyforge_domain::agent::PostProcessResult;
        use storyforge_domain::character_knowledge::{
            CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
        };
        let outcome = PostProcessOutcome {
            summary: None,
            summary_attempted: false,
            post_process_attempted: true,
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![CharacterKnowledgeUpdate {
                    character_id: Id::from_str("alice"),
                    knowledge_text: "酒窖暗门".to_string(),
                    source: KnowledgeSource::Witnessed,
                    source_character_id: None,
                    pinned: false,
                    broadcast: None,
                    propagation: PropagationPolicy::Open,
                }],
                variable_updates: vec![storyforge_domain::agent::VariableUpdate {
                    instance_id: None,
                    key: "hp".to_string(),
                    value: serde_json::json!(99),
                }],
                task_updates: vec![],
                parse_succeeded: true,
            }),
        };
        ProductionPostprocessResult {
            applied: true,
            skipped_reason: None,
            derivation: DerivationComponents {
                summary_derivation: DerivationStatus::Succeeded,
                state_derivation: DerivationStatus::Succeeded,
            },
            summary_text: None,
            batch: None,
            outcome: Some(outcome),
        }
    }

    fn skipped_result(reason: &str) -> ProductionPostprocessResult {
        ProductionPostprocessResult {
            applied: false,
            skipped_reason: Some(reason.to_string()),
            derivation: DerivationComponents {
                summary_derivation: DerivationStatus::Disabled,
                state_derivation: DerivationStatus::Disabled,
            },
            summary_text: None,
            batch: None,
            outcome: None,
        }
    }

    fn expect_done(event: &Option<PipelineEvent>, k: usize, v: usize, t: usize) {
        match event {
            Some(PipelineEvent::PostProcessDone {
                knowledge_count,
                variable_count,
                task_count,
            }) => {
                assert_eq!((*knowledge_count, *variable_count, *task_count), (k, v, t));
            }
            other => panic!("expected PostProcessDone, got {other:?}"),
        }
    }

    fn expect_skipped(event: &Option<PipelineEvent>) {
        match event {
            Some(PipelineEvent::PostProcessSkipped { reason }) => {
                assert_eq!(reason, "postprocess cancelled");
            }
            other => panic!("expected PostProcessSkipped, got {other:?}"),
        }
    }

    fn expect_failed(event: &Option<PipelineEvent>, reason: &str) {
        match event {
            Some(PipelineEvent::PostProcessFailed { reason: got }) => {
                assert_eq!(got, reason);
            }
            other => panic!("expected PostProcessFailed, got {other:?}"),
        }
    }

    // ─── 三.3：四条路径各恰好一个正确事件，全部经真实 helper ────────────

    #[test]
    fn normal_path_derives_exactly_one_done_event() {
        let event = postprocess_pipeline_event(Some(&Ok(applied_result())), "", false);
        expect_done(&event, 1, 1, 0);
        // 恰好一个：再调用一次仍是一个（结果 → 事件是确定函数）。
        expect_done(
            &postprocess_pipeline_event(Some(&Ok(applied_result())), "", false),
            1,
            1,
            0,
        );
    }

    #[test]
    fn cancel_before_runner_derives_skipped_event() {
        // runner 前取消：run_shared_postprocess_background 的 early-return 分支
        // 用 cancelled=true 调用同一 helper——同一个事件。
        let event = postprocess_pipeline_event(None, "", true);
        expect_skipped(&event);
    }

    #[test]
    fn cancel_after_runner_derives_skipped_event() {
        // runner 后取消：同样 cancelled=true → PostProcessSkipped（不是 Failed）。
        let event = postprocess_pipeline_event(None, "", true);
        expect_skipped(&event);
        // apply_outcome 内部取消检查产生的 skipped_reason="cancelled" 也走同一事件。
        expect_skipped(&postprocess_pipeline_event(
            Some(&Ok(skipped_result("cancelled"))),
            "",
            false,
        ));
    }

    #[test]
    fn persist_failure_derives_failed_event() {
        let err = ProductionPostprocessError::Storage("turns.json 写入失败".into());
        let event = postprocess_pipeline_event(
            Some(&Err(err)),
            "postprocess storage failed: turns.json 写入失败",
            false,
        );
        expect_failed(&event, "postprocess storage failed: turns.json 写入失败");
    }

    #[test]
    fn other_skip_reasons_derive_no_event() {
        let event = postprocess_pipeline_event(
            Some(&Ok(skipped_result("late_or_superseded_attempt"))),
            "",
            false,
        );
        assert!(event.is_none(), "非取消跳过不产生事件: {event:?}");
    }

    // ─── 三.4：backend-neutral 角色解析器（stored/source/card/name 经 facade）─

    fn resolver_fixture(
        dir: &std::path::Path,
    ) -> (
        Arc<storage_backend::StorageFacade>,
        storage::StoredCharacter,
    ) {
        std::fs::create_dir_all(dir).unwrap();
        for name in [
            "campaigns.json",
            "instances.json",
            "knowledge.json",
            "tasks.json",
            "round_summaries.json",
            "turns.json",
            "mvu_translations.json",
            "compress_jobs.json",
            "characters.json",
        ] {
            std::fs::write(
                dir.join(name),
                serde_json::to_vec_pretty(&serde_json::json!([])).unwrap(),
            )
            .unwrap();
        }
        // 卡片：source_character_id = src-1（card id 键解析目标）。
        std::fs::write(
            dir.join("cards.json"),
            serde_json::to_vec_pretty(&serde_json::json!([{
                "card": {
                    "id": "resolver-card-1",
                    "name": "Resolver 卡",
                    "source_character_id": "src-1",
                    "character_definitions": [],
                    "campaign_variable_schema": [],
                    "raw_card_json": {},
                    "extraction_status": "extracted",
                    "extraction_message": null
                },
                "imported_at": "2026-07-01T00:00:00Z"
            }]))
            .unwrap(),
        )
        .expect("write resolver cards.json");
        let storage = Arc::new(storage_backend::StorageFacade::new(
            dir.to_path_buf(),
            storyforge_infra_sqlite::backend::PinnedBackend::new(
                storyforge_infra_sqlite::backend::StorageBackend::Json,
                storyforge_infra_sqlite::backend::BackendSource::Default,
            ),
        ));
        // 带 scoped regex 脚本的角色（extensions.regex_scripts，ST 形状）。
        let stored = storage
            .save_character(crate::commands::characters::CharacterInfo {
                source_character_id: Some("src-1".to_string()),
                name: "Alice".to_string(),
                description: "主角".to_string(),
                personality: "勇敢".to_string(),
                scenario: "地下城".to_string(),
                first_mes: "你好".to_string(),
                mes_example: String::new(),
                post_history_instructions: String::new(),
                alternate_greetings: vec![],
                system_prompt: "扮演".to_string(),
                tags: vec![],
                creator: "test".to_string(),
                character_version: "1.0".to_string(),
                spec_version: "2.0".to_string(),
                extensions: serde_json::json!({
                    "regex_scripts": [{
                        "scriptName": "酒窖别名",
                        "findRegex": "酒窖|地窖",
                        "replaceString": "密窖",
                        "placement": [2]
                    }]
                }),
                embedded_world_info: None,
                renderable_assets: None,
                raw_card_json: serde_json::json!({}),
                has_world_info: false,
                has_renderable_assets: false,
                world_info_count: 0,
                world_info_entries: vec![],
            })
            .expect("save resolver character");
        (storage, stored)
    }

    #[test]
    fn scoped_regex_resolver_maps_stored_source_card_and_name_through_facade() {
        let dir = std::env::temp_dir().join(format!("sf-resolver-{}", uuid::Uuid::new_v4()));
        let (storage, stored) = resolver_fixture(&dir);
        let characters: Vec<std::sync::Arc<storyforge_domain::character::Character>> = storage
            .list_characters()
            .expect("list characters")
            .iter()
            .map(|c| std::sync::Arc::new(crate::stored_info_to_character(c)))
            .collect();
        assert_eq!(characters.len(), 1);
        assert_eq!(characters[0].id.as_str(), "src-1");

        let resolve = |key: &str| {
            collect_scoped_regex_scripts_for_backend(Some(key), &characters, Some(storage.as_ref()))
                .expect("resolver")
        };

        // stored id 键。
        let by_stored = resolve(&stored.id);
        assert_eq!(by_stored.len(), 1, "stored id 必须解析出 scoped 脚本");
        assert_eq!(by_stored[0].script_name, "酒窖别名");
        // source id 键。
        assert_eq!(resolve("src-1").len(), 1, "source id 必须解析");
        // name 键。
        assert_eq!(resolve("Alice").len(), 1, "name 必须解析");
        // card id 键（card.source_character_id → Character.id）。
        assert_eq!(resolve("resolver-card-1").len(), 1, "card id 必须解析");
        // 缺失 id → 空（不 panic、不误匹配）。
        assert!(resolve("不存在").is_empty());
        // None → 空。
        assert!(
            collect_scoped_regex_scripts_for_backend(None, &characters, Some(storage.as_ref()))
                .expect("none key")
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── 三.3 端到端 ─────────────────────────────────────────────────────
    // runner 前取消：`run_shared_postprocess_background` 必须向 channel 发出
    // **恰好一个** PostProcessSkipped（旧实现不发任何事件）。
    #[tokio::test]
    async fn early_cancel_emits_single_skipped_event() {
        let dir = std::env::temp_dir().join(format!("sf-pp-event-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = Arc::new(storage_backend::StorageFacade::new(
            dir.clone(),
            storyforge_infra_sqlite::backend::PinnedBackend::new(
                storyforge_infra_sqlite::backend::StorageBackend::Json,
                storyforge_infra_sqlite::backend::BackendSource::Default,
            ),
        ));
        let conv_store = Arc::new(storyforge_app_conversation::ConversationStore::new(
            dir.join("conversations"),
        ));
        let tool_ctx = Arc::new(storyforge_app_agent::ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        // 永不调用的 stub LLM（early cancel 在 run_postprocess 之前返回）。
        struct NeverCalledLlm;
        #[async_trait::async_trait]
        impl storyforge_infra_llm::LlmClient for NeverCalledLlm {
            async fn chat(
                &self,
                _req: &storyforge_domain::llm::ChatRequest,
            ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError>
            {
                panic!("early cancel path must not call the LLM")
            }
            async fn chat_stream(
                &self,
                _req: &storyforge_domain::llm::ChatRequest,
                _tx: tokio::sync::mpsc::UnboundedSender<storyforge_domain::llm::StreamChunk>,
                _cancel: tokio::sync::watch::Receiver<bool>,
            ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError>
            {
                panic!("early cancel path must not call the LLM")
            }
        }
        let pipeline = storyforge_app_pipeline::PipelineOrchestrator::new(
            Arc::new(NeverCalledLlm),
            conv_store,
            tool_ctx,
            None,
        );
        let writing_ctx = storyforge_app_pipeline::WritingContext {
            characters: vec![],
            world_info: None,
            conversation_id: Id::from_str("conv-pp-event"),
            campaign_id: None,
            turn: 0,
            pending_tasks: vec![],
            story_clock: String::new(),
            profile: None,
            modules: vec![],
            regex_scripts: vec![],
            campaign_runtime: None,
            agent_profile_config: None,
            recent_summaries: vec![],
            chronicle_prompt_catalog: vec![],
            far_memory_hits: vec![],
            template_random_seed: None,
            context_epoch: None,
            chronicle_revision: 0,
        };
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        cancel_tx.send(true).expect("pre-cancel");

        let result = run_shared_postprocess_background(
            storage,
            pipeline,
            writing_ctx,
            "正文".to_string(),
            vec![],
            vec![],
            vec![],
            vec![],
            event_tx,
            cancel_rx,
            None,
            None,
        )
        .await;
        assert!(
            !result.expect("early cancel is Ok(false)"),
            "cancelled → not applied"
        );

        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }
        assert_eq!(
            events.len(),
            1,
            "early cancel 必须恰好一个事件，got {events:?}"
        );
        match &events[0] {
            PipelineEvent::PostProcessSkipped { reason } => {
                assert_eq!(reason, "postprocess cancelled");
            }
            other => panic!("expected PostProcessSkipped, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

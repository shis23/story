use std::sync::Arc;

use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::{RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;

use crate::services::writing_regenerate::{
    RegenerateRequestDto, parse_target_dto, validate_regenerate_campaign_scope,
};
use crate::error::TauriCommandError;
use crate::last_user_intent_before;
use crate::production_postprocess;
use crate::turn_lifecycle;
use crate::{
    AppState, BackendTurnAttemptSink, append_missing_campaign_scoped_regex_scripts,
    begin_writing_operation, check_turn_barrier, clear_current_cancel_if,
    collect_mvu_fallback_fragments_for_backend, collect_mvu_update_rules_for_backend,
    collect_scoped_regex_scripts_for_backend, fill_agent_profile_context,
    fill_campaign_context_async, fill_far_memory_hits, fill_profile_context, fill_regex_context,
    get_active_turn_for_backend, get_global_regex_store, get_preset_store,
    postprocess_variable_keys, prepare_start_conversation_async, run_shared_postprocess_background,
    service_fail_turn, update_turn_record,
};

pub use storyforge_app_service::services::writing::*;

// ─── M1 写作命令 ───────────────────────────────────────────────────────────

/// Regenerate command entrypoint. The orchestration implementation remains
/// temporarily in the root module while the final Gate 1 helper extraction is
/// completed; the IPC command itself belongs to this domain module.
#[tauri::command]
pub(crate) async fn regenerate(
    req: crate::RegenerateRequestDto,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<WritingEvent>,
) -> Result<String, TauriCommandError> {
    regenerate_impl(req, state, on_event).await
}

pub(crate) async fn regenerate_impl(
    req: RegenerateRequestDto,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<WritingEvent>,
) -> Result<String, TauriCommandError> {
    let app = state.inner().clone();
    app.require_active_llm()?;

    // 解析 targets
    let targets: Vec<PartialRollTarget> = req
        .targets
        .iter()
        .map(parse_target_dto)
        .collect::<Result<_, _>>()?;

    let conversation_id = Id::from_str(&req.conversation_id);
    let node_id = Id::from_str(&req.node_id);
    let recall_hint = req.hint.clone();

    // Scope before constructing/running the pipeline. A cross-campaign
    // request used to mutate the requested conversation first and only then
    // attach an Attempt to the currently selected Campaign.
    let conversation = app.conv_store.get(&conversation_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("conversation {conversation_id} was not found"))
    })?;
    let active_campaign = app
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let active_turn = active_campaign
        .as_ref()
        .map(|campaign_id| get_active_turn_for_backend(app.storage(), campaign_id))
        .transpose()
        .map_err(TauriCommandError::internal)?
        .flatten();
    validate_regenerate_campaign_scope(
        active_campaign.as_ref(),
        &conversation,
        active_turn.as_ref(),
    )?;

    let pipeline_req = RegenerateRequest {
        conversation_id: conversation_id.clone(),
        node_id: node_id.clone(),
        targets,
        generation_mode: req.generation_mode,
        hint: req.hint,
        seed: req.seed,
    };

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let writing_event = WritingEvent::from_pipeline_event(&event);
            let _ = on_event_clone.send(writing_event);
        }
    });

    // tool_ctx 快照（保持与 start_writing 一致）
    let tool_snapshot = app.snapshot_tool_ctx();
    let regex_character_id = conversation.character_id.clone();
    let mut ctx = WritingContext {
        characters: tool_snapshot.characters.clone(),
        world_info: tool_snapshot.world_info.clone(),
        conversation_id: conversation_id.clone(),
        campaign_id: None,
        turn: 0,
        pending_tasks: vec![],
        story_clock: String::new(),
        profile: None,
        modules: vec![],
        regex_scripts: collect_scoped_regex_scripts_for_backend(
            regex_character_id.as_deref(),
            &tool_snapshot.characters,
            Some(app.storage()),
        )
        .map_err(TauriCommandError::storage)?,
        campaign_runtime: None,
        agent_profile_config: None,
        recent_summaries: vec![],
        chronicle_prompt_catalog: vec![],
        far_memory_hits: vec![],
        // A2：regenerate 用户 seed 直接注入模板 random/roll
        template_random_seed: req.seed,
        context_epoch: None,
        chronicle_revision: 0,
    };
    fill_regex_context(&mut ctx, get_preset_store(), get_global_regex_store());
    fill_profile_context(&mut ctx, &app);
    fill_agent_profile_context(&mut ctx, &app);
    fill_campaign_context_async(&mut ctx, &app).await?;
    // regenerate：hint 优先；无 hint 时回退到该 AI 节点之前最近一条 user 意图
    let fallback_intent = if recall_hint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        last_user_intent_before(&app.conv_store, &conversation_id, &node_id)
    } else {
        None
    };
    let far_query = recall_hint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or(fallback_intent);
    if let Some(query) = far_query.as_deref() {
        fill_far_memory_hits(&mut ctx, &app, query).await;
    }

    // Operation-owned cancel for regenerate.
    let (operation_id, cancel_rx) = begin_writing_operation(&app);

    let prompt_hook = frontend_prompt_hook(event_tx.clone(), app.prompt_hook_pending.clone());
    let mut pipeline =
        app.new_pipeline_with_regex_and_prompt_hook(&ctx.regex_scripts, Some(prompt_hook))?;
    let result = pipeline
        .regenerate(
            pipeline_req.clone(),
            &ctx,
            event_tx.clone(),
            cancel_rx.clone(),
        )
        .await;

    // ─── P2 后处理（best-effort，同 start_writing）─────────────────────────
    // auto-fix 后命令返回值必须是修复稿，且 Attempt.draft_hash 必须同步。
    let mut response_text: Option<String> = None;
    if let Ok((text, provenance)) = &result {
        // Phase A: regenerate 创建新 TurnAttempt,旧 Attempt Superseded
        // regenerate 的 replace_active_variant 改变了 node_id 的 active variant,
        // 新 variant 在同一 node 上,用 req 的 node_id 作为 variant_id
        // SQLite: atomic preaccept UoW owns conversation + attempt land.
        let regen_attempt_id = if let Some(campaign_id) = &ctx.campaign_id {
            if let Some(turn) = get_active_turn_for_backend(app.storage(), campaign_id)
                .map_err(TauriCommandError::internal)?
            {
                let new_attempt_id = Id::new();
                match app.turn_workflow.append_regenerate_attempt(
                    crate::backend_workflows::RegenerateAttemptRequest {
                        campaign_id,
                        conversation_id: &conversation_id,
                        turn_id: &turn.turn_id,
                        previous_variant_id: &node_id,
                        attempt_id: &new_attempt_id,
                        draft_text: text,
                        pending_temporary_instances: pipeline
                            .pending_temporary_instances()
                            .to_vec(),
                        provenance: Some(provenance.clone()),
                    },
                ) {
                    Ok(outcome) => Some(outcome.attempt_id),
                    Err(e) => {
                        clear_current_cancel_if(&app, &operation_id);
                        return Err(TauriCommandError::internal(e));
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let (final_text, present_chars, var_keys) = (
            text.clone(),
            pipeline
                .session()
                .and_then(|s| s.plan.as_ref())
                .map(|p| {
                    p.subagent_tasks
                        .iter()
                        .map(|t| t.character_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            postprocess_variable_keys(&ctx),
        );
        // Operation-owned cancel clones only (no global re-subscribe / no false fallback).
        let pp_rx = cancel_rx.clone();
        // W10: 收集在场角色的 MVU fallback 片段（JS 执行用）+ 变量更新规则（注入后处理提示词）
        let mvu_fragments =
            collect_mvu_fallback_fragments_for_backend(app.storage(), &ctx, &present_chars)
                .map_err(TauriCommandError::internal)?;
        let mvu_rules = collect_mvu_update_rules_for_backend(app.storage(), &ctx, &present_chars)
            .map_err(TauriCommandError::internal)?;
        // B3/B DraftQualityGate + 有界 1× Editor auto-fix
        // regenerate 返回的 node 即当前 node_id（variant 更新）
        let draft_node_for_fix = pipeline_req.node_id.clone();
        let (final_text, quality_report, autofix_provenance) =
            quality_gate_with_optional_editor_autofix(
                final_text,
                QualityAutofixCtx {
                    pipeline: &mut pipeline,
                    draft_node_id: &draft_node_for_fix,
                    conversation_id: &pipeline_req.conversation_id,
                    writing_ctx: &ctx,
                    event_tx: &event_tx,
                    cancel: cancel_rx.clone(),
                    log_prefix: "regenerate",
                    original_provenance: Some(provenance.clone()),
                },
            )
            .await
            .map_err(|error| {
                TauriCommandError::internal(format!("quality auto-fix failed closed: {error}"))
            })?;
        // 返回给前端的必须是 auto-fix 后的正文
        response_text = Some(final_text.clone());
        // 挂到 regenerate 新建的 Attempt：同步 quality_report + draft_hash（Accept 硬校验）。
        // 关键同步失败必须传播，不能 best-effort 返回修复稿却留下原稿 hash。
        let active_turn_after_regenerate = match &ctx.campaign_id {
            Some(campaign_id) => get_active_turn_for_backend(app.storage(), campaign_id)
                .map_err(TauriCommandError::internal)?,
            None => None,
        };
        let pp_identity = match (
            active_turn_after_regenerate.as_ref(),
            regen_attempt_id.as_ref(),
            ctx.campaign_id.as_ref(),
        ) {
            (Some(turn), Some(att_id), Some(campaign_id)) => {
                Some(production_postprocess::PostprocessIdentity {
                    turn_id: turn.turn_id.clone(),
                    attempt_id: att_id.clone(),
                    campaign_id: campaign_id.clone(),
                    conversation_id: conversation_id.clone(),
                    turn_number: ctx.turn,
                })
            }
            _ => None,
        };
        if let Some(identity) = &pp_identity {
            let sink = BackendTurnAttemptSink::production(app.storage().clone());
            if let Err(e) = production_postprocess::TurnAttemptSink::sync_autofix_with_provenance(
                &sink,
                identity,
                &final_text,
                quality_report.clone(),
                autofix_provenance.clone(),
            ) {
                let combined = service_fail_turn(&sink, identity, e);
                clear_current_cancel_if(&app, &operation_id);
                return Err(TauriCommandError::internal(format!(
                    "regenerate auto-fix 后 Attempt 同步失败（draft_hash/quality_report）: {combined}"
                )));
            }
            // SQLite UoW mutated the conversation; JSON cache refresh is a
            // harmless no-op reload. (Gate 3: no backend flag in commands.)
            app.conv_store.invalidate();
        }

        // regenerate 保持同步语义：await 共享后处理；关键失败向上返回。
        let pp_runtime = ctx.campaign_runtime.clone();
        if let Err(e) = run_shared_postprocess_background(
            app.storage().clone(),
            pipeline,
            ctx,
            final_text,
            present_chars,
            var_keys,
            mvu_fragments,
            mvu_rules,
            event_tx.clone(),
            pp_rx,
            pp_identity,
            pp_runtime,
        )
        .await
        {
            clear_current_cancel_if(&app, &operation_id);
            return Err(TauriCommandError::internal(format!(
                "regenerate postprocess 关键失败: {e}"
            )));
        }
    }

    clear_current_cancel_if(&app, &operation_id);

    match result {
        Ok((orig_text, _provenance)) => Ok(turn_lifecycle::prefer_autofix_response_text(
            response_text,
            orig_text,
        )),
        // 保留 PipelineError 分类（Gate 8 审查 P2-B6）。
        Err(e) => Err(TauriCommandError::from(e)),
    }
}

#[tauri::command]
pub(crate) async fn start_writing(
    intent: String,
    character_id: Option<String>,
    conversation_id: Option<String>,
    opening_message: Option<String>,
    generation_mode: Option<storyforge_domain::generation::GenerationMode>,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<WritingEvent>,
) -> Result<serde_json::Value, TauriCommandError> {
    let app = state.inner().clone();
    app.require_active_llm()?;
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();

    // Phase A 屏障：存在非 terminal Turn → 拒绝启动（在追加 user 消息之前）
    check_turn_barrier(&app)?;

    // 前端事件转发任务
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let writing_event = WritingEvent::from_pipeline_event(&event);
            let _ = on_event_clone.send(writing_event);
        }
    });

    // 构造写作上下文（从 tool_ctx 快照读取，导入的角色卡/世界书自动可见）
    let tool_snapshot = app.snapshot_tool_ctx();

    // 在写入开场白/用户意图之前完成自动路由预检。自动升到昂贵群像档时
    // fail closed，调用方须把建议模式作为显式 generation_mode 重试。
    let provisional_conversation_id = conversation_id
        .as_deref()
        .map(Id::from_str)
        .unwrap_or_default();
    let mut ctx = WritingContext {
        characters: tool_snapshot.characters.clone(),
        world_info: tool_snapshot.world_info.clone(),
        conversation_id: provisional_conversation_id,
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
    fill_campaign_context_async(&mut ctx, &app).await?;
    let route_decision = storyforge_domain::generation::route_generation_mode(
        &generation_route_signals(&intent, &ctx, generation_mode),
    );
    enforce_generation_cost_confirmation(&route_decision)?;
    tracing::info!(
        mode = ?route_decision.mode,
        reason = ?route_decision.reason,
        explicit = generation_mode.is_some(),
        "selected writing generation mode"
    );

    // 一 Campaign 一对话：Campaign 模式下用 Campaign 绑定的 conversation_id，
    // 覆盖前端传入的（前端可能在切档时传错或传 null）
    let legacy_opening_character = tool_snapshot.characters.first().cloned();
    let start_target = prepare_start_conversation_async(
        app.clone(),
        None,
        conversation_id,
        character_id.clone(),
        legacy_opening_character,
        opening_message,
        intent.clone(),
    )
    .await?;
    let conversation_id = start_target.conversation_id;
    let regex_character_id = start_target.regex_character_id;
    ctx.conversation_id = conversation_id.clone();
    let campaign_regex_scripts = std::mem::take(&mut ctx.regex_scripts);
    ctx.regex_scripts = collect_scoped_regex_scripts_for_backend(
        regex_character_id.as_deref(),
        &tool_snapshot.characters,
        Some(app.storage()),
    )
    .map_err(TauriCommandError::storage)?;
    append_missing_campaign_scoped_regex_scripts(&mut ctx, campaign_regex_scripts);
    fill_regex_context(&mut ctx, get_preset_store(), get_global_regex_store());
    // 从模块/Profile 存储加载预设配置
    fill_profile_context(&mut ctx, &app);
    // 从活跃 Agent Profile Config 加载运行时配置覆盖
    fill_agent_profile_context(&mut ctx, &app);
    // ContextCompiler：按用户意图自动召回 ArchivedSummary 远记忆
    fill_far_memory_hits(&mut ctx, &app, &intent).await;

    // Phase A: Campaign 模式下创建 TurnRecord
    let turn_record = if let Some(campaign_id) = &ctx.campaign_id {
        // 获取当前 Campaign revision 作为 base
        let base_revision = app
            .storage()
            .get_campaign(campaign_id)
            .map_err(TauriCommandError::internal)?
            .map(|record| record.campaign.revision)
            .unwrap_or(0);
        let input_node = start_target.input_node_id.clone().unwrap_or_else(|| {
            tracing::warn!(
                "Phase A: user 消息节点 ID 未知，TurnRecord.input_node_id 用 placeholder"
            );
            Id::from_str("unknown-input-node")
        });
        let record = storyforge_domain::turn::TurnRecord::new(
            campaign_id.clone(),
            conversation_id.clone(),
            input_node,
            base_revision,
        );
        let create_turn = app.storage().save_turn(&record);
        if let Err(e) = create_turn {
            tracing::error!("Phase A: 创建 TurnRecord 失败: {e}");
            // Gate 8 审查 P2-B1：并发 start_writing 的败者已追加 user 消息，
            // save_turn 因「已有活动 Turn」拒绝——回滚自己的孤儿消息（仅当
            // 它仍是对话最后一个节点，避免误删胜者已追加的节点）。
            if let Some(input) = &start_target.input_node_id {
                rollback_orphaned_user_message(&app, &conversation_id, input);
            }
            return Err(TauriCommandError::internal(format!(
                "创建 TurnRecord 失败（可能已有写作在进行中）: {e}"
            )));
        }
        Some(record)
    } else {
        None // 非 Campaign 模式，不创建 TurnRecord
    };

    // Operation-owned cancel: pipeline / autofix / postprocess all clone this receiver.
    let (operation_id, cancel_rx) = begin_writing_operation(&app);

    // 每次用最新 tool_ctx 快照构造 orchestrator（保证导入后立刻生效）
    let prompt_hook = frontend_prompt_hook(event_tx.clone(), app.prompt_hook_pending.clone());
    let mut pipeline =
        app.new_pipeline_with_regex_and_prompt_hook(&ctx.regex_scripts, Some(prompt_hook))?;
    let result = pipeline
        .start_writing_with_mode(
            intent,
            &ctx,
            route_decision.mode,
            event_tx.clone(),
            cancel_rx.clone(),
        )
        .await;

    // ─── P2 后处理流水线（后台执行，不阻断成文返回）──────────────────────
    // 成文（DraftReady）后跑：剧情总结 + 后处理三合一。
    // postprocess 放后台 spawn——draft_ready 后立即返回成文给前端，
    // postprocess 在后台跑（知识/变量/摘要写回），通过 event_tx 推进度。
    // 仅在有活跃 Campaign 时执行（无 Campaign 跳过，向后兼容）。
    //
    // auto-fix 可能改写正文：命令返回值必须用修复后的 final_text，
    // 不能再回退到 pipeline 原始 result 里的原稿。
    let mut response_text: Option<String> = None;
    if let Ok((final_text, draft_node_id, provenance)) = &result {
        // Phase A: 成文后创建 TurnAttempt 并更新 TurnRecord → DraftReady。
        // SQLite opt-in: atomic preaccept UoW (conversation + attempt + outbox).
        // JSON default: pipeline already landed the draft; attach attempt separately.
        // 注意：本地 turn_record 快照不含新 Attempt，必须捕获 attempt_id 给后续写回。
        let mut landed_draft_node_id = draft_node_id.clone();
        let created_attempt_id = if let Some(ref turn) = turn_record {
            let attempt_id = Id::new();
            let temps = pipeline.pending_temporary_instances().to_vec();
            let campaign_id = ctx.campaign_id.as_ref().ok_or_else(|| {
                TauriCommandError::internal(
                    "sqlite preaccept draft requires campaign_id".to_string(),
                )
            })?;
            match app.turn_workflow.create_draft_attempt(
                crate::backend_workflows::DraftAttemptRequest {
                    campaign_id,
                    conversation_id: &conversation_id,
                    turn_id: &turn.turn_id,
                    attempt_id: &attempt_id,
                    provisional_variant_id: Some(draft_node_id),
                    draft_text: final_text,
                    pending_temporary_instances: temps,
                    provenance: provenance.clone(),
                },
            ) {
                Ok(outcome) => {
                    landed_draft_node_id = outcome.variant_id;
                    Some(outcome.attempt_id)
                }
                Err(e) => {
                    clear_current_cancel_if(&app, &operation_id);
                    return Err(TauriCommandError::internal(e));
                }
            }
        } else {
            None
        };
        // A.1：临时 instance 不再在 accept 前直接写 Campaign；挂在 Attempt，accept 时 Mutation 落盘

        // 从 session.plan 取在场角色 + 基础变量键
        let present_chars: Vec<String> = pipeline
            .session()
            .and_then(|s| s.plan.as_ref())
            .map(|p| {
                p.subagent_tasks
                    .iter()
                    .map(|t| t.character_id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let final_text = final_text.clone();
        let var_keys = postprocess_variable_keys(&ctx);
        // Reuse the operation-owned cancel receiver (no global re-subscribe / no fallback).
        let pp_cancel_rx = cancel_rx.clone();
        let mvu_fragments =
            collect_mvu_fallback_fragments_for_backend(app.storage(), &ctx, &present_chars)
                .map_err(TauriCommandError::internal)?;
        let mvu_rules = collect_mvu_update_rules_for_backend(app.storage(), &ctx, &present_chars)
            .map_err(TauriCommandError::internal)?;

        // B3/B DraftQualityGate + 有界 1× Editor auto-fix
        // SQLite: use the authoritative landed variant id (not the provisional pipeline id).
        let (final_text, quality_report, autofix_provenance) =
            quality_gate_with_optional_editor_autofix(
                final_text,
                QualityAutofixCtx {
                    pipeline: &mut pipeline,
                    draft_node_id: &landed_draft_node_id,
                    conversation_id: &conversation_id,
                    writing_ctx: &ctx,
                    event_tx: &event_tx,
                    cancel: cancel_rx.clone(),
                    log_prefix: "start_writing",
                    original_provenance: provenance.clone(),
                },
            )
            .await
            .map_err(|error| {
                TauriCommandError::internal(format!("quality auto-fix failed closed: {error}"))
            })?;
        // 返回给前端的必须是 auto-fix 后的正文
        response_text = Some(final_text.clone());
        // 质量报告挂到刚创建的 Attempt，便于 accept 前复查；auto-fix 后同步 draft_hash。
        // 关键同步失败必须传播：否则命令返回修复稿但 Attempt 仍指原稿 hash，Accept 会硬失败。
        let pp_identity = match (&turn_record, &created_attempt_id, &ctx.campaign_id) {
            (Some(turn), Some(attempt_id), Some(campaign_id)) => {
                Some(production_postprocess::PostprocessIdentity {
                    turn_id: turn.turn_id.clone(),
                    attempt_id: attempt_id.clone(),
                    campaign_id: campaign_id.clone(),
                    conversation_id: conversation_id.clone(),
                    turn_number: ctx.turn,
                })
            }
            _ => None,
        };
        if let Some(identity) = &pp_identity {
            let sink = BackendTurnAttemptSink::production(app.storage().clone());
            if let Err(e) = production_postprocess::TurnAttemptSink::sync_autofix_with_provenance(
                &sink,
                identity,
                &final_text,
                quality_report.clone(),
                autofix_provenance.clone(),
            ) {
                let combined = service_fail_turn(&sink, identity, e);
                clear_current_cancel_if(&app, &operation_id);
                return Err(TauriCommandError::internal(format!(
                    "auto-fix 后 Attempt 同步失败（draft_hash/quality_report）: {combined}"
                )));
            }
            // SQLite UoW mutated the conversation; JSON cache refresh is a
            // harmless no-op reload. (Gate 3: no backend flag in commands.)
            app.conv_store.invalidate();
        }

        // postprocess 后台跑，不阻塞 start_writing 返回；业务状态机走共享服务。
        // Keep current_cancel alive until the background task finishes so cancel_writing
        // can still reach postprocess after the command returns.
        let pp_event_tx = event_tx.clone();
        let pp_runtime = ctx.campaign_runtime.clone();
        let app_for_pp = app.clone();
        let operation_id_for_pp = operation_id.clone();
        // final_text 所有权移入后台 postprocess，返回值用克隆副本
        let final_text_for_response = final_text.clone();
        tokio::spawn(async move {
            let result = run_shared_postprocess_background(
                app_for_pp.storage().clone(),
                pipeline,
                ctx,
                final_text,
                present_chars,
                var_keys,
                mvu_fragments,
                mvu_rules,
                pp_event_tx,
                pp_cancel_rx,
                pp_identity,
                pp_runtime,
            )
            .await;
            if let Err(e) = result {
                tracing::error!("start_writing background postprocess failed closed: {e}");
            }
            clear_current_cancel_if(&app_for_pp, &operation_id_for_pp);
        });
        // Defer clear_current_cancel to the spawn completion path below.
        // Skip the normal clear for the success path with background postprocess.
        // （此块位于 `if let Ok(..) = &result` 内，失败路径由函数尾部的第二个
        // `match result` 统一处理——曾存在的内层 Err 分支在运行时不可达，
        // 且与真实失败路径重复维护回滚语义，2026-09-01 全量审查移除。）
        let text =
            turn_lifecycle::prefer_autofix_response_text(response_text, final_text_for_response);
        // Prefer the authoritative landed variant id (SQLite UoW may replace provisional).
        return Ok(serde_json::json!({
            "text": text,
            "conversation_id": conversation_id.to_string(),
            "node_id": landed_draft_node_id.to_string(),
            "generation_mode": route_decision.mode,
            "generation_route_reason": route_decision.reason,
        }));
    }

    // No background postprocess path: clear only this operation.
    clear_current_cancel_if(&app, &operation_id);

    match result {
        Ok((orig_text, node_id, _provenance)) => {
            let text = turn_lifecycle::prefer_autofix_response_text(response_text, orig_text);
            Ok(serde_json::json!({
                "text": text,
                "conversation_id": conversation_id.to_string(),
                "node_id": node_id.to_string(),
                "generation_mode": route_decision.mode,
                "generation_route_reason": route_decision.reason,
            }))
        }
        Err(e) => {
            // Phase A: 写作失败 → TurnRecord 标 Failed（无副作用，安全失败）
            if let Some(ref turn) = turn_record
                && let Err(mark_err) = update_turn_record(app.storage(), &turn.turn_id, |record| {
                    record.status = storyforge_domain::turn::TurnStatus::Failed;
                    record.failure_reason = Some(format!("写作失败: {e}"));
                    record.touch();
                })
            {
                tracing::warn!(
                    "写作失败后标记 Turn {} 为 Failed 失败: {mark_err}",
                    turn.turn_id
                );
            }
            // Gate 8 复评：回滚本次 user 消息（is_last 守卫）——失败后重试不
            // 得留下重复输入节点。
            if let Some(input) = &start_target.input_node_id {
                rollback_orphaned_user_message(&app, &conversation_id, input);
            }
            Err(TauriCommandError::from(e))
        }
    }
}

/// Tauri command: 取消当前运行的写作流水线
///
/// 触发 AppState.current_cancel 的 sender，导演/子Agent/编剧全部中止。
#[tauri::command]
pub(crate) fn cancel_writing(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<bool, TauriCommandError> {
    let slot = state
        .current_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(handle) = slot.as_ref() {
        let _ = handle.cancel_tx.send(true);
        Ok(true)
    } else {
        Ok(false)
    }
}

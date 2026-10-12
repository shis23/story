use std::sync::Arc;

use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::{RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;

use crate::error::TauriCommandError;
use crate::last_user_intent_before;
use crate::production_postprocess;
use crate::services::writing_regenerate::{
    RegenerateRequestDto, parse_target_dto, validate_regenerate_campaign_scope,
};
use crate::turn_lifecycle;
use crate::{
    AppState, BackendTurnAttemptSink, begin_writing_operation, clear_current_cancel_if,
    collect_mvu_fallback_fragments_for_backend, collect_mvu_update_rules_for_backend,
    collect_scoped_regex_scripts_for_backend, fill_agent_profile_context,
    fill_campaign_context_async, fill_far_memory_hits, fill_profile_context, fill_regex_context,
    get_active_turn_for_backend, get_global_regex_store, get_preset_store,
    postprocess_variable_keys, run_shared_postprocess_background, service_fail_turn,
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
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();

    // 前端事件转发任务
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let writing_event = WritingEvent::from_pipeline_event(&event);
            let _ = on_event_clone.send(writing_event);
        }
    });

    let prompt_hook = frontend_prompt_hook(event_tx.clone(), app.prompt_hook_pending.clone());
    start_writing_for_host(
        app,
        StartWritingRequest {
            intent,
            character_id,
            conversation_id,
            opening_message,
            generation_mode,
        },
        event_tx,
        Some(prompt_hook),
    )
    .await
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

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};

use storyforge_app_agent::runtime::{PromptHook, PromptHookContext};
use storyforge_app_conversation::PartialRollTarget;
use storyforge_app_pipeline::{PipelineOrchestrator, RegenerateRequest, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::conversation::Provenance;
use storyforge_domain::llm::ChatMessage;

use crate::campaign_store;
use crate::commands::writing_regenerate::{
    RegenerateRequestDto, parse_target_dto, validate_regenerate_campaign_scope,
};
use crate::error::TauriCommandError;
use crate::last_user_intent_before;
use crate::production_postprocess;
use crate::turn_lifecycle;
use crate::{
    AppState, BackendTurnAttemptSink, PromptHookPendingGuard, PromptHookPendingMap,
    append_missing_campaign_scoped_regex_scripts, begin_writing_operation, check_turn_barrier,
    clear_current_cancel_if, collect_mvu_fallback_fragments_for_backend,
    collect_mvu_update_rules_for_backend, collect_scoped_regex_scripts_for_backend,
    fill_agent_profile_context, fill_campaign_context_async, fill_far_memory_hits,
    fill_profile_context, fill_regex_context, get_active_turn_for_backend, get_global_regex_store,
    get_preset_store, postprocess_variable_keys, prepare_start_conversation_async,
    run_shared_postprocess_background, service_fail_turn, update_turn_record,
};

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

/// 写作流水线事件（Tauri Channel 用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WritingEvent {
    pub(crate) event_type: String,
    pub(crate) data: serde_json::Value,
}

impl WritingEvent {
    pub(crate) fn from_pipeline_event(event: &PipelineEvent) -> Self {
        let (event_type, data) = match event {
            PipelineEvent::Started { session_id } => (
                "started".into(),
                serde_json::json!({ "session_id": session_id }),
            ),
            PipelineEvent::DirectorStarted => ("director_started".into(), serde_json::json!({})),
            PipelineEvent::DirectorProgress { delta } => (
                "director_progress".into(),
                serde_json::json!({ "delta": delta }),
            ),
            PipelineEvent::DirectorDone {
                scene_brief,
                subagent_count,
            } => (
                "director_done".into(),
                serde_json::json!({
                    "scene_brief": scene_brief,
                    "subagent_count": subagent_count,
                }),
            ),
            PipelineEvent::SubagentStarted {
                character_id,
                index,
                total,
            } => (
                "subagent_started".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                    "total": total,
                }),
            ),
            PipelineEvent::SubagentProgress {
                character_id,
                index,
                delta,
            } => (
                "subagent_progress".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                    "delta": delta,
                }),
            ),
            PipelineEvent::SubagentDone {
                character_id,
                index,
                full_text,
            } => (
                "subagent_done".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                    "full_text": full_text,
                }),
            ),
            PipelineEvent::SubagentCancelled {
                character_id,
                index,
            } => (
                "subagent_cancelled".into(),
                serde_json::json!({
                    "character_id": character_id,
                    "index": index,
                }),
            ),
            PipelineEvent::WriterStarted => ("writer_started".into(), serde_json::json!({})),
            PipelineEvent::WriterProgress { delta } => (
                "writer_progress".into(),
                serde_json::json!({ "delta": delta }),
            ),
            PipelineEvent::EditorStarted => ("editor_started".into(), serde_json::json!({})),
            PipelineEvent::EditorProgress { delta } => (
                "editor_progress".into(),
                serde_json::json!({ "delta": delta }),
            ),
            PipelineEvent::DraftReady { text } => {
                ("draft_ready".into(), serde_json::json!({ "text": text }))
            }
            PipelineEvent::QualityChecked {
                passed,
                warning_count,
                error_count,
                warnings,
            } => (
                "quality_checked".into(),
                serde_json::json!({
                    "passed": passed,
                    "warning_count": warning_count,
                    "error_count": error_count,
                    "warnings": warnings,
                }),
            ),
            PipelineEvent::PromptHookRequest {
                request_id,
                role,
                round,
                model,
                messages,
            } => (
                "prompt_hook_request".into(),
                serde_json::json!({
                    "request_id": request_id,
                    "role": role,
                    "round": round,
                    "model": model,
                    "messages": messages,
                }),
            ),
            PipelineEvent::PostProcessStarted {
                summarizer_enabled,
                postprocessor_enabled,
            } => (
                "postprocess_started".into(),
                serde_json::json!({
                    "summarizer_enabled": summarizer_enabled,
                    "postprocessor_enabled": postprocessor_enabled,
                }),
            ),
            PipelineEvent::PostProcessDone {
                knowledge_count,
                variable_count,
                task_count,
            } => (
                "postprocess_done".into(),
                serde_json::json!({
                    "knowledge_count": knowledge_count,
                    "variable_count": variable_count,
                    "task_count": task_count,
                }),
            ),
            PipelineEvent::PostProcessFailed { reason } => (
                "postprocess_failed".into(),
                serde_json::json!({ "reason": reason }),
            ),
            PipelineEvent::PostProcessSkipped { reason } => (
                "postprocess_skipped".into(),
                serde_json::json!({ "reason": reason }),
            ),
            PipelineEvent::SummaryDone { char_count } => (
                "summary_done".into(),
                serde_json::json!({ "char_count": char_count }),
            ),
            PipelineEvent::Committed {
                session_id,
                variant_id,
            } => (
                "committed".into(),
                serde_json::json!({
                    "session_id": session_id,
                    "variant_id": variant_id,
                }),
            ),
            PipelineEvent::Error { message } => {
                ("error".into(), serde_json::json!({ "message": message }))
            }
            PipelineEvent::StateChanged { state } => (
                "state_changed".into(),
                serde_json::json!({ "state": format!("{:?}", state) }),
            ),
        };

        Self { event_type, data }
    }
}

/// Tauri command: 启动写作流水线（通过 Channel 推送事件）
///
/// cancel sender 存进 AppState.current_cancel，前端可调 cancel_writing 中止。
/// 返回 { text, conversation_id, node_id } 供前端后续重 roll 定位。
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PromptHookReply {
    #[serde(default)]
    pub(crate) messages: Option<Vec<ChatMessage>>,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

pub(crate) fn resolve_prompt_hook_pending(
    pending: &PromptHookPendingMap,
    request_id: &str,
    messages: Option<Vec<ChatMessage>>,
    error: Option<String>,
) -> bool {
    let sender = pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(request_id);
    if let Some(sender) = sender {
        let _ = sender.send(PromptHookReply { messages, error });
        true
    } else {
        false
    }
}

pub(crate) fn frontend_prompt_hook(
    event_tx: tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    pending: PromptHookPendingMap,
) -> PromptHook {
    Arc::new(move |ctx: PromptHookContext| {
        let event_tx = event_tx.clone();
        let pending = pending.clone();
        Box::pin(async move {
            let request_id = Id::new().as_str().to_string();
            let original_messages = ctx.messages.clone();
            let (tx, rx) = oneshot::channel();
            pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(request_id.clone(), tx);
            let mut pending_guard =
                PromptHookPendingGuard::new(request_id.clone(), pending.clone());

            if event_tx
                .send(PipelineEvent::PromptHookRequest {
                    request_id: request_id.clone(),
                    role: ctx.role,
                    round: ctx.round,
                    model: ctx.model,
                    messages: ctx.messages,
                })
                .is_err()
            {
                return Ok(original_messages);
            }

            match tokio::time::timeout(std::time::Duration::from_secs(8), rx).await {
                Ok(Ok(reply)) => {
                    pending_guard.disarm();
                    if let Some(error) = reply.error {
                        tracing::warn!(
                            "frontend prompt hook returned error for {request_id}: {error}"
                        );
                    }
                    Ok(reply.messages.unwrap_or(original_messages))
                }
                Ok(Err(_)) => {
                    pending_guard.disarm();
                    Ok(original_messages)
                }
                Err(_) => {
                    tracing::warn!("frontend prompt hook timed out for {request_id}");
                    Ok(original_messages)
                }
            }
        })
    })
}

/// 阶段 B：有界 1× Editor auto-fix 的输入上下文（合并参数以避开 clippy too_many_arguments）。
pub(crate) struct QualityAutofixCtx<'a> {
    pub(crate) pipeline: &'a mut PipelineOrchestrator,
    pub(crate) draft_node_id: &'a Id,
    pub(crate) conversation_id: &'a Id,
    pub(crate) writing_ctx: &'a WritingContext,
    pub(crate) event_tx: &'a tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    pub(crate) cancel: watch::Receiver<bool>,
    pub(crate) log_prefix: &'a str,
    pub(crate) original_provenance: Option<Provenance>,
}

/// 阶段 B：有界 1× Editor auto-fix。
///
/// 对草稿跑 NarrativeContract QualityGate；若有 Error 且尚未 auto-fix，
/// 仅 Editor 重跑一次（hint 来自警告摘要），再 gate。最多 1 次。
pub(crate) async fn quality_gate_with_optional_editor_autofix(
    final_text: String,
    ctx: QualityAutofixCtx<'_>,
) -> Result<
    (
        String,
        storyforge_domain::turn::QualityReport,
        Option<Provenance>,
    ),
    storyforge_app_pipeline::PipelineError,
> {
    let QualityAutofixCtx {
        pipeline,
        draft_node_id,
        conversation_id,
        writing_ctx,
        event_tx,
        cancel,
        log_prefix,
        original_provenance,
    } = ctx;
    production_postprocess::run_quality_gate_with_optional_editor_autofix(
        final_text,
        production_postprocess::QualityAutofixRequest {
            pipeline,
            draft_node_id,
            conversation_id,
            writing_ctx,
            event_tx,
            cancel,
            log_prefix,
            original_provenance,
        },
    )
    .await
}

#[derive(Debug, Clone)]
struct RouteActorSignal {
    id: String,
    name: String,
    agenda: Option<String>,
    private_facts: Vec<String>,
}

#[derive(Debug, Clone)]
struct RouteTaskSignal {
    related_actor_ids: Vec<String>,
    imminent: bool,
}

fn normalized_route_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace() && !character.is_ascii_punctuation())
        .flat_map(char::to_lowercase)
        .collect()
}

fn private_fact_is_relevant(intent: &str, fact: &str) -> bool {
    let intent = normalized_route_text(intent);
    let fact = normalized_route_text(fact);
    if fact.is_empty() || intent.is_empty() {
        return false;
    }
    if intent.contains(&fact) {
        return true;
    }
    let chars = fact.chars().collect::<Vec<_>>();
    const ROUTE_STOP_FRAGMENTS: &[&str] = &[
        "已经", "知道", "他们", "她们", "自己", "这个", "那个", "因为", "所以", "但是",
    ];
    chars
        .windows(2)
        .map(|window| window.iter().collect::<String>())
        .any(|fragment| {
            !ROUTE_STOP_FRAGMENTS.contains(&fragment.as_str()) && intent.contains(&fragment)
        })
}

fn generation_route_signals_from_parts(
    intent: &str,
    actors: &[RouteActorSignal],
    tasks: &[RouteTaskSignal],
    explicit_mode: Option<storyforge_domain::generation::GenerationMode>,
) -> storyforge_domain::generation::GenerationRouteSignals {
    use storyforge_domain::generation::GenerationRouteSignals;

    let large_scene_intent = [
        "全员",
        "所有人",
        "众人",
        "群像",
        "宴会",
        "舞会",
        "会议",
        "集会",
        "战场",
        "围攻",
        "审判",
        "多人",
    ]
    .iter()
    .any(|keyword| intent.contains(keyword));
    let high_tension = [
        "对峙", "质问", "争吵", "冲突", "威胁", "决裂", "打斗", "厮杀", "紧张", "逼问",
    ]
    .iter()
    .any(|keyword| intent.contains(keyword));

    let explicitly_named = actors
        .iter()
        .filter(|actor| intent.contains(&actor.name) || intent.contains(&actor.id))
        .map(|actor| actor.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let task_related = tasks
        .iter()
        .filter(|task| task.imminent)
        .flat_map(|task| task.related_actor_ids.iter().cloned())
        .collect::<std::collections::HashSet<_>>();
    let mut selected_ids = explicitly_named
        .union(&task_related)
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    if large_scene_intent {
        selected_ids.extend(actors.iter().map(|actor| actor.id.clone()));
    }
    if selected_ids.is_empty()
        && let Some(actor) = actors.first()
    {
        selected_ids.insert(actor.id.clone());
    }
    let selected = actors
        .iter()
        .filter(|actor| selected_ids.contains(&actor.id))
        .collect::<Vec<_>>();
    let relevant_private_knowledge_divergence = selected.len() == 2
        && selected.iter().any(|actor| {
            actor
                .private_facts
                .iter()
                .any(|fact| private_fact_is_relevant(intent, fact))
        });
    let opposing_agendas = selected.len() == 2
        && selected[0]
            .agenda
            .as_deref()
            .zip(selected[1].agenda.as_deref())
            .is_some_and(|(left, right)| {
                !left.trim().is_empty()
                    && !right.trim().is_empty()
                    && normalized_route_text(left) != normalized_route_text(right)
            });

    GenerationRouteSignals {
        explicit_mode,
        principal_actor_count: selected.len(),
        large_scene_intent,
        imminent_task_count: tasks.iter().filter(|task| task.imminent).count(),
        direct_interaction: selected.len() == 2 && explicitly_named.len() >= 2,
        relevant_private_knowledge_divergence,
        opposing_agendas,
        high_tension,
    }
}

fn generation_route_signals(
    intent: &str,
    ctx: &WritingContext,
    explicit_mode: Option<storyforge_domain::generation::GenerationMode>,
) -> storyforge_domain::generation::GenerationRouteSignals {
    use storyforge_domain::character_knowledge::PropagationPolicy;
    use storyforge_domain::story_task::{TaskStatus, TaskTrigger};

    if let Some(runtime) = ctx.campaign_runtime.as_ref() {
        let actors = runtime
            .instances
            .iter()
            .map(|instance| RouteActorSignal {
                id: instance.id.to_string(),
                name: instance.name.clone(),
                agenda: instance
                    .variables
                    .iter()
                    .find(|variable| {
                        matches!(
                            variable.key.as_str(),
                            "agenda" | "current_desire" | "goal" | "ongoing_action"
                        )
                    })
                    .map(|variable| {
                        variable
                            .value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| variable.value.to_string())
                    }),
                private_facts: runtime
                    .knowledge_for_instance(instance)
                    .into_iter()
                    .filter(|knowledge| matches!(knowledge.propagation, PropagationPolicy::Private))
                    .map(|knowledge| knowledge.knowledge_text.clone())
                    .collect(),
            })
            .collect::<Vec<_>>();
        let tasks = ctx
            .pending_tasks
            .iter()
            .map(|task| RouteTaskSignal {
                related_actor_ids: task
                    .related_characters
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                imminent: matches!(task.status, TaskStatus::Active)
                    || task.triggers.iter().any(|trigger| {
                        matches!(
                            trigger,
                            TaskTrigger::TurnReminder { at_turn }
                                if *at_turn <= ctx.turn.saturating_add(1)
                        )
                    }),
            })
            .collect::<Vec<_>>();
        return generation_route_signals_from_parts(intent, &actors, &tasks, explicit_mode);
    }

    let actors = ctx
        .characters
        .iter()
        .map(|character| RouteActorSignal {
            id: character.id.to_string(),
            name: character.name.clone(),
            agenda: None,
            private_facts: vec![],
        })
        .collect::<Vec<_>>();
    generation_route_signals_from_parts(intent, &actors, &[], explicit_mode)
}

fn enforce_generation_cost_confirmation(
    decision: &storyforge_domain::generation::GenerationRouteDecision,
) -> Result<(), TauriCommandError> {
    if !decision.requires_cost_confirmation {
        return Ok(());
    }
    let wire_mode =
        serde_json::to_string(&decision.mode).unwrap_or_else(|_| "\"sequential_crew\"".to_string());
    Err(TauriCommandError::validation(format!(
        "自动路由建议升级到 {}（预计 {}）。为避免静默产生高成本调用，本轮尚未启动；请确认后显式重试 generation_mode={}。",
        wire_mode.trim_matches('"'),
        decision.mode.estimated_call_label(),
        wire_mode.trim_matches('"'),
    )))
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
        match result {
            Ok((orig_text, _provisional_node_id, _provenance)) => {
                let text = turn_lifecycle::prefer_autofix_response_text(response_text, orig_text);
                // Prefer the authoritative landed variant id (SQLite UoW may replace provisional).
                return Ok(serde_json::json!({
                    "text": text,
                    "conversation_id": conversation_id.to_string(),
                    "node_id": landed_draft_node_id.to_string(),
                    "generation_mode": route_decision.mode,
                    "generation_route_reason": route_decision.reason,
                }));
            }
            Err(e) => {
                if let Some(ref turn) = turn_record {
                    let _ = update_turn_record(app.storage(), &turn.turn_id, |record| {
                        record.status = storyforge_domain::turn::TurnStatus::Failed;
                        record.failure_reason = Some(format!("写作失败: {e}"));
                        record.touch();
                    });
                }
                clear_current_cancel_if(&app, &operation_id);
                // 保留 PipelineError 分类（retryable/429/超时），不让前端契约丢失
                // （Gate 8 审查 P2-B6：format! 拍平会把一切变成 Internal）。
                return Err(TauriCommandError::from(e));
            }
        }
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
            if let Some(ref turn) = turn_record {
                let _ = update_turn_record(app.storage(), &turn.turn_id, |record| {
                    record.status = storyforge_domain::turn::TurnStatus::Failed;
                    record.failure_reason = Some(format!("写作失败: {e}"));
                    record.touch();
                });
            }
            Err(TauriCommandError::from(e))
        }
    }
}

/// 回滚并发败者的孤儿 user 消息（Gate 8 审查 P2-B1）。
///
/// 仅当 `input_node_id` 仍是对话的最后一个节点时才截断删除——胜者若已在
/// 其后追加节点，保守保留（避免误删胜者输入），孤儿消息交由用户手动清理。
fn rollback_orphaned_user_message(
    app: &Arc<crate::AppState>,
    conversation_id: &Id,
    input_node_id: &Id,
) {
    let Some(conv) = app.conv_store.get(conversation_id) else {
        return;
    };
    let is_last = conv
        .nodes
        .last()
        .map(|n| &n.id == input_node_id)
        .unwrap_or(false);
    if !is_last {
        tracing::debug!("孤儿 user 消息非最后节点，保守保留: {input_node_id}");
        return;
    }
    match app.conv_store.truncate_from(conversation_id, input_node_id) {
        Ok(()) => tracing::info!("已回滚并发败者的孤儿 user 消息: {input_node_id}"),
        Err(e) => tracing::warn!("回滚孤儿 user 消息失败（保留原样）: {e}"),
    }
}

pub(crate) fn normalize_task_update_for_postprocess(
    camp_id: &Id,
    mut task: storyforge_domain::story_task::StoryTask,
    new_status: storyforge_domain::story_task::TaskStatus,
) -> Option<storyforge_domain::story_task::StoryTask> {
    if task.campaign_id != *camp_id {
        tracing::warn!(
            "跳过非当前 Campaign 任务 '{}' 的状态更新（task campaign: {}, current campaign: {}）",
            task.id,
            task.campaign_id,
            camp_id
        );
        return None;
    }

    task.status = new_status;
    Some(task)
}

/// 按名字或 Id 查 campaign 内的 CharacterInstance（后处理 Agent 输出的是角色名，需翻译成 instance）
/// P3：内部按 KnowledgeSource 分流——ToldByOther/Backstory 不查在场直接放行，Witnessed/Inferred 才查在场。
/// P4：同名收紧——name_collisions 传入 campaign 内出现 ≥2 次的 name 集合，同名时 name 路失效。
/// W6 方向 1：broadcast 非空时分发给多个 target（All=全体, Group=身份组），返回 Vec。
pub fn normalize_knowledge_update_for_postprocess(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    turn: u32,
    present_ids: &std::collections::HashSet<String>,
    name_collisions: &std::collections::HashSet<String>,
) -> Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    normalize_knowledge_update_for_postprocess_with_extras(
        store,
        camp_id,
        update,
        turn,
        present_ids,
        name_collisions,
        &[],
    )
}

/// V2 变体：`extra_instances` 是 attempt 上尚未落盘的临时角色，参与目标/来源解析。
/// accept 时 `prepare_commit_batch` 前置 `UpsertInstance`，指向其 id 的条目落库安全。
pub fn normalize_knowledge_update_for_postprocess_with_extras(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    turn: u32,
    present_ids: &std::collections::HashSet<String>,
    name_collisions: &std::collections::HashSet<String>,
    extra_instances: &[storyforge_domain::campaign::CharacterInstance],
) -> Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    use storyforge_domain::character_knowledge::PropagationPolicy;

    if update.propagation == PropagationPolicy::Private && update.broadcast.is_some() {
        tracing::warn!("跳过 private 知识的广播写入：{}", update.knowledge_text);
        return vec![];
    }

    if update.broadcast.is_some()
        && should_block_source_knowledge_propagation(store, camp_id, update, None)
    {
        return vec![];
    }

    // 方向 1：广播分发——broadcast 非空时遍历 instances，每个生成一条 ToldByOther
    if let Some(ref broadcast) = update.broadcast {
        return dispatch_broadcast(store, camp_id, update, turn, broadcast, extra_instances);
    }

    // 非广播：单角色逻辑（原有 P3/P4 流程）
    let target = match find_instance_by_name_or_id_with_extras(
        store,
        camp_id,
        &update.character_id,
        extra_instances,
    ) {
        Some(inst) => inst,
        None => {
            tracing::warn!(
                "跳过无法解析到 Campaign instance 的知识写入目标: {}",
                update.character_id
            );
            return vec![];
        }
    };

    if should_block_source_knowledge_propagation(store, camp_id, update, Some(&target)) {
        return vec![];
    }

    // P3 分流：ToldByOther/Backstory 不受在场约束（跨在场告知 + 开局已有），
    // Witnessed/Inferred 才查在场。
    let knowledge_exempt_from_presence = matches!(
        update.source,
        storyforge_domain::character_knowledge::KnowledgeSource::ToldByOther
            | storyforge_domain::character_knowledge::KnowledgeSource::Backstory
    );
    if !knowledge_exempt_from_presence {
        // 知识路径收紧：空集时 Witnessed/Inferred 也拒绝（无人在场不可能见证/推断）
        // 注意：is_postprocess_instance_present 的空集放行仍服务变量路径，此处绕过它。
        if present_ids.is_empty()
            || !is_postprocess_instance_present(
                &target,
                &update.character_id,
                present_ids,
                name_collisions,
            )
        {
            tracing::warn!(
                "跳过非在场角色 '{}' 的知识写入（source={:?}，present_chars 校验）",
                target.name,
                update.source
            );
            return vec![];
        }
    }

    let source_character_id = update
        .source_character_id
        .as_ref()
        .and_then(|source_id| {
            find_instance_by_name_or_id_with_extras(store, camp_id, source_id, extra_instances)
        })
        .map(|source| source.id);
    let source_knowledge_id =
        matching_source_knowledge_for_update(store, camp_id, update).map(|entry| entry.id);

    vec![
        storyforge_domain::character_knowledge::CharacterKnowledgeEntry {
            id: Id::new(),
            campaign_id: camp_id.clone(),
            character_id: target.id,
            knowledge_text: update.knowledge_text.clone(),
            source: update.source.clone(),
            source_character_id,
            source_knowledge_id,
            turn_number: turn,
            event_id: None,
            pinned: update.pinned,
            propagation: update.propagation.clone(),
        },
    ]
}

/// 方向 1：广播分发——根据 BroadcastTarget 遍历 campaign 内 instance，各生成一条 ToldByOther。
///
/// - `All`：campaign 内所有 instance（排除广播发起者自身）
/// - `Group(g)`：definition.group == g 的 instance（通过 definition_id 反查 CharacterDefinition）
///
/// 广播条目的 source 统一为 `ToldByOther`，source_character_id 记广播发起者（若有）。
fn dispatch_broadcast(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    turn: u32,
    broadcast: &storyforge_domain::character_knowledge::BroadcastTarget,
    extra_instances: &[storyforge_domain::campaign::CharacterInstance],
) -> Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    use storyforge_domain::character_knowledge::BroadcastTarget;

    // 解析广播发起者（source_character_id）的 persisted id，用于排除自身 + 记录来源
    let broadcaster_inst = update.source_character_id.as_ref().and_then(|sid| {
        find_instance_by_name_or_id_with_extras(store, camp_id, sid, extra_instances)
    });
    let broadcaster_id = broadcaster_inst.as_ref().map(|i| i.id.clone());
    let source_character_id = broadcaster_id.clone();
    let source_knowledge_id =
        matching_source_knowledge_for_update(store, camp_id, update).map(|entry| entry.id);

    // V2: 未落盘临时角色也在广播受众内（本轮它们已是 campaign 成员，accept 时落库）
    let mut all_instances = store.list_instances(camp_id);
    for temp in extra_instances {
        if temp.campaign_id == *camp_id && !all_instances.iter().any(|i| i.id == temp.id) {
            all_instances.push(temp.clone());
        }
    }

    let targets: Vec<_> = match broadcast {
        BroadcastTarget::All => all_instances
            .into_iter()
            // 排除广播发起者自身（不应该给自己发广播知识）
            .filter(|inst| Some(&inst.id) != broadcaster_id.as_ref())
            .collect(),
        BroadcastTarget::Group(group) => all_instances
            .into_iter()
            .filter(|inst| {
                // 排除广播发起者自身
                if Some(&inst.id) == broadcaster_id.as_ref() {
                    return false;
                }
                // 通过 definition_id 反查 definition.group
                instance_matches_group(store, inst, group)
            })
            .collect(),
    };

    if targets.is_empty() {
        tracing::warn!(
            "广播分发: broadcast={:?} 无匹配 instance（campaign={}）",
            broadcast,
            camp_id
        );
    }

    targets
        .into_iter()
        .map(|inst| {
            storyforge_domain::character_knowledge::CharacterKnowledgeEntry {
                id: Id::new(),
                campaign_id: camp_id.clone(),
                character_id: inst.id,
                knowledge_text: update.knowledge_text.clone(),
                // 广播统一为 ToldByOther（被告知/公告）
                source: storyforge_domain::character_knowledge::KnowledgeSource::ToldByOther,
                source_character_id: source_character_id.clone(),
                source_knowledge_id: source_knowledge_id.clone(),
                turn_number: turn,
                event_id: None,
                pinned: update.pinned,
                propagation: update.propagation.clone(),
            }
        })
        .collect()
}

fn matching_source_knowledge_for_update(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
) -> Option<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    use storyforge_domain::character_knowledge::KnowledgeSource;

    let is_relay =
        update.broadcast.is_some() || matches!(update.source, KnowledgeSource::ToldByOther);
    if !is_relay {
        return None;
    }

    let source_id = update.source_character_id.as_ref()?;
    let source = find_instance_by_name_or_id(store, camp_id, source_id)?;

    store
        .list_knowledge_of(camp_id, &source.id)
        .into_iter()
        .filter(|entry| knowledge_text_matches(&entry.knowledge_text, &update.knowledge_text))
        .max_by(|a, b| {
            a.turn_number
                .cmp(&b.turn_number)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        })
}

fn should_block_source_knowledge_propagation(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    target: Option<&storyforge_domain::campaign::CharacterInstance>,
) -> bool {
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, KnowledgeSource, PropagationPolicy,
    };

    let is_propagating =
        update.broadcast.is_some() || matches!(update.source, KnowledgeSource::ToldByOther);
    if !is_propagating {
        return false;
    }

    let Some(source_id) = &update.source_character_id else {
        return false;
    };
    let Some(source) = find_instance_by_name_or_id(store, camp_id, source_id) else {
        return false;
    };

    let source_entries = store.list_knowledge_of(camp_id, &source.id);
    for entry in source_entries {
        if !knowledge_text_matches(&entry.knowledge_text, &update.knowledge_text) {
            continue;
        }

        let blocked = match &entry.propagation {
            PropagationPolicy::Open => false,
            PropagationPolicy::Private => true,
            PropagationPolicy::GroupRestricted(group) => match (&update.broadcast, target) {
                (Some(BroadcastTarget::Group(target_group)), _) => target_group != group,
                (Some(BroadcastTarget::All), _) => true,
                (None, Some(target_inst)) => !instance_matches_group(store, target_inst, group),
                (None, None) => true,
            },
        };

        if blocked {
            tracing::warn!(
                "阻止知识传播：source={} policy={:?} text={}",
                source.name,
                entry.propagation,
                update.knowledge_text
            );
            return true;
        }
    }

    false
}

pub(crate) fn knowledge_text_matches(restricted: &str, candidate: &str) -> bool {
    let restricted = normalize_knowledge_text(restricted);
    let candidate = normalize_knowledge_text(candidate);
    if restricted.is_empty() || candidate.is_empty() {
        return false;
    }
    if restricted == candidate {
        return true;
    }

    let min_len = restricted.chars().count().min(candidate.chars().count());
    min_len >= 8 && (restricted.contains(&candidate) || candidate.contains(&restricted))
}

fn normalize_knowledge_text(text: &str) -> String {
    text.split_whitespace().collect::<String>().to_lowercase()
}

/// 判断 instance 的 CharacterDefinition.group 是否匹配目标组名。
/// 通过 instance.definition_id → 遍历所有 card 的 character_definitions 找匹配。
fn instance_matches_group(
    store: &campaign_store::CampaignStore,
    inst: &storyforge_domain::campaign::CharacterInstance,
    target_group: &str,
) -> bool {
    let def_id = match &inst.definition_id {
        Some(id) => id,
        None => return false, // 无 definition_id（临时角色）→ 不匹配
    };
    // 遍历所有 card，找 definition_id 匹配的 definition
    for stored_card in store.list_cards() {
        if let Some(def) = stored_card
            .card
            .character_definitions
            .iter()
            .find(|d| d.id == *def_id)
        {
            return def.group.as_deref() == Some(target_group);
        }
    }
    false
}

pub fn is_postprocess_instance_present(
    inst: &storyforge_domain::campaign::CharacterInstance,
    raw_id: &Id,
    present_ids: &std::collections::HashSet<String>,
    name_collisions: &std::collections::HashSet<String>,
) -> bool {
    if present_ids.is_empty() {
        tracing::warn!(
            "postprocess 写回: present_chars 为空集，放行 '{}'（向后兼容逃生口，变量路径仍依赖）",
            inst.name
        );
        return true;
    }
    // id 路：只认 instance id（或 raw_id 本身就是该 instance id）。
    // 不能把 raw_id 的任意字符串命中 present 都当 id 路——同名碰撞时 Agent 常给角色名，
    // 若把 name 当成 id 命中，会绕过下面的 name_collisions 收紧并静默写到第一个同名目标。
    if present_ids.contains(inst.id.as_str())
        || (raw_id == &inst.id && present_ids.contains(raw_id.as_str()))
    {
        return true;
    }
    // name 路兜底：P4 同名收紧——campaign 内存在同名 instance 时 name 路失效，逼 id
    if present_ids.contains(&inst.name) && !name_collisions.contains(&inst.name) {
        tracing::debug!(
            "postprocess 写回: '{}' 通过 name 匹配在场（非 id 匹配）",
            inst.name
        );
        return true;
    }
    false
}

pub(crate) fn find_instance_by_name_or_id(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    name_or_id: &Id,
) -> Option<storyforge_domain::campaign::CharacterInstance> {
    find_instance_by_name_or_id_with_extras(store, camp_id, name_or_id, &[])
}

/// V2 变体：解析域 = 已持久化 instance + attempt 上未落盘的临时实例（按 id 去重）。
pub(crate) fn find_instance_by_name_or_id_with_extras(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    name_or_id: &Id,
    extra_instances: &[storyforge_domain::campaign::CharacterInstance],
) -> Option<storyforge_domain::campaign::CharacterInstance> {
    let mut instances = store.list_instances(camp_id);
    for extra in extra_instances {
        if extra.campaign_id == *camp_id && !instances.iter().any(|i| i.id == extra.id) {
            instances.push(extra.clone());
        }
    }
    // 先精确 id 匹配
    if let Some(i) = instances.iter().find(|i| i.id == *name_or_id) {
        return Some(i.clone());
    }
    // 再按 instance.name 匹配（后处理 Agent 给的是角色名）
    instances
        .into_iter()
        .find(|i| i.name.as_str() == name_or_id.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::generation::{
        GenerationMode, GenerationRouteDecision, GenerationRouteReason,
    };

    #[tokio::test]
    async fn frontend_prompt_hook_round_trips_messages_through_pending_reply() {
        use std::sync::Mutex;
        use storyforge_domain::agent::AgentRole;

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
        let pending: PromptHookPendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let hook = frontend_prompt_hook(event_tx, pending.clone());
        let original = vec![ChatMessage::user("before hook")];

        let hook_task = tokio::spawn(hook(PromptHookContext {
            role: AgentRole::Editor,
            round: 1,
            model: "test-model".into(),
            messages: original.clone(),
        }));

        let event = event_rx.recv().await.expect("hook should emit request");
        let request_id = match event {
            PipelineEvent::PromptHookRequest {
                request_id,
                role,
                round,
                model,
                messages,
            } => {
                assert_eq!(role, AgentRole::Editor);
                assert_eq!(round, 1);
                assert_eq!(model, "test-model");
                assert_eq!(messages[0].content, "before hook");
                request_id
            }
            other => panic!("expected prompt hook request, got {other:?}"),
        };

        let sender = pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&request_id)
            .expect("pending reply sender should be registered");
        sender
            .send(PromptHookReply {
                messages: Some(vec![
                    ChatMessage::system("plugin system"),
                    ChatMessage::user("after hook"),
                ]),
                error: None,
            })
            .unwrap();

        let hooked = hook_task.await.unwrap().unwrap();
        assert_eq!(hooked.len(), 2);
        assert_eq!(hooked[0].content, "plugin system");
        assert_eq!(hooked[1].content, "after hook");
    }

    #[tokio::test]
    async fn frontend_prompt_hook_cleans_pending_request_when_cancelled() {
        use std::sync::Mutex;
        use storyforge_domain::agent::AgentRole;

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
        let pending: PromptHookPendingMap = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let hook = frontend_prompt_hook(event_tx, pending.clone());

        let hook_task = tokio::spawn(hook(PromptHookContext {
            role: AgentRole::Editor,
            round: 1,
            model: "test-model".into(),
            messages: vec![ChatMessage::user("before hook")],
        }));

        let event = event_rx.recv().await.expect("hook should emit request");
        let request_id = match event {
            PipelineEvent::PromptHookRequest { request_id, .. } => request_id,
            other => panic!("expected prompt hook request, got {other:?}"),
        };
        assert!(
            pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(&request_id)
        );

        hook_task.abort();
        let _ = hook_task.await;

        assert!(
            !pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(&request_id)
        );
    }

    #[test]
    fn automatic_route_promotes_large_roster_to_sequential_crew() {
        let actors = ["林秋", "陈默", "周岚", "守门人"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| RouteActorSignal {
                id: format!("actor-{index}"),
                name: name.into(),
                agenda: None,
                private_facts: vec![],
            })
            .collect::<Vec<_>>();
        let signals = generation_route_signals_from_parts("所有人在审判厅对峙", &actors, &[], None);

        assert_eq!(signals.principal_actor_count, 4);
        assert!(signals.large_scene_intent);
        assert_eq!(
            storyforge_domain::generation::route_generation_mode(&signals).mode,
            GenerationMode::SequentialCrew,
        );
    }

    #[test]
    fn automatic_route_detects_explicit_two_actor_interaction() {
        let actors = vec![
            RouteActorSignal {
                id: "lin".into(),
                name: "林秋".into(),
                agenda: Some("隐瞒钥匙".into()),
                private_facts: vec!["钥匙藏在钟里".into()],
            },
            RouteActorSignal {
                id: "chen".into(),
                name: "陈默".into(),
                agenda: Some("找出钥匙".into()),
                private_facts: vec![],
            },
        ];
        let signals =
            generation_route_signals_from_parts("陈默围绕钥匙质问林秋", &actors, &[], None);

        assert_eq!(signals.principal_actor_count, 2);
        assert!(signals.direct_interaction);
        assert!(signals.relevant_private_knowledge_divergence);
        assert!(signals.opposing_agendas);
        assert_eq!(
            storyforge_domain::generation::route_generation_mode(&signals).mode,
            GenerationMode::Duet,
        );
    }

    #[test]
    fn automatic_expensive_route_requires_an_explicit_resubmission() {
        let decision = GenerationRouteDecision {
            mode: GenerationMode::SequentialCrew,
            reason: GenerationRouteReason::ActorCount,
            requires_cost_confirmation: true,
        };

        let error = enforce_generation_cost_confirmation(&decision)
            .expect_err("automatic expensive upgrade must fail closed");
        let message = error.to_string();
        assert!(message.contains("sequential_crew"));
        assert!(message.contains("2+N"));
        assert!(message.contains("generation_mode"));
    }

    #[test]
    fn cheap_or_explicit_route_needs_no_extra_confirmation() {
        for decision in [
            GenerationRouteDecision {
                mode: GenerationMode::Continuation,
                reason: GenerationRouteReason::EconomyDefault,
                requires_cost_confirmation: false,
            },
            GenerationRouteDecision {
                mode: GenerationMode::SequentialCrew,
                reason: GenerationRouteReason::ExplicitChoice,
                requires_cost_confirmation: false,
            },
        ] {
            enforce_generation_cost_confirmation(&decision).unwrap();
        }
    }

    #[test]
    fn postprocess_task_update_skips_other_campaign() {
        use storyforge_domain::story_task::{StoryTask, TaskStatus};

        let task = StoryTask::user_planned(
            Id::from_str("campaign-b"),
            "Find the archive",
            "Unrelated campaign task",
            vec![],
            1,
        );

        assert!(
            normalize_task_update_for_postprocess(
                &Id::from_str("campaign-a"),
                task,
                TaskStatus::Completed,
            )
            .is_none()
        );
    }

    #[test]
    fn postprocess_task_update_allows_current_campaign() {
        use storyforge_domain::story_task::{StoryTask, TaskStatus};

        let task = StoryTask::user_planned(
            Id::from_str("campaign-a"),
            "Find the archive",
            "Current campaign task",
            vec![],
            1,
        );

        let updated = normalize_task_update_for_postprocess(
            &Id::from_str("campaign-a"),
            task,
            TaskStatus::Completed,
        )
        .expect("same campaign task should update");
        assert_eq!(updated.status, TaskStatus::Completed);
    }
}

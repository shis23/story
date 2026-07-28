use super::super::*;

// \u{2500}\u{2500}\u{2500} M1 \u{5199}\u{4f5c}\u{547d}\u{4ee4} \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}

/// \u{5199}\u{4f5c}\u{6d41}\u{6c34}\u{7ebf}\u{4e8b}\u{4ef6}\u{ff08}Tauri Channel \u{7528}\u{ff09}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WritingEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

impl WritingEvent {
    pub fn from_pipeline_event(event: &PipelineEvent) -> Self {
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

/// Tauri command: \u{542f}\u{52a8}\u{5199}\u{4f5c}\u{6d41}\u{6c34}\u{7ebf}\u{ff08}\u{901a}\u{8fc7} Channel \u{63a8}\u{9001}\u{4e8b}\u{4ef6}\u{ff09}
///
/// cancel sender \u{5b58}\u{8fdb} AppState.current_cancel\u{ff0c}\u{524d}\u{7aef}\u{53ef}\u{8c03} cancel_writing \u{4e2d}\u{6b62}\u{3002}
/// \u{8fd4}\u{56de} { text, conversation_id, node_id } \u{4f9b}\u{524d}\u{7aef}\u{540e}\u{7eed}\u{91cd} roll \u{5b9a}\u{4f4d}\u{3002}
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

/// \u{9636}\u{6bb5} B\u{ff1a}\u{6709}\u{754c} 1\u{d7} Editor auto-fix \u{7684}\u{8f93}\u{5165}\u{4e0a}\u{4e0b}\u{6587}\u{ff08}\u{5408}\u{5e76}\u{53c2}\u{6570}\u{4ee5}\u{907f}\u{5f00} clippy too_many_arguments\u{ff09}\u{3002}
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

/// \u{9636}\u{6bb5} B\u{ff1a}\u{6709}\u{754c} 1\u{d7} Editor auto-fix\u{3002}
///
/// \u{5bf9}\u{8349}\u{7a3f}\u{8dd1} NarrativeContract QualityGate\u{ff1b}\u{82e5}\u{6709} Error \u{4e14}\u{5c1a}\u{672a} auto-fix\u{ff0c}
/// \u{4ec5} Editor \u{91cd}\u{8dd1}\u{4e00}\u{6b21}\u{ff08}hint \u{6765}\u{81ea}\u{8b66}\u{544a}\u{6458}\u{8981}\u{ff09}\u{ff0c}\u{518d} gate\u{3002}\u{6700}\u{591a} 1 \u{6b21}\u{3002}
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
pub(crate) struct RouteActorSignal {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) agenda: Option<String>,
    pub(crate) private_facts: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RouteTaskSignal {
    related_actor_ids: Vec<String>,
    imminent: bool,
}

pub(crate) fn normalized_route_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace() && !character.is_ascii_punctuation())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn private_fact_is_relevant(intent: &str, fact: &str) -> bool {
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
        "\u{5df2}\u{7ecf}",
        "\u{77e5}\u{9053}",
        "\u{4ed6}\u{4eec}",
        "\u{5979}\u{4eec}",
        "\u{81ea}\u{5df1}",
        "\u{8fd9}\u{4e2a}",
        "\u{90a3}\u{4e2a}",
        "\u{56e0}\u{4e3a}",
        "\u{6240}\u{4ee5}",
        "\u{4f46}\u{662f}",
    ];
    chars
        .windows(2)
        .map(|window| window.iter().collect::<String>())
        .any(|fragment| {
            !ROUTE_STOP_FRAGMENTS.contains(&fragment.as_str()) && intent.contains(&fragment)
        })
}

pub(crate) fn generation_route_signals_from_parts(
    intent: &str,
    actors: &[RouteActorSignal],
    tasks: &[RouteTaskSignal],
    explicit_mode: Option<storyforge_domain::generation::GenerationMode>,
) -> storyforge_domain::generation::GenerationRouteSignals {
    use storyforge_domain::generation::GenerationRouteSignals;

    let large_scene_intent = [
        "\u{5168}\u{5458}",
        "\u{6240}\u{6709}\u{4eba}",
        "\u{4f17}\u{4eba}",
        "\u{7fa4}\u{50cf}",
        "\u{5bb4}\u{4f1a}",
        "\u{821e}\u{4f1a}",
        "\u{4f1a}\u{8bae}",
        "\u{96c6}\u{4f1a}",
        "\u{6218}\u{573a}",
        "\u{56f4}\u{653b}",
        "\u{5ba1}\u{5224}",
        "\u{591a}\u{4eba}",
    ]
    .iter()
    .any(|keyword| intent.contains(keyword));
    let high_tension = [
        "\u{5bf9}\u{5cd9}",
        "\u{8d28}\u{95ee}",
        "\u{4e89}\u{5435}",
        "\u{51b2}\u{7a81}",
        "\u{5a01}\u{80c1}",
        "\u{51b3}\u{88c2}",
        "\u{6253}\u{6597}",
        "\u{53ae}\u{6740}",
        "\u{7d27}\u{5f20}",
        "\u{903c}\u{95ee}",
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

pub(crate) fn generation_route_signals(
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

pub(crate) fn enforce_generation_cost_confirmation(
    decision: &storyforge_domain::generation::GenerationRouteDecision,
) -> Result<(), TauriCommandError> {
    if !decision.requires_cost_confirmation {
        return Ok(());
    }
    let wire_mode =
        serde_json::to_string(&decision.mode).unwrap_or_else(|_| "\"sequential_crew\"".to_string());
    Err(TauriCommandError::validation(format!(
        "\u{81ea}\u{52a8}\u{8def}\u{7531}\u{5efa}\u{8bae}\u{5347}\u{7ea7}\u{5230} {}\u{ff08}\u{9884}\u{8ba1} {}\u{ff09}\u{3002}\u{4e3a}\u{907f}\u{514d}\u{9759}\u{9ed8}\u{4ea7}\u{751f}\u{9ad8}\u{6210}\u{672c}\u{8c03}\u{7528}\u{ff0c}\u{672c}\u{8f6e}\u{5c1a}\u{672a}\u{542f}\u{52a8}\u{ff1b}\u{8bf7}\u{786e}\u{8ba4}\u{540e}\u{663e}\u{5f0f}\u{91cd}\u{8bd5} generation_mode={}\u{3002}",
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

    // Phase A \u{5c4f}\u{969c}\u{ff1a}\u{5b58}\u{5728}\u{975e} terminal Turn \u{2192} \u{62d2}\u{7edd}\u{542f}\u{52a8}\u{ff08}\u{5728}\u{8ffd}\u{52a0} user \u{6d88}\u{606f}\u{4e4b}\u{524d}\u{ff09}
    check_turn_barrier(&app)?;

    // \u{524d}\u{7aef}\u{4e8b}\u{4ef6}\u{8f6c}\u{53d1}\u{4efb}\u{52a1}
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let writing_event = WritingEvent::from_pipeline_event(&event);
            let _ = on_event_clone.send(writing_event);
        }
    });

    // \u{6784}\u{9020}\u{5199}\u{4f5c}\u{4e0a}\u{4e0b}\u{6587}\u{ff08}\u{4ece} tool_ctx \u{5feb}\u{7167}\u{8bfb}\u{53d6}\u{ff0c}\u{5bfc}\u{5165}\u{7684}\u{89d2}\u{8272}\u{5361}/\u{4e16}\u{754c}\u{4e66}\u{81ea}\u{52a8}\u{53ef}\u{89c1}\u{ff09}
    let tool_snapshot = app.snapshot_tool_ctx();

    // \u{5728}\u{5199}\u{5165}\u{5f00}\u{573a}\u{767d}/\u{7528}\u{6237}\u{610f}\u{56fe}\u{4e4b}\u{524d}\u{5b8c}\u{6210}\u{81ea}\u{52a8}\u{8def}\u{7531}\u{9884}\u{68c0}\u{3002}\u{81ea}\u{52a8}\u{5347}\u{5230}\u{6602}\u{8d35}\u{7fa4}\u{50cf}\u{6863}\u{65f6}
    // fail closed\u{ff0c}\u{8c03}\u{7528}\u{65b9}\u{987b}\u{628a}\u{5efa}\u{8bae}\u{6a21}\u{5f0f}\u{4f5c}\u{4e3a}\u{663e}\u{5f0f} generation_mode \u{91cd}\u{8bd5}\u{3002}
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

    // \u{4e00} Campaign \u{4e00}\u{5bf9}\u{8bdd}\u{ff1a}Campaign \u{6a21}\u{5f0f}\u{4e0b}\u{7528} Campaign \u{7ed1}\u{5b9a}\u{7684} conversation_id\u{ff0c}
    // \u{8986}\u{76d6}\u{524d}\u{7aef}\u{4f20}\u{5165}\u{7684}\u{ff08}\u{524d}\u{7aef}\u{53ef}\u{80fd}\u{5728}\u{5207}\u{6863}\u{65f6}\u{4f20}\u{9519}\u{6216}\u{4f20} null\u{ff09}
    let legacy_opening_character = tool_snapshot.characters.first().cloned();
    let start_target = prepare_start_conversation_async(
        app.clone(),
        get_campaign_store(),
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
    ctx.regex_scripts =
        collect_scoped_regex_scripts(regex_character_id.as_deref(), &tool_snapshot.characters);
    append_missing_campaign_scoped_regex_scripts(&mut ctx, campaign_regex_scripts);
    fill_regex_context(&mut ctx, get_preset_store(), get_global_regex_store());
    // \u{4ece}\u{6a21}\u{5757}/Profile \u{5b58}\u{50a8}\u{52a0}\u{8f7d}\u{9884}\u{8bbe}\u{914d}\u{7f6e}
    fill_profile_context(&mut ctx, &app);
    // \u{4ece}\u{6d3b}\u{8dc3} Agent Profile Config \u{52a0}\u{8f7d}\u{8fd0}\u{884c}\u{65f6}\u{914d}\u{7f6e}\u{8986}\u{76d6}
    fill_agent_profile_context(&mut ctx, &app);
    // ContextCompiler\u{ff1a}\u{6309}\u{7528}\u{6237}\u{610f}\u{56fe}\u{81ea}\u{52a8}\u{53ec}\u{56de} ArchivedSummary \u{8fdc}\u{8bb0}\u{5fc6}
    fill_far_memory_hits(&mut ctx, &app, &intent).await;

    // Phase A: Campaign \u{6a21}\u{5f0f}\u{4e0b}\u{521b}\u{5efa} TurnRecord
    let turn_record = if let Some(campaign_id) = &ctx.campaign_id {
        // \u{83b7}\u{53d6}\u{5f53}\u{524d} Campaign revision \u{4f5c}\u{4e3a} base
        let base_revision = if sqlite_runtime::is_sqlite_active() {
            sqlite_runtime::get_campaign(campaign_id)
                .map_err(TauriCommandError::internal)?
                .ok_or_else(|| {
                    TauriCommandError::internal(format!("sqlite campaign {} missing", campaign_id))
                })?
                .revision
        } else {
            get_campaign_store()
                .get_campaign(campaign_id)
                .map(|c| c.revision)
                .unwrap_or(0)
        };
        let input_node = start_target.input_node_id.unwrap_or_else(|| {
            tracing::warn!(
                "Phase A: user \u{6d88}\u{606f}\u{8282}\u{70b9} ID \u{672a}\u{77e5}\u{ff0c}TurnRecord.input_node_id \u{7528} placeholder"
            );
            Id::from_str("unknown-input-node")
        });
        let record = storyforge_domain::turn::TurnRecord::new(
            campaign_id.clone(),
            conversation_id.clone(),
            input_node,
            base_revision,
        );
        let create_turn = if sqlite_runtime::is_sqlite_active() {
            sqlite_runtime::save_turn(&record)
        } else {
            get_turn_store().create_turn(record.clone())
        };
        if let Err(e) = create_turn {
            tracing::error!("Phase A: \u{521b}\u{5efa} TurnRecord \u{5931}\u{8d25}: {e}");
            return Err(TauriCommandError::internal(format!(
                "\u{521b}\u{5efa} TurnRecord \u{5931}\u{8d25}: {e}"
            )));
        }
        Some(record)
    } else {
        None // \u{975e} Campaign \u{6a21}\u{5f0f}\u{ff0c}\u{4e0d}\u{521b}\u{5efa} TurnRecord
    };

    // Operation-owned cancel: pipeline / autofix / postprocess all clone this receiver.
    let (operation_id, cancel_rx) = begin_writing_operation(&app);

    // \u{6bcf}\u{6b21}\u{7528}\u{6700}\u{65b0} tool_ctx \u{5feb}\u{7167}\u{6784}\u{9020} orchestrator\u{ff08}\u{4fdd}\u{8bc1}\u{5bfc}\u{5165}\u{540e}\u{7acb}\u{523b}\u{751f}\u{6548}\u{ff09}
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

    // \u{2500}\u{2500}\u{2500} P2 \u{540e}\u{5904}\u{7406}\u{6d41}\u{6c34}\u{7ebf}\u{ff08}\u{540e}\u{53f0}\u{6267}\u{884c}\u{ff0c}\u{4e0d}\u{963b}\u{65ad}\u{6210}\u{6587}\u{8fd4}\u{56de}\u{ff09}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
    // \u{6210}\u{6587}\u{ff08}DraftReady\u{ff09}\u{540e}\u{8dd1}\u{ff1a}\u{5267}\u{60c5}\u{603b}\u{7ed3} + \u{540e}\u{5904}\u{7406}\u{4e09}\u{5408}\u{4e00}\u{3002}
    // postprocess \u{653e}\u{540e}\u{53f0} spawn\u{2014}\u{2014}draft_ready \u{540e}\u{7acb}\u{5373}\u{8fd4}\u{56de}\u{6210}\u{6587}\u{7ed9}\u{524d}\u{7aef}\u{ff0c}
    // postprocess \u{5728}\u{540e}\u{53f0}\u{8dd1}\u{ff08}\u{77e5}\u{8bc6}/\u{53d8}\u{91cf}/\u{6458}\u{8981}\u{5199}\u{56de}\u{ff09}\u{ff0c}\u{901a}\u{8fc7} event_tx \u{63a8}\u{8fdb}\u{5ea6}\u{3002}
    // \u{4ec5}\u{5728}\u{6709}\u{6d3b}\u{8dc3} Campaign \u{65f6}\u{6267}\u{884c}\u{ff08}\u{65e0} Campaign \u{8df3}\u{8fc7}\u{ff0c}\u{5411}\u{540e}\u{517c}\u{5bb9}\u{ff09}\u{3002}
    //
    // auto-fix \u{53ef}\u{80fd}\u{6539}\u{5199}\u{6b63}\u{6587}\u{ff1a}\u{547d}\u{4ee4}\u{8fd4}\u{56de}\u{503c}\u{5fc5}\u{987b}\u{7528}\u{4fee}\u{590d}\u{540e}\u{7684} final_text\u{ff0c}
    // \u{4e0d}\u{80fd}\u{518d}\u{56de}\u{9000}\u{5230} pipeline \u{539f}\u{59cb} result \u{91cc}\u{7684}\u{539f}\u{7a3f}\u{3002}
    let mut response_text: Option<String> = None;
    if let Ok((final_text, draft_node_id, provenance)) = &result {
        // Phase A: \u{6210}\u{6587}\u{540e}\u{521b}\u{5efa} TurnAttempt \u{5e76}\u{66f4}\u{65b0} TurnRecord \u{2192} DraftReady\u{3002}
        // SQLite opt-in: atomic preaccept UoW (conversation + attempt + outbox).
        // JSON default: pipeline already landed the draft; attach attempt separately.
        // \u{6ce8}\u{610f}\u{ff1a}\u{672c}\u{5730} turn_record \u{5feb}\u{7167}\u{4e0d}\u{542b}\u{65b0} Attempt\u{ff0c}\u{5fc5}\u{987b}\u{6355}\u{83b7} attempt_id \u{7ed9}\u{540e}\u{7eed}\u{5199}\u{56de}\u{3002}
        let mut landed_draft_node_id = draft_node_id.clone();
        let created_attempt_id = if let Some(ref turn) = turn_record {
            let attempt_id = Id::new();
            let temps = pipeline.pending_temporary_instances().to_vec();
            if sqlite_runtime::is_sqlite_active() {
                let campaign_id = ctx.campaign_id.as_ref().ok_or_else(|| {
                    TauriCommandError::internal(
                        "sqlite preaccept draft requires campaign_id".to_string(),
                    )
                })?;
                match sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
                    campaign_id,
                    conversation_id: &conversation_id,
                    turn_id: &turn.turn_id,
                    attempt_id: &attempt_id,
                    draft_text: final_text,
                    pending_temporary_instances: temps,
                    provenance: provenance.clone(),
                }) {
                    Ok(outcome) => {
                        landed_draft_node_id = outcome.variant_id;
                        app.conv_store.invalidate();
                        Some(outcome.attempt_id)
                    }
                    Err(e) => {
                        let _ = update_turn_record(&turn.turn_id, |record| {
                            record.status = storyforge_domain::turn::TurnStatus::Failed;
                            record.failure_reason =
                                Some(format!("sqlite preaccept draft \u{5931}\u{8d25}: {e}"));
                            record.touch();
                        });
                        clear_current_cancel_if(&app, &operation_id);
                        return Err(TauriCommandError::internal(format!(
                            "sqlite preaccept draft \u{5931}\u{8d25}: {e}"
                        )));
                    }
                }
            } else {
                let attempt = turn_lifecycle::new_draft_attempt(
                    attempt_id.clone(),
                    draft_node_id.clone(),
                    final_text,
                    temps,
                );
                // A.1/P0-4\u{ff1a}Attempt \u{521b}\u{5efa}\u{5931}\u{8d25}\u{5fc5}\u{987b}\u{4f20}\u{64ad}\u{ff0c}\u{5e76}\u{8865}\u{507f}\u{8f6f}\u{5220}\u{65e0}\u{4e3b} Draft
                if let Err(e) = update_turn_record(&turn.turn_id, |record| {
                    record.attempts.push(attempt);
                    record.status = storyforge_domain::turn::TurnStatus::DraftReady;
                    record.touch();
                }) {
                    if let Err(comp_e) = app
                        .conv_store
                        .soft_delete_variant(&conversation_id, draft_node_id)
                    {
                        tracing::error!(
                            "P0-4 \u{8865}\u{507f}\u{5931}\u{8d25}: soft_delete \u{65e0}\u{4e3b} Draft {} \u{5931}\u{8d25}: {comp_e}\u{ff08}\u{539f}\u{9519}\u{8bef}: {e}\u{ff09}",
                            draft_node_id
                        );
                    }
                    // Turn \u{6807} Failed\u{ff0c}\u{907f}\u{514d}\u{5c4f}\u{969c}\u{5361}\u{6b7b}\u{540e}\u{7eed}\u{5199}\u{4f5c}
                    let _ = update_turn_record(&turn.turn_id, |record| {
                        record.status = storyforge_domain::turn::TurnStatus::Failed;
                        record.failure_reason = Some(format!(
                            "TurnAttempt \u{6301}\u{4e45}\u{5316}\u{5931}\u{8d25}: {e}"
                        ));
                        record.touch();
                    });
                    clear_current_cancel_if(&app, &operation_id);
                    return Err(TauriCommandError::internal(format!(
                        "TurnAttempt \u{6301}\u{4e45}\u{5316}\u{5931}\u{8d25}\u{ff08}\u{5df2}\u{5c1d}\u{8bd5}\u{8f6f}\u{5220}\u{65e0}\u{4e3b} Draft\u{ff09}: {e}"
                    )));
                }
                Some(attempt_id)
            }
        } else {
            None
        };
        // A.1\u{ff1a}\u{4e34}\u{65f6} instance \u{4e0d}\u{518d}\u{5728} accept \u{524d}\u{76f4}\u{63a5}\u{5199} Campaign\u{ff1b}\u{6302}\u{5728} Attempt\u{ff0c}accept \u{65f6} Mutation \u{843d}\u{76d8}

        // \u{4ece} session.plan \u{53d6}\u{5728}\u{573a}\u{89d2}\u{8272} + \u{57fa}\u{7840}\u{53d8}\u{91cf}\u{952e}
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
        let mvu_fragments = collect_mvu_fallback_fragments_for_backend(&ctx, &present_chars);
        let mvu_rules = collect_mvu_update_rules_for_backend(&ctx, &present_chars);

        // B3/B DraftQualityGate + \u{6709}\u{754c} 1\u{d7} Editor auto-fix
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
        // \u{8fd4}\u{56de}\u{7ed9}\u{524d}\u{7aef}\u{7684}\u{5fc5}\u{987b}\u{662f} auto-fix \u{540e}\u{7684}\u{6b63}\u{6587}
        response_text = Some(final_text.clone());
        // \u{8d28}\u{91cf}\u{62a5}\u{544a}\u{6302}\u{5230}\u{521a}\u{521b}\u{5efa}\u{7684} Attempt\u{ff0c}\u{4fbf}\u{4e8e} accept \u{524d}\u{590d}\u{67e5}\u{ff1b}auto-fix \u{540e}\u{540c}\u{6b65} draft_hash\u{3002}
        // \u{5173}\u{952e}\u{540c}\u{6b65}\u{5931}\u{8d25}\u{5fc5}\u{987b}\u{4f20}\u{64ad}\u{ff1a}\u{5426}\u{5219}\u{547d}\u{4ee4}\u{8fd4}\u{56de}\u{4fee}\u{590d}\u{7a3f}\u{4f46} Attempt \u{4ecd}\u{6307}\u{539f}\u{7a3f} hash\u{ff0c}Accept \u{4f1a}\u{786c}\u{5931}\u{8d25}\u{3002}
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
            let sink = BackendTurnAttemptSink::production();
            // new_json is fine for autofix: it only uses sink, not batch_source.
            let service = production_postprocess::ProductionPostprocessService::new_json(
                get_campaign_store(),
                &sink,
            );
            if let Err(e) = service.sync_autofix_attempt_with_provenance(
                identity,
                &final_text,
                quality_report.clone(),
                autofix_provenance.clone(),
            ) {
                let combined = service_fail_turn(&sink, identity, e);
                clear_current_cancel_if(&app, &operation_id);
                return Err(TauriCommandError::internal(format!(
                    "auto-fix \u{540e} Attempt \u{540c}\u{6b65}\u{5931}\u{8d25}\u{ff08}draft_hash/quality_report\u{ff09}: {combined}"
                )));
            }
            if sqlite_runtime::is_sqlite_active() {
                app.conv_store.invalidate();
            }
        }

        // postprocess \u{540e}\u{53f0}\u{8dd1}\u{ff0c}\u{4e0d}\u{963b}\u{585e} start_writing \u{8fd4}\u{56de}\u{ff1b}\u{4e1a}\u{52a1}\u{72b6}\u{6001}\u{673a}\u{8d70}\u{5171}\u{4eab}\u{670d}\u{52a1}\u{3002}
        // Keep current_cancel alive until the background task finishes so cancel_writing
        // can still reach postprocess after the command returns.
        let pp_event_tx = event_tx.clone();
        let pp_runtime = ctx.campaign_runtime.clone();
        let app_for_pp = app.clone();
        let operation_id_for_pp = operation_id.clone();
        tokio::spawn(async move {
            let result = run_shared_postprocess_background(
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
                let text = prefer_autofix_response_text(response_text, orig_text);
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
                    let _ = update_turn_record(&turn.turn_id, |record| {
                        record.status = storyforge_domain::turn::TurnStatus::Failed;
                        record.failure_reason =
                            Some(format!("\u{5199}\u{4f5c}\u{5931}\u{8d25}: {e}"));
                        record.touch();
                    });
                }
                clear_current_cancel_if(&app, &operation_id);
                return Err(TauriCommandError::from(format!(
                    "\u{5199}\u{4f5c}\u{5931}\u{8d25}: {e}"
                )));
            }
        }
    }

    // No background postprocess path: clear only this operation.
    clear_current_cancel_if(&app, &operation_id);

    match result {
        Ok((orig_text, node_id, _provenance)) => {
            let text = prefer_autofix_response_text(response_text, orig_text);
            Ok(serde_json::json!({
                "text": text,
                "conversation_id": conversation_id.to_string(),
                "node_id": node_id.to_string(),
                "generation_mode": route_decision.mode,
                "generation_route_reason": route_decision.reason,
            }))
        }
        Err(e) => {
            // Phase A: \u{5199}\u{4f5c}\u{5931}\u{8d25} \u{2192} TurnRecord \u{6807} Failed\u{ff08}\u{65e0}\u{526f}\u{4f5c}\u{7528}\u{ff0c}\u{5b89}\u{5168}\u{5931}\u{8d25}\u{ff09}
            if let Some(ref turn) = turn_record {
                let _ = update_turn_record(&turn.turn_id, |record| {
                    record.status = storyforge_domain::turn::TurnStatus::Failed;
                    record.failure_reason = Some(format!("\u{5199}\u{4f5c}\u{5931}\u{8d25}: {e}"));
                    record.touch();
                });
            }
            Err(TauriCommandError::from(format!(
                "\u{5199}\u{4f5c}\u{5931}\u{8d25}: {e}"
            )))
        }
    }
}

/// \u{547d}\u{4ee4}\u{54cd}\u{5e94}\u{4f18}\u{5148}\u{4f7f}\u{7528} auto-fix \u{540e}\u{7684}\u{6b63}\u{6587}\u{ff1b}\u{65e0}\u{4fee}\u{590d}\u{65f6}\u{56de}\u{9000} pipeline \u{539f}\u{7a3f}\u{3002}
pub(crate) fn prefer_autofix_response_text(
    response_text: Option<String>,
    original: String,
) -> String {
    turn_lifecycle::prefer_autofix_response_text(response_text, original)
}

/// auto-fix \u{540e}\u{540c}\u{6b65} Attempt\u{ff1a}quality_report + draft_hash \u{5fc5}\u{987b}\u{5bf9}\u{9f50}\u{6700}\u{7ec8}\u{6b63}\u{6587}\u{3002}
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn sync_attempt_after_autofix(
    attempt: &mut storyforge_domain::turn::TurnAttempt,
    final_text: &str,
    report: storyforge_domain::turn::QualityReport,
) {
    turn_lifecycle::sync_attempt_after_autofix(attempt, final_text, report)
}

/// Routes Attempt/Turn persistence through the active backend (JSON or SQLite).
pub(crate) struct BackendTurnAttemptSink<'a> {
    /// Production uses the process-wide store; tests can inject an isolated store so
    /// backend adapter coverage never writes to the user's real AppData directory.
    json_turn_store: Option<&'a turn_store::TurnStore>,
}

impl<'a> BackendTurnAttemptSink<'a> {
    pub(crate) fn production() -> Self {
        Self {
            json_turn_store: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_json_store(json_turn_store: &'a turn_store::TurnStore) -> Self {
        Self {
            json_turn_store: Some(json_turn_store),
        }
    }

    fn json_store(&self) -> &'a turn_store::TurnStore {
        match self.json_turn_store {
            Some(store) => store,
            None => get_turn_store(),
        }
    }

    fn mutate_json_if<P, M>(&self, turn_id: &Id, predicate: P, mutate: M) -> Result<bool, String>
    where
        P: FnOnce(&storyforge_domain::turn::TurnRecord) -> bool,
        M: FnOnce(&mut storyforge_domain::turn::TurnRecord),
    {
        self.json_store()
            .mutate_if(turn_id, predicate, mutate)
            .map_err(|e| {
                format!("\u{6761}\u{4ef6}\u{66f4}\u{65b0} TurnRecord \u{5931}\u{8d25}: {e}")
            })
    }
}

impl production_postprocess::TurnAttemptSink for BackendTurnAttemptSink<'_> {
    fn load_turn(
        &self,
        turn_id: &Id,
    ) -> Result<Option<storyforge_domain::turn::TurnRecord>, String> {
        if sqlite_runtime::is_sqlite_active() {
            sqlite_runtime::get_turn(turn_id)
        } else {
            Ok(self.json_store().get_turn(turn_id))
        }
    }

    fn sync_autofix(
        &self,
        identity: &production_postprocess::PostprocessIdentity,
        final_text: &str,
        report: storyforge_domain::turn::QualityReport,
    ) -> Result<(), production_postprocess::ProductionPostprocessError> {
        self.sync_autofix_with_provenance(identity, final_text, report, None)
    }

    fn sync_autofix_with_provenance(
        &self,
        identity: &production_postprocess::PostprocessIdentity,
        final_text: &str,
        report: storyforge_domain::turn::QualityReport,
        provenance: Option<Provenance>,
    ) -> Result<(), production_postprocess::ProductionPostprocessError> {
        use production_postprocess::ProductionPostprocessError;
        use storyforge_domain::turn::{AttemptStatus, TurnStatus};

        if sqlite_runtime::is_sqlite_active() {
            // Typed pre-validation against the authoritative SQLite turn before UoW.
            let turn = sqlite_runtime::get_turn(&identity.turn_id)
                .map_err(ProductionPostprocessError::AutofixSync)?
                .ok_or_else(|| ProductionPostprocessError::AttemptMissing {
                    turn_id: identity.turn_id.to_string(),
                    attempt_id: identity.attempt_id.to_string(),
                })?;
            if turn.campaign_id != identity.campaign_id {
                return Err(ProductionPostprocessError::ScopeMismatch {
                    field: "campaign_id",
                    expected: identity.campaign_id.to_string(),
                    actual: turn.campaign_id.to_string(),
                });
            }
            if turn.conversation_id != identity.conversation_id {
                return Err(ProductionPostprocessError::ScopeMismatch {
                    field: "conversation_id",
                    expected: identity.conversation_id.to_string(),
                    actual: turn.conversation_id.to_string(),
                });
            }
            if turn.find_attempt(&identity.attempt_id).is_none() {
                return Err(ProductionPostprocessError::AttemptMissing {
                    turn_id: identity.turn_id.to_string(),
                    attempt_id: identity.attempt_id.to_string(),
                });
            }
            let writable = matches!(
                turn.status,
                TurnStatus::DraftReady | TurnStatus::DerivingState
            ) && turn
                .find_attempt(&identity.attempt_id)
                .is_some_and(|attempt| {
                    matches!(
                        attempt.status,
                        AttemptStatus::DraftReady | AttemptStatus::DerivingState
                    )
                });
            if !writable {
                // Concurrent supersede / late status: durable zero-write, non-fatal.
                return Ok(());
            }
            return sqlite_runtime::sync_autofix(AutofixSyncRequest {
                campaign_id: &identity.campaign_id,
                conversation_id: &identity.conversation_id,
                turn_id: &identity.turn_id,
                attempt_id: &identity.attempt_id,
                final_text,
                quality_report: report,
                provenance: provenance.clone(),
            })
            .map_err(ProductionPostprocessError::AutofixSync);
        }

        // Capture typed validation under the same conditional durable mutation.
        let mut precondition: Option<ProductionPostprocessError> = None;
        let mut not_writable = false;
        let applied = self
            .mutate_json_if(
                &identity.turn_id,
                |record| {
                    if record.campaign_id != identity.campaign_id {
                        precondition = Some(ProductionPostprocessError::ScopeMismatch {
                            field: "campaign_id",
                            expected: identity.campaign_id.to_string(),
                            actual: record.campaign_id.to_string(),
                        });
                        return false;
                    }
                    if record.conversation_id != identity.conversation_id {
                        precondition = Some(ProductionPostprocessError::ScopeMismatch {
                            field: "conversation_id",
                            expected: identity.conversation_id.to_string(),
                            actual: record.conversation_id.to_string(),
                        });
                        return false;
                    }
                    if record.find_attempt(&identity.attempt_id).is_none() {
                        precondition = Some(ProductionPostprocessError::AttemptMissing {
                            turn_id: identity.turn_id.to_string(),
                            attempt_id: identity.attempt_id.to_string(),
                        });
                        return false;
                    }
                    let writable = matches!(
                        record.status,
                        TurnStatus::DraftReady | TurnStatus::DerivingState
                    ) && record.find_attempt(&identity.attempt_id).is_some_and(
                        |attempt| {
                            matches!(
                                attempt.status,
                                AttemptStatus::DraftReady | AttemptStatus::DerivingState
                            )
                        },
                    );
                    if !writable {
                        not_writable = true;
                        return false;
                    }
                    true
                },
                |record| {
                    if let Some(att) = record.find_attempt_mut(&identity.attempt_id) {
                        turn_lifecycle::sync_attempt_after_autofix(att, final_text, report);
                        if let Some(provenance) = provenance {
                            att.provenance = Some(provenance);
                        }
                    }
                    record.touch();
                },
            )
            .map_err(ProductionPostprocessError::AutofixSync)?;
        if applied {
            Ok(())
        } else if let Some(err) = precondition {
            Err(err)
        } else if not_writable {
            // Concurrent supersede / late status: durable zero-write, non-fatal.
            Ok(())
        } else {
            Ok(())
        }
    }

    fn attach_postprocess(
        &self,
        identity: &production_postprocess::PostprocessIdentity,
        batch: Option<storyforge_domain::turn::MutationBatch>,
        derivation: storyforge_domain::turn::DerivationComponents,
    ) -> Result<bool, String> {
        if sqlite_runtime::is_sqlite_active() {
            use storyforge_infra_sqlite::preaccept::PostprocessApplyOutcome;
            return match sqlite_runtime::apply_postprocess(PostprocessApplyRequest {
                campaign_id: &identity.campaign_id,
                conversation_id: &identity.conversation_id,
                turn_id: &identity.turn_id,
                attempt_id: &identity.attempt_id,
                batch,
                derivation,
            })? {
                PostprocessApplyOutcome::Applied | PostprocessApplyOutcome::AlreadyApplied => {
                    Ok(true)
                }
                PostprocessApplyOutcome::SkippedLate => Ok(false),
            };
        }
        self.mutate_json_if(
            &identity.turn_id,
            |record| {
                record.campaign_id == identity.campaign_id
                    && record.conversation_id == identity.conversation_id
                    && is_current_attempt_ready_for_postprocess(record, &identity.attempt_id)
            },
            |record| {
                if let Some(att) = record.find_attempt_mut(&identity.attempt_id) {
                    turn_lifecycle::apply_postprocess_to_attempt(att, batch, derivation);
                }
                record.status = storyforge_domain::turn::TurnStatus::AwaitingAcceptance;
                record.touch();
            },
        )
    }

    fn mark_failed_if_current(
        &self,
        identity: &production_postprocess::PostprocessIdentity,
        reason: String,
    ) -> Result<bool, String> {
        self.mutate_json_if(
            &identity.turn_id,
            |record| {
                record.campaign_id == identity.campaign_id
                    && record.conversation_id == identity.conversation_id
                    && is_current_attempt_ready_for_postprocess(record, &identity.attempt_id)
            },
            |record| {
                record.status = storyforge_domain::turn::TurnStatus::Failed;
                record.failure_reason = Some(reason);
                record.touch();
            },
        )
    }
}

pub(crate) fn normalize_task_update_for_postprocess(
    camp_id: &Id,
    mut task: storyforge_domain::story_task::StoryTask,
    new_status: storyforge_domain::story_task::TaskStatus,
) -> Option<storyforge_domain::story_task::StoryTask> {
    if task.campaign_id != *camp_id {
        tracing::warn!(
            "\u{8df3}\u{8fc7}\u{975e}\u{5f53}\u{524d} Campaign \u{4efb}\u{52a1} '{}' \u{7684}\u{72b6}\u{6001}\u{66f4}\u{65b0}\u{ff08}task campaign: {}, current campaign: {}\u{ff09}",
            task.id,
            task.campaign_id,
            camp_id
        );
        return None;
    }

    task.status = new_status;
    Some(task)
}

/// \u{6309}\u{540d}\u{5b57}\u{6216} Id \u{67e5} campaign \u{5185}\u{7684} CharacterInstance\u{ff08}\u{540e}\u{5904}\u{7406} Agent \u{8f93}\u{51fa}\u{7684}\u{662f}\u{89d2}\u{8272}\u{540d}\u{ff0c}\u{9700}\u{7ffb}\u{8bd1}\u{6210} instance\u{ff09}
/// P3\u{ff1a}\u{5185}\u{90e8}\u{6309} KnowledgeSource \u{5206}\u{6d41}\u{2014}\u{2014}ToldByOther/Backstory \u{4e0d}\u{67e5}\u{5728}\u{573a}\u{76f4}\u{63a5}\u{653e}\u{884c}\u{ff0c}Witnessed/Inferred \u{624d}\u{67e5}\u{5728}\u{573a}\u{3002}
/// P4\u{ff1a}\u{540c}\u{540d}\u{6536}\u{7d27}\u{2014}\u{2014}name_collisions \u{4f20}\u{5165} campaign \u{5185}\u{51fa}\u{73b0} \u{2265}2 \u{6b21}\u{7684} name \u{96c6}\u{5408}\u{ff0c}\u{540c}\u{540d}\u{65f6} name \u{8def}\u{5931}\u{6548}\u{3002}
/// W6 \u{65b9}\u{5411} 1\u{ff1a}broadcast \u{975e}\u{7a7a}\u{65f6}\u{5206}\u{53d1}\u{7ed9}\u{591a}\u{4e2a} target\u{ff08}All=\u{5168}\u{4f53}, Group=\u{8eab}\u{4efd}\u{7ec4}\u{ff09}\u{ff0c}\u{8fd4}\u{56de} Vec\u{3002}
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

/// V2 \u{53d8}\u{4f53}\u{ff1a}`extra_instances` \u{662f} attempt \u{4e0a}\u{5c1a}\u{672a}\u{843d}\u{76d8}\u{7684}\u{4e34}\u{65f6}\u{89d2}\u{8272}\u{ff0c}\u{53c2}\u{4e0e}\u{76ee}\u{6807}/\u{6765}\u{6e90}\u{89e3}\u{6790}\u{3002}
/// accept \u{65f6} `prepare_commit_batch` \u{524d}\u{7f6e} `UpsertInstance`\u{ff0c}\u{6307}\u{5411}\u{5176} id \u{7684}\u{6761}\u{76ee}\u{843d}\u{5e93}\u{5b89}\u{5168}\u{3002}
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
        tracing::warn!(
            "\u{8df3}\u{8fc7} private \u{77e5}\u{8bc6}\u{7684}\u{5e7f}\u{64ad}\u{5199}\u{5165}\u{ff1a}{}",
            update.knowledge_text
        );
        return vec![];
    }

    if update.broadcast.is_some()
        && should_block_source_knowledge_propagation(store, camp_id, update, None)
    {
        return vec![];
    }

    // \u{65b9}\u{5411} 1\u{ff1a}\u{5e7f}\u{64ad}\u{5206}\u{53d1}\u{2014}\u{2014}broadcast \u{975e}\u{7a7a}\u{65f6}\u{904d}\u{5386} instances\u{ff0c}\u{6bcf}\u{4e2a}\u{751f}\u{6210}\u{4e00}\u{6761} ToldByOther
    if let Some(ref broadcast) = update.broadcast {
        return dispatch_broadcast(store, camp_id, update, turn, broadcast, extra_instances);
    }

    // \u{975e}\u{5e7f}\u{64ad}\u{ff1a}\u{5355}\u{89d2}\u{8272}\u{903b}\u{8f91}\u{ff08}\u{539f}\u{6709} P3/P4 \u{6d41}\u{7a0b}\u{ff09}
    let target = match find_instance_by_name_or_id_with_extras(
        store,
        camp_id,
        &update.character_id,
        extra_instances,
    ) {
        Some(inst) => inst,
        None => {
            tracing::warn!(
                "\u{8df3}\u{8fc7}\u{65e0}\u{6cd5}\u{89e3}\u{6790}\u{5230} Campaign instance \u{7684}\u{77e5}\u{8bc6}\u{5199}\u{5165}\u{76ee}\u{6807}: {}",
                update.character_id
            );
            return vec![];
        }
    };

    if should_block_source_knowledge_propagation(store, camp_id, update, Some(&target)) {
        return vec![];
    }

    // P3 \u{5206}\u{6d41}\u{ff1a}ToldByOther/Backstory \u{4e0d}\u{53d7}\u{5728}\u{573a}\u{7ea6}\u{675f}\u{ff08}\u{8de8}\u{5728}\u{573a}\u{544a}\u{77e5} + \u{5f00}\u{5c40}\u{5df2}\u{6709}\u{ff09}\u{ff0c}
    // Witnessed/Inferred \u{624d}\u{67e5}\u{5728}\u{573a}\u{3002}
    let knowledge_exempt_from_presence = matches!(
        update.source,
        storyforge_domain::character_knowledge::KnowledgeSource::ToldByOther
            | storyforge_domain::character_knowledge::KnowledgeSource::Backstory
    );
    if !knowledge_exempt_from_presence {
        // \u{77e5}\u{8bc6}\u{8def}\u{5f84}\u{6536}\u{7d27}\u{ff1a}\u{7a7a}\u{96c6}\u{65f6} Witnessed/Inferred \u{4e5f}\u{62d2}\u{7edd}\u{ff08}\u{65e0}\u{4eba}\u{5728}\u{573a}\u{4e0d}\u{53ef}\u{80fd}\u{89c1}\u{8bc1}/\u{63a8}\u{65ad}\u{ff09}
        // \u{6ce8}\u{610f}\u{ff1a}is_postprocess_instance_present \u{7684}\u{7a7a}\u{96c6}\u{653e}\u{884c}\u{4ecd}\u{670d}\u{52a1}\u{53d8}\u{91cf}\u{8def}\u{5f84}\u{ff0c}\u{6b64}\u{5904}\u{7ed5}\u{8fc7}\u{5b83}\u{3002}
        if present_ids.is_empty()
            || !is_postprocess_instance_present(
                &target,
                &update.character_id,
                present_ids,
                name_collisions,
            )
        {
            tracing::warn!(
                "\u{8df3}\u{8fc7}\u{975e}\u{5728}\u{573a}\u{89d2}\u{8272} '{}' \u{7684}\u{77e5}\u{8bc6}\u{5199}\u{5165}\u{ff08}source={:?}\u{ff0c}present_chars \u{6821}\u{9a8c}\u{ff09}",
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

/// \u{65b9}\u{5411} 1\u{ff1a}\u{5e7f}\u{64ad}\u{5206}\u{53d1}\u{2014}\u{2014}\u{6839}\u{636e} BroadcastTarget \u{904d}\u{5386} campaign \u{5185} instance\u{ff0c}\u{5404}\u{751f}\u{6210}\u{4e00}\u{6761} ToldByOther\u{3002}
///
/// - `All`\u{ff1a}campaign \u{5185}\u{6240}\u{6709} instance\u{ff08}\u{6392}\u{9664}\u{5e7f}\u{64ad}\u{53d1}\u{8d77}\u{8005}\u{81ea}\u{8eab}\u{ff09}
/// - `Group(g)`\u{ff1a}definition.group == g \u{7684} instance\u{ff08}\u{901a}\u{8fc7} definition_id \u{53cd}\u{67e5} CharacterDefinition\u{ff09}
///
/// \u{5e7f}\u{64ad}\u{6761}\u{76ee}\u{7684} source \u{7edf}\u{4e00}\u{4e3a} `ToldByOther`\u{ff0c}source_character_id \u{8bb0}\u{5e7f}\u{64ad}\u{53d1}\u{8d77}\u{8005}\u{ff08}\u{82e5}\u{6709}\u{ff09}\u{3002}
pub(crate) fn dispatch_broadcast(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    update: &storyforge_domain::character_knowledge::CharacterKnowledgeUpdate,
    turn: u32,
    broadcast: &storyforge_domain::character_knowledge::BroadcastTarget,
    extra_instances: &[storyforge_domain::campaign::CharacterInstance],
) -> Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry> {
    use storyforge_domain::character_knowledge::BroadcastTarget;

    // \u{89e3}\u{6790}\u{5e7f}\u{64ad}\u{53d1}\u{8d77}\u{8005}\u{ff08}source_character_id\u{ff09}\u{7684} persisted id\u{ff0c}\u{7528}\u{4e8e}\u{6392}\u{9664}\u{81ea}\u{8eab} + \u{8bb0}\u{5f55}\u{6765}\u{6e90}
    let broadcaster_inst = update.source_character_id.as_ref().and_then(|sid| {
        find_instance_by_name_or_id_with_extras(store, camp_id, sid, extra_instances)
    });
    let broadcaster_id = broadcaster_inst.as_ref().map(|i| i.id.clone());
    let source_character_id = broadcaster_id.clone();
    let source_knowledge_id =
        matching_source_knowledge_for_update(store, camp_id, update).map(|entry| entry.id);

    // V2: \u{672a}\u{843d}\u{76d8}\u{4e34}\u{65f6}\u{89d2}\u{8272}\u{4e5f}\u{5728}\u{5e7f}\u{64ad}\u{53d7}\u{4f17}\u{5185}\u{ff08}\u{672c}\u{8f6e}\u{5b83}\u{4eec}\u{5df2}\u{662f} campaign \u{6210}\u{5458}\u{ff0c}accept \u{65f6}\u{843d}\u{5e93}\u{ff09}
    let mut all_instances = store.list_instances(camp_id);
    for temp in extra_instances {
        if temp.campaign_id == *camp_id && !all_instances.iter().any(|i| i.id == temp.id) {
            all_instances.push(temp.clone());
        }
    }

    let targets: Vec<_> = match broadcast {
        BroadcastTarget::All => all_instances
            .into_iter()
            // \u{6392}\u{9664}\u{5e7f}\u{64ad}\u{53d1}\u{8d77}\u{8005}\u{81ea}\u{8eab}\u{ff08}\u{4e0d}\u{5e94}\u{8be5}\u{7ed9}\u{81ea}\u{5df1}\u{53d1}\u{5e7f}\u{64ad}\u{77e5}\u{8bc6}\u{ff09}
            .filter(|inst| Some(&inst.id) != broadcaster_id.as_ref())
            .collect(),
        BroadcastTarget::Group(group) => all_instances
            .into_iter()
            .filter(|inst| {
                // \u{6392}\u{9664}\u{5e7f}\u{64ad}\u{53d1}\u{8d77}\u{8005}\u{81ea}\u{8eab}
                if Some(&inst.id) == broadcaster_id.as_ref() {
                    return false;
                }
                // \u{901a}\u{8fc7} definition_id \u{53cd}\u{67e5} definition.group
                instance_matches_group(store, inst, group)
            })
            .collect(),
    };

    if targets.is_empty() {
        tracing::warn!(
            "\u{5e7f}\u{64ad}\u{5206}\u{53d1}: broadcast={:?} \u{65e0}\u{5339}\u{914d} instance\u{ff08}campaign={}\u{ff09}",
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
                // \u{5e7f}\u{64ad}\u{7edf}\u{4e00}\u{4e3a} ToldByOther\u{ff08}\u{88ab}\u{544a}\u{77e5}/\u{516c}\u{544a}\u{ff09}
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

pub(crate) fn matching_source_knowledge_for_update(
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

pub(crate) fn should_block_source_knowledge_propagation(
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
                "\u{963b}\u{6b62}\u{77e5}\u{8bc6}\u{4f20}\u{64ad}\u{ff1a}source={} policy={:?} text={}",
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

pub(crate) fn normalize_knowledge_text(text: &str) -> String {
    text.split_whitespace().collect::<String>().to_lowercase()
}

/// \u{5224}\u{65ad} instance \u{7684} CharacterDefinition.group \u{662f}\u{5426}\u{5339}\u{914d}\u{76ee}\u{6807}\u{7ec4}\u{540d}\u{3002}
/// \u{901a}\u{8fc7} instance.definition_id \u{2192} \u{904d}\u{5386}\u{6240}\u{6709} card \u{7684} character_definitions \u{627e}\u{5339}\u{914d}\u{3002}
pub(crate) fn instance_matches_group(
    store: &campaign_store::CampaignStore,
    inst: &storyforge_domain::campaign::CharacterInstance,
    target_group: &str,
) -> bool {
    let def_id = match &inst.definition_id {
        Some(id) => id,
        None => return false, // \u{65e0} definition_id\u{ff08}\u{4e34}\u{65f6}\u{89d2}\u{8272}\u{ff09}\u{2192} \u{4e0d}\u{5339}\u{914d}
    };
    // \u{904d}\u{5386}\u{6240}\u{6709} card\u{ff0c}\u{627e} definition_id \u{5339}\u{914d}\u{7684} definition
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
            "postprocess \u{5199}\u{56de}: present_chars \u{4e3a}\u{7a7a}\u{96c6}\u{ff0c}\u{653e}\u{884c} '{}'\u{ff08}\u{5411}\u{540e}\u{517c}\u{5bb9}\u{9003}\u{751f}\u{53e3}\u{ff0c}\u{53d8}\u{91cf}\u{8def}\u{5f84}\u{4ecd}\u{4f9d}\u{8d56}\u{ff09}",
            inst.name
        );
        return true;
    }
    // id \u{8def}\u{ff1a}\u{53ea}\u{8ba4} instance id\u{ff08}\u{6216} raw_id \u{672c}\u{8eab}\u{5c31}\u{662f}\u{8be5} instance id\u{ff09}\u{3002}
    // \u{4e0d}\u{80fd}\u{628a} raw_id \u{7684}\u{4efb}\u{610f}\u{5b57}\u{7b26}\u{4e32}\u{547d}\u{4e2d} present \u{90fd}\u{5f53} id \u{8def}\u{2014}\u{2014}\u{540c}\u{540d}\u{78b0}\u{649e}\u{65f6} Agent \u{5e38}\u{7ed9}\u{89d2}\u{8272}\u{540d}\u{ff0c}
    // \u{82e5}\u{628a} name \u{5f53}\u{6210} id \u{547d}\u{4e2d}\u{ff0c}\u{4f1a}\u{7ed5}\u{8fc7}\u{4e0b}\u{9762}\u{7684} name_collisions \u{6536}\u{7d27}\u{5e76}\u{9759}\u{9ed8}\u{5199}\u{5230}\u{7b2c}\u{4e00}\u{4e2a}\u{540c}\u{540d}\u{76ee}\u{6807}\u{3002}
    if present_ids.contains(inst.id.as_str())
        || (raw_id == &inst.id && present_ids.contains(raw_id.as_str()))
    {
        return true;
    }
    // name \u{8def}\u{515c}\u{5e95}\u{ff1a}P4 \u{540c}\u{540d}\u{6536}\u{7d27}\u{2014}\u{2014}campaign \u{5185}\u{5b58}\u{5728}\u{540c}\u{540d} instance \u{65f6} name \u{8def}\u{5931}\u{6548}\u{ff0c}\u{903c} id
    if present_ids.contains(&inst.name) && !name_collisions.contains(&inst.name) {
        tracing::debug!(
            "postprocess \u{5199}\u{56de}: '{}' \u{901a}\u{8fc7} name \u{5339}\u{914d}\u{5728}\u{573a}\u{ff08}\u{975e} id \u{5339}\u{914d}\u{ff09}",
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

/// V2 \u{53d8}\u{4f53}\u{ff1a}\u{89e3}\u{6790}\u{57df} = \u{5df2}\u{6301}\u{4e45}\u{5316} instance + attempt \u{4e0a}\u{672a}\u{843d}\u{76d8}\u{7684}\u{4e34}\u{65f6}\u{5b9e}\u{4f8b}\u{ff08}\u{6309} id \u{53bb}\u{91cd}\u{ff09}\u{3002}
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
    // \u{5148}\u{7cbe}\u{786e} id \u{5339}\u{914d}
    if let Some(i) = instances.iter().find(|i| i.id == *name_or_id) {
        return Some(i.clone());
    }
    // \u{518d}\u{6309} instance.name \u{5339}\u{914d}\u{ff08}\u{540e}\u{5904}\u{7406} Agent \u{7ed9}\u{7684}\u{662f}\u{89d2}\u{8272}\u{540d}\u{ff09}
    instances
        .into_iter()
        .find(|i| i.name.as_str() == name_or_id.as_str())
}

/// Tauri command: \u{53d6}\u{6d88}\u{5f53}\u{524d}\u{8fd0}\u{884c}\u{7684}\u{5199}\u{4f5c}\u{6d41}\u{6c34}\u{7ebf}
///
/// \u{89e6}\u{53d1} AppState.current_cancel \u{7684} sender\u{ff0c}\u{5bfc}\u{6f14}/\u{5b50}Agent/\u{7f16}\u{5267}\u{5168}\u{90e8}\u{4e2d}\u{6b62}\u{3002}
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

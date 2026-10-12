use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};

use storyforge_app_agent::runtime::{PromptHook, PromptHookContext};
use storyforge_app_pipeline::{PipelineOrchestrator, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::conversation::Provenance;
use storyforge_domain::llm::ChatMessage;

use crate::campaign_store;
use crate::error::TauriCommandError;
use crate::production_postprocess;
use crate::{PromptHookPendingGuard, PromptHookPendingMap};

/// 写作流水线事件（Tauri Channel 用）
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

/// Tauri command: 启动写作流水线（通过 Channel 推送事件）
///
/// cancel sender 存进 AppState.current_cancel，前端可调 cancel_writing 中止。
/// 返回 { text, conversation_id, node_id } 供前端后续重 roll 定位。
#[derive(Debug, Clone, Deserialize)]
pub struct PromptHookReply {
    #[serde(default)]
    pub messages: Option<Vec<ChatMessage>>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn resolve_prompt_hook_pending(
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

pub fn frontend_prompt_hook(
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
pub struct QualityAutofixCtx<'a> {
    pub pipeline: &'a mut PipelineOrchestrator,
    pub draft_node_id: &'a Id,
    pub conversation_id: &'a Id,
    pub writing_ctx: &'a WritingContext,
    pub event_tx: &'a tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
    pub cancel: watch::Receiver<bool>,
    pub log_prefix: &'a str,
    pub original_provenance: Option<Provenance>,
}

/// 阶段 B：有界 1× Editor auto-fix。
///
/// 对草稿跑 NarrativeContract QualityGate；若有 Error 且尚未 auto-fix，
/// 仅 Editor 重跑一次（hint 来自警告摘要），再 gate。最多 1 次。
pub async fn quality_gate_with_optional_editor_autofix(
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
pub struct RouteActorSignal {
    pub id: String,
    pub name: String,
    pub agenda: Option<String>,
    pub private_facts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RouteTaskSignal {
    pub related_actor_ids: Vec<String>,
    pub imminent: bool,
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

pub fn generation_route_signals_from_parts(
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

pub fn generation_route_signals(
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

pub fn enforce_generation_cost_confirmation(
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

/// 回滚并发败者的孤儿 user 消息（Gate 8 审查 P2-B1）。
///
/// 仅当 `input_node_id` 仍是对话的最后一个节点时才截断删除——胜者若已在
/// 其后追加节点，保守保留（避免误删胜者输入），孤儿消息交由用户手动清理。
pub fn rollback_orphaned_user_message(
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

pub fn normalize_task_update_for_postprocess(
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

    // R12-R3：与 production_postprocess.rs 的 SQLite 门禁对称——三处 fail-open
    //（源缺失/源解析不出/源无同文本条目）是有意放行，但此前完全静默；这里补上
    // 同款 warn（reason 字符串保持一致，便于跨路径聚合计数），不改变放行语义。
    let Some(source_id) = &update.source_character_id else {
        tracing::warn!(
            target: "knowledge_propagation",
            reason = "source_character_id missing",
            text = %update.knowledge_text,
            "知识传播门禁未能判定源策略（JSON 直写路径），按放行处理（W-11 下游 fail-open）"
        );
        return false;
    };
    let Some(source) = find_instance_by_name_or_id(store, camp_id, source_id) else {
        tracing::warn!(
            target: "knowledge_propagation",
            reason = "source character unresolved",
            source = %source_id.as_str(),
            text = %update.knowledge_text,
            "知识传播门禁未能判定源策略（JSON 直写路径），按放行处理（W-11 下游 fail-open）"
        );
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

    tracing::warn!(
        target: "knowledge_propagation",
        reason = "source knowledge entry not found (policy unknown)",
        source = %source.name,
        text = %update.knowledge_text,
        "知识传播门禁未能判定源策略（JSON 直写路径），按放行处理（W-11 下游 fail-open）"
    );

    false
}

pub fn knowledge_text_matches(restricted: &str, candidate: &str) -> bool {
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

pub fn find_instance_by_name_or_id(
    store: &campaign_store::CampaignStore,
    camp_id: &Id,
    name_or_id: &Id,
) -> Option<storyforge_domain::campaign::CharacterInstance> {
    find_instance_by_name_or_id_with_extras(store, camp_id, name_or_id, &[])
}

/// V2 变体：解析域 = 已持久化 instance + attempt 上未落盘的临时实例（按 id 去重）。
pub fn find_instance_by_name_or_id_with_extras(
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

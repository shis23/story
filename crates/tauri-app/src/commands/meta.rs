use super::super::*;

// ─── Meta Agent 命令 ──────────────────────────────────────────────────────

/// Meta 对话流式事件（Tauri Channel 用）
///
/// 复用 WritingEvent 的扁平模式。目前只有 token 增量一种事件；
/// 命令的最终聚合结果（agent_message/messages/new_patch）仍由命令返回值携带。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaStreamEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

impl MetaStreamEvent {
    /// token 增量事件
    pub fn progress(delta: String) -> Self {
        Self {
            event_type: "meta_progress".into(),
            data: serde_json::json!({ "delta": delta }),
        }
    }
}

/// 接受并执行 Patch（修改世界书条目或角色字段）
#[tauri::command]
pub(crate) fn meta_accept_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    meta_backend::ensure_json_meta_backend_supported(
        sqlite_runtime::is_sqlite_active(),
        "legacy Meta patch accept",
    )
    .map_err(TauriCommandError::validation)?;
    // P0-7 residual：legacy 世界书 patch 与 typed Meta 一样，活动 Turn 期间禁止直接写
    check_turn_barrier(state.inner())?;

    // 从 PatchStore 取出 patch
    let patch = {
        let patches = state.meta_patches.read().unwrap_or_else(|p| p.into_inner());
        patches
            .iter()
            .find(|p| p.id == patch_id)
            .cloned()
            .ok_or_else(|| TauriCommandError::not_found(format!("Patch 不存在: {patch_id}")))?
    };

    // 执行 patch：clone 世界书 → 修改 → 写回
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        if let Some(ref world_info) = ctx.world_info {
            let mut entries_json: Vec<serde_json::Value> = world_info
                .entries
                .iter()
                .map(|e| to_json_value(e, "world info entry"))
                .collect::<Result<_, _>>()?;

            let mut patch_ctx = storyforge_app_meta::PatchContext {
                world_info_entries: Some(&mut entries_json),
                character_fields: None,
            };

            storyforge_app_meta::execute_patch(&patch, &mut patch_ctx)
                .map_err(|e| TauriCommandError::internal(e.to_string()))?;

            // 反序列化回 WorldInfoEntry 并替换
            let new_entries: Vec<storyforge_domain::world_info::WorldInfoEntry> = entries_json
                .into_iter()
                .map(|v| {
                    serde_json::from_value(v.clone()).map_err(|e| {
                        TauriCommandError::internal(format!(
                            "Patch entry failed to deserialize as WorldInfoEntry: {e}, value: {v}"
                        ))
                    })
                })
                .collect::<Result<_, _>>()?;

            let mut new_book = (**world_info).clone();
            new_book.entries = new_entries;
            ctx.world_info = Some(Arc::new(new_book));
        }
    }

    // 游玩态：世界书真相源是 Campaign 书，不写回角色卡模板。
    // 无活跃活动时保留库维护路径（写回 CharacterStore 合并视图分流）。
    let active_campaign_id = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if let Some(campaign_id) = active_campaign_id {
        if !sqlite_runtime::is_sqlite_active() {
            let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
            if let Some(ref world_info) = ctx.world_info {
                if let Err(e) =
                    get_campaign_store().set_world_info(&campaign_id, (**world_info).clone())
                {
                    tracing::warn!(
                        "meta_accept_patch 写回活动世界书失败 campaign={}: {e}",
                        campaign_id
                    );
                } else {
                    tracing::info!(
                        "meta_accept_patch 已写回本局世界书 campaign={} entries={}",
                        campaign_id,
                        world_info.entries.len()
                    );
                }
            }
        }
    } else {
        // 持久化到 CharacterStore（同步 world_info_entries）——仅库维护 / 无活动
        // 从 tool_ctx 取最新的世界书，按 is_global 分流回写：
        //   - 全局条目：写回所有角色卡（跨卡共享语义）
        //   - 非全局条目：只保留在各卡原有的非全局条目里
        //
        // 历史 bug：曾用 `all_stored.last()` 把整个合并视图（全局+多卡 merge）
        // 全部写回最后一张卡，并把 is_global 硬编码 false，导致数据污染与全局标记丢失。
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        if let Some(ref world_info) = ctx.world_info {
            let global_entries: Vec<crate::WorldInfoEntryInfo> = world_info
                .entries
                .iter()
                .filter(|e| {
                    matches!(
                        e.route,
                        storyforge_domain::world_info::LoreRoute::Constant
                            | storyforge_domain::world_info::LoreRoute::Both
                    )
                })
                .map(|e| crate::WorldInfoEntryInfo {
                    keys: e.keys.clone(),
                    content: e.content.clone(),
                    constant: e.constant,
                    route: format!("{:?}", e.route),
                    is_global: true,
                    depth: e.depth,
                    order: e.order,
                })
                .collect();
            let global_keys_set: std::collections::HashSet<String> =
                global_entries.iter().map(|e| e.keys.join(",")).collect();

            let all_stored = get_store().list();
            for stored in &all_stored {
                let preserved_private: Vec<crate::WorldInfoEntryInfo> = stored
                    .info
                    .world_info_entries
                    .iter()
                    .filter(|e| !e.is_global && !global_keys_set.contains(&e.keys.join(",")))
                    .cloned()
                    .collect();
                let mut new_entries = preserved_private;
                new_entries.extend(global_entries.clone());
                let _ = get_store().update_world_info_entries_bulk(&stored.id, new_entries);
            }
        }
    }

    // 标记为已执行
    if let Some(p) = state
        .meta_patches
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .iter_mut()
        .find(|p| p.id == patch_id)
    {
        p.applied = true;
    }

    Ok(())
}

// ─── P3：Meta Agent 多轮对话 / MVU 五合一分析 / ST 预设 LLM 分类 ────────────

/// GenerationExplainer 的 tauri-app 实现：从 ConversationStore 查 Provenance
/// 并调 `explain_generation`，让 Meta 多轮对话的 inspect_generation 工具
/// 能引用真实生成溯源（而非 app-meta 层的 None 占位）。
///
/// 实现 app-meta 的 GenerationExplainer trait，注入到 MetaSession。
/// 持有 Arc<ConversationStore>（与 AppState.conv_store 共享同一份）。
/// 查不到对话/节点/变体/溯源时返回 None，由工具层转成错误信息，不 panic。
pub(crate) struct ConvGenerationExplainer {
    pub(crate) conv_store: Arc<storyforge_app_conversation::ConversationStore>,
}

impl storyforge_app_meta::meta_conversation::GenerationExplainer for ConvGenerationExplainer {
    fn explain(
        &self,
        conversation_id: String,
        node_id: String,
    ) -> storyforge_app_meta::meta_conversation::GenerationExplainFuture {
        let conv_store = self.conv_store.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || {
                explain_generation_from_conversation_store(conv_store, conversation_id, node_id)
                    .map(storyforge_app_meta::GenerationExplanation::without_reasoning)
            })
            .await
            {
                Ok(explanation) => explanation,
                Err(e) => {
                    tracing::warn!("解释生成溯源任务失败: {e}");
                    None
                }
            }
        })
    }
}

pub(crate) fn explain_generation_from_conversation_store(
    conv_store: Arc<ConversationStore>,
    conversation_id: String,
    node_id: String,
) -> Option<storyforge_app_meta::GenerationExplanation> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    let conv = conv_store.get(&conv_id)?;
    let node = conv.find_node(&nid)?;
    let variant = node.active()?;
    let provenance = variant.provenance.as_ref()?;
    Some(storyforge_app_meta::explain_generation(provenance))
}

/// 把当前活跃角色卡 + 世界书同步进 MetaSession（每次 meta 操作前调）
pub(crate) fn sync_meta_session_from_tool_ctx(state: &tauri::State<'_, Arc<AppState>>) {
    let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
    if let Some(card) = ctx.characters.last() {
        state.meta_session.set_character(card.clone());
    }
    if let Some(book) = &ctx.world_info {
        state.meta_session.set_world_info(book.clone());
    }
    // 同步 campaign runtime 快照
    if let Some(rt) = &ctx.campaign_runtime {
        state.meta_session.set_campaign_runtime(rt.clone());
    } else {
        // 无 active campaign 时清空，避免读到过期快照
        *state
            .meta_session
            .campaign_runtime
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }
}

/// Tauri command: 开始一个新的 Meta 对话（返回 conversation_id）
#[tauri::command]
pub(crate) fn meta_start_conversation(state: tauri::State<'_, Arc<AppState>>) -> String {
    sync_meta_session_from_tool_ctx(&state);
    let conv = storyforge_app_meta::MetaConversation::new();
    let id = conv.id.clone();
    state
        .meta_conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(id.clone(), conv);
    id
}

/// Tauri command: 跑一轮 Meta 对话（返回本轮 Agent 回复 + 新 patch）
#[tauri::command]
pub(crate) async fn meta_chat(
    conversation_id: String,
    user_input: String,
    state: tauri::State<'_, Arc<AppState>>,
    on_event: tauri::ipc::Channel<MetaStreamEvent>,
) -> Result<serde_json::Value, TauriCommandError> {
    let app = state.inner().clone();
    let llm = app.require_active_llm()?;
    sync_meta_session_from_tool_ctx(&state);

    // 取出对话；不存在则返回错误（而非静默创建空对话，避免用户感觉"历史突然清空"）。
    // 新对话应由 meta_start_conversation 命令显式建立。
    let mut conv = {
        let mut convs = app
            .meta_conversations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        convs.remove(&conversation_id).ok_or_else(|| {
            format!("Meta 对话不存在: {conversation_id}（请先调用 meta_start_conversation 创建）")
        })?
    };

    // 构造 AgentRuntime（只允许真实活跃连接）
    let tool_ctx = app.snapshot_tool_ctx();
    let runtime = storyforge_app_agent::AgentRuntime::new(llm, tool_ctx);

    // 流式转发：meta_chat 内部把 token delta 推到 progress_tx，
    // 一个转发任务把它包成 MetaStreamEvent 推给前端 Channel
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(delta) = progress_rx.recv().await {
            let _ = on_event_clone.send(MetaStreamEvent::progress(delta));
        }
    });

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let turn = storyforge_app_meta::meta_chat(
        &runtime,
        &mut conv,
        app.meta_session.clone(),
        &user_input,
        cancel_rx,
        progress_tx,
    )
    .await
    .map_err(|e| TauriCommandError::internal(e.to_string()))?;

    // 把新提议的 patch 同步进 AppState.meta_patches（前端可用 meta_accept_patch 采纳）
    if let Some(patch) = &turn.new_patch {
        let mut patches = app.meta_patches.write().unwrap_or_else(|p| p.into_inner());
        if !patches.iter().any(|p| p.id == patch.id) {
            patches.push(patch.clone());
        }
    }

    // 把新提议的 typed patch 同步进 AppState.typed_patches
    if !turn.new_typed_patches.is_empty() {
        let mut typed = app.typed_patches.write().unwrap_or_else(|p| p.into_inner());
        for tp in &turn.new_typed_patches {
            if !typed.iter().any(|p| p.id == tp.id) {
                typed.push(tp.clone());
            }
        }
    }

    // 存回对话
    let conv_id = conv.id.clone();
    let messages = to_json_value(&conv.messages, "meta conversation messages")?;
    app.meta_conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(conv_id.clone(), conv);

    Ok(serde_json::json!({
        "conversation_id": conv_id,
        "agent_message": turn.agent_message,
        "messages": messages,
        "new_patch": turn.new_patch,
        "new_typed_patches": turn.new_typed_patches,
    }))
}

/// Tauri command: 获取某个 Meta 对话的完整消息历史
#[tauri::command]
pub(crate) fn meta_get_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Option<serde_json::Value>, TauriCommandError> {
    let convs = state
        .meta_conversations
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    convs
        .get(&conversation_id)
        .map(|conv| to_json_value(conv, "meta conversation"))
        .transpose()
}

/// Tauri command: 列所有待采纳的 Meta Patch
#[tauri::command]
pub(crate) fn meta_list_pending_patches(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    state
        .meta_patches
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter(|p| !p.applied)
        .map(|p| to_json_value(p, "pending meta patch"))
        .collect()
}

/// Tauri command: 忽略一个 Meta Patch（从 pending 移除）
#[tauri::command]
pub(crate) fn meta_dismiss_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let mut patches = state
        .meta_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    patches.retain(|p| p.id != patch_id);
    Ok(())
}

/// Tauri command: Campaign 健康检查（确定性数据校验，零 LLM）
///
/// 扫描指定 Campaign 的数据，找出孤立 instance、未解析的知识引用、孤儿任务引用、
/// 变量 schema 不一致等问题。返回问题列表，空列表 = 健康。
#[tauri::command]
pub(crate) fn meta_health_check(
    campaign_id: String,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let cid = Id::from_str(&campaign_id);

    if sqlite_runtime::is_sqlite_active() {
        return meta_backend::sqlite_campaign_health_issues(&cid)
            .map_err(TauriCommandError::storage)?
            .into_iter()
            .map(|issue| to_json_value(&issue, "campaign health issue"))
            .collect();
    }

    let store = get_campaign_store();

    // 确认 campaign 存在
    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;

    // 获取关联的 card → definitions
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();

    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let snapshot = storyforge_app_meta::CampaignHealthSnapshot {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
    };

    let issues = storyforge_app_meta::check_campaign_health(&snapshot);

    issues
        .into_iter()
        .map(|i| to_json_value(&i, "campaign health issue"))
        .collect()
}

/// Tauri command: 解释某条消息的生成溯源（确定性，零 LLM）
///
/// 从指定对话节点的 active variant 的 Provenance 提取可读解释。
/// 返回 `GenerationExplanation` 结构体，前端可直接渲染。
#[tauri::command]
pub(crate) fn meta_explain_generation(
    conversation_id: String,
    node_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);

    let conv = state
        .conv_store
        .get(&conv_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("对话不存在: {conversation_id}")))?;

    let node = conv
        .find_node(&nid)
        .ok_or_else(|| TauriCommandError::not_found(format!("消息节点不存在: {node_id}")))?;

    let variant = node.active().ok_or("该节点无可用变体")?;

    let provenance = variant
        .provenance
        .as_ref()
        .ok_or("该消息没有生成溯源信息（可能是用户手动输入）")?;

    let explanation = storyforge_app_meta::explain_generation(provenance);

    to_json_value(&explanation, "generation explanation")
}

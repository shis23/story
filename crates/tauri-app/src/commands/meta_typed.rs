use super::super::*;
use crate::storage_backend::BackendCapability;

// ─── 类型化 Patch 命令（第三轮：campaign-runtime 修复闭环）───────────────────

/// 从 CampaignStore 组装 PreviewInput（类型化 patch 纯函数所需的快照）
pub(crate) fn build_preview_input<'a>(
    _store: &campaign_store::CampaignStore,
    campaign: &'a storyforge_domain::campaign::Campaign,
    instances: &'a [storyforge_domain::campaign::CharacterInstance],
    definitions: &'a Vec<storyforge_domain::character::CharacterDefinition>,
    knowledge: &'a [storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
    tasks: &'a [storyforge_domain::story_task::StoryTask],
) -> storyforge_app_meta::PreviewInput<'a> {
    storyforge_app_meta::PreviewInput {
        instances,
        definitions,
        knowledge,
        tasks,
        campaign: Some(campaign),
    }
}

/// 对 Campaign 做健康检查并生成类型化修复建议
#[tauri::command]
pub(crate) fn meta_propose_campaign_repairs(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    state
        .storage()
        .require_supported(
            BackendCapability::TypedMetaPatch,
            "campaign repair proposals",
        )
        .map_err(TauriCommandError::validation)?;
    let store = state.json_campaign_store(
        BackendCapability::TypedMetaPatch,
        "campaign repair proposals",
    )?;
    meta_propose_campaign_repairs_in_store(store, &campaign_id, state.inner().as_ref())
}

pub(crate) fn meta_propose_campaign_repairs_in_store(
    store: &campaign_store::CampaignStore,
    campaign_id: &str,
    state: &AppState,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let cid = Id::from_str(campaign_id);
    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;

    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();

    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let input = build_preview_input(
        store,
        &campaign,
        &instances,
        &definitions,
        &knowledge,
        &tasks,
    );

    let issues =
        storyforge_app_meta::check_campaign_health(&storyforge_app_meta::CampaignHealthSnapshot {
            instances: &instances,
            definitions: &definitions,
            knowledge: &knowledge,
            tasks: &tasks,
        });

    let mut patches: Vec<storyforge_app_meta::TypedPatch> = Vec::new();
    for issue in &issues {
        if let Some(patch) = storyforge_app_meta::build_patch_for_issue(issue, &input) {
            patches.push(patch);
        }
    }

    let visible_patches = {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        let mut visible = Vec::new();
        for patch in patches {
            if let Some(existing) = typed
                .iter_mut()
                .find(|existing| is_same_pending_typed_patch(existing, &patch))
            {
                existing.description = patch.description.clone();
                existing.diff = patch.diff.clone();
                visible.push(existing.clone());
            } else {
                visible.push(patch.clone());
                typed.push(patch);
            }
        }
        visible
    };

    visible_patches
        .into_iter()
        .map(|p| to_json_value(&p, "typed patch proposal"))
        .collect()
}

pub(crate) fn is_same_pending_typed_patch(
    existing: &storyforge_app_meta::TypedPatch,
    proposed: &storyforge_app_meta::TypedPatch,
) -> bool {
    existing.status == storyforge_app_meta::TypedPatchStatus::Pending
        && existing.source_issue_category == proposed.source_issue_category
        && existing.affected_id == proposed.affected_id
        && canonical_typed_patch_actions_json(&existing.actions)
            == canonical_typed_patch_actions_json(&proposed.actions)
}

pub(crate) fn canonical_typed_patch_actions_json(
    actions: &[storyforge_app_meta::TypedPatchAction],
) -> Option<serde_json::Value> {
    use storyforge_app_meta::TypedPatchAction;

    let mut canonical = actions.to_vec();
    for action in &mut canonical {
        match action {
            TypedPatchAction::SyncInstanceVariables {
                add_keys,
                remove_keys,
                ..
            } => {
                add_keys.sort();
                remove_keys.sort();
            }
            TypedPatchAction::PruneOrphanTaskReferences {
                orphan_character_ids,
                ..
            } => orphan_character_ids.sort_by(|a, b| a.as_str().cmp(b.as_str())),
            _ => {}
        }
    }
    serde_json::to_value(canonical).ok()
}

/// 列出所有 Pending 状态的类型化 patch
#[tauri::command]
pub(crate) fn meta_list_typed_patches(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    typed
        .iter()
        .filter(|p| p.status == storyforge_app_meta::TypedPatchStatus::Pending)
        .map(|p| to_json_value(p, "typed patch"))
        .collect()
}

/// 预览一条类型化 patch：检查是否过期，返回 diff
#[tauri::command]
pub(crate) fn meta_preview_typed_patch(
    patch_id: String,
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    state
        .storage()
        .require_supported(
            BackendCapability::TypedMetaPatch,
            "typed Meta patch preview",
        )
        .map_err(TauriCommandError::validation)?;
    let store = state.json_campaign_store(
        BackendCapability::TypedMetaPatch,
        "typed Meta patch preview",
    )?;
    meta_preview_typed_patch_in_store(store, &patch_id, &campaign_id, state.inner().as_ref())
}

pub(crate) fn meta_preview_typed_patch_in_store(
    store: &campaign_store::CampaignStore,
    patch_id: &str,
    campaign_id: &str,
    state: &AppState,
) -> Result<serde_json::Value, TauriCommandError> {
    let cid = Id::from_str(campaign_id);
    // 找到 patch
    let mut typed = state
        .typed_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    let patch = typed
        .iter_mut()
        .find(|p| p.id == patch_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("类型化 Patch 不存在: {patch_id}")))?;

    // 取当前 campaign 快照
    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();
    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let input = storyforge_app_meta::PreviewInput {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
        campaign: Some(&campaign),
    };

    // Preview 与 Accept 共用同一前置条件纯函数（Gate 2 Batch 2.4）：
    // target 缺失或 definition/schema 前置条件不满足时都标 Stale，确保 Preview 看到的
    // 可接受性与 Accept 的判定一致（不再出现「Preview 通过但 Accept 失败」）。
    if storyforge_app_meta::validate_patch_preconditions(patch, &input).is_err() {
        patch.status = storyforge_app_meta::TypedPatchStatus::Stale;
        let patch_json = to_json_value(&*patch, "typed patch preview")?;
        return Ok(serde_json::json!({
            "stale": true,
            "patch": patch_json,
        }));
    }

    let patch_json = to_json_value(&*patch, "typed patch preview")?;
    let diff_json = to_json_value(&patch.diff, "typed patch diff")?;
    Ok(serde_json::json!({
        "stale": false,
        "patch": patch_json,
        "diff": diff_json,
    }))
}

/// 接受一条类型化 patch：纯函数预演 → 写盘
#[tauri::command]
pub(crate) fn meta_accept_typed_patch(
    patch_id: String,
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .storage()
        .require_supported(BackendCapability::TypedMetaPatch, "typed Meta patch accept")
        .map_err(TauriCommandError::validation)?;
    // Phase A 屏障：活动 Turn 存在时拒绝 Meta patch accept（防并发写竞争）
    let cid = Id::from_str(&campaign_id);
    reject_if_active_turn(state.storage(), &cid)?;
    let store =
        state.json_campaign_store(BackendCapability::TypedMetaPatch, "typed Meta patch accept")?;
    meta_accept_typed_patch_in_store(store, &patch_id, &campaign_id, state.inner().as_ref())
}

pub(crate) fn meta_accept_typed_patch_in_store(
    store: &campaign_store::CampaignStore,
    patch_id: &str,
    campaign_id: &str,
    state: &AppState,
) -> Result<(), TauriCommandError> {
    let _accept_guard = state
        .typed_patch_accept_lock
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let cid = Id::from_str(campaign_id);
    // 1. 找到 patch，必须 Pending
    let patch = {
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        let p = typed.iter().find(|p| p.id == patch_id).ok_or_else(|| {
            TauriCommandError::not_found(format!("类型化 Patch 不存在: {patch_id}"))
        })?;
        if p.status != storyforge_app_meta::TypedPatchStatus::Pending {
            return Err(TauriCommandError::validation(format!(
                "Patch 状态不是 Pending（当前: {:?}），无法接受",
                p.status
            )));
        }
        p.clone()
    };

    // 2. 取当前 campaign 快照
    let campaign = store
        .get_campaign(&cid)
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();
    let instances = store.list_instances(&cid);
    let knowledge = store.list_knowledge(&cid);
    let tasks = store.list_tasks(&cid);

    let input = storyforge_app_meta::PreviewInput {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
        campaign: Some(&campaign),
    };

    // 3. stale 检查
    if storyforge_app_meta::is_patch_stale(&patch, &input) {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(p) = typed.iter_mut().find(|p| p.id == patch_id) {
            p.status = storyforge_app_meta::TypedPatchStatus::Stale;
        }
        return Err("patch 已过期，target 不存在".into());
    }

    if let Err(e) = validate_typed_patch_targets(
        &patch,
        &campaign,
        &definitions,
        &instances,
        &knowledge,
        &tasks,
    ) {
        mark_typed_patch_stale(state, patch_id);
        return Err(e);
    }

    // 4. 纯函数预演（clone 可变快照）
    {
        // A 的 PreviewInputMut 字段为 &mut Vec<...>，需 clone 到本地变量再借。
        let mut inst_clone = instances.clone();
        let mut def_clone = definitions.clone();
        let mut know_clone = knowledge.clone();
        let mut task_clone = tasks.clone();
        let mut camp_clone = campaign.clone();
        let mut snap = storyforge_app_meta::PreviewInputMut {
            instances: &mut inst_clone,
            definitions: &mut def_clone,
            knowledge: &mut know_clone,
            tasks: &mut task_clone,
            campaign: Some(&mut camp_clone),
            turn: 0, // preview 不持久化，turn 值不影响验证
        };
        storyforge_app_meta::apply_to_snapshot(&patch, &mut snap)
            .map_err(|e| TauriCommandError::pipeline(format!("纯函数预演失败: {e}"), false))?;
    }
    // 5. 真正写盘（P0-6：与 TurnCommit 共享全局提交锁）
    turn_coordinator::with_campaign_lock(|| {
        for (idx, action) in patch.actions.iter().enumerate() {
            let result = apply_typed_action(store, &cid, action);
            if let Err(e) = result {
                // 写盘失败，patch 保持 Pending，报错包含第几个 action
                return Err(turn_coordinator::CommitError::Storage(format!(
                    "第 {} 个 action 失败: {}",
                    idx + 1,
                    e
                )));
            }
        }
        Ok(())
    })
    .map_err(|e| TauriCommandError::storage(e.to_string()))?;

    // 6. 写盘成功，标记 Accepted
    {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(p) = typed.iter_mut().find(|p| p.id == patch_id) {
            p.status = storyforge_app_meta::TypedPatchStatus::Accepted;
        }
    }

    Ok(())
}

pub(crate) fn mark_typed_patch_stale(state: &AppState, patch_id: &str) {
    let mut typed = state
        .typed_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(p) = typed.iter_mut().find(|p| p.id == patch_id) {
        p.status = storyforge_app_meta::TypedPatchStatus::Stale;
    }
}

pub(crate) fn validate_typed_patch_targets(
    patch: &storyforge_app_meta::TypedPatch,
    campaign: &storyforge_domain::campaign::Campaign,
    definitions: &[storyforge_domain::character::CharacterDefinition],
    instances: &[storyforge_domain::campaign::CharacterInstance],
    knowledge: &[storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
    tasks: &[storyforge_domain::story_task::StoryTask],
) -> Result<(), TauriCommandError> {
    // 委托 app-meta 的单一纯函数：target 存在性 + SyncInstanceVariables 的
    // definition/schema 前置条件都只此一份实现（Gate 2 Batch 2.4）。
    let input = storyforge_app_meta::PreviewInput {
        instances,
        definitions,
        knowledge,
        tasks,
        campaign: Some(campaign),
    };
    storyforge_app_meta::validate_patch_preconditions(patch, &input).map_err(|e| match e {
        storyforge_app_meta::TypedPatchError::TargetMissing(msg) => {
            TauriCommandError::not_found(msg)
        }
        storyforge_app_meta::TypedPatchError::PreconditionFailed(msg) => {
            TauriCommandError::validation(msg)
        }
        storyforge_app_meta::TypedPatchError::Stale => {
            TauriCommandError::validation("patch 已过期")
        }
    })
}

/// 执行单个 TypedPatchAction 到 CampaignStore（写盘辅助）
pub(crate) fn apply_typed_action(
    store: &campaign_store::CampaignStore,
    campaign_id: &Id,
    action: &storyforge_app_meta::TypedPatchAction,
) -> Result<(), TauriCommandError> {
    use storyforge_app_meta::TypedPatchAction;

    match action {
        TypedPatchAction::SyncInstanceVariables {
            instance_id,
            definition_id,
            add_keys,
            remove_keys,
        } => {
            let mut instance = store
                .get_instance(campaign_id, instance_id)
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Instance 不存在: {}",
                        instance_id.as_str()
                    ))
                })?;

            // 与 A 的 apply_to_snapshot 纯函数保持一致：从 definition.variable_schema
            // 取 add_keys 的 default 值，而非硬编码 null。否则 accept 前的纯函数预演
            // （显示 schema 默认值）与真正写盘（写 null）结果不一致。
            // 查找路径：campaign -> card_id -> card.character_definitions -> 按 definition_id 匹配
            let schema_defaults: std::collections::HashMap<String, serde_json::Value> = (|| {
                let campaign = store.get_campaign(campaign_id)?;
                let stored = store.get_card(&campaign.card_id)?;
                let def = stored
                    .card
                    .character_definitions
                    .iter()
                    .find(|d| &d.id == definition_id)?;
                Some(
                    def.variable_schema
                        .iter()
                        .map(|f| (f.key.clone(), f.default.clone()))
                        .collect(),
                )
            })(
            )
            .unwrap_or_default();

            // 添加缺失 key（用 schema default，缺失 schema 时回退 null）
            for key in add_keys {
                if instance.get_variable(key).is_none() {
                    let default_val = schema_defaults
                        .get(key)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    instance.set_variable(key, default_val, 0);
                }
            }

            // 删除多余 key
            instance.variables.retain(|v| !remove_keys.contains(&v.key));

            store
                .update_instance(instance)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::PruneOrphanTaskReferences {
            task_id,
            orphan_character_ids,
        } => {
            let mut task = store.get_task(task_id).ok_or_else(|| {
                TauriCommandError::not_found(format!("Task 不存在: {}", task_id.as_str()))
            })?;

            task.related_characters
                .retain(|id| !orphan_character_ids.contains(id));

            store
                .update_task(task)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::DeleteOrphanKnowledge { knowledge_id } => {
            store
                .delete_knowledge(knowledge_id)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
                .then_some(())
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Knowledge 不存在: {}",
                        knowledge_id.as_str()
                    ))
                })?;
            Ok(())
        }
        TypedPatchAction::RepointInstanceDefinition {
            instance_id,
            new_definition_id,
        } => {
            let mut instance = store
                .get_instance(campaign_id, instance_id)
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Instance 不存在: {}",
                        instance_id.as_str()
                    ))
                })?;

            instance.definition_id = new_definition_id.clone();
            store
                .update_instance(instance)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::UpdateCampaignVariable { key, value } => {
            let mut campaign = store.get_campaign(campaign_id).ok_or_else(|| {
                TauriCommandError::not_found(format!("Campaign 不存在: {}", campaign_id.as_str()))
            })?;
            campaign.set_variable(key, value.clone(), 0);
            store
                .update_campaign(campaign)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::UpdateInstanceVariable {
            instance_id,
            key,
            value,
        } => {
            let mut instance = store
                .get_instance(campaign_id, instance_id)
                .ok_or_else(|| {
                    TauriCommandError::not_found(format!(
                        "Instance 不存在: {}",
                        instance_id.as_str()
                    ))
                })?;
            instance.set_variable(key, value.clone(), 0);
            store
                .update_instance(instance)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::AddKnowledge {
            character_id,
            knowledge_text,
            source,
        } => {
            let entry = storyforge_domain::character_knowledge::CharacterKnowledgeEntry {
                id: Id::new(),
                campaign_id: campaign_id.clone(),
                character_id: character_id.clone(),
                knowledge_text: knowledge_text.clone(),
                source: source.clone(),
                source_character_id: None,
                source_knowledge_id: None,
                turn_number: 0,
                event_id: None,
                pinned: false,
                propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
            };
            store
                .add_knowledge(vec![entry])
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
        TypedPatchAction::UpdateTaskStatus {
            task_id,
            new_status,
        } => {
            let mut task = store.get_task(task_id).ok_or_else(|| {
                TauriCommandError::not_found(format!("Task 不存在: {}", task_id.as_str()))
            })?;
            task.status = new_status.clone();
            store
                .update_task(task)
                .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
            Ok(())
        }
    }
}

/// 忽略一条类型化 patch（标记 Dismissed，保留审计痕迹）
#[tauri::command]
pub(crate) fn meta_dismiss_typed_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    meta_dismiss_typed_patch_in_state(&patch_id, state.inner().as_ref())
}

pub(crate) fn meta_dismiss_typed_patch_in_state(
    patch_id: &str,
    state: &AppState,
) -> Result<(), TauriCommandError> {
    let mut typed = state
        .typed_patches
        .write()
        .unwrap_or_else(|p| p.into_inner());
    let patch = typed
        .iter_mut()
        .find(|p| p.id == patch_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("类型化 Patch 不存在: {patch_id}")))?;
    patch.status = storyforge_app_meta::TypedPatchStatus::Dismissed;
    Ok(())
}

/// MVU 翻译的精简 DTO（前端列表用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuTranslationSummaryDto {
    pub source_character_id: String,
    pub character_name: String,
    pub analyzed_at: String,
    pub routing: String,
    pub ui_binding_count: usize,
    pub fallback_count: usize,
    pub analysis_confidence: f64,
}

/// MVU 翻译的完整 DTO（前端渲染状态栏用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuTranslationDetailDto {
    pub source_character_id: String,
    pub character_name: String,
    pub analyzed_at: String,
    pub translation: storyforge_domain::mvu_translation::MvuTranslation,
    pub complexity: serde_json::Value,
}

/// Tauri command: 手动触发 MVU 五合一分析（D44：手动按钮，不自动跑）
///
/// 流程：取角色卡 → 启发式打分 → LLM 五合一分析 → 持久化 → 返回结果
/// 失败降级：分析失败回退到字段级 schema，不报错
#[tauri::command]
pub(crate) async fn meta_analyze_mvu_card(
    source_character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<MvuTranslationDetailDto, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;

    state
        .storage()
        .require_supported(BackendCapability::MvuTranslation, "analyze MVU card")
        .map_err(TauriCommandError::validation)?;

    // #22：分析与持久化两个后端都支持——SQLite 活跃时写 mvu_translations 表。

    // 取原 Character
    let character = {
        let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
        ctx.characters
            .iter()
            .find(|c| c.id.as_str() == source_character_id)
            .map(|c| (*c).clone())
    }
    .ok_or_else(|| TauriCommandError::not_found(format!("找不到角色卡 {source_character_id}")))?;

    // 启发式打分（纯 Rust，先跑，给 LLM 当判据）
    let complexity = storyforge_app_meta::score_card_complexity(&character);
    let complexity_json = to_json_value(&complexity, "MVU complexity")?;
    tracing::info!(
        "卡「{}」MVU 启发式分类: {:?}",
        character.name,
        complexity.classification
    );

    // 跑 LLM 五合一分析
    let llm = state.require_active_llm()?;
    let tool_ctx = state.snapshot_tool_ctx();
    let runtime = AgentRuntime::new(llm, tool_ctx);
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let translation = storyforge_app_meta::analyze_mvu_card(&runtime, &character, cancel_rx)
        .await
        .map_err(|e| TauriCommandError::validation(format!("MVU 分析失败: {e}")))?;

    // 持久化（后端分流：SQLite → mvu_translations 表；否则 JSON CampaignStore）
    let analyzed_at = chrono::Utc::now().to_rfc3339();
    let stored = campaign_store::StoredMvuTranslation {
        source_character_id: character.id.clone(),
        character_name: character.name.clone(),
        translation: translation.clone(),
        analyzed_at: analyzed_at.clone(),
    };
    let storage = state.storage().clone();
    tokio::task::spawn_blocking(move || storage.save_mvu(&stored))
        .await
        .map_err(|e| TauriCommandError::internal(format!("保存 MVU 翻译任务失败: {e}")))?
        .map_err(TauriCommandError::storage)?;

    Ok(MvuTranslationDetailDto {
        source_character_id: character.id.as_str().to_string(),
        character_name: character.name.clone(),
        analyzed_at,
        translation,
        complexity: complexity_json,
    })
}

#[cfg(test)]
pub(crate) async fn save_mvu_translation_async(
    store: Arc<campaign_store::CampaignStore>,
    stored: campaign_store::StoredMvuTranslation,
) -> Result<(), TauriCommandError> {
    tokio::task::spawn_blocking(move || save_mvu_translation_to_store(store.as_ref(), stored))
        .await
        .map_err(|e| TauriCommandError::internal(format!("保存 MVU 翻译任务失败: {e}")))?
}

#[cfg(test)]
pub(crate) fn save_mvu_translation_to_store(
    store: &campaign_store::CampaignStore,
    stored: campaign_store::StoredMvuTranslation,
) -> Result<(), TauriCommandError> {
    store
        .save_mvu(stored)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
}

/// Tauri command: 列所有已分析的 MVU 翻译
///
/// #22：SQLite 活跃时读 mvu_translations 表（V005 起为权威），不再拒绝。
#[tauri::command]
pub(crate) fn meta_list_mvu_translations(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<MvuTranslationSummaryDto>, TauriCommandError> {
    state
        .storage()
        .require_supported(BackendCapability::MvuTranslation, "list MVU translations")
        .map_err(TauriCommandError::validation)?;
    let list = state
        .storage()
        .list_mvu()
        .map_err(TauriCommandError::storage)?;
    Ok(list
        .iter()
        .map(|m| MvuTranslationSummaryDto {
            source_character_id: m.source_character_id.as_str().to_string(),
            character_name: m.character_name.clone(),
            analyzed_at: m.analyzed_at.clone(),
            routing: format!("{:?}", m.translation.routing),
            ui_binding_count: m.translation.ui_bindings.len(),
            fallback_count: m.translation.fallback_fragments.len(),
            analysis_confidence: m.translation.analysis_confidence,
        })
        .collect())
}

/// Tauri command: 查某角色卡的 MVU 翻译详情（前端渲染状态栏用）
///
/// #22：SQLite 活跃时读 mvu_translations 表，不再拒绝。
#[tauri::command]
pub(crate) fn meta_get_mvu_translation(
    source_character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Option<MvuTranslationDetailDto>, TauriCommandError> {
    state
        .storage()
        .require_supported(BackendCapability::MvuTranslation, "get MVU translation")
        .map_err(TauriCommandError::validation)?;
    let id = Id::from_str(source_character_id);
    let found = state
        .storage()
        .get_mvu(&id)
        .map_err(TauriCommandError::storage)?;
    Ok(found.map(|m| MvuTranslationDetailDto {
        source_character_id: m.source_character_id.as_str().to_string(),
        character_name: m.character_name.clone(),
        analyzed_at: m.analyzed_at.clone(),
        translation: m.translation.clone(),
        complexity: serde_json::Value::Null,
    }))
}

/// Tauri command: 预览 MVU schema 合并结果（每个 definition 一条预览）
#[tauri::command]
pub(crate) fn meta_preview_mvu_apply(
    source_character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<MvuApplyPreview>, TauriCommandError> {
    state
        .storage()
        .require_supported(
            BackendCapability::MvuSchemaApply,
            "MVU schema apply preview",
        )
        .map_err(TauriCommandError::validation)?;
    let store = state.json_campaign_store(
        BackendCapability::MvuSchemaApply,
        "MVU schema apply preview",
    )?;
    let id = Id::from_str(&source_character_id);
    let mvu = store.get_mvu(&id).ok_or_else(|| {
        TauriCommandError::not_found(
            MvuApplyError::TranslationNotFound(source_character_id.clone()).to_string(),
        )
    })?;

    // 通过 source_character_id 找到 card
    let stored_card = store.get_card_by_source(&id).ok_or_else(|| {
        TauriCommandError::not_found(format!(
            "找不到 source_character_id={source_character_id} 的 card"
        ))
    })?;

    let previews: Vec<MvuApplyPreview> = stored_card
        .card
        .character_definitions
        .iter()
        .map(|def| {
            compute_apply_preview(
                &def.variable_schema,
                &mvu.translation.variable_schema,
                def.id.as_str(),
                &def.name,
                &source_character_id,
            )
        })
        .collect();

    Ok(previews)
}

/// Tauri command: 把 MVU schema 合并应用到指定 definition（写盘）
///
/// 必须先 compute_apply_preview，无变化则拒绝写盘。
#[tauri::command]
pub(crate) fn meta_apply_mvu_schema(
    source_character_id: String,
    definition_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .storage()
        .require_supported(BackendCapability::MvuSchemaApply, "MVU schema apply")
        .map_err(TauriCommandError::validation)?;
    let store = state.json_campaign_store(BackendCapability::MvuSchemaApply, "MVU schema apply")?;
    meta_apply_mvu_schema_in_store(store, source_character_id, definition_id)
}

pub(crate) fn meta_apply_mvu_schema_in_store(
    store: &campaign_store::CampaignStore,
    source_character_id: String,
    definition_id: String,
) -> Result<(), TauriCommandError> {
    let src_id = Id::from_str(&source_character_id);
    let def_id = Id::from_str(&definition_id);

    let mvu = store.get_mvu(&src_id).ok_or_else(|| {
        TauriCommandError::not_found(
            MvuApplyError::TranslationNotFound(source_character_id.clone()).to_string(),
        )
    })?;

    let stored_card = store.get_card_by_source(&src_id).ok_or_else(|| {
        TauriCommandError::not_found(format!(
            "找不到 source_character_id={source_character_id} 的 card"
        ))
    })?;

    // 找到目标 definition
    let def = stored_card
        .card
        .character_definitions
        .iter()
        .find(|d| d.id == def_id)
        .ok_or_else(|| {
            TauriCommandError::not_found(
                MvuApplyError::DefinitionNotFound(definition_id.clone()).to_string(),
            )
        })?;

    // 先计算预览，确认有变化。存量翻译产物可能带旧记法键（斜杠 / stat_data.
    // 前缀 / <> 占位符），应用边界统一归一（新导入已在解析层归一，此处兜底）。
    let normalized_mvu_schema = storyforge_domain::variables::normalize_schema_keys(
        mvu.translation.variable_schema.clone(),
    );
    let preview = compute_apply_preview(
        &def.variable_schema,
        &normalized_mvu_schema,
        def.id.as_str(),
        &def.name,
        &source_character_id,
    );
    if !preview.has_changes {
        return Err(TauriCommandError::validation(
            MvuApplyError::NoChanges.to_string(),
        ));
    }

    // 写盘：clone card → 改对应 definition → update_card
    let mut card = stored_card.card.clone();
    if let Some(target_def) = card
        .character_definitions
        .iter_mut()
        .find(|d| d.id == def_id)
    {
        apply_schema_to_definition(target_def, preview.merged_schema.clone());
    }
    let updated_card = store
        .update_card(card.clone())
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
    if updated_card.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到要更新的 card: {}",
            card.id
        )));
    }

    // Best-effort：对已存在的 instances 补齐合并后 schema 中仍缺失的变量。
    // 注意：instance 与 definition 的关联是 definition_id，与 campaign 无关——
    // 一张卡的某个 definition 可能被多个 campaign 引用，全部都该 backfill。
    // 因此这里用 list_all_instances() 全量遍历，再按 definition_id 过滤。
    // （历史 bug：曾用 list_instances(&card.id)，把 card.id 当 campaign_id 传，
    // 而 list_instances 按 campaign_id 过滤 → instance.campaign_id 永不等于
    // card.id → loop 体永不执行 → 生产 backfill 是死代码。）
    for instance in store.list_all_instances() {
        if instance.definition_id.as_ref() != Some(&def_id) {
            continue;
        }
        let mut updated = instance.clone();
        let missing_fields: Vec<_> = preview
            .merged_schema
            .iter()
            .filter(|f| !updated.variables.iter().any(|v| v.key == f.key))
            .collect();
        if missing_fields.is_empty() {
            continue;
        }
        for field in missing_fields {
            use storyforge_domain::variables::VariableValue;
            updated.variables.push(VariableValue::new(
                field.key.clone(),
                field.default.clone(),
                0,
            ));
        }
        store
            .update_instance(updated)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
    }

    Ok(())
}

/// Tauri command: 手动触发 ST 预设 LLM 分类（增强现有纯启发式 bridge）
///
/// 失败时返回 Err，前端降级到现有 import_preset_as_modules。
#[tauri::command]
pub(crate) async fn meta_classify_st_preset(
    preset_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    use storyforge_app_agent::AgentRuntime;

    let stored = load_preset_for_classification_async(get_preset_store(), preset_id).await?;

    let llm = state.require_active_llm()?;
    let tool_ctx = state.snapshot_tool_ctx();
    let runtime = AgentRuntime::new(llm, tool_ctx);
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    let classification =
        storyforge_app_meta::classify_st_preset_with_llm(&runtime, &stored.preset, cancel_rx)
            .await
            .map_err(|e| TauriCommandError::validation(format!("ST 分类失败: {e}")))?;

    serde_json::to_value(&classification)
        .map_err(|e| TauriCommandError::internal(format!("序列化失败: {e}")))
}

pub(crate) async fn load_preset_for_classification_async(
    store: &'static PresetStore,
    preset_id: String,
) -> Result<preset_store::StoredPreset, TauriCommandError> {
    tokio::task::spawn_blocking(move || load_preset_for_classification(store, preset_id))
        .await
        .map_err(|e| TauriCommandError::internal(format!("读取预设任务失败: {e}")))?
}

pub(crate) fn load_preset_for_classification(
    store: &PresetStore,
    preset_id: String,
) -> Result<preset_store::StoredPreset, TauriCommandError> {
    store
        .get(&preset_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("预设不存在: {preset_id}")))
}

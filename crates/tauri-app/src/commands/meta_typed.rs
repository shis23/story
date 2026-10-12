use super::super::*;
use crate::storage_backend::BackendCapability;

pub use storyforge_app_service::services::meta_typed::*;

/// 对 Campaign 做健康检查并生成类型化修复建议
#[tauri::command]
pub fn meta_propose_campaign_repairs(
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
    meta_propose_campaign_repairs_for_backend(state.storage(), &campaign_id, state.inner().as_ref())
}

/// 列出所有 Pending 状态的类型化 patch。
///
/// M-03：可按 Campaign 过滤（前端传 `activeCampaign.id`）。不传时保持旧行为，
/// 但 accept/preview 都会拒绝跨战役 patch，写边界不再依赖列表过滤。
#[tauri::command]
pub(crate) fn meta_list_typed_patches(
    campaign_id: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, TauriCommandError> {
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    typed
        .iter()
        .filter(|p| p.status == storyforge_app_meta::TypedPatchStatus::Pending)
        .filter(|p| match campaign_id.as_deref() {
            Some(cid) => p.campaign_id.as_deref() == Some(cid),
            None => true,
        })
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
    let snapshot = load_meta_snapshot(state.storage(), &Id::from_str(&campaign_id))?;
    meta_preview_typed_patch_with_snapshot(
        &patch_id,
        &campaign_id,
        &snapshot,
        state.inner().as_ref(),
    )
}

/// 接受一条类型化 patch：纯函数预演 → 写盘
#[tauri::command]
pub fn meta_accept_typed_patch(
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
    let snapshot = load_meta_snapshot(state.storage(), &cid)?;
    let storage = state.storage().clone();
    meta_accept_typed_patch_with_writer(
        move |campaign_id, actions, expected_revision| {
            crate::backend_workflows::apply_typed_patch_actions_for_backend(
                &storage,
                campaign_id,
                actions,
                expected_revision,
            )
            .map_err(TauriCommandError::storage)
        },
        &patch_id,
        &campaign_id,
        &snapshot,
        state.inner().as_ref(),
    )
}

/// 忽略一条类型化 patch（标记 Dismissed，保留审计痕迹）
#[tauri::command]
pub(crate) fn meta_dismiss_typed_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    meta_dismiss_typed_patch_in_state(&patch_id, state.inner().as_ref())
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
    list.iter()
        .map(|m| -> Result<MvuTranslationSummaryDto, TauriCommandError> {
            Ok(MvuTranslationSummaryDto {
                source_character_id: m.source_character_id.as_str().to_string(),
                character_name: m.character_name.clone(),
                analyzed_at: m.analyzed_at.clone(),
                // M-27d/e：走 serde 序列化（前端按 {kind} 对象解析），失败显式
                // 传播而不是 `unwrap_or(Null)` 静默吞掉。
                routing: serde_json::to_value(&m.translation.routing).map_err(|error| {
                    TauriCommandError::internal(format!("MVU routing 序列化失败: {error}"))
                })?,
                ui_binding_count: m.translation.ui_bindings.len(),
                fallback_count: m.translation.fallback_fragments.len(),
                analysis_confidence: m.translation.analysis_confidence,
            })
        })
        .collect()
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
    let id = Id::from_str(&source_character_id);
    crate::backend_workflows::preview_mvu_apply_for_backend(state.storage(), &id)
        .map_err(TauriCommandError::storage)
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
    let src_id = Id::from_str(&source_character_id);
    let def_id = Id::from_str(&definition_id);
    crate::backend_workflows::apply_mvu_schema_for_backend(state.storage(), &src_id, &def_id)
        .map_err(TauriCommandError::storage)
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

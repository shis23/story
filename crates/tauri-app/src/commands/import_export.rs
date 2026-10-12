use super::super::*;

pub use storyforge_app_service::services::import_export::*;

/// 导出单个角色卡为 ST PNG
///
/// 从 CharacterStore 取原始 Character（含 raw_card_json），
/// 用 to_st_data 构建 StCharacterData，再写入 PNG。
#[tauri::command]
pub(crate) fn export_st_card_png(
    character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<u8>, TauriCommandError> {
    let stored = state
        .storage()
        .get_character(&character_id)
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;

    // 从 CharacterStore 恢复 Character（精简版，但够 to_st_data 用）
    let character = stored_info_to_character(&stored);

    // 构建 ST 数据（用内嵌世界书作为 character_book）
    let book = character
        .embedded_world_info
        .as_ref()
        .map(|b| b.to_st_book());
    let st_data = storyforge_domain::character::to_st_data(&character, None, book);
    let card = storyforge_infra_import::png::make_st_card(st_data, &character.spec_version);

    storyforge_infra_import::png::write_st_card_png(&card, None)
        .map_err(|e| TauriCommandError::internal(format!("PNG 导出失败: {e}")))
}

/// 导出 Campaign 全部角色为 ST PNG + 共享 lorebook
///
/// 策略（用户已定）：每角色一张 PNG + 共享 lorebook。
/// 共享知识/世界书转 ST lorebook 格式。
#[tauri::command]
pub async fn export_campaign_st_cards(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignExportResult, TauriCommandError> {
    // 2026-09-13 域4 修复 T-12：本命令为每个角色实例生成一张 PNG（CPU/IO 密集，
    // 工作量随实例数线性增长），而**非 async** 的 `#[tauri::command]` 由 Tauri 按
    // `ExecutionContext::Blocking` 在 wry IPC handler 内联执行（无线程卸载），
    // 会占住 IPC/事件线程。移到 blocking 池后 UI 事件循环不再被导出阻塞。
    let storage = state.storage().clone();
    tokio::task::spawn_blocking(move || export_campaign_st_cards_impl(&storage, &campaign_id))
        .await
        .map_err(|e| TauriCommandError::internal(format!("Campaign 卡导出任务失败: {e}")))?
}

/// 导出 StoryForge Campaign 完整 JSON Bundle
///
/// 包含 Campaign 元数据 + Instances + Definitions + Knowledge + Tasks + Summaries。
#[tauri::command]
pub(crate) async fn export_campaign_bundle(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    // 2026-09-13 域4 修复 T-12：长 Campaign 的 bundle 序列化可达数十 MB，
    // 非 async 命令会在 IPC 线程内联完成全部序列化 + 磁盘写；移到 blocking 池。
    let app = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let camp_id = Id::from_str(&campaign_id);
        // 消息归档（会话级远记忆）在向量库里，随快照一起导出（不含向量）。
        app.storage()
            .export_campaign_bundle_with(&camp_id, |bundle| {
                if let Some(runtime) = bundle.runtime.as_mut() {
                    runtime.message_archives = crate::vector_index::export_message_archives(
                        &app,
                        &runtime.conversation.id,
                    );
                }
            })
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("Bundle 导出任务失败: {e}")))?
}

/// 导入 StoryForge Campaign JSON Bundle。
///
/// 导入始终生成全新 card/campaign/instance/knowledge/task/summary ID，避免覆盖现有数据。
#[tauri::command]
pub(crate) async fn import_campaign_bundle(
    bundle_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignImportResult, TauriCommandError> {
    // H-2: bound the IPC payload before deserializing (16 MiB). A hijacked plugin
    // iframe or buggy frontend could otherwise OOM the process with a huge string.
    crate::error::require_ipc_size(
        &bundle_json,
        crate::error::MAX_BUNDLE_JSON_BYTES,
        "Campaign Bundle",
    )?;
    // 2026-09-13 域4 修复 T-12：≤16 MiB 的 JSON 解析与后续多表写入都是重 CPU/IO
    // 工作，非 async 命令会在 IPC 线程内联执行；解析与落盘一起移到 blocking 池。
    // 体积校验留在本线程（纯长度检查，让超大载荷快速失败、不进池）。
    let app = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut bundle: CampaignBundle = serde_json::from_str(&bundle_json)
            .map_err(|e| TauriCommandError::validation(format!("Bundle JSON 解析失败: {e}")))?;
        // 消息归档不进存储层：先取出来，导入成功后写进向量库并恢复归档水位。
        let (archives, archived_upto) = bundle
            .runtime
            .as_mut()
            .map(|runtime| {
                (
                    std::mem::take(&mut runtime.message_archives),
                    runtime.conversation.archived_upto,
                )
            })
            .unwrap_or_default();
        let result = app
            .storage()
            .import_campaign_bundle(bundle, app.conv_store.as_ref())?;
        let campaign_id = Id::from_str(&result.campaign_id);
        // 兑现导入提示里的「向量索引将重建」：为新 id 的摘要建远记忆索引
        // （关键词立即可召回；配置了 Embedder 时后台补嵌入）。
        crate::vector_index::index_campaign_summaries(&app, &campaign_id, None);
        crate::vector_index::import_message_archives(
            &app,
            archives,
            archived_upto,
            &campaign_id,
            &Id::from_str(&result.conversation_id),
        );
        Ok(result)
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("Bundle 导入任务失败: {e}")))?
}

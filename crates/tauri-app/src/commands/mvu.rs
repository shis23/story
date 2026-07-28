use super::super::*;

// ─── W8 MVU JS Runtime 命令 ─────────────────────────────────────────────

/// 前端确认 unload 完成
#[tauri::command]
pub(crate) async fn mvu_unload_ack() -> Result<(), TauriCommandError> {
    tracing::debug!("[MVU] unload ack received");
    Ok(())
}

/// 前端确认 load assets 完成
#[tauri::command]
pub(crate) async fn mvu_load_ack(error: Option<String>) -> Result<(), TauriCommandError> {
    if let Some(err) = error {
        tracing::warn!("[MVU] load assets error: {err}");
    } else {
        tracing::debug!("[MVU] load ack received");
    }
    Ok(())
}

/// 前端回传 execute 结果（完成 WebViewMvuRuntime 的 pending oneshot）
#[tauri::command]
pub(crate) async fn mvu_execute_result(
    pending: tauri::State<'_, MvuPendingMap>,
    request_id: String,
    variable_updates: std::collections::HashMap<String, serde_json::Value>,
    side_effects: Vec<String>,
    error: Option<String>,
) -> Result<(), TauriCommandError> {
    let response = MvuExecuteResponse {
        request_id: request_id.clone(),
        variable_updates,
        side_effects,
        error,
    };
    let mut map = pending.lock().await;
    if let Some(tx) = map.remove(&request_id) {
        let _ = tx.send(response);
    } else {
        tracing::warn!("[MVU] mvu_execute_result: unknown request_id {request_id}");
    }
    Ok(())
}

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use storyforge_infra_plugin_host::mvu_runtime::{
    EVENT_MVU_EXECUTE, EVENT_MVU_LOAD_CARD_ASSETS, EVENT_MVU_UNLOAD_CARD, MvuExecResult,
    MvuExecuteResponse, MvuRuntime, MvuRuntimeError,
};
use tauri::Emitter;
use tokio::sync::{Mutex, oneshot};

const DEFAULT_TIMEOUT_MS: u64 = 15_000;

pub type MvuPendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<MvuExecuteResponse>>>>;

pub fn new_mvu_pending_map() -> MvuPendingMap {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Tauri adapter for the pure infra-plugin-host MVU runtime trait.
pub struct WebViewMvuRuntime {
    app_handle: tauri::AppHandle,
    pending: MvuPendingMap,
}

impl WebViewMvuRuntime {
    pub fn with_shared_pending(app_handle: tauri::AppHandle, pending: MvuPendingMap) -> Self {
        Self {
            app_handle,
            pending,
        }
    }
}

#[async_trait]
impl MvuRuntime for WebViewMvuRuntime {
    async fn execute_fragment(
        &self,
        fragment_js: &str,
        current_variables: &HashMap<String, serde_json::Value>,
    ) -> Result<MvuExecResult, MvuRuntimeError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<MvuExecuteResponse>();

        {
            let mut map = self.pending.lock().await;
            map.insert(request_id.clone(), tx);
        }

        let payload = serde_json::json!({
            "request_id": request_id,
            "fragment_js": fragment_js,
            "variables": current_variables,
            "timeout_ms": DEFAULT_TIMEOUT_MS,
        });
        self.app_handle
            .emit(EVENT_MVU_EXECUTE, &payload)
            .map_err(|e| MvuRuntimeError::ChannelError(format!("emit failed: {e}")))?;

        let response = tokio::time::timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS), rx)
            .await
            .map_err(|_| {
                let pending = self.pending.clone();
                let rid = request_id.clone();
                tokio::spawn(async move {
                    pending.lock().await.remove(&rid);
                });
                MvuRuntimeError::Timeout(format!("{DEFAULT_TIMEOUT_MS}ms"))
            })?
            .map_err(|e| MvuRuntimeError::ChannelError(format!("oneshot recv: {e}")))?;

        if let Some(err) = &response.error {
            return Err(MvuRuntimeError::ExecutionFailed(err.clone()));
        }

        Ok(MvuExecResult {
            variable_updates: response.variable_updates,
            side_effects: response.side_effects,
        })
    }

    async fn load_card_assets(
        &self,
        html: Option<&str>,
        css: Option<&str>,
        js: Option<&str>,
    ) -> Result<(), MvuRuntimeError> {
        let payload = serde_json::json!({
            "html": html.unwrap_or(""),
            "css": css.unwrap_or(""),
            "js": js.unwrap_or(""),
        });
        self.app_handle
            .emit(EVENT_MVU_LOAD_CARD_ASSETS, &payload)
            .map_err(|e| MvuRuntimeError::ChannelError(format!("emit failed: {e}")))?;
        Ok(())
    }

    async fn unload_card(&self) -> Result<(), MvuRuntimeError> {
        self.app_handle
            .emit(EVENT_MVU_UNLOAD_CARD, &serde_json::json!({}))
            .map_err(|e| MvuRuntimeError::ChannelError(format!("emit failed: {e}")))?;
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

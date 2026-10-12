use super::super::*;

pub use storyforge_app_service::services::diagnostics::*;

#[tauri::command]
pub(crate) fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub(crate) fn storage_health_report() -> Vec<storage_health::StorageIncident> {
    storage_health::incidents()
}

/// V4：用户确认损坏文件「从空白开始」→ 解除该路径的写栅栏（此后保存合法）。
/// 返回是否确有该路径的冻结/阻断事件。
#[tauri::command]
pub(crate) fn storage_health_acknowledge(path: String) -> bool {
    tracing::warn!("用户确认存储损坏文件从空白开始: {path}");
    storage_health::acknowledge(&path)
}

#[tauri::command]
pub(crate) fn log_query(
    filter: LogFilterDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<LogEntryDto> {
    let log_filter = LogFilter {
        kind: filter.kind.as_deref().and_then(|k| match k {
            "backend" => Some(LogKind::Backend),
            "llm" => Some(LogKind::LlmCall),
            "frontend" => Some(LogKind::FrontendPlugin),
            _ => None,
        }),
        // 前端 Select 可能传 Error / error；统一大小写
        level: filter
            .level
            .as_deref()
            .and_then(|l| match l.to_ascii_lowercase().as_str() {
                "debug" => Some(LogLevel::Debug),
                "info" => Some(LogLevel::Info),
                "warn" => Some(LogLevel::Warn),
                "error" => Some(LogLevel::Error),
                _ => None,
            }),
        keyword: filter.keyword,
        since: None,
        until: None,
        limit: filter.limit,
    };

    state
        .log_store
        .query(&log_filter)
        .into_iter()
        .map(|e| {
            // A2：从 llm_detail 提取 token 字段
            let (prompt_tokens, completion_tokens, cached_tokens) =
                if let Some(detail) = &e.llm_detail {
                    (
                        Some(detail.prompt_tokens),
                        Some(detail.completion_tokens),
                        if detail.cached_tokens > 0 {
                            Some(detail.cached_tokens)
                        } else {
                            None
                        },
                    )
                } else {
                    (None, None, None)
                };
            LogEntryDto {
                id: e.id.to_string(),
                kind: format!("{:?}", e.kind),
                level: format!("{:?}", e.level),
                timestamp: e.timestamp.to_rfc3339(),
                message: e.message,
                prompt_tokens,
                completion_tokens,
                cached_tokens,
            }
        })
        .collect()
}

#[tauri::command]
pub(crate) fn log_get_llm_call(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<LlmCallDetailDto> {
    state
        .log_store
        .get_llm_call(&storyforge_domain::Id::from_str(&id))
        .map(|d| LlmCallDetailDto {
            connection_name: d.connection_name,
            model: d.model,
            prompt_tokens: d.prompt_tokens,
            completion_tokens: d.completion_tokens,
            cached_tokens: d.cached_tokens,
            cache_creation_tokens: d.cache_creation_tokens,
            latency_ms: d.latency_ms,
            error: d.error,
        })
}

/// 清空日志。
///
/// `kind`：`None` / `"all"` = 清空全部；`"backend"` / `"llm"` / `"frontend"` = 只清对应缓冲。
///
/// 2026-09-13 域4 修复 T-07：旧实现把未知 kind 经 `and_then(.. , _ => None)` 静默
/// 降级为 `None`，而 `LogStore::clear(None)` 的语义是**清空全部**
/// （`crates/app-logging/src/lib.rs`）——"参数非法"反而扩大了破坏面，且命令返回
/// `()`、前端拿不到任何提示。现在非法值显式报错，不再静默清库。
/// 前端唯一调用点 `LogPanel.vue` 的取值域是 `all|backend|llm|frontend`，
/// 且已用 try/catch 处理错误，故此为向后兼容的行为收紧。
#[tauri::command]
pub(crate) fn log_clear(
    kind: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let log_kind = match kind.as_deref() {
        None | Some("all") => None,
        Some("backend") => Some(LogKind::Backend),
        Some("llm") => Some(LogKind::LlmCall),
        Some("frontend") => Some(LogKind::FrontendPlugin),
        Some(other) => {
            return Err(TauriCommandError::validation(format!(
                "未知的日志类型 {other:?}（可选 all/backend/llm/frontend）"
            )));
        }
    };
    state.log_store.clear(log_kind);
    Ok(())
}

#[tauri::command]
pub(crate) fn log_export_bundle(
    redact_content: bool,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let opts = ExportOptions {
        redact_content,
        ..Default::default()
    };
    let mut bundle = storyforge_app_logging::export_bundle(&state.log_store, &opts)
        .map_err(TauriCommandError::internal)?;
    if let Some(obj) = bundle.as_object_mut() {
        obj.insert(
            "diagnostic_context".into(),
            diagnostic_context_for_data_dir(&state.data_dir),
        );
    }
    Ok(bundle)
}

/// 前端日志上报（console.log/warn/error 转发到后端 LogStore）
///
/// 单条消息上限 4KB（M-21），防止恶意/异常前端灌爆 LogStore。
#[tauri::command]
pub(crate) fn log_append_frontend(
    level: String,
    message: String,
    state: tauri::State<'_, Arc<AppState>>,
) {
    const MAX_LOG_MSG_LEN: usize = 4096;
    let message = if message.len() > MAX_LOG_MSG_LEN {
        format!("{}...(截断)", &message[..MAX_LOG_MSG_LEN])
    } else {
        message
    };
    let log_level = match level.as_str() {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    };

    state.log_store.push(storyforge_app_logging::LogEntry {
        id: Id::new(),
        kind: LogKind::FrontendPlugin,
        level: log_level,
        timestamp: Utc::now(),
        message,
        fields: std::collections::HashMap::new(),
        llm_detail: None,
    });
}

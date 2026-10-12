use super::super::*;

// ─── M1 日志命令 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntryDto {
    pub id: String,
    pub kind: String,
    pub level: String,
    pub timestamp: String,
    pub message: String,
    /// A2：LLM 调用的 prompt token 数（非 LLM 调用为 None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    /// A2：LLM 调用的 completion token 数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,
    /// A2：缓存命中 token 数（仅当 > 0 时有意义）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogFilterDto {
    pub kind: Option<String>,
    pub level: Option<String>,
    pub keyword: Option<String>,
    pub limit: Option<usize>,
}

/// A2：获取单条 LLM 调用的完整详情（含 request_payload / response_text / token 统计）
#[derive(Debug, Clone, Serialize)]
pub struct LlmCallDetailDto {
    pub connection_name: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    pub cache_creation_tokens: u32,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// AND-3：`storage_meta.json`——记录 schema 版本与 app 版本轨迹。
/// 首次运行写 first_created_*；每次启动更新 last_opened_*。
/// 移动端升级不丢数据的最小可观测基础：升级后能看出「上次是哪个版本写的盘」。
/// 全程 best-effort：读坏/写失败只 warn（AND-3 任务 4：不 panic）。
pub fn touch_storage_meta(data_dir: &Path) {
    let path = data_dir.join("storage_meta.json");
    let now = chrono::Utc::now().to_rfc3339();
    let version = env!("CARGO_PKG_VERSION");
    let mut meta = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| {
            serde_json::json!({
                "schema": 1,
                "first_created_at": now,
                "first_created_version": version,
            })
        });
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("schema".into(), serde_json::json!(1));
        obj.insert("last_opened_at".into(), serde_json::json!(now));
        obj.insert("last_opened_version".into(), serde_json::json!(version));
    }
    if let Err(e) = storyforge_infra_util::atomic_write_json(&path, &meta) {
        tracing::warn!("storage_meta 写入失败（不影响启动）: {e}");
    }
}

pub fn diagnostic_context_for_data_dir(data_dir: &Path) -> serde_json::Value {
    const STORE_FILES: &[&str] = &[
        "connections.json",
        "embed.json",
        "active_campaign.json",
        "characters.json",
        "cards.json",
        "campaigns.json",
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "mvu_translations.json",
        "vectors.json",
        "plugins.json",
        "custom_modules.json",
        "disabled_modules.json",
        "profiles.json",
        "active_profile.json",
        "agent_profile_configs.json",
        "active_agent_profile_config.json",
        "presets.json",
        "active_preset.json",
        "global_regex_scripts.json",
        "storage_meta.json",
    ];

    let log_dir = data_dir.join("logs");
    let conversation_dir = data_dir.join("conversations");
    let store_files: Vec<serde_json::Value> = STORE_FILES
        .iter()
        .map(|name| summarize_file(data_dir.join(name), name))
        .collect();

    serde_json::json!({
        "schema_version": 1,
        "app": {
            "name": "StoryForge",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "paths": {
            "data_dir": data_dir.display().to_string(),
            "log_dir": log_dir.display().to_string(),
            "conversation_dir": conversation_dir.display().to_string(),
        },
        "directories": [
            summarize_dir(&log_dir, "logs"),
            summarize_dir(&conversation_dir, "conversations"),
        ],
        "store_files": store_files,
    })
}

pub fn summarize_file(path: PathBuf, name: &str) -> serde_json::Value {
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => serde_json::json!({
            "name": name,
            "exists": true,
            "bytes": metadata.len(),
            "has_bytes": metadata.len() > 0,
        }),
        Ok(metadata) => serde_json::json!({
            "name": name,
            "exists": true,
            "bytes": metadata.len(),
            "has_bytes": metadata.len() > 0,
            "kind": if metadata.is_dir() { "directory" } else { "other" },
        }),
        Err(_) => serde_json::json!({
            "name": name,
            "exists": false,
            "bytes": 0,
            "has_bytes": false,
        }),
    }
}

pub fn summarize_dir(path: &Path, name: &str) -> serde_json::Value {
    let (exists, entries) = match std::fs::read_dir(path) {
        Ok(read_dir) => (true, read_dir.filter_map(Result::ok).count()),
        Err(_) => (path.exists(), 0),
    };
    serde_json::json!({
        "name": name,
        "exists": exists,
        "entries": entries,
    })
}

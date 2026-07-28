use super::super::*;

// ─── Card Shell manifest + host-mediated fetch ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardShellManifestDto {
    pub character_id: String,
    pub shells: Vec<serde_json::Value>,
    pub remote_urls: Vec<String>,
    pub opening_home_url: Option<String>,
    pub opening_custom_url: Option<String>,
    pub status_bar_url: Option<String>,
    pub tavern_helper_count: usize,
}

#[tauri::command]
pub(crate) fn get_card_shell_manifest(
    character_id: String,
) -> Result<CardShellManifestDto, TauriCommandError> {
    let stored = get_store()
        .get(&character_id)
        .or_else(|| stored_character_for_source_id(&Id::from_str(&character_id)))
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;
    let character = stored_info_to_character(&stored);
    let manifest = storyforge_domain::card_shell::extract_card_shell_manifest(&character);
    // 大 inline TH（创意工坊 60KB+）不整包塞进 manifest，避免 IPC/前端一次反序列化撑爆 WebView。
    // 前端按 label 再调 get_card_shell_inline_js 按需取正文。
    let shells: Vec<serde_json::Value> = manifest
        .shells
        .iter()
        .filter_map(|s| {
            let mut v = serde_json::to_value(s).ok()?;
            if let Some(entry) = v.get_mut("entry")
                && let Some(obj) = entry.get_mut("inline_js")
                && let Some(js) = obj.get("js").and_then(|j| j.as_str())
                && js.len() > 8_192
            {
                let len = js.len();
                if let Some(m) = obj.as_object_mut() {
                    m.insert("js".into(), serde_json::Value::String(String::new()));
                    m.insert("deferred".into(), serde_json::Value::Bool(true));
                    m.insert("byte_len".into(), serde_json::Value::Number(len.into()));
                }
            }
            Some(v)
        })
        .collect();
    Ok(CardShellManifestDto {
        character_id: stored.id.clone(),
        shells,
        remote_urls: manifest.remote_urls.clone(),
        opening_home_url: manifest.opening_home_url().map(|s| s.to_string()),
        opening_custom_url: manifest.opening_custom_url().map(|s| s.to_string()),
        status_bar_url: manifest.status_bar_url().map(|s| s.to_string()),
        tavern_helper_count: manifest.tavern_helper_modules().len(),
    })
}

/// 按需取某条 TH inline JS 正文（manifest 里 deferred 的大脚本）。
#[tauri::command]
pub(crate) fn get_card_shell_inline_js(
    character_id: String,
    label: String,
) -> Result<String, TauriCommandError> {
    let stored = get_store()
        .get(&character_id)
        .or_else(|| stored_character_for_source_id(&Id::from_str(&character_id)))
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;
    let character = stored_info_to_character(&stored);
    let manifest = storyforge_domain::card_shell::extract_card_shell_manifest(&character);
    for s in manifest.tavern_helper_modules() {
        if s.label != label {
            continue;
        }
        if let storyforge_domain::card_shell::CardShellEntry::InlineJs { js } = &s.entry {
            return Ok(js.clone());
        }
    }
    Err(TauriCommandError::not_found(format!(
        "未找到 inline TH: {label}"
    )))
}

#[tauri::command]
pub(crate) fn card_shell_list_allowed_hosts() -> Vec<String> {
    get_card_shell_cache().list_allowed_hosts()
}

/// Register a shell document for the isolated `storyforge-shell` origin and
/// return its opaque token. The frontend builds the iframe URL as
/// `<shell_doc_protocol::SHELL_DOC_ORIGIN>/<token>`. V5 CSP isolation — see
/// shell_doc_protocol.rs.
#[tauri::command]
pub(crate) fn card_shell_register_doc(html: String) -> Result<String, String> {
    shell_doc_protocol::register_shell_doc(html)
}

#[tauri::command]
pub(crate) fn card_shell_register_module(source: String) -> Result<String, String> {
    shell_doc_protocol::register_shell_module(source)
}

#[tauri::command]
pub(crate) fn card_shell_unregister_doc(token: String) -> bool {
    shell_doc_protocol::unregister_shell_doc(&token)
}

#[tauri::command]
pub(crate) fn card_shell_allow_host(host: String) -> Result<(), TauriCommandError> {
    if host.trim().is_empty() {
        return Err(TauriCommandError::validation("host 为空"));
    }
    get_card_shell_cache().allow_host(&host);
    Ok(())
}

/// 清空卡壳磁盘缓存（L6）：未 pin 的远程依赖首取即冻结，需要显式刷新通道。
/// 返回清掉的缓存对象数；下次壳加载会重新拉取全部远程资源。
#[tauri::command]
pub(crate) fn card_shell_clear_cache() -> Result<usize, TauriCommandError> {
    get_card_shell_cache()
        .clear_cache()
        .map_err(TauriCommandError::internal)
}

/// 宿主代持拉取远程壳资源（allowlist + 磁盘缓存）。失败显式返回错误，不降级为空成功。
#[tauri::command]
pub(crate) fn card_shell_fetch_url(
    url: String,
) -> Result<card_shell_cache::ShellFetchResult, TauriCommandError> {
    let cache = get_card_shell_cache();
    let client = cache.build_client().map_err(TauriCommandError::internal)?;
    cache
        .fetch_blocking_with_client(&url, &client)
        .map_err(|e| {
            if e.contains("allowlist") {
                TauriCommandError::validation(e)
            } else {
                TauriCommandError::internal(e)
            }
        })
}

/// Serve large, already-validated card assets from the host cache without
/// moving them through the IPC bridge as base64. The URI path is an opaque
/// generated cache filename, never a caller-provided local filesystem path.
pub(crate) fn card_shell_cache_protocol_response(
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Method, Response, StatusCode, header};

    let with_cors = |builder: tauri::http::response::Builder| {
        builder
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
    };
    if request.method() == Method::OPTIONS {
        return with_cors(Response::builder())
            .status(StatusCode::NO_CONTENT)
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return with_cors(Response::builder())
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(b"method not allowed".to_vec())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    }

    let resource_name = request.uri().path().trim_start_matches('/');
    match get_card_shell_cache().read_protocol_resource(resource_name) {
        Ok((bytes, content_type)) => with_cors(Response::builder())
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "private, max-age=86400")
            .body(if request.method() == Method::HEAD {
                Vec::new()
            } else {
                bytes
            })
            .unwrap_or_else(|_| Response::new(Vec::new())),
        Err(_) => with_cors(Response::builder())
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(b"card-shell cache resource not found".to_vec())
            .unwrap_or_else(|_| Response::new(Vec::new())),
    }
}

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
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardShellManifestDto, TauriCommandError> {
    state
        .storage()
        .require_supported(
            storage_backend::BackendCapability::CharacterCommands,
            "get card shell manifest",
        )
        .map_err(TauriCommandError::validation)?;
    // Gate 4 六审 P1：经 backend-neutral `get_character` 读取（SQLite 下
    // facade 不构造 JSON CharacterStore）。
    let stored = state
        .storage()
        .get_character(&character_id)
        .map_err(TauriCommandError::storage)?
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
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    state
        .storage()
        .require_supported(
            storage_backend::BackendCapability::CharacterCommands,
            "get card shell inline JavaScript",
        )
        .map_err(TauriCommandError::validation)?;
    // Gate 4 六审 P1：经 backend-neutral `get_character` 读取。
    let stored = state
        .storage()
        .get_character(&character_id)
        .map_err(TauriCommandError::storage)?
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

/// Map a shell-doc registry failure to the structured command-error DTO
/// (T-05): size/entry limits are caller-facing validation errors, anything else
/// is an internal failure. Returning a bare `String` here used to be the only
/// exception among the `Result` commands and surfaced as `[object Object]`-class
/// rendering problems in the frontend.
fn shell_doc_error(message: String) -> TauriCommandError {
    if message.contains("exceeds") || message.contains("registry reached") {
        TauriCommandError::validation(message)
    } else {
        TauriCommandError::internal(message)
    }
}

/// Register a shell document for the isolated `storyforge-shell` origin and
/// return its opaque token. The frontend builds the iframe URL as
/// `<shell_doc_protocol::SHELL_DOC_ORIGIN>/<token>`. V5 CSP isolation — see
/// shell_doc_protocol.rs.
#[tauri::command]
pub(crate) fn card_shell_register_doc(html: String) -> Result<String, TauriCommandError> {
    shell_doc_protocol::register_shell_doc(html).map_err(shell_doc_error)
}

#[tauri::command]
pub(crate) fn card_shell_register_module(source: String) -> Result<String, TauriCommandError> {
    shell_doc_protocol::register_shell_module(source).map_err(shell_doc_error)
}

#[tauri::command]
pub(crate) fn card_shell_unregister_doc(token: String) -> bool {
    shell_doc_protocol::unregister_shell_doc(&token)
}

/// 提权命令：把 host 加进卡壳网络 allowlist。
///
/// T-09：前端 wrapper 已被删除（域5），当前**没有任何生产调用点**；按"保留 +
/// 标注"口径与 T-15 保持一致（删除需要同步改 lib.rs 注册表与域4的命令基线
/// 快照，属跨域改动）。
///
/// 已知残余风险（记入 06 fixes 记录）：在 M-01（子帧持有 Tauri IPC）未修复
/// 前，这条命令会放大爆炸半径——壳 iframe 可以自己 allowlist 一个任意远端
/// host，再经 host 代持代理拉取。M-01 落地修复时应与本命令的删除一起评估。
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
///
/// T-03：这条命令是 reqwest blocking 客户端 + 磁盘缓存的同步 IO，过去直接跑在
/// Tauri 的命令线程上，会让前端热路径（壳加载）在等待网络时阻塞 IPC。改为
/// `async` + `spawn_blocking`，把阻塞 IO 挪出命令线程；错误映射与返回结构不变。
#[tauri::command]
pub(crate) async fn card_shell_fetch_url(
    url: String,
) -> Result<card_shell_cache::ShellFetchResult, TauriCommandError> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("card shell fetch task failed: {e}")))?
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

use super::super::*;

// ─── LLM 连接管理命令 ──────────────────────────────────────────────────────

/// 列出内置连接模板
#[tauri::command]
pub(crate) fn list_connection_templates() -> Vec<storyforge_domain::llm::ConnectionTemplate> {
    storyforge_domain::llm::builtin_connection_templates()
}

/// 列出已配置的连接（不含 api_key）
#[tauri::command]
pub(crate) fn list_connections(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<LlmConnectionSummary> {
    let active_id = state.active_conn_id();
    get_conn_store()
        .list()
        .into_iter()
        .map(|stored| LlmConnectionSummary {
            id: stored.id.clone(),
            name: stored.connection.name.clone(),
            base_url: stored.connection.base_url.clone(),
            model: stored.connection.model.clone(),
            protocol: stored.connection.protocol.clone(),
            tool_mode: stored.connection.tool_mode.clone(),
            active: active_id.as_deref() == Some(stored.id.as_str()),
        })
        .collect()
}

/// 查询当前活跃连接（不含 api_key）
#[tauri::command]
pub(crate) fn get_active_connection(
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<LlmConnectionSummary> {
    let active_id = state.active_conn_id()?;
    get_conn_store()
        .get(&active_id)
        .map(|stored| LlmConnectionSummary {
            id: stored.id,
            name: stored.connection.name,
            base_url: stored.connection.base_url,
            model: stored.connection.model,
            protocol: stored.connection.protocol,
            tool_mode: stored.connection.tool_mode,
            active: true,
        })
}

/// 创建连接请求 DTO
#[derive(Debug, Clone, Deserialize)]
pub struct CreateConnectionDto {
    /// 模板 id（可选，从模板创建时自动填默认值）
    #[allow(dead_code)]
    pub template_id: Option<String>,
    pub name: String,
    pub base_url: String,
    /// 协议字符串："openai" / "anthropic" / "gemini" / "custom:xxx"
    pub protocol: String,
    pub model: String,
    pub api_key: String,
    /// "native" / "text_fallback"
    pub tool_mode: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub max_tokens_explicit: bool,
    /// A1：推理模式 "disabled" / "native" / "prompted"。
    /// 默认 prompted（角色化 CoT）。native = 厂商原生 thinking，同时抑制 CoT 提示模块。
    #[serde(default)]
    pub reasoning: Option<String>,
    /// 厂商扩展参数（P3-3），透传到请求体顶层。key=字段名(如 thinking/reasoning_effort),
    /// value=任意 JSON。前端可填如 {"thinking":{"type":"enabled"},"reasoning_effort":"max"}。
    #[serde(default)]
    pub extra: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 创建连接（从模板或自定义）
///
/// 返回新连接的 id。若这是首个连接，自动设为活跃。
#[tauri::command]
pub(crate) async fn create_connection(
    req: CreateConnectionDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    let conn = llm_connection_from_create_dto(req, Id::new())?;

    // 预先验证：构造 client 看是否成功（base_url 格式等）
    // 注意：不实际发请求，只验证能构造出 client
    storyforge_infra_llm::create_client(&conn)
        .map_err(|e| TauriCommandError::llm(format!("连接配置无效: {e}"), false))?;

    create_connection_with_store_async(state.inner().clone(), get_conn_store(), conn).await
}

/// 更新连接请求 DTO。`api_key` 为空字符串时保留原密钥。
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateConnectionDto {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub protocol: String,
    pub model: String,
    /// 空串 = 不改 key；非空 = 覆盖写入 SecretStore
    #[serde(default)]
    pub api_key: String,
    pub tool_mode: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub max_tokens_explicit: bool,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub extra: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 编辑用连接详情（永不返回真实 api_key）。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionEditDetailDto {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    /// 前端表单用小写："openai" / "anthropic" / ...
    pub protocol: String,
    pub tool_mode: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub max_tokens_explicit: bool,
    pub reasoning: String,
    pub extra: Option<serde_json::Map<String, serde_json::Value>>,
    pub has_api_key: bool,
    pub active: bool,
}

/// 获取单条连接详情供编辑（不含 api_key 明文）。
#[tauri::command]
pub(crate) fn get_connection(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ConnectionEditDetailDto, TauriCommandError> {
    let active_id = state.active_conn_id();
    let stored = get_conn_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("连接不存在: {id}")))?;
    Ok(connection_edit_detail_from_stored(
        &stored,
        active_id.as_deref() == Some(stored.id.as_str()),
    ))
}

/// 更新已有连接。`api_key` 为空时保留原密钥。若该连接当前活跃，同步刷新内存 client。
#[tauri::command]
pub(crate) async fn update_connection(
    req: UpdateConnectionDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    update_connection_with_store_async(state.inner().clone(), get_conn_store(), req).await
}

/// 删除连接（若为活跃的，同时清除活跃状态）
#[tauri::command]
pub(crate) async fn delete_connection(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    delete_connection_with_store_async(state.inner().clone(), get_conn_store(), id).await
}

/// 设置活跃连接
#[tauri::command]
pub(crate) async fn set_active_connection(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state.inner().clone().set_active_connection_async(id).await
}

pub(crate) async fn set_active_connection_with_store_async(
    state: Arc<AppState>,
    conn_store: Arc<ConnectionStore>,
    id: String,
) -> Result<(), TauriCommandError> {
    let _guard = state.active_connection_update.lock().await;
    let id_for_io = id.clone();
    let conn = tokio::task::spawn_blocking(move || {
        conn_store
            .set_active(&id_for_io)
            .map_err(TauriCommandError::from)?
            .ok_or_else(|| TauriCommandError::not_found(format!("连接不存在: {id_for_io}")))
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("设置活跃连接任务失败: {e}")))??;

    // Keep in-memory active client aligned with the just-persisted active_id.
    state.apply_active_connection(&id, conn)
}

pub(crate) async fn create_connection_with_store_async(
    state: Arc<AppState>,
    conn_store: Arc<ConnectionStore>,
    conn: LlmConnection,
) -> Result<String, TauriCommandError> {
    let _guard = state.active_connection_update.lock().await;
    let conn_id = conn.id.as_str().to_string();
    let id_for_io = conn_id.clone();
    let maybe_active = tokio::task::spawn_blocking(move || {
        let was_empty = conn_store.list().is_empty();
        conn_store
            .save(conn)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
        if was_empty {
            conn_store
                .set_active(&id_for_io)
                .map_err(TauriCommandError::from)?
                .ok_or_else(|| TauriCommandError::not_found(format!("连接不存在: {id_for_io}")))
                .map(Some)
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("创建连接持久化任务失败: {e}")))??;

    if let Some(active_conn) = maybe_active {
        state.apply_active_connection(&conn_id, active_conn)?;
    }

    Ok(conn_id)
}

pub(crate) async fn update_connection_with_store_async(
    state: Arc<AppState>,
    conn_store: Arc<ConnectionStore>,
    req: UpdateConnectionDto,
) -> Result<(), TauriCommandError> {
    let id = req.id.trim().to_string();
    if id.is_empty() {
        return Err(TauriCommandError::validation("连接 id 不能为空"));
    }
    // api_key 可为空（保留原密钥）；其它字段与 create 相同校验
    let draft = llm_connection_from_update_dto(req)?;
    // 先用「保留/新 key」解析出可构造 client 的运行时配置再落盘
    // 空 key：取现有 resolved key 仅用于 create_client 校验，不写回明文
    let key_for_validate = if draft.api_key.trim().is_empty() {
        conn_store
            .resolved(&id)
            .map_err(TauriCommandError::from)?
            .ok_or_else(|| TauriCommandError::not_found(format!("连接不存在: {id}")))?
            .api_key
    } else {
        draft.api_key.clone()
    };
    let mut validate_conn = draft.clone();
    validate_conn.api_key = key_for_validate;
    storyforge_infra_llm::create_client(&validate_conn)
        .map_err(|e| TauriCommandError::llm(format!("连接配置无效: {e}"), false))?;

    let _guard = state.active_connection_update.lock().await;
    let was_active = state.active_conn_id().as_deref() == Some(id.as_str());
    let id_for_io = id.clone();
    let resolved = tokio::task::spawn_blocking(move || {
        conn_store.update_existing(&id_for_io, draft).map_err(|e| {
            if e.contains("不存在") {
                TauriCommandError::not_found(e)
            } else {
                TauriCommandError::storage(format!("存储写入失败: {e}"))
            }
        })
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("更新连接持久化任务失败: {e}")))??;

    if was_active {
        state.apply_active_connection(&id, resolved)?;
    }
    Ok(())
}

pub(crate) fn llm_connection_from_create_dto(
    req: CreateConnectionDto,
    id: Id,
) -> Result<LlmConnection, TauriCommandError> {
    let protocol = parse_protocol(&req.protocol)?;
    let tool_mode = parse_tool_mode(&req.tool_mode)?;
    Ok(LlmConnection {
        id,
        name: req.name,
        base_url: req.base_url,
        api_key: req.api_key,
        model: req.model,
        protocol,
        params: SamplingParams {
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_tokens,
            max_tokens_explicit: req.max_tokens_explicit,
            reasoning: parse_reasoning_mode(req.reasoning.as_deref()),
            extra: req.extra,
        },
        tool_mode,
    })
}

pub(crate) fn llm_connection_from_update_dto(
    req: UpdateConnectionDto,
) -> Result<LlmConnection, TauriCommandError> {
    let protocol = parse_protocol(&req.protocol)?;
    let tool_mode = parse_tool_mode(&req.tool_mode)?;
    Ok(LlmConnection {
        id: Id::from_str(&req.id),
        name: req.name,
        base_url: req.base_url,
        api_key: req.api_key,
        model: req.model,
        protocol,
        params: SamplingParams {
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_tokens,
            max_tokens_explicit: req.max_tokens_explicit,
            reasoning: parse_reasoning_mode(req.reasoning.as_deref()),
            extra: req.extra,
        },
        tool_mode,
    })
}

pub(crate) fn parse_reasoning_mode(s: Option<&str>) -> storyforge_domain::llm::ReasoningMode {
    match s {
        Some("native") | Some("Native") => storyforge_domain::llm::ReasoningMode::Native,
        Some("prompted") | Some("Prompted") => storyforge_domain::llm::ReasoningMode::Prompted,
        Some("disabled") | Some("Disabled") | Some("off") | Some("none") => {
            storyforge_domain::llm::ReasoningMode::Disabled
        }
        // 与 create 路径一致：缺省 Disabled
        _ => storyforge_domain::llm::ReasoningMode::Disabled,
    }
}

pub(crate) fn protocol_to_form_str(p: &LlmProtocol) -> String {
    match p {
        LlmProtocol::OpenAi => "openai".into(),
        LlmProtocol::Anthropic => "anthropic".into(),
        LlmProtocol::Gemini => "gemini".into(),
        LlmProtocol::Custom(s) => format!("custom:{s}"),
    }
}

pub(crate) fn tool_mode_to_form_str(m: &ToolMode) -> String {
    match m {
        ToolMode::Native => "native".into(),
        ToolMode::TextFallback => "text_fallback".into(),
    }
}

pub(crate) fn reasoning_mode_to_form_str(m: &storyforge_domain::llm::ReasoningMode) -> String {
    match m {
        storyforge_domain::llm::ReasoningMode::Native => "native".into(),
        storyforge_domain::llm::ReasoningMode::Prompted => "prompted".into(),
        storyforge_domain::llm::ReasoningMode::Disabled => "disabled".into(),
    }
}

pub(crate) fn connection_edit_detail_from_stored(
    stored: &connection_store::StoredConnection,
    active: bool,
) -> ConnectionEditDetailDto {
    let c = &stored.connection;
    ConnectionEditDetailDto {
        id: stored.id.clone(),
        name: c.name.clone(),
        base_url: c.base_url.clone(),
        model: c.model.clone(),
        protocol: protocol_to_form_str(&c.protocol),
        tool_mode: tool_mode_to_form_str(&c.tool_mode),
        temperature: c.params.temperature,
        top_p: c.params.top_p,
        max_tokens: c.params.max_tokens,
        max_tokens_explicit: c.params.max_tokens_explicit,
        reasoning: reasoning_mode_to_form_str(&c.params.reasoning),
        extra: c.params.extra.clone(),
        has_api_key: !c.api_key.is_empty(),
        active,
    }
}

pub(crate) async fn delete_connection_with_store_async(
    state: Arc<AppState>,
    conn_store: Arc<ConnectionStore>,
    id: String,
) -> Result<(), TauriCommandError> {
    let _guard = state.active_connection_update.lock().await;
    let was_active = state.active_conn_id().as_deref() == Some(id.as_str());
    let id_for_io = id.clone();
    let deleted = tokio::task::spawn_blocking(move || {
        conn_store
            .delete(&id_for_io)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("删除连接持久化任务失败: {e}")))??;

    if !deleted {
        return Err(TauriCommandError::not_found(format!("连接不存在: {id}")));
    }
    if was_active {
        state.clear_active_connection();
    }
    Ok(())
}

/// 测试连接请求 DTO（用临时配置测试，不持久化）
#[derive(Debug, Clone, Deserialize)]
pub struct TestConnectionDto {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub protocol: String,
    pub tool_mode: String,
    /// 与正式连接一致；Prompted/Native 测试会验证 reasoning 确实可捕获。
    #[serde(default)]
    pub reasoning: Option<String>,
}

/// 测试连接结果
#[derive(Debug, Clone, Serialize)]
pub struct TestConnectionResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: Option<u64>,
}

/// 测试连接连通性（发一个最小 ping 请求）
#[tauri::command]
pub(crate) async fn test_connection(
    req: TestConnectionDto,
) -> Result<TestConnectionResult, TauriCommandError> {
    let tool_mode = parse_tool_mode(&req.tool_mode)?;
    let protocol = parse_protocol(&req.protocol)?;
    let conn = LlmConnection {
        id: Id::new(),
        name: "test".into(),
        base_url: req.base_url,
        api_key: req.api_key,
        model: req.model,
        protocol,
        params: SamplingParams {
            max_tokens: Some(256),
            max_tokens_explicit: true,
            reasoning: req
                .reasoning
                .as_deref()
                .map(|mode| match mode {
                    "native" | "Native" => storyforge_domain::llm::ReasoningMode::Native,
                    "disabled" | "Disabled" | "off" | "none" => {
                        storyforge_domain::llm::ReasoningMode::Disabled
                    }
                    _ => storyforge_domain::llm::ReasoningMode::Disabled,
                })
                .unwrap_or(storyforge_domain::llm::ReasoningMode::Disabled),
            ..Default::default()
        },
        tool_mode,
    };

    let client = storyforge_infra_llm::create_client(&conn)
        .map_err(|e| TauriCommandError::llm(format!("构造客户端失败: {e}"), false))?;

    let start = std::time::Instant::now();
    let chat_req = storyforge_domain::llm::ChatRequest {
        messages: vec![storyforge_domain::llm::ChatMessage::user("ping")],
        tools: None,
        params: conn.params.clone(),
        model: conn.model.clone(),
    };

    match client.chat(&chat_req).await {
        Ok(_resp) => {
            let latency = start.elapsed().as_millis() as u64;
            Ok(TestConnectionResult {
                success: true,
                message: format!("连通成功（模型: {}）", conn.model),
                latency_ms: Some(latency),
            })
        }
        Err(e) => Ok(TestConnectionResult {
            success: false,
            message: format!("{e}"),
            latency_ms: Some(start.elapsed().as_millis() as u64),
        }),
    }
}

/// 拉取服务商可用模型列表（GET /v1/models）
///
/// 用临时连接配置调用，不持久化。失败时返回空数组（前端走模板兜底）。
#[tauri::command]
pub(crate) async fn list_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<String>, TauriCommandError> {
    let conn = LlmConnection {
        id: Id::new(),
        name: "models-probe".into(),
        base_url,
        api_key,
        model: String::new(),
        protocol: LlmProtocol::OpenAi,
        params: SamplingParams::default(),
        tool_mode: ToolMode::Native,
    };
    match storyforge_infra_llm::fetch_models(&conn).await {
        Ok(models) => Ok(models),
        Err(e) => {
            tracing::warn!("拉取模型列表失败，回退空列表: {e}");
            Ok(vec![])
        }
    }
}

/// 把协议字符串解析为 LlmProtocol
pub(crate) fn parse_protocol(s: &str) -> Result<LlmProtocol, TauriCommandError> {
    match s {
        "openai" => Ok(LlmProtocol::OpenAi),
        "anthropic" => Ok(LlmProtocol::Anthropic),
        "gemini" => Ok(LlmProtocol::Gemini),
        s if s.starts_with("custom:") => Ok(LlmProtocol::Custom(s[7..].to_string())),
        other => Err(TauriCommandError::validation(format!("未知协议: {other}"))),
    }
}

/// 把 tool_mode 字符串解析为 ToolMode
pub(crate) fn parse_tool_mode(s: &str) -> Result<ToolMode, TauriCommandError> {
    match s {
        "native" => Ok(ToolMode::Native),
        "text_fallback" => Ok(ToolMode::TextFallback),
        other => Err(TauriCommandError::validation(format!(
            "未知工具模式: {other}"
        ))),
    }
}

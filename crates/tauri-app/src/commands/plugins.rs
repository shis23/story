use super::super::*;

pub use storyforge_app_service::services::plugins::*;

#[tauri::command]
pub(crate) fn list_plugins(state: tauri::State<'_, Arc<AppState>>) -> Vec<InstalledPluginDto> {
    state
        .plugin_registry
        .list()
        .iter()
        .map(plugin_to_dto)
        .collect()
}

#[tauri::command]
pub(crate) fn install_plugin(
    manifest_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let manifest: storyforge_infra_plugin_host::PluginManifest =
        serde_json::from_str(&manifest_json)
            .map_err(|e| TauriCommandError::validation(format!("manifest 解析失败: {e}")))?;
    state
        .plugin_registry
        .install(manifest)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

#[tauri::command]
pub(crate) fn uninstall_plugin(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .plugin_registry
        .uninstall(&id)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

#[tauri::command]
pub(crate) fn set_plugin_enabled(
    id: String,
    enabled: bool,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .plugin_registry
        .set_enabled(&id, enabled)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

// ─── M4 插件 API 命令（带权限二次校验）──────────────────────────────────────

#[tauri::command]
pub(crate) fn plugin_list_characters(
    plugin_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<CharacterSummary>, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::ReadCharacters)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    // Gate 4 六审 P1：经 backend-neutral facade 读取角色库（SQLite 下
    // facade 不构造 JSON CharacterStore）。
    Ok(state
        .storage()
        .list_characters()
        .map_err(|e| TauriCommandError::storage(format!("角色库读取失败: {e}")))?
        .into_iter()
        .map(CharacterSummary::from)
        .collect())
}

#[tauri::command]
pub(crate) fn plugin_read_character(
    plugin_id: String,
    character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInfo, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::ReadCharacters)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    // Gate 4 六审 P1：经 backend-neutral facade 读取角色。
    state
        .storage()
        .get_character(&character_id)
        .map_err(|e| TauriCommandError::storage(format!("角色读取失败: {e}")))?
        .map(|s| s.info)
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))
}

#[tauri::command]
pub(crate) fn plugin_read_world_info(
    plugin_id: String,
    character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CharacterInfo, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::ReadWorldInfo)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    // Gate 4 六审 P1：经 backend-neutral facade 读取角色（含世界书）。
    state
        .storage()
        .get_character(&character_id)
        .map_err(|e| TauriCommandError::storage(format!("角色读取失败: {e}")))?
        .map(|s| s.info)
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))
}

#[tauri::command]
pub(crate) fn plugin_get_variable(
    plugin_id: String,
    campaign_id: String,
    instance_id: String,
    key: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    // 读=读、写=写：仅持 WriteVariables 的插件不得读取实例全部变量（Gate 8
    // 审查 P2-C1——「任一权限即放行 + 忽略 key 返回全部」使拆分形同虚设）。
    ensure_plugin_any_permission(
        &state.plugin_registry,
        &plugin_id,
        &[Permission::ReadVariables],
    )?;
    state
        .storage()
        .require_supported(
            storage_backend::BackendCapability::VariableRead,
            "plugin get variable",
        )
        .map_err(TauriCommandError::validation)?;
    let variables = state
        .storage()
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map_err(TauriCommandError::storage)?
        .map(|i| i.variables)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到实例 {instance_id}")))?;
    // M-31：`key` 过去被完全忽略（形参名 `_key`），"按 key 取单值"的调用会拿到
    // 实例全部变量。按声明语义过滤（未给 key 时保持原行为）。
    Ok(
        match key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            Some(key) => variables
                .into_iter()
                .filter(|variable| variable.key == key)
                .collect(),
            None => variables,
        },
    )
}

#[tauri::command]
pub(crate) fn plugin_set_variable(
    plugin_id: String,
    campaign_id: String,
    instance_id: String,
    key: String,
    value: serde_json::Value,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::WriteVariables)
        .map_err(|e| TauriCommandError::internal(e.to_string()))?;
    // 三.5：活动 Turn 屏障 + 读改写 + 写盘在同一原子单元内完成
    // （旧实现先 `reject_if_active_turn` 再单独写盘，存在 TOCTOU 窗口）。
    let campaign_id = Id::from_str(&campaign_id);
    let instance_id = Id::from_str(&instance_id);
    let applied = state
        .storage()
        .mutate_idle_instance(&campaign_id, &instance_id, |inst| {
            inst.set_variable(&key, value.clone(), 0);
            Ok(())
        })
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到实例 {instance_id}"
        )));
    }
    Ok(())
}

/// 结算一条 prompt hook 结果（`GENERATE_BEFORE_COMBINE_PROMPTS` /
/// `CHAT_COMPLETION_PROMPT_READY` 的最终 messages 级 hook）。
///
/// M-08：改写真·最终 LLM messages 是提权操作。旧实现只凭 `request_id` 就接受
/// 替换，唯一门禁是前端 `canModifyPrompt`（**不是**安全边界）。现在后端复核：
/// 提交 messages 的调用必须声明是哪些插件做的改写（`plugin_id` 或
/// `modifier_plugin_ids`），且每个声明插件都必须在 PluginRegistry 中启用并持
/// `ModifyPrompt`；任一条不满足即丢弃 messages（保留原始提示词），同时仍然结算
/// pending，避免后端 8s 超时等待。
///
/// 前端契约（task-12 域）：`pluginPromptHookResult(requestId, messages, error,
/// pluginId, modifierPluginIds)`；未声明改写者时功能安全降级为「不改写」。
///
/// 残余：request_id 是广播给宿主的，后端没有把 request_id 与签发时的插件集合
/// 绑定（需改 `commands/writing.rs` 的 pending 表结构），因此持 ModifyPrompt 的
/// 插件 A 仍可抢占插件 B 的 request_id——这是下一步（见 06 fixes 记录）。
#[tauri::command]
pub(crate) async fn plugin_prompt_hook_result(
    request_id: String,
    messages: Option<Vec<ChatMessage>>,
    error: Option<String>,
    plugin_id: Option<String>,
    modifier_plugin_ids: Option<Vec<String>>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let mut messages = messages;
    if messages.is_some()
        && let Some(reason) = prompt_mutation_denial(
            state.plugin_registry.as_ref(),
            plugin_id.as_deref(),
            modifier_plugin_ids.as_deref(),
        )
    {
        tracing::warn!(
            "plugin_prompt_hook_result: refusing prompt mutation for {request_id}: {reason}"
        );
        messages = None;
    }

    if !resolve_prompt_hook_pending(&state.prompt_hook_pending, &request_id, messages, error) {
        tracing::warn!("plugin_prompt_hook_result: unknown request_id {request_id}");
    }
    Ok(())
}
/// 插件通道读取对话（CARD-SHELL-REVIEW L4，严格门禁）：与 `get_conversation`
/// 同一数据，但要求后端 PluginRegistry 中该插件已启用且声明 ReadMemory。
/// fail closed：未注册的插件（含卡壳虚拟插件的临时 id）一律拒绝——前端权限
/// 数组不再是唯一边界。
#[tauri::command]
pub(crate) fn plugin_get_conversation(
    plugin_id: String,
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    use storyforge_infra_plugin_host::Permission;
    state
        .plugin_registry
        .ensure_permission(&plugin_id, &Permission::ReadMemory)
        .map_err(|e| TauriCommandError::validation(e.to_string()))?;
    get_conversation(id, state)
}

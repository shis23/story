use super::super::*;

pub use storyforge_app_service::services::variables::*;

/// 查角色实例的当前变量值
#[tauri::command]
pub(crate) fn get_character_variables(
    campaign_id: String,
    instance_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, TauriCommandError> {
    state
        .storage()
        .require_supported(
            storage_backend::BackendCapability::VariableRead,
            "get character variables",
        )
        .map_err(TauriCommandError::validation)?;
    state
        .storage()
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .map_err(TauriCommandError::storage)?
        .map(|i| i.variables)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))
}

/// 手动改角色实例变量值（调试/纠错用，turn 用 0 占位）
#[tauri::command]
pub fn set_character_variable(
    campaign_id: String,
    instance_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // 旧实现先 `reject_if_active_turn` 再单独写盘，检查与写入之间存在
    // Turn 插入窗口（TOCTOU）；屏障、读改写与写盘现在都在宿主共用入口里。
    set_character_variable_for_host(
        state.storage(),
        &Id::from_str(&campaign_id),
        &Id::from_str(&instance_id),
        &key,
        value,
        turn.unwrap_or(0),
    )
}

/// 查 Campaign 全局变量
#[tauri::command]
pub(crate) fn get_campaign_variables(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<storyforge_domain::variables::VariableValue>, TauriCommandError> {
    state
        .storage()
        .require_supported(
            storage_backend::BackendCapability::VariableRead,
            "get campaign variables",
        )
        .map_err(TauriCommandError::validation)?;
    state
        .storage()
        .get_campaign(&Id::from_str(&campaign_id))
        .map_err(TauriCommandError::storage)?
        .map(|record| record.campaign.variables)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))
}

/// 查 Campaign 全局变量 schema（旧档由 serde 默认补系统字段）。
#[tauri::command]
pub(crate) fn get_campaign_variable_schema(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<storyforge_domain::variables::VariableField>, TauriCommandError> {
    state
        .storage()
        .require_supported(
            storage_backend::BackendCapability::VariableRead,
            "get campaign variable schema",
        )
        .map_err(TauriCommandError::validation)?;
    state
        .storage()
        .get_campaign(&Id::from_str(&campaign_id))
        .map_err(TauriCommandError::storage)?
        .map(|record| record.campaign.variable_schema)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))
}

/// 新增一个 Campaign 全局变量定义与初始值。
#[tauri::command]
pub fn add_campaign_variable(
    campaign_id: String,
    key: String,
    label: String,
    value_type: String,
    default_value: serde_json::Value,
    description: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    let key = storyforge_domain::variables::normalize_mvu_key(&key);
    let label = label.trim();
    let description = description
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    validate_campaign_variable_input(&key, label, description.as_deref(), &default_value)?;
    let value_type = parse_campaign_variable_type(&value_type)?;
    validate_campaign_variable_default(&value_type, &default_value)?;
    let field = storyforge_domain::variables::VariableField {
        key: key.clone(),
        label: if label.is_empty() {
            key.clone()
        } else {
            label.to_string()
        },
        value_type,
        default: default_value,
        description,
        group: Some("全局".into()),
    };

    // 三.5：活动 Turn 屏障 + Campaign 读改写 + 写盘在同一原子单元内完成。
    let applied = state
        .storage()
        .mutate_idle_campaign(&campaign_id, |campaign| {
            campaign.add_variable_field(field.clone())
        })
        .map_err(|error| TauriCommandError::storage(format!("新增 Campaign 变量失败: {error}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 campaign {campaign_id}"
        )));
    }
    Ok(())
}

/// 把卡模板后来新增的全局 schema 显式同步进旧 Campaign。
#[tauri::command]
pub fn sync_campaign_variable_schema(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignVariableSchemaSyncDto, TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    sync_campaign_variable_schema_for_backend(state.storage(), &campaign_id)
}

/// 改 Campaign 全局变量
#[tauri::command]
pub fn set_campaign_variable(
    campaign_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    set_campaign_variable_for_host(
        state.storage(),
        &Id::from_str(&campaign_id),
        &key,
        value,
        turn.unwrap_or(0),
    )
}

/// 把临场角色升级为常驻（仅翻 is_temporary flag）
#[tauri::command]
pub fn promote_temporary_instance(
    campaign_id: String,
    instance_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // 三.5：活动 Turn 屏障 + 读改写 + 写盘在同一原子单元内完成。
    let campaign_id = Id::from_str(&campaign_id);
    let instance_id = Id::from_str(&instance_id);
    let applied = state
        .storage()
        .mutate_idle_instance(&campaign_id, &instance_id, |inst| {
            if !inst.is_temporary {
                return Err("该角色已是常驻".into());
            }
            inst.promote_to_permanent();
            Ok(())
        })
        .map_err(|e| TauriCommandError::storage(format!("升级临时角色失败: {e}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 instance {instance_id}"
        )));
    }
    Ok(())
}

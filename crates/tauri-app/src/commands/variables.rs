use super::super::*;

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
pub(crate) fn set_character_variable(
    campaign_id: String,
    instance_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // P0-7：活动 Turn 期间拒绝直接写变量
    let store = state.json_campaign_store(
        storage_backend::BackendCapability::VariableCommands,
        "set character variable",
    )?;
    reject_if_active_turn(state.storage(), &Id::from_str(&campaign_id))?;
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))?;
    inst.set_variable(&key, value, turn.unwrap_or(0));
    store
        .update_instance(inst)
        .map_err(|e| TauriCommandError::storage(format!("更新角色变量失败: {e}")))?;
    Ok(())
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

pub(crate) fn parse_campaign_variable_type(
    value_type: &str,
) -> Result<storyforge_domain::variables::VariableType, TauriCommandError> {
    use storyforge_domain::variables::VariableType;
    match value_type.trim().to_ascii_lowercase().as_str() {
        "int" | "integer" => Ok(VariableType::Int),
        "float" | "number" => Ok(VariableType::Float),
        "string" | "text" => Ok(VariableType::String),
        "bool" | "boolean" => Ok(VariableType::Bool),
        "json" | "object" | "array" => Ok(VariableType::Json),
        _ => Err(TauriCommandError::validation(format!(
            "不支持的变量类型: {value_type}"
        ))),
    }
}

pub(crate) fn validate_campaign_variable_default(
    value_type: &storyforge_domain::variables::VariableType,
    value: &serde_json::Value,
) -> Result<(), TauriCommandError> {
    use storyforge_domain::variables::VariableType;
    let valid = match value_type {
        VariableType::Int => value.as_i64().is_some(),
        VariableType::Float => value.as_f64().is_some(),
        VariableType::String => value.as_str().is_some(),
        VariableType::Bool => value.as_bool().is_some(),
        VariableType::Json => true,
    };
    if valid {
        Ok(())
    } else {
        Err(TauriCommandError::validation("初始值与变量类型不匹配"))
    }
}

pub(crate) fn validate_campaign_variable_input(
    key: &str,
    label: &str,
    description: Option<&str>,
    default_value: &serde_json::Value,
) -> Result<(), TauriCommandError> {
    let valid_key = !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && !key.split('.').any(|segment| segment.starts_with("__"));
    if !valid_key {
        return Err(TauriCommandError::validation(
            "变量键名只能包含文字、数字、点、下划线或连字符，不能超过 128 字节或使用 __ 内部段",
        ));
    }
    if label.chars().count() > 80 {
        return Err(TauriCommandError::validation(
            "变量中文名称不能超过 80 个字符",
        ));
    }
    if description.is_some_and(|value| value.chars().count() > 500) {
        return Err(TauriCommandError::validation("变量说明不能超过 500 个字符"));
    }
    if serde_json::to_vec(default_value)
        .map(|bytes| bytes.len() > 64 * 1024)
        .unwrap_or(true)
    {
        return Err(TauriCommandError::validation("变量初始值不能超过 64 KiB"));
    }
    Ok(())
}

/// 新增一个 Campaign 全局变量定义与初始值。
#[tauri::command]
pub(crate) fn add_campaign_variable(
    campaign_id: String,
    key: String,
    label: String,
    value_type: String,
    default_value: serde_json::Value,
    description: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    let store = state.json_campaign_store(
        storage_backend::BackendCapability::VariableCommands,
        "add campaign variable",
    )?;
    reject_if_active_turn(state.storage(), &campaign_id)?;
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

    let mut campaign = store
        .get_campaign(&campaign_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))?;
    campaign
        .add_variable_field(field)
        .map_err(TauriCommandError::validation)?;
    store
        .update_campaign(campaign)
        .map_err(|error| TauriCommandError::storage(format!("新增 Campaign 变量失败: {error}")))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CampaignVariableSchemaSyncDto {
    pub(crate) added: usize,
}

/// 把卡模板后来新增的全局 schema 显式同步进旧 Campaign。
#[tauri::command]
pub(crate) fn sync_campaign_variable_schema(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignVariableSchemaSyncDto, TauriCommandError> {
    let campaign_id = Id::from_str(&campaign_id);
    let store = state.json_campaign_store(
        storage_backend::BackendCapability::VariableCommands,
        "sync campaign variable schema",
    )?;
    reject_if_active_turn(state.storage(), &campaign_id)?;
    sync_campaign_variable_schema_in_store(store, &campaign_id)
}

pub(crate) fn sync_campaign_variable_schema_in_store(
    store: &campaign_store::CampaignStore,
    campaign_id: &Id,
) -> Result<CampaignVariableSchemaSyncDto, TauriCommandError> {
    let mut campaign = store
        .get_campaign(campaign_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))?;
    let card = store
        .get_card(&campaign.card_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 card {}", campaign.card_id)))?;
    let added = campaign.sync_variable_schema(&card.card.effective_campaign_variable_schema());
    store
        .update_campaign(campaign)
        .map_err(|error| TauriCommandError::storage(format!("同步 Campaign 变量失败: {error}")))?;
    Ok(CampaignVariableSchemaSyncDto { added })
}

/// 改 Campaign 全局变量
#[tauri::command]
pub(crate) fn set_campaign_variable(
    campaign_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // P0-7：活动 Turn 期间拒绝直接写变量
    let store = state.json_campaign_store(
        storage_backend::BackendCapability::VariableCommands,
        "set campaign variable",
    )?;
    reject_if_active_turn(state.storage(), &Id::from_str(&campaign_id))?;
    let mut camp = store
        .get_campaign(&Id::from_str(&campaign_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))?;
    camp.set_variable(&key, value, turn.unwrap_or(0));
    store
        .update_campaign(camp)
        .map_err(|e| TauriCommandError::storage(format!("更新 Campaign 变量失败: {e}")))?;
    Ok(())
}

/// 把临场角色升级为常驻（仅翻 is_temporary flag）
#[tauri::command]
pub(crate) fn promote_temporary_instance(
    campaign_id: String,
    instance_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // P0-7：活动 Turn 期间拒绝直接改实例
    let store = state.json_campaign_store(
        storage_backend::BackendCapability::VariableCommands,
        "promote temporary instance",
    )?;
    reject_if_active_turn(state.storage(), &Id::from_str(&campaign_id))?;
    let mut inst = store
        .get_instance(&Id::from_str(&campaign_id), &Id::from_str(&instance_id))
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 instance {instance_id}")))?;
    if !inst.is_temporary {
        return Err("该角色已是常驻".into());
    }
    inst.promote_to_permanent();
    store
        .update_instance(inst)
        .map_err(|e| TauriCommandError::storage(format!("升级临时角色失败: {e}")))?;
    Ok(())
}

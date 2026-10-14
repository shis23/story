use super::super::*;

pub fn parse_campaign_variable_type(
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

pub fn validate_campaign_variable_default(
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

pub fn validate_campaign_variable_input(
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

/// 写入路径（`set_*_variable`）的键名守卫。
///
/// 2026-09-13 域4 修复 T-06：`set_campaign_variable` / `set_character_variable`
/// 此前对 `key` **不做任何校验**，而 `__storyforge*` 内部命名空间（例如
/// `__storyforge_card_shell_variables`）的拒绝只写在前端
/// `frontend/src/utils/shellVariableOutbox.js`。前端守卫不是安全边界——这与本项目
/// 对插件通道已确立的标准（`plugin-bridge` 前端权限数组之外，后端
/// `PluginRegistry` 二次校验）不一致，属防御纵深缺口：经命令层可写入内部键，
/// 而 `utils/mvuStatTree.js` 正是靠该前缀把内部键排除出 stat_data 树的。
/// 这里复用 `validate_campaign_variable_input` 的键名规则做后端侧守卫。
///
/// 卡壳选择器桶 [`CARD_SHELL_VARIABLES_KEY`] 不经本守卫放行，而是走专用入口
/// [`set_card_shell_variables_for_backend`]（只写该键、校验桶形状与大小）。
pub fn validate_variable_write_key(key: &str) -> Result<(), TauriCommandError> {
    let valid = !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && !key.split('.').any(|segment| segment.starts_with("__"));
    if valid {
        Ok(())
    } else {
        Err(TauriCommandError::validation(
            "变量键名只能包含文字、数字、点、下划线或连字符，不能超过 128 字节或使用 __ 内部段",
        ))
    }
}

/// 改角色实例变量（Tauri `set_character_variable` 与非 Tauri 宿主共用）。
///
/// 三.5：活动 Turn 屏障 + 读改写 + 写盘在**同一原子单元**内完成
/// （JSON：turns 锁守卫内 candidate→persist→swap；SQLite：单 UoW 事务）。
/// T-06：后端侧键名守卫（__ 内部段 / 字符集 / 长度），前端守卫不是安全边界。
pub fn set_character_variable_for_host(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: &Id,
    instance_id: &Id,
    key: &str,
    value: serde_json::Value,
    turn: u32,
) -> Result<(), TauriCommandError> {
    validate_variable_write_key(key)?;
    let applied = storage
        .mutate_idle_instance(campaign_id, instance_id, |inst| {
            inst.set_variable(key, value.clone(), turn);
            Ok(())
        })
        .map_err(|e| TauriCommandError::storage(format!("更新角色变量失败: {e}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 instance {instance_id}"
        )));
    }
    Ok(())
}

/// 改 Campaign 全局变量（Tauri `set_campaign_variable` 与非 Tauri 宿主共用；屏障与守卫同上）。
pub fn set_campaign_variable_for_host(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: &Id,
    key: &str,
    value: serde_json::Value,
    turn: u32,
) -> Result<(), TauriCommandError> {
    validate_variable_write_key(key)?;
    let applied = storage
        .mutate_idle_campaign(campaign_id, |camp| {
            camp.set_variable(key, value.clone(), turn);
            Ok(())
        })
        .map_err(|e| TauriCommandError::storage(format!("更新 Campaign 变量失败: {e}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 campaign {campaign_id}"
        )));
    }
    Ok(())
}

/// 卡壳 TavernHelper `setVariables` / `replaceVariables` 选择器桶所在的
/// Campaign 变量键（镜像 `frontend/src/utils/cardShellVariableStore.js`）。
pub const CARD_SHELL_VARIABLES_KEY: &str = "__storyforge_card_shell_variables";

/// 整个桶对象序列化后的上限：每次写入都会改写整条 Campaign 记录。
pub const CARD_SHELL_VARIABLES_MAX_BYTES: usize = 2 * 1024 * 1024;

/// 选择器类型，镜像 `cardShellVariableStore.js` 的 `SUPPORTED_BUCKETS`。
const CARD_SHELL_SELECTOR_TYPES: [&str; 7] = [
    "message",
    "character",
    "local",
    "chat",
    "global",
    "script",
    "preset",
];

/// 选择器身份段长度上限，镜像前端 `selectorKey` 的 256 截断。
const CARD_SHELL_SELECTOR_IDENTITY_MAX_CHARS: usize = 256;

/// 卡壳选择器桶的形状校验：顶层是对象，每个键为 `<类型>:<身份>`，每个桶也是对象。
///
/// 与前端 `normalizeCardShellVariables` 的规范形一致；前端写入前总会先归一化，
/// 所以合法写入不会被拒。
pub fn validate_card_shell_variables(value: &serde_json::Value) -> Result<(), TauriCommandError> {
    let Some(buckets) = value.as_object() else {
        return Err(TauriCommandError::validation("卡壳变量必须是选择器桶对象"));
    };
    let size = serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if size > CARD_SHELL_VARIABLES_MAX_BYTES {
        return Err(TauriCommandError::validation(format!(
            "卡壳变量不能超过 {} KiB（当前 {} KiB）",
            CARD_SHELL_VARIABLES_MAX_BYTES / 1024,
            size.div_ceil(1024)
        )));
    }
    for (key, bucket) in buckets {
        let valid_key = key.split_once(':').is_some_and(|(kind, identity)| {
            CARD_SHELL_SELECTOR_TYPES.contains(&kind)
                && !identity.is_empty()
                && identity.chars().count() <= CARD_SHELL_SELECTOR_IDENTITY_MAX_CHARS
        });
        let shown = || key.chars().take(64).collect::<String>();
        if !valid_key {
            return Err(TauriCommandError::validation(format!(
                "卡壳变量桶键无效: {}",
                shown()
            )));
        }
        if !bucket.is_object() {
            return Err(TauriCommandError::validation(format!(
                "卡壳变量桶必须是对象: {}",
                shown()
            )));
        }
    }
    Ok(())
}

/// 卡壳选择器桶的专用写入口：只写 [`CARD_SHELL_VARIABLES_KEY`]。
///
/// 通用的 `set_campaign_variable` 经 [`validate_variable_write_key`] 拒绝 `__`
/// 段（T-06），该守卫不放宽；桶写入在这里校验形状与大小后整体替换。
/// 活动 Turn 屏障 + 读改写 + 写盘与其它变量写入相同（`mutate_idle_campaign`）。
pub fn set_card_shell_variables_for_backend(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: &Id,
    variables: serde_json::Value,
) -> Result<(), TauriCommandError> {
    validate_card_shell_variables(&variables)?;
    let applied = storage
        .mutate_idle_campaign(campaign_id, |campaign| {
            campaign.set_variable(CARD_SHELL_VARIABLES_KEY, variables.clone(), 0);
            Ok(())
        })
        .map_err(|error| TauriCommandError::storage(format!("保存卡壳变量失败: {error}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 campaign {campaign_id}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct CampaignVariableSchemaSyncDto {
    pub added: usize,
}

/// 测试用 JSON 直连同步（等价 JSON `CampaignStore` 语义，仅测试夹具）。
#[cfg(any(test, feature = "test-support"))]
pub fn sync_campaign_variable_schema_in_store(
    store: &crate::campaign_store::CampaignStore,
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

/// backend-neutral 版本：读 Campaign/卡经 facade，同步后经
/// `StorageFacade::update_campaign` 落盘（JSON 文件 / SQLite 行）。
pub fn sync_campaign_variable_schema_for_backend(
    storage: &crate::storage_backend::StorageFacade,
    campaign_id: &Id,
) -> Result<CampaignVariableSchemaSyncDto, TauriCommandError> {
    let record = storage
        .get_campaign(campaign_id)
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 campaign {campaign_id}")))?;
    let card = storage
        .get_card(&record.campaign.card_id)
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| {
            TauriCommandError::not_found(format!("找不到 card {}", record.campaign.card_id))
        })?;
    let schema = card.card.effective_campaign_variable_schema();
    // 三.5：活动 Turn 屏障 + 读改写 + 写盘在同一原子单元内完成。
    let added = std::cell::Cell::new(0usize);
    let applied = storage
        .mutate_idle_campaign(campaign_id, |campaign| {
            let n = campaign.sync_variable_schema(&schema);
            added.set(n);
            Ok(())
        })
        .map_err(|error| TauriCommandError::storage(format!("同步 Campaign 变量失败: {error}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到 campaign {campaign_id}"
        )));
    }
    Ok(CampaignVariableSchemaSyncDto { added: added.get() })
}

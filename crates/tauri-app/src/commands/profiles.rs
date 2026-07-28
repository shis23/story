use super::super::*;

// ─── 模块、Prompt Profile 与 Agent Profile Config 命令 ─────────────────────

#[tauri::command]
pub(crate) fn list_modules(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<module_store::PromptModuleDto> {
    state
        .module_store
        .list_all()
        .iter()
        .map(|(module, enabled)| module_store::module_to_dto(module, *enabled))
        .collect()
}

#[tauri::command]
pub(crate) fn update_module(
    id: String,
    content: Option<String>,
    enabled: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if state
        .module_store
        .update(&id, content.as_deref(), enabled)
        .map_err(|error| TauriCommandError::storage(format!("存储写入失败: {error}")))?
    {
        Ok(())
    } else {
        Err("内置模块不能修改内容".into())
    }
}

#[tauri::command]
pub(crate) fn list_profiles(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<module_store::ProfileSummaryDto> {
    state.profile_store.list()
}

#[tauri::command]
pub(crate) fn get_active_profile(
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<module_store::PromptProfileDto> {
    state
        .profile_store
        .get_active()
        .map(|profile| module_store::profile_to_dto(&profile, true))
}

#[tauri::command]
pub(crate) fn save_profile(
    profile_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let profile: PromptProfile = serde_json::from_str(&profile_json)
        .map_err(|error| TauriCommandError::validation(format!("Profile 解析失败: {error}")))?;
    state
        .profile_store
        .save(profile)
        .map_err(|error| TauriCommandError::storage(format!("存储写入失败: {error}")))?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_active_profile(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .profile_store
        .set_active(&id)
        .map_err(|error| TauriCommandError::storage(format!("存储写入失败: {error}")))
}

#[tauri::command]
pub(crate) fn list_agent_profile_configs(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<storyforge_domain::agent_profile_config::AgentProfileConfigSummaryDto> {
    state.agent_profile_config_store.list()
}

#[tauri::command]
pub(crate) fn get_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Option<storyforge_domain::agent_profile_config::AgentProfileConfig> {
    state.agent_profile_config_store.get(&id)
}

#[tauri::command]
pub(crate) fn get_active_agent_profile_config(
    state: tauri::State<'_, Arc<AppState>>,
) -> storyforge_domain::agent_profile_config::AgentProfileConfig {
    state.agent_profile_config_store.get_active()
}

#[tauri::command]
pub(crate) fn save_agent_profile_config(
    config_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let mut config: storyforge_domain::agent_profile_config::AgentProfileConfig =
        serde_json::from_str(&config_json).map_err(|error| {
            TauriCommandError::validation(format!("Agent Profile Config 解析失败: {error}"))
        })?;
    config.sanitize();
    state
        .agent_profile_config_store
        .save(config)
        .map_err(TauriCommandError::from)
}

#[tauri::command]
pub(crate) fn export_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    state
        .agent_profile_config_store
        .export_json(&id)
        .map_err(TauriCommandError::from)
}

#[tauri::command]
pub(crate) fn import_agent_profile_config(
    config_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<storyforge_domain::agent_profile_config::AgentProfileConfig, TauriCommandError> {
    state
        .agent_profile_config_store
        .import_json(&config_json)
        .map_err(TauriCommandError::from)
}

#[tauri::command]
pub(crate) fn delete_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<bool, TauriCommandError> {
    state
        .agent_profile_config_store
        .delete(&id)
        .map_err(TauriCommandError::from)
}

#[tauri::command]
pub(crate) fn set_active_agent_profile_config(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    state
        .agent_profile_config_store
        .set_active(&id)
        .map_err(TauriCommandError::from)
}

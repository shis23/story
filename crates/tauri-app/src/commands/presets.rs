use super::super::*;

#[tauri::command]
pub(crate) fn import_preset(data: Vec<u8>) -> Result<String, TauriCommandError> {
    let preset = storyforge_infra_import::import_preset(&data).map_err(TauriCommandError::from)?;
    let preset_id = get_preset_store()
        .save(preset.clone())
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
    Ok(format!(
        "预设 '{}' (id: {}) 导入成功，含 {} 条提示词、{} 条正则",
        preset.name,
        preset_id,
        preset.prompts.len(),
        preset.regex_scripts.len()
    ))
}

/// 预设摘要 DTO（列表用）
#[derive(Debug, Clone, Serialize)]
pub struct PresetSummaryDto {
    pub id: String,
    pub name: String,
    pub prompt_count: usize,
    pub regex_count: usize,
    pub imported_at: String,
    pub active: bool,
}

/// 预设详情 DTO
#[derive(Debug, Clone, Serialize)]
pub struct PresetDetailDto {
    pub id: String,
    pub name: String,
    pub prompts: Vec<PresetPromptDto>,
    pub regex_scripts: Vec<RegexScriptDto>,
    pub imported_at: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PresetPromptDto {
    pub identifier: String,
    pub name: String,
    pub role: String,
    pub content: String,
    pub enabled: bool,
    pub marker: bool,
    pub is_system_prompt: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegexScriptDto {
    pub id: String,
    pub script_name: String,
    pub find_regex: String,
    pub replace_string: String,
    pub placement: String,
    pub placement_codes: Vec<i32>,
    pub disabled: bool,
}

pub(crate) fn regex_script_dto(r: &RegexScript) -> RegexScriptDto {
    RegexScriptDto {
        id: r.id.clone(),
        script_name: r.script_name.clone(),
        find_regex: r.find_regex.clone(),
        replace_string: r.replace_string.clone(),
        placement: match r.placement {
            storyforge_domain::preset::RegexPlacement::Input => "input",
            storyforge_domain::preset::RegexPlacement::Output => "output",
            storyforge_domain::preset::RegexPlacement::SlashCommand => "slash_command",
            storyforge_domain::preset::RegexPlacement::WorldInfo => "world_info",
            storyforge_domain::preset::RegexPlacement::Reasoning => "reasoning",
        }
        .to_string(),
        placement_codes: r.placement_codes.clone(),
        disabled: r.disabled,
    }
}

#[tauri::command]
pub(crate) fn list_presets() -> Vec<PresetSummaryDto> {
    let store = get_preset_store();
    let active_id = store.active_id();
    store
        .list()
        .iter()
        .map(|sp| PresetSummaryDto {
            id: sp.id.clone(),
            name: sp.preset.name.clone(),
            prompt_count: sp.preset.prompts.len(),
            regex_count: sp.preset.regex_scripts.len(),
            imported_at: sp.imported_at.clone(),
            active: active_id.as_deref() == Some(sp.id.as_str()),
        })
        .collect()
}

#[tauri::command]
pub(crate) fn get_preset(id: String) -> Result<PresetDetailDto, TauriCommandError> {
    let store = get_preset_store();
    let active_id = store.active_id();
    let sp = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到预设 {id}")))?;
    Ok(PresetDetailDto {
        id: sp.id.clone(),
        name: sp.preset.name.clone(),
        prompts: sp
            .preset
            .prompts
            .iter()
            .map(|p| PresetPromptDto {
                identifier: p.identifier.clone(),
                name: p.name.clone(),
                role: match p.role {
                    storyforge_domain::preset::PromptRole::System => "system",
                    storyforge_domain::preset::PromptRole::User => "user",
                    storyforge_domain::preset::PromptRole::Assistant => "assistant",
                }
                .to_string(),
                content: p.content.clone(),
                enabled: p.enabled,
                marker: p.marker,
                is_system_prompt: p.is_system_prompt,
            })
            .collect(),
        regex_scripts: sp
            .preset
            .regex_scripts
            .iter()
            .map(regex_script_dto)
            .collect(),
        imported_at: sp.imported_at.clone(),
        active: active_id.as_deref() == Some(sp.id.as_str()),
    })
}

#[tauri::command]
pub(crate) fn get_active_preset() -> Option<PresetSummaryDto> {
    let sp = get_preset_store().active()?;
    Some(PresetSummaryDto {
        id: sp.id.clone(),
        name: sp.preset.name.clone(),
        prompt_count: sp.preset.prompts.len(),
        regex_count: sp.preset.regex_scripts.len(),
        imported_at: sp.imported_at.clone(),
        active: true,
    })
}

#[tauri::command]
pub(crate) fn set_active_preset(id: Option<String>) -> Result<(), TauriCommandError> {
    let store = get_preset_store();
    match id {
        Some(id) => {
            if store
                .set_active(&id)
                .map_err(|e| TauriCommandError::storage(format!("storage write failed: {e}")))?
            {
                Ok(())
            } else {
                Err(TauriCommandError::not_found(format!(
                    "preset not found: {id}"
                )))
            }
        }
        None => store
            .clear_active()
            .map_err(|e| TauriCommandError::storage(format!("storage write failed: {e}"))),
    }
}

#[tauri::command]
pub(crate) fn delete_preset(id: String) -> Result<(), TauriCommandError> {
    if get_preset_store()
        .delete(&id)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!("找不到预设 {id}")))
    }
}

#[tauri::command]
pub(crate) fn update_preset_prompt(
    preset_id: String,
    prompt_index: usize,
    content: Option<String>,
    enabled: Option<bool>,
) -> Result<(), TauriCommandError> {
    if get_preset_store()
        .update_prompt(&preset_id, prompt_index, content.as_deref(), enabled)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!(
            "找不到预设 {preset_id} 的第 {prompt_index} 条 prompt"
        )))
    }
}

#[tauri::command]
pub(crate) fn update_preset_regex(
    preset_id: String,
    regex_index: usize,
    disabled: Option<bool>,
) -> Result<(), TauriCommandError> {
    if get_preset_store()
        .update_regex(&preset_id, regex_index, disabled)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!(
            "找不到预设 {preset_id} 的第 {regex_index} 条正则"
        )))
    }
}

#[tauri::command]
pub(crate) fn list_global_regex_scripts() -> Vec<RegexScriptDto> {
    get_global_regex_store()
        .list()
        .iter()
        .map(regex_script_dto)
        .collect()
}

#[tauri::command]
pub(crate) fn import_global_regex_settings(
    settings_json: String,
) -> Result<usize, TauriCommandError> {
    get_global_regex_store()
        .import_from_settings_json(&settings_json)
        .map_err(|e| TauriCommandError::storage(format!("global regex import failed: {e}")))
}

#[tauri::command]
pub(crate) fn clear_global_regex_scripts() -> Result<(), TauriCommandError> {
    get_global_regex_store()
        .clear()
        .map_err(|e| TauriCommandError::storage(format!("global regex clear failed: {e}")))
}

#[tauri::command]
pub(crate) fn update_global_regex(
    regex_index: usize,
    disabled: Option<bool>,
) -> Result<(), TauriCommandError> {
    if get_global_regex_store()
        .update_regex(regex_index, disabled)
        .map_err(|e| TauriCommandError::storage(format!("global regex update failed: {e}")))?
    {
        Ok(())
    } else {
        Err(TauriCommandError::not_found(format!(
            "global regex not found at index {regex_index}"
        )))
    }
}

/// 将 ST 预设的 prompts 转换为 PromptModule 并存入 ModuleStore
#[tauri::command]
pub(crate) fn import_preset_as_modules(
    preset_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    use storyforge_domain::agent::AgentRole;
    use storyforge_domain::prompt_module::{
        Exclusivity, ModuleCategory, ModuleSource, PromptModule,
    };

    let stored = get_preset_store()
        .get(&preset_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到预设 {preset_id}")))?;
    let mut count = 0;

    for prompt in &stored.preset.prompts {
        // 跳过 marker 和空内容
        if prompt.marker || prompt.content.trim().is_empty() {
            continue;
        }

        // 根据 ST role 映射到 ModuleCategory
        let category = match prompt.role {
            storyforge_domain::preset::PromptRole::System => ModuleCategory::Quality,
            storyforge_domain::preset::PromptRole::User => ModuleCategory::Output,
            storyforge_domain::preset::PromptRole::Assistant => ModuleCategory::Style,
        };

        let module_id = format!("st-{}-{}", preset_id, prompt.identifier);
        let module = PromptModule {
            id: Id::from_str(&module_id),
            name: prompt.name.clone(),
            category,
            content: prompt.content.clone(),
            exclusivity: Exclusivity::Multiple,
            source: ModuleSource::ImportedFromST,
            applicable_roles: vec![AgentRole::Editor, AgentRole::Subagent("*".into())],
            tags: vec!["ST导入".into(), stored.preset.name.clone()],
        };

        state
            .module_store
            .add(module)
            .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;
        count += 1;
    }

    Ok(count)
}

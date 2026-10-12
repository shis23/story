use super::super::*;

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

pub fn regex_script_dto(r: &RegexScript) -> RegexScriptDto {
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

use serde::{Deserialize, Serialize};

use crate::Source;

/// 预设（系统提示词模板 + 正则脚本）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    /// 预设的提示词模块列表（ST 的 prompts 数组）
    pub prompts: Vec<PresetPrompt>,
    /// 正则脚本列表（ST 的 extensions.regex_scripts）
    pub regex_scripts: Vec<RegexScript>,
    /// 来源
    pub source: Source,
}

/// 预设中的单条提示词（ST prompt 对象）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetPrompt {
    pub identifier: String,
    pub name: String,
    pub role: PromptRole,
    pub content: String,
    /// 是否启用（ST 的 disable 字段取反）
    pub enabled: bool,
    /// 是否为 marker（ST 内部用的占位符）
    pub marker: bool,
    /// 是否为系统提示词（ST 的 system_prompt 字段）
    pub is_system_prompt: bool,
    /// 是否可删除
    pub deletable: bool,
}

/// 提示词角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptRole {
    System,
    User,
    Assistant,
}

/// 正则脚本（ST regex_scripts 数组元素）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexScript {
    pub id: String,
    pub script_name: String,
    pub find_regex: String,
    pub replace_string: String,
    /// 作用域（D12：输入正则 vs 输出正则）
    pub placement: RegexPlacement,
    /// ST 原始 placement 数组，用于后续恢复 World Info/Slash/Reasoning 等作用域语义。
    #[serde(default)]
    pub placement_codes: Vec<i32>,
    #[serde(default)]
    pub source: RegexScriptSource,
    /// 是否禁用
    pub disabled: bool,
    /// ST 原始字段（flags 等）
    pub flags: String,
    pub only_format_formatting: Option<bool>,
    pub markdown_only: Option<bool>,
    pub prompt_only: Option<bool>,
    pub run_on_edit: Option<bool>,
    pub substitute_regex: Option<i32>,
    #[serde(default)]
    pub trim_strings: Vec<String>,
    pub min_depth: Option<i32>,
    pub max_depth: Option<i32>,
}

/// 正则作用域（D12）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegexPlacement {
    /// 输入正则：用户→导演前
    Input,
    /// 输出正则：编剧成文后
    Output,
}

/// ST regex script source. Defaulting to Preset keeps older stored preset JSON compatible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegexScriptSource {
    Global,
    #[default]
    Preset,
    Scoped,
}

/// ST 预设 JSON 结构（用于反序列化导入）
#[derive(Debug, Deserialize)]
pub struct StPreset {
    pub name: Option<String>,
    #[serde(default)]
    pub prompts: Vec<StPresetPrompt>,
    #[serde(default)]
    pub extensions: serde_json::Value,
}

/// ST 预设 prompt
#[derive(Debug, Deserialize)]
pub struct StPresetPrompt {
    pub identifier: Option<String>,
    pub name: Option<String>,
    pub role: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub system_prompt: bool,
    #[serde(default)]
    pub marker: bool,
    #[serde(default)]
    pub disable: bool,
    #[serde(default)]
    pub deletable: bool,
}

/// ST 正则脚本（从 extensions.regex_scripts 解析）
#[derive(Debug, Deserialize)]
pub struct StRegexScript {
    pub id: Option<String>,
    #[serde(rename = "scriptName")]
    pub script_name: Option<String>,
    #[serde(rename = "findRegex")]
    pub find_regex: Option<String>,
    #[serde(rename = "replaceString")]
    pub replace_string: Option<String>,
    #[serde(default)]
    pub placement: Vec<i32>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub flags: String,
    #[serde(rename = "onlyFormatFormatting")]
    pub only_format_formatting: Option<bool>,
    #[serde(rename = "markdownOnly")]
    pub markdown_only: Option<bool>,
    #[serde(rename = "promptOnly")]
    pub prompt_only: Option<bool>,
    #[serde(rename = "runOnEdit")]
    pub run_on_edit: Option<bool>,
    #[serde(rename = "substituteRegex")]
    pub substitute_regex: Option<i32>,
    #[serde(default, rename = "trimStrings")]
    pub trim_strings: Vec<String>,
    #[serde(rename = "minDepth")]
    pub min_depth: Option<i32>,
    #[serde(rename = "maxDepth")]
    pub max_depth: Option<i32>,
}

impl Preset {
    /// 从 ST 预设 JSON 解析
    pub fn from_st(st: StPreset) -> Self {
        let name = st.name.unwrap_or_else(|| "Imported Preset".into());

        let prompts: Vec<PresetPrompt> = st
            .prompts
            .into_iter()
            .map(|p| PresetPrompt {
                identifier: p
                    .identifier
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                name: p.name.unwrap_or_default(),
                role: match p.role.as_deref() {
                    Some("user") => PromptRole::User,
                    Some("assistant") => PromptRole::Assistant,
                    _ => PromptRole::System,
                },
                content: p.content,
                enabled: !p.disable,
                marker: p.marker,
                is_system_prompt: p.system_prompt,
                deletable: p.deletable,
            })
            .collect();

        // 提取正则脚本（从 extensions.regex_scripts）
        let regex_scripts = extract_regex_scripts(&st.extensions);

        Self {
            name,
            prompts,
            regex_scripts,
            source: Source::ImportedFromST,
        }
    }

    /// 获取所有启用的系统提示词（按 ST 的 prompt_order 顺序，这里简化为原始顺序）
    pub fn enabled_system_prompts(&self) -> Vec<&PresetPrompt> {
        self.prompts
            .iter()
            .filter(|p| p.enabled && p.role == PromptRole::System && !p.marker)
            .collect()
    }

    /// 获取输入正则（作用于用户→导演前）
    pub fn input_regex_scripts(&self) -> Vec<&RegexScript> {
        self.regex_scripts
            .iter()
            .filter(|r| !r.disabled && r.placement == RegexPlacement::Input)
            .collect()
    }

    /// 获取输出正则（作用于编剧成文后）
    pub fn output_regex_scripts(&self) -> Vec<&RegexScript> {
        self.regex_scripts
            .iter()
            .filter(|r| !r.disabled && r.placement == RegexPlacement::Output)
            .collect()
    }
}

/// 从 ST extensions 中提取正则脚本
pub(crate) fn extract_regex_scripts(extensions: &serde_json::Value) -> Vec<RegexScript> {
    extract_regex_scripts_with_source(extensions, RegexScriptSource::Preset)
}

pub(crate) fn extract_regex_scripts_with_source(
    extensions: &serde_json::Value,
    source: RegexScriptSource,
) -> Vec<RegexScript> {
    let scripts = match extensions.get("regex_scripts") {
        Some(v) => v,
        None => return Vec::new(),
    };

    let arr = match scripts.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    arr.iter()
        .filter_map(|v| serde_json::from_value::<StRegexScript>(v.clone()).ok())
        .map(|s| {
            // ST placement: [0] = 位置（0=主输入, 1=世界书, 2=输出）, [1] = 编辑器
            // 我们简化：0 → Input, 2 → Output, 其他 → Input
            let placement = if s.placement.first() == Some(&2) {
                RegexPlacement::Output
            } else {
                RegexPlacement::Input
            };
            let placement_codes = s.placement;

            RegexScript {
                id: s.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                script_name: s.script_name.unwrap_or_default(),
                find_regex: s.find_regex.unwrap_or_default(),
                replace_string: s.replace_string.unwrap_or_default(),
                placement,
                placement_codes,
                source,
                disabled: s.disabled,
                flags: s.flags,
                only_format_formatting: s.only_format_formatting,
                markdown_only: s.markdown_only,
                prompt_only: s.prompt_only,
                run_on_edit: s.run_on_edit,
                substitute_regex: s.substitute_regex,
                trim_strings: s.trim_strings,
                min_depth: s.min_depth,
                max_depth: s.max_depth,
            }
        })
        .collect()
}

pub fn merge_regex_script_sources(
    global: &[RegexScript],
    preset: &[RegexScript],
    scoped: &[RegexScript],
) -> Vec<RegexScript> {
    let mut merged = Vec::with_capacity(global.len() + preset.len() + scoped.len());
    append_scripts_with_source(&mut merged, global, RegexScriptSource::Global);
    append_scripts_with_source(&mut merged, preset, RegexScriptSource::Preset);
    append_scripts_with_source(&mut merged, scoped, RegexScriptSource::Scoped);
    merged
}

fn append_scripts_with_source(
    merged: &mut Vec<RegexScript>,
    scripts: &[RegexScript],
    source: RegexScriptSource,
) {
    for script in scripts {
        let mut script = script.clone();
        script.source = source;
        merged.push(script);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_script(id: &str, name: &str, source: RegexScriptSource) -> RegexScript {
        RegexScript {
            id: id.to_string(),
            script_name: name.to_string(),
            find_regex: name.to_string(),
            replace_string: String::new(),
            placement: RegexPlacement::Output,
            placement_codes: vec![2],
            source,
            disabled: false,
            flags: String::new(),
            only_format_formatting: None,
            markdown_only: None,
            prompt_only: None,
            run_on_edit: None,
            substitute_regex: None,
            trim_strings: vec![],
            min_depth: None,
            max_depth: None,
        }
    }

    #[test]
    fn merge_regex_script_sources_preserves_st_priority_order_and_marks_source() {
        let global = vec![make_script("g1", "global", RegexScriptSource::Preset)];
        let preset = vec![make_script("p1", "preset", RegexScriptSource::Scoped)];
        let scoped = vec![make_script("s1", "scoped", RegexScriptSource::Global)];

        let merged = merge_regex_script_sources(&global, &preset, &scoped);

        let ids: Vec<_> = merged.iter().map(|script| script.id.as_str()).collect();
        assert_eq!(ids, vec!["g1", "p1", "s1"]);

        let sources: Vec<_> = merged.iter().map(|script| script.source).collect();
        assert_eq!(
            sources,
            vec![
                RegexScriptSource::Global,
                RegexScriptSource::Preset,
                RegexScriptSource::Scoped,
            ]
        );
    }

    #[test]
    fn regex_script_source_defaults_to_preset_for_old_stored_json() {
        let script: RegexScript = serde_json::from_value(serde_json::json!({
            "id": "old",
            "script_name": "old stored script",
            "find_regex": "a",
            "replace_string": "b",
            "placement": "Output",
            "disabled": false,
            "flags": "gm",
            "only_format_formatting": null,
            "markdown_only": null,
            "prompt_only": null,
            "run_on_edit": null,
            "substitute_regex": null,
            "trim_strings": [],
            "min_depth": null,
            "max_depth": null
        }))
        .expect("old regex script should deserialize");

        assert_eq!(script.source, RegexScriptSource::Preset);
    }
}

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::storage::json_store;
use serde_json::Value;
use storyforge_domain::preset::{Preset, RegexScript, RegexScriptSource, StPreset};

pub struct GlobalRegexStore {
    path: PathBuf,
    inner: Mutex<Vec<RegexScript>>,
}

impl GlobalRegexStore {
    pub fn new(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("global_regex_scripts.json");
        let mut scripts: Vec<RegexScript> = json_store::load_json_with_tmp_backup_or_default(
            &path,
            |e| tracing::warn!("global regex JSON parse failed ({e}), trying .tmp backup"),
            |path, e| {
                tracing::error!(
                    "global regex JSON and .tmp backup are corrupt, file: {}, error: {}. saved .corrupt backup",
                    path.display(),
                    e
                )
            },
        );
        normalize_global_sources(&mut scripts);

        Self {
            path,
            inner: Mutex::new(scripts),
        }
    }

    pub fn list(&self) -> Vec<RegexScript> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn replace_all(&self, mut scripts: Vec<RegexScript>) -> Result<usize, String> {
        // H-1: trial-compile every find_regex before accepting the batch, so a
        // malformed/catastrophic regex in imported ST settings JSON is rejected at
        // import time with an actionable error naming the offending script, instead
        // of freezing the writing pipeline on first application. (Runtime
        // catastrophic backtracking is still backstopped by apply_single_script's
        // wall-clock timeout.)
        for script in &scripts {
            storyforge_infra_regex::validate_regex(&script.find_regex, &script.flags).map_err(
                |error| format!("全局正则 '{}' 校验失败: {error}", script.script_name),
            )?;
        }
        normalize_global_sources(&mut scripts);
        let len = scripts.len();
        {
            let mut current = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            *current = scripts;
            self.persist(&current)?;
        }
        Ok(len)
    }

    pub fn import_from_settings_json(&self, settings_json: &str) -> Result<usize, String> {
        let settings: Value = serde_json::from_str(settings_json)
            .map_err(|e| format!("parse ST settings JSON failed: {e}"))?;
        let extensions = find_regex_extensions(&settings)
            .ok_or_else(|| "no ST regex_scripts found in settings JSON".to_string())?;
        self.replace_all(regex_scripts_from_extensions(extensions))
    }

    pub fn clear(&self) -> Result<(), String> {
        self.replace_all(Vec::new()).map(|_| ())
    }

    pub fn update_regex(&self, regex_index: usize, disabled: Option<bool>) -> Result<bool, String> {
        let mut scripts = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(regex) = scripts.get_mut(regex_index) {
            if let Some(disabled) = disabled {
                regex.disabled = disabled;
            }
            self.persist(&scripts)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn persist(&self, scripts: &[RegexScript]) -> Result<(), String> {
        storyforge_infra_util::atomic_write_json(&self.path, scripts).map_err(|e| {
            let msg = format!("persist global regex failed: {e}");
            tracing::error!("{msg}");
            msg
        })
    }
}

fn regex_scripts_from_extensions(extensions: Value) -> Vec<RegexScript> {
    let preset = Preset::from_st(StPreset {
        name: Some("Global Regex".into()),
        prompts: vec![],
        extensions,
    });
    let mut scripts = preset.regex_scripts;
    normalize_global_sources(&mut scripts);
    scripts
}

fn find_regex_extensions(value: &Value) -> Option<Value> {
    match value {
        Value::Object(map) => {
            if map.get("regex_scripts").and_then(Value::as_array).is_some() {
                return Some(value.clone());
            }

            if let Some(global_scripts) = map
                .get("global_regex_scripts")
                .or_else(|| map.get("globalRegexScripts"))
                .filter(|v| v.as_array().is_some())
            {
                return Some(serde_json::json!({ "regex_scripts": global_scripts }));
            }

            for child in map.values() {
                if let Some(found) = find_regex_extensions(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(values) => values.iter().find_map(find_regex_extensions),
        _ => None,
    }
}

fn normalize_global_sources(scripts: &mut [RegexScript]) {
    for script in scripts {
        script.source = RegexScriptSource::Global;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::preset::{RegexPlacement, RegexScript};

    fn script(id: &str, source: RegexScriptSource) -> RegexScript {
        RegexScript {
            id: id.to_string(),
            script_name: id.to_string(),
            find_regex: id.to_string(),
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
    fn replace_all_persists_and_marks_scripts_global() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_global_regex_store_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let store = GlobalRegexStore::new(&dir);
        assert_eq!(
            store
                .replace_all(vec![script("global-1", RegexScriptSource::Scoped)])
                .unwrap(),
            1
        );

        let reloaded = GlobalRegexStore::new(&dir);
        let scripts = reloaded.list();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "global-1");
        assert_eq!(scripts[0].source, RegexScriptSource::Global);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_from_settings_json_extracts_nested_regex_scripts() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_global_regex_import_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let store = GlobalRegexStore::new(&dir);
        let count = store
            .import_from_settings_json(
                r#"{
                    "extension_settings": {
                        "regex": {
                            "regex_scripts": [{
                                "id": "settings-global",
                                "scriptName": "Settings global",
                                "findRegex": "foo",
                                "replaceString": "bar",
                                "placement": [2],
                                "disabled": false
                            }]
                        }
                    }
                }"#,
            )
            .unwrap();

        let scripts = store.list();
        assert_eq!(count, 1);
        assert_eq!(scripts[0].id, "settings-global");
        assert_eq!(scripts[0].source, RegexScriptSource::Global);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_without_regex_scripts_keeps_existing_scripts() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_global_regex_missing_import_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let store = GlobalRegexStore::new(&dir);
        store
            .replace_all(vec![script("existing-global", RegexScriptSource::Global)])
            .unwrap();

        let result = store.import_from_settings_json(r#"{ "extension_settings": {} }"#);

        assert!(result.is_err());
        let scripts = store.list();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "existing-global");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_recovers_from_tmp_and_marks_scripts_global_without_corrupt_backup() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_global_regex_tmp_recovery_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("global_regex_scripts.json");
        std::fs::write(&path, "{ invalid").unwrap();
        std::fs::write(
            path.with_extension("json.tmp"),
            serde_json::to_string(&vec![script("tmp-scoped", RegexScriptSource::Scoped)]).unwrap(),
        )
        .unwrap();

        let store = GlobalRegexStore::new(&dir);
        let scripts = store.list();

        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "tmp-scoped");
        assert_eq!(scripts[0].source, RegexScriptSource::Global);
        assert!(!path.with_extension("json.corrupt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

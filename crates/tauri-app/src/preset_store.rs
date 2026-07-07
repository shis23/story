use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::storage::json_store;
use storyforge_domain::preset::Preset;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPreset {
    pub id: String,
    pub preset: Preset,
    pub imported_at: String,
}

pub struct PresetStore {
    path: PathBuf,
    active_path: PathBuf,
    inner: Mutex<Vec<StoredPreset>>,
    active_id: Mutex<Option<String>>,
}

impl PresetStore {
    pub fn new(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("presets.json");
        let active_path = app_data_dir.join("active_preset.json");
        let presets: Vec<StoredPreset> = json_store::load_json_with_tmp_backup_or_default(
            &path,
            |e| tracing::warn!("preset JSON parse failed ({e}), trying .tmp backup"),
            |path, e| {
                tracing::error!(
                    "preset JSON and .tmp backup are corrupt, file: {}, error: {}. saved .corrupt backup",
                    path.display(),
                    e
                )
            },
        );
        let active_id = if active_path.exists() {
            match std::fs::read_to_string(&active_path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
                    tracing::error!(
                        "active preset JSON parse failed, file: {}, error: {}. saved .corrupt backup",
                        active_path.display(),
                        e
                    );
                    let _ = std::fs::copy(&active_path, active_path.with_extension("json.corrupt"));
                    None
                }),
                Err(e) => {
                    tracing::warn!("active preset file read failed ({e}), returning None");
                    None
                }
            }
        } else {
            None
        };

        Self {
            path,
            active_path,
            inner: Mutex::new(presets),
            active_id: Mutex::new(active_id),
        }
    }

    pub fn save(&self, preset: Preset) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let stored = StoredPreset {
            id: id.clone(),
            preset,
            imported_at: now,
        };
        let mut presets = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        presets.push(stored);
        self.persist(&presets)?;
        Ok(id)
    }

    pub fn list(&self) -> Vec<StoredPreset> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn get(&self, id: &str) -> Option<StoredPreset> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub fn active_id(&self) -> Option<String> {
        self.active_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn active(&self) -> Option<StoredPreset> {
        let active_id = self.active_id()?;
        self.get(&active_id)
    }

    pub fn set_active(&self, id: &str) -> Result<bool, String> {
        let exists = self
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .any(|p| p.id == id);
        if !exists {
            return Ok(false);
        }

        {
            let mut active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
            *active_id = Some(id.to_string());
        }
        self.persist_active()?;
        Ok(true)
    }

    pub fn clear_active(&self) -> Result<(), String> {
        {
            let mut active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
            *active_id = None;
        }
        self.persist_active()
    }

    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let mut presets = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let before = presets.len();
        presets.retain(|p| p.id != id);
        if presets.len() < before {
            self.persist(&presets)?;
            if self.active_id().as_deref() == Some(id) {
                self.clear_active()?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn update_prompt(
        &self,
        preset_id: &str,
        prompt_index: usize,
        content: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<bool, String> {
        let mut presets = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(stored) = presets.iter_mut().find(|p| p.id == preset_id)
            && let Some(prompt) = stored.preset.prompts.get_mut(prompt_index)
        {
            if let Some(c) = content {
                prompt.content = c.to_string();
            }
            if let Some(e) = enabled {
                prompt.enabled = e;
            }
            self.persist(&presets)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn update_regex(
        &self,
        preset_id: &str,
        regex_index: usize,
        disabled: Option<bool>,
    ) -> Result<bool, String> {
        let mut presets = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(stored) = presets.iter_mut().find(|p| p.id == preset_id)
            && let Some(regex) = stored.preset.regex_scripts.get_mut(regex_index)
        {
            if let Some(d) = disabled {
                regex.disabled = d;
            }
            self.persist(&presets)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn persist(&self, presets: &[StoredPreset]) -> Result<(), String> {
        storyforge_infra_util::atomic_write_json(&self.path, presets).map_err(|e| {
            let msg = format!("persist preset failed: {e}");
            tracing::error!("{msg}");
            msg
        })
    }

    fn persist_active(&self) -> Result<(), String> {
        let active_id = self.active_id();
        let json = serde_json::to_string(&active_id).unwrap_or_else(|_| "null".into());
        storyforge_infra_util::atomic_write_json_str(&self.active_path, &json).map_err(|e| {
            let msg = format!("persist active preset failed: {e}");
            tracing::error!("{msg}");
            msg
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Source;
    use storyforge_domain::preset::{Preset, RegexPlacement, RegexScript, RegexScriptSource};

    fn make_preset(name: &str, regex_id: &str) -> Preset {
        Preset {
            name: name.to_string(),
            prompts: vec![],
            regex_scripts: vec![RegexScript {
                id: regex_id.to_string(),
                script_name: regex_id.to_string(),
                find_regex: regex_id.to_string(),
                replace_string: String::new(),
                placement: RegexPlacement::Output,
                placement_codes: vec![2],
                source: RegexScriptSource::Preset,
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
            }],
            source: Source::ImportedFromST,
        }
    }

    #[test]
    fn active_preset_is_validated_persisted_and_clearable() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_preset_store_active_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let store = PresetStore::new(&dir);
        let first_id = store.save(make_preset("first", "first-regex")).unwrap();
        let second_id = store.save(make_preset("second", "second-regex")).unwrap();

        assert!(store.set_active(&first_id).unwrap());
        assert_eq!(store.active_id().as_deref(), Some(first_id.as_str()));
        assert_eq!(store.active().unwrap().id, first_id);

        assert!(!store.set_active("missing-preset").unwrap());
        assert_eq!(store.active_id().as_deref(), Some(first_id.as_str()));

        let reloaded = PresetStore::new(&dir);
        assert_eq!(reloaded.active().unwrap().id, first_id);

        assert!(reloaded.set_active(&second_id).unwrap());
        assert_eq!(reloaded.active().unwrap().id, second_id);

        reloaded.clear_active().unwrap();
        assert!(PresetStore::new(&dir).active().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_active_preset_clears_active_marker() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_preset_store_delete_active_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let store = PresetStore::new(&dir);
        let id = store.save(make_preset("active", "active-regex")).unwrap();
        assert!(store.set_active(&id).unwrap());

        assert!(store.delete(&id).unwrap());
        assert!(store.active().is_none());
        assert!(PresetStore::new(&dir).active().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

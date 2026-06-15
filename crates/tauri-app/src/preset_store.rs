use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

use storyforge_domain::preset::Preset;

/// 已存储的预设（带 id 和导入时间）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPreset {
    pub id: String,
    pub preset: Preset,
    pub imported_at: String,
}

/// 预设存储（JSON 文件）
pub struct PresetStore {
    path: PathBuf,
    inner: Mutex<Vec<StoredPreset>>,
}

impl PresetStore {
    pub fn new(app_data_dir: &PathBuf) -> Self {
        let path = app_data_dir.join("presets.json");
        let presets = if path.exists() {
            let data = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            path,
            inner: Mutex::new(presets),
        }
    }

    /// 保存预设（导入时调用），返回 id
    pub fn save(&self, preset: Preset) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let stored = StoredPreset {
            id: id.clone(),
            preset,
            imported_at: now,
        };
        let mut presets = self.inner.lock().unwrap();
        presets.push(stored);
        self.persist(&presets);
        id
    }

    /// 列出所有预设（元数据）
    pub fn list(&self) -> Vec<StoredPreset> {
        self.inner.lock().unwrap().clone()
    }

    /// 获取单个预设详情
    pub fn get(&self, id: &str) -> Option<StoredPreset> {
        self.inner.lock().unwrap().iter().find(|p| p.id == id).cloned()
    }

    /// 删除预设
    pub fn delete(&self, id: &str) -> bool {
        let mut presets = self.inner.lock().unwrap();
        let before = presets.len();
        presets.retain(|p| p.id != id);
        if presets.len() < before {
            self.persist(&presets);
            true
        } else {
            false
        }
    }

    /// 更新单条 prompt 的内容和/或启用状态
    pub fn update_prompt(&self, preset_id: &str, prompt_index: usize, content: Option<&str>, enabled: Option<bool>) -> bool {
        let mut presets = self.inner.lock().unwrap();
        if let Some(stored) = presets.iter_mut().find(|p| p.id == preset_id) {
            if let Some(prompt) = stored.preset.prompts.get_mut(prompt_index) {
                if let Some(c) = content {
                    prompt.content = c.to_string();
                }
                if let Some(e) = enabled {
                    prompt.enabled = e;
                }
                self.persist(&presets);
                return true;
            }
        }
        false
    }

    /// 更新单条 regex 的禁用状态
    pub fn update_regex(&self, preset_id: &str, regex_index: usize, disabled: Option<bool>) -> bool {
        let mut presets = self.inner.lock().unwrap();
        if let Some(stored) = presets.iter_mut().find(|p| p.id == preset_id) {
            if let Some(regex) = stored.preset.regex_scripts.get_mut(regex_index) {
                if let Some(d) = disabled {
                    regex.disabled = d;
                }
                self.persist(&presets);
                return true;
            }
        }
        false
    }

    fn persist(&self, presets: &[StoredPreset]) {
        if let Err(e) = storyforge_infra_util::atomic_write_json(&self.path, presets) {
            tracing::error!("持久化预设失败: {e}");
        }
    }
}

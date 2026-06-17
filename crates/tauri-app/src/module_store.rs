use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

use storyforge_domain::Id;
use storyforge_domain::agent_profile_config::{
    AgentProfileConfig, AgentProfileConfigSummaryDto, BUILTIN_DEFAULT_AGENT_PROFILE_ID,
    default_agent_profile_config,
};
use storyforge_domain::prompt_module::{
    ModuleCategory, ModuleSource, ProfileSource, PromptModule, PromptProfile,
};

// ─── DTO（前端友好的序列化结构）──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptModuleDto {
    pub id: String,
    pub name: String,
    pub category: String,
    pub content: String,
    pub exclusivity: String,
    pub source: String,
    pub applicable_roles: Vec<String>,
    pub tags: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSummaryDto {
    pub id: String,
    pub name: String,
    pub source: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptProfileDto {
    pub id: String,
    pub name: String,
    pub selections:
        std::collections::HashMap<String, std::collections::HashMap<String, Vec<String>>>,
    pub overrides: std::collections::HashMap<String, Option<String>>,
    pub source: String,
}

pub fn module_to_dto(m: &PromptModule, enabled: bool) -> PromptModuleDto {
    PromptModuleDto {
        id: m.id.to_string(),
        name: m.name.clone(),
        category: format!("{:?}", m.category),
        content: m.content.clone(),
        exclusivity: format!("{:?}", m.exclusivity),
        source: format!("{:?}", m.source),
        applicable_roles: m
            .applicable_roles
            .iter()
            .map(|r| format!("{r:?}"))
            .collect(),
        tags: m.tags.clone(),
        enabled,
    }
}

pub fn profile_to_dto(p: &PromptProfile, _is_active: bool) -> PromptProfileDto {
    let mut selections = std::collections::HashMap::new();
    for (role, cats) in &p.selections {
        let role_key = format!("{role:?}");
        let mut cat_map = std::collections::HashMap::new();
        for (cat, ids) in cats {
            cat_map.insert(
                format!("{cat:?}"),
                ids.iter().map(|id| id.to_string()).collect(),
            );
        }
        selections.insert(role_key, cat_map);
    }
    let overrides = p
        .overrides
        .iter()
        .map(|(k, v)| (format!("{k:?}"), v.clone()))
        .collect();
    PromptProfileDto {
        id: p.id.to_string(),
        name: p.name.clone(),
        selections,
        overrides,
        source: format!("{:?}", p.source),
    }
}

fn profile_summary_dto(p: &PromptProfile, is_active: bool) -> ProfileSummaryDto {
    ProfileSummaryDto {
        id: p.id.to_string(),
        name: p.name.clone(),
        source: format!("{:?}", p.source),
        is_active,
    }
}

// ─── ModuleStore（模块存储）──────────────────────────────────────────────────

/// 模块存储：内置模块 + 用户模块 + ST 导入模块
pub struct ModuleStore {
    // 内置模块（不可修改，只读）
    builtins: Vec<PromptModule>,
    // 用户/导入的模块（可 CRUD）
    custom: Mutex<Vec<PromptModule>>,
    // 禁用的模块 ID 列表（内置模块可以通过这个禁用）
    disabled: Mutex<Vec<String>>,
    custom_path: PathBuf,
    disabled_path: PathBuf,
}

impl ModuleStore {
    pub fn new(app_data_dir: &PathBuf) -> Self {
        let custom_path = app_data_dir.join("custom_modules.json");
        let disabled_path = app_data_dir.join("disabled_modules.json");

        let custom: Vec<PromptModule> = if custom_path.exists() {
            std::fs::read_to_string(&custom_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let disabled: Vec<String> = if disabled_path.exists() {
            std::fs::read_to_string(&disabled_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Self {
            builtins: storyforge_domain::prompt_module::builtins::preset_modules(),
            custom: Mutex::new(custom),
            disabled: Mutex::new(disabled),
            custom_path,
            disabled_path,
        }
    }

    /// 列出所有模块（内置 + 自定义），附带 enabled 状态
    pub fn list_all(&self) -> Vec<(PromptModule, bool)> {
        let disabled = self.disabled.lock().unwrap_or_else(|p| p.into_inner());
        let custom = self.custom.lock().unwrap_or_else(|p| p.into_inner());
        let mut result = Vec::new();

        for m in &self.builtins {
            let enabled = !disabled.contains(&m.id.to_string());
            result.push((m.clone(), enabled));
        }
        for m in custom.iter() {
            let enabled = !disabled.contains(&m.id.to_string());
            result.push((m.clone(), enabled));
        }
        result
    }

    /// 添加自定义模块
    pub fn add(&self, module: PromptModule) {
        let mut custom = self.custom.lock().unwrap_or_else(|p| p.into_inner());
        custom.push(module);
        drop(custom);
        self.persist_custom();
    }

    /// 更新模块内容（仅自定义模块可改内容；内置模块只能改 enabled）
    pub fn update(&self, id: &str, content: Option<&str>, enabled: Option<bool>) -> bool {
        // 处理 enabled 状态
        if let Some(enabled) = enabled {
            let mut disabled = self.disabled.lock().unwrap_or_else(|p| p.into_inner());
            if enabled {
                disabled.retain(|d| d != id);
            } else if !disabled.contains(&id.to_string()) {
                disabled.push(id.to_string());
            }
            drop(disabled);
            self.persist_disabled();
        }

        // 处理内容更新（仅自定义模块）
        if let Some(content) = content {
            let mut custom = self.custom.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(m) = custom.iter_mut().find(|m| m.id.to_string() == id) {
                m.content = content.to_string();
                drop(custom);
                self.persist_custom();
                return true;
            }
            return false; // 内置模块不能改内容
        }

        true
    }

    /// 删除自定义模块
    pub fn delete(&self, id: &str) -> bool {
        let mut custom = self.custom.lock().unwrap_or_else(|p| p.into_inner());
        let before = custom.len();
        custom.retain(|m| m.id.to_string() != id);
        if custom.len() < before {
            drop(custom);
            self.persist_custom();
            true
        } else {
            false // 内置模块不能删
        }
    }

    /// 获取单个模块
    pub fn get(&self, id: &str) -> Option<(PromptModule, bool)> {
        let disabled = self.disabled.lock().unwrap_or_else(|p| p.into_inner());
        let enabled = !disabled.contains(&id.to_string());

        if let Some(m) = self.builtins.iter().find(|m| m.id.to_string() == id) {
            return Some((m.clone(), enabled));
        }

        let custom = self.custom.lock().unwrap_or_else(|p| p.into_inner());
        custom
            .iter()
            .find(|m| m.id.to_string() == id)
            .map(|m| (m.clone(), enabled))
    }

    fn persist_custom(&self) {
        let custom = self.custom.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(e) = storyforge_infra_util::atomic_write_json(&self.custom_path, &*custom) {
            tracing::error!("持久化自定义模块失败: {e}");
        }
    }

    fn persist_disabled(&self) {
        let disabled = self.disabled.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(e) = storyforge_infra_util::atomic_write_json(&self.disabled_path, &*disabled) {
            tracing::error!("持久化禁用模块列表失败: {e}");
        }
    }
}

// ─── ProfileStore（Profile 存储）────────────────────────────────────────────

/// Profile 存储
pub struct ProfileStore {
    profiles: Mutex<Vec<PromptProfile>>,
    active_id: Mutex<Option<String>>,
    profiles_path: PathBuf,
    active_path: PathBuf,
}

impl ProfileStore {
    pub fn new(app_data_dir: &PathBuf) -> Self {
        let profiles_path = app_data_dir.join("profiles.json");
        let active_path = app_data_dir.join("active_profile.json");

        let profiles: Vec<PromptProfile> = if profiles_path.exists() {
            std::fs::read_to_string(&profiles_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let active_id: Option<String> = if active_path.exists() {
            std::fs::read_to_string(&active_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
        } else {
            None
        };

        Self {
            profiles: Mutex::new(profiles),
            active_id: Mutex::new(active_id),
            profiles_path,
            active_path,
        }
    }

    /// 列出所有 Profile
    pub fn list(&self) -> Vec<ProfileSummaryDto> {
        let profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());
        let active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
        profiles
            .iter()
            .map(|p| {
                let is_active = active_id.as_deref() == Some(&p.id.to_string());
                profile_summary_dto(p, is_active)
            })
            .collect()
    }

    /// 获取当前活跃 Profile（如果没有用户选的，返回内置默认）
    pub fn get_active(&self) -> Option<PromptProfile> {
        let active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
        let profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());

        if let Some(ref aid) = *active_id {
            if let Some(p) = profiles.iter().find(|p| &p.id.to_string() == aid) {
                return Some(p.clone());
            }
        }

        // 回退：找内置默认
        profiles
            .iter()
            .find(|p| p.source == ProfileSource::BuiltIn)
            .cloned()
    }

    /// 获取指定 Profile
    pub fn get(&self, id: &str) -> Option<PromptProfileDto> {
        let profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());
        let active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
        profiles
            .iter()
            .find(|p| p.id.to_string() == id)
            .map(|p| profile_to_dto(p, active_id.as_deref() == Some(id)))
    }

    /// 保存/更新 Profile
    pub fn save(&self, profile: PromptProfile) {
        let mut profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
        } else {
            profiles.push(profile);
        }
        drop(profiles);
        self.persist();
    }

    /// 设置活跃 Profile
    pub fn set_active(&self, id: &str) {
        let json = {
            let mut active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
            *active_id = Some(id.to_string());
            serde_json::to_string(&*active_id).unwrap_or_else(|_| "null".into())
        };
        if let Err(e) = storyforge_infra_util::atomic_write_json_str(&self.active_path, &json) {
            tracing::error!("持久化活跃 Profile 失败: {e}");
        }
    }

    /// 删除 Profile
    pub fn delete(&self, id: &str) -> bool {
        let mut profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());
        let before = profiles.len();
        profiles.retain(|p| p.id.to_string() != id);
        if profiles.len() < before {
            drop(profiles);
            self.persist();
            // 如果删的是活跃 Profile，清除活跃标记
            let mut active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
            if active_id.as_deref() == Some(id) {
                *active_id = None;
                drop(active_id);
                if let Err(e) =
                    storyforge_infra_util::atomic_write_json_str(&self.active_path, "null")
                {
                    tracing::error!("清除活跃 Profile 失败: {e}");
                }
            }
            true
        } else {
            false
        }
    }

    /// 确保至少有一个默认 Profile（启动时调用）
    pub fn ensure_default(&self) {
        let profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());
        let has_any = !profiles.is_empty();
        drop(profiles);
        if !has_any {
            let (default_profile, _) =
                storyforge_domain::prompt_module::builtins::default_profile();
            self.save(default_profile);
        }
    }

    fn persist(&self) {
        let profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(e) = storyforge_infra_util::atomic_write_json(&self.profiles_path, &*profiles) {
            tracing::error!("持久化 Profile 失败: {e}");
        }
    }
}

// ─── AgentProfileConfigStore（Agent Profile 配置存储）────────────────────────

/// Agent Profile 配置存储
///
/// 管理用户的 Agent 运行时配置（模型覆盖、轮次、并发等）。
/// 内置默认配置始终可用，不可删除。
pub struct AgentProfileConfigStore {
    configs: Mutex<Vec<AgentProfileConfig>>,
    active_id: Mutex<Option<String>>,
    configs_path: PathBuf,
    active_path: PathBuf,
}

impl AgentProfileConfigStore {
    pub fn new(app_data_dir: &PathBuf) -> Self {
        let configs_path = app_data_dir.join("agent_profile_configs.json");
        let active_path = app_data_dir.join("active_agent_profile_config.json");

        let mut configs: Vec<AgentProfileConfig> = if configs_path.exists() {
            std::fs::read_to_string(&configs_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for config in &mut configs {
            config.sanitize();
        }

        let active_id: Option<String> = if active_path.exists() {
            std::fs::read_to_string(&active_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
        } else {
            None
        };

        let store = Self {
            configs: Mutex::new(configs),
            active_id: Mutex::new(active_id),
            configs_path,
            active_path,
        };
        // 确保内置默认始终存在
        store.ensure_builtin_default();
        store
    }

    /// 列出所有配置（摘要 DTO）
    pub fn list(&self) -> Vec<AgentProfileConfigSummaryDto> {
        let configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
        let active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
        configs
            .iter()
            .map(|c| {
                let is_active = active_id.as_deref() == Some(&c.id.to_string());
                AgentProfileConfigSummaryDto {
                    id: c.id.to_string(),
                    name: c.name.clone(),
                    description: c.description.clone(),
                    source: format!("{:?}", c.source),
                    is_active,
                    max_concurrent_subagents: c.max_concurrent_subagents,
                    enable_postprocess: c.enable_postprocess,
                    enable_summarizer: c.enable_summarizer,
                }
            })
            .collect()
    }

    /// 获取指定配置（完整 DTO）
    pub fn get(&self, id: &str) -> Option<AgentProfileConfig> {
        let configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
        configs.iter().find(|c| c.id.to_string() == id).cloned()
    }

    /// 获取当前活跃配置（如果没有用户选的，返回内置默认）
    pub fn get_active(&self) -> AgentProfileConfig {
        let active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
        let configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());

        if let Some(ref aid) = *active_id {
            if let Some(c) = configs.iter().find(|c| &c.id.to_string() == aid) {
                return c.clone();
            }
        }

        // 回退：找内置默认
        configs
            .iter()
            .find(|c| c.is_builtin())
            .cloned()
            .unwrap_or_else(default_agent_profile_config)
    }

    /// 保存/更新配置
    ///
    /// 内置默认配置不允许覆盖。
    pub fn save(&self, mut config: AgentProfileConfig) -> Result<(), String> {
        config.sanitize();
        if config.id.to_string() == BUILTIN_DEFAULT_AGENT_PROFILE_ID {
            // 检查是否已存在内置默认
            let configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
            let exists = configs
                .iter()
                .any(|c| c.id.to_string() == BUILTIN_DEFAULT_AGENT_PROFILE_ID);
            drop(configs);
            if exists {
                return Err("内置默认配置不可覆盖".into());
            }
        }
        let mut configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = configs
            .iter_mut()
            .find(|c| c.id.to_string() == config.id.to_string())
        {
            *existing = config;
        } else {
            configs.push(config);
        }
        drop(configs);
        self.persist_configs();
        Ok(())
    }

    /// 设置活跃配置
    pub fn set_active(&self, id: &str) -> Result<(), String> {
        // 验证配置存在
        {
            let configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
            if !configs.iter().any(|c| c.id.to_string() == id) {
                return Err(format!("配置 {id} 不存在"));
            }
        }
        let json = {
            let mut active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
            *active_id = Some(id.to_string());
            serde_json::to_string(&*active_id).unwrap_or_else(|_| "null".into())
        };
        if let Err(e) = storyforge_infra_util::atomic_write_json_str(&self.active_path, &json) {
            tracing::error!("持久化活跃 Agent Profile Config 失败: {e}");
        }
        Ok(())
    }

    /// 删除配置
    ///
    /// 内置默认配置不可删除。
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        if id == BUILTIN_DEFAULT_AGENT_PROFILE_ID {
            return Err("内置默认配置不可删除".into());
        }
        let mut configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
        let before = configs.len();
        configs.retain(|c| c.id.to_string() != id);
        if configs.len() < before {
            drop(configs);
            self.persist_configs();
            // 如果删的是活跃配置，清除活跃标记（下次 get_active 回退到内置默认）
            let mut active_id = self.active_id.lock().unwrap_or_else(|p| p.into_inner());
            if active_id.as_deref() == Some(id) {
                *active_id = None;
                drop(active_id);
                if let Err(e) =
                    storyforge_infra_util::atomic_write_json_str(&self.active_path, "null")
                {
                    tracing::error!("清除活跃 Agent Profile Config 失败: {e}");
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 确保内置默认配置存在（启动时调用）
    fn ensure_builtin_default(&self) {
        let mut configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
        let has_builtin = configs
            .iter()
            .any(|c| c.id.to_string() == BUILTIN_DEFAULT_AGENT_PROFILE_ID);
        if !has_builtin {
            configs.push(default_agent_profile_config());
            drop(configs);
            self.persist_configs();
        }
    }

    fn persist_configs(&self) {
        let configs = self.configs.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(e) = storyforge_infra_util::atomic_write_json(&self.configs_path, &*configs) {
            tracing::error!("持久化 Agent Profile Config 失败: {e}");
        }
    }
}

#[cfg(test)]
mod agent_profile_config_store_tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_agent_profile_store_test_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_sanitizes_zero_concurrency_from_disk() {
        let dir = temp_dir();
        let legacy_configs = serde_json::json!([
            {
                "id": "legacy-zero",
                "name": "Legacy Zero",
                "max_concurrent_subagents": 0
            }
        ]);
        std::fs::write(
            dir.join("agent_profile_configs.json"),
            legacy_configs.to_string(),
        )
        .unwrap();

        let store = AgentProfileConfigStore::new(&dir);
        let cfg = store.get("legacy-zero").unwrap();
        assert_eq!(cfg.max_concurrent_subagents, 1);
        assert_eq!(cfg.effective_max_concurrent_subagents(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

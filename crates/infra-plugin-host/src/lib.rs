/// 插件运行时（对应设计 §8）
///
/// 核心能力：
/// - PluginManifest：插件清单（权限声明 + 入口 HTML + UI 挂载点）
/// - Permission：权限模型（声明式 + 后端二次校验）
/// - UiSlot：UI 挂载点枚举
/// - PluginRegistry：插件注册表（CRUD + 权限校验）
///
/// 前端侧的 iframe 沙箱宿主 + postMessage API 桥在 JS 层实现，
/// 本 crate 定义后端的权限校验和插件管理逻辑。
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

// ─── 权限模型（对应设计 §8.2）───────────────────────────────────────────────

/// 插件权限（声明式，manifest 中声明）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// 读取角色卡
    ReadCharacters,
    /// 读取世界书
    ReadWorldInfo,
    /// 读取记忆
    ReadMemory,
    /// 读写变量
    WriteVariables,
    /// 调用 LLM
    CallLlm,
    /// 网络请求
    Network,
    /// 通知
    Notifications,
}

// ─── UI 挂载点（对应设计 §8.3）─────────────────────────────────────────────

/// 插件 UI 挂载位置
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UiSlot {
    /// 消息装饰（在消息下方添加内容）
    MessageDecorator,
    /// 侧栏面板
    SidebarPanel,
    /// Meta Agent 工具栏
    MetaAgentToolbar,
    /// 输入框扩展
    ComposerAddon,
}

// ─── 插件清单（对应设计 §5 PluginManifest）─────────────────────────────────

/// 插件清单
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// 插件 ID（唯一标识）
    pub id: String,
    /// 插件名称
    pub name: String,
    /// 版本号
    pub version: String,
    /// 声明的权限列表
    pub permissions: Vec<Permission>,
    /// 入口 HTML（srcdoc 内容）
    pub entry_html: String,
    /// 要挂载的 UI 位置
    pub ui_slots: Vec<UiSlot>,
    /// 订阅的事件名列表
    pub event_subscriptions: Vec<String>,
    /// 插件描述
    pub description: Option<String>,
    /// 作者
    pub author: Option<String>,
}

// ─── 插件注册表 ────────────────────────────────────────────────────────────

/// 已安装的插件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    /// 安装时间
    pub installed_at: chrono::DateTime<chrono::Utc>,
    /// 是否启用
    pub enabled: bool,
}

/// 插件注册表（管理已安装的插件）
pub struct PluginRegistry {
    plugins: RwLock<HashMap<String, InstalledPlugin>>,
    persist_path: Option<std::path::PathBuf>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: RwLock::new(HashMap::new()),
            persist_path: None,
        }
    }

    /// 带持久化的构造
    pub fn with_persistence(path: std::path::PathBuf) -> Self {
        let plugins = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        Self {
            plugins: RwLock::new(plugins),
            persist_path: Some(path),
        }
    }

    /// 安装插件
    pub fn install(&self, manifest: PluginManifest) -> Result<(), PluginError> {
        let mut plugins = self.plugins.write().unwrap();

        // 检查是否已安装
        if plugins.contains_key(&manifest.id) {
            return Err(PluginError::AlreadyInstalled(manifest.id));
        }

        let plugin = InstalledPlugin {
            manifest,
            installed_at: chrono::Utc::now(),
            enabled: true,
        };

        plugins.insert(plugin.manifest.id.clone(), plugin);
        drop(plugins);
        self.persist();
        Ok(())
    }

    /// 卸载插件
    pub fn uninstall(&self, id: &str) -> Result<(), PluginError> {
        let mut plugins = self.plugins.write().unwrap();
        plugins.remove(id).ok_or_else(|| PluginError::NotFound(id.into()))?;
        drop(plugins);
        self.persist();
        Ok(())
    }

    /// 启用/禁用插件
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), PluginError> {
        let mut plugins = self.plugins.write().unwrap();
        let plugin = plugins.get_mut(id).ok_or_else(|| PluginError::NotFound(id.into()))?;
        plugin.enabled = enabled;
        drop(plugins);
        self.persist();
        Ok(())
    }

    /// 获取插件
    pub fn get(&self, id: &str) -> Option<InstalledPlugin> {
        self.plugins.read().unwrap().get(id).cloned()
    }

    /// 列出所有插件
    pub fn list(&self) -> Vec<InstalledPlugin> {
        self.plugins.read().unwrap().values().cloned().collect()
    }

    /// 列出已启用的插件
    pub fn list_enabled(&self) -> Vec<InstalledPlugin> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .cloned()
            .collect()
    }

    /// 校验权限（后端二次校验，对应设计 §8.2）
    pub fn ensure_permission(&self, plugin_id: &str, permission: &Permission) -> Result<(), PluginError> {
        let plugins = self.plugins.read().unwrap();
        let plugin = plugins.get(plugin_id).ok_or_else(|| PluginError::NotFound(plugin_id.into()))?;

        if !plugin.enabled {
            return Err(PluginError::Disabled(plugin_id.into()));
        }

        if !plugin.manifest.permissions.contains(permission) {
            return Err(PluginError::PermissionDenied {
                plugin_id: plugin_id.into(),
                permission: permission.clone(),
            });
        }

        Ok(())
    }

    /// 获取在指定 UI 挂载点的插件列表
    pub fn plugins_for_slot(&self, slot: &UiSlot) -> Vec<InstalledPlugin> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled && p.manifest.ui_slots.contains(slot))
            .cloned()
            .collect()
    }

    fn persist(&self) {
        if let Some(path) = &self.persist_path {
            let plugins = self.plugins.read().unwrap();
            if let Ok(json) = serde_json::to_string_pretty(&*plugins) {
                let _ = std::fs::write(path, json);
            }
        }
    }
}

// ─── 错误类型 ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("插件不存在: {0}")]
    NotFound(String),

    #[error("插件已安装: {0}")]
    AlreadyInstalled(String),

    #[error("插件已禁用: {0}")]
    Disabled(String),

    #[error("权限被拒绝: 插件 {plugin_id} 无权 {permission:?}")]
    PermissionDenied {
        plugin_id: String,
        permission: Permission,
    },

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),
}

// ─── ST → 我们的 API 映射（对应设计 §8.5）──────────────────────────────────

/// ST 扩展 API 到 StoryForge API 的映射关系
/// 用于 Meta Agent 插件生成（M5）
pub const ST_API_MAPPING: &[(&str, &str, &str)] = &[
    ("SillyTavern.getContext()", "storyforge.character.getCurrent() + storyforge.worldInfo.search()", "拆分成多个细粒度 API"),
    ("getContext().characters", "storyforge.character.list() / .get(id)", ""),
    ("getContext().chat", "storyforge.memory.getRecent()", "走记忆系统"),
    ("eventTypes.MESSAGE_RECEIVED", "storyforge.events.on('message.finalized', cb)", "事件名重定义"),
    ("eventTypes.GENERATION_STARTED", "storyforge.events.on('pipeline.state_changed', cb)", ""),
    ("setLocalVar(key, val)", "storyforge.variables.set(key, val)", "需 WriteVariables 权限"),
    ("getLocalVar(key)", "storyforge.variables.get(key)", ""),
    ("replaceVariables(msg)", "（不暴露，变量替换由后端统一做）", ""),
    ("triggerSlash('/genraw ...')", "storyforge.llm.generate(prompt)", "需 CallLlm 权限"),
    ("$('#chat').append(html)", "storyforge.ui.mountToSlot('message_decorator', el)", "DOM → UI slot"),
    ("extension_settings[myExt]", "storyforge.storage.get/set", "持久化"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest(id: &str, permissions: Vec<Permission>) -> PluginManifest {
        PluginManifest {
            id: id.into(),
            name: format!("Test Plugin {id}"),
            version: "1.0.0".into(),
            permissions,
            entry_html: "<h1>Hello</h1>".into(),
            ui_slots: vec![UiSlot::SidebarPanel],
            event_subscriptions: vec![],
            description: None,
            author: None,
        }
    }

    #[test]
    fn test_install_and_list() {
        let registry = PluginRegistry::new();
        registry.install(make_manifest("p1", vec![])).unwrap();
        registry.install(make_manifest("p2", vec![Permission::ReadCharacters])).unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_install_duplicate() {
        let registry = PluginRegistry::new();
        registry.install(make_manifest("p1", vec![])).unwrap();
        let result = registry.install(make_manifest("p1", vec![]));
        assert!(matches!(result, Err(PluginError::AlreadyInstalled(_))));
    }

    #[test]
    fn test_uninstall() {
        let registry = PluginRegistry::new();
        registry.install(make_manifest("p1", vec![])).unwrap();
        registry.uninstall("p1").unwrap();
        assert!(registry.get("p1").is_none());
    }

    #[test]
    fn test_enable_disable() {
        let registry = PluginRegistry::new();
        registry.install(make_manifest("p1", vec![])).unwrap();

        registry.set_enabled("p1", false).unwrap();
        assert!(!registry.get("p1").unwrap().enabled);

        registry.set_enabled("p1", true).unwrap();
        assert!(registry.get("p1").unwrap().enabled);
    }

    #[test]
    fn test_permission_check() {
        let registry = PluginRegistry::new();
        registry.install(make_manifest("p1", vec![Permission::ReadCharacters])).unwrap();

        // 有权限
        assert!(registry.ensure_permission("p1", &Permission::ReadCharacters).is_ok());

        // 无权限
        assert!(registry.ensure_permission("p1", &Permission::WriteVariables).is_err());
    }

    #[test]
    fn test_permission_check_disabled_plugin() {
        let registry = PluginRegistry::new();
        registry.install(make_manifest("p1", vec![Permission::ReadCharacters])).unwrap();
        registry.set_enabled("p1", false).unwrap();

        let result = registry.ensure_permission("p1", &Permission::ReadCharacters);
        assert!(matches!(result, Err(PluginError::Disabled(_))));
    }

    #[test]
    fn test_plugins_for_slot() {
        let registry = PluginRegistry::new();

        let mut manifest1 = make_manifest("p1", vec![]);
        manifest1.ui_slots = vec![UiSlot::SidebarPanel];
        registry.install(manifest1).unwrap();

        let mut manifest2 = make_manifest("p2", vec![]);
        manifest2.ui_slots = vec![UiSlot::MessageDecorator];
        registry.install(manifest2).unwrap();

        let sidebar_plugins = registry.plugins_for_slot(&UiSlot::SidebarPanel);
        assert_eq!(sidebar_plugins.len(), 1);
        assert_eq!(sidebar_plugins[0].manifest.id, "p1");
    }

    #[test]
    fn test_st_api_mapping_exists() {
        assert!(!ST_API_MAPPING.is_empty());
        assert!(ST_API_MAPPING.iter().all(|(st, sf, _)| !st.is_empty() && !sf.is_empty()));
    }
}

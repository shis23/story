/// 插件运行时（对应设计 §8）
///
/// 核心能力：
/// - PluginManifest：插件清单（权限声明 + 入口 HTML + UI 挂载点）
/// - Permission：权限模型（声明式 + 后端二次校验）
/// - UiSlot：UI 挂载点枚举
/// - PluginRegistry：插件注册表（CRUD + 权限校验）
/// - mvu_runtime：MVU 兜底执行运行时 trait（P3 新增，当前为桩）
///
/// 前端侧的 iframe 沙箱宿主 + postMessage API 桥在 JS 层实现，
/// 本 crate 定义后端的权限校验和插件管理逻辑。
pub mod audit;
pub mod compat_matrix;
pub mod mvu_runtime;

pub use audit::{
    AuditChainVerification, AuditOrderBy, AuditPage, AuditQuery, AuditRecord, AuditRecordInput,
    CorrelationId, chain_audit_records, compute_audit_record_hash, ensure_live_permission,
    next_correlation_id, paginate_audit_records, query_audit_records, retain_audit_records,
    verify_audit_record_chain,
};
pub use compat_matrix::{
    CompatEntry, CompatStatus, INTENTIONALLY_UNSUPPORTED_ST_EVENTS, PERMISSION_COMPAT_MATRIX,
    ST_API_COMPAT_MATRIX, ST_EVENT_COMPAT_MATRIX, find_permission_entry, unsupported_event_names,
};

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
    /// 读取变量
    ReadVariables,
    /// 直接写变量（兼容旧插件；新插件应优先走提议更新/预览）
    WriteVariables,
    /// 写回聊天记录（桥 `chat.save`）。M-31c：旧实现把 `chat.save` 放在通用权限门
    /// 之前 `return`，等于给任何插件开了一条无权限写通道；现在必须显式声明本权限。
    WriteChat,
    /// 提议变量更新，由宿主预览/确认后应用
    ProposeVariableUpdate,
    /// 读取并改写最终发送给 LLM 的 prompt/messages
    ModifyPrompt,
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
            match std::fs::read_to_string(&path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
                    tracing::warn!("插件注册表 JSON 解析失败({e})，尝试 .tmp 备份");
                    let tmp = std::path::PathBuf::from(format!("{}.tmp", path.display()));
                    std::fs::read_to_string(&tmp)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| {
                            tracing::error!("插件注册表 JSON 主文件和 .tmp 备份均损坏，文件: {}, 错误: {}. 已保存 .corrupt 备份", path.display(), e);
                            let _ = std::fs::copy(&path, path.with_extension("json.corrupt"));
                            HashMap::new()
                        })
                }),
                Err(e) => {
                    // IO 失败（Windows 上 AV/索引器锁是常态）：与 infra-vector 同策略——
                    // 先试 .tmp 备份，仍失败则保 .corrupt 副本后空表启动。
                    // 直接空表会让下一次 persist() 把近空注册表覆盖回真文件，
                    // 永久丢失已安装插件（2026-09-01 全量审查修复）。
                    tracing::warn!("插件注册表文件读取失败({e})，尝试 .tmp 备份");
                    let tmp = std::path::PathBuf::from(format!("{}.tmp", path.display()));
                    std::fs::read_to_string(&tmp)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| {
                            tracing::error!(
                                "插件注册表读取失败且无可用备份，文件: {}, IO 错误: {}. 已保存 .corrupt 备份",
                                path.display(),
                                e
                            );
                            let _ = std::fs::copy(&path, path.with_extension("json.corrupt"));
                            HashMap::new()
                        })
                }
            }
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
        let mut plugins = self.plugins.write().unwrap_or_else(|p| p.into_inner());

        // 检查是否已安装
        if plugins.contains_key(&manifest.id) {
            return Err(PluginError::AlreadyInstalled(manifest.id));
        }

        let plugin = InstalledPlugin {
            manifest,
            installed_at: chrono::Utc::now(),
            enabled: true,
        };

        let id = plugin.manifest.id.clone();
        plugins.insert(id.clone(), plugin);
        if let Err(e) = self.persist(&plugins) {
            plugins.remove(&id);
            return Err(e);
        }
        Ok(())
    }

    /// 卸载插件
    pub fn uninstall(&self, id: &str) -> Result<(), PluginError> {
        let mut plugins = self.plugins.write().unwrap_or_else(|p| p.into_inner());
        let removed = plugins
            .remove(id)
            .ok_or_else(|| PluginError::NotFound(id.into()))?;
        if let Err(e) = self.persist(&plugins) {
            plugins.insert(id.to_string(), removed);
            return Err(e);
        }
        Ok(())
    }

    /// 启用/禁用插件
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), PluginError> {
        let mut plugins = self.plugins.write().unwrap_or_else(|p| p.into_inner());
        let plugin = plugins
            .get_mut(id)
            .ok_or_else(|| PluginError::NotFound(id.into()))?;
        let previous = std::mem::replace(&mut plugin.enabled, enabled);
        if let Err(e) = self.persist(&plugins) {
            if let Some(plugin) = plugins.get_mut(id) {
                plugin.enabled = previous;
            }
            return Err(e);
        }
        Ok(())
    }

    /// 获取插件
    pub fn get(&self, id: &str) -> Option<InstalledPlugin> {
        self.plugins
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    /// 列出所有插件
    pub fn list(&self) -> Vec<InstalledPlugin> {
        self.plugins
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// 校验权限（后端二次校验，对应设计 §8.2）
    pub fn ensure_permission(
        &self,
        plugin_id: &str,
        permission: &Permission,
    ) -> Result<(), PluginError> {
        let plugins = self.plugins.read().unwrap_or_else(|p| p.into_inner());
        let plugin = plugins
            .get(plugin_id)
            .ok_or_else(|| PluginError::NotFound(plugin_id.into()))?;

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

    /// 在持有写锁时落盘；失败由调用方回滚内存并把错误返回给命令层。
    ///
    /// 2026-10-11 审计：此前只 `error!` 后返回 Ok——安装/卸载/启停对用户报告成功，
    /// 重启后却丢失（其它 store 均已传播落盘错误，插件注册表是唯一例外）。
    fn persist(&self, plugins: &HashMap<String, InstalledPlugin>) -> Result<(), PluginError> {
        if let Some(path) = &self.persist_path {
            storyforge_infra_util::atomic_write_json(path, plugins).map_err(|e| {
                tracing::error!("持久化插件数据失败: {e}");
                PluginError::Io(e)
            })?;
        }
        Ok(())
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
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
    (
        "SillyTavern.getContext()",
        "storyforge.character.getCurrent() + storyforge.worldInfo.search()",
        "拆分成多个细粒度 API",
    ),
    (
        "getContext().characters",
        "storyforge.character.list() / .get(id)",
        "",
    ),
    (
        "getContext().chat",
        "storyforge.memory.getRecent()",
        "走记忆系统",
    ),
    (
        "eventTypes.MESSAGE_RECEIVED",
        "storyforge.events.on('message.finalized', cb)",
        "事件名重定义",
    ),
    (
        "eventTypes.GENERATION_STARTED",
        "storyforge.events.on('pipeline.state_changed', cb)",
        "",
    ),
    (
        "setLocalVar(key, val)",
        "storyforge.variables.set(key, val)",
        "兼容路径需 WriteVariables；新插件应使用 ProposeVariableUpdate",
    ),
    (
        "getLocalVar(key)",
        "storyforge.variables.get(key)",
        "需 ReadVariables 权限",
    ),
    (
        "replaceVariables(msg)",
        "（不暴露，变量替换由后端统一做）",
        "",
    ),
    (
        "triggerSlash('/genraw ...')",
        "storyforge.llm.generate(prompt)",
        "需 CallLlm 权限",
    ),
    (
        "$('#chat').append(html)",
        "storyforge.ui.mountToSlot('message_decorator', el)",
        "DOM → UI slot",
    ),
    (
        "extension_settings[myExt]",
        "storyforge.storage.get/set",
        "持久化",
    ),
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
        registry
            .install(make_manifest("p2", vec![Permission::ReadCharacters]))
            .unwrap();

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
        registry
            .install(make_manifest("p1", vec![Permission::ReadCharacters]))
            .unwrap();

        // 有权限
        assert!(
            registry
                .ensure_permission("p1", &Permission::ReadCharacters)
                .is_ok()
        );

        // 无权限
        assert!(
            registry
                .ensure_permission("p1", &Permission::WriteVariables)
                .is_err()
        );
        assert!(
            registry
                .ensure_permission("p1", &Permission::ReadVariables)
                .is_err()
        );
    }

    #[test]
    fn test_permission_check_unregistered_plugin_fails_closed() {
        // L4 严格门禁的地基：未注册的插件 id（如卡壳虚拟插件的随机临时 id）
        // 必须 NotFound 拒绝，而不是放行——后端注册表是最终边界。
        let registry = PluginRegistry::new();
        let result = registry.ensure_permission("card-shell-abc123", &Permission::ReadMemory);
        assert!(matches!(result, Err(PluginError::NotFound(_))));
    }

    #[test]
    fn test_permission_check_disabled_plugin() {
        let registry = PluginRegistry::new();
        registry
            .install(make_manifest("p1", vec![Permission::ReadCharacters]))
            .unwrap();
        registry.set_enabled("p1", false).unwrap();

        let result = registry.ensure_permission("p1", &Permission::ReadCharacters);
        assert!(matches!(result, Err(PluginError::Disabled(_))));
    }

    #[test]
    fn test_modify_prompt_permission_manifest_roundtrip() {
        let manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "id": "prompt-hook",
            "name": "Prompt Hook",
            "version": "1.0.0",
            "permissions": ["ModifyPrompt"],
            "entry_html": "<script></script>",
            "ui_slots": [],
            "event_subscriptions": ["CHAT_COMPLETION_PROMPT_READY"],
            "description": null,
            "author": null
        }))
        .unwrap();

        let registry = PluginRegistry::new();
        registry.install(manifest).unwrap();

        registry
            .ensure_permission("prompt-hook", &Permission::ModifyPrompt)
            .unwrap();
        assert!(
            registry
                .ensure_permission("prompt-hook", &Permission::ReadMemory)
                .is_err()
        );
    }

    /// 2026-10-11 审计：落盘失败必须返回错误并回滚内存（此前吞错后报告成功，
    /// 重启即丢失安装/卸载/启停）。
    #[test]
    fn persist_failure_is_reported_and_rolled_back() {
        let dir = std::env::temp_dir().join(format!(
            "sf_plugins_persist_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let path = dir.join("plugins.json");
        let registry = PluginRegistry::with_persistence(path.clone());
        registry.install(make_manifest("kept", vec![])).unwrap();

        storyforge_infra_util::write_fence::freeze(&path);
        assert!(matches!(
            registry.install(make_manifest("new", vec![])),
            Err(PluginError::Io(_))
        ));
        assert!(registry.get("new").is_none(), "安装失败不得残留在内存");
        assert!(registry.set_enabled("kept", false).is_err());
        assert!(
            registry.get("kept").unwrap().enabled,
            "启停失败必须恢复原状态"
        );
        assert!(registry.uninstall("kept").is_err());
        assert!(registry.get("kept").is_some(), "卸载失败必须放回插件");
        storyforge_infra_util::write_fence::unfreeze(&path);

        registry.set_enabled("kept", false).unwrap();
        let reopened = PluginRegistry::with_persistence(path.clone());
        assert!(!reopened.get("kept").unwrap().enabled);
        assert!(reopened.get("new").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_st_api_mapping_exists() {
        assert!(!ST_API_MAPPING.is_empty());
        assert!(
            ST_API_MAPPING
                .iter()
                .all(|(st, sf, _)| !st.is_empty() && !sf.is_empty())
        );
    }
}

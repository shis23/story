use super::super::*;

// ─── M4 插件命令 ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct InstalledPluginDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub permissions: Vec<String>,
    /// 入口 HTML：PluginHost.vue 依赖它在沙箱 iframe 内启动插件桥。
    /// 缺了它前端 `iframeDoc` 恒为空，插件的事件订阅/prompt hook 全部失效。
    /// 安全边界在 `sandbox="allow-scripts"` iframe 与权限门控命令桥，不在此处。
    pub entry_html: String,
    pub ui_slots: Vec<String>,
    pub event_subscriptions: Vec<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub enabled: bool,
    pub installed_at: String,
}

pub fn plugin_to_dto(p: &storyforge_infra_plugin_host::InstalledPlugin) -> InstalledPluginDto {
    InstalledPluginDto {
        id: p.manifest.id.clone(),
        name: p.manifest.name.clone(),
        version: p.manifest.version.clone(),
        entry_html: p.manifest.entry_html.clone(),
        permissions: p
            .manifest
            .permissions
            .iter()
            .map(|perm| format!("{perm:?}"))
            .collect(),
        ui_slots: p
            .manifest
            .ui_slots
            .iter()
            .map(|slot| format!("{slot:?}"))
            .collect(),
        event_subscriptions: p.manifest.event_subscriptions.clone(),
        description: p.manifest.description.clone(),
        author: p.manifest.author.clone(),
        enabled: p.enabled,
        installed_at: p.installed_at.to_rfc3339(),
    }
}

pub fn ensure_plugin_any_permission(
    registry: &storyforge_infra_plugin_host::PluginRegistry,
    plugin_id: &str,
    permissions: &[storyforge_infra_plugin_host::Permission],
) -> Result<(), TauriCommandError> {
    let mut last_error = None;
    for permission in permissions {
        match registry.ensure_permission(plugin_id, permission) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }
    Err(TauriCommandError::internal(
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "插件权限不足".to_string()),
    ))
}
/// M-08 判定核心（抽出来便于单测）：提交 messages 改写的调用必须声明是哪些
/// 插件改的，且每个声明的插件都必须在 PluginRegistry 中启用并持 `ModifyPrompt`。
/// 返回 `Some(reason)` = 拒绝改写（fail closed，保留原始提示词）。
///
/// 为什么不是"只认一个 plugin_id"：prompt hook 是广播给宿主的，最终 messages
/// 可能由多个插件依次改写，所以真实契约是"改写者集合"；单值 `plugin_id` 作为
/// 兼容形式一并接受。
pub fn prompt_mutation_denial(
    registry: &storyforge_infra_plugin_host::PluginRegistry,
    plugin_id: Option<&str>,
    modifier_plugin_ids: Option<&[String]>,
) -> Option<String> {
    use storyforge_infra_plugin_host::Permission;

    let mut declared: Vec<&str> = Vec::new();
    if let Some(id) = plugin_id.map(str::trim).filter(|id| !id.is_empty()) {
        declared.push(id);
    }
    if let Some(ids) = modifier_plugin_ids {
        for id in ids {
            let id = id.trim();
            if !id.is_empty() && !declared.contains(&id) {
                declared.push(id);
            }
        }
    }
    if declared.is_empty() {
        return Some("no plugin declared as the prompt modifier".to_string());
    }
    declared.iter().find_map(|id| {
        registry
            .ensure_permission(id, &Permission::ModifyPrompt)
            .err()
            .map(|error| format!("plugin {id}: {error}"))
    })
}

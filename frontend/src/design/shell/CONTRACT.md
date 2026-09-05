# 应用壳 · 接线契约卡（design/shell）

## AppFrame

纯布局，生产已挂 AppV2。

| prop | 来源 |
| --- | --- |
| `sidebarOpen` | `ui.showSidebar` |
| `sidebarCollapsed` | `ui.sidebarCollapsed`（桌面收起状态，独立于移动抽屉） |
| `inspectorOpen` | `ui.showDebugDrawer` |

| event | 接到 |
| --- | --- |
| `update:sidebarOpen` | `ui.showSidebar = $event` |
| `update:sidebarCollapsed` | `ui.sidebarCollapsed = $event` |
| `update:inspectorOpen` | `ui.showDebugDrawer = $event` |

| slot | 生产注入 |
| --- | --- |
| `sidebar` | `PrimarySidebar`（事件 → ui / import / new-campaign） |
| `topbar` | `TopBar` |
| `content` | overview / history / writing |
| `composer` | 空（Composer 已并入 WritingScreen） |
| `inspector` | `InspectorDrawer` |
| `panels` | CharacterList / Campaign / Meta / config / NewCampaignForm / **PluginHost** / **MvuJsRuntime** |

`topbar` 插槽提供 `{ docked, sidebarVisible, toggleSidebar }`，传给 TopBar 的状态与切换事件。
`sidebar` 插槽提供 `{ docked, collapseSidebar }`，后者只用于显式收起按钮。
导航项的 `close` 事件仍只关闭移动抽屉，不改变桌面收起偏好。
收起侧栏使用 `v-show` 保留唯一 runtime 实例；窗口跨断点时清除旧的移动抽屉开关。

## 配色

PrimarySidebar 底部挂载 ThemePicker。`useTheme` 分别维护 `theme`（light/dark）
与 `palette`（teal/blue/rose/classic），沿用 `storyforge-theme` 并新增
`storyforge-palette` 本地存储键。配色只修改根节点主题属性，不触发导航或卸载。
`style.css` 是语义颜色的来源；浏览器 `theme-color` 从当前背景 token 同步。
ThemePicker 使用原生单选组和独立夜读开关；收起侧栏不重置选择。
classic 恢复原版浅色/夜读色值，不恢复旧布局或改变默认配色。夜读浅金色
实心按钮使用深色前景，兼容现有 `bg-accent text-white` 控件并保证文字可读。

## 红线

- `#panels` 必须继续挂载隐藏 `PluginHost` 循环与 `MvuJsRuntime`。
- Inspector 为覆盖式抽屉，不得改回挤压主栏的常驻 aside（selected 规格）。

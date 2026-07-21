# 应用壳 · 接线契约卡（design/shell）

## AppFrame

纯布局，生产已挂 AppV2。

| prop | 来源 |
| --- | --- |
| `sidebarOpen` | `ui.showSidebar` |
| `inspectorOpen` | `ui.showDebugDrawer` |

| event | 接到 |
| --- | --- |
| `update:sidebarOpen` | `ui.showSidebar = $event` |
| `update:inspectorOpen` | `ui.showDebugDrawer = $event` |

| slot | 生产注入 |
| --- | --- |
| `sidebar` | `PrimarySidebar`（事件 → ui / import / new-campaign） |
| `topbar` | `TopBar` |
| `content` | overview / history / writing |
| `composer` | 空（Composer 已并入 WritingScreen） |
| `inspector` | `InspectorDrawer` |
| `panels` | CharacterList / Campaign / Meta / config / NewCampaignForm / **PluginHost** / **MvuJsRuntime** |

## 红线

- `#panels` 必须继续挂载隐藏 `PluginHost` 循环与 `MvuJsRuntime`。
- Inspector 为覆盖式抽屉，不得改回挤压主栏的常驻 aside（selected 规格）。

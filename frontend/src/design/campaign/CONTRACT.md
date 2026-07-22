# 活动管理 · 接线契约卡（design/campaign）

> 纯展示主屏，对齐 selected 图④。不替代 `CampaignInstancesTab` 的 MVU 深度编辑；
> 深度变量编辑与提取角色卡仍走既有 v2 子组件 / Panel，由 adapter 决定挂接。

## Props ↔ 数据源

| prop | 来源 |
| --- | --- |
| `campaigns` | `listCampaigns` / adapter 状态 |
| `selectedCampaignId` / `selectedCampaign` | adapter 选择态 |
| `instances` | `listInstances` |
| `knowledge` | campaign knowledge list API |
| `tasks` | campaign tasks API |
| `summaries` | campaign summaries API |
| `detailTab` | adapter UI state |

## Events

| 事件 | 接到 |
| --- | --- |
| `close` | 关面板 |
| `select-campaign` | 选中活动 |
| `set-active` | `setActiveCampaign` + store 同步 |
| `delete-campaign` | `deleteCampaign`：整局删除（会话+实例/知识/任务/总结） |
| `change-tab` | 切换 detail tab 并懒加载 |
| `new-campaign` | 打开 NewCampaignForm |
| `export-st` / `export-bundle` / `import-bundle` | 既有导出导入 |
| `refresh` | 刷新列表与详情 |

## 红线

- 不得破坏 `CampaignPanel.refreshActiveDetailTab` 对 Meta `mvu-applied` 的调用链。
- 若生产仍挂旧 `CampaignPanel`，新屏可作为 `#design-campaign` 预览或逐步替换；替换时必须保留 refresh 暴露。

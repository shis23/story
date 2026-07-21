# Meta 助手 · 接线契约卡（design/meta）

## 原则

- `MetaScreen` 只提供纸面壳 + tab；chat/patch/health/mvu/explain **业务组件**
  仍用 `components-v2/meta/*`，由 adapter 按 activeTab 注入 slot。
- 契约红线不变：`activeCampaign` / `lastConversationNode` props；
  `close` / `mvu-applied` emits；`mvu-applied` → `CampaignPanel.refreshActiveDetailTab`。

## Events

| 事件 | 接到 |
| --- | --- |
| `close` | 关面板 |
| `change-tab` | adapter 切换 activeTab |

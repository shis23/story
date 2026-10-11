# 会话历史 · 接线契约卡（design/history → AppV2）

> 纯展示。替换旧 `components-v2/writing/ConversationHistoryList.vue`（2026-10-11 已删除）。

## Props

| prop | 来源 |
| --- | --- |
| `conversations` | `campaignStore.conversationHistory` |
| `title` | 固定「会话历史」或派生 |

## Events

| 事件 | payload | 接到 |
| --- | --- | --- |
| `open` | `conv` | `useConversation.openConversation` |
| `delete` | `conv, event` | `handleDeleteConversation` |
| `new-campaign` | — | `openNewCampaignDialog` |

# Campaign 概览 · 接线契约

| prop | 来源 |
| --- | --- |
| `campaignName` | `activeCampaign.name` |
| `storyClock` | `activeCampaign.story_clock` |
| `createdAtText` | 格式化 `created_at` |
| `conversationCount` | `conversationHistory.length` |

| event | 接到 |
| --- | --- |
| `open-campaign` | `ui.showCampaignPanel = true` |
| `view-history` | `ui.viewHistory` |
| `new-campaign` | `openNewCampaignDialog` |
| `continue-writing` | `ui.viewWrite` |

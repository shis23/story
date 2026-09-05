# 写作主屏 · 接线契约卡（design/writing → AppV2）

> 面向接线 agent。本目录组件**纯展示、零 store 依赖**。
> **2026-07-22**：已通过 `adapter/useWritingScreenAdapter.js` 接到 AppV2 生产路径，
> 替换 `ConversationViewport` + `Composer`。视觉规范：`docs/VISUAL-REDESIGN-2026-07-21.md`。
> 预览：`http://localhost:1420/#design-writing`。

## 组件清单

| 组件 | 职责 | 对应旧组件 |
| --- | --- | --- |
| `WritingScreen.vue` | 整屏组合（滚动区 + 指令条）；expose `scrollToBottom` | `ConversationViewport` + `Composer` |
| `StoryPage.vue` | 稿纸页 | — |
| `MessageItem.vue` | 稿纸段落；8 emit；可选 `contentComponent` | `ChatMessage` |
| `VariantStrip.vue` | 三栏变体卡 | MessageVariantSwitcher |
| `StreamingBody.vue` | 流式段落 | `StreamingMessage` |
| `ProcessTimeline.vue` | 横向步骤回顾 | `ProcessReview` |
| `GreetingCards.vue` | 开场白 | `GreetingSelector` |
| `ComposerBar.vue` | 指令条 + chips | `Composer` |
| `EmptyHero.vue` | 空态三入口 | ConversationViewport 空态 |

## Props ↔ Store 映射（adapter 已实现）

| WritingScreen prop | 来源 |
| --- | --- |
| `title` | campaign / char 名 |
| `messages` | `writingStore.messages` |
| `isWriting` / `pipeline` / `showPipeline` / `streamingRoleLabel` | writing store |
| `canBranch` | campaign 模式 + activeCampaign + conversationId |
| `saveVariant` | `handleEditVariant(payload): Promise<boolean>`，贯穿 WritingScreen / StoryPage / MessageItem |
| `greetingOptions` | `canChooseGreeting` 时才传 |
| `composerDisabled` / `composerPlaceholder` | writingMode |
| `qualityAcceptHint` | pipeline.quality 派生 |
| `contentComponent` | adapter `markRaw(RichContent)` |
| `subagentRolesByMessage` | provenance → `subagentRolesFromProvenance` |

## Events → Handler（契约红线）

| 事件 | payload | 接到 |
| --- | --- | --- |
| `start-writing` | `text` | `useWriting.startWriting` |
| `cancel` | — | `cancelWriting` |
| `import` / `new-campaign` / `view-history` | — | import / NewCampaignForm / ui.viewHistory |
| `select-greeting` | `index` | `useGreeting.selectGreeting` |
| `reroll` | `{ messageId, nodeId, kind, hint }` | `handleReroll`（design 菜单：all/editor/subagent:id） |
| `reroll-user` … `branch` | 见 useMessageVariants | 8 handler 原样 |

## 生产接线决策（已落地）

1. RichContent 由 adapter 注入，design 不 import 功能层。
2. 删除前 adapter 调 tauri `ask`，明确包含后续消息及同轮输入；确认失败不执行删除。测试可注入 `askDialog`。
3. 重 roll 三级菜单在 MessageItem 内联实现。
4. `scrollToBottom` 由 WritingScreen expose，AppV2 转发。
5. Composer 已并入 WritingScreen；AppShell `#composer` 槽可空。
6. 旧 `components-v2/writing/*` 保留作对照与回退，生产主路径不再引用 Viewport/Composer。
7. 保存不是 Vue emit 事件，只有 `saveVariant` 回调明确返回 `true` 才退出编辑；返回 `false` 或拒绝时保留输入。
8. `add-variant` 必须传 `{ nodeId }`；分支只在已采纳末尾显示，后端独立验证并拒绝活动轮次和历史节点。

## 验收

1. 主路径：开场白 → 写作 → 流式 → 停止 → 成文 → 变体/重 roll/分支/删除 → 回顾。
2. `npm test` 313+ ✅；`npm run test:ui` 28 ✅；`npm run build` ✅。
3. 未触碰：`tauri-api.js` / `plugin-bridge.js` / `utils/**` / `PluginHost` / `MvuJsRuntime` / store 字段 / composable 签名。

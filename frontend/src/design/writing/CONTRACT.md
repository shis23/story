# 写作主屏 · 接线契约卡（design/writing → AppV2）

> 面向接线 agent。本目录组件**纯展示、零 store 依赖**；你的任务是把它们接到现有
> Pinia store / composables 上，替换 `components-v2/writing/ConversationViewport.vue` + `Composer.vue`
> 在 AppV2 中的使用。视觉规范：`docs/VISUAL-REDESIGN-2026-07-21.md`。
> 预览：`http://localhost:1420/#design-writing`（fixture 全状态 + 事件监视条）。

## 组件清单

| 组件 | 职责 | 对应旧组件 |
| --- | --- | --- |
| `WritingScreen.vue` | 整屏组合（滚动区 + 指令条） | `ConversationViewport.vue` + `Composer.vue` |
| `StoryPage.vue` | **稿纸页**：页头（题/字数/用时/状态徽章）+ 文稿小节 + 页脚生成状态与停止键 | —（新结构，对齐 selected 图②③） |
| `MessageItem.vue` | 稿纸小节：用户意图=淡注 / 助手成文=正文段 | `ChatMessage.vue`（契约红线） |
| `VariantStrip.vue` | 三栏变体卡（预览 3 行 + 字数 + 选中圈） | MessageVariantSwitcher 行为 |
| `StreamingBody.vue` | 流式小节（过程一行折叠 + 正文光标） | `StreamingMessage.vue` |
| `ProcessTimeline.vue` | 横向步骤条式过程回顾 | `ProcessReview.vue` |
| `GreetingCards.vue` | 开场白卡片选择 | `GreetingSelector.vue` |
| `ComposerBar.vue` | 指令条（快捷 chips 为纯文本预填，不发明后端能力） | `Composer.vue` |
| `EmptyHero.vue` | 空态三入口 | ConversationViewport 空态 |
| `fixtures.js` | 全状态假数据（message/pipeline shape 注释即文档） | — |

## Props ↔ Store 映射

| WritingScreen prop | 来源 | 备注 |
| --- | --- | --- |
| `title` | `campaignStore.activeCampaign?.name` 或会话派生题 | 页题；无则为「未命名文稿」类兜底 |
| `durationText` | pipeline 计时段（若后端无则留空隐藏） | 可选；现有数据无耗时字段时传 '' |
| `messages` | `writingStore.messages` | shape 一致，直接传 |
| `isWriting` | `writingStore.isWriting` | |
| `pipeline` | `writingStore.pipeline` | shape 一致，直接传 |
| `showPipeline` | `writingStore.showPipeline` | |
| `streamingRoleLabel` | `writingStore.streamingRoleLabel` | |
| `canBranch` | 现 `ConversationViewport.canBranch(m)` 的计算逻辑（Campaign 模式 + activeCampaign + currentConversationId） | 当前按屏级布尔传入；若要按消息粒度，接线时在 `MessageItem` 的 `can-branch` 上逐条计算 |
| `greetingOptions` | `writingStore.greetingOptions`（仅 `canChooseGreeting` 时传入） | |
| `selectedGreetingIndex` | `writingStore.selectedGreetingIndex` | |
| `composerDisabled` | `writingStore.writingMode === 'none'` | 对齐 AppV2 现用法 |
| `composerPlaceholder` | AppV2 `composerPlaceholder()` | |

## Events → Handler 映射（契约红线区）

| 事件 | payload | 接到 | 备注 |
| --- | --- | --- | --- |
| `start-writing` | `text: string` | `useWriting.startWriting` | AppV2 现接线一致 |
| `cancel` | — | `useWriting.cancelWriting` | |
| `import` | — | `useCharacterImport.handleImport` | |
| `new-campaign` | — | `useNewCampaignForm.openNewCampaignDialog` | |
| `view-history` | — | `uiStore.viewHistory` | |
| `select-greeting` | `index: number` | `useGreeting.selectGreeting` | |
| `reroll` | `{ messageId, nodeId, kind, hint }` | `useMessageVariants.handleReroll` | **红线 1/8**：现 design 版只发 `kind:'all'`；整体/编剧/子Agent 三级菜单需在接线层恢复（参考旧 ChatMessage 的 BaseDropdown + `subagentRolesFromProvenance`） |
| `reroll-user` | `{ messageId }` | `handleRerollUser` | **红线 2/8** |
| `switch-variant` | `{ messageId, index }` | `handleSwitchVariant` | **红线 3/8** |
| `edit-variant` | `{ nodeId, newContent }` | `handleEditVariant` | **红线 4/8** |
| `accept-variant` | `{ nodeId }` | `handleAcceptVariant` | **红线 5/8** |
| `delete-variant` | `{ nodeId }` | `handleDeleteVariant` | **红线 6/8**；旧实现里删除前弹 tauri `ask` 确认，接线层保留该确认 |
| `add-variant` | `{ messageId }` | `handleAddVariant` | **红线 7/8** |
| `branch` | `{ nodeId }` | `handleBranch` | **红线 8/8** |

## 接线时的已知差异（必须有适配决策）

1. **质量门禁提示**：旧 ChatMessage 有 `qualityAcceptHint`（采纳按钮上的 warn 文案），
   design 版未画。接线时二选一：a) 在 MessageItem 采纳按钮处补回（推荐，改动小）；
   b) 依赖 ProcessTimeline 的质量步骤承担提示。
2. **RichContent / display HTML**：design 版正文用纯文本段落渲染（`paragraphs()`），
   **生产必须换回** `components-v2/st/RichContent.vue`（安全 display-only HTML 与
   Markdown 降级是能力红线，display_content/source_content 双参传法照抄旧 ChatMessage）。
3. **自动滚动**：旧 ConversationViewport 暴露 `scrollToBottom()` 给 useWriting/usePipeline
   注入。WritingScreen 接线时需实现同名 expose（滚动容器 ref + nextTick scrollTop），
   签名不变。
4. **reroll 三级菜单**：见红线 1/8。
5. **删除确认**：见红线 6/8。
6. **页内停止键**：StoryPage 页脚「停止生成」与 ComposerBar 停止键同发 `cancel`，
   两处并存是刻意的（长文稿滚动时停止键始终可达）；接线只需接同一 `cancelWriting`。
7. **快捷指令 chips**：纯文本预填（如「继续往下写：」），直接作为 start-writing 文本发出；
   若产品决定去掉或改词，只改 ComposerBar 的 `chips` 数组。
8. **预览分支**：接线完成并验收后，删除 `main.js` 中 `#design-writing` 分支。

## 验收（接线 agent 完成定义）

1. AppV2 写作视图改挂 WritingScreen 后，主路径可走通：开场白选择 → 写作 → 流式 →
   停止 → 成文 → 变体切换/采纳/编辑/重 roll/分支/删除 → 过程回顾。
2. 8 个变体事件全部落到 `useMessageVariants` 原 handler（payload 形状不变）。
3. `npm run build` ✅；`npm test` 313 ✅；`npm run test:ui` 全绿
   （ChatMessage 相关测试如引用旧组件可保留旧文件不删，或迁移断言——需在 PR 说明取舍）。
4. 深浅两主题 + 移动端窄屏人工走查通过。
5. 未触碰：`tauri-api.js` / `plugin-bridge.js` / `utils/**` / `mvu-runtime-bridge.js` /
   `components/PluginHost.vue` / `components/MvuJsRuntime.vue` / store 字段 / composable 签名。

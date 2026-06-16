# 计划：前端 Campaign 工作台

> 状态：待执行
> 前置：`PLAN-CAMPAIGN-MAINLINE.md` 至少完成后端 Campaign 快照和 instance id 主线。

## 目标

把前端从“选择一个角色卡后聊天/写作”改成“选择一个 Campaign 后持续写作和维护”。

## 非目标

- 不重写整个前端。
- 不引入新 UI 框架。
- 不在后端 Campaign 主线未稳定前伪造 Campaign-first 体验。
- 不把 Meta、插件、日志都塞进首屏。

## 当前事实

- `frontend/src/App.vue` 已有 `activeCampaign`，mounted 时调用 `getActiveCampaign()`。
- 主写作仍调用 `apiStartWriting(intent, activeChar.value?.id, ..., currentConversationId.value)`。
- 顶部和消息标识仍主要显示 `activeChar`。
- `CampaignPanel.vue` 已能管理 cards、campaigns、instances、variables、knowledge、tasks、summaries。
- `PipelinePanel.vue` 展示 subagent 事件，但 subagent 名称来自后端 `character_id` 字符串。
- `MetaPanel.vue` 是独立弹层，尚未与 Campaign health/trace 深度打通。

## 阶段 1：首屏显示 active Campaign 状态

改动文件：

- `frontend/src/App.vue`
- `frontend/src/components/AppHeader.vue`
- `frontend/src/components/CampaignPanel.vue`

任务：

1. AppHeader 增加 active campaign 显示：
   - campaign name
   - 当前 turn，如有
   - instance count
2. 无 active Campaign 时，首屏 CTA 指向：
   - 导入卡
   - 识别角色
   - 创建 Campaign
3. 保留 activeChar 作为导入/兼容状态，不再作为首屏唯一主状态。

验收：

- 用户一眼能知道当前是不是在 Campaign 中。
- 无 active Campaign 时不会误导用户直接写 Campaign。

## 阶段 2：写作入口绑定 active Campaign

前置：

- 后端 `start_writing` 已能从 active campaign 装配 runtime。

改动文件：

- `frontend/src/App.vue`
- `frontend/src/tauri-api.js`
- `crates/tauri-app/src/lib.rs` 如需要扩展 DTO

任务：

1. 前端写作前检查：
   - 有 active Campaign：走 Campaign 写作。
   - 无 active Campaign 但有 activeChar：走 legacy ST 单卡写作，并显示兼容模式状态。
   - 两者都没有：阻止写作，提示先导入或开档。
2. API 层命名清晰：
   - 保留 `startWriting(intent, characterId, ...)` 兼容旧命令，或新增 `startCampaignWriting(intent, campaignId, ...)`。
   - 不要让前端继续以为 `characterId` 是 Campaign 主身份。
3. 写作返回值中展示 campaign/turn 信息，如后端提供。

验收：

- active Campaign 下写作不依赖 `activeChar`。
- 关闭 Campaign 后 legacy 写作仍可用。

## 阶段 3：CampaignPanel 拆成工作台标签

改动文件：

- `frontend/src/components/CampaignPanel.vue`
- 可新增：
  - `CampaignInstancesTab.vue`
  - `CampaignVariablesTab.vue`
  - `CampaignKnowledgeTab.vue`
  - `CampaignTasksTab.vue`
  - `CampaignSummariesTab.vue`

任务：

1. 保留现有功能，先组件拆分，不改后端。
2. 每个 tab 独立加载和错误状态。
3. variables 支持类型显示：
   - bool 用 checkbox
   - number 用 input number
   - string 用 text input
   - json 用 textarea 或只读折叠
4. knowledge 支持按 instance/source 过滤。
5. tasks 支持 status filter。

验收：

- CampaignPanel 不再是一个超长单文件。
- 移动端每个 tab 可独立滚动。

## 阶段 4：Pipeline trace 使用 instance display name

前置：

- 后端事件或 provenance 已提供 instance id -> display name 映射。

改动文件：

- `frontend/src/App.vue`
- `frontend/src/components/PipelinePanel.vue`

任务：

1. subagent event 内部保留 `instance_id`。
2. UI 展示 display name。
3. 展开项显示：
   - instance id
   - role_type
   - 输入摘要
   - 输出摘要
4. reroll target 使用稳定 id，而非 display name。

验收：

- 同名角色在 trace 中可区分。
- reroll 不因角色改名失效。

## 阶段 5：MetaPanel 接入 Campaign health

前置：

- `PLAN-META-AGENT.md` 阶段 1 完成。

改动文件：

- `frontend/src/components/MetaPanel.vue`
- `frontend/src/tauri-api.js`
- `frontend/src/App.vue`

任务：

1. MetaPanel 顶部展示 active Campaign health summary。
2. 从 health issue 一键生成 Meta 对话上下文。
3. patch preview 在侧栏展示 diff。
4. accept 后刷新 CampaignPanel 对应 tab。

验收：

- Meta 不再像孤立聊天窗口。
- 接受 patch 后 UI 状态同步刷新。

## 阶段 6：移动端布局专项

改动文件：

- `frontend/src/App.vue`
- `frontend/src/components/*.vue`
- `frontend/src/style.css` 如存在

检查 viewport：

- 360x800
- 390x844
- 480x900
- 768x1024
- 桌面 1280x800

任务：

1. 所有弹层移动端全屏或底部 sheet，避免双层滚动失控。
2. 主输入区固定，长输出不遮挡。
3. PipelinePanel 长文本折叠默认关闭。
4. Campaign tabs 不横向溢出。
5. 文本按钮不使用过长中文导致挤压。

验证：

```bash
cd frontend
npm run build
```

手动验收：

- 导入卡 -> 识别角色 -> 创建 Campaign -> 写第一轮 -> 查看变量/知识/摘要，移动端可完成。

## 禁止改动

- 禁止为了视觉重做而改后端语义。
- 禁止删除 legacy activeChar 导入能力。
- 禁止在前端生成假的 Campaign state。
- 禁止把所有状态塞进一个全局巨型对象后不分层。


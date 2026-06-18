# W5 执行手册：Phase 4 阶段 3/4/6 — 前端工作台

> 交接对象：Claude Code（在 worktree `storyforge-w5-frontend` 分支 `w5-frontend` 工作）
> 前置必读：`docs/PLAN-FRONTEND-WORKBENCH.md`（阶段 3/4/6）、`docs/HANDOFF-W4-FRONTEND.md`（阶段 1/2/5 已完成）
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w5-frontend`
> 分支：`w5-frontend`（已基于 `main` 64cd522）
> 性质：独立长期线，与 W6（后端知识传播）零文件交集。

## 一、任务概述

Phase 4 前端剩三个阶段。阶段 2（写作入口绑定 Campaign）已完成，现在做 3/4/6。

| 阶段 | 内容 | 优先级 |
|---|---|---|
| 3 | CampaignPanel 拆成独立 tab 组件 | 先做 |
| 4 | Pipeline trace 用 instance display name | 后做 |
| 6 | 移动端布局适配 | 最后 |

## 二、阶段 3：CampaignPanel 拆工作台标签（先做）

### 现状（已核实）

`CampaignPanel.vue` 已有 tab 结构，但全在一个文件里：
- 顶层 tab：`activeTab: 'cards'|'campaigns'|'detail'`（:15）
- detail 内有 sub-tab：`detailSubTab: 'instances'|'knowledge'|'tasks'|'summaries'`（:125）
- 四类数据用 `v-if` 切换，全在同一 .vue 文件的 template 里
- `refreshDetail()`（:127）一次拉全部四类数据（instances/knowledge/tasks/summaries）

### 要做的

把 detail 的四个 sub-tab 抽成独立组件：
- `CampaignInstancesTab.vue` — 角色实例列表 + 变量展开编辑
- `CampaignKnowledgeTab.vue` — 知识列表（按 instance/source 过滤）
- `CampaignTasksTab.vue` — 任务列表（按 status filter）
- `CampaignSummariesTab.vue` — 轮次摘要

每个 tab 组件：
1. **独立加载**：接收 `campaignId` prop，自己 onMounted 拉数据（不再由父组件 refreshDetail 一次全拉）。每个 tab 有自己的 loading/error 状态。
2. **独立刷新**：暴露 refresh 方法或用 emit 通知父组件，accept patch 后能单独刷新对应 tab。
3. **CampaignPanel.vue 变薄**：只保留顶层 tab 切换 + cards/campaigns 管理 + 传 campaignId 给 detail 子组件。

### 变量类型化显示（阶段 3 要求）

`CampaignInstancesTab` 展开变量时按类型显示（现在 `handleVariableChange` :146 只解析布尔）：
- bool → checkbox
- number → input number
- string → text input
- json → textarea 或只读折叠

变量类型在后端 schema（`VariableField.value_type`），前端 `getCharacterVariables` 返回的变量是否带类型？读 `tauri-api.js` 的 `getCharacterVariables` 返回结构确认。若不带类型，可能需后端 DTO 补类型字段——但优先用现有数据，不轻易扩后端。

### 验收

- CampaignPanel.vue 不再是超长单文件（行数显著减少）。
- 每个 tab 独立加载，切 tab 不重新拉全部数据。
- `npm run build` 通过。
- 移动端每个 tab 可独立滚动（为阶段 6 打底）。

## 三、阶段 4：Pipeline trace 用 instance display name

### 现状

`PipelinePanel.vue` 显示 subagent 事件时用后端 `character_id` 字符串（裸 id）。同名角色无法区分，reroll 用名字会因改名失效。

### 要做的

1. subagent event 内部保留 `instance_id`，UI 展示 display name（从 CampaignRuntimeContext 的 instance 映射）。
2. 展开项显示：instance id / role_type / 输入摘要 / 输出摘要。
3. reroll target 用稳定 instance_id 而非 display name。

前置：后端事件/provenance 已提供 instance id→display name 映射（Phase 1-3 完成，`SubagentSnapshot` 有 `display_name`）。读 `PipelinePanel.vue` 确认事件结构里有没有 display_name 字段，没有则看能否从 campaign store 前端侧映射。

### 验收

- 同名角色在 trace 中可区分。
- reroll 不因角色改名失效。
- `npm run build` 通过。

## 四、阶段 6：移动端布局专项

### 检查 viewport

360x800 / 390x844 / 480x900 / 768x1024 / 桌面 1280x800。

### 要做的

1. 所有弹层移动端全屏或底部 sheet，避免双层滚动失控。
2. 主输入区（Composer）固定，长输出不遮挡。
3. PipelinePanel 长文本折叠默认关闭。
4. Campaign tabs 不横向溢出。
5. 文本按钮不用过长中文导致挤压。

### 验收

- `cd frontend && npm run build` 通过。
- 各 viewport 下主流程可操作（无遮挡/无溢出/可滚动）。

## 五、关键约束

- **不重写整个前端**（沿用现有 Vue 栈，不引入新框架）。
- **不改后端**（除非变量类型化确实需 DTO 扩展，且优先尝试前端解决）。
- **每阶段做完 `npm run build`**，不破坏构建。
- **不 commit**（留给用户审）。可分阶段多 commit。
- **改 tauri-app/src/lib.rs 要注明区段**（与 W6 共享文件——但 W5 阶段 3/4/6 理论上不改 lib.rs，若改了要注明）。

## 六、给 Claude Code 的提示词

```
请阅读 docs/PLAN-FRONTEND-WORKBENCH.md（阶段 3/4/6）和 docs/HANDOFF-W5-FRONTEND.md（本文件），
然后推进 Phase 4 前端剩余阶段。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w5-frontend
分支：w5-frontend

现状：阶段 1/2/5 已完成。按 3→4→6 顺序做。

先从阶段 3（CampaignPanel 拆工作台标签）开始：
- 读 frontend/src/components/CampaignPanel.vue 理解现状：
  顶层 tab cards/campaigns/detail，detail 内 sub-tab instances/knowledge/tasks/summaries
  全在一个文件用 v-if 切换，refreshDetail 一次拉全部。
- 把 detail 四个 sub-tab 抽成独立组件：
  CampaignInstancesTab / CampaignKnowledgeTab / CampaignTasksTab / CampaignSummariesTab。
- 每个 tab 接收 campaignId prop，独立 onMounted 加载 + 独立 loading/error 状态。
- CampaignPanel.vue 变薄，只保留顶层 tab + cards/campaigns 管理 + 传 campaignId。
- CampaignInstancesTab 变量按类型显示（bool→checkbox/number→input number/string→text/json→textarea）。
  先看 getCharacterVariables 返回结构是否带类型，优先前端解决不轻易扩后端。

每阶段做完 npm run build 验证。不 commit（留给用户审），可分阶段多 commit。
不重写前端 / 不引入新框架 / 阶段 3/4/6 尽量不改后端 lib.rs。

阶段 3 做完报告，再进阶段 4。
```

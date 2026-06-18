# W4 执行手册：Phase 4 前端工作台重构

> 交接对象：Claude Code（在 worktree `storyforge-w4-frontend` 分支 `w4-frontend` 工作）
> 前置必读：`docs/PLAN-FRONTEND-WORKBENCH.md`（完整阶段划分）、`docs/ROADMAP.md` Phase 4
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w4-frontend`
> 分支：`w4-frontend`（已基于 `main` c0456aa）
> 性质：**独立长期线**，与其他 worktree（W1 文档 / W2 I1 / W3 P3P4 后端）零文件交集，可一直挂着推进。

## 一、任务概述

ROADMAP Phase 4 前端工作台重构，约 30% 完成。按 `PLAN-FRONTEND-WORKBENCH.md` 的 6 个阶段推进。

### 阶段现状（已核实）

| 阶段 | 内容 | 状态 |
|---|---|---|
| 1 | 首屏显示 active Campaign 状态（AppHeader） | ✅ 已完成 |
| 2 | 写作入口绑定 active Campaign | ❌ 未开始 |
| 3 | CampaignPanel 拆成工作台标签 | ❌ 未开始 |
| 4 | Pipeline trace 使用 instance display name | 🟡 已起步（subagent event 有 instance_id，UI 展示未改） |
| 5 | MetaPanel 接入 Campaign health | ✅ 已完成（体检按钮 + issue 列表） |
| 6 | 移动端布局专项 | ❌ 未开始 |

## 二、执行顺序建议

按依赖关系：**2 → 3 → 4 → 6**（5 已完成）。

- **阶段 2 优先**：写作入口绑定 Campaign 是后续所有前端体验的基础。不绑定，Campaign 写作路径走不通，3/4 的展示无从验证。
- **阶段 3 次之**：CampaignPanel 拆标签是纯组件重构（不改后端），为 4/6 的展示和移动端打底。
- **阶段 4**：Pipeline trace 用 display name，依赖后端 provenance（已有 instance id→name 映射，后端就绪）。
- **阶段 6 最后**：移动端布局，需前面功能成型后做响应式调优。

## 三、各阶段要点（详见 PLAN-FRONTEND-WORKBENCH.md）

### 阶段 2：写作入口绑定 active Campaign（先做）

改动：`App.vue`、`tauri-api.js`、必要时 `tauri-app/src/lib.rs`（扩展 DTO）。

核心：
- 写作前检查：有 active Campaign → 走 Campaign 写作；无 Campaign 但有 activeChar → legacy ST 单卡 + 显示兼容模式；都没有 → 阻止 + 提示。
- API 命名清晰：保留 `startWriting(intent, characterId, ...)` 兼容旧命令，或新增 `startCampaignWriting(intent, campaignId, ...)`。
- **不要让前端继续以为 `characterId` 是 Campaign 主身份**（这是当前混淆点）。
- 写作返回值展示 campaign/turn 信息。

验收：active Campaign 下写作不依赖 `activeChar`；关闭 Campaign 后 legacy 写作仍可用。

⚠️ 后端注意：若需扩展 DTO，`tauri-app/src/lib.rs` 是共享文件——但 W3 也改这个文件（postprocess 区段）。**两者改的是不同区段**（W2 写作命令 vs W3 postprocess），合并时大概率无冲突，但若 W4 改了 lib.rs 要在 commit 注明改的区段，便于合并时核对。

### 阶段 3：CampaignPanel 拆工作台标签

改动：`CampaignPanel.vue` + 可新增 5 个 tab 组件（Instances/Variables/Knowledge/Tasks/Summaries）。

核心：
- 先组件拆分不改后端。
- 每个 tab 独立加载/错误状态。
- variables 类型化显示（bool→checkbox / number→input number / string→text / json→textarea）。
- knowledge 按 instance/source 过滤；tasks 按 status filter。

验收：CampaignPanel 不再是超长单文件；移动端每个 tab 独立滚动。

### 阶段 4：Pipeline trace 用 instance display name

改动：`App.vue`、`PipelinePanel.vue`。

核心：
- subagent event 内部保留 `instance_id`，UI 展示 display name。
- 展开项显示 instance id / role_type / 输入摘要 / 输出摘要。
- reroll target 用稳定 id 而非 display name。

验收：同名角色在 trace 可区分；reroll 不因角色改名失效。

### 阶段 6：移动端布局专项

改动：`App.vue`、各 `*.vue`、`style.css`。

检查 viewport：360x800 / 390x844 / 480x900 / 768x1024 / 桌面 1280x800。

核心：弹层移动端全屏或底部 sheet / 主输入固定 / 长文本折叠默认关 / Campaign tabs 不横向溢出 / 按钮不挤压。

验收：`cd frontend && npm run build` 通过。

## 四、关键约束

- **不重写整个前端**（PLAN 非目标）。
- **不引入新 UI 框架**（沿用现有 Vue 栈）。
- **不伪造 Campaign-first 体验**——后端 Campaign 主线已稳定（Phase 1-3 完成），可放心绑定。
- **后端 Campaign 主线未稳定的功能不塞首屏**。
- **改 lib.rs 要注明区段**（与 W3 共享文件，便于合并）。
- **每阶段做完 `npm run build` 验证**，不破坏构建。
- **不 commit**（用户没要求；改完留给用户审）。
- **可分阶段多个 commit**（每阶段一个，便于回滚）。

## 五、给 Claude Code 的提示词

```
请阅读 docs/PLAN-FRONTEND-WORKBENCH.md 和 docs/HANDOFF-W4-FRONTEND.md（本文件），
然后推进 Phase 4 前端工作台重构。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w4-frontend
分支：w4-frontend

现状：阶段 1/5 已完成，2/3/4/6 未做。按 2→3→4→6 顺序推进。

先从阶段 2（写作入口绑定 active Campaign）开始：
- 读 frontend/src/App.vue 理解当前写作入口（activeChar vs activeCampaign 混淆点）。
- 读 frontend/src/tauri-api.js 理解现有 API 命名。
- 写作前检查三态：有 Campaign→Campaign 写作；无 Campaign 有 activeChar→legacy+兼容标识；
  都没有→阻止+提示。
- 保留 startWriting 兼容旧命令，或新增 startCampaignWriting。
- 不要让前端以为 characterId 是 Campaign 主身份。

每阶段做完跑 npm run build 验证。改 tauri-app/src/lib.rs 要在 commit 注明区段
（与 W3 共享文件）。

红线：不重写前端 / 不引入新框架 / 不伪造 Campaign 体验 / 不 commit（留给用户审）。
可分阶段多 commit。

阶段 2 做完报告，再进阶段 3。
```

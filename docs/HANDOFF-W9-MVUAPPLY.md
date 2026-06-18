# W9 执行手册：MVU Apply 前端接线

> 交接对象：Claude Code（在 worktree `storyforge-w9-mvuapply` 分支 `w9-mvuapply` 工作）
> 前置必读：`docs/PLAN-PLUGIN-MVU.md` 阶段 2/3、`docs/HARNESS-FINDINGS-2026-06-18.md` §F3
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w9-mvuapply`
> 分支：`w9-mvuapply`（已基于 `main` a47d645）
> 性质：纯前端接线，后端命令早已有，与 W10（后端 JS runtime 接通）零文件交集。

## 一、任务概述

后端的 MVU apply 命令早已有（Round 2 实现 + F3 修了 backfill 死代码），但**前端从未接这些命令**——用户在界面上无法触发"预览/应用 MVU schema"。本 worktree 把前端接上。

## 二、现状摸底（已核实）

**后端已有命令**（`tauri-app/src/lib.rs`，无需改后端）：
- `meta_preview_mvu_apply(source_character_id)` → `Vec<MvuApplyPreview>`：预览每个 definition 的 schema 合并结果（added_fields/changed_fields/has_changes）。
- `meta_apply_mvu_schema(source_character_id, definition_id)` → `Result<(), String>`：应用合并 schema 到 definition + backfill 已有 instance（F3 已修 backfill，用 `list_all_instances`）。
- `meta_analyze_mvu_card(source_character_id)`：分析 MVU 卡（前端已接 `metaAnalyzeMvuCard`）。
- `meta_list_mvu_translations` / `meta_get_mvu_translation`：查翻译记录（前端已接）。

**前端现状**（`tauri-api.js`）：
- ✅ `metaAnalyzeMvuCard`（已接）
- ✅ `metaListMvuTranslations` / `metaGetMvuTranslation`（已接）
- ❌ **缺** `metaPreviewMvuApply`（预览 apply）
- ❌ **缺** `metaApplyMvuSchema`（应用 apply）

**前端 UI 现状**：
- `MetaPanel.vue` 能触发 MVU 分析 + 查看 translation。
- 但**没有"预览 apply diff"和"确认 apply"的 UI 流程**——分析完只能看 translation，不能把 schema 应用到 Campaign。

## 三、任务

### 任务 1：tauri-api.js 补两个 API 函数

改动：`frontend/src/tauri-api.js`。

```js
/** 预览 MVU schema 合并结果（每个 definition 一条 preview） */
export async function metaPreviewMvuApply(sourceCharacterId) {
  return await invoke('meta_preview_mvu_apply', { sourceCharacterId })
}

/** 应用 MVU schema 到指定 definition（写盘 + backfill instance） */
export async function metaApplyMvuSchema(sourceCharacterId, definitionId) {
  return await invoke('meta_apply_mvu_schema', { sourceCharacterId, definitionId })
}
```

### 任务 2：MetaPanel 加 MVU apply 流程

改动：`frontend/src/components/MetaPanel.vue`（或 CampaignPanel，看 MVU 操作现挂在哪）。

流程：
1. 用户已分析 MVU（有 `MvuTranslation`）→ 显示"应用 Schema"按钮。
2. 点击 → 调 `metaPreviewMvuApply(sourceCharacterId)` → 拿到 `Vec<MvuApplyPreview>`。
3. 展示 diff：每个 definition 一行，显示 `added_fields`（新增变量）+ `changed_fields`（变更）+ `has_changes`。无变化的 definition 灰显。
4. 每个 definition 旁有"应用"按钮（`has_changes` 为 true 才可点）。
5. 点击应用 → 调 `metaApplyMvuSchema(sourceCharacterId, definitionId)`。
6. 成功后刷新 CampaignPanel 对应 tab（变量 tab，让用户看到新变量）。

`MvuApplyPreview` 字段（读后端 DTO 确认，大致）：
- `definition_id` / `definition_name`
- `added_fields: Vec<{key,label,value_type,default}>`
- `changed_fields`（类型/默认值变更）
- `has_changes: bool`
- `merged_schema`（合并后的完整 schema，preview 用）

### 任务 3：接通刷新

改动：MetaPanel/CampaignPanel 事件联动。

- apply 成功后，触发 CampaignPanel 的变量 tab 刷新（W5 已让每个 tab 暴露 `refresh()`，或用 emit 通知父组件）。
- 读 W5 的 CampaignInstancesTab 怎么暴露 refresh + 父组件怎么调，复用同样模式。

## 四、关键约束

- **不改后端**（后端命令全有，只接前端）。
- **不碰 W10 的文件**（app-pipeline/infra-plugin-host 的 JS runtime 接通）。
- **apply 是写盘操作**——必须先 preview 展示 diff，用户确认才 apply（后端已强制 `has_changes` 才写）。
- **不 commit**（留给用户审）。
- **npm run build 通过**。

## 五、给 Claude Code 的提示词

```
请阅读 docs/PLAN-PLUGIN-MVU.md 阶段2/3 和 docs/HANDOFF-W9-MVUAPPLY.md（本文件），
然后接通 MVU apply 前端（后端命令已有，只接前端）。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w9-mvuapply
分支：w9-mvuapply

先读这些理解现状:
- frontend/src/tauri-api.js(:839 metaAnalyzeMvuCard 已接;缺 metaPreviewMvuApply/metaApplyMvuSchema)
- crates/tauri-app/src/lib.rs grep meta_preview_mvu_apply/meta_apply_mvu_schema
  (后端命令已有,看 DTO:MvuApplyPreview 字段:definition_id/name/added_fields/
  changed_fields/has_changes/merged_schema)
- frontend/src/components/MetaPanel.vue(MVU 分析现挂这里,加 apply 流程)
- frontend/src/components/CampaignInstancesTab.vue(W5 拆的变量 tab,看 refresh 怎么暴露)

任务:
1. tauri-api.js 补 metaPreviewMvuApply(sourceCharacterId)→Vec<MvuApplyPreview> +
   metaApplyMvuSchema(sourceCharacterId,definitionId)→Result。
2. MetaPanel 加 apply 流程:已分析 MVU→显示「应用 Schema」→调 preview→展示 diff
   (每 definition 一行:added_fields/changed_fields/has_changes)→每 definition 旁
   「应用」按钮(has_changes true 才可点)→调 apply→成功刷新 CampaignPanel 变量 tab。
3. apply 成功后触发变量 tab 刷新(复用 W5 的 refresh 模式)。

红线: 不改后端 / 不碰 app-pipeline/infra-plugin-host(W10) / apply 必须先 preview
后确认(后端已强制 has_changes 才写) / 不 commit / npm run build 通过。

先补 tauri-api.js 两个函数,再做 MetaPanel apply UI,最后接刷新。
```

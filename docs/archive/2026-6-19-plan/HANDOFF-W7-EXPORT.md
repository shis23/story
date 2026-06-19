# W7 执行手册：Campaign 导出 — ST 卡 PNG + 共享 Lorebook + StoryForge JSON Bundle

> 交接对象：Claude Code（在 worktree `storyforge-w7-export` 分支 `w7-export` 工作）
> 前置必读：`docs/PLAN-ST-IMPORT-EXPORT.md`（T4/T5）、`docs/PLAN-PLUGIN-MVU.md`
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w7-export`
> 分支：`w7-export`（已基于 `main` 1945d21）
> 性质：后端为主 + 前端按钮，与 W8（JS runtime）零文件交集。

## 一、任务概述

StoryForge 目前只能导入 ST 卡，**完全不能导出**。本 worktree 实现三种导出：

1. **单角色 → ST 卡 PNG**（含 tEXt "chara" 块）
2. **多角色 Campaign → 多张 PNG + 共享 lorebook**（用户已定：每角色一张 PNG + 共享 lorebook）
3. **StoryForge 专有 JSON bundle**（Campaign 完整状态：元数据/instances/definitions/knowledge/variables/tasks/summaries）

## 二、现状摸底（已核实）

**导入侧（已有，导出反向参考）**：
- `infra-import/src/png.rs`：自写 PNG 解析，能读 tEXt 块（`parse_png` 返回 `PngChunk::Text{keyword,text}`）。**无写 tEXt 能力，要新写 `write_png` 或用 png crate 编码**。
- `infra-import/src/lib.rs:55` `import_character_from_png`：PNG → tEXt "chara" → base64 decode → JSON → `StCharacterCard` → `Character::from_st_card()`。
- `domain/src/character.rs:60` `StCharacterData`：ST 卡 data 字段结构（name/description/personality/.../extensions/character_book）。
- `domain/src/character.rs:179` `from_st_card`：**导入映射**。**无反向 `to_st_card`/`to_st_data`——导出要新写**。
- `Character.raw_card_json`（:33）：导入时保留原始 JSON——**导出回 ST 时优先用 raw_card_json round-trip 保底**，CharacterDefinition 字段覆盖缺失部分。
- `Character.renderable_assets`（:27）：卡图资产——**PNG 底图来源**。

**导出侧（全无，本 worktree 新建）**：
- 无 `to_st_card`/`to_st_data` 反向映射。
- 无 PNG 写 tEXt 能力。
- 无导出命令。
- 无前端导出按钮。

## 三、任务拆解

### 任务 1：反向映射 `CharacterDefinition → StCharacterData`

改动：`crates/domain/src/character.rs` 新增 `to_st_data` 方法。

- 优先用 `raw_card_json`（若 Character 级有保留）round-trip，保证不丢 ST 扩展字段。
- 用 CharacterDefinition 的字段覆盖核心字段（name/description/personality/scenario/first_mes/mes_example/system_prompt/post_history_instructions/tags/creator/character_version/alternate_greetings）。
- `extensions`：若 raw_card_json 有则保留，否则用 definition 的 extensions（含 MVU stat_data 若有）。
- `character_book`：Campaign 共享知识/世界书转成 `StWorldInfoBook`（见任务 3）。

### 任务 2：PNG 写 tEXt 块（导出 ST 卡 PNG）

改动：`crates/infra-import/src/png.rs` 新增 `write_png`（或新模块 `png_writer.rs`）。

- PNG 底图：优先 `renderable_assets` 的图，缺则生成纯色占位 PNG（1x1 或 256x256 纯色）。
- 流程：`StCharacterCard` → serde_json → base64 encode → 写入 tEXt 块 keyword="chara" → 输出 PNG bytes。
- 参考 `parse_png` 的 chunk 格式（CRC、length、type）反向写。
- 可选：用 `png` crate（若已在依赖）的 encoder 写 tEXt，省去手写 CRC。看 Cargo.toml 是否已有 png crate。

### 任务 3：多角色 Campaign → 多张 PNG + 共享 lorebook

改动：`crates/tauri-app/src/lib.rs` 新增导出命令 + DTO。

- 遍历 Campaign 的 CharacterInstances，每个找到对应 CharacterDefinition，各生成一张 ST PNG。
- 共享 lorebook：Campaign 的共享知识（`CharacterKnowledgeEntry` 无角色归属或 Campaign 级世界书）+ 内嵌世界书，转成 ST lorebook 格式（独立 `.json` 或嵌入每张卡的 `character_book`）。
- 打包：多张 PNG + lorebook.json → ZIP？或前端分别下载？**先做分别返回（命令返回 Vec<PngBytes> + lorebook json），前端逐个下载或打包 ZIP**。

### 任务 4：StoryForge JSON bundle 导出

改动：`crates/tauri-app/src/lib.rs` 新增 `export_campaign_bundle` 命令。

- 导出范围：Campaign 元数据 + CharacterInstances + CharacterDefinitions + Knowledge + Variables + Tasks + Summaries。
- 格式：JSON bundle（带版本号 `format_version`，向前兼容）。
- ID 冲突：导入时重新生成 ID（记录 `original_id` 映射），不直接复用。

### 任务 5：前端导出按钮

改动：`frontend/src/components/CampaignPanel.vue` + `frontend/src/tauri-api.js`。

- CampaignPanel 加"导出"按钮组：导出 ST 卡 PNG（单/多）/ 导出 lorebook / 导出 StoryForge bundle。
- tauri-api.js 加 `exportStCardPng`/`exportCampaignBundle` 等函数。
- 前端用 Tauri 的 save 对话框（dialog plugin）让用户选保存位置。

## 四、关键约束

- **不破坏导入**（导入路径零改动，导出是新增）。
- **导出回 ST 用 raw_card_json round-trip 保底**——不丢 ST 扩展字段。
- **多角色策略**（用户已定）：每角色一张 PNG + 共享 lorebook。
- **不碰 W8 的文件**（infra-plugin-host/JS runtime）。
- **不 commit**（留给用户审）。
- PNG 底图缺失时生成占位图，不报错阻断。

## 五、给 Claude Code 的提示词

```
请阅读 docs/PLAN-ST-IMPORT-EXPORT.md（T4/T5）和 docs/HANDOFF-W7-EXPORT.md（本文件），
然后实现 Campaign 导出（ST 卡 PNG + 共享 lorebook + StoryForge JSON bundle）。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w7-export
分支：w7-export

先读这些理解现状：
- crates/infra-import/src/png.rs（parse_png 读 tEXt，无写能力，要新写 write_png）
- crates/infra-import/src/lib.rs:55 import_character_from_png（导入流程，导出反向参考）
- crates/domain/src/character.rs StCharacterData(:60) + from_st_card(:179)
  [无反向 to_st_data，要新写] + raw_card_json(:33) [round-trip 保底] + renderable_assets(:27) [PNG 底图]
- crates/tauri-app/src/campaign_store.rs（Campaign/instance/knowledge/variables/tasks/summaries 存取）

任务（按顺序）:
1. domain/character.rs 新增 to_st_data：CharacterDefinition→StCharacterData。
   优先 raw_card_json round-trip 保底，definition 字段覆盖核心字段。
2. infra-import/src/png.rs 新增 write_png：StCharacterCard→JSON→base64→tEXt"chara"块→PNG。
   底图优先 renderable_assets，缺则占位纯色 PNG。看 Cargo.toml 是否有 png crate 可复用 encoder。
3. lib.rs 新增导出命令：单角色→ST PNG；多角色→Vec<PNG>+共享 lorebook；
   StoryForge JSON bundle（带 format_version）。
4. 前端 CampaignPanel 加导出按钮组 + tauri-api.js 加导出函数（用 Tauri dialog 选保存位置）。

多角色策略（用户已定）：每角色一张 PNG + 共享 lorebook（共享知识/世界书转 ST lorebook 格式）。

验收: cargo test --workspace 0 回归 + 新增导出单测（to_st_data round-trip / PNG tEXt 写入 /
bundle 序列化）；npm run build 通过。

红线: 不破坏导入 / raw_card_json round-trip 保底 / 不碰 infra-plugin-host(JS runtime) /
不 commit。PNG 底图缺失生成占位图不报错。

先做 to_st_data + write_png（后端核心），跑通单角色 PNG 导出，再多角色 + bundle + 前端。
```

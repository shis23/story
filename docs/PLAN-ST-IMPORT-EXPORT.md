# ST 导入/导出计划（SillyTavern Import/Export）

> 状态：已实现（2026-06-19，W7 Campaign 导出已落地：ST 卡 PNG tEXt 写入 + 共享 lorebook + JSON bundle；ST 导入保真已覆盖 V2/V3；多角色识别 fallback 就绪）。
> 关联：`docs/ROADMAP.md` Phase 5（已完成）、`docs/PLAN-CHARACTER-EXTRACTION.md`、`docs/PLAN-PLUGIN-MVU.md`

## 目标

保持 SillyTavern 卡兼容，同时不被 ST 数据形态限制内部架构。覆盖：

1. ST V2/V3 导入保真范围明确化。
2. raw JSON 和 extensions 保留策略。
3. 多角色识别失败时稳定 fallback。
4. StoryForge Campaign 导出格式设计。
5. 评估是否支持导出回 ST 卡或 Lorebook。

## 非目标

- 不做 ST 预设（Preset）的完整导入/导出（当前只读取 regex_scripts，见 `infra-import`）。
- 不做 ST 聊天记录导入。
- 不做 ST 插件系统兼容。

> 2026-07-06 更新：Preset `extensions.regex_scripts` 导入已保留 ST 正则元数据（原始 `placement` 数组、`markdownOnly`、`promptOnly`、`runOnEdit`、`substituteRegex`、`trimStrings`、`minDepth`、`maxDepth`）；角色卡 `data.extensions.regex_scripts` 也可通过 `Character::scoped_regex_scripts()` typed 读取。`merge_regex_script_sources()` 已可按 Global → Preset → Scoped 顺序合并并标记来源。`infra-regex` 执行器已支持 ST 常见 `/pattern/flags` 形式的 `findRegex`。2026-07-06 增量：`WritingContext.regex_scripts` 已接入流水线，首写和重 roll 会执行 Input/Output 正则；Tauri legacy 写作会从本次选中的角色卡收集 Scoped 正则，且 `CharacterInfo.extensions` 已持久化卡内扩展以支持重启后恢复。2026-07-06 追加：`PresetStore` 已持久化 `active_preset.json`，预设面板可设置/清除运行时预设，写作与重 roll 会按 Global → Preset → Scoped 顺序合并 `global_regex_scripts.json`、active Preset 正则和 Scoped 正则；Campaign 模式会从 active Campaign 的 `CharacterCard.raw_card_json.extensions.regex_scripts` 追加卡内 Scoped 正则，并跳过同 ID 的既有 Scoped 脚本以避免 legacy/Campaign 双路径重复执行。Tauri 已提供 `import_global_regex_settings` / `list_global_regex_scripts` / `clear_global_regex_scripts` / `update_global_regex` 命令，预设面板已提供对应的 Global 正则导入、查看、启停、清空 UI，可从 ST settings JSON 中抽取 `regex_scripts` 作为 Global 来源。当前 Input/Output 运行时已优先尊重 ST 原始 `placement_codes`（0=Input，2=Output），`[0,2]` 会在两端执行；World Info、Slash、Reasoning 等非 Input/Output 执行点仍属于后续 ST 运行时兼容工作。

## 当前事实

### ST 导入现状

| 代码位置 | 能力 |
|----------|------|
| `crates/infra-import/src/lib.rs::import_character(data)` | 自动判断 PNG/JSON，调用对应解析 |
| `crates/infra-import/src/lib.rs::import_character_from_json(data)` | JSON → `StCharacterCard` → `Character::from_st_card()` |
| `crates/infra-import/src/lib.rs::import_character_from_png(data)` | PNG tEXt "chara" 块 → Base64 → JSON → 同上 |
| `crates/infra-import/src/lib.rs::import_preset(data)` | ST 预设 JSON → `Preset::from_st()` |
| `crates/domain/src/character.rs::Character::from_st_card()` | 字段映射 + `extensions` 保留 + `raw_card_json` 保留 + 内嵌世界书提取 + 可渲染资产提取 |

### 数据保留情况

- **`extensions`**：完整保留在 `Character.extensions` 字段（`serde_json::Value`）。
- **`raw_card_json`**：`from_st_card()` 时将 `StCharacterData` 序列化为 `raw_card_json`，保留原始字段。
- **`spec_version`**：V2 默认 "2.0"，V3 从 `spec_version` 字段读取。
- **内嵌世界书**：`character_book` → `WorldInfoBook::from_st()`，保留 entries、keys、content、position、order、selective/constant 路由。
- **可渲染资产**：从 `extensions.assets` 或 `extensions.character_assets` 提取。

### 角色识别现状

- `crates/app-agent/src/character_extractor.rs` 已实现多角色识别（见 `PLAN-CHARACTER-EXTRACTION.md`）。
- 5 层兜底解析确保 LLM 输出不规范时仍能提取。
- 识别失败时 `ExtractError::Parse` 会返回错误，调用方可 fallback 到单角色模式。

### ST V2/V3 差异点

- V2：无 `spec_version` 字段，无 `character_book`（或为空），`extensions` 可能为空。
- V3：有 `spec_version: "3.0"`，`character_book` 结构化，`extensions` 含 assets。
- 当前代码通过 `spec_version.unwrap_or("2.0")` 兼容两者。

### 导出现状

- **无导出功能**。当前只能导入，不能导出 Campaign 或角色为 ST 格式或 StoryForge 专有格式。

## 任务

### T1: 明确 ST V2/V3 导入保真范围

**目标**：列出哪些字段必须保留、哪些可以降级、哪些明确不支持。

**当前事实**：`Character::from_st_card()` 已覆盖 name/description/personality/scenario/first_mes/mes_example/system_prompt/post_history_instructions/tags/creator/character_version/alternate_greetings/extensions/raw_card_json/embedded_world_info/renderable_assets。

**待确认**：

- [ ] `extensions` 中哪些子字段有 StoryForge 语义（如 MVU `stat_data`）？当前只有 `assets`/`character_assets` 被提取。
- [ ] `alternate_greetings` 在 Campaign 写作中如何使用？当前只存储不消费。
- [ ] ST 的 `group_only`、`post_history_instructions` 等边缘字段是否需要特殊处理？

### T2: raw JSON 和 extensions 保留策略

**目标**：明确 `raw_card_json` 和 `extensions` 的生命周期和用途。

**当前事实**：两者都是 `serde_json::Value`，导入时写入，不做二次处理。

**待设计**：

- [ ] `raw_card_json` 是否用于"导出回 ST"时的保底？如果是，需确保 round-trip 不丢字段。
- [ ] `extensions` 中的 MVU 数据是否需要在导入时解析为 typed 结构？当前由 `character_extractor` 的 `mvu_schema` 参数处理。
- [ ] 大型 extensions（如 ST 插件数据）的存储成本评估。

### T3: 多角色识别失败时稳定 fallback

**目标**：识别失败时用户仍能正常使用导入的卡。

**当前事实**：`extract_characters()` 返回 `ExtractError::Parse` 时，调用方可 fallback 到 `CharacterDefinition::fallback_from_character()` 生成单角色定义。

**待实现**：

- [ ] 确认 `tauri-app` 导入流程中 fallback 路径已接入。
- [ ] fallback 时是否提示用户"识别失败，已按单角色处理"？
- [ ] 是否支持用户手动触发重跑识别？

### T4: StoryForge Campaign 导出格式设计

**目标**：设计 Campaign 级别的导出格式，支持完整 Campaign 状态持久化或分享。

**待设计**：

- [ ] 导出范围：Campaign 元数据 + CharacterInstances + CharacterDefinitions + Knowledge + Variables + Tasks + Summaries？
- [ ] 格式选择：JSON bundle？ZIP（含 JSON + 附件）？
- [ ] 版本号和向前兼容策略。
- [ ] 导入时如何处理 ID 冲突（同 Campaign 已存在）？

### T5: 评估导出回 ST 卡或 Lorebook

**目标**：评估 StoryForge Campaign 能否导出为 ST 兼容格式。

**评估方向**：

- [ ] 单角色 Campaign → ST 卡：可行，CharacterDefinition 字段可映射回 `StCharacterData`。
- [ ] 多角色 Campaign → 多张 ST 卡 + 共享 Lorebook：需设计角色 → 卡的拆分规则和知识 → Lorebook 条目的映射。
- [ ] 变量/任务 → ST 无对应概念，导出时必然丢失。需明确告知用户。
- [ ] 是否做"导出为 ST 格式"功能，还是只做"导出为 StoryForge 专有格式 + 提供转换工具"？

## 验证

- [ ] 常见 ST V2/V3 卡能导入且不丢关键字段。
- [ ] 不认识的 extensions 不丢（`raw_card_json` 保底）。
- [ ] 多角色识别失败时 fallback 到单角色，不报错崩溃。
- [ ] Campaign 导出 → 导入 round-trip 数据一致（待 T4 完成后验证）。

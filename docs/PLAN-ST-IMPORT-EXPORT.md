# ST 导入/导出计划（SillyTavern Import/Export）

> 状态：已实现（2026-06-19，W7 Campaign 导出已落地：ST 卡 PNG tEXt 写入 + 共享 lorebook + JSON bundle；ST 导入保真已覆盖 V2/V3；多角色识别 fallback 就绪）。2026-07-07 追加：StoryForge Campaign JSON Bundle 已支持导入 round-trip，导入时生成全新 card/campaign/instance/knowledge/task/summary ID 并重写引用，避免覆盖现有数据。2026-07-07 续补：ST `data` 顶层未知字段（如 `group_only`、`creator_notes`）通过 flatten `extra` 保留到 `raw_card_json` 并参与回导 round-trip；单卡 CharacterStore 也已持久化 raw/mes_example/post_history_instructions/character_version/embedded_world_info，使 `export_st_card_png` 不再经过精简 DTO 丢保真字段。
> 关联：`docs/ROADMAP.md` Phase 5（已完成）、`docs/archive/2026-07-08-completed-plans/PLAN-CHARACTER-EXTRACTION.md`、`docs/PLAN-PLUGIN-MVU.md`

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

> 2026-07-06 更新：Preset `extensions.regex_scripts` 导入已保留 ST 正则元数据（原始 `placement` 数组、`markdownOnly`、`promptOnly`、`runOnEdit`、`substituteRegex`、`trimStrings`、`minDepth`、`maxDepth`）；角色卡 `data.extensions.regex_scripts` 也可通过 `Character::scoped_regex_scripts()` typed 读取。`merge_regex_script_sources()` 已可按 Global → Preset → Scoped 顺序合并并标记来源。`infra-regex` 执行器已支持 ST 常见 `/pattern/flags` 形式的 `findRegex`。2026-07-06 增量：`WritingContext.regex_scripts` 已接入流水线，首写和重 roll 会执行 Input/Output 正则；Tauri legacy 写作会从本次选中的角色卡收集 Scoped 正则，且 `CharacterInfo.extensions` 已持久化卡内扩展以支持重启后恢复。2026-07-06 追加：`PresetStore` 已持久化 `active_preset.json`，预设面板可设置/清除运行时预设，写作与重 roll 会按 Global → Preset → Scoped 顺序合并 `global_regex_scripts.json`、active Preset 正则和 Scoped 正则；Campaign 模式会从 active Campaign 的 `CharacterCard.raw_card_json.extensions.regex_scripts` 追加卡内 Scoped 正则，并跳过同 ID 的既有 Scoped 脚本以避免 legacy/Campaign 双路径重复执行。Tauri 已提供 `import_global_regex_settings` / `list_global_regex_scripts` / `clear_global_regex_scripts` / `update_global_regex` 命令，预设面板已提供对应的 Global 正则导入、查看、启停、清空 UI，可从 ST settings JSON 中抽取 `regex_scripts` 作为 Global 来源。当前 Input/Output 运行时已优先尊重 ST 原始 `placement_codes`（当前 ST：1=User Input，2=AI Output），`[1,2]` 会在两端执行；2026-07-07 增量：`infra-regex` 已提供 Prompt/Persisted/Display 执行目标，`app-pipeline` 的 Input 走 Prompt、Output 落盘走 Persisted，`promptOnly` 不再改显示/存储，`markdownOnly` 不再污染 prompt 或持久化文本；Tauri `get_conversation` 已提供派生 `display_content`，前端消息展示使用展示态内容、编辑仍使用原始 `content`；`ChatMessage` 已接入 display-only 派生 HTML 片段安全渲染，原始 `<data_block>` 仍保持转义文本；`minDepth/maxDepth` 已在执行器生效，当前轮 Input/Output 按 depth 0，消息展示按离末尾深度过滤；World Info 正则已按当前 ST placement 5 在世界书注入 prompt 前执行，覆盖常驻、关键词触发和 `search_world_info` 工具返回。Reasoning placement 6 已接到 AI 输出中的 `<think>` / `<thinking>` 推理块，持久化输出与 display-only 派生展示都会按目标分流执行；Slash placement 3 已在用户/导演意图以 `/` 开头时运行，并会先于 Input 正则执行。2026-07-07 续补：插件桥已提供常用 Slash 命令注册/触发 fallback，`PluginHost` 已按 slot 隔离默认内容、slash、sidebar 和 statusbar 挂载，状态栏清空不会误清其他 slot；仍不承诺 ST 99 事件全集、完整 prompt hooks 或冷门 Slash 参数管道全量兼容。

> 2026-07-07 追加：正则替换已遵循 `g` flag 语义，含 `g` 时全局替换，不含 `g` 时只替换首个匹配；这修正了 `/(.*)/s` 这类用户输入包裹脚本被结尾空匹配重复包裹的问题。

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
- **内嵌世界书**：`character_book` → `WorldInfoBook::from_st()`，保留 entries、keys、secondary_keys、content、position、order、selective/constant 路由。Constant/Both 条目进入 Director system；Selective/Both 条目会按本轮 intent 做确定性关键词触发并进入 Director tail，`search_world_info` 工具复用同一套触发规则。
- **可渲染资产**：从 `extensions.assets` 或 `extensions.character_assets` 提取。

### 角色识别现状

- `crates/app-agent/src/character_extractor.rs` 已实现多角色识别（见 `docs/archive/2026-07-08-completed-plans/PLAN-CHARACTER-EXTRACTION.md`）。
- 5 层兜底解析确保 LLM 输出不规范时仍能提取。
- 识别失败时 `ExtractError::Parse` 会返回错误，调用方可 fallback 到单角色模式。

### ST V2/V3 差异点

- V2：无 `spec_version` 字段，无 `character_book`（或为空），`extensions` 可能为空。
- V3：有 `spec_version: "3.0"`，`character_book` 结构化，`extensions` 含 assets。
- 当前代码通过 `spec_version.unwrap_or("2.0")` 兼容两者。

### 导入/导出现状

- `export_st_card_png(character_id)`：单角色卡导出为 ST PNG（含 tEXt `chara` 块）。
- `export_campaign_st_cards(campaign_id)`：Campaign 导出为多张 ST PNG + 共享 lorebook JSON。
- `export_campaign_bundle(campaign_id)`：StoryForge 专有 JSON Bundle v2，包含完整 `CharacterCard`、Campaign 元数据、CharacterInstances、Definitions、Knowledge、Tasks、Summaries。
- `import_campaign_bundle(bundle_json)`：导入 StoryForge JSON Bundle；v2 使用 bundle 内的完整 `CharacterCard`，兼容 v1 只有 `definitions` 的旧 bundle；导入永远生成新 ID 并重写引用，不覆盖现有卡或游玩档。

## 任务

### T1: 明确 ST V2/V3 导入保真范围

**目标**：列出哪些字段必须保留、哪些可以降级、哪些明确不支持。

**当前事实**：`Character::from_st_card()` 已覆盖 name/description/personality/scenario/first_mes/mes_example/system_prompt/post_history_instructions/tags/creator/character_version/alternate_greetings/extensions/raw_card_json/embedded_world_info/renderable_assets；未知 ST `data` 顶层字段会进入 `StCharacterData.extra`，并随 `raw_card_json` / `to_st_data()` 保留。

**待确认**：

- [ ] `extensions` 中哪些子字段有 StoryForge 语义（如 MVU `stat_data`）？当前只有 `assets`/`character_assets` 被提取。
- [x] `alternate_greetings` 在 Campaign 写作中如何使用？当前 legacy 单卡新会话与 Campaign 新建游玩档均可切换并消费默认/备选开场，后端会拒绝不属于源角色卡的开场并回退到首个有效 greeting。
- [x] ST 的 `group_only`、`post_history_instructions` 等边缘字段是否需要特殊处理？`post_history_instructions` 已是 typed 字段；`group_only` 等未知顶层字段按原样保留，不赋予 StoryForge 业务语义。

### T2: raw JSON 和 extensions 保留策略

**目标**：明确 `raw_card_json` 和 `extensions` 的生命周期和用途。

**当前事实**：`extensions` 是 `serde_json::Value`，导入时写入，不做二次处理；`raw_card_json` 由 typed 字段 + `StCharacterData.extra` 组成，可作为导出回 ST 的保底数据。`CharacterInfo` 持久化层已保留 ST 回导需要的 raw/示例对话/后历史指令/版本/内嵌世界书字段，启动恢复和单卡 PNG 导出会使用这些完整字段。

**待设计**：

- [x] `raw_card_json` 是否用于"导出回 ST"时的保底？已用于 `to_st_data()` / `to_st_data_from_card()` 的 base；`test_st_data_unknown_fields_round_trip_through_raw_json` 覆盖未知顶层字段保真，`character_info_restore_preserves_st_round_trip_fields` 覆盖单卡存储恢复链路。
- [ ] `extensions` 中的 MVU 数据是否需要在导入时解析为 typed 结构？当前由 `character_extractor` 的 `mvu_schema` 参数处理。
- [ ] 大型 extensions（如 ST 插件数据）的存储成本评估。

### T3: 多角色识别失败时稳定 fallback

**目标**：识别失败时用户仍能正常使用导入的卡。

**当前事实**：`extract_characters()` 返回 `ExtractError::Parse` 时，调用方可 fallback 到 `CharacterDefinition::fallback_from_character()` 生成单角色定义。

**已实现**：

- [x] 确认 `tauri-app` 导入流程中 fallback 路径已接入。`extract_characters` 失败时会保存单角色 fallback definition，并持久化 `extraction_status = fallback`。
- [x] fallback 时是否提示用户"识别失败，已按单角色处理"？后端 DTO 返回 `extraction_status` / `extraction_message`，前端卡片列表和详情会显示降级状态。
- [x] 是否支持用户手动触发重跑识别？前端提供"重新识别"，调用 `extract_characters(force=true)`；若卡已创建 Campaign，为避免断开既有 `definition_id` 引用，后端会拒绝重跑并提示用户先导入新卡。

### T4: StoryForge Campaign 导出格式设计

**目标**：设计 Campaign 级别的导出格式，支持完整 Campaign 状态持久化或分享。

**已实现**：

- [x] 导出范围：完整 `CharacterCard` + Campaign 元数据 + CharacterInstances + CharacterDefinitions + Knowledge + Variables + Tasks + Summaries。
- [x] 格式选择：JSON Bundle。当前前端保存为 `campaign-bundle.json`；多文件 ZIP 可作为后续包装层，不影响核心格式。
- [x] 版本号和向前兼容策略：`format_version = 2`；导入兼容 v1（无完整 card，仅 definitions）。
- [x] 导入 ID 冲突策略：所有导入对象生成全新 ID，并重写 definition、instance、knowledge chain、task related characters、summary conversation 引用；同时创建新的空 conversation 并绑定到导入 Campaign。

### T5: 评估导出回 ST 卡或 Lorebook

**目标**：评估 StoryForge Campaign 能否导出为 ST 兼容格式。

**已实现/已定边界**：

- [x] 单角色 Campaign → ST 卡：`export_st_card_png` 已以 `raw_card_json` round-trip 为保底写回 ST PNG。
- [x] 多角色 Campaign → 多张 ST 卡 + 共享 Lorebook：`export_campaign_st_cards` 已按每角色一张 PNG + 共享 lorebook JSON 导出。
- [x] 变量/任务 → ST 无对应概念，导出时必然丢失；发布说明和用户指南需持续保留降级边界。
- [x] 当前选择：同时提供 StoryForge JSON Bundle（完整保真）和 ST 兼容导出（降级互通），不再等待独立转换工具。

## 验证

- [ ] 常见 ST V2/V3 卡能导入且不丢关键字段（已补 `scripts/run-real-card-smoke.ps1` 覆盖仓库本地 `test-card.png` 的复杂卡导入保真；发布候选仍需真实卡矩阵和 UI/Campaign/bundle 链路实跑）。
- [x] 不认识的 extensions 不丢（`raw_card_json` 保底）；未知 ST `data` 顶层字段也会经 `extra` 保留。
- [x] 多角色识别失败时 fallback 到单角色，不报错崩溃；`test_card_summary_treats_fallback_as_not_extracted` 覆盖 fallback 不被误报为成功识别，`test_prepare_character_extraction_*` 覆盖手动重跑安全边界。
- [x] Campaign 导出 → 导入 round-trip ID 重写和引用一致性由 `import_campaign_bundle_rewrites_ids_and_references` 覆盖。

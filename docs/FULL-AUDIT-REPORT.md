# StoryForge 全量代码审查报告

> 审查日期：2026-06-19
> 审查方法：6 轮双盲独立审查，每任务独立运行 2 次，共 25 个 Opus Agent 调用
> 审查范围：39,445 行 Rust（15 crate）+ 8,376 行前端（35 文件）= 47,821 行

## 执行摘要

| 严重度 | 数量 | 说明 |
|--------|------|------|
| **Critical** | 1 | 向量存储角色删除完全失效 |
| **High** | 15 | 数据丢失风险、安全漏洞、架构违规 |
| **Medium** | 35 | 错误处理缺陷、性能问题、响应式 bug |
| **Low** | 40+ | 死代码、代码质量、小问题 |

---

## Critical（1 条）

### C-001: ✅ `delete_by_character` 使用错误的元数据键已修复

- **文件**: `crates/infra-vector/src/lib.rs:340`
- **历史代码**: `records.retain(|_, r| r.metadata.get("character_id") != Some(&target));`
- **问题**: 所有插入代码使用 `"owner_character_id"` 作为元数据键（见 `MetadataFilter::for_character` 第 81 行），但 `delete_by_character` 查询 `"character_id"`。键名不匹配导致此函数永远不会删除任何记录。角色知识清理完全失效。
- **影响**: 删除角色时，其向量知识条目永远残留，占用空间且可能在搜索中返回已删除角色的数据。
- **修复**: 改为 `"owner_character_id"`，并补充单元测试。
- **当前状态**: `delete_by_character()` 已查询 `"owner_character_id"`，`test_delete_by_character` 覆盖按角色删除行为。
- **置信度**: R6 双盲一致确认 ✅✅

---

## High（15 条）

### H-001: ✅ 所有 Store 的 `persist()` 吞掉磁盘写入错误已修复

- **文件**: `crates/tauri-app/src/campaign_store.rs:499`, `crates/tauri-app/src/storage.rs:228`, `crates/tauri-app/src/connection_store.rs:166`, `crates/tauri-app/src/preset_store.rs`
- **问题**: `CampaignStore::save_campaign`, `add_instance`, `add_knowledge` 等所有 CRUD 方法返回 `()`。`persist()` 调用 `atomic_write_json` 失败时只记录日志，不传播错误。磁盘满或权限错误时用户以为保存成功，实际数据丢失。
- **修复**: 使 `persist()` 返回 `Result`，调用者传播错误。
- **当前状态（2026-07-06）**: `CampaignStore`、`CharacterStore`、`ConnectionStore` 等写入路径已改为返回 `Result`；主要 Tauri 命令会返回结构化 `storage` 错误，postprocess 后台写回失败会记录 warning。`CampaignStore` 单 Mutex 已拆为集合级锁；剩余风险转为同步 I/O 性能问题，见 H-013/H-014。
- **置信度**: R4 双盲 ✅✅ + R6 验证确认

### H-002: ✅ API key 明文存储已修复

- **文件**: `crates/tauri-app/src/connection_store.rs:42`, `crates/tauri-app/src/lib.rs:95`
- **问题**: LLM 和 Embedding API key 以明文 JSON 存储在 `data/connections.json` 和 `data/embed.json`。代码有 TODO 注释承认此问题。
- **修复**: 使用 OS 密钥链（Windows Credential Manager / macOS Keychain）。
- **当前状态（2026-07-06）**: 已新增 `storyforge-infra-util::secret_store`，通过 `keyring` 写入系统凭据库；`connections.json` 和 `embed.json` 只保存 `storyforge-secret:v1:*` 引用。旧明文文件在加载时迁移，运行时再解析回真实 key；真实 LLM harness 已支持从 SecretRef 解析 active 连接。Windows Credential Manager 写/读/删已用 ignored 冒烟测试实跑通过；Android arm64 后端已编译通过，真机写读删仍待验证。
- **置信度**: R1 双盲 ✅✅

### H-003: ✅ `data_dir` 基于可执行文件路径而非 OS 标准目录已修复

- **文件**: `crates/tauri-app/src/lib.rs:75-83`
- **问题**: `get_app_data_dir()` 使用 `exe_dir.join("data")` 而非 `%APPDATA%`。共享安装位置下数据可能被其他用户读取。
- **修复**: 使用 Tauri 的 `app_data_dir`。
- **当前状态（2026-07-06）**: 生产路径已使用 OS 标准数据目录并保留旧目录迁移；`AppState::new_for_test()` 使用临时目录，避免单元测试读取真实用户数据。
- **置信度**: R1 双盲 ✅✅

### H-004: ✅ `CharacterDefinition.role_type` 缺少 `#[serde(default)]` 已修复

- **文件**: `crates/domain/src/character.rs:418`
- **问题**: 旧 JSON 数据缺少 `role_type` 字段时反序列化会失败，而非回退到默认值 `Supporting`。
- **修复**: 添加 `#[serde(default)]`。
- **当前状态**: `role_type` 已有 `#[serde(default)]`，`test_role_type_default_is_supporting` 覆盖旧数据兼容。
- **置信度**: R2 双盲 ✅✅ + R6 验证确认

### H-005: ✅ `ToolContext.campaign_runtime` 不含临时 instance 已修复

- **文件**: `crates/app-pipeline/src/lib.rs:366` + `crates/app-agent/src/runtime.rs:610`
- **问题**: `effective_runtime`（含临时角色）未传播到子 Agent 的 ToolContext。子 Agent 的 `get_character` 工具对临时角色返回 NotFound。
- **修复**: 将 effective_runtime 写入子 Agent 的 ToolContext。
- **当前状态**: `PipelineOrchestrator` 会把 `with_temporaries_for()` 生成的 `effective_runtime` 传入 `spawn_subagents()`；子 Agent 独立 `ToolContext` 设置 `campaign_runtime` 和 `current_character_instance_id`，`test_spawn_subagents_with_temporary_instance` 覆盖临时角色路径。
- **置信度**: R2 双盲 ✅✅

### H-006: ✅ BaseOverlay ESC 监听器泄漏已修复

- **文件**: `frontend/src/components/base/BaseOverlay.vue:82-92`
- **问题**: 每次打开 overlay 添加 keydown 监听器，非 Escape 方式关闭时监听器永远不被移除。N 次开关后按一次 Escape 触发 N 次 close()。
- **修复**: 在 modelValue 变为 false 时也移除监听器。
- **当前状态（2026-07-07）**: 已抽出 `documentKeydownController` 统一管理 document keydown listener；`BaseOverlay` 使用 `immediate` watcher 覆盖初始打开态，并在关闭/卸载时释放监听器。新增 `frontend/tests/base-keydown.test.mjs` 覆盖重复 enable 不累积、disable/dispose 移除、非 Escape 不触发。
- **置信度**: R4 双盲 ✅✅

### H-007: ✅ BaseDropdown ESC 监听器同样的泄漏已修复

- **文件**: `frontend/src/components/base/BaseDropdown.vue:30-39`
- **修复**: 同 H-006。
- **当前状态（2026-07-07）**: `BaseDropdown` 复用 `documentKeydownController`，打开态初始挂载也会注册 ESC，关闭/卸载都会清理。
- **置信度**: R4 双盲 ✅✅

### H-008: ✅ AppSidebar navItems 非响应式已修复

- **文件**: `frontend/src/components/AppSidebar.vue:36-55`
- **问题**: `navItems` 是普通数组而非 `computed`。badge 计数、连接状态、Campaign 高亮在首次渲染后永远不更新。
- **修复**: 改为 `computed(() => [...])`。
- **当前状态**: `navItems` 已是 `computed(() => [...])`，依赖的连接状态、Campaign 计数和激活态会随响应式源更新。
- **置信度**: R4 双盲 ✅✅

### H-009: ✅ LLM 重试逻辑已修复

- **文件**: `crates/app-agent/src/runtime.rs:104`
- **问题**: 单次 500/超时/429 立即失败整个写作流程。`infra-llm` 已分类错误变体（RateLimited/ServerError/Timeout）但从未用于重试决策。
- **修复**: 新增 `RetryConfig`、`LlmError::is_retryable()` 与 `RetryingClient`，对 RateLimited/ServerError/Timeout 使用指数退避重试，并支持 `Retry-After` 与流式取消安全。
- **当前状态**: `crates/infra-llm/src/retry.rs` 已导出 `with_retry`/`RetryingClient`，并覆盖成功重试、不可重试错误、重试耗尽和 Retry-After 解析测试。
- **置信度**: R4 双盲 ✅✅

### H-010: ✅ Tauri 层丢失错误类型信息已修复

- **文件**: `crates/tauri-app/src/lib.rs`（所有 `Result<_, String>` Tauri 命令）
- **问题**: 所有类型化错误被 flatten 为 `format!("写作失败: {e}")`。前端无法区分可重试/不可重试错误。
- **修复**: 定义结构化错误 DTO 返回前端。
- **当前状态**: `crates/tauri-app/src/error.rs` 已定义 `TauriCommandError`，使用 `serde(tag = "type")` 返回结构化错误，并覆盖 LLM/Agent/Pipeline/Storage/Validation/NotFound/Cancelled 等转换测试。
- **置信度**: R4 双盲 ✅✅

### H-011: ✅ 损坏 JSON 文件静默返回空数据已修复

- **文件**: `storage.rs:28`, `preset_store.rs:26`, `connection_store.rs:45`, `infra-vector:190`, `infra-plugin-host:109`, `module_store.rs:417`
- **问题**: 主文件 + 备份文件都损坏时静默返回空 Vec/HashMap。所有用户数据丢失无提示。
- **修复**: 返回错误而非空数据，提示用户数据损坏。
- **当前状态**: 相关 store 的 JSON 加载路径会记录错误并保存 `.json.corrupt` 备份；向量库路径另有 `.tmp` 恢复与 `.corrupt` 备份回归测试。
- **置信度**: R4 双盲 ✅✅

### H-012: ✅ infra-plugin-host Tauri 依赖已解除（2026-07-06）

- **文件**: `crates/infra-plugin-host/Cargo.toml`, `crates/tauri-app/src/mvu_webview_runtime.rs`
- **问题**: 基础设施层 crate 曾直接依赖 UI 框架。阻止在非 Tauri 环境（CLI、harness）中使用。
- **修复**: 已将 `WebViewMvuRuntime` 移入 `tauri-app` adapter，`infra-plugin-host` 只保留 `MvuRuntime` trait/DTO/事件协议。
- **置信度**: R5 单次确认

### H-013: CampaignStore 曾持锁做 7 次文件写入（部分修复 2026-07-06）

- **文件**: `crates/tauri-app/src/campaign_store.rs:148-186`
- **问题**: `delete_card` 过去在单个 Mutex 锁内顺序执行 7 次 `persist()` 文件写入，所有并发读者会被阻塞。当前已拆为集合级锁，普通跨集合读写不再共享同一把锁；级联删除仍会按固定顺序持有受影响集合锁并同步写盘。
- **修复**: 已完成集合级锁拆分，并增加并发写回回放测试覆盖跨集合写入后重载一致性。2026-07-07 已将写作/重 roll 入口的 Campaign 快照读取搬到 `spawn_blocking`；同日新增临时 instance 落盘、postprocess summary/knowledge/variable/task 写回、对话变体采纳写盘 `spawn_blocking` 切片；嵌入配置写盘属于 H-014 范围，已单独 offload。剩余工作是继续压测同步 JSON 写入，并评估其他命令写入、后台 flush / 更广泛 `spawn_blocking`。
- **置信度**: R5 单次确认 + 并发回放测试

### H-014: 同步 `std::fs` 调用阻塞 tokio 异步运行时

- **文件**: `crates/tauri-app/src/lib.rs`, `campaign_store.rs` 全部 persist
- **问题**: async Tauri 命令过去会直接调用同步 `fill_campaign_context` 读取 active Campaign 与 CampaignStore 快照。在慢磁盘上可能导致 UI 卡顿；其他 store 写入路径仍是同步 JSON I/O。
- **当前状态（2026-07-07）**: `start_writing` / `regenerate` 已改用 `fill_campaign_context_async`，在清空旧 runtime 后通过 `tokio::task::spawn_blocking` 加载 active Campaign 与 CampaignRuntimeContext 快照，再回到 async 主线写入 `WritingContext` / `ToolContext`。成功写作/重 roll 后的临时 instance 落盘已通过 `persist_temporary_instances_async` offload，随后再跑 postprocess，保证知识/变量写回仍能看到临时角色；postprocess summary/knowledge/variable/task 写回也已通过 `persist_postprocess_outcome_async` offload 到 blocking pool；`accept_variant` 采纳对话变体的 `ConversationStore` 写盘已通过 `accept_variant_async` offload，保留自动归档触发顺序；`configure_embedder` 已改为 async Tauri command，并把 `embed.json` / SecretRef 持久化移入 `configure_embedder_async` 的 `spawn_blocking`；手动/自动归档读取对话消息已共用 `archivable_messages_async`，在 blocking pool 中读取 `ConversationStore` 并过滤 Discarded 变体；连接创建、删除和切换 active 已通过 active connection async helpers 将 `connections.json` / SecretRef / `last_used_at` 写盘 offload 到 blocking pool，并用同一 async mutex 序列化“写盘 + 更新内存 LLM client”；`meta_analyze_mvu_card` 已通过 `save_mvu_translation_async` 将 `mvu_translations.json` 保存 offload 到 blocking pool，并统一返回/持久化的 `analyzed_at`；`extract_characters` 已通过 `save_character_card_async` 将识别后的 `cards.json` 保存 offload 到 blocking pool。新增 `test_campaign_context_snapshot_applies_runtime_to_contexts`、`test_postprocess_persistence_helper_writes_all_campaign_outputs`、`test_accept_variant_async_persists_final_variant`、`test_configure_embedder_async_persists_secret_ref_and_updates_state`、`test_archivable_messages_async_filters_discarded_variants`、`test_set_active_connection_async_persists_and_updates_state`、`test_set_active_connection_async_serializes_concurrent_updates`、`test_create_connection_async_auto_activates_first_connection`、`test_delete_connection_async_serializes_with_set_active`、`test_save_mvu_translation_async_persists_and_replaces_existing`、`test_save_character_card_async_persists_and_replaces_source` 与临时 instance 回归测试覆盖关键行为。剩余为其他 store 写入与后台 flush 评估。
- **置信度**: R5 单次确认

### H-015: ✅ 向量存储加载 IO 错误静默返回空数据已修复

- **文件**: `crates/infra-vector/src/lib.rs:200`
- **问题**: 文件读取失败时（Windows 文件锁、权限）返回空 HashMap。下次 persist 覆盖为全空，永久丢失数据。
- **修复**: 记录错误并尝试恢复或提示用户。
- **当前状态**: `BruteForceStore::with_persistence()` 会先尝试主文件，再尝试 `.tmp` 备份；主文件损坏且无可用备份时会保存 `.json.corrupt`。新增测试覆盖 `.tmp` 恢复与 `.corrupt` 备份。
- **置信度**: R6 验证确认

---

## Medium（35 条 — 关键条目）

| ID | 文件 | 描述 |
|----|------|------|
| M-001 | `runtime.rs:160` | ✅ 已修复：畸形 tool-call 参数不会执行真实工具，runtime 直接把 `Invalid JSON arguments` 作为 tool result 反馈给 LLM |
| M-002 | `campaign_runtime.rs:103` | ✅ 已修复：`with_temporaries_for()` 会跳过空白 unmatched character_id，避免通过 `temporary_with_overrides` 生成 `Unknown Character` 临时角色 |
| M-003 | `conversation.rs:87` | ✅ 已修复：`MessageNode` 手写反序列化会钳制越界 `active_variant`，并补回归测试 |
| M-004 | `campaign.rs:88` | ✅ 已修复：`Campaign::set_variable("story_clock", ...)` 同步顶层字段，`current_story_clock()` 以变量为权威并兼容旧数据 |
| M-005 | `lib.rs:1576` | ✅ 已修复：`postprocess_variable_keys()` 从 CampaignRuntimeContext 合并默认角色/全局变量、当前 Campaign 变量、定义 schema 与实例变量 |
| M-006 | `app-pipeline:1069` | ✅ 已修复：多个 Subagent regenerate targets 会全部重跑，并按 `character_id` 回填，避免 target 顺序打乱旧 provenance/plan 顺序 |
| M-007 | `runtime.rs:448` | ✅ 已修复：`run_tool_loop_with_layout` 与其他循环一致，命中 `terminal_tools` 后立即返回 |
| M-008 | `campaign_runtime.rs:103` | ✅ 已修复：`with_temporaries_for()` 使用 lowercase key 去重，避免大小写差异生成重复临时角色 |
| M-009 | `plugin-bridge.js:78` | ✅ 已修复：插件侧 `postMessage` 使用注入的宿主 origin；宿主侧只处理当前 iframe source，响应优先回传请求 origin |
| M-010 | `Composer.vue:3` | ✅ 已修复：`sampleIntent` 已内联为本地常量，不再导入 `mock.js` |
| M-011 | `AgentProfileManager.vue:97` | ✅ 已修复：Profile ID 使用 `crypto.randomUUID()` 生成，避免 `Date.now()` 碰撞 |
| M-012 | `MetaPanel.vue:17` | ✅ 2026-07-07 已修：`App.vue` 传入最后一条带 provenance 的 assistant 节点，生成溯源入口不再是死代码 |
| M-013 | `CampaignInstancesTab.vue:184` | ✅ 已修复：JSON 变量编辑会先 `JSON.parse()`，解析失败阻止保存并提示错误 |
| M-014 | 多个 Tab 组件 | ✅ 已修复：Instances/Knowledge/Tasks/Summaries Tab 已监听 `campaignId` 变化并重新加载 |
| M-015 | `BaseOverlay.vue:95` | ✅ 已修复：body overflow 使用模块级引用计数，多个 overlay 关闭顺序不再互相解锁 |
| M-016 | `connection_store.rs:108` | ✅ 已修复：`ConnectionStore` 生产路径使用 `.unwrap_or_else(|p| p.into_inner())` 恢复毒锁 |
| M-017 | `archiver.rs:223` | ✅ 已修复：归档器从调用方传入模型名，`archive_batch_uses_supplied_model` 固化不再硬编码 `"deepseek-chat"` |
| M-018 | `embedder.rs:119` | ✅ 已修复：embedding 非数值 JSON 元素返回 `LlmError::Internal`，并由 `test_parse_embedding_non_numeric_element_errors` 覆盖 |
| M-019 | `sse.rs:175` | ✅ 已确认非问题：SSE 解析器由 `http_client.rs` 流循环驱动，调用点用 `tokio::select!` 监听取消并返回 `LlmError::Cancelled` |
| M-020 | `lib.rs:287` | ✅ 已修复：日志写入 `writeln!` 失败会输出 stderr，不再静默丢弃 |
| M-021 | `crates/app-logging/src/lib.rs` | ✅ 2026-07-07 已修复：LogStore JSONL 落盘使用专用互斥锁，并发回放测试验证完整行 |
| M-022 | 多处 | ✅ 2026-07-07 已修复：用户可见 JSON 序列化失败返回结构化错误，不再伪造 null/default |
| M-023 | `crates/tauri-app/src/lib.rs` | ✅ 2026-07-07 已加强：Patch 执行后 WorldInfoEntry 反序列化失败直接返回结构化错误，不再静默丢条目 |
| M-024 | `crates/app-meta/src/lib.rs` | ✅ 2026-07-07 已修复：`execute_patch` 使用工作副本事务执行，失败时不回写已执行动作 |
| M-025 | `app-meta Cargo.toml` | ✅ 已修复：`storyforge-app-meta` 不再依赖 `infra-plugin-host` |
| M-026 | `BaseOverlay.vue:102` | ✅ 已修复：外层弹层补充 `role="dialog"` / `aria-modal` |
| M-027 | `AgentConfigCard.vue:36` | ✅ 已修复：`activeConnName` 使用 `computed()` |
| M-028 | `PluginHost.vue:86` | ✅ 已修复：移除插件对象 deep watch，避免不必要重建 |
| M-029 | `MetaPanel.vue:250` | ✅ 已修复：typed patch stale preview 使用 `Promise.all` 并发检查 |
| M-030 | `App.vue:934` | ✅ 2026-07-07 已修复：Campaign 名称/概览字段使用可选链保护 |

---

## Low（关键条目）

| ID | 文件 | 描述 |
|----|------|------|
| L-001 | `connection_store.rs:133` | ✅ 已修复：删除 `ConnectionStore::active_id()` 死 API，测试改用 `active_connection()` 验证行为 |
| L-002 | `character_extractor.rs:249` | ✅ 已修复：删除 `match_braces()` wrapper，调用点直接使用 `llm_parse::match_braces` |
| L-003 | `tools.rs:487` | ✅ 已修复：`register_editor_tools()` 已删除 |
| L-004 | `harness-real-llm:301` | ✅ 已修复：删除零引用 `RuntimeCtx` type alias |
| L-005 | `infra-util:61` | ✅ 已修复：删除零生产调用的 `recover_*()` 公共函数，保留各调用点就地 poison recovery |
| L-006 | 多个 Cargo.toml | 19 个未使用 Cargo 依赖 |
| L-007 | `app-pipeline:128` | ✅ 已修复：`WritingContext.recent_messages` 字段已移除，历史消息由 `ConversationStore::recent_messages_as_chat()` 按需读取 |
| L-008 | `PipelinePanel.vue` | ✅ 已修复：死组件文件已删除 |
| L-009 | `App.vue:1053` | ✅ 已修复：`showCharDetail` 已移除 |
| L-010 | `useTheme.js:26` | ✅ 已修复：theme watcher 在模块作用域注册一次，不随 `useTheme()` 调用累积 |
| L-011 | `ChatMessage.vue:23` | ✅ 已修复：`rerolling` 未读 ref 已移除 |
| L-012 | 多处 | ✅ 已修复：当前前端未发现 `v-for` 使用 index/idx/i 作 key |
| L-013 | 多处 | 硬编码中文字符串无 i18n |

---

## 正面发现（代码质量良好）

- ✅ 无 `unsafe` 代码
- ✅ 全类型化错误系统（18 个 thiserror 枚举，零 anyhow）
- ✅ 毒锁恢复模式 80+ 处一致使用
- ✅ `v-html` + `formatContent` XSS 安全（先转义后格式化）
- ✅ Plugin sandbox 配置正确（`allow-scripts` 无 `allow-same-origin`）
- ✅ API key Debug impl 正确掩码为 `***`
- ✅ 无路径遍历风险
- ✅ PipelineEvent 全部 17 变体在前端被处理
- ✅ Tool whitelist 三层同步正确
- ✅ 临时 instance 生命周期正确
- ✅ 测试覆盖 ~603 个测试函数 + 38 个集成测试
- ✅ Crate 依赖 DAG 无环，所有硬规则遵守

---

## 优先修复计划

### Quick Fix（< 30 分钟/项）

1. **C-001**: ✅ `infra-vector:340` 已从 `"character_id"` 改为 `"owner_character_id"`
2. **M-016**: ✅ `connection_store.rs` 生产路径已使用 `.unwrap_or_else(|p| p.into_inner())`
3. **H-004**: ✅ `character.rs:418` 已添加 `#[serde(default)]`
4. **L-001/L-002/L-003/L-004**: ✅ 死函数/类型/API 已删除或确认移除
5. **M-010**: ✅ `Composer.vue:3` 已将 `sampleIntent` 内联为常量
6. **H-008**: ✅ `AppSidebar.vue` navItems 已改为 computed
7. **L-006**: 移除 19 个未使用 Cargo 依赖

### Medium Refactor（1-4 小时/项）

8. **H-006/H-007**: ✅ BaseOverlay/BaseDropdown ESC 监听器生命周期已统一并补测试（2026-07-07）
9. **H-001**: ✅ Store persist 已返回 Result 并向命令层传播错误
10. **M-001**: ✅ tool-call 参数解析错误已反馈给 LLM，且不再 dispatch 真实工具（2026-07-07）
11. **M-005**: ✅ `postprocess_variable_keys()` 已合并实际 runtime schema/value keys（2026-07-07）
12. **M-014**: ✅ Tab 组件已添加 campaignId watch
13. **H-009**: ✅ LLM 重试逻辑（指数退避）已实现
14. **M-024**: ✅ patch 执行已添加事务回滚（2026-07-07）

### Major Restructure（> 4 小时/项）

15. **H-014**: 全量同步 I/O 替换为异步（写作/重 roll Campaign 快照读取、临时 instance 落盘、postprocess 写回、accept variant 写盘、configure embedder 写盘、归档消息读取、连接 active-state 写盘、MVU 翻译保存、角色识别卡保存已先行 `spawn_blocking`）
16. **H-013 剩余项**: CampaignStore 后台 flush / 异步写入评估（集合级锁、快照读取 offload、临时 instance 落盘 offload、postprocess 写回 offload 已完成）
17. **H-002**: ✅ API key 加密存储已完成（Android 真机 keyring 仍需发布前验证）
18. **H-012**: infra-plugin-host 移除直接 Tauri 依赖（已完成 2026-07-06）
19. **R3 Top5**: CampaignRuntimeContext / ToolContext 大对象改为 Arc 共享
20. 测试覆盖：app-pipeline、storage、app-memory 补测试

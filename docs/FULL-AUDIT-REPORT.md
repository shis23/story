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

### C-001: `delete_by_character` 使用错误的元数据键 — 角色知识删除完全失效

- **文件**: `crates/infra-vector/src/lib.rs:340`
- **代码**: `records.retain(|_, r| r.metadata.get("character_id") != Some(&target));`
- **问题**: 所有插入代码使用 `"owner_character_id"` 作为元数据键（见 `MetadataFilter::for_character` 第 81 行），但 `delete_by_character` 查询 `"character_id"`。键名不匹配导致此函数永远不会删除任何记录。角色知识清理完全失效。
- **影响**: 删除角色时，其向量知识条目永远残留，占用空间且可能在搜索中返回已删除角色的数据。
- **修复**: 改为 `"owner_character_id"`，并补充单元测试。
- **置信度**: R6 双盲一致确认 ✅✅

---

## High（15 条）

### H-001: 所有 Store 的 `persist()` 吞掉磁盘写入错误

- **文件**: `crates/tauri-app/src/campaign_store.rs:499`, `crates/tauri-app/src/storage.rs:228`, `crates/tauri-app/src/connection_store.rs:166`, `crates/tauri-app/src/preset_store.rs`
- **问题**: `CampaignStore::save_campaign`, `add_instance`, `add_knowledge` 等所有 CRUD 方法返回 `()`。`persist()` 调用 `atomic_write_json` 失败时只记录日志，不传播错误。磁盘满或权限错误时用户以为保存成功，实际数据丢失。
- **修复**: 使 `persist()` 返回 `Result`，调用者传播错误。
- **当前状态（2026-07-06）**: `CampaignStore`、`CharacterStore`、`ConnectionStore` 等写入路径已改为返回 `Result`；主要 Tauri 命令会返回结构化 `storage` 错误，postprocess 后台写回失败会记录 warning。`CampaignStore` 单 Mutex 已拆为集合级锁；剩余风险转为同步 I/O 性能问题，见 H-013/H-014。
- **置信度**: R4 双盲 ✅✅ + R6 验证确认

### H-002: API key 明文存储

- **文件**: `crates/tauri-app/src/connection_store.rs:42`, `crates/tauri-app/src/lib.rs:95`
- **问题**: LLM 和 Embedding API key 以明文 JSON 存储在 `data/connections.json` 和 `data/embed.json`。代码有 TODO 注释承认此问题。
- **修复**: 使用 OS 密钥链（Windows Credential Manager / macOS Keychain）。
- **当前状态（2026-07-06）**: 已新增 `storyforge-infra-util::secret_store`，通过 `keyring` 写入系统凭据库；`connections.json` 和 `embed.json` 只保存 `storyforge-secret:v1:*` 引用。旧明文文件在加载时迁移，运行时再解析回真实 key；真实 LLM harness 已支持从 SecretRef 解析 active 连接。Windows Credential Manager 写/读/删已用 ignored 冒烟测试实跑通过；Android arm64 后端已编译通过，真机写读删仍待验证。
- **置信度**: R1 双盲 ✅✅

### H-003: `data_dir` 基于可执行文件路径而非 OS 标准目录

- **文件**: `crates/tauri-app/src/lib.rs:75-83`
- **问题**: `get_app_data_dir()` 使用 `exe_dir.join("data")` 而非 `%APPDATA%`。共享安装位置下数据可能被其他用户读取。
- **修复**: 使用 Tauri 的 `app_data_dir`。
- **当前状态（2026-07-06）**: 生产路径已使用 OS 标准数据目录并保留旧目录迁移；`AppState::new_for_test()` 使用临时目录，避免单元测试读取真实用户数据。
- **置信度**: R1 双盲 ✅✅

### H-004: `CharacterDefinition.role_type` 缺少 `#[serde(default)]`

- **文件**: `crates/domain/src/character.rs:418`
- **问题**: 旧 JSON 数据缺少 `role_type` 字段时反序列化会失败，而非回退到默认值 `Supporting`。
- **修复**: 添加 `#[serde(default)]`。
- **置信度**: R2 双盲 ✅✅ + R6 验证确认

### H-005: `ToolContext.campaign_runtime` 不含临时 instance

- **文件**: `crates/app-pipeline/src/lib.rs:366` + `crates/app-agent/src/runtime.rs:610`
- **问题**: `effective_runtime`（含临时角色）未传播到子 Agent 的 ToolContext。子 Agent 的 `get_character` 工具对临时角色返回 NotFound。
- **修复**: 将 effective_runtime 写入子 Agent 的 ToolContext。
- **置信度**: R2 双盲 ✅✅

### H-006: BaseOverlay ESC 监听器泄漏

- **文件**: `frontend/src/components/base/BaseOverlay.vue:82-92`
- **问题**: 每次打开 overlay 添加 keydown 监听器，非 Escape 方式关闭时监听器永远不被移除。N 次开关后按一次 Escape 触发 N 次 close()。
- **修复**: 在 modelValue 变为 false 时也移除监听器。
- **置信度**: R4 双盲 ✅✅

### H-007: BaseDropdown ESC 监听器同样的泄漏

- **文件**: `frontend/src/components/base/BaseDropdown.vue:30-39`
- **修复**: 同 H-006。
- **置信度**: R4 双盲 ✅✅

### H-008: AppSidebar navItems 非响应式

- **文件**: `frontend/src/components/AppSidebar.vue:36-55`
- **问题**: `navItems` 是普通数组而非 `computed`。badge 计数、连接状态、Campaign 高亮在首次渲染后永远不更新。
- **修复**: 改为 `computed(() => [...])`。
- **置信度**: R4 双盲 ✅✅

### H-009: 无 LLM 重试逻辑

- **文件**: `crates/app-agent/src/runtime.rs:104`
- **问题**: 单次 500/超时/429 立即失败整个写作流程。`infra-llm` 已分类错误变体（RateLimited/ServerError/Timeout）但从未用于重试决策。
- **修复**: 在 AgentRuntime 中添加指数退避重试。
- **置信度**: R4 双盲 ✅✅

### H-010: Tauri 层丢失错误类型信息

- **文件**: `crates/tauri-app/src/lib.rs`（所有 `Result<_, String>` Tauri 命令）
- **问题**: 所有类型化错误被 flatten 为 `format!("写作失败: {e}")`。前端无法区分可重试/不可重试错误。
- **修复**: 定义结构化错误 DTO 返回前端。
- **置信度**: R4 双盲 ✅✅

### H-011: 损坏 JSON 文件静默返回空数据

- **文件**: `storage.rs:28`, `preset_store.rs:26`, `connection_store.rs:45`, `infra-vector:190`, `infra-plugin-host:109`, `module_store.rs:417`
- **问题**: 主文件 + 备份文件都损坏时静默返回空 Vec/HashMap。所有用户数据丢失无提示。
- **修复**: 返回错误而非空数据，提示用户数据损坏。
- **置信度**: R4 双盲 ✅✅

### H-012: infra-plugin-host Tauri 依赖已解除（2026-07-06）

- **文件**: `crates/infra-plugin-host/Cargo.toml`, `crates/tauri-app/src/mvu_webview_runtime.rs`
- **问题**: 基础设施层 crate 曾直接依赖 UI 框架。阻止在非 Tauri 环境（CLI、harness）中使用。
- **修复**: 已将 `WebViewMvuRuntime` 移入 `tauri-app` adapter，`infra-plugin-host` 只保留 `MvuRuntime` trait/DTO/事件协议。
- **置信度**: R5 单次确认

### H-013: CampaignStore 曾持锁做 7 次文件写入（部分修复 2026-07-06）

- **文件**: `crates/tauri-app/src/campaign_store.rs:148-186`
- **问题**: `delete_card` 过去在单个 Mutex 锁内顺序执行 7 次 `persist()` 文件写入，所有并发读者会被阻塞。当前已拆为集合级锁，普通跨集合读写不再共享同一把锁；级联删除仍会按固定顺序持有受影响集合锁并同步写盘。
- **修复**: 已完成集合级锁拆分，并增加并发写回回放测试覆盖跨集合写入后重载一致性。剩余工作是压测同步 JSON I/O，并评估后台 flush / `spawn_blocking`。
- **置信度**: R5 单次确认 + 并发回放测试

### H-014: 同步 `std::fs` 调用阻塞 tokio 异步运行时

- **文件**: `crates/tauri-app/src/lib.rs:81,104`, `campaign_store.rs` 全部 persist
- **问题**: `fill_campaign_context`（从 async Tauri 命令调用）内部执行同步文件读写。在慢磁盘上导致 UI 卡顿。
- **修复`: 使用 `tokio::fs` 或 `spawn_blocking`。
- **置信度**: R5 单次确认

### H-015: 向量存储加载 IO 错误静默返回空数据

- **文件**: `crates/infra-vector/src/lib.rs:200`
- **问题**: 文件读取失败时（Windows 文件锁、权限）返回空 HashMap。下次 persist 覆盖为全空，永久丢失数据。
- **修复**: 记录错误并尝试恢复或提示用户。
- **置信度**: R6 验证确认

---

## Medium（35 条 — 关键条目）

| ID | 文件 | 描述 |
|----|------|------|
| M-001 | `runtime.rs:160` | 畸形 tool-call 参数静默替换为 `{}`，不通知 LLM |
| M-002 | `character.rs:156` | `temporary_with_overrides` 无空名验证 |
| M-003 | `conversation.rs:87` | `active_variant` 无越界验证，腐败数据导致静默失败 |
| M-004 | `campaign.rs:88` | `story_clock` 顶层字段与 variables 数组去同步 |
| M-005 | `lib.rs:1576` | `default_variable_keys()` 硬编码 — 遗漏自定义 variable_schema |
| M-006 | `app-pipeline:1069` | 多个 Subagent regenerate targets 只重跑第一个 |
| M-007 | `runtime.rs:448` | `run_tool_loop_with_layout` 缺少 terminal_tools 检查 |
| M-008 | `campaign_runtime.rs:103` | 大小写敏感名匹配 — LLM 输出不一致产生重复 |
| M-009 | `plugin-bridge.js:78` | `postMessage` 使用 `'*'` 原点 |
| M-010 | `Composer.vue:3` | `mock.js` 进入生产 bundle |
| M-011 | `AgentProfileManager.vue:97` | `Date.now()` 生成 Profile ID 可能碰撞 |
| M-012 | `MetaPanel.vue:17` | ✅ 2026-07-07 已修：`App.vue` 传入最后一条带 provenance 的 assistant 节点，生成溯源入口不再是死代码 |
| M-013 | `CampaignInstancesTab.vue:184` | JSON 变量编辑发送字符串而非解析对象 |
| M-014 | 多个 Tab 组件 | 无 campaignId watch — 切换 Campaign 时数据过期 |
| M-015 | `BaseOverlay.vue:95` | body overflow 多实例冲突 — 一个关闭解锁所有 |
| M-016 | `connection_store.rs:108` | 唯一原始 `.lock().unwrap()` — 毒锁崩溃 |
| M-017 | `archiver.rs:223` | 硬编码 "deepseek-chat" 模型名 |
| M-018 | `embedder.rs:119` | 非数值 JSON 静默转为 0.0 — 损坏 embedding |
| M-019 | `sse.rs:175` | 接收端断开后 SSE 继续处理完整流 |
| M-020 | `lib.rs:287` | 日志写入 `writeln!` 结果被丢弃 |
| M-021 | `lib.rs:244` | LogStore 并发写文件可能交错字节（Windows） |
| M-022 | 多处 | `to_value().unwrap_or(Null)` 12 处 — 序列化失败返回 null 给前端 |
| M-023 | `lib.rs:3153` | Patch 执行后 WorldInfoEntry 反序列化失败条目静默丢弃 |
| M-024 | `crates/app-meta/src/lib.rs` | ✅ 2026-07-07 已修复：`execute_patch` 使用工作副本事务执行，失败时不回写已执行动作 |
| M-025 | `app-meta Cargo.toml` | 死依赖 `infra-plugin-host`（从未 import） |
| M-026 | `BaseOverlay.vue:102` | 缺少 `role="dialog"` / `aria-modal` — 无障碍缺陷 |
| M-027 | `AgentConfigCard.vue:36` | `activeConnName` 是普通函数而非 computed |
| M-028 | `PluginHost.vue:86` | Deep watch on plugin object — 不必要的重建 |
| M-029 | `MetaPanel.vue:250` | Sequential await for patch staleness — 应用 Promise.all |
| M-030 | `App.vue:934` | `activeCampaign.name` 无 optional chaining — 可能 null 崩溃 |

---

## Low（关键条目）

| ID | 文件 | 描述 |
|----|------|------|
| L-001 | `connection_store.rs:133` | `active_id()` 死代码 |
| L-002 | `character_extractor.rs:249` | `match_braces()` wrapper 死代码 |
| L-003 | `tools.rs:487` | `register_editor_tools()` 零调用者 |
| L-004 | `harness-real-llm:301` | `RuntimeCtx` type alias 零引用 |
| L-005 | `infra-util:61` | `recover_*()` 生产代码零调用 — 80+ 处内联替代 |
| L-006 | 多个 Cargo.toml | 19 个未使用 Cargo 依赖 |
| L-007 | `app-pipeline:128` | `WritingContext.recent_messages` 死字段 |
| L-008 | `PipelinePanel.vue` | 整个组件死代码（209 行） |
| L-009 | `App.vue:1053` | `showCharDetail` 永远不设为 true |
| L-010 | `useTheme.js:26` | watcher 累积泄漏 |
| L-011 | `ChatMessage.vue:23` | `rerolling` ref 设置但从未读取 |
| L-012 | 多处 | v-for 使用 index 作 key |
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

1. **C-001**: `infra-vector:340` `"character_id"` → `"owner_character_id"`
2. **M-016**: `connection_store.rs:108` `.unwrap()` → `.unwrap_or_else(|p| p.into_inner())`
3. **H-004**: `character.rs:418` 添加 `#[serde(default)]`
4. **L-002/L-003/L-004**: 删除 3 个死函数/类型
5. **M-010**: `Composer.vue:3` 将 `sampleIntent` 内联为常量
6. **H-008**: `AppSidebar.vue` navItems 改为 computed
7. **L-006**: 移除 19 个未使用 Cargo 依赖

### Medium Refactor（1-4 小时/项）

8. **H-006/H-007**: BaseOverlay/BaseDropdown ESC 监听器泄漏修复
9. **H-001**: Store persist 返回 Result
10. **M-001**: tool-call 参数解析错误反馈给 LLM
11. **M-005**: `default_variable_keys()` 合并实际 schema
12. **M-014**: Tab 组件添加 campaignId watch
13. **H-009**: LLM 重试逻辑（指数退避）
14. **M-024**: ✅ patch 执行已添加事务回滚（2026-07-07）

### Major Restructure（> 4 小时/项）

15. **H-014**: 全量同步 I/O 替换为异步
16. **H-013 剩余项**: CampaignStore 后台 flush / 异步 I/O 评估（集合级锁已完成）
17. **H-002**: API key 加密存储
18. **H-012**: infra-plugin-host 移除直接 Tauri 依赖（已完成 2026-07-06）
19. **R3 Top5**: CampaignRuntimeContext / ToolContext 大对象改为 Arc 共享
20. 测试覆盖：app-pipeline、storage、app-memory 补测试

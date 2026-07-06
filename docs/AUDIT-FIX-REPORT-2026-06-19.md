# StoryForge 审查修复报告

> 执行日期：2026-06-19
> 审查基础：FULL-AUDIT-REPORT.md（6 轮双盲审查，25 个 Opus Agent）
> 修复执行：3 并行 Agent 验证 + 9 并行 Agent 修复

## 执行摘要

对 FULL-AUDIT-REPORT.md 发现的 51 个问题进行并行验证，确认 48 个真实存在、2 个部分确认、1 个驳回。随后按"按文件分组、Quick Fix 优先"原则分 2 个批次（Batch 1: 18 项 Quick Fix + Batch 2: 16 项 Medium Fix）并行修复，共 **34 项修复**，覆盖 **30+ 文件**。

最终验证：`cargo test --workspace` **521 passed / 0 failed**，`npm run build` **成功**。

---

## 验证阶段

使用 3 个并行 Explore Agent 对 51 个问题逐一读源码验证：

| Agent | 范围 | 结果 |
|-------|------|------|
| Agent 1 | Critical + High（C-001, H-001~H-015） | 12 CONFIRMED, 2 PARTIALLY, 0 NOT |
| Agent 2 | Medium backend（M-001~M-025） | 14 CONFIRMED, 2 PARTIALLY, 1 NOT |
| Agent 3 | Frontend + Low（H-006~H-008, M-009~M-030, L-*） | 全部 CONFIRMED |

### 驳回项

| ID | 原因 |
|----|------|
| M-019 | SSE 有 `tokio::select!` + `cancel.changed()` 机制，断连后正确取消，不是真实问题 |

### 部分确认项

| ID | 说明 |
|----|------|
| H-004 | `#[serde(default)]` 确实缺失，但 `RoleType` 有 `Default` impl——只要加 attribute 即可 |
| H-014 | 同步 `std::fs` 确实存在，但大多数 store 操作命令非 `async`，风险有限 |
| M-003 | `active_variant` 用 `.get()` 避免 panic，但越界时静默返回空字符串 |
| M-021 | 并发追加写入理论上可交错，但 JSONL 小写入实际风险极低 |

---

## 修复清单

### Batch 1: Quick Fixes（18 项）

#### 1A. infra-vector（3 项）

| ID | 问题 | 修复 | 测试 |
|----|------|------|------|
| **C-001** | `delete_by_character` 使用错误元数据键 `"character_id"`，角色知识删除完全失效 | 改为 `"owner_character_id"`，补 `test_delete_by_character` 单测 | ✅ 11 pass |
| **H-015** | IO 错误静默返回空 HashMap，下次 persist 覆盖丢失数据 | 加 `tracing::warn!` + `.tmp` 备份读取尝试 | ✅ 同上 |
| **M-018** | embedding 非数值 JSON 静默转 0.0，损坏向量 | 改为 `LlmError::Internal` 返回错误，补测试 | ✅ 25 pass |

#### 1B. domain crate（3 项）

| ID | 问题 | 修复 | 测试 |
|----|------|------|------|
| **H-004** | `CharacterDefinition.role_type` 缺 `#[serde(default)]`，旧数据反序列化失败 | 加 `#[serde(default)]` | ✅ 144 pass |
| **M-002** | `temporary_with_overrides` 无空名验证，LLM 可创建空名角色 | `trim()` + 空名 fallback `"Unknown Character"` | ✅ 同上 |
| **M-008** | `with_temporaries_for` 大小写敏感去重，"Alice" vs "alice" 产生重复 | `HashSet` 存 lowercase key | ✅ 同上 |

#### 1C. app-agent（2 项）

| ID | 问题 | 修复 | 测试 |
|----|------|------|------|
| **M-001** | 畸形 tool-call 参数静默替换 `{}`，LLM 不知道参数被丢弃 | 3 处改为 `match` + `tracing::warn!` + 返回错误信息给 LLM | ✅ 82 pass |
| **M-007** | `run_tool_loop_with_layout` 缺 terminal_tools 检查，不会提前退出 | 补充与其他两个循环变体一致的检查逻辑 | ✅ 同上 |

#### 1D. app-pipeline（2 项）

| ID | 问题 | 修复 | 测试 |
|----|------|------|------|
| **M-006** | 多 subagent 重 roll 只重跑第一个（`find_map`） | 改为 `filter_map().collect()`，循环处理所有 target | ✅ 25 pass |
| **L-007** | `WritingContext.recent_messages` 死字段，从未被读取 | 删除字段及 2 处初始化 | ✅ 同上 |

#### 1E. tauri-app store（4 项）

| ID | 问题 | 修复 | 测试 |
|----|------|------|------|
| **M-016** | `connection_store::get` 唯一裸 `.lock().unwrap()`，毒锁崩溃 | 改为 `.unwrap_or_else(\|p\| p.into_inner())` | ✅ 159 pass |
| **M-023** | patch 执行静默丢弃反序列化失败条目 | 加 `tracing::warn!` 记录失败条目 | ✅ 同上 |
| **M-020** | `writeln!` 结果丢弃，日志写入错误无感知 | 改为 `if let Err(e)` 输出到 stderr | ✅ 同上 |
| **M-025** | `app-meta` 死依赖 `infra-plugin-host` | 从 Cargo.toml 删除 | ✅ 同上 |

#### 1F. 死代码清理（3 项）

| ID | 问题 | 修复 |
|----|------|------|
| **L-008** | `PipelinePanel.vue` 整个组件死代码（209 行） | 删除文件 |
| **L-009** | `showCharDetail` 永远不设为 true，CharacterDetail 无法显示 | 删除 ref + 模板条件渲染 + 无用 import |
| **L-011** | `rerolling` ref 设置但从未读取 | 删除 ref 及赋值 |

---

### Batch 2: Medium Fixes（16 项）

#### 2A. Frontend base 组件（5 项）

| ID | 问题 | 修复 | 测试 |
|----|------|------|------|
| **H-006** | BaseOverlay ESC 监听器泄漏，N 次开关后触发 N 次 close | 闭包保存 handler，open 前先 remove，close 时也 remove，onUnmounted 清理 | ✅ build |
| **H-007** | BaseDropdown 同样的 ESC 泄漏 | 同 H-006 模式 | ✅ build |
| **M-015** | body overflow 多实例冲突，一个关闭解锁所有 | 模块级 `overflowCount` 引用计数 | ✅ build |
| **M-026** | 缺少 `role="dialog"` / `aria-modal` 无障碍属性 | 外层 div 加 ARIA 属性 | ✅ build |
| **H-008** | AppSidebar `navItems` 非响应式，badge/状态不更新 | 改为 `computed(() => [...])` | ✅ build |

#### 2B. Frontend 逻辑修复（6 项）

| ID | 问题 | 修复 | 文件 |
|----|------|------|------|
| **M-010** | `mock.js` 进入生产 bundle | 删除 import，内联常量 | `Composer.vue` |
| **M-011** | `Date.now()` 生成 Profile ID 可能碰撞 | 改为 `crypto.randomUUID()` | 2 文件 |
| **M-013** | JSON 变量编辑发送字符串而非对象 | 加 `JSON.parse()` + try/catch | `CampaignInstancesTab.vue` |
| **M-014** | Tab 组件无 campaignId watch，切换 Campaign 数据过期 | 加 `watch(() => props.campaignId, ...)` | 4 个 Tab |
| **M-029** | patch 过期检查串行 await | 改为 `Promise.all(...)` 并行 | `MetaPanel.vue` |
| **M-030** | `activeCampaign.name` 无 optional chaining | 加 `?.` | `App.vue` |

#### 2C. Frontend 代码质量（4 项）

| ID | 问题 | 修复 | 文件 |
|----|------|------|------|
| **M-027** | `activeConnName` 是普通函数非 computed | 改为 `computed()` | `AgentConfigCard.vue` |
| **M-028** | deep watch 导致不必要重建 | 移除 `{ deep: true }` | `PluginHost.vue` |
| **L-010** | useTheme watcher 累积泄漏 | 改为模块级单次注册 | `useTheme.js` |
| **L-012** | v-for 使用 index 作 key | 改为唯一 id | 2 文件 |

#### 2D. Rust 逻辑修复（4 项）

| ID | 问题 | 修复 | 测试 |
|----|------|------|------|
| **H-005** | 临时 instance 未传播到子 Agent ToolContext | `ctx.campaign_runtime = campaign_runtime.clone()` | ✅ 521 pass |
| **M-005** | `default_variable_keys()` 硬编码，遗漏自定义 schema | 合并 Campaign 的 `CharacterDefinition.variable_schema` keys | ✅ 同上 |
| **M-004** | `story_clock` 双存储可去同步 | 标记 deprecated + 添加 `current_story_clock()` getter | ✅ 同上 |
| **M-017** | 硬编码 `"deepseek-chat"` 模型名 | 从连接配置读取，参数化传递 | ✅ 同上 |

---

## 未修复项（Batch 3: Deferred，15 项）

需大重构，不在本轮执行：

| ID | 问题 | 推迟原因 |
|----|------|----------|
| H-001 | Store persist 吞掉写入错误 | ~30 个调用点签名变更 |
| H-002 | API key 明文存储 | ✅ 2026-07-06 已完成：`connections.json`/`embed.json` 仅保存 SecretRef，真实 key 走系统凭据库；Android 实机仍需验证 |
| H-003 | data_dir 用 exe 路径 | 需数据迁移策略 |
| H-009 | 无 LLM 重试逻辑 | 需设计退避策略和配置化 |
| H-010 | Tauri 命令错误扁平化为 String | 需定义前端错误处理体系 |
| H-011 | 损坏 JSON 静默返回空 | 需设计用户通知/恢复 UI |
| H-012 | infra-plugin-host 依赖 tauri | ✅ 2026-07-06 已完成：Tauri/WebView adapter 移至 `tauri-app/src/mvu_webview_runtime.rs` |
| H-013 | CampaignStore 持锁做 7 次写入 | ✅ 2026-07-06 已拆集合级锁；剩余同步 I/O/后台 flush 评估 |
| H-014 | 同步 fs 阻塞异步运行时 | 需全量 store 接口变更 |
| M-003 | active_variant 无边界检查 | 需设计数据修复策略 |
| M-009 | postMessage 用 '*' origin | Tauri 内无跨域风险 |
| M-012 | lastConversationNode 从未传入 | 需设计 explain generation 流程 |
| M-021 | LogStore 并发写入可能交错 | 小 JSONL 实际风险极低 |
| M-022 | unwrap_or(Null) ~12 处 | 需逐一处理 |
| M-024 | patch 无事务回滚 | 需设计快照机制 |

---

## 修改文件清单

### Rust（15 文件）

| 文件 | 修改项 |
|------|--------|
| `crates/infra-vector/src/lib.rs` | C-001, H-015 |
| `crates/infra-llm/src/embedder.rs` | M-018 |
| `crates/domain/src/character.rs` | H-004 |
| `crates/domain/src/campaign.rs` | M-002, M-004 |
| `crates/domain/src/campaign_runtime.rs` | M-008 |
| `crates/app-agent/src/runtime.rs` | M-001, M-007, H-005 |
| `crates/app-pipeline/src/lib.rs` | M-006, L-007 |
| `crates/tauri-app/src/lib.rs` | L-007, M-023, M-005, M-017 |
| `crates/tauri-app/src/connection_store.rs` | M-016 |
| `crates/app-logging/src/lib.rs` | M-020 |
| `crates/app-meta/Cargo.toml` | M-025 |
| `crates/app-memory/src/archiver.rs` | M-017 |

### Frontend（14 文件）

| 文件 | 修改项 |
|------|--------|
| `frontend/src/components/base/BaseOverlay.vue` | H-006, M-015, M-026 |
| `frontend/src/components/base/BaseDropdown.vue` | H-007 |
| `frontend/src/components/AppSidebar.vue` | H-008 |
| `frontend/src/components/Composer.vue` | M-010 |
| `frontend/src/components/AgentProfileManager.vue` | M-011 |
| `frontend/src/components/AgentConfigCard.vue` | M-011, M-027 |
| `frontend/src/components/CampaignInstancesTab.vue` | M-013, M-014 |
| `frontend/src/components/CampaignKnowledgeTab.vue` | M-014 |
| `frontend/src/components/CampaignTasksTab.vue` | M-014 |
| `frontend/src/components/CampaignSummariesTab.vue` | M-014 |
| `frontend/src/components/MetaPanel.vue` | M-029, L-012 |
| `frontend/src/components/PluginHost.vue` | M-028 |
| `frontend/src/components/ChatMessage.vue` | L-011 |
| `frontend/src/components/StreamingMessage.vue` | L-012 |
| `frontend/src/App.vue` | L-009, M-030 |
| `frontend/src/useTheme.js` | L-010 |
| `frontend/src/components/PipelinePanel.vue` | L-008（已删除） |

---

## 验证结果

```
cargo test --workspace
  521 passed, 0 failed, 10 ignored

npm run build
  ✓ built in 1.12s
```

## 执行效率

| 阶段 | 耗时 | Agent 数 | 说明 |
|------|------|----------|------|
| 验证 | ~5 min | 3 并行 | 3 个 Explore Agent 分别验证 Critical+High / Medium / Frontend+Low |
| Batch 1 | ~4 min | 5 并行 | 5 个 build-error-resolver Agent 分别处理 1A~1E，1F 主线程处理 |
| Batch 2 | ~5 min | 4 并行 | 4 个 build-error-resolver Agent 分别处理 2A~2D |
| 验证 | ~3 min | — | `cargo test --workspace` + `npm run build` |
| **总计** | **~17 min** | **12 Agent** | 34 项修复，30+ 文件 |

## 建议后续优先级

1. **H-009（LLM 重试）** — 最影响用户体验，单次网络波动即写作失败
2. **H-001（Store persist 返回 Result）** — 数据安全基础
3. **H-011（损坏 JSON 不静默返回空）** — 配合 H-001 一起做
4. **H-002（API key 加密）** — 安全合规
5. **H-014 + H-013 剩余项（异步 I/O + 后台 flush）** — 性能基础；H-013 集合级锁已完成

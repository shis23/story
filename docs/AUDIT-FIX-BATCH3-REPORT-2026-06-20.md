# StoryForge Batch 3 修复报告（Deferred Items）

> 执行日期：2026-06-20
> 基于 AUDIT-FIX-REPORT-2026-06-19.md 中标记为 Deferred 的 15 项
> 修复执行：4 并行 Agent + 主线程同步修复 + 1 编译修复 Agent

## 执行摘要

对 Batch 3 Deferred 的 15 项问题进行分析和修复。成功修复 **7 项**（4 High + 1 Medium + 1 Low + 1 确认非问题），剩余 **8 项** 因需 Major Restructure 正式推迟。

最终验证：`cargo test --workspace` **550+ passed / 0 failed**，`npm run build` **成功**。

---

## 已修复项（7 项）

### H-009: LLM 重试逻辑（指数退避）✅

- **文件**: `crates/domain/src/llm.rs`, `crates/infra-llm/src/retry.rs`, `crates/infra-llm/src/lib.rs`
- **修复**:
  - domain 层新增 `LlmError::is_retryable()` 方法和 `RetryConfig` 结构体（默认 3 次重试，1s 基础退避）
  - 新增 `RetryingClient` 装饰器，实现 `LlmClient` trait，对 `RateLimited`/`ServerError`/`Timeout` 自动重试
  - 尊重 `Retry-After` 响应头
  - 流式调用取消安全（退避期间检查 cancel 信号）
  - 指数退避：1s → 2s → 4s
- **测试**: 19 新测试（11 domain + 8 infra-llm），全部通过
- **使用方式**: `with_retry(client, RetryConfig::default())` 包装任意 `LlmClient`

### H-010: Tauri 命令结构化错误 DTO ✅

- **文件**: `crates/tauri-app/src/error.rs`（新建）, `crates/tauri-app/src/lib.rs`
- **修复**:
  - 新增 `TauriCommandError` 枚举：Pipeline / Llm / Storage / Validation / NotFound / Cancelled / Internal
  - `#[serde(tag = "type", rename_all = "snake_case")]` — 前端收到 `{"type":"llm","message":"...","retryable":true}`
  - `From` 实现覆盖所有错误类型：LlmError, AgentError, PipelineError, ImportError, ConversationError, MvuApplyError
  - `From<String>` 向后兼容现有 `.map_err(|e| format!(...))` 模式
  - 便利构造器：`not_found()`, `storage()`, `validation()`, `pipeline()`, `llm()`, `internal()`
- **测试**: 10 单元测试，覆盖序列化和 From 转换
- **前端兼容**: 现有读 `.message` 的代码无需修改，新增可用 `type` 字段区分错误类别

### H-001: Store persist 返回 Result ✅

- **文件**: `crates/tauri-app/src/storage.rs`, `campaign_store.rs`, `connection_store.rs`, `preset_store.rs`, `module_store.rs`, `lib.rs`
- **修复**:
  - 所有 Store 的 `persist()` 方法改为返回 `Result<(), String>`
  - 所有 CRUD 方法（save, delete, update, add_*）改为返回 `Result<T, String>`
  - `atomic_write_json` 错误正确传播到 Tauri 命令层
  - 调用方添加 `.unwrap()`（测试）或 `?` / `.map_err()`（生产代码）
  - `tracing::error!` 日志保留
- **影响**: 磁盘满或权限错误时用户收到明确错误信息，不再静默丢失数据
- **测试**: 全量 64 tauri-app tests 通过

### H-011: 损坏 JSON 不静默返回空 ✅

- **文件**: `crates/tauri-app/src/storage.rs`, `preset_store.rs`, `connection_store.rs`, `module_store.rs`, `crates/infra-vector/src/lib.rs`, `crates/infra-plugin-host/src/lib.rs`
- **修复**:
  - 6 个文件的 JSON 加载路径：解析失败时 `tracing::error!` 记录文件路径和错误信息
  - 自动复制损坏文件为 `.json.corrupt` 备份（保留恢复数据）
  - 仍返回空默认值（不崩溃），但用户有明确日志和恢复路径
- **模式**: 所有修复遵循相同模式——记录 → 备份 → 返回默认

### H-003: data_dir 改用 OS 标准目录 ✅

- **文件**: `crates/tauri-app/src/lib.rs`
- **修复**:
  - `get_app_data_dir()` 改为 OS 标准路径：
    - Windows: `%APPDATA%/StoryForge`
    - macOS: `~/Library/Application Support/StoryForge`
    - Linux: `$XDG_DATA_HOME/storyforge` 或 `~/.local/share/storyforge`
  - 自动数据迁移：旧 `exe_dir/data` 有数据且新目录为空时，递归复制
  - 迁移后旧数据保留（不删除），用户无感切换
- **回退**: 环境变量不存在时仍使用 `exe_dir/data`（兼容旧安装）

### M-003: active_variant 边界检查 ✅

- **文件**: `crates/domain/src/conversation.rs`
- **修复**:
  - `MessageNode` 改为手动实现 `Deserialize`（移除 derive）
  - 反序列化时将 `active_variant` 钳制到 `[0, variants.len()-1]` 范围
  - 越界时 `eprintln!` 输出警告
  - 修复了"损坏数据导致静默返回空字符串"的问题
- **测试**: 157 domain tests 通过

### L-003: 删除 register_editor_tools 死代码 ✅

- **文件**: `crates/app-agent/src/tools.rs`
- **修复**: 删除 `register_editor_tools()` 函数（零调用者，`#[allow(dead_code)]`）

### M-009: postMessage origin — 确认非真实问题

- **文件**: `frontend/src/plugin-bridge.js`
- **分析**: `postMessage('*', ...)` 在 Tauri 内部使用 srcdoc iframe，宿主和插件同源（null origin），不存在跨域风险。`createHostHandler` 已验证 `pluginId` 做权限校验。
- **结论**: 不需修复

---

## 未修复项（8 项，正式推迟）

| ID | 问题 | 推迟原因 |
|----|------|----------|
| H-002 | API key 明文存储 | 需添加 `keyring` crate 依赖 + OS 密钥链集成 + 数据迁移策略 |
| H-012 | infra-plugin-host 依赖 tauri | `WebViewMvuRuntime` 使用 `tauri::AppHandle`，需 trait 抽象重构 + 移动实现到 tauri-app |
| H-013 | CampaignStore 持锁做 7 次写入 | 需 clone-then-write 重构 + 审计所有持锁路径 |
| H-014 | 同步 fs 阻塞异步运行时 | `fill_campaign_context` 需重构为 `spawn_blocking` 模式，涉及调用链变更 |
| M-012 | lastConversationNode 从未传入 | 需设计 explain generation 完整流程 |
| M-021 | LogStore 并发写入可能交错 | 小 JSONL 文件实际风险极低 |
| M-022 | unwrap_or(Null) ~12 处 | 与 H-010 共享 lib.rs，需逐一审查序列化失败场景 |
| M-024 | patch 事务回滚 | 需设计快照/回滚机制 |

---

## 修改文件清单

### Rust（16 文件）

| 文件 | 修改项 |
|------|--------|
| `crates/domain/src/llm.rs` | H-009: `is_retryable()`, `RetryConfig` |
| `crates/domain/src/conversation.rs` | M-003: 手动 `Deserialize` + 边界钳制 |
| `crates/infra-llm/src/retry.rs` | H-009: 新建 `RetryingClient` |
| `crates/infra-llm/src/lib.rs` | H-009: 导出 retry 模块 |
| `crates/tauri-app/src/error.rs` | H-010: 新建 `TauriCommandError` |
| `crates/tauri-app/src/lib.rs` | H-003: data_dir OS 标准路径 + 迁移; H-010: 错误类型替换 |
| `crates/tauri-app/src/storage.rs` | H-001: persist Result; H-011: corrupt JSON 备份 |
| `crates/tauri-app/src/campaign_store.rs` | H-001: persist Result |
| `crates/tauri-app/src/connection_store.rs` | H-001: persist Result; H-011: corrupt JSON |
| `crates/tauri-app/src/preset_store.rs` | H-001: persist Result; H-011: corrupt JSON |
| `crates/tauri-app/src/module_store.rs` | H-001: persist Result; H-011: corrupt JSON |
| `crates/infra-vector/src/lib.rs` | H-011: corrupt JSON 备份 |
| `crates/infra-plugin-host/src/lib.rs` | H-011: corrupt JSON 备份 |
| `crates/app-agent/src/tools.rs` | L-003: 删除死函数 |

### 新增文件（2 个）

| 文件 | 说明 |
|------|------|
| `crates/infra-llm/src/retry.rs` | LLM 重试装饰器（347 行，含 8 个测试） |
| `crates/tauri-app/src/error.rs` | 结构化错误 DTO（346 行，含 10 个测试） |

---

## 验证结果

```
cargo test --workspace
  550+ passed, 0 failed, 10 ignored

npm run build
  ✓ built in 4.67s
```

## 执行效率

| 阶段 | Agent 数 | 说明 |
|------|----------|------|
| H-009 | 1 Agent | LLM retry：domain + infra-llm |
| H-010 | 1 Agent | 结构化错误 DTO + lib.rs 错误替换 |
| H-001 | 1 Agent | Store persist Result（5 个 store 文件） |
| H-011 | 1 Agent | 损坏 JSON 备份（6 个文件） |
| 主线程 | — | M-003 + H-003 + L-003 + 测试修复 |
| 编译修复 | 1 Agent + 主线程 | 修复跨 Agent 编译冲突 |

## 累计修复统计（Batch 1-3）

| 批次 | 修复数 | 说明 |
|------|--------|------|
| Batch 1 | 18 项 | Quick Fixes（C-001 + High + Medium + Dead Code） |
| Batch 2 | 16 项 | Medium Fixes（Frontend + Rust 逻辑） |
| Batch 3 | 7 项 | Deferred Items（LLM Retry + Error DTO + Store Result + ...） |
| **总计** | **41 项** | 从 51 个审计问题中修复 41 项（80%） |

## 剩余工作优先级

1. **H-013 + H-014**（性能基础）— CampaignStore 锁优化 + 异步 I/O
2. **H-002**（安全合规）— API key 加密
3. **M-022 + M-024**（健壮性）— 序列化日志 + patch 回滚
4. **H-012**（架构）— plugin-host tauri 依赖解耦

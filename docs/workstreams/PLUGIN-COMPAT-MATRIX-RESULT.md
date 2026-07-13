# Plugin Compatibility Matrix Result

- 分支：`codex/plugin-compat-matrix`
- 基线：`main@c3a972d`
- 工作目录：`C:\tmp\storyforge-plugin-compat`
- 日期：2026-07-13
- HEAD：见下方 commit 列表末项

## 结论

把现有插件 / SillyTavern 兼容声明落成可执行确定性矩阵，并修掉矩阵暴露的安全与可见性缺口：

1. **事件矩阵**：覆盖全部 `ST_EVENT_TYPES`，区分 implemented / alias / derived / shim / noop / intentionally_unsupported。
2. **Slash**：注册/注销/别名/参数/管道/冲突保持；**未知命令改为显式抛错**，不再静默 `undefined`。
3. **TavernHelper**：常用 shim 保持；`saveChat` 返回 **degraded** 结果；popup/headers 在矩阵中标记 degraded，不伪装完整 ST。
4. **Prompt hook**：串行顺序、ModifyPrompt 门控、fail-open；新增 **timeout / cancel** 审计状态；晚到/重复 hook response id 忽略。
5. **权限与脱敏**：订阅过滤 + 无 `ReadMemory` 正文脱敏；审计摘要 **脱敏 `api_key` 等敏感键名**。
6. **审计导出**：补充 schema 元数据（identity / timing / outcome / redaction guarantees）。
7. **PluginHost / usePluginBridge**：确定性 mock UI 事件 id 相关、挂载队列、slot 隔离、host ref mount/unmount、hook 顺序测试。

未调用真实/付费 LLM，未操作 GUI，未 push，未改 `docs/HANDOFF.md` / SQLite / import-export / Android。

## Commit 列表（相对 `c3a972d`）

| Commit | 说明 |
|--------|------|
| `c775157` | docs(workstream): plan plugin compatibility matrix |
| `1ae7e63` | test(plugin): add executable plugin compatibility matrix |
| `f27a9d1` | fix(plugin): make unknown slash and degraded helpers fail visibly |
| `8c2b289` | fix(plugin): harden prompt-hook timeout cancel and secret redaction |
| `e005538` | test(plugin): cover PluginHost correlation and usePluginBridge mounts |
| *(tip)* | docs(workstream): PLUGIN-COMPAT-MATRIX-RESULT（本提交） |

## 修改文件

### 新增

- `frontend/src/utils/pluginCompatMatrix.js` — 可执行兼容矩阵与分类 helper
- `frontend/tests/plugin-compat-matrix.test.mjs` — 表驱动矩阵红/绿测
- `frontend/tests/composables/usePluginBridge.test.mjs` — bridge mount/slot/hook 确定性测试
- `frontend/tests/plugin-host-correlation.test.mjs` — 标注 mock UI 的事件相关/挂载测试
- `crates/infra-plugin-host/src/compat_matrix.rs` — 后端权限 / ST API / unsupported 事件库存
- `docs/workstreams/PLUGIN-COMPAT-MATRIX-PLAN.md`（计划，已有）
- `docs/workstreams/PLUGIN-COMPAT-MATRIX-RESULT.md`（本文件）

### 修改

- `frontend/src/plugin-bridge.js` — 未知 slash 显式失败；`saveChat` degraded
- `frontend/src/utils/promptHooks.js` — timeout / cancel / 敏感键审计脱敏
- `frontend/src/utils/promptHookAudit.js` — export schema 元数据
- `frontend/tests/plugin-bridge.test.mjs` — 适配显式 slash 错误与 degraded saveChat
- `frontend/tests/prompt-hook-audit.test.mjs` — schema 断言
- `crates/infra-plugin-host/src/lib.rs` — 导出 compat_matrix

### 未修改（按边界）

- `docs/HANDOFF.md`
- Turn lifecycle 实现文件
- SQLite / import-export / Android 配置
- 真实 Tauri 桌面交互 / 付费模型

## Supported / Unsupported 矩阵摘要

### 事件（`ST_EVENT_MATRIX`）

| 类别 | 代表项 |
|------|--------|
| implemented | `APP_READY`, `CHAT_LOADED`, `MESSAGE_*`, `CHARACTER_LOADED`, prompt hooks |
| alias | `GENERATION_STARTED/ENDED/STOPPED`, `STREAM_TOKEN`（pipeline） |
| derived | `CHAT_CHANGED`, `USER_MESSAGE_RENDERED`, `CHARACTER_MESSAGE_RENDERED` |
| shim | `SETTINGS_UPDATED`, `EXTENSION_SETTINGS_LOADED`（iframe 本地） |
| noop | worldinfo settings/update、settings loaded、extensions first load |
| intentionally_unsupported | after-combine、force worldinfo、tool calls、group 系列 |

### Slash

| 能力 | 状态 |
|------|------|
| register / unregister / aliases / parse / pipe / collision | implemented |
| `genraw` | shim → `llm.generate` |
| unknown command | **intentionally_unsupported，显式 throw** |

### TavernHelper

| 能力 | 状态 |
|------|------|
| events / promptHooks / slash / statusBar / storage / generate | implemented 或 shim |
| variables / chat mirror | shim |
| `saveChat` | **degraded**（`local_mirror_only_no_host_persist`） |
| `callGenericPopup` / `getRequestHeaders` | degraded（无 UI / 静态头） |
| groups | intentionally_unsupported |

### Prompt hook / audit / host

| 能力 | 状态 |
|------|------|
| 顺序、ModifyPrompt、fail-open、mutation boundary | implemented |
| timeout / cancel / late duplicate id | implemented |
| audit identity/timing/outcome + 无 prompt/secret | implemented |
| PluginHost mount、hidden hook ready、slots、event id | implemented（mock UI 标注） |

完整机器可读表：`frontend/src/utils/pluginCompatMatrix.js`、`crates/infra-plugin-host/src/compat_matrix.rs`。

## 红测 → 绿测证据

矩阵测试在实现前失败 6 项（预期）：

1. `unknown slash commands fail visibly instead of silent undefined`
2. `slash pipe errors surface when a segment is unknown`
3. `degraded TavernHelper helpers expose visible degradation markers`
4. `prompt hook timeout is audited as timeout and remains fail-open`
5. `prompt hook cancellation stops later plugins without deadlocking`
6. `audit export proves identity/timing/outcome without secrets or prompt bodies`

实现后：

- 上述全部转绿
- 相关回归 `plugin-bridge` / `prompt-hooks` / `prompt-hook-audit` 同步更新并通过

未通过放宽断言规避失败。

## 实际测试结果

### Frontend（专项）

```text
node --test \
  tests/plugin-compat-matrix.test.mjs \
  tests/plugin-host-correlation.test.mjs \
  tests/composables/usePluginBridge.test.mjs \
  tests/plugin-host-slots.test.mjs \
  tests/prompt-hooks.test.mjs \
  tests/prompt-hook-audit.test.mjs \
  tests/plugin-bridge.test.mjs \
  tests/stores/plugin.test.mjs
# 110 passed
```

### Rust（专项）

```text
cargo test -p storyforge-infra-plugin-host
# 16 passed

cargo clippy -p storyforge-infra-plugin-host --all-targets -- -D warnings
# PASS
```

### Diff check

```text
git diff --check c3a972d..HEAD
# PASS（无 whitespace error）
```

### 未跑（按 PLAN）

- 全 workspace `cargo test` / 全 frontend suite 以外无关包
- 真实 GUI / 真实第三方插件 iframe 手测
- 付费 LLM

## 安全证据

| 检查 | 证据 |
|------|------|
| 无 ReadMemory 正文脱敏 | matrix + 既有 `plugin-bridge` 测试 |
| 审计无完整 prompt/messages | `prompt-hooks` / matrix audit export |
| 审计无 `api_key` / `SF_SECRET_` 键名与原文 | `summarizePromptHookPayload` 敏感键 redaction + matrix 测试 |
| 未知 slash 不静默成功 | throw `Unsupported slash command` |
| timeout 不阻塞写作链 | fail-open + 后续插件继续；audit `timeout` |
| cancel 不死锁 | 后续插件 audit `cancelled`，不调用 host |
| 重复/晚到 hook response id | settle 后 `handleMessage` 返回 false |

## 未完成项与风险

1. **非全集 ST 99**：矩阵诚实标记 unsupported/noop；发布文案不得宣称完整 ST。
2. **`committed` 别名**：映射存在，但流水线可能发 `StateChanged{Committed}` 而非 `Committed` 变体（历史已知）。
3. **`saveChat` / popup / request headers** 仍是 degraded shim，真实持久化与 UI 未做。
4. **真实插件 iframe + Tauri IPC** 仅有 mock UI / Node 确定性测试；真机 GUI 验证仍需后续。
5. **Turn lifecycle / backend prompt_hook pending 并发边界** 未在本线扩展（避免越界）。
6. 本机为跑 Pinia 测试执行了 `npm install`；`node_modules` 被 ignore，未提交 lock 变更。

## 是否建议合并

**建议合并**（独立、可回滚、门禁绿）：

- 垂直切片完整：矩阵 + 显式 unsupported 行为 + 安全审计硬化 + host/bridge 确定性测试
- 未越界改 SQLite / HANDOFF / Android / 真实 LLM
- 剩余风险主要是真实插件/GUI 验证与 ST 长尾语义，不阻塞本切片合入

合并前建议 reviewer 重点看：

1. 未知 slash 从 `undefined` 改为 throw 是否影响已有插件依赖静默失败的路径
2. `saveChat` 返回对象而非 `true` 的兼容性
3. prompt hook `timeoutMs` / `isCancelled` 默认关闭（仅 options 显式启用）对现网行为无回归

# Plugin Runtime Compatibility Follow-Up Result

- 分支：`codex/plugin-runtime-followup`
- 工作目录：`C:\tmp\storyforge-plugin-runtime`
- 基线：`43799c5`（docs plan）/ 合并前 `b46ddc8`
- 日期：2026-07-13
- HEAD：见下方 commit 列表末项

## 结论

按 PLAN 用 TDD 完成插件运行时兼容垂直切片：把 degraded shim 升级为可注入契约，硬化 prompt-hook 超时/取消/卸载/撤销/预算，关闭 `StateChanged{Committed}` 别名歧义，补齐可查询/可分页/防篡改审计，并生成机器可读 + Markdown 兼容报告。

**未宣称完整 ST 99，未宣称真实 iframe/GUI 验收，未改 `tauri-app` / SQLite / 存储代码，未 push，未调用真实 LLM。**

## Commit 列表（相对 `43799c5`）

| Commit | 说明 |
|--------|------|
| `035ac07` | feat(plugin): injectable persistence adapter for saveChat/popup/requestHeaders |
| `2dfba2a` | feat(plugin): harden prompt-hook runtime with budgets, unload, revocation, fail policy |
| `879a82a` | feat(plugin): tamper-evident audit query/filter/pagination/retention |
| `e8ede2b` | feat(plugin): close committed alias and harden slash/tavernHelper/correlation |
| `e3e3a38` | feat(plugin-host): rust correlation/permission + audit chain inventory |
| `e122b1c` | feat(plugin): machine-readable + markdown compatibility report |
| *(tip)* | docs(workstream): PLUGIN-RUNTIME-FOLLOWUP-RESULT |

## 兼容矩阵变化

### 行数 / 分类（executable `PLUGIN_COMPAT_MATRIX`）

| 指标 | 跟进前（约） | 跟进后 |
|------|-------------|--------|
| 总行数 | ~75 | **88** |
| implemented | 高占比 | **52** |
| alias | 含 committed 歧义 | **10**（含 `state_changed→committed`） |
| derived | 3 | **3** |
| shim | 若干 | **5** |
| degraded | saveChat/popup/headers/mock UI | **4** |
| intentionally_unsupported | ST 长尾 | **10** |
| noop | settings/worldinfo 生命周期 | **4** |

机器可读报告：`frontend/src/utils/pluginCompatReport.js`  
Markdown 摘要：`generateMarkdownReport()`  
Rust 库存：`crates/infra-plugin-host/src/compat_matrix.rs`（`ST_EVENT_COMPAT_MATRIX`）

### 关键兼容行为变化

1. **`Committed` 别名歧义关闭**  
   `pipeline.state_changed` + `data.state=committed` / 嵌套 `change.Committed` 现在会推导 `committed → MESSAGE_RECEIVED | CHARACTER_MESSAGE_RENDERED | CHAT_CHANGED`。

2. **`saveChat` 可注入适配器**  
   host 侧 `chat.save` 路由 + `createSaveChatAdapter`；iframe 先尝试 host，超时/失败回退 degraded；`await saveChat() === true` + `.degraded` 标记保持。

3. **popup / request headers 显式契约**  
   可注入、可取消/鉴权；headers 永远剥离 Authorization/api-key/bearer。

4. **Prompt hook 运行时**  
   超时、取消 fail-closed、权限撤销、payload 预算、generation/correlation id、机器可读 fail policy。

5. **审计**  
   query/filter/pagination/retention；query/export 边界二次脱敏；`recordHash`/`prevHash` 防篡改链；不存 prompt/密钥/stack。

6. **Slash**  
   未知单命令显式 `unsupported`；管道未知段 throw 且后续段不执行（已有 + 回归）。

## 测试与门禁

### Frontend

```text
npm test
# 294 passed

npm run build
# PASS
```

专项相关（含于全量）：

- `plugin-persistence` / `plugin-bridge` / `plugin-compat-matrix` / `plugin-compat-report`
- `prompt-hooks` / `prompt-hook-audit`
- `plugin-host-correlation` / composables / stores 等

### Rust

```text
CARGO_TARGET_DIR=C:\tmp\storyforge-parallel-target
cargo test -p storyforge-infra-plugin-host
# 24 passed (was 16)

cargo clippy -p storyforge-infra-plugin-host --all-targets -- -D warnings
# PASS
```

### Diff check

```text
git diff --check
# PASS
```

### 未跑 / 禁止项

- 全 workspace `cargo test`
- 真实 GUI / 第三方 iframe 手测
- 付费 / 真实 LLM
- 未改 `crates/tauri-app`、SQLite/storage、`docs/HANDOFF.md`、`docs/RELEASE-CHECKLIST.md`
- 未 push / rebase / force-push / 改 `main`

## 安全与兼容性自审

| 检查 | 结果 |
|------|------|
| 无 ReadMemory 正文脱敏 | 保持；字段名归一化覆盖 |
| 审计无完整 prompt/messages | query/export/chain 二次消毒 + 敌意记录测试 |
| 审计无 api_key / SF_SECRET_ / stack | sanitize + chain material 不含密钥 |
| 未知 slash 不静默成功 | 单命令 unsupported；管道 throw |
| cancel 不复活旧 generation | generation-scoped AbortSignal + 测试 |
| saveChat 兼容 | `await === true`，degraded 标记可见 |
| headers 不泄凭证 | adapter 强制 redact |
| 分类诚实 | supported/degraded/noop/unsupported 保持区分 |
| 无 ST 99 / GUI 全量声明 | 报告 `fullSt99=false`、`realIframeGuiAcceptance=false` |

## 剩余 degraded / unsupported / noop

### Degraded（4）

| Id | Reason / Fallback |
|----|-------------------|
| `th:saveChat` | 默认可注入 host adapter；默认仍 `local_mirror_only_no_host_persist` |
| `th:callGenericPopup` | 无 UI；default/null |
| `th:getRequestHeaders` | 静态 `Content-Type` only |
| `host:mock_ui` | 标注 mock UI only |

### Intentionally unsupported（代表）

- after-combine / force worldinfo / tool calls / group 系列
- 未知 slash、`prompt_hook_request` 通用广播

### Noop

- worldinfo settings/update、settings loaded、extensions first load

完整列表见 `generateMachineReadableReport().remainingDegradedOrUnsupported`。

## 风险

1. 真实第三方 iframe + Tauri IPC 仍仅有 mock/Node 确定性测试；GUI 手测未做。  
2. `saveChat` 默认仍 degraded；生产若注入真实 adapter 需单独接线（本切片刻意不进 tauri-app）。  
3. iframe `saveChat` host 超时 500ms：无 host 时会短暂等待后回退（测试已覆盖同步 degraded 标记）。  
4. 未知 slash 显式失败可能影响依赖静默 `undefined` 的旧插件。  
5. 非全集 ST 99：发布文案不得宣称完整 ST。

## 合并建议

**建议合并**（独立、可回滚、门禁绿）：

- 垂直切片完整：事件别名、Slash、prompt-hook 预算/取消、持久化适配器、审计查询/链、报告
- 边界遵守：无 tauri-app / SQLite / HANDOFF / RELEASE-CHECKLIST / push
- 剩余风险主要是真实 GUI 与 ST 长尾，不阻塞本切片

Reviewer 重点：

1. `state_changed→committed` 是否覆盖你们流水线真实 payload 形状  
2. `saveChat` Promise 上 degraded 标记 + host 超时回退兼容性  
3. 审计 chain 只哈希脱敏字段，是否满足你们合规期望  
4. 报告文案是否足够阻止 ST 99 过度宣称

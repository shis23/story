# 后端架构拆分与 SQLite 收口：Gate 1 Meta Agent 子批结果（2026-07-28）

> 状态：**PASS（Gate 1 Meta Agent 子批）**。本结果记录 Gate 0 保护网与 Gate 1 前三批、世界书、变量、MVU runtime 和 Meta Agent 机械拆分，不代表巨石拆分或 SQLite 彻底迁移已经完成。
> 代码基线：`main@ae5b720`。
> 计划：`docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-PLAN-2026-07-28.md`。

## 1. Gate 0 产物

- 可重复基线脚本：`scripts/architecture/backend-baseline.mjs`。
- 前端 IPC → Tauri 注册合同测试：`frontend/tests/tauri-command-contract.test.mjs`。
- 计划文档中的 9 个 Gate 已建立，后续阶段不得跳过 Gate 1–3 直接堆 SQLite 分支。

## 2. 当前基线

基线由脚本从源码实时计算，当前结果为：

| 指标 | 当前值 |
| --- | ---: |
| workspace crate | 16 |
| `lib.rs` 行数 | 19,261（脚本按源码换行计数） |
| `#[tauri::command]` 属性（`src/**/*.rs`） | 175 |
| `generate_handler!` 注册命令（去重） | 175 |
| `frontend/src/tauri-api.js` unique invoke | 162 |
| 前端调用但后端未注册 | 0 |
| `is_sqlite_active()` 引用（`src/**/*.rs`） | 68 |

当前 `lib.rs` 的注册命令中存在若干只由内部/插件/运行时使用、未被
`tauri-api.js` 直接调用的命令；因此前端 162 与后端注册 175 不要求相等，合同要求是
“前端调用集合必须是后端注册集合的子集”。

## 3. 确定性验证

### 3.1 Gate 0 合同测试

命令：

```powershell
Set-Location frontend
npm.cmd test -- tests/tauri-command-contract.test.mjs
```

结果：**PASS（3/3）**。测试固定了命令属性数、注册命令数、前端调用数、workspace
crate 数和 SQLite 分支计数，并检查前端调用不存在后端漏注册。

### 3.2 基线生成

```powershell
node scripts/architecture/backend-baseline.mjs
```

输出包含：

- 后端命令清单及重复注册；
- 前端 invoke 清单及漏注册；
- workspace crate 数；
- `lib.rs` 行数和 command 属性数；
- SQLite 分支计数；
- 当前 Meta/SQLite unsupported 代码位置。

### 3.3 已执行的静态检查

- `git diff --check`：通过。
- `node --test tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `cargo test -p storyforge --lib scope_validation_errors_do_not_mark_turn_failed -- --nocapture`：1 passed / 0 failed。
- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1`：在提升权限环境通过 7/7；受限沙箱运行时仅因 esbuild 读取项目根目录被拒而中止，不属于代码测试失败。
- 为保证门禁的确定性，`BackendTurnAttemptSink` 增加临时 `TurnStore` 注入，仅测试使用；生产路径仍使用进程级后端存储。

## 4. 当前能力矩阵快照

Gate 0 确认以下项目必须继续处理：

- JSON/SQLite Accept 业务决策仍需进一步共用状态机；
- `is_sqlite_active()` 仍散落在 Tauri 命令层；
- SQLite Meta typed patch / MVU schema apply 仍有 fail-closed 路径；
- SQLite Chronicle compressor worker 仍有 JSON-only 跳过路径；
- 默认后端仍是 JSON；
- Native12/TextFallback/Full100 真实 SQLite 证据仍未封存为 PASS；
- Android APK、真机和 Windows runner/签名现场证据仍是后续发布闸门。

## 5. Gate 0 / Gate 1 当前结论

Gate 0 的保护网代码、测试隔离、合同测试和完整确定性门禁均已通过。Gate 1 前三批已完成：
`presets`、`connections`、`diagnostics`、`import/export`、`cards`、`characters`、`plugins`、`card-shell` 已移入 `commands/`，根模块仅保留导入、注册和跨域接线。
下一批为 typed patch；其与 Campaign runtime / MVU schema apply 耦合度较高，需单独建立更细的编译/测试检查点。

## 6. Gate 1 第一批结果

- 新增 `crates/tauri-app/src/commands/{presets,connections,diagnostics}.rs` 与模块入口。
- 保持前端 IPC 名称、Tauri 注册顺序和生成器行为不变。
- `lib.rs` 从 23,781 行降至 22,610 行；未进行跨批业务重构。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。

## 7. Gate 1 第二批结果

- 新增 `crates/tauri-app/src/commands/{characters,cards,import_export}.rs`。
- 保持 Tauri command 名称、参数、返回 DTO、注册顺序和前端 IPC 合同不变。
- `lib.rs` 从 22,610 行降至 20,922 行；本批只做机械移动与最小可见性调整。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 本批提交：`c4ffd40 refactor(tauri): split character card and import commands`。

## 8. Gate 1 第三批结果

- 新增 `crates/tauri-app/src/commands/{plugins,card_shell}.rs`。
- 保持插件权限校验、Card Shell allowlist/cache/protocol 和 Tauri command 合同不变。
- `lib.rs` 从 20,922 行降至 20,496 行；本批只做机械移动与最小可见性调整。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 本批提交：`d2df672 refactor(tauri): split plugin and card shell commands`。

## 9. Gate 1 世界书/变量子批结果

- 新增 `crates/tauri-app/src/commands/{world_info,variables}.rs`。
- 保持世界书路由、allowlist 语义、Campaign/角色变量读写和活动 Turn barrier 不变。
- `lib.rs` 从 20,496 行降至 19,776 行；本批只做机械移动与最小可见性调整。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 子批提交：`48b6645 refactor(tauri): split world info commands`、`5244de0 refactor(tauri): split campaign variable commands`。

## 10. Gate 1 MVU runtime 子批结果

- 新增 `crates/tauri-app/src/commands/mvu.rs`，移动 W8 runtime ack/execute 命令。
- 保持 pending request 生命周期、错误回传和前端 IPC 合同不变。
- `lib.rs` 从 19,776 行降至 19,732 行。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 子批提交：`852c7e4 refactor(tauri): split mvu runtime commands`。

## 11. Gate 1 Meta Agent 子批结果

- 新增 `crates/tauri-app/src/commands/meta.rs`，移动 legacy Meta patch、Meta 会话、Campaign 健康检查和生成溯源解释命令。
- 保持 Meta stream event、权限/Turn barrier、SQLite fail-closed 语义和前端 IPC 合同不变。
- `lib.rs` 从 19,732 行降至 19,261 行。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 子批提交：`ae5b720 refactor(tauri): split meta agent commands`。

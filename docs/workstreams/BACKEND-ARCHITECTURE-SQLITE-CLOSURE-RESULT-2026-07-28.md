# 后端架构拆分与 SQLite 收口：Gate 1 子批结果（2026-07-28）

> 状态：**PASS（Gate 1 已完成子批与返修）**。本结果记录 Gate 0 保护网及已完成的命令拆分子批；Gate 1 总体、backend facade 和 SQLite 彻底迁移仍未完成。
> 当前代码基线：`main@0485d98`。
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
| `lib.rs` 行数 | 16,365（脚本按源码换行计数） |
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
下一批为写作主链；它与 PipelineOrchestrator、后处理、取消和测试夹具耦合度最高，继续按入口/生命周期拆分并保持独立回滚点。

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

## 12. Gate 1 typed patch/MVU 子批结果

- 新增 `crates/tauri-app/src/commands/meta_typed.rs`，移动 typed patch、MVU 五合一分析/查询/应用和 ST 预设分类命令。
- 保持 typed patch stale/accept/turn barrier、MVU SQLite fail-closed 和 schema backfill 语义不变。
- `lib.rs` 从 19,261 行降至 18,186 行。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 子批提交：`388891e refactor(tauri): split typed patch and mvu analysis commands`。

## 13. Gate 1 Campaign 子批结果

- 新增 `crates/tauri-app/src/commands/campaigns.rs`，移动角色识别、Campaign、角色实例和开档/分支命令。
- 保持角色卡识别降级、开档世界书播种、Campaign 对话绑定、实例校验和前端 IPC 合同不变。
- `lib.rs` 从 18,186 行降至 17,373 行。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 子批提交：`fbb93b6 refactor(tauri): split campaign and instance commands`。

## 14. Gate 1 P2 记忆子批结果

- 新增 `crates/tauri-app/src/commands/memory.rs`，移动角色知识、任务和 RoundSummary 命令。
- 保持知识可见性/传播语义、任务状态转换、RoundSummary DTO 和前端 IPC 合同不变。
- `lib.rs` 从 17,373 行降至 17,013 行。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 子批提交：`15ae614 refactor(tauri): split memory and task commands`。

## 15. Gate 1 Turn 子批结果

- 新增 `crates/tauri-app/src/commands/turns.rs`，移动变体编辑/采纳/丢弃、Turn commit、Chronicle compressor worker、分支和切换命令。
- 保持 Accept 幂等、revision/turn barrier、Attempt 状态、Chronicle 压缩恢复和前端 IPC 合同不变。
- `lib.rs` 从 17,013 行降至 16,365 行。
- `cargo fmt --all`：通过。
- `cargo check -p storyforge --all-targets`：通过。
- `cargo clippy -p storyforge --all-targets -- -D warnings`：通过。
- `cargo test -p storyforge --lib`：344 passed / 3 ignored。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：3 passed / 0 failed。
- `node scripts/architecture/backend-baseline.mjs`：command attributes 175、registered 175、frontend missing 0、SQLite active references 68。
- 子批提交：`168ce32 refactor(tauri): split turn operation commands`。
## 16. Gate 1 Writing 子批结果

- 提交：`79a5b56 refactor(tauri): split writing commands`。
- 新增 `crates/tauri-app/src/commands/writing.rs`，迁移 start_writing、prompt hook、自动修复、后处理归一化、BackendTurnAttemptSink 与 cancel_writing；根 `lib.rs` 仅保留跨域后处理接线和注册。
- `lib.rs` 当前 14,721 行；`#[tauri::command]` 175，注册 175，前端唯一 invoke 162，缺失后端命令 0，SQLite active flag references 68。
- 验证：`cargo fmt --all`、`cargo check -p storyforge --all-targets`、`cargo clippy -p storyforge --all-targets -- -D warnings`、`cargo test -p storyforge --lib`（344 passed, 3 ignored）、`node --test frontend/tests/tauri-command-contract.test.mjs`，均通过。
- 结论：Writing 子批 PASS；Gate 1 总体仍未收口，bootstrap/AppState/注册与 inline tests 仍在 `lib.rs`，Gate 2 状态机、Gate 3 facade、Gate 4–8 SQLite/迁移/平台验收尚未完成。
## 17. Gate 1 Conversations 子批结果

- 提交：`069b7a4 refactor(tauri): split conversation commands`。
- 新增 `commands/conversations.rs`，根 `lib.rs` 移除会话列表/详情/删除及展示正则辅助；Campaign 级联删除保持原语义。
- `lib.rs` 当前 14,360 行；命令属性/注册 175/175，前端唯一 invoke 162，缺失后端命令 0，SQLite active flag references 68。
- 验证全绿：`cargo check -p storyforge --all-targets`、`cargo clippy -p storyforge --all-targets -- -D warnings`、`cargo test -p storyforge --lib`（344 passed, 3 ignored）、前端合同 3/3。

## 18. 2026-07-28 拆分返修与删除一致性结果

- 前置拆分提交：`cb79875 refactor(tauri): tighten command module boundaries`；本次返修提交：`0485d98 fix(storage): make playthrough deletion retryable`。
- 两个新命令模块恢复为可读 UTF-8 中文源码；移除根模块通配导入，改为显式依赖列表。
- writing/conversations 相关纯测试分别归位到所属模块；命令级 start/regenerate 集成测试仍保留在根模块；内部 DTO、路由和展示辅助恢复私有可见性。
- Campaign 删除命令归回 `commands/campaigns.rs`，级联删除实现抽至 `playthrough_lifecycle.rs`，并改为先删会话、失败保留 Campaign 可重试；旧版本遗留的孤立会话也可补偿清理。
- `CampaignStore::delete_campaign` 现在对 Campaign、实例、知识、任务、总结和本局世界书执行补偿回滚；导入回滚以最终内存/磁盘快照判定，避免把已成功补偿误报为失败。
- 新增会话删除失败、孤立会话重试、级联写入失败回滚测试。
- 当前 `lib.rs` 为 13,976 行；命令属性/注册数 175/175，前端唯一 invoke 162，缺失后端命令 0，SQLite active flag references 68。
- 返修验证：`cargo fmt --all`、`cargo check -p storyforge --all-targets --no-default-features`、`cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings`、`cargo test -p storyforge --lib --no-default-features`（347 passed, 3 ignored）、前端合同测试 3/3 和 `git diff --check`，均通过。

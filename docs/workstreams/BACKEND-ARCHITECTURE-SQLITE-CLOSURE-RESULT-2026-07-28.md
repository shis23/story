# 后端架构拆分与 SQLite 收口：Gate 1 验收结果（2026-07-28）

> 状态：**Gate 1 PASS（最终边界返修已完成）**；Gate 2–8（backend facade、SQLite 迁移、平台验收和发布封存）尚未完成。
>
> code-under-test：`main@d340d99`。
>
> document HEAD：本文件所在文档提交；不把文档提交当作被测代码。

## 1. Gate 1 交付物

- 根模块 `crates/tauri-app/src/lib.rs` 从 2,455 行降至 1,320 行。
- 具体命令实现已归位到 `commands/connections.rs`、`conversations.rs`、`turns.rs`、`writing.rs` 及既有域模块。
- 根测试文件 `lib_tests.rs` 只保留模块声明、公共夹具和导入；域测试拆到：
  - `lib_tests_campaigns.rs`
  - `lib_tests_connections.rs`
  - `lib_tests_conversations.rs`
  - `lib_tests_diagnostics.rs`
  - `lib_tests_import_export.rs`
  - `lib_tests_meta.rs`
  - `lib_tests_startup.rs`
  - `lib_tests_turns.rs`
  - `lib_tests_writing.rs`
  - `lib_tests_writing_regenerate.rs`
- 新增完整注册命令快照：`frontend/tests/fixtures/tauri-registered-commands.snapshot.json`。
- 架构测试升级为四层门禁：命令全集、实现不得回流根模块、根测试模块零测试属性、域文件确实拥有可执行测试。

## 2. 基线

| 指标 | 结果 |
| --- | ---: |
| workspace crate | 16 |
| `lib.rs` 行数 | 1,320 |
| Tauri command 属性 | 175 |
| 注册命令 | 175 |
| 前端唯一 invoke | 162 |
| 前端缺失后端命令 | 0 |
| SQLite active flag references | 68 |

## 3. 验证证据

- `cargo fmt --all -- --check`：通过。
- `cargo check -p storyforge --all-targets --no-default-features`：通过。
- `cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings`：通过。
- `cargo test -p storyforge --no-default-features`：351 passed、3 ignored、0 failed。
- `node --test frontend/tests/tauri-command-contract.test.mjs`：8 passed、0 failed。
- `node scripts/architecture/backend-baseline.mjs`：175/175 注册一致，前端缺失 0。
- `git diff --check`：通过。

## 4. Gate 1 结论

Gate 1 可以正式收口。此前审查指出的两个边界问题已处理：

1. `lib.rs` 不再承载 regenerate、连接配置、会话归档、回合小票、后处理重试和质量查询等具体业务实现。
2. 根测试不再是单体搬家；测试按域文件归位，并由架构合同测试持续约束。

## 5. 未完成范围

Gate 1 PASS 不等于项目总体 PASS。以下仍属于后续 Gate：

- Gate 2：统一 JSON/SQLite 业务状态机与写作后处理。
- Gate 3：单一 backend facade。
- Gate 4–5：SQLite 能力补齐、迁移/反向导出、等价性和故障注入。
- Gate 6：Windows、Android、真机和发布证据。
- Gate 7–8：默认切换、兼容回退、文档和发布封存。

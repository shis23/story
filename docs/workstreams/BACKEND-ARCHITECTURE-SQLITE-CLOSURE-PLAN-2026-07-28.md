# 后端架构拆分与 SQLite 收口计划（2026-07-28）

> 状态：Gate 1 已完成最终边界返修；Gate 2–8 仍未完成。
>
> code-under-test：`main@d340d99`。
>
> document HEAD：本文件所在文档提交；与 code-under-test 分开记录。

## 目标

按 Gate 顺序完成后端架构收口：

1. Gate 0：命令注册、前端 IPC、SQLite 分支和 workspace 基线可重复生成。
2. Gate 1：`tauri-app/src/lib.rs` 只保留启动组装、`AppState`、注册和跨域接线；具体命令实现与域测试归位。
3. Gate 2：Turn/Attempt/Accept、writing、postprocess 和 typed patch 统一状态机。
4. Gate 3：建立单一 backend facade，命令层不再散落 backend selector。
5. Gate 4：补齐 SQLite 的 Meta、MVU、Chronicle、变量、知识、任务和世界书能力。
6. Gate 5：完成 JSON↔SQLite 迁移、等价性、故障注入和重启恢复。
7. Gate 6：完成 Windows、Android、真机和发布证据。
8. Gate 7–8：默认后端切换、兼容回退、文档和发布封存。

## Gate 1 已完成范围

- 命令实现已迁入 `commands/{connections,conversations,turns,writing}.rs` 等域模块。
- `lib.rs` 当前 1,320 行；无 `#[tauri::command]`，无已迁移的具体实现。
- 根测试模块只保留模块声明、公共夹具和导入；可执行测试分布在 10 个域测试文件。
- 命令注册完整快照固定在 `frontend/tests/fixtures/tauri-registered-commands.snapshot.json`。
- 架构门禁同时检查：完整命令集合、根实现泄漏、根测试零属性、域测试归属。
- 当前注册/前端基线：175/175/162，前端缺失后端命令 0。

## Gate 1 验收命令

```powershell
cargo fmt --all -- --check
cargo check -p storyforge --all-targets --no-default-features
cargo clippy -p storyforge --all-targets --no-default-features -- -D warnings
cargo test -p storyforge --no-default-features
node --test frontend/tests/tauri-command-contract.test.mjs
node scripts/architecture/backend-baseline.mjs
git diff --check
```

## 后续执行顺序

### Gate 2：业务状态机

- 把 start/regenerate/accept/postprocess 的决策统一到 backend-agnostic service。
- 用同一组 fixture 对照 JSON 与 SQLite 的状态、错误和事件。
- 保留 active-turn barrier、revision CAS、幂等 Accept 和故障恢复。

### Gate 3：backend facade

- 启动时解析一次 backend，注入 AppState/facade。
- 命令层只依赖 facade，不直接读取 `is_sqlite_active()`。
- 明确 supported、degraded、unsupported、migration-required 能力。

### Gate 4–5：SQLite 与迁移

- 补齐 Meta/MVU/Chronicle/变量/知识/任务/世界书写入路径。
- 为迁移、反向导出、锁冲突、磁盘失败和重启恢复增加可复核证据。
- 禁止静默双写、隐式 JSON fallback 和第二权威数据源。

### Gate 6–8：平台与发布

- Linux CI 先完成 Android 编译门；再做 Windows/Android 真机验收。
- 记录 APK、picker、keyring、SAF、WebView2 和发布签名证据。
- 更新 README、架构、handoff、roadmap 和 release checklist。

## 约束

- 不改变既有 Tauri 命令名、参数、DTO、事件和前端 IPC 合同。
- 每个 Gate 独立验证、独立提交；失败停在当前 Gate。
- 编译通过不等于真机通过；平台证据必须分级记录。

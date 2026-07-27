# 可复制提示词：Android Phase 6 第一切片

你负责 StoryForge 的 **Android Phase 6 第一切片**。你不是唯一在项目中工作的代理；Release CI、CSP 和桌面验收在其他 worktree/主会话进行。不要还原或覆盖他人的改动。

## 开工要求

1. 建立独立 worktree/分支 `codex/android-phase6-slice1`，从开工时最新 `main` 记录 base SHA。
2. 完整阅读：
   - `AGENTS.md`
   - `docs/workstreams/PARALLEL-WORKSTREAM-HANDOFF-2026-07-27.md`
   - `docs/PLAN-ANDROID.md`
   - `docs/workstreams/OPEN-ISSUES-SWEEP-2026-07-27.json` 中 Android 条目
   - 当前 capabilities、导入前端与 Tauri 命令实现
3. 先区分已实现、仅桌面验证、仅编译验证和真机未验证，OPEN-ISSUES 中与当前 HEAD 冲突的旧事实不得照抄。

## 本切片目标

优先完成或诚实封存三项：

1. AND-2：Android 系统选择器导入 PNG/JSON，覆盖中文文件名、较大文件、Downloads/Documents；选择后内容进入 app data，不依赖临时 URI 长期存活。
2. Android SecretRef/keyring：提供最小写/读/删冒烟路径；能接真机就实跑，不能接则完成编译、命令和明确的人工检查点，不得声明通过。
3. 诊断包 save/share：核对现有 `log_export_bundle` 与 Android 系统保存/分享入口；真机不可用时明确 BLOCKED。

不要在本切片同时实施完整 schema 迁移、后台生命周期重构、SQLite 切换或所有 Phase 6 项目。

## 文件所有权与约束

你拥有 Android picker/capability/manifest、必要的前端导入适配及对应测试。本轮若确有必要，可以修改 `crates/tauri-app/src/lib.rs`，但应保持最小差异并在 RESULT 中列出具体函数。

禁止修改：

- `.gitea/workflows/**`
- `tauri.conf.json` 的应用级 CSP
- 写作流水线模式/路由
- SQLite 默认后端和数据模型
- `docs/workstreams/EXECUTION-PROGRAM-2026-07-27.md`

禁止扩大文件系统权限、绕过 Tauri capability、把 key 写入文件/日志/诊断包、手工重写整个 `gen/android`、安装未知二进制或删除现有 Android 工程。

## 工作方法

1. 为路径归一、临时 URI 复制、错误分类和诊断脱敏先补测试或可复现探针。
2. 使用真实 Android 条件编译；桌面测试不能冒充 Android 证据。
3. 真机步骤按 checkpoint 执行并保存脱敏截图/日志到 gitignored artifacts。
4. 若设备/ADB/SDK/keyring provider 缺失，完成所有可离线工作后标为 PARTIAL/BLOCKED，并给出用户只需执行的最短命令或点击步骤。
5. 生成 `docs/workstreams/ANDROID-PHASE6-SLICE1-RESULT-2026-07-27.md`，逐项区分 compiled、emulator、physical-device。

## 验证与交付

至少运行相关 Rust/前端测试、capabilities 测试、Android aarch64 check 或现有主流构建门，并执行 `git diff --check`。完成代码审查后独立提交，不 push。

最终报告 base/head SHA、真机是否实际连接、三项状态、验证命令、artifact/RESULT、commit 和明确剩余阻塞，不得使用“应该可用”作为 PASS。

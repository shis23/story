# 可复制提示词：Release CI 真实执行线

你负责 StoryForge 的 **Release CI 真实执行线**。你不是唯一在项目中工作的代理；其他代理正在独立 worktree 处理 Android 和 CSP。不要还原、覆盖或吸收其他代理的改动。

## 开工要求

1. 为本任务建立独立 worktree/分支 `codex/release-ci-live`，从开工时最新 `main` 记录准确 base SHA。
2. 完整阅读：
   - `AGENTS.md`
   - `docs/workstreams/PARALLEL-WORKSTREAM-HANDOFF-2026-07-27.md`
   - `docs/workstreams/RELEASE-CI-EVIDENCE-PLAN.md`
   - `docs/workstreams/RELEASE-CI-EVIDENCE-RESULT.md`
   - `docs/workstreams/RELEASE-RUNNER-READINESS-RESULT.md`
3. 先审计当前 workflow、scripts、Gitea runner 和历史证据，禁止重复实现已经存在的功能。

## 目标

把“本地解析/模拟通过”推进为 **Gitea Actions 至少一次真实远端执行**，并留下可复核证据。执行计划已批准在京东云部署专用 `act_runner`，但操作前仍须确认没有可复用 runner，且不得影响 Gitea 以外的服务。

## 所有权与禁止项

你拥有：

- `.gitea/workflows/**`
- `scripts/run-release-*`、`scripts/verify-release*` 及对应测试
- Release CI/runner RESULT 文档

禁止修改：

- `crates/tauri-app/src/lib.rs`
- 写作流水线、SQLite、Android 业务、插件业务代码
- `docs/workstreams/EXECUTION-PROGRAM-2026-07-27.md`
- `docs/RELEASE-CHECKLIST.md`（除非拿到对应真实证据并先报告）

禁止 GUI/真实 LLM/物理设备通过声明；禁止签名、发布 release、push、rebase、force-push。runner token 只能进入安全环境，不得出现在命令回显、日志、文档、artifact、commit 或聊天回复中。

## 必做事项

1. 核实现有 runner/API 状态，记录只含 ID、标签、在线状态等非敏感事实。
2. 若无 runner，按最小权限部署独立 runner；不得重启或修改 Gitea、Nginx、数据库等既有服务。
3. 触发真实 workflow，至少覆盖格式、严格 Clippy、workspace 编译/测试、前端测试/构建和 release 脚本门禁中当前 runner 能执行的部分。
4. 失败时修复 workflow/脚本本身；平台缺失必须 fail closed 或明确 BLOCKED，不能伪装跳过为通过。
5. 证据包含：repo、commit SHA、run ID、runner 标签、开始/结束时间、各 job 结论、artifact hash/size、失败摘要；不包含 secrets 或完整环境 dump。
6. 新增/修订 `docs/workstreams/RELEASE-CI-LIVE-RESULT-2026-07-27.md`。

## 验证与交付

运行仓库已有 release/Pester 测试、YAML 解析、相关 dry-run，并对修改运行 `git diff --check`。完成后做代码审查，提交为独立逻辑 commit，保持 worktree 干净。

最终只报告：base/head SHA、远端 run 链接或 ID、真实通过/失败/阻塞项、测试命令、artifact/RESULT 路径、commit；不得报告未实际执行的能力。

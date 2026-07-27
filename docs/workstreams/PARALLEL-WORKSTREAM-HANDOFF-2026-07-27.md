# 并行工作流交接（2026-07-27）

> 决策：暂停多意图/多 seed 评测。当前并行三条工程线；桌面 Bronze 真实验收由主会话与用户共同执行。

## 1. 本轮并行布局

| 工作流 | 建议分支 | 本轮交付 | 明确不做 |
| --- | --- | --- | --- |
| Release CI | `codex/release-ci-live` | Gitea Actions runner 与 workflow 真实执行证据 | GUI、真实 LLM、签名、发布 |
| Android 第一切片 | `codex/android-phase6-slice1` | AND-2 文件导入、keyring/诊断导出真机就绪度 | 数据模型分叉、完整 Phase 6、SQLite 切换 |
| V5 CSP | `codex/security-app-csp` | 应用级 CSP 与卡壳兼容回归 | V6 权限体系统一、插件 API 重构 |
| Desktop Bronze | 主会话 | Windows GUI + 真实模型 + 独立数据目录证据 | 由其他代理代报人工通过 |

完整提示词分别位于：

- `AGENT-PROMPT-RELEASE-CI-2026-07-27.md`
- `AGENT-PROMPT-ANDROID-PHASE6-2026-07-27.md`
- `AGENT-PROMPT-SECURITY-CSP-2026-07-27.md`
- 主会话 Runbook：`DESKTOP-BRONZE-LIVE-ACCEPTANCE-2026-07-27.md`

## 2. 分支与工作区纪律

1. 每条线使用独立 Git worktree，不在共享 `main` 工作树直接开发。
2. 开工先记录准确 base SHA；不假设提示词编写时的 HEAD 仍是最新。
3. 不得 reset、checkout、清理或覆盖其他代理的改动；发现交叉修改需求先停下并报告。
4. 每条线独立提交，提交信息只描述本线；不得 push、rebase、force-push，除非用户另行要求。
5. 生成证据写入 gitignored `artifacts/`；入库 RESULT 只记录脱敏摘要、命令、SHA 和结论。
6. API key、runner token、签名材料只能进入进程环境或系统密钥存储，不得写入命令日志、补丁、文档或 artifact。

## 3. 文件所有权

| 路径/能力 | 本轮所有者 | 说明 |
| --- | --- | --- |
| `.gitea/workflows/**`、release scripts/tests | Release CI | 不改产品业务代码 |
| Android picker/capability/manifest、必要的导入适配 | Android | 本轮唯一允许因 Android 修改 `tauri-app/src/lib.rs` 的线 |
| `tauri.conf.json`、CSP 构造与前端资源策略 | V5 CSP | 不修改 `tauri-app/src/lib.rs` |
| 桌面验收数据、截图、日志、RESULT | 主会话 | 其他代理不得声称 GUI 已验收 |

热点文件规则：

- `crates/tauri-app/src/lib.rs`：本轮只归 Android 线；CSP 与 CI 线禁止修改。
- `frontend/src/tauri-api.js`：Android 如必须修改须在 RESULT 中点名；CSP 线绕开。
- `docs/workstreams/EXECUTION-PROGRAM-2026-07-27.md`：三个代理都不直接改，由主会话合并后统一回写。
- `docs/RELEASE-CHECKLIST.md`：没有真实新证据不得勾选。

## 4. 依赖与合并顺序

三条线可以独立开发和验证，推荐合并顺序：

1. Release CI（业务风险最低）；
2. V5 CSP（需重新跑前端与卡壳回归）；
3. Android 第一切片（平台差异最大，最后做集成回归）。

桌面 Bronze 以主会话开始验收时的 SHA 为候选基线。并行分支合入后，若修改了运行时资源策略或导入路径，只补跑受影响的桌面检查点，不伪造整轮复跑。

## 5. 暂缓队列

以下工作不在本轮并行范围：

- 多意图、多 seed、Duet cohort 评测；
- V6 插件/卡壳权限体系统一；
- JSON/SQLite 默认后端选择与迁移；
- Accept 状态机统一；
- `tauri-app/src/lib.rs` 巨石拆分；
- 写作流水线 V2.1 功能开发。

V6、状态机和巨石拆分共享核心文件，待 Android 第一切片结束后顺序排期。

## 6. 每条线的完成定义

- 有准确 base/head SHA 和干净 worktree；
- 有失败先行或至少可复现的基线测试，修复后验证命令全绿；
- 有独立 RESULT，明确 PASS、PARTIAL 或 BLOCKED，不以“命令未报错”替代证据；
- 不包含密钥、正文或设备私密数据；
- 有一个或多个逻辑提交，未擅自推送。

# M5 Evidence Retention 工作计划

> 分支：`codex/m5-evidence-retention`
> 基线：`8732e21`
> 类型：真实评估运行前的证据持久化与 fail-closed 基建；本线 **0 次真实模型调用**

## 目标

让下一次 M5 真实运行可在脱敏前提下保留、校验、归档和恢复证据，而不是把唯一 JSONL/checkpoint 放在会被空间清理的 `C:\tmp`。这条线不重新运行已丢失的 45/100 checkpoint；它为未来的新 Full 100-Accept run 提供可靠证据链。

## 已知事实

- 2026-07-14 的 45/100 是历史 Partial Evidence；四个 `C:\tmp\endurance-evidence-*` 目录和原始 `campaign_data` 已清理，不能续跑或独立重放。
- runner、checkpoint/resume、预算和 `check_no_secrets()` 已有确定性测试；缺口是运行后保存和离线审计，不是把旧数据找回。

## 交付范围

1. 设计并实现一个显式的持久 evidence root（环境变量或 CLI 参数；默认行为必须保守且文档化），拒绝 repo 内、临时清理目录、live Campaign data 目录和含路径逃逸的目标。
2. 为一次运行生成唯一 run ID、原子 manifest、schema/version、时间戳、模型与预算的**非敏感**摘要、文件 hashes 和完成/中断状态。
3. 对 `calls`、`turns`、`checkpoint`、`manifest` 与必要 campaign snapshot 建立封存/验证流程：
   - 只允许已脱敏内容；
   - archive/restore 后可离线 re-hash；
   - 缺文件、hash 不符、混用 run ID 或 schema 不符时 fail closed；
   - 不把原始 prompt、story、private knowledge、API response body、credential 或绝对主机路径写入证据。
4. 提供显式、可审计的 retention/cleanup 命令：只删除命名空间内的已完成证据，永不自动删除当前运行或未知目录。
5. 更新 runner 使用说明与本线 RESULT；同时明确旧 45/100 仍不可恢复。

## 明确不做

- 不设置或读取真实 API key，不运行 ignored real-LLM 测试，不产生付费调用。
- 不伪造、重写或补填旧 45/100 的 JSONL。
- 不改变 M5 `max_tokens`、模型、预算、acceptance 门槛或生产 Context 参数。
- 不改 Tauri/SQLite/GUI/Android，以及 `docs/HANDOFF.md`、`docs/RELEASE-CHECKLIST.md`、历史 RESULT。

## 冲突隔离

- 本线拥有 `crates/harness-real-llm/**`、仅相关的 `scripts/**` 和本线 docs。
- 不修改 `crates/tauri-app/**`、`crates/infra-sqlite/**` 或 ProductionPostprocessService 线的文件。
- 任何把运行证据上传到外部服务的动作都必须停下并请求用户授权；本线只实现本地/可搬运的脱敏封存。

## TDD 与验收

先增加确定性测试。至少覆盖：

1. 正常完成、中断和 resume 的 manifest/state 转换；不重放已记录 Accept。
2. 缺 checkpoint、篡改 hash、混合 run、重复 run ID、schema 漂移、非法 evidence root 全部 fail closed。
3. archive/restore/re-hash 完整性，且不会写入原始 prompt/正文/secret/绝对路径。
4. retention 只操作受控命名空间，保护 active run，拒绝 reparse/path traversal。
5. 证据路径、终端输出和错误消息均不回显凭证。

最低门禁：

```powershell
cargo fmt --all -- --check
cargo test -p harness-real-llm --lib
cargo test -p harness-real-llm --test endurance_deterministic
cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic
cargo clippy -p harness-real-llm --all-targets -- -D warnings
git diff --check
```

若增加 PowerShell 脚本，同步增加 Pester 测试。不得把“脚本可运行”表述为真实模型证据。

## 交付与提交

- 建议提交：schema/红测 → archive/verify → runner/preflight → cleanup → RESULT。
- 新增 `docs/workstreams/M5-EVIDENCE-RETENTION-RESULT.md`，列出可复现的 deterministic 证据和明确未执行的真实运行。
- 不 push、不 merge、不清理其他 worktree。

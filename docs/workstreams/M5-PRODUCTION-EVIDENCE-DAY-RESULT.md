# M5 生产证据线 Day Result

> 分支：`codex/m5-production-evidence`
> 基线：`main@053847f`
> 工作目录：`C:\tmp\storyforge-m5`
> 日期：2026-07-13

## 结论

本线提供了跨 `H_anchor + E` 的多轮 evidence orchestration，但必须诚实拆分为：

1. **write path**：真实入口使用 production `PipelineOrchestrator::start_writing`。
2. **Chronicle path**：当前使用 `synthetic_chronicle_fixture`，不是 Tauri 完整
   Summarizer/PostProcessor/TurnAttempt 后台写回。
3. **accept path**：使用 `production_faithful_commit_probe`，复刻生产 Accept 的关键
   Turn/Attempt、revision、hash、MutationBatch 与 FinalizeVariant 行为。

因此最终结论是：

- 多轮 Context/epoch orchestration：**PASS（确定性/假客户端）**
- Budget、hard deadline、脱敏 JSONL、fail-closed：**PASS（自动测试）**
- production pipeline write 真实入口：**可执行但默认 ignored**
- production Summarizer/postprocess 完整闭环：**未覆盖**
- 真实模型证据与完整 M5 验收：**Inconclusive / 未通过**

本线真实/付费模型调用次数为 **0**。

## Commit

| Commit | 说明 |
| --- | --- |
| `4f84797` | `docs(workstream): plan M5 production evidence day` |
| `3a6ab23` | `feat(eval): run production-faithful M5 evidence loop` |
| `4ba4cc6` | `docs(workstream): record M5 production evidence result` |
| *(本次返修提交)* | cursor/deadline/诚实字段/fixture/DryRun 修复与 RESULT 更新 |

## 返修问题与处理

### 1. setup/extract sample 污染

真实入口在进入 runner 前可能已由 `extract_characters` 使用同一个
`BudgetedLlmClient`。runner 现在：

- 进入时记录 `calls_before` 与 `sample_cursor = llm.samples().len()`。
- calls JSONL 只写此后每轮新增 sample。
- 每轮要求 sample 增量 `> 0`；setup sample 不能替零调用 turn 放行。
- report 的 `calls_used` 是 runner 增量；`max_calls` 仍是包含 setup/extract 的 suite-wide
  硬预算。

### 2. suite hard deadline

默认 hard deadline 为“剩余 calls × per-call timeout”，测试可显式传更短 deadline。
deadline 从 evidence writer 创建前开始，并在以下阶段前后检查：

- loop/input/context fill
- writer future
- call evidence 写入
- prepare/Accept
- turn evidence 写入
- final Context fill

writer 或 pre-evidence stage 超时时，会尽最大可能把已经完成的 `UsageSample` 脱敏写入
calls JSONL；写入错误优先传播，不能被 timeout 吞掉。同步 Accept/evidence/final fill 即使
不能被异步取消，也会在返回后立即检查 deadline 并 fail closed。

### 3. synthetic Chronicle 诚实标识

完整 production postprocess 没有可在本 harness 切片安全复用的公开接口；Tauri 路径包含
私有后台 postprocess、`build_mutation_batch` 与 TurnAttempt 写回。为避免跨边界大重构，
本线采用诚实降级：

- `WrittenProductionTurn` 必须显式携带 `ChronicleCandidateSource`。
- 当前唯一来源为 `SyntheticChronicleFixture`。
- turns JSONL 新增 `write_path`、`chronicle_path`、`accept_path`、
  `production_postprocess_complete`。
- 当前真实入口记录：
  - `write_path=production_pipeline`
  - `chronicle_path=synthetic_chronicle_fixture`
  - `accept_path=production_faithful_commit_probe`
  - `production_postprocess_complete=false`
- evidence kind 为 `pipeline_write_synthetic_chronicle_accept`，不再使用容易误解的完整
  production CommitTurn 闭环命名。

### 4. fixture fail-closed

仓库内没有默认 `test-card-seraphina.png`。真实入口与 smoke 脚本现在都强制要求：

```text
STORYFORGE_EVAL_FIXTURE_CARD=<existing character-card PNG>
```

缺变量或文件不存在时，在创建真实 LLM client / 执行 cargo test 之前失败。

### 5. PowerShell DryRun

`-DryRun` 现在先于凭证、付费授权与 fixture 校验生效。无
`LLM_BASE_URL/API_KEY/MODEL`、无 `STORYFORGE_EVAL_REAL_LLM`、无 fixture 时仍可打印准确的
eval cargo 命令，且不会执行模型调用。

## TDD 红测 → 绿测

### 红测证据（旧实现）

1. 预先产生 setup sample 后，成功用例得到 calls **17** 行而不是 16 行。
2. setup sample 让零调用 writer 的 turn1 被误放行，错误直到 **turn2** 才出现。
3. `-DryRun` 无凭证时退出 1：`Missing required LLM environment variable(s)`。
4. deadline stage hook、显式 Chronicle source、显式 fixture API 在旧实现中不存在，编译红灯。

### 绿测证据

```text
cargo test -p harness-real-llm --test m5_production_evidence -- --nocapture
PASS: 8 passed / 0 failed
```

覆盖：

- setup cursor 隔离与逐轮 sample 增量。
- calls/turns JSONL 脱敏与 epoch rollover。
- writer 零调用、budget 耗尽、evidence I/O 失败、未跨 epoch。
- deadline 覆盖 pre-evidence、post-evidence、post-Accept 与 final fill。
- timeout 后已完成 usage best-effort 落盘。
- `production_postprocess_complete=false` 与三段路径字段。
- 显式 fixture override。

```text
cargo test -p harness-real-llm --test real_llm_smoke_script
PASS: 2 passed / 0 failed
```

覆盖：

- eval DryRun 无凭证成功。
- 非 DryRun 缺 fixture 时在 cargo/model 调用前失败。

## 最终门禁

| 命令 | 结果 |
| --- | --- |
| `cargo test -p harness-real-llm` | PASS，71 passed / 17 ignored / 0 failed |
| `cargo test -p harness-real-llm --lib` | PASS，17 passed |
| `cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic` | PASS，5 passed |
| `cargo test -p harness-real-llm --test eval_m5_phaseb_real_llm` | PASS，0 passed / 1 ignored |
| `cargo clippy -p harness-real-llm --all-targets -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 053847f..HEAD` | PASS |

真实 ignored 用例名：

```text
eval_real_llm_pipeline_write_synthetic_chronicle_accept_across_epoch
```

## 真实模型运行命令

以下命令会产生付费调用，必须另行明确授权：

```powershell
$env:STORYFORGE_EVAL_REAL_LLM='1'
$env:LLM_BASE_URL='https://provider.example/v1'
$env:LLM_API_KEY='<secret>'
$env:LLM_MODEL='<model>'
$env:STORYFORGE_EVAL_FIXTURE_CARD='C:\path\to\existing-card.png'
$env:STORYFORGE_EVAL_MAX_CALLS='96'
$env:STORYFORGE_EVAL_MAX_TURNS='16'
$env:STORYFORGE_EVAL_TIMEOUT_SECS='180'
$env:STORYFORGE_EVAL_EVIDENCE_DIR='C:\tmp\storyforge-m5-evidence'
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite eval
```

无凭证预览命令：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite eval -DryRun
```

## 未完成项与边界

1. 没有真实 ≥16 轮 calls/turns JSONL、token/cache/延迟曲线；真实调用数为 0。
2. 没有 production Summarizer/PostProcessor/TurnAttempt 后台完整闭环证据。
3. synthetic Chronicle fixture 只能验证 Context/Accept/epoch orchestration，不能验证真实摘要
   语义质量或压缩损失。
4. `CommitProbeEnv` 仍需随 Tauri 私有 Accept 路径漂移而复核。
5. 不得据此标定或修改 `200/4`、`H_anchor/E`，不得宣称完整 M5 通过。

本线未修改 SQLite、GUI、Android、`docs/HANDOFF.md` 或生产默认参数。

# M5 生产证据线 Day Result

> 分支：`codex/m5-production-evidence`
> 基线：`main@053847f`
> 工作目录：`C:\tmp\storyforge-m5`
> 日期：2026-07-13

## 结论

本线已补齐当前 M5 最大的本地可推进缺口：真实模型入口不再是单轮 smoke，而是可执行
同一 Campaign / Conversation 上的多轮 `write → CommitTurn/Accept` 生产证据循环。循环每轮
重新走生产 `fill_campaign_context`，使用实际持久化的 `ContextEpochSnapshot` 观察 rollover，
并要求 Accept 数严格大于 `H_anchor + E = 15`。

本线没有调用任何付费模型。结论是：

- **多轮生产证据 orchestration：PASS（确定性/假客户端）**
- **预算、timeout、脱敏 JSONL、fail-closed：PASS（自动测试）**
- **真实模型 ≥16 轮证据：Inconclusive（0 次真实调用，入口保持 ignored）**
- **M5 参数标定与完整验收：未宣称通过**

## Commit

| Commit | 说明 |
| --- | --- |
| `4f84797` | `docs(workstream): plan M5 production evidence day` |
| `3a6ab23` | `feat(eval): run production-faithful M5 evidence loop` |
| *(本 RESULT 提交)* | `docs(workstream): record M5 production evidence result` |

## 实现内容

### 多轮 runner

新增 `crates/harness-real-llm/src/production_evidence.rs`：

1. 每轮追加真实 user input node。
2. 每轮调用 `HarnessEnv::fill_campaign_context`，复用生产 Context 编译入口。
3. 真实 writer 调用 `PipelineOrchestrator::start_writing`。
4. 使用实际 input node 创建 `TurnRecord` / `TurnAttempt`，再通过现有生产忠实
   `CommitProbeEnv` 完成 AwaitingAcceptance → CommitTurn/Accept。
5. 下一轮继续使用同一 Campaign、Conversation、CampaignStore、ConversationStore 与
   TurnStore；不存在第二份磁盘快照或手工轮数替代 epoch 的旁路。
6. 最后一轮 Accept 后再执行一次 Context 编译，要求观察到至少两个不同 epoch id 指纹。

`CommitProbeEnv` 与 `HarnessEnv` 现在可共享同一组 `Arc` store；原有确定性 Accept 测试与
其他 harness 用例保持通过。

### suite-wide 预算与 timeout

同一次运行只构造一个 `BudgetedLlmClient`：

- `max_calls` 使用共享原子计数，是整个 suite 的硬上限。
- 每次 `chat` / `chat_stream` 使用 `timeout_secs`。
- runner 另设 `timeout_secs × max_calls` 的 suite wall-clock 上限。
- 成功、客户端错误、timeout 调用都会生成脱敏 usage sample；不写供应商原始错误正文。
- `max_turns` 小于 16、`max_calls=0`、预算耗尽或 suite timeout 均失败。

### JSONL 证据

沿用 `eval-m5-phaseb-v1`，不建立第二套 schema：

- `calls.jsonl`：逐调用写 role/tag/streaming、segment hash、usage、耗时与 outcome。
- `turns.jsonl`：逐 Accept 写 revision、Chronicle A code、Attempt/Turn status、正文 hash、
  epoch id/source hash 指纹与 anchor 数量。
- `EvidenceWriter` 的任何创建或写入错误都会向上传播，运行失败。
- 自动守卫继续拒绝 API key、`sk-`、Bearer、完整 `prompt`/`messages` 与 `SF_SECRET_*` 原文。

### fail-closed 条件

以下条件均不会返回 PASS：

- fixture 缺失；fixture 检查发生在真实 client 构造之前。
- 某一写作轮产生零个 LLM 调用。
- 未完成全部目标 Accept。
- Accept 数未严格超过 `H_anchor + E`。
- 实际生产 Context 编译只观察到一个 epoch。
- 预算耗尽、调用/总 suite timeout、模型/写作错误。
- calls/turns JSONL 创建或写入失败。

## 自动验证

### harness 全包测试

```text
cargo test -p harness-real-llm
PASS: 66 passed / 0 failed / 17 ignored
```

其中真实 M5 生产证据用例按预期：

```text
eval_real_llm_production_write_commit_accept_across_epoch ... ignored
```

### 新增 orchestration 专项

```text
cargo test -p harness-real-llm --test m5_production_evidence -- --nocapture
PASS: 5 passed / 0 failed
```

覆盖：

1. 16 次真实 store Accept 后，生产 Context 编译实际观察到 epoch rollover；calls/turns
   JSONL 各 16 行且脱敏。
2. 16 次 Accept 但不产 Chronicle A 时，生产编译器不发生 epoch 进展，runner fail closed。
3. writer 零 LLM 调用时立即失败。
4. suite-wide `max_calls` 耗尽时失败。
5. evidence 路径不可写与 fixture 缺失时失败。

### 既有专项与静态门禁

| 命令 | 结果 |
| --- | --- |
| `cargo test -p harness-real-llm --lib` | PASS，17 passed |
| `cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic` | PASS，5 passed |
| `cargo test -p harness-real-llm --test eval_m5_phaseb_real_llm` | PASS，0 passed / 1 ignored |
| `cargo clippy -p harness-real-llm --all-targets -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check 053847f..HEAD` | PASS |
| `run-real-llm-smoke.ps1 -Suite eval -DryRun` | PASS；命令只指向新的多轮真实用例 |

## 真实模型调用

| 项 | 值 |
| --- | --- |
| 真实/付费调用次数 | **0** |
| 费用 | N/A |
| 真实模型耗时 | N/A |
| 仓库内真实证据 JSONL | 无；运行时目录不入 Git |

推荐的真实运行命令（执行会产生付费调用，需用户另行明确授权）：

```powershell
$env:STORYFORGE_EVAL_REAL_LLM='1'
$env:LLM_BASE_URL='https://provider.example/v1'
$env:LLM_API_KEY='<secret>'
$env:LLM_MODEL='<model>'
$env:STORYFORGE_EVAL_MAX_CALLS='96'
$env:STORYFORGE_EVAL_MAX_TURNS='16'
$env:STORYFORGE_EVAL_TIMEOUT_SECS='180'
$env:STORYFORGE_EVAL_EVIDENCE_DIR='C:\tmp\storyforge-m5-evidence'
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite eval
```

也可直接运行：

```powershell
cargo test -p harness-real-llm eval_real_llm_production_write_commit_accept_across_epoch -- --ignored --nocapture
```

## 未完成项与风险

1. **真实模型证据仍未产生**：没有真实 ≥16 轮 calls/turns JSONL、token/缓存/延迟曲线，
   不得据此标定 `200/4`、`H_anchor/E` 或宣称完整 M5 验收通过。
2. 真实 writer 使用生产写作 Pipeline，但 Tauri 的后台 Summarizer/PostProcessor 不作为独立
   command 暴露；runner 使用正文指纹生成最小 Chronicle A 候选，再通过生产忠实
   MutationBatch/Accept 路径落盘。真实摘要语义质量仍需付费运行后的独立证据。
3. `CommitProbeEnv` 仍镜像 Tauri 私有 `commit_turn_attempt` 的关键行为；若生产私有路径后续
   漂移，harness 需要同步复核。
4. 真实 Pipeline 每轮通常不止一次 LLM 调用。`96 calls / 16 turns` 是首次实跑建议预算，
   不是生产默认参数；预算不足会正确 fail closed。

## 边界确认

本线未修改：

- SQLite 或 JSON Store 架构
- Tauri GUI、Android、桌面发布流程
- `docs/HANDOFF.md`
- 生产默认 `overview_max_entries=200`、压缩 `200/4`、`H_anchor=5`、`E=10`

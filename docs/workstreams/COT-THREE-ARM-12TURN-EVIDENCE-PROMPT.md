# 交给执行 Agent 的任务说明

请在 StoryForge 工作区完成 CoT 三臂 × 12 轮真实模型证据任务。

先完整阅读：

1. `docs/workstreams/COT-THREE-ARM-12TURN-EVIDENCE-PLAN.md`
2. `crates/harness-real-llm/fixtures/cot_three_arm_12turn_v1.json`
3. `crates/harness-real-llm/tests/endurance_sqlite_real_llm.rs`
4. `crates/harness-real-llm/src/sqlite_endurance.rs`
5. `crates/harness-real-llm/src/budget.rs`
6. `crates/harness-real-llm/src/coverage_ledger.rs`

任务分两阶段，禁止跳过第一阶段直接消耗真实模型预算。

## 阶段一：补齐并验证 harness

严格完成 PLAN 的 G0.1-G0.8：

- fixture 可选择、identity/resume 绑定同一解析结果。
- 新卡所有已声明的 production-supported 字段真实 seed；unsupported 明示。
- arm 可配置 provider extra。
- 所有真实 LLM 调用必须写独立、受限、可恢复的 private raw trace。
- 每 arm 420 calls、180s/call、4h deadline，禁止 `u32::MAX`。
- per-agent prompt/tool/call/result 关联证据。
- 第 12 轮 warning-only 质量 fault + Editor-only autofix。
- 第 6 轮 graceful checkpoint stop，新 PID 从第 7 轮恢复。

每项先写确定性测试。至少运行并通过：

```powershell
cargo fmt --all -- --check
cargo test -p storyforge-infra-llm
cargo test -p storyforge-app-agent
cargo test -p storyforge-app-pipeline
cargo test -p storyforge-app-meta
cargo test -p harness-real-llm --lib
cargo test -p harness-real-llm --test endurance_sqlite_deterministic
cargo test -p harness-real-llm --test endurance_sqlite_real_llm
cargo test -p harness-real-llm --test evidence_retention_deterministic
cargo test --workspace --no-run
git diff --check
```

`endurance_sqlite_real_llm` 中被 `#[ignore]` 的真实入口此阶段不要启用。确定性门通过后提交 harness 改动；真实运行前 `git status --porcelain` 必须为空。

## 阶段二：三条真实 12 轮 run

先做同一模型 capability/preflight：

- Disabled 能关闭 surfaced reasoning。
- Native 能返回 provider reasoning。
- Prompted 能返回 reasoning，且角色 CoT 注入正确。
- 同一 immutable context 下 Disabled/Native system 相同，Prompted 只多预期角色 CoT 模块。

若同一模型无法满足三臂，停止并报告 blocker；不得换模型拼接结果。

然后用同一 commit/model/endpoint/protocol/tool mode/fixture/intent/sampling 依次运行：

1. Disabled：12/12。
2. Native：12/12。
3. Prompted：12/12。

每条 arm：

- 全新 SQLite/public evidence/private trace roots。
- 第 6 轮 Accept 后 graceful stop；独立新 PID resume 7-12。
- 监控 PID、stdout、stderr、checkpoint、call reservation 与 trace，不要让 poll loop 终止 cargo。
- 任一 fail-closed gate 失败就保留现场并停止，不删除失败证据。
- 完成后立即 seal、offline verify、secret scan、SQLite reopen audit、private trace exact-set。

完成三臂后，先对 36 篇 accepted 正文做盲评，再看 reasoning。逐调用审阅 reasoning 格式、工具选择和 Agent 作用；不得只看 hash 或只抽最后一轮。

结果写入：

```text
docs/workstreams/COT-THREE-ARM-12TURN-EVIDENCE-RESULT.md
```

RESULT 必须覆盖 PLAN 第 12 节全部字段，给 public evidence 与 private trace index 的绝对路径。原始 prompt/reasoning/output 不提交进 Git；只提交脱敏 RESULT 和必要的 harness/tests/scripts。不要 push。

结论必须克制：这是一条 12 轮轨迹/arm，只能报告本次配对观察；不能凭单轨迹宣称 Prompted 普遍优化。

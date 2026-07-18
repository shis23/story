# 交给执行 Agent 的任务说明

请在 StoryForge 工作区完成 CoT 三臂 × 80 轮真实模型综合验证。不要把它降级成 12 轮 smoke test，也不要只审最后一轮。

先完整阅读：

1. `docs/workstreams/COT-THREE-ARM-80TURN-EVIDENCE-PLAN.md`
2. `crates/harness-real-llm/fixtures/cot_three_arm_80turn_v1.json`
3. `crates/harness-real-llm/tests/endurance_sqlite_real_llm.rs`
4. `crates/harness-real-llm/src/sqlite_endurance.rs`
5. `crates/harness-real-llm/src/budget.rs`
6. `crates/harness-real-llm/src/coverage_ledger.rs`

任务分为“补齐 harness”和“三条真实 run”两阶段。第一阶段未全部通过前，禁止启用 ignored 真实入口或消耗真实模型预算。

## 阶段一：补齐并验证 harness

严格完成 PLAN 的 G0.1–G0.10：

- fixture/schedule 单次解析，identity 与 resume 绑定完整 SHA-256。
- 新卡所有 production-supported 字段真实 seed；unsupported 明示，不得静默 PASS。
- 三臂可配置 provider extra，并证明实际 request body。
- 所有真实 LLM 调用必须写独立、权限受限、可恢复的 private raw trace。
- 新增 typed 80-turn `long_coverage` stage；每 arm 2800 calls、180s/call、86400s deadline，禁止 `u32::MAX`。
- per-agent prompt/tool/call/result/terminal 关联证据。
- turn 12/29/57/58/59/78 的多 profile Warning/Error → Editor-only autofix → 复检。
- accepted turn 6/20/40/60 后 graceful stop，以独立新 PID resume。
- fixture 80 个 canonical action 全部被 typed interpreter 支持；未知 action fail-closed。
- cancel/crash、stale edit、fork、Meta/MVU、regex、stream parity、text fallback、type rejection 等旁路探针只在 clone 执行，不污染主轨迹。
- 先修复已知的两个 budget 单测债务：测试显式构造 Prompted/Native，不得把生产 `SamplingParams::default()` 从 Disabled 改回 Prompted，也不得删除 reasoning assertion。

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

## 阶段二：三条真实 80 轮 run

先用同一模型做不计入正式预算的 capability/preflight：

- Disabled 可以关闭 surfaced reasoning；若仍返回，必须捕获并把该臂标为 unexpected，而不是冒充无 CoT。
- Native 返回独立 provider reasoning。
- Prompted 返回 reasoning，且角色 CoT 模块 exact-set 正确。
- immutable context 下 Disabled/Native system canonical hash 相同，Prompted 只增加预期角色模块。
- provider thinking 参数在 Native/Prompted 完全相同。

若同一模型无法满足三臂，停止并报告 blocker；不得换模型拼接结果。

然后在同一 commit/model/endpoint/protocol/tool mode/fixture/schedule/sampling 下依次执行：

1. Disabled：80/80。
2. Native：80/80。
3. Prompted：80/80。

每条 arm：

- 使用全新 SQLite、public evidence 和 private trace roots。
- 严格执行 fixture turns 1–80 及全部 side probes，不得挑轮或把未知 action 当普通 write。
- turn 6、20、40、60 Accept 后完成 checkpoint/SQLite audit/trace flush，graceful exit；由独立新 PID 分别从 7、21、41、61 恢复。
- 监控 PID、stdout/stderr、checkpoint、call reservation 与 trace；poll loop 不得托管或杀死 cargo。
- 每 10 轮做阶段性结构审计：reasoning exact-set、工具配对、Agent cardinality、质量/私密/禁用 lore、SQLite 单调性。只审计，不改 fixture 或放宽门。
- 任一 required fail-closed gate 失败即保留现场并停止；不得删除失败证据、换 run ID 或手改 SQLite 伪装通过。
- turn 80 后立即 public seal、offline verify、secret scan、SQLite reopen audit、private trace exact-set。

重点逐调用检查：

- reasoning 是 provider 独立字段，不混正文；required call 不缺失，Disabled 无意外 reasoning。
- Prompted 的 Director/Editor/Subagent/Summarizer 只注入自己的模块；其他角色不得伪称有模块。
- offered tools、tool call/result ID、args、handler outcome、terminal 状态、下一轮响应完整关联。
- Director 规划、Subagent 独立行动、Editor 合并/修订、Summarizer 长线记忆、PostProcessor 状态写回、CharacterExtractor exact-set、Meta 只读职责真实起效。
- 蓝灯/绿灯 AND-OR-NOT/Both/Global/Disabled、regex、同名角色、临时角色、私密知识、变量五型、任务生命周期、知识传播、Chronicle、epoch、三类 regenerate、质量回灌和工具故障都按 fixture assertion 验收。
- turn 77 只阶段性回收棘轮证据链；turn 80 仍保留异常脉冲的制度性未决。

## 盲评与 reasoning 审计顺序

三臂全部封存后，先去掉 arm/mode 标签，以固定种子打乱 240 篇 accepted 正文，按 PLAN 七个维度盲评。grader 不得先看 reasoning。

盲评完成后再逐调用审阅所有 role/phase 的 reasoning 格式、角色 rubric、工具选择、CoT 注入与 Agent 作用。不得只看长度/hash，不得只抽最后一轮，也不得把 reasoning 更长或工具更多直接判为更好。

## 结果与提交

结果写入：

```text
docs/workstreams/COT-THREE-ARM-80TURN-EVIDENCE-RESULT.md
```

RESULT 必须覆盖 PLAN 第 11 节全部字段，给 public evidence 与 private trace index 的绝对路径，并列出所有失败、偏差、人工干预和未完成项。完整 prompt/reasoning/output/tool result 不提交 Git；只提交脱敏 RESULT 与必要的 harness/tests/scripts。不要 push。

结论必须克制：这是每个 arm 一条 80 轮轨迹。只能报告本次配对观察；不得凭 240 篇正文宣称 Prompted 普遍优化。

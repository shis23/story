# M5 / Phase B 评估线结果

> 分支：`codex/eval-m5-phaseb`
> 基线：`7fb1899`
> 日期：2026-07-13
> 工作目录：`C:\tmp\storyforge-eval`

## 1. Commit 列表（相对基线）

| Commit | 说明 |
|--------|------|
| `01210bf` | docs(workstream): plan M5 and Phase B evaluation |
| `beb7221` | feat(eval): add redacted JSONL evidence writer for M5/Phase B |
| `4e9d4bd` | feat(eval): production-faithful CommitTurn Accept probe |
| `863e99e` | feat(eval): deterministic long-session runner across H_anchor+E |
| `995d686` | feat(eval): Phase B A/B matrix fixtures with shared seed |
| `27c3d71` | test(eval): wire deterministic gates and ignored real-LLM eval suite |
| *(本 RESULT 提交)* | docs(workstream): EVAL-M5-PHASEB-RESULT |

## 2. 修改文件

### 新增

- `crates/harness-real-llm/src/evidence.rs` — 脱敏 JSONL writer / budget / 守卫
- `crates/harness-real-llm/src/commit_probe.rs` — 生产忠实 CommitTurn/Accept 探针
- `crates/harness-real-llm/src/long_session.rs` — ≥20 Accept 长会话 runner
- `crates/harness-real-llm/src/phase_b_matrix.rs` — Phase B A/B 对照矩阵
- `crates/harness-real-llm/tests/eval_m5_phaseb_deterministic.rs` — 确定性门禁
- `crates/harness-real-llm/tests/eval_m5_phaseb_real_llm.rs` — 真实模型入口（`#[ignore]`）
- `docs/workstreams/EVAL-M5-PHASEB-PLAN.md`（计划）
- `docs/workstreams/EVAL-M5-PHASEB-RESULT.md`（本文件）

### 修改

- `crates/harness-real-llm/src/lib.rs` — 导出 eval 模块
- `crates/harness-real-llm/Cargo.toml` / `Cargo.lock` — `sha2` / `chrono`
- `scripts/run-real-llm-smoke.ps1` — suite `eval` + 预算开关打印 / 鉴权

### 未修改（按边界）

- SQLite / JSON Store 架构
- 生产默认 `200/4`、`H_anchor`/`E`
- `docs/HANDOFF.md`
- Android / 桌面发布流程

## 3. 实际执行的测试

### 门禁（本机，无付费模型）

| 门禁 | 结果 |
|------|------|
| `cargo fmt --all -- --check` | PASS |
| `cargo test --workspace --quiet` | PASS（真实 LLM / eval real 用例按预期 ignore） |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `git diff --check 7fb1899..HEAD` | PASS |

### 确定性评估专项

```text
cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic --lib
# lib: 15 passed
# eval_m5_phaseb_deterministic: 5 passed
```

覆盖：

1. **生产 CommitTurn Accept** — TurnRecord/Attempt、draft_hash、QualityGate 拦截/force→Degraded、MutationBatch、Campaign revision bump、Chronicle A 经 Accept 落盘、variant Final
2. **长会话 20 Accept** — `turns_accepted=20`，`max_near_raw = H_anchor+E = 15`，`crossed_h_plus_e=true`，早期事实仍在 store
3. **Phase B A/B** — 同 fixture / 同 seed；baseline 不扫 private leak；B 臂检出后 1× autofix；证据无 `SF_SECRET_*` 原文
4. **JSONL 脱敏** — 拒绝 `api_key` / `SF_SECRET_*` / 完整 messages 写盘

### 真实模型调用

| 项 | 值 |
|----|----|
| 调用次数 | **0**（未经用户明确授权付费模型） |
| 费用 | N/A |
| 耗时（实跑 LLM） | N/A |

真实入口已就绪但默认 ignore：

- `STORYFORGE_EVAL_REAL_LLM=1`
- `LLM_BASE_URL` / `LLM_API_KEY` / `LLM_MODEL`
- `STORYFORGE_EVAL_MAX_CALLS` / `MAX_TURNS` / `TIMEOUT_SECS`
- `scripts/run-real-llm-smoke.ps1 -Suite eval`（无开关会明确失败，不假通过）

## 4. 可独立审计的证据位置

| 证据 | 位置 |
|------|------|
| 模块实现 | `crates/harness-real-llm/src/{evidence,commit_probe,long_session,phase_b_matrix}.rs` |
| 确定性测试 | `crates/harness-real-llm/tests/eval_m5_phaseb_deterministic.rs` |
| 真实模型入口 | `crates/harness-real-llm/tests/eval_m5_phaseb_real_llm.rs` |
| 运行时 JSONL | 测试 tempdir / `STORYFORGE_EVAL_EVIDENCE_DIR`（不入仓） |
| Schema | `eval-m5-phaseb-v1`（call / turn / ab_row） |

## 5. 分项结论

| 目标 | 结论 | 说明 |
|------|------|------|
| 生产 CommitTurn/Accept 探针 | **Pass（确定性）** | 非手工 `add_summary`；复刻 hash/gate/batch/revision/A 落盘 |
| ≥20 Accept 跨 H+E | **Pass（确定性）** | 20/20 Accept；H+E=15；早期事实在 store 可达 |
| 脱敏 JSONL | **Pass** | 自动脱敏测试 + 写盘守卫 |
| Phase B A/B 矩阵 | **Partial Evidence** | 确定性 fixture/同 seed/泄漏+autofix 指标已立；**无**固定真实模型成本/延迟对照样本 |
| 真实模型长会话/缓存标定 | **Inconclusive** | 0 次付费调用；不改 `200/4`、`H_anchor`/`E` |
| 完整 M5 验收 | **未宣称通过** | 探针执行成功 ≠ 参数已标定 ≠ 完整 M5 ACCEPTANCE |

总体：**PROBE EXECUTION PASS（确定性） / M5+PhaseB ACCEPTANCE: Partial Evidence**

## 6. 是否建议合并

**建议合并到主线评估栈（eval 分支）**，作为可复跑的 harness 基建。

**不建议**仅凭本线宣称：

- 生产参数已标定
- 真实模型 Phase B 质量/成本已对照完成
- 完整 M5 验收通过

### 剩余风险

1. CommitTurn 探针复刻公开 store API，与 Tauri `commit_turn_attempt` 私有路径若后续漂移需同步。
2. 长会话确定性路径用合成 draft/summary，不验证真实 LLM 早期事实**叙述可达性**。
3. Phase B 真实 A/B 仍缺固定模型、温度、seed 的多采样成本矩阵。
4. 编译 tauri-app 时本地需要 `frontend/dist`（gitignore）；本线未改发布产物。

### 后续建议

1. 授权后跑：`STORYFORGE_EVAL_REAL_LLM=1` + `run-real-llm-smoke.ps1 -Suite eval`（先小预算）。
2. 真实 ≥20 写作+Accept 长会话（注意 token 预算）。
3. 固定模型参数下 Phase B 泄漏率/延迟/token 对照表写入新证据目录（仍脱敏）。
4. 证据足够前 **不改** 生产 `200/4` 与 `H_anchor`/`E`。

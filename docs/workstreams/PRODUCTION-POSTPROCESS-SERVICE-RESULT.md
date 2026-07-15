# ProductionPostprocessService Result

- 分支：`codex/production-postprocess-service`
- 基线：`23ad240`（plan）/ `5bbb463`（初版共享服务）
- 返修提交：见下方 commit 列表
- 工作目录：`C:\tmp\storyforge-production-postprocess`
- 日期：2026-07-15

## 结论

完成共享 `ProductionPostprocessService` 垂直切片，并完成独立复核指出的 **P0 返修**：

1. **真实取消传播**：postprocess 订阅 `cancel_writing` 同一 `current_cancel` source；`start_writing` 在后台 task 结束前不清理 cancel sender。
2. **关键失败不可吞**：`apply_outcome` / autofix sync / `mark_failed` 有明确错误语义；`regenerate` 向上返回；后台路径 fail-closed 标 Failed 并发出 `PostProcessFailed`；`mark_failed` 自身失败合并为 `MarkFailed`。
3. **identity scope guard**：写 batch 前校验 Turn 的 campaign/conversation；sink attach 原子校验 scope；Attempt 缺失返回 Err，不得 `Ok(())`。
4. **harness evidence 不可伪造**：`production_postprocess_complete` 必须经 `verify_production_postprocess_claim` 对 durable Turn/Attempt 状态与 proof 校验；伪造 `chronicle_source` / 无 proof / applied=false / synthetic+proof 均 fail-closed；overall complete 仅当**全部**已 Accept 轮均为 verified production service。

**共享边界声明（诚实）：**

- 本服务共享的是 **outcome 写回状态机**（Attempt/derivation/batch/Chronicle A 候选、取消/迟到守卫、失败语义）。
- **Summarizer/PostProcessor LLM 调度仍由 command 层 `PipelineOrchestrator::run_postprocess` 执行**，不是完整共享 runner 编排。
- harness 确定性路径用 `FixedPostprocessRunner` / `apply_production_postprocess`，不调用真实模型。

未切换默认 backend、未改 M5 参数、未调用真实模型、未修改 `docs/HANDOFF.md` /
`docs/RELEASE-CHECKLIST.md` / `crates/infra-sqlite/**`。

## 共享范围

| 能力 | 共享入口 | 备注 |
| --- | --- | --- |
| 服务 API / 失败语义 | `ProductionPostprocessService` | 含 `ScopeMismatch` / `AttemptMissing` / `MarkFailed` |
| autofix 同步 | `sync_autofix_attempt(identity, …)` | 校验 scope + attempt 存在 |
| 迟到/取消/重复 apply | `apply_outcome` | 取消后不返回 summary/batch/outcome |
| DerivationComponents | `derive_components` | |
| JSON / Runtime MutationBatch | `build_json_mutation_batch` / `build_runtime_mutation_batch` | |
| 可注入 runner | `PostprocessRunner` / `FixedPostprocessRunner` | 仅 outcome 生产侧注入 |
| 可注入持久化 | `TurnAttemptSink` + scope-aware attach | `Json` / `Backend` |
| harness 入口 | `HarnessEnv::apply_production_postprocess` | 返回 verified `ProductionPostprocessProof` |
| evidence 证明 | `verify_production_postprocess_claim` | fail-closed |

## 明确未共享 / 未覆盖

1. QualityGate + Editor autofix 编排仍在 command 层（仅 hash 同步走服务）。
2. 真实 Summarizer/PostProcessor runner 编排仍在 command/`PipelineOrchestrator`。
3. Accept / revision CAS 仍由 `TurnLifecycleService` 负责。
4. Accept 后向量索引 / Chronicle 压缩调度仍在 Tauri command。
5. SQLite opt-in 仅复用 runtime batch builder + Backend sink；未跑 sqlite 生命周期集成。
6. 真实模型时序竞态（多 LLM 并行 + 用户取消）未在本线实跑。
7. `PipelineProductionTurnWriter` 默认仍 synthetic，避免未跑真实 Summarizer 误报 complete。
8. 未重跑 M5 100-turn endurance / 付费 real-LLM。

## 契约测试

| 契约 | 测试 |
| --- | --- |
| 成功路径 identity/scope/candidates | `happy_path_aligns_identity_scope_and_candidates` |
| 迟到 / 取消不写回 | `late_or_cancelled_results_do_not_write_current_turn` |
| runner 完成后取消丢弃 outcome | `cancel_after_runner_discards_outcome_and_does_not_await_acceptance` |
| 跨 campaign/conversation/同 revision 越界零写入 | `scope_mismatch_campaign_or_conversation_writes_nothing` |
| Attempt 缺失 Err | `missing_attempt_returns_error_not_ok` |
| 存储失败传播 + mark_failed 组合 | `storage_attach_failure_propagates_and_fail_turn_combines_mark_errors` |
| 降级 derivation | `agent_degrade_still_finishes_attempt_with_explicit_derivation` |
| autofix hash | `autofix_sync_aligns_text_and_draft_hash` |
| 重复 apply 幂等 | `repeat_apply_is_idempotent_without_duplicate_write` |
| harness 共享服务 evidence | `multi_turn_loop_uses_shared_production_postprocess_service` |
| 伪造 production claim fail-closed | `forged_production_postprocess_claim_without_proof_is_rejected` |

## 测试原始结果（返修后）

```text
cargo fmt --all -- --check
  PASS

cargo test -p storyforge --lib
  PASS: 264 passed; 0 failed; 3 ignored

cargo test -p storyforge-app-pipeline --lib
  (covered by clippy -D warnings workspace subset; lib suite previously 87)

cargo test -p storyforge-app-agent --lib
  (covered by clippy -D warnings workspace subset; lib suite previously 110)

cargo test -p harness-real-llm --lib
  PASS: 61 passed

cargo test -p harness-real-llm --test m5_production_evidence
  PASS: 10 passed
  (含 shared service + forged claim)

cargo clippy -p storyforge -p storyforge-app-pipeline -p storyforge-app-agent -p harness-real-llm --all-targets -- -D warnings
  PASS

git diff --check
  PASS（仅 CRLF 提示）
```

未运行：全 workspace、真实/付费 LLM、GUI/Android、M5 100-turn endurance、SQLite 集成。

## 修改文件

- `crates/tauri-app/src/production_postprocess.rs` — 服务、scope/cancel/failure 契约
- `crates/tauri-app/src/lib.rs` — 取消源、autofix 共享同步、失败传播、事件
- `crates/harness-real-llm/src/lib.rs` — verified proof + 真实 input_node_id
- `crates/harness-real-llm/src/production_evidence.rs` — fail-closed verification / overall complete
- `crates/harness-real-llm/tests/m5_production_evidence.rs` — 伪造 claim 与共享路径回归
- `docs/workstreams/PRODUCTION-POSTPROCESS-SERVICE-RESULT.md` — 本结果

## 是否建议合并

**在 P0 返修合入后建议合并。** 默认行为与 backend 未改；取消、失败传播、scope 与 evidence 不可伪造已有契约/回归。剩余是真实模型时序与 SQLite/M5 evidence 重跑，不阻塞本线合并。

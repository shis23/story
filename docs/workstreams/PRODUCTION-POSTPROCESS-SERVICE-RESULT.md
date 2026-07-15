# ProductionPostprocessService Result

- 分支：`codex/production-postprocess-service`
- 基线：`23ad240`（plan）
- 当前 HEAD：见 commit 列表
- 工作目录：`C:\tmp\storyforge-production-postprocess`
- 日期：2026-07-15

## 结论

共享 `ProductionPostprocessService` 已落地，并完成三轮 P0 返修：

### 共享写回状态机
- Attempt/derivation/MutationBatch/Chronicle A 候选
- cancel / 迟到 / 重复 apply 守卫
- autofix `draft_hash` 同步
- agent 降级 vs 关键存储失败语义

### P0 返修（取消 / 失败 / scope / evidence）
1. **operation-owned cancel**
   `begin_writing_operation` 生成 `operation_id + cancel_rx`；pipeline / autofix / postprocess 只 clone 该 receiver。
   删除 `watch::channel(false).1` fallback 与“事后从全局 slot 再 subscribe”。
   `clear_current_cancel_if(operation_id)` compare-and-clear，A 结束不会清掉 B。
2. **identity validation 零写入（typed + 原子）**
   `TurnAttemptSink::sync_autofix` 在**同一次** durable `mutate_if` 内校验
   `campaign_id` / `conversation_id` / `attempt_id` / 可写状态。
   typed `ScopeMismatch` / `AttemptMissing` 原样向上返回，**不得** stringify 成
   `AutofixSync` 后再被 `service_fail_turn` / `fail_turn_or_combine` 调 `mark_failed`。
   并发 supersede / 非可写状态：零写入、非致命 no-op。
3. **harness evidence 不可伪造**
   `ProductionPostprocessProof` 绑定 `input_node_id` / `turn_index` / `variant_id` /
   `draft_hash` / `batch_digest` / 精确 summary。
   `verify_production_postprocess_claim` 接收 evidence loop **当前**
   `input_node_id` + `turn_index`，与 proof/Turn/MutationBatch 精确比较；
   禁止复用不同 input/turn 的有效旧 proof。
   overall `production_postprocess_complete=true` 仅当**全部** accepted turn 均 verified production service。

**共享边界（诚实）：**
本服务共享 **outcome 写回**，不共享完整 Summarizer/PostProcessor LLM runner 编排。command 层仍调用 `PipelineOrchestrator::run_postprocess`。

未改默认 backend / M5 参数 / HANDOFF / RELEASE-CHECKLIST / infra-sqlite；未调用真实模型；未 merge/push。

## 契约与回归测试

| 项 | 测试 |
| --- | --- |
| 成功路径 | `happy_path_aligns_identity_scope_and_candidates` |
| 迟到/取消 | `late_or_cancelled_results_do_not_write_current_turn` |
| runner 后取消丢弃 | `cancel_after_runner_discards_outcome_and_does_not_await_acceptance` |
| 跨 scope 零写入 | `scope_mismatch_campaign_or_conversation_writes_nothing` |
| Attempt 缺失 typed Err | `missing_attempt_returns_error_not_ok` |
| sync_autofix typed + supersede | `sync_autofix_preserves_typed_scope_and_attempt_errors` |
| storage + mark 组合 | `storage_attach_failure_propagates_and_fail_turn_combines_mark_errors` |
| operation-owned 交错取消 | `operation_owned_cancel_interleaving_preserves_active_generation` |
| adapter zero-write 回归 | `scope_validation_errors_do_not_mark_turn_failed` |
| harness 共享路径 | `multi_turn_loop_uses_shared_production_postprocess_service` |
| 伪造 proof fail-closed | `forged_production_postprocess_claim_without_proof_is_rejected` |
| 恶意复用旧 proof | `malicious_writer_cannot_reuse_prior_turn_proof` |

## 测试原始结果

```text
cargo fmt --all -- --check
  PASS

cargo test -p storyforge --lib
  PASS: 267 passed; 3 ignored

cargo test -p storyforge --lib production_postprocess
  PASS: 10 passed

cargo test -p storyforge --lib scope_validation_errors_do_not_mark_turn_failed
  PASS

cargo test -p harness-real-llm --lib
  PASS: 61 passed

cargo test -p harness-real-llm --test m5_production_evidence
  PASS: 11 passed

cargo clippy -p storyforge -p storyforge-app-pipeline -p storyforge-app-agent -p harness-real-llm --all-targets -- -D warnings
  PASS

git diff --check
  PASS
```

未运行：真实/付费 LLM、GUI/Android、M5 100-turn endurance、SQLite 生命周期集成。

## 修改文件

- `crates/tauri-app/src/production_postprocess.rs`
- `crates/tauri-app/src/lib.rs`（Backend atomic sync_autofix、adapter 回归）
- `crates/harness-real-llm/src/production_evidence.rs`
- `crates/harness-real-llm/tests/m5_production_evidence.rs`
- `docs/workstreams/PRODUCTION-POSTPROCESS-SERVICE-RESULT.md`

## 仍未覆盖边界

1. 真实模型 Summarizer/PostProcessor + 用户取消的端到端时序
2. SQLite opt-in 集成（仅 runtime batch + Backend sink）
3. QualityGate autofix 编排仍在 command 层
4. 更广磁盘故障矩阵
5. real-LLM suite 默认仍 synthetic（防误报 complete）

## 是否建议合并

**建议合并（typed identity + 原子 autofix + evidence loop 绑定后）。**
默认行为未切 backend；关键取消/失败/scope/evidence 语义已有契约测试。

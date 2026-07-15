# ProductionPostprocessService Result

- 分支：`codex/production-postprocess-service`
- 基线：`23ad240`（plan commit）/ 上游 `8732e21`
- 工作目录：`C:\tmp\storyforge-production-postprocess`
- 日期：2026-07-15

## 结论

完成共享 `ProductionPostprocessService` 垂直切片：Summarizer/PostProcessor 结果校验、
`draft_hash` / quality 同步、Attempt 写回、Chronicle A 候选构建、取消/迟到结果守卫与失败传播
已收敛到 `crates/tauri-app/src/production_postprocess.rs`。

- Tauri `start_writing` / `regenerate` 改为薄适配：命令层保留 DTO、事件、后台 task 与
  `PipelineOrchestrator::run_postprocess` 调用；Attempt 状态机与 MutationBatch 构造走共享服务。
- Harness 可通过 `HarnessEnv::apply_production_postprocess` 与
  `FixedProductionPostprocessWriter` 走同一服务，不必再用 `synthetic_chronicle_fixture`
  冒充生产后处理。
- 未切换默认 backend、未改 M5 参数、未调用真实模型、未修改 `docs/HANDOFF.md` /
  `docs/RELEASE-CHECKLIST.md` / `crates/infra-sqlite/**`。

## 共享范围

| 能力 | 共享入口 | 原路径 |
| --- | --- | --- |
| 服务 API / 失败语义 | `ProductionPostprocessService` | Tauri command 内联 |
| autofix 后 Attempt 同步 | `sync_autofix_attempt` + `TurnAttemptSink` | command 内 `update_turn_record` |
| 迟到/取消/重复 apply 守卫 | `apply_outcome` | command 内 `update_turn_record_if` |
| DerivationComponents | `derive_components` | `derive_components_from_outcome` |
| JSON MutationBatch / Chronicle A | `build_json_mutation_batch` | `build_mutation_batch` |
| Runtime MutationBatch（SQLite opt-in） | `build_runtime_mutation_batch` | `build_mutation_batch_from_runtime` |
| 可注入 runner | `PostprocessRunner` / `FixedPostprocessRunner` | 无 |
| 可注入持久化 | `TurnAttemptSink` / `JsonTurnAttemptSink` / `BackendTurnAttemptSink` | 全局 store 直写 |
| harness 入口 | `HarnessEnv::apply_production_postprocess` | `synthetic_chronicle_fixture` |

## 仍未共享 / 明确边界

1. **QualityGate + 1× Editor auto-fix 编排**仍在 Tauri command
   （`quality_gate_with_optional_editor_autofix`）。共享层只强制 autofix 后的 hash/report 同步。
2. **Summarizer/PostProcessor 真实 LLM 调度**仍通过 `PipelineOrchestrator::run_postprocess`
   在 command 层执行；服务只消费 outcome。harness 确定性路径用 `FixedPostprocessRunner`。
3. **Accept / revision CAS / Final** 仍由 `TurnLifecycleService` 负责（前一线已完成）。
4. **Accept 后远记忆向量索引 / Chronicle 压缩调度**仍在 Tauri command 侧 best-effort。
5. **非 Campaign 旧路径**仍可直接 `persist_postprocess_outcome_async`。
6. **M5 real-LLM suite**（`PipelineProductionTurnWriter`）默认仍标记
   `synthetic_chronicle_fixture`，避免把“未跑真实 Summarizer”误报为生产后处理完成。
   新确定性路径用 `production_postprocess_service` 且 `production_postprocess_complete=true`。
7. **未重跑 M5 real evidence / 100-turn endurance**；本线只证明 deterministic 共享路径可用。

## 契约测试（TDD）

| 契约 | 测试 |
| --- | --- |
| 成功路径：正文/quality/Attempt/Chronicle A/变量候选身份与 scope 一致 | `happy_path_aligns_identity_scope_and_candidates` |
| 旧 Attempt / 取消不能写回当前 Turn | `late_or_cancelled_results_do_not_write_current_turn` |
| Summarizer/PostProcessor 降级时 Attempt 终态与 derivation 明确 | `agent_degrade_still_finishes_attempt_with_explicit_derivation` |
| auto-fix 后文本与 `draft_hash` 一致 | `autofix_sync_aligns_text_and_draft_hash` |
| 重复 apply 不产生重复 Chronicle/Mutation 写回 | `repeat_apply_is_idempotent_without_duplicate_write` |
| harness 经生产服务完成 postprocess | `multi_turn_loop_uses_shared_production_postprocess_service` |

## 测试原始结果

```text
cargo fmt --all -- --check
  PASS

cargo test -p storyforge --lib
  PASS: 260 passed; 0 failed; 3 ignored

cargo test -p storyforge-app-pipeline --lib
  PASS: 87 passed

cargo test -p storyforge-app-agent --lib
  PASS: 110 passed

cargo test -p harness-real-llm --lib
  PASS: 61 passed

cargo test -p harness-real-llm --test m5_production_evidence
  PASS: 9 passed
  (含 multi_turn_loop_uses_shared_production_postprocess_service)

cargo clippy -p storyforge -p storyforge-app-pipeline -p storyforge-app-agent -p harness-real-llm --all-targets -- -D warnings
  PASS

git diff --check
  PASS（仅 CRLF 提示）
```

未运行：全 workspace、真实/付费 LLM、GUI/Android、M5 100-turn endurance。

## 修改文件

- `crates/tauri-app/src/production_postprocess.rs` — 新增共享服务与契约测试
- `crates/tauri-app/src/lib.rs` — start_writing/regenerate 薄适配；batch 构造委托共享服务
- `crates/harness-real-llm/src/lib.rs` — `apply_production_postprocess`
- `crates/harness-real-llm/src/production_evidence.rs` — production postprocess writer/path 字段
- `crates/harness-real-llm/tests/m5_production_evidence.rs` — 共享路径确定性回归
- `docs/workstreams/PRODUCTION-POSTPROCESS-SERVICE-RESULT.md` — 本结果

## 仍未覆盖边界 / 风险

1. 真实 Summarizer + PostProcessor + Tauri 后台 spawn 的端到端实时序（取消竞态）未用真实模型验证。
2. 关键存储失败（磁盘满/锁损坏）在 harness 中未注入；仅有 API 错误传播路径。
3. SQLite opt-in 路径只复用 runtime batch builder；未在本线跑 sqlite 生命周期集成。
4. 旧 synthetic fixture 路径仍保留以兼容历史 evidence 断言；新路径需主动选择。
5. Quality autofix 仍在 command 层，可能与共享 postprocess 的时序继续漂移。

## 是否建议合并

**建议合并。** 默认后端与生产参数未改；Tauri 与 harness 已共享 Attempt/Chronicle 后处理状态机，
且有契约测试 + deterministic evidence 回归。合并后可继续用该服务替换 real-LLM suite 中的
synthetic Chronicle，但那是证据重跑工作，不属于本线。

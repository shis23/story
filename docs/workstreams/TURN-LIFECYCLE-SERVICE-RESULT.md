# Turn Lifecycle Shared Service Result

- 分支：`codex/turn-lifecycle-service`
- 基线：`main@c3a972d`
- 工作目录：`C:\tmp\storyforge-turn-lifecycle`
- 日期：2026-07-13

## 结论

完成共享 Turn Lifecycle 垂直切片：生产 Accept / Attempt 状态机 / autofix hash 同步 /
regenerate supersede / 启动恢复 已收敛到 `crates/tauri-app/src/turn_lifecycle.rs`。

- Tauri `commit_turn_attempt`、`recover_turns_on_startup`、`start_writing` / `regenerate`
  的 Attempt 构造与 postprocess 写回，改为调用同一服务/helper。
- Harness `CommitProbeEnv::accept_production` 不再镜像状态机，而是直接调用
  `TurnLifecycleService::accept_by_variant`。
- 未切换 SQLite 后端，未改 UI / Android / 模型默认 / `200/4/H/E`，未编辑 `docs/HANDOFF.md`。

## 移动到生产共享路径的逻辑

| 能力 | 共享入口 | 原路径 |
| --- | --- | --- |
| SHA-256 `draft_hash` | `turn_lifecycle::compute_draft_hash` | `tauri-app` 私有 + harness `draft_hash_hex`（hash 算法仍一致） |
| autofix 后 Attempt 同步 | `sync_attempt_after_autofix` | `tauri-app` 私有 helper |
| regenerate supersede + 新 Attempt | `append_regenerate_attempt` / `new_draft_attempt` | `start_writing` / `regenerate` 内联 |
| 迟到 postprocess 守卫 | `is_current_attempt_ready_for_postprocess` / `apply_postprocess_to_attempt` | `tauri-app` 私有 |
| Chronicle A 序号 | `next_chronicle_a_seq` | Tauri + CommitProbe 双份 |
| Accept / force Degraded / CAS / MutationBatch | `TurnLifecycleService::accept_by_variant` | `commit_turn_attempt` + `CommitProbe::accept_production` |
| 启动恢复 | `TurnLifecycleService::recover_turns_on_startup` | `recover_turns_on_startup` |

## 仍无法共享 / 仍残留的镜像

1. **Tauri 后台 postprocess spawn 编排**
   `start_writing` 仍在 command 内 `tokio::spawn` 跑 Summarizer/PostProcessor，并依赖
   `AppState`、全局 store、prompt hook、前端 Channel。服务层只拥有 Attempt 写回契约，不拥有
   后台任务调度本身。
2. **QualityGate + 1× Editor auto-fix 编排**
   仍在 `tauri-app` 的 `quality_gate_with_optional_editor_autofix`（依赖 `PipelineOrchestrator`）。
   共享层只强制 autofix 后的 hash/report 同步。
3. **`build_mutation_batch` 与知识/变量/任务 normalize**
   仍在 `tauri-app`（依赖 `CampaignStore` 查询与 postprocess DTO）。共享 Accept 只消费已落盘的
   `pending_state_changes`。
4. **Accept 成功后的远记忆向量索引 / Chronicle 压缩调度**
   仍由 Tauri command 在共享 Accept 返回后 best-effort 触发；CommitProbe 不跑这两步。
5. **Harness `draft_hash_hex`**
   仍保留在 `evidence.rs` 作证据字段 helper；算法与共享 `compute_draft_hash` 相同，但未强制
   删除以免波及证据 JSON 路径。
6. **SQLite `SqliteProductionRepository::accept_turn`**
   仍是默认关闭的并行事务实现，本线未接入 JSON 生产路径。

## 红测 → 绿测证据

共享契约测试落在 `turn_lifecycle::tests`：

| 契约 | 测试 |
| --- | --- |
| 稳定 SHA-256 hash | `compute_draft_hash_is_stable_sha256_hex` |
| autofix 必须同步 hash | `autofix_sync_updates_hash_to_final_text` / `failure_after_draft_cannot_accept_mismatched_hash_text` |
| 迟到 postprocess 不能复活旧 Attempt | `postprocess_race_guard_rejects_superseded_attempt` / `regenerate_supersedes_old_attempt_and_accepts_only_new` |
| Accept 提交 + revision bump + Chronicle A | `accept_happy_path_commits_and_bumps_revision` |
| Quality Error 拦截 | `accept_blocks_quality_errors_without_force` |
| force → Degraded | `accept_force_marks_degraded` |
| 编辑草稿 hash 失效 | `accept_rejects_edited_draft_hash_mismatch` |
| 重复 accept 无二次 revision | `duplicate_accept_is_rejected_after_commit` |
| 恢复：活动 Failed / finalize 失败保持 Committing | `recovery_fails_active_and_keeps_committing_when_finalize_missing` |

Harness 既有生产 Accept 探针在改为共享路径后仍绿：

- `production_accept_persists_chronicle_a_and_bumps_revision`
- `production_accept_blocks_on_quality_error_without_force`
- `production_accept_force_marks_degraded`

## Commit 列表

（相对基线 `c3a972d`，含既有 PLAN commit）

1. `0a31e65` docs(workstream): plan shared turn lifecycle service
2. `f0b06c4` feat(turn-lifecycle): add shared TurnLifecycleService and contract tests
3. `7cca144` refactor(tauri): thin turn lifecycle adapters over shared service
4. `33c4a66` refactor(harness): route CommitProbe accept through shared lifecycle
5. `274c539` docs(workstream): record turn lifecycle shared service result

## 修改文件

- `crates/tauri-app/src/turn_lifecycle.rs` — 新增共享服务与契约测试
- `crates/tauri-app/src/lib.rs` — command/recovery 薄适配；Attempt/postprocess helper 委托
- `crates/harness-real-llm/src/commit_probe.rs` — Accept 改调共享服务
- `docs/workstreams/TURN-LIFECYCLE-SERVICE-RESULT.md` — 本结果

## 实际测试结果（专项门禁）

```text
cargo fmt --all -- --check
cargo test -p storyforge --lib
cargo test -p storyforge-domain
cargo test -p storyforge-app-pipeline
cargo test -p storyforge-app-agent --lib
cargo test -p harness-real-llm --lib
cargo test -p harness-real-llm --test bronze_deterministic --test eval_m5_phaseb_deterministic
cargo clippy -p storyforge -p storyforge-app-pipeline -p storyforge-app-agent -p storyforge-domain -p harness-real-llm --all-targets -- -D warnings
git diff --check c3a972d..HEAD
```

结果：

| 命令 | 结果 |
| --- | --- |
| fmt check | PASS |
| `storyforge --lib` | PASS：225 passed, 3 ignored |
| `storyforge-domain` | PASS：243 passed |
| `storyforge-app-pipeline` | PASS：87 passed |
| `storyforge-app-agent --lib` | PASS：109 passed |
| `harness-real-llm --lib` | PASS：17 passed |
| bronze_deterministic | PASS：8 passed |
| eval_m5_phaseb_deterministic | PASS：5 passed |
| clippy `-D warnings`（受影响 crate） | PASS |
| `git diff --check c3a972d..HEAD` | PASS |

未运行：全 workspace、真实/付费 LLM、GUI。

## 未完成项与风险

1. postprocess / quality autofix 编排仍在 Tauri command，共享度尚未覆盖完整写作后台闭环。
2. `build_mutation_batch` 仍私有；若后续再抽服务，需一并迁移 knowledge normalize。
3. Accept 侧效应失败时服务保持 `Committing` 并返回错误；依赖启动恢复重试。与旧 Tauri
   路径一致，但 harness 失败断言文案可能因共享错误类型略有变化（Quality block 已保持旧文案）。
4. 全局 `OnceLock` store 与 per-test 隔离 store 并存；共享服务支持注入，但 command 路径仍走全局。

## 是否建议合并

**建议合并。** 默认行为未改，Accept/恢复/Attempt 状态机有契约测试与 Bronze/Phase B 确定性回归，
且 Tauri 与 harness 已走同一 Accept 实现。合并后可降低 `CommitProbe` 与私有 `commit_turn_attempt`
继续漂移的风险。

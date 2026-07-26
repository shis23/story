# SQLite Pre-Accept Lifecycle Result

- 分支：`codex/sqlite-preaccept-lifecycle`
- 基线：`6f24d55`（plan）/ 工作基于 `8732e21` 事实面
- 工作目录：`C:\tmp\storyforge-sqlite-preaccept`
- 日期：2026-07-15
- 默认 backend 仍为 **JSON**（本线交付时未切换、未双写）

> **Supersession（2026-07-21）**  
> 下文 “尚未接 production command” 仅描述 **本 RESULT 冻结当日** 的交付范围。  
> 之后 `main` 已通过 `88dd2c6` 等提交把 pre-accept UoW 接到 Tauri / `sqlite_runtime` / endurance adapter。  
> 当前接线与证据边界以  
> `docs/workstreams/SQLITE-CURRENT-STATUS-AUDIT-2026-07-21.md`  
> 与 `docs/HANDOFF.md` 为准。  
> **不要** 把本历史 RESULT 读成“现在仍未接 production”。

## 结论（先读）

**本线交付：Repository 就绪；当时尚未接 production command。**  
**当前 main（审计后）：production command / gateway 已接（见 supersession）。**

| 状态 | 含义（2026-07-15 本线） | 2026-07-21 main |
| --- | --- | --- |
| **Repository 就绪** | `infra-sqlite` 提供类型化 pre-accept UoW API；draft / Attempt 中间态 / autofix / postprocess / regenerate / edit-stale / recovery / 故障注入均可在单 `BEGIN IMMEDIATE` 事务内完成，且无 JSON/SQLite 双权威、无部分写入。 | 仍成立 |
| **已接 production command** | **本线未完成。** 未改 `tauri-app` 写作命令、启动选择器、默认 backend。 | **已完成接线**：`sqlite_runtime` + Tauri `start_writing`/autofix/postprocess/regenerate/edit + harness endurance；默认仍 JSON |

本线当时只交付可被后续 Tauri / postprocess 服务线接入的 **adapter contract + 测试证据**。

本线返修时仍不接 production command；后续 hardening 补齐 Committing 守卫，并删除 `sync_autofix` 中不可达的 Stale 重激活承诺。

## 交付范围对照

| 计划项 | 结果 |
| --- | --- |
| 审计 JSON pre-accept 状态转换并映射 SQLite | 见下方状态机映射 |
| 类型化 repository/UoW | `SqlitePreacceptRepository` |
| Migration 排序/幂等/fail-closed | V4 `preaccept_outbox`；runner 行为未放宽 |
| 适配接口 + 测试构造器 | 请求/结果类型 + fault 注入 API + integration tests |
| reverse export 新字段分类 | `preaccept_outbox` → `unsupported_fields`（显式，不静默丢失） |
| 不改 tauri-app / 默认 backend / HANDOFF / RELEASE-CHECKLIST | 遵守 |

## 状态机映射（JSON pre-accept → SQLite）

| 步骤 | JSON 路径（现状事实） | SQLite 权威写 | 事务边界 |
| --- | --- | --- | --- |
| 首稿 Draft + Attempt | `append_ai_draft` + `TurnStore` → DraftReady | `create_draft_attempt` | conversation node + turn/attempt + outbox |
| quality / autofix 写回 | 改 variant 文本 + `sync_attempt_after_autofix` | `sync_autofix` | conversation content + attempt `draft_hash`/`quality_report` + outbox |
| postprocess 候选态 | `apply_postprocess_to_attempt` + AwaitingAcceptance | `apply_postprocess` | turn/attempt only（**不写 Campaign revision / accept ledger**） |
| regenerate | soft-delete/add variant + supersede + 新 Attempt | `append_regenerate_attempt` | conversation variants + turn attempts + outbox |
| 用户编辑草稿 | edit variant + Attempt Stale（保留原 hash） | `mark_stale_after_edit` | conversation + attempt status + outbox |
| 启动恢复（pre-accept） | `TurnStore` 扫活动 Turn | `recover_active_state` / `fail_incomplete_preaccept` | 活动 Turn → Failed；pending outbox → failed |

权威源规则：

- 调用方显式传入 `&mut Database`；本模块 **不** 打开/写入任何 JSON store。
- pre-accept **绝不** bump Campaign revision，也 **不** 写 `mutation_commits`。
- postprocess 只落候选 `MutationBatch` 到 Attempt；Accept 仍由 `SqliteProductionRepository::accept_turn` 负责。

## API

新增 `crates/infra-sqlite/src/preaccept.rs`：

- `SqlitePreacceptRepository::create_draft_attempt` / `_with_fault`
- `sync_autofix` / `_with_fault`
- `apply_postprocess` / `_with_fault` → `PostprocessApplyOutcome::{Applied, AlreadyApplied, SkippedLate}`
- `append_regenerate_attempt` / `_with_fault`（`previous_variant_id` 必须等于当前 active Attempt 的 variant）
- `mark_stale_after_edit`
- `list_outbox_for_turn`
- `recover_active_state`
- `fail_incomplete_preaccept`
- 类型：`DraftAttemptRequest` / `DraftAttemptOutcome` / `AutofixSyncRequest` /
  `PostprocessApplyRequest` / `PostprocessApplyOutcome` / `RegenerateAttemptRequest` /
  `PreacceptOutboxRow` / `PreacceptOutboxKind` / `PreacceptOutboxStatus` /
  `PreacceptRecoverySnapshot` / `PreacceptFault`

### Pre-accept 契约 hardening（返修）

| 契约 | 行为 |
| --- | --- |
| postprocess ownership | `attempt_id` 必须属于目标 `turn_id`；跨 Turn → `Conflict`，**零 outbox 写入** |
| postprocess replay | 同 payload → `AlreadyApplied`，不新增 Skipped/outbox；不同 payload → conflict |
| regenerate variant | `previous_variant_id` 必须等于当前 active Attempt 的 variant，不能是会话内任意节点 |
| regenerate/edit during Committing | Turn 或 active Attempt 为 `Committing` → `Conflict`，**零写入** conversation/turn/attempt/outbox |
| autofix fingerprint | 幂等指纹覆盖完整 canonical `QualityReport`（不是只比 `error_count`） |
| autofix vs Stale | `Stale` 为 terminal，不能成为 active；`sync_autofix` 不重激活 Stale（删除不可达分支） |

`crates/infra-sqlite/src/lib.rs` 已 re-export。

### 故障注入

`PreacceptFault::{None, BeforeCommit, AfterConversation}` 用于验证：

- 事务中途失败 → conversation / turn / attempt / outbox 全量回滚；
- 无孤立 draft node、无错误 active Attempt、无半截 outbox。

## Schema / migration

新增单向 migration：

| Version | Name | Purpose |
| --- | --- | --- |
| V1 | `init_schema_v1` | 不变 |
| V2 | `production_commit_ledger` | 不变 |
| V3 | `chronicle_publication_jobs` | 不变 |
| **V4** | **`preaccept_lifecycle`** | `preaccept_outbox` + attempt status index |

`preaccept_outbox` 字段：

- `outbox_id` PK
- `campaign_id` / `conversation_id` / `turn_id` / `attempt_id` FK
- `kind`：`draft_ready` / `autofix_sync` / `postprocess_apply` / `regenerate` / `edit_stale` / `recovery_fail`
- `draft_hash` / `payload_hash` / `status` / `payload_json`
- `created_at` / `updated_at`
- 索引：turn/status、campaign/status、attempt
- partial unique：`(attempt_id, kind) WHERE status = 'pending'`

schema 当前版本：`4`。

## Reverse export

`export_sqlite_to_json` 对 `preaccept_outbox`：

- **不** 静默丢弃；
- 写入 `unsupported_fields`，标明 SQLite-native recovery ledger 在 JSON 后端无对等文件；
- Turn/Conversation 业务 payload 仍经 `turns.json` / `conversations/` 导出（候选 batch 在 attempt payload 内）。

## 测试证据

`crates/infra-sqlite/tests/preaccept_lifecycle.rs`（20）：

| 契约 | 测试 |
| --- | --- |
| draft 原子提交 | `create_draft_attempt_atomically_persists_conversation_and_attempt` |
| draft 故障回滚 | `create_draft_attempt_fault_rolls_back_conversation_turn_and_outbox` |
| scope fail-closed | `create_draft_rejects_campaign_conversation_scope_mismatch` |
| autofix 原子 + hash 一致 | `autofix_sync_rewrites_conversation_and_attempt_hash_atomically` |
| autofix 故障回滚 | `autofix_fault_before_commit_leaves_original_draft_intact` |
| autofix 完整 QualityReport 指纹 | `autofix_idempotency_fingerprint_covers_full_quality_report` |
| postprocess 原子 + 幂等 replay | `postprocess_apply_is_atomic_and_idempotent_on_replay` |
| postprocess 同 payload 不增 outbox | `postprocess_same_payload_replay_is_idempotent_without_extra_outbox` |
| 跨 Turn attempt fail-closed 零 outbox | `postprocess_rejects_attempt_owned_by_another_turn_with_zero_outbox` |
| 迟到 postprocess 跳过 | `postprocess_skips_when_attempt_no_longer_current` |
| regenerate 取代旧 Attempt | `regenerate_supersedes_previous_attempt_atomically` |
| regenerate 拒绝非 active variant | `regenerate_rejects_previous_variant_not_owned_by_active_attempt` |
| regenerate 拒绝 Committing 零写入 | `regenerate_rejects_committing_turn_with_zero_writes` |
| 编辑 Stale 保留原 hash | `mark_stale_after_edit_keeps_hash_and_content_consistent` |
| mark_stale 拒绝 Committing 零写入 | `mark_stale_rejects_committing_turn_or_attempt_with_zero_writes` |
| autofix 不重激活 Stale | `sync_autofix_rejects_stale_attempt_instead_of_dead_reactivation` |
| recovery / fail incomplete | `recovery_lists_active_preaccept_state_and_fail_incomplete_is_atomic` |
| 并发 draft 串行 | `concurrent_draft_create_serializes_to_single_active_attempt` |
| reverse export 分类 | `reverse_export_marks_preaccept_outbox_as_unsupported_without_silent_loss` |
| V1→V4 升级 | `schema_upgrades_to_v4_preaccept_outbox` |

既有 migration / cutover / production accept 测试已同步到 schema v4，且保持绿灯。

## 门禁结果

| 命令 | 结果 |
| --- | --- |
| `cargo fmt -p storyforge-infra-sqlite -- --check` | PASS（via `cargo fmt`） |
| `cargo test -p storyforge-infra-sqlite` | PASS（含 20 preaccept + 既有 suites） |
| `cargo clippy -p storyforge-infra-sqlite --all-targets -- -D warnings` | PASS |
| `git diff --check` | PASS |

未跑全 workspace；未调用真实模型；未 push / merge / rebase main。

说明：偶发 `importer::tests::concurrent_same_manifest_converges_to_one_completed_run` 在 Windows 高负载下可能因 `database is locked` 抖失败；重跑通过，与本线 preaccept 改动无逻辑耦合。

## 明确未做 / 留给集成线

1. **未** 把 `start_writing` / `regenerate` / autofix / 后台 postprocess 接到 `SqlitePreacceptRepository`。
2. **未** 改 `TurnLifecycleService`、`TurnStore`、`sqlite_runtime` 生产 wiring。
3. **未** 切换默认 backend，**未** 双写。
4. **未** 改 HANDOFF / RELEASE-CHECKLIST / GUI / Android / M5 runner。

建议后续集成顺序：

1. SQLite mode 下 `append_ai_draft` + 首 Attempt 改走 `create_draft_attempt`；
2. autofix 同步改走 `sync_autofix`；
3. 后台 postprocess 写回改走 `apply_postprocess`（保留 late-skip 语义）；
4. regenerate / edit 改走对应 API；
5. startup recovery 在 SQLite 权威时调用 `fail_incomplete_preaccept` 或与现有 `fail_incomplete_turns` 合并策略评审。

## 文件清单

- `crates/infra-sqlite/migrations/V004__preaccept_lifecycle.sql`（新）
- `crates/infra-sqlite/src/preaccept.rs`（新）
- `crates/infra-sqlite/tests/preaccept_lifecycle.rs`（新）
- `crates/infra-sqlite/src/{lib,migrations,production,exporter,backend}.rs`
- `crates/infra-sqlite/tests/{cutover,migration_concurrency,migration_readiness}.rs`
- `docs/workstreams/SQLITE-PREACCEPT-LIFECYCLE-RESULT.md`（本文件）

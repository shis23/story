# 夜间集成与复核结果

> 集成分支：`codex/integration-nightly-2026-07-13`
> 基线：`7fb1899`
> 收口 HEAD：`504a06a`
> 日期：2026-07-13
> 工作目录：`C:\tmp\storyforge-integration`

## 0. 角色与边界

本线为集成与复核线，只负责读取、审查、合并、完整验证，以及复核发现的阻断项修复。
未修改 `main`、未 rebase、未 force push、未修改 `docs/HANDOFF.md`。三条开发分支的代码均在
各自分支上产生，冲突只在本集成分支解决。未经授权未调用任何付费真实模型（0 次付费调用）。

## 1. 审查的分支

| 分支 | 相对基线 commits | RESULT 文档 | 结论 |
| --- | --- | --- | --- |
| `codex/release-bronze` | 6 | `RELEASE-BRONZE-RESULT.md` | **接收全部** |
| `codex/eval-m5-phaseb` | 7 | `EVAL-M5-PHASEB-RESULT.md` | **接收全部**，集成侧补阻断修复 |
| `codex/sqlite-migration-foundation` | 4 | `SQLITE-MIGRATION-FOUNDATION-RESULT.md` | **选择性接收全部基础 commit**，集成侧补 migration 排序 |

每条线均提供了：清晰 commit 列表、独立 RESULT、修改文件与边界、实际测试、未完成项与风险、
干净工作区。secret scan 已复核。

开发线合计：**6 + 7 + 4 = 17** 个 non-merge commit。

## 2. 接收 / 拒绝的 commit

### release-bronze（接收 6/6）

| Commit | 说明 |
| --- | --- |
| `d7fee81` | docs(workstream): plan Bronze release evidence |
| `9602476` | fix(phase-b): 同名碰撞时禁止把角色名误判为 id 路 |
| `7609558` | test(release-bronze): 桌面 Bronze 确定性证据与 smoke 入口 |
| `e3b78e1` | style(release-bronze): fmt bronze evidence tests and manual template |
| `c6ba98b` | fix(release-bronze): clippy needless borrow in bronze_deterministic |
| `0dad20e` | docs(release-bronze): record Bronze automated evidence and RESULT |

### eval-m5-phaseb（接收 7/7）

| Commit | 说明 |
| --- | --- |
| `01210bf` | docs(workstream): plan M5 and Phase B evaluation |
| `beb7221` | feat(eval): add redacted JSONL evidence writer for M5/Phase B |
| `4e9d4bd` | feat(eval): production-faithful CommitTurn Accept probe |
| `863e99e` | feat(eval): deterministic long-session runner across H_anchor+E |
| `995d686` | feat(eval): Phase B A/B matrix fixtures with shared seed |
| `27c3d71` | test(eval): wire deterministic gates and ignored real-LLM eval suite |
| `c180d40` | docs(workstream): record EVAL-M5-PHASEB-RESULT |

### sqlite-migration-foundation（接收 4/4）

| Commit | 说明 |
| --- | --- |
| `a37206e` | docs(workstream): plan SQLite migration foundation |
| `0d49e08` | docs(adr): SQLite migration foundation decisions |
| `48c5f29` | feat(infra-sqlite): SQLite foundation crate with migrations and importer |
| `5b70495` | docs(workstream): SQLite migration foundation result |

**未拒绝任何 commit。** SQLite 线经审计满足选择性合并条件（见 §4）。

### 集成者新增 commit

| Commit | 说明 |
| --- | --- |
| `6e81e97` | merge(release-bronze) |
| `9b6853e` | merge(eval-m5-phaseb) |
| `947135a` | chore(integration): 移除 release+eval 合并后重复的 `sha2` 依赖 |
| `496d717` | merge(sqlite-foundation) |
| `6629a09` | style(integration): 清理 eval RESULT 的 markdown 硬换行尾随空格 |
| `e7a1425` | docs(workstream): 初版 nightly integration RESULT |
| `669d2c9` | fix(infra-sqlite): migration 按 version 排序 + 拒绝非法集合 |
| `922d018` | feat(eval): BudgetedLlmClient 真正限制 max_calls / timeout |
| `799ea2f` | fix(eval): fixture 缺失失败；诚实 H/E ContextEpoch；真实 usage 落盘 |
| `0177d0e` | docs(workstream): 更新 RESULT（HEAD/计数/阻断修复） |

## 3. 冲突处理记录

| 重叠文件 | 涉及分支 | 处理 |
| --- | --- | --- |
| `Cargo.lock` | 三条全有 | ort 自动合并；未手工拼接 |
| `crates/harness-real-llm/Cargo.toml` | release + eval | 合并后重复 `sha2`，独立 commit 去重，保留 `sha2` + `chrono` |
| `Cargo.toml`（workspace root） | 仅 sqlite | 无冲突 |

`git diff --check 7fb1899..HEAD` 初次因 eval RESULT markdown 硬换行双空格失败；已清理。

## 4. SQLite 选择性合并判定

- 新增 crate `storyforge-infra-sqlite` **未接入** `tauri-app` / `harness-real-llm` / 生产 Store。
- 默认生产后端仍为 JSON；无 `storage.backend=sqlite`；无双真相源。
- 未修改任何 `crates/app-*` / `domain` / 生产 `infra-*` Store 实现。

**满足选择性合并条件 → 接收全部基础 commit。** 未合并的 SQL commit：无。

## 5. 复核阻断项与修复批次

初版 RESULT 在静态门禁通过后，复核发现 eval 真实入口与 migration 的阻断问题。本线已在集成分支修复：

| 级别 | 问题 | 修复 |
| --- | --- | --- |
| P0 | `MAX_CALLS` / `TIMEOUT_SECS` 只读不生效 | 新增 `BudgetedLlmClient`：原子计数 + `tokio::timeout`；超限返回 `LlmError::Internal` / `Timeout` |
| P0 | fixture 缺失时零调用假通过 | `eval_real_llm_single_turn_evidence` 在 fixture 缺失时 **panic Inconclusive**；断言 `calls_used >= 1` |
| P1 | 跨 H+E 测试未跑 ContextCompiler | `long_session` 接入 `compute_epoch_membership` / `refresh_context_epoch`；断言 near_raw 截断与 early turn 挤出 |
| P1 | 真实 usage JSONL 占位 0 | 单轮真实测试写入 `BudgetedLlmClient` 录制的 prompt/cached/completion + segment hash |
| P1 | migration 不按 version 排序 | `migrate_with` 复制后升序；拒绝非正/重复 version；补测试 |
| P1 | 确定性骨架混在真实 suite | 从 `eval_m5_phaseb_real_llm` 移除 long/phase-b skeleton；保留 deterministic 测试 |

## 6. 完整发布门禁结果（本机实测，修复后）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 格式 | `cargo fmt --all -- --check` | **PASS** |
| 空白检查 | `git diff --check 7fb1899..HEAD` | **PASS** |
| 严格 lint | `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** |
| workspace 测试 | `cargo test --workspace` | **PASS — 985 passed / 0 failed / 24 ignored** |
| 前端 Node 测试 | `npm test` | **PASS — 218 passed / 0 failed** |
| 前端生产构建 | `npm run build` | **PASS — built in ~1.9s** |
| secret scan | 7fb1899..HEAD 新增行 | **PASS — 0 真实密钥** |
| Bronze smoke | `scripts/run-bronze-smoke.ps1` | **PASS**（4/4） |
| SQLite 专项 | `cargo test -p storyforge-infra-sqlite` | **PASS — 18 passed**（含排序/非法 version） |

### SQLite 专项覆盖

| 计划要求 | 覆盖测试 |
| --- | --- |
| migration 首次/重复 | `migrate_applies_v1_and_is_idempotent` |
| migration 乱序输入 | `migrate_with_sorts_by_version_regardless_of_input_order` |
| 非法 version | `migrate_with_rejects_duplicate_and_non_positive_versions` |
| importer 重跑 | `import_is_idempotent_across_two_runs` |
| transaction rollback | `rollback_discards_writes` 等 |
| 默认 JSON 回归 | workspace 985；sqlite 未接线生产路径 |

### Diff 统计（7fb1899..HEAD）

- **37 files** changed
- **+7086 / -36**

## 7. 未合并 SQL commit 及原因

**无。**

## 8. 未完成项与风险

1. **真实模型证据仍缺失（跨线）**：0 次付费调用。探针/预算/入口已 fail-closed，但真实 ≥20 写作循环、Phase B 真实 A/B 成本对照、Bronze GUI 截图仍未跑。**不得宣称**参数标定或 Bronze 完全验收。
2. **真实 ≥20 写作循环尚未实现**：已从 real suite 移除假 skeleton；完整真实循环需单独授权与更高预算。
3. **CommitTurn 探针与 Tauri 私有路径漂移风险**仍在。
4. **SQLite 接线债务**：UoW/migration/importer 就绪；生产 accept 仍走 JSON；Android/Windows 文件锁随接线再验。
5. **前端 dist/node_modules 仍 gitignore**：发布机需 `npm ci && npm run build`。

## 9. 推荐的 main 合并 / push 操作

**在修复批次之后，建议将 `codex/integration-nightly-2026-07-13` 合入 `main`。**

理由：

- 三条开发线 17 个 commit 全部接收。
- 复核阻断项（预算不生效、零调用假通过、H/E 不诚实、usage 占位、migration 乱序）已在集成分支修复。
- 完整门禁通过：fmt / diff-check / clippy(-D warnings) / workspace 985 / 前端 218+build / secret scan / SQLite 18 / Bronze smoke。
- 默认生产行为未变（JSON backend；SQLite crate 未启用）。
- 无真实密钥入仓。

### 合并前仍应明确（release notes 降级项，非阻塞基建合并）

- GUI / 真实模型条目仍属发布阻塞或显式降级项。
- M5 完整验收 / Phase B 真实成本对照未完成。

### 操作（由拥有 main 权限者执行；本线不自行 push main）

```text
git checkout main
git merge --no-ff codex/integration-nightly-2026-07-13 \
  -m "merge(integration): nightly 2026-07-13 release-bronze + eval-m5-phaseb + sqlite-foundation + eval/sqlite fix batch"
git push origin main
```

# 夜间集成与复核结果

> 集成分支：`codex/integration-nightly-2026-07-13`
> 基线：`7fb1899`
> 收口 HEAD：`6629a09`
> 日期：2026-07-13
> 工作目录：`C:\tmp\storyforge-integration`

## 0. 角色与边界

本线为集成与复核线，只负责读取、审查、合并和完整验证，不开发新功能。未修改 `main`、未
rebase、未 force push、未修改 `docs/HANDOFF.md`。三条开发分支的代码均在各自分支上产生，
冲突只在本集成分支解决。未经授权未调用任何付费真实模型（0 次付费调用）。

## 1. 审查的分支

| 分支 | 相对基线 commits | RESULT 文档 | 结论 |
| --- | --- | --- | --- |
| `codex/release-bronze` | 5 | `RELEASE-BRONZE-RESULT.md` | **接收全部** |
| `codex/eval-m5-phaseb` | 7 | `EVAL-M5-PHASEB-RESULT.md` | **接收全部** |
| `codex/sqlite-migration-foundation` | 4 | `SQLITE-MIGRATION-FOUNDATION-RESULT.md` | **选择性接收全部基础 commit** |

每条线均提供了：清晰 commit 列表、独立 RESULT、修改文件与边界、实际测试、未完成项与风险、
干净工作区，且不含 API key / 完整 secret / 原始 prompt / 用户数据（secret scan 已复核）。

## 2. 接收 / 拒绝的 commit

### release-bronze（接收 5/5）

| Commit | 说明 |
| --- | --- |
| `d7fee81` | docs(workstream): plan Bronze release evidence |
| `9602476` | fix(phase-b): 同名碰撞时禁止把角色名误判为 id 路（高风险写回隔离 bug 修复 + 回归） |
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

**未拒绝任何 commit。** SQLite 线经审计满足选择性合并条件（见 §4），全部基础 commit 已接收。
未发现任何分支包含默认后端切换或大规模 Store 重写，故无需保留独立分支。

### 集成者新增 commit（仅集成线）

| Commit | 说明 |
| --- | --- |
| `6e81e97` | merge(release-bronze) |
| `9b6853e` | merge(eval-m5-phaseb) |
| `947135a` | chore(integration): 移除 release+eval 合并后重复的 `sha2` 依赖 |
| `496d717` | merge(sqlite-foundation) |
| `6629a09` | style(integration): 清理 eval RESULT 的 markdown 硬换行尾随空格（满足 `git diff --check`） |

## 3. 冲突处理记录

三条分支相对基线的文件重叠分析：

| 重叠文件 | 涉及分支 | 处理 |
| --- | --- | --- |
| `Cargo.lock` | 三条全有 | ort 策略自动合并；未手工拼接。合并后随构建自然更新 |
| `crates/harness-real-llm/Cargo.toml` | release + eval | 两线各加 `sha2.workspace`（release 在 `serde_json` 后；eval 在 `uuid` 后）。ort 自动合并后产生**重复 `sha2` 行**，集成者以独立 commit 删除重复项，保留 `sha2` + `chrono` |
| `Cargo.toml`（workspace root） | 仅 sqlite | 无冲突 |

**没有为解决冲突而删除任一分支的测试或错误处理。** `Cargo.lock` 按计划要求重新生成（随
clippy/test 构建自动完成），未手工拼接。

`git diff --check 7fb1899..HEAD` 初次失败：eval 分支的 RESULT 文档含 17 处 markdown 硬换行
双空格尾随。集成者以独立 commit 清理（纯文档格式，非删除测试/逻辑），使该门禁通过。

## 4. SQLite 选择性合并判定

按计划：SQLite 若含默认 backend 切换或大规模 Store 重写，保持独立不合并。审计结论：

- 新增 crate `storyforge-infra-sqlite` **未接入** `tauri-app` / `harness-real-llm` / 任何
  生产 Store（`tauri-app` 与 `harness` 的 Cargo.toml 无 sqlite 依赖）。
- 默认生产后端仍为 JSON；未做 `storage.backend=sqlite`；无双真相源。
- 未修改任何 `crates/app-*` / `domain` / 生产 `infra-*` Store 实现。
- 仅修改 workspace `Cargo.toml`（注册成员 + path 依赖）、新增 ADR / crate / 测试。

**满足选择性合并条件 → 接收全部基础 commit。** 未合并的 SQL commit：无。

## 5. 完整发布门禁结果（本机实测）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 格式 | `cargo fmt --all -- --check` | **PASS** |
| 空白检查 | `git diff --check 7fb1899..HEAD` | **PASS**（0 处尾随空白） |
| 严格 lint | `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** |
| workspace 测试 | `cargo test --workspace` | **PASS — 981 passed / 0 failed / 25 ignored**（ignored 为真实 LLM / eval `#[ignore]` 用例） |
| 前端 Node 测试 | `npm test`（frontend） | **PASS — 218 passed / 0 failed** |
| 前端生产构建 | `npm run build`（vite） | **PASS — built in 2.07s** |
| secret scan | 人工 + 模式扫描 7fb1899..HEAD 全部新增行 | **PASS — 0 真实密钥**（仅测试 fixture `sk-abc`/`sk-test-should-not-write`/`sk-live-bronze-secret`，且断言这些被脱敏不落盘） |

### SQLite 专项门禁（`cargo test -p storyforge-infra-sqlite`）

**16 passed / 0 failed**，覆盖计划要求的全部项：

| 计划要求 | 覆盖测试 |
| --- | --- |
| migration 首次执行与重复执行 | `migrate_applies_v1_and_is_idempotent` |
| importer 重跑（幂等） | `import_is_idempotent_across_two_runs` |
| transaction rollback | `rollback_discards_writes`、`drop_without_commit_rolls_back`、`transaction_rollback_on_fk_violation_mid_import` |
| 默认 JSON backend 回归 | workspace 全量 981 passed；SQLite 未接线生产路径 |
| migration 失败不留半 schema | `failed_migration_does_not_leave_half_schema` |
| FK 强制 | `foreign_keys_enforced_after_v1` |
| 损坏输入拒绝且无半导入 | `corrupt_input_fails_and_leaves_no_partial_import` |

> Windows 文件锁 / 重启 smoke：`tauri-app` 默认未链接 sqlite crate，无文件锁路径可测；
> 此项随 SQLite 真正接线后再补，当前不阻塞集成。

### Bronze smoke（`scripts/run-bronze-smoke.ps1`）

**result=passed**，4/4 步：

- bronze_deterministic 8/8（B1–B4 store 级 + recover/hash/regen）
- writeback_isolation 12/12（含同名碰撞回归）
- meta smoke 4/4（B5 后端）
- diagnostic redaction 2/2（B6 脱敏）

> 说明：Bronze smoke 首次因**磁盘空间不足（os error 112，C 盘 100% 满）**中断于第 2 步。
> 清理其他 worktree 编译产物（释放约 36G）后重跑，全部通过。这是环境问题，非代码问题。

## 6. 测试证据位置

| 证据 | 位置 |
| --- | --- |
| Bronze smoke 汇总 | `artifacts/bronze/20260713-082905/SUMMARY.txt`（`result=passed`，gitignore，不入库） |
| SQLite 测试 | `cargo test -p storyforge-infra-sqlite`（16 passed） |
| workspace 测试 | `cargo test --workspace`（981 passed） |
| 前端测试 / 构建 | `frontend/`（218 passed；dist 生产构建 OK） |
| 前端 dist 占位 | `frontend/dist/index.html`（gitignore；为 Tauri `generate_context!` 创建） |

## 7. 未合并 SQL commit 及原因

**无。** SQLite 基础线全部 commit 已接收（见 §2、§4）。未出现需要保留独立的 SQL commit。

## 8. 未完成项与风险

1. **真实模型证据缺失（跨线）**：M5/Phase B 的真实 LLM 长会话、Phase B 真实 A/B 成本/延迟
   对照、Bronze 真实 GUI 截图均未执行（本集成线未获付费模型授权，0 次调用）。探针/harness
   已就绪但默认 ignore。这不阻塞合并基建，但**不得据此宣称**生产参数已标定或桌面 Bronze
   主流程完全验收。
2. **CommitTurn 探针与 Tauri 私有路径漂移风险**：探针复刻公开 store API，若
   `commit_turn_attempt` 私有路径后续漂移需同步（eval 线已记录）。
3. **SQLite 接线债务**：UoW / migration / importer 原语已就绪，但生产 accept 仍走 JSON 多文件
   写，原子性债务仍在；Android native / Windows 文件锁 / 发布体积随真正链接后再验。
4. **前端 `frontend/dist` / `node_modules` 仍 gitignore**：CI/发布机需 `npm ci && npm run build`
   才能 `generate_context!`；本机已验证可生成。
5. **环境磁盘**：本机 C 盘曾满（os error 112）；已清理但发布机应注意 target 体积。

## 9. 推荐的 main 合并 / push 操作

**建议将本集成分支 `codex/integration-nightly-2026-07-13` 合入 `main`**，理由：

- 三条开发线 commit 全部接收，无被拒 commit。
- 本机完整门禁全部通过：fmt / diff-check / clippy(-D warnings) / workspace test(981) /
  前端 test(218) / 前端生产构建 / secret scan / SQLite 专项(16) / Bronze smoke(4 步)。
- 默认生产行为未变（JSON backend 不变，SQLite crate 未启用）。
- 无 API key / secret / 原始 prompt / 用户数据泄漏。

### 合并前应明确（release notes 降级项，非阻塞合并）

- GUI / 真实模型条目（B1–B4 桌面真实导入/写作/截图、B5 MetaPanel UI、B6 失败 UI 文案、
  真实 LLM T1/T2/T3）仍属发布阻塞或 release notes 显式降级项。
- M5 完整验收 / Phase B 真实成本对照未完成（探针执行 PASS ≠ 参数标定 ≠ 完整 ACCEPTANCE）。

### 操作（由拥有 main 权限者执行，本线不自行 push main）

```text
# 在 main 上 fast-forward 或 --no-ff 合并本集成分支
git checkout main
git merge --no-ff codex/integration-nightly-2026-07-13 \
  -m "merge(integration): nightly 2026-07-13 release-bronze + eval-m5-phaseb + sqlite-foundation"
git push origin main
```

合入后可在 main 上打 release tag（视发布策略）。

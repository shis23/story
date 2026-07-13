# Bronze 发布证据线结果

- 分支：`codex/release-bronze`
- 基线：`7fb1899`
- 收口 HEAD：见下方 commit 列表
- 日期：2026-07-13
- 工作目录：`C:\tmp\storyforge-release`

## 已跑用例与证据类型

| 用例 | 证据类型 | 结果 | 说明 |
| --- | --- | --- | --- |
| B1 | Automated (store/command-path partial) | 部分通过 | `bronze_b1_card_campaign_active_and_worldbook_persist`：卡保存、Campaign 开档、Protagonist/Supporting 实例化、active_campaign 重载。未覆盖真实 GUI 启动/导入截图与真实首轮 LLM 写作。 |
| B2 | Automated (accept + MutationBatch) | 部分通过 | `bronze_b2_three_turn_accept_writeback_and_reload`：3 轮 Draft→Accept、summary/knowledge/variable/task 持久化与 Final 变体。未覆盖真实 Director/Subagent/Editor 文本质量。 |
| B3 | Automated + product fix | 部分通过 | `bronze_b3_same_name_instance_isolation_no_silent_cross_write`；发现并修复同名碰撞 name 误判 id 路。GUI 面板仍待截图。 |
| B4 | Automated | 部分通过 | 三轮写回 + reload；active campaign 与 turn recovery 另测。真实 postprocess/传话链 UI 仍待跑。 |
| B5 | Automated (backend) | 部分通过 | `scripts/run-meta-smoke.ps1` 4/4 再次通过；GUI MetaPanel 仍待截图。 |
| B6 | Automated (recovery + redaction) | 部分通过 | diagnostic 不泄密 + 启动恢复：Generating/AwaitingAcceptance→Failed，Committing 可恢复。GUI 失败文案仍待跑。 |
| regenerate / draft_hash | Automated | 通过（状态机层） | `bronze_regenerate_supersedes_old_attempt_and_keeps_new_accept_path`、`bronze_edit_draft_invalidates_hash_and_rederive_restores_accept`。 |
| Bronze smoke 入口 | Automated | 通过 | `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-bronze-smoke.ps1` → `artifacts/bronze/20260713-003802/SUMMARY.txt` |
| 人工 GUI 模板 | Manual template | 已提供 | `docs/workstreams/RELEASE-BRONZE-MANUAL-TEMPLATE.md` |

证据标签约定（对应计划要求）：

- Automated：本机确定性测试/脚本
- Manual：需桌面 GUI 或真实模型
- Not Run：本线未执行

## Commits（相对基线 `7fb1899` 之后本线新增）

1. `d7fee81` docs(workstream): plan Bronze release evidence
2. `9602476` fix(phase-b): 同名碰撞时禁止把角色名误判为 id 路
3. `7609558` test(release-bronze): 桌面 Bronze 确定性证据与 smoke 入口
4. `e3b78e1` style(release-bronze): fmt bronze evidence tests and manual template
5. `c6ba98b` fix(release-bronze): clippy needless borrow in bronze_deterministic
6. （本 RESULT / checklist 文档提交，若随后产生）

## 修改文件（本线主要）

- `crates/tauri-app/src/lib.rs` — 同名碰撞 present 判定修复
- `crates/harness-real-llm/tests/bronze_deterministic.rs` — Bronze 确定性证据扩展
- `crates/harness-real-llm/tests/writeback_isolation.rs` — 同名 raw_id=name 回归
- `crates/harness-real-llm/Cargo.toml` / `Cargo.lock` — `sha2` dev/runtime 依赖
- `scripts/run-bronze-smoke.ps1` — Bronze smoke 入口
- `docs/workstreams/RELEASE-BRONZE-MANUAL-TEMPLATE.md` — 人工证据模板
- `docs/workstreams/RELEASE-BRONZE-RESULT.md` — 本结果
- `docs/RELEASE-CHECKLIST.md` — 仅在拥有真实新证据时更新 B1–B6 状态

## 实际测试结果

本地已执行：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --quiet
git diff --check 7fb1899..HEAD
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-bronze-smoke.ps1
```

结果：

- fmt：通过
- clippy `-D warnings`：通过
- workspace tests：通过（真实 LLM ignored 用例按预期 ignore）
- whitespace check：通过
- bronze smoke 4/4：通过
  - bronze_deterministic 8/8
  - writeback_isolation 12/12
  - meta smoke 4/4
  - diagnostic redaction 2/2

未执行：

- `frontend npm test` / `npm run build`：本线未改前端；本机 `frontend/node_modules` 缺失，未联网安装依赖
- 真实付费模型 T1/T2/T3
- 真实 Tauri GUI 截图

备注：本机缺 `frontend/dist` 时 Tauri `generate_context!` 会失败；`run-bronze-smoke.ps1` 会创建本地 placeholder（gitignore，不入库）。

## 新发现产品 bug

| 级别 | 问题 | 状态 |
| --- | --- | --- |
| High（写回隔离） | 同名 instance 碰撞时，`is_postprocess_instance_present` 把 `raw_id=display_name` 误判为 id 路，可绕过 `name_collisions` 静默写到某一同名目标 | 已修于 `9602476`，回归 `b4_name_collision_raw_name_must_not_masquerade_as_id_path` |

## 仍需人工 GUI / 真实模型验证

- B1–B4 桌面真实导入、写作、面板截图
- B2/B4 真实 LLM 三轮质量与传话链可见性
- B5 MetaPanel UI
- B6 人为失败 UI 文案 + 真实 `log_export_bundle` 路径截图
- regenerate / Editor-only 在真实 GUI 的端到端体感

模板：`docs/workstreams/RELEASE-BRONZE-MANUAL-TEMPLATE.md`

## 是否建议合并

**有条件建议合并到 main（作为 Bronze 自动化证据 + 同名隔离修复切片），但不建议据此宣称桌面 Bronze 主流程已完全验收。**

建议合并理由：

- 新增确定性证据与 smoke 入口，断言持久化状态而非仅 exit 0
- 修掉真实写回隔离高风险 bug，并带回归
- 本机 Rust 门禁通过

合并前仍应明确：

- GUI / 真实模型条目仍是发布阻塞或至少 release notes 降级项
- 前端 npm 门禁需在有 `node_modules` 的环境补跑完整 `verify-release.ps1`

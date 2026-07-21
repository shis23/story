# SQLite + M5 100 轮真实模型集成结果

> 状态：**Partial Evidence（未封存 PASS）**  
> 日期：2026-07-21  
> 文档提交对照 HEAD：见本 commit  
> 代码事实审计：`docs/workstreams/SQLITE-CURRENT-STATUS-AUDIT-2026-07-21.md`  
> 计划：`docs/workstreams/SQLITE-M5-100-ENDURANCE-PLAN.md`

## 1. 结论（先读）

| 声明 | 是否成立 |
| --- | --- |
| SQLite opt-in 生产 pre-accept / Accept 接线在 main | **是** |
| SQLite endurance harness 走 `sqlite_runtime` 生产网关 | **是** |
| Native tool mode Coverage 12/12 accepted（本地旁证） | **是** |
| Native 12 seal / offline verify / secret-scan 封存 PASS | **否**（历史 seal 失败；本轮修了根因，但未重跑真实模型并封存） |
| TextFallback 3 专项 | **未完成** |
| Full 100 SQLite 封存 PASS | **未完成** |
| Meta 全覆盖 PASS | **否**（typed patch 等仍 explicit unsupported） |

**不得**把本文件读成 “SQLite Full100 / Native12 已验收通过”。

## 2. Gate A / B（代码）

已进入 `main` 的关键提交（历史）：

- `88dd2c6 feat(sqlite): wire preaccept lifecycle through production writing path`
- `d4f2f90 feat(sqlite): add M5 SQLite endurance harness and coverage ledger`
- `113fef8 feat(sqlite): wire regenerate UoW into SQLite endurance adapter`
- 后续一系列 `fix(sqlite|eval): ...` endurance / authority / recovery hardening

确定性生产网关测试：

- `crates/tauri-app/tests/sqlite_preaccept_production_lifecycle.rs`

Harness 路径：

- `pipeline.start_writing` + `sqlite_runtime::create_draft_attempt`
- autofix / `ProductionPostprocessService`
- regenerate → `sqlite_runtime::append_regenerate_attempt`
- Accept → `sqlite_runtime::accept_by_variant`
- coverage ledger：`sqlite_authoritative=true`，`json_fallback=false`

## 3. Gate C 真实模型旁证（Native Coverage 12）

### 3.1 最完整一次失败-于-seal 的 run

| 字段 | 值 |
| --- | --- |
| run_id | `run-coverage-b8445f64-ab5c-4942-a120-106cb65f4d54` |
| public evidence root | `C:\Users\Predator\storyforge-evidence\`（本机，非 Git） |
| stage | coverage |
| accepted_turns | **12/12** |
| calls_used | 258 |
| acceptance | `pass`（stage manifest） |
| fixture_hash16 | `113b880ee1811f63` |
| tool_mode | native |
| reasoning_mode | disabled |
| sqlite_authoritative | true |
| json_fallback | false |
| coverage_ledger_exact_set | planned=12 observed=12 |
| sqlite audit committed_turns | 12 |
| seal | **失败** |
| 失败点 | `seal_run` → `scan_for_forbidden` 读取 `campaign_data/storyforge.sqlite3` |
| 错误 | `evidence retention I/O error: 另一个程序已锁定文件的一部分，进程无法访问。 (os error 33)` |
| 日志 | `C:\tmp\sqlite-native12-postchange-20260717-114242.err.log` |

同 residual 说明见：

- `C:\Users\Predator\storyforge-evidence\PRIOR-RUN-RESIDUE-2026-07-17.md`

另有 early residue：

- `run-coverage-3738b639-2dec-4fab-a31d-3331528cbde9`（12/12 accepted，seal 未完成）

### 3.2 未完成项

| 项 | 状态 |
| --- | --- |
| TextFallback 3 | 未跑 / 无 RESULT |
| Stability 30 SQLite | 无封存 RESULT |
| Full 100 SQLite | 无封存 RESULT |
| 历史 run 离线 re-seal | 未做（见 §4；当前 harness seal 绑定 live clean-git 运行末尾） |

### 3.3 偏差

- 该 run 的 `max_calls=4294967295`（`u32::MAX`）出现在 checkpoint/manifest；后续 CoT/long_coverage 计划要求有界 budget，不得再把无界 calls 写成目标形态。
- 进程内 `sqlite_runtime` OnceLock 在 seal 时仍持有 live DB；seal 扫描误读二进制页导致 Windows 共享锁失败。

## 4. Seal 根因与代码修复（2026-07-21）

### 根因

`evidence_retention::scan_for_forbidden` 对 run 树做全量 `fs::read`，包括：

- `campaign_data/storyforge.sqlite3`（live，可被打开的 SQLite handle 独占）
- `.call-reservations.active.lock`（进程内 reservation writer 持锁）

这些路径**本来就不是** sealed subject（digest 已排除 `campaign_data`），但 secret scan 仍强制读完整文件 → Windows ERROR_LOCK_VIOLATION (33)。

### 修复

`crates/harness-real-llm/src/evidence_retention.rs`：

1. 跳过 live 二进制 / lock 文件：`.sqlite3` / `.sqlite` / `.db` / `-wal` / `-shm` / `.active.lock`
2. 在 live 工作目录下，对 sharing/lock I/O 不 fail seal
3. **仍扫描** live 目录下的文本叶子；文本 secret 继续 fail-closed

确定性回归：

- `seal_skips_live_sqlite_binaries_and_still_scans_text_secrets`
- `cargo test -p harness-real-llm --test evidence_retention_deterministic` → **60 passed**

### 修复不等于历史 run 已封存

本轮**没有**：

- 重跑真实 Native12 / TextFallback / Full100
- 对 `b8445f64...` 做 offline re-seal 并宣称 PASS  
  （现有 `seal_run` 仍假设 live 运行末尾 + 当前 git provenance；历史 residual 需专用 offline seal 工具或新 live 跑）

## 5. Meta

| 能力 | SQLite 状态 |
| --- | --- |
| campaign health（只读） | 支持（`meta_backend::sqlite_campaign_health_issues`） |
| typed patch preview/accept | **unsupported**（显式 fail-closed） |
| 其他 JSON-era Meta 写路径 | **unsupported** 直至 SQLite Meta UoW |

## 6. 下一刀

1. 用本 seal 修复后的二进制，重新跑 **Coverage Native 12**（新 run id），确认 seal+verify+secret scan 端到端 PASS。  
2. TextFallback 3 专项（或明确 unsupported）。  
3. 再决定 Stability 30 / Full 100 是否开跑。  
4. 可选：离线 re-seal 工具（不重放 LLM）处理 residual；在未完成前 residual 仅作旁证。

## 7. 可对外短句

可用：

- “SQLite M5 harness 与生产 pre-accept 接线已在 main；Native Coverage 曾 accepted 12/12。”
- “历史 Native12 因 seal 扫描 live SQLite 文件锁失败；2026-07-21 已修 seal 扫描，确定性测试通过。”
- “尚无已封存的 SQLite Full100 / TextFallback RESULT。”

不可用：

- “Native12 / SQLite Full 已 PASS。”
- “Meta 在 SQLite 下全覆盖。”
- “默认后端已是 SQLite。”

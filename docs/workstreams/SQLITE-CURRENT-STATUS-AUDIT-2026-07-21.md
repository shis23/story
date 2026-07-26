# SQLite 当前状态审计（2026-07-21）

> 状态：代码事实审计；**不是**真实模型 Full100 封存报告  
> 审计 HEAD：`bf36e04`（`fix(eval): serialize provider dispatch retries`）  
> 工作树：`main` clean  
> 目的：纠正 `HANDOFF` / preaccept RESULT / M5 plan 中已过期的 SQLite 结论，明确“已接线”与“已封存证据”边界

## 1. 总结论

SQLite 不是“还没做”。在 `main@bf36e04`：

1. **默认 backend 仍是 JSON**；SQLite 仍是显式 opt-in。
2. **opt-in 后，生产写作 pre-accept / Accept / recovery / barrier 已接同一 SQLite 权威路径**。  
   旧文档写“Repository 就绪但未接 production command”已**过时**。
3. **M5 SQLite endurance harness 已存在**，并通过 `sqlite_runtime` / `ProductionPostprocessService` / `accept_by_variant` 走生产网关，而不是固定 JSON `CommitProbe` 伪造成功。
4. **真实模型证据仍未收口**：  
   - 没有 `SQLITE-M5-100-ENDURANCE-RESULT.md`  
   - Native 12 曾 `turns=12/12 accepted`，但 seal/verify 因文件锁失败  
   - Meta typed patch / 多个 JSON-era Meta 命令在 SQLite 下仍明确 unsupported  
   - 不得把旧 JSON M5 45/100 或未封存日志写成 SQLite Full PASS

一句话：

> **SQLite 生产接线：大体完成（opt-in）**  
> **SQLite 真实证据与 Meta 全覆盖：未完成**

## 2. 切片状态表

| 切片 | 文档 | 代码现状（2026-07-21） | 证据等级 |
| --- | --- | --- | --- |
| Migration foundation | RESULT 有 | schema / importer / readiness 在 `infra-sqlite` | 确定性测试 |
| Accept UoW | RESULT 有 | `SqliteProductionRepository::accept_turn`；Tauri `accept_by_variant` 在 SQLite 路径调用 | 确定性 + 生产分支 |
| Chronicle publication / backup / reverse export | RESULT 有 | publication / reverse export / cutover backup 仍在 | 确定性测试 |
| Opt-in backend | RESULT 有 | `StorageBackend` selector、fail-closed cutover、marker、`PinnedBackend`、启动恢复 | 确定性 + Tauri startup |
| Pre-accept repository | RESULT 有（结论过时） | `SqlitePreacceptRepository` + V4 `preaccept_outbox` | 确定性测试 |
| Pre-accept → production command | RESULT 写“未接” | **已接**：`tauri-app` 命令层与 `sqlite_runtime` gateway | 生产分支 + `sqlite_preaccept_production_lifecycle` |
| SQLite M5 endurance harness | PLAN 有 / RESULT 无 | `sqlite_endurance` + real/deterministic tests | 代码存在；真实封存未完成 |
| Meta SQLite 全覆盖 | 计划要求显式 unsupported | health 可读 SQLite；typed patch 等 JSON Meta 仍 fail-closed unsupported | 代码门禁 |
| Windows/Android 真机启用 SQLite | HANDOFF 待证 | 本审计未做真机/GUI/大文件锁现场 | 无 |

## 3. 已核实的生产接线

### 3.1 启动与权威选择

- `crates/tauri-app/src/storage_backend.rs`：解析 config/env，默认 JSON；`STORYFORGE_STORAGE_BACKEND=sqlite` 才切 SQLite。
- `crates/tauri-app/src/sqlite_runtime.rs`：进程内 `OnceLock` 打开/迁移 DB；`is_sqlite_active()` 决定权威路径。
- 活跃时 conversation persistence、campaign/turn 读写、Accept/recovery 走 SQLite，不双写 JSON。

### 3.2 Pre-accept 生命周期（已接 production）

关键路径均在 `sqlite_runtime::is_sqlite_active()` 分支调用 preaccept UoW：

| 操作 | 生产入口 | SQLite API |
| --- | --- | --- |
| 首稿 draft + attempt | `start_writing` 成文后 | `sqlite_runtime::create_draft_attempt` |
| autofix 写回 | `ProductionPostprocess` / Backend sink | `sqlite_runtime::sync_autofix` |
| postprocess 候选 | Backend `attach_postprocess` | `sqlite_runtime::apply_postprocess` |
| regenerate | regenerate command | `sqlite_runtime::append_regenerate_attempt` |
| 用户编辑 stale | edit command | `sqlite_runtime::mark_stale_after_edit` |
| 启动恢复 | startup recovery | `fail_incomplete_preaccept` + production fail incomplete |

确定性证明入口：

- `crates/tauri-app/tests/sqlite_preaccept_production_lifecycle.rs`  
  经 **gateway**（不是只调 repository）完成 cutover → draft → autofix → postprocess → Accept → regenerate / late skip / scope fail-closed，并在移除 legacy JSON 后验证权威仍在 SQLite。

相关合并提交（历史）：

- `88dd2c6 feat(sqlite): wire preaccept lifecycle through production writing path`
- `f11e352 merge(sqlite-preaccept): add opt-in pre-accept lifecycle repository`

因此，`SQLITE-PREACCEPT-LIFECYCLE-RESULT.md` 中  
“Repository 就绪，但尚未接 production command”  
只对 **2026-07-15 该分支当时** 成立；对当前 `main` **不再成立**。

### 3.3 Accept / recovery / barrier

- Accept：SQLite 活跃时走 `sqlite_runtime::accept_by_variant` → `SqliteProductionRepository::accept_turn`
- recovery：startup 调 preaccept + production incomplete fail
- barrier / active-turn：读侧优先 SQLite active turn

### 3.4 Harness / M5 SQLite 路径

`crates/harness-real-llm/src/sqlite_endurance.rs`：

- `sqlite_runtime::activate`
- 写作：`pipeline.start_writing` + `sqlite_runtime::create_draft_attempt`
- regenerate：`sqlite_runtime::append_regenerate_attempt`
- postprocess：`ProductionPostprocessService`
- Accept：`sqlite_runtime::accept_by_variant`
- coverage ledger 记录 `sqlite_authoritative=true` / `json_fallback=false`

这推翻了旧 HANDOFF 的笼统说法：

> “M5 harness … 不执行 Tauri command 或 SQLite Accept 路径”

更准确的边界是：

- **旧 JSON M5 / Phase B 100 证据路径**：共享 JSON lifecycle / probe，不覆盖 SQLite Accept。
- **当前 SQLite endurance harness**：已调用 SQLite production gateway 与 Accept；它仍是 harness adapter，**不是**完整桌面 GUI command 端到端，也不自动覆盖所有 Meta 命令。

## 4. 仍未完成 / 仍不可宣称

### 4.1 文档与 RESULT

| 项 | 现状 |
| --- | --- |
| `SQLITE-M5-100-ENDURANCE-RESULT.md` | **不存在** |
| `SQLITE-PREACCEPT-LIFECYCLE-RESULT.md` | 历史 RESULT；“未接 production”过时 |
| `HANDOFF.md`（审计前） | 仍写 pre-accept 未迁完、harness 不走 SQLite Accept |
| CoT 三臂 80 轮 RESULT | 不存在；与 SQLite 接线正交，但共享 endurance harness |

### 4.2 真实模型证据

本地日志（非 Git 追踪，只能作旁证，不能当封存 RESULT）：

| 观察 | 含义 |
| --- | --- |
| `/tmp` 下 `sqlite-native12-*.log` | Native 12 曾跑到 `turns=12/12 accepted` |
| 同次 run seal 报 `os error 33` 文件锁 | **accepted ≠ sealed PASS** |
| 无正式 RESULT | 不能宣称 Native 12 / TextFallback 3 / Full100 SQLite 完成 |
| 旧 JSON endurance full 日志 | 与 SQLite M5 不是同一权威路径；旧 45/100 仍是历史 partial |

`SQLITE-M5-100-ENDURANCE-PLAN.md` 头注释  
“历史 Full100 已完成；Native 12 + TextFallback 3 专项补测进行中”  
应理解为：**计划执行中的工作状态**，不是已提交的封存结论。在 RESULT 落盘前，对外只能说：

- harness/code path 已具备 SQLite Full stage 能力；
- 真实模型补测与 seal **未正式收口**。

### 4.3 Meta / 非写作命令

`crates/tauri-app/src/meta_backend.rs`：

- `sqlite_campaign_health_issues`：只读 health **支持** SQLite
- `ensure_json_meta_backend_supported` / typed patch：SQLite 下 **明确 unsupported**  
  文案：`SQLite {capability} is unsupported until an atomic SQLite Meta UoW exists`

多个 Meta command handler 在 SQLite 活跃时走该门禁。正确报告方式是 **unsupported 清单**，不是“Meta 全绿”。

### 4.4 其他缺口

- 默认仍 JSON；没有用户数据强制迁移。
- Windows/Android **真机启用 SQLite** 的文件锁、生命周期、大数据现场证据仍缺。
- reverse export 对 `preaccept_outbox` 仍标 `unsupported_fields`（设计如此，非静默丢失）。
- 本审计**未**重跑 `cargo test --workspace` / 真实 LLM；只做代码与文档对照。

## 5. 对旧文档的处理

| 文档 | 处理 |
| --- | --- |
| 本文件 | 成为 2026-07-21 起 SQLite **当前事实**入口 |
| `docs/HANDOFF.md` | 同步为当前接线与证据边界 |
| `SQLITE-PREACCEPT-LIFECYCLE-RESULT.md` | 保留历史交付，顶部加 supersession 说明；2026-07-26 已归档至 `docs/archive/2026-07-26-completed-workstreams/` |
| `SQLITE-M5-100-ENDURANCE-PLAN.md` | 更新状态行，避免“Full100 已完成”被读成已封存 |
| `RELEASE-CHECKLIST.md` | 同步 pre-accept 已接线、证据未封存 |

历史 RESULT **不回写改写**为仿佛当时就完成；只标注后续 main 进展。

## 6. 下一优先级（SQLite 线）

1. **证据收口优先于再接线**  
   修复 Native 12 seal 文件锁，完成 offline verify / secret scan，写  
   `SQLITE-M5-100-ENDURANCE-RESULT.md`（诚实 PASS 或 partial）。
2. TextFallback 3 专项补测；未支持模式写 unsupported，不伪造。
3. Meta matrix：health 记 PASS；typed patch / MVU 等继续 unsupported，直到 SQLite Meta UoW。
4. 可选：Windows/Android 真机 opt-in 现场；与 CoT 80 轮三臂证据线分开记账。
5. 只有 RESULT 封存后，才允许更新 HANDOFF/RELEASE 把“专项补测进行中”改成具体日期结论。

## 7. 审计方法（可复核）

本审计依据：

- `git log` 中 `feat(sqlite): wire preaccept...` / endurance / opt-in 提交
- `crates/tauri-app/src/sqlite_runtime.rs`、`lib.rs` SQLite 分支、`meta_backend.rs`
- `crates/harness-real-llm/src/sqlite_endurance.rs` 与相关 tests
- `crates/tauri-app/tests/sqlite_preaccept_production_lifecycle.rs`
- workstream PLAN/RESULT 与本地 `/tmp` 旁证日志

未做：

- 付费真实模型重跑
- 全 workspace 发布闸门重跑
- 删除或续跑任何失败 evidence 目录

## 8. 可对外使用的短句

可用：

- “SQLite 已是显式 opt-in 生产后端；默认仍为 JSON。”
- “opt-in 后 draft / autofix / postprocess / regenerate / edit-stale / Accept / recovery 走 SQLite UoW。”
- “SQLite M5 harness 已接生产网关；真实 Full100 / Native12 封存 RESULT 尚未提交。”

不可用：

- “默认已切 SQLite。”
- “pre-accept 还完全没接 production。”
- “SQLite Full100 / Meta 全覆盖已验收通过。”
- “Native 12 PASS（仅 accepted 未 seal）。”

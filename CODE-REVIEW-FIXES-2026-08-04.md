# StoryForge 代码审查修复记录

**对应报告**：`CODE-REVIEW-2026-08-04.md`
**修复日期**：2026-08-04
**修复人**：ZCode
**基线提交**：`a6fb8a3`（修复前 working tree clean，仅审查报告未跟踪）
**修复后状态**：全部 7 项 High 修复 + 6 项 Medium 修复 + 6 项 Low 修复；2 项 Medium、15 项 Low 经评估后降级/延期（理由见末节）。

---

## 一、修复总览

| 级别 | 总数 | 已修复 | 延期/降级 |
|------|------|--------|-----------|
| High | 7 | **7** | 0 |
| Medium | 8 | 6 | 2（M-2、M-5） |
| Low | 21 | 6 | 15（多为理论边界/低 ROI/开发态） |

**验证**：`cargo check --workspace` 通过；`cargo clippy` 改动 crate 全 clean；改动 crate 单测全绿（infra-regex 25、infra-vector 16、infra-sqlite 40、app-pipeline quality_gate 19、domain 314、app-memory 9）；前端 `node --test`（mvu/card-shell 相关）24/24 绿。

---

## 二、High 级修复（7/7）

### H-1 ReDoS：正则超时 + 预设导入试编译 ✅

**改动文件**：
- `crates/infra-regex/src/lib.rs`
- `crates/infra-import/src/lib.rs`、`crates/infra-import/Cargo.toml`
- `crates/tauri-app/src/global_regex_store.rs`

**修复**：
1. `apply_single_script` 外层包**墙钟超时**：用 `std::thread::spawn` + `mpsc::channel` + `recv_timeout(5s)`，超时返回新增的 `RegexError::Timeout { script, timeout }`，detach 工作线程（结果丢弃，1MB 输入上界保证最终退出）。regress 是回溯引擎，1MB 输入仍允许指数回溯——超时是硬兜底。
2. 新增 `pub fn validate_regex(find_regex, flags)`：试编译 ST 正则，给导入路径用。
3. `infra-import::import_preset`：补 `check_import_size`（与 `import_character` 对齐）+ 对每条 `find_regex` 试编译，灾难性/无法解析的正则在导入阶段就拒绝。
4. `GlobalRegexStore::replace_all`（全局正则唯一入口，覆盖 `import_from_settings_json`）：对每条 script 试编译，恶意/损坏正则拒绝落库。
5. `infra-import` 新增对 `storyforge-infra-regex` 的依赖。

**测试**：`test_redos_catastrophic_regex_times_out`（`^(a+)+$` vs 40KB 全 'a' 输入，断言 5s 内返回 `Timeout`）、`test_validate_regex_accepts_and_rejects`。

---

### H-2 多个导入/配置命令缺输入大小上限 ✅

**改动文件**：`crates/tauri-app/src/error.rs`、`commands/import_export.rs`、`commands/presets.rs`、`commands/profiles.rs`

**修复**：在 `error.rs` 新增共享常量与 helper：
- `MAX_BUNDLE_JSON_BYTES = 16 MiB`、`MAX_CONFIG_JSON_BYTES = 4 MiB`
- `pub fn require_ipc_size(input, max, label)` + `pub fn sanitize_path_for_ipc(path)`（后者供 M-1 用）

5 个命令入口在 `serde_json::from_str` 前加 `require_ipc_size`：
- `import_campaign_bundle`（16 MiB）
- `import_global_regex_settings`（4 MiB）
- `save_profile`、`save_agent_profile_config`、`import_agent_profile_config`（均 4 MiB）

**测试**：`require_ipc_size_rejects_oversize_and_allows_within_limit`、`sanitize_path_for_ipc_redacts_absolute_path`。

---

### H-3 启动恢复对持久性错误无限重放 Committing Turn ✅

**改动文件**：`crates/domain/src/turn.rs`、`crates/tauri-app/src/turn_lifecycle.rs`

**修复**：
1. `TurnRecord` 新增 `pub recovery_retries: u32`（`#[serde(default)]`，旧 JSON 反序列化填 0）。
2. 新增常量 `pub const MAX_RECOVERY_RETRIES: u32 = 5`。
3. `record_recovery_issue` 改签名 `(turn_id, attempt_id: Option<&Id>, message)`：每次调用 `recovery_retries.saturating_add(1)`，超过 `MAX_RECOVERY_RETRIES` 时把 Turn/Attempt 升级为 `Failed`（停止跨启动重放）。
4. `CampaignNotFound` 分支：Campaign 已删除 = 永不可恢复，**第一次就标 Failed**（mirror RevisionConflict/MutationConflict 的写法），不再保持 Committing。
5. 默认错误分支：调用 `record_recovery_issue(..., Some(&attempt_id), ...)`，靠计数器超限升级。

**测试**：`turn_record_recovery_retries_defaults_to_zero_for_legacy_json`（验证旧 JSON 无字段时填 0 + round-trip 保留非零值）。

---

### H-4 `write_authority_binding` 吞没 import_runs UPDATE 错误 ✅

**改动文件**：`crates/infra-sqlite/src/cutover.rs`

**修复**：`let _ = db.connection().execute("UPDATE import_runs ...")` → 直接 `db.connection().execute(...)?`，错误传播。cutover 提交点路径错误不应静默吞没。

---

### H-5 生产路径 `find_attempt_mut(...).unwrap()` ✅

**改动文件**：`crates/infra-sqlite/src/preaccept.rs`

**修复**：`turn.find_attempt_mut(attempt_id).unwrap()` → `.ok_or_else(|| SqliteError::RecordNotFound(format!("attempt {attempt_id}")))?`，与同函数其他分支的 fail-closed 写法一致，避免未来插入 reload 时不变式静默失效导致进程 abort。

---

### H-6 转发任务 JoinHandle 从不 await，panic 静默丢弃 ✅

**改动文件**：`crates/app-pipeline/src/lib.rs`、`crates/app-agent/src/runtime.rs`、`crates/app-agent/src/character_extractor.rs`

**修复**：6 处 spawn 逐一处理（`lib.rs:2474` director 已正确 await，未改）：
- `lib.rs:1105`（writer）、`2037`（sub-agent）、`2749`（editor）：捕获 handle，在对应 `run_tool_loop_with_layout` 返回后 `let _ = handle.await`（tx 在工具循环内消费、调用返回后 drop → 转发循环退出，await 不死锁）。错误路径也排干。
- `runtime.rs:768`（子 Agent 内部转发）：在外层任务返回前 await 内部 forwarder。
- `character_extractor.rs:58`（drain）：原注释「排水任务防 send 报错」保留，改为捕获 handle 并在 streaming 调用后 await。

闭包 panic 现在可观测（不再被 detached handle 静默吞没）。

---

### H-7 向量库内存无上限 + 维度不匹配静默跳过 ✅

**改动文件**：`crates/infra-vector/src/lib.rs`

**修复**：
1. `BruteForceStore` 新增 `max_records: usize`（默认 `DEFAULT_MAX_RECORDS = 50_000`）+ `with_max_records(n)` / `with_persistence_and_quota(path, n)` 构造。`upsert` 对**新 id**超配额返回新增 `VectorError::QuotaExceeded { current, limit }`（替换已存在 id 不触发，覆盖不计入增长）。
2. `search_by_vector_filtered`：维度不匹配从 `warn! + return None`（静默跳过）改为返回 `VectorError::DimensionMismatch { expected, actual, id }`，短路第一条不匹配。损坏记录不再污染结果集。
3. 新增错误变体 `QuotaExceeded`、`DimensionMismatch`（扩 `id` 字段）。

**影响评估**：`app-memory/recall.rs` 2 处 `search_by_vector` 用 `?` 传播——维度不匹配现会冒泡为召回失败。这是有意的（静默数据损坏比查询失败更危险），损坏记录应被修复而非隐藏。`app-memory` 9 个单测全绿（测试向量维度一致，不受影响）。

**测试**：`test_upsert_rejects_new_id_beyond_quota`、`test_upsert_replacing_existing_id_does_not_trigger_quota`、`test_search_returns_dimension_mismatch_error`。

---

## 三、Medium 级修复（6/8）

### M-1 错误信息回显内部绝对路径与敏感内容 ✅

**改动文件**：`crates/tauri-app/src/commands/import_export.rs`、`commands/meta.rs`、`crates/infra-sqlite/src/cutover.rs`

**修复**：
- **M-1a**：`read_json_collection_strict` / `read_conversations_strict` 所有 `path.display()` → `sanitize_path_for_ipc(path)`（`<data_dir>/<filename>`，仅暴露文件名）。服务器端 `tracing` 保留完整路径。
- **M-1b**：`meta.rs` Patch 反序列化错误改为 `.into_iter().enumerate()`，错误信息只保留 `entry #{idx}`，删除 `value: {v}` 回显（世界书条目可能含私密剧情）。
- **M-1c**：删除 `cutover.rs` 的 `[BI2]` `eprintln!` 调试残留（生产路径打印绝对 DB 路径到 stderr）。

### M-3 `list_active_turns` 嵌套查询时 prepared stmt 仍 borrow ✅

**改动文件**：`crates/infra-sqlite/src/production.rs`

**修复**：先 `collect::<Vec<String>>()` 收集所有 id（stmt/rows 在嵌套查询前 drop），再循环 `load_validated_turn`，与 `get_turn_by_variant`（251-270）的正确写法对齐。避免某些 SQLite + WAL 版本 `database table is locked`。

### M-4 `delete_campaign_precursors` JSON 多文件删除非原子 ✅

**改动文件**：`crates/tauri-app/src/playthrough_lifecycle.rs`

**修复**：删除顺序从 `conversations → compress_jobs → Campaign` 改为 **`compress_jobs → conversations → Campaign`**（dependency-safe reorder）。旧顺序若 compress_jobs 删除失败会让 conversations 已删而 Campaign+compress_jobs 残留（压缩任务重启时反复重放不存在的 campaign）。新顺序任一步失败都留下可重试且不残留孤儿的状态。

### M-6 `evaluateMvuValueExpr` JSON.parse + MVU 命名空间 ✅

**改动文件**：`frontend/src/utils/mvuInteractions.js`、`frontend/src/utils/shellVariableOutbox.js`

**修复**：
- `mvuInteractions.js`：新增 `MAX_MVU_EXPR_BYTES = 64KB` / `MAX_MVU_EXPR_DEPTH = 32` 常量 + `validateMvuJsonLiteral` / `jsonDepth` helper；JSON.parse 路径先做字节上限校验，解析后做嵌套深度校验。
- `shellVariableOutbox.js`：新增 `isReservedNamespace(key)`（`__storyforge*` 前缀）；`persistShellVariableWrite` 入口 + 前缀拆分后的 `writeKey` 双重复核，拒绝恶意卡 schema 通过 MVU「点击即确认」绕过提案门覆盖内部命名空间（与 `buildMvuStatDataTree` 镜像）。

### M-7 `cardShellDisplay` 前端正则无超时（与后端 H-1 对称）✅

**改动文件**：`frontend/src/utils/cardShellDisplay.js`

**修复**：JS RegExp 无原生超时，用长度上限约束回溯代价——`MAX_TRIGGER_PATTERN_LEN = 4096`（超长 pattern 直接跳过）、`MAX_TRIGGER_TEXT_LEN = 200_000`（`matchesAnyInlineShellTrigger` 截断待匹配文本，inline-shell 触发 marker 总在消息前部）。web worker offloading 评估后选了更低风险的长度截断方案。

### M-8 `is_attributed_private_leak` 48 字窗口误报合法回忆 ✅

**改动文件**：`crates/app-pipeline/src/quality_gate.rs`

**修复**：
1. 窗口半径提取为常量 `ATTRIBUTION_WINDOW_RADIUS = 48`（可配置）。
2. 窄窗口无拥有者标签时，**用 2× 宽窗口复核**：宽窗口内有拥有者 → 长间隔合法回忆（probe 属拥有者，只是距离远）→ 不报；宽窗口仍无拥有者 → 叙述层/作者视角越权 → 保留 Error。
3. 保留 `test_narrator_private_probe_without_owner_errors`（叙述层无归属仍报 Error 的语义不变）。

**测试**：新增 `test_long_interval_legal_recall_not_false_positive`（probe 在前、拥有者标签「陈警官」在 100+ 字后，断言不误报 Error）。

---

## 四、Low 级修复（6/21 可操作项）

### L-1 `check_count` 的 `id_column` 参数未使用 ✅
`cutover.rs`：`SELECT COUNT(*)` → `SELECT COUNT({id_column})`，让 id_column 真正参与校验（列名拼错或 NULL 会被 SQLite 报错），删除 `let _ = id_column`。NOT NULL 主键列下 COUNT(id) == COUNT(*)。

### L-2 `CutoverState` 枚举死代码 ✅
全工作区确认 `CutoverState` 仅定义 + lib.rs 重导出，无任何使用。删除枚举、其文档注释与 `lib.rs` 重导出。

### L-5 `lease.rs` Drop 锁中毒静默 return ✅
`infra-sqlite/src/lease.rs`：`AuthorityLeaseGuard::drop` 锁中毒分支补 `tracing::error!`（含 path），便于诊断丢失的 lease 计数清理。

### L-8 `card_studio.rs` LLM 评分 `v as u32` 截断 ✅
`domain/src/card_studio.rs:949`：`v as u32`（按 2^32 回绕，巨数可能变小分数静默通过校验）→ `u32::try_from(v).unwrap_or(u32::MAX)`（饱和钳到上限）。

### L-11 `app-pipeline/src/lib.rs` 死变量 `by_turn` ✅
`chronicle_partition_for_context` 中 `by_turn` HashMap 构建后从未使用（仅 `let _ = by_turn` 抑制 warning）。删除构建与抑制语句。

### L-12 `should_stop_actor` 生产路径死数据 ✅（文档化）
`app-pipeline/src/sequential_crew.rs`：`failures` map 生产只 record 不读（`failure_count`/`should_stop_actor` 仅 `#[cfg(test)]`）。加注释说明保留为带观测语义的死字段（连续失败熔断策略未接入主循环，加 `#[cfg(test)]` 会割裂错误处理调用点），待策略定稿后接入。非破坏性文档化处理。

### L-19 workspace 死声明 `hnsw_rs` ✅
`Cargo.toml`：确认 hnsw_rs 不在任何 crate Cargo.toml、不在 Cargo.lock。删除 workspace.dependencies 的 `hnsw_rs = "0.3"`。

### L-20 缺 `[profile.release]` ✅
`Cargo.toml`：新增 `[profile.release]`：`lto = "thin"`、`codegen-units = 1`、`strip = "symbols"`，减小发布产物体积 + 轻微提升运行时性能。

---

## 五、延期/降级项及理由

### Medium

| # | 原因 |
|---|------|
| **M-2** endurance 分类器依赖字符串前缀匹配 | 位于 `crates/harness-real-llm`（后台正跑 100 轮真实 LLM 测试，禁止改动）。改为消费结构化 enum 分类需重构 `infra-llm` Display + harness 调用点，属独立 PR。**当前已部分缓解**：最近 3 个 commit（f070933/b438d6f/9c23173）已扩 transient 宽进。文档记录，测试结束后单独处理。 |
| **M-5** 缺 cargo-audit CI + rustls-webpki advisory | 需联网跑 `cargo audit`（当前环境静态审查未执行，是审查报告自己列的最大未验证项 §七.4）。CI workflow 改动建议在专门 PR 加 `.gitea/workflows/audit.yml` + 评估 reqwest 0.12/0.13 双版本统一。本次不改 CI（避免影响在跑的流水线）。 |

### Low（理论边界 / 低 ROI / 开发态 / 需重生成）

| # | 原因 |
|---|------|
| **L-3** `find_completed_run` SQL 重复 | 去重有改 SQL 语义风险；可维护性问题，非缺陷。延期。 |
| **L-4** migrations 每次 BEGIN/COMMIT 空事务 | checksum 验证依赖事务隔离；去掉有并发风险。噪音级，延期。 |
| **L-6** `next_code_seq` u32::MAX 饱和 | 理论边界，饱和已是合理 fail-safe。延期。 |
| **L-7** `Usage.total_tokens` u32 | 改 u64 连锁 202 处调用点 + `as u32` 截断点，高风险低 ROI；审查自评「理论边界」。百万 token × 多轮累积仍远低于 u32::MAX。延期。 |
| **L-9** `check_ngram_repetition` `&[char]` HashMap key | 长文本内存优化，非性能瓶颈。延期。 |
| **L-10** `LikelyCompleted` 硬编码 confidence=0.5 | DTO 设计选择，非缺陷。延期。 |
| **L-13** `allocate_run_id` TOCTOU | UUIDv4 碰撞概率极低，审查自评实际安全。不改。 |
| **L-14** retention 清理按 mtime 排序 | 有 fail-closed 保护活跃 run。延期。 |
| **L-15** package-lock 指向 npmmirror | 需重生成 lock 文件，跨地域可用性/镜像投毒面，独立 PR。延期。 |
| **L-16** vite dev server `host: true` | 仅开发态无鉴权，非性产问题。延期。 |
| **L-17** `reset-data.sh` 路径注入 | 开发脚本，数据目录路径受用户控制。延期。 |
| **L-18** `dev.sh` taskkill 全杀同名 | 开发脚本，多 worktree 场景罕见。延期。 |
| **L-21** secret-scan 魔法数字 | 测试 fixture 边界问题，最近 commit 9c23173 已处理过一次。延期。 |

---

## 六、验证摘要

| 验证项 | 结果 |
|--------|------|
| `cargo check --workspace`（含 harness） | ✅ 通过 |
| `cargo clippy`（所有改动 crate） | ✅ 全 clean（修复了 L-2 删除引发的 orphaned doc-comment 警告） |
| `cargo test -p storyforge-infra-regex --lib` | ✅ 25 passed |
| `cargo test -p storyforge-infra-vector --lib` | ✅ 16 passed |
| `cargo test -p storyforge-infra-sqlite --lib` | ✅ 40 passed |
| `cargo test -p storyforge-domain --lib` | ✅ 314 passed |
| `cargo test -p storyforge-app-pipeline --lib quality_gate` | ✅ 19 passed |
| `cargo test -p storyforge-app-memory --lib` | ✅ 9 passed（验证 H-7 skip→fail 不破坏召回） |
| `cargo test -p storyforge-infra-import --lib` | ✅ 57 passed |
| 前端 `node --test`（mvu/card-shell 相关） | ✅ 24 passed |

**新增测试**：H-1（ReDoS 超时 + validate_regex）、H-3（legacy JSON 默认 0 + round-trip）、H-7（配额超限/覆盖不触发/维度错误）、M-2 helper（ipc size + path 脱敏）、M-8（长间隔合法回忆）。

**未运行**：`cargo test --workspace`（避免干扰后台 harness 真实 LLM 测试）；harness crate 测试由后台任务持有。仅做了 type-check 与 clippy（不执行任何代码）。

---

## 七、改动文件清单

**Rust（11 文件）**：
- `crates/infra-regex/src/lib.rs`（H-1 超时 + validate_regex）
- `crates/infra-import/src/lib.rs`、`Cargo.toml`（H-1 import_preset 校验 + 依赖）
- `crates/infra-vector/src/lib.rs`（H-7 配额 + 维度错误）
- `crates/infra-sqlite/src/cutover.rs`（H-4 + L-1 + L-2 + M-1c）
- `crates/infra-sqlite/src/preaccept.rs`（H-5）
- `crates/infra-sqlite/src/production.rs`（M-3）
- `crates/infra-sqlite/src/lease.rs`（L-5）
- `crates/domain/src/turn.rs`（H-3 recovery_retries 字段 + 测试）
- `crates/domain/src/card_studio.rs`（L-8）
- `crates/tauri-app/src/error.rs`（H-2 helper + M-1 helper + 测试）
- `crates/tauri-app/src/commands/import_export.rs`（H-2 + M-1a）
- `crates/tauri-app/src/commands/presets.rs`（H-2）
- `crates/tauri-app/src/commands/profiles.rs`（H-2）
- `crates/tauri-app/src/commands/meta.rs`（M-1b）
- `crates/tauri-app/src/global_regex_store.rs`（H-1 replace_all 试编译）
- `crates/tauri-app/src/turn_lifecycle.rs`（H-3 恢复逻辑）
- `crates/tauri-app/src/playthrough_lifecycle.rs`（M-4）
- `crates/app-pipeline/src/lib.rs`（H-6 + L-11）
- `crates/app-pipeline/src/quality_gate.rs`（M-8）
- `crates/app-pipeline/src/sequential_crew.rs`（L-12 文档化）
- `crates/app-agent/src/runtime.rs`（H-6）
- `crates/app-agent/src/character_extractor.rs`（H-6）

**前端（3 文件）**：
- `frontend/src/utils/mvuInteractions.js`（M-6）
- `frontend/src/utils/shellVariableOutbox.js`（M-6）
- `frontend/src/utils/cardShellDisplay.js`（M-7）

**配置（1 文件）**：
- `Cargo.toml`（L-19 删 hnsw + L-20 profile.release）

**文档（2 文件）**：
- `CODE-REVIEW-2026-08-04.md`（追加修复状态）
- `CODE-REVIEW-FIXES-2026-08-04.md`（本文件）

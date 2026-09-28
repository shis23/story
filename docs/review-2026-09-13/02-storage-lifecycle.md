# 域 2 全量审查报告：存储与 Turn 生命周期

- 审查对象：StoryForge `HEAD ab894c6`，工作区干净（`git status` 无改动）
- 审查方法：**只读**审查（read/grep/glob + git log/show/diff），**未运行** cargo / npm / 任何构建或测试命令；**未创建或修改任何文件**（本报告除外）
- 执行方式：本域由 4 个只读子审计分头覆盖（Turn 生命周期 / cutover 与迁移 / JSON↔SQLite 双路径 / 测试覆盖）。**所有进入发现清单的源码引用（file:line + 摘录）都由我本人再次打开原文核对**；测试覆盖结论（S-18 的文件/测试计数与用例定位）来自对 34 个测试文件的机械清点，未逐条运行验证。标注「疑似」的条目明确表示静态代码无法定论、需要运行期复现

---

## 1 范围与覆盖率

### 1.1 文件范围（含行数，非测试代码）

| 区域 | 文件 | 行数（约） |
|---|---|---|
| 存储基础设施 `crates/infra-sqlite/src/` | `cutover.rs` | 1709 |
| | `readiness.rs` | 1922 |
| | `production.rs`（Accept UoW 核心） | 1810 |
| | `preaccept.rs` | 1118 |
| | `importer.rs` / `exporter.rs` | 1200 / 1735 |
| | `publication.rs` | 793 |
| | `migrations.rs` + `migrations/V001..V008`（8 个 SQL） | 578 |
| | `rollback.rs` / `lease.rs` / `connection.rs` | 490 / 342 / 185 |
| | `backend.rs` / `contract.rs` / `audit.rs` / `error.rs` / `unit_of_work.rs` / `lib.rs` / `bin/*` | ~1200 |
| Tauri 应用边界 `crates/tauri-app/src/` | `storage_backend.rs` | 2388 |
| | `backend_workflows.rs` | 2498 |
| | `sqlite_runtime.rs` | 2139 |
| | `turn_lifecycle.rs` | 2263 |
| | `turn_coordinator.rs` / `turn_store.rs` | 711 / 701 |
| | `playthrough_lifecycle.rs` / `storage.rs` / `json_store.rs` / `storage_health.rs` / `connection_store.rs` / `module_store.rs` / `startup_recovery.rs` / `lib.rs`（存储与启动段） | ~3000 |
| 测试（只读清点，未运行） | `crates/infra-sqlite/tests/*.rs` 19 个 + `crates/tauri-app/tests/*.rs` 19 个（含 15 个 `sqlite_*`） | 269 个 `#[test]`/`#[tokio::test]` |

覆盖统计：两 crate `src/` 共 87 个 `.rs`、约 2.55 MB 源码；本域精读约 60% 行数为逐行核对（上表列出的文件均已通读非测试部分），其余（前端、LLM 管线、card-shell 等非存储面）不在本域范围。

### 1.2 审查维度与判据

- **A 目标完成度**：声明（文档 + 任务书）与实际行为逐条对照，见 §4。
- **B 逻辑正确性（最高优先）**：事务边界 / 原子性 / CAS / 锁 / 崩溃恢复 / 幂等 replay / 发布顺序 / 回滚 / 并发首开 / 跨进程租约 / 终态 Degraded / late guard / 失败传播。
- **C 冗余与死代码**、**D 错误处理与安全**（fail-open / 静默 IO / unwrap / SecretRef 落盘）、**E 测试缺口**、**F 文档漂移**。
- 严重度：P0 = 数据损坏/丢失、安全、主流程不可用或与声明严重不符；P1 = 主路径在特定输入/时序下错误、原子性/边界被破坏；P2 = 边界未处理、错误处理不当、明显冗余；P3 = 命名/注释/文档漂移。
- 置信度：**高** = 代码路径逐行核实且触发条件明确；**中** = 代码路径已核实但触发需运行期时序/环境；**低** = 仅静态推断。

### 1.3 任务书与代码的一处事实差异（先说明）

任务书写作「`migrations/V001..V007`」，实际仓库为 **V001..V008**（`V008__authority_binding.sql`），当前 schema 版本为 **8**（`migrations.rs` 内置列表 + 测试 `migration_concurrency.rs:29` 断言 version=8）。本报告按代码事实（V001..V008 / v8）审查。

---

## 2 结论摘要

**计数：P0 2 条、P1 5 条、P2 11 条、P3 4 条（合计 20 条编号发现，其中 3 条为同类问题聚合条目，内部逐点给出 file:line）。**

**三句判断：**

1. **SQLite 侧的 Accept 主链路（UoW 原子性、事务内 CAS、ledger 幂等 replay、Degraded 不可逆、late attempt guard、失败传播到命令边界）经逐行核对是成立的，属于本次审查中质量最高的部分**，未发现「响应成功而磁盘滞后」或「重复 accept 二次扣减」的实现缺陷。
2. **真正危险的是三条「静默吞数据」路径：legacy 树不完整（`cards.json` 缺失/为空）时把全部 Campaign 判为悬空卡孤儿 → 空库 + 权威 marker 永久固化（S-01，默认启动路径可达）；cutover 发布后崩溃导致 `Stale` 闸门永久拒绝启动，而库层本已具备自愈能力却不可达（S-02）；以及 `run_cutover` 对 fresh-start 用户的内容无关身份判据 + 自由文本 stale 白名单，使「空库顶替有数据库 / 陈旧 JSON 重发布」成为一行调用即可能的数据丢失路径（S-03）。**
3. **JSON 显式回退路径的语义一致性与 SQLite 存在系统性差距**：accept 缺少 Committing/终态守卫、失败语义为「已提交却报错且进程内无法自愈」（S-04/S-05），叠加多处以 `let _ =` 吞掉的持久化错误（S-06/S-07/S-13），以及测试对「迟到 attempt 被拒」「Degraded 不可解除」等关键守卫零覆盖（S-18），构成次高风险面。

---

## 3 发现清单

### S-01 | P0 | 类别 B（数据完整性）+ A（与声明不符）
**标题**：legacy 树不完整时「缺失=空 + 悬空卡过滤」连锁成静默全量丢弃，并被权威 marker 永久固化

**位置**：`crates/infra-sqlite/src/readiness.rs:123-134`、`:164`、`:369-377`、`:201-245`、`:250-272`；`crates/infra-sqlite/src/cutover.rs:1050-1057`；`crates/tauri-app/src/storage_backend.rs:1884`

**证据**（所有核心集合一律 `optional = true`，缺失即空）：
```rust
// readiness.rs:123-134
    let cards = read_json_array(data_dir.join("cards.json"), true)?;
    let campaigns = read_json_array(data_dir.join("campaigns.json"), true)?;
    let instances = read_json_array(data_dir.join("instances.json"), true)?;
    let knowledge = read_json_array(data_dir.join("knowledge.json"), true)?;
    let tasks = read_json_array(data_dir.join("tasks.json"), true)?;
    let summaries = read_json_array(data_dir.join("round_summaries.json"), true)?;
    let turns = read_json_array(data_dir.join("turns.json"), true)?;
```
```rust
// readiness.rs:369-377 —— card_id 不在源 cards 中的 Campaign 全部丢弃（不是报错）
        if card_ids.contains(&card_id) {
            kept.push(campaign);
        } else {
            dropped += 1;
        }
    }
    if dropped > 0 {
        skipped.push(("campaigns_no_card", dropped));
    }
```
```rust
// readiness.rs:250-272 —— manifest hash 在「过滤之后」计算，因此空投影自洽
    hash_named_array(&mut hasher, "cards", &cards);
    hash_named_array(&mut hasher, "campaigns", &campaigns);
    ...
    let manifest_hash = hex_encode(hasher.finalize());
```
**完整链路（已逐环节核对）**：`cards.json` 缺失或为 `[]`（`campaigns.json` 等其它 legacy 文件仍存在 → `legacy_json_layout_present` 为真，走正常 cutover 而非 fresh start）→ `card_ids` 为空 → **所有 Campaign 被 `campaigns_no_card` 丢弃** → `campaign_ids` 为空 → `instances/knowledge/tasks/round_summaries/turns` 全部按孤儿跳过（`readiness.rs:201-245`，`filter_orphan_rows_counted` 只计数不报错）→ `world_info` 被过滤为空 → 快照即「只有 conversations」（`conversations.campaign_id` 在 `V001__init_schema.sql:59-69` **无 FK**，故不会因 FK 报错打断）→ manifest hash 按过滤后数组计算 → temp DB 校验与源快照**自洽通过** → `audit_published_database` 只做版本 + `integrity_check` → `write_marker_atomically`（`cutover.rs:1050-1057`，marker 之后无可失败步骤）→ 此后 SQLite 为唯一权威，`MarkerStatus::SqliteAuthoritative` 分支注释明写 "Never re-open JSON source trees"（`cutover.rs:862-865`）。

**影响**：用户在**首次 SQLite 启动**（默认路径，无需崩溃、无需并发）后看到「所有战役/角色/记忆消失」，而磁盘上的 JSON 仍完好却永不再被读取；`CutoverReport` 里 `skipped_orphan_rows` 只在 `tracing::info!`（`cutover.rs:1073-1078`）留痕，且报告在应用边界被整体丢弃（`storage_backend.rs:1884`：`let cutover_performed = matches!(outcome, CutoverOutcome::Completed(_));`）→ **无确认、无提示、无可审计计数**。数据未物理删除（可通过「恢复 `cards.json` + 删除 marker 与 DB」人工重迁移），但应用内不可恢复、用户无从得知。

**建议**：
1. `build_import_snapshot` 区分 `Missing` 与 `[]` 两种状态；至少要求 `cards.json` 与 `campaigns.json` 同时存在（或同时为合法空），否则 fail-closed 并给出「数据目录不完整，已拒绝迁移」的可见提示。
2. 孤儿过滤仅在**父集合文件确实存在**时生效：`cards.json` 缺失时不得把全部 Campaign 判为「悬空卡孤儿」。
3. cutover 前把 `skipped_orphan_rows` 分集合明细写入 `import_runs` 并向 UI 暴露；非零时要求用户确认或至少显著告警。

**置信度**：**高**（每个环节均已读原文；触发后的静默后果由哈希投影顺序与 marker 固化直接推出）。**疑似**：`Path::exists()` 把云盘占位/未同步/网络盘掉线判为「缺失」，从而在无人工操作时命中本链——需现场环境验证（置信度 中）。

---

### S-02 | P1 | 类别 B（崩溃恢复）
**标题**：cutover 两个崩溃窗口 → `Stale` 闸门永久拒绝启动，而库层的自愈入口在 app 路径上不可达（注释与代码行为相反）

**位置**：`crates/infra-sqlite/src/cutover.rs:1029-1032`（注释）、`:324-330`（Stale 判定）、`:664-676`（版本对账）、`:885-891`（自愈分支）；`crates/tauri-app/src/storage_backend.rs:1952-1957`（闸门）；`crates/tauri-app/src/lib.rs:1224` → `:1237-1239`（调用顺序）

**证据 1**（注释声称会自动重跑）：
```rust
// cutover.rs:1029-1032
    if fault == CutoverFault::AfterPublishBeforeMarker {
        // The DB is published but the marker is not. This is the ambiguous
        // window. On the next run, inspect_marker will see Absent (no marker)
        // and re-run the cutover, which will re-import and re-publish.
```
**证据 2**（实际返回 Stale）：
```rust
// cutover.rs:324-330
        if orphan_storyforge_db_exists(&plan.db_path) {
            return MarkerStatus::Stale {
                reason: "marker absent but a StoryForge database with authority binding exists \
                         (interrupted cutover); resolve manually or re-run cutover"
                    .into(),
            };
        }
```
**证据 3**（app 层把 Stale 当致命错误；`recover_or_verify` 只在其后才被调用）：
```rust
// storage_backend.rs:1952-1957（Stale 分支）
            MarkerStatus::Stale { reason } => Err(BackendWiringError::Marker(format!(
                "backend marker is stale; refusing to start until resolved: {reason}"
            ))),
// lib.rs:1224  resolve_backend(...)?      ← 闸门在这里
// lib.rs:1237-1239 sqlite_runtime::activate(db_path)?   ← 对账/激活在此之后
```
**证据 4**（库层本可自愈，且已有测试证明）：
```rust
// crates/infra-sqlite/tests/cutover.rs:264-269
    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
```

**影响**：两条崩溃窗口都会让应用**永久无法启动**（数据本身不损坏）：
- (a) 窗口位于 `fs::rename(temp→final)` + `fsync_file` 完成之后、marker 写入之前（`cutover.rs:1482-1485` → `:1057`），中间还夹着 `audit_published_database` 的 `PRAGMA integrity_check`（大库可达秒级）；
- (b) 新 migration 提交后、`reconcile_marker_schema_version` 回写前崩溃：闸门的 `verify_database_with_marker` 对 `version != marker.schema_version` 一律判 Stale（`cutover.rs:686-690`），而对账只在 `sqlite_runtime::activate` 内、**晚于闸门**调用（`sqlite_runtime.rs:103-109`）→ 自锁且永不自愈。
唯一出路是人工删库/删 marker，仓库内**没有** repair CLI（`bin/` 只有 `storyforge_rollback`，且它要求 marker 为 `SqliteAuthoritative`）。

**建议**：把 `Stale` 拆成 typed 分类；对「确属本次中断产物」的情形（身份 + nonce 证据）让 app 在闸门处调用 `recover_or_verify`（幂等），或提供 `storyforge_cutover --repair` CLI；把版本对账移到闸门判定**之前**；修正 `cutover.rs:1029-1032` 注释。

**置信度**：**高**（调用顺序、判定分支、测试均已核实）；触发为崩溃窗口（概率性），故按影响/概率权衡定为 P1（若只看后果可达 P0：应用完全不可用）。

---

### S-03 | P1 | 类别 B（数据丢失）+ D（fail-open）
**标题**：fresh-start 权威身份与 DB 内容无关 + `is_recoverable_stale` 自由文本白名单 → 「空库顶替有数据库」与「陈旧 JSON 重发布」

**位置**：`crates/infra-sqlite/src/cutover.rs:576-584`、`:419-427`、`:951-953`、`:1444-1470`、`:603-607`、`:885-891`、`:377-379`

**证据 1**（无 legacy 布局时身份退化为「data_dir + 常量空库 hash」，与库内容无关）：
```rust
// cutover.rs:576-584
fn orphan_belongs_to_this_cutover(plan: &CutoverPlan) -> bool {
    if !legacy_json_layout_present(&plan.data_dir) {
        // 源 manifest 无法对空目录计算（核心文件必需），改用空库 hash。
        let Ok(empty_hash) = empty_db_content_hash() else {
            return false;
        };
        let (authority_id, _nonce) = new_authority_identity(&plan.data_dir, &empty_hash);
        return orphan_db_matches_authority(&plan.db_path, &authority_id);
    }
```
```rust
// cutover.rs:1415-1470（所有权探测通过后：清理 sidecar → 让位旧库 → 发布空库）
        if !owned_by_storyforge_readonly(final_path, expected_manifest_hash, expected_authority_id)?
        { return Err(... "refusing to overwrite existing non-StoryForge database" ...); }
        for sidecar in ["-wal", "-shm"] { remove_sidecar_ignoring_missing(&path)?; }
        ...
        fs::rename(final_path, &aside)?;
```
**证据 2**（stale 分类用自由文本子串，`verify_database_with_marker` 的任何失败都被归入其中）：
```rust
// cutover.rs:603-607
fn is_recoverable_stale(reason: &str) -> bool {
    reason.contains("database file is missing")
        || reason.contains("database verification failed")
        || reason.contains("marker absent but a StoryForge database")
}
// cutover.rs:377-379：Err(e) => MarkerStatus::Stale { reason: format!("database verification failed: {e}") }
```

**影响**：对「无 legacy JSON 的 fresh-start 用户」（DB 中已积累大量战役），一旦 marker 丢失/损坏/不可读：(1) 身份判据**必然匹配**（空库 hash 是常量、authority_id 只由 data_dir 派生）；(2) 所有权探测也必然放行（该库 `import_runs.source_manifest_hash` 恒等于空 hash）；(3) `run_fresh_start_cutover` 新建**空** temp DB 并 `atomic_publish_db` 把真实库改名为 `.pre-publish-<ts>.sqlite3`、删其 `-wal/-shm`、空库顶替、照写权威 marker → **静默全量数据丢失**（报告与 UI 都不指向 `.pre-publish` 文件）。此外，`"database verification failed"` 这一类还覆盖「DB 被占用/权限/网络盘瞬时打不开」与「DB schema 版本高于 marker（更新二进制迁移过）」，使 `cutover.rs:593-602` 注释所声称的「跨版本降级不可能」并不成立——用陈旧 JSON 重导入即可让 SQLite 时代的写入离开活动路径。

**可达性（诚实标注）**：**默认 app 启动路径当前打不开这条链**——`inspect_marker` 返回 Stale 时 `storage_backend.rs:1952-1957` 直接 `Err`，而 `run_cutover`/`recover_or_verify` 是 `pub` API 且被文档描述为启动入口（`cutover.rs:1083-1090`），错误文案还在引导操作者「re-run cutover」。即：**当前唯一屏障是那道闸门**，任何新增「修复/切换后端」CLI、测试夹具或按提示手工重跑，都会命中数据丢失路径。

**建议**：(1) fresh-start 覆盖前**必须断言孤儿库为空**（或把 DB 内容 hash/行数/cutover_nonce 纳入身份判据），禁止内容无关身份授权替换非空库；(2) 用 typed enum 取代自由文本子串分类（DbMissing / BindingMismatch / SchemaNewer / ProbeBusy…），schema 较新一律拒绝并要求升级二进制；(3) 让 `.pre-publish-*` 在报告与 UI 中显式可见。

**置信度**：**高**（代码逻辑链逐行核实）；当前 app 入口**不可达**（中）。

---

### S-04 | P1 | 类别 B（原子性/失败语义）
**标题**：JSON accept 存在「副作用已全部落盘却返回 Err」窗口，且进程内没有恢复入口 → Campaign 卡死到重启

**位置**：`crates/tauri-app/src/turn_lifecycle.rs:654-701`（`with_campaign_lock` 内落盘 → 终态标记）、`:406-411`（再次 accept 被拒）；`crates/tauri-app/src/commands/turns.rs:703-724`

**证据**：
```rust
// turn_lifecycle.rs:695-701
                    // Storage/lock failures remain Committing for idempotent recovery.
                    return Err(AcceptError::Commit(error.to_string()));
                }
                // Side effects landed: terminal mark must surface persistence failure.
                self.mark_terminal_after_side_effects(&turn_id, &attempt_id, final_status.clone())?;
```
```rust
// commands/turns.rs:703-724（abandon 的 CAS 谓词排除 Committing）
        .mutate_turn_if(&turn.turn_id,
            |record| record.status.is_active() && !record.status.has_side_effects_started(),
            |record| { record.status = TurnStatus::Abandoned; ... })
        ...
    if !abandoned { return Err(... "Turn 已进入提交或终态，无法放弃" ...); }
```
**影响**：正文变体（`accept_variant`）与全部 campaign mutation 已在同一临界区内落盘、revision 已 bump，只有「终态标记」写失败时命令返回 `Err`。该 Turn 停在 `Committing`：再次 accept 被 `InvalidAttemptStatus` 拒绝（只有 `AwaitingAcceptance` 可 accept），abandon 被上列谓词拒绝，新轮次被 active-turn 屏障拒绝，而 `recover_turns_on_startup` 只在 setup hook 调用（`startup_recovery.rs`）→ **该 Campaign 在本进程内一直卡死**，只能重启应用。反向窗口同样存在：`apply_mutation_batch` 中途 IO 失败留下「部分 mutation 已落盘、revision 未 bump」，只能等重启重放。SQLite 侧不存在该窗口（准备事务 + 单 UoW + ledger replay，见 §5）。

**建议**：把错误分为「无副作用可重试」与「副作用已落地」两类，后者前端文案明确写「已提交，请重启完成收口」；或在返回 `Commit` 错误时同步触发一次幂等 `recover_committing_turn`。

**置信度**：**高**（落盘顺序与屏障谓词逐条核对）。

---

### S-05 | P1 | 类别 B（不变量被破坏 / 双路径语义分叉）
**标题**：JSON 路径三处「无条件写回」缺少 Committing/终态守卫（SQLite 同操作显式拒绝）→ 终态被回退、正文与记忆分叉

**位置**：`crates/tauri-app/src/turn_lifecycle.rs:154-163`；调用点 `crates/tauri-app/src/backend_workflows.rs:278-280`；`crates/tauri-app/src/turn_store.rs:243-252`；对照 `crates/infra-sqlite/src/preaccept.rs:610-615`、`:752-768`

**证据**：
```rust
// turn_lifecycle.rs:154-163 —— 无任何状态谓词
pub fn append_regenerate_attempt(record: &mut TurnRecord, new_attempt: TurnAttempt) {
    for att in &mut record.attempts {
        if att.status.is_active() { att.status = AttemptStatus::Superseded; }
    }
    record.attempts.push(new_attempt);
    record.status = TurnStatus::DraftReady;
    record.touch();
}
// turn_store.rs:243-252 —— 所谓 with_turn_mut 就是「无条件」
    pub fn with_turn_mut<F>(&self, turn_id: &Id, mutate: F) -> Result<(), String> {
        let applied = self.mutate_if(turn_id, |_| true, mutate)?;
```
```rust
// backend_workflows.rs:278-280（调用点确实无条件）
        if let Err(e) = self.storage.update_turn_record(request.turn_id, |record| {
            turn_lifecycle::append_regenerate_attempt(record, new_attempt);
```
**影响**：JSON 下 regenerate 与 accept 并发时（`read → 无条件 write` 的 TOCTOU，µs~ms 级）可出现：(1) regenerate 落在 accept 终态标记之后 → 把 `Committed/Degraded` 改回 `DraftReady` 并 Supersede **已提交** Attempt，而 revision 已 bump → 后续 accept 必然 `RevisionConflict`，该 Turn 主流程报废；(2) edit 的 Stale 写落在 CAS 之后 → `accept_variant` 把**编辑后**正文置 Final，而 knowledge/variable/summary 是按**编辑前**正文推导的 → 记忆与正文分叉。同类缺口还有 `commands/turns.rs:652-658`（`soft_delete_variant` 的 Attempt 写回无谓词且 `let _ =`）。SQLite 侧对同一操作显式拒绝（`preaccept.rs:610-615` "turn is Committing; cannot regenerate"、`:752-768` "cannot mark stale"）。

**建议**：把「Committing / 终态不可写」下沉为 `TurnStore::mutate_if` 的公共谓词层（新增 `mutate_pre_commit_only`），一次性覆盖 regenerate / mark_stale / soft_delete，而不是逐点补丁。

**置信度**：**高**（机制与 SQLite 对照均已核实）；触发需并发时序（中）。

---

### S-06 | P1 | 类别 D（静默 IO 失败）
**标题**：旧数据目录迁移的复制失败只 `warn`/`_ =` 吞掉 → 部分布局永久化，成为 S-01 的现实触发器

**位置**：`crates/tauri-app/src/lib.rs:503-529`（执行时机早于 `resolve_backend`：`lib.rs:1196` vs `:1224`）

**证据**：
```rust
// lib.rs:503-513
    if let Ok(entries) = std::fs::read_dir(&old_dir) {
        for entry in entries.flatten() {
            let dest = new_dir.join(entry.file_name());
            if entry.path().is_dir() { copy_dir_recursive(&entry.path(), &dest); }
            else if let Err(e) = std::fs::copy(entry.path(), &dest) {
                tracing::warn!("迁移文件失败 {}: {e}", entry.path().display());
            }
        }
        tracing::info!("数据迁移完成");
    }
// lib.rs:517-528（递归分支连日志都没有）
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).ok();
    ...
            } else { let _ = std::fs::copy(entry.path(), &dest); }
```
**影响**：占用/权限/路径过长/网络盘/中途取消导致的单文件复制失败**不上报、不重试、不阻断启动**；`new_has_data`（`lib.rs:490-496`）此后为真，下次启动也不会补拷。于是「部分布局」永久化，首次 SQLite 启动即按 S-01 把未拷到的集合当空集导入并写 marker → 用户数据静默缺失且不可自动恢复。

**建议**：改为「全量拷贝到临时目录 → 校验文件数与大小 → 原子换位」，任一文件失败即返回 `Err` 并阻断进入 cutover；至少把 `warn` 升级为用户可见的阻断错误。

**置信度**：**高**（代码已核实；现场复现需一次真实复制失败，中）。

---

### S-07 | P1 | 类别 D（静默失败导致功能卡死）
**标题**：启动恢复把活动 Turn 标 `Failed` 的写盘失败被完全吞掉 → Turn 残留活动态、Campaign 静默卡死且无提示

**位置**：`crates/tauri-app/src/turn_lifecycle.rs:937-942`（同文件其它恢复分支均有 `tracing::error!`）

**证据**：
```rust
// turn_lifecycle.rs:937-942
            let _ = self.update_turn_record(&turn_id, |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(format!("启动恢复：崩溃时处于 {status:?} 态"));
                record.intended_terminal_status = None;
                record.touch();
            });
```
**影响**：写盘失败（磁盘满/权限/写栅栏冻结）后无日志、无计数、无重试；该 Turn 保持活动态 → `get_active_turn` 命中 → `create_turn` 拒绝、`reject_if_active_turn` 拒绝一切直接写 Campaign 的命令 → **整个进程内该 Campaign 无法继续写作**，且用户看不到任何「恢复失败」提示。

**建议**：至少 `tracing::error!` + 计入 `storage_health` 阻断事件；更好的是向 UI 暴露「恢复未完成，需人工处理」。

**置信度**：**高**。

---

### S-08 | P2 | 类别 B（发布完整性）
**标题**：发布前无 WAL checkpoint、发布后无内容复验；temp 库清理全量吞错

**位置**：`crates/infra-sqlite/src/cutover.rs:1480-1487`、`:999-1013`、`:1704-1722`；`:1724-1732`（`discard_temp_db`）；不变式声明在 `:60-61`

**证据**：
```rust
// cutover.rs:1480-1487
    fs::rename(temp, final_path)?;
    // 审查一.5：rename 后 fsync 已发布 DB 文件（持久化发布结果）。
    fsync_file(final_path)?;
```
```rust
// cutover.rs:1724-1732
fn discard_temp_db(plan: &CutoverPlan) {
    let temp = plan.temp_db_path();
    let _ = fs::remove_file(&temp);
    for sidecar in ["-wal", "-shm"] {
        let path = format!("{}{sidecar}", temp.display());
        let _ = fs::remove_file(&path);
    }
}
```
**影响**：verify 在 rename **之前**（`cutover.rs:1011-1014`），rename 只搬主文件、从不处理临时库的 `-wal/-shm`；发布后审计只查「版本非 0 + `integrity_check`」。若临时库关闭时 WAL checkpoint 未完成（rusqlite 0.32.1 `InnerConnection::drop` 丢弃 `close()` 错误），`-wal` 与主文件脱钩 → **已发布库静默缺行**，而 `integrity_check` 与后续启动审计都不会发现「内容少于源 manifest」。另外 `discard_temp_db` 吞掉所有删除失败（与 `:60-61` 声明的「任何失败都丢弃临时库」不符）：残留库会被下一次 `Database::open(temp)` 复用，且 importer 对「completed 且 hash 相同」的 run 走 duplicate 短路（`importer.rs:87-89`）。

**建议**：publish 前显式 `PRAGMA wal_checkpoint(TRUNCATE)` 并断言 `<temp>-wal` 不存在；rename+fsync 后用 `recompute_db_content_hash` 对 `marker.manifest_hash` 复验后再写 marker；清理失败改为 warn/失败。

**置信度**：**高**（代码缺口）；WAL 未 checkpoint 的实际触发需运行期注入证明（中）。

---

### S-09 | P2 | 类别 B（备份有效性）
**标题**：备份检查点在内容校验**之前**生成、失败不清理、且从不与源 manifest 对账

**位置**：`crates/infra-sqlite/src/cutover.rs:999-1013`；`crates/infra-sqlite/src/readiness.rs:1231-1252`

**证据**：
```rust
// cutover.rs:999-1014
    let backup = readiness::create_backup_checkpoint(&import_db, &plan.backup_dir, &request.label)?;
    drop(import_db);
    ...
    // ── Step 5: Verify the temp DB ───────────────────────────────────
    let verify_db = Database::open(plan.temp_db_path())?;
    verify_imported_database(&verify_db, &manifest, schema_version)?;
```
```rust
// readiness.rs:1250-1252 —— 备份 manifest 的 hash 是「文件字节」hash，与源 manifest 不同命名空间
    let mut hasher = Sha256::new();
    hasher.update(fs::read(&backup_db_path)?);
    let manifest_hash = hex_encode(hasher.finalize());
```
**影响**：校验失败时 `discard_temp_db` 只删临时库，`sqlite-backups/` 里那份「checkpoint」永远留下（报告 `backup_label` 还指向它）；它只做过 `integrity_check`，从未与源 manifest 的计数/内容 hash 对账，marker 也不引用它 → **被当作「有备份」的产物可能正是校验失败的坏数据**，灾难恢复时会拿到不可用的备份而不自知。

**建议**：先 verify 再 backup（或 backup 后立即用 `recompute_db_content_hash` 对账并记录结果）；失败路径清理本次 checkpoint；`marker`/审计报告中记录 backup 的源 hash。

**置信度**：**高**。

---

### S-10 | P2 | 类别 B/D（连接与探测语义）
**标题**：只读探测无 `busy_timeout`、`immutable=1` 不回答「是否被占用」、`journal_mode` 返回值不校验

**位置**：`crates/infra-sqlite/src/connection.rs:61-68`、`:93`、`:98`、`:105-122`；`crates/infra-sqlite/src/cutover.rs:1618`、`:1543`、`:1439-1457`

**证据**：
```rust
// connection.rs:61-68 —— 无 busy_timeout、无 PRAGMA 配置
    pub fn open_readonly(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self { path, conn })
    }
```
```rust
// connection.rs:98 —— 返回值被丢弃（SQLite 在无法切换时返回原模式而不报错）
    conn.pragma_update(None, "journal_mode", "WAL")
// connection.rs:117-122 —— foreign_keys 却是事后校验的（口径不一致）
    conn.pragma_update(None, "foreign_keys", "ON")?;
```
```rust
// cutover.rs:1618（探测 URI，immutable 绕过锁）
    let uri = format!("file:{uri}?mode=ro&immutable=1");
```
**影响**：(1) 启动闸门的只读探测没有 5s busy_timeout（`connection.rs:92-93` 只对读写连接配置），瞬时 `SQLITE_BUSY` 会被包装成 `"database verification failed"` → 命中 `is_recoverable_stale` 白名单 → 可能走重发布（与 S-03 联动）；(2) `immutable=1` 只回答「是不是我的库」，**不回答「是否正被使用」**，而 `cutover.rs:1439-1443` 的注释正声称该探测能防住「目标被其它程序使用的 WAL 库」——真正互斥只剩 authority 租约（只对走租约的 SF 进程有效）；(3) `journal_mode` 若静默退化，`-wal/-shm` 清理、rename 语义、并发读假设与 `synchronous=NORMAL` 的持久性含义全部随之改变且无诊断。

**建议**：只读连接同样设置 busy_timeout 并区分 BUSY 与损坏；删 `-wal/-shm` 前用非 `immutable` 探测或 `BEGIN IMMEDIATE` 确认无人使用；断言 `PRAGMA journal_mode` 返回 `wal`（与 foreign_keys 同口径 fail-closed）。

**置信度**：**高**（代码缺口）；触发需运行期证明（中）。

---

### S-11 | P2 | 类别 B（跨进程互斥）
**标题**：`AuthorityLeaseGuard::drop` 提前 `return` 时真实 OS 锁仍被释放，但进程内簿记仍记为持有

**位置**：`crates/infra-sqlite/src/lease.rs:259-271`

**证据**：
```rust
// lease.rs:259-266
        if let Some((_, count)) = book.entries.get_mut(&self.path) {
            if *count > 1 {
                *count -= 1;
                // Still held by another guard in this process; keep OS lock.
                return;
            }
            book.entries.remove(&self.path);
        }
        drop(book);
        if !self.reentrant { self.release_os_lock(); }
```
**影响**：`return` 只跳出 `Drop` 函数体，字段仍被自动 drop——`file: Option<File>`（unix）/`_file`（Windows）关闭句柄即释放 flock / 共享锁，而 `book.entries` 仍记「持有中」。当同进程同路径出现「真实 guard 先 drop、重入 guard 后 drop」（Exclusive 真锁 + Shared 降级重入）时，进程自认仍持锁、实际已无锁 → **跨进程互斥静默失效**，另一进程的 cutover/rollback 可与本进程写者并发。当前生产每进程仅一个静态 shared guard，可达性主要在测试与未来调用方，但这是「独占」承诺的实现基础。

**建议**：把「是否持有真实 OS 句柄」与簿记计数绑定，禁止在 Drop 提前 return 时跳过真实锁的释放语义；补一个「重入 guard 乱序 drop」的单元测试。

**置信度**：**高**（Rust 字段 drop 语义 + 代码）；当前可达性中。

---

### S-12 | P2 | 类别 B/D（回滚持久化）
**标题**：rollback 的「全有或全无」在 rename 后 fsync 失败时被打破；marker 提交点的父目录 fsync 是 best-effort

**位置**：`crates/infra-sqlite/src/rollback.rs:440-453`、`:491`、`:503-510`；对照 `crates/infra-sqlite/src/cutover.rs:1505-1519`、`:1699`

**证据**：
```rust
// rollback.rs:445-451
    fs::write(&tmp, bytes)...?;
    fsync_file(&tmp)...?;
    fs::rename(&tmp, dst)...?;
    fsync_file(dst).map_err(|e| SqliteError::Other(format!("fsync {}: {e}", dst.display())))?;
```
```rust
// rollback.rs:503-510 —— 打开/同步两类错误全部吞掉，返回 ()
#[cfg(unix)]
fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) { let _ = dir.sync_all(); }
    }
}
```
**影响**：(a) 末步 fsync 失败时 `dst` 已被覆盖，但该路径只在成功后才进 `written`（`rollback.rs:417`），失败分支只逆序恢复已记录的路径 → 失败的那个文件保留新内容、其余回滚为旧内容 → data_dir 出现**新旧混合 JSON 树**（并残留 `*.json.rollback-tmp`），与函数文档「全部安装或全部保留」不符；(b) 把权威从 SQLite 翻成 JSON 的**提交点**却同时吞掉「打开目录失败」与「sync 失败」，断电后 marker rename 可能未持久化（回滚看似发生但重启仍是 SQLite 权威），且故障不可观测——与 cutover 同类函数的 `?` 传播口径不一致。

**建议**：安装前记录「目标 → 旧字节」并在失败时按记录恢复（含当前路径）；`fsync_parent_dir` 改为 `Result<()>` 并传播；Windows 至少显式记录为 no-op。

**置信度**：**高**（代码路径）；(a) 需 fsync 失败触发（低）。

---

### S-13 | P2 | 类别 D（吞错）+ C（死代码）
**标题**：JSON 级联删除吞掉 `delete_mvu`/`delete_card` 错误并在删除后做恒为 `None` 的二次查询

**位置**：`crates/tauri-app/src/storage_backend.rs:1406-1430`；声明侧 `crates/tauri-app/src/commands/characters.rs:308-313`

**证据**：
```rust
// storage_backend.rs:1413-1426
            for source_id in extra_source_ids {
                let _ = campaign_store.delete_mvu(source_id);
                if let Some(stored_card) = campaign_store.get_card_by_source(source_id) {
                    let _ = campaign_store.delete_card(&stored_card.card.id);
                }
            }
            if let Some(stored) = character_store.get(id)      // ← 主记录已删，恒为 None
                && let Some(source_id) = stored.info.source_character_id.as_ref()
            { ... }
            Ok(removed)
```
**影响**：`characters.json` 里的角色已删除、级联失败被完全吞掉，函数仍返回 `Ok(true)` → 用户「删除角色」成功，但该角色的卡与 MVU 翻译残留为孤儿（后续 `save_card` 去重会命中残留卡），与 `commands/characters.rs:308-313` 的「级联错误向上传播」声明不符。`character_store.get(id)` 分支在 `:1410` 删除之后恒不执行（功能由 `extra_source_ids` 覆盖）。SQLite 分支为单事务级联，无此问题。

**建议**：级联错误上抛或聚合返回「部分删除失败 + 明细」；删除顺序改为先级联后主记录；删除该死分支。

**置信度**：**高**。

---

### S-14 | P2 | 类别 D（吞错汇总）
**标题**：其余静默吞错点（同一反模式，逐点 file:line）

**证据与影响**：
1. `crates/tauri-app/src/backend_workflows.rs:218-222` 与 `:290-294`——draft/regenerate 失败后的补偿写 `let _ = self.storage.update_turn_record(... TurnStatus::Failed ...)` **无日志**：补偿失败时 Turn 停在 `Generating/DraftReady`，占着 active-turn 屏障，与 S-07 同后果（P2）。
2. `crates/tauri-app/src/commands/turns.rs:652-658`——`soft_delete_variant` 的 Attempt 写回既无状态谓词也 `let _ =` 吞错：命令返回 Ok 而 Attempt 可能仍是 `AwaitingAcceptance`（与 S-05 同源）。
3. `crates/tauri-app/src/module_store.rs:457`、`:720`——启动期 `let _ = self.save(default_profile)` / `let _ = self.persist_configs()`：best-effort（P3）。
4. `crates/infra-sqlite/src/importer.rs:102`——失败 run 的审计行 `let _ = ... INSERT ... 'failed'`：磁盘满/回滚失败时失败导入**完全无记录**（P3）。
5. `crates/tauri-app/src/storage_backend.rs:1889`、`:1977-1978`——`current_version(&db).unwrap_or(0)`、`Database::open(...).ok()?`：诊断可能谎报 schema_version=0（P3）。
6. `crates/infra-sqlite/src/cutover.rs:1140`、`crates/infra-sqlite/src/publication.rs:146`——`.ok()`/`.ok().flatten()` 把 DB 错误伪装成「无记录」（仍 fail-closed，但原因被掩盖）（P3）。

**建议**：统一口径——「补偿/清理类失败必须 `tracing::error!` 并计入 `storage_health`；审计类写入失败必须传播」；`unwrap_or(0)` 这类诊断缺省值改为显式 `None`/「未知」。

**置信度**：**高**。

---

### S-15 | P2 | 类别 B（并发护栏缺失）
**标题**：`fail_incomplete_*` 在事务外读取活动集；`delete_campaign` 无活动 Turn 屏障导致 accept journal 可被删

**位置**：`crates/infra-sqlite/src/production.rs:811-843`、`crates/infra-sqlite/src/preaccept.rs:853-920`；`crates/tauri-app/src/playthrough_lifecycle.rs:138-166`、`crates/tauri-app/src/storage_backend.rs:774-780`

**证据**：
```rust
// production.rs:813（读在事务外）→ …:820-843（写入在事务内，但不复核状态）
    let active = repo.list_active_turns()?;     // outside the UoW
    ...
    for turn in active { ... uow.execute("UPDATE turns SET status='failed' ...")? ... }
```
```rust
// playthrough_lifecycle.rs:146-150（JSON：先删会话 + Turn，再删 Campaign；删除路径不取 campaign 锁）
    if let Err(error) = state.storage().delete_campaign_precursors(
        campaign_id, conversation_ids,
        |conversation_id| deleter.delete(conversation_id),
    ) { ... return Err(... "活动仍保留可重试" ...); }
```
**影响**：(1) 启动恢复的「读活动集（事务外）→ 写 Failed（事务内）」在并发 accept 于两读之间提交时，会把**已提交**的 Turn 覆盖为 Failed（当前唯一调用点是单线程 setup，故按 latent P2；但这是共享 API，任何后台/命令调用都会命中）；(2) `delete_campaign` 没有 `reject_if_active_turn` 屏障，而与 accept 并发时会先删 `turns.json` 里本局全部 Turn（含 `Committing` journal），若随后删 Campaign 失败并返回「删除失败，可重试」，结果是 **Campaign 保留、Turn 历史与 journal 已永久丢失**，那次 accept 再无法重放/回滚。

**建议**：把状态复核放进 `UPDATE ... WHERE status IN (…非终态…)` 条件里（或先 `BEGIN IMMEDIATE` 后再读）；`delete_campaign` 入口加与 `reject_if_active_turn` 一致的活动 Turn 屏障，或在 `delete_turns_for_campaign` 内拒绝删除 `Committing` Turn。

**置信度**：**高**（机制）；触发需并发（中）。

---

### S-16 | P2 | 类别 A/B（双路径语义差距）
**标题**：JSON 的 CAS/幂等语义弱于 SQLite（无 `target == expected + 1`、用数字相等当「已重放」、payload 比较失败开放、`campaign_revision_after` 可被伪造）

**位置**：`crates/tauri-app/src/turn_coordinator.rs:294-319`、`:79-81`；`crates/tauri-app/src/turn_lifecycle.rs:703-707`、`:477-488`、`:291-294` vs `:618-619`

**证据**：
```rust
// turn_coordinator.rs:294-303（JSON 用「数字相等」判定本 batch 已重放）
        let is_replay = campaign.revision == batch.target_revision;
        let is_first_apply = campaign.revision == batch.expected_revision;
        if !is_replay && !is_first_apply { return Err(CommitError::RevisionConflict { ... }); }
// turn_coordinator.rs:79-81（序列化失败时 None == None → 判「一致」）
    fn payloads_match<T: serde::Serialize>(a: &T, b: &T) -> bool {
        serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
    }
```
```rust
// turn_lifecycle.rs:703-707（提交成功后回读失败被吞，伪造成「未 bump」）
                let campaign_revision_after = self
                    .campaign_store.get_campaign(&owner_campaign_id)
                    .map(|c| c.revision)
                    .unwrap_or(campaign_revision_before);
```
```rust
// turn_lifecycle.rs:291-294（契约要求 Replay 必须重确认持久性）vs :618-619（JSON 直接 Ok(outcome)）
    /// must re-confirm durability against its ledger/CAS and return the supplied
    /// outcome unchanged. No state mutation is allowed.
    Replay(AcceptOutcome),
```
**影响**：(1) JSON 没有 `target_revision == expected_revision + 1` 校验，也没有 `mutation_commits` 台账（SQLite 在 `production.rs:1010`、`:915-946` 两者都有），「数字巧合 = 已提交」的语义基础更弱；(2) `payloads_match` 在双方序列化同时失败时把冲突降级为「匹配」，属反向安全默认；(3) `campaign_revision_after` 是 revision bump 的**证据源**（`harness-real-llm/src/commit_probe.rs:282` 断言 `== before + 1`），一次瞬时读失败会让证据链误报「未 bump」，而 SQLite 端明确规避了这次回读（`sqlite_runtime.rs:1251-1255`）；(4) `Replay` 分支的 JSON 实现没有注释所要求的任何重确认（SQLite 有 `reconfirm_accept_replay`，ledger 缺失即 fail-closed），注释会让维护者误以为两端等价。

**建议**：JSON 侧补 `target == expected + 1` 断言；`payloads_match` 返回 `Result` 并 fail-closed；`campaign_revision_after` 两端统一取 `batch.target_revision`；拆开 `Replay` 注释并写明两端证据来源差异。

**置信度**：**高**（缺校验与吞错为代码事实）；「数字相等」被实际利用的可能性**低**（`campaign.revision` 在 JSON 路径只有 `turn_coordinator.rs:315` 一个写点，未能构造可达场景）。

---

### S-17 | P2 | 类别 E（可审计性/可运维性缺口）
**标题**：孤儿跳过计数不可审计；`storage_health` 只认 JSON；运行期无 marker/DB 复检

**位置**：`crates/infra-sqlite/src/importer.rs:216-234`、`crates/infra-sqlite/src/cutover.rs:1189-1192`、`crates/infra-sqlite/migrations/V001__init_schema.sql:134-142`、`crates/tauri-app/src/storage_backend.rs:1884`、`:284-292`、`:1962-1965`、`crates/tauri-app/src/commands/diagnostics.rs:9-11`

**证据**：
```rust
// cutover.rs:1189-1192（已迁移后的审计路径把跳过计数硬编码为 0）
        import_skipped_duplicate: false,
        skipped_orphan_rows: 0,
        backup_label: String::new(),
```
```rust
// storage_backend.rs:1884（应用边界丢弃整份报告，只留一个 bool）
        let cutover_performed = matches!(outcome, CutoverOutcome::Completed(_));
```
**影响**：(1) `import_runs` 无 skip 计数列（`V001:134-142`），`ImportSnapshot.skipped` 的分集合明细在边界被丢，真实跳过只在 `tracing::info!` 留痕且重启后报告恒为 0 → S-01 这类事故**无法事后追责**；(2) `storage_health` 的数据源只有 JSON store 的加载结果，不含 backend 与 SQLite 错误：运行期 SQLite 写失败/DB 失联不产生任何 incident，前端显示「健康」；`check_marker_status` 无生产调用方，前端拿不到 Stale 原因与恢复指引；(3) 运行期完全没有 marker/DB 复检，`validate_runtime_authority` 只比路径，`inspect_marker` 的调用方只有启动与 CLI。

**建议**：`import_runs` 增加 `skipped_orphan_rows`（分集合 JSON）并在审计/健康面板读回真实值；把 SQLite 侧失败写入 `storage_health`（新增 backend incident）；给 `storage_health_report`/诊断命令附带 backend、marker 状态、schema_version、DB 路径存在性与最近一次 DB 错误。

**置信度**：**高**。

---

### S-18 | P2 | 类别 E（测试缺口，缺口≠缺陷）
**标题**：关键守卫零覆盖与两类「假测试」反模式（本域 34 个测试文件、269 个测试函数清点结论）

**覆盖清单（已核实，非「看起来有测试」）**：revision CAS（`production_uow.rs:814` 零副作用 + `:933` 双线程并发 accept → `[Applied, AlreadyCommitted]`）、重复 commit 幂等、ledger 归属校验、UoW 注入回滚（`AfterMutations` / `BeforeLedger`）、cutover 故障阶梯（`cutover.rs:243/646/685`、`gate5_migration_matrix.rs:779` 六阶段 marker-last 矩阵、JSON 字节不变/最终 DB 不存在断言）、rollback 三阶段故障、导出安全（路径穿越/symlink/FK/checksum）、跨进程 lease 与导出锁（真实 helper bin）、孤儿/缺集合语义、JSON↔SQLite 快照等价、迁移幂等与并发首开（8 线程 Barrier）。

**缺口（按价值排序）**：
1. **`accept_turn` 对 Superseded/Stale attempt 的拒绝无任何测试**：守卫在 `production.rs:960-965`（attempt 必须 `AwaitingAcceptance`）与 `:948-953`（turn 必须 `AwaitingAcceptance`），`production_uow.rs` 里 `AttemptStatus::Superseded` 只出现在 `:370`、`Stale` 只在 `:406`，均不接 accept。
2. **事务中途失败时「已 INSERT 的行」是否回滚未覆盖**：唯一全 mutation 用例 `production_uow.rs:543` 是成功路径；`assert_accept_unchanged`（`:131`）只查 campaign/turn/variant/summaries/ledger，不查 instances/knowledge/tasks。
3. **Degraded 不可解除无 SQLite 集成测试**：`infra-sqlite` 19 个测试文件 grep `Degraded` **零命中**，而 `production.rs:977-983` 明确允许该终态；现有唯一覆盖在 JSON 侧（`turn_lifecycle.rs:1907` "must not upgrade Degraded to Committed"）。
4. **commit 期失败传播未钉住**：`sqlite_runtime.rs:1257-1258` 丢弃 UoW outcome（见 S-20），重构前缺少一条「commit 返回 Err ⇒ 调用方看到 Err」的判别测试（建议先加测试再重构）。
5. **弱覆盖**：真实 `integrity_check`/备份失败/磁盘满、migration SQL 自身失败（无任何 in-migration 故障注入）、真双进程冷启动迁移、SecretRef 经 cutover 的字节级存活（importer 源列表不含 `connections.json`）。
6. **反模式（可证明）**：(a) `backend_parity_suite.rs:2340` `restart_child_entry` 在 `STORYFORGE_RESTART_CHILD != 1` 时直接 `return`——默认 CI 下唯一走完整生产启动恢复的端到端检查**永远不执行**（no-op pass）；(b) `gate5_bigdata_perf.rs:293`、`sqlite_bigdata_perf.rs:420` 所有耗时只 `println!`、无任何阈值断言且未加 `#[ignore]`（有成本、无闸门）；(c) 弱断言 `assert!(err.is_err())`（`sqlite_preaccept_production_lifecycle.rs:352`）、裸 `is_ok/is_err`（`backend_selection.rs:68/77` 等）。

**影响**：缺口本身不构成本次审查的缺陷，但上述 1/3/4 恰好落在「最危险语义」上（旧 attempt 覆盖新 attempt、终态不可逆、失败不得静默成功），当前只有实现没有判别测试——任何重构都可能在不被告警的情况下破坏它们。

**建议**：按 1→3→4→2 顺序补集成测试；把 `backend_parity_suite.rs` 的重启子进程改为默认执行（或独立 job），给 perf 测试加阈值或 `#[ignore]` 并纳入显式 perf job。

**置信度**：**高**（清点为机械核对）。

---

### S-19 | P3 | 类别 F（注释/文档漂移）
**位置与证据**：
1. `crates/infra-sqlite/src/production.rs:1-4`——模块注释称 "deliberately not wired into the default application backend"，与 Gate 7 已默认 SQLite 的事实相反（实际经 `sqlite_runtime.rs:1095` 接入）。
2. `crates/infra-sqlite/src/contract.rs:3` 仍写「本阶段不切换默认 JSON Store」；`:173` 注释「核心文件缺失必须报错」与 Gate 7 新语义（缺失=空，`readiness.rs:115-122`）相反。
3. `crates/infra-sqlite/src/cutover.rs:1029-1032`——注释称「下次 inspect_marker 看到 Absent 并自动重跑」，实际返回 `Stale`（见 S-02）。
4. `crates/tauri-app/src/turn_lifecycle.rs:291-294` 的 `Replay` 契约与 JSON 实现（`:618-619`）不符（见 S-16）。
5. `crates/tauri-app/src/turn_lifecycle.rs:477-488`——`read_variant_content` 把「会话缺失/节点不存在/无 active 变体」都折叠成空串，`evaluate_accept_decision` 只能报 `DraftHashMismatch`（"草稿已被编辑但未重新推导状态"），把结构性问题误报成用户编辑；SQLite 侧同位置用 `.ok_or_else(Storage(...))`（`sqlite_runtime.rs:1145-1154`）。
6. `crates/tauri-app/src/commands/diagnostics.rs:24-27` 等文档/命令文案仍以 `connections.json`/`embed.json` 为主要存储面（非迁移集合，属措辞层面）。

**建议**：把 1-4 作为「注释与代码不符」一次性清理（其中 3 属于 S-02 的一部分，修复代码时同步改）；5 改为返回 `Result` 并区分错误类型。

**置信度**：**高**。

---

### S-20 | P3 | 类别 C（死代码/误导代码）
**位置与证据**：
1. `crates/tauri-app/src/sqlite_runtime.rs:1257-1258`——`let _ = outcome;` 与 `let _ = matches!(outcome, SqliteAcceptOutcome::AlreadyCommitted);`：纯死语句（`outcome` 已在 `:1233-1249` 解包并分类），却暗示「Applied/AlreadyCommitted 被有意区分后又忽略」，掩盖了排障所需的可观测信号。
2. `crates/infra-sqlite/src/contract.rs` 整个 `SqliteCampaignContract` 骨架仅被自身测试引用 = 死代码。
3. `crates/tauri-app/src/storage_backend.rs:1419-1427`——删除后 `character_store.get(id)` 恒为 `None`（见 S-13）。
4. `crates/tauri-app/src/storage_backend.rs:652-655` 附近存在仅用于取值的 guard 调用结果被丢弃（`let _checked = ...` 型），需要清理为显式断言或删除。
5. `crates/infra-sqlite/src/cutover.rs:1505-1509` 的 `fsync_file` 与 `crates/infra-sqlite/src/rollback.rs:496-500` 的 `fsync_file` 完全重复（未抽公共 util）；`crates/infra-sqlite/src/production.rs:127-128` 与 `:304-305` 两处硬编码活动状态字面量列表（当前与 `TurnStatus` 枚举一致，但需两处同步维护）。

**建议**：删除 1/2/3；4 改为断言；5 抽公共函数并给状态字面量列表加一条「与 domain 枚举一致性」的测试。

**置信度**：**高**。

---

### S-21 | P3 | 类别 B（状态一致性细节）
**标题**：恢复把 Turn 升级为 `Failed` 时未同步把 Committing Attempt 置 Failed；启动期配置持久化 best-effort

**位置**：`crates/tauri-app/src/turn_lifecycle.rs:757-772`、`:795-800`、`:804-809`；`crates/tauri-app/src/module_store.rs:457`、`:720`

**证据**：
```rust
// turn_lifecycle.rs:795-800（调用点传 attempt_id = None）
            let Some(attempt) = attempt else {
                self.record_recovery_issue(&turn_id, None,
                    "启动恢复缺少 Committing Attempt，保持 Committing".into());
                continue;
            };
// turn_lifecycle.rs:764-771（超过 MAX_RECOVERY_RETRIES 后置 Failed，但 Attempt 未同步）
            if record.recovery_retries > MAX_RECOVERY_RETRIES {
                record.status = TurnStatus::Failed;
                record.intended_terminal_status = None;
                if let Some(attempt_id) = attempt_id && let Some(attempt) = record.find_attempt_mut(attempt_id) {
                    attempt.status = AttemptStatus::Failed;
                }
```
**影响**：这两处 `attempt_id = None`，于是 Turn 已是终态 `Failed` 而 Attempt 仍为 `Committing`（active）→ `active_attempt()`/`find_attempt_by_variant` 仍会命中它，后续 accept 落到 `InvalidAttemptStatus`，UI 小票与 Turn 终态互相矛盾。

**建议**：`record_recovery_issue` 在升级 Failed 时把全部 `Committing/active` Attempt 一并标 `Failed`。

**置信度**：**高**（影响有限）。

---

### S-22 | P3 | 类别 其余低风险观察（聚合，逐点 file:line）
1. **`synchronous=NORMAL` 的断电持久性窗口**（`connection.rs:112`）：WAL+NORMAL 不损一致性，但断电可能丢掉最近提交；cutover 提交点自身对 DB 与 marker 都有显式 fsync，故未列为发现——建议在文档里显式写明该取舍。
2. **`cutover` 对源目录读两次**（`cutover.rs:956` 计算 manifest hash，`:985` importer 落库）：两次读之间源文件被改动 → `import_runs` hash ≠ marker hash → 校验报错、迁移中止（fail-closed，**不会产出错误 marker**），表现为「迁移随机失败、重试才好」。建议合并为「一次读、一次 hash、一次落库」。
3. **多进程首开无租约窗口（疑似）**：`storage_backend.rs:1881-1894`——`run_cutover` 返回（独占租约已释放）到 `hold_process_shared_lease` 之间存在无租约窗口，并发进程可重跑 cutover 覆盖刚发布的 DB（Unix 上 rename 成功 → 两库分叉；Windows 上 rename 因句柄在开会报错）。源码路径已核实，跨进程时序未复现（置信度 **中**）。
4. **导出成功后旧导出目录「让位」却永不清理**（`exporter.rs:217-232`）：每次重导出残留一份 `.export.pre-export-*` 完整旧副本（含正文/角色卡），无任何代码清理。
5. **导出 hash 与导入 manifest hash 投影不同却被注释称「同口径」**（`exporter.rs:1400-1405` vs `readiness.rs:259-271`：空集合前者参与 hash、后者不写前缀）：两套命名空间不可互比，目前无代码跨比，属措辞风险。
6. **rollback 安装的 JSON 树缺少 `active_campaign.json`**（`rollback.rs:282-294` 的 `JSON_ARRAY_FILES` 与 `SUBDIRS` 均不含它，exporter 也不产出），且不清理迁移前的陈旧指针 → 回滚后活跃指针缺失或失效（启动时有存在性校验，只 warn 丢弃，无损坏）。
7. **疑似：`derivation == None` 被判为「无失败」**（`turn_lifecycle.rs:413-419` 的 `is_some_and(has_failure)`）：只要 Attempt 是 `AwaitingAcceptance`，**没有任何推导记录**也会 `AllowCommit` 并以 `Committed`（非 Degraded）落库；当前生产写入路径都会 `Some(...)`，仅 legacy/半写状态可达（置信度 **中**）。
8. **疑似：SQLite 按 variant 选 Attempt 不过滤终态**（`sqlite_runtime.rs:1131-1137` 取最后一个同 variant 的 Attempt，含 Superseded/Stale/Discarded），JSON 用 `find_attempt_by_variant`（仅 active）→ 错误类型在两端分叉（`InvalidAttemptStatus` vs `NoAttempt`），影响前端文案与幂等分支（置信度 **中**）。

---

## 4 目标完成度核对表

| # | 声明/目标 | 判定 | 证据 |
|---|---|---|---|
| 1 | SQLite 是默认后端（Gate 7） | **达成** | `backend.rs:111-124` 无配置时默认 `Sqlite`；`storage_backend.rs:1943-1951` `Absent` → `run_sqlite`；`lib.rs:1224` 启动即解析；`env=json` 才回退且无 marker 时回退 JSON（`MarkerStatus::Absent` 分支） |
| 2 | 显式回退通道（`STORYFORGE_STORAGE_BACKEND=json`）fail-closed | **达成** | `backend.rs:96-124`（env 优先、空串忽略）；`storage_backend.rs:1923-1934`：marker=sqlite + env=json → `Err`（不静默改写权威） |
| 3 | SQLite accept 为单事务、原子提交 | **达成** | `production.rs:909` `UnitOfWork::begin`(IMMEDIATE) → `:1046-1056` mutations/revision → `:1099` `uow.commit()?`；准备事务（`sqlite_runtime.rs:1195-1221`）在 UoW 之前且只写候选 batch，崩溃可重试无副作用 |
| 4 | 「只有活动 Attempt 可写回」 | **SQLite 达成 / JSON 未达成** | SQLite：`production.rs:948-976`（turn/attempt status + draft_hash + batch 指纹全在事务内校验）、`preaccept.rs:500-533`（late guard 与写回同事务）；JSON：S-05（regenerate/mark_stale/soft_delete 无谓词） |
| 5 | accept 阶段存储失败必须传播（不得响应成功而磁盘滞后） | **accept 主路径达成，旁路未达成** | `sqlite_runtime.rs:1236-1249` 类型化映射（RevisionConflict/Storage/Commit）；`commands/turns.rs:603-609` `AcceptError::Storage\|Commit → internal`（前端可见）；旁路违约见 S-04/S-06/S-07/S-13/S-14 |
| 6 | revision CAS | **SQLite 达成 / JSON 部分** | SQLite：CAS 在 IMMEDIATE 事务内对**重读**的 campaign 校验（`production.rs:996-1018`），并强制 `revision == turn.base_campaign_revision == batch.expected_revision`、`target == expected + 1` → 无 TOCTOU；JSON：S-16（无 +1 校验、数字相等当 replay） |
| 7 | 幂等 replay（重复 accept 不二次扣减） | **达成** | `production.rs:915-946` ledger + `validate_ledger_replay` → `AlreadyCommitted`；`production_uow.rs:269` 断言第二次不 bump；`sqlite_runtime.rs:1079-1103` `reconfirm_accept_replay`（ledger 缺失即 fail-closed） |
| 8 | force accept 终态 Degraded 不可逆 | **达成** | `turn_lifecycle.rs:345-371`（Degraded/Committed replay 原样返回）、`:422-428`（force 归一）、`:737-755` `recovery_terminal_status` 不把 Degraded 升回；`production.rs:977-983` 只允许 `terminal_status ∈ {Committed, Degraded}` |
| 9 | cutover「marker 最后写」 | **写序列达成，重启行为未达成** | `cutover.rs:1039-1057`：审计在 marker 前，marker 之后只有纯内存报告（`:1065-1080`）；`write_marker_atomically`（`tmp+fsync+rename+fsync+父目录 fsync`）。崩溃后的重启行为见 S-02 |
| 10 | 迁移事务性与幂等 | **达成** | `migrations.rs:131-171/196-262`：每条 migration 一个 `BEGIN IMMEDIATE` 事务，DDL 与 `schema_migrations` 行同事务提交，事务内复查；全仓 `grep user_version` 无匹配；`gap` 测试 `migration_concurrency.rs:8`（8 线程 Barrier → version=8、行数=8） |
| 11 | 跨进程互斥（首开/迁移/回滚/导出） | **基本达成，有一处语义缺口** | 锁均为 OS 句柄锁（flock / Windows `share_mode`），进程死亡即释放、无 pid 文件与 stale takeover（`cutover.rs:1738-1812`、`lease.rs:174-243`）；缺口见 S-11（Drop 语义）与 S-22.3（无租约窗口） |
| 12 | 备份与恢复 | **存在但有效性不足** | 备份用在线 backup API + `integrity_check`（`readiness.rs:1231-1248`），但顺序/对账/清理见 S-09 |
| 13 | RESULT §36(a) 缺失 legacy 文件按空集合导入 | **达成但对 `cards.json` 不安全** | `readiness.rs:123-134` 全部 `optional=true`、`:1881-1899` 缺失→空 / 损坏→fail-closed；风险见 S-01 |
| 14 | RESULT §36(b) 孤儿行跳过并计数 | **内存层面达成，可审计层面未达成** | `readiness.rs:201-245/315-340/246`；计数不落库、verify 恒 0、明细被丢（S-17） |
| 15 | RESULT §36(c) 不双写、不删旧 JSON | **cutover/启动路径达成；rollback 措辞需修正** | SQLite 下四个权威 JSON store 全为 `None`（`storage_backend.rs:186-206`）且取用 fail-closed（`:294-370`）；全仓无「同记录双写」路径；cutover 不删 JSON（全 crate `remove_file/remove_dir_all` 清点仅 temp/staging/sidecar）。例外：rollback 安装会以 `[]` 覆盖/删除旧 JSON（`rollback.rs:305-328/384-392`），属用户显式确认的回滚 |
| 16 | RESULT §36(d) stale marker 拒绝（不 fail-open 到 JSON） | **达成** | `storage_backend.rs:1952-1958` 无条件 `Err`；Stale 判定覆盖 marker 不可读/损坏/版本过新/version 0/DB 缺失/绑定失败（`cutover.rs:317-387`），锁后复检（`:908-938`）；但它是 S-02 的另一面 |
| 17 | 原子写与写栅栏（`atomic_write_json`、freeze） | **达成** | `infra-util/src/lib.rs:29-99`（写栅栏检查 + `.tmp` + `sync_all` + rename 重试 ×3）；`storage_health.rs:61-71` 冻结/解冻；`json_store.rs:12-55` 三态处理（缺失→默认、读失败→冻结、解析失败→`.tmp`→`.corrupt`+冻结） |
| 18 | 运行期健康/诊断可反映真实后端状态 | **未达成** | S-17(2)(3)：`storage_health` 只认 JSON store；无 marker/DB 复检 |

---

## 5 未发现问题与低风险观察

### 5.1 明确未发现问题（已核对范围）

- **SQLite Accept UoW 原子性与事务边界**（`production.rs:897-1101`、`unit_of_work.rs:1-104`）：`BEGIN IMMEDIATE`、失败/`Drop` 回滚、`commit()?` 传播；turn 终态 + attempt + campaign mutations + revision + conversation 写回 + `mutation_commits` 台账同一事务；注入回滚测试覆盖 mutation 后与 ledger 前两个点位。**未发现「成功但未提交」或「提交后响应失败」的窗口**。
- **事务内 revision CAS（无 TOCTOU）**：CAS 的取值来自事务内重读的 campaign（`production.rs:996-1018`），不接受事务外快照；并发 accept 由测试证明只会得到 `[Applied, AlreadyCommitted]`。
- **幂等 replay / 重复提交**：ledger + 请求指纹校验（`production.rs:915-946`），重复 accept 不二次 bump；`reconfirm_accept_replay` 在 ledger 缺失时 fail-closed。
- **Degraded 不可逆（accept/replay/恢复三处）**：见 §4 第 8 行；此外 `Degraded` 只能由 force 或质量 Error 产生（`turn_lifecycle.rs:422-428`）。
- **Late attempt guard（postprocess 方向）**：`turn_lifecycle.rs:49-60` 的谓词与写回在**同一** `mutate_if` 临界区内（`production_postprocess.rs:505-543`），无检查-写回 TOCTOU；SQLite 对等实现同在事务内（`preaccept.rs:500-533`）。
- **marker 写入的原子性与「marker 最后」**：`tmp 写 → fsync → rename → fsync → unix 父目录 fsync`（`cutover.rs:1690-1701`），之后无可失败步骤；marker 绑定校验（schema 版本 + 源 hash + authority_id + nonce 三项一致，`cutover.rs:680-753`）无缺陷。
- **migration 原子性与幂等**、**跨进程首开串行化**（WAL 升级 BUSY 重试 + `foreign_keys` 事后校验）：无缺陷；不存在「版本先于 DDL 提交」。
- **锁/租约的崩溃释放语义**：OS 句柄锁，进程死亡即释放，无陈旧锁双跑风险（`lease.rs` 的 Drop 顺序问题见 S-11，属语义而非泄漏）。
- **进程内单写者**：`sqlite_runtime.rs:36-38` 全局 `OnceLock<Arc<Mutex<Database>>>` + 每闭包持锁（`:153-181`）；`mutate_turn_if`（`:377-393`）是「同锁内读-改-写」，进程内两次 accept 不可能丢失更新；5 个关键文件无 `async fn`、无 Mutex 跨 `.await`，无锁序反转（turns → campaign 顺序一致）。
- **`backend.rs` / `error.rs` / `lib.rs` / `audit.rs` / `storage_health.rs` / `storage.rs` / `json_store.rs` / publication.rs**：纯选择/纯校验/机制层，未发现缺陷（`publication.rs` 单 UoW、按 `(job_id, batch_index)` 幂等、无文件 IO，且未接入默认后端）。
- **RESULT §36 各条**：见 §4 第 13-16 行（其中 S-01/S-17 是其安全边界的例外，已单列）。
- **panic/unwrap 面**：`turn_lifecycle.rs` / `turn_coordinator.rs` / `turn_store.rs` / `playthrough_lifecycle.rs` 非测试路径**无** `unwrap/expect/panic!/unreachable!`；`sqlite_runtime.rs` 唯一 `expect`（`:177`）只在测试路径；JSON 损坏文件的路径是 fail-closed（冻结 + 阻断），不会把空 journal 写回覆盖磁盘。

### 5.2 低风险观察（不单列为发现）

- `save_turn`/`list_active_turns` 中硬编码的状态字面量列表（`production.rs:127-128`、`:304-305`）经与 `domain/src/turn.rs` 的 `TurnStatus` 枚举逐项比对**完整且正确**（`Abandoned` 正确地不在活动集内）；仅维护性风险（S-20.5）。
- `contract.rs` 的 `SqliteCampaignContract` 是未接入的死代码（S-20.2），删除即可。
- `module_store` 的启动期 best-effort 持久化（S-14.3）实际影响很小。
- `open_in_memory` / `open_readonly` 无 `application_id` 校验属设计（只读探测不写 header，是有意为之）。

---

## 6 需要 Lead 重点复核的结论

1. **S-01 必须先定案（P0）**：请裁决「缺失 = 空集合」是否应无条件适用于 `cards.json`/`campaigns.json`。当前组合（缺失=空 + 悬空卡 Campaign 丢弃 + 过滤后再算 hash）会让「迁移复制失败/云盘未同步」直接变成「空权威 + 权威 marker」，且**默认启动路径即可达、无确认、无一键回退**。若判定为可接受语义，至少需要「核心集合存在性门禁 + 用户可见提示 + 跳过计数落库」。
2. **S-02 是可用性问题而非数据问题**：库层 `recover_or_verify` 已有测试证明可自愈，但 app 闸门在它之前就硬失败；请决定「闸门内自愈」还是「提供 repair CLI」，并同步修正 `cutover.rs:1029-1032` 的注释——目前注释、错误文案与代码三者互相矛盾。
3. **S-03 是最危险的潜在数据丢失路径**：需要 Lead 明确「fresh-start 身份的 nonce 必须来自本进程本次尝试，而非 `(data_dir, 空库 hash)` 推导」，否则任何新增的 cutover/修复入口都会把「marker 丢失」升级为「全量数据被空库顶替」。
4. **S-04/S-05 决定 JSON 显式回退是否还能作为「兼容后端」**：当前 JSON 侧已出现「已提交却报错且进程内无法自愈」与「终态守卫缺失」两类与 SQLite 明确不一致的行为。请确认这是长期接受的差异，还是把守卫下沉到 `TurnStore` 公共谓词层 + 补运行时幂等恢复入口。
5. **S-18 的测试缺口需要与实现修复排同一批次**：`accept` 对迟到 attempt 的拒绝、Degraded 不可解除、commit 期失败传播三项都是「有实现、无判别测试」，任何重构都会在无告警的情况下破坏它们；同时 `backend_parity_suite.rs:2340` 的 env 门控使唯一的生产重启恢复端到端检查在默认 CI 中是 no-op。
6. **交叉模块复核建议**：`sqlite_runtime.rs:103-109`（版本对账位置）、`storage_backend.rs:1952-1957`（Stale 闸门）、`lib.rs:1224/1237`（闸门 → activate 顺序）是 S-02/S-22.3 的关键上下文；`commands/turns.rs:652`（S-14.2）与 `playthrough_lifecycle.rs:138-166`（S-15.2）应由对应文件的 owner 与其他域的发现去重。

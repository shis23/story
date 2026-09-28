# R13 修复：`readiness` 合法 legacy 布局回归（N-R1-01）/ 父级全丢（N-R1-02）/ `Path::exists()` 误分类（N-R1-04）

- **修复人**：`review-storage`（域2 owner）
- **任务**：task-36（R13，P1 启动阻断回归；Lead 指令：先做 task-36 再做 task-35/R12）
- **输入**：`docs/review-2026-09-13/round2/R1-domain-storage-recheck.md`（冻结）附录 A 最小复现、`N-R1-01` / `N-R1-02` / `N-R1-04`
- **写入范围**（Lead 指定）：`crates/infra-sqlite/**`、`crates/tauri-app/src/storage_backend.rs`、本文件、`docs/review-2026-09-13/fixes/02-storage-fixes.md`
- **实际写入**：`crates/infra-sqlite/src/{readiness,cutover,importer,error}.rs`、`crates/infra-sqlite/tests/importer_diagnostics.rs`、本文件、`fixes/02-storage-fixes.md §13`。
  `crates/tauri-app/src/storage_backend.rs` **本轮无改动**（理由见 §1.5：应用层已 fail-closed，无需接线）。
- **未触碰**：`round2/R1..R7*.md`（冻结件）、前端、其它域文件。

---

## 0. 结论速览

| 条目 | 严重度 | 裁定 | 实现位置 | 判别性测试 |
| --- | --- | --- | --- | --- |
| **N-R1-01** Rule B 把全部 conversations 当 campaign 依赖 → 合法非 Campaign 老用户启动即被拒 | P1 | **已修复** | `readiness.rs:160`（新 helper）、`:292`（Rule B 只统计 campaign 归属会话） | `legitimate_empty_and_partial_layouts_are_not_rejected` 第 ⑤ 组 + 反向锁 `campaign_scoped_conversations_still_require_campaigns_json` |
| **N-R1-02** `cards.json=[]` + campaigns 非空 → 整棵 campaign 树静默丢成空库且 `Completed` | P2 | **已修复（采纳 fail-closed）** | `readiness.rs:254-279`（Rule A′，`cards_file_present && cards_total == 0 && campaigns_total > 0`） | `empty_cards_json_with_all_campaigns_dangling_is_rejected`（原 `..._is_skipped_with_audit` 改写） |
| **N-R1-04** 守卫用 `Path::exists()`，stat 失败被当成「不存在」 | P3 | **已修复** | `readiness.rs:113-152`（`PathPresence` + `path_presence` + `stat_error_means_missing`）、`error.rs:47`（新错误变体）；8 个调用点见 §3.2 | `readiness.rs::path_presence_tests`（3 条单测） |

**核心口径**：这三条判据的方向都是「**读不了 / 父级全丢 ⇒ 拒绝，不静默发布空权威**」；已修复的每一处都配了**失败可控**证据（反转判据 → 测试必红，见 §1.4 / §2.4）。

---

## 1. N-R1-01（P1）Rule B 收窄：只有 campaign 归属的会话才依赖 `campaigns.json`

### 1.1 根因

`crates/infra-sqlite/src/readiness.rs` 的 S-01 守卫（`:232` 起）在 `campaigns.json` **缺失**时统计「依赖集合」是否非空，其中一条是 `("conversations", conversations.len())`。但 conversations 是 legacy 布局里**唯一允许非 campaign 归属**的集合：

- `strict_validate_entries` 的 conversations 分支用 `opt_str_strict(kind, index, item, "campaign_id")`（`:1021` 附近）→ `campaign_id` 可为 `null` / 缺失（非 Campaign 聊天）；
- `instances` / `knowledge` / `tasks` / `round_summaries` / `turns` 用 `req_str(..., "campaign_id")`（`:880/887/911/940/953`）→ **领域必填**，`len()` 就是 campaign 依赖数，不需要过滤。

于是合法布局「`cards.json` + `conversations/`（`campaign_id: null`）+ **无** `campaigns.json`」被判 `ImportSourceIncomplete`。默认后端是 SQLite（`storage_backend.rs:1882/1916/1929`），因此**非 Campaign 老用户升级后启动即失败**（P1）。

### 1.2 修法

```rust
// readiness.rs:155-165
/// 会话是否绑定到某个 Campaign（`campaign_id` 是非空字符串）。
/// N-R1-01：conversations 是 legacy 布局里**唯一**允许非 Campaign 归属的集合…
fn conversation_is_campaign_scoped(conversation: &Value) -> bool {
    conversation.get("campaign_id").and_then(|v| v.as_str())
        .is_some_and(|id| !id.trim().is_empty())
}
```

Rule B（`readiness.rs:280-301`）改为：

```rust
let campaign_conversations = conversations.iter()
    .filter(|c| conversation_is_campaign_scoped(c)).count();
for (name, count) in [
    ("instances", instances.len()), ("knowledge", knowledge.len()),
    ("tasks", tasks.len()), ("round_summaries", summaries.len()),
    ("turns", turns.len()), ("conversations", campaign_conversations),
] { … }
```

错误文案仍以 `conversations=N` 报数，但 N 的含义收窄为「campaign 归属的会话数」——**不被削弱**：只要有一条 campaign 归属会话存在，缺 `campaigns.json` 依旧 `ImportSourceIncomplete`。

### 1.3 测试

1. `crates/infra-sqlite/tests/importer_diagnostics.rs:1410` `legitimate_empty_and_partial_layouts_are_not_rejected` — 第 ⑤ 组新增 `legacy-conversations-no-campaigns`（`cards.json` + `conversations/conv-legacy.json`（`campaign_id: null`、`nodes: []`、RFC3339 时间戳）+ 无 `campaigns.json`），断言：`validate_source_manifest` Ok、`issues` 为空、导入 `ImportStatus::Completed`、`skipped_orphan_rows == 0`，并**额外**断言 `imported.conversations == 1`（会话真的落库，而不是被当孤儿丢掉）。
2. `importer_diagnostics.rs:1502` `campaign_scoped_conversations_still_require_campaigns_json` — 反向锁：`campaign_id: "camp-1"` 的会话 + 无 `campaigns.json` ⇒ readiness 与 importer **都**返回 `ImportSourceIncomplete`，且文案点名 `conversations=1`。

### 1.4 失败可控（负控）证据

临时把 Rule B 反转回旧判据（`.filter(|_c| true)`，等价 `conversations.len()`）后：

```
cargo test -p storyforge-infra-sqlite --test importer_diagnostics -- \
  legitimate_empty_and_partial_layouts_are_not_rejected \
  campaign_scoped_conversations_still_require_campaigns_json
→ test legitimate_empty_and_partial_layouts_are_not_rejected ... FAILED   (importer_diagnostics.rs:1484)
  test campaign_scoped_conversations_still_require_campaigns_json ... ok   ← 反向锁两态都 ok（设计如此）
  test result: FAILED. 1 passed; 2 failed; exit 101
```

反转已还原，还原后三条定向用例 `3 passed; 0 failed; exit 0`（§4 第 5 行）。**第 ⑤ 组是判别性用例**，不是「怎么改都绿」的装饰。

### 1.5 启动路径闭环（为什么不再需要改 app 层）

`lib.rs:1229-1230` → `storage_backend::resolve_backend` → `resolve_backend_inner`（`:1916`）→ `run_sqlite`（`:1929`）→ `recover_or_verify(&request).map_err(BackendWiringError::Cutover)`（`:1938-1939`）。S-01/S-17 的审计钩子（`:1941-1965`）只对**成功完成但发生跳过**的报告记 `cutover_skipped_orphans` 非阻断事件；导入错误本身**没有 JSON 回退分支**（R3 已复检 `run_sqlite` 尾部 `:2028/:2033/:2044`）。因此：

- 修复前：非 Campaign 老用户 → `ImportSourceIncomplete` → 启动失败（P1 症状）；
- 修复后：同一路径 → 正常导入 → 启动成功；
- 反向锁证明：真正的源不完整仍会在同一路径上失败，且错误可被用户看到。

---

## 2. N-R1-02（P2）父级全丢：裁定 = **fail-closed**（而非仅升级健康事件）

### 2.1 证据链（为何可以 fail-closed）

1. **正常完成路径不会产生该形态**：`campaign_store.rs::delete_card`（`:298`）在同一次「快照 + 候选副本 + 补偿」写里级联删 cards→campaigns→instances→knowledge→tasks→summaries→mvu→world_info，落盘顺序是 **`cards.json` 先（`:377`）、`campaigns.json` 后（`:378`）**；任何**正常结束**（成功或补偿回滚）的删除都会让两文件一致。
2. **可能产生该形态的都是「源目录不完整」**：硬崩溃/断电恰好落在 `:377` 与 `:378` 之间、外部部分拷贝/恢复中断、磁盘故障、手工编辑。前者的用户意图是「删最后一张卡」，后者是 S-01 要拦的破坏态。
3. **放行的代价不对称**：该形态下每个 campaign 都悬空 → `filter_campaigns_without_card` 全丢 → 依赖行按孤儿全跳过 → **0 campaigns 落库 + `ImportStatus::Completed` + 权威 marker 永久固化**（整棵 campaign 树静默消失）。
4. **误伤面**：仅「删最后一张卡时进程被强杀」这一条罕见路径，且错误文案给出可操作处置（恢复 `cards.json`，或确认后清理 `campaigns.json` 重试）。
5. **首轮边界的判词不成立**：原测试注释称该形态是「用户确实删掉了唯一那张卡」的合法数据，但按 2.1.1 的级联+顺序，正常删除不会留下悬空 campaigns；R1 的复检结论与此一致。

**备选（阻断级健康事件）对比**：`storage_health::record_backend_incident`（`storage_health.rs:78`）是 `blocking: false`；升级为 blocking 需要新的记录入口 + 前端 `StorageHealthGate` 交互路径，而导入已经把数据丢光了——「先丢数据再提示确认」不是等价方案。故采纳 fail-closed。若 Lead 更偏好「不阻断启动」，回滚点很小（删掉 §2.2 的判据块即可，测试同步改回跳过+审计），见 §5。

### 2.2 判据（刻意收窄，不溢出到合法老数据）

```rust
// readiness.rs:271
if cards_file_present && cards_total == 0 && campaigns_total > 0 {
    return Err(SqliteError::ImportSourceIncomplete(format!(
        "cards.json exists but is empty (0 cards) while campaigns.json contains \
         {campaigns_total} campaign(s); every campaign would be dropped as a dangling \
         orphan and the whole campaign tree would be lost — refusing to publish an empty \
         authority (restore cards.json, or delete the stale campaigns.json if the campaigns \
         were really removed) and retry"
    )));
}
```

- `cards_total` / `campaigns_total` 都是**过滤前**的原始长度，只依赖「文件存在性 + 原始条数」，不看过滤结果；
- **不收紧的反例**（仍在测试里断言）：`cards.json` 非空 + 个别 campaign 引用缺失卡（`save_card` 按 `source_character_id` 覆盖去重会留下悬空引用）⇒ 仍走 Gate 8 P2-A3 的「跳过 + 计数」；
- **不收紧的相邻用例**：`empty_campaigns_json_with_child_rows_is_skipped_with_audit`（`campaigns.json` 存在但为空 + 子行残留）不受影响——该判据要求 `campaigns_total > 0`。

### 2.3 测试改写

`empty_cards_json_with_all_campaigns_dangling_is_skipped_with_audit` → **`empty_cards_json_with_all_campaigns_dangling_is_rejected`**（`importer_diagnostics.rs:1159`），断言四层：

1. `validate_source_manifest` ⇒ `ImportSourceIncomplete`，文案含 `cards.json exists but is empty`（区别于「缺失」误报）；
2. importer ⇒ 同错误，且 `campaigns` 表 0 行、`import_runs` 无 `completed` 记录；
3. cutover ⇒ 拒绝，且**不发布 DB、不写 marker**（`inspect_marker == Absent`）；
4. 反例对比：`cards.json` 非空 + 一条悬空 campaign ⇒ `Completed` / `campaigns == 1` / `skipped_orphan_rows == 1`。

### 2.4 失败可控证据

临时把判据改成 `if false && …`（等价「移除 Rule A′」）后：

```
cargo test -p storyforge-infra-sqlite --test importer_diagnostics -- \
  empty_cards_json_with_all_campaigns_dangling_is_rejected
→ test ... FAILED  (importer_diagnostics.rs:1179)  exit 101
```

还原后绿。原「跳过+审计」判据下该用例必红，说明它钉的是**行为**而不是实现细节。

---

## 3. N-R1-04（P3）`Path::exists()` 把「读不了」吞成「不存在」

### 3.1 修法：三态判定 + 独立错误

```rust
// readiness.rs:113-152
pub(crate) enum PathPresence { Missing, Present }
pub(crate) fn path_presence(path: &Path) -> Result<PathPresence> {
    match fs::metadata(path) {
        Ok(_) => Ok(PathPresence::Present),
        Err(e) if stat_error_means_missing(&e) => Ok(PathPresence::Missing),
        Err(e) => Err(SqliteError::ImportSourceUnreadable(format!("{}: {e}", path.display()))),
    }
}
fn stat_error_means_missing(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}
```

```rust
// error.rs:42-47
/// N-R1-04：`Path::exists()` 把「读不了」吞成「不存在」…
#[error("import source unreadable: {0}")]
ImportSourceUnreadable(String),
```

**语义裁定：`unreadable` = fail-closed（独立错误）**。理由：optional 读法对「缺失」的定义是「空集合」（与 JSON `load_or_default` 同口径），把 stat 失败也当缺失就会静默把整个集合按空导入并发布空权威；而「缺失」与「读不了」的处置完全不同（前者是数据丢失，后者是权限/磁盘问题），因此必须可区分。`display` 文案为 `import source unreadable: <path>: <os error>`，经 `windows(…)?`/`?` 一路上抛，用户能看到具体路径与 OS 错误。

### 3.2 调用点（8 处，全部落在「导入源 + 权威判定」面）

| # | 位置 | 修复前 | 修复后 | 失败方向 |
| --- | --- | --- | --- | --- |
| 1 | `readiness.rs:168-171` `build_import_snapshot` data_dir 守卫 | `!data_dir.exists()` ⇒ `ImportSourceMissing` | `path_presence`；stat 失败 ⇒ `ImportSourceUnreadable` | fail-closed |
| 2 | `readiness.rs:224` `cards_file_present`（S-01 Rule A） | `.exists()` ⇒ 权限错判成「缺失」 | `!path_presence(...)?.is_missing()` | 读不了 ⇒ 显式错误，不误报「缺失」 |
| 3 | `readiness.rs:225` `campaigns_file_present`（S-01 Rule B 前置） | 同上 | 同上 | 同上 |
| 4 | `readiness.rs:2069` `read_json_array(_, optional)` | 读不了 ⇒ `Ok(vec![])`（**静默空库**） | 读不了 ⇒ `Err(ImportSourceUnreadable)` | fail-closed（本轮最关键） |
| 5 | `readiness.rs:565` `read_world_info_dir` | 读不了 ⇒ `Ok(vec![])`（世界书静默消失） | 同上 | fail-closed |
| 6 | `readiness.rs:2092` `read_conversation_dir` | 读不了 ⇒ `Ok(vec![])` | 同上 | fail-closed |
| 7 | `importer.rs:84` `import_data_dir_inner` 守卫 | `!data_dir.exists()` | `path_presence` | fail-closed |
| 8 | `cutover.rs:522` `legacy_json_layout_present`（fresh-start 探测） | 11 处 `.exists()`；任一 stat 失败 ⇒ 该文件「不存在」；全失败 ⇒ 判「全新用户」⇒ **建空库、跳过源校验** | `present(path) = !matches!(path_presence(p), Ok(Missing))`：**stat 报错按「存在」处理** | fail-closed（读不了 ⇒ 走正常 cutover，由 `validate_source_manifest` 用真实错误挡住启动） |

**顺带修掉的两个同类缺口**（同一判别函数，风险方向相反）：

- `cutover.rs:381` `inspect_marker`：原 `if !marker_path.exists()` ⇒ 带权威绑定的 marker 在 ACL 拒绝时被判 `Absent`（后续 cutover 可能覆盖它）。仓库**本就存在** `StaleKind::MarkerUnreadable`（"marker 文件读不出来（权限/占用）"，`cutover.rs:287`，且 read 错误分支已在用，`:401/:410`），此前只是被 `.exists()` 短路掉了；现改为 stat 失败 ⇒ `Stale { MarkerUnreadable }`（零枚举/零调用方变更）。
- `cutover.rs:899` `reconcile_marker_schema_version`：原 stat 失败 ⇒ `Ok(None)`（静默跳过版本对账）；现传播 `ImportSourceUnreadable`。两个调用点都把 `Err` 当 fail-closed（`sqlite_runtime.rs:108-109` 激活失败；`storage_backend.rs:2061-2076` 记录后仍走 Stale 硬失败），语义只增不减。

### 3.3 测试与平台限制

`readiness.rs:2204` 新增 `#[cfg(test)] mod path_presence_tests`（本文件首个单测模块）：

- `only_not_found_counts_as_missing`：`NotFound` ⇒ 缺失；`PermissionDenied` / `InvalidInput` / `TimedOut` / `Other` ⇒ **不算缺失**（判别函数的纯逻辑锁，绕开「平台能不能造出 EACCES」的问题）；
- `missing_and_present_are_distinguished`：真实临时目录上的缺失/文件/目录三态；
- `stat_failure_that_is_not_notfound_is_fail_closed`：带 NUL 的路径在两个平台上都会让 stat 立即失败且**不是** `NotFound` ⇒ 必须得到 `ImportSourceUnreadable`，绝不能退化成「缺失 = 空集合」。

**诚实记录的平台限制**：沙箱内无法构造真实 `EACCES`（Windows 下无 chmod/ACL 处置手段，且当前用户是所有者）；因此这条修复的判别证据是「判定函数 + 非法路径」两级，而**不是**一次真实的权限拒绝端到端复现。若要端到端复现，需要一个以受限账户运行、对 `cards.json` 单独 deny-Read-Attributes 的环境（记录为残留风险 R13-R1）。

### 3.4 残留 `Path::exists()` 清单（有意未改 + 风险方向）

| 位置 | 语义 | 方向 | 处置 |
| --- | --- | --- | --- |
| `cutover.rs:442` `!plan.db_path.exists()` ⇒ `Stale::DbMissing` | marker 声称 sqlite、DB 探测失败 | **fail-closed**（显式 Stale，交人工） | 保留 |
| `cutover.rs:852` `orphan_db_matches_authority` | stat 失败 ⇒ `false`（不匹配） | **fail-closed**（拒绝自动恢复） | 保留 |
| `cutover.rs:1718` publish 前 `!temp.exists()` ⇒ `Err` | temp 库缺失 | **fail-closed** | 保留 |
| `cutover.rs:1736` publish 时既有 `final_path` 让位判定；`cutover.rs:1935/2146` 原子写 helper | stat 失败 ⇒ 视为「不存在」⇒ 跳过让位/清理判定 | 极窄：需「目标文件 stat 被拒、但 rename/删除仍被允许」；且让位前有只读所有权探测 | 记录（R13-R2，未改） |
| `readiness.rs:1384-1385` / `:1450` 备份清单命名与覆盖检查 | stat 失败 ⇒ 视为未占用 ⇒ 可能覆盖同名备份清单 | 低（备份路径；写失败会报错，且清单覆盖不改变权威数据） | 记录（R13-R3，未改） |
| `exporter.rs:218/1637`、`rollback.rs:311/337/359/370/396/468`、`lease.rs` 断言 | 导出/回滚/租约工具的目标路径判定 | 多在工具面且方向为拒绝 | 记录，本任务范围外 |

---

## 4. 验证记录（本成员实际运行，含退出码与真实数字）

| # | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 1 | `cargo check -p storyforge-infra-sqlite --all-targets` | 0 | Finished（0 error） |
| 2 | `cargo fmt -p storyforge-infra-sqlite -- --check` | 0 | 无 diff |
| 3 | `cargo clippy -p storyforge-infra-sqlite --all-targets -- -D warnings` | 0 | Finished（0 warning） |
| 4 | `cargo test -p storyforge-infra-sqlite` | 0 | **24 个 target / 320 passed / 0 failed / 0 ignored** |
| 5 | 定向三例（⑤ 组 + 反向锁 + N-R1-02） | 0 | 3 passed / 0 failed / 26 filtered out |
| 6 | 失败可控负控（Rule B 反转） | 101（预期红） | 2 failed（`importer_diagnostics.rs:1484` 等）/ 1 passed；还原后绿 |
| 7 | 失败可控负控（Rule A′ 关闭） | 101（预期红） | 1 failed（`importer_diagnostics.rs:1179`）；还原后绿 |
| 8 | `cargo test -p storyforge --lib` | 0 | **477 passed / 0 failed / 3 ignored**（工作树含其它成员并行改动） |
| 9 | `cargo test -p storyforge --test backend_parity_suite` | 0 | 2 passed / 0 failed |
| 10 | `cargo test -p storyforge --test sqlite_optin_lifecycle` | 0 | 1 passed / 0 failed |
| 11 | `cargo test -p storyforge --test sqlite_command_lifecycle` | 0 | 1 passed / 0 failed |
| 12 | `cargo test --workspace`（99 套 / 2165 用例） | **未由本成员运行** | Lead 指令：不跑 workspace 级 cargo；引用 `fixes/GATE-REPORT.md` |
| 13 | `npm test` / vitest / build | **未由本成员运行** | 沙箱 npm `spawn EPERM` + Lead 指令；引用 `fixes/GATE-REPORT.md`（npm 532/532、vitest 31 文件/151） |

覆盖面校验：`cargo test -p storyforge-infra-sqlite` 的 24 个 target 里包含 `cutover.rs`（42）、`marker_reconcile.rs`（7）、`backend_selection.rs`（8）、`gate5_fault_matrix.rs`（10）、`gate5_migration_matrix.rs`（6）、`rollback.rs`（16）、`reverse_export*.rs`（15）等与本轮改动直接相关的二进制，均为 0 failed。

**规模说明**：`git diff --stat` 里 `readiness.rs`/`cutover.rs` 的行数包含 task-9（S-01…S-22）尚未提交的改动，不作为 R13 增量指标；R13 的增量 = 上表 §3.2 的 8 处调用点 + 新 helper/变体/测试（新增 4 条测试：3 单测 + 1 集成；改写 1 条）。

---

## 5. 残留风险 / 建议下一步

| 编号 | 内容 | 级别 | 建议 |
| --- | --- | --- | --- |
| R13-R1 | N-R1-04 无真实权限拒绝的端到端复现（仅判定函数级 + 非法路径级） | P3 | 若要端到端证据，需受限账户 + 对单文件 deny-Read-Attributes 的环境；或在 CI 用 Linux 容器跑 `chmod 000 cards.json` 一条集成测试 |
| R13-R2 | `cutover.rs:1736/1935/2146` 原子写路径仍用 `.exists()`（stat 失败 ⇒ 视为不存在） | P3 | 与 `path_presence` 统一；影响前提极窄（stat 被拒但 rename 允许），本轮未改以免扩大 P1 修复面 |
| R13-R3 | `readiness.rs:1384-1385/1450` 备份清单存在性判定同属该模式 | P3 | 备份面独立小改（fail-closed 更保守即可），建议并入后续 P3 清尾 |
| R13-R4 | N-R1-02 采纳 fail-closed 后，**极端路径**「删最后一张卡时被强杀」会看到启动被拒 | P2（已知代价） | 错误文案已给出两条可操作处置；若 Lead 更偏好不阻断启动，回滚点 = 删除 `readiness.rs:271-278` 判据块并把 `importer_diagnostics.rs:1159` 改回「跳过 + 计数」，其余改动不受影响 |
| R13-R5 | `ImportSourceUnreadable` 是新错误文本，前端/`TauriCommandError` 映射未新增分支 | P3 | 该错误发生在启动装配层（`BackendWiringError::Cutover` → 启动失败提示），不经命令层 DTO；若后续要细化用户指引，可在 app 层按 variant 分支给处置建议 |
| R13-R6 | 冻结件引用漂移：`round2/R5-p0-adversarial-reverify.md:180` 仍引用旧测试名 `empty_cards_json_with_all_campaigns_dangling_is_skipped_with_audit`（`importer_diagnostics.rs:1158-1188`） | P3（文档） | R5 属冻结件，本轮未改；该引用描述的是**修复前**的边界判定，作为历史证据仍成立，但按名字检索会找不到测试。建议 Lead 收口时在 R5 追加一行勘误指针（或引用本报告 §2.3） |

**建议下一步**：本任务闭环后回到 task-35（R12 W-11 下游 fail-open 裁定）；R13-R2/R3 可作为 P3 清尾并入下一次域2 小修（不阻断本轮）。

---

## 附：R13 改动文件与位置

| 文件 | 改动 |
| --- | --- |
| `crates/infra-sqlite/src/error.rs` | `:42-47` 新增 `ImportSourceUnreadable(String)`（含裁定理由注释） |
| `crates/infra-sqlite/src/readiness.rs` | `:113-152` `PathPresence` / `path_presence` / `stat_error_means_missing`；`:155-165` `conversation_is_campaign_scoped`；`:168-171` `build_import_snapshot` data_dir 守卫；`:224-225` S-01 文件存在性；`:254-279` N-R1-02 判据块（Rule A′，判据在 `:271`）；`:280-301` Rule B 收窄（过滤在 `:290-293`）；`:565` `read_world_info_dir`；`:2069/2092` `read_json_array` / `read_conversation_dir`；`:2195-2257` `path_presence_tests` 单测模块 |
| `crates/infra-sqlite/src/importer.rs` | `:81-85` data_dir 守卫改用 `path_presence` |
| `crates/infra-sqlite/src/cutover.rs` | `:519-545` `legacy_json_layout_present`（stat 错误按存在处理）；`:375-405` `inspect_marker` 的 stat 失败 ⇒ `StaleKind::MarkerUnreadable`；`:891-901` `reconcile_marker_schema_version` 传播 stat 错误 |
| `crates/infra-sqlite/tests/importer_diagnostics.rs` | `:1158-1262` N-R1-02 用例（改写，原 `..._is_skipped_with_audit`）；`:1409-1497` 第 ⑤ 组布局（`legacy-conversations-no-campaigns`，`:1461` 起）；`:1501-1537` N-R1-01 反向锁 |
| `docs/review-2026-09-13/fixes/02-storage-fixes.md` | 追加 `§13 R13`（本报告摘要 + 验证数字 + 残留风险） |
| `crates/tauri-app/src/storage_backend.rs` | **无改动**（§1.5：应用层已 fail-closed，错误经同路径上抛） |

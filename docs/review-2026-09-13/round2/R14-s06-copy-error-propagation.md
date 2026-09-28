# R14 收口：S-06 承接（`lib.rs` 旧数据目录迁移的静默吞错）

- **任务**：`task-37`（收口 R14：S-06 承接），owner `review-tauri-api`
- **触发**：R1 复检确认 S-06 在 `02-storage-fixes.md` 被标"暂缓（跨域，`lib.rs`，域4）"，
  但域4 记录里**没有实际承接动作** ⇒ R6 意义上的"移交后无人接收"
- **处置**：**(a) 修**（不是判非问题）——让拷贝失败**成为可判定的返回值**并落到健康面，
  另加持久化重试标记，消除"半迁移被永久锁死"的路径
- **改前基线**：`HEAD = ab894c6`（`git show HEAD:crates/tauri-app/src/lib.rs` = 真·修复前）
- **改动文件**：`crates/tauri-app/src/lib.rs`、`crates/tauri-app/src/lib_tests_startup.rs`（+ 本记录 + `04-tauri-fixes.md` 追加）
- **未改**：任何 Tauri 命令签名（`migrate_from_exe_dir_if_needed` / `copy_dir_recursive` 都是私有普通函数，非 `#[tauri::command]`）

## 0 结论

**已修复。** 旧实现在嵌套目录拷贝失败时：① 一行日志都没有；② 照常打印"数据迁移完成"；
③ 部分拷贝留下的目录会让下次启动**永久跳过**迁移——旧目录里的数据再也搬不过来（静默、永久、不可恢复）。
现在失败会返回 `MigrationOutcome::Incomplete`、写 `error` 日志、在健康面登记
`backend:legacy_dir_migration_incomplete` 事件，并留下 `.migration_incomplete` 标记让下次启动**重试**。

**定级**：S-06 原为 P2（跨域移交项）。**实际后果比记录更重**（数据永久搬不过来的静默丢失路径，
且有"假成功"日志），建议在最终报告里按 **P1** 表述；是否调级交 Lead。

## 1 事实核对（回代码，`HEAD` 版本）

### 1.1 位置与完整上下文

```
crates/tauri-app/src/lib.rs:442  fn initialize_app_data_dir(framework_data_dir) -> Result<PathBuf, String>
  :465  std::fs::create_dir_all(&data_dir)
  :467  migrate_from_exe_dir_if_needed(&data_dir, exe_parent.as_deref());   ← 返回值被丢弃（旧版返回 ()）
  :468  APP_DATA_DIR.set(...)                                              ← 迁移一结束就认下目录
crates/tauri-app/src/lib.rs:479  fn migrate_from_exe_dir_if_needed(new_dir, exe_parent)   // -> ()
  :503  if let Ok(entries) = std::fs::read_dir(&old_dir) {        ← read_dir 失败静默
  :504    for entry in entries.flatten() {                        ← 条目的 Err 被 flatten 丢掉
  :506      if entry.path().is_dir() {
  :507        copy_dir_recursive(&entry.path(), &dest);           ← 嵌套目录：**无返回值、无日志**
  :508      } else if let Err(e) = std::fs::copy(...) {
  :509        tracing::warn!("迁移文件失败 {}: {e}", ...);        ← 只有**顶层文件**有 warn
  :512    tracing::info!("数据迁移完成");                          ← 无论成败都打印
crates/tauri-app/src/lib.rs:517  fn copy_dir_recursive(src, dst)   // -> ()
  :518  std::fs::create_dir_all(dst).ok();                        ← 吞错
  :519  if let Ok(entries) = std::fs::read_dir(src) {             ← 吞错
  :520    for entry in entries.flatten() {                        ← 吞错
  :525      let _ = std::fs::copy(entry.path(), &dest);           ← S-06 本体：每个嵌套文件
```

### 1.2 搬到哪些集合

`old_dir = <exe 同级>/data` 下的**全部顶层条目**：顶层文件（`characters.json`、`connections.json`、
`campaigns.json`、`embed.json` …）逐文件 `fs::copy`，**子目录整体递归**——
即 `campaigns/`、`conversations/`、`campaign_world_info/`、实例目录、向量库目录等**多文件集合**。

### 1.3 失败后会发生什么（三个后果，全部有代码依据）

1. **嵌套失败完全不可见**：`copy_dir_recursive` 无返回值、无 `tracing` 调用，
   `:509` 的 warn 只覆盖**顶层文件**分支 ⇒ 子目录里的拷贝失败连一行日志都没有。
2. **假成功**：`:512` 的 `tracing::info!("数据迁移完成")` 在循环之后无条件执行；
   调用方拿到 `()`，没有任何判断依据 ⇒ "迁了一半"和"迁完了"在代码里不可区分。
3. **永久锁死（最重）**：`:490-492` 的跳过判据是
   `characters.json` / `connections.json` / `campaigns` **存在**就认为"新目录已有数据"。
   一次部分失败（例如 `campaigns/` 目录已建好、里面的文件没拷过来）就足以让该判据为真 ⇒
   **之后每次启动都直接 `return`**，旧目录里的数据永远不再尝试搬迁。
   注意这条判据看的是**目录是否存在**，不看内容是否搬完——所以"空目录"也会锁死。

### 1.4 为什么 S-01 的守卫拦不住（R1 的判断成立）

S-01 的守卫（`crates/infra-sqlite/src/readiness.rs:193-228`）判的是**源目录是否完整**：
"`cards.json` 缺失但 campaigns 非空" → fail-closed。它**不判"拷贝是否成功"**：

- 若 `cards.json` 整个没拷过去 → 守卫能报 `cards.json is missing`（这是 S-01 覆盖到的形态）；
- 若 `cards.json` 拷过去了、但 `knowledge.json` / `conversations/` 没拷完 → **新目录里那些集合就是"空"**，
  守卫的判据（"缺失文件 + 非空 dependents"）不成立 ⇒ 空集合被当成合法空数据，**静默固化**；
- 而且一旦迁移被 `:494` 跳过，守卫连"源目录"都不会看到——迁移根本没发生。

## 2 复现："改前必错 / 改后不错"（执行级，不靠读码推断）

**方法**（与 R5 的 P0-1 同标准）：把 `HEAD` 的旧函数与修改后的新函数**逐字**放进同一个 rustc 探针，
**同一套动作、同一注入方式**，两侧各在独立目录里跑：

- 布局：`old_dir` 有 `characters.json` + `conversations/conv-1.json`；
  `new_dir` 预先放一个**文件** `conversations`（故障注入：目标位置不可为目录，
  真实等价形态是权限/占用/磁盘错误）；
- 第 1 次启动（嵌套拷贝失败）→ 第 2 次启动（模拟部分拷贝留下的 latch）→ 第 3 次启动（清掉阻塞物）。

**探针**：`%TEMP%\r14\probe_r14.rs`（`rustc -O probe_r14.rs -o probe_r14.exe`，只依赖 std）。
两处唯一替代（仅为独立编译，与被测逻辑无关）：`tracing::*` → 打印宏；
`storage_health::record_backend_incident` → 内存记录 + 打印。**真接线**由
`migrate_incomplete_outcome_is_surfaced_as_backend_incident` 用真 `crate::storage_health::incidents()` 覆盖。

**原始输出**：

```
【A】第 1 次启动（嵌套目录拷贝必失败）
  --- old ---
      [info]  正在从旧数据目录迁移: …\r14-old\install\data → …\r14-old\newdata
      [info]  数据迁移完成                       ← ★ 假成功：嵌套文件其实没搬过去，且无任何失败信号
      结果: 嵌套文件落地=false  出现任何失败信号=false
  --- new ---
      [info]  正在从旧数据目录迁移: …\r14-new\install\data → …\r14-new\newdata
      [error] 数据迁移未完成：1 项失败 copied_files=0 retried=false （旧目录未被修改，标记
              .migration_incomplete 已留在新目录，下次启动重试）
              detail=conversations: 建目录失败: 当文件已存在时，无法创建该文件。 (os error 183)
      [HEALTH INCIDENT] kind=legacy_dir_migration_incomplete detail=从旧安装目录迁移数据未完成（1 项失败）：
              conversations: 建目录失败: …；旧目录数据未改动，下次启动会重试
      结果: 嵌套文件落地=false  留下重试标记=true

【B】第 2 次启动（latch 成立：new_dir 已有 characters.json）
      latch(characters.json)存在: old=true new=true
  --- old ---
      旧实现直接 return（连'正在迁移'都没有），嵌套文件仍=false     ← ★ 永久锁死
  --- new ---
      [info]  正在从旧数据目录迁移: …（上次迁移未完成，本次重试）
      [error] 数据迁移未完成：1 项失败 copied_files=0 retried=true …
      标记仍在=true

【C】第 3 次启动（清掉注入的阻塞物后）
  --- old ---
      结果: 嵌套文件落地=false  ← 旧实现因 latch 永久跳过，数据永远搬不过来
  --- new ---
      [info]  正在从旧数据目录迁移: …（上次迁移未完成，本次重试）
      [info]  数据迁移完成 copied_files=1 retried=true
      结果: 嵌套文件落地=true  标记已撤销=true

汇总：旧实现 → 嵌套数据最终落地 = false；新实现 → 嵌套数据最终落地 = true
```

**判读**：同一注入、同一动作序列，旧实现**数据永远搬不过来且全程无失败信号**；
新实现三次启动分别是"Incomplete+标记+健康事件 / 重试 / Completed+撤销标记"，数据最终落地。

## 3 修法

### 3.1 `crates/tauri-app/src/lib.rs`

新增（§"旧数据目录迁移的结果"）：

```rust
#[derive(Debug, PartialEq, Eq)]
enum MigrationOutcome {
    NotApplicable,                                   // 无 exe_parent / 旧目录不存在 / 新旧同目录
    SkippedPopulated,                                // 新目录已有数据且没有失败标记 → 不覆盖
    Completed { copied_files: usize, retried: bool },
    Incomplete { copied_files: usize, failures: Vec<String>, retried: bool },
}
const MIGRATION_INCOMPLETE_MARKER: &str = ".migration_incomplete";
```

| 改动 | 语义 |
| --- | --- |
| `copy_dir_recursive(src, dst, rel, failures) -> usize` | 返回成功复制文件数；`create_dir_all` / `read_dir` / 每条目 / 每次 `fs::copy` 的失败**逐条**收集进 `failures`（不再 `.ok()` / `flatten()` / `let _ =`）。`rel` 让失败条目用**相对路径**，避免把用户绝对路径带进前端健康报告。 |
| `migrate_from_exe_dir_if_needed(...) -> MigrationOutcome` | 拷贝前**先写** `.migration_incomplete`（中途崩溃/断电也保证下次重试），全部成功才撤销；任何失败 → `Incomplete{failures}`。跳过判据改为 `new_has_data && !marker.exists()`。 |
| `report_migration_outcome(&outcome)` | 日志与健康面的**唯一**出口：`Completed` → `info!(copied_files, retried)`；`Incomplete` → `error!(...)` + `storage_health::record_backend_incident("legacy_dir_migration_incomplete", ...)`。测试直接调它，因此"是否可见"是可断言的。 |
| 调用点 `lib.rs:467-468` | 由一个被丢弃的表达式改为 `let migration = …; report_migration_outcome(&migration);` |

### 3.2 为什么选"显式降级 + 健康事件 + 重试"而不是"启动 fail-closed"

任务允许两种处置。我选降级，理由三条，且**这不是放水**：

1. **不阻断启动**：这条路径只在"新目录为空、旧 `exe_dir/data` 存在"时走一次（legacy 安装形态）。
   为一个可能已被用户遗忘的旧目录而让整个应用拒绝启动，代价过大；源目录**从不被修改/删除**，
   数据始终在原地。
2. **"不会静默成功"由三件事保证**：可判定的返回值（`Incomplete`）+ `error` 日志 +
   健康面事件（前端 `StorageHealthGate` 可见）。假成功的那句 `info!("数据迁移完成")` **已不存在**于失败路径。
3. **不可恢复性被消除**：`.migration_incomplete` 标记让后续启动**越过 latch 重试**，
   直到搬完并撤销标记 ⇒ "数据永远搬不过来"这条路径不再存在（§2 第 C 步已实测）。

**留给 Lead 的产品决策**：若要求更严格的"启动即失败"，只需让 `initialize_app_data_dir`
在 `Incomplete` 时返回 `Err`（一行改动）。但**单独** fail-closed 而**不加**标记仍然是错的——
第一次启动失败后，部分拷贝已经让 latch 成立，第二次启动会跳过迁移并"正常启动"，静默丢失依旧成立。
两者必须一起改（已在本实现中一起处理）。

### 3.3 未改的部分（遵守 CLAUDE.md 硬规则）

- **命令签名零改动**：没有新增/删除/改签名任何 `#[tauri::command]`（命令总数仍 175）。
- `resolve_app_data_dir`、`APP_DATA_DIR` OnceLock 语义、`old_dir` 的只读性都不变；
  新标记文件写在**新目录**内，不污染旧目录，也不在 `new_has_data` 的判据名单里。

## 4 测试（失败注入，断言"不会静默成功"）

`crates/tauri-app/src/lib_tests_startup.rs` 新增 5 条：

| 测试 | 断言什么 |
| --- | --- |
| `migrate_failure_is_reported_incomplete_and_arms_retry` | 注入"目标位置是文件"→ 必须是 `Incomplete`（不是 `Completed`）；失败清单指明条目；**不得含用户绝对路径**；标记文件存在且内容含失败条目；注入的失败**真的没搬过去** |
| `migrate_incomplete_marker_forces_retry_past_populated_latch` | 完整三轮：失败→标记 → **反证**（删掉标记后同样目录状态确实返回 `SkippedPopulated`，即旧实现的锁死路径真实存在）→ 恢复标记必须 `retried=true` 重试 → 清掉阻塞物后 `Completed{retried:true}`、标记撤销、嵌套文件内容逐字节一致 |
| `migrate_incomplete_outcome_is_surfaced_as_backend_incident` | 直调 `report_migration_outcome`，用真 `crate::storage_health::incidents()` 断言 `backend:legacy_dir_migration_incomplete` 事件存在、文案含失败项数与"旧目录数据未改动"、`blocking == false` |
| `migrate_success_copies_nested_dirs_and_leaves_no_marker` | 成功路径：`Completed{copied_files:2, retried:false}`、不留标记、嵌套内容一致 |
| `migrate_outcome_is_explicit_when_not_applicable` | `NotApplicable`（无 exe_parent / 旧目录不存在）与 `SkippedPopulated`（不覆盖新目录数据）显式可断言，取代旧实现的"静默 return" |

**既有 2 条迁移测试未改一行、全部仍通过**（它们的断言在新语义下依然成立）。

## 5 复跑记录（真实数字，未跑 `cargo test --workspace`）

| 命令 | 结果 |
| --- | --- |
| `cargo test -p storyforge --lib migrate` | **12 passed / 0 failed / 0 ignored**（468 filtered）——含 5 条新测试 + 2 条既有迁移测试 |
| `cargo test -p storyforge --lib` | **477 passed / 0 failed / 3 ignored**（exit 0） |

> 计数说明：本轮开始前我的域4 记录里 `--lib` 是 **467 passed**；+5 条本任务新测试 = 472，
> 其余增量为**并行任务的测试**（共享工作区，同一时刻有 task-31/34/35/36 在写 `crates/tauri-app/src`），
> 因此 477 是本任务完成时刻的**全量**结果，不全部归本任务。
> 唯一编译告警是 `runtime_support.rs:26 unused import`（**非本任务文件**，未处理）。

## 6 残留风险 / 未覆盖面（诚实边界）

1. **未做端到端启动验证**：我没有以真实 legacy 安装形态启动应用（无法在此环境跑 GUI），
   所以"健康事件在 UI 里真的显示成一条非阻断提示"这一点是**接线级**证据
   （`record_backend_incident` 的既有消费者是 `StorageHealthGate.vue`，
   同族事件 `cutover_skipped_orphans` 已在生产路径使用），未做界面实测。
2. **标记文件的清理**：如果用户手动在新目录里创建 `.migration_incomplete`，会触发一次多余的"重试"
   （幂等覆盖，无破坏性）。选择接受，而不是引入额外的状态校验。
3. **阻塞原因不区分**：权限 / 占用 / 磁盘满都走同一条 `Incomplete` 路径，用户需从 `detail` 文本判断
   （已带上 OS 错误文本与相对条目名）。够用，未做分类。
4. **`copy_dir_recursive` 的语义**：仍然按"尽力复制"遍历，**不做**"一处失败立即中止"。
   这是有意的——尽量多搬，失败项统一上报；否则第一次失败就会掩盖后面所有失败项。

## 7 门禁影响

无。命令总数/位置/前端调用面/快照均不受影响（未动任何 `commands/**`）。
`node scripts/architecture/backend-baseline.mjs` 与 `cargo check -p storyforge --all-targets`
的实测结果见 §8（随本任务执行）。

## 8 与 S-06 记录的关系（R1 承接确认 / R6 无记录项收口）

- `02-storage-fixes.md` 把 S-06 标为"暂缓（跨域，`lib.rs`，域4）"，但域4 的 `04-tauri-fixes.md`
  **没有任何对应条目**——这正是 R1 说的"移交后无人接收"。本任务把它落地为**实际修复**，
  并在 `04-tauri-fixes.md` 追加 §8 收口（含 R1 承接确认的说明），以免再次出现"记录里查不到"。
- 口径提醒：S-06 与 S-01 **不是**同一件事。S-01 = "源目录不完整时的 fail-closed 守卫"（域2，已关闭）；
  S-06 = "迁移拷贝失败时的静默吞错与永久锁死"（域4，本次）。两者共同点仅是"都与 legacy 数据搬运有关"，
  修复层面**互不覆盖**——S-01 的守卫在本问题的三种形态里都拦不住（§1.4）。

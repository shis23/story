# 修复记录：域4 Tauri 命令层与前后端契约（task-11，T-01..T-16）

- 修复人：review-tauri-api（域4）
- 任务：task-11「修复域4：Tauri 命令层与前后端契约（T-01..T-16）」
- 依据：`docs/review-2026-09-13/04-tauri-commands.md`（域4 审查报告，P0=0 / P1=3 / P2=9 / P3=4）
- 起始 HEAD：`ab894c6`
- 工作区状态：多域并发修复中（同一工作树由 7 个域 + Lead 并行修改）

## 0 状态汇总

| 状态 | 数量 | 条目 |
| --- | --- | --- |
| 已修复 | 8 | T-01、T-02、T-04、T-06、T-07、T-08、T-12、T-13/T-16 |
| 已修复(降级方案) | 2 | T-15（保留+声明，Lead 裁定）、T-11（仅补 T-02 相关命令的落盘回归测试） |
| 已修复(未运行时验证) | 1 | T-10（CI 接入；**保留**"建议打预发布 tag 或 `workflow_dispatch` 干跑"的验证路径，不得写成已验证） |
| 移交他域 | 4 | T-03→域6、T-05→域6、T-09→域6（前端 wrapper 已由域5 删除）、T-14→task-16 |
| 判定非问题 | 0 | — |
| 顺带新发现并修复 | 1 | N-01（基线脚本把注释里的 `#[tauri::command]` 计成命令） |
| **暂缓（产品决策）** | 1 | 20 个零入口后端命令：本轮保留 + 声明、不删除（Lead 裁定，另行列为 **N-02（P2）**，见 §6.3） |

**命令总数保持 175 不变**（T-15 采纳"保留 + 声明"，未删除任何命令）。

附注：域5 按 Lead 裁决删除 20 个死 wrapper 后，**零入口后端命令从 3 个增至 23 个**，
全部已在本域 `RETAINED_NO_FRONTEND_CALLER` 显式声明（20 条新增，见 §6.3）。
Lead 已裁定**本轮保留 + 声明、不删除**；风险（P0-2 未关闭前等同额外 IPC 攻击面）记为 **N-02（P2）**，
后续若删除按 §3 T-15 的连删清单执行。

---

## 1 P1 逐条

### T-01 契约测试 invoke 扫描盲区 → 已修复

- **修法**：`scripts/architecture/backend-baseline.mjs` 的 `extractFrontendInvokes` 把
  `\._invoke\(`（要求字面量点号）改为 `(?<![\w])_invoke\(`，同时覆盖 `this._invoke(...)`
  与 `shellDocUrl.js` 的模块局部绑定裸调用 `_invoke(...)`；并在正则上方写明为什么
  `\binvoke\(` 不会与之重复匹配（`_`/`i` 之间无词边界）。
- **修法(前端)**：`frontend/tests/tauri-command-contract.test.mjs`
  - 新增 `Gate 8 review: the invoke scan catches bare _invoke(...) call sites`：显式断言
    `card_shell_register_doc` / `card_shell_register_module` / `card_shell_unregister_doc`
    必须出现在 `invokedCommands` 中（扫描一旦回退这条立刻红），并断言
    `uniqueInvokeCount == invokedCommands.length` 自洽。
  - **删除硬编码的 `uniqueInvokeCount === 169`**（Lead 建议）：该数字是扫描中间产物而非契约，
    且跨域并发增删 wrapper 时变动频繁。契约改由三条不变量承担：
    ① `missingBackendCommands == []`；② `invokedCommands ∪ retainedNoFrontendCaller == registeredCommands`；
    ③ 上面那条 T-01 回归锁。维护者与事实来源已写进断言旁注释。
- **验证**：
  ```
  node scripts/architecture/backend-baseline.mjs
  → [backend-baseline] 定义 175 / 注册 175 / 前端唯一 invoke 172 / 孤儿命令 3
  → [backend-baseline] 门禁通过          （exit 0）
  ```
  `uniqueInvokeCount` **169 → 172**，且 3 个 `card_shell_*` 均已进入 `invokedCommands`
  （修复前 `grep 'card_shell_register_doc' → invokedCommands: MISSING`，修复后 `IN`）。
  注：域5 随后按 Lead 裁决删除 20 个死 wrapper，该数进一步降为 **152**（见 §6.3）——说明它**不该**被硬编码成断言。
  ```
  cd frontend && node tests/tauri-command-contract.test.mjs
  → tests 11 / pass 11 / fail 0        （exit 0）
  ```
- **扫描结果不可回退的原因**（诚实说明）：CI 里跑的是 `node --test tests/tauri-command-contract.test.mjs`，
  断言 ② 会在"新增孤儿未登记"或"声明过期"时失败，断言 ③ 会在"裸 `_invoke` 扫描被改回去"时失败。

### T-02 Card Studio 8 处落盘错误被吞 → 已修复（无降级）

- **修法**：`crates/tauri-app/src/card_studio_api.rs`。新增两个 helper，替换全部 8 处
  `let _ = store.update(...)`（原 `:308 :361 :486 :574 :579 :710 :718 :814`）：
  ```rust
  /// 命令侧唯一允许的落盘写法：保证"返回 Ok"等价于"已写入磁盘"。
  fn persist_project(store: &CardStudioStore, project: CardProject)
      -> Result<CardProject, TauriCommandError> {
      store.update(project).map_err(TauriCommandError::storage)
  }
  /// 已在返回 Err 的路径上：失败状态尽力保存，但落盘失败必须留痕、且不覆盖原始错误。
  fn persist_failure_state(store: &CardStudioStore, project: CardProject) {
      if let Err(e) = store.update(project) {
          tracing::error!(error = %e, "写卡项目失败状态落盘失败（原始错误仍返回给前端）");
      }
  }
  ```
  分流原则（写进注释）：
  - **3 处成功返回路径**（`cardstudio_run_review` 的纯规则分支与 LLM 分支结尾、
    `cardstudio_import_compiled` 结尾）→ `persist_project(...)?`，落盘失败即返回 `Err(Storage)`；
  - **5 处错误返回路径**（`run_review:361`、`complete_manual_stage:486`、`prefill_from_novel:574/579`、
    `run_stage:710/718`）→ `persist_failure_state`，原错误优先返回、落盘失败升为 error 级日志。
- **测试**（`card_studio_api.rs` 的 `#[cfg(test)] mod tests`，3 个新用例）：
  | 测试名 | 覆盖 |
  | --- | --- |
  | `update_reports_error_when_persist_fails` | 基础不变量：把 `card_projects.json` 换成同名目录使 `atomic_write` 的 rename 必然失败 → `CardStudioStore::update` 返回 Err |
  | `persist_project_propagates_disk_failure_as_command_error` | 命令侧：落盘失败 → `TauriCommandError::Storage`，**命令不再返回 Ok** |
  | `persist_project_returns_ok_when_disk_write_succeeds` | 反向：落盘正常仍返回 Ok，防止把"传播错误"写成"无条件报错" |
- **状态**：**已实证**——`cargo check -p storyforge --all-targets` exit 0；`cargo test -p storyforge --lib card_studio`
  → `8 passed; 0 failed`（含上述 3 个新用例）。

### T-03 `card_shell_fetch_url` 非 async 阻塞 IPC → 移交域6

本域不处理（Lead 裁定：`card_shell*` 整体归域6）。精确改法已在 04 号报告 §T-03：
`commands/card_shell.rs:147-163` 改 `pub async fn`，并把 `card_shell_cache::fetch_blocking_with_client`
（`card_shell_cache.rs:250-255`，`reqwest::blocking` + 30s 超时 `:89`）包进
`tokio::task::spawn_blocking`。前端热路径为 `CardShellHost.vue:292` / `TavernHelperRuntime.vue:643`。

---

## 2 P2 逐条

### T-04 `add_variant` 参数契约漂移 → 已修复

- **修法**：`crates/tauri-app/src/commands/turns.rs` 的 `add_variant` 增加
  `provenance: Option<Provenance>` 形参并透传给 `conv_store.add_variant(..., provenance)`，
  替换原先硬编码的 `None`。
- **依据**：前端 `frontend/src/tauri-api.js:646` 一直在发 `provenance: provenance || null`，
  而 Tauri 只按 camelCase 键逐个取参、对多余键静默忽略 → 该参数此前永不生效。
  `Option<T>` 形参可缺省，故为向后兼容（前端唯一调用方 `useMessageVariants.js:469` 传 `null`）。
- **Hard Rule 例外说明**：CLAUDE.md 要求"保留既有 Tauri 命令签名，除非任务明确要求修改"——
  T-04 即是该任务授权，故改签名是被明确要求的；改动方向是"让签名与前端既有调用一致"。

### T-05 两处错误 DTO 不统一 → 移交域6

`commands/card_shell.rs:114-122` 的 `card_shell_register_doc` / `card_shell_register_module`
返回 `Result<_, String>`（149 个 Result 命令中仅此 2 个非 `TauriCommandError`）。该文件归域6。
精确改法：改为 `Result<String, TauriCommandError>`，超限→`validation`、其它→`internal`。
前端 `errorText.js` 对裸字符串有兜底，故非功能性故障。

### T-06 变量写命令无键名校验（`__storyforge*` 守卫只在前端）→ 已修复

- **修法**：`crates/tauri-app/src/commands/variables.rs` 新增
  `validate_variable_write_key(key)`（复用 `validate_campaign_variable_input` 的键名规则：
  非空 / ≤128 字节 / 仅文字·数字·点·下划线·连字符 / 任一段不得以 `__` 开头），
  在 `set_character_variable` 与 `set_campaign_variable` 的原子单元之前调用。
- **依据**：同文件的 crud 路径 `add_campaign_variable:179-184` 早已 normalize + validate，
  而 `set_*` 此前**零校验**；前端 `shellVariableOutbox.js:59-62` 的 M-6 守卫不是安全边界，
  而 `utils/mvuStatTree.js:16` 正是靠该前缀排除内部键。
- **未越界说明**：`crates/domain/**` 不在我的写作用域，故**未**在 domain 层加同校验。
  **留待域1 判断**：`Campaign::set_variable`（`crates/domain/src/campaign.rs:223`）与
  `CharacterInstance::set_variable` 是否也应内建同一守卫（当前只有命令层拦截，
  domain 可被其他调用方绕过）。
- **未处理 M-15（移交，附理由）**：域6 的 M-15 要求 `set_*_variable` 补
  `normalize_mvu_key` 归一化（消除 `stat_data.hp` 与 `hp` 双记法）。**我判定不该在本任务里连带做**：
  ① 它改的是**写入语义**（落库的键名会变），T-06 只加校验、不改语义，两者性质不同；
  ② 需要"回查旧键迁移"的配套设计，且必须与前端镜像 `utils/mvuKey.js` 同步；
  ③ 目标文件 `crates/domain/src/variables.rs` 当时正被域1 并发修改（`git status` 显示 M），
  在其上叠加语义改动有跨域冲突风险。
  精确补丁（供域6/域1 直接采用）：
  ```rust
  // commands/variables.rs 的 set_campaign_variable / set_character_variable 开头
  let key = storyforge_domain::variables::normalize_mvu_key(&key);  // 与 add_campaign_variable:179 对齐
  validate_variable_write_key(&key)?;                              // 归一化后再校验
  ```

### T-07 `log_clear` 未知 kind 静默清空全部日志 → 已修复

- **修法**：`crates/tauri-app/src/commands/diagnostics.rs` 的 `log_clear` 由返回 `()` 改为
  `Result<(), TauriCommandError>`，用显式 `match` 取代 `and_then(.., _ => None)`：
  `None | Some("all")` → 清全部；`backend|llm|frontend` → 对应缓冲；其它 → `Err(validation)`。
- **依据**：`crates/app-logging/src/lib.rs:230-240` 的 `clear(None)` 语义是**清空全部三类缓冲**，
  即"参数非法"反而扩大破坏面。前端唯一调用点 `LogPanel.vue:136-145` 的 `activeTab` 取值域恰为
  `all|backend|llm|frontend` 且已有 try/catch，故为向后兼容的行为收紧（并新增显式 `"all"` 支持）。

### T-08 世界书四条写命令静默吞重读错误 → 已修复

- **修法**：`crates/tauri-app/src/commands/world_info.rs` 新增
  `refresh_tool_ctx_world_info(state, campaign_id, command)`，替换 4 处
  `if let Ok(book) = ... { apply(...) }`（`add_campaign_world_info_entry:255`、
  `update_campaign_world_info_entry:303`、`delete_campaign_world_info_entry:340`、
  `set_campaign_world_info_route:360`）。重读 `Err` 时 `tracing::error!`，带 `campaign`/`command`/`error` 字段。
- **为何不把命令改成返回 Err**（已写进代码注释）：写操作已持久化成功，报错会诱导调用方重试，
  而 `add_campaign_world_info_entry` 非幂等（重试会重复插入条目），
  把"快照过期"升级成"数据重复"更糟。这是"至少留痕"的最小正确修法，
  符合 04 号报告的建议下限；**更强方案**（让 store 写方法直接返回更新后的书，省掉重读）留给域2/域6。

### T-09 `cardShellAllowHost` 提权 wrapper 无调用点 → 移交（域6 + 域5）

- **域5 已处置**：`frontend/src/tauri-api.js:1186` 的 `cardShellAllowHost` wrapper 已随死代码清理**删除**
  （该命令因此进入 §6.3 的 20 条零入口声明）。重新接线时必须走用户确认（对齐
  `ShellVariableProposalBar` 的提案确认模式），不可静默放行。
- **域6 待处置**：Rust 侧 `commands/card_shell.rs:129-136` 仍在，`card_shell_allow_host` 未接线（白名单写入入口）。

### T-10 契约测试未接入 CI + 快照无再生成路径 → 已修复（**未运行时验证**）

- **修法 1（CI）**：`.github/workflows/release.yml` 的 `windows` 与 `android` 两个 job
  在 `Build frontend`（`npm ci` + `npm run build`）之后、`Install pinned tauri-cli` 之前
  各加一步：
  ```yaml
      - name: Contract test (frontend invoke ↔ Tauri command registry)
        working-directory: frontend
        run: node --test tests/tauri-command-contract.test.mjs
  ```
  按 Lead 要求**只用聚焦测试**、不加整包 `npm test`。
- **修法 2（快照再生成）**：`backend-baseline.mjs` 新增
  `node scripts/architecture/backend-baseline.mjs --write-snapshot`
  （写 `frontend/tests/fixtures/tauri-registered-commands.snapshot.json`，并在 stderr 报告条数）；
  命令本体也是可执行门禁：缺失/重复/定义≠注册/位置越界/未声明孤儿任一存在即 `exit 1`（结果为 stderr，stdout 保持纯 JSON 以便重定向）。
- **capability 测试的去向（Lead 点已确认）**：capability 断言在 Rust 侧
  `crates/tauri-app/tests/capabilities.rs`，由 `cargo test` 覆盖，**不在**新增的 CI 步骤里；
  它声明的是能力集最小性（8 条权限、无 `fs:default`/`dialog:default`、v1 helper 黑名单），
  与前端契约测试是两条独立护栏。
- **诚实标注：本项未经真实 release 运行验证。** CI 步骤只在 GitHub Actions 上生效，
  本地无法执行（且本会话沙箱禁止 `node --test` 的子进程管道 stdio，见 §6）。
  验证方法：打一个预发布 tag（如 `v0.1.3-rc1`）或对 `workflow_dispatch` 干跑，
  确认两个 job 里 `Contract test (frontend invoke ↔ Tauri command registry)` 步骤出现且为绿。
- **G-12 交叉结论**（Lead 要求同步给域7，已发 review-goals）：
  - v0.1.2 的 `SHA256SUMS.txt`（196 B = 2 行 × (64+2+30+2)，CRLF）是 **Windows** 那份；
    `SHA256SUMS-android.txt`（88 B = 64+2+21+1，21 字符 = `app-arm64-release.apk`）是 **05:58 人工补传**。
  - 我原先据 `files:` 顺序推断"android 覆盖 windows"**已撤回**：实测留存的是 Windows 那份，
    说明 `softprops/action-gh-release` 遇同名资产是跳过而非覆盖。正确表述：
    「同名冲突导致后到的 Android 校验和被跳过、Windows 校验和留存 → v0.1.2 缺 Android 校验和（后经人工补传）」。
  - **Lead 要求的第 1 项（补生成 `SHA256SUMS-windows.txt` 并入上传/Release 列表）在 HEAD 已成立**
    （`ab894c6` 引入，`:90-94`），故我未重复添加写入点——避免造出第二个同名写入点冲突。

### T-11 命令级测试覆盖缺口 → 已修复(降级方案)

- **实际做法**：只为 **T-02 直接涉及的 5 个 Card Studio 命令**补了落盘失败/成功路径的命令级回归测试
  （3 个用例，见 T-02）。125/175 零引用的全量缺口**未**处理。
- **降级理由**：为 125 个命令补测试远超"修复审查发现"的范围（属持续投入而非缺陷修复），
  且多域并发改动同一工作树时大批新增测试会放大门禁噪音。
- **建议**（留给后续）：优先给 `commands/plugins.rs` 的 7 个 `plugin_*` 通道命令补注册校验测试，
  那族是插件沙箱的唯一后端边界。

### T-12 大载荷 import/export 命令在 IPC 线程内联执行 → 已修复

- **修法**：`crates/tauri-app/src/commands/import_export.rs`，3 条命令移入 blocking 池：
  | 命令 | 改法 |
  | --- | --- |
  | `export_campaign_st_cards`（每实例一张 PNG，CPU/IO 密集） | 改 `pub async fn`，克隆 `Arc<StorageFacade>`，`spawn_blocking` 调新抽出的同步实现体 `export_campaign_st_cards_impl(storage, campaign_id)` |
  | `export_campaign_bundle`（序列化可达数十 MB） | 改 `pub(crate) async fn` + `spawn_blocking` |
  | `import_campaign_bundle`（≤16 MiB JSON 解析 + 多表写入） | 改 `pub(crate) async fn` + `spawn_blocking`；`require_ipc_size` 仍留在 IPC 线程快速失败 |
- **连带改动**：`crates/tauri-app/src/lib_tests_review_history.rs:379-385`。
  `export_campaign_st_cards` 变 async 后，同步测试 `review_bundle_to_st_export_preserves_campaign_worldbook`
  出现 `E0599: no method named unwrap found for impl Future`。修法：让同步测试直接调用
  `export_campaign_st_cards_impl(target.storage(), &imported.campaign_id)`
  （实现体为 `pub(crate)`；async 包装只做 storage 克隆与线程卸载，逻辑全在 impl 内）。
  **说明：该文件位于 `src/`，不在 task-11 声明的写作用域内，但这是 T-12 的必要连带改动**
  （不改则 `cargo check --all-targets` 无法通过），特此显式列出以免被当作越界修改。
- **依据**（Tauri 线程模型）：`tauri-macros-2.6.3/src/command/wrapper.rs` 默认
  `ExecutionContext::Blocking`（`:50`）、只把 async fn 提升为 `Async`（`:158-160`）、
  非 async 走 `body_blocking` 原地调用（`:429-435`）；只有 `Async` 才
  `respond_async_serialized` → `tauri-2.11.5/src/ipc/mod.rs:375` 的 `async_runtime::spawn`。

---

## 3 P3 与顺带发现

### T-13 / T-16 命令位置无门禁 + 差异化入口口径 → 已修复

- **修法**（`scripts/architecture/backend-baseline.mjs`）：
  - 新增 `extractCommandLocations()`：逐命令记录"命令名 → 工作区相对文件"；
  - 新增 `COMMAND_LOCATION_ALLOWLIST`：**显式登记** `card_studio_api.rs` 的 19 个命令
    （Card Studio Phase 1 历史位置），并写明"位置本身可接受，但此前没有任何断言保护它"；
  - 新增 `findCommandsOutsideAllowedLocations()` → `backend.commandsOutsideAllowedLocations`；
  - 新增 `backend.definedCommandCount` → 门禁断言"定义数 == 注册数"，可捕获"定义了但忘了注册"。
- **前端断言**：`tauri-command-contract.test.mjs` 新增
  `Gate 0 keeps every command inside commands/*.rs or an explicit allowlist`。
- **差异化入口口径**（原 T-16 说明项）：基线新增 `orphanRegisteredCommands`（后端注册但前端零入口），
  与 `invokedCommands` 一起构成"注册表 = 前端可达 ∪ 声明的保留 API"这一条**无魔数契约**，
  取代原先"175 vs 169 靠人对"的口径。

### T-14 `CLAUDE.md` 陈旧事实条目 → 移交 task-16（仅列条目）

- `CLAUDE.md:116`：`cardShellClearCache`（L6）标注"UI 入口未接"，实际已接线
  （`frontend/src/components-v2/shell/InspectorDrawer.vue:18, 45`）。
- `CLAUDE.md:105`：`apply_campaign_opening` 标注为"Tauri 命令（lib.rs）"，实际在
  `crates/tauri-app/src/commands/campaigns.rs:486`（Gate 1 已把 lib.rs 拆成装配层）。
- `docs/DOCS-CODE-AUDIT.md:11` 与 `:54`：声称"全部 175 个命令位于 `crates/tauri-app/src/commands/*.rs`"，
  实测 156 在 `commands/`、19 在 `src/card_studio_api.rs`（命令总数 175 不变）。

### T-15 三个孤儿命令 → 已修复(降级方案：保留 + 声明)，Lead 裁定

`abandon_turn`（`commands/turns.rs:673`）、`archive_conversation`（`commands/conversations.rs:24`）、
`soft_delete_variant`（`commands/turns.rs:636`）。

**决策：保留命令与注册条目，把"零入口"从静默遗留变成被声明的状态。**

三条依据（Lead 已复核认可）：
1. `abandon_turn` 不是普通死代码——它承载 **Gate 8 复评专项加固的 Turn CAS 谓词**
   （`commands/turns.rs:697-724`：先 CAS 置 `Abandoned`（谓词排除 `Committing`/已终态）再软删变体，
   防"并发 Accept 已提交的故事被静默回退"），并配 `turn_store.rs:453` 的谓词注释；删除等于丢弃经审查的并发逻辑。
2. 删 `archive_conversation` 会**连带** `archive_conversation_impl`（`commands/conversations.rs:31`）
   变成死代码——它的唯一调用点就是该死命令（已核实 `auto_archive_if_needed:192/253`
   直接调 `run_archive_with_watermark`，不经 impl），需连删，改动面扩大。
3. 删除需跨 3 个当时不可写的文件同步：`frontend/tests/tauri-command-contract.test.mjs`、
   `frontend/tests/fixtures/tauri-registered-commands.snapshot.json`、`README.md`/`docs/DOCS-CODE-AUDIT.md`
   （文档口径"175 个命令"将变 172）。

**落地物**：
- Rust 侧 3 处定义处各加一行声明注释（Lead 指定的措辞）：
  `// 保留 API：当前无前端入口（wrapper 于 2026-09-01 移除），保留原因见 docs/review-2026-09-13/fixes/04-tauri-fixes.md`
- 基线脚本新增 `RETAINED_NO_FRONTEND_CALLER`（含每条的保留理由）→
  `backend.retainedNoFrontendCaller` / `undeclaredOrphanCommands` / `staleRetainedDeclarations`；
  **新增门禁**：出现未声明的零入口命令即 `exit 1`（"静默遗留到此为止"）。
- 前端新增断言：`undeclaredOrphanCommands == []`、`staleRetainedDeclarations == []`、
  `orphanRegisteredCommands == retainedNoFrontendCaller`。

**若将来要删这 3 个命令，需连删（清单）**：
1. 命令 `fn` 本体（3 处）+ 其 `#[tauri::command]` 属性；
2. `crates/tauri-app/src/lib.rs` 的 `generate_handler!` 注册条目（`abandon_turn`、`archive_conversation`、`soft_delete_variant`；后者是 `commands::turns::soft_delete_variant`）；
3. `archive_conversation_impl`（若不再有内部调用方）；
4. `scripts/architecture/backend-baseline.mjs` 的 `RETAINED_NO_FRONTEND_CALLER` 三条目；
5. 快照：`node scripts/architecture/backend-baseline.mjs --write-snapshot`（175 → 172 条）；
6. `frontend/tests/tauri-command-contract.test.mjs` 的 `commandAttributes` / `registeredCommandCount`（175 → 172）；
7. 文档计数：`README.md`、`docs/DOCS-CODE-AUDIT.md` 的"175 个命令"→ 172（归 task-16）；
8. 删除后 `staleRetainedDeclarations` 会先报"声明过期"，可用来验证第 4 步是否漏做。

### N-01 顺带发现并修复：基线脚本把注释里的 `#[tauri::command]` 计成命令

- **发现经过**：我在 `commands/import_export.rs` 的 T-12 注释里写了"非 async 的 `#[tauri::command]`…"，
  随后新加的"定义数 == 注册数"门禁立刻报 `定义 176 / 注册 175`，并把它误判成
  `export_campaign_st_cards_impl` 未注册——**是我自己的注释触发的**。根因是
  `extractCommandAttributes` 用无锚点全局正则，`#[tauri::command]` 出现在文档注释里也会被计数。
- **修法**：新增 `COMMAND_ATTRIBUTE_LINE = /^\s*#\[tauri::command(?:\([^\]]*\))?\]/`，
  `extractCommandAttributes` 与 `extractCommandLocations` 都改为**逐行、行首锚定**匹配。
- **意义**：这是一类真实缺陷（"数量类事实"的门禁可被注释污染），
  修复后 `commandAttributes` 重新等于 175 且不再随注释漂移。

---

## 4 门禁结果

| 门禁 | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 架构基线门禁 | `node scripts/architecture/backend-baseline.mjs` | **0** | `定义 175 / 注册 175 / 前端唯一 invoke 152 / 孤儿命令 23 (…全部已登记)` + `门禁通过` |
| 前端契约测试 | `cd frontend && node tests/tauri-command-contract.test.mjs` | **0** | `tests 11 / pass 11 / fail 0`（含 2 条新增门禁：裸 `_invoke` 回归锁、命令位置白名单；另有 3 条不变量断言） |
| Rust 编译 | `cargo check -p storyforge --all-targets` | **0** | `errors=0`（含全部 test target） |
| Rust 单测（本域聚焦） | `cargo test -p storyforge --lib card_studio` | **0** | `8 passed; 0 failed; 462 filtered out`，其中 3 个是 T-02 新增用例 |
| Rust 集成测试（T-12 连带） | `cargo test -p storyforge --test sqlite_character_lifecycle` | **0** | `1 passed; 0 failed`（改 async/`.await` 后实跑通过） |
| Rust 全量单测（`storyforge` crate） | `cargo test -p storyforge --lib` | **0** | `467 passed; 0 failed; 3 ignored; 0 measured`（`3 ignored` 为既有 `#[ignore]` 用例） |

**验证时点说明（诚实边界）**：上表 6 条已在**同一次连续复跑中全部同时通过**（域1 修完 `storyforge-domain`
的语法错误与 borrow 中间态之后），即本域全部改动 + `--write-snapshot` 安全加固后的**最终状态为全绿**。
此前多次失败均发生在**依赖 crate 的临时状态**上，本域源码未变、也未触碰 `crates/domain/**`。
按 §6.1 的教训，**最终裁决仍以 Lead 的 task-15 全量门禁为准**（共享树仍会被其他域继续编辑）。

**命令名勘误**：task-11 与 Lead 指示里的 `cargo check -p storyforge-tauri-app --all-targets` **不存在该包**——
`crates/tauri-app/Cargo.toml` 的 `[package] name = "storyforge"`（`[lib] name = "storyforge_lib"`），
实测报 `error: package ID specification 'storyforge-tauri-app' did not match any packages`。
正确命令是 `cargo check -p storyforge --all-targets`。

**中间态观察（诚实记录，非本域改动）**：首轮 `cargo test -p storyforge --lib` 曾出现
`458 passed; 3 failed`，三个失败全部落在**他域正在编辑的文件**里：
`shell_doc_protocol::tests::shell_csp_allows_only_the_restricted_protocol_as_a_module_origin`（域6）、
`tests::import_export::default_real_card_fixture_path_targets_data_local_test_card`（域6，即我先前移交回域6 的
`data/local/test-card.png` 缺文件问题）、`tests::meta::test_meta_typed_patch_is_bound_to_its_campaign`
（域6，`Validation { message: "Patch 状态不是 Pending（当前: Stale）" }`）。
这三条均不涉及本域任何改动（`git diff --name-only` 可证本域未触碰这三个文件）；
其后重跑即全绿——是并发编辑的中间态，已由域6 收敛。
另有一轮 `turn_lifecycle::tests::accept_rejects_conversation_scope_mismatch`（域2）单条失败，同属该模式。

### 4.1 沙箱限制导致的验证方式调整（非项目缺陷）

- `node --test tests/tauri-command-contract.test.mjs` 在本会话**必然失败**：
  `Error: spawn EPERM`（node:test 的 runner 用管道 stdio 生成子进程，被当前沙箱策略拒绝）。
  改为**进程内直跑** `node tests/tauri-command-contract.test.mjs`（同一份断言、同一份 import，
  不生成子进程），结果 11/11 通过。
- CI 侧不受此限（GitHub Actions 无该沙箱），因此 T-10 加的步骤写成 `node --test ...` 是正确的；
  但这意味着**"CI 步骤能否在真实 runner 上跑通"我无法本地证明**，属"未运行时验证"的一部分。
- task-15 的描述里也预判了这一点（"前端命令在受限沙箱下可能因 Node/esbuild 子进程管道 stdio 报 spawn EPERM，
  那是环境限制，不是项目缺陷"），与本记录一致。

---

## 5 改动文件清单

| 文件 | 关联 | 说明 |
| --- | --- | --- |
| `scripts/architecture/backend-baseline.mjs` | T-01/T-10/T-13/T-15/T-16/N-01 | 扫描正则修正、命令位置门禁、保留 API 声明表、门禁求值与 `--write-snapshot`、属性行锚定统计 |
| `frontend/tests/tauri-command-contract.test.mjs` | T-01/T-13/T-16 | 去魔数不变量断言 + 裸 `_invoke` 回归锁 + 位置白名单断言（Lead 单独授权改动） |
| `crates/tauri-app/src/card_studio_api.rs` | T-02 | 2 个 helper + 8 处替换 + 3 个测试 |
| `crates/tauri-app/src/commands/turns.rs` | T-04/T-15 | `add_variant` 接受 `provenance`；`abandon_turn`/`soft_delete_variant` 加保留声明注释 |
| `crates/tauri-app/src/commands/conversations.rs` | T-15 | `archive_conversation` 加保留声明注释 |
| `crates/tauri-app/src/commands/variables.rs` | T-06 | `validate_variable_write_key` + 2 处调用 |
| `crates/tauri-app/src/commands/diagnostics.rs` | T-07 | `log_clear` 显式 kind 解析 + 返回 `Result` |
| `crates/tauri-app/src/commands/world_info.rs` | T-08 | `refresh_tool_ctx_world_info` + 4 处替换 |
| `crates/tauri-app/src/commands/import_export.rs` | T-12 | 3 条命令异步化 + `spawn_blocking` + 同步实现体抽取 |
| `crates/tauri-app/src/lib_tests_review_history.rs` | T-12（**连带修复，已获 Lead 批准**） | 同步测试改调 `export_campaign_st_cards_impl`。Lead 2026-09-13 明确批准：这是保持 `--all-targets` 编译的必要连带改动，**不属于范围蔓延**；R3 复检必须把它列为"连带修复"而非隐藏改动 |
| `crates/tauri-app/tests/sqlite_character_lifecycle.rs` | T-12（连带，在声明作用域内） | 该测试改 `#[tokio::test] async fn` + 对命令调用加 `.await` |
| `.github/workflows/release.yml` | T-10 + G-12 项 | 2 个 job 各加聚焦契约测试步骤；Release 正文 APK 通配 `*-arm64-*-release.apk` → `*arm64-release.apk`（实际资产 `app-arm64-release.apk`） |
| `docs/review-2026-09-13/fixes/04-tauri-fixes.md` | 本记录 | — |

未改动（明确移交，未碰）：`commands/card_shell.rs`、`card_shell_cache.rs`、`shell_doc_protocol.rs`、
`commands/plugins.rs`、`commands/meta_typed.rs`、`commands/meta.rs`、`capabilities/**`（域6）；
`crates/domain/**`、`crates/infra-sqlite/**`、`crates/app-*/**`（其他域）；
`README.md`、`CLAUDE.md`、`docs/**`（task-14/16/17）。

---

## 6 遗留、阻塞与移交

### 6.1 门禁一度被他域并发编辑阻塞（已解除，最终全绿）

**最终结论：本域全部 6 条门禁通过（见 §4）。** 过程中 `cargo check -p storyforge --all-targets`
因**依赖 crate 不编译**而长时间无法取得结论——`storyforge` 依赖 `storyforge-domain` / `storyforge-app-pipeline`，
它们编译失败时我的 crate 根本不会被检查。按时间顺序观察到的阻塞点（**均为他人文件**）：

1. `crates/tauri-app/src/lib_tests_meta.rs:917/944` `E0425: cannot find value 'campaign'`（域6）
2. `crates/tauri-app/src/lib_tests_meta.rs:1129` `E0277: MetaSnapshot doesn't implement Debug`（域6）
3. `crates/tauri-app/src/storage_backend.rs:2003` `E0382: borrow of moved value db_path`（域2，出现两次）
4. `crates/tauri-app/src/turn_store.rs` `E0592: duplicate definitions with name get_turn`（域2）
5. `crates/domain/src/card_studio.rs:944 → 979` `E0308: expected &str, found String`（域1）
6. `crates/app-pipeline/src/lib.rs:3251` `cannot find macro debug in this scope`（域3）
7. `crates/app-pipeline/src/lib.rs:1345/2377/2644` `E0277: SequentialActorOutcome doesn't implement Display` ×2 + `E0308`（域3）

其中 1/2/5/6/7 与 3/4 分别通报了域6、域1、域3、域2（send_message，附精确改法）。
**这一节的保留意义**：说明"某一轮 check 失败"未必是最后一次编辑的问题，
在多域共用一棵依赖树时，**编译结论只对最后一次成功的检查有效**；task-15 的全量门禁必须作为最终裁决。

**过程中唯一由本域改动引起的编译/测试失败（已修）**：
- `crates/tauri-app/src/lib_tests_review_history.rs:383` `E0599`（同步测试调用 async 命令）→ 改调 `export_campaign_st_cards_impl`
- `crates/tauri-app/tests/sqlite_character_lifecycle.rs:544` `E0599`（同上）→ 该测试改 `#[tokio::test] async fn` + `.await`

最后重跑命令（供 Lead 复核用，全部可复现）：
```
node scripts/architecture/backend-baseline.mjs
cargo check -p storyforge --all-targets
cargo test -p storyforge --lib
cargo test -p storyforge --test sqlite_character_lifecycle
cd frontend && node tests/tauri-command-contract.test.mjs
```

### 6.2 已知未被本域测试覆盖的改动

| 改动 | 缺的验证 | 建议验证方式 |
| --- | --- | --- |
| T-04 `add_variant(provenance)` | 无测试证明 `provenance` 真的落到 `MessageVariant`（`cargo check` 只证明签名兼容） | 补一条命令/store 测试传非 None provenance 并断言读回一致 |
| T-06 `validate_variable_write_key` | 无测试；且未验证"前端既有写入键不会被误拒" | 补 `__storyforge_x` 被拒 + 正常键通过的单元测试；前端 `shellVariableOutbox` 键集回归 |
| T-07 `log_clear` 未知 kind | 无测试（`cargo test --lib` 全绿只说明无回归，不证明新分支被覆盖） | 补"未知 kind 返回 validation 且日志未被清空"的测试 |
| T-08 `refresh_tool_ctx_world_info` | 无测试（重读失败路径需要 store 故障注入） | 复用 T-02 的"目标路径改成同名目录"手法构造读失败 |
| T-12 三条命令异步化 | async 包装本身的运行时行为未测（`export_campaign_st_cards` 的既有测试**已实跑**覆盖逻辑与 `.await` 链路：`sqlite_character_lifecycle` 1 passed；`lib_tests_review_history` 的 impl 直调覆盖逻辑） | release 干跑验证前端 `await` 行为；`export/import_campaign_bundle` 仍缺命令级运行时测试 |
| T-10 CI 步骤 | **未运行时验证**（本地无法执行 `node --test`，见 §4.1） | 打预发布 tag 或 `workflow_dispatch` 干跑 |

**已由本次门禁实证覆盖的部分**（不再是"未验证"）：T-02 三条新测试实跑通过（`cargo test -p storyforge --lib card_studio`
→ `8 passed; 0 failed`），其中 `persist_project_propagates_disk_failure_as_command_error` 真实构造了磁盘写失败并断言
命令返回 `TauriCommandError::Storage`，`persist_project_returns_ok_when_disk_write_succeeds` 反向断言不误报。

### 6.3 新增的 20 个零入口后端命令：**暂缓（产品决策）**——本轮保留 + 声明，不删除

> **状态：暂缓（产品决策）**，**不是"已修复"**。Lead 2026-09-13 裁定：本轮**保留 + 声明，现状即终态**；
> 是否删除交由用户做产品决策，Lead 已将其列为**新发现 N-02（P2）**写入第二版报告。

域5（review-frontend）按 Lead 裁决 A 删除了 21 个孤儿前端 wrapper（20 删 + `cardstudioListStages` 接线保留）。
删除后这 20 个后端命令由「有 wrapper 但无人调用」变为「**后端注册、前端永不可达**」，
触发本域新加的门禁（`undeclaredOrphanCommands`）——**这正是该门禁存在的意义**：
不做任何处理的死代码删除会让后端 API 表面积悄悄与前端脱节。

**本域处置：只做声明，不删后端命令。** 20 条已加入 `RETAINED_NO_FRONTEND_CALLER`
（含文件位置与逐条理由），表头注释写明这是**声明事实、不是为"应当保留"背书**。
20 个命令及其位置：

| 命令 | 文件 |
| --- | --- |
| `add_world_info_entry` / `update_world_info_entry` / `delete_world_info_entry` / `update_world_info_route` | commands/characters.rs |
| `delete_character` | commands/characters.rs |
| `card_shell_allow_host` | commands/card_shell.rs |
| `export_st_card_png` | commands/import_export.rs |
| `configure_embedder` / `get_embed_config` | commands/connections.rs |
| `get_active_agent_profile_config` / `get_active_profile` / `list_modules` / `list_profiles` / `save_profile` / `set_active_profile` / `update_module` | commands/profiles.rs |
| `get_active_preset` | commands/presets.rs |
| `log_get_llm_call` | commands/diagnostics.rs |
| `meta_classify_st_preset` | commands/meta_typed.rs |
| `meta_get_conversation` | commands/meta.rs |

**裁定理由（Lead）**：
- **(a)** 删命令属 API/产品面决策，会连带改 `lib.rs` 注册表 + 快照 + "175 个命令"的文档口径（175→155），
  超出本轮"修复评审发现"的授权范围；
- **(b)** 这些命令可能是后续功能的预留入口，删除会造成**不可逆的接口收缩**。

**域5 独立核对结论（2026-09-13，为"保留"裁定补强证据）**：
- 这 20 个命令从纯前端事实看是**零引用**，且删除 wrapper 后**没有任何调用点被替代**——
  即**不是"被别的命令取代"，而是本来就没接 UI**。这排除了"已被取代故可删"的可能。
- 域5 **不追加删除意见、不建议删任何一个**；倾向保留 + 声明（即本记录终态）。
- 唯一有明确文档依据的是 `delete_character`：`CLAUDE.md` 明确写了它的级联语义
  （对 `StoredCharacter.id` / `source_character_id` / 同会话 `tool_ctx` 域 id 做 Campaign/MVU/向量清理），
  属**文档化的保留 API**，不是随手遗留 → 该条 reason 已据此改写为引用 CLAUDE.md。
- `configure_embedder` / `get_embed_config` / `meta_get_conversation` / `meta_classify_st_preset` /
  `log_get_llm_call` 形态为"能力已实现、UI 未接"，删掉等于砍掉未来入口。
- 域5 复测：`node tests/tauri-command-contract.test.mjs` → 11/11 绿；全量 `node:test` **67 文件 / 532 pass / 0 fail**；
  **没有第 21 个孤儿**（与本域 23 条 = 3 + 20 的计账一致）。

**N-02（P2）风险提示**（Lead 写入第二版报告，此处留档）：在 **P0-2（子帧可调任意命令）未彻底关闭前，
23 个"前端不可达但已注册"的命令等同于额外 IPC 攻击面**——前端不调用 ≠ 不可被调用，
子帧/插件侧仍可发出 `invoke('update_module', …)`。这正是"零入口 ≠ 无风险"的要点，
也是本域门禁只做"声明"而不等于"安全豁免"的原因。
（上一条"域5 不认为可删"是**产品/维护面**证据，与本条的**安全面**结论并不冲突：
命令功能上值得保留 ≠ 它不增加攻击面。）

**后续变更方案**：若用户决定删除，按 §3 T-15 的「若将来要删这 3 个命令，需连删（清单）」执行——
步骤完全一致，把条目数从 3 扩展到 23 即可（命令 fn + `lib.rs` 注册条目 + 可能的 impl 连带 +
`RETAINED_NO_FRONTEND_CALLER` 条目 + `--write-snapshot` + 前端契约测试的计数断言 + 文档命令计数）。

**副作用已确认无**：无后端命令被删，故 `frontend/tests/fixtures/tauri-registered-commands.snapshot.json`
（175 个注册命令名）**无需重新生成**；前端唯一 invoke 数由 172 降为 **152**，但该数字不参与任何断言
（契约全部是不变量形式），故域5 再删 wrapper 只需追加声明、无需同步计数常量。

### 6.4 移交清单

| 项 | 归属 | 交付物 |
| --- | --- | --- |
| T-03 `card_shell_fetch_url` 异步化 | 域6（task-13） | 改法见 §1 T-03 |
| T-05 两处错误 DTO | 域6 | `commands/card_shell.rs:114-122` → `TauriCommandError` |
| T-09 `cardShellAllowHost` | 域6（Rust）+ 域5（`tauri-api.js:1186`） | 接线须走用户确认 |
| T-14 文档陈旧条目 3 处 | task-16 | 见 §3 T-14 |
| M-15 `set_*_variable` 键记法归一 | 域6 + 域1（domain） | 见 §2 T-06 的精确补丁与不做的三条理由 |
| T-11 剩余 120 条命令的测试覆盖 | 后续投入 | 优先 `plugin_*` 7 条通道命令 |
| `lib_tests_import_export.rs:21-28` 的 `default_real_card_fixture_path_targets_data_local_test_card`（断言默认真实卡路径指向 `data/local/test-card.png`） | **回退给域6** | 该文件在 `crates/tauri-app/src/`，不在 task-11 写作用域。**证据**：本域跑 `cargo test -p storyforge --lib` 时它确实失败过一轮（随后域6 的后续编辑使其通过），说明该断言对文件是否存在敏感、值得域6 收尾时确认 |
| `CLAUDE.md` / `DOCS-CODE-AUDIT.md` 命令位置与计数口径、基线门禁的说明 | task-16 | 见 §7 |

---

## 7 需文档同步条目（供 task-16）

1. **命令总数**：**保持 175**（T-15 未删命令）。`README.md`、`docs/DOCS-CODE-AUDIT.md` 的 175 无需改数。
2. **命令位置**：`docs/DOCS-CODE-AUDIT.md:11` 与 `:54` 的"全部 175 个位于 `commands/*.rs`"
   → 改为"156 个在 `crates/tauri-app/src/commands/*.rs`，19 个在 `crates/tauri-app/src/card_studio_api.rs`
   （已在 `scripts/architecture/backend-baseline.mjs` 的 `COMMAND_LOCATION_ALLOWLIST` 显式登记）"。
   现在这条位置约束**有门禁**，文档可写明。
3. **新门禁的存在**：基线脚本不再只是快照打印机——`node scripts/architecture/backend-baseline.mjs`
   会校验"前端缺失=0 / 定义=注册 / 位置在白名单 / 零入口命令已声明"，失败 `exit 1`。
   **CI 安全（Lead 关注点，已验证）**：无参数运行**只读不写**；快照再生成入口 `--write-snapshot`
   需显式传参才写，且**门禁未通过时会拒绝写入**（防止把违规状态固化成 fixture、让"快照深比较"测试失效）。
   实测：`node scripts/architecture/backend-baseline.mjs` 前后快照文件哈希不变（175 条）。
4. **零入口命令**：3 个保留 API（`abandon_turn`、`archive_conversation`、`soft_delete_variant`）
   应在文档中从"孤儿/死代码"改为"声明的保留 API（wrapper 于 2026-09-01 移除）"；
   另 20 个（域5 死代码清理后新增）同属"声明的保留 API"，Lead 裁定本轮保留、风险记为 N-02（P2）。
   > ⚠️ **回写硬约束**：本轮**只删前端 wrapper，后端命令一个都没删**（`commandAttributes` /
   > `registeredCommandCount` / 快照均为 **175**）。请**不要**把这 23 个命令写成"已废弃 / 死代码 / 待删"，
   > 也不要把"wrapper 删除"误译成"命令废弃"——正确写法是"**无前端入口的声明的保留 API**"。
   > 特别地，`delete_character` 的保留依据**就是 `CLAUDE.md` 自身的级联删除语义**
   > （见 `scripts/architecture/backend-baseline.mjs` 的 `RETAINED_NO_FRONTEND_CALLER`），
   > 回写该条时**不要削弱或删除**那段级联语义说明，否则依据链会断。
   > （域5 已在其 `05-frontend-fixes.md` §5 移交清单写入同一条约束，两份记录口径一致。）
5. **契约清单口径**：前端唯一 invoke 数为 **152**（T-01 修正后为 172、169 是漏扫假值；域5 死代码清理后为 152）。
   该数字**会持续变化且不参与任何断言**——文档宜写成"由基线脚本输出、不硬编码"。
   同理"零入口命令"数从 3 增至 **23**（全部已在 `RETAINED_NO_FRONTEND_CALLER` 声明，见 §6.3）。
6. **行为契约变更**（写文档/用户可见行为时注意）：
   - `log_clear` 现在返回 `Result`，未知 `kind` 报错；新增显式 `"all"`。
   - `add_variant` 新增可选 `provenance` 形参。
   - `export_campaign_bundle` / `import_campaign_bundle` / `export_campaign_st_cards` 变为 async
     （前端 `await` 语义不变，无破坏性）。
7. **CI 描述**：`.github/workflows/release.yml` 的 `windows`/`android` 两 job 各多一步
   `Contract test (frontend invoke ↔ Tauri command registry)`（`node --test tests/tauri-command-contract.test.mjs`）；
   文档若写"远端 CI 只构建"需补这一句，且标注**未经真实 release 运行验证**。已同步域7。
8. **G-12 口径**：v0.1.2 留存的是 **Windows** 校验和（196 B），Android 校验和缺失后经人工补传
   （88 B，`SHA256SUMS-android.txt`，05:58）；"android 覆盖 windows"的说法应改为
   "同名冲突导致后到的 Android 校验和被跳过"。已同步域7（review-goals）。

---

## 8 R1 承接确认 / R6 无记录项收口：S-06（`lib.rs` 旧数据目录迁移的静默吞错）

**承接确认**：R1 复检指出 S-06 在 `02-storage-fixes.md` 被标为"暂缓（跨域，`lib.rs`，域4）"，
而**本记录此前没有任何对应条目**——即"移交后无人接收"。本条即为该承接的落地记录，
任务 `task-37`，完整报告见 `docs/review-2026-09-13/round2/R14-s06-copy-error-propagation.md`。

**处置：已修复**（不是判非问题）。改动文件：`crates/tauri-app/src/lib.rs`、
`crates/tauri-app/src/lib_tests_startup.rs`。

| 项 | 内容 |
| --- | --- |
| 事实（`HEAD` 版） | `lib.rs:479` `migrate_from_exe_dir_if_needed` 返回 `()`；`:517` `copy_dir_recursive` 对 `create_dir_all`/`read_dir`/`entries.flatten()`/每个 `fs::copy` **全部吞错**（`:525` `let _ =`），嵌套目录失败**连日志都没有**（`:509` 的 warn 只覆盖顶层文件）；`:512` 无论成败都打印"数据迁移完成"；`:490-492` 的跳过判据只看 `characters.json`/`connections.json`/`campaigns` **是否存在** |
| 失败后果 | ① 嵌套失败不可见；② 假成功日志；③ **永久锁死**——一次部分失败就足以让"新目录已有数据"成立，之后每次启动都 `return`，旧目录数据再也搬不过来（**静默 + 永久**） |
| 修复 | `MigrationOutcome{NotApplicable/SkippedPopulated/Completed/Incomplete}` 返回给调用方；`copy_dir_recursive` 逐条收集失败（相对路径，不带用户绝对路径进前端）；失败时 `error` 日志 + `storage_health::record_backend_incident("legacy_dir_migration_incomplete", …)` 健康事件；新增 `.migration_incomplete` 标记，**拷贝前先写、全部成功才撤销**，并让跳过判据变为 `new_has_data && !marker.exists()` ⇒ 下次启动越过 latch 重试 |
| 复现证据 | 旧/新实现逐字放进同一 rustc 探针、同一注入方式：旧实现三次启动后嵌套数据**最终落地=false**（第 1 次打印"数据迁移完成"却是假的、无失败信号；第 2/3 次永久跳过）；新实现 `Incomplete+标记+健康事件` → `重试` → `Completed+撤销标记`，**落地=true** |
| 测试 | 新增 5 条（失败注入断言"不会静默成功"、标记越过 latch 的重试 + **反证**（删标记后确实 `SkippedPopulated`）、健康事件真断言、成功路径不留标记、`NotApplicable`/`SkippedPopulated` 显式化）；既有 2 条迁移测试未改一行仍通过 |
| 复跑 | `cargo test -p storyforge --lib migrate` → **12 passed / 0 failed**；`cargo test -p storyforge --lib` → **477 passed / 0 failed / 3 ignored**；`node scripts/architecture/backend-baseline.mjs` → 175/175 **门禁通过**；`cargo check -p storyforge --all-targets` → 0 error |
| 命令签名 | **零改动**（两个函数都是私有普通函数，非 `#[tauri::command]`；命令总数仍 175） |

**口径提醒（供 task-16 文档回写）**：S-06 与 S-01 **不是同一件事**，不要合并表述——
S-01 = "源目录不完整时的 fail-closed 守卫"（域2，已关闭）；
S-06 = "迁移拷贝失败时的静默吞错与永久锁死"（域4，本次修复）。
S-01 的守卫**拦不住** S-06 的三种形态：`cards.json` 拷过去了但其它集合没拷完时，
新目录里那些集合只是"空"，守卫判据（缺失文件 + 非空 dependents）不成立；
而迁移一旦被 latch 跳过，守卫连源目录都不会看到。

**残留（已披露，未做）**：① 未做端到端启动验证（无法在此环境跑 GUI），健康事件在 UI 的显示是
接线级证据；② 若要求"迁移失败即拒绝启动"，需让 `initialize_app_data_dir` 在 `Incomplete` 时返回 `Err`
（一行改动）——但**必须与标记一起改**，单独 fail-closed 仍会在第二次启动静默丢失；③ 失败原因不分类
（权限/占用/磁盘满同路径，靠 `detail` 文本区分）。**定级建议**：S-06 原记 P2，
实际含"静默 + 永久数据不可达"路径，建议在最终报告按 **P1** 表述，是否调级交 Lead。


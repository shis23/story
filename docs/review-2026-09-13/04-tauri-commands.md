# 域4 审查报告：Tauri 命令层与前后端契约

- 审查日期：2026-09-13
- 审查对象：git HEAD `ab894c6`，工作树干净（`git status --porcelain` 无输出）
- 审查性质：只读静态审查（未运行任何 cargo / npm / 构建 / 测试命令）
- 报告文件：`docs/review-2026-09-13/04-tauri-commands.md`（本次审查唯一写入的文件）

---

## 1 范围与覆盖率

### 1.1 已核对范围

| 范围 | 内容 | 覆盖 |
| --- | --- | --- |
| `crates/tauri-app/src/lib.rs` | 1494 行装配层 + `generate_handler!` 注册块（1294–1487） | 全文关键段已读 |
| `crates/tauri-app/src/commands/**` | 22 个 .rs（19 个含命令 + `bundle_runtime.rs`/`writing_regenerate.rs` 无命令） | 命令清单 100%、重点文件全文 |
| `crates/tauri-app/src/card_studio_api.rs` | 19 个命令 | 命令清单 100%、关键函数已读 |
| `error.rs` / `runtime_support.rs` / `startup_support.rs` / `meta_backend.rs` / `mvu_webview_runtime.rs` / `shell_doc_protocol.rs` / `card_shell_cache.rs` / `card_studio_store.rs` / `storage_backend.rs` | 错误 DTO、锁、阻塞、落盘语义 | 关键段已读 |
| `crates/tauri-app/tests/capabilities.rs`、`capabilities/default.json` | capability 断言 | 全文 |
| 交叉参考 `frontend/src/tauri-api.js`（1691 行）、`frontend/src/**`、`frontend/tests/tauri-command-contract.test.mjs`、`frontend/tests/fixtures/tauri-registered-commands.snapshot.json`、`scripts/architecture/backend-baseline.mjs`、`.github/workflows/release.yml` | 契约核对与测试缺口 | 相关部分 |
| 框架侧佐证（只读 registry 源码，非本仓库） | `tauri-macros-2.6.3`、`tauri-2.11.5`、`tauri-plugin-fs-2.5.1`、`tauri-plugin-dialog-2.7.2` | 用于确认 Tauri v2 命令线程模型 / 参数键名 / fs scope 语义 |

未覆盖/未执行：未编译、未运行门禁、未做真实 IPC 端到端验证；所有"运行期会怎样"的结论均由源码推导，已在各条目标注置信度。

### 1.2 统计口径（数量类结论的可复现命令与结果）

所有命令均在仓库根目录执行。

**(a) 后端命令定义总数与分布**

```powershell
$all = Select-String -Path (Get-ChildItem crates/tauri-app/src -Recurse -Filter *.rs |
        Where-Object {$_.Name -notlike 'lib_tests*'}).FullName -Pattern '#\[tauri::command' -AllMatches
"total = $((($all|Measure-Object).Count))"
"in commands/ = $((($all|Where-Object{$_.Path -match '\\commands\\'})|Measure-Object).Count)"
"in lib.rs     = $((($all|Where-Object{$_.Path -match '\\lib\.rs$'})|Measure-Object).Count)"
"in card_studio_api.rs = $((($all|Where-Object{$_.Path -match 'card_studio_api\.rs$'})|Measure-Object).Count)"
```

结果：`total = 175`，`in commands/ = 156`，`in lib.rs = 0`，`in card_studio_api.rs = 19`。
`commands/` 共 22 个 .rs 文件，其中 19 个含命令（`bundle_runtime.rs`、`writing_regenerate.rs`、`mod.rs` 为纯 helper/模块声明）。

**(b) 定义 ↔ 注册一致性**：从 `#[tauri::command]` 后继行提取 fn 名得到集合 A（175，无重名）；从 `lib.rs` 的 `generate_handler![...]` 提取条目名（去 `card_studio_api::` 前缀）得到集合 B（175，无重复）。

- `A - B` = ∅（**无"定义了但未注册"的命令**）
- `B - A` = ∅（**无"注册了但不存在的命令"**）

**(c) 前端 invoke 集合**：完全复刻 `scripts/architecture/backend-baseline.mjs:55-67` 的三条正则（`\binvoke\(`、`\._invoke\(`、`(?<![\w])command:`），对 `frontend/src/**/*.{js,mjs,vue}` 扫描：

- 该实现给出的 `uniqueInvokeCount = 169`，与 `tauri-command-contract.test.mjs:24` 的断言一致；
- 但把手写正则换成"任意 `invoke(` 子串匹配"后，前端真实 invoke 唯一名 = **172**，差集恰为 3 个 `card_shell_*` 命令（见 T-01）。

**(d) 参数契约全量比对**：解析 175 个命令的 Rust 形参名（跳过 `State<`/`AppHandle`/`Window`/`WebviewWindow`/`Request`）→ 按 Tauri 默认 `ArgumentCase::Camel` 折算成期望键名；解析 165 个 JS `invoke(...)` 站点的实参对象键名；逐站点求差集。

- 结果：**HARD 漂移 1 处**（`add_variant` 的 `provenance`），**0 处 snake_case 误用**，其余 164 个站点键名与后端期望完全一致。

**(e) 落盘/吞错/阻塞统计**

```powershell
# 生产路径 unwrap/expect/panic（以文件中首个 #[cfg(test)] 为界）
PRODUCTION-ONLY unwrap/expect/panic hits = 20   # commands/ 内仅 1 处
# let _ = / .ok(); / if let Ok( 等静默吞错点（同上界）
hits = 73   # card_studio_api.rs 9、commands/writing.rs 9、commands/characters.rs 5、commands/world_info.rs 4 …
# spawn_blocking 使用点
tauri-app/src 全量 = 22 处（commands/ 18 处）
```

**(f) 命令名在测试源中的引用**：以 `crates/tauri-app/src/lib_tests*.rs` + `crates/tauri-app/tests/*.rs`（共 32 文件）为语料，逐命令正则 `\b<cmd>\b` 命中：**50/175 命中，125 个零引用**（下界式指标，见 T-11 说明）。

---

## 2 结论摘要

**P0 = 0，P1 = 3，P2 = 9，P3 = 4。**

1. 命令清单本身是干净的：**175 个 `#[tauri::command]` 与 175 条 `generate_handler!` 注册条目双向一致，无漏注册、无重复注册、无前端调用而缺失的后端命令**；参数契约在 165 个 JS 调用站点中只有 1 处漂移，说明这一层日常维护质量明显高于本项目平均水平。
2. 真正的问题集中在**"测试护栏宣称的保证与实际不符"**与**"成功不等于落盘"**两类：契约测试的正则漏掉 3 个仍在生产使用的 `shellDocUrl.js` `_invoke` 调用（T-01），Card Studio 有 8 处 `let _ = store.update(...)` 丢弃落盘错误（T-02），`card_shell_fetch_url` 以非 async 命令 + `reqwest::blocking` + 30s 超时阻塞 IPC 线程（T-03）。
3. 文档层面唯一实质性失真是 `docs/DOCS-CODE-AUDIT.md` 两处"全部 175 个命令位于 `commands/*.rs`"——实际有 19 个（10.9%）在 `crates/tauri-app/src/card_studio_api.rs`，这正是本域最容易在下一次重构中被误删/漏改的位置（T-13）。

---

## 3 发现清单

### P1

#### T-01　前端契约测试的 invoke 扫描有 3 处盲区，使"每个前端 invoke 都在后端注册"的断言失真

- 严重度：P1　类别：E 测试缺口 / A 目标完成度
- 位置：`scripts/architecture/backend-baseline.mjs:55-67`、`frontend/tests/tauri-command-contract.test.mjs:13-15`、`frontend/src/utils/shellDocUrl.js:60-121`
- 状态：**已核实**　置信度：**高**

`backend-baseline.mjs` 用三条正则收集前端 invoke 名，其中处理 shell-doc 调用的是 `\._invoke\(`（**要求字面量点号**）：

```javascript
// backend-baseline.mjs:58-63
for (const m of source.matchAll(/\binvoke\(\s*['"]([^'"]+)['"]/g)) found.push(m[1])
// shellDoc / adapter wrappers call this._invoke('cmd', ...).
for (const m of source.matchAll(/\._invoke\(\s*['"]([^'"]+)['"]/g)) found.push(m[1])
// Dynamic command tables (plugin-bridge.js API_METHODS) declare command: '...'.
for (const m of source.matchAll(/(?<![\w])command:\s*['"]([^'"]+)['"]/g)) found.push(m[1])
```

但 `shellDocUrl.js` 把它们保存在**模块局部变量** `_invoke` 上，调用处是裸函数调用，前面没有 `.`，因此两条正则都不命中：

```javascript
// frontend/src/utils/shellDocUrl.js:40
let _invoke = null
// :64
  const token = await _invoke('card_shell_register_doc', { html })
// :83
  const token = await _invoke('card_shell_register_module', { source })
// :105 / :120
  return Boolean(await _invoke('card_shell_unregister_doc', { token }))
```

实测复刻结果：按基线正则 `uniqueInvokeCount = 169`（与 `tauri-command-contract.test.mjs:24` 断言一致）；按 `invoke(` 子串匹配的真实唯一名 = **172**；差集恰好是这 3 个命令。

影响：`tauri-command-contract.test.mjs:13-15` 的断言 `baseline.frontend.missingBackendCommands == []` 对这 3 个 live 调用点**永远"通过"**。若将来把 `card_shell_register_doc` / `card_shell_register_module` / `card_shell_unregister_doc` 改名或从 `generate_handler!` 移除，契约测试仍全绿，而卡壳文档注册/释放（V5 CSP 隔离链路的关键环节）会在运行时静默失败。

建议：把扫描正则改为 `(?<![\w])_?invoke\(\s*['"]`（覆盖裸 `_invoke(`），同时把 `tauri-command-contract.test.mjs:24` 的期望值从 169 提到 172；并在测试里补一条"后端已注册命令里不存在前端零引用"的断言方向（当前只断言了 frontend→backend 一个方向）。

#### T-02　Card Studio 8 处 `let _ = store.update(...)` 丢弃落盘错误：命令返回成功但数据可能未落盘

- 严重度：P1　类别：B 逻辑正确性（成功≠落盘）
- 位置：`crates/tauri-app/src/card_studio_api.rs:308, 361, 486, 574, 579, 710, 718, 814`；`crates/tauri-app/src/card_studio_store.rs:53-61`
- 状态：**已核实**　置信度：**高**

`CardStudioStore::update` 明确返回 `Result`，`persist` 失败即 `Err`：

```rust
// crates/tauri-app/src/card_studio_store.rs:53-61
pub fn update(&self, project: CardProject) -> Result<CardProject, String> {
    let mut projects = self.inner.lock().unwrap_or_else(|p| p.into_inner());
    let Some(slot) = projects.iter_mut().find(|p| p.id == project.id) else {
        return Err(format!("写卡项目不存在: {}", project.id));
    };
    *slot = project.clone();
    self.persist(&projects)?;      // ← 磁盘失败在这里变成 Err
    Ok(project)
}
```

但调用方一律丢弃返回值：

```rust
// crates/tauri-app/src/card_studio_api.rs:705-713（cardstudio_run_stage）
    let json = match extract_json_object(&resp.content) {
        Ok(v) => v,
        Err(e) => {
            project.set_stage_status(&stage_id, StageStatus::Failed);
            project.last_error = Some(e.clone());
            let _ = store.update(project);        // ← 落盘失败被吞
            return Err(TauriCommandError::validation(e));
        }
    };
```

```rust
// crates/tauri-app/src/card_studio_api.rs:304-310（cardstudio_run_review）
    if !use_llm {
        project.last_stage_output =
            Some(serde_json::to_string_pretty(&rule_report).unwrap_or_else(|_| "{}".into()));
        project.touch();
        let _ = store.update(project);            // ← 落盘失败被吞，仍返回 Ok(rule_report)
        return Ok(rule_report);
    }
```

8 处分别落在 5 个命令：`cardstudio_run_review`（308、361）、`cardstudio_complete_manual_stage`（486）、`cardstudio_prefill_from_novel`（574、579）、`cardstudio_run_stage`（710、718）、`cardstudio_import_compiled`（814）。注意 308 与 814 是**直接返回 Ok 的路径**——磁盘写失败时用户看到"成功"，但项目状态（阶段输出/`last_error`/artifacts）在重启后回到旧值，属于"命令成功不代表已落盘"的契约违反；710/718 虽随后返回 `Err`，但用户看到的错误是 JSON 解析失败，真实原因（未落盘）被掩盖，重试会重复消费 LLM 调用。

建议：统一改 `store.update(project).map_err(|e| TauriCommandError::storage(e))?;`；错误路径上的"尽力保存失败状态"至少要 `tracing::error!` 并把落盘失败并入返回的错误信息。

#### T-03　`card_shell_fetch_url` 为非 async 命令 + `reqwest::blocking` + 30s 超时，在 IPC 处理线程同步阻塞

- 严重度：P1　类别：B 逻辑正确性（阻塞 IO）
- 位置：`crates/tauri-app/src/commands/card_shell.rs:147-163`、`crates/tauri-app/src/card_shell_cache.rs:89`、`:250-255`、`:310-315`
- 状态：**已核实**（框架线程语义由 tauri-macros 源码佐证）　置信度：**高**

```rust
// crates/tauri-app/src/commands/card_shell.rs:147-155
/// 宿主代持拉取远程壳资源（allowlist + 磁盘缓存）。失败显式返回错误，不降级为空成功。
#[tauri::command]
pub(crate) fn card_shell_fetch_url(
    url: String,
) -> Result<card_shell_cache::ShellFetchResult, TauriCommandError> {
    let cache = get_card_shell_cache();
    let client = cache.build_client().map_err(TauriCommandError::internal)?;
    cache
        .fetch_blocking_with_client(&url, &client)
```

```rust
// crates/tauri-app/src/card_shell_cache.rs:89（超时）/ :251-255（阻塞客户端）
            timeout: Duration::from_secs(30),
...
    pub fn fetch_blocking_with_client(
        &self,
        url: &str,
        client: &reqwest::blocking::Client,
    ) -> Result<ShellFetchResult, String> {
```

线程语义（读自框架源码，非文档转述）：`tauri-macros-2.6.3/src/command/wrapper.rs:50` 默认 `execution_context: ExecutionContext::Blocking`；`:158-160` 只把 **async fn** 提升为 `Async`；`:248-253` 据此分派到 `body_blocking`，`:429-435` 是**原地同步调用** `let result = $path(...)`，无线程卸载；只有 `ExecutionContext::Async` 才走 `respond_async_serialized` → `tauri-2.11.5/src/ipc/mod.rs:375` 的 `async_runtime::spawn`。

本仓库 175 个命令中 **151 个是非 async**（`Select-String '\basync\s+fn'` 计数：async 24 / 非 async 151），因此它们都在 IPC handler 上下文内联执行。前端每个"壳发起 fetch"都会走这条链路：

```javascript
// frontend/src/components/CardShellHost.vue:291-292
async function hostFetch(url) {
  const res = await cardShellFetchUrl(url)
// frontend/src/components/TavernHelperRuntime.vue:642-643
async function hostFetchText(url) {
  const res = await cardShellFetchUrl(url)
```

影响：单次桥接 fetch 最长阻塞 30 秒（超时上限），期间窗口不处理其他 IPC/事件循环工作。同一族里 `card_shell_fetch_url` 是唯一发网络的成员，其余 `card_shell_*` 只做内存/磁盘操作。

建议：改成 `pub async fn card_shell_fetch_url(...)` 并把 `fetch_blocking_with_client` 包进 `tokio::task::spawn_blocking`（仓库已有 22 处同类先例，例如 `commands/connections.rs:277`、`commands/turns.rs:598`），或换用异步 `reqwest::Client`。

---

### P2

#### T-04　`add_variant` 参数契约漂移：前端传 `provenance`，后端不接受，Tauri 静默忽略

- 严重度：P2　类别：A/B 契约不匹配
- 位置：`frontend/src/tauri-api.js:643-649`、`crates/tauri-app/src/commands/turns.rs:766-780`
- 状态：**已核实**　置信度：**高**

```javascript
// frontend/src/tauri-api.js:643-649
/** 添加新变体（分支/swipe），返回新 variant 索引 */
export async function addVariant(conversationId, nodeId, content, provenance) {
  if (isTauri()) {
    return await invoke('add_variant', { conversationId, nodeId, content, provenance: provenance || null })
  }
  return 0
}
```

```rust
// crates/tauri-app/src/commands/turns.rs:766-779
#[tauri::command]
pub(crate) fn add_variant(
    conversation_id: String,
    node_id: String,
    content: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    state
        .conv_store
        .add_variant(&conv_id, &nid, content, None)   // ← provenance 硬编码 None
```

Tauri 只按 camelCase 键逐个取参（`tauri-2.11.5/src/ipc/command.rs:97` 的 `v.get(self.key)`），**多余的 `provenance` 键被静默丢弃**，而 store 层的第 4 个参数（`Option<Provenance>`）永远为 `None`。这是 165 个调用站点中唯一的 HARD 漂移。

影响：`addVariant(..., provenance)` 这一 API 表面承诺了记录来源信息的能力，实际永不生效。当前仓库内唯一调用方传 `null`（`frontend/src/composables/useMessageVariants.js:469`），因此**今日无数据损失**，但这是一个静默失效的契约，任何新增调用方都会踩空。

建议：二者取一——后端补 `provenance: Option<Provenance>` 形参并透传给 `add_variant(..., provenance)`，或前端删掉该参数并把 store 层第 4 参一并简化，避免"看起来支持实际不支持"。

#### T-05　错误 DTO 不统一：`card_shell_register_doc` / `card_shell_register_module` 返回裸字符串错误

- 严重度：P2　类别：B/D 错误处理
- 位置：`crates/tauri-app/src/commands/card_shell.rs:114-122`
- 状态：**已核实**　置信度：**高**

```rust
#[tauri::command]
pub(crate) fn card_shell_register_doc(html: String) -> Result<String, String> {
    shell_doc_protocol::register_shell_doc(html)
}

#[tauri::command]
pub(crate) fn card_shell_register_module(source: String) -> Result<String, String> {
    shell_doc_protocol::register_shell_module(source)
}
```

统计：149 个返回 `Result` 的命令中，147 个以 `TauriCommandError` 为错误类型（→ 前端拿到 `{type, message}` 结构化 DTO），只有这 2 个是 `Result<_, String>`（另 26 个命令不返回 `Result`）。

影响：`frontend/src/utils/errorText.js:13-15` 对裸字符串走 `typeof e === 'string'` 分支，UI 文案不会崩，但本项目"Tauri 命令错误是结构化 DTO"的约定被破坏：这两条路径没有 `type`，无法被 `isCancelledError` 之类的分类逻辑处理，也无法区分"输入过大/校验失败"与"内部错误"。属于约定一致性缺陷，非功能性故障。

建议：改为 `Result<String, TauriCommandError>`，`shell_doc_protocol::register_shell_doc` 的错误经 `TauriCommandError::validation`（超限）/`internal`（其它）分流。

#### T-06　变量写命令不校验键名：`__storyforge*` 内部命名空间的守卫只存在于前端

- 严重度：P2　类别：B/D 参数校验、防御纵深
- 位置：`crates/tauri-app/src/commands/variables.rs:27-54`、`:284-307`；`crates/domain/src/campaign.rs:223-244`；对照 `frontend/src/utils/shellVariableOutbox.js:59-62`
- 状态：**已核实**　置信度：**高**

后端 `set_campaign_variable` / `set_character_variable` 对 `key` **不做任何校验**（对比同文件的 `add_campaign_variable` → `validate_campaign_variable_input`，`variables.rs:133-165`，会拒绝长度 >128、非白名单字符、以及含 `__` 段）：

```rust
// crates/tauri-app/src/commands/variables.rs:284-300
#[tauri::command]
pub fn set_campaign_variable(
    campaign_id: String,
    key: String,
    value: serde_json::Value,
    turn: Option<u32>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // 三.5：活动 Turn 屏障 + 读改写 + 写盘在同一原子单元内完成。
    let campaign_id = Id::from_str(&campaign_id);
    let applied = state
        .storage()
        .mutate_idle_campaign(&campaign_id, |camp| {
            camp.set_variable(&key, value.clone(), turn.unwrap_or(0));   // ← 无键名校验
```

```rust
// crates/domain/src/campaign.rs:223-239（domain 层同样直接写）
    pub fn set_variable(&mut self, key: &str, value: serde_json::Value, turn: u32) {
        ...
        if let Some(v) = self.variables.iter_mut().find(|v| v.key == key) {
            v.value = value.clone();
            v.last_updated_turn = turn;
        } else {
            self.variables.push(VariableValue::new(key, value, turn));
        }
```

而 `__storyforge*` 保留命名空间的拒绝只写在前端：

```javascript
// frontend/src/utils/shellVariableOutbox.js:59-62
  // M-6：拒绝 __storyforge* 内部命名空间。dispatchMvuInteraction（用户点 MVU 按钮）
  // 覆盖内部命名空间（如 __storyforge_card_shell_variables）。与 buildMvuStatDataTree
```

影响：`__storyforge_card_shell_variables` 等内部键经 `set_campaign_variable` 可直接落库，绕过 M-6 守卫；`frontend/src/utils/mvuStatTree.js:16` 正是靠这个前缀把内部键排除出 `stat_data` 树的。当前不可从 UI 触达（shell 运行在隔离 origin，插件通道是固定 allowlist），因此**不是可利用漏洞，而是防御纵深缺口**——项目自身对插件通道已确立"前端权限数组不再是唯一边界"的标准（`frontend/src/plugin-bridge.js:185-190`），变量键约束却仍是前端唯一。

建议：把 `validate_campaign_variable_input` 的键名规则（或至少 `__` 段拒绝）下沉为 domain 层 `set_variable` 的前置校验，两条写命令都走同一函数。

#### T-07　`log_clear` 对未知 `kind` 静默降级为"清空全部日志"

- 严重度：P2　类别：B 参数校验 / 静默吞错
- 位置：`crates/tauri-app/src/commands/diagnostics.rs:145-154`；`crates/app-logging/src/lib.rs:230-240`
- 状态：**已核实**　置信度：**高**

```rust
// crates/tauri-app/src/commands/diagnostics.rs:145-154
#[tauri::command]
pub(crate) fn log_clear(kind: Option<String>, state: tauri::State<'_, Arc<AppState>>) {
    let log_kind = kind.as_deref().and_then(|k| match k {
        "backend" => Some(LogKind::Backend),
        "llm" => Some(LogKind::LlmCall),
        "frontend" => Some(LogKind::FrontendPlugin),
        _ => None,                       // ← 未知 kind 与"清全部"共用 None
    });
    state.log_store.clear(log_kind);
}
```

```rust
// crates/app-logging/src/lib.rs:230-240
    pub fn clear(&mut self, kind: Option<LogKind>) {
        if let Some(k) = kind {
            if let Some(buf) = self.entries.get_mut(&k) { buf.clear(); }
        } else {
            for buf in self.entries.values_mut() { buf.clear(); }   // ← None = 清全部
        }
    }
```

影响：`log_clear(Some("llm-call"))`、`Some("all")` 或任何拼写变体都会**清空三类日志**（含 LLM 调用审计日志），且返回 `()` 无任何提示。前端 `frontend/src/components-v2/debug/LogPanel.vue:139` 是唯一调用点，取值为本组件自造，目前不会误传；但命令是公开 IPC 表面，语义上"参数非法 ⇒ 扩大破坏面"是危险默认值。

建议：`kind` 改为非 `Option` 枚举（serde 拒绝未知值）或对未知值返回 `Err(TauriCommandError::validation(...))`；"清全部"用显式 `"all"` 表达。

#### T-08　世界书四条写命令用 `if let Ok(book)` 吞掉重读错误，`tool_ctx` 世界书可能停留在旧值

- 严重度：P2　类别：B 状态刷新一致性
- 位置：`crates/tauri-app/src/commands/world_info.rs:255-257, 303-305, 340-342, 360-362`
- 状态：**已核实**（是否可实际发生未验证）　置信度：**中**

```rust
// crates/tauri-app/src/commands/world_info.rs:299-306（update_campaign_world_info_entry）
    state
        .storage()
        .update_world_info_entry(&id, req.entry_index, entry)
        .map_err(TauriCommandError::storage)?;          // ← 落盘错误正确传播
    if let Ok(book) = state.storage().get_world_info(&id) {
        apply_campaign_world_info_to_tool_ctx(state.inner(), &id, &book);
    }
    Ok(())                                                // ← 重读失败静默跳过刷新
```

同样写法出现在 `add_campaign_world_info_entry:255`、`delete_campaign_world_info_entry:340`、`set_campaign_world_info_route:360`。对照 `commands/cards.rs:181` 的注释（"不得用 if let Ok / .ok() 吞掉——否则删除成功却漏清关联数据"）可知项目已明确不认可这种写法。

影响：落盘已成功，但进程内的 `tool_ctx` 世界书快照未刷新 → 后续写作轮次按旧条目注入。触发需要"写成功 + 立刻重读失败"，现实概率低，故定 P2 且标注为**疑似可发生**；但它与同一仓库已确立的约定不一致。

建议：重读失败至少 `tracing::error!` 记录；更稳妥的做法是让 `update_world_info_entry` 直接返回更新后的书，省掉这次重读（`set_world_info_entry_enabled:322` 就是这么做的，是正确范式）。

#### T-09　提权型 wrapper `cardShellAllowHost` 无任何调用点，且命令本身无审核/确认

- 严重度：P2　类别：C 死代码 / D 最小权限
- 位置：`frontend/src/tauri-api.js:1186-1189`、`crates/tauri-app/src/commands/card_shell.rs:129-136`
- 状态：**已核实**　置信度：**高**

```javascript
// frontend/src/tauri-api.js:1186-1189
export async function cardShellAllowHost(host) {
  if (isTauri()) {
    return await invoke('card_shell_allow_host', { host })
  }
}
```

```rust
// crates/tauri-app/src/commands/card_shell.rs:129-136
#[tauri::command]
pub(crate) fn card_shell_allow_host(host: String) -> Result<(), TauriCommandError> {
    if host.trim().is_empty() {
        return Err(TauriCommandError::validation("host 为空"));
    }
    get_card_shell_cache().allow_host(&host);
    Ok(())
}
```

全量 grep（`frontend/src`、`scripts/`、`docs/` 的 tracked 文件）中 `cardShellAllowHost` 只在这两行出现，**无 UI 入口**。而 `card_shell_list_allowed_hosts` 正是卡壳 CSP 的网络向白名单来源（`crates/tauri-app/src/lib.rs:1445`，文档称"网络向指令钉死 `card_shell_list_allowed_hosts` 白名单"）。

影响：一个能永久放宽卡壳网络白名单的命令，其唯一调用方（wrapper）无人使用；一旦后续 UI 接线，若不加用户确认，任何前端脚本（含被注入的壳侧桥）都能扩白名单。当前不可达，故 P2。

建议：接线时必须走用户确认（对齐 shell 变量提案 `ShellVariableProposalBar` 的模式）；或在无入口期间把 wrapper 与命令一并标注为 debug-only。

#### T-10　契约测试与 capability 测试都不在 CI 中执行，snapshot 无再生成路径

- 严重度：P2　类别：E 测试缺口
- 位置：`.github/workflows/release.yml:63-65, 148-150`；`frontend/package.json:7`
- 状态：**已核实**　置信度：**高**

```yaml
# .github/workflows/release.yml:63-65（desktop job）；:148-150（android job）同形
      - run: |
          npm ci
          npm run build
```

全 workflow 无 `npm test` / `node --test`：`Select-String .github/workflows/release.yml -Pattern 'npm'` 只命中 `cache: npm`、`npm ci`、`npm run build`。而契约测试挂在 `frontend/package.json:7` 的 `"test": "node --test \"tests/*.test.mjs\" ..."` 上（`tauri-command-contract.test.mjs` 位于 `tests/` 根，会被该 glob 收录）。

同时，`frontend/tests/fixtures/tauri-registered-commands.snapshot.json` 只被 `tauri-command-contract.test.mjs:10` 读取，**全仓库没有任何生成/更新脚本或文档提及它**（`Select-String -Pattern 'tauri-registered-commands\.snapshot'` 仅 1 处命中；`Select-String -Pattern 'backend-baseline' -Include *.md` 无命中）。实际可行的再生成路径是 `node scripts/architecture/backend-baseline.mjs` 后手工取 `backend.registeredCommands`，属口口相传。

影响：新增命令后必须手工同步 snapshot，而唯一会因忘记同步而失败的门禁不在 CI 里 —— 本地忘了跑 `npm test` 就会直接进 release 分支。这与 `RELEASE-STATUS.md:32`"两轮完整 11 步门禁全绿且计数完全一致"的表述不矛盾（那是指本地门禁），但 CI 无兜底。

建议：release.yml 增加 `npm test` 步骤；在 `backend-baseline.mjs` 加一个 `--write-snapshot` 开关并写进 `DOCS-CODE-AUDIT.md`。

#### T-11　175 个命令中 125 个命令符号在 tauri-app 测试源中零引用

- 严重度：P2　类别：E 测试缺口
- 位置：`crates/tauri-app/src/lib_tests*.rs`（10 文件）、`crates/tauri-app/tests/*.rs`
- 状态：**已核实**（指标定义见下）　置信度：**中**

统计命令：以 22 个 `lib_tests*.rs` + `tests/*.rs`（共 32 文件）拼接为语料，逐命令做 `\b<cmd_name>\b` 匹配 → **命中 50、零引用 125**。

**指标口径警告**：这是"命令**符号**零引用"的下界式指标，不等价于"命令无测试覆盖"。相当一部分命令是薄 wrapper（如 `get_version` → `env!("CARGO_PKG_VERSION")`），其底层 helper 已被测试覆盖；少数由 `lib_tests.rs` 之外的模块内 `#[cfg(test)]` 覆盖。零引用集合中风险最高的子集是**自带分支逻辑且零测试**的命令，典型如：

- `cardstudio_*` 19 个命令中 13 个零引用（含带 8 处吞落盘错误的 `cardstudio_run_review` / `cardstudio_run_stage` / `cardstudio_import_compiled`）
- `commands/plugins.rs` 全部 7 个 `plugin_*` 通道命令零引用（这族命令是插件沙箱的唯一后端边界）
- `add_variant` / `switch_variant` / `edit_variant` / `abandon_turn` / `archive_conversation` / `soft_delete_variant` 全部零引用

建议：优先为 T-02 涉及的 5 个 Card Studio 命令补"落盘失败 ⇒ 命令返回 Err"的命令级测试（可用只读目录/注入失败 store 制造 persist 失败）；其次给 `plugin_*` 7 条通道补注册校验测试。

#### T-12　大载荷 import/export 命令为非 async，最多在 IPC 线程内完成 16 MiB JSON 解析 + 多文件 IO

- 严重度：P2　类别：B 阻塞 IO
- 位置：`crates/tauri-app/src/commands/import_export.rs:404-428, 434-553, 558-565, 605-...`
- 状态：**已核实**（载荷上限有注释与常量佐证）　置信度：**中**

```rust
// crates/tauri-app/src/commands/import_export.rs:555-565
/// 导出 StoryForge Campaign 完整 JSON Bundle
///
/// 包含 Campaign 元数据 + Instances + Definitions + Knowledge + Tasks + Summaries。
#[tauri::command]
pub(crate) fn export_campaign_bundle(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    let camp_id = Id::from_str(&campaign_id);
    state.storage().export_campaign_bundle(&camp_id)
}
```

同族 `import_campaign_bundle`（`:605-611`）在函数体内明写上限注释："H-2: bound the IPC payload before deserializing (16 MiB)"，与 `error.rs:255` 的 `MAX_BUNDLE_JSON_BYTES = 16 * 1024 * 1024` 对应；`export_campaign_st_cards`（`:435`）还要为每个实例生成 PNG。

影响：与 T-03 同源（非 async ⇒ 在 IPC 线程内联执行），但载荷为受控上限的本地 IO，单次耗时量级远低于 30s 网络超时，且是用户显式点击的一次性操作。故与 T-03 分开定级 P2，建议与 T-03 一并处理（`spawn_blocking` 包裹）。

---

### P3

#### T-13　文档漂移：`DOCS-CODE-AUDIT.md` 两处声称"全部 175 个命令位于 `commands/*.rs`"

- 严重度：P3　类别：F 文档漂移
- 位置：`docs/DOCS-CODE-AUDIT.md:11`、`docs/DOCS-CODE-AUDIT.md:54`
- 状态：**已核实**　置信度：**高**

```
docs/DOCS-CODE-AUDIT.md:11
- 命令数量与位置（修正 2026-07-26 口径）：Gate 1 拆分后 `crates/tauri-app/src/lib.rs` 已不含 `#[tauri::command]`（仅装配），全部 **175 个**命令分布在 `crates/tauri-app/src/commands/*.rs`。README 的「175 个命令」与当前代码一致。

docs/DOCS-CODE-AUDIT.md:54
- Tauri command 当前共 **175 个**（2026-09-01 核对），全部位于 `crates/tauri-app/src/commands/*.rs`（Gate 1 已把 lib.rs 拆分至 ~1.5k 行装配层；…）
```

实测（1.2(a)）：总数 175 ✔、`lib.rs` 中 0 个 ✔，但 `commands/*.rs` 只占 **156**，另有 **19** 个在 `crates/tauri-app/src/card_studio_api.rs`。

影响：数字对、**位置错**。`commands/mod.rs` 不含 `card_studio_api`，`lib.rs` 注册块用 `card_studio_api::` 前缀（`:1301-1319`）区分，而 Gate 1 的静态测试只断言 `lib.rs` 里没有 `#[tauri::command]`（`frontend/tests/tauri-command-contract.test.mjs:57-59`）——这意味着**没有任何门禁保护 `card_studio_api.rs` 这 19 个命令的位置约束**，而文档恰好把它们描述成在 `commands/` 里。

建议：把两处改为"156 个在 `commands/*.rs`，19 个在 `src/card_studio_api.rs`"；顺带把 Card Studio 19 个命令移入 `commands/card_studio.rs`（并同步 `lib.rs` 注册前缀）以消除特例。

#### T-14　文档漂移：`CLAUDE.md` 称 `cardShellClearCache` "UI 入口未接"，实际已接线

- 严重度：P3　类别：F 文档漂移
- 位置：`CLAUDE.md:116`；`frontend/src/components-v2/shell/InspectorDrawer.vue:18, 45`
- 状态：**已核实**　置信度：**高**

```
CLAUDE.md:116
  - L1：… L6：`card_shell_clear_cache` 命令 + wrapper `cardShellClearCache`（UI 入口未接）。
```

```javascript
// frontend/src/components-v2/shell/InspectorDrawer.vue:18
import { cardShellClearCache } from '../../tauri-api.js'
// :45
    const n = await cardShellClearCache()
```

影响：仅文档失真，但 `CLAUDE.md` 是本项目的"必读事实清单"，错误事实会被后续 Agent 当成约束（例如据此认为该命令是死代码而删除）。建议更新为已接线（入口：InspectorDrawer）。

#### T-15　3 个孤儿命令：后端已注册，前端全仓库无入口

- 严重度：P3　类别：C 死代码
- 位置：`crates/tauri-app/src/commands/turns.rs:673`（`abandon_turn`）、`commands/conversations.rs:24`（`archive_conversation`）、`commands/turns.rs:636`（`soft_delete_variant`）
- 状态：**已核实**　置信度：**高**

差集来源：后端 175 个已注册命令 − 前端真实 invoke 集合 172 = 3。这 3 个命令在 `frontend/src`（含 `tauri-api.js`、`plugin-bridge.js` 动态表、`.vue` 直调）、`scripts/`、`docs/` 中均无任何引用：

```powershell
git grep -n -- abandon_turn        # 仅 crates/tauri-app/src 与 snapshot fixture 命中（被过滤后为空）
git grep -n -- archive_conversation # 同上
```

对照佐证：`frontend/tests/tauri-command-contract.test.mjs:23` 的注释记录了 2026-09-01 主动删除零引用 wrapper（"移除零引用 wrapper softDeleteVariant/archiveConversation → 171→169"）——**wrapper 删了，命令与注册条目留着**。

影响：死 API 表面（约 1.7% 的命令），会被 devtools/未来的自动生成工具当成活接口；`archive_conversation` 的 ACTIVE 语义在 `DOCS-CODE-AUDIT.md:106` 仍被引用为可复用 helper。建议：要么补回入口（若产品仍需"归档"与"放弃轮次"），要么连同 `generate_handler!` 条目一起删除并同步 snapshot（注意：删除前须确认 `docs/` 中相关描述同步更新）。

#### T-16　15 个命令未在 `tauri-api.js` 中 wrapper 化：契约图的第二类"孤儿"需按入口分层看

- 严重度：P3　类别：A 契约完整性（说明性）
- 位置：见 4.2
- 状态：**已核实**　置信度：**高**

`tauri-api.js` 有 160 个 `export async function` wrapper、160 个 `invoke(...)`，与后端 160 条一一对应；剩下 15 个命令的入口在别处，必须分层看，否则会误判为孤儿：

| 类别 | 数量 | 命令 | 入口 |
| --- | --- | --- | --- |
| 真孤儿 | 3 | `abandon_turn`、`archive_conversation`、`soft_delete_variant` | 无（见 T-15） |
| 插件通道（动态表） | 6 | `plugin_list_characters`、`plugin_read_character`、`plugin_read_world_info`、`plugin_get_conversation`、`plugin_get_variable`、`plugin_set_variable` | `plugin-bridge.js:182-190` 的 `API_METHODS`，经 `plugin-bridge.js:1899` `invoke(method.command, params)` 转发 |
| 壳文档协议 | 3 | `card_shell_register_doc`、`card_shell_register_module`、`card_shell_unregister_doc` | `frontend/src/utils/shellDocUrl.js:64/83/105/120` 的裸 `_invoke`（正是 T-01 的盲区） |
| MVU 运行时 | 3 | `mvu_execute_result`、`mvu_load_ack`、`mvu_unload_ack` | `frontend/src/components/MvuJsRuntime.vue:385/405/447/433/377` |

建议：把这张分层表固化进 `docs/DOCS-CODE-AUDIT.md`（或直接由 `backend-baseline.mjs` 输出），使"孤儿"判断有单一权威口径。

---

## 4 契约核对表（后端命令 ↔ 前端 wrapper）

### 4.1 逐命令全量清单（175 行）

- "前端入口"列：`` `wrapperName` `` 表示 `frontend/src/tauri-api.js` 的导出函数；其余为直接 invoke 站点；**无前端入口** = 全仓库零引用（= T-15）。
- 后端 175 条全部位于 `crates/tauri-app/src/`（相对路径省略该前缀；`card_studio_api.rs` 为 `src/` 根，其余为 `src/commands/`）。
- 注册列：175/175 全部在 `crates/tauri-app/src/lib.rs:1294-1487` 的 `generate_handler!` 中，无遗漏、无重复，故不再单列。

| 后端命令 | 定义位置 | 前端入口 |
| --- | --- | --- |
| `abandon_task` | commands\memory.rs:322 | `abandonTask` |
| `abandon_turn` | commands\turns.rs:673 | **无前端入口** |
| `accept_variant` | commands\turns.rs:508 | `acceptVariant` |
| `add_campaign_instance` | commands\campaigns.rs:1235 | `addCampaignInstance` |
| `add_campaign_variable` | commands\variables.rs:169 | `addCampaignVariable` |
| `add_campaign_world_info_entry` | commands\world_info.rs:224 | `addCampaignWorldInfoEntry` |
| `add_variant` | commands\turns.rs:768 | `addVariant`（参数漂移，见 T-04） |
| `add_world_info_entry` | commands\characters.rs:492 | `addWorldInfoEntry` |
| `apply_campaign_opening` | commands\campaigns.rs:486 | `applyCampaignOpening` |
| `archive_conversation` | commands\conversations.rs:24 | **无前端入口** |
| `cancel_writing` | commands\writing.rs:1677 | `cancelWriting` |
| `card_shell_allow_host` | commands\card_shell.rs:130 | `cardShellAllowHost`（wrapper 无调用点，见 T-09） |
| `card_shell_clear_cache` | commands\card_shell.rs:141 | `cardShellClearCache` |
| `card_shell_fetch_url` | commands\card_shell.rs:149 | `cardShellFetchUrl` |
| `card_shell_list_allowed_hosts` | commands\card_shell.rs:106 | `cardShellListAllowedHosts` |
| `card_shell_register_doc` | commands\card_shell.rs:115 | frontend\src\utils\shellDocUrl.js:64 |
| `card_shell_register_module` | commands\card_shell.rs:120 | frontend\src\utils\shellDocUrl.js:83 |
| `card_shell_unregister_doc` | commands\card_shell.rs:125 | frontend\src\utils\shellDocUrl.js:105<br>frontend\src\utils\shellDocUrl.js:120 |
| `cardstudio_compile` | card_studio_api.rs:366 | `cardstudioCompile` |
| `cardstudio_complete_manual_stage` | card_studio_api.rs:454 | `cardstudioCompleteManualStage` |
| `cardstudio_create_from_character` | card_studio_api.rs:144 | `cardstudioCreateFromCharacter` |
| `cardstudio_create_from_novel` | card_studio_api.rs:103 | `cardstudioCreateFromNovel` |
| `cardstudio_create_project` | card_studio_api.rs:85 | `cardstudioCreateProject` |
| `cardstudio_delete_project` | card_studio_api.rs:198 | `cardstudioDeleteProject` |
| `cardstudio_export_gate` | card_studio_api.rs:423 | `cardstudioExportGate` |
| `cardstudio_export_png` | card_studio_api.rs:437 | `cardstudioExportPng` |
| `cardstudio_get_project` | card_studio_api.rs:187 | `cardstudioGetProject` |
| `cardstudio_import_compiled` | card_studio_api.rs:747 | `cardstudioImportCompiled` |
| `cardstudio_list_projects` | card_studio_api.rs:73 | `cardstudioListProjects` |
| `cardstudio_list_stages` | card_studio_api.rs:825 | `cardstudioListStages` |
| `cardstudio_prefill_from_novel` | card_studio_api.rs:511 | `cardstudioPrefillFromNovel` |
| `cardstudio_run_checks` | card_studio_api.rs:278 | `cardstudioRunChecks` |
| `cardstudio_run_review` | card_studio_api.rs:291 | `cardstudioRunReview` |
| `cardstudio_run_stage` | card_studio_api.rs:645 | `cardstudioRunStage` |
| `cardstudio_set_options` | card_studio_api.rs:248 | `cardstudioSetOptions` |
| `cardstudio_set_stage` | card_studio_api.rs:227 | `cardstudioSetStage` |
| `cardstudio_update_artifacts` | card_studio_api.rs:209 | `cardstudioUpdateArtifacts` |
| `clear_global_regex_scripts` | commands\presets.rs:248 | `clearGlobalRegexScripts` |
| `complete_task` | commands\memory.rs:299 | `completeTask` |
| `configure_embedder` | commands\connections.rs:6 | `configureEmbedder` |
| `create_campaign` | commands\campaigns.rs:311 | `createCampaign` |
| `create_connection` | commands\connections.rs:170 | `createConnection` |
| `create_task` | commands\memory.rs:269 | `createTask`（`createdTurn` 从不传，见 5.2） |
| `delete_agent_profile_config` | commands\profiles.rs:161 | `deleteAgentProfileConfig` |
| `delete_campaign` | commands\campaigns.rs:914 | `deleteCampaign` |
| `delete_campaign_world_info_entry` | commands\world_info.rs:329 | `deleteCampaignWorldInfoEntry` |
| `delete_card` | commands\cards.rs:174 | `deleteCard` |
| `delete_character` | commands\characters.rs:278 | `deleteCharacter` |
| `delete_connection` | commands\connections.rs:254 | `deleteConnection` |
| `delete_conversation` | commands\conversations.rs:347 | `deleteConversation` |
| `delete_message_from` | commands\turns.rs:753 | `deleteMessageFrom` |
| `delete_preset` | commands\presets.rs:175 | `deletePreset` |
| `delete_world_info_entry` | commands\characters.rs:527 | `deleteWorldInfoEntry` |
| `edit_variant` | commands\turns.rs:484 | `editVariant` |
| `export_agent_profile_config` | commands\profiles.rs:133 | `exportAgentProfileConfig` |
| `export_campaign_bundle` | commands\import_export.rs:559 | `exportCampaignBundle` |
| `export_campaign_st_cards` | commands\import_export.rs:435 | `exportCampaignStCards` |
| `export_st_card_png` | commands\import_export.rs:405 | `exportStCardPng` |
| `extract_characters` | commands\campaigns.rs:82 | `extractCharacters` |
| `fork_campaign` | commands\campaigns.rs:832 | `forkCampaign` |
| `get_active_agent_profile_config` | commands\profiles.rs:104 | `getActiveAgentProfileConfig` |
| `get_active_campaign` | commands\campaigns.rs:1077 | `getActiveCampaign` |
| `get_active_connection` | commands\connections.rs:120 | `getActiveConnection` |
| `get_active_preset` | commands\presets.rs:140 | `getActivePreset` |
| `get_active_profile` | commands\profiles.rs:43 | `getActiveProfile` |
| `get_active_turn_quality` | commands\turns.rs:25 | `getActiveTurnQuality` |
| `get_active_turn_receipt` | commands\turns.rs:7 | `getActiveTurnReceipt` |
| `get_agent_profile_config` | commands\profiles.rs:96 | `getAgentProfileConfig` |
| `get_campaign` | commands\campaigns.rs:898 | `getCampaign` |
| `get_campaign_variable_schema` | commands\variables.rs:79 | `getCampaignVariableSchema` |
| `get_campaign_variables` | commands\variables.rs:58 | `getCampaignVariables` |
| `get_campaign_world_info_entry` | commands\world_info.rs:441 | `getCampaignWorldInfoEntry` |
| `get_card` | commands\cards.rs:290 | `getCard` |
| `get_card_shell_inline_js` | commands\card_shell.rs:72 | `getCardShellInlineJs` |
| `get_card_shell_manifest` | commands\card_shell.rs:17 | `getCardShellManifest` |
| `get_character` | commands\characters.rs:242 | `getCharacter` |
| `get_character_variables` | commands\variables.rs:5 | `getCharacterVariables` |
| `get_character_world_info` | commands\world_info.rs:369 | `getCharacterWorldInfo` |
| `get_character_world_info_entry` | commands\world_info.rs:405 | `getCharacterWorldInfoEntry` |
| `get_connection` | commands\connections.rs:229 | `getConnection` |
| `get_conversation` | commands\conversations.rs:389 | `getConversation` |
| `get_embed_config` | commands\connections.rs:17 | `getEmbedConfig` |
| `get_instance` | commands\campaigns.rs:1127 | `getInstance` |
| `get_preset` | commands\presets.rs:100 | `getPreset` |
| `get_version` | commands\diagnostics.rs:4 | `getVersion` |
| `import_agent_profile_config` | commands\profiles.rs:144 | `importAgentProfileConfig` |
| `import_campaign_bundle` | commands\import_export.rs:606 | `importCampaignBundle` |
| `import_character` | commands\characters.rs:160 | `importCharacter` |
| `import_global_regex_settings` | commands\presets.rs:233 | `importGlobalRegexSettings` |
| `import_preset` | commands\presets.rs:4 | `importPreset` |
| `import_preset_as_modules` | commands\presets.rs:273 | `importPresetAsModules` |
| `install_plugin` | commands\plugins.rs:62 | `installPlugin` |
| `list_agent_profile_configs` | commands\profiles.rs:89 | `listAgentProfileConfigs` |
| `list_campaign_world_info` | commands\world_info.rs:195 | `listCampaignWorldInfo` |
| `list_campaigns` | commands\campaigns.rs:878 | `listCampaigns` |
| `list_cards` | commands\cards.rs:154 | `listCards` |
| `list_character_knowledge` | commands\memory.rs:171 | `listCharacterKnowledge` |
| `list_characters` | commands\characters.rs:228 | `listCharacters` |
| `list_connection_templates` | commands\connections.rs:93 | `listConnectionTemplates` |
| `list_connections` | commands\connections.rs:99 | `listConnections` |
| `list_conversations` | commands\conversations.rs:318 | `listConversations` |
| `list_global_regex_scripts` | commands\presets.rs:224 | `listGlobalRegexScripts` |
| `list_instances` | commands\campaigns.rs:1101 | `listInstances` |
| `list_models` | commands\connections.rs:590 | `listModels` |
| `list_modules` | commands\profiles.rs:6 | `listModules` |
| `list_plugins` | commands\plugins.rs:52 | `listPlugins` |
| `list_presets` | commands\presets.rs:82 | `listPresets` |
| `list_profiles` | commands\profiles.rs:36 | `listProfiles` |
| `list_round_summaries` | commands\memory.rs:381 | `listRoundSummaries` |
| `list_tasks` | commands\memory.rs:246 | `listTasks` |
| `log_append_frontend` | commands\diagnostics.rs:302 | `logAppendFrontend` |
| `log_clear` | commands\diagnostics.rs:146 | `logClear` |
| `log_export_bundle` | commands\diagnostics.rs:157 | `logExportBundle` |
| `log_get_llm_call` | commands\diagnostics.rs:126 | `logGetLlmCall` |
| `log_query` | commands\diagnostics.rs:50 | `logQuery` |
| `meta_accept_patch` | commands\meta.rs:28 | `metaAcceptPatch` |
| `meta_accept_typed_patch` | commands\meta_typed.rs:280 | `metaAcceptTypedPatch` |
| `meta_analyze_mvu_card` | commands\meta_typed.rs:730 | `metaAnalyzeMvuCard` |
| `meta_apply_mvu_schema` | commands\meta_typed.rs:892 | `metaApplyMvuSchema` |
| `meta_chat` | commands\meta.rs:293 | `metaChat` |
| `meta_classify_st_preset` | commands\meta_typed.rs:1018 | `metaClassifyStPreset` |
| `meta_dismiss_patch` | commands\meta.rs:421 | `metaDismissPatch` |
| `meta_dismiss_typed_patch` | commands\meta_typed.rs:680 | `metaDismissTypedPatch` |
| `meta_explain_generation` | commands\meta.rs:459 | `metaExplainGeneration` |
| `meta_get_conversation` | commands\meta.rs:390 | `metaGetConversation` |
| `meta_get_mvu_translation` | commands\meta_typed.rs:848 | `metaGetMvuTranslation` |
| `meta_health_check` | commands\meta.rs:438 | `metaHealthCheck` |
| `meta_list_mvu_translations` | commands\meta_typed.rs:819 | `metaListMvuTranslations` |
| `meta_list_pending_patches` | commands\meta.rs:406 | `metaListPendingPatches` |
| `meta_list_typed_patches` | commands\meta_typed.rs:190 | `metaListTypedPatches` |
| `meta_preview_mvu_apply` | commands\meta_typed.rs:872 | `metaPreviewMvuApply` |
| `meta_preview_typed_patch` | commands\meta_typed.rs:206 | `metaPreviewTypedPatch` |
| `meta_propose_campaign_repairs` | commands\meta_typed.rs:63 | `metaProposeCampaignRepairs` |
| `meta_start_conversation` | commands\meta.rs:279 | `metaStartConversation` |
| `mvu_execute_result` | commands\mvu.rs:25 | frontend\src\components\MvuJsRuntime.vue:385<br>frontend\src\components\MvuJsRuntime.vue:405<br>frontend\src\components\MvuJsRuntime.vue:447 |
| `mvu_load_ack` | commands\mvu.rs:14 | frontend\src\components\MvuJsRuntime.vue:433 |
| `mvu_unload_ack` | commands\mvu.rs:7 | frontend\src\components\MvuJsRuntime.vue:377 |
| `plugin_get_conversation` | commands\plugins.rs:260 | frontend\src\plugin-bridge.js:187（动态表） |
| `plugin_get_variable` | commands\plugins.rs:181 | frontend\src\plugin-bridge.js:189（动态表） |
| `plugin_list_characters` | commands\plugins.rs:101 | frontend\src\plugin-bridge.js:182（动态表） |
| `plugin_prompt_hook_result` | commands\plugins.rs:244 | `pluginPromptHookResult` |
| `plugin_read_character` | commands\plugins.rs:122 | frontend\src\plugin-bridge.js:183（动态表） |
| `plugin_read_world_info` | commands\plugins.rs:142 | frontend\src\plugin-bridge.js:184（动态表） |
| `plugin_set_variable` | commands\plugins.rs:212 | frontend\src\plugin-bridge.js:190（动态表） |
| `promote_temporary_instance` | commands\variables.rs:311 | `promoteTemporaryInstance` |
| `regenerate` | commands\writing.rs:39 | `regenerate` |
| `retry_active_turn_postprocess` | commands\turns.rs:16 | `retryActiveTurnPostprocess` |
| `save_agent_profile_config` | commands\profiles.rs:111 | `saveAgentProfileConfig` |
| `save_profile` | commands\profiles.rs:53 | `saveProfile` |
| `set_active_agent_profile_config` | commands\profiles.rs:172 | `setActiveAgentProfileConfig` |
| `set_active_campaign` | commands\campaigns.rs:927 | `setActiveCampaign` |
| `set_active_connection` | commands\connections.rs:263 | `setActiveConnection` |
| `set_active_preset` | commands\presets.rs:153 | `setActivePreset` |
| `set_active_profile` | commands\profiles.rs:73 | `setActiveProfile` |
| `set_campaign_variable` | commands\variables.rs:285 | `setCampaignVariable`（键名校验缺口，见 T-06） |
| `set_campaign_world_info_enabled` | commands\world_info.rs:312 | `setCampaignWorldInfoEnabled` |
| `set_campaign_world_info_route` | commands\world_info.rs:347 | `setCampaignWorldInfoRoute` |
| `set_character_variable` | commands\variables.rs:27 | `setCharacterVariable`（键名校验缺口，见 T-06） |
| `set_plugin_enabled` | commands\plugins.rs:87 | `setPluginEnabled` |
| `soft_delete_variant` | commands\turns.rs:636 | **无前端入口** |
| `start_writing` | commands\writing.rs:872 | `startWriting` |
| `storage_health_acknowledge` | commands\diagnostics.rs:16 | `storageHealthAcknowledge` |
| `storage_health_report` | commands\diagnostics.rs:9 | `storageHealthReport` |
| `switch_variant` | commands\turns.rs:784 | `switchVariant` |
| `sync_campaign_variable_schema` | commands\variables.rs:222 | `syncCampaignVariableSchema` |
| `test_connection` | commands\connections.rs:527 | `testConnection` |
| `uninstall_plugin` | commands\plugins.rs:76 | `uninstallPlugin` |
| `update_campaign_world_info_entry` | commands\world_info.rs:275 | `updateCampaignWorldInfoEntry` |
| `update_connection` | commands\connections.rs:245 | `updateConnection` |
| `update_global_regex` | commands\presets.rs:255 | `updateGlobalRegex` |
| `update_module` | commands\profiles.rs:18 | `updateModule` |
| `update_preset_prompt` | commands\presets.rs:187 | `updatePresetPrompt` |
| `update_preset_regex` | commands\presets.rs:206 | `updatePresetRegex` |
| `update_world_info_entry` | commands\characters.rs:462 | `updateWorldInfoEntry` |
| `update_world_info_route` | commands\characters.rs:423 | `updateWorldInfoRoute` |

### 4.2 差集结论

| 方向 | 数量 | 明细 |
| --- | --- | --- |
| **缺失**：前端调用但后端无此命令 | **0** | `baseline.frontend.missingBackendCommands == []`（已核实；注意 T-01 的 3 个盲区不在该集合内，属"漏扫"而非"缺失"） |
| **孤儿**：后端已注册但前端零入口 | **3** | `abandon_turn`、`archive_conversation`、`soft_delete_variant`（T-15） |
| **未 wrapper 化但有入口** | **12** | 6 插件通道 + 3 壳文档协议 + 3 MVU 运行时（T-16） |
| 有 `tauri-api.js` wrapper | **160** | 160 wrapper ↔ 160 invoke ↔ 160 个后端命令，一一对应，无重复指向 |
| 定义未注册 / 注册未定义 | **0 / 0** | 见 1.2(b) |
| **参数签名漂移** | **1 / 165 站点** | `add_variant` 的 `provenance`（T-04）；0 处 snake_case/camelCase 误用 |

---

## 5 未发现问题与低风险观察

### 5.1 明确"未发现问题（已核对范围：…）"

- **命令定义 ↔ 注册一致性**：已核对全部 175 个 `#[tauri::command]` 与 `lib.rs:1294-1487` 的 175 条 `generate_handler!` 条目 → 双向差集为空、无重名命令、无重复注册条目。`commands/mod.rs` 的 21 条模块声明与 `commands/` 下 22 个文件一致（`mod.rs` 本身不列入）。
- **错误 DTO 主体统一性**：已核对 `error.rs` 全文与 175 个命令的返回类型 → 149 个返回 `Result` 的命令中 147 个用 `TauriCommandError`（`#[serde(tag = "type", rename_all = "snake_case")]`，7 个变体）；异常仅 `card_shell.rs` 2 处（T-05）。`error.rs:60-201` 的 `From<LlmError/AgentError/PipelineError/ImportError/ConversationError/MvuApplyError/String/&str>` 映射完整，`retryable` 语义（RateLimited/Timeout/ServerError=true，Auth=false，Cancelled 无 message）自洽，与 `frontend/src/utils/errorText.js:9-35` 的处理分支对得上（含无 `message` 字段的 `cancelled` 分支）。
- **生产路径 panic/unwrap**：已核对 `crates/tauri-app/src`（排除 `lib_tests*`）23 个文件 → 生产代码中 `unwrap/expect/panic!/unreachable!/todo!/unimplemented!` 共 20 处，`commands/` 目录内仅 1 处（`commands/bundle_runtime.rs:39` `expect("card scope checked")`，其前置的第 26-38 行分支已保证 `bundle.card` 为 `Some`，是不变量断言，非缺陷）。`error.rs` 的 12 处全在 `#[cfg(test)] mod tests`。
- **capability 最小权限**：已核对 `capabilities/default.json`（8 条权限）、`crates/tauri-app/tauri.conf.json:20-22`（CSP）、`crates/tauri-app/tests/capabilities.rs` 全文、`frontend/src` 全部插件导入点 → 前端只导入 `@tauri-apps/plugin-dialog`（open/save/message/ask）与 `@tauri-apps/plugin-fs`（readFile/writeFile），无 `dialog:default`、无 `fs:default`、无 shell/process/http 插件；`fs:allow-read-file`/`allow-write-file` 虽未配 `fs:scope`，但读写的路径全部来自用户通过 `open`/`save` 选择的路径，而 `tauri-plugin-dialog-2.7.2/src/commands.rs:194-212` 在选路径时会调用 `s.allow_file(&path)` 把该路径写入 fs scope —— 即**有效权限 = "用户显式选过的文件"**，是正确且最小的用法。`tests/capabilities.rs:97-121` 还对 `readDir/mkdir/exists/remove/rename/copyFile/stat/lstat/writeBinaryFile/readBinaryFile/readTextFile/writeTextFile` 做了防回归黑名单断言。
- **插件通道的提权风险**：已核对 `frontend/src/plugin-bridge.js:181-203`（`API_METHODS` 固定 8 个映射 + 2 个 `command: null` 本地路由 + 1 个显式 `unsupported`）与 `:1877-1911`（权限校验 → `command` 为空则显式报错 → `invoke(method.command, params)`）→ 插件**无法**构造任意 Tauri 命令名（无 `invoke(nameVariable)` 形式的调用；全仓库唯一的动态 `invoke(method.command, ...)` 的来源是上述固定表），且 `permission` 校验在调用前、`pluginId` 由宿主注入而非插件自报，与 `:185-190` 记录的"后端 PluginRegistry 二次校验"形成双层。
- **变量写命令的原子性**：已核对 `commands/variables.rs` 全文与 `storage_backend.rs:605-653` → `set_character_variable` / `set_campaign_variable` / `add_campaign_variable` / `sync_campaign_variable_schema` / `promote_temporary_instance` 全部经 `mutate_idle_instance` / `mutate_idle_campaign`，在"活动 Turn 屏障 + 读改写 + 写盘"同一原子单元内完成（`variables.rs:35-38` 的注释记录了旧实现的 TOCTOU 已被修掉），且不存在命中即视为成功的假象——`applied.is_none()` 会转成 `not_found`。
- **AppState 锁与重入**：已核对 `lib.rs:704-747` 的状态定义与 `commands/` 内 46 处 `.lock()/.read()/.write()` 调用点 → 未发现"持锁跨 `.await`"的组合：需要跨 await 的场景使用 `tokio::sync::Mutex`（`lib.rs:715` `active_connection_update`、`mvu_webview_runtime.rs:15` 的 pending map，后者在 `commands/mvu.rs:38` 以 `.lock().await` 使用），其余 `std::sync::Mutex` 的守卫作用域均为单语句/短块；`campaign_store.rs:94-102` 等 store 内部各字段独立加锁，不存在嵌套获取顺序不一致导致的 ABBA 风险路径（本域未逐一证明，仅就命令层可见路径判断）。
- **重复 DTO / 重复 helper**：已扫描 `crates/tauri-app/src`（排除测试）的 129 个 `struct/enum` 生产定义 → 仅 `CompressJobStatus` 在 `compress_job_store.rs:20` 与 `sqlite_compress_jobs.rs:19` 各定义一次，属两个后端实现层的同名类型（JSON store vs SQLite repo），非重复 DTO。命令层无重复 DTO。
- **`card_shell_register_doc` 返回令牌的校验**：`frontend/src/utils/shellDocUrl.js:65-67` 用 `TOKEN_RE = /^[a-f0-9]{64}$/` 校验后端返回的 token，`shellDocUrl.js:128-139` 的 URL 构造也只接受合法 token → 前端不会把任意 token 拼进 iframe `src`。此链路未发现问题。

### 5.2 低风险观察（不单列为发现）

1. **`commands/writing.rs:1677` `cancel_writing` 是非 async 命令**，函数体走 `handle.cancel_tx.send(true)`（`:1685` 用 `let _ =` 吞发送结果）——`cancel_tx` 是 oneshot/广播发送，无阻塞 IO，可接受；`let _ =` 在取消路径上是合理的 best-effort。
2. **`commands/mvu.rs:7/14` 两个 `async fn` 体内无 `.await`**（纯日志 no-op），声明 async 只是让它们走线程池；无功能影响。
3. **`create_task` 的 `createdTurn`（`Option<u32>`）前端从不传**（`tauri-api.js:1378` → `commands/memory.rs:269`）。因 `Option` 参数在 Tauri 中可缺省（`tauri-2.11.5/src/ipc/command.rs:134-145` 的 `deserialize_option` → `visit_none()`），不构成契约错误，但"任务创建于第几轮"这一溯源字段实际恒为 `None`。同理 `set_campaign_variable`/`set_character_variable` 的 `turn` 恒缺省为 0（`variables.rs:44/297` 的 `turn.unwrap_or(0)`）。
4. **`log_query` 无 `Result`**（`diagnostics.rs:49-53`），`filter` 的非法值经 `and_then` 静默丢弃（`:54-...`），返回空/全量列表；查询类命令无错误通道属可接受设计，但与 T-07 的 `log_clear` 同源（非法输入不报错），若统一整改宜一并处理。
5. **`storage_health_acknowledge(path: String)` 接受调用方任意路径**（`diagnostics.rs:15-19`），`storage_health::acknowledge` 对未登记路径返回 `false`（无副作用），但 `:17` 会把完整路径写进 `tracing::warn!`。`error.rs:284-289` 的 `sanitize_path_for_ipc` 只约束跨 IPC 的错误字符串，服务端日志保留全路径是既定设计（`error.rs:280-283` 注释），不算泄露。
6. **`card_shell_cache_protocol_response`（`card_shell.rs:168-208`）对缓存资源返回 `Access-Control-Allow-Origin: *`**，并注释说明"URI path 是生成的缓存文件名，绝不接受调用方本地文件路径"。该函数不是 `#[tauri::command]`（自定义协议处理器），且 `read_protocol_resource` 做名字解析，未发现路径穿越问题路径；仅提示该响应头较宽松，值得在安全域交叉复核。

---

## 6 需要 Lead 重点复核的结论

按"如果这条错了，结论会翻转"的优先级排列：

1. **T-01 的差集算术（169 vs 172）** —— 这是本报告最有价值也最可复核的结论。复核方式：改跑一次 `node scripts/architecture/backend-baseline.mjs`（本域禁跑，需 Lead 执行），看 `frontend.uniqueInvokeCount` 是否为 169、`frontend.invokedCommands` 是否**不含** `card_shell_register_doc` / `card_shell_register_module` / `card_shell_unregister_doc`。若基线实现已被修改（例如 `\._invoke\(` 已改），本结论作废。
2. **T-03 的线程模型前提** —— 结论依赖"非 async `#[tauri::command]` 在 IPC handler 上下文内联执行，不卸载线程"。本报告的依据是 `tauri-macros-2.6.3/src/command/wrapper.rs:50/158-160/248-253/429-435`（默认 `ExecutionContext::Blocking` → `body_blocking` 原地调用）＋ `tauri-2.11.5/src/ipc/mod.rs:343-389`（只有 `Async` 才 `async_runtime::spawn`）＋ `tauri-runtime-wry-2.11.4/src/lib.rs:5164-5170`（wry IPC handler 注册）。**注意 `Cargo.lock` 锁定的是哪个 tauri-macros 版本未被本域核实**——registry 里同时存在 2.6.2 与 2.6.3，若实际使用 2.6.2 且语义不同，需要重判。建议 Lead 用 `cargo tree -p tauri-macros` 或 `Select-String Cargo.lock` 核准版本后再定 T-03/T-12 的严重度。
3. **T-02 的实际影响面** —— 结论要求"Card Studio 的 8 处确实在可触发路径上"。`cardstudio_run_review(use_llm=false)`（`:304-310`）与 `cardstudio_import_compiled`（`:814`）是**成功返回**的路径，最值得确认；`cardstudio_run_stage`（`:710/718`）的落盘失败会与 JSON 解析失败混淆，需确认真实用户可见错误。若 Lead 认为 Card Studio 是实验特性，可将 T-02 降为 P2，但"成功≠落盘"的性质不变。
4. **T-06 的可达性判断** —— 我判定"当前不可从 UI 触达"基于两点：壳运行在隔离 origin 且无 Tauri 直调通道；插件通道是固定 allowlist（`plugin-bridge.js:181-203`）。若 Lead 发现有第三条通道（例如卡壳通过 `window.__TAURI_INTERNALS__.invoke` 直调），T-06 应升为 P0/P1（安全）。
5. **T-13 的位置约束缺失** —— 门禁只保护 `lib.rs`（`tauri-command-contract.test.mjs:57-59` 断言 `lib.rs` 内 `#[tauri::command]` 计数为 0），对 `card_studio_api.rs` 的 19 个命令没有任何位置断言。请确认这是有意的架构例外（若是，文档应写明）还是 Gate 1 拆分时的遗漏。
6. **T-10 的 CI 判断** —— 我读的是 `.github/workflows/release.yml`（253 行，仅 1 个 workflow）。若存在被 `.gitignore` 排除或未纳入本次工作树的 CI 配置（例如 Gitea Actions，`RELEASE-STATUS.md:32` 提到"Gitea Actions 已停用"），则"契约测试不在 CI"的结论需要修正。

### 遗留不确定项（本域未能闭环）

- 未运行任何测试/门禁，因此**没有一条结论经过运行时验证**；"运行期会怎样"全部是源码推导。
- `Cargo.lock` 中实际解析到的 `tauri`/`tauri-macros`/`wry` 版本未核实（registry 中存在多个并存版本），影响 T-03/T-12 的框架语义前提。
- `frontend/src/utils/shellDocUrl.js` 的 3 个 invoking 命令「是否真的在生产被调用」未做运行时确认（只确认了静态调用点存在且无其他入口）。
- 插件侧后端二次校验（`commands/plugins.rs` 的 `PluginRegistry` 校验）只读了命令签名与注释，未逐条核对 7 个 `plugin_*` 命令内部的权限校验实现——这属于插件安全域，建议与对应域的审查交叉验证。

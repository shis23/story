# R5 对抗性复验：三条 P0 独立复现 + 两条 P1 抽查

- **任务**：`task-22`（复检 R5：P0 对抗性独立复现 D-01 / S-01 / M-01），owner `review-tauri-api`
- **日期 / 仓库状态**：2026-09-13，`HEAD = ab894c6`（**三条 P0 的修复全部是未提交的工作区改动**，
  因此 `git show HEAD:<path>` 就是可用的"修复前"基线）
- **方法约束（严格遵守）**：仓库**只读**，未修改任何仓库内文件（本报告除外）；
  所有临时工程/探针在 `%TEMP%` 下创建；未运行 `cargo test --workspace`（Lead 独占），只跑包级聚焦测试。
- **核心原则**：**不采信任何一方表述**。三条 P0 的结论全部来自我自己构造的探针/夹具 + 我自己读的依赖源码，
  修复记录只用于"知道要验什么"，不作为证据。

## 0 结论摘要

| # | 原 ID | 命题 | **我的裁决** | 关键依据（本节全部自测/自读） |
| --- | --- | --- | --- | --- |
| P0-1 | D-01 | 卡壳 ES module 扫描在 `from 原作` 等 CJK 邻接处 panic | **已关闭** | 自建 rustc 探针：同一语料 **旧版 12/20 panic、新版 0/20 panic**；仓库回归测试的 4 条 panic 输入逐条复现旧版 panic；旧版还会**连带丢掉后文合法 URL** |
| P0-3 | S-01 | 不完整 legacy 目录 → 孤儿行静默丢弃 + 权威 marker 固化 | **已关闭（原始 P0 路径）**；**残留 1 条变体（新增发现 R5-02，P2）** | 自建 6 种目录布局 + 真 `run_cutover`：`cards.json` 缺失 → readiness 与 cutover **双 Err**，**无 DB 文件、无 marker 文件、`inspect_marker = Absent`**；但 `cards.json` **存在且为 `[]`** 时仍会发布权威 marker（见 R5-02） |
| P0-2 | M-01 | Windows 上子帧可调用任意命令 | **仍开放（仅"收敛"，未关闭）** | 依赖源码逐环节自读：wry 在 Windows 上**无视** `for_main_frame_only`（`wry-0.55.1/src/lib.rs:990`）→ 子帧拿到 IPC 引导脚本；wry 把**发信帧自身 URL** 作为请求 URL（`webview2/mod.rs:896-910`）；StoryForge 的壳源是**已注册自定义协议** `storyforge-shell` → `is_local=true`；本仓库无 `permissions/` 且 `acl-manifests.json` **无 `__app__`** → `has_app_acl_manifest=false` → `webview/mod.rs:1823` 的 ACL 分支**整段跳过**。域6 的守卫测试**通过即告警**，不是阻断 |
| P1 | W-04 | 质量门禁误杀正常中文对白 | **主诉已关闭；同类残留 1 类（新增发现 R5-01）** | 探针直调 `run_quality_gate`：②③④⑤ 旧版 Error → 新版无告警、`blocks_accept=false`；但**引号内**的「我将为你」「我来为你」仍无条件 Error 且 `blocks_accept=true` |
| P1 | T-02 | Card Studio 落盘失败被吞、命令仍返回 Ok | **已关闭** | 旧版 8 处 `let _ = store.update(...)`（其中 **3 处在成功返回路径**）→ 新版 0 处真吞错（唯一匹配是解释性注释）；3 条 T-02 测试 6/6 通过；故障注入前提（rename 到目录必失败）已用独立探针在本机证明 |

**一句话**：两条 P0 真关闭，第三条 P0（子帧 IPC）**仍是开放的**，这不意外——修复记录本来也只声称"收敛"；
两条 P1 抽查一条关闭（T-02）、一条**关闭主诉但留有同类残留**（W-04）。

---

## 1 P0-1（D-01）卡壳 UTF-8 字符边界 panic —— **已关闭**

### 1.1 复现方法（不碰仓库，纯 rustc）

把两个版本的函数体**逐字**放进同一个探针，用同一份语料对比：

| 列 | 来源 |
| --- | --- |
| `old` | `git show HEAD:crates/domain/src/card_shell.rs` → `fn capture_es_module_urls`（HEAD 第 421-450 行） |
| `new` | 工作区 `crates/domain/src/card_shell.rs:455-489` |

探针用 `catch_unwind` + 自定义 panic hook 捕获 panic 消息（不靠进程退出码猜），
文件：`%TEMP%\r5p1\probe.rs`，`rustc -O probe.rs -o probe.exe`（只依赖 std）。

**复现命令**（任何人可照跑）：

```powershell
$work = "$env:TEMP\r5p1"; New-Item -ItemType Directory -Path $work -Force | Out-Null
# probe.rs 见本报告附录 A（两个版本函数体逐字拷贝 + 语料）
rustc -O probe.rs -o probe.exe ; .\probe.exe
```

> 环境提示：本会话的沙箱下 `rustc` 无法在我最初选的字面 `%TEMP%\r5-p0-1` 里建临时目录（os error 5），
> 必须用 `$env:TEMP`（指向 `...\Temp\dsh-*`）作为工作目录。这是**环境限制，不是项目缺陷**。

### 1.2 原始输出（节选，完整 20 例）

```
== 前提验证：to_ascii_lowercase 的字节长度是否不变 ==
  "from 原作" len=11  lower_len=11  equal=true
  "IMPORT 模块 İı" len=18  lower_len=18  equal=true
  "ABCabc 中文🚀" len=17  lower_len=17  equal=true
  "ĞÜŞİÖÇ ğüşiöç" len=24  lower_len=24  equal=true

--- needle 后紧跟 CJK（from）
    input = "from 原作"
    old   = PANIC: start byte index 6 is not a char boundary; it is inside '原' (bytes 5..8) of `from 原作`
    new   = Ok([])
--- needle 后紧跟 CJK（4 字节 emoji）
    input = "from 🚀"
    old   = PANIC: start byte index 6 is not a char boundary; it is inside '🚀' (bytes 5..9)
    new   = Ok([])
--- 中文在前、合法 import 在后
    input = "原文：from 原作\nimport 'https://example.com/ok.js'"
    old   = PANIC: start byte index 15 is not a char boundary ...
    new   = Ok(["https://example.com/ok.js"])
--- 仓库测试#1  "// 移植 from 原作：设定集"          old = PANIC  new = Ok([])
--- 仓库测试#2  "// 参考 import 模块实现"            old = PANIC  new = Ok([])
--- 仓库测试#3  "注释 from 原作\nimport '…js-yaml…'"  old = PANIC  new = Ok([…])
--- 仓库测试#4  "注释 import 模块\nfrom '…a.js'"     old = PANIC  new = Ok([…])
--- 仓库测试#5（正向路径）"说明：从原作提取\nimport '…ok.js'"  old = Ok([…])  new = Ok([…])

== 汇总 ==
cases=20 old_panics=12 new_panics=0 diverged=12
```

### 1.3 判定与两点增量

1. **旧版确实 panic，新版确实不 panic**：20 例里旧版 12 例 panic（覆盖 3 字节 CJK 与 4 字节 emoji、
   `import ` / `from ` / `import(` 三个 needle 全部命中），新版 **0 例 panic**。
   panic 消息与修复注释描述完全一致：`start byte index N is not a char boundary`，根因是
   `search_from = abs + 1` 落在多字节字符内部后，下一轮 `lower[search_from..]` 直接切片。
2. **增量①：旧版不只是"会 panic"，还会丢数据**。`"原文：from 原作\nimport 'https://example.com/ok.js'"`
   在旧版是 **panic**（整个扫描中断），在新版返回 `["https://example.com/ok.js"]`。
   即：一个"中文注释 + 合法 ES import"的真实卡片，旧版连**本来能提取到的模块 URL** 都拿不到。
3. **增量②：修复所依赖的前提我单独验了**。边界推进用 `text.is_char_boundary` 保护对 `lower` 的切片，
   这要求 `to_ascii_lowercase()` 保持字节长度（1:1 映射）。上面 4 组混合语料（含 `İ`、emoji）全部
   `equal=true`，前提成立；否则这个修复本身是错的。

### 1.4 可达性（为什么它是 P0 而不是理论问题）

```
crates/domain/src/card_shell.rs:101  pub fn extract_card_shell_manifest(character: &Character)
  └─ :208  fn extract_from_tavern_helper(...)
       └─ :236  let content = sc.get("content")/("value")   ← 来自导入卡片的 TavernHelper 脚本正文
            └─ :246  capture_es_module_urls(&content)       ← panic 点
调用者：
crates/tauri-app/src/commands/card_shell.rs:36   （Tauri 命令路径）
crates/tauri-app/src/commands/card_shell.rs:91   （Tauri 命令路径）
```

**用户可控的卡片文本 → Tauri 命令同步路径**。补充一条我自己的核查：`tauri-2.11.5/src/**` 下
`catch_unwind` **0 命中**（`Get-ChildItem -Recurse | Select-String 'catch_unwind'` 为空），
因此该 panic 会沿 IPC 调用栈向上 unwind，**命令必然失败**；是否进一步终止进程取决于
wry/事件循环边界，本次**未做进程级验证**（见 §6 诚实边界）。

### 1.5 冻结项

- 仓库回归测试 `card_shell::tests::es_module_scan_handles_multibyte_after_needle`
  （`crates/domain/src/card_shell.rs:744-764`）**我实跑通过**：
  `cargo test -p storyforge-domain es_module_scan_handles_multibyte_after_needle` →
  `1 passed; 0 failed; 387 filtered out`（exit 0）。
  其 4 条 panic 输入与 1 条正向输入都已进入我的探针语料，结论一致。

---

## 2 P0-3（原 S-01）迁移静默丢数据 —— **已关闭（原始路径）+ 残留变体 R5-02**

### 2.1 复现方法（自建夹具 + 真 cutover + 自己查磁盘）

不用任何既有测试夹具，自己写 6 种 legacy 目录布局，然后直接调用 crate 的公开 API：

| 步骤 | API | 我看什么 |
| --- | --- | --- |
| 1 | `readiness::validate_source_manifest(dir)` | 是否 `Err`（fail-closed） |
| 2 | `run_cutover(&CutoverRequest{..})` | 真实切换是否被拒 |
| 3 | **我自己查文件系统** | `storyforge.sqlite3` 是否存在、`storyforge.backend.json` 是否存在 |
| 4 | `inspect_marker(&plan)` + `Database::open` 计数 | marker 是否 `Absent`、库里留下了什么 |

探针：`%TEMP%\r5p3\probe_s01.rs`，**复用仓库已构建的 rlib**（不重编译、不改仓库）：

```powershell
$deps = "<repo>\target\debug\deps"
$rlib = (Get-ChildItem $deps -Filter "libstoryforge_infra_sqlite-*.rlib" |
         Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
rustc -O --edition 2021 probe_s01.rs -L "dependency=$deps" --extern "storyforge_infra_sqlite=$rlib" -o probe_s01.exe
.\probe_s01.exe "$env:TEMP\r5p3\data"
```

### 2.2 原始输出（6 个 case 全量）

| case | 布局 | `validate_source_manifest` | `run_cutover` | DB 文件 | marker 文件 | `inspect_marker` |
| --- | --- | --- | --- | --- | --- | --- |
| ① | `cards.json` **缺失** + campaigns=1 + 会话 1 | **Err** `import source incomplete: cards.json is missing (cards read: 0) but campaigns.json contains 1 campaign(s)` | **Err**（同一条） | **false** | **false** | **Absent** |
| ② | `cards.json` **存在但 `[]`** + campaigns=1 + 会话 1 | **Ok** `cards=0 campaigns=0 skipped_orphan_rows=1 skipped_detail=[("campaigns_no_card",1)]` | **Ok** | **true** | **true** | `SqliteAuthoritative{schema_version:8, manifest_hash:…}` |
| ③ | `campaigns.json` **缺失** + 会话非空 | **Err** `campaigns.json is missing but dependent collections are non-empty (conversations=1); refusing to drop every dependent row as an orphan` | **Err** | **false** | **false** | **Absent** |
| ④ | 完整一致（控制组） | Ok `cards=1 campaigns=1 conversations=1 skipped_orphan_rows=0` | Ok | true | true | `SqliteAuthoritative` |
| ⑤ | 空目录（控制组） | Ok（全 0） | Ok | true | true | `SqliteAuthoritative` |
| ⑥ | `cards.json` 缺失 + campaigns **为 `[]`**（控制组） | Ok（全 0） | Ok | true | true | `SqliteAuthoritative` |

case ② 库内计数：`campaigns=0 … conversations=1 completed_import_runs=1`。

### 2.3 判定

**原始 P0 路径 = 已关闭，且是"双保险 + 零残留"级关闭**：

- ①②③ 三种"源目录不完整"形态在 **readiness（dry-run）与 cutover（真实切换）两层都 fail-closed**；
- ①③ 的**磁盘状态我亲自查过**：DB 文件不存在、marker 文件不存在、`inspect_marker = Absent`
  —— 即"空库 + completed 导入记录 + 权威 marker 永久固化"这条链**真的断了**，
  不是"记录里说断了"；
- ④⑤⑥ 控制组证明守卫**没有误伤**合法数据（完整数据、全新用户、无交叉引用的缺失都照常放行）。

### 2.4 残留变体 → **新增发现 R5-02（P2）**

case ② 是**同一根因（源目录不完整）的不同触发形态**：`cards.json` 不是"缺失"而是**"存在但被清空/截断为 `[]`"**
（拷贝中断写到一半、误删内容、磁盘故障留下空数组），此时：

- 守卫**不触发**（判据刻意只看"文件是否缺失"，见 `readiness.rs:185-192` 的设计说明）；
- 于是该仓库唯一那张卡引用的 Campaign 被当作 `campaigns_no_card` **跳过**，
  `campaigns=0`，**仍然发布 `SqliteAuthoritative` marker + `completed` 导入记录**；
- 库里还留下 `conversations=1`（其 `campaign_id` 指向已被丢弃的 `camp-1`）；
- 可见性：`skipped_orphan_rows=1` / `skipped_detail=[("campaigns_no_card",1)]` 确实进入了报告。

**这一条不是"修复失败"**——修复记录（`02-storage-fixes.md:33-59`）与仓库测试
`empty_cards_json_with_all_campaigns_dangling_is_skipped_with_audit`（`importer_diagnostics.rs:1158-1188`）
都**显式声明**了该语义（"文件存在但为空 = 合法老数据，不 fail-closed，但必须可审计"），
我实跑该测试也是 `ok`。我把它列为发现，是因为它**不满足任务给的验收判据里的后半句**——
"不会再写出权威 marker 吞掉数据"只对**缺失**触发成立，对**空文件**触发仍会写出权威 marker。

**审计链路我实测确认存在**（不是只写在注释里）：

```
crates/tauri-app/src/storage_backend.rs:1942-1965
  if let CutoverOutcome::Completed(report) = &outcome && report.skipped_orphan_rows > 0 {
      tracing::warn!(skipped_orphan_rows=…, skipped_detail=…, "JSON→SQLite 导入跳过了孤儿行…");
      crate::storage_health::record_backend_incident("cutover_skipped_orphans", "…请在清理 JSON 前核对数据");
  }
```

**建议（P2，产品决策类）**：把守卫从"文件缺失"扩一档即可覆盖：
`cards_file_present && cards_total == 0 && campaigns_total > 0` → 走"需用户确认后才发布权威"或直接
fail-closed。注意不能简单 fail-closed：那会误伤"用户真删光了卡但留着 Campaign"的形态
（该形态确实是合法老数据），所以更合适的是**发布前要求显式确认**。

> 探针读数说明：我打印的 `cards=` 计数恒为 `-1`，因为我用了 `SELECT COUNT(*) FROM cards`，
> 而该表并不叫 `cards`（探针里是 `unwrap_or(-1)`）。**这是我探针的产物，不是项目缺陷**，
> 请勿引用该列。

---

## 3 P0-2（原 M-01）子帧 IPC —— **仍开放（部分收敛）**

### 3.1 我自己读出来的完整机制链（全部来自依赖源码与本仓库，不引用任何一方结论）

| # | 环节 | 证据（我读的原文） | 含义 |
| --- | --- | --- | --- |
| 1 | Tauri **要求** IPC 引导脚本只进主帧 | `tauri-2.11.5/src/manager/webview.rs`：`fn main_frame_script(script) -> InitializationScript { for_main_frame_only: true }`；`src/webview/webview.rs:182` 用它推入 `invoke_initialization_script` | Tauri 意图是 main-only，这点没问题 |
| 2 | **wry 在 Windows 上无视该意图** | `wry-0.55.1/src/lib.rs:990`（文档原文）：**"**Windows:** scripts are always added to subframes regardless of the `for_main_frame_only` option."** | 子帧照样拿到全部初始化脚本 |
| 3 | IPC 桥本身也是全帧注入 | `wry-0.55.1/src/webview2/mod.rs:882-887`：用 `add_script_to_execute_on_document_created` 注入 `window.ipc.postMessage = window.chrome.webview.postMessage` | WebView2 的 document-created 脚本对子帧同样生效 |
| 4 | 请求携带**发信帧自身**的 URL | `wry-0.55.1/src/webview2/mod.rs:896-900,910`：`args.Source(&mut url)` → `Request::builder().uri(url)` | 判定"是否本地源"用的是**帧**的 URL |
| 5 | 壳/插件/TH/MVU 文档跑在一个**已注册自定义协议**上 | 注册点 `crates/tauri-app/src/lib.rs:1139` `register_uri_scheme_protocol(shell_doc_protocol::SHELL_DOC_SCHEME, …)`；常量 `shell_doc_protocol.rs:30` `SHELL_DOC_SCHEME = "storyforge-shell"`（缓存协议 `storyforge-cache` 同型，`lib.rs:1131`）；前端确认：`frontend/src/components/CardShellHost.vue:261` "Serve the shell document on the isolated storyforge-shell origin"，`frontend/src/utils/appCsp.js:55-63` 列 `http://storyforge-shell.localhost` / `storyforge-shell://localhost` | Windows 上该帧 URL 形如 `http://storyforge-shell.localhost/<token>` |
| 6 | **该帧被判定为 local** | `tauri-2.11.5/src/webview/mod.rs:1698-1739` `is_local_url`：Windows 分支取 `domain().strip_suffix(".localhost")` 后查 `uri_scheme_protocols`，命中即 local | `is_local = true` |
| 7 | 本仓库**没有** app ACL manifest | `crates/tauri-app/permissions/` **不存在**；且 `crates/tauri-app/gen/schemas/acl-manifests.json`（137KB）实测 **不含 `__app__`**（键只有 core / core:* / dialog / fs 等插件）；`tauri-2.11.5/src/ipc/authority.rs:132-134` `has_app_manifest() == has_app_acl` | `has_app_acl_manifest = false` |
| 8 | **ACL 分支整段跳过** | `tauri-2.11.5/src/webview/mod.rs:1819-1852`：`if (plugin_command.is_some() ‖ has_app_acl_manifest ‖ !is_local) && … && invoke.acl.is_none() { reject; return; }` | 非 plugin 的应用命令 + local 源 + 无 manifest ⇒ 条件为假 ⇒ **不检查、不拒绝** |

**结论**：在 Windows 上，`storyforge-shell` 源里的脚本（卡片壳 / 插件 / TavernHelper / MVU 运行时）
**可以调用 175 个注册命令中的任意一个**，而不仅是桥接所需的那几个。前端不调用 ≠ 不可被调用。

### 3.2 域6 做了什么（为什么是"收敛"而非"关闭"）

`crates/tauri-app/src/shell_doc_protocol.rs` 里有 4 条 M-01 守卫测试，我全部实跑：

```
cargo test -p storyforge --lib shell_doc_protocol::tests -- --nocapture
→ test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 456 filtered out   (exit 0)
```

其中 `acl_manifest_absence_is_a_known_risk`（`:409-444`）**通过的同时打印**：

```
warning: M-01 known risk — no `permissions/` app ACL manifest, so `has_app_acl_manifest == false`
and Tauri skips ACL for every app command issued from the local shell origin (card / plugin / MVU /
TH iframes). Runtime PoC: load a plugin with `permissions: []` and log `typeof
window.__TAURI_INTERNALS__` (or call `await window.__TAURI_INTERNALS__.invoke('list_conversations')`)
inside its iframe. Real fixes: (a) upgrade wry so main-frame-only init scripts are actually
main-frame-only, (b) host untrusted HTML in a separate webview, (c) require a main-frame credential
at the command layer. Do NOT add a manifest without whitelisting every command the app UI calls.
```

即：**风险被记录、被守卫、被打印，但没有任何运行时阻断**。另外三条守卫是
CSP 收紧（`shell_csp_stays_locked_down`）、CSP 保留显式 IPC 来源
（`shell_csp_keeps_tauri_ipc_sources_for_custom_protocol_fetch`）、capability 只绑主窗口
（`capability_grants_only_the_main_window`，我另读了 `capabilities/default.json` 确认
`"windows": ["main"]` 且无 `webviews`/`remote`）。这些**都不改变 §3.1 第 8 步的结论**。

### 3.3 最小运行时 PoC 步骤（照做即可复现；需真实桌面会话）

> 为什么必须运行时验证：本会话无法启动 GUI 应用，`args.Source()` 在子帧下的**实际字符串**
> （`http://storyforge-shell.localhost/<token>` vs `about:srcdoc`）只能由真实 WebView2 给出。
> 按 §3.1 的注册方式与 `is_local_url` 逻辑，我判定它命中 local 分支，但这一步是**推断**，PoC 用来终结它。

**准备**：Windows 桌面、`cargo tauri dev` 或已构建的 release exe、开发者工具可用。

1. 启动应用，进入任意 Campaign 写作界面（或卡片壳页面），让一个壳 iframe 出现。
2. 打开 DevTools（右键 → 检查 / F12）。在 **Elements** 里选中壳 `<iframe>`，
   在 **Console** 顶部的执行上下文下拉里**切到该 iframe 的上下文**（不要停在 top）。
3. **先证"帧 URL 是 local"**（这一步直接决定 ACL 分支是否被跳过）：
   ```js
   location.href      // 期望: http://storyforge-shell.localhost/<token>  ← local ⇒ ACL 被跳过
   location.origin    // 期望: http://storyforge-shell.localhost
   ```
   若这里看到的是 `about:srcdoc` / `blob:` / 远程 `https://`，则 ACL 分支**会被进入**
   （`!is_local` 为真）→ 命令应被拒；此时请把该结果作为"本机未复现"如实记录。
4. **证 IPC 引导脚本进了子帧**：
   ```js
   typeof window.__TAURI_INTERNALS__            // 期望: "object"（Windows 上 wry 无视 main-only）
   typeof window.__TAURI_INTERNALS__.invoke     // 期望: "function"
   ```
5. **决定性调用**（选一个**只读但敏感**的命令，避免破坏数据）：
   ```js
   await window.__TAURI_INTERNALS__.invoke('list_conversations', { campaignId: '<任取一个>' })
   ```
   - 返回数组/对象 → **漏洞确认开放**（子帧拿到了本不该有的能力）；
   - 报 `Command … not allowed by ACL` → 该帧被 ACL 挡下（**debug 构建**给的是
     `resolve_access_message` 的说明文案，**release 构建**给的是
     `Command <cmd> not allowed by ACL`，见 `webview/mod.rs:1828-1850`）。
6. **负对照（强烈建议一并做）**：在主窗口 top 上下文执行同一条 invoke，应**成功**——
   证明差异来自"帧"而不是"命令不可用"。
7. **升级面证明（可选，只读）**：在子帧里调用
   `await window.__TAURI_INTERNALS__.invoke('list_agent_profile_configs')` 或
   任选一条**前端零入口**命令（23 条清单见 `scripts/architecture/backend-baseline.mjs`
   的 `RETAINED_NO_FRONTEND_CALLER`），能成功即证明"无 UI 入口 ≠ 不可调用"。

### 3.4 与域4 既有发现 N-02 的关系

我在域4 修复记录里登记的 **N-02（P2）**指出：23 个"前端不可达但已注册"的命令是额外 IPC 攻击面。
**本次复验把这条从"条件式风险"变成"已定位的可用路径"**：它们的可达性不依赖任何前端代码，
只依赖"子帧 + local 源 + 无 manifest"这三个已经成立的事实。建议 Lead 在最终报告里把
N-02 与 P0-2 写成**同一条链的上游/下游**，而不是两条独立风险。

---

## 4 P1 抽查

### 4.1 W-04 质量门禁误杀正常中文对白 —— **主诉已关闭，同类残留 1 类**

**方法**：`old` 列把 `git show HEAD:crates/app-pipeline/src/quality_gate.rs` 的
`check_meta_description` 判据**逐字复刻**（`const PATTERNS` + `text.contains(pat) → Error`，
共 14 条，含 `让我来` / `好的，我` / `没问题，我` / `我来为你` / `我将为你`）；
`new` 列**直接调用工作区真函数** `storyforge_app_pipeline::quality_gate::run_quality_gate`
（通过 `--extern libstoryforge_app_pipeline-*.rlib` 链接，不重编译）。
判读 `error_count()` / `blocks_accept(false)`。探针：`%TEMP%\r5w04\probe_w04.rs`。

| case | 输入（摘要） | old | new | `blocks_accept` | 判定 |
| --- | --- | --- | --- | --- | --- |
| ① | 真 meta（旧测试样本"好的，我来为你写一个精彩的场景…"） | Error`好的，我` | Error`我来为你` | true | 正确（期望 Error） |
| ② | `「让我来帮你。」…` | **Error**`让我来` | **无告警** | **false** | ✅ 改前必错 → 改后不错 |
| ③ | `「好的，我这就去。」…` | **Error**`好的，我` | **无告警** | **false** | ✅ 改前必错 → 改后不错 |
| ④ | 无引号`让我来` + 后文无线索词 | **Error**`让我来` | **无告警** | **false** | ✅ 改前必错 → 改后不错 |
| ⑤ | `「没问题，我马上到。」…` | **Error**`没问题，我` | **无告警** | **false** | ✅ 改前必错 → 改后不错 |
| ⑥ | **`「我将为你复仇。」她握紧剑柄…`** | Error`我将为你` | **Error`我将为你`** | **true** | ⚠️ **同类误杀残留** |
| ⑦ | **`「我来为你撑伞。」他笑着说…`** | Error`我来为你` | **Error`我来为你`** | **true** | ⚠️ **同类误杀残留** |
| ⑧ | 控制组 真 meta`作为AI…` | Error`作为AI` | Error`作为AI`（Error 总数 2） | true | 正确 |
| ⑨ | 控制组 正常长段落 | 无告警 | 无告警 | false | 正确 |

> 探针把所有"old 与 new 都 Error"的格子都打了 `[!!]` 标记；①⑧ 属于**期望就应是 Error** 的控制组，
> 那是探针输出的**误报标记**，判读时以表格"判定"列为准。

**结论**：W-04 主诉（`让我来` / `好的，我` / `没问题，我` / `我来写` 这类正常对白开场被无条件判 Error）
**已完成"改前必错 → 改后不错"的执行级验证**。但 **⑥⑦ 是新发现的同类残留（R5-01，见 §5）**：
`STRICT_PATTERNS` 里的 `我将为你` / `我来为你` **不带任何对白豁免也没有线索词要求**，
而这两句在中文小说里是**标准台词**（"我将为你复仇"/"我来为你撑伞"），
后果与 W-04 完全同级：Error ⇒ 1× Editor auto-fix + **默认拦截 Accept**。

### 4.2 T-02 Card Studio 落盘错误传播 —— **已关闭**

**"改前必错"（静态事实，逐字取自 `git show HEAD:crates/tauri-app/src/card_studio_api.rs`）**：
修复前有 **8 处** `let _ = store.update(...)`。我**逐处回代码确认了它落在成功路径还是失败路径**
（不采信自己的注释）：

| 旧行号 | 所属命令 | 落点 | 失败后果 |
| --- | --- | --- | --- |
| 308 | `cardstudio_run_review`（纯规则分支） | 紧随 `return Ok(rule_report)` | **静默成功**：`last_stage_output` 未落盘但命令返回 Ok |
| 361 | `cardstudio_run_review`（LLM 分支） | 紧随 `Ok(merged)` | **静默成功**：同上 |
| 814 | `cardstudio_import_compiled` | 紧随 `Ok(ImportCompiledResultDto{..})` | **静默成功**：`imported_character_id`/阶段状态未落盘但命令返回 Ok |
| 486 | `cardstudio_complete_manual_stage` | 紧邻 `return Err(validation)` | 失败状态未落盘（原始错误仍返回） |
| 574 / 579 | `cardstudio_prefill_from_novel` | `map_err` 闭包内 / `return Err` | 同上 |
| 710 / 718 | `cardstudio_run_stage` | `return Err(validation)` | 同上 |

即 **3 处在成功返回路径**（`cardstudio_run_review` ×2 + `cardstudio_import_compiled` ×1，与 `card_studio_api.rs:24-26`
的注释一致 —— 这条注释我也独立核对过，**没有虚报**），5 处在失败路径。成功路径那 3 处才是真正的 P1：
落盘失败时用户看到"成功"，重启后阶段输出 / `last_error` / `imported_character_id` 回退旧值。

**"改后不错"（工作区实测）**：

| 检查 | 结果 |
| --- | --- |
| 残余 `let _ = store.update(...)` | **0 处真吞错**（唯一字符串匹配是 `card_studio_api.rs:23` 的解释性注释，注意别误读为 1 处残留） |
| 成功路径 | 3 处改为 `persist_project(store, project)?`（:334 / :387 / :840），失败 → `TauriCommandError::storage` |
| 失败路径 | 5 处改为 `persist_failure_state(...)`（:512 / :600 / :605 / :736 / :744），落盘失败 `tracing::error!` 留痕且**不覆盖**原始错误 |
| 聚焦测试 | `cargo test -p storyforge --lib card_studio_api` → **6 passed / 0 failed**（含 3 条 T-02 用例：`update_reports_error_when_persist_fails`、`persist_project_propagates_disk_failure_as_command_error`、`persist_project_returns_ok_when_disk_write_succeeds`） |

**故障注入前提我另做了独立验证**（防止"测试其实没真的失败"这种假绿）：
`CardStudioStore::update`（`card_studio_store.rs:53-61`）确实 `self.persist(&projects)?`；
`atomic_write`（`infra-util/src/lib.rs:29-52`）最后一步是 `fs::rename(<path>.tmp, path)`。
我用 6 行探针在本机证明"把目标位置做成目录 → rename 必失败"：

```
target = …\r5-rename-probe\card_projects.json (is_dir=true)
结果: rename 失败 kind=PermissionDenied raw_os_error=Some(5) msg=拒绝访问。(os error 5) —— 注入前提成立
对照: 正常文件目标 rename 成功（前提有效，非环境性失败）
```

---

## 5 本轮新增发现（均**不在**原 P0 范围内，属复验副产品）

| ID | 严重度 | 位置 | 内容 | 建议 |
| --- | --- | --- | --- | --- |
| **R5-01** | **P2**（影响机制与 W-04 同级，仅触发面更窄；是否按 P1 定级交 Lead） | `crates/app-pipeline/src/quality_gate.rs:103-114,122-126` | `STRICT_PATTERNS` 命中即 Error，**且不经过 `inside_dialogue` 与 `META_CUES` 判定**；其中 `我将为你` / `我来为你` 是中文小说的正常台词。实测 `「我将为你复仇。」` / `「我来为你撑伞。」` → Error 且 `blocks_accept=true`（= 1× Editor auto-fix + 默认拦截 Accept，与 W-04 完全同级） | 两条路：**(a)** 把这两个短语降级到 `AMBIGUOUS_PATTERNS`（保留 `作为AI` / `以下是故事` 等真正的自述残留为严格）；**(b)** 给 STRICT 也加 `inside_dialogue` 豁免——但注意 `作为AI` 出现在对白里也可能是真泄漏，故 (a) 更安全。建议先 (a) |
| **R5-02** | **P2**（产品决策类） | `crates/infra-sqlite/src/readiness.rs:193-199` | `cards.json` **存在但为 `[]`** + campaigns 非空 ⇒ 不 fail-closed，Campaign 被 `campaigns_no_card` 跳过，**仍发布 `SqliteAuthoritative` marker + completed 导入记录**（实测 case ②）。审计链路存在（`storage_backend.rs:1942-1965` 的 warn + `cutover_skipped_orphans` 健康事件），故非静默；但"不会再写出权威 marker 吞掉数据"这一判据对**空文件触发**不成立 | 守卫从"文件缺失"扩一档：`cards_file_present && cards_total==0 && campaigns_total>0` → **发布权威前要求显式确认**（不要直接 fail-closed，会误伤"用户真删光了卡"的合法形态） |
| **R5-03** | 关联性（无独立严重度） | `webview/mod.rs:1823` + `RETAINED_NO_FRONTEND_CALLER` | 域4 的 **N-02** 从"条件式风险"升级为"已定位可用路径"：23 条零前端入口命令在 Windows 子帧下可被直接 `invoke` | 最终报告把 N-02 与 P0-2 串成同一条链；P0-2 关闭前这 23 条的"声明"不构成安全豁免 |

---

## 6 诚实边界（我**没有**验证到的部分）

1. **P0-2 没有做机器级运行时 PoC**：本会话无法启动 GUI 应用（无桌面会话）。
   §3.1 的机制链是**依赖源码级**证据（每条都可复核），§3.3 的 PoC 步骤**是待执行的验证方法，不是我已得到的结果**。
   特别是 `args.Source()` 在子帧下的实际 URL 字符串需要真实 WebView2 才能确认——
   我按 `is_local_url` 的逻辑判它命中 local 分支，这一步属于**高置信推断**而非实测。
   若 PoC 显示帧 URL 是 `about:srcdoc`/`blob:`，则本机该路径被 `!is_local` 分支挡下，
   **请以 PoC 结果覆盖我的推断**。
2. **P0-1 的进程级后果未测**：我只证明"panic 发生"+"tauri src 里无 `catch_unwind`"，
   因此"命令必然失败"成立；"是否终止整个应用"取决于 wry/事件循环边界，未验证。
3. **P0-3 用的是 crate 公开 API**（不可避免——行为就在 crate 里），但**夹具与磁盘断言全部是我自建的**；
   我没有跑真实应用启动路径（`storage_backend.rs` 的 startup 分支），
   §2.4 的"审计链路存在"是**读码 + 条件判断复核**，未做端到端触发。
4. **版本冻结**：结论只对 `tauri 2.11.5` / `wry 0.55.1` 成立，均在依赖源码中逐行核对过。
5. 本轮**未新增/未修改任何仓库代码与测试**（写作用域只允许本报告），
   因此所有"改后不错"结论均来自**既有测试实跑**或**外部探针调用真函数**，没有为我临时改代码。

---

## 7 给 Lead 的裁决建议

1. **P0-1 → 记为"已关闭"**，证据强度：函数级执行复现（新旧同语料对比）+ 真实测试通过 + 可达性链路。
2. **P0-3 → 记为"已关闭（原始路径）"**，并把 **R5-02** 单列为 P2 待用户拍板；
   不要把 R5-02 写成"P0 未修"——它是**修复记录已披露**的设计边界，只是验收判据的后半句不成立。
3. **P0-2 → 必须记为"仍开放"**，写法建议：
   > M-01 经 R5 独立复验**仍开放**：Windows 上 `storyforge-shell` 源（卡片壳/插件/TH/MVU）
   > 可直接调用任意已注册命令；域6 的 4 条守卫只做到"风险可见 + CSP 收紧 + capability 不扩散"，
   > **无运行时阻断**。最小 PoC 步骤见 R5 报告 §3.3。
   >
   > 真正的修法（按代价排序）：(a) 给应用命令加**主帧凭证**（命令层校验，代价可控、不依赖上游）；
   > (b) 把不可信 HTML 放进**独立 webview**（能力面干净，但改动大）；
   > (c) 升级/替换 wry 让 main-frame-only 真正生效（受上游限制）。
   > 注意 `acl_manifest_absence_is_a_known_risk` 的排期警告：**不要**只加 `permissions/` 而不逐条
   > 白名单前端所需命令，否则主窗口命令会全被 ACL 拒掉（假安全感 → 直接不可用）。
4. **两条 P1**：T-02 记为"已关闭（含独立注入前提验证）"；W-04 记为"**主诉已关闭 + 新增残留 R5-01**"。
5. 若需把 P0-2 的 PoC 落地，建议交给有桌面会话的执行方，**只做 §3.3 第 3-6 步**（只读命令 + 负对照），
   不要用写命令做 PoC。

---

## 附录 A：P0-1 探针的可复现配方（不贴全文，只给"唯一差异 + 语料"）

两个函数体逐字来自：`old` = `git show HEAD:crates/domain/src/card_shell.rs`（HEAD 第 421-450 行），
`new` = 工作区 `crates/domain/src/card_shell.rs:455-489`。**两者唯一差异**就是循环尾部这 4 行：

```diff
             search_from = abs + 1;
+            // Advance by UTF-8 char boundaries — a bare `abs + 1` panics when the
+            // byte after the needle is inside a multi-byte char (e.g. `from 原作`).
+            while search_from < text.len() && !text.is_char_boundary(search_from) {
+                search_from += 1;
+            }
             if search_from >= text.len() {
                 break;
             }
```

探针外壳（`%TEMP%\r5p1\probe.rs`）的实质代码：

```rust
mod old { /* 旧版函数体逐字 */ }
mod new { /* 新版函数体逐字 */ }

static PANIC_MSG: Mutex<Option<String>> = Mutex::new(None);
// set_hook 记录 payload + location；catch_unwind(AssertUnwindSafe(|| f(text))) 分类 Ok/Panic
// 逐例打印 old / new 的 Outcome，最后统计 old_panics / new_panics / diverged
```

**语料（20 条，可直接照抄）**：`from 原作`、`import 模块`、`import(模块)`、`from 🚀`、
`from 原文说明`、`原文：from 原作\nimport 'https://example.com/ok.js'`、
`from 原作 from 'https://example.com/late.js'`、`import 模块 from 原作 import(设置)`，
4 条控制组（合法 `from` / 合法 `import` / 合法 `import(` / `from ` 结尾无后继字符 / 纯 ASCII），
1 条回归位点（`中文 import 'https://example.com/d.js'`），
以及仓库回归测试的 5 条原始输入（§1.2 已列出）。
另有 4 条 `to_ascii_lowercase` 字节长度不变的**前提验证**语料（含 `İ`、emoji）。

其余三个探针（`probe_s01.rs` / `probe_w04.rs` / `probe_rename.rs`）的编译命令已分别写在 §2.1 / §4.1 / §4.2，
它们都**复用仓库 `target/debug/deps` 下已构建的 rlib**（`--extern …`），因此不需要重编译工作区、也不改仓库。
`%TEMP%` 下的探针文件会随环境清理消失，但按上述配方可从 git + 工作区完整重建。

# 域 6 评审报告：Meta Agent / MVU / 插件宿主 / 卡壳 / ST 导入导出

- **仓库**：`C:\Users\Predator\ZCodeProject\storyforge` · **HEAD**：`ab894c6` · **评审日**：2026-09-13
- **评审者**：`review-meta-plugin`（task-6）
- **约束**：全程只读；仅创建本报告文件；未运行任何 cargo/npm/build/test 命令。所有结论以 `file:line` + 代码片段为证据，文档不作为证据。
- **证据来源标注**：`自核` = 本人逐行读证；`交叉` = 子代理提供且本人抽查一致（抽查点已注明）；`子代理` = 由子代理读证、本人未复读（需 Lead 或修复者复核）。

---

## 1. 范围与覆盖率

### 1.1 负责的子系统

| 子系统 | 代码范围 |
|---|---|
| Meta Agent | `crates/app-meta/src/**`（typed_patch / meta_conversation / health_check / explain / mvu_apply / mvu_import / prompts）、`crates/tauri-app/src/commands/meta{,_typed}.rs`、`sqlite_meta_repo.rs`、`meta_backend.rs` |
| MVU | 同上 MVU 部分 + `crates/tauri-app/src/{commands/mvu.rs,sqlite_mvu_repo.rs,mvu_webview_runtime.rs}`、`crates/domain/src/variables.rs`、前端 `mvu*` |
| 插件宿主 | `crates/infra-plugin-host/**`、`crates/tauri-app/src/commands/plugins.rs`、`frontend/src/plugin-bridge.js`、`PluginHost.vue`、`usePluginBridge.js`、`promptHooks*.js` |
| 卡壳（card shell） | `crates/tauri-app/src/{card_shell.rs,card_shell_cache.rs,shell_doc_protocol.rs}`、`frontend/src/components/CardShellHost.vue`、`TavernHelperRuntime.vue`、`utils/cardShell*.js`、`utils/shellVariable*.js`、`components-v2/st/ShellAwareContent.vue` |
| ST 导入/导出 | `crates/infra-import/**`、`crates/domain/src/{character.rs,world_info.rs,card_studio.rs}`、`crates/tauri-app/src/commands/{import_export.rs,bundle_runtime.rs}` |
| 依赖源码（安全链证据） | `…\cargo\registry\…\tauri-2.11.5`、`wry-0.55.1`、`url-2.5.8`、`tauri-plugin-dialog-2.7.2` |

### 1.2 实际打开并阅读的文件（覆盖率）

**Rust**
- 全文精读（自核）：`commands/meta_typed.rs`(1-440)、`commands/meta.rs`(关键段)、`app-meta/src/typed_patch.rs`(55-109,700-770)、`card_shell_cache.rs`(全文 + 测试段)、`shell_doc_protocol.rs`(1-290)、`commands/card_shell.rs`(100-209)、`lib.rs`(104-148,1120-1140,1294-1487)、`build.rs`、`capabilities/default.json`、`tauri.conf.json`、`gen/schemas/acl-manifests.json`(键集)
- 交叉复核（子代理读证 + 本人抽查）：`domain/src/world_info.rs`(270-300)、`app-pipeline/src/lib.rs`(1528-1545)、`plugin_prompt_hook_result` 链、Meta accept 链、`domain/src/variables.rs`、`sqlite_meta_repo.rs`、`sqlite_mvu_repo.rs`、`infra-import/src/{lib,png,compat}.rs`、`mvu_analyzer.rs`、`mvu_import.rs`、`import_export.rs`、`bundle_runtime.rs`、`backend_workflows.rs`、`runtime_support.rs`、`production_postprocess.rs`
- 子代理覆盖并由其声明已核对：`infra-plugin-host/{lib,audit,compat_matrix,mvu_runtime}.rs`、`app-meta` 全部、`infra-import` 全部、`commands/{plugins,writing,conversations,diagnostics,variables}.rs`

**前端**
- 自核：`components/PluginHost.vue`(100-184)、`components/CardShellHost.vue`(关键段)、`components/TavernHelperRuntime.vue`(关键段)、`components/MvuJsRuntime.vue`(关键段)、`utils/{cardShellCsp.js,shellVariableProposals.js,shellVariableOutbox.js,cardShellDisplay.js,promptHooks.js}`、`components-v2/st/ShellAwareContent.vue`、`AppV2.vue`(276-364)、`mvu-runtime-bridge.js`
- 子代理覆盖：`plugin-bridge.js`、`PluginHost.vue` 全文、`usePluginBridge.js`、`promptHookAudit.js`、`pluginPersistence.js`、`composables/{useWriting,usePipeline,useMvuStatusPanel}.js`、`components-v2/config/PluginPanel.vue`、`utils/{mvuKey,mvuInteractions,mvuStatTree,mvuStatusBarModel,campaignMvuStatusBar}.js`、`stores/plugin.js`、`components-v2/meta/*.vue`、`components-v2/st/*`、`node_modules/dompurify/src/tags.ts`

**依赖源码**（安全链）：`tauri-2.11.5/src/{webview/mod.rs,manager/webview.rs,ipc/{protocol,authority}.rs,scripts/ipc-protocol.js}`、`wry-0.55.1/src/{lib.rs,webview2/mod.rs}`、`url-2.5.8/src/parser.rs`

### 1.3 未覆盖 / 覆盖薄弱

- 未运行应用：所有需要 WebView2 运行时语义的结论（唯一一条见 M-01 第 6 环）标为「疑似」，已给出 30 秒验证步骤。
- 未逐行读：`docs/{ROADMAP,ST-EVENTS-COVERAGE}.md` 全文（仅按断言点抽查）、`frontend/tests/plugin-bridge.test.mjs`(2302 行)逐条、`TavernHelperRuntime.vue` 1-635、`infra-import/src/compat.rs` 1300-1660。
- 未覆盖域：`01-domain-infra.md`、`03-writing-pipeline.md`、`07-goals-and-claims.md` 负责的范围（仅在其与本域交叠处引用）。

---

## 2. 结论摘要

**计数**：P0 = **1**；P1 = **7**；P2 = **18**；P3 = **6**（分组条目）。合计 32 条，其中 `自核`/`交叉` 31 条、纯 `子代理` 0 条（P0/P1 全部经本人复核或抽查）。

**三句判断**：

1. **安全边界在 Windows 上不是「可绕过」而是「不存在」**：卡壳/插件/MVU/TH 四类壳文档在 V5 改造后统一落到 `storyforge-shell` 自定义协议源，而 wry 的 WebView2 后端丢弃 `for_main_frame_only`、Tauri 又把该源判定为 local 且本应用没有 app ACL manifest，导致这些按设计运行第三方 JS 的 iframe 可以直接 `invoke` 任意应用命令（M-01）——插件权限桥、卡壳白名单、变量提案门这三道门在同一个洞面前同时失效。
2. **「文档里的不变式」与「代码里的不变式」存在系统性落差**：Meta 的 revision 陈旧性校验对 LLM 提案恒被跳过（M-03/M-09）、「先 preview 才 accept」只是 UI 约定（M-27）、JSON 后端 accept 的原子性只写在注释里（M-10）、插件兼容矩阵把 `Network`/`CallLlm` 标为 Implemented 而无强制点（M-31）、审计链在 Rust 侧零调用（M-23）——这些不是实现 bug，而是**契约声明高于实现**。
3. **数据面本身比边界面干净得多**：PNG tEXt/CRC/UTF-8、未知字段 `flatten extra` 保留、Bundle 外键与版本门、SQLite 单事务回滚、MVU apply 幂等与「不覆盖已有值」、`value_expr` 无任意 JS 执行面，均已逐条核实无问题；但**导出 wire 契约**（`position` 数字 vs ST spec 字符串，M-05）与**保真闸门**（真实卡测试全 `#[ignore]`、fixture 全桩、分诊器把漂移记为 Intentional，M-06）让「ST 往返保真」这一对外声明缺乏可证伪性。

---

## 3. 发现清单

### M-01 [P0] Windows 上壳/插件/MVU/TH iframe 仍持有 Tauri IPC，可直连任意应用命令（权限桥整体绕过）

- **类别**：安全边界绕过（跨子系统：卡壳 / 插件 / MVU / TH）
- **位置**：`frontend/src/utils/cardShellCsp.js:48-58`、`crates/tauri-app/src/shell_doc_protocol.rs:52-77`、`crates/tauri-app/src/lib.rs:1134-1136`、`crates/tauri-app/build.rs:18`、`crates/tauri-app/gen/schemas/acl-manifests.json`（无 `__app__`）；wry `src/webview2/mod.rs:492-495,882-887,896-910`、wry `src/lib.rs:990`；tauri `src/manager/webview.rs:159-182`、`scripts/ipc-protocol.js:59-68,84`、`src/webview/mod.rs:1716-1737,1742-1762,1819-1826`、`src/ipc/authority.rs:132-134`
- **证据（5 环，全部为本机依赖源码）**：

```
// 环2  wry-0.55.1/src/webview2/mod.rs:492-495  —— 初始化脚本无 for_main_frame_only 分支
    for init_script in attributes.initialization_scripts {
      Self::add_script_to_execute_on_document_created(&webview, init_script.script)?;
    }
// 同文件 :896-910 —— IPC 请求 URI 取自「发送方文档 URL」
    let url = { let mut url = PWSTR::null(); args.Source(&mut url)?; take_pwstr(url) };
    ipc_handler(Request::builder().uri(url).body(js).unwrap());
// 环3  tauri-2.11.5/scripts/ipc-protocol.js:59-68,84 —— CSP 拦掉自定义协议后回退到原生 postMessage
    console.warn('IPC custom protocol failed, Tauri will now use the postMessage interface instead', e)
    ... window.ipc.postMessage(data)
// 环5  tauri-2.11.5/src/webview/mod.rs:1819-1826 —— 只在「plugin 命令 / 有 app ACL / 非 local」时查 ACL
    if (plugin_command.is_some() || has_app_acl_manifest || !is_local) && ... && invoke.acl.is_none()
```

- **链路**：① 卡壳/插件/MVU/TH 文档全部经 `registerShellDoc()` 注册到 `storyforge-shell` 源（`PluginHost.vue:160-166`、`CardShellHost.vue`、`MvuJsRuntime.vue`、`TavernHelperRuntime.vue`，`自核`）；② 该源的 CSP（`shell_doc_protocol.rs:52-77`）`connect-src` **不含** `ipc:`/`http://ipc.localhost`，故 `ipc-protocol.js` 的 fetch 版 IPC 必被拒；③ 回退路径用原生 `window.ipc.postMessage`，**不受 CSP 约束**；④ Tauri 的 `__TAURI_INTERNALS__` + invoke key 以 `for_main_frame_only: true` 注册（`manager/webview.rs:159-182`），但 wry WebView2 后端把该标志整个丢弃（环 2，`wry lib.rs:990` 亦明文记录 "scripts are always added to subframes regardless of the `for_main_frame_only` option"），`window.ipc` 同样无条件下发；⑤ `is_local_url`（`webview/mod.rs:1716-1737`）在 Windows 上把「scheme == tauri 协议 scheme && domain 以 `.localhost` 结尾且是已注册协议」判为 local → `storyforge-shell.localhost` 命中；⑥ 本应用**没有** app ACL manifest（`permissions/` 目录不存在、`acl-manifests.json` 无 `__app__` 键、`build.rs:18` 仅 `tauri_build::build()`，三项 `自核`）⇒ 应用命令完全跳过 ACL。
- **影响**：一个 `permissions: []` 的第三方插件（或一张被自动挂载的恶意卡、一段 MVU fallback 片段）在其 iframe 内即可调用 `generate_handler!` 注册的全部应用命令（`lib.rs:1294-1487`，126 个）：`list_conversations`/`get_conversation`（全量记忆与正文，**无 ReadMemory 也能读**）、`delete_conversation`/`delete_campaign`、`install_plugin`/`uninstall_plugin`、`start_writing`（消耗 API 额度）、`export_*`、`meta_accept_typed_patch`、`plugin_set_variable` 等。设计文档与代码注释反复声明的「唯一通道是经权限门控的桥消息」（`PluginHost.vue:127-133`、`installed_plugin.entry_html` 注释）在该路径上不成立。
- **触发路径**：安装 `{"id":"pwn","permissions":[],"entry_html":"<script>…</script>"}`（`entry_html` 的 `<script>` 是刻意放行的插件本体）→ iframe 内 `await window.__TAURI_INTERNALS__.invoke('get_conversation', { id })`。
- **回归性质（重要）**：该暴露源于 V5「壳文档迁到独立源」改造。若文档仍走 `srcdoc:`/`blob:`（`PluginHost.vue:167-172` 的非 Tauri 回退分支），其 URL 既非 tauri 协议也非已注册协议 ⇒ `is_local = false` ⇒ ACL 生效、应用命令被拒。**CSP 隔离的收益换来了 IPC 暴露的代价。**
- **建议**（按性价比）：① 立即补 app ACL manifest（`crates/tauri-app/permissions/` 只列前端真正需要的命令），使 `has_app_acl_manifest = true`，非白名单命令对主窗与子帧一并被拒；② 初始化脚本加帧门禁 `if (window.top !== window) { delete window.__TAURI_INTERNALS__; delete window.ipc }`；③ 结构化方案：壳文档改由独立 window label + 专属 capability 承载；④ 防回归断言：壳帧内 `typeof window.__TAURI_INTERNALS__ === 'undefined'`。
- **置信度**：**高**（5 环中 4 环 `自核` 于依赖源码；第 6 环「WebView2 是否真把 document-created 脚本投递进 `sandbox="allow-scripts"` 子帧」属运行时语义，标 **疑似**）。两名评审者（本人 + 插件子代理）独立收敛到同一链路与同一结论。
- **Lead 需裁决**：这是本域唯一的 P0，且是**跨域候选总分最高项之一**（它同时废掉域 6 的三道边界与域 4/5 可能依赖的 ACL）。

---

### M-02 [P1] 卡壳网络白名单 + IP 字面量拦截可被 `\@` 反斜杠绕过（SSRF / 白名单逃逸）

- **类别**：输入校验分歧（SSRF）
- **位置**：`crates/tauri-app/src/card_shell_cache.rs:429-440`（`host_of`）、`:129-139`（`is_url_allowed`）、`:144-150`（`validate_fetch_url`）、`:321-352`（`send_checked_request`）；`url-2.5.8/src/parser.rs:899`（`parse_userinfo` 遇反斜杠 break）
- **证据（自核）**：

```
// card_shell_cache.rs:429-440 —— 手写字符串解析充当「主机名权威」
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split('/').next()?;
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    Some(host_port.split(':').next().unwrap_or(host_port).to_ascii_lowercase())
}
// :144-150 —— IP 字面量拦截同样建立在 host_of 之上
if host.parse::<std::net::IpAddr>().is_ok() { return Err(...) }
// :330 —— 真正发出的请求
let mut request = client.get(&current);
```

- **分歧点**：WHATWG/`url` crate 对 special scheme 把 `\` 当作 authority 终止符（`url-2.5.8/src/parser.rs:40` 注释 "The backslash (\) character is treated as a path separator in special URLs"；`parse_userinfo` 在 `is_special()` 时遇 `'\\'` 直接 break，:899），因此
  `https://evil.example\@cdn.jsdelivr.net/x`
  在 `url` crate 眼中 host = `evil.example`（`\@…` 整体成为 path），而 `host_of` 眼中 host = `cdn.jsdelivr.net`（allowlist 命中）。`validate_fetch_url` 与 `is_url_allowed` 都过、`send_checked_request` 用 `reqwest`（内部走 `url`）发出 → **实际连到 evil.example**。同一手法也绕过 IP 字面量检查（`https://127.0.0.1\@cdn.jsdelivr.net/` 在 `host_of` 下是 `cdn.jsdelivr.net`）。
- **影响**：卡壳网络请求可打到任意非白名单主机（含内网/回环），并把响应体交给卡/插件 JS（`fetch_text`），既是 SSRF 又是白名单逃逸；`is_safe_cache_resource_name`、`fetch_by_ranges` 等以同一 URL 为输入，缓存键随之可被污染。
- **测试为何没发现**：`host_parser`（:730-736）只覆盖正常 URL 与 `not-a-url`；`rejects_ip_literal_redirect_targets…`（:716-727）只测 `http://127.0.0.1/internal`。无「解析器与真实 client 语义一致性」测试。
- **建议**：`host_of` 改为 `url::Url::parse` 后取 `host_str()`，并在 `validate_fetch_url` 里拒绝任何 host ≠ `host_of` 结果的输入（或直接删掉手写解析）；重定向每一跳已校验（`send_checked_request`），需同步换成同源解析。
- **置信度**：**高**（`自核`；攻击串 `'https://evil.example\\@cdn.jsdelivr.net/'`，单字面反斜杠）。

---

### M-03 [P1] Typed patch 未与 Campaign 绑定：accept 只信前端传入的 `campaign_id` → 可把 A 战役的提案写进 B 战役

- **类别**：授权/作用域
- **位置**：`crates/app-meta/src/typed_patch.rs:71-86`、`crates/tauri-app/src/commands/meta_typed.rs:279-292,345-380`、`:189-201`、`crates/app-meta/src/meta_conversation.rs:788-801`、`crates/tauri-app/src/commands/meta.rs:361-369`
- **证据（自核）**：

```
// typed_patch.rs:72-86 —— 结构体无 campaign_id
pub struct TypedPatch {
    pub id: String, pub description: String, pub source_issue_category: String,
    pub affected_id: Option<String>, pub actions: Vec<TypedPatchAction>,
    pub diff: Vec<FieldDiff>, pub created_at: chrono::DateTime<chrono::Utc>,
    pub status: TypedPatchStatus,
    #[serde(default)] pub campaign_revision: Option<u64>,   // 只有 revision，没有 campaign 身份
}
// meta_typed.rs:365-380 —— patch 取自全局 state.typed_patches，只按 id 找
let p = typed.iter().find(|p| p.id == patch_id)... ;
// meta_typed.rs:189-201 —— 列表无战役过滤
.filter(|p| p.status == TypedPatchStatus::Pending)
```

- **影响**：`UpdateCampaignVariable` 是唯一无 target id 的 action，`validate_typed_patch_targets` 只判「campaign 存在」（`typed_patch.rs:707-711`），因此 A 战役里由 Meta 提议的「改 weather」可以在 B 战役的 UI 上预览通过（`stale:false`）并被接受，写入 B 的变量。全仓 `typed_patches` 引用无任何「切换战役即清空/失效」逻辑。
- **触发路径**：战役 A 提议 patch → 切到战役 B → `HealthCheckPanel` 用 `props.activeCampaign.id` 预览 → preview 通过 → 点「接受」写入 B。M-09 使 revision 兜底同时失效。
- **建议**：`TypedPatch` 增 `campaign_id`（propose 时盖章，`serde(default)` 兼容内存旧 patch）；accept 校验 `patch.campaign_id == campaign_id == active_campaign`；列表按 active campaign 过滤；切换战役时把非 active 的 Pending 标 Stale。
- **置信度**：**高**（`自核`：结构体、查找、列表、校验四处已读证）。
- **测试缺口**：`tests/sqlite_meta_lifecycle.rs:229-233` 的 "foreign campaign 必须被拒绝" 用的是带 target id 的 action，覆盖不到本场景（假安全）。

---

### M-04 [P1] MVU JS fallback 的变量回写绕过 M-6 保留命名空间守卫，且一律落 campaign 作用域

- **类别**：安全边界/命名空间
- **位置**：`frontend/src/components/MvuJsRuntime.vue:447-452`、`frontend/src/utils/shellVariableOutbox.js:60-68`、`crates/app-pipeline/src/lib.rs:1534-1541`
- **证据（`自核` 三处）**：

```
// MvuJsRuntime.vue:447-452 —— 卡产出的键原样回传后端，无过滤
    invoke('mvu_execute_result', {
      requestId: d.request_id, variableUpdates: d.variable_updates || {},
      sideEffects: d.side_effects || [], error: d.error || null,
    })
// shellVariableOutbox.js:60-68 —— M-6 守卫只存在于这一条前端通道
  // M-6：拒绝 __storyforge* 内部命名空间。dispatchMvuInteraction 走「点击即确认」…
  if (isReservedNamespace(k)) { ... }
// app-pipeline/src/lib.rs:1534-1541 —— 后端照单全收，instance_id=None（campaign 级）
    for (key, value) in exec_result.variable_updates {
        pp.variable_updates.push(VariableUpdate { instance_id: None, key, value });
```

- **影响**：`variable_updates` 由卡派生的 JS 片段（`MvuTranslation.fallback_fragments[].js_snippet`）产生，键既不查 `__storyforge*` 保留位也不过 `normalize_mvu_key`。卡写 `_.set('__storyforge_card_shell_variables', …)` 即可覆盖卡壳变量桶（`cardShellVariableStore.js:2`）导致卡壳状态丢失；写任意 campaign 键可污染其它子系统消费的状态。**全仓 `*.rs` 中 `"__storyforge"` 零命中**（`自核` PowerShell 计数 = 0），即后端无任何镜像守卫。
- **缓解事实**：ROADMAP:139 已把「JS 产出统一写 campaign 级变量」记为**已知限制**，故「作用域」部分是设计取舍；**缺命名空间过滤**才是需要修的破口（它使前端 M-6 的设计意图在后端不成立）。
- **建议**：命名空间/键校验下沉到 Rust 写入边界（`build_variable_mutations` 或 `VariableUpdate` 入口），JS 键同样过 `normalize_mvu_key` + 保留位拒绝；`MvuJsRuntime` 转发前加一层镜像过滤。
- **置信度**：**高**（路径 `自核`）；触发需卡翻译含该片段，**中**（子代理评估一致）。

---

### M-05 [P1] 世界书条目 `position` 导出为数字，违反 ST V2/V3 的字符串契约

- **类别**：导出契约/互操作
- **位置**：`crates/domain/src/world_info.rs:286`；对照 `crates/domain/src/character.rs:135`、`crates/domain/src/card_studio.rs:1110`
- **证据（`交叉`：本人复读 `world_info.rs:270-293` 确认第 286 行）**：

```
// world_info.rs:286（导出侧）
            position: Some(Value::Number(self.position.into())),
// character.rs:135（导入侧自述——字符串才是「新版」）
    /// ST 新版用字符串（"before_char" 等），旧版用数字，都要兼容
// card_studio.rs:1110（同一仓库另一条导出链写的是 v3 字符串口径）
    let st_card = StCharacterCard { spec: Some("chara_card_v3".into()), spec_version: Some("3.0".into()),
```

- **影响**：StoryForge 导出的每张 PNG/JSON 卡，`character_book.entries[].position` 都是 `0`/`1`，而 ST 官方 spec 类型是 `'before_char' | 'after_char'`（SillyTavern `src/types/spec-v2.d.ts`）。严格校验的第三方卡工具会拒绝或忽略该字段，`before_char`/`after_char` 语义随之丢失。
- **触发条件**：任何 `export_st_card_png` / `cardstudio_export_png` / `export_campaign_st_cards`。
- **建议**：导出按其 value 语义写回字符串标签（v2/v3 口径），数字形态只作为**导入**兼容分支保留；`position_as_i32`（M-26）同步补告警。
- **置信度**：**高**（代码事实）；第三方工具实际行为 **中**（未实测）。

---

### M-06 [P1] 保真闸门失灵：真实卡测试全部 `#[ignore]`、fixture 全是手工桩、分诊器把 wire 漂移判为 Intentional

- **类别**：测试可信度/门禁
- **位置**：`crates/infra-import/src/compat.rs:690,843,828`、`crates/infra-import/src/lib.rs:539`、`crates/tauri-app/src/lib_tests_import_export.rs:108,1639`、`crates/infra-import/fixtures/*`、`scripts/run-real-card-smoke.ps1:64-66`
- **证据（`交叉`；路径与默认 fixture 缺失已由子代理实测、本人复核引用行）**：

```
// compat.rs:690 —— position 变化被硬编码为「有意归一化」
    // Position string → number is intentional normalization on export via to_st_entry.
        severity: CompatSeverity::Intentional,
// compat.rs:828 —— compare_world_books 的字段清单里没有 extra
                if le.keys != re.keys || ... || canonicalize(&le.extensions) != canonicalize(&re.extensions)
// lib.rs:539 —— 真实卡测试默认路径在仓库根，实测不存在
    #[ignore = "requires a local real ST card fixture; run scripts/run-real-card-smoke.ps1"]
```

- **影响**：`cargo test --workspace` 完全不覆盖真实卡；6 个 fixture 均为 135 B~60 KB 手工桩（`st_v3_large_worldbook.json` 无 `insertion_order`/`case_sensitive`/`match_whole_words`/`group` 等字段）；Leg 2 只做「域对象→PNG→域对象」往返，**从不比对导出后的 wire JSON 与源 JSON**。因此 M-05 这类 wire 层漂移在 CI 中结构性不可见，而分诊器还把它主动标成 Intentional —— 闸门报告「Preserved」不构成保真证据（这正是本文档要求「文档不是证据」的同一条原则在测试上的体现）。
- **建议**：① Leg 2 增加「导出 wire JSON vs 源卡 JSON」的字段级比对；② position/order 等已知改写改为显式 `Normalized` 且带 before/after，禁止泛化 Intentional 兜底；③ 至少一个真实卡 fixture 入库（或 CI 参数化下载）并去掉 `#[ignore]`；④ `run-real-card-smoke.ps1` 的硬编码根路径改为 `data/local/` 回退。
- **置信度**：**高**（fixture 体量、`#[ignore]` 计数、分诊器分支均已核实）。

---

### M-07 [P1] TavernHelperRuntime 的 postMessage 桥不校验 `event.source`（跨 frame 未授权消息）

- **类别**：跨 frame 信任
- **位置**：`frontend/src/components/TavernHelperRuntime.vue:680-683`、`:873`；对照 `frontend/src/mvu-runtime-bridge.js:1-4`、`CardShellHost.vue:1019-1036`
- **证据（`自核`）**：

```
// TavernHelperRuntime.vue:680-683 —— 只认标记字段，不认来源
async function onBridgeMessage(ev) {
  const d = ev.data
  if (!d || !d.__sf_th_bridge) return
  // blob: iframe is cross-origin — do not require contentWindow identity.   ← 注释已过时（现走壳源）
// mvu-runtime-bridge.js:1-4（同仓正确基线）
  if (!event || !runtimeWindow || event.source !== runtimeWindow) return null
```

- **影响**：监听器挂在 `window`，任何能向主窗 postMessage 的 frame（含加载远程卡 HTML 的 CardShellHost）都可伪造 `__sf_th_bridge` 消息，被宿主当作 TH 桥请求：`register_module`（把任意源码注册成壳模块）、`prepare_remote_script`、`report`（伪造脚本状态）、`var_write`（进提案队列，仍需用户点应用）、`fetch_text`（受 M-02 影响的 allowlist）。注释里「blob: iframe is cross-origin」在 V5 之后已不成立（壳文档统一走 `registerShellDoc`），属于**注释已过期 + 校验缺失**的组合。
- **建议**：与 MVU runtime 一致按 `event.source` 做归属校验（壳内嵌套 TH 场景可参考 CardShellHost 的 parent 链判定）。
- **置信度**：**高**（`自核`；攻击串 `parent.postMessage({__sf_th_bridge:true,type:'report',payload:{index:0,state:'done'}},'*')`）。两名评审者独立发现同一项。

---

### M-08 [P1] `plugin_prompt_hook_result` 无 plugin 绑定/权限校验：改写 LLM messages 的唯一门禁在前端 JS

- **类别**：权限门禁位置错误
- **位置**：`crates/tauri-app/src/commands/plugins.rs:243-254`、`crates/tauri-app/src/commands/writing.rs:577-595`、`frontend/src/utils/promptHooks.js:387-402`
- **证据（`交叉`：本人抽查 `plugins.rs` 与 `writing.rs` 引用行一致）**：

```
// commands/plugins.rs:243-254 —— 只有 request_id + messages，无 plugin_id/ensure_permission
pub(crate) async fn plugin_prompt_hook_result(request_id: String, messages: Option<Vec<ChatMessage>>, error: Option<String>, ...)
    if !resolve_prompt_hook_pending(&state.prompt_hook_pending, &request_id, messages, error) {
        tracing::warn!(...); }     // 未知 request_id 也返回 Ok
// commands/writing.rs:577-595 —— 后端无条件用回传值替换提示词
            Ok(Ok(reply)) => { ... Ok(reply.messages.unwrap_or(original_messages)) }
```

- **影响**：`ModifyPrompt` 权限仅在前端 `canModifyPrompt(plugin)` 里检查；后端对「谁改的、是否有权改」不做校验。同文件其它命令（`plugin_get_conversation:260-270` 等）都有 `ensure_permission`，此处是唯一缺口。
- **可利用性**：request_id 由 `Id::new()` 生成且**不广播给插件**（`plugin-bridge.js:366-368` 显式剔除），故单独利用需主世界 JS，或经由 **M-01**（一旦 P0 成立即完全可达）。标 **疑似·中**。
- **建议**：命令增 `plugin_id` 参数 + `ensure_permission(ModifyPrompt)`；request_id 与签发绑定（一次性、限本 turn）。
- **置信度**：代码事实 **高**；可利用性 **中**。

---

### M-09 [P2] `campaign_revision` 陈旧性校验对 LLM 提案恒被跳过

- **类别**：不变式未强制
- **位置**：`crates/app-meta/src/typed_patch.rs:755-765`、`crates/tauri-app/src/commands/meta_typed.rs:382-392`；对照 `:116-118`
- **证据（`自核`）**：`build_patch_from_action` 唯一构造点写死 `campaign_revision: None`（:764）；只有 health 提案路径盖章 `patch.campaign_revision = Some(snapshot.campaign.revision)`（:118）。accept 的 `if let Some(proposed_revision) = patch.campaign_revision && …` 因此对 agent 提案整体跳过，`expected_revision` 传到 SQLite UoW 也是 `None`。
- **影响**：turn 3 的提案可在 turn 50 落盘并覆盖新状态；`typed_patch.rs:81-85` 注释声明的 Gate 4 不变式在生产代码里不成立。这是 M-03 得以成立的前提。
- **建议**：在 `propose_campaign_patch` handler 里由 `ctx.campaign.revision` 盖章；`campaign_revision` 收紧为必填 + 迁移期 `serde(default)`。
- **置信度**：**高**（`自核`）。

---

### M-10 [P2] JSON 后端 accept 无回滚能力（多 action 半提交），文档却宣称「nothing is partially applied」

- **类别**：事务/回滚
- **位置**：`crates/tauri-app/src/commands/meta_typed.rs:312-341`（`:319-335` 循环）、`crates/tauri-app/src/backend_workflows.rs:1349-1352`（注释）、`campaign_store.rs:961-972,1176-1187`
- **证据（`自核` 循环体）**：`with_campaign_lock(|| for (index, action) in actions.iter().enumerate() { apply_typed_action(store, campaign_id, action)?; })` —— `with_campaign_lock` 是**互斥锁不是事务**，每个 action 各自 `persist()`（4 个独立 JSON 文件）。
- **可达性**：当前所有 builder（`typed_patch.rs:760`、`build_patch_for_issue`）都产**单 action** patch，故今天打不出中途失败；但测试已手工构造 2-action patch（`lib_tests_meta.rs:644/891`），说明多 action 是预期形态。错误文案「第 2 个 action 失败」会让用户以为整体未生效。
- **建议**：JSON 路径改为「候选快照上全部预演 → 一次性提交」，或明确文档写「SQLite 原子，JSON 逐 action 无回滚」。
- **置信度**：代码事实 **高**；后果 **条件性**。

---

### M-11 [P2] 卡缺失时 JSON 健康检查降级为空 definitions → 所有实例被判孤立并生成破坏性修复

- **类别**：fail-open 降级
- **位置**：`crates/tauri-app/src/commands/meta_typed.rs:20-29`、`crates/app-meta/src/health_check.rs:57-77`、`meta_typed.rs:602-603`；对照 `crates/tauri-app/src/meta_backend.rs:16-34`
- **证据（`交叉`）**：

```
// meta_typed.rs:23-26 —— 卡查不到就当空 definitions
let definitions = store.get_card(&campaign.card_id).map(|c| c.card.character_definitions).unwrap_or_default();
// meta_backend.rs:19-22（SQLite 路径的既定不变式）
/// fail closed instead of being treated as an empty card/definition set.
    .ok_or_else(|| format!("Campaign card does not exist: {}", campaign.card_id))?;
```

- **影响**：definitions 为空 ⇒ 每个 `definition_id: Some(_)` 的实例被报 `orphan_instance`(Error) ⇒ 生成 `RepointInstanceDefinition{new_definition_id: None}` ⇒ 用户接受后 `instance.is_temporary = true`，角色定义/persona/schema 全部脱离，且纯函数预演与落盘一致（diff 看起来"正常"）。
- **触发条件**：存档有 Campaign 但 card 行缺失（部分恢复、导入中断、手工改数据）——数据完整性边界，非远程攻击。**疑似**。
- **建议**：JSON 快照/健康检查与 `meta_backend.rs` 对齐 fail closed。
- **置信度**：代码事实 **高**；触发场景 **疑似**。

---

### M-12 [P2] Meta patch 的变量写入无 key 白名单，可覆盖 `__storyforge*` 内部命名空间

- **类别**：命名空间/输入校验
- **位置**：`crates/app-meta/src/typed_patch.rs:706-753`、`crates/tauri-app/src/commands/meta_typed.rs:610-637`、`crates/domain/src/campaign.rs:223`；对照 `frontend/src/utils/shellVariableOutbox.js:15-17`
- **证据（`交叉`；与 M-04 同一命名空间的另一条路径）**：propose 侧只校验「campaign 存在」，key 原样进 diff；落盘侧 `campaign.set_variable(key, value, 0)` 无过滤；工具 schema 只要求 `{"kind": string}`。
- **影响**：prompt injection 可让 agent 提议 `UpdateCampaignVariable{key:"__storyforge_card_shell_variables"}`，仅靠用户对技术键名的辨识把关；实例级还可写入 definition schema 之外的键（绕过 `SyncInstanceVariables` 的 schema 校验）。
- **建议**：在 preview/accept 共用的 `validate_patch_preconditions` 里统一拒绝保留前缀；实例键限定 schema（或需显式"允许新键"标记）。
- **置信度**：**高**（无校验与前端有镜像防护两条均已读证）。

---

### M-13 [P2] legacy `meta_accept_patch` 对越界/缺失 target 静默 no-op，却仍标 `applied = true`（假成功）

- **类别**：错误处理/假成功
- **位置**：`crates/app-meta/src/lib.rs:319-361`、`crates/tauri-app/src/commands/meta.rs:57-99`
- **证据（`交叉`）**：`execute_action` 的 `if let Some(TargetRef::Index(idx)) = target_ref && let Some(entry) = entries.get_mut(idx) … { insert }` **无 else 分支**；`Delete` 越界、`character` 分支（`character_fields: None` 写死）同样静默。调用方随后把 patch 标 `applied` 并持久化，前端显示"采纳成功"而数据未变，且 `applied` 会被 `PatchStore::pending()` 过滤，用户无法重试。
- **建议**：`execute_action` 在索引不存在/上下文缺失时返回 `ExecutionFailed`；或比对 working copy 是否真的变化。
- **置信度**：**高**。

---

### M-14 [P2] JSON 后端 accept 的 Turn 屏障在锁外（检查与写盘之间 TOCTOU）

- **类别**：并发/竞态
- **位置**：`crates/tauri-app/src/commands/meta_typed.rs:289-292` vs `:319-335`；对照 `crates/tauri-app/src/sqlite_meta_repo.rs:69-88`
- **证据**：`reject_if_active_turn` + 快照加载在 `with_campaign_lock` **之前**；SQLite 路径已把同一检查搬进事务，并在注释里写明"杜绝『先检查、后开事务』之间的竞态窗口"。同源问题：`AddKnowledge` 的 `character_id` 存在性只在事务外快照校验（`sqlite_meta_repo.rs:244-262` 不复查实例）。
- **影响**：JSON 后端下，检查通过后、写入前若新一轮 turn 落库，Meta patch 会写进活动 Turn 期间（预演快照亦可能过期）。
- **置信度**：结构事实 **高**；实际发生 **中**（桌面单窗，窗口窄）。

---

### M-15 [P2] MVU 运行期写边界不做键归一 → 同一变量两种记法并存

- **类别**：数据一致性
- **位置**：`crates/tauri-app/src/commands/variables.rs:285,293-299`、`crates/domain/src/campaign.rs:234-239,368-375`、`production_postprocess.rs:1277-1301`、`runtime_support.rs:1820-1838`；对照 `variables.rs:178-184`（管理面反而归一）
- **证据（`交叉`）**：`set_campaign_variable` / `set_character_variable` 直接 `camp.set_variable(&key, …)`，零归一零校验；`Campaign::set_variable` 精确匹配即新增。而 `mvu_import` 解析层、`meta_typed.rs:943`、`sqlite_mvu_repo.rs:111` 都调用了 `normalize_mvu_key`。
- **影响**：`stat_data.hp` 与 `hp` 可并存；前端 `findMvuVariable` 精确优先（`mvuKey.js:56-63`），查询键本身是旧记法时命中旧条目→ UI/卡壳读到错值。
- **建议**：在 `Campaign::set_variable`/`CharacterInstance::set_variable`（或命令入口 / `Mutation::SetVariable` 应用点）统一归一，并回查旧键做迁移。
- **置信度**：**高**。

---

### M-16 [P2] MVU preview 未归一而 apply 归一 → 预览与落盘不一致（含按钮误置灰）

- **类别**：预览/落盘分歧
- **位置**：`crates/tauri-app/src/commands/meta_typed.rs:941-952`、`crates/tauri-app/src/backend_workflows.rs:1416-1424`、`frontend/src/components-v2/meta/MvuAnalyzer.vue:360`
- **证据（`交叉`）**：apply 侧先 `normalize_schema_keys` 再 `compute_apply_preview`；preview 侧传入**未归一**的 `mvu.translation.variable_schema`。前端用 `:disabled="!p.has_changes …"` 决定按钮可用性。
- **影响**：旧记法翻译键被预览报成「新增」而实际是覆盖；反向（def 为旧记法）时预览判 unchanged → 按钮置灰，绕过 UI 调用 apply 则 `merge_schema` 按字面 key 去重后**两个键并存**（同变量双记法的真实来源之一）。
- **置信度**：**高**。

---

### M-17 [P2] 分析器「24K 大额度」的判据是 comment 含 `initvar`，其它变量条仍旧 4K 截断

- **类别**：预算/覆盖率
- **位置**：`crates/app-meta/src/prompts/mvu_analyzer.rs:250-259`、`:236-239`（注释所述现象）、`:459-471`（截断实现）
- **证据（`交叉`）**：`let per_entry_cap = if c_lower.contains("initvar") { 24_000 } else { 4_000 };` —— 注释写成 `[变量初始化]`/`变量树`/`状态栏数据` 且承载变量树的条目仍按 4K 截断，`truncate_for_prompt` 保留头 80% + 尾 20%，中段子树再次被吃掉（与 2026-07-27 修复的是同一类缺口）。
- **建议**：按条目内容形态（YAML/JSON 树启发式）而非注释字符串决定额度；或对全部变量类条目统一大额度。
- **置信度**：分支 **高**；真实卡命中比例 **中**。

---

### M-18 [P2] MVU 交互按钮作用域推断错误：`instance:<id>:` 前缀当路由 + 多实例时回退 campaign

- **类别**：作用域越界
- **位置**：`frontend/src/utils/shellVariableOutbox.js:81-94`、`frontend/src/composables/useMvuStatusPanel.js:104-120`
- **证据（`交叉`）**：键前缀 `instance:`/`inst:` 被拆成「目标实例 + 写键」；`useMvuStatusPanel` 在 `singleInstanceId == null`（实例数 0 或 ≥2）时把卡产物键**原样**下传。后端只保证「同 campaign」（`sqlite_runtime.rs:600-609`、`campaign_store.rs:1011-1016`），不保证「卡绑定实例」。
- **影响**：① 卡产物键 `instance:<他实例id>:hp` 被解释成写另一个实例（需知悉 UUID，难度中）；② 多实例时卡按钮语义（"攻击 hp-10"）落到 campaign 变量，成为整局可见状态，且被所有实例面板以 campaign 打底混入（`campaignMvuStatusBar.js:35-44`）。
- **建议**：scope/target 改为显式参数，键不再承担路由；按钮绑定 UI 上所属 `section.instanceId`。
- **置信度**：代码路径 **高**；可利用性 **中**。

---

### M-19 [P2] 内联壳信任粒度是「消息」而非「文档」：一条 trigger 命中即放行该消息内全部内联 script

- **类别**：信任分级/内容注入
- **位置**：`frontend/src/components-v2/st/ShellAwareContent.vue:60-66,90+`、`frontend/src/utils/cardShellDisplay.js:257`、`AppV2.vue:276-305`
- **证据（`自核`）**：`inlineDocsTrusted = matchesAnyInlineShellTrigger(props.sourceContent || props.content, inlineShellTriggers)` 是对**整条消息 sourceContent** 求值一次，随后按该布尔值放行该消息内所有内联壳文档；未命中的走确认卡。`sourceContent` 是原始模型文本（`MessageItem.vue:52,168,196` → `currentVariant.content`）。
- **影响**：只要消息里任意一段命中白名单卡的 `find_regex`（例如模型复述了卡的 trigger 词），该消息内**其余**内联 script 文档也一并自动挂载（在壳源 + `unsafe-eval` 下执行）。属"信任锚过于粗糙"：H3/H4 的意图是"命中 manifest 的文档才可信"，实现是"消息里出现过 trigger 就整条可信"。
- **建议**：改为逐文档匹配——每个内联文档各自与 manifest 的 find_regex 比对，命中才自动挂载；否则一律确认卡或降级为静态渲染。
- **置信度**：**中**（结构 `自核`；是否值得利用取决于触发词是否易被模型复述）。

---

### M-20 [P2] 插件禁用/卸载后 iframe、hook 与权限不即时失效

- **类别**：状态失效
- **位置**：`frontend/src/components-v2/config/PluginPanel.vue:71-86`、`frontend/src/AppV2.vue:990-993,171-183`、`frontend/src/composables/usePluginBridge.js:46-51`
- **证据（`交叉`）**：面板只调 `setPluginEnabled`，宿主列表只在面板 **close** 时刷新（`@close="…; loadSidebarPlugins()"`）；`hookPlugins` 是 `getLivePluginPermissions` 的唯一数据源，故"吊销"分支读的是陈旧权限。
- **影响**：面板保持打开期间，被禁用/卸载的插件 iframe 继续运行、事件继续广播、prompt hook 继续生效（结合 M-01/M-08 更严重）；面板关闭时组件卸载路径是干净的（`PluginHost.vue:317-329` 解绑 + dispose + releaseShellDoc）。
- **置信度**：**高**。

---

### M-21 [P2] prompt hook 返回无结构校验（可清空/替换 system 提示词），且只有每插件 5s 超时、无整链预算

- **类别**：契约/可用性（DoS）
- **位置**：`frontend/src/utils/promptHooks.js:429-447,499-501`、`frontend/src/composables/usePluginBridge.js:258-287,300-310,39`、`frontend/src/plugin-bridge.js:16`
- **证据（`交叉`）**：`resolveHookedMessages` 只检查 `Array.isArray`，插件可返回 `[]`（清空）或 `[{role:'system',content:'…'}]`（替换系统指令）；预算只限总量 256 KB 不限差量。超时是 per-plugin 5 s，串行遍历两个事件 × 全部插件 ⇒ 最坏 5s×N×2 全部落在写作前的 `await runPromptHookEvents(intent)` 上；`setPromptHookTimeoutMs(null)` 可彻底关闭超时（当前仅导出，无生产调用点）。
- **建议**：返回后做角色白名单 + 必须保留原 system 前缀 + 只允许对指定 message 做 diff；给整条链加墙钟总预算。
- **置信度**：**高**。

---

### M-22 [P2] `ReadMemory` 门控可被事件的 `full_text` 字段绕过

- **类别**：权限门控旁路
- **位置**：`frontend/src/plugin-bridge.js:237-270,296-321,390-397`、`crates/tauri-app/src/commands/writing.rs:394-405`
- **证据（`交叉`）**：`SENSITIVE_EVENT_FIELDS` 含 `content/text/messages/...`，但 `PipelineEvent::SubagentDone` 的 payload 用 `full_text`；归一化后 `fulltext` 不在表内 ⇒ 无 `ReadMemory` 的插件订阅 `*`（订阅不需权限）即可拿到各 subagent 的完整生成正文。
- **建议**：键级/值级双重策略，把 `full_text/fullText/warnings/reason` 加入敏感表或改为白名单透出。
- **置信度**：**高**（字段名与消毒逻辑已读证）。

---

### M-23 [P2] 审计链在 Rust 侧是死代码 + 前端审计不落盘 + 变量写入无审计

- **类别**：安全控制名不副实
- **位置**：`crates/infra-plugin-host/src/audit.rs`（`redacted`/`chain_audit_records`/`verify_audit_record_chain`/`ensure_live_permission` 全仓仅再导出与单测命中）、`frontend/src/composables/usePluginBridge.js:144-165`、`crates/app-logging/src/lib.rs:275-285`、`frontend/src/utils/promptHooks.js:228-234,448-458`、`commands/plugins.rs:212-242`
- **证据（`交叉`）**：`let should_persist = entry.level == LogLevel::Error || entry.kind == LogKind::LlmCall;` ⇒ 审计以 `info/warn` 记入内存环（上限 100），重启即丢；`emitAudit` 在 hook 返回**之后**写且吞异常 ⇒ 审计失败不影响插件改动生效；`plugin_set_variable` 全程无审计记录。
- **建议**：要么接入真实链路，要么删掉误导性死代码；prompt hook 与变量写入审计提升到可落盘级别，并明确「审计不构成门禁」。
- **置信度**：**高**。

---

### M-24 [P2] 插件帧自我导航后宿主仍信任该窗口

- **类别**：信任锚
- **位置**：`frontend/src/components/PluginHost.vue:254-256,184-189,117,224,232`
- **证据（`交叉`）**：`isTrustedPluginSource(event){ return !!iframeRef.value?.contentWindow && event.source === iframeRef.value.contentWindow }` —— 信任锚是 browsing context（WindowProxy），`sandbox` 不阻止帧自导航（`location.href=…`），导航后 WindowProxy 身份不变，新文档仍通过 source 校验；宿主继续以 `targetOrigin='*'` 推送事件与 hook 请求（含 ReadMemory 级数据）。
- **缓解现状**：新文档仍为 opaque origin，拿不到宿主 DOM/localStorage，但**继承插件全部权限**并可自行实现 postMessage 协议（嵌套子 iframe 的 `===` 拒绝是干净的）。
- **建议**：`@load` 后重置 handler 并要求首帧 nonce 握手；或监听 `contentWindow.location` 变化。
- **置信度**：**高**（代码）；利用性 **疑似**。

---

### M-25 [P2] 前端把整张卡序列化成 JSON 数字数组过 IPC，大小守卫在反序列化之后才生效

- **类别**：内存/工程
- **位置**：`frontend/src/tauri-api.js:14`、`crates/tauri-app/src/commands/characters.rs:160`、`crates/infra-import/src/lib.rs:50`
- **证据（`交叉`）**：`invoke('import_character', { data: Array.from(data) })` ⇒ `[137,80,…]` 约 4 字节/字节；`check_import_size(data.len(), MAX_IMPORT_SIZE=100 MiB)` 在 `Vec<u8>` 反序列化之后执行。仓库自带 `data/local/test-card.png`（6.4 MB）即 ~25 MB 文本/IPC 缓冲，100 MB 上限卡约 400 MB。
- **建议**：改用二进制通道（Tauri v2 支持 `tauri::ipc::Response`/`Vec<u8>` 的高效路径）或先在前端校验大小。
- **置信度**：**高**；机型 OOM 阈值 **中**。

---

### M-26 [P2] `position_as_i32` 的 `_ => 0` 兜底把未知/内部语义静默折叠为 Before Char

- **类别**：导入容错过度
- **位置**：`crates/domain/src/character.rs:153`
- **证据（`交叉`）**：未知字符串、以及 ST 内部数字语义（2=ANTop/3=ANBottom/4=atDepth）全部落到 `0`，无 warning。与 M-05 叠加即为「导出丢语义 + 导入错位」的双向漂移。
- **建议**：未知值记 warning 并保留原值/原文，不做静默折叠。
- **置信度**：**高**。

---

### M-27 [P3] Meta 杂项

| # | 位置 | 事实 | 影响 |
|---|---|---|---|
| a | `sqlite_meta_repo.rs:226-231,402-407` | accept 后写入 `campaign.revision` 的仍是旧值，无自增 | revision 戳体系在 Meta 路径单向失效；同批 patch 互不 stale（`domain/src/campaign.rs:38` 契约称「一次 MetaCommit bump 一次」） |
| b | `meta_conversation.rs:205-207,300-305,801`；`commands/meta.rs:341-350` | 失败路径 `return Err` 不做 drain | 本轮已提议的 patch 永不可见、后续 drain 按下标跳过 → 会话内无界增长 |
| c | `frontend/src/utils/metaPanelFlow.js:16-24`；`HealthCheckPanel.vue:269-275` | `catch { patch._stale = false }` | preview 失败被当成「不过期」，接受按钮保持可点；服务端 accept 也不要求先 preview（`meta_typed.rs:345-434`）⇒ 文档「patch 先 preview 不直接写核心数据」只是 UI 约定 |
| d | `meta_typed.rs:836,866`；`frontend/src/utils/campaignDisplay.js:29-34` | `routing: format!("{:?}", …)`（Rust Debug 字符串）而前端按 `{kind}` 对象解析 | `routingText()` 恒返回「混合（）」；同族 `complexity: Null` |
| e | `commands/meta.rs:325`；`meta_typed.rs:185,520-536`；`sqlite_meta_repo.rs:112-118` | 四处静默吞错/降级 | 排障无痕 |

### M-28 [P3] MVU 杂项

| # | 位置 | 事实 | 影响 |
|---|---|---|---|
| a | `shellVariableOutbox.js:83-90`；`mvu_import.rs:570-572` | `instance:` 拆分后不复核 writeKey 非空 | 空键写入实例变量（campaign 作用域因 `!k` 被拒，仅实例有洞） |
| b | `mvuInteractions.js:68,130-133` | 算术守卫 ASCII-only | `hp ＋ 10`/`hp×2`/`攻击力／2` 被判为字符串字面量写入数值变量，违反「绝不落成字符串」不变量 |
| c | `mvuStatTree.js:21-35` | 叶子写入可覆盖子树；`blocked` 分支不可达（非 record 时已被赋 `{}`） | 顺序相关（先叶子后标量时整棵子树被覆盖）；死代码 |
| d | `variables.rs:237,283-307` | `extract_mvu_schema_from_extensions` 命中即返回、只做顶层扁平解析、不归一 | 嵌套 initvar 变 `主角 = null` 之类垃圾字段进 schema 与提示词；LLM 失败时作为 fallback schema 回填 |

### M-29 [P3] 前端死代码（对照 CLAUDE.md 的"保留导出"声明逐项区分）

- **无任何调用方**：`classifyPromptHookFailurePolicy`（`frontend/src/utils/promptHooks.js:307`，仅定义处 1 处命中；且 `stage`/`operationType` 参数未被使用，`cancelled` 也不在 `PROMPT_HOOK_FAIL_OPEN_STATUSES` 里）——即**函数里表达的策略表未被任何代码使用**，真实策略散落在 `emitPromptHookEventAndWaitForPlugins` 内联分支；`partitionShellMountsByTrust`（`cardShellDisplay.js:286`，全前端 0 调用、0 测试）——**H3 信任分区的纯函数实现是死代码**，真实分级在 `ShellAwareContent.vue` 里另行实现（两条实现存在漂移风险）。
- **已声明保留**：`extractShellMountsFromDisplay`/`extractInlineShellDocsFromDisplay`（各 2 处命中：定义 + 内部互调），CLAUDE.md 明写"保留导出但组件已不用" ⇒ 不算漂移，仅记录。
- `自核`（grep 计数见 1.2）。

### M-30 [P3] 文档漂移

| 位置 | 文档说法 | 代码事实 |
|---|---|---|
| `CLAUDE.md`（card-shell 清尾 L6） | "`card_shell_clear_cache` 命令 + wrapper（UI 入口未接）" | `frontend/src/components-v2/shell/InspectorDrawer.vue:18,45` 已调用 `cardShellClearCache()` ⇒ 已接（`自核`） |
| `docs/PLAN-ST-IMPORT-EXPORT.md`/ROADMAP:123 | 只讲 JSON bundle `format_version` | `commands/import_export.rs:22` `BUNDLE_FORMAT_VERSION = 2`，导入接受 `0..=3`（:631,:1024），`bundle_runtime.rs:20` 有 `>= 3` 分支 ⇒ 实际存在 v3 语义但常量仍是 2（`自核`） |
| `docs/RELEASE-STATUS.md:60` | 明确声明"不等于密码学审计或插件沙箱攻击面评估" | 该免责声明成立；但同文件 `:29`「第三方插件两条通道闭合」在 M-01 成立前提下**字面为真、边界为假**（通道闭合 ≠ 无可绕过路径）⇒ 建议 Lead 在总报告中统一口径 |

### M-31 [P3] 插件宿主低危项

`MSG_MOUNT` 无权限门控且用 DOMPurify **默认**配置插入宿主 DOM（默认允许 `<style>` ⇒ 可注入全局 CSS 做界面伪装/隐藏，无脚本执行与外传通道）；`storage.set/get` 无权限无限额（可撑爆宿主 localStorage 配额）；`chat.save` 无权限门控（当前 adapter 为 degraded no-op，注入真实 adapter 后即成为无 ReadMemory 的写通道）；`plugin_get_variable` 忽略 `_key` 返回实例全部变量；安装无签名/无"安装即授权"确认（权限仅展示徽章）；`compat_matrix.rs:97-112` 把 `Network`/`Notifications`/`CallLlm` 标为 `Implemented` 而无强制点（声明高于实现）；Rust 审计哈希用裸 `|`/`,` 拼接未转义字段（字段边界歧义，死代码）；`commands/diagnostics.rs:307-312` 用字节索引切 `&message[..4096]`，多字节字符跨界可 panic（审计 payload 键集由插件可控可撑大触发）。（`交叉`）

### M-32 [P3] 导入导出低危项

`UnsupportedFormat` 错误变体全仓无构造点（死变体）；`domain/src/character.rs:313` `raw_card_json` 反序列化失败仅 `warn` 后回退空卡，仍产出"正常"PNG（丢全部 ST 扩展）；`domain/src/world_info.rs:288-291` 同时写 `order` 与 `extra.insertion_order`（互异时 ST 读后者、StoryForge 读前者）；`infra-import/src/png.rs:234` `make_st_card` 硬编码 `spec: "chara_card_v2"` 而 `spec_version` 可为 `"3.0"`（组合非法，测试 `png.rs:465` 已固化该行为）。（`交叉`）

---

## 4. 目标完成度核对表

| 声明（出处） | 代码事实 | 判定 |
|---|---|---|
| ROADMAP Phase 3：Meta 体检 4 类 issue（`health_check.rs` + `meta_health_check`） | 4 条规则齐备、severity 与前端排序一致；但 JSON 路径卡缺失时降级（M-11） | **基本达成**，边界 fail-open |
| ROADMAP Phase 3：Meta patch 类型化（preview/accept/dismiss 闭环） | 命令齐备、状态机自洽、preview 只做只读克隆预演、无自动落盘（子代理穷举 apply 调用点后确认） | **达成**；但无 campaign 绑定（M-03）、revision 校验恒跳过（M-09）、「先 preview」非强制（M-27c） |
| ROADMAP:84「Meta patch 不直接越权改数据（propose → preview → accept 才写盘）」 | 「写盘必先 accept」为真；**「不越权」为假**：跨战役写（M-03）+ 任意 key（M-12） | **部分达成**（措辞过强） |
| ROADMAP Phase 3：explain 解释本轮生成 | 读真实 provenance（`conv_store→node.active()→variant.provenance`），LLM 工具路径 `without_reasoning()` 脱敏 | **达成**（无编造） |
| ROADMAP Phase 5 / PLAN-PLUGIN-MVU 阶段 2：MVU 分析→schema apply（"前端展示 diff，不直接应用"） | `analyze` 落盘的是分析产物（非 patch）；apply 需按钮确认 | **达成**；preview 与 apply 归一不一致（M-16） |
| PLAN-PLUGIN-MVU 阶段 6：插件权限分级、「不能直接写 store，只能 propose patch」、「没有安装即全权限」 | 桥的 `API_METHODS` 权限表完备且 fail-closed、Rust 侧 5 个 RPC 均有 `ensure_permission`、插件 id 冒充被封死 | **达成于桥层**；但 **M-01 使整层可被绕过**，M-08 使 prompt 通道无后端校验 |
| PLAN-PLUGIN-MVU 禁止项「禁止插件直接拿 CampaignStore / 禁止绕过 Meta patch preview 修改变量」 | 直接拿 store 不可（Rust 侧无命令）；「绕过 preview」在 UI 约定下不可、在命令层可行（M-27c） | **达成**（命令层无强制） |
| ROADMAP Phase 5：ST V2/V3 导入保真 + PNG tEXt + 共享 lorebook + JSON bundle | 核心字段保真、未知字段 `flatten` 保留、PNG 编解码干净；`position` wire 契约破（M-05）、闸门失真（M-06） | **部分达成**（对外互操作性未闭合） |
| ROADMAP Phase 5：JS Fallback 接入写作流程 | `execute_fragment` 已接线，`variable_updates` 统一写 campaign 级（ROADMAP:139 记为已知限制） | **达成**；缺命名空间过滤（M-04） |
| RELEASE-STATUS:29「第三方插件两条通道闭合」 | 通道本身可用；M-01 证明边界可绕过 | **字面达成、边界未达成** |
| RELEASE-STATUS:60「不等于插件沙箱攻击面评估」 | 免责成立 | **达成**（诚实） |

---

## 5. 未发现问题（已核对范围）与低风险观察

### 5.1 明确未发现问题

1. **卡壳 CSP 内容与 HTTP 头一致性 / fail-closed**：`cardShellCsp.js:26-36` 的 `sanitizeHosts` 只放行 `^[a-z0-9][a-z0-9.-]*$` 并强制 `https://`；白名单获取失败 fail closed 到 `data:/blob:/storyforge-cache`（`CardShellHost.vue` wrap 路径）。范围：`cardShellCsp.js`(全文)。
2. **壳文档协议硬化**：仅 GET/HEAD（`shell_doc_protocol.rs:261-266`）、64 位小写 hex token（:223-227）、文档一次性消费（:270-277）、128 条/16 MB 上限（:79-80）、响应不设 ACAO（:284-289，单测 :354-359）。范围：`shell_doc_protocol.rs`(1-290)。
3. **`var_write` 提案门真实有效（M4）**：`var_write` → `upsertShellVariableProposal` → 用户点应用 → `persistShellVariableWrite`；失败保留提案（`shellVariableProposals.js` + `AppV2.vue:307-364`）。
4. **`__storyforge*` 保留位在前端两条通道被拒**：`shellVariableOutbox.js:15-17,64-101`（前缀分割前后各判一次）、`mvuStatTree.js:16` 黑名单。范围：上述文件 + 相应测试。
5. **MvuJsRuntime 的 source 校验与 pending 生命周期**：`getTrustedMvuRuntimeMessage` 严格 `event.source !== runtimeWindow`；`MvuPendingMap` 成功/超时/未就绪三路都清理并回传 error（`commands/mvu.rs:38-43`、`mvu_webview_runtime.rs:61-71`、`MvuJsRuntime.vue:383-414`）。
6. **插件桥权限表与 id 冒充防护**：`API_METHODS` 6 方法逐一映射权限；`requiredPermissions`+`hasAnyPermission`；`llm.generate` 显式 unsupported；未知方法报错；`isTrustedSource` + `data.pluginId !== plugin.id` 双校验（`plugin-bridge.js:181-213,1785-1896`）；`prompt_hook_request` 不广播给插件（:366-368，有单测）。范围：`plugin-bridge.js` 关键段。
7. **sandbox 属性正确**：`PluginHost.vue:7`、`CardShellHost.vue:30`、`MvuJsRuntime.vue:8` 均为 `sandbox="allow-scripts"`（无 `allow-same-origin`/`allow-popups`/`allow-top-navigation`/`allow-forms`），单测锁定（`frontend/tests/components-v2/card-shell-host-sandbox.test.mjs`）。
8. **Rust 侧命令权限校验与未注册插件拒绝**：`commands/plugins.rs:108,130,150,191-195,223,268` 全部 `ensure_permission`；`infra-plugin-host/src/lib.rs:246-268,471-477` 未注册一律 `NotFound`（card-shell 虚拟 id 亦然）。
9. **SQLite Meta/MVU 事务原子性与故障注入**：`sqlite_meta_repo.rs:62-135` 单 UoW 内含 barrier+revision+全部 action；`infra-sqlite/src/unit_of_work.rs` drop→rollback 有测试；`sqlite_mvu_repo.rs:158-162,225-231` 同款；`tests/sqlite_meta_lifecycle.rs:194-227`、`sqlite_meta_accept_fault.rs`、`sqlite_meta_multi_role_atomic.rs`、`sqlite_mvu_repo.rs:396-447` 端到端断言回滚。
10. **preview/accept 共用单一校验函数**：`typed_patch.rs:186-209` `validate_patch_preconditions` 被 preview 与 accept 共调，无「两套逻辑」分叉；preview 不写盘（`meta_typed.rs:414-431` 克隆快照）。
11. **MVU apply 幂等且不覆盖存量值**：`merge_schema` BTreeMap 去重 ⇒ 二次 apply `NoChanges` 拒绝写盘；实例回填只补缺失键（`meta_typed.rs:990-1005`、`sqlite_mvu_repo.rs:184-200`，`lib_tests_meta.rs:160-176` 断言保留 hp=42/turn=9）。
12. **`value_expr` 无任意 JS 执行面 & `run_original_js` 是桩**：唯一求值是 `JSON.parse` 分支；无 `eval`/`new Function`（grep 零命中）；`InteractionAction::RunOriginalJs` 全仓无执行器，真正执行的是另一字段 `fallback_fragments`。
13. **截断无 UTF-8 边界 panic**：`truncate_for_prompt` 全程 `chars()`；`normalize_mvu_key` 的字节切片有 ASCII 边界前置保证（`variables.rs:181-193`）。
14. **MVU 键归一前后端镜像一致**：`mvuKey.js:7-22` 与 `variables.rs:170-193` 逐行等价，双向测试覆盖。
15. **PNG 编解码干净**：`png.rs:178-179` base64（ASCII 文本，无 Latin-1 截断——中文实测断言 `png.rs:367`/`lib.rs:679`）；null 分隔符与 CRC 覆盖 type+data（:147-164）；越界/溢出守卫（:27,34,46）；IEND 前插入失败即报错（:205-228，fail-closed）；截断 chunk 有测试（:594-600）。
16. **未知 ST 字段保留**：`character.rs:101-102,110-111,147-148` 三层 `#[serde(flatten)] extra`，全仓无 `deny_unknown_fields`；book 级与 entry 级往返有测试（`compat.rs:1897-1928`、`character.rs:1177-1190`）。
17. **Bundle 外键与版本门 fail-closed**：`import_export.rs:839-845,862-898,909-915,939-957` 悬空引用全拒；summary 图对称/层级/span 连续/环检测（:285-384）；`format_version == 0 || > 3` 双侧同口径拒绝（:631,:1024）；SQLite bundle 单事务 + 4 个注入点无残留（`tests/sqlite_character_lifecycle.rs:657-707`）。
18. **锁中毒处理一致**：Meta 相关状态全部 `unwrap_or_else(|p| p.into_inner())`，无 `lock().unwrap()` panic 路径。
19. **app-meta / mvu_import / commands/mvu 生产码无 unwrap/expect**（测试模块除外，已逐处核对）。
20. **前端导出链不参与序列化**：`tEXt`/`ccv3`/`chara` 全部 Rust 侧产出，前端仅传 `Array.from(data)`（M-25 的放大问题另计）。

### 5.2 低风险观察（不单列发现）

- `install_plugin` 无「安装即授权」提示（权限仅徽章展示）——若 M-01 修复，风险显著降低。
- `plugin_prompt_hook_result` 未知 request_id 仅 `warn`；`MAX_PROMPT_HOOK_AUDIT_RECORDS=100` 意味着高频 hook 可冲刷早期审计（`recordedAt` 宿主写入不可伪造，刷量有效）。
- `tauri dev` 下主窗 CSP 不生效（`devUrl` 由 Vite 提供，CSP 由 Tauri 协议响应头下发），仅影响开发态。
- `shellVariableOutbox` 的 `instance:` 前缀语义已被测试固化为契约（`frontend/tests/shell-variable-outbox.test.mjs:35-52`）⇒ 修复 M-18 时需同步改测试。

---

## 6. 需要 Lead 重点复核的结论

1. **M-01 的运行时裁决（最高优先，30 秒可定论）**：本域唯一的 P0，链路 5 环已在依赖源码中核实，剩「WebView2 是否向 `sandbox="allow-scripts"` 子帧投递 document-created 脚本」。验证：装一个 `permissions: []` 的插件，`entry_html` 打印 `typeof window.__TAURI_INTERNALS__`（或壳卡加载后 `await window.__TAURI_INTERNALS__.invoke('list_conversations')`）。**若成立，本项应上升为跨域最高优先修复项**，因为它同时使「插件权限桥」「卡壳白名单」「Meta preview 门」的边界声明失效；若证伪（脚本未注入子帧），M-08/M-24 等措施的紧迫性也应随之重估。
   - **交叉验证说明**：本人与插件子代理**各自独立**得出同一链路、同一结论，且 `permissions/` 目录缺失、`acl-manifests.json` 无 `__app__` 两项由本人在本机复测确认。
2. **M-02 的解析器语义**：`host_of` 手写解析与 `url` crate 的分歧已在本机依赖源码中定位（`url-2.5.8/src/parser.rs:40,899`）。Lead 若需 100% 确证，可用一行 `url::Url::parse("https://evil.example\\@cdn.jsdelivr.net/x")` 打印 host；但即使不证，**手写解析作为安全边界本身就是缺陷**，建议直接改。
3. **M-05 的外部契约**：需要 Lead（或跑真实卡脚本的人）用 ST 官方 spec 或第三方卡工具实测确认「position 应为字符串」的后果等级；本文引用 spec 类型 `'before_char' | 'after_char'`，未在本机验证第三方工具行为。
4. **M-04/M-15/M-16 的跨域依赖**：落盘证据链跨到 `crates/app-pipeline/src/lib.rs` 与 `crates/tauri-app/{production_postprocess,runtime_support}.rs`（本人已抽查 `app-pipeline:1534-1541`）。修复排期请以那两处写入边界为准，勿只改前端。
5. **M-27c「先 preview 才 accept」的契约裁决**：文档（`RELEASE-CHECKLIST.md:73,127`）称 preview→accept 是命令层契约，代码无 preview 状态/令牌。需 Lead 决定是「收紧代码」还是「下调文档措辞」——这直接影响 ROADMAP:84「不越权」的判定口径。
6. **M-06 的门禁口径**：若本次评审的结论要进入「ST 往返保真」的对外表述，必须先接受「现有 compat 报告不构成保真证据」这一点（真实卡全 `#[ignore]`、闸门把漂移标 Intentional）。
7. **本文档与 `docs/RELEASE-STATUS.md:29` 的口径冲突**：请 Lead 在总报告里统一（通道闭合 ≠ 边界不可绕过）。

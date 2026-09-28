# 域6 修复记录：Meta / MVU / 插件 / Card Shell / 导入导出

- **任务**：`task-13`（修复域6：M-01..M-32 + T-03），owner `review-meta-plugin`
- **审查基线**：`docs/review-2026-09-13/06-meta-mvu-plugin.md`（565 行，P0 1 / P1 7 / P2 18 / P3 6）
- **代码基线**：HEAD `ab894c6` + 本轮批量修复工作树
- **状态口径**（严格按 Lead 要求，不谎报）：
  - 已修复 = 代码改动 + 本域测试落地
  - 已修复(降级方案) = 收敛了真实风险但未达报告中的完整方案，风险与前置依赖已写明
  - 判定非问题(附证据) = 复核后不成立，附代码/源码依据
  - 暂缓(附理由+建议) = 本轮不修，给出为什么 + 下一步怎么做
- **硬约束遵守**：只改本域文件（`crates/{infra-plugin-host,infra-import,app-meta}`、`crates/tauri-app/src/{card_shell_cache.rs,shell_doc_protocol.rs}`、`commands/{card_shell,plugins,meta,meta_typed}.rs`、`capabilities/**`、frontend 域6 文件及其测试）；**未**改 `docs/**`（除本文件）、`README.md`、`CLAUDE.md`；**未**新建 `crates/tauri-app/tests/**` 文件；未跑 `cargo test --workspace`（Lead 独占）。

---

## 2 结论摘要

1. **P0（M-01）没有修好，也不该被说成修好**：本轮完成了 4 条静态守卫测试 + CSP 卫生收敛 + 半径收敛，并**用 tauri/wry 源码证据否决了"加 app ACL manifest"这条看似可行的方案**；真正的修复需要在主帧注入隔离 / 独立 webview / 命令层主帧凭据三者中做产品决策，且必须在 Windows 运行时做 PoC —— 现状标为"部分收敛 + 主体暂缓"。
2. **P1 全部落地或降级落地**：M-02（URL 解析换成 reqwest 同源 `url::Url`，`\@`/IP/userinfo/端口全被测试钉住）、M-03（patch ↔ Campaign 绑定，preview/accept/列表三处 + 测试）、M-07（`event.source`，前端）、M-08（后端复核 `ModifyPrompt`，未更新前端时安全降级）由本域完成；M-04/M-05/M-06 分别由前端镜像、domain、infra-import 落地（跨域台账见 §10）。
3. **审计链（M-23）与 JSON accept 的原子性（M-10）/ Turn 屏障 TOCTOU（M-14）明确暂缓**，理由与补丁方向写在条目里 —— 这三条都涉及跨域写作用域或锁序风险，不适合在修复末段抢改。

| 严重度 | 已修复 | 已修复(降级方案) | 判定非问题 | 暂缓 | 跨域承接 |
|---|---|---|---|---|---|
| P0 | 0 | 0 | 0 | 1（M-01，含 4 条守卫 + 3 项收敛） | 0 |
| P1 | 5（M-02、M-03、M-07、T-03、T-05） | 2（M-04、M-08） | 0 | 0 | 2（M-05→域1、M-06→域6 infra-import） |
| P2 | 8（M-09、M-11、M-12、M-13、M-15、M-17、M-19〜M-22 中的本域项） | 3（M-16→域2、M-18→域5、M-25→域5） | 1（M-29 部分更正） | 3（M-10、M-14、M-23） | M-26→域1、M-16→域2 |
| P3 | M-27d/e、M-28a/b/c、M-31、M-32 见 §6 | — | — | — | M-30→task-f7 |

（上表为入口速览；每条的实际状态与证据以下文条目为准。）

---

## 3 P0

### M-01 [P0] Windows 壳/插件/MVU/TH iframe 仍持有 Tauri IPC —— **暂缓（主体）**，本轮完成 4 项收敛

**判定**：`暂缓(附理由+建议)`。这不是"改一行就修好"的问题，本轮**没有**把子帧可达 IPC 这件事关掉，任何"已修复"的表述都是假的。

**本轮已落地（可静态验证）**

1. **壳文档 CSP 卫生收敛**（`crates/tauri-app/src/shell_doc_protocol.rs`，`SHELL_DOC_CSP.connect-src`）
   增补 `ipc: http://ipc.localhost`，与主窗 CSP 同源。
   依据：`tauri-2.11.5/scripts/ipc-protocol.js`——自定义协议 fetch 失败（该文件注释：要么 webview 拦了自定义协议，要么是 CSP 错误）时会回退到 `window.ipc.postMessage`，而回退路径不受 CSP 约束。
   **明确标注：这是消除回退路径依赖的卫生改动，不构成 M-01 的修复**——两条路径最终都到达同一个 IPC handler。前端镜像同步改动见 `frontend/src/utils/cardShellCsp.js`（M-01 分工内）。
2. **4 条静态守卫测试**（同文件 `#[cfg(test)]`，CI 随 `cargo test -p storyforge` 生效）
   - `shell_csp_keeps_tauri_ipc_sources_for_custom_protocol_fetch`：`connect-src` 必须保留自定义协议 IPC 来源（防止有人"收紧 CSP"时又把 IPC 推回 fallback）。
   - `shell_csp_stays_locked_down`：壳 CSP 必须保持 `default-src 'none'` / `object-src 'none'` / `form-action 'none'`，且不得出现 `https://` 远端主机或通配符。
   - `capability_grants_only_the_main_window`：`capabilities/default.json` 的 `windows` 只能等于 `["main"]`，且不得出现 `webviews`/`remote`。
     依据：`tauri-2.11.5/src/webview/mod.rs:1787-1852`，ACL 判定用 `Origin::Local` + 窗口标签，子帧与主帧在 ACL 眼里完全一样。
   - `acl_manifest_absence_is_a_known_risk`：当前无 `permissions/` ⇒ `has_app_acl_manifest == false` ⇒ **对本地来源的应用命令整体跳过 ACL**，测试打印醒目 warning（含 PoC 步骤与三条真修复方向）把已知风险留在 CI 日志；一旦有人新增 `permissions/`，则强制断言 `gen/schemas/acl-manifests.json` 含 `"__app__"`，防止"以为加了 ACL"的假安全感。
3. **否决"新增 app ACL manifest"方案（附源码依据，避免后人重复踩）**
   - ACL 生效条件：`tauri-2.11.5/src/ipc/authority.rs:132-134` 解析 manifest，`webview/mod.rs:1823` 的 `(plugin_command.is_some() || has_app_acl_manifest || !is_local) && invoke.acl.is_none()` ⇒ 一旦 App manifest 存在，**所有** app 命令都要过 ACL；没有逐命令 capability 授权，主窗口自己的命令会被拒（`Command X not allowed by ACL`）。
   - 更关键：**ACL 无法区分父子帧**——`Origin::Local` 不带 URL，窗口标签恒为 `main`（子 iframe 与主帧同 label），所以加 ACL 修不了 M-01 这个洞。
   - 结论：不提交该改动；本文与守卫测试把它固化为"已知风险 + 禁止盲改"。
4. **爆炸半径收敛**（属于本域的其他门禁）
   - M-03 Campaign 绑定：跨战役 patch 不能再写盘（§4）。
   - M-08 `plugin_prompt_hook_result` 后端复核 `ModifyPrompt`（§4）。
   - M-07 `event.source` 校验（S3，前端）。
   - M-02 URL 白名单解析不再可绕过（§4），使壳侧"自己 allowlist 再拉取"的面收敛。

**仍需 Windows 运行时 PoC（本环境无法伪证）**

- 步骤：装一个 `permissions: []` 的插件 → 在其 iframe 内 `console.log(typeof window.__TAURI_INTERNALS__)` → 若为 `object`，再 `await window.__TAURI_INTERNALS__.invoke('list_conversations')`；对照主窗同一调用。期望（按源码推断）：子帧同样拿到对象且调用成功 ⇒ 证实 M-01。
- 需要 PoC 的原因：`wry 0.55.1` 在 WebView2 分支丢弃 `for_main_frame_only`（`wry/src/webview2/mod.rs:492-495`，`wry/src/lib.rs:990`），`__TAURI_INTERNALS__` + invoke key 因此进入所有子帧——这是**源码级**结论，但"实际到达"必须运行时确认。

**建议的真修复（需产品决策，不在本轮实施）**

1. 升级/回移 wry，让 `for_main_frame_only` 在 WebView2 真正生效（最小、最贴近根因，但依赖上游）。
2. 把不可信 HTML（卡壳/插件/MVU/TH）放到**独立 webview**，不再与宿主共享同一帧树（隔离最彻底，改动面最大）。
3. 命令层要求"主帧凭据"：宿主在 `init` 时生成一次性令牌，只有主帧持有；敏感命令校验令牌，子帧一律拒绝（不依赖上游，但要逐个命令盘点，且需防令牌泄漏到子帧）。

---

## 4 P1（含 T-03）

### M-02 [P1] 卡壳白名单 + IP 字面量可被 `\@` 绕过 —— **已修复**

- **修法**：`crates/tauri-app/src/card_shell_cache.rs` 新增 `parse_http_url`，改用 **`reqwest::Url`**（= `url::Url`，reqwest 直接再导出，无需新增依赖）解析，**与实际请求同一套 WHATWG 语义**；`host_of`/`is_url_allowed`/`validate_fetch_url`/`should_fetch_by_ranges` 全部走它。额外硬约束：只允许 `http|https`；拒绝 URL 凭据（`user:pass@`）；拒绝非默认端口；IP 字面量检查前剥掉 IPv6 方括号（`host_str()` 保留 `[::1]`），并对 `2130706433`/`0x7f.0.0.1` 这类十进制/十六进制写法（`url` 已归一成点分形式）同样拒绝。
  - 关键点：旧实现 `https://evil.example\@cdn.jsdelivr.net/x` 返回 `cdn.jsdelivr.net`（在白名单里），而 URL 标准在反斜杠处终止 authority ⇒ 真实目标 `evil.example`。前后两条路径共用同一解析器后，**判定与实际连接不可能再分歧**。
- **测试**：`host_parser`（扩展：大小写归一、query/fragment、非 http scheme、显式端口）、`authority_smuggling_cannot_bypass_allowlist_or_ip_guard`（`\@` 双变体、IPv4/IPv6/十进制/十六进制字面量、凭据、端口、正常 URL 仍放行，allowlist 与 fetch 守卫双向断言）。
- **残留**：`should_fetch_by_ranges` 的 `i.ibb.co` 特判未改（行为不变）。

### M-03 [P1] Typed patch 未与 Campaign 绑定 —— **已修复**

- **修法（跨 crate 两半）**：
  - `crates/app-meta/src/typed_patch.rs`：`TypedPatch` 新增 `campaign_id: Option<String>` / `campaign_revision: Option<u64>`，builder 统一盖章（`stamp_patch_scope`，取自 `PreviewInput.campaign`）。
  - `crates/tauri-app/src/commands/meta_typed.rs`：
    - `patch_campaign_binding_ok()`：`None`（旧格式内存 patch）一律视为不满足 ⇒ fail closed；
    - **preview**：绑定不符 → `TypedPatchStatus::Stale` + 返回 `{stale:true,reason:"campaign_mismatch"}`（与既有 stale 语义一致，前端接受按钮已在 M-27c 下 fail-closed）；
    - **accept**：绑定不符 → 先 `mark_typed_patch_stale` 再返回 `validation` 错误（错误文案含 `patch.campaign_id` 与目标 campaign），**在写盘前**；
    - **列表**：`meta_list_typed_patches` 新增可选 `campaign_id` 过滤（不传保持旧行为，写边界不依赖列表过滤）。
  - `crates/tauri-app/src/lib_tests_meta.rs`：7 处 `TypedPatch { .. }` 字面量补 `campaign_id`（有 campaign 的用其 id，纯去重/dismiss 用例给固定 id）。
- **测试**：`test_meta_typed_patch_is_bound_to_its_campaign` —— 用**无 target id** 的 `UpdateCampaignVariable` 造 A 战役 patch：B 上 preview `stale:true` + `reason=campaign_mismatch`；B 上 accept 报错且 B 变量未被写脏；回到 A 仍可接受且变量写在 A。
  - 为什么必须用无 target 的 action：带 target id 的跨战役用例会被 target 查找先拒绝，属"假通过"（旧 `sqlite_meta_lifecycle.rs` 用例正是如此）。
- **跨域验收**：`crates/tauri-app/tests/sqlite_meta_lifecycle.rs` 的同语义 sqlite 用例由 **review-storage** 承接（已确认，按上述 4 点改写），完成后记入 `02-storage-fixes.md`。

### M-04 [P1] MVU JS fallback 变量回写绕过保留命名空间守卫 —— **已修复(降级方案)**

- **前端（本域）**：新增 `frontend/src/utils/mvuExecuteResult.js`（`buildMvuExecuteResultData`：丢弃 `__storyforge*` 键、默认 `console.warn`、可注入 `onDrop`），`MvuJsRuntime.vue` 改为 `invoke('mvu_execute_result', buildMvuExecuteResultData(d))`；测试 `frontend/tests/mvu-execute-result.test.mjs`（丢弃保留位 / 无回调时告警 / 恶意或缺字段归一）。
- **Rust（落点在 `crates/app-pipeline`，跨域）**：`crates/app-pipeline/src/lib.rs` 的 `push_mvu_js_variable_updates` 正在由 **review-pipeline** 落地同一口径（丢弃保留命名空间键）并落测试 `mvu_js_fallback_drops_reserved_namespace_keys`，已两次对齐（含"命令层 T-06 守卫不覆盖 accept 路径，前端不是安全边界"的理由）。
- **为什么是降级**：作用域语义（一律落 campaign）未动 —— 见 M-18（域5）。

### M-05 [P1] 世界书 `position` 导出为数字 —— **跨域承接（域1 `review-domain`）**

- 落点 `crates/domain/src/card_studio.rs`（`position_as_i32` 及其导出路径），不在本域写作用域。本域只保留证据与验收口径（`ST` 契约要求字符串；M-26 是同族的兜底折叠问题）。域1 修完后由 Lead 全量门禁收口。

### M-06 [P1] 保真闸门失灵 —— **跨域承接（本域 `crates/infra-import`，由 S1 落地）**

- 范围：真实卡测试 `#[ignore]`、fixture 手工桩、分诊器把 wire 漂移判为 `Intentional`。已在本域内（infra-import）修复/降级，细节见 §10 台账。
- **本域附带修复（独立）**：`crates/tauri-app/src/lib_tests_import_export.rs` 两条真实卡 ignored 用例的默认路径从仓库根 `test-card.png`（**不存在**）改为 `data/local/test-card.png`（与 infra-import 同约定），`#[ignore]` 保留；新增守卫测试 `default_real_card_fixture_path_targets_data_local_test_card`。修复前这两条用例即使在本机有真实卡也必然 panic。

### M-07 [P1] TavernHelperRuntime 桥不校验 `event.source` —— **已修复（前端，S3）**

- 落点 `frontend/src/components/TavernHelperRuntime.vue`（与 `mvu-runtime-bridge.js` 基线对齐：沿 `parent` 链归属本壳 iframe，不再信任消息里的 `pluginId` 字段）。测试见 S3 报告（§10）。

### M-08 [P1] `plugin_prompt_hook_result` 无插件绑定/权限校验 —— **已修复(降级方案)**

- **修法**：`crates/tauri-app/src/commands/plugins.rs`
  - `plugin_prompt_hook_result` 新增两个**可选**入参 `plugin_id: Option<String>` 与 `modifier_plugin_ids: Option<Vec<String>>`（`Option` 缺省 ⇒ 旧前端不报错）；
  - 判定核心抽成 `prompt_mutation_denial(registry, plugin_id, modifier_plugin_ids)`：提交 `messages` 时**必须**声明改写者，且每个声明插件都必须在 `PluginRegistry` 中**启用**并持 `ModifyPrompt`（`ensure_permission` 同时查 enabled 与权限）；任一不合格 ⇒ 丢弃 messages（保留原始提示词）、`tracing::warn`，**仍然结算 pending**（不挂起、不报错）。
  - 为什么是"改写者集合"而不是单值：prompt hook 广播给宿主，最终 messages 可能由多个插件依次改写；前端应在链内收集"实际参与改写且声明了 `ModifyPrompt` 的插件 id"用 `modifier_plugin_ids` 传，`plugin_id` 单值仅兜底（契约已 message 给域5）。
- **测试**：`prompt_hook_mutation_requires_a_permissioned_modifier_plugin`（未声明 / 空白 id / 未注册 / 无权限 / 已禁用 / 多改写者中含不合格者 → 全拒绝；单一有权限者 → 放行；单值与列表等价）。
- **为什么是降级**：前端 wrapper 尚未传参（域5 已接受该分派），在此之前插件改 prompt 功能**安全降级为"不改写"**（不是崩溃）；且 `request_id` 未与签发时的插件集合绑定（需改 `commands/writing.rs` 的 pending 表结构，属域6 但不在本轮），持 `ModifyPrompt` 的插件 A 仍可抢占插件 B 的 `request_id`。
- **状态**：后端门禁已生效 + 测试落地；前端传参为已知前置依赖。

### T-03 [P1，域4 转来] `card_shell_fetch_url` 阻塞命令线程 —— **已修复**

- **修法**：改 `async fn` + `tauri::async_runtime::spawn_blocking`（reqwest blocking 客户端 + 磁盘缓存在命令线程上会阻塞 IPC 热路径）；错误映射与返回结构不变，join 失败显式转 `internal`。
- 同文件顺带完成 **T-05**：`card_shell_register_doc` / `card_shell_register_module` 从 `Result<_, String>` 改为结构化 `Result<_, TauriCommandError>`，新增 `shell_doc_error()`（超限/登记表满 → `validation`，其余 → `internal`），与其余命令一致（前端 `errorText.js` 才能显示结构化 `{type,message}`）。
- **T-09**：`card_shell_allow_host` **保留 + 标注**（与 T-15 口径一致，删除需同步 `lib.rs` 注册表与域4 基线快照）。代码注释与本文都记下它的放大效应：M-01 未修复前，壳 iframe 可自己 allowlist 任意 host 再经宿主代持代理拉取 —— M-01 落地修复时应一并评估删除。

---

## 5 P2

| ID | 状态 | 说明（修法 / 证据） |
|---|---|---|
| M-09 patch revision 陈旧校验对 LLM 提案恒被跳过 | **已修复** | `stamp_patch_scope` 同时盖 `campaign_revision`（`typed_patch.rs:350-356`）；accept 侧 revision 校验因此对 LLM 提案也生效。测试见 S2 报告 §10。 |
| M-10 JSON accept 无回滚（半提交） | **暂缓(附理由+建议)** | 现状已有"全部 action 先预演再写盘"（`validate_patch_preconditions` + `test_meta_accept_typed_patch_preflights_all_actions_before_writing`），确定性的目标缺失/前置条件失败不会半提交；但**写盘中途 IO 失败**仍可能半提交。真回滚需要 pre-image 快照 + 恢复（跨 campaign/instances/knowledge/tasks 四类集合），属跨 `campaign_store` 的写作用域。建议：把 accept 的写盘包进 `CampaignStore` 的"快照-试写-回滚"事务 API（或统一走 SQLite UoW），同时在文档里把"nothing is partially applied"改成"preflight 失败不落盘；IO 失败可能半提交"（文档条目见 §11）。 |
| M-11 卡缺失降级为空 definitions | **已修复** | `load_meta_snapshot_from_store` 改为卡缺失直接 `not_found`（"拒绝按空 definitions 体检"），与 SQLite 路径同口径。测试 `test_meta_snapshot_fails_closed_when_card_is_missing`。 |
| M-12 变量写入无 key 白名单 | **已修复** | `typed_patch.rs` 增 `RESERVED_VARIABLE_NAMESPACE = "__storyforge"`，写入键先剥 `stat_data.` 前缀再判保留位（大小写不敏感）+ 拒绝空键；测试含 `__storyforge_card_shell_variables` 与 `stat_data.__storyforge_hidden` 两变体（`typed_patch.rs:2070-2100` 区）。 |
| M-13 legacy `meta_accept_patch` 假成功 | **已修复** | `commands/meta.rs` 改为"持久化全部成功后才提交内存世界书与 `applied`"，失败直接返回错误、绝不静默标 applied。**残点已补修（task-25）**：`crates/app-meta/src/lib.rs` `execute_action` 的 `Create{target:"world_info"}` 缺 context 时由静默 no-op 改为 `MetaError::ExecutionFailed`（错误点名 `world_info_entries`），测试 `test_execute_action_create_world_info_without_context_errors`。 |
| M-14 JSON accept Turn 屏障在锁外（TOCTOU） | **暂缓(附理由+建议)** | 需要①`CampaignStore`/`AppState` 暴露 turn store（当前 `AppState.turn_store` 非公有字段，`CampaignStore` 无 turn 访问器）或②把 barrier 下沉进 `turn_coordinator::with_campaign_lock`。两者都要改 task-9/task-11 的文件（`turn_store.rs`/`lib.rs`/`turn_coordinator.rs`），且在全局提交锁内取存储锁存在**锁序反转**风险（commit → storage）。建议：给 `with_campaign_lock` 增加可选 campaign_id 参数，统一在锁内做 `reject_if_active_turn`，JSON 与 SQLite 两条 accept 路径共用。当前缓解：`typed_patch_accept_lock` 串行化 + 前置 revision/绑定校验 + action 预演。 |
| M-15 MVU 运行期写边界不做键归一 | **已修复(降级方案)** | 命令层新增键归一守卫（域4 T-06，`commands/variables.rs` 区）阻止新增双记法写入；**历史已存在的双记法不在本任务内合并**（需要迁移脚本 + 产品确认），域4 会在 T-01..T-16 后评估是否做全量归一。 |
| M-16 MVU preview 不归一 / apply 归一 | **跨域承接（域2 `review-storage`）** | `backend_workflows.rs` preview 侧补 `normalize_schema_keys`，与 `commands/meta_typed.rs` apply 侧同参，保证单一事实来源（已确认，完成后记入 `02-storage-fixes.md`）。 |
| M-17 分析器 24K 判据按注释字符串 | **已修复 + 暂缓（需产品决策）** | 额度门已修：`crates/app-meta/src/prompts/mvu_analyzer.rs` 改按**正文形态启发式** `body_looks_like_variable_tree`（缩进 `key: value` 行 ≥6 且占比 ≥0.6，排除 `- `/`* `/`# ` 列表与 `//` 注释）给 24K/40K 额度，规则类仍 4K；**收录门未动**（见下方"暂缓（需产品决策）"）。 |
| M-18 MVU 交互按钮作用域推断错误 | **跨域承接（域5 `review-frontend`）** | `frontend/src/composables/useMvuStatusPanel.js` 改为显式 `instanceId` 传递；域5 已确认按"能改就改，否则记暂缓 + 提议"处理，本域不重复记录。 |
| M-19 内联壳信任粒度是"消息" | 见 §10（S3，前端） | 前端信任分级收敛（文档级命中 + 未命中确认卡）。 |
| M-20 插件禁用/卸载不即时失效 | 见 §10（S3，前端） | iframe/hook/权限即时失效。 |
| M-21 prompt hook 无结构校验 + 无整链预算 | **已修复** | `validateHookedMessages`（角色白名单/非空/system 原样保留）+ `resolveHookedMessages` 出口校验；`DEFAULT_PROMPT_HOOK_CHAIN_BUDGET_MS = 15000` 整链预算（单插件 min(per-plugin, remaining)，`null` 显式关闭）；两个新审计状态进 `SAFE_STATUSES`。**裁决：保留"整体拒绝改写 system 内容"（不放宽）**——system 提示词是宿主信任根，前端校验是纵深防御而非唯一门禁；追加自己的 system 消息仍允许。 |
| M-22 `ReadMemory` 被事件 `full_text` 绕过 | 见 §10（S3） | 事件载荷按权限裁剪。 |
| M-23 审计链死代码 + 前端审计不落盘 + 变量写入无审计 | **暂缓(附理由+建议)** | 三点事实复核成立（见审查报告）：`app-logging` 只对 `Error`/`LlmCall` 落盘、`emitAudit` 在 hook 返回后写且吞异常、`plugin_set_variable` 无审计。真修复需要：①把审计 kind 提升到可落盘级别（改 `crates/app-logging`，域3/task-10 范围）；②前端审计事件落盘通道（域5）；③决定"审计不构成门禁"的产品口径；④`infra-plugin-host/src/audit.rs` 的链式 API 要么接入链路、要么删除（删除会破坏其既有单测与 `compat_matrix` 断言）。本轮只在本域代码注释里保留事实描述，未做半改。 |
| M-24 插件帧自导航后宿主仍信任 | 见 §10（S3） | `@load` 后重置 handler / 首帧 nonce。 |
| M-25 整卡 JSON 数字数组过 IPC | **跨域承接（域5）** | 域5 与审查域6 结论一致：**暂缓**（改为 ArrayBuffer/base64 需前端 + `import_character` 命令签名协同），记录引用本域方案。 |
| M-26 `position_as_i32` 的 `_ => 0` 兜底 | **跨域承接（域1）** | 与 M-05 同族，落点 `crates/domain/src/card_studio.rs`（本次编译期报错即在该文件的在途改动）。 |
| M-27 Meta 杂项 | 已修复（本域项） | (a) accept 后 revision 递增 → 域2；(b) 失败轮次提案堆积 → 见 §10（app-meta）；(c) preview 失败 fail-open → 已修复（域6 前端，`_stale=true` + `_previewError`）；(d) **本域已修复**：`MvuTranslationSummaryDto.routing` 从 `format!("{:?}")` 改为 serde 序列化（`{"kind":"native"\|"hybrid",…}`），前端 `routingText()` 不再恒显示"混合（）"；(e) **本域已修复**：该字段序列化失败显式返回 `internal` 错误，不再 `unwrap_or(Null)` 静默吞错。 |
| M-28 MVU 杂项 | 已修复（前端，S4） | (a) `instance:`/`campaign:` 前缀切分后空键拒绝；(b) 全角/数学运算符表达式拒绝（而不是当字符串写入）；(c) `mvuStatTree` 优先级确定化（子树优先于整值写、删除不可达分支）；(d) 见 §10（domain）。 |

---

## 6 P3

| ID | 状态 | 说明 |
|---|---|---|
| M-27(d)(e) | 已修复 | 见 §5（routing 序列化 + 失败传播）。 |
| M-28(a)(b)(c) | 已修复 | 见 §5（前端，S4）。 |
| M-29 前端死代码 | **判定非问题(附证据) + 部分修复** | 复核更正：`partitionShellMountsByTrust` 并非"0 测试"——`frontend/tests/card-shell-display.test.mjs:61-79` 有专门测试；"无生产调用方"成立（`cardShellDisplay.js:286` 之外无引用）。处理：改为"让策略函数成为唯一来源"（`promptHooks.js` 策略表新增两个链状态 + 返回 `surface`，`catch` 改为按策略 `failOpen` 判定），**未删除**函数（删除会连文档化策略表与既有测试一起删，接线仅 2 行）。 |
| M-30 文档漂移 | **跨域承接（task-f7）** | 见 §11 需文档同步条目。 |
| M-31 插件宿主低危项 | 见 §10（S3/S2） | 按报告逐条处理。 |
| M-32 导入导出低危项 | 见 §10（本域 infra-import，S1） | 含本条附带修复：真实卡 ignored 用例默认路径（§4 M-06）。 |

---

## 7 门禁与证据

> 说明：本域修复期间，多个域在同一工作树上并发编辑（`crates/domain`、`crates/app-agent`、`crates/app-pipeline`、`crates/tauri-app/src/{turn_store,storage_backend}.rs` 都出现过在途编译错误），因此本域按"单 crate `cargo check --all-targets` + 本域聚焦测试"取证，全量门禁由 Lead 在 task-15 统一执行（本域不跑 `cargo test --workspace`）。

| 命令 | 结果 |
|---|---|
| `cargo check -p storyforge --all-targets` | 见下方"最终一次"记录 |
| `cargo test -p storyforge --lib -- <本域测试名>` | 见下方"最终一次"记录 |
| `cargo check -p storyforge-infra-import --all-targets` | 见 §10（S1 报告） |
| `cargo check -p storyforge-app-meta --all-targets` | 见 §10（S2 报告） |
| `cd frontend && npm.cmd test && npm.cmd run test:ui && npm.cmd run build` | 见下方"最终一次"记录 |

<!-- GATE-RESULTS:START -->
| 命令 | 结果（本域最终一轮，均为干净树） |
|---|---|
| `cargo check -p storyforge --all-targets` | **exit 0**（`Finished dev profile`，1m35s） |
| `cargo test -p storyforge --lib -- <本域 11 条测试>` | **exit 0**：11 passed / 0 failed（`host_parser`、`authority_smuggling_cannot_bypass_allowlist_or_ip_guard`、`test_meta_typed_patch_is_bound_to_its_campaign`、`test_meta_snapshot_fails_closed_when_card_is_missing`、`prompt_hook_mutation_requires_a_permissioned_modifier_plugin`、`shell_csp_keeps_tauri_ipc_sources_for_custom_protocol_fetch`、`shell_csp_stays_locked_down`、`shell_csp_allows_only_the_restricted_protocol_as_a_module_origin`、`capability_grants_only_the_main_window`、`acl_manifest_absence_is_a_known_risk`、`default_real_card_fixture_path_targets_data_local_test_card`） |
| `cargo test -p storyforge-app-meta` | **exit 0**：130 passed / 0 failed（S2 新增 18 条 + task-25 补的 `test_execute_action_create_world_info_without_context_errors`） |
| `cargo test -p storyforge-infra-import` | **exit 0**：69 passed / 0 failed / 1 ignored（忽略项为真实卡用例，`#[ignore]` 保留）；另手工 `--ignored` 跑通（新默认路径命中真实卡） |
| `cargo test -p storyforge-infra-plugin-host` | **exit 0**：28 passed / 0 failed（含新增 `Permission::WriteChat` + compat matrix 覆盖） |
| `cargo check -p storyforge-app-meta / -infra-import / -infra-plugin-host --all-targets` | 三个 crate 均 **exit 0** |
| **前端（我方直跑，绕过被沙箱禁止的 npm 子进程）**：`node frontend/tests/<file>.mjs` × 11 个 node:test 文件 | **192 passed / 0 failed**：`prompt-hooks`(23)、`plugin-bridge`(78)、`usePluginBridge`(18)、`card-shell-csp`(5)、`meta-panel-flow`(8)、`mvu-execute-result`(3)、`shell-variable-outbox`(8)、`mvu-interactions`(9)、`mvu-stat-tree`(5)、`prompt-hook-audit`(23)、`card-shell-display`(12) |
| `cd frontend && npm.cmd test && npm.cmd run test:ui && npm.cmd run build` | **本会话沙箱下不可执行**：`Error: spawn EPERM (errno -4048, syscall 'spawn')`，node:test runner 需要 spawn 子进程 + 管道 stdio。**按 Lead 裁决（task-15）由 Lead 收口执行**；本记录**不声称前端门禁已绿**。 |
| `node frontend/tests/components-v2/{plugin-host-handshake,tavern-helper-runtime-bridge,shell-aware-content}.test.mjs` | `node` 直跑**无输出**（vitest/happy-dom + SFC 转换用例，需转换器）。**域5 用 `%TEMP%` 下的单进程 vitest 等价 harness（`@vue/compiler-sfc` + happy-dom + `@vue/test-utils` + 自写 vitest shim，未入库）逐文件跑出**：`shell-aware-content.test.mjs` **13/13 绿（真绿，无 mock 依赖 ⇒ M-19 证据可用）**；`plugin-host-handshake.test.mjs` **harness 无法运行**（用 `vi.mock`，shim 不能替换模块，显式抛错不假装通过）⇒ **必须由 Lead 的 `npm run test:ui` 裁决，本记录不记为绿**；`tavern-helper-runtime-bridge.test.mjs` **2/4**（伪造 source / 无 source 两条绿；own iframe / nested frame 两条因 happy-dom 不执行 iframe 脚本而无法判定，**不确定 ≠ 红**）。域5 harness 汇总：`tests/components-v2/**` 31 文件 / 28 绿 / 136 pass（非本域文件不在此列）。**结论：M-24 的组件级证据仍待 Lead 门禁**；M-07 的两条归属校验用例为真绿 |
| `cargo test -p storyforge --lib`（全量 crate 内测试） | **exit 0**：**467 passed / 0 failed / 3 ignored**（域2 修掉 `turn_lifecycle.rs:1729` 编译红后复跑通过；此前该行曾挡住整个 lib test target，见 §12 第 7 条时间点事实） |

**前端改动验证口径（重要）**：域6 的前端代码**未经 `npm test`/`vitest`/`build` 完整门禁**（沙箱 EPERM）；但 11 个 node:test 文件已用单进程直跑取得 192/192 绿证，其中 3 处**真实红项**由 review-frontend 的直跑发现、在本轮修掉（见 §5 M-21 行）。仍可能存在 vitest 组件层（happy-dom/SFC 编译）差异，由 Lead 门禁最终判定。
<!-- GATE-RESULTS:END -->

---

## 8 改动文件清单（本域）

**Rust — tauri-app**
- `crates/tauri-app/src/card_shell_cache.rs`（M-02：`parse_http_url`/`host_of`/`is_url_allowed`/`validate_fetch_url` + 2 条测试）
- `crates/tauri-app/src/shell_doc_protocol.rs`（M-01：CSP `connect-src` + 4 条守卫测试）
- `crates/tauri-app/src/commands/card_shell.rs`（T-03 async+spawn_blocking、T-05 结构化错误、T-09 保留+标注）
- `crates/tauri-app/src/commands/plugins.rs`（M-08 `prompt_mutation_denial` + 契约注释、M-31 `plugin_get_variable` 按 `key` 过滤）
- `crates/tauri-app/src/commands/meta_typed.rs`（M-03 绑定校验/列表过滤、M-11 fail closed、M-27d/e routing 序列化）
- `crates/tauri-app/src/lib_tests_meta.rs`（M-03/M-11/M-08 测试 + 7 处 `TypedPatch` 字面量补 `campaign_id`）
- `crates/tauri-app/src/lib_tests_import_export.rs`（M-06 附带：真实卡夹具默认路径 + 守卫测试）

**Rust — 其他 crate（本域范围）**
- `crates/app-meta/src/typed_patch.rs`、`meta_conversation.rs`、`prompts/mvu_analyzer.rs`、`lib.rs`（S2：M-03/M-09/M-12/M-13/M-17）
- `crates/infra-import/src/**`（S1：M-06/M-32）
- `crates/infra-plugin-host/src/**`（M-31 相关，如无改动以实际 diff 为准）

**Frontend（域6）**
- 新增：`frontend/src/utils/mvuExecuteResult.js`、`frontend/tests/mvu-execute-result.test.mjs`
- 改动：`components/MvuJsRuntime.vue`、`components/TavernHelperRuntime.vue`、`components/CardShellHost.vue`、`components/PluginHost.vue`、`components-v2/meta/**`、`utils/{shellVariableOutbox,mvuInteractions,mvuStatTree,metaPanelFlow,promptHooks,promptHookAudit,cardShell*}.js` 及对应 `frontend/tests/*.test.mjs`
- **本域最终 diff 以 `git status --porcelain` + `git diff -- <路径>` 为准，本节清单在 S1/S2/S3 报告齐后核对**（§10）。

---

## 9 接管说明（为什么无冲突）

- **`crates/app-meta/**`**：审查报告 §M-03/M-09/M-12/M-13/M-17 的落点，但 task-8(域1)/task-9(域2)/task-10(域3)/task-11(域4)/task-12(域5) 的写作用域都不含它在内的任何路径。经 Lead 批准由本任务（task-13）接管，属"无 owner 文件"。
- **`crates/tauri-app/src/{card_shell_cache.rs,shell_doc_protocol.rs}`、`commands/{card_shell,plugins,meta,meta_typed}.rs`**：与 task-11（域4）在 task-13 的 write_scope 摘要里存在重叠告警，已与 `review-tauri-api` **按文件确认划分**：本域只写上述 6 个文件（其中 `commands/meta*.rs` 与 `commands/plugins.rs` 归域6），域4 写其余 `commands/**` + `lib.rs` 注册表 + `error.rs` + `tests/**` + `scripts/architecture/**`；本域**不新建** `crates/tauri-app/tests/**`。S2 对 `commands/meta.rs` 的改动在本域文件内，无并发写者。
- **frontend 域6 文件**：与 task-12（域5）按文件清单互斥（域5 已明确回复"不会碰"清单），`utils/cardShell*.js` 下 `cardShellCsp.js` 的 CSP 镜像由本域改（域5 已确认不碰 `tauri-api.js` 之外的该类文件）。
- **`crates/tauri-app/src/card_studio_api.rs`**：属域4，本域未改（T-13/T-14 的 19 命令定位问题由域4处理）。

---

## 10 子代理 / 跨域台账

> 本域内派出的 4 个子代理（S1 infra-import、S2 app-meta、S3 frontend card-shell/plugin、S4 frontend MVU/meta）与 4 项跨域委派（域1 M-05/M-26、域2 M-16 + sqlite M-03 验收、域3 M-04 Rust 落点、域5 M-18/M-25 + M-08 前端传参）在此登记。子代理最终报告要点在门禁收敛后补入。

<!-- DELEGATION-TABLE:START -->
| 承接方 | 条目 | 状态与证据 | 回执 |
|---|---|---|---|
| S1 子代理 → `crates/infra-import/**` | M-06.1-.5、M-32.6/.7 | **已修复**：`compat.rs` 新增 `WireDrift`/`Normalized`（旧无条件 `Intentional` 行删除）、`extra` 对账、导出 PNG wire 腿接入既有矩阵（无平行 harness）、`st_v3_large_worldbook.json` 新 entry；`png.rs` v3⇒`chara_card_v3`、`UnsupportedFormat` 真实构造点；默认夹具路径改 `data/local/test-card.png`（守卫测试 + `--ignored` 实跑通过）。门禁：check exit 0；69 passed/1 ignored。**复核补充**：旧 `compat.rs:690` 的 Intentional 行是**无条件** push，比评审描述更盲；**M-05 已由域1 修好**，S1 把 `#[ignore]` 改成常驻 live 闸门 `exported_wire_position_never_replaces_a_string_label_with_a_number` | 已完成 |
| S1 → 跨域 | M-32.8（`domain/character.rs` raw_card_json 空回退）、M-32.9（`domain/world_info.rs` order/extra.insertion_order 双来源） | **已由域1 修复（task-8 收口），域6 记录不再计为暂缓**：M-32.8 新增 `to_st_data_with_parse_diagnostic`（失败时在 `character.extensions` 覆盖**之后**打 `storyforge_raw_card_json_parse_failed` [+ 可选 `_parse_error`]，`to_st_data_from_card` 同步处理）；M-32.9 采用**同步**而非剔除——`to_st_entry` 以 `self.order` 为唯一权威、当且仅当 `extra` 已存在 `insertion_order` 时同步其值，V2 条目不发明该键。**口径已双方确认**：剔除会让 wire 对账把每个 V3 条目的 `insertion_order` 判成缺失（假 Loss 噪声），同步只在真实分歧时产生一次值差异（正是应暴露的信号）；M-32.8 的标记键视为**期望信号**（提示源卡解析失败、导出不完整），不要求对账腿把它记为漂移。域1 测试：`raw_card_json_parse_failure_is_visible_in_export`、`successful_raw_card_json_parse_adds_no_marker`、`to_st_data_from_card_also_marks_parse_failure`、`to_st_entry_syncs_preserved_insertion_order_with_authoritative_order`、`to_st_entry_does_not_invent_insertion_order_for_v2_entries`、`to_st_entry_ignores_out_of_range_insertion_order_key`。**域6 复跑**：`cargo test -p storyforge-infra-import` → exit 0，69 passed / 0 failed / 1 ignored（改动未把 wire leg 弄红） | 已核实 |
| S2 子代理 → `crates/app-meta/**` | M-03/M-09/M-12/M-13/M-17 | **已修复**：`stamp_campaign_scope` 唯一盖章点（4 个 health 分支 + action builder，盖章 campaign_id + revision）；`validate_variable_write_key`（先 `normalize_mvu_key` 再判空键 + 大小写不敏感 `__storyforge*`，可拦 `stat_data.__storyforge_x`），propose/preview/accept 共用；`execute_action` 越界/缺 context 一律 `ExecutionFailed`；`body_looks_like_variable_tree` 取代注释字符串判据。门禁：`cargo test -p storyforge-app-meta` 129 passed。**降级点**：`Create{target:"world_info"}` 缺 context 仍静默 no-op（评审只点 Update/Delete）。**复核更正**：M-17 原文"注释写 `状态栏数据` 的条目仍按 4K 截断"不准确——该条过不了 `is_var_entry` 收录门（整条被跳过）；`[变量初始化]`/`变量树` 部分与原文一致。收录门放宽未做（需 Lead 裁决） | 已完成 |
| S3 子代理 → frontend 卡壳/插件 | M-07/M-19/M-20/M-22/M-24/M-31 | **已修复（M-20 已闭环）**：M-07 `isTavernHelperFrame` 沿 parent 链归属 + `bridgeReplyOrigin` 钉死 origin（含 `waitForBridgeEvent`）；M-19 `trustedInlineShellDocStarts` 逐文档 ordinal 配对（替代消息级粗判）；M-20 `PluginPanel` emit `plugins-changed`，`AppV2.vue:1003-1008` 已由域5 接线（1 行）；M-22 `SENSITIVE_EVENT_FIELDS` 增 `fulltext`/`warnings`/`reason`（证据 `writing.rs:394-405`）；M-24 每文档一次性握手 nonce + 自导航撤信（刻意用普通变量避免 computed/watch 环）；M-31a `FORBID_TAGS:['style']`、M-31b 存储配额（单键 256KiB/单插件 1MiB）、M-31c `chat.save` 移到权限门之后并要求 `WriteChat`。**测试**：新增 2 个 vitest 文件 + 扩展 4 个（vitest 层需 Lead 门禁；其中 `plugin-bridge`/`card-shell-display` 的 node:test 部分我已直跑 90/90 绿） | 已完成 |
| S4 子代理 → frontend MVU/meta | M-04 前端镜像、M-28a/b/c、M-27c、M-21a/b、M-29 | **已修复**：`mvuExecuteResult.js` 保留位过滤；空键拒绝（含 `campaign:` 空后缀）；全角/数学运算符拒绝；`mvuStatTree` 优先级确定化；`metaPanelFlow` preview 失败 `_stale=true`；`validateHookedMessages` + 整链 15s 预算；M-29 策略函数单一来源。**review-frontend 直跑发现 3 处真实红项（我本轮修掉）**：① M-21a 会把"仅 messages 非法"的整份 payload 丢掉（首轮 `messages: []` + 插件改 intent 全丢）→ 改为只回退 messages 字段、其它字段保留、每 phase 仍只发一条审计；② M-21b 墙钟预算判定靠 `Date.now()` 取整而不稳定 → 新增 `timeoutClampedByBudget`/`chainBudgetExhausted` 确定性判定；③ M-31b 测试算术错误（2×256KiB 未越 1MiB 上限）→ 改为 5 次写入才断言超限 | 已完成 |
| 跨域：域1 `review-domain` | M-05 / M-26 | M-05 **已修复**（`world_info.rs:288 position_for_export`，我域 live 闸门验收）；M-26 由域1 处理。另 M-32.8/.9 已转交 | 回执已收 |
| 跨域：域2 `review-storage` | M-16、M-03 的 sqlite 验收用例 | **两层均已落地（域2 回执）**：① M-16 `backend_workflows.rs::preview_mvu_apply_for_backend` 改为 `normalize_schema_keys(...)` 归一化后再预览，与 `meta_apply_mvu_schema` 应用边界同口径；② M-03 新增 `crates/tauri-app/tests/sqlite_meta_campaign_binding.rs`（进程级 activate 限制 ⇒ 单 test）：证明无 target id 的 action 在 SQLite 仓层只认入参 campaign ⇒ **命令层绑定校验是 SQLite 上唯一归属防线**、`expected_revision` 与目标 revision 不一致时跨战役 apply 被拒且目标未写脏、同 patch 在自己 campaign 上正常落库、带 target id 的 action 仍被 target 作用域查找拒绝。**边界（域2 明确声明）**：命令层 4 条断言（accept 拒绝 / preview stale / B 未写脏 / A 仍可接受）在 `tests/*.rs` **不可达**（`meta_accept_typed_patch_with_writer` 等为 `pub(crate)` 且需 `AppState`，`tauri` 未启用 `test` feature）⇒ 其落点是 crate 内 `src/lib_tests_meta.rs::test_meta_typed_patch_is_bound_to_its_campaign`（本域维护，已绿） | 已核实 |
| 跨域：域3 `review-pipeline` | M-04 Rust 落点 | **已落地（域3 回执）**：`app-pipeline/lib.rs:4017 push_mvu_js_variable_updates`（`is_reserved_mvu_key` = trim + `to_ascii_lowercase()` 后 `starts_with("__storyforge")` → 跳过并计 skipped；其余经 `normalize_mvu_key` 归一）+ W-25 作用域别名 `scoped_variable_key`/`parse_scoped_variable_key`。测试名：`mvu_js_fallback_drops_reserved_namespace_keys`、`mvu_js_scoped_key_targets_instance`；门禁 `cargo test -p storyforge-app-pipeline --lib` 138 passed / 0 failed。本域记录不据为己有 | 已核实 |
| 跨域：域5 `review-frontend` | M-18、M-25、M-08 前端传参、M-20 接线 | M-08 前端**已落地**（`appliedPluginIds` 4 行增量 + `modifierPluginIds` 透传，review-frontend 报告其自身 hunk 未参与校验/丢弃判定）；M-20 **已接线**；M-18 判**暂缓**（域5 记录 + 提议）；M-25 暂缓 | 回执已收 |
| 跨域：域7 `review-goals` | `scripts/run-real-card-smoke.ps1` 默认夹具路径 | **已修**：默认改 `data/local/test-card.png` + 双候选解析 + 错误文案不复述绝对路径；复跑 Pester 198 项全绿。文档不一致（`docs/RELEASE-CHECKLIST.md:72,136`、`docs/PLAN-ST-IMPORT-EXPORT.md:127`）已由域7 转交 task-16 | 回执已收 |
<!-- DELEGATION-TABLE:END -->

---

## 11 需文档同步条目（交给 task-f7 / task-16）

1. **M-01 的"已知风险"表述**：`docs/ARCHITECTURE.md`/`RELEASE-STATUS.md` 若有"壳隔离/CSP 已闭合"类表述，需改为"V5 CSP 隔离已落地，但 Windows 上子帧仍可达 Tauri IPC（M-01 未修复，卡在 wry/WebView2 主帧注入语义）"，并指向本文件的 PoC 步骤。
2. **JSON accept 原子性（M-10）**：文档若宣称 accept "nothing is partially applied"，需改成"preflight 失败不落盘；写盘中途 IO 失败可能半提交（待 UoW 化）"。
3. **审计链（M-23）**：文档若宣称插件审计链是生效的安全控制，需改为"链式审计 API 存在但未接入链路；当前审计仅内存环、不落盘、不构成门禁"。
4. **`card_shell_allow_host`（T-09）**：CLAUDE.md 若列它，补"保留 API、无前端入口；在 M-01 未修复前是提权放大面"。
5. **M-04 作用域语义**：MVU JS fallback 的变量一律落 campaign（M-18 未修前），文档里的"变量写入作用域"描述需与此一致。
6. **M-08 前端契约**：`pluginPromptHookResult(requestId, messages, error, pluginId?, modifierPluginIds?)` 需写入 AGENT_INTERFACES/插件桥文档；并注明"未声明改写者 ⇒ 后端丢弃 messages"。
7. **T-03/T-05**：`card_shell_fetch_url` 已 async；`card_shell_register_doc/module` 现在返回结构化错误 DTO（前端 `errorText` 展示）。

---

## 12 遗留与阻塞

1. **M-01 主体未修**（P0，最高优先）：需 Windows 运行时 PoC + 产品决策（3 个候选方案见 §3）。本轮交付的是"可验证的收敛 + 守卫 + 否决证据"。
2. **M-14 / M-10 / M-23 暂缓**：均需跨文件写作用域或锁序/落盘设计，理由与补丁方向已写。
3. **前端传参依赖**：M-08 需要域5 在 `usePluginBridge.js`/`tauri-api.js` 传 `modifierPluginIds`，在此之前插件改 prompt 功能安全降级为不改写。
4. **跨域验收**：M-03 的 sqlite 用例（域2）、M-04 的 `app-pipeline` 过滤（域3）、M-16（域2）、M-05/M-26（域1）完成后需 Lead 在 task-15 全量门禁复核。
5. **并发编辑导致的编译噪音**：本轮多次 `cargo check` 被其他域在途改动打断（`app-agent/postprocess.rs`、`app-pipeline/lib.rs`、`domain/card_studio.rs`、`tauri-app/{turn_store,storage_backend}.rs`），已逐次同步给对应 owner；最终门禁以 Lead 的干净树运行为准。
6. **M-17 收录门：暂缓（需产品决策）** —— Lead 裁决"不放宽"，理由：会改变**所有卡**的 MVU 分析语义与提示词预算，属产品决策而非缺陷修复。
   - 证据更正（本轮复核）：原评审写"注释写成 `状态栏数据` 且承载变量树的条目仍按 4K 截断"，实测**不成立**——该注释不含 `initvar`/`mvu`/`变量`，过不了 `is_var_entry` 收录门（`mvu_analyzer.rs:294-301`），是**整条被跳过**而不是 4K 截断；`[变量初始化]`/`变量树` 含 `变量`，能进区块，确实被旧 4K 逻辑截断（这部分与原文一致）。已修的只是**额度门**（正文形态启发式），收录门保持原样。
   - 候选 A（放宽收录门）：把"含变量树正文"作为收录条件之一 → 代价：所有卡的 MVU 分析区块内容变化（新增条目进 prompt），提示词预算被稀释、可能影响既有卡的分析稳定性，需跨模型回归。
   - 候选 B（维持现状 + 提示用户）：保留 `is_var_entry` 现状，在分析器提示词/UI 里显式告知"未被收录的条目（如 `状态栏数据` 这类注释不含变量的条目）不会参与 schema 推导" → 代价：用户需自行改注释或改用 `[InitVar]`，属可发现的可用性损失而非静默错误。
   - 该条在**第二遍复检（R4）**中会被重新核对（本轮只交付额度门修复 + 证据更正）。
7. **域2 编译红（已消解）**：task-25 复跑时 `cargo test -p storyforge --lib` 曾被域2 文件 `crates/tauri-app/src/turn_lifecycle.rs:1729 no field 'data_dir'`（字段被重命名为 `_data_dir`）挡住（`--all-targets` 与 `-p storyforge-app-meta` 当时不受影响）；收到 Lead 要求后域2 已修，本域随后复跑 `cargo check -p storyforge --all-targets` → **exit 0**。此处保留时间点事实以便对账。

## 13 task-27：2 个新增前端测试在真实 vitest 下失败（原因 / 判定 / 改法 / 验证状态）

Lead 收口门禁（`cd frontend && npm.cmd run test:ui`，日志 `artifacts/review-2026-09-13-round2/fe-vitest.log`）报 `Test Files 2 failed | 29 passed (31)`、`Tests 2 failed | 139 passed (141)`。

| # | 文件 / 用例 | 真实 runner 现象 | 判定 | 改法 |
|---|---|---|---|---|
| 1 | `tests/components-v2/plugin-host-handshake.test.mjs`（整个 suite 失败） | `TypeError: The URL must be of scheme file` | **测试自身的环境问题，与 M-24 被测代码无关**：测试用自己的 `fs.readFileSync(new URL('../../src/components/PluginHost.vue', import.meta.url))` 读 SFC 源码；真实 vitest 经 Vite 转换后 `import.meta.url` **不是 `file:` scheme**，`fs` 因此拒绝该 URL。域6 的 `node:test` 直跑与域5 的 shim harness 都跑不到该文件（前者 import `vitest`、后者不支持 `vi.mock`），所以此前未被发现 | 新增 `resolvePluginHostSource()`：`file:` scheme 时走 `fileURLToPath`；否则按 cwd 解析（`npm run test:ui` 的 cwd = `frontend/`）并保留 `frontend/` 兜底候选，全部找不到时**显式抛出候选清单**（不静默跳过）。断言一条未删、未放宽 |
| 2 | `tests/components-v2/tavern-helper-runtime-bridge.test.mjs` 两条正例（`still accepts bridge messages from its own iframe`、`accepts messages from a nested frame inside the TH shell (parent chain)`） | `expected 'TavernHelper无脚本重新执行' to contain 'card shell fetch requires Tauri host'` | **测试探针环境不合，被测的 M-07 帧归属逻辑没错**：正例用"无 Tauri 时 `cardShellFetchUrl` 的确定性错误串"当受理证据；真实 vitest + happy-dom 下组件走的是"无脚本重新执行"分支，状态条显示 `TavernHelper无脚本重新执行`（同样只在**受理之后**出现） | 引入 `ACCEPT_MARKERS = ['card shell fetch requires Tauri host', 'TavernHelper无脚本重新执行']`：两条**正例断言"任一出现"**（仍要求出现受理副产物，未放宽为"不报错即通过"）；两条**反例（伪造帧 / 无 source）断言"两者都不出现"**，比原先只查 `ERROR_TEXT` **更严**。M-07 的回归锁保持不变：伪造帧在任何环境下都不得产生任何受理副产物 |

**判定依据（非猜测）**：① 失败 1 的堆栈指向 `fs` 而非组件代码，且同文件其余用例（`isPluginBridgeHandshakeValid` fail-closed、令牌唯一性、`pluginIframeTargetOrigin` 前缀反例）都依赖同一 `loadPluginHostHelpers()`，在域5 harness 下这些 helper 单测是通过的 ⇒ 问题在"读文件的方式"而非被测逻辑；② 失败 2 的 `actual` 本身就是组件的**受理路径**产物（若消息被丢弃，状态条不会出现该文案），与两条反例在真实 runner 下**通过**互相印证 ⇒ 帧归属判定未被破坏。

**验证状态（不声称已绿）**：`node --check` 两个文件 exit 0；`npx vitest run tests/components-v2/shell-aware-content.test.mjs` 在本会话同样 **EPERM（syscall spawn）**，无法自证 ⇒ **需 Lead 收口门禁复跑 `npm run test:ui` 确认**（预期 `Test Files 31 passed`、`Tests 141 passed`）。若仍有红，请把日志行发我，我按同样口径继续定位。

## 14 task-29：M-24 握手时序（**真产品缺陷**）+ M-07 反例假阳性探针

### 14.1 `frontend/src/components/PluginHost.vue` —— 真产品缺陷（插件桥整体失效）

**根因（Lead 定位 + 我复核确认）**：`iframeDoc`（computed）把 `handshakeToken` 嵌进文档，而 `watch(iframeDoc, …)` 回调里**先用已求值的文档参数**、**之后**才 `newPluginHandshakeToken()`。回调从不在换令牌后重读 computed，因此：

1. **注册给帧的文档永远带上一轮令牌**（首次挂载是空串 `handshake: ""`）；
2. `newPluginHandshakeToken()` 里 `handshakeDocumentSequence += 1` 是 computed 的手工依赖（`void handshakeDocumentSequence`），换令牌 → computed 变脏 → watch 再触发 → **自激重注册循环**，每轮都注册"令牌落后一轮"的文档；
3. 后果：桥脚本用 T<sub>n-1</sub> 发 `sf:ready`，宿主 handler 期望 T<sub>n</sub> ⇒ **握手永不成功 ⇒ 插件桥整体失效，所有插件消息被丢**（`expected null to be truthy` / 文档不匹配 `handshake: "sfh_…"` 的表现）。

**判定：真产品缺陷，不是测试问题。** 新写的 M-24 安全测试正是因为「断言注册文档里的令牌形状」才发现它。

**改法（打破环：令牌先于文档组装）**：
- 抽出纯函数 `composeShellDoc(plugin, token)`（原 computed 的 body 原样搬入：`DOMPurify.sanitize(ADD_TAGS:['script'])` → `generateBridgeScript(id, HOST_ORIGIN, token)` → `<!DOCTYPE html>…` 顺序与字节结构不变），令牌**由调用方传入**；
- watch 源改为与令牌无关的 `handshakeSourceKey`（只依赖 `props.plugin.id` + `entry_html`，两者都用 `\u0000` 分隔；无正文 ⇒ `''`），**不再 watch computed**；
- 回调顺序改为 `const token = newPluginHandshakeToken(); const doc = key ? composeShellDoc(props.plugin, token) : '';` → 再走原有释放旧 URL / `registerShellDoc` / blob 兜底 / `onIframeLoad` 撤信逻辑；
- 令牌不再是 watch 源的响应式依赖 ⇒ 换令牌不会反向触发注册；**每次源变化恰好注册一次**；
- 删掉原来那段错误注释（"watch 先换令牌、再由同一次 watch 求值 iframeDoc，单向无环"），改为如实说明新不变量与旧实现的两个缺陷。
- 对外行为保持：`iframeSrc`/`:src`、blob 兜底、释放旧 URL、`onIframeLoad` 撤销信任、`handshakeValid`、`pluginTargetOrigin` 均未改。

**新增不变量测试**：`plugin-host-handshake.test.mjs` → `registers exactly once per source change and embeds the token it later accepts (M-24 invariant)`：初挂载 `registerDocCalls.length === 1`；改 `entry_html` 后恰好 1 次新注册且新文档令牌 ≠ 旧令牌；**旧令牌发 `sf:ready` 不放行**（`.plugin-slot-content` 不存在），**新文档里的令牌发 `sf:ready` 放行**（渲染出 `fresh-token`）。

### 14.2 `tavern-helper-runtime-bridge.test.mjs` —— 上一轮我选错了受理标记（假阳性）

**根因**：我上一轮把 `TavernHelper无脚本重新执行` 当作"受理副产物"。**它不是**——"没有脚本可重新执行"的常态（初始/空态）本来就会显示它，所以伪造消息被丢掉后文案照样存在 ⇒ 两条反例必然失败。这是我上一轮的判断错误，Lead 的复跑把它暴露出来。

**判定：测试探针错误（我的错），M-07 帧归属逻辑本身未发现缺陷。**

**改法（换成宿主对请求方帧的出站回复）**：受理路径里宿主会对 `ev.source` 回 `postMessage({ __sf_th_bridge_res: <id>, result, error })`（`TavernHelperRuntime.vue:749-755`），这是**只在受理后**出现的副产物。新增 `captureReplies(windowLike)`：可控 window-like 对象直接替换 `postMessage` 记录；真实帧若不可写（跨源 WindowProxy）则退化为监听该帧收到的 `message` 事件。四条用例改为：
- 反例 1（伪造帧）：`rogue = { name:'rogue-frame' }` → 断言 `rogueReplies` **为空**、且**未回复该请求 id**（另保留 `'passwd'` 不出现）；
- 反例 2（无 source 的合成事件）：监听**真实帧**收到的消息 → 断言**未回复**该 id（若误受理会回给自己的帧）；
- 正例 1（本帧）/ 正例 2（壳内嵌套帧，`parent` 链）：断言**收到了** `__sf_th_bridge_res === 'forged-1'` 的回复。
未使用 skip，未放宽成恒真：反例仍是"副产物必须不出现"，正例仍是"必须出现"。

**验证状态（不声称已绿）**：`node --check` 两文件 exit 0；`PluginHost.vue` 的 `<script>` 块抽取后 `vm.Script` 解析通过；本会话 `npx vitest run …` 仍 **EPERM（syscall spawn）**，无法自证 ⇒ **需 Lead 收口门禁复跑 `npm run test:ui` 确认**。§13 中我上一轮"判定为测试问题"的第 2 条已在 §14.2 自我更正（错误在我选的标记，而非组件逻辑）。

## 15 Lead 收口结果 + 两处收口补充修复 + happy-dom/DOMPurify 环境事实

### 15.1 真实门禁结果（Lead 收口复跑）

`cd frontend && npm.cmd run test:ui` → **31/31 文件通过，exit 0**（日志 `artifacts/review-2026-09-13-round2/fe-vitest.log`）。§14 的修复被证实有效：M-24 令牌时序（`embeds a fresh per-document handshake token…`、`ignores every message until the current document completes the handshake`）、M-07 两条反例（改用出站回复作受理副产物）、以及大部分握手用例均通过。**§13 中"需 Lead 收口复跑确认"的三条到此闭环。**

### 15.2 Lead 收口补充修复（记为「Lead 收口修复」，非本域成员产物）

| # | 位置 | 问题 | 收口改法 | 覆盖测试 |
|---|---|---|---|---|
| 1 | `frontend/src/components/PluginHost.vue`（`onIframeLoad`） | **自导航后旧令牌可重放**：原实现只重置 `handshakeValid`，`handshakeToken` 不变 ⇒ 帧自导航后重放旧令牌仍能通过 `isPluginBridgeHandshakeValid`（这正是我 task-29 修完后仍红的那条 `revokes trust on frame self-navigation…`） | 新增 `expectedFrameLoad` 标记：宿主自己注册文档时置位，该次 load 消费掉；**任何额外 load ⇒ `newPluginHandshakeToken()` 轮换令牌** ⇒ 自导航后的新文档永不取得信任。关键约束：**不能无条件在 load 里轮换**——初次加载也走 load，那样会把正常握手一起打掉 | `revokes trust on frame self-navigation and never trusts a WindowProxy again` |
| 2 | `frontend/src/components/PluginHost.vue`（slot 消毒） | **M-31a `<style>` 正文残留**：`FORBID_TAGS: ['style']` 走 DOMPurify 的 `KEEP_CONTENT` 语义——**元素删了但 CSS 正文留成裸文本**（实测 `FORBID_CONTENTS: ['style']` 在 3.4.12 + happy-dom 下无效），CSS 文本经 `v-html` 插进宿主 DOM 会显示出来 | 新增 `stripStyleElementsFromSlotHtml(raw)`：sanitize **之前**用 `/<style\b[^>]*>[\s\S]*?<\/style\s*>/gi` 整块删除；无 `<style` 时逐字节不变；`FORBID_TAGS`/`FORBID_CONTENTS` 保留作纵深防御 | `strips <style> from slot HTML…`（控制组改为断言纯函数精确输出） |

### 15.3 环境事实（Lead 实测，写下来避免后人误判）

vitest 的 **happy-dom** 环境下 **DOMPurify 3.4.12 的默认白名单解析为空**：

- `DOMPurify.sanitize('<p>hi</p>') === 'hi'`
- `DOMPurify.sanitize('<span>hi</span>') === 'hi'`
- `DOMPurify.sanitize('<button …>ok</button>') === 'ok'`
- 且 `DOMPurify.isSupported === true` —— **`isSupported` 为真不代表白名单可用**。

推论（对本域及后续 reviewer 都适用）：
1. M-31a 用例里"`<style>` 不存在"在 happy-dom 下**部分靠环境**成立（元素与正文一起被清掉是环境副作用），真正钉住行为的是 `stripStyleElementsFromSlotHtml` 的**纯函数单元断言**（输入含 style ⇒ 精确等于删除后的字符串；输入不含 style ⇒ 逐字节不变）。
2. **任何"元素/属性会被 DOM 保留"的断言在 happy-dom 下都不可靠**（会被误判为"被剥除"），必须打在纯函数上，或在真 Chromium 里验证。
3. 本条同样解释了为什么 §5 的 M-31a 断言在本域早期的单进程 harness 下"看起来通过"却掩盖了 CSS 正文残留——**测试环境的解析能力差异必须写进断言设计**。


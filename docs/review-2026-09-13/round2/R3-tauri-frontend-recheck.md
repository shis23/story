# R3 复检：域4（Tauri 命令层与契约）+ 域5（前端通用层）第一轮修复

- **复检人**：`review-storage`（域2 owner，本轮做跨域 R3 复检）
- **任务**：task-20（revision 2），blocked_by task-15/task-16（均已 ready）
- **复检输入**：`docs/review-2026-09-13/fixes/04-tauri-fixes.md`（541 行）、`docs/review-2026-09-13/fixes/05-frontend-fixes.md`（471 行）、`git diff -- crates/tauri-app frontend scripts .github`
- **代码冻结**：本轮**唯一写入 = 本文件**。所有"负控/破坏性验证"都在 `%TEMP%\sf-r3-negctl` 的一次性影印副本里做，仓库工作树未新增/删除任何文件（含 `--write-snapshot`，只在副本里跑）。
- **判定词**：`通过(独立复现)` / `通过(仅静态核对)` / `未复验(引用门禁)` / `新发现`

---

## 0. 验证口径与"未由本成员运行"的项

| 项 | 本成员状态 | 证据 |
| --- | --- | --- |
| `node scripts/architecture/backend-baseline.mjs` | **已运行**（仓库 + 影印副本各 1 次，结果一致） | 定义 175 / 注册 175 / 前端唯一 invoke 152 / 孤儿 23 / exit 0 |
| 负控 3 组（注入前端假调用、注入保留声明过期、`--write-snapshot` 拒绝写） | **已运行**（全部在影印副本内） | §2.2–§2.4 |
| `node frontend/tests/tauri-command-contract.test.mjs`（`node:test`，非 npm） | **已运行** | 11 tests / 11 pass / 0 fail / exit 0 |
| 契约不变量集合等式（导出函数级独立计算） | **已运行** | §3.1 |
| `cargo test -p storyforge --lib card_studio` | **已运行** | 8 passed / 0 failed / 465 filtered out / exit 0 |
| 删除面全树引用检索（20 wrapper + 9 基元 + 动态引用） | **已运行**（node 脚本 + 全文扫描） | §7 |
| `npm test`（532）/ `npm run test:ui`（vitest 31 文件 / 151 用例）/ `npm run build` | **未由本成员运行** | Lead 指令：不要跑 npm；且沙箱内 npm/vitest 会 `spawn EPERM`。以 `docs/review-2026-09-13/fixes/GATE-REPORT.md` 的 Lead 门禁数字为准（npm test 532/532 全绿、vitest 31 文件/151 全绿、build exit 0） |
| `cargo test --workspace` 全量 | **未由本成员运行**（Lead 指令：不跑 workspace 级 cargo） | 同上 GATE-REPORT（99 套 / 2165 用例 / 0 failed / 33 ignored） |

**"A′ 等价 harness"的可复现性提示**（不影响结论，见 N-R3-05）：`05-frontend-fixes.md §6.1.1` 的 A′ 结果是仓库外 `%TEMP%` 一次性脚本跑的，脚本未入库；本成员用 `node frontend/tests/components-v2/composer-ime.test.mjs` 直接跑会得到 `ERR_UNKNOWN_FILE_EXTENSION: .vue`（该文件 import vitest + SFC，只能在 vitest 下跑）。因此本轮对 vitest 用例的结论**只认 Lead 的 vitest 门禁**。

---

## 1. 逐条复检结论

### 1.1 域4（`04-tauri-fixes.md`）

| 条目 | 记录状态 | R3 判定 | 依据 |
| --- | --- | --- | --- |
| **T-01** 契约扫描漏 3 个裸 `_invoke`、魔数断言 | 已修复 | **通过(独立复现)** | §2（正向 + 2 组负控 + 快照双向） |
| **T-02** `card_studio_api.rs` 8 处 `let _ = store.update` | 已修复 | **通过(独立复现)** | §4 |
| **T-04/T-07/T-12** 行为契约变更（`log_clear` 返回 Result、`add_variant` provenance、3 个 export async） | 已修复 | **通过(仅静态核对)** | 记录与 `CLAUDE.md` 2026-09-13 补充事实一致；签名/调用点由 Lead 的 cargo 门禁覆盖 |
| **T-06/T-08/T-13/T-16** 命令位置白名单、`defined == registered`、越界门禁 | 已修复 | **通过(独立复现)** | `commandsOutsideAllowedLocations == []`；实测位置分布 `commands/*.rs` **156** + 根 `card_studio_api.rs` **19** = 175，与 `COMMAND_LOCATION_ALLOWLIST`（19 条）完全吻合 |
| **T-10** `--write-snapshot` 再生成入口 | 未运行时验证 | **通过(独立复现)** | §2.3/§2.4：脏状态拒绝写且快照字节不变；干净状态写入 175 条且与已入库快照 **SHA256 相同** |
| **T-11 / T-15** 零入口命令"保留 + 声明" | 已修复(降级) | **通过(独立复现)** | §8：23/23 有 `reason`，23/23 `file` 与实际定义位置一致，`undeclaredOrphanCommands == []`、`staleRetainedDeclarations == []` |
| T-03 / T-05 / T-09 / T-14（移交域6 / task-16） | 移交 | 非本轮范围 | 由对应域复检 |
| N-01（注释计数漂移） | 已修复 | **通过(独立复现)** | `extractCommandAttributes` 已改锚定行正则（`backend-baseline.mjs:60`）；探针见 §3.3，`//`、`///` 注释不再计数 |
| N-02（20 条零入口命令保留+声明） | 暂缓 | **通过(独立复现)** | 同 T-11/T-15 |

### 1.2 域5（`05-frontend-fixes.md`）

R3 按 Lead 点名口径复检 **F-01 / F-02 / F-30（删除面）**，并对"删除类"修复做**全树回归检索**；其余 40+ 条（F-03…F-47）未由本成员逐条重验，其可运行证据统一引用 GATE-REPORT 的 `npm test 532/532` + `vitest 31/151`。

| 条目 | 记录状态 | R3 判定 | 依据 |
| --- | --- | --- | --- |
| **F-01** IME 组合态守卫 | 已修复 | **通过(独立复现，静态 + 用例存在性)**；**真机 IME 未验证** | §5：三重守卫在源码中确实存在且裸 Enter 仍提交；6 例回归测试文件存在并 import vitest（本成员无法在无 npm 环境下执行）；中文输入法**真机**行为未由任何人验证（记录亦未声称） |
| **F-02**「采纳」失败静默 | 已修复 | **通过(独立复现)** | §6：失败分支 `console.error` + `alertDialog('采纳失败: ' + errorText(e))`；用户主动取消（强制采纳确认框点取消）刻意不报错，语义正确 |
| **F-16/F-17/F-18** 删除 9 个基元 | 已修复(删除) | **通过(独立复现)** | §7.2：9 个文件都不存在，`frontend/src`+`tests` 全树 **0** 处引用（含裸标签/字符串/`:is` 动态名） |
| **F-30** 删 20 个零引用 wrapper、保留并接线 `cardstudioListStages` | 已修复 | **通过(独立复现)** | §7.1：20 个名字在 `frontend/src`+`tests` 全树 **0** 处引用；`cardstudioListStages` 在 `tauri-api.js` 与 `CardStudio.vue` 都被调用 |
| **F-11/F-12/F-19/F-25/F-26** 等"失败必须可见"类修复 | 已修复/降级 | **未复验(引用门禁)** | 无独立复现手段（需 vitest/DOM 或真机） |
| F-03…F-47 其余条目 | 混合 | **未复验(引用门禁)** | GATE-REPORT 的 532 + 151 全绿是这些条目的可运行证据 |

---

## 2. T-01 独立复现

### 2.1 正向：3 个 `card_shell_*` 确实进入 `invokedCommands`

用导出函数独立计算（不是读测试、也不只读脚本输出）：

```
missingBackendCommands   = []
undeclaredOrphanCommands = []
staleRetainedDeclarations = []
card_shell_register_doc:true  card_shell_register_module:true  card_shell_unregister_doc:true
uniqueInvokeCount = 152   len(invokedCommands) = 152   invokeCount(带重复) = 156
union(invokedCommands 152 ∪ retainedNoFrontendCaller 23) == registeredCommands 175  → true
```

调用点实证：`frontend/src/utils/shellDocUrl.js:64 / :83 / :105 / :120` 是**裸 `_invoke(...)`**（模块局部绑定，无点号）。新正则 `(?<![\w])_invoke\(`（`backend-baseline.mjs:87`）对裸形式与 `this._invoke` 形式都能命中，且 `\binvoke\(`（`:71`）不会与它重复计数（`_`/`i` 之间无词边界）——这一条我用探针逐形状验证过（§2.5）。

### 2.2 负控 1：故意造"前端调用不存在的后端命令"

在影印副本里新增 `frontend/src/R3NegativeControl.js`（内容：`export const r3Neg = () => _invoke('r3_negative_control_missing')`）；**仓库文件未改动**。

| 观测 | 结果 |
| --- | --- |
| `backend-baseline.mjs` 输出 | `前端唯一 invoke 153`（152 → 153），`门禁失败 1 项`，**exit 1** |
| 失败原因 | 命中 `frontend.missingBackendCommands`（`evaluateGates`，`backend-baseline.mjs:324-329`） |
| `node frontend/tests/tauri-command-contract.test.mjs` | **exit 1**，`AssertionError: Expected values to be strictly deep-equal`（`every frontend Tauri invoke is registered by the backend`） |

→ 结论：门禁与测试都能红，**不是"永远通过"的断言**；且注入用的是**裸 `_invoke`** 形态，等于顺带再次证明 T-01 的扫描修正真的在拦这类 live 调用点。

### 2.3 负控 1 续：`--write-snapshot` 在门禁失败时是否真拒绝写

| 观测 | 结果 |
| --- | --- |
| stderr | `[backend-baseline] --write-snapshot 已跳过：门禁未通过，拒绝把违规状态写进快照`（`:571-574`） |
| exit | 1 |
| `frontend/tests/fixtures/tauri-registered-commands.snapshot.json` | **写入前/后 SHA256 完全一致**（`14E0437C7CB5B976B8FB08C9065928761E5836324D3A3A66985E28E300DAEA62`） |

→ 拒绝写是真的（不是只打印提示）。**Lead 点名项已闭环。**

### 2.4 负控 3：清理注入后 `--write-snapshot` 正常写，且与已入库快照字节一致

删掉注入文件后：`已写入快照 175 条`、exit 0，产物 SHA256 = `14E0437C…DAEA62`，与仓库里已入库的 fixture **相同** → "快照 ↔ 代码"当前同步，再生成路径可用。

### 2.5 扫描器边界探针（内存注入，不落盘）

`extractFrontendInvokes`（源码字符串级）：

| 输入形状 | 结果 |
| --- | --- |
| `invoke('cmd_a', {})` / `this._invoke('cmd_b')` / 裸 `await _invoke('cmd_c')` | 命中（`cmd_a` / `cmd_b` / `cmd_c`） |
| `command: 'cmd_d'`（plugin-bridge 静态表） | 命中 |
| `slash_command: 'cmd_not_real'` | **不命中**（负向前瞻正确，避免斜杠命令误报） |
| `_invoke(commandName, {})`（纯动态拼接） | **不命中**（已知边界：全动态命令名无法静态核对；本次审计范围内未发现这种写法） |

---

## 3. 契约不变量与断言强度（"有没有被削弱成永远通过"）

### 3.1 不变量独立计算（不依赖测试文件）

`invokedCommands ∪ retainedNoFrontendCaller == registeredCommands`：**成立**（152 + 23 = 175，且去重后长度仍为 175）。等价的命名化断言也全绿：`missingBackendCommands == []`、`undeclaredOrphanCommands == []`、`staleRetainedDeclarations == []`、`orphanRegisteredCommands == retainedNoFrontendCaller`（23 条）。

数学上这条等式是 `注册 = 已接入口 ∪ 零入口` 的定义式展开；配合 `definedCommandCount == registeredCommandCount` 与位置白名单，覆盖了"静默遗留 + 静默消失 + 位置漂移"三类风险。**关键点：这条等式在"保留声明过期"时也会破（并集比注册集大），所以它确实是有牙齿的。**

### 3.2 `git diff` 断言强度审查（`frontend/tests/tauri-command-contract.test.mjs`）

`git diff` 显示：**只增不减**——删掉的唯一断言是硬编码 `assert.equal(baseline.frontend.uniqueInvokeCount, 169)`；新增 4 个测试：

1. 裸 `_invoke` 扫描回归锁（3 个 `card_shell_*` + `uniqueInvokeCount == invokedCommands.length` 自洽锁）；
2. `Gate 0 registry equals frontend reachable set ∪ declared retained API`（集合等式 + 3 条命名化诊断 + `staleRetainedDeclarations == []`）；
3. `Gate 0 keeps every command inside commands/*.rs or an explicit allowlist`（位置白名单 + `defined == registered`）；
4. 原有 `duplicateRegisteredCommands == []`、快照深比较、Gate 1/Gate 3 各条**未改动**。

→ **判定：断言强度净增强**，没有"把红改绿"式的削弱。旧魔数 `uniqueInvokeCount` 已改由"集合等式 + 孤儿/过期诊断 + 快照深比较"承担。

### 3.3 计数类断言现状与漂移面

| 断言 | 位置 | 判定 |
| --- | --- | --- |
| `commandAttributes == 175` | `tauri-command-contract.test.mjs:42` | **保留（有意）**，注释写明 175 是产品契约（README/CLAUDE.md）。残留漂移面见 **N-R3-04**：`extractCommandAttributes` 按"整行是属性"计数（`backend-baseline.mjs:60`），`/* */` 块注释内以 `#[tauri::command]` 开头的行、`#[cfg(test)] mod tests` 内的 mock 命令都会被计入 → 可能**无真实命令变化却变红**（fail-closed 的噪声，不是假绿） |
| `registeredCommandCount == 175` | 同上 `:43` | 同一性质；由快照深比较兜底 |
| `workspace.crateCount == 16`、`activeFlagReferences == 2`、`facadeSelectedWriterConstructors == 4` 等 Gate 3/4/5 计数 | 同上 `:80-102` | 属"阶段性事实"，跨 crate 重构会红；由 Lead 门禁覆盖，非本次范围 |
| 注释里的历史数字 | `backend-baseline.mjs:86`"修正后真实值 172"、测试文件 `:44`"169→172" | **新发现 N-R3-02**：实际已是 **152**（域5 删了 20 wrapper）。仅注释，不影响断言 |

---

## 4. T-02：`card_studio_api.rs` 落盘失败不再被吞

- `let _ = store.update(...)`：**全仓 0 处**（唯一命中是 `card_studio_api.rs:23` 的修复说明注释）。
- 结构（`crates/tauri-app/src/card_studio_api.rs`）：
  - `persist_project(...) -> Result<CardProject, TauriCommandError>` = `store.update(project).map_err(TauriCommandError::storage)`（`:28-32`）；
  - `persist_failure_state(...)`（`:37-40`）只在"**已经要返回错误**"的分支打日志；
  - 成功路径 3 处 `persist_project(store, project)?`（`:334 / :387 / :840`）；
  - 错误路径 5 处 `persist_failure_state`（`:512 / :600 / :605 / :736 / :744`）；
  - 另有 6 处直接 `store.update(...).map_err(TauriCommandError::storage)`（`:249 / :269 / :300 / :532 / :667 / :769`），同样向上传播。
- 测试：`cargo test -p storyforge --lib card_studio` → **8 passed / 0 failed / 465 filtered out / exit 0**，含 `persist_project_propagates_disk_failure_as_command_error`（`:973`）、`persist_project_returns_ok_when_disk_write_succeeds`（`:989`）、`update_reports_error_when_persist_fails`。

→ **通过(独立复现)**：落盘失败确实变成 `Err`，不再有静默成功。

---

## 5. F-01：IME 组合态守卫

源码（`frontend/src/design/writing/ComposerBar.vue`）：

```
:35  function onKeydown(event) {
:37    if (event.isComposing || composing.value || event.keyCode === 229) return   // 组合态一律放行，不 preventDefault
:40    if (event.key !== 'Enter') return
:41    if (event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) return // 保持 .exact 语义
:42    event.preventDefault();  submit()
:86  @keydown="onKeydown"  @compositionstart="composing = true"  @compositionend="composing = false"
```

- 旧的 `@keydown.enter.exact.prevent="submit"` 已不存在（`event.isComposing`/`composing` 双保险 + `keyCode === 229` 覆盖旧 IME）。
- **正常 Enter 仍提交**：守卫顺序为"组合态 → 非 Enter → 修饰键 → 提交"，裸 Enter 走到 `submit()`；`submit()`（`:29-33`）自身的 guard 未变。
- 回归测试 `frontend/tests/components-v2/composer-ime.test.mjs`（100 行，6 例）存在，覆盖 isComposing / compositionstart 未 end / compositionend 后正常提交 / keyCode 229 / Shift+Enter / 空白不提交。
- **未验证**：中文输入法**真机**（或 Chromium 真实 composition 事件）行为；用例里的 `isComposing`/`keyCode` 是 `Object.defineProperty` 手工注入（记录已如实说明）。本轮判定 = **代码通过 + 用例存在 + 真机未验证**，与记录口径一致。

---

## 6. F-02：「采纳」失败有用户可见反馈

`frontend/src/composables/useMessageVariants.js`：

- 采纳失败分支（`:282-309`）：质量门禁 Error 级 → 先 `askForceAccept`；用户点取消 → `return`（`:298-300`，**主动取消不是错误**，注释写明不再补一条"采纳失败"）；否则落到 `console.error('采纳失败:', e)` + `await alertDialog('采纳失败: ' + errorText(e))`（`:307-308`），走统一结构化错误 DTO（`errorText`，`:39` import）。
- 同文件其它失败路径（`:214-216` 编辑失败、`:243-246` 小票读取失败）同样是 `alertDialog + errorText` → 口径统一。
- 降级出口：未注入 `alertDialog` 时降级为 `console.error`（`:77`），属组合式 API 的安全默认，不是静默吞。

→ **通过(独立复现，静态)**；其"界面上真的弹出"由 Lead 的 vitest/DOM 门禁覆盖。

---

## 7. 删除面回归检索（含动态引用）

### 7.1 20 个被删 wrapper（域5 F-30）

在 `frontend/src/**` + `frontend/tests/**` 全树扫描 20 个 wrapper 名（`addWorldInfoEntry`、`cardShellAllowHost`、`configureEmbedder`、`deleteCharacter`、`deleteWorldInfoEntry`、`exportStCardPng`、`getActiveAgentProfileConfig`、`getActivePreset`、`getActiveProfile`、`getEmbedConfig`、`listModules`、`listProfiles`、`logGetLlmCall`、`metaClassifyStPreset`、`metaGetConversation`、`saveProfile`、`setActiveProfile`、`updateModule`、`updateWorldInfoEntry`、`updateWorldInfoRoute`）：

- **引用命中数 = 0**（含字符串、注释、测试）。
- 保留项 `cardstudioListStages`：`frontend/src/tauri-api.js` 中**存在**，且 `frontend/src/components-v2/campaign/CardStudio.vue` 中**被调用**。
- 未跟踪临时脚本：`frontend/tests/_*` = **0 个**（`_m24_selfcheck.cjs` / `_m31_selfcheck.mjs` 已删，且不会再让 secret scan 失败）。

### 7.2 9 个被删基元（F-16/F-17/F-18）

- 9 个文件 `Checkbox/DiffView/ErrorState/Menu/Progress/SegmentedControl/Slider/Toast/Tooltip.vue` **全部不存在**。
- `ui/` 目录**没有 barrel/index 文件**（无可断引用）；`frontend/src` 内**没有** `import()`/`require()`/`resolveComponent()`/`defineAsyncComponent()` 形式的动态组件解析。
- 全文（去掉 `//` 注释行）扫描裸标识符/标签/字符串：**剩余命中全部是无关用法**（`type="checkbox"`、headlessui `<Menu>`、`@lucide/vue` 的 `Menu` 图标、`progress:` 字段、`window.toastr`），**没有一处指向被删基元**。

→ 结论：删除面干净，未发现"动态引用已删组件/wrapper"的残留（这是 Lead 点名要查的项）。

---

## 8. 23 条"无前端入口的声明的保留 API"复核

| 检查 | 结果 |
| --- | --- |
| 条数与唯一性 | 23 条，无重复（`retainedNoFrontendCaller.length == 23`，去重后仍是 23） |
| 每条是否有保留理由 | 23/23 有非空 `reason`；**21 条**给出了保留依据/替代路径（如 `delete_character` 引 CLAUDE.md 级联语义、`configure_embedder` 写明"能力已实现、UI 未接"、`card_shell_allow_host` 写明"重新接线必须走用户确认"） |
| `file` 字段是否真实 | 23/23 与 `commandLocations` 实际定义文件**逐条一致**（我按导出函数做了交叉核对，无一漂移） |
| 是否与实现一致 | `undeclaredOrphanCommands == []`、`staleRetainedDeclarations == []`：既没有"零入口未声明"，也没有"声明了但其实有入口/已删除" |
| 是否被文档写成死代码/待删 | **没有**。反向检索 docs + CLAUDE.md 的"死代码/待删/待删除"字样相邻命令名：命中 3 处，全部是**正确口径**的声明——`docs/DOCS-CODE-AUDIT.md:13`"23 条……不是死代码、不是待删"、`CLAUDE.md:87`（`delete_character` 保留语义）、`CLAUDE.md:109`（20 条 wrapper 删除 + 23 条保留口径） |
| 表头边界声明 | `backend-baseline.mjs:174-176` 明确写"本表声明的是**事实**，是否彻底删除需 Lead 裁定"，与 Lead 口径一致；`:177` 提到"重新接线后 `staleRetainedDeclarations` 会立刻提示"——**该提示只在测试里会红，脚本门禁本身不会红**，见 N-R3-01 |

---

## 9. N-R7-04 回执：S-01 fail-closed 是否真被启动路径消费

**判定：已接通（fail-closed 硬失败，且不存在"静默回退 JSON"的旁路）。** 结论基于完整调用链（静态）+ 库级负例测试；**应用层（`resolve_backend`）缺一个直接测试**，见 N-R3-06。

R7 引用的 `crates/tauri-app/src/storage_backend.rs:268/285` 是**修复后**的行号：`:268` = `require_supported`、`:285` = `validate_runtime_authority`（HEAD 里这两个函数分别在 `:267`/`:284`，且 `validate_runtime_authority` 当时还没有 marker 复检）。它真正关心的"新判据是否被启动路径消费"，我按三条候选判据全部给出链路：

### 9.1 判据 A：S-01 readiness 源清单 fail-closed（域2 修复）

```
readiness.rs:194 / :223  Err(SqliteError::ImportSourceIncomplete(...))     ← 新判据本体
  ↑ importer.rs:311      let snapshot = crate::readiness::build_import_snapshot(data_dir)?;
  ↑ cutover.rs:1324      run_cutover(request)  （另见 cutover.rs:1058）
  ↑ storage_backend.rs:1939  recover_or_verify(&request).map_err(|e| BackendWiringError::Cutover(...))?
  ↑ storage_backend.rs:1929  let run_sqlite = || -> Result<BackendResolution, BackendWiringError> { ... }
  ↑ storage_backend.rs:2028 / :2033 / :2044   marker=SqliteAuthoritative / (JsonAuthoritative+env=sqlite) / Absent → run_sqlite()
  ↑ storage_backend.rs:1916  resolve_backend_inner  →  :1882 resolve_backend
  ↑ lib.rs:1229-1230         storage_backend::resolve_backend(&data_dir).map_err(|e| std::io::Error::other(e.to_string()))?
```

`resolve_backend_inner` 的三个 `run_sqlite()` 调用点都是**尾表达式**（`:2028`、`:2033`、`:2044`），没有任何 `or_else(run_json)`；JSON 分支（`run_json`，`:2001-2015`）只在**显式** `STORYFORGE_STORAGE_BACKEND=json` 时进入（`:2041`，且 `MarkerStatus::SqliteAuthoritative` 下显式 json 会被直接拒绝，`:2020-2027`）。因此 `ImportSourceIncomplete` → `BackendWiringError::Cutover` → `lib.rs:1230 ?` → **Tauri `setup` 返回 Err → 应用不启动**。

库级负例（域2 已入库，本轮未重跑）：`crates/infra-sqlite/tests/importer_diagnostics.rs` 对"缺 `cards.json` 但依赖集合非空"断言 readiness/importer/cutover 三层都 `Err`；反例覆盖在 `storage_backend.rs:2661 missing_collections_import_as_empty_like_json_store`（缺文件但当依赖为空 → 合法旧用户仍可启动），证明不是"一刀切拒绝"。

### 9.2 判据 B：S-17 运行时权威复检（`storage_backend.rs:285`）

```
lib.rs:764   storage.validate_runtime_authority()?;          ← AppState::new_with_backend 内
  ↑ lib.rs:1249-1251  AppState::new_with_backend(data_dir, storage).map_err(std::io::Error::other)?
  ↑ lib.rs:1238-1245  StorageFacade::new(...) + sqlite_runtime::activate(db_path)?
```
`validate_runtime_authority`（`:285-294`）对 SQLite facade 调用 `sqlite_runtime::validate_active_path(...)` + `self.revalidate_marker_authority()`（`:288` → `:309-328`：marker 翻成 JSON / 损坏 / 绑定不一致 / 版本不一致 / 探测失败 → `Err`）。任一 `Err` 经 `lib.rs:764 ?`、`:1250` 冒泡为 setup 失败 → **启动被拒**。测试侧：`crates/tauri-app/tests/*.rs` 有 13 个二进制在真实 SQLite AppState 上调用 `validate_runtime_authority`（含 `sqlite_optin_lifecycle.rs:132` 的"JSON facade × active runtime 必须 Err"负例）。

### 9.3 判据 C：`require_supported`（`storage_backend.rs:268`，R7 引用的另一行）

它不是启动闸门，而是**命令路径**的能力闸门：全仓 20+ 个命令侧调用点（`commands/characters.rs:7`、`commands/card_shell.rs:23/79`、`commands/meta_typed.rs:326/791/...`、`commands/campaigns.rs:1107/1134`、`commands/meta.rs:34/444`、`commands/variables.rs:12/66/87`、`commands/plugins.rs:198`）。SQLite 下 `ActiveCampaignPersistence` 是 `Degraded`（`storage_backend.rs:262`）→ 命中即 `Err`，所以它"被消费"是**命令层**的事实，不应按启动路径评价。

### 9.4 与 R7 的差异说明

R7 说"在 `lib.rs`/setup 装配点没找到直接调用命中"，成立的前提是**只看 setup 里是否直接出现 `readiness`/`require_supported` 字样**——真正的命中在 `lib.rs:1229-1230 → resolve_backend → run_sqlite → recover_or_verify → run_cutover → importer → readiness` 这条**间接链路**上（域2 的 S-01 判据落在 infra 层，app 层只负责传播错误）。按"必须给出 file:line + 调用链"的口径，本条**判为已接通**，不是"函数存在即算接通"。

（另：R7 顺带提的 **W-01 身份归一两侧不一致**（domain `campaign_runtime.rs:108` 的 Unicode `to_lowercase()` 且不 trim vs app-agent `runtime.rs:608` 的 `trim()+eq_ignore_ascii_case()`）**不属于本次 R3 范围**，本报告不引用、不混淆、不判定。）

---

## 10. 新发现（N-R3-01 … N-R3-06）

> 严重度沿用审查口径：P0 数据丢失/安全/主流程不可用；P1 主路径错/原子性破坏；P2 边界、错误处理、冗余；P3 命名/注释/文档/门禁健壮性。**均不阻塞本轮收口**。

| # | 严重度 | 位置 | 现象 | 建议 |
| --- | --- | --- | --- | --- |
| **N-R3-01** | **P3** | `scripts/architecture/backend-baseline.mjs:514-516`（计算）vs `:322-355`（`evaluateGates`） | `staleRetainedDeclarations`（"声明为保留，但已重新接线或已被删除"）**只被计算、不进 `evaluateGates`**。负控 2 实证：注入 `_invoke('abandon_turn')` 后 `staleRetainedDeclarations == ["abandon_turn"]`、`undeclaredOrphanCommands == []`，脚本仍打印"门禁通过"、**exit 0**；同一状态下 `tauri-command-contract.test.mjs` 会红（exit 1）。影响面有限：CI 跑的是测试文件（`.github/workflows/release.yml:69/158`），`verify-release.ps1` 也不直接跑本脚本；但脚本 `:177` 的注释"`staleRetainedDeclarations` 会立刻提示"对**单独跑脚本**的场景不成立 | 在 `evaluateGates` 加 1 条 `failures.push(...)`（3 行），使"架构门禁"独立自足；或把 `:177` 注释改成"由契约测试断言" |
| **N-R3-02** | **P3** | `scripts/architecture/backend-baseline.mjs:86`；`frontend/tests/tauri-command-contract.test.mjs:44` | 注释里的计数值过期：脚本写"修正后真实值 **172**"、测试写"**169→172**"，实测唯一 invoke = **152**（域5 删除 20 个 wrapper 之后）。断言本身用的是不变量，不受影响 | 把两处注释改成"修正时 172；域5 删 20 wrapper 后现为 152，数值以 `invokedCommands` 为准"，避免下一次审查再按注释复盘数字 |
| **N-R3-03** | **P3** | `scripts/architecture/backend-baseline.mjs:240-243 / :259-263` | `RETAINED_NO_FRONTEND_CALLER` 里 `list_modules`（"模块列表读取"）与 `update_module`（"模块更新"）的 `reason` 是**同义反复**（只是描述命令做什么），没有像同批其它 21 条那样给出保留依据或替代路径（对照 `configure_embedder`"能力已实现、UI 未接"、`get_active_preset`"Preset 当前项读取"） | 补成同一口径，例如"模块列表读取；能力已实现、UI 未接（域5 独立核对：零引用而非被取代）" |
| **N-R3-04** | **P3** | `scripts/architecture/backend-baseline.mjs:51-66`（`extractCommandAttributes`）；断言在 `tauri-command-contract.test.mjs:42` | 锚定行正则已修好 `//`、`///` 注释（N-01 已闭环），但**仍会**把 (a) `/* */` 块注释内以 `#[tauri::command]` 开头的行、(b) `#[cfg(test)] mod tests` 里的 mock 命令计入"命令数"。探针实测：block comment → 1、cfg(test) 模块 → 1。后果是 `commandAttributes == 175` 可能**在没有任何真实命令变化时变红**（fail-closed 噪声；不会假绿）。此外 `commandAttributes` 统计 `backendSource`（**含** `lib_tests*`），而 `definedCommandCount` 只统计生产源 | 保留 175 契约值，另加一条自洽断言 `commandAttributes === definedCommandCount`（并考虑让 `extractCommandAttributes` 只用 `productionBackendSources`）；若要更彻底，按"属性行 + 其后紧跟 `fn`"配对统计 |
| **N-R3-05** | **P3** | `docs/review-2026-09-13/fixes/05-frontend-fixes.md:406-431` | A′ 级等价 harness（`%TEMP%` 下的 `sf-vue-loader.mjs` / `sf-vitest-shim.mjs`）**未入库、不可复现**；本成员直接 `node frontend/tests/components-v2/composer-ime.test.mjs` 得到 `ERR_UNKNOWN_FILE_EXTENSION: .vue`（该文件 `import { test, expect } from 'vitest'`，只能在 vitest 下跑）。记录本身已声明工具在仓库外，但"6/6 绿"这一行因此**无法被第三方复核** | 结论改挂 Lead 门禁（vitest 31 文件/151 全绿，已覆盖该文件）；后续若要 A′ 级证据，把 loader/shim 纳入 `frontend/tests/harness/`（有版本、可重跑），不再写"仓库外一次性脚本"结果 |
| **N-R3-06** | **P3** | `crates/tauri-app/src/storage_backend.rs`（测试模块）/ `crates/tauri-app/src/lib.rs` | S-01 fail-closed 的**应用层传播**目前只有静态链路证明：现有测试里 `resolve_backend` 的负例只有"损坏 JSON / 环境值非法 / marker 冲突"（`:2699`、`:2537`、`:2546`），没有"S-01 源清单不完整的真实目录 → `resolve_backend` 必须 `Err(BackendWiringError::Cutover)`"。库级三层断言（readiness/importer/cutover）已覆盖判据本体，故不判"未接通"，但这一环缺回归锁 | 在 `storage_backend.rs` 测试模块加 1 例：铺 `cards.json` 缺失 + `campaigns.json` 非空 → 断言 `resolve_backend_inner` 为 `BackendWiringError::Cutover` 且**未创建** `storyforge.db` 与 marker（Node 侧无需改动） |

---

## 11. 未复验、移交与遗留

1. **未由本成员运行**：`npm test`、`npm run test:ui`、`npm run build`、`cargo test --workspace`、`cargo clippy`、`cargo fmt`、Pester —— 全部以 `docs/review-2026-09-13/fixes/GATE-REPORT.md` 为准（Lead 在收口门禁执行）。
2. **未复验内容**：域4 T-04/T-07/T-12 的 wire 行为（`log_clear` 返回值、`add_variant` 的 `provenance` 是否真被后端消费、3 个 export 的 async 语义）本轮只做签名/调用点静态核对；域5 F-03…F-47 除 F-01/F-02/F-16/F-17/F-18/F-30 外的条目未逐条重验。
3. **移交他域**：T-03/T-05/T-09 → 域6；T-14 → task-16；W-01 身份归一 → R1/R2（本报告不判定）。
4. **遗留（非本轮）**：`ui/Select` 的 `portal` 能力已交付但**无调用点开启**（F-22 降级，属后续项）；`ui/**` 删除后 3 个面板各自内联"错误行 + 重试"（F-19 降级）；`--write-snapshot` 目前是手工入口，未接入 `verify-release.ps1`。
5. **本报告唯一写入**：`docs/review-2026-09-13/round2/R3-tauri-frontend-recheck.md`。所有破坏性验证均在 `%TEMP%\sf-r3-negctl`（robocopy 影印，已剔除 `target`/`.git`/`node_modules`/`artifacts`/`.worktrees`/`gen`，61.5 MB）内进行并在其后清理；仓库工作树内未新增/删除任何文件。

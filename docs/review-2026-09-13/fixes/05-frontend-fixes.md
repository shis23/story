# 修复记录：域5 前端通用层（task-12，F-01..F-47 全量修复）

- 修复人：review-frontend（域5）
- 任务：task-12「修复域5：前端通用层（F-01..F-47 全量修复）」，Lead 指令（revision 2）
- 依据：`docs/review-2026-09-13/05-frontend.md`（域5 审查报告，P1=14 / P2=19 / P3=14）
- 起始 HEAD：`ab894c6`
- 工作区状态：多域并发修复（同一工作树由 7 个域 + Lead 并行写），本域只写自己的 scope

## ⚠️ 0 验证限制（先读这一条，再读任何"已修复"）

**本会话（子代理沙箱）无法运行 `npm test` / `npm run test:ui` / `npm run build`。**

已确诊为沙箱边界而非工程问题：所有"带管道 stdio 的子进程 spawn"被拒（`EPERM errno -4048 syscall spawn`）。
- `npm.cmd test` → `node:internal/test_runner/runner:415` EPERM
- `npm.cmd run test:ui`（vitest）→ `esbuild/lib/main.js:1978 ensureServiceIsRunning` EPERM
- `npm.cmd run build` → 同一个 esbuild EPERM
- 已尝试 `node node_modules/vitest/vitest.mjs run --pool=threads` 绕过 fork（worker_threads 方案）→ 仍死在 vite 配置打包阶段的 esbuild service spawn

可替代的本地验证手段（均为单进程、无 spawn）：`node <某个 test 文件>.mjs`（node:test 同进程跑）、`node --check`、
`@vue/compiler-sfc` 编译校验。**因此**：

| 验证等级 | 含义 | 覆盖 |
| --- | --- | --- |
| A 已本地跑通 | 我在沙箱内单进程执行过并给出 pass/fail | node:test 套件（`tests/**/*.test.mjs` **67 文件 / 532 pass / 0 fail**） |
| A' 已用等价 harness 跑通 | vitest 的 vite+esbuild 路径被沙箱拒绝，我用 `@vue/compiler-sfc` + happy-dom + `@vue/test-utils` + 自写 vitest shim 组了一个**单进程** runner，逐文件跑 `tests/components-v2/**` | **31 文件：28 全绿 / 136 pass**；剩 3 文件受 harness 能力限制（见 §6.2） |
| B 已静态校验 | SFC 全量编译通过 + 调用点/引用扫描 | 87/87 个 `src/**/*.vue` 编译 ok；删除项的"零引用"扫描 |
| C 未运行验证 | harness 与沙箱都跑不了，**必须由 Lead 在收口门禁跑** | `npm test` / `npm run test:ui` / `npm run build` 的 npm 入口与最终产物 |

> harness 的定位说明：它是**信号工具**，不是 vitest 的替代品（`vi.mock` 明确不支持、matcher/timer 只实现了用到的那部分）。
> 它已抓到一个真实回归（见 §3 F-22 的 N-03），但**任何"绿"都不能替代 Lead 的 `npm run test:ui`**。

**本文件中每一个"已修复"都请按"代码已改 + A/A'/B 级证据"理解，C 级项一律在此列出、不做任何"门禁通过"的表述。**
收口门禁（Lead 跑）建议顺序：`cd frontend; npm test` → `npm run test:ui` → `npm run build` → 根目录 `cargo check`/`verify-release.ps1`。

## 0.1 状态汇总

| 状态 | 数量 | 条目 |
| --- | --- | --- |
| 已修复 | 41 | F-01..F-13、F-41、F-14..F-21、F-23..F-33、F-35..F-39、F-42..F-45 |
| 已修复(降级) | 4 | F-19、F-22、F-34、F-47 |
| 判定非问题 | 0 | — |
| 暂缓 | 1 | F-46（Lead 裁定：`components-v2/writing/**` 保留代码，文档措辞移交 task-f7） |
| 移交他域/他任务 | 1 | F-40（文档漂移 → task-f7）、F-30 的"3 个零调用后端命令"部分（→ 域4/T-15）、F-19 的"收敛到 ErrorState"部分（ErrorState 已按 F-18 删除，条目失效） |
| 顺带新发现并修复 | 3 | N-01（`ConnectionConfigPanel` 的 `latency_ms` 双处漂移，见 F-42）、N-02（`PromptHookAuditLog` 复制态按下标存储，见 F-20）、**N-03（本域自己引入的 Select portal 回归：headlessui v1.7 的 `Portal` 没有 disabled prop，用它当开关会让默认态也 teleport；已改 Vue 内置 `<Teleport :disabled>` 并补 2 条测试）** |
| 顺带新发现（域外，已上报） | 3 | 见 §7「域外红项/上报」 |

按严重度核对：P1 14 条（F-01..F-13 + F-41）**全部已修复**；P2 19 条（F-14..F-30 + F-42..F-45）全部已修复（F-19/F-22 降级）；P3 14 条（F-31..F-40 + F-46、F-47）已修复 12、降级 2（F-34/F-47）、暂缓 1（F-46）、移交 1（F-40）。

---

# 1 P1 逐条（F-01..F-13、F-41）

### F-01 中文输入法下按回车确认候选词会直接提交写作意图 → 已修复

- **修法**：`frontend/src/design/writing/ComposerBar.vue`
  - 删除 `@keydown.enter.exact.prevent="submit"`（Vue 的按键修饰符只校验修饰键，**不校验 `isComposing`**），改为 `@keydown="onKeydown"`。
  - 新增 `composing` ref + `@compositionstart` / `@compositionend`；`onKeydown` 三重守卫：
    ```js
    if (event.isComposing || composing.value || event.keyCode === 229) return   // 组合态一律放行，不 preventDefault
    if (event.key !== 'Enter') return
    if (event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) return // 保持 .exact 语义
    event.preventDefault(); submit()
    ```
- **证据（A' 级）**：`node tests/components-v2/composer-ime.test.mjs` 的等价 harness **6/6 绿**（见 §6.1.1）；SFC 编译 ok。
- **新增回归测试**：`frontend/tests/components-v2/composer-ime.test.mjs`（6 例：isComposing=true / compositionstart 未 end / compositionend 后正常提交 / keyCode 229 伪键 / Shift+Enter 不提交 / 空白不提交）。测试内对 `isComposing`、`keyCode` 用 `Object.defineProperty` 显式定义，避免 happy-dom 对 `KeyboardEventInit` 支持不完整导致"测的是 DOM 而不是组件"；`compositionend` 后的提交用 `await nextTick()` 等 v-model 回写（首跑 harness 就是在这里红的，属测试自身缺陷，已修）。

### F-02 「采纳」失败被静默吞掉 → 已修复

- **修法**：`frontend/src/composables/useMessageVariants.js`。取消路径静默返回（用户主动取消不是错误）；真实失败改为 `console.error` + `alertDialog('采纳失败: ' + errorText(e))`，`errorText` 走统一结构化错误 DTO 通道（禁止裸 `+ e`）。
- **证据**：`node --check` exit 0；采纳成功/失败分支都在同一个 `try/catch`，无其它吞异常路径。

### F-03 开场卡壳加载状态是死状态 → 已修复

- **修法**：`frontend/src/AppV2.vue`：新增 `showCardShellOpeningLoading` computed，`AppFrame` 的 `opening` 槽渲染 `role="status" aria-live="polite"` 的「序章准备中…」。此前该状态由 `cardShellOpening` 计算但模板无出口。
- **证据**：编译 ok；`role="status"` 与 F-38 的 a11y 口径一致。

### F-04 日志面把错误对象裸插值 → 已修复

- **修法**：`frontend/src/AppV2.vue` 的日志/错误拼接统一改 `errorText(e)`。
- **证据**：`node --check` exit 0；全 `src/` 无新增 `'…' + e` 直拼（本次修复的 6 个文件均已替换）。

### F-05 `ui/DataList` 选中态恒 false（`activeKey` 语义失效）→ 已修复

- **修法**：`frontend/src/components-v2/ui/DataList.vue` 重写。新增 `activeId` prop（`[String,Number]`），`isActive(item, index)` 真实比对 `item[activeKey] === activeId`；删除恒 false 的双分支与注释里不存在的 `activeItem` 承诺；`:key="item?.[activeKey] ?? index"`。
- **证据**：唯一生产调用点 `CardLibrary.vue:140-146` 现传 `:active-id="expandedCardId"`，▲▼ 与行高亮由同一状态驱动。

### F-06 `ui/DataList` 的 `select` 事件生产不可达且无人监听 → 已修复（方案 a：受控列表接线）

- **修法**：`DataList.vue` 保留 `select` 但把它变成真正可用的受控契约：整行 `@click="emit('select', item, index)"`；`CardLibrary.vue` 去掉行内 `@click.stop`，改 `@select="toggleCard"`，并传 `:active-id`。兜底渲染（`item.label || item.name || JSON.stringify(item)`）保留为无 slot 调用点的降级出口。
- **证据**：`src/` 中 DataList 调用点仍为 1 个（CardLibrary），`@select` 已接线；`tests/components-v2/ui-components.test.mjs` 同步更新。

### F-07 「可能完成」任务被永久锁死 → 已修复

- **修法**：`frontend/src/components-v2/campaign/CampaignTasksTab.vue`。`likely_completed` 行渲染「完成 / 确认完成」按钮（走既有 `setTaskStatus` → `TaskStatus::Completed`），不再只显示不可操作状态。
- **证据**：按钮文案与 `isLikelyCompleted(status)` 绑定；行级 busy（F-24）同时生效。

### F-08 任务状态渲染成「可能完成 (NaN%)」 → 已修复

- **修法**：`frontend/src/utils/taskStatus.js`
  - 新增 `likelyCompletedConfidence(status)`（按 serde externally-tagged 形状 `{ likely_completed: { confidence: f32 } }` 取值，缺失/非数值返回 `null`）与 `isLikelyCompleted(status)`。
  - `taskStatusText` 改用它们：有置信度显示 `可能完成 (50%)`，无置信度显示 `可能完成`，**永不出现 `NaN%`**。
- **证据（A 级）**：`node tests/task-status.test.mjs` → **pass 5 / fail 0（exit 0）**，含真实 serde 形状用例与 `confidence` 缺失用例；未改任何 Rust。

### F-09 「设为当前活动」失败零提示 + 被外层误报 → 已修复

- **修法**：`frontend/src/components-v2/campaign/CampaignPanel.vue`：`handleSetActive` 返回 boolean（成功/失败），失败路径自己 `alertDialog`，不再被外层 catch 误报成「创建失败/导入失败」。
- **证据**：调用点按返回值分流。

### F-10 活动列表加载失败伪装成「还没有活动档」 → 已修复

- **修法**：
  - `CampaignPanel.vue`：新增 `campaignsError` ref；`refreshCampaigns` 内 catch 写入错误文案；onMounted 的加载失败也 `alertDialog`；`loadSelectedCampaignCardDetail` 补 catch。
  - `frontend/src/design/campaign/CampaignScreen.vue`：新增 `listError` prop，在 loading/empty 分支**之前**渲染错误行 + 「重试」按钮 `@click="emit('refresh')"`；`CampaignPanel` 用 `:list-error="campaignsError || ''"` 与 `@refresh="onScreenRefresh"` 接线。
- **证据**：错误态不再落入空态分支（模板顺序：`listError` → `loading` → `empty`）。

### F-11 角色卡库加载失败被吞 → 已修复

- **修法**：`frontend/src/components-v2/campaign/CardLibrary.vue`：`cardsError` + 错误行 + 「重试」；`refreshCards()` 补 catch；`character_definitions` 访问加守卫。
- **证据**：`CardLibrary` 的加载失败与"卡库为空"现在可区分。

### F-12 写卡工作室「保存产物」失败后仍继续编译导入 → 已修复

- **修法**：`frontend/src/components-v2/campaign/CardStudio.vue`：`saveArtifacts()` 改为返回 `true/false`；9 个调用点统一 `if (!(await saveArtifacts())) return`，失败即中止后续编译/导入。
- **证据**：全文件无 `await saveArtifacts()` 后无条件继续的路径。

### F-13 会话历史加载失败静默 + 单个 DTO 缺 `updated_at` 会清空列表 → 已修复

- **修法**：`frontend/src/composables/useConversation.js`
  - 排序改为容忍缺字段：缺 `updated_at` 的记录排最后，同键用原始 index 兜底保持稳定（不再因 `undefined` 比较导致整个列表被判空）。
  - 加载失败 `alertDialog('加载会话历史失败: …')`。
- **证据**：排序回调全程 NaN 安全（`Number.isFinite` 判定）。

### F-41 审计面板「展开详情」100% 抛 `ReferenceError` → 已修复

- **修法**：`frontend/src/components-v2/debug/PromptHookAuditLog.vue`
  - `toggleExpand(row)` 内 `recordKey(record)` → `const key = recordKey(row)`（形参名不一致导致 100% 抛 `ReferenceError: record is not defined`）。
  - 顺带：详情块由 `v-for + v-show + index` 改为 `expandedRecords` computed（只渲染已展开项）；`copyStatusTimer` 在 `onUnmounted` 清理；剪贴板失败改为 `复制失败: errorText(e)`。
- **新增回归测试**：`frontend/tests/components-v2/audit-log-expand.test.mjs`（4 例：未展开不渲染详情 / 点击展开不抛错且详情出现 / 只渲染 1 条详情且可收起 / 空态）。

---

# 2 P2 逐条（F-14..F-30、F-42..F-45）

### F-14 `Overlay` 的 `showClose` 传了 `title` 时被忽略（双 ×） → 已修复

- **修法**：`ui/Overlay.vue` 标题分支的关闭按钮也受 `showClose` 约束；两处关闭按钮共用同一 `v-if`。
- **证据**：模板中 `showClose` 出现在两个分支。

### F-15 `Overlay` 同一布尔两个 prop、关闭时三事件齐发 → 已修复

- **修法**：`open` 的 setter 只 emit 绑定的那个 prop（用 `useSlots`/props 判定调用方实际绑定的形式），不再 `update:open` + `update:modelValue` + `close` 三连。
- **证据**：单个 setter 分支。

### F-16 `ui/Menu` 三处死契约 + 生产内联同一套 headlessui Menu → 已修复(删除)

- **修法**：删除 `ui/Menu.vue`（生产零引用、仅测试引用）；生产页面继续用其内联的 headlessui Menu（行为不变）。
- **证据（B 级）**：`src/`+`tests/` 全树 `/Menu.vue` 引用数 = 0；`ui-components.test.mjs` 已移除对应 import/用例。

### F-17 `ui/DiffView` 用按下标逐行当 diff → 已修复(删除)

- **修法**：删除 `ui/DiffView.vue`（零引用的伪 diff 实现）。**真实 diff 能力不在本次引入**：如后续需要行级 diff，应按 LCS/Myers 实现并配测试，而不是恢复这个版本。

### F-18 9 个注释齐全的基元零引用/仅测试引用 → 已修复(删除)

- **删除清单（9 个）**：`ui/Checkbox.vue`、`ui/DiffView.vue`、`ui/ErrorState.vue`、`ui/Menu.vue`、`ui/Progress.vue`、`ui/SegmentedControl.vue`、`ui/Slider.vue`、`ui/Toast.vue`、`ui/Tooltip.vue`。
- **证据（B 级）**：逐个全树引用扫描均为 0（含 `tests/**`）；`tests/components-v2/ui-components.test.mjs` 已同步删除对应 import 与断言。
- **副作用（诚实记录）**：F-19 建议的"把加载失败+重试收敛到 `ErrorState`"因此改为各面板内联错误行（见 F-19 降级说明）；F-38 中针对 Progress/Toast/Checkbox/Tooltip/Slider 的 a11y 条目随之失效。

### F-19 「加载失败」静默成空态且无重试 → 已修复(降级)

- **修法（内联错误行 + 重试，未收敛到共享组件）**：`LogPanel.vue` 新增 `loadError` + 重试行；`CardStudio.vue` 新增 `projectsError` + 重试行（并补 `LoadingState`）；`CardLibrary.vue` 见 F-11；`CampaignKnowledgeTab.vue` 原本就有错误文案。
- **降级原因**：审查建议的收敛目标是 `ui/ErrorState.vue`，而该组件在 F-18 死亡代码清理中已按 Lead 的"ui/** 死基元默认删除"裁定删除。三个面板各自内联（同一视觉/交互口径：错误行 + 重试按钮），不引入新共享组件。
- **未覆盖**：`AgentProfileManager`、`PluginPanel` 已是内联错误文案，未改为统一组件。

### F-20 `ui/DataTable` 行 key 用数组下标 → 已修复

- **修法**：
  - `ui/DataTable.vue`：新增 `rowKey`（`[String, Function]`，空则回退下标）；`:key="rowKeyValue(row, ri)"`。
  - 6 个调用点接线：`CampaignInstancesTab`/`CampaignKnowledgeTab`/`CampaignSummariesTab`/`CampaignTasksTab`（`row-key="id"`）、`WorldInfoReadonlyPanel`（`row-key="index"`）、`PromptHookAuditLog`（`:row-key="recordKey"`，审计记录无 `id`）。
  - **N-02（顺带）**：`PromptHookAuditLog` 的"已复制"状态由 `copiedIndex` 改为 `copiedKey`（按 `recordKey` 比），插入新记录不再漂移到别的行；`copyRecord(row)` 不再吃下标。
- **证据（A 级）**：`node tests/components-v2/ui-components.test.mjs` 属 vitest（C 级）；`node --check`/SFC 编译 ok。

### F-21 `ui/Tabs` 在 `modelValue` 非法时静默选中第 0 项 → 已修复

- **修法**：`ui/Tabs.vue` 新增 DEV-only `console.warn`（含非法值），生产不刷屏；选中回退行为保留（不 throw）。

### F-22 `ui/Select` 下拉未 portal，会被 `overflow-hidden` 裁剪 → 已修复(降级)

- **修法（含 N-03：我先写错的版本已纠正）**：`ui/Select.vue` 新增 `portal` prop（默认 `false`，opt-in）。**第一版我用了 headlessui v1.7 的 `<Portal :disabled="!portal">`——这是错的**：该组件的 props 只有 `as`（见 `node_modules/@headlessui/vue/dist/components/portal/portal.js`），`disabled` 会被当普通 attr 丢掉，**它永远 teleport**。后果：连默认态也把面板挂到 `#headlessui-portal-root`，`absolute left-0 w-full` 失去按钮锚点 → 生产下拉会跑到页面左上角、宽度撑满视口。改为 Vue 内置 `<Teleport to="body" :disabled="!portal">`（Vue 的 Teleport 真的支持 `disabled`）。
- **为什么能发现**：`tests/components-v2/select.test.mjs` 在等价 harness 里立刻红（`w.find('[role="listbox"]')` 找不到），这就是"N-03 自引入回归"的证据链。
- **测试**：`select.test.mjs` 原 4 例保持绿（默认 portal=false 时面板留在组件内，行为与修复前一致），**新增 2 例**：`portal=false（默认）时选项面板留在组件内部`、`portal=true 时选项面板挂到 body 且仍可选中`。harness：**6/6 pass**。
- **降级点（诚实）**：**没有任何调用点开启 `portal`**。全 `src/` 扫描：13 个 `<Select>` 调用点里，只有 `CampaignWorldInfoTab.vue:368` 的祖先链上有 `overflow-hidden`（L298），而 body 直挂的 `absolute` 面板缺定位锚点——盲开会把"被裁剪"换成"跑到页面左上角"。所以本轮只交付**可用且真实可关的 opt-in 能力**，开启需先补 fixed 定位（量按钮 rect + 视口翻转），属后续项。

### F-23 `ui/Toggle` 无 `role="switch"` 可访问名 → 已修复

- **修法**：`ui/Toggle.vue` 新增 `label` / `ariaLabel` prop 并输出 `:aria-label`；调用点接线：`AgentProfileManager.vue`（两处 Toggle 补 `label="后处理"` / `label="剧情总结"`）、`config/PluginPanel.vue`（补 `:aria-label="\`${item.enabled ? '禁用' : '启用'}插件 ${item.name || item.id}\`"`）。
- **跨域声明**：`PluginPanel.vue` 是域6 scope，本次只加了这 1 行 a11y 属性，已同步告知 review-meta-plugin。

### F-24 写操作缺 busy 防重 → 已修复

- **修法**：`CampaignTasksTab.vue` 新增 `creatingTask`（创建按钮 `:disabled` + `:loading`）与 `busyTaskId`（行内动作按钮按 `busyTaskId === row.id` 禁用）；`PresetPanel.vue` 新增 `importingModules` + `:disabled`。
- **说明**：`Button.vue` 的 `loading` 契约在 F-38 中改为"保留文案 + spinner"，所以「创建中…」「导入中…」现在真正可见（此前文案被 spinner 顶掉）。

### F-25 变量写入失败后输入框仍显示新值 → 已修复

- **修法**：
  - `CampaignVariablesTab.vue`：新增 `revertNonce`，6 个受控控件加 `:key="…-${revertNonce}"`；写入失败时 `revertNonce += 1` 强制用回滚后的 store 值重建控件。
  - `CampaignInstancesTab.vue`：`variableRevertNonce` + 4 个控件同理；并把两种失败拆开提示——「设置变量失败（已回滚显示）」vs「变量已写入，但回读失败（请刷新查看）」。
- **证据**：失败分支必经 nonce 自增；无 `PLACEHOLDER`/临时探针残留（本次修复中曾误写坏一次 `CampaignVariablesTab.vue`，已按 git 版本恢复并复核）。

### F-26 世界书路由「选中即写库」，失败后草稿与后端长期不一致 → 已修复

- **修法**：`CampaignWorldInfoTab.vue`：新增 `savingRoute` 守卫；失败时回滚 `draft.route` + 重新加载 + `alertDialog`；路由 `Select` 绑定 `:disabled="savingRoute"`。

### F-27 两处死 emit → 已修复

- **修法**：`CampaignTasksTab.vue` 删除 `defineEmits(['refresh'])` 与 3 处 `emit('refresh')`；`NewCampaignForm.vue` 删除 `created` 声明与 `emit('created')`，并把误导性注释改正。
- **证据（B 级）**：两个文件内已无对应 emit/声明。

### F-28 `utils/consoleForwarding.js` 无人引用 + `JSON.stringify` 循环对象会抛 → 已修复

- **修法**：
  - 生产改走 util：`AppV2.vue` 用 `installConsoleForwarding()`（`utils/consoleForwarding.js` 的 `setupConsoleForwarding`），删除内联副本——**测试保护的实现与生产跑的实现从此同一份**。
  - `consoleForwarding.js` 导出 `safeSerializeConsoleArg`，循环/不可序列化参数降级为 `[Unserializable]`，不再让日志转发自身抛异常。
- **证据（A 级）**：`node tests/console-forwarding.test.mjs` → **pass 5 / fail 0（exit 0）**。

### F-29 角色识别入口未回退 `source_character_id` → 已修复

- **修法**：`CardLibrary.vue` 新增 `sourceCharacterId(card)`（优先 `source_character_id`，回退 `id`），识别/禁用判定与调用参数统一走它；老数据不再必然失败并禁用整页按钮。

### F-30 21 个孤儿 wrapper 与 3 个零调用后端命令 → 已修复（wrapper 部分）/ 移交（后端命令部分）

- **修法（Lead 裁决 A：协同删除）**：`frontend/src/tauri-api.js` 删除 20 个零引用 wrapper，保留并接线 1 个：

  | wrapper | 后端命令 | 处置 |
  | --- | --- | --- |
  | addWorldInfoEntry / cardShellAllowHost / configureEmbedder / deleteCharacter / deleteWorldInfoEntry / exportStCardPng / getActiveAgentProfileConfig / getActivePreset / getActiveProfile / getEmbedConfig / listModules / listProfiles / logGetLlmCall / metaClassifyStPreset / metaGetConversation / saveProfile / setActiveProfile / updateModule / updateWorldInfoEntry / updateWorldInfoRoute | 对应 snake_case 同名命令（20 个） | **删除** |
  | cardstudioListStages | `cardstudio_list_stages` | **保留**：F-39 要求阶段轨由后端权威定义驱动，已在 `CardStudio.vue` 接线（`loadStageMeta()` → `stageMeta`，失败回退本地常量） |

- **证据（A/B 级）**：`node --check src/tauri-api.js` exit 0；20 个名字在 `src/`+`tests/` 全树引用数 = 0。
- **移交**：`docs/review-2026-09-13/05-frontend.md` 的"3 个零调用后端命令（含契约基线盲区）"属域4 T-01/T-15；`card_shell_register_doc/module/unregister_doc` 三个走 `utils/shellDocUrl.js` 裸 `_invoke` 的调用点**一个未动**。
- **必须联动**：`scripts/architecture/backend-baseline.mjs` 的 `RETAINED_NO_FRONTEND_CALLER` 需把这 20 个命令登记为"保留 API"（域4 维护）。已发送完整映射表给 review-tauri-api（cc Lead）；未补登记前 `tests/tauri-command-contract.test.mjs` 的 `Gate 0 registry equals frontend reachable set ∪ declared retained API` 会红（见 §7）。**域4 已补完 20 条声明**（含逐条 reason，`delete_character` 引用 `CLAUDE.md` 的级联语义），复测 **11/11 绿**；Lead 已裁定本轮"保留 + 声明 = 现状即终态"。
- **⚠️ 两个面不要混读（与域4 的 N-02 对齐）**：我这里的结论只覆盖**产品/维护面**——这 20 个命令"零引用且没有调用点被取代（不是被替代，而是本来就没接 UI）"，因此删与不删都是产品决策、无维护性风险。**安全面另说**：它们**仍然是已注册的后端 IPC 命令**，在域4 的 **P0-2（子帧可调任意命令）/ N-02** 风险关闭之前，等同**额外 IPC 攻击面**。"功能上值得保留" ≠ "没有风险"，两者不冲突；本域不对此做安全结论。

### F-42 「测试连接」的延迟永远不显示（camelCase vs snake_case） → 已修复

- **修法**：
  - `frontend/src/components-v2/config/ConnectionConfigPanel.vue` 模板读 `testResult.latency_ms`（Tauri 只 camelCase **命令参数**，不碰响应体）。
  - `tauri-api.js` 的 `testConnection` JSDoc 与 mock 同步为 `{ success: true, message: '（mock）连通成功', latency_ms: 42 }`——**N-01**：JSDoc/mock 里的 camelCase 是同一漂移的第二处，审查报告只列了模板那处。
- **证据**：`node --check` exit 0；模板与 mock 字段名一致。

### F-43 配置面板角色列表缺 `Writer` → 已修复

- **修法**：`AgentProfileManager.vue` 角色表新增 `{ key: 'Writer', label: '执笔' }`。
- **证据（代码事实）**：`crates/domain/src/agent.rs:18` 的 `AgentRole::Writer` 存在，配置面板此前无法为执笔者设模型/轮次。**未改 Rust**。

### F-44 流水线 trace 面板不认识 `writer_*` 事件 → 已修复

- **修法**：`PipelineTracePanel.vue` 把 `writer_*` 事件并入 `editorEvents` 流（含 `writer_progress` 增量与「执笔者续写」detail），执笔者在面板中可见。

### F-45 `PresetPanel` 加载/展开失败无 catch → 已修复

- **修法**：`PresetPanel.vue` 新增 `loadError` + 重试块；`refreshGlobalRegexScripts` 补 catch；`togglePreset` 失败回滚展开态并给 `detailLoading`（「详情加载中…」），不再出现"箭头翻转但详情永不出现"。

---

# 3 P3 逐条（F-31..F-40、F-46、F-47）

### F-31 生产入口保留 3 条设计演示 hash 路由 → 已修复

- **修法**：`frontend/src/main.js` 新增 `DESIGN_PREVIEW_ROUTES` map，仅在 `import.meta.env.DEV` 时启用，生产分支为 `null`（可被 tree-shake 掉，演示屏不进产物）。
- **证据（C 级）**：产物体积/tree-shake 需 `npm run build` 验证（Lead 门禁）；已知 build 有 >560KB chunk 警告（既有、非失败）。

### F-32 `ui/DataList` 空态能力生产不可达 + 注释承诺不存在的 `activeItem` → 已修复

- **修法**：见 F-05/F-06：注释改成真实存在的 `activeId`；空态加 `empty-icon` / `empty-action` 槽；`emptyTitle`/`emptyDescription` 生效。

### F-33 `ui/DataTable` 死 emit、空态 colspan 与列注释 → 已修复

- **修法**：`ui/DataTable.vue` 删除从未作为事件使用的 `row-action` emit 声明（3 个调用点用的都是 `#row-action` 槽）；`colSpan` 改为 `columns.length + (hasRowAction ? 1 : 0)`（无槽调用点不再多一列）；`useSlots()` 判定插槽存在性。

### F-34 列表基元无分页/虚拟化，卡库与日志全量渲染 → 已修复(降级)

- **修法**：`DataList.vue` / `DataTable.vue` 新增 `maxItems` + 「显示更多（还有 N 条）」。`CardLibrary` 传 `:max-items="60"`；`LogPanel` 改为渲染上限 + 展开态 Set 上限 50（淘汰最旧）。
- **降级说明**：**未实现虚拟滚动**（工作量大、需真实浏览器验证滚动锚定）。当前是"首屏截断 + 显式展开"，能消除"一次性全量渲染"这一主要风险，但大列表展开后仍是全量 DOM。建议单列后续任务。

### F-35 `ui/Textarea` `autoResize` 首帧高度错误且是死 prop → 已修复

- **修法**：`ui/Textarea.vue`：`onMounted(() => nextTick(resize))` + `onInput` 重新计算高度；新增 `invalid` prop → `aria-invalid`。

### F-36 `ui/CodeBlock` 复制定时器未清理、失败静默、无标题栏时按钮遮挡首行 → 已修复

- **修法**：`onBeforeUnmount` 清理定时器；复制失败显示「复制失败」；无 toolbar 时给 `pr-16` 预留按钮位置。

### F-37 headlessui 语义缺口（Tabs 无 `TabPanel`、Overlay 无 `DialogTitle`） → 已修复

- **修法**：`ui/Tabs.vue` 内容包 `TabPanel`（每个 tab 有对应面板，读屏可跳转）；`ui/Overlay.vue` import + 渲染 `DialogTitle`，并在无可见标题时用 `:aria-label="title || undefined"` 提供名称。

### F-38 a11y 缺口汇总 → 已修复

- **修法**：`ui/Input.vue` `aria-invalid`；`ui/IconButton.vue` `ariaLabel` + `aria-busy`；`ui/LoadingState.vue` 两个分支均 `role="status" aria-live="polite"`；`ui/Button.vue` spinner 保留文案 + `aria-busy`；`index.html` 删除 `maximum-scale=1.0, user-scalable=no`（保留 `viewport-fit=cover`）。
- **契约变更（重要）**：`Button` 的 `loading` 由"spinner 顶替文案"改为"文案 + spinner"。旧行为让 `ConnectionConfigPanel`「测试中…」「保存中…」等文案永不显示，且测试锁死了该缺陷——`tests/components-v2/button.test.mjs` 已同步改为断言"保留文案 + spinner + `aria-busy`"。
- **条目失效**：针对 Progress/Toast/Checkbox/Tooltip/Slider 的 a11y 条目随 F-18 删除而失效（组件不存在）。

### F-39 死状态与陈旧注释 → 已修复

- **修法**：
  - `CampaignPanel.vue`：删除死 `activeTab` ref + 陈旧注释 + 9 处写入。
  - `AgentProfileManager.vue`：删除死 `activeId`/`void activeId`、多余 `computed` import，以及无人传的 `embedded` prop 分支（改为直接 `<PanelHost show title=… side="left" @close>`）。
  - `PipelineTracePanel.vue`：`statusFromEventType` + 反向扫描 `lastStatus`；`directorDetailJson`/`subagentDetailJson`/`editorDetailJson`/`postprocessDetailJson` 四个 detail 计算属性缓存在 script 层（模板里不再反复 `JSON.stringify`）。
  - `LogPanel.vue`：内容比较后再替换 `logs`。
  - `stores/ui.js`：陈旧注释改正（`viewWrite` 确实被使用；真正死的是 `powerMode`/`togglePower`/`viewOverview`——**保留**，`tests/stores/ui.test.mjs` 覆盖它们，删除会连带删测试）。
  - `CardStudio.vue`：接线 `cardstudioListStages`（原 wrapper 零调用）。
  - `useNewCampaignForm.js`：新增 `detailLoadInFlight`/`newCampaignCardDetailLoading`，同卡片详情重复加载去重 + 竞态守卫。
- **证据（A 级）**：`node tests/stores/ui.test.mjs` → pass 15 / fail 0；`node --check` exit 0。

### F-40 文档与代码事实漂移 → 移交 task-f7（文档域）

- **本域不动 `docs/**`**（Lead 硬约束）。已在 §5 给出可直接照抄的修正清单（原文位置 + 事实）。
- 其中的测试数字一项（`ROADMAP.md:210`）本域给出今天可复现的口径：node 套件 67 个文件 / 532 pass（本次实测，见 §6），vitest 套件 `tests/components-v2/` 31 个文件（原有 27 + 本域新增 2 + 域6 新增 2；A' harness 已跑 28 绿 / 136 pass）。

### F-46 「回退树」`components-v2/writing/**` 交互缺陷 → 暂缓（Lead 裁定：保留代码，文档措辞移交 task-f7）

- **Lead 的死亡代码政策**：`components-v2/writing/**`（8 个文件）是**文档化的回退树**，因此**保留代码不删**；本域不碰这些文件（1 行未改）。
- **本域记录（供 task-f7 的文档措辞修正直接使用）**：`design/writing/CONTRACT.md:55` 现写"保留作对照与回退，生产主路径不再引用"——按现状把回退开关打开后 **(a) Campaign「采纳」永久静默**（小票 UI 只存在于 `design/writing/MessageItem.vue`，该树 `pendingReceipt` 零消费）、**(b)** 保存编辑失败仍关编辑器（不消费 `handleEditVariant` 的 `false`）、**(c)** 变体切换/采纳/删除无 busy 门、**(d)** `add-variant` 声明无按钮、**(e)** 非取消类失败只置 `pipeline.state='error'`，"生成中"硬编码且停止键消失、**(f)** `GreetingSelector.vue:24` `:key="option.label"` 同文案重复 key、**(g)** `ConversationViewport.vue:28` `ui` 零使用 + `:71-74` `canBranch(m)` 忽略入参。
- **建议措辞**：把"可回退"改成"仅存档参考（回退等于不可用，见 05-frontend.md F-46）"，或为回退路径补一条 mount 冒烟测试后才能声明可用。**未修 (a)-(e)**：Lead 未授权改这棵树，且改到"与 design/writing 等价"需要小票 UI 迁移，属新功能而非修复。

### F-47 debug/config 面板次要缺陷 → 已修复(降级)

- **已修**：`PromptHookAuditLog.vue`（展开态按 record 身份键 + 只渲染已展开项 + `copyStatusTimer` 清理 + 剪贴板失败反馈）；`PipelineTracePanel.vue`（见 F-39）；`ConnectionConfigPanel.vue`（`applyTemplate` 复制 `t.protocol`，temperature/topP 仅在 `Number.isFinite(parseFloat(...))` 时写入，否则 `null`）；`AgentProfileManager.vue`（见 F-39）；`LogPanel.vue`（见 F-39）。
- **降级说明**：`PipelineTracePanel` 的 `lastStatus` 由事件流反向扫描得出，**没有**与 `writingStore.pipeline.state` 做交叉校验；若进程被硬杀（无终态事件），面板仍可能停留在 running/进度态。补齐需跨 store 读取 + 终态判定规则，属后续项。
- **顺带纠正审查报告的一处前提**：报告称 `<script setup>` 顶层函数在模板中不可用（`lastStatus` "在模板里不可用"）。**该前提不成立**：`<script setup>` 的顶层绑定（含函数/`const`）会自动暴露给模板。本域的真实问题是"每次渲染都重算 + 下标漂移"，已按性能/稳定性修掉（缓存 computed + 稳定 key），而不是"不可见"。

---

# 4 死亡代码与删除清单（Lead 政策：`ui/**` 死基元默认删除）

| 类别 | 处置 | 明细 |
| --- | --- | --- |
| `ui/**` 死基元（9） | **删除** | Checkbox、DiffView、ErrorState、Menu、Progress、SegmentedControl、Slider、Toast、Tooltip |
| `ui/**` 死 emit / 死 prop | **删除** | DataTable `row-action` emit；Overlay 冗余 prop 的重复 emit；DataList `activeKey` 恒假语义（重写为真实语义） |
| `components-v2/writing/**`（8） | **保留** | Lead 裁定为文档化回退树；本域 1 行未碰；文档措辞 → task-f7（见 F-46） |
| `tauri-api.js` 孤儿 wrapper（20） | **删除** | 见 F-30 表；保留 1 个（`cardstudioListStages` 已接线） |
| 仓库内临时脚本（2） | **删除** | 未跟踪的 `frontend/tests/_m24_selfcheck.cjs`、`_m31_selfcheck.mjs`——它们会让 `verify-release.ps1` 的 secret scan（第 1 步）失败。复核：`frontend` 下 `_*` 残留 = 0；本域新增文件均为正式 `*.test.mjs`，无临时脚本入库 |

**删除后不变量（B 级扫描）**：`src/`+`tests/` 对 9 个被删基元的 `*.vue` 引用数全为 0；`tests/components-v2/ui-components.test.mjs` 已同步删除相应用例（不再有"测已删除文件"的红）。

**测试文件变更清单**：
| 文件 | 变更 |
| --- | --- |
| `tests/components-v2/composer-ime.test.mjs` | 新增（F-01，6 例） |
| `tests/components-v2/audit-log-expand.test.mjs` | 新增（F-41，4 例） |
| `tests/components-v2/button.test.mjs` | 改：loading 契约（保留文案 + spinner + aria-busy） |
| `tests/components-v2/ui-components.test.mjs` | 改：删 2 个被删基元用例，新增 Input aria-invalid / DataTable rowKey+colspan+maxItems / Toggle aria-label |
| `tests/components-v2/select.test.mjs` | 改：新增 2 条 portal 断言（默认留在组件内 / portal=true 挂 body） |
| `tests/task-status.test.mjs` | 改：真实 serde 形状 + confidence 缺失用例 |
| `tests/_m24_selfcheck.cjs`、`tests/_m31_selfcheck.mjs` | 删（未跟踪临时脚本，见上表） |

---

# 5 移交清单（task-f7 文档域可直接照抄）

| # | 文档位置 | 应改成 |
| --- | --- | --- |
| 1 | `docs/FRONTEND-COMPONENTS.md:270,275` | 把 `components-v2/writing/**` 的"写作区组件蓝图"改为指向 `design/writing/**`（生产写作面），并注明 `components-v2/writing/**` 是**阶段 8 遗留、零引用、未接线** |
| 2 | `docs/ROADMAP.md:202,212` | "ChatMessage 保留 8 emit 契约"的真实载体是 `design/writing/MessageItem.vue`（8 个 emit 全部接线）；`components-v2/writing/ChatMessage.vue` 零引用 |
| 3 | `docs/ROADMAP.md:210` | 测试数字改为可复现口径：`cd frontend && npm test`（node:test 67 文件 / 532 用例）+ `npm run test:ui`（vitest `tests/components-v2/` 31 文件） |
| 4 | `CLAUDE.md`（wrapper 清单段） | `getActiveAgentProfileConfig` 等 20 个 wrapper 已在 F-30 删除，不再是前端入口；保留入口为 `cardstudioListStages` 等。**注意别写成"后端命令被删"**：本轮只删前端 wrapper，后端**命令仍注册**（175 不变），并按 Lead 裁定在 `scripts/architecture/backend-baseline.mjs` 的 `RETAINED_NO_FRONTEND_CALLER` 登记为"保留 API"。其中 `delete_character` 的保留依据就是 `CLAUDE.md` 的**级联删除语义**（对 `StoredCharacter.id` / `source_character_id` / 同会话 `tool_ctx` 域 id 做 Campaign/MVU/向量清理）→ 回写该文档时**不要**把这条命令写成"死代码/待删" |
| 5 | `design/writing/CONTRACT.md:55` | "保留作对照与回退"→"仅存档参考（回退等于不可用，见 `05-frontend.md` F-46）" |
| 6 | `docs/FRONTEND-COMPONENTS.md`（基元章节） | 删除 Checkbox/DiffView/ErrorState/Menu/Progress/SegmentedControl/Slider/Toast/Tooltip 共 9 个基元的条目；`ui/` 现存 15 个文件 |
| 7 | `docs/ROADMAP.md`（如提及 `ui/ErrorState` 的错误态方案） | 改为"各面板内联错误行 + 重试"（F-19 降级方案） |
| 8 | 任何"21 个孤儿 wrapper"表述 | 更新为"20 个已删除 + 1 个接线（`cardstudioListStages`）"，并注明这 20 个命令**已由域4 在 `RETAINED_NO_FRONTEND_CALLER` 登记为保留 API**（前端唯一 invoke 数 172 → 152，注册命令总数仍 175；是否连删属域4 的 N-02/P2 产品决策，交用户） |

---

# 6 验证记录（命令 + 退出码 + 计数）

## 6.1 沙箱内可跑（A 级，均已实测）

```
# 1) 全量 node:test 套件（npm test 的等价口径，逐文件单进程执行）
文件数 67；TOTAL pass=532 fail=0      → 全部 exit 0
（会话中期曾有 1 条红：tests/tauri-command-contract.test.mjs 的 Gate 0
 retained 声明缺 20 条；域4 已于本轮补完 backend-baseline.mjs 的
 RETAINED_NO_FRONTEND_CALLER，复测 11/11 绿）

# 2) 本域直接相关的重点文件
node tests/task-status.test.mjs                 → pass 5  / fail 0  (exit 0)
node tests/console-forwarding.test.mjs          → pass 5  / fail 0  (exit 0)
node tests/stores/ui.test.mjs                   → pass 15 / fail 0  (exit 0)
node tests/composables/usePluginBridge.test.mjs → pass 18 / fail 0  (exit 0)   # M-08 通道未破坏
node tests/prompt-hooks.test.mjs                → pass 23 / fail 0  (exit 0)
node tests/plugin-bridge.test.mjs               → pass 78 / fail 0  (exit 0)
node tests/tauri-command-contract.test.mjs      → pass 11 / fail 0  (exit 0)   # 域4 补声明后

# 3) 语法检查（本域改动的全部 .js）
node --check src/tauri-api.js src/utils/taskStatus.js src/utils/consoleForwarding.js \
  src/composables/useConversation.js src/composables/useMessageVariants.js \
  src/composables/useNewCampaignForm.js src/composables/usePluginBridge.js \
  src/utils/promptHooks.js src/stores/ui.js src/main.js        → 全部 exit 0

# 4) 全量 SFC 编译校验（@vue/compiler-sfc：parse + compileScript + compileTemplate）
compiled ok: 87/87   → exit 0
```

## 6.1.1 A' 级：`tests/components-v2/**` 的等价 harness 结果（31 文件 / 136 pass / 28 绿）

工具（**全部在仓库外**，`%TEMP%` 下一次性脚本，未入库）：
`sf-vue-loader.mjs`（ESM loader：`vitest` → shim，`.vue` → `@vue/compiler-sfc` 编译）、
`sf-vitest-shim.mjs`（test/describe/it.each/expect/vi 的最小实现）、
`sf-vitest-runner.mjs`（happy-dom 全局 + `__TAURI_INTERNALS__` 双向代理 + 逐文件隔离运行）。

| 文件（`tests/components-v2/`） | 结果 | 说明 |
| --- | --- | --- |
| **composer-ime.test.mjs**（本域新增，F-01） | **6/6 绿** | IME 三重守卫逐条验证 |
| **audit-log-expand.test.mjs**（本域新增，F-41） | **4/4 绿** | 展开不再抛 ReferenceError |
| **button.test.mjs**（本域改） | **9/9 绿** | loading 保留文案 + spinner + aria-busy |
| **ui-components.test.mjs**（本域改） | **16/16 绿** | 含 rowKey/colspan/maxItems/aria-invalid |
| **select.test.mjs**（本域改） | **6/6 绿** | 含 2 条 portal 测试 |
| card-studio.test.mjs（本域 F-12/F-19/F-39 改动面） | **3/3 绿** | 含 `cardstudio_list_stages` 接线后的挂载冒烟 |
| campaign-variables-tab / campaign-instances-add / campaign-world-info-layout | 绿 | F-25、行内重试 |
| log-panel-filter / message-edit-contract / new-campaign-opening | 绿 | F-19/F-27/Composer 相关 |
| character-detail-navigation / app-frame-walkthrough / app-frame-responsive-sidebar | 绿 | 13+15+6 |
| overlay / shell-aware-content / mvu-status-panel / heavy-shell-dock 等其余 | 绿 | 共 28 个文件全绿 |
| plugin-host-handshake.test.mjs | **harness 无法运行** | 该文件用 `vi.mock`，shim 不实现模块替换（显式抛错，不假装通过） |
| tavern-helper-runtime-bridge.test.mjs | 2/4 | 剩余 2 条断言组件文本含 iframe/宿主相关串；happy-dom 不执行 iframe 脚本 → harness 不可判定 |
| theme-picker.test.mjs | 1/3 | 断言差异疑似 localStorage/夜间模式在 harness 下的差异；**本域未改动该文件与组件**，以 Lead 门禁为准 |

**harness 已抓到的真实问题（价值证明）**：N-03（Select portal 回归，见 F-22）、
`card-studio` 的 `cardstudio_list_stages` 非数组返回值（本地回退分支实测生效）、
`new-campaign-opening` 的 `invocationCallOrder` 顺序断言（F-27 删除 `created` emit 后仍绿）。

## 6.2 沙箱内跑不了（C 级，**必须由 Lead 在收口门禁执行**）

```
cd frontend
npm test            # node:test（等价于 6.1-1；已用单进程等价口径实测 67 文件 / 532 pass / 0 fail）
npm run test:ui     # vitest 31 文件 —— 含本域新增的 2 个文件；A' harness 已给出 28 绿 / 136 pass
npm run build       # 产物/tree-shake（F-31）；>560KB chunk 警告为既有、非失败
```
本域**新增/修改的 vitest 文件**（harness 已实测绿，但**尚未经 vitest 本体验证**，请重点看这四处）：
- `tests/components-v2/composer-ime.test.mjs`（新增，6 例，F-01）
- `tests/components-v2/audit-log-expand.test.mjs`（新增，4 例，F-41）
- `tests/components-v2/button.test.mjs`（改动：loading 契约断言）
- `tests/components-v2/ui-components.test.mjs`（改动：删除被删基元用例 + 新增 Input/DataTable/Toggle 断言）
- `tests/components-v2/select.test.mjs`（改动：默认态留在组件内 + 新增 2 条 portal 断言）

**已做的静态前置核对**（降低 vitest 首跑红的风险）：`vitest.config.mjs` 的 `include` 覆盖 `tests/components-v2/**/*.test.mjs`；`pinia`/`@vue/test-utils` 在 devDependencies；`usePluginStore().promptHookAuditRecords` 是 `ref` 可赋值；`IconButton` 会把 `title` 渲染到 `<button>`（测试靠 `title="展开"` 定位）；`DataTable` 无 `#row-action` 时不会多出空列。

# 7 域外红项与跨域上报（**不是本域引入，也未由本域修改**）

| # | 现象 | 责任域 | 状态 |
| --- | --- | --- | --- |
| 1 | `tests/tauri-command-contract.test.mjs`：`Gate 0 registry equals frontend reachable set ∪ declared retained API` 曾红。原因是 F-30 删除 20 个 wrapper 后，`scripts/architecture/backend-baseline.mjs` 的 `RETAINED_NO_FRONTEND_CALLER` 尚未登记这 20 个后端命令 | 域4（review-tauri-api） | **已修**：域4 补完 20 条声明（含理由与文件位置；`delete_character` 用 `CLAUDE.md` 的级联语义作依据），复测 `node tests/tauri-command-contract.test.mjs` → **11/11 绿**；`node scripts/architecture/backend-baseline.mjs` → exit 0「门禁通过」；前端唯一 invoke 数 172 → **152**；`orphanRegisteredCommands` 23 = 域4/T-15 原有 3 + 本轮 20；无第 21 个孤儿。**注意区分面**：本行是"声明事实/产品面"，与域4 的 **P0-2/N-02 安全面**（子帧可调任意命令未关闭前，这 23 条等同额外 IPC 攻击面）**不冲突** |
| 2 | M-08 落地（本域做前端侧）：`pluginPromptHookResult` 增 `modifierPluginIds`，`promptHooks.js` 的 hook 链在 mutation 被采纳后记录改写者插件 id | 域5↔域6 | 已完成；`prompt-hooks.test.mjs` 23/23、`usePluginBridge.test.mjs` 18/18 绿 |
| 3 | 会话中一度出现 3 个域6 红项（`prompt-hooks` 墙钟预算、`plugin-bridge` M-31b 配额、`usePluginBridge` intent 链被 M-21a 校验整份丢弃） | 域6 | **已由域6 修复**：M-21a 改为"只回退 messages、其余字段生效"；M-21b 用 `timeoutClampedByBudget`/`chainBudgetExhausted` 去掉毫秒取整不确定性；M-31b 是测试算术错（2×256KiB 未越 1MiB 上限）。复测 23/78/18 全绿 |
| 4 | M-20 接线（`AppV2.vue` 的 `<PluginPanel>` 加 `@plugins-changed="loadSidebarPlugins"`） | 域5（应域6 请求） | 已加；禁用插件即时失效 |
| 5 | M-22 敏感字段门控 | — | 本域无 `CHAT_CHANGED.reason` / `SubagentDone.*` 消费者，已回复域6"收到、无需权限区分" |
| 6 | M-18（`useMvuStatusPanel.js` scope/target 显式化） | 域6 | 本域**暂缓**并已通知域6（避免跨域并发写），提议不改也不影响正确性 |
| 7 | `Permission::WriteChat`（M-31c）的权限展示 | — | `PluginPanel.vue:174-176` 是 `item.permissions` 字符串直出，无白名单 → 新权限名自动显示，本域无需改动（已回复域6） |
| 8 | 域6 请求代跑 3 个 vitest 文件（M-07/M-19/M-24 证据） | 域6 | `shell-aware-content` **13/13 绿**；`plugin-host-handshake` harness 无法运行（`vi.mock`）；`tavern-helper-runtime-bridge` 2/4（harness 限制，见 §6.1.1）。已回复 review-meta-plugin |

# 8 剩余风险（诚实清单）

1. **C 级未验证面**：`npm run test:ui`（vitest 本体）与 `npm run build` 未跑过（沙箱 EPERM）。A' harness 覆盖了 31 个组件测试文件里的 28 个，但 `plugin-host-handshake`（`vi.mock`）、`tavern-helper-runtime-bridge` 的 2 条 iframe 断言、`theme-picker` 的 3 条断言仍需 Lead 用 vitest 本体裁决。**本域所有"绿"都不等于门禁绿灯。**
2. **F-34 未做虚拟滚动**：大列表展开后仍是全量 DOM（已降级说明）。
3. **F-47 未做跨源终态校验**：硬杀进程后 trace 面板可能残留 running 态。
4. **F-46 回退树仍不可用**：按 Lead 裁定保留代码，只修文档措辞（task-f7）；`components-v2/writing/**` 的 (a)-(e) 缺陷仍在。
5. **F-19 未收敛到共享组件**：4 个面板各自内联错误行 + 重试，存在轻微重复（因 `ui/ErrorState.vue` 已按 F-18 删除）。
6. **`stores/ui.js` 死成员保留**：`powerMode`/`togglePower`/`viewOverview` 被 `tests/stores/ui.test.mjs` 覆盖，删除会连带删测试；已改为"注释说明真实使用情况"而非静默遗留。
7. **跨域并发写同一工作树**：`promptHooks.js`/`usePluginBridge.js`/`PluginPanel.vue` 三处本域与域6 有交集（已在 §7 逐条披露），合并时请以工作树现状为准。

# 9 R6 无记录项收口（2026-09-13，task-33）

R6 对账（`round2/R6-fix-completeness-audit.md`）的 4 条"无记录"中，**W-31 [P3]** 归本域：前端 `validGenerationModes` 允许 `big_scene`，但档位目录（`utils/generationModes.js`）没有该项，ComposerBar 只渲染 3 档 ⇒ 旧 `localStorage` 里的 `big_scene` 被接受为当前档位，参与 `start_writing` / `allowPartialReroll` 判定，用户处在"看不到选中项"的昂贵模式（原始发现 `03-writing-pipeline.md:701-705`；R2 复检 N-R2-09）。

| 项 | 内容 |
| --- | --- |
| 处置 | **已修**：`stores/writing.js` 的 `validGenerationModes` 改为**从档位目录派生**（`new Set(generationModeCatalog.map(m => m.value))`），`big_scene` 不再是前端合法档位；解析到它时回退默认档位 `continuation`；读取阶段**不重写/不删除**用户偏好 blob（旧的 `big_scene` 不会被当成合法档位使用，也不会破坏其它 Campaign 的已存档位） |
| 为什么不选"保留 + 映射为可见档位" | 会让产品面从 3 档变 4 档，与 `README.md:19` / `ARCHITECTURE.md:56` / `AGENT_INTERFACES.md:12`（"仅保留后端兼容路径"）的定位冲突；静默映射到别的档位则会改写用户意图且成本语义不同。**后端 `BigScene` 兼容面未动**，非 Campaign 路径行为不变 |
| 测试 | 新增 `tests/components-v2/writing-generation-mode-store.test.mjs`（vitest，6 例：陈旧值回退默认 + blob 逐字节不变、`setGenerationMode` 拒绝 `big_scene`/未知值且不写盘、合法三档正常、重建后按 Campaign 记忆、陈旧值不会打开 legacy 局部重 roll）；`tests/stores/writing.test.mjs` 新增 1 例等价 node:test 用例；`tests/composables/useWritingScreenAdapter.test.mjs:70-71` 原断言"选中 `big_scene` ⇒ `allowPartialReroll === true`"**固化了缺陷行为**，已更正为"选中被拒绝" |
| 失败可控性 | 把实现临时改回旧版做变异测试：等价复算 6 例 **1 pass / 5 fail**、`tests/stores/writing.test.mjs` **18 pass / 1 fail**、adapter 用例红；恢复实现后全绿（`git diff` 只含上述 3 个跟踪文件 + 1 个新文件） |
| 我实跑的验证 | `node --check` ×4 全过；`node tests/stores/writing.test.mjs` **19/19**、`tests/generation-modes.test.mjs` **2/2**、`tests/reroll-policy.test.mjs` **2/2**、`tests/composables/useWritingScreenAdapter.test.mjs` **7/7**、`useMessageVariants-receipt` **3/3**、`useMessageVariants-guards` **4/4**；vitest 用例的等价 node:test 复算 **6/6** |
| **未验证（需 Lead）** | `npm run test:ui`（含新增 6 例）、`npm test` 全套（`node --test` 逐文件 spawn 在本会话报 `spawn EPERM`）、`npm run build` —— **我未声称已绿**。详见 `round2/R10-unrecorded-frontend.md` §4.4 |
| 转交 | `adapter/useWritingScreenAdapter.js:89` 的 `\|\| writing.generationMode === 'big_scene'` 现为死分支 → 归 **task-32 / N-R2-01** 清理；`08-docs-sync-fixes.md:123` 的虚假归属不在本项范围 |

详见 `docs/review-2026-09-13/round2/R10-unrecorded-frontend.md`。

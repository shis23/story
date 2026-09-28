# 域5 审查报告：前端通用层与组件可用性

- 审查日期：2026-09-13
- 审查对象：git HEAD `ab894c6`（"fix(release): platform-suffixed checksum files; record v0.1.2 closure"），工作树在我开始前仅含 `docs/review-2026-09-13/`（其他域的报告）
- 审查性质：**只读静态审查**。未运行任何 `npm` / `npx` / `vite` / `vitest` / `playwright` / `cargo` / 构建命令；未修改 `docs/review-2026-09-13/05-frontend.md` 以外的任何文件
- 报告文件：`docs/review-2026-09-13/05-frontend.md`（本次审查唯一写入的文件）
- 审查视角：**前端组件是否真的可用**（零引用 / 仅测试引用组件、交互缺陷、静默失败、加载态缺失、事件契约断链、错误文案覆盖、前后端 wrapper 契约），而非"有没有测试"

### 协作说明（本次判定如何得出）

| 来源 | 覆盖 | 说明 |
| --- | --- | --- |
| 本域亲自通读 | `AppV2.vue`、`main.js`、`design/writing/*`（10 个）、`design/shell/AppFrame.vue`、`components-v2/shell/*`、`stores/*`、`composables/useWriting|useMessageVariants|useGreeting|useConversation|usePipeline|useNewCampaignForm`、`adapter/*`、`utils/errorText|taskStatus|consoleForwarding|formatContent`、`components/base/BaseDialog.js`、`vite.config.js`、`frontend/package.json` | 结论均带 `file:line` + 代码引文（≤15 行） |
| 深读子代理 A（ui 基元层） | `src/components-v2/ui/**`（23 个基元）逐行 + 全部调用点 | 产出 U-01..U-32；本域对 P1/P2 条目逐条复核代码引文后收录为 F-xx |
| 深读子代理 B（campaign 面板） | `src/components-v2/campaign/**`（12 个）逐行 + 后端 DTO 交叉 | 产出 C-01..C-28；本域复核其 P1 引文后收录 |
| debug / config 面板 | `LogPanel` / `PipelineTracePanel` / `PluginEventLog` / `PromptHookAuditLog` / `AgentProfileManager` / `ConnectionConfigPanel` / `PresetPanel`（+`components-v2/writing/**` 8 个死组件） | 逐行通读（深读子代理 C，本轮已返回）＋本域 handler 级普查（catch/finally/busy/error 全量导出，见 §1.3(g)）复核 |

**范围外**（域6 负责，本域不判定、不报告缺陷）：`components/CardShellHost.vue`、`PluginHost.vue`、`MvuJsRuntime.vue`、`TavernHelperRuntime.vue`、`CharacterList.vue`、`components-v2/st/**`、`components-v2/meta/**`、`components-v2/config/PluginPanel.vue`、`utils/{cardShell*,mvu*,shell*,plugin*,promptHook*,tavernHelper*}` 及其测试。`components/CardShellFloatingStatus.vue` 命名归 `cardShell*`，同样不判定。

---

## 1 范围与覆盖率

### 1.1 已核对范围

| 范围 | 内容 | 覆盖 |
| --- | --- | --- |
| 入口与外壳 | `src/main.js`（23 行）、`src/AppV2.vue`（1028 行）、`src/design/shell/AppFrame.vue`、`components-v2/shell/{PrimarySidebar,TopBar,InspectorDrawer,PanelHost,StorageHealthGate,ThemePicker}.vue`、`src/useTheme.js`、`src/themePreferences.js`、`index.html` | 全文 / 关键段 |
| 生产屏（design/） | `design/writing/{WritingScreen,StoryPage,MessageItem,VariantStrip,ComposerBar,StreamingBody,ProcessTimeline,EmptyHero,GreetingCards}.vue`、`design/history/HistoryScreen.vue`、`design/overview/OverviewScreen.vue`、`design/campaign/CampaignScreen.vue`、`design/meta/MetaScreen.vue` | 主要文件全文 / props+emits 提取 100% |
| composable 层 | `useWriting.js`(228)、`useMessageVariants.js`(520)、`useGreeting.js`、`useConversation.js`、`usePipeline.js`(214)、`useNewCampaignForm.js`、`useMvuStatusPanel.js`、`useCharacterImport.js`、`usePluginBridge.js`(范围外主体) | 前 6 个关键路径全文 |
| store 层 | `stores/{ui,writing,campaign,plugin}.js` + `tests/stores/*.test.mjs` 4 个 | 全文 |
| 基元层 | `components-v2/ui/**` 23 个基元 + 全部调用点 | 逐行（子代理 A） |
| campaign 面板 | `components-v2/campaign/**` 12 个 | 逐行（子代理 B） |
| debug/config 面板 | 见协作说明 | handler 级普查 |
| adapter 层 | `adapter/*.js` 5 个（128+22+33+121+38 行） | 全文 |
| 契约与统计 | `src/tauri-api.js`(1691 行导出表)、`tests/tauri-command-contract.test.mjs`、`scripts/architecture/backend-baseline.mjs`、`frontend/package.json`、`vitest.config.mjs`、`vite.config.js`、5 个 playwright 配置/spec、`tests/` 全量基线 | 相关部分 |
| 文档漂移 | `docs/FRONTEND-COMPONENTS.md`、`docs/ROADMAP.md`、`CLAUDE.md`、`frontend/CONTRACT.md` | 相关段落 |

未覆盖：未编译、未运行门禁、未做真机 UI 验证；所有"运行期会怎样"的结论由源码 + 框架语义推导，逐条标注置信度。

### 1.2 统计口径（数量类结论的可复现命令与结果）

以下命令均在 `frontend/` 下执行（记 `$FE = C:\Users\Predator\ZCodeProject\storyforge\frontend`）。

**(a) 组件总数与目录分布**

```powershell
$vue = @(Get-ChildItem src -Recurse -File -Filter *.vue); "vue total = $($vue.Count)"
foreach ($d in @('design','components-v2','components','composables','stores','utils','adapter')) {
  "  $d files=$(@(Get-ChildItem "src\$d" -Recurse -File).Count) vue=$(@(Get-ChildItem "src\$d" -Recurse -File -Filter *.vue).Count)" }
```

结果：`vue total = 96`；`design` 25 文件/17 vue、`components-v2` 78/70、`components` 11/8、`composables` 9/0、`stores` 5/0、`utils` 45/0、`adapter` 5/0。加 `AppV2.vue` 即 96。

**(b) 零引用组件普查**（口径：`import` + 标签是唯一引用通道；`vite.config.js:10` 为 `plugins: [vue(), tailwindcss()]`，无 unplugin 自动导入/全局注册；对每个 `.vue` 基名在 `src/**`（排除自身）做**大小写敏感**子串匹配）

```powershell
$src = @(Get-ChildItem src -Recurse -File | Where-Object { $_.Extension -in '.vue','.js','.mjs' })
foreach ($f in (Get-ChildItem src -Recurse -File -Filter *.vue)) {
  $hits = @($src | Where-Object { $_.FullName -ne $f.FullName } | Select-String -Pattern $f.Name -SimpleMatch)
  if ($hits.Count -eq 0) { $f.FullName }
}
```

结果：**14 个零引用**（含域6 的 `components-v2/st/StCompatibilityBadge.vue` → 本域 **13 个**：ui 9 + writing 4）＋ **5 个传递死引用**（`ChatMessage` / `GreetingSelector` / `ProcessReview` / `StreamingMessage` 只被零引用的 `ConversationViewport.vue` 引用；`components/base/BaseDropdown.vue` 只被死代码 `ChatMessage.vue:3` 引用）→ **本域死组件合计 18 个**，清单见 §4.5。注意：`components-v2/writing/**` 8 个文件整体为死代码。两份仓内自述与普查一致：`design/writing/CONTRACT.md:55`「旧 `components-v2/writing/*` 保留作对照与回退，生产主路径不再引用 Viewport/Composer」、`design/history/CONTRACT.md:3` 把 `ConversationHistoryList` 记为被替换对象。

**(c) 组件测试引用覆盖率**（口径：`tests/**` 中任意文件出现该 `.vue` 基名；**上界**——`tests/fixtures`/stub 注册也算命中）

```powershell
$t = @(Get-ChildItem tests -Recurse -File | Where-Object { $_.Extension -in '.mjs','.js','.ts','.vue' })
foreach ($f in (Get-ChildItem src -Recurse -File -Filter *.vue)) { $hits = @($t | Select-String -Pattern $f.Name -SimpleMatch); ... }
```

结果：`vue=96 covered=41 uncovered=55`。生产屏中未被任何测试引用到的：`design/{history/HistoryScreen,meta/MetaScreen,overview/OverviewScreen}`、`design/writing/{EmptyHero,GreetingCards,StoryPage,VariantStrip}`、`components-v2/campaign/{CampaignPanel,CampaignKnowledgeTab,CampaignSummariesTab,CampaignTasksTab,CardLibrary,CharacterCardDetail,WorldInfoReadonlyPanel}`、`components-v2/config/*` 4 个、`components-v2/debug/{PipelineTracePanel,PluginEventLog,PromptHookAuditLog}`、`components/base/BaseDropdown.vue` 等。（**此为覆盖率事实，不单列为缺陷**；只在 §3 的具名缺陷里引用，例如 `utils/taskStatus.js` 的测试夹具把错误 DTO 形状冻结。）

**(d) 前后端命令契约**（口径同 `scripts/architecture/backend-baseline.mjs:55-67`，本域独立复现）

```powershell
# 后端注册命令
Select-String -Path crates\tauri-app\src\lib.rs -Pattern 'tauri::generate_handler!\[' -Context 0,200
# 前端 invoke（三条正则：\binvoke\(\s*['"]…  /  \._invoke\(\s*['"]…  /  (?<![\w])command:\s*['"]…）
```

结果：`#[tauri::command]` = 175、`generate_handler!` = 175（双向差集 ∅）；前端 `uniqueInvokeCount = 169`、`invokeCount = 171`、缺失命令 ∅ —— 与 `tests/tauri-command-contract.test.mjs` 钉住的 175/169/171 完全一致。**基线正则的 `\._invoke\(` 分支漏掉 `utils/shellDocUrl.js:64,83,105,120` 的裸 `_invoke('card_shell_*')`**（域4 报告 T-01 已独立得出同一结论，本域复核一致）：按"任意 `invoke(` 子串"重扫，前端真实唯一命令 = 172。

**(e) 孤儿 wrapper**（口径：`tauri-api.js` 的 `export function/const` 基名，在 `src/**` 中**大小写敏感**零命中）

```powershell
$api = Get-Content src/tauri-api.js -Raw
$exports = @([regex]::Matches($api,'export\s+(?:async\s+)?function\s+([A-Za-z0-9_]+)').Groups[1].Value) + …
```

结果：导出 160 个，**零产品调用方 = 21 个**（清单见 F-30）。另有 **3 个后端命令全仓无前端调用**：`abandon_turn`、`archive_conversation`、`soft_delete_variant`（`card_shell_register_doc/module/unregister_doc` 实际被调用，只是被 (d) 的正则盲区漏计）。

**(f) 错误文案覆盖率**

```powershell
@(Get-ChildItem src -Recurse -File | Select-String -Pattern 'errorText\(').Count
```

结果：`errorText(` **115 处 / 31 个文件**；裸错误对象/`.message` 直拼的**用户可见**路径 **7 处**（AppV2:186、AppV2:558、ConnectionConfigPanel:95、ConnectionConfigPanel:419、CampaignVariablesTab:149、InspectorDrawer:48、useMessageVariants:327）。其中只有 `AppV2.vue:186,558` 会真正渲染 `[object Object]`（插值整个对象）；其余 5 处取 `e.message`，对 Tauri DTO `{type,message}` 恰好可用 → 归为**规约漂移（P3）**而非缺陷。

**(g) debug/config 面板 handler 级普查**（catch 块 + 后继 4 行全量导出）

```powershell
foreach ($f in @('src\components-v2\debug\LogPanel.vue', …)) { $l = Get-Content $f
  for ($i=0; $i -lt $l.Count; $i++) { if ($l[$i] -match 'catch') { $l[$i..([Math]::Min($i+4,$l.Count-1))] } } }
```

结果：`AgentProfileManager.vue` 8 个写入/读取 handler **全部** `errorMsg + errorText` 且 `saving/loading` 在 `finally` 复位（干净）；`ConnectionConfigPanel.vue` 10 个 catch 中 8 个合规，2 处裸 `e.message`（95、419）；`PresetPanel.vue` 10 个 catch 全部 `alertDialog + errorText`、`saving` 在 `finally`，唯一例外 `handleImportAsModules`（217-224）无 busy 防重；`LogPanel.vue` 4 个 catch 中 `loadLogs`（127-129）只 `console.error` → 失败时列表空；`PluginEventLog.vue` 3 个 catch（50/60/104）为格式化兜底与剪贴板静默；`PromptHookAuditLog.vue` 3 个 catch（71 格式化、100 有提示、115 剪贴板静默）；`PipelineTracePanel.vue` **无任何 async/await/错误分支**（纯 props 展示，`activeStage` 一个 ref）。

**(h) 测试套件基线**

```powershell
@(Get-ChildItem tests -Recurse -File -Filter *.test.mjs).Count                       # 93
@(Get-ChildItem tests\components-v2 -Recurse -File -Filter *.test.mjs).Count          # 27 (vitest)
@(Get-ChildItem tests -Recurse -File -Filter *.spec.mjs).Count                        # 5  (playwright)
```

结果：`npm test`（`package.json:7`）跑 `tests/*.test.mjs` 54 + `tests/stores/*` 4 + `tests/composables/*` 8 = **66 个 node 套件**；`npm run test:ui` 跑 **27 个 vitest 套件**（`vitest.config.mjs` include = `tests/components-v2/**`）；playwright 5 个 spec（`ui-smoke` / `mobile-chrome` / `theme-palettes` / `csp-inheritance` / `workbench-motion`）。**注意 `npm test` 不含 vitest**，Lead 验收需跑 `npm test && npm run test:ui`。

### 1.3 抽样/逐行覆盖度声明

- **逐行**：ui 23 基元、campaign 12 文件、design/writing 主路径、stores 4 个、adapter 5 个。
- **关键路径**：AppV2 的 onMounted / 开场壳 / 日志转发 / 视图路由，composables 的错误与 loading 路径。
- **handler 级**：debug/config 7 个文件（见 §1.2(g)）。
- **未做**：`design/writing/ProcessTimeline.vue`（纯展示，仅 props→class 映射）、`design/*Demo.vue`、`components/base/BaseDropdown.vue`（传递死引用，未展开）、`useCharacterImport.js` / `useMvuStatusPanel.js` 全文。

---

## 2 结论摘要

**P0 = 0，P1 = 14，P2 = 21，P3 = 12（合计 47 条）。**

三句判断：

1. **主流程是通的，"可用性"问题集中在"失败不可见"这一族**：写作面（意图→流水线→草稿→采纳小票→变体）与 campaign 面（开档→实例→变量→世界书→任务→摘要→导出）我逐环核对了 props/emits/wrapper 契约，**没有发现断链**（`WritingScreen.vue:100` 的历史 payload 丢失 bug 已修，`MessageItem.vue:116` ↔ adapter `saveVariant` 返回值契约一致，16 个写作事件与 11 个 campaign 事件全部在 adapter/父级有监听者）；真正的风险是**加载/写入失败被吞成"空列表"或"无反应"**（F-09/F-10/F-11/F-13/F-19）与**写操作失败后界面显示成功**（F-12/F-25/F-26），用户会误判数据丢失或改动已保存。
2. **中文输入法下写作框会误提交（F-01，本域最该先修的一条）**：`design/writing/ComposerBar.vue:73` 用 `@keydown.enter.exact.prevent="submit"`，全仓 `isComposing`/`compositionend` 命中 0 处 —— 用拼音输入时按回车确认候选词会直接触发 `submit()` 并清空输入。
3. **冗余与"实现但未接线"并存**：23 个 ui 基元里 **9 个零引用/仅测试引用**，而生产页面手写同款（F-18）；`components-v2/writing/**` 8 个组件整体死代码（文档仍把它当作写作面，见 F-40）；`tauri-api.js` 21 个孤儿 wrapper（F-30）。这些不影响今天的功能，但**任何"按文档改 ui 基元"的后续工作都会改到死代码上**。

P1 里三条最应该立刻复核：F-01（IME 误提交）、F-02（采纳失败静默）、F-41（审计面板展开 100% 抛错，生产调试抽屉里唯一可读的 prompt hook 详情永远是坏的）、以及 F-07+F-08（`likely_completed` 任务既无确认入口又把状态渲染成 `NaN%`）。

---

## 3 发现清单

严重度按任务书定义：P0 = 主流程不可用/安全/数据损坏或与声明严重不符；P1 = 组件契约错误、交互缺陷、状态不一致、契约不符；P2 = 未处理的边界/不良错误处理/明显冗余；P3 = 命名/注释/文档漂移。类别码：A 目标完成度、B 逻辑正确性、C 死代码、D 错误处理、E 测试缺口、F 文档漂移。

### P1

#### F-01 中文输入法下按回车确认候选词会直接提交写作意图
- 严重度：P1　类别：B 交互缺陷
- 位置：`frontend/src/design/writing/ComposerBar.vue:73`
- 证据：

```html
        rows="1"
        class="composer-textarea min-w-0 min-h-[3.5rem] max-h-36 flex-1 resize-none ..."
        @keydown.enter.exact.prevent="submit"
        @input="$event.target.style.height='auto'; $event.target.style.height=$event.target.scrollHeight+'px'"
      ></textarea>
```

  配套事实（普查口径：`Select-String -Path src -Recurse -Pattern 'isComposing|compositionstart|compositionend'`）：**整个 `frontend/src` 命中 0 处**；`submit()`（`ComposerBar.vue:26-30`）在通过校验后 `emit('start-writing', intent.value)` 并 `intent.value = ''`（清空输入）。
- 影响：Vue 的按键修饰符**不过滤 IME 组字状态**（`keydown` 在组字回车时仍以 `isComposing: true` / `keyCode 229` 派发）；本应用的主要输入语言是中文，用户在拼音候选态按回车"选词"会被当成"发送"——发出的是半截意图（甚至只是拼音串），且输入框被清空、随即触发一次真实的 LLM 写作调用（浪费一次生成 + 上下文污染）。写作框是产品的第一入口，故列为 P1。
- 建议：改为 `@keydown.enter.exact="onEnter"`，`onEnter(e) { if (e.isComposing || e.keyCode === 229) return; e.preventDefault(); submit() }`；同类 `@keyup.enter` 的 5 个调用点（`MetaChat.vue:199`、`CampaignTasksTab.vue:136`、`NewCampaignForm.vue:81`、`CampaignPanel.vue:510`）虽在 keyup 阶段风险低，也建议一并加守卫。**需要真机验证一次（我无法运行 UI）**。
- 置信度：已核实（代码事实）/高（机理与触发条件）；「IME 行为在我的环境未实测」这一限制请在验收时确认。

#### F-02 「采纳」失败被静默吞掉，用户看不到任何反馈
- 严重度：P1　类别：D 错误处理 / B 状态不一致
- 位置：`frontend/src/composables/useMessageVariants.js:282-303`
- 证据：

```js
      } catch (e) {
        const msg = String(e?.message || e || '')
        if (!forceAccept && /质量门禁|force_accept|Error 级/i.test(msg)) {
          try {
            const ok = await askForceAccept('质量门禁发现 Error 级问题。…', { title: '强制采纳确认', kind: 'warning' })
            if (ok) { return handleAcceptVariant({ nodeId, forceAccept: true, selectedMutationIndices: null, skipReceipt: true }) }
          } catch (dialogErr) { console.error('强制采纳确认失败:', dialogErr) }
        }
        console.error('采纳失败:', e)
      }
```

- 影响：质量门禁以外的任何失败（存储/校验/流水线/权限）只写 `console.error`。用户点了「采纳此版」或「确认采纳」后**界面毫无变化**：草稿仍是 `draft`、`pendingReceipt` 仍在（`writingStore.clearTurnReceipt()` 只在成功分支执行），也没有任何提示说明为什么没采纳。同一函数的兄弟路径（:246 小票拉取失败、:355 及之后的重试）都用了 `alertDialog(… + errorText(e))`，此处是唯一的例外；`MessageItem.vue:335-349` 的「确认采纳」按钮也没有本地 pending 态可依赖。
- 建议：`catch` 末尾补 `await alertDialog('采纳失败: ' + errorText(e))`（或在 `pendingReceipt.retry_error` 之外新增 `accept_error` 供 `MessageItem.vue:325-327` 渲染；后者已有渲染位，改动更小）。
- 置信度：已核实/高。

#### F-03 开场卡壳加载状态是死状态，加载期间没有任何 UI
- 严重度：P1　类别：B 状态不一致（声明与实现不符）
- 位置：`frontend/src/AppV2.vue:216`（声明）、`:434`（置 true）、`:449`（置 false）
- 证据：

```js
const cardShellLoading = ref(false)      // :216
…
  try {
    cardShellLoading.value = true        // :434
    const manifest = await …
  } finally {
    cardShellLoading.value = false       // :449
  }
```

  模板中 `cardShellLoading` **零读取**（`Select-String -Path src\AppV2.vue -Pattern 'cardShellLoading'` 仅命中上述 3 行 + 无模板行）；`showCardShellOpening`（`:241-246`）只依赖 `openingUrl`/`openingArmed`/消息数，不看加载态。
- 影响：开场壳（开档后第一屏的卡壳渲染）在清单拉取/URL 解析期间 `openingUrl` 为空 → `showCardShellOpening=false` → 屏幕是空白写作面，用户无法区分"在加载"与"没有开场"；加载失败（`refreshCardShellManifest` catch 只 `console.error`）同样无声。这是"组件可用性"意义上的**声明了加载态但从不呈现**。
- 建议：把 `cardShellLoading` 接进开场区（`v-if="cardShellLoading"` 渲染 `ui/LoadingState` 文案"正在准备开场…"），或在 `showCardShellOpening` 增加 loading 分支；并在失败时给出可重试提示。
- 置信度：已核实/高。

#### F-04 日志面把错误对象裸插值，用户看到 `[object Object]`
- 严重度：P1　类别：D 错误处理 / F 规约漂移
- 位置：`frontend/src/AppV2.vue:186`、`frontend/src/AppV2.vue:558`
- 证据：

```js
    console.error('加载侧栏插件失败:', e)
    logAppendFrontend('error', `loadSidebarPlugins: ${e}`).catch(() => {})     // :186
…
    logAppendFrontend('warn', `开场选择落库失败: ${e}`).catch(() => {})        // :558
```

  对照正确写法（同文件 `:553` 的邻近分支）：`applyCampaignOpening` 失败路径用的就是 `errorText(e)`；仓库硬规约见 `CLAUDE.md`「前端用户可见错误串一律走 `frontend/src/utils/errorText.js`…新代码禁止 `'失败: ' + e` 直拼」。
- 影响：这两条会进入用户可见的日志面板（`debug/LogPanel.vue` 渲染后端 LogStore），插件加载失败与开场落库失败会显示成 `loadSidebarPlugins: [object Object]` —— 与 2026-08-31 验收记录过的 `[object Object]` 缺陷同类，且诊断价值归零。
- 建议：`logAppendFrontend('error', \`loadSidebarPlugins: ${errorText(e)}\`)`，同理改 :558。
- 置信度：已核实/高。

#### F-05 `ui/DataList` 选中态契约恒为 false（`activeKey` 语义失效）
- 严重度：P1　类别：B 逻辑正确性 / C 死代码
- 位置：`frontend/src/components-v2/ui/DataList.vue:11-14`（另 `:31`、`:34` 消费）
- 证据：

```js
function isActive(item, index) {
  if (props.activeKey && item && props.activeKey in item) return false
  return false
}
```

  消费点：`:31` `:class="{ 'bg-accent-soft !border-accent': isActive(item, index) }"`、`:34` 传给 `#item` 插槽的 `:active="isActive(item, index)"`；`activeKey` 的声明注释（`:4`）写的是「选中态:传一个 id 字段名,或用 activeItem 引用对比」（**`activeItem` 这个 prop 不存在**）。
- 影响：两个分支都返回 `false`，选中高亮与 `active` 插槽参数永远不成立。唯一调用点 `campaign/CardLibrary.vue:105` 特意传了 `active-key="id"`（它只在 `:29` 的 `:key` 上生效），自己用 `expandedCardId` + ▲▼ 绕过。今天无用户可见故障，但这是**文档化契约失效 + 无测试保护**（`tests/components-v2/**` 无 DataList 用例）——后续任何按注释接线的新调用点都会踩空。
- 建议：实现真实语义（新增 `selectedKey` 或在 DataList 内部维护 `activeIndex`），或删掉 `activeKey` 的选中语义与注释，仅保留 key 用途。
- 置信度：已核实/高（子代理 A 发现，本域复核代码）。

#### F-06 `ui/DataList` 的 `select` 事件在生产不可达且无人监听
- 严重度：P1　类别：B 契约断链
- 位置：`frontend/src/components-v2/ui/DataList.vue:32` ↔ `frontend/src/components-v2/campaign/CardLibrary.vue:105-108`
- 证据：

```html
      @click="emit('select', item, index)"          <!-- DataList.vue:32 -->
```

```html
      <DataList :items="cards" active-key="id">      <!-- CardLibrary.vue:105 -->
        <template #item="{ item: card }">
          <div class="flex items-center gap-3" @click.stop="toggleCard(card)">
```

- 影响：唯一调用点没有 `@select` 监听器，插槽内容又用 `@click.stop` 阻断冒泡 → `select` 既不可达也无监听者；`item.label || item.name || JSON.stringify(item)` 的兜底渲染（`:35-37`）在生产同样不可达。列表基元的"选择"能力实际只被一个调用方用自制状态绕过，契约形同虚设。
- 建议：二选一——(a) 让 DataList 成为受控列表（暴露 `active`/`select` 并让 CardLibrary 接线，去掉 `@click.stop`）；(b) 删除 `select`/`activeKey`/兜底渲染，把 DataList 降级为"带空态的列表容器"。
- 置信度：已核实/高。

#### F-07 「可能完成」的任务在 UI 上被永久锁死，没有任何确认入口
- 严重度：P1　类别：A 目标完成度 / B 契约不符
- 位置：`frontend/src/components-v2/campaign/CampaignTasksTab.vue:173-186` ↔ `crates/domain/src/story_task.rs:232`
- 证据：

```html
      <template #row-action="{ row }">
        <div class="flex flex-col gap-1">
          <Button
            v-if="row.status !== 'completed' && row.status?.likely_completed == null"
            variant="default"
            size="sm"
            @click.stop="handleCompleteTask(row.id)"
          >完成</Button>
```

  后端契约（`story_task.rs` 注释原文，`:232` 附近）：「LikelyCompleted(>0.8) 的不自动注入…**由前端提示用户确认**」；全仓 `grep likely_completed` 只命中本文件与 `utils/taskStatus.js`，**不存在任何"确认完成"入口**。
- 影响：后处理 Agent 判定"可能完成"的任务，其「完成」按钮被 `v-if` 隐藏，用户唯一能点的是「放弃」——等于被迫销毁一条本可确认完成的伏笔；任务会一直留在 `likely_completed` 状态继续被注入导演提示词（`is_injectable()` 只含 Pending/Active，故不会再注入，但界面上也无法推进）。
- 建议：对 `status?.likely_completed != null` 的行显示「确认完成 / 这是误判」两个动作（前者 `completeTask`、后者 `abandonTask` 或新增 reject 语义），并把置信度展示出来。
- 置信度：已核实/高（子代理 B 发现，本域复核行号与后端注释）。

#### F-08 任务状态渲染成「可能完成 (NaN%)」——前端与后端 DTO 形状不一致
- 严重度：P1　类别：B 逻辑正确性 / E 测试缺口（夹具与真实 DTO 不符）
- 位置：`frontend/src/utils/taskStatus.js:3` ↔ `crates/domain/src/story_task.rs:30-44`
- 证据：

```js
export function taskStatusText(status) {
  if (typeof status === 'string') return status
  if (status?.likely_completed != null) return `可能完成 (${Math.round(status.likely_completed * 100)}%)`
  return JSON.stringify(status)
}
```

```rust
/// 任务状态（LikelyCompleted 是软状态，需用户确认）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    …
    LikelyCompleted { confidence: f32 },
```

  `#[serde(rename_all="snake_case")]` 是**外部标签**枚举，结构体变体序列化为 `{"likely_completed":{"confidence":0.5}}`；`crates/tauri-app/src/commands/memory.rs:259` 自注同一事实（`// TaskStatus 序列化为 "pending"/"active"/{"likely_completed":...}/…`）。因此 `status.likely_completed * 100` = `{confidence:0.5} * 100` = `NaN`。
- 影响：任务表格的状态徽章显示「可能完成 (NaN%)」（`CampaignTasksTab.vue:164-166`）；`taskStatusClass()`（`taskStatus.js:7-14`）对对象一律走 `s=''` → 落 `bg-warn/10 text-warn`，所以颜色也是兜底黄。`tests/task-status.test.mjs:14` 用扁平形状 `{likely_completed:0.756}` 把**错误形状冻结成契约**，所以门禁不会发现（这是"测试存在但测的是错形状"的典型）。
- 建议：`const c = status?.likely_completed?.confidence ?? status?.likely_completed`（兼容数字），并用后端真实形状重写单测夹具；`taskStatusClass` 同步处理对象分支。
- 置信度：已核实/高。

#### F-09 「设为当前活动」失败零提示，且失败会被外层误报成「创建失败/导入失败」
- 严重度：P1　类别：D 错误处理 / B 状态不一致
- 位置：`frontend/src/components-v2/campaign/CampaignPanel.vue:190-205`（另 `:182`、`:431-442` 调用）
- 证据：

```js
async function handleSetActive(campaignId) {
  await setActiveCampaign(campaignId)
  activeCampaign.value = await getActiveCampaign()
  // 同步到 store
  campaignStore.activeCampaign = activeCampaign.value
  // 回填活动 Turn 质量报告（若有）
  try { … } catch (e) { console.error('getActiveTurnQuality:', e) }
  emit('campaign-changed', activeCampaign.value)
}
```

  `setActiveCampaign` 是唯一未被 try/catch 包住的一步（后半个 try 只护 `getActiveTurnQuality`）；后端 `set_active_campaign` 会因 `validate()?` / `save_active_pointer(...)?` 失败返回错误。调用点：`handleCreateCampaign`（`:182`，在它自己的 try 内 → 报「创建失败」）、`handleImportBundle`（`:435`，同 → 报「导入失败」）、`design/campaign/CampaignScreen.vue:219` 的「设为当前活动」按钮（直达函数 → **零提示**）。
- 影响：两种误导：(1) 用户点「设为当前活动」失败时界面无任何反馈，且活动指针仍是旧的；(2) 新建档/导入包**本体已成功入库**，却因为切指针失败被外层 catch 报成「创建失败/导入失败」→ 用户重复创建/重复导入，产生重复档（`create_campaign` 非幂等）。
- 建议：`handleSetActive` 内 try/catch + `alertDialog('切换活动失败: ' + errorText(e))` 并 `return false`；`handleCreateCampaign`/`handleImportBundle` 据此区分「本体成功、切指针失败」，后者只作警告。
- 置信度：已核实/高（子代理 B 发现，本域复核代码）。

#### F-10 活动列表加载失败伪装成「还没有活动档」
- 严重度：P1　类别：D 错误处理
- 位置：`frontend/src/components-v2/campaign/CampaignPanel.vue:142-150`（另 `:108-124` 的 onMounted 裸 await）
- 证据：

```js
async function refreshCampaigns() {
  loadingCampaigns.value = true
  try {
    // 有选中角色卡时按卡过滤；否则列出全部（对齐 design 双栏「我的活动」）
    campaigns.value = await listCampaigns(selectedCardId.value || null)
  } finally {
    loadingCampaigns.value = false
  }
}
```

  只有 `finally` 没有 `catch`，而 `onMounted`（`:108-124`）里 `await getActiveCampaign()` / `await refreshCampaigns()` 均裸 await。
- 影响：任一步失败 → 未处理 rejection、`campaigns` 保持 `[]`、`loadingCampaigns=false` → 侧栏渲染 `CampaignScreen.vue:164` 的空态「还没有活动档」。**用户会以为整局档丢了**（数据其实完好）。此外 `onMounted` 里 `selectedCardId.value = savedCardId`（`:125`）之后的逻辑不再执行，后续新建档会被判成「未选卡」。
- 建议：`refreshCampaigns` 补 `catch` 落到 error 状态，`CampaignScreen` 增加错误态 + 重试；`onMounted` 整体 try/catch。
- 置信度：已核实/高（子代理 B 发现，本域复核代码）。

#### F-11 角色卡库加载失败被 `.catch(() => {})` 吞掉，空态与"数据丢失"不可区分
- 严重度：P1　类别：D 错误处理
- 位置：`frontend/src/components-v2/campaign/CardLibrary.vue:32-42`
- 证据：

```js
async function refreshCards() {
  loadingCards.value = true
  try {
    cards.value = await listCards()
  } finally {
    loadingCards.value = false
  }
}

// 挂载即加载(与原 CampaignPanel onMounted 调 refreshCards 一致)
refreshCards().catch(() => {})
```

  本文件没有 `error` ref（`Select-String -Path … -Pattern 'error'` 仅命中 `errorText` 的调用点），模板只有 loading / empty / list 三态。
- 影响：加载失败 → `cards=[]` → 渲染 `EmptyState「还没有角色卡」`。用户看到的是"卡库被清空"，真实数据仍在磁盘；`CampaignPanel.vue:431` 的导入流程也会因 `refreshCards()` 抛出而走进「导入失败」分支（见 F-09 同类问题）。
- 建议：加 `error` ref + `catch (e) { error.value = errorText(e) }`，模板在列表区渲染错误与「重试」按钮（可直接用现成但零引用的 `ui/ErrorState.vue`，见 F-18/F-19）。
- 置信度：已核实/高（子代理 B 发现，本域复核代码）。

#### F-12 写卡工作室「保存产物」失败后仍继续编译导入，编辑静默不生效
- 严重度：P1　类别：B 逻辑正确性 / D 错误处理
- 位置：`frontend/src/components-v2/campaign/CardStudio.vue:454-474`（saveArtifacts）↔ `:547-559`（importCompiled）
- 证据：

```js
async function saveArtifacts() {
  if (!project.value) return
  busy.value = true
  try {
    …
    project.value = await cardstudioUpdateArtifacts(project.value.id, artifacts)
    …
    statusText.value = '产物已保存'
  } catch (e) {
    await alertDialog('保存失败: ' + errorText(e))
  } finally {
    busy.value = false
  }
}
```

```js
async function importCompiled() {
  if (!project.value) return
  await saveArtifacts()        // :549 —— 失败也照样往下走
  busy.value = true
  try { … statusText.value = `已导入角色卡：…` … }
```

- 影响：`saveArtifacts` 成功/失败都返回 `undefined`，调用方无法感知；保存失败时用户已经看到「保存失败」弹窗，但紧接着**同一操作继续编译导入旧产物**，并以 `statusText`/`emit('imported')` 宣告「已导入角色卡」。用户在工作室里的编辑**静默没有进入新卡**。同模式还有 `completeManual`（`:478`）、`runStage`/`runChecks`/`runReview`/`prefillFromNovel`/`exportStJson`/`exportStPng` 等 7 处（`grep -c 'await saveArtifacts()'` = 8）。
- 建议：`saveArtifacts` 成功返回 `true`、失败返回 `false`；所有调用方 `if (!(await saveArtifacts())) return`，并在状态栏明确「产物未保存，已中止」。这也是本域唯一涉及"产出物与用户预期不符"的 P1。
- 置信度：已核实/高（子代理 B 发现，本域复核两段代码）。

#### F-13 会话历史加载失败静默；单个 DTO 缺 `updated_at` 会清空整个历史列表
- 严重度：P1　类别：D 错误处理 / B 健壮性
- 位置：`frontend/src/composables/useConversation.js:74-86`
- 证据：

```js
  async function loadConversationHistory() {
    try {
      const convList = await listConversations()
      if (!convList || convList.length === 0) {
        campaign.conversationHistory = []
        return
      }
      convList.sort((a, b) => b.updated_at.localeCompare(a.updated_at))
      campaign.conversationHistory = convList
    } catch (e) {
      console.error('加载会话历史失败:', e)
    }
  }
```

- 影响：两种失败都表现为"历史为空"（`HistoryScreen`/侧栏渲染空态），用户以为故事记录丢了。(1) `listConversations()` 抛错 → 只 `console.error`；(2) 更隐蔽的是 `:81` 的 `b.updated_at.localeCompare(...)`：**只要有一条会话 DTO 缺 `updated_at`（或为 null）**，`sort` 就抛 TypeError，被同一个 catch 吞掉 —— 由于赋值 `campaign.conversationHistory = convList` 排在 sort 之后，**整个历史列表一条都不会显示**（不是跳过坏数据，而是全灭）。`onMounted`（`AppV2.vue`）随后还会因为"历史为空"而走不到"从最近会话恢复工作上下文"的分支。
- 建议：`sort` 前做 `const ts = (c) => c?.updated_at || c?.created_at || ''`，先赋值再排序（或排序失败时回退未排序列表）；catch 里给出用户可见提示。
- 置信度：已核实（代码与失败传播路径）/高；「后端 DTO 是否真会缺 `updated_at`」未核实 → 该触发条件为**疑似**。

#### F-41 提示词钩子审计面板的「展开详情」100% 抛 `ReferenceError`
- 严重度：P1　类别：B 逻辑正确性（功能完全失效）
- 位置：`frontend/src/components-v2/debug/PromptHookAuditLog.vue:37-43`（另 `:210-216` 消费 `expanded`）
- 证据：

```js
function recordKey(record) {
  return `${record.pluginId || ''}|${record.correlationId || ''}|${record.recordedAt || ''}`
}
function toggleExpand(row) {
  const key = recordKey(record)          // ← 形参是 row，这里引用了未声明的 record
  const next = new Set(expanded.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  expanded.value = next
}
```

  全文件 `record` 只作为 `recordKey`/`copyRecord` 的**形参**出现（`Select-String '(?<![\w.])record(?![\w])'` 命中 :30/:32 注释、:34/:35/:38/:108/:110），没有模块级 `record` 变量。
- 影响：该面板经 `components-v2/shell/InspectorDrawer.vue:16,75` 挂在生产调试抽屉里（`AppV2.vue:44,923`），用户点表格行内「展开」按钮即抛 `ReferenceError: record is not defined`，`expanded` 永不变化 → `v-show="expanded.has(recordKey(row))"`（`:213`）的详情块永不出现：**审计详情（输入/输出摘要、错误）在 UI 上完全不可读**，而这正是 prompt hook 排障唯一的界面。组件无组件级测试（`frontend/tests` 只有 `prompt-hook-audit.test.mjs` 测 `utils`），CI 不拦。
- 建议：改为 `const key = recordKey(row)`，并补一条 mount 级用例（点展开断言详情出现）。
- 置信度：已核实/高（本域复核代码 + 标识符普查）。

### P2

#### F-14 `ui/Overlay` 的 `showClose` 在传了 `title` 时被静默忽略（双 ×）
- 严重度：P2　类别：B 契约错误
- 位置：`frontend/src/components-v2/ui/Overlay.vue:139-152`、`:153-155`
- 证据：

```html
              v-if="title"
              class="sf-toolbar flex items-center justify-between px-4 border-b border-line"
            >
              <h2 class="min-w-0 truncate text-sm font-semibold text-ink">{{ title }}</h2>
              <button type="button" …（× 关闭键，无条件渲染）
```

```html
            <div
              v-else-if="showClose"
```

  prop 注释（`:23`）原文：「子组件自带关闭键时传 false，避免双 ×」。
- 影响：`showClose` 只约束"无 title"分支；带 title 的 4 个生产调用点（`campaign/NewCampaignForm.vue:57`、`campaign/CampaignInstancesTab.vue:426`、`meta/MvuAnalyzer.vue:219,300`）目前都没传 `show-close`，所以今天"碰巧"不出现双 ×；一旦有人按 prop 注释传 `:show-close="false"`，它会**静默失效**。`tests/components-v2/overlay.test.mjs:26` 的回归用例只覆盖无 title 路径，缺陷无保护。
- 建议：title 分支的 × 也加 `v-if="showClose"`，并补一条带 title 的回归用例。
- 置信度：已核实/高（子代理 A 发现，本域复核行号）。

#### F-15 `ui/Overlay` 同一布尔有两个 prop，关闭时三事件齐发
- 严重度：P2　类别：B 契约冗余 / 状态一致性风险
- 位置：`frontend/src/components-v2/ui/Overlay.vue:19-20`、`:32-44`
- 证据：

```js
  show: { type: Boolean, default: false },
  modelValue: { type: Boolean, default: null },
```

```js
  set(val) {
    emit('update:show', val)
    emit('update:modelValue', val)
    if (!val) emit('close')
  },
```

- 影响：没有单一事实源；`modelValue` 默认 `null` 被当作"未提供"哨兵（`isOpen` 回落 `show`），于是"只绑 `:show`"的调用方（`shell/PanelHost.vue:21`）会收到没人处理的 `update:modelValue`/`close`，而"用 `v-model` 且初值 `null`"的调用方永远打不开。当前 5 个调用点恰好都在工作，属潜在陷阱。
- 建议：只保留一个 prop（推荐 `modelValue` + `update:modelValue`，`show` 作 deprecated 别名），`close` 仅表达"请求关闭"。
- 置信度：已核实（代码事实）/中（影响面未触发）。

#### F-16 `ui/Menu` 三处死契约，而生产页面内联了同一套 headlessui Menu
- 严重度：P2　类别：C 死代码 / B 契约错误
- 位置：`frontend/src/components-v2/ui/Menu.vue:12`、`:51-64`（零引用）↔ `frontend/src/design/campaign/CampaignScreen.vue:114-119`
- 证据：

```html
        <!-- 默认 slot：渲染 MenuItem 列表，每项可拿到 active 与 select(key) -->
        <slot>
          <!-- 未提供 slot 时，回退渲染 label-only 菜单项 -->
          <MenuItem v-slot="{ active }">
            <button type="button" :class="['w-full text-left px-3 py-2 …', active ? 'bg-surface-2 text-ink' : 'text-ink']">
              {{ label }}
            </button>
```

  `defineEmits(['select'])`（`:12`）在全文件中**从未 emit**；默认 slot 不向调用方传任何 props（注释却写"可拿到 active 与 select(key)"）；label 兜底项没有 `@click`。唯一引用它的只有 `tests/components-v2/ui-components.test.mjs`。
- 影响：叠加零引用，该组件实际不可用；生产改用内联 `@headlessui/vue` 的 `Menu/MenuButton/MenuItems`（`CampaignScreen.vue:114-119`，带 `aria-label`、`min-h-11`、`MenuItem disabled`，a11y 反而更全）→「基元没人用 + 页面手写同款」。
- 建议：删除 `ui/Menu.vue`（或修好 slot/emit 后把两处菜单迁进来）。
- 置信度：已核实/高。

#### F-17 `ui/DiffView` 用"按下标逐行"当 diff，插入一行会把后续全部标成删+增
- 严重度：P2　类别：B 算法正确性 / C 冗余
- 位置：`frontend/src/components-v2/ui/DiffView.vue:10-24`
- 证据：

```js
  const beforeLines = props.before.split('\n')
  const afterLines = props.after.split('\n')
  const max = Math.max(beforeLines.length, afterLines.length)
  for (let i = 0; i < max; i += 1) {
    const b = beforeLines[i]
    const a = afterLines[i]
    if (b === undefined && a !== undefined) result.push({ type: 'added', text: a })
    else if (a === undefined && b !== undefined) result.push({ type: 'removed', text: b })
    else if (b !== a) { result.push({ type: 'removed', text: b }); result.push({ type: 'added', text: a }) }
    else result.push({ type: 'unchanged', text: a })
  }
```

- 影响：`before="a\nb"` / `after="x\na\nb"` 会输出 `removed a / added x / removed b / added a / added b`——首行或中段插入即全量错标，用作 patch/schema 预览会严重误导。另 `lines()` 在模板 `v-for` 里直接调用（`:50`），每次渲染重算全文，无缓存、无行号。该组件**零生产引用**（唯一引用是 `ui-components.test.mjs:113-121`），而最需要 diff 的 `meta/PatchPreview.vue:15` 注释明确说"用列表形式渲染（非 before/after diff）"→ 实现了但从未接线，且算法也不足以支撑真实预览。
- 建议：改 LCS/Myers 或至少先裁公共前后缀；`lines()` 用 computed；接进 PatchPreview/MvuAnalyzer 的 apply 预览，或删除组件与测试。
- 置信度：已核实（算法事实）/高。

#### F-18 9 个注释齐全的基元零引用/仅测试引用，生产页面各自手写同款
- 严重度：P2　类别：C 死代码 / 冗余
- 位置与同款实现：

| 基元（引用情况） | 生产里的手写同款 |
| --- | --- |
| `ui/Progress.vue`（0） | `components-v2/writing/StreamingMessage.vue:101-105`、`components-v2/st/MvuStatusBar.vue:100-106`（三份实现、三种高度 h-1/h-1.5/h-2） |
| `ui/Checkbox.vue`（0） | 7 处裸 `<input type="checkbox">`：`CampaignVariablesTab.vue:435-443`/`:357`、`CampaignWorldInfoTab.vue:268`/`:328`/`:332`、`CampaignInstancesTab.vue:383`、`CardStudio.vue:726`、`design/writing/MessageItem.vue:312` |
| `ui/ErrorState.vue`（0，含 retry 插槽） | 各面板手写一行 `<div class="…text-err…">加载失败: {{ error }}</div>`，多数没有重试入口 |
| `ui/Toast.vue`（0） | v2 应用层没有 toast 通道（`--z-toast` 仅域6 的 `CardShellFloatingStatus.vue:41` 用；插件侧另走 `plugin-bridge.js` 注入的 `window.toastr`） |
| `ui/SegmentedControl.vue`（仅测试） | `CampaignInstancesTab.vue:434-453` 逐字复刻选中态 `'bg-surface text-ink font-medium shadow-card'` |
| `ui/Slider.vue`（0） | 全仓 `type="range"` 只出现在 `Slider.vue:14`（无重复实现，纯冗余） |
| `ui/Tooltip.vue`（0） | 被各处原生 `title=` 替代 |
| `ui/Tooltip.vue` 能力缺口 | 即使接线也不给触发器加 `aria-describedby`（只有 `role="tooltip"`），读屏不播报 |

- 影响：不是"少一个 UI 组件"，而是**维护面分裂**：同一视觉/交互有三份实现，改一处不会同步；后续开发按文档去改基元会改成死代码。同时 `Checkbox`/`ErrorState` 手写点普遍缺失基元本可提供的 `aria-label` 关联、`focus:ring` 与"重试"入口。
- 建议：逐项二选一——(a) 补 `ariaLabel`/`indeterminate` 等缺口后替换生产手写点；(b) 删除基元与对应测试。建议优先级：`ErrorState`（用户可见收益最大）> `Checkbox`（a11y）> `Progress`/`SegmentedControl`（一致性）> `Slider`/`Tooltip`（直接删）。
- 置信度：已核实/高（子代理 A 普查 + 本域复核引用数）。

#### F-19 「加载失败」在多个面板被静默成空态，且没有统一的重试入口
- 严重度：P2　类别：D 错误处理 / A 一致性
- 位置：`components-v2/debug/LogPanel.vue:127-129`、`components-v2/campaign/CardStudio.vue:139-146`+`:595`+`:661`、`components-v2/campaign/CampaignKnowledgeTab.vue:111`（错误文案手写）、`components-v2/campaign/CardLibrary.vue:42`（见 F-11）
- 证据：

```js
  } catch (e) {
    console.error('加载日志失败:', e)        // LogPanel.vue:127-129，无用户可见提示
  } finally { if (!quiet) loading.value = false; firstLoad = false; … }
```

```js
async function refreshProjects() {
  loading.value = true
  try { projects.value = await cardstudioListProjects() } finally { loading.value = false }   // CardStudio.vue:139-146
}
… refreshProjects().catch(() => {})                                                            // :595
```

- 影响：日志面板失败显示"无日志"、写卡工作室失败显示 `EmptyState「还没有写卡项目」`（模板 `:661` 只有 `v-if="!loading && projects.length === 0"` 与 `v-else`，**加载中也是空白区**，本文件根本没 import `LoadingState`）。用户在"没有数据"与"读取失败"之间无法区分，且没有重试按钮——而 `ui/ErrorState.vue` 正是为此设计（零引用，见 F-18）。
- 建议：把「加载失败 + 重试」收敛到 `ErrorState`，优先覆盖 campaign 三个 Tab、`CardStudio`、`CardLibrary`、`LogPanel`、`AgentProfileManager`、`PluginPanel`。
- 置信度：已核实（代码）；「用户实际遇到频率」未核实。

#### F-20 `ui/DataTable` 行 key 用数组下标，展开态/复制态会漂移
- 严重度：P2　类别：B 逻辑正确性（触发条件）
- 位置：`frontend/src/components-v2/ui/DataTable.vue:41-45`，消费点 `components-v2/debug/PromptHookAuditLog.vue:188-204`
- 证据：

```html
        <tr
          v-for="(row, ri) in rows"
          :key="ri"
          class="border-b border-line last:border-0 hover:bg-surface-2/60 transition-colors"
        >
```

  `DataTable` 没有 `rowKey` prop（6 个调用点都无法指定稳定键）；`PromptHookAuditLog.vue:188-204` 把 index 当稳定标识：`@click="copyRecord(row, index)"` + `copiedIndex === index ? '✓' : '⧉'`。
- 影响：`CampaignKnowledgeTab.vue:118`、`WorldInfoReadonlyPanel.vue:147` 传的是**过滤后数组**，`LogPanel`/`PromptHookAuditLog` 的列表会持续增长：插入/过滤变化时 DOM 按位置复用 → 「✓ 已复制」标记漂移到别的记录、展开态串行错位。
- 建议：加 `rowKey`（属性名或函数，默认回退 index）并在 `:key` 使用；行内状态改为按行标识存储。
- 置信度：已核实（代码事实）/中（触发条件）。

#### F-21 `ui/Tabs` 在 `modelValue` 非法时静默选中第 0 项
- 严重度：P2　类别：B 契约错误（潜在）
- 位置：`frontend/src/components-v2/ui/Tabs.vue:12-15`
- 证据：

```js
const selectedIndex = computed(() => {
  const idx = props.tabs.findIndex((t) => t.key === props.modelValue)
  return idx >= 0 ? idx : 0
})
```

- 影响：headlessui 会把第 0 个 tab 渲染为 `aria-selected=true` 但不 emit，调用方的 `v-if="activeTab === 'x'"` 内容与选中态不一致，且无告警。5 个调用点初值当前都合法（`InspectorDrawer.vue:22` `'trace'`、`LogPanel.vue:21` `'all'`、`PipelineTracePanel.vue:24` `'director'`、`AgentProfileManager.vue:50` `'list'`、`PresetPanel.vue:29` `'presets'`）→ 属潜在陷阱。
- 建议：`findIndex` 为 -1 时 dev 下 warn 并 emit 首个 key 让父级同步；或渲染空态而不是静默选 0。
- 置信度：已核实（代码事实）/高（当前未触发）。

#### F-22 `ui/Select` 的下拉未 portal，放进 `overflow-hidden` 容器会被裁剪
- 严重度：P2　类别：B 布局缺陷（潜在）
- 位置：`frontend/src/components-v2/ui/Select.vue:66-67`，风险容器 `ui/DataTable.vue:19`、`ui/DataList.vue:18`、`ui/Overlay.vue:170-175`
- 证据：

```html
        <ListboxOptions
          class="absolute left-0 z-[var(--z-overlay)] mt-1 w-full max-h-60 overflow-auto bg-surface border border-line rounded-lg shadow-rise py-1 focus:outline-none"
```

- 影响：headlessui 1.7.23 的 `ListboxOptions` 默认不 portal（本仓未用 `<Portal>`），下拉是按钮旁的 `absolute`；祖先一旦 `overflow-hidden` 即被裁剪。当前 10 个调用点的 Select 都位于表格/列表**之外**的过滤条里（如 `CampaignKnowledgeTab.vue:103/106` 在 DataTable 之前），尚未触发。
- 建议：`<Portal>` 包住 ListboxOptions，或在基元文档里明确"不得放入 overflow-hidden 容器"。
- 置信度：已核实（代码事实）/中（潜在）。

#### F-23 `ui/Toggle` 的 `role="switch"` 按钮没有可访问名称
- 严重度：P2　类别：a11y
- 位置：`frontend/src/components-v2/ui/Toggle.vue:15-18`，调用点 `config/AgentProfileManager.vue:396-401`、`config/PluginPanel.vue:154-158`
- 证据：

```html
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
```

  调用点用**兄弟** `<label class="text-[11px] text-ink-soft block">后处理</label>`（无 `for`，label 无法关联 button）；组件不提供 `label`/`ariaLabel` prop，按钮内只有装饰性 span。
- 影响：读屏只播报"开关，未选中"，不知道开关的是"后处理"还是别的；视觉用户不受影响。
- 建议：Toggle 增加 `label`/`ariaLabel` prop → 输出 `aria-label`；调用点改用 id + `aria-labelledby`。
- 置信度：已核实/高。

#### F-24 写操作缺 busy 防重：重复创建任务、重复导入预设
- 严重度：P2　类别：B 交互缺陷（重复提交）
- 位置：`frontend/src/components-v2/campaign/CampaignTasksTab.vue:60-72`+`:140-146`、`frontend/src/components-v2/config/PresetPanel.vue:217-224`
- 证据：

```html
      <Button variant="primary" size="md" class="flex-1"
        :disabled="!newTaskTitle.trim()"        <!-- 只看标题非空，无 busy -->
        @click="handleCreateTask">创建</Button>
```

```js
async function handleImportAsModules(preset) {
  try { const count = await importPresetAsModules(preset.id); await alertDialog(`已导入 ${count} 条提示词为模块…`) }
  catch (e) { await alertDialog('导入失败: ' + errorText(e)) }
}
```

- 影响：`create_task` 非幂等（每次都新建 id），连点「创建」会产生多条重复任务；行内「完成/放弃」（`:175-186`）同样无行级 busy。`Button.vue:36` 是 `:disabled="disabled || loading"`，即"绑了 `loading` 就防重"——这两处恰好是唯一漏绑的写操作（`CardStudio`/`ConnectionConfigPanel`/`PresetPanel` 其余写操作都绑了 `:loading="saving"`）。`importPresetAsModules` 连点会在导演配置里生成重复模块。
- 建议：加 `creatingTask`/`importingModules` ref 并绑 `:loading` + `:disabled`；行内按钮按 `busyTaskId === row.id` 禁用。
- 置信度：已核实/高。

#### F-25 变量写入失败后输入框仍显示新值（用户以为已保存）
- 严重度：P2　类别：B 状态不一致
- 位置：`frontend/src/components-v2/campaign/CampaignVariablesTab.vue:193-206`、`components-v2/campaign/CampaignInstancesTab.vue:264-277`
- 证据：

```js
async function persist(variable, rawValue, scope, instanceId = null) {
  const type = inferVarType(variable.value)
  const parsed = parseVariableInput(rawValue, type)
  try {
    if (scope === 'campaign') { await setCampaignVariable(props.campaignId, variable.key, parsed) }
    else { await setCharacterVariable(props.campaignId, instanceId, variable.key, parsed) }
    variable.value = parsed
  } catch (e) {
    await alertDialog(`设置变量失败：${errorText(e)}`)
  }
```

  输入控件是 `:value` **单向**绑定（`:368/:371/:378/:448/:451/:458`），不是 `v-model`。
- 影响：失败时弹窗提示了，但 `variable.value` 未更新、DOM 里保留用户输入的新值 → 界面看起来"已保存"，切走再回来才变回旧值；用户无从判断改动是否生效。`CampaignInstancesTab.vue:264-277` 是实例版同款；且其 `else` 分支里 `getCharacterVariables` 抛错也落到同一个 catch，提示会变成"设置变量失败"（其实写已成功、只是回读失败）。
- 建议：失败分支强制回滚控件显示（或先进入 pending 态，成功后再提交值）；把"写入失败"与"回读失败"拆成两条提示。相关：`parseVariableInput` 对 int 输入框清空会 `parseInt('') → NaN → 回退字符串 ''`，变量类型被静默改写（**疑似**，未构造数据验证）。
- 置信度：已核实（渲染层行为）/中高。

#### F-26 世界书路由「选中即写库」，失败后草稿与后端长期不一致
- 严重度：P2　类别：B 状态一致性
- 位置：`frontend/src/components-v2/campaign/CampaignWorldInfoTab.vue:177-186`
- 证据：

```js
async function handleRouteChange(route) {
  if (expandedIndex.value == null) return
  draft.route = route                      // 先改草稿
  try {
    await setCampaignWorldInfoRoute(props.campaignId, expandedIndex.value, route)
    await load()
  } catch (e) {
    await alertDialog('改路由失败: ' + errorText(e))   // 不回滚、不 reload
  }
}
```

  调用点是 Select 的 `@update:model-value`（`:355`），无 busy 标志。
- 影响：失败时 `draft.route` 已是新值且不再与服务端同步，用户下次点「保存修改」会把错误路由一起写回；快速多次切换还会并发写（最后一个响应回来的 `load()` 结果可能覆盖）。
- 建议：失败分支 `await load()`（用服务端真值覆盖草稿）或回滚 `draft.route`；加 `savingRoute` 禁用 Select。
- 置信度：已核实（代码）/中（不一致窗口受时序影响）。

#### F-27 两处死 emit：`CampaignTasksTab.refresh`、`NewCampaignForm.created`
- 严重度：P2　类别：B 契约断链
- 位置：`components-v2/campaign/CampaignTasksTab.vue:66-68`（emit）↔ `CampaignPanel.vue:553-557`（无监听）；`components-v2/campaign/NewCampaignForm.vue:16`+`:45`（emit）↔ `AppV2.vue:1003-1008`（无监听）
- 证据：

```js
    showNewTask.value = false
    await load()
    emit('refresh')          // CampaignTasksTab.vue:66-68；defineEmits(['refresh']) 且在 :68/:78/:90 三处 emit
```

```js
const emit = defineEmits(['update:show', 'close', 'created'])   // NewCampaignForm.vue:16
… if (ok) { emit('created'); emit('update:show', false) }        // :44-47
```

- 影响：当前靠本 tab 自己的 `load()` 兜住、composable 自己关表单，所以不表现为 bug；但契约语义已失效——后续若把刷新上移到父级、或依赖 `created` 刷新档列表，会静默不生效（对比：`CampaignInstancesTab` 在 `CampaignPanel.vue:536` 是有 `@refresh` 的）。
- 建议：要么在父级补 `@refresh`/`@created` 并让子组件不再自刷新，要么删除 emit 与 `defineEmits` 项。
- 置信度：已核实/高。

#### F-28 `utils/consoleForwarding.js` 无人引用，生产内联了同一实现；`JSON.stringify` 遇循环对象会抛
- 严重度：P2　类别：C 死代码 / E 测试覆盖错位 / B 健壮性
- 位置：`frontend/src/utils/consoleForwarding.js:11-21` ↔ `frontend/src/AppV2.vue:191-201`（生产内联版）
- 证据：

```js
export function setupConsoleForwarding(logFn) {          // utils/consoleForwarding.js
  const levels = { log: 'info', warn: 'warn', error: 'error', debug: 'debug' }
  for (const [method, level] of Object.entries(levels)) {
    const original = console[method]
    console[method] = (...args) => {
      original.apply(console, args)
      const msg = args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ')
      logFn(level, msg)
```

```js
function setupConsoleForwarding() {                      // AppV2.vue:191-201（同名，独立实现）
  …
      const msg = args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ')
      logAppendFrontend(level, msg).catch(() => {})
```

  引用普查：`Select-String -Path src -Recurse -Pattern 'consoleForwarding'` 只命中 `AppV2.vue` 的注释与其自身；该 util 的唯一消费者是 `tests/console-forwarding.test.mjs`（5 个用例全绿），而**生产跑的是未被任何测试覆盖的内联副本**。
- 影响：(1) 测试保护的实现与生产实现不是同一份，测试给人虚假的安全感；(2) 两者都用 `JSON.stringify(a)` 序列化非字符串参数——**循环引用对象会抛 `TypeError: Converting circular structure to JSON`，且是在被 patch 过的 `console.error` 内同步抛出**，把"打日志"变成"调用点崩溃"（Vue 响应式代理、DOM 节点、事件对象都容易循环）。测试没有覆盖循环对象分支。
- 建议：AppV2 改为 `import { setupConsoleForwarding } from './utils/consoleForwarding.js'` 并传入 `(level, msg) => logAppendFrontend(level, msg).catch(() => {})`；util 内加 `try/catch` + 降级（`String(a)` / 只报 `[Unserializable]`）；补一条循环对象的用例。
- 置信度：已核实/高（死代码与重复是实现事实；循环对象崩溃为机理推导，触发条件未实证 → **疑似**）。

#### F-29 角色识别入口未回退 `source_character_id`，老数据必然失败且会禁用整页按钮
- 严重度：P2　类别：B 兼容性
- 位置：`frontend/src/components-v2/campaign/CardLibrary.vue:57-58`（另 `:51-55` 按钮文案用它）
- 证据：

```js
async function handleExtract(card) {
  extractingCardId.value = card.source_character_id          // 未回退 card.id
  try {
    const result = await extractCharacters(card.source_character_id, { force: true })
```

  同文件 `:149` 已承认该字段可能缺失：`card.source_character_id || cardDetail.source_character_id`；`CampaignPanel.vue:271` 的 `openStudioForRevise` 有 `|| card.id` 回退；`CLAUDE.md` 记录"old data falls back to `StoredCharacter.id`"。
- 影响：对没有 `source_character_id` 的存量卡，`extract_characters(undefined)` 必然失败；更糟的是 `extractingCardId` 被设成 `undefined`，使**所有** `source_character_id` 为空的老卡一起进入 `extractButtonText` 的"识别中…"禁用态（`:53`）。
- 建议：`const sourceId = card.source_character_id || card.id`，并让 `extractingCardId` 用同一 id。
- 置信度：已核实（代码路径）/中（是否命中取决于存量数据分布——需 Lead 用真实数据目录确认）。

#### F-30 21 个孤儿 wrapper 与 3 个零调用后端命令（含契约基线盲区）
- 严重度：P2　类别：C 死代码 / E 契约统计缺口
- 位置：`frontend/src/tauri-api.js`（导出 160 个）；`scripts/architecture/backend-baseline.mjs:55-67`
- 证据（口径见 §1.2(e)）：零产品调用方的 21 个 wrapper —— `addWorldInfoEntry`、`cardShellAllowHost`、`cardstudioListStages`、`configureEmbedder`、`deleteCharacter`、`deleteWorldInfoEntry`、`exportStCardPng`、`getActiveAgentProfileConfig`、`getActivePreset`、`getActiveProfile`、`getEmbedConfig`、`listModules`、`listProfiles`、`logGetLlmCall`、`metaClassifyStPreset`、`metaGetConversation`、`saveProfile`、`setActiveProfile`、`updateModule`、`updateWorldInfoEntry`、`updateWorldInfoRoute`（测试引用数全为 0）。
- 影响：(1) 21 个 wrapper 是"看起来有 API、实际无 UI 入口"的假能力面，`CLAUDE.md` 甚至把 `getActiveAgentProfileConfig` 列为前端 wrapper（无调用方）；(2) 后端 **3 个命令全仓零调用**：`abandon_turn`、`archive_conversation`、`soft_delete_variant`（`card_shell_register_doc/module/unregister_doc` 实际被 `utils/shellDocUrl.js:64,83,105,120` 以裸 `_invoke()` 调用，只是被基线正则 `\._invoke\(` 漏计 → 与域4 T-01 同结论；这些属域6/域4 范围，本域仅作交叉核对）；(3) 上述盲区使 `tauri-command-contract.test.mjs` 的"175/169/171"看似严密，实际漏掉 3 个真实调用点——**任何依赖该统计判断"前端是否已接线"的结论都要打折扣**。
- 建议：对 21 个 wrapper 逐个定性（保留兼容面 / 删除）；把基线正则改为"任意 `invoke(`/`_invoke(` 子串 + 白名单排除"并补一条 anti-regression 用例；`abandon_turn`/`archive_conversation`/`soft_delete_variant` 的取舍请 Lead 与域4/域6 合并判定。
- 置信度：已核实/高（本域独立复现计数；后端侧结论与域4 T-01 一致）。

#### F-42 「测试连接」的延迟永远不显示：前端读 camelCase，后端序列化 snake_case
- 严重度：P2　类别：B 字段级契约错
- 位置：`frontend/src/components-v2/config/ConnectionConfigPanel.vue:786` ↔ `crates/tauri-app/src/commands/connections.rs`（`TestConnectionResult`）
- 证据：

```html
            <span>{{ testResult.success ? '✓' : '✗' }}</span>
            <span class="flex-1">{{ testResult.message }}</span>
            <span v-if="testResult.latencyMs" class="text-ink-soft">· {{ testResult.latencyMs }}ms</span>
```

```rust
pub struct TestConnectionResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: Option<u64>,
}
```

- 影响：Tauri 只对**命令参数名**做 camelCase 折算，**不改响应体**，故 `testResult.latency_ms` 有值而 `latencyMs` 恒为 `undefined` → 「· 42ms」永不渲染（成功的连通性测试看不到耗时）。全仓 `latencyMs` 只出现在本行 + `tauri-api.js:470` 的 JSDoc + `:485` 的非 Tauri mock（`return { success: true, message: '（mock）连通成功', latencyMs: 42 }`）——**mock 用 camelCase 恰好掩盖了该 bug**，纯前端环境测不出来。同组件其它响应字段（`has_api_key`/`base_url`/`top_p`/`max_tokens`，`:334-345`）都按 snake_case 读，可排除"后端其实返 camelCase"。
- 建议：改读 `testResult.latency_ms`；同步修 `tauri-api.js` 的 JSDoc 与 mock（或在 wrapper 层统一做 snake→camel 转换，一次解决同类问题）。
- 置信度：已核实/高。

#### F-43 配置面板角色列表缺 `Writer`：执笔者的模型/轮次无法配置
- 严重度：P2　类别：A 目标完成度（能力缺口）
- 位置：`frontend/src/components-v2/config/AgentProfileManager.vue:33-39` ↔ `crates/app-pipeline/src/lib.rs:3592`
- 证据：

```js
const ROLES = [
  { key: 'Director', label: '导演' },
  { key: 'Editor', label: '编剧' },
  { key: 'Subagent:*', label: '子 Agent（通配）' },
  { key: 'Summarizer', label: '总结器' },
  { key: 'PostProcessor', label: '后处理' },
]
```

```rust
    let run = agent_profile_config.map(|config| config.run_config_for(&AgentRole::Writer));
```

- 影响：后端在 continuation 路径上确实按 `AgentRole::Writer` 读 profile（Writer 是默认正文产出者），而 UI 的 5 个角色里没有 Writer → 它的 `model_override` / `max_tool_rounds` **无法通过界面配置**，永远用全局默认模型（`run_config_for` 对 Writer 无回退，`agent_profile_config.rs:185-203` 拿到的是 `EMPTY`）。用户以为"每个 Agent 都能配"（面板标题如此），实际少一个关键角色。
- 建议：`ROLES` 增加 `{ key: 'Writer', label: '执笔者' }`（`AgentRole` 的 serde 字面量与前端字符串一致，可直接用 `'Writer'`）；是否需要同时加入 `ROLES_WITH_TOOLS` 视其工具面决定。
- 置信度：已核实/高（本域复核前端列表与后端调用点）。

#### F-44 流水线 trace 面板不认识 `writer_*` 事件：执笔者在面板里不可见
- 严重度：P2　类别：A 目标完成度（可观测性缺口）
- 位置：`frontend/src/components-v2/debug/PipelineTracePanel.vue:46-56` ↔ `frontend/src/composables/usePipeline.js:27,84-101`
- 证据：

```js
const editorEvents = computed(() =>
  pipelineEvents.value.filter(
    (r) =>
      r.event.event_type.startsWith('editor_') ||
      r.event.event_type === 'draft_ready' ||
      r.event.event_type === 'quality_checked',
  ),
)
```

- 影响：`writer_started` / `writer_progress` 确实被广播进 `pluginStore.pluginPipelineEvents`（`usePipeline.js:27` 对每个事件先 `broadcastPluginPipelineEvent(event)`，`:84-101` 处理这两个事件；后端映射见 `commands/writing.rs:416-425`），但该面板四个 tab 的分类谓词都没有 `startsWith('writer_')` → 执笔者阶段只出现在「插件事件」原始 feed 里，正常判读会以为"编剧没输出"/正文来源不明。排查"谁写的这段正文"是写作面最常见的诊断动作。
- 建议：把 `startsWith('writer_')` 并入 `editorEvents`（或按 `writingStore.pipeline.editor.role` 分成"编剧/执笔者"两栏）。
- 置信度：已核实/高（谓词与事件名已核；未实跑 UI）。

#### F-45 `PresetPanel` 的加载与展开失败无 catch：错误伪装成空态，箭头翻转但详情永不出现
- 严重度：P2　类别：D 错误处理 / B 状态不一致
- 位置：`frontend/src/components-v2/config/PresetPanel.vue:52-68`（refresh）、`:79-90`（togglePreset）
- 证据：

```js
onMounted(() => { refresh() })

async function refresh() {
  loading.value = true
  loadingGlobalRegex.value = true
  try {
    const [presetList, globalRegexList] = await Promise.all([listPresets(), listGlobalRegexScripts()])
    presets.value = presetList
    globalRegexScripts.value = globalRegexList
  } finally {
    loading.value = false
    loadingGlobalRegex.value = false
  }
}
```

```js
async function togglePreset(preset) {
  if (expandedId.value === preset.id) { expandedId.value = null; detail.value = null; editingPrompt.value = null }
  else {
    expandedId.value = preset.id                 // 先置位
    detail.value = await getPreset(preset.id)    // 失败则永久 pending
```

- 影响：(1) `refresh()` 只有 `finally` 没有 `catch`，`onMounted` 也未 `await`/`.catch()` → 加载失败是**未处理的 promise rejection**，界面渲染 `presets.length === 0` 的空态「还没有预设」，用户把"读取失败"当成"没有数据"（与 F-10/F-11/F-19 同类）；其它写操作成功/失败后也会再走 `refresh()`。(2) `expandedId` 在 `await getPreset` 之前就赋值，而详情渲染条件是 `v-if="expandedId === p.id && detail"`（`:329`）→ 读取失败时**箭头已翻成展开态但详情永不出现**，再点一次才收得回，且没有任何错误提示、没有 loading 态。
- 建议：两处补 `try/catch` + `alertDialog(errorText(e))`（同文件其它 10 处已经这么做）；`refresh` 增加 error 状态让模板区分空态；`togglePreset` 失败回滚 `expandedId` 并加 `detailLoading`。
- 置信度：已核实/高。

### P3

#### F-31 生产入口保留 3 条设计演示 hash 路由，演示屏被打进产物
- 严重度：P3　类别：C 死代码 / F 交付面
- 位置：`frontend/src/main.js:12-22`
- 证据：

```js
// 开发预览：hash 路由（正式接线并验收后删除对应分支）
if (location.hash === '#design-writing') return mountDemo(WritingScreenDemo)
if (location.hash === '#design-campaign') return mountDemo(CampaignScreenDemo)
if (location.hash === '#design-meta') return mountDemo(MetaScreenDemo)
```

  （`WritingScreenDemo` / `CampaignScreenDemo` / `MetaScreenDemo` 为**静态 import**，会进入生产 bundle。）
- 影响：Phase 8 已被标记完成（`docs/ROADMAP.md:205`），但这三条分支仍在生产入口且注释自称"临时"。影响面：产物体积（3 个 Demo 及其实验依赖）、可被用户用 URL 打开的非产品界面。不含安全风险（demo 数据是本地常量）。
- 建议：删除三条 hash 分支与 `design/*Demo.vue`，或改为 `import.meta.env.DEV` 守卫的 `import()`。
- 置信度：已核实/高。

#### F-32 `ui/DataList` 的空态能力在生产不可达，注释承诺 `activeItem` 不存在
- 严重度：P3　类别：C 死代码 / F 注释漂移
- 位置：`frontend/src/components-v2/ui/DataList.vue:4`、`:19-25`，调用点 `CardLibrary.vue:98-105`
- 证据：

```js
  // 选中态:传一个 id 字段名,或用 activeItem 引用对比
  activeKey: { type: String, default: 'id' },
```

```html
      <slot name="empty-icon">
```

- 影响：`activeItem` prop 不存在（与 F-05 同源）；`emptyDescription` / `empty-icon` / `empty-action` 三个空态契约零调用（`grep 'empty-description|#empty-action|#empty-icon'` 在 `src/` 命中 0）——因为唯一调用点在 `cards.length === 0` 时自己渲染 `EmptyState` 且不给 DataList 传 items，故 DataList 的空态分支在生产不可达。
- 建议：删注释与未使用插槽，或让 CardLibrary 改用 DataList 自带空态。
- 置信度：已核实/高。

#### F-33 `ui/DataTable` 的死 emit、空态 colspan 与列注释
- 严重度：P3　类别：C 契约漂移
- 位置：`frontend/src/components-v2/ui/DataTable.vue:7`、`:36-40`、`:3`
- 证据：

```js
const emit = defineEmits(['row-action'])      // :7，全文件未使用 emit
```

```html
        <tr v-if="rows.length === 0">
          <td :colspan="columns.length + 1" class="text-center text-ink-faint py-8">{{ emptyTitle }}</td>
```

```js
  columns: { type: Array, default: () => [] }, // [{key, label, width?}]   // :3
```

- 影响：(1) 真实契约是 `#row-action` 插槽（3 个调用点都用插槽，无一处 `@row-action`），声明式 emit 是死的；(2) 空态 colspan 恒为 `columns.length + 1`，没有 row-action 插槽的调用点（`CampaignKnowledgeTab`/`CampaignSummariesTab`/`WorldInfoReadonlyPanel`）会多出一列（视觉上略偏左）；(3) 列注释未记录代码实际读取的 `col.mono`（`:27`、`:50`）。
- 建议：删死 emit；colspan 按是否有插槽计算；注释补 `mono`。
- 置信度：已核实/高。

#### F-34 两个列表基元都没有分页/虚拟化，卡库与日志全量渲染
- 严重度：P3　类别：B 性能边界
- 位置：`frontend/src/components-v2/ui/DataList.vue:28-29`、`ui/DataTable.vue:42-43`、`campaign/CardLibrary.vue:104-108`
- 证据：

```html
      v-for="(item, index) in items"
      :key="item[activeKey] ?? index"
```

- 影响：`CardLibrary` 是唯一 DataList 调用点且卡数量无上限（每卡展开还会额外 `getCard`）；`LogPanel`/`PromptHookAuditLog` 列表可持续增长，每行在插槽里实例化 Badge/IconButton/CodeBlock → 线性放大渲染成本。注意：DataList 的 key 反而是稳的（`active-key="id"`），DataTable 相反（F-20）。
- 建议：加可视窗口/slice + 「显示更多」或分页；日志类加渲染上限。
- 置信度：已核实（无分页/虚拟化）/中（性能阈值随机器与数据量）。

#### F-35 `ui/Textarea` 的 `autoResize` 首帧高度错误，且当前是死 prop
- 严重度：P3　类别：B 交互缺陷（未启用）
- 位置：`frontend/src/components-v2/ui/Textarea.vue:15-24`
- 证据：

```js
function resize() {
  if (!props.autoResize || !el.value) return
  el.value.style.height = 'auto'
  el.value.style.height = el.value.scrollHeight + 'px'
}
watch(() => props.modelValue, () => nextTick(resize))
```

- 影响：没有 `onMounted(resize)` → 首帧按 `rows` 渲染，首屏高度不对；父级若只读 `:model-value` 不同步 v-model，则用户输入完全不触发 resize。全仓 0 个调用点使用 auto-resize（`CampaignTasksTab.vue:137 :rows="2"`、`CampaignInstancesTab.vue:536/546`）→ 属"一旦启用就以首帧高度错误暴露"的死 prop。（对比：`design/writing/ComposerBar.vue:74` 用内联 `@input` 手动实现，首帧同样不 resize，但它在 `mounted` 后首行内容为空，观感无差。）
- 建议：加 `onMounted(() => nextTick(resize))` 与 `@input` 内联 resize、加 `max-height` 保护；或删除该 prop。
- 置信度：已核实/高。

#### F-36 `ui/CodeBlock` 的复制定时器未清理、剪贴板失败静默、无标题栏时按钮遮挡首行
- 严重度：P3　类别：D 反馈缺失 / 资源泄漏
- 位置：`frontend/src/components-v2/ui/CodeBlock.vue:10-17`、`:40-49`
- 证据：

```js
async function copy() {
  try {
    await navigator.clipboard.writeText(props.code)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)      // 无 onBeforeUnmount 清理
  } catch {
    // clipboard 不可用时静默                                 // 无成功/失败反馈
  }
}
```

- 影响：卸载后定时器仍会跑（修改已卸载组件的 ref，Vue 不会报错但属泄漏）；剪贴板不可用（非 https/权限拒绝）时点"复制"毫无反馈；无 `language` 且无 toolbar 插槽时，复制按钮绝对定位压在 `<pre class="p-3">` 首行右上角且无底色，长首行会被遮（3 个调用点都传了 language，当前只走带标题栏分支）。
- 建议：卸载时 `clearTimeout`；失败时给出 toast/文案；无标题栏分支的 `pre` 加 `pr-16` 或用带底色浮动按钮。
- 置信度：已核实/高（第 3 点为潜在）。

#### F-37 headlessui 语义缺口：`ui/Tabs` 无 `TabPanel`、`ui/Overlay` 无 `DialogTitle`
- 严重度：P3　类别：a11y / 契约完整性
- 位置：`ui/Tabs.vue:32-35`+`:50-52`、`ui/Overlay.vue:130`+`:142`
- 证据：

```html
  <TabGroup :selected-index="selectedIndex" @change="handleChange">
…
    <div class="pt-3 focus:outline-none min-w-0"><slot /></div>
```

```html
    <Dialog @close="isOpen = false" class="relative z-[var(--z-drawer)]">
…
              <h2 class="min-w-0 truncate text-sm font-semibold text-ink">{{ title }}</h2>
```

- 影响：`Tab` 的 `aria-controls` 指向的 panel id 在 DOM 中不存在，调用方塞进默认插槽的面板也没有 `role="tabpanel"`；`Dialog`/`DialogPanel` 上没有 `aria-label`/`aria-labelledby`，标题是普通 `h2` 而非 `DialogTitle` → 弹层对读屏没有可访问名称。ESC 关闭与焦点陷阱/恢复由 headlessui 保证（`Overlay.vue:130`，无需自实现），方向键与焦点管理由 Tabs 提供，故仅列 P3。
- 建议：内容用 `TabPanel` 包住或补 `role/id`；标题改 `DialogTitle` 或给 `DialogPanel` 加 `:aria-label="title"`。
- 置信度：已核实（代码事实）/中（读屏实际表现未实测）。

#### F-38 a11y 缺口汇总（单点小修，合并一条）
- 严重度：P3　类别：a11y
- 位置与证据：

| 位置 | 事实 | 影响 |
| --- | --- | --- |
| `ui/Progress.vue:29-39` | 无 `role="progressbar"`/`aria-valuenow`/`aria-label`，indeterminate 无 `aria-busy` | 进度对读屏完全不可见（且该组件零引用，见 F-18） |
| `ui/Toast.vue:21-25,33-35` | 无 `role="status"`/`aria-live`；`duration` 只在 `onMounted` 读一次，运行中改 prop 不重启/不清理计时器；hover/focus 不暂停 | 动态提示不被播报；卸载后定时器仍可能触发（组件零引用） |
| `ui/Checkbox.vue:15-27` | 真 input `sr-only`，视觉框只有 `peer-hover`，无 `peer-focus-visible` | 键盘 Tab 到复选框看不到焦点环 |
| `ui/Input.vue:6,30` | `invalid` 只换边框色，不输出 `aria-invalid`，无 error 文案/`aria-describedby`（`Textarea` 连 `invalid` 都没有） | 校验失败对读屏不可见 |
| `ui/LoadingState.vue:9-11,18-21` | 无 `role="status"`/aria-live | 16 个调用点的"加载中"不播报 |
| `ui/IconButton.vue:27-37` | 只用 `title` 作可访问名；loading 时用 spinner 替换插槽、无 `aria-busy` | 可接受但不如 `aria-label` 稳；宽度抖动 |
| `ui/Button.vue:39-42` | `loading` 时**整段文案被 spinner 替换**（`button.test.mjs:19-24` 把这个行为锁成契约），无 `aria-busy`；生产证据：`ConnectionConfigPanel.vue:791-804` 的 `{{ testing ? '测试中…' : '测试连接' }}` / `{{ saveLabel }}`、`PresetPanel.vue:432`、`AgentProfileManager.vue:474` 的 `保存中…` **全部永远不会显示**，用户只看到转圈 | 同一工具条内按钮跳动；读屏拿不到"正在提交"，按钮文案（如"测试连接"）在加载态消失 |
| `index.html:5` | `<meta name="viewport" … maximum-scale=1.0, user-scalable=no>` | 禁止双指缩放，移动端低视力用户无法放大（可访问性反模式） |

- 建议：按表逐点补 aria 属性（低成本、无行为变化）；`Button` 的 loading 建议保留文案并把 spinner 插在文案前，同时同步更新 `button.test.mjs` 的契约断言；`index.html` 去掉 `maximum-scale`/`user-scalable`（或至少放宽到 `maximum-scale=5`）。
- 置信度：已核实/高（代码事实；读屏表现未实测）。

#### F-39 死状态与陈旧注释（维护陷阱）
- 严重度：P3　类别：C 死代码 / F 注释漂移
- 位置与证据：

| 位置 | 证据 | 说明 |
| --- | --- | --- |
| `stores/ui.js:53` | 注释称 `viewWrite` 是死代码 | 实际 `AppV2.vue:503,710` 调用 `ui.viewWrite()` → **注释错**；同文件 `powerMode`/`togglePower`/`viewOverview` 才是真死状态（全仓零调用） |
| `components-v2/campaign/CampaignPanel.vue:42-43` | `// 兼容旧 activeTab 语义：cards \| campaigns \| detail` + `const activeTab = ref('detail')` | 被写 8 次（`:118/:155/:252/:263/:280/:286/:292/:436`），模板与 `defineExpose` **零读取** |
| `components-v2/campaign/NewCampaignForm.vue:9-11` | 注释称"组件内不再自建实例 + watch 重复加载，此前每次打开都会 listCards/getCard 两次" | 但 `:37-39` 的 `watch(newCampaignCardId)` 仍在，而 `useNewCampaignForm.js:101-102` 已自行 `newCampaignCardId.value = …` 并 `await loadNewCampaignCardDetail()` → **每次打开（选卡变化时）仍调用两次 `get_card`**，注释与代码相反 |
| `components-v2/campaign/CardStudio.vue:36-44` | `STAGE_META` 硬编码 7 个阶段 | 后端有 `cardstudio_list_stages`（`tauri-api.js:981` 有 wrapper，属 F-30 的 21 个孤儿之一）→ 阶段定义变更时 UI 静默漂移 |
| `ui/DataTable.vue:3`、`ui/DataList.vue:4` | 列/选中语义注释与实际不符 | 同上 F-32/F-33 |

- 建议：删除死 ref 与 8 处赋值；修正 3 条错误注释；`STAGE_META` 改为挂载时用 `cardstudioListStages()` 驱动、失败回退本地常量。
- 置信度：已核实/高。

#### F-40 文档与代码事实漂移
- 严重度：P3　类别：F 文档漂移
- 位置与证据：

| 文档位置 | 原文/事实 | 代码事实 |
| --- | --- | --- |
| `docs/FRONTEND-COMPONENTS.md:270,275` | 把 `components-v2/` 的 `writing/ConversationViewport.vue`、`ChatMessage.vue` 等列为写作区组件蓝图 | 该目录 8 个文件**全是死代码**（§1.2(b)），生产写作面在 `design/writing/**` |
| `docs/ROADMAP.md:202,212` | "重写写作工作台（**ChatMessage 保留 8 emit 契约**）"、验收"契约红线全部保留：ChatMessage 8 emit…" | 真实契约在 `design/writing/MessageItem.vue`（8 个 emit 全部接线）；`components-v2/writing/ChatMessage.vue` 零引用 |
| `docs/ROADMAP.md:210` | "node --test 212 pass + vitest 21 pass" | 当前 `tests/*.test.mjs` 口径：node 套件 66 个（54+4+8）、vitest 套件 27 个 —— 文档口径（用例数）与今天无法对齐且 vitest 文件数已超过文档的用例数，**至少已过期**（未能运行测试，故标 疑似） |
| `CLAUDE.md`（wrapper 清单） | "Frontend user-facing error strings go through … `getActiveAgentProfileConfig`" 被列为前端 wrapper | 该 wrapper 属 21 个零调用孤儿（F-30） |
| `frontend/CONTRACT.md:27` | 事件载荷表 | 与 `usePluginBridge.js` 现状未逐条核对（域6 范围）→ 本域不复核 |

- 建议：把 `FRONTEND-COMPONENTS.md` / `ROADMAP.md` 的写作面指向改为 `design/**`，或明确标注 `components-v2/writing/**` 为"阶段 8 遗留、未接线"；测试数字改为可复现的口径（套件数 + 命令）。**文档修正本身属写操作，需 Lead 统一安排**。
- 置信度：已核实/高（除测试用例数字为 疑似）。

#### F-46 「回退树」`components-v2/writing/**` 的交互缺陷：现状下回退等于不可用
- 严重度：P3　类别：C 死代码质量（跨文件一组）
- 位置：`ChatMessage.vue:17/27-31/76-79/86-88/90-95/113-118`、`Composer.vue:39`、`StreamingMessage.vue:54-58`、`GreetingSelector.vue:22-26`、`ConversationViewport.vue:28/71-74`
- 证据（三处最关键）：

```js
function saveEdit() {                                    // ChatMessage.vue:76-79
  emit('edit-variant', { nodeId: props.message.id, newContent: editContent.value })
  editing.value = false                                  // 不消费 saveVariant 的返回值
}
```

```js
function acceptVariant() {                               // ChatMessage.vue:86-88
  emit('accept-variant', { nodeId: props.message.id })
}
```

```html
      <span class="text-[11px] text-running flex items-center gap-1.5">   <!-- StreamingMessage.vue:56-58 -->
        <span class="w-1.5 h-1.5 rounded-full bg-running animate-pulse"></span>生成中
```

- 影响：整棵树零引用（§4.5），今天不影响用户；但 `design/writing/CONTRACT.md:55` 明写它"保留作对照与回退，生产主路径不再引用 Viewport/Composer"。按现状把回退开关打开后：(a) **Campaign 模式「采纳」永久静默无操作** —— `useMessageVariants.js:236-241` 在小票存在时只 `openTurnReceipt` 然后 `return`，而小票 UI 只存在于 `design/writing/MessageItem.vue:282-349`，这棵树里 `pendingReceipt` 零消费（全 `src` 唯一消费者是 adapter 与 design/writing）→ 草稿永远无法落库；(b) 保存编辑失败也关编辑器（`handleEditVariant` 已 `alertDialog` 并 `return false`，此处不消费）→ 用户编辑丢失；(c) `busy` 只覆盖编辑/重roll/分支，变体切换/采纳/删除无门，生成中可并发写同一 node；(d) `add-variant` 在 emits 里声明但全树无按钮（`handleAddVariant` 已实现）；(e) 非取消类失败只置 `pipeline.state='error'` 而不收 `showPipeline`（`useWriting.js:199-202`），"生成中"是硬编码 → 该组件常驻且停止键消失，用户无任何刷新入口；(f) `GreetingSelector.vue:24` `:key="option.label"` 同文案即重复 key；(g) `ConversationViewport.vue:28` 的 `ui` 声明后零使用、`:71-74` 的 `canBranch(m)` 忽略入参。
- 建议：要么删除整棵树（同时删 `docs/FRONTEND-COMPONENTS.md` 里对它的引用，F-40），要么把 (a)-(e) 修到与 `design/writing` 等价并补一条 mount 冒烟测试；文档里"可回退"的表述应改成"仅存档参考"，否则会误导维护者。
- 置信度：已核实/高（代码事实）；其中 (a) 依赖"后端对活动 attempt 恒返小票"（域4 已核 `turns.rs:156`）→ 机制 高、触发条件 中。

#### F-47 debug/config 面板的次要缺陷（性能、一致性、死代码）
- 严重度：P3　类别：B/C/D 混合
- 位置与证据：

| 位置 | 事实 | 影响 |
| --- | --- | --- |
| `PromptHookAuditLog.vue:210-216` | 详情块用 `v-for` + `v-show` + `:key="\`detail-${index}\`"`，≤100 条记录全部常驻 DOM（每条两个 `CodeBlock` + `pre`） | 面板打开即渲染全部详情；index key 与倒序列表（`:28`）叠加会让展开内容错位 |
| `PromptHookAuditLog.vue:108-112` | `copyRecord` 的 1.5s `setTimeout` 未纳入 `onUnmounted`（同文件 `exportStatusTimer` 已清理） | 卸载后定时器仍执行（低危泄漏） |
| `PipelineTracePanel.vue:59-67` | `lastStatus` 只看最后一次事件后缀，末事件不以 `_done/_failed` 结尾即回落 `'running'` | 阶段可能长期显示"运行中"（缓冲区 500 条不清空、无 pipeline state 参照） |
| `PipelineTracePanel.vue:152-159` | 模板里直接调用 `eventDetailJson(events)`，每次重渲染对全量事件 `JSON.stringify(…, 2)`（上限 500 条，含流式 delta） | 抽屉打开 + 流式生成期间 CPU/GC 抖动 |
| `ConnectionConfigPanel.vue:281-289` | `applyTemplate` 未复制 `t.protocol`（DTO 有该字段） | 一旦新增非 OpenAi 协议模板，会以 `openai` 协议落库；`startEdit` 回显 `gemini`/`custom:*` 时下拉（`:111-114` 仅 openai/anthropic）显示空白 |
| `ConnectionConfigPanel.vue:424-438` | 清空 `temperature`/`top_p` 时 `v-model.number` 给出 `''`，`parseFloat('')`=NaN → invoke 的 JSON 序列化把 NaN 变成 `null` → 后端 `Option<f32>` 收到 None | 采样参数静默回退厂商默认，无提示（UI 也未标注可留空）——**疑似/中** |
| `AgentProfileManager.vue:62-63` | `const activeId = computed(…); void activeId // 保留用于将来展示` | 死代码；`embedded` prop（`:24-27`）无任何调用方传值，其动态组件分支（`:283-287`）与 `emit('close')` 在 embedded 下永久不可达 |
| `LogPanel.vue:189-197` | 3s 轮询每次整表替换 `logs.value`（120 条全量重渲染，无内容 diff）；`expanded` Set（`:24`）只增不删 | 长会话下无界累积；定时器/超时清理本身是正确的 |

- 建议：按表逐条小修；`PipelineTracePanel` 的两点建议改为读 `writingStore.pipeline.state` + `computed` 缓存。
- 置信度：已核实/高（除 NaN→null 一行为 疑似/中）。

---

## 4 组件可用性核对表（组件 → 调用方 → 状态覆盖 → 判定）

判定口径：**可用** = 主路径可用且空/加载/错误态齐备；**可用（有瑕疵）** = 主路径可用但存在本报告具名缺陷；**不可用/死代码** = 无生产调用方或契约断链。

### 4.1 写作面与三大 screen（生产路径）

| 组件 | 调用方（生产） | 空 / 错 / 加载态覆盖 | 判定 |
| --- | --- | --- | --- |
| `design/writing/WritingScreen.vue` | `AppV2.vue`（`writingScreenRef`） | 空态 `hasEmpty`（`:78-98`）、流水线错误条（`:165-172`）、`isWriting` 态 | 可用 |
| `design/writing/StoryPage.vue` | `WritingScreen`（`:117-131`） | 流式页脚 + 停止按钮（`:145-163`）、`pendingReceipt` 条件传递 | 可用 |
| `design/writing/MessageItem.vue` | `StoryPage` | 编辑保存错误 `role="alert"`（`:181`/`:205`）、采纳小票四分支（`:281-351`）、`busy` 禁用 | 可用（有瑕疵：F-02 无采纳失败提示） |
| `design/writing/VariantStrip.vue` | `MessageItem:212-218` | `busy` 禁用；无独立错误态（依赖父级） | 可用（有瑕疵：F-02 同源；`accept` 不带 `selectedMutationIndices`，见 §6-2） |
| `design/writing/ComposerBar.vue` | `WritingScreen` | 输入为空即禁用按钮、`writing` 时切停止键、Enter 提交 | **可用（有瑕疵：F-01 IME 误提交）** |
| `design/writing/StreamingBody.vue` | `WritingScreen` | 过程折叠 + 进度点 + 流式光标；`contentComponent` 缺省降级为纯文本段落 | 可用 |
| `design/writing/ProcessTimeline.vue` / `EmptyHero.vue` / `GreetingCards.vue` | `WritingScreen` | 纯展示 / 空态引导 / 开场选择（`select(index)` 与 `select-greeting` 一致） | 可用 |
| `design/history/HistoryScreen.vue` | `AppV2.vue:964` | 列表由 `conversations` 驱动，空态在屏内 | 可用（有瑕疵：F-13 历史加载失败静默） |
| `design/overview/OverviewScreen.vue` | `AppV2.vue` | 4 props / 4 emits 全接线；无异步 | 可用 |
| `design/campaign/CampaignScreen.vue` | `CampaignPanel.vue:472+` | 11 emits 全部有监听者（除 `refresh` 仅在 adapter 内映射，见下）；内联 headlessui Menu a11y 更全 | 可用（有瑕疵：F-14 传 title 时 `show-close` 无效） |
| `design/meta/MetaScreen.vue` | `MetaPanel.vue` | 8 props / `change-tab`+`close` 全接线 | 可用 |
| `design/shell/AppFrame.vue` | `AppV2.vue` | 移动抽屉 `:inert`、resize 监听在 `onBeforeUnmount` 清理；**无 ESC 关闭**（桌面端抽屉可点遮罩关闭） | 可用（瑕疵：抽屉无 ESC，P3 级） |
| `components-v2/shell/PanelHost.vue` | `AppV2.vue` | 用 `ui/Overlay`（headlessui Dialog → ESC + 焦点陷阱 + 滚动锁） | 可用 |
| `components-v2/shell/{TopBar,PrimarySidebar,InspectorDrawer,StorageHealthGate,ThemePicker}.vue` | `AppV2.vue` | 均生产接线；`PrimarySidebar` 的 `v-html` 仅硬编码 SVG 路径（无注入面） | 可用（`InspectorDrawer.vue:48` 裸 `e?.message`，规约漂移） |
| `components-v2/writing/**`（8 个） | **无**（`ConversationViewport` 仅被自己引用） | — | **不可用/死代码**（F-30/§4.5） |
| `design/writing/*Demo.vue`（3 个） | `main.js` hash 路由 | — | 演示面（F-31） |

### 4.2 基元层 `components-v2/ui/**`（23 个）

| 组件 | 生产调用方 | 空 / 错 / 加载态 | 判定 |
| --- | --- | --- | --- |
| `Badge` | 21 文件 | 纯展示，映射函数无静默回退 | 可用（无缺陷） |
| `EmptyState` | 23 文件 | 空态 + icon/action 插槽 | 可用（无缺陷） |
| `Button` | 22 文件 / 23 处 | `:disabled="disabled \|\| loading"` 防重正确 | 可用（F-38 loading 替换文案） |
| `Input` / `Textarea` | 10 / 2 文件 | v-model 载荷正确；`invalid` 仅换色 | 可用（F-38、F-35） |
| `Select` | 10 文件 | 展开/收起/禁用/对象选项有测试；未 portal | 可用（F-22） |
| `Toggle` | 2 文件 | disabled 守卫正确 | 可用（F-23 无可访问名） |
| `Tabs` | 5 文件 | 无 TabPanel；非法值静默回落 | 可用（F-21、F-37） |
| `DataTable` | 6 文件 / 3 处用 `#row-action` | 空态 title 可配；行 key 不稳 | 可用（F-20、F-33、F-34） |
| `LoadingState` | 16 文件 | 无 `role="status"` | 可用（F-38） |
| `CodeBlock` | 3 文件 | 复制成功有 ✓；失败静默 | 可用（F-36） |
| `IconButton` | 3 文件 | `title` + disabled 正确 | 可用（F-38） |
| `Overlay` | 4 文件 / 5 处 | ESC/焦点/遮罩齐备（headlessui） | 可用（F-14、F-15、F-37） |
| `DataList` | 1（`CardLibrary.vue:105`） | 自带空态不可达 | **契约双重死链**（F-05、F-06） |
| `Checkbox` / `ErrorState` / `Menu` / `Progress` / `Slider` / `Toast` / `Tooltip` | **0** | — | **不可用/死代码**（F-16、F-18） |
| `DiffView` / `SegmentedControl` | 仅测试（`ui-components.test.mjs`） | — | **不可用/仅测试引用**（F-17、F-18） |

（子代理 A 小结把这些基元合计为 23 个、其中"生产在用的"写成 16 个，与本域逐个复数的结果 **14 个** 不同；以本表 14 为准。）

### 4.3 Campaign 面板 `components-v2/campaign/**`（12 个，全部有生产调用方）

| 组件 | 调用方 | 空 / 错 / 加载态 | 判定 |
| --- | --- | --- | --- |
| `CampaignPanel.vue` | `AppV2.vue:966` | loading 三态有；`onMounted`/`refreshCampaigns`/`handleSetActive` 缺 catch | 可用（**P1：F-09、F-10**；F-39 死 `activeTab`） |
| `CampaignInstancesTab.vue` | `CampaignPanel:532` | 有测试；实例变量读失败空 catch（B-C16） | 可用（F-25） |
| `CampaignVariablesTab.vue` | `CampaignPanel:538` | 有测试；写失败不回滚控件 | 可用（F-25；`String(e?.message)` 规约漂移） |
| `CampaignWorldInfoTab.vue` | `CampaignPanel:548` | 有测试；路由写入失败草稿不一致 | 可用（F-26） |
| `CampaignTasksTab.vue` | `CampaignPanel:553` | **无测试**；两类契约/渲染缺陷 | **可用但功能残缺**（F-07、F-08、F-24、F-27） |
| `CampaignKnowledgeTab` / `CampaignSummariesTab` | `CampaignPanel:543/558` | 三态齐备（B 已逐行核）；无测试 | 可用（F-19 无重试入口） |
| `CardLibrary.vue` | `CampaignPanel:489` | 无 error 态；吞错 | 可用（**P1：F-11**；F-29） |
| `CardStudio.vue` | `CampaignPanel:482` | 有测试；保存失败仍继续 | 可用（**P1：F-12**；F-19、F-34） |
| `NewCampaignForm.vue` | `AppV2.vue:1003` | 有测试；失败保持打开（设计正确） | 可用（F-27、F-39 双拉详情） |
| `CharacterCardDetail.vue` | `AppV2.vue:951`（legacy 路径） | 用 `BaseOverlay`（ESC+遮罩可关） | 可用（`:205-210` index key，P3 级） |
| `WorldInfoReadonlyPanel.vue` | `CardLibrary:149` | `expanding` 单标志竞态 | 可用（P3 级） |

### 4.4 其余生产组件

| 组件 | 调用方 | 状态覆盖 | 判定 |
| --- | --- | --- | --- |
| `components-v2/debug/LogPanel.vue` | `InspectorDrawer.vue:17` ← `AppV2.vue:44,923` | loading/空态有，加载失败静默，导出/清空有提示；轮询/定时器清理正确 | 可用（F-19、F-47） |
| `components-v2/debug/PipelineTracePanel.vue` | `InspectorDrawer.vue:14` | 纯 props 展示，无 async；分类谓词缺 `writer_*` 与 `lastStatus` 兜底偏乐观 | 可用（F-44、F-47） |
| `components-v2/debug/PluginEventLog.vue` | `InspectorDrawer.vue:15` | 三态分明、`row.id` 唯一、格式化有 try/catch；复制失败静默 | 可用（无其他缺陷） |
| `components-v2/debug/PromptHookAuditLog.vue` | `InspectorDrawer.vue:16` | 导出失败有提示；**展开详情抛 `ReferenceError`** | **可用但关键功能失效**（**P1：F-41**；F-47） |
| `components-v2/config/AgentProfileManager.vue` | `AppV2.vue:55,996` | **8 个 handler 全部 errorText + finally**（§1.2(g)）；`embedded`/`activeId` 死代码 | 可用（F-43 缺 Writer 角色、F-23 Toggle a11y） |
| `components-v2/config/ConnectionConfigPanel.vue` | `AppV2.vue:52,959` | loading/testing/saving 全 finally；2 处裸 `e.message`（对 DTO 可用）；`pointerdown` 监听在 unmount 清理 | 可用（F-42 延迟不显示、F-47） |
| `components-v2/config/PresetPanel.vue` | `AppV2.vue:53,984` | 10 个写操作 catch 全 alert+errorText；加载/展开无 catch | 可用（F-45、F-24、F-47） |
| `components/base/BaseDialog.js`（`alertDialog`/`confirmDialog`/`promptDialog`） | 全仓 | 原生 Tauri 对话框 + 命令式 `BaseOverlay`（自动聚焦/Enter/Escape） | 可用 |
| `components/base/BaseOverlay.vue` | `BaseDialog.js:67`、`CharacterList.vue:104`、`CharacterCardDetail.vue:40` | ESC/遮罩/尺寸档位 | 可用（与 `ui/Overlay` 双实现冗余，属 P2 级重复） |
| `components/base/BaseDropdown.vue` | **0** | — | **不可用/死代码**（§4.5） |
| `src/components/**` 其余 8 个 | 域6 范围 | 不判定 | — |

### 4.5 零引用 / 仅测试引用组件清单（本域 18 个 + 域6 交叉 1 个）

| # | 组件 | 状态 |
| --- | --- | --- |
| 1-8 | `components-v2/writing/{CampaignOverview,Composer,ConversationHistoryList,ConversationViewport}.vue`（零引用）+ `{ChatMessage,GreetingSelector,ProcessReview,StreamingMessage}.vue`（只被 `ConversationViewport` 引用） | 整个 `components-v2/writing/**` 死代码 |
| 9 | `components-v2/ui/Checkbox.vue` | 零引用（F-18） |
| 10 | `components-v2/ui/ErrorState.vue` | 零引用（F-18、F-19） |
| 11 | `components-v2/ui/Menu.vue` | 零引用 + 三处死契约（F-16） |
| 12 | `components-v2/ui/Progress.vue` | 零引用（F-18） |
| 13 | `components-v2/ui/Slider.vue` | 零引用（F-18） |
| 14 | `components-v2/ui/Toast.vue` | 零引用（F-18） |
| 15 | `components-v2/ui/Tooltip.vue` | 零引用（F-18） |
| 16 | `components-v2/ui/DiffView.vue` | 仅测试引用（F-17） |
| 17 | `components-v2/ui/SegmentedControl.vue` | 仅测试引用（F-18） |
| 18 | `components/base/BaseDropdown.vue` | 传递死引用：只被死代码 `ChatMessage.vue:3` 引用（未展开细读） |
| 交叉 | `components-v2/st/StCompatibilityBadge.vue` | 零引用，但属域6 范围（仅记录） |

---

## 5 未发现问题与低风险观察

### 5.1 明确「未发现问题（已核对范围：…）」

- **写作面事件契约全链路**（已核对范围：`design/writing/WritingScreen.vue:100` 的 `forward`、16 个 emit、`adapter/useWritingScreenAdapter.js:118-135` 的全部 `screenEvents`、`StoryPage.vue:117-131` 的 `v-on="messageEvents"`、`MessageItem.vue:92-98`/`116`、`VariantStrip.vue:31-38`、`ComposerBar.vue:19-30`）：**历史 bug —— `forward()` 柯里化丢 payload —— 已修复**（`const forward = (name, payload) => emit(name, payload)`），16 个事件与监听键一一对应，`saveVariant` 的 `=== true` 判定与 `handleEditVariant` 返回值一致，未发现未接线/错 payload 的事件。
- **消息数据形状**（已核对范围：`useGreeting.js:45-59`、`useWriting.js:113-124/160-174`、`useConversation.applyConversation`、`useMessageVariants` 的全部消息查找）：**每个 producer 都写入 `variants` 数组与 `active_variant`**，故 `MessageItem.vue:48/61` 的无可选链访问（`props.message.variants[...]` / `.some(...)`）当前不可达；`StoryPage.vue:84` 用了 `?.` 兜底。该处属"输入契约守卫缺失"而非当前缺陷，不单列。
- **campaign 面板的 loading 与确认流**（已核对范围：12 个文件的全部异步标志与销毁入口）：所有 loading/busy 标志（`loading`/`creatingCampaign`/`adding`/`saving`/`syncingSchema`/`addingGlobal`/`promotingInstanceId`/`busy`/`extractingCardId`/`importingBundle`/`exporting`）**全部走 `finally` 复位**；删除整局活动（`CampaignPanel.vue:212-216`）、删写卡项目（`CardStudio.vue:182-185`）、删世界书条目（`CampaignWorldInfoTab.vue:189-193`）、放弃任务（`CampaignTasksTab.vue:85`）**全部有确认框**。
- **基元正确性**（已核对范围 = 整文件）：`Badge.vue`（1-31）、`EmptyState.vue`（1-22）逐调用点核对后无缺陷；`Button.vue:36` 双击防护、`Toggle.vue:8-11`/`SegmentedControl.vue:14-17` 的 disabled 守卫、`Input/Textarea/Slider/Checkbox/Toggle` 的 v-model 载荷形状与调用方期望一致；`Select` 的 `modelValue: { type: [String, Number, null] }` 经 Vue 3.5.38 `assertType` 的 `null` 分支确认合法无告警。
- **store 层**（已核对范围：4 个 store 全文）：`writing.js` 三态（campaign/legacy/none）+ pipeline 状态机自洽；`campaign.js` 的 `lastConversationNode` 在 getter 内调用 `useWritingStore()` 规避了模块层循环 import 死锁；`plugin.js` 的非响应式 `Map`/序号隔离正确；除 `stores/ui.js` 的死状态与错误注释（F-39）外无问题。
- **安全面**（已核对范围：全部 `v-html` 站点 + CSP 来源）：`v-html` 只有 3 处 —— `components-v2/writing/StreamingMessage.vue:116`（死代码）、`components-v2/st/RichContent.vue:33-34`（域6）、`components-v2/shell/PrimarySidebar.vue:117,162`（硬编码 SVG 常量）；`utils/formatContent.js` 先做 HTML 转义再套 markdown，含 `shouldRenderHtmlDisplay` 闸门；严格 CSP 在 `crates/tauri-app/tauri.conf.json`（`default-src 'self'`、无 `script-src 'unsafe-inline'`、`object-src 'none'`），`index.html` 无 CSP meta 属正常分工（由 Tauri 注入），本域未发现前端侧注入面。
- **adapter 层**（已核对范围：5 个文件全文）：`useWritingScreenAdapter` 的 props/events 与 `WritingScreen` 对齐；`useCampaignScreenAdapter` 的 `screenEvents` 覆盖 `design/campaign` 的 11 个 emit 中的 9 个（余 `change-mode`/`delete-campaign` 由 `CampaignPanel` 另行处理，属"adapter 供预览/整屏切换"的既定边界，`useCampaignScreenAdapter.js:1-7` 已注明）；`useMetaScreenAdapter` 覆盖 `close`/`change-tab`。
- **debug/config 面板**（已核对范围：7 个生产面板，深读子代理 C 逐行 + 本域 handler 级普查 §1.2(g)）：`AgentProfileManager` 的 8 个 handler 全部 `errorMsg + errorText` 且 `saving/loading` 在 `finally` 复位；`ConnectionConfigPanel` 的 `loadingEdit/testing/saving` 全 `finally`，`document pointerdown` 在 unmount 清理；`PresetPanel` 10 个写操作 catch 全 `alertDialog + errorText`；`LogPanel` 的 interval + exportStatusTimeout 清理正确；`PluginEventLog.vue`（全 164 行）**无缺陷**（`row.id` 唯一、过滤/空态/无匹配三态分明、格式化有 try/catch）；`ProcessReview.vue`（全 127 行）与 `ConversationHistoryList.vue`（全 82 行）、`CampaignOverview.vue`（全 82 行）**无缺陷**（后两者属死代码，但其 emit/空态契约本身是正确的，`ConversationHistoryList` 的 `emit('delete', conv, $event)` 冒泡已被 `useConversation.js:89-90` 正确 `stopPropagation`）。
- **已专门排查并排除的疑点**（避免误报）：`LogPanel` 的「最低级别」语义与后端一致（`entry.level < min_level`）、`kind`/`level` 取值与后端匹配臂对齐、`LogEntryDto` 字段齐备；`ui/Tabs` 的 `v-model` + 显式 `@update:model-value` 双绑定按源码顺序先赋值后回调（读到的是新值）；`PresetPanel` 的 `v-for` 与 `v-else` 同元素在 Vue 3 下编译正确；`update_preset_prompt/regex`、`update_global_regex` 的 `Option<T>` 参数与前端传 `null` 一致；`ProfileSource`/`AgentRole` 的 serde 字面量（`BuiltIn`/`UserCreated`/`Subagent:*`）与前端字符串逐一对齐；导出/导入走 `readFile`+`TextDecoder`/`writeFile`，capability 与动态 import 的 API 名匹配（无 v1 遗留 API）；配置面板均改本地 `editing`/`form` 副本，未发现直接改 props。
- **明确不报的项**：零引用组件**不因"没有测试"计为缺陷**；`ui-components.test.mjs`/`button.test.mjs`/`overlay.test.mjs`/`select.test.mjs` 已覆盖基元渲染与交互契约，缺的是**生产内联副本**（F-28）与**错误形状夹具**（F-08）这两类"测了但测错对象"的情况。

### 5.2 低风险观察（不单列为发现）

1. `utils/appCsp.js` 在 `src/` 零引用，但被 `tests/app-csp.test.mjs` 与 `tests/csp-inheritance.spec.mjs` 用作"与 `tauri.conf.json` 字节级一致"的审计锚点 → **有意设计**，非死代码。
2. `useWriting.js:191` 的 `loadInstanceNameMap()` 未 await（fire-and-forget）；失败由内部 catch 记 `console.error`，不阻塞写作完成 → 可接受，但会让"完成瞬间的子 Agent 名"短暂显示为 id。
3. `design/campaign/CampaignScreen.vue` 内联 headlessui Menu 的 trigger slot 是脆弱契约（headlessui 1.7.23 `dist/utils/render.js` 在 `as="template"` 需透传 props 时要求单根节点）→ 当前调用点满足，仅记录。
4. `VaraintStrip`…（原文即 `VariantStrip.vue:51` 的 `:key="i"`）与 `CharacterCardDetail.vue:205-210` 的 index key：列表不重排，风险低；`VariantStrip.vue:51` 建议改 `v.id ?? i`。
5. `CampaignPanel.vue:73-79` 的 `loadSelectedCampaignCardDetail()` 裸 await（B-C17）：失败时"新建档没有开场白可选"但弹层仍开 → 与 F-09/F-10 同类，量级更小，未单列。
6. `WorldInfoReadonlyPanel.vue:96-108` 的 `expanding` 全局单标志竞态（连续点两条截断条目时前一条的 `finally` 会清掉后一条的加载提示）→ 仅在快速连点时可见。
7. `CampaignVariablesTab.vue:86-94` 按实例并发发 N 个 `get_character_variables`（N+1 IPC，无批量）→ 大会话切 tab 时有可感知延迟。
8. 移动端 `mobile-chrome` / 主题 `theme-palettes` / CSP `csp-inheritance` / `ui-smoke` / `workbench-motion` 共 5 个 playwright spec 存在，但**本域未通读**（域5 不执行 UI 测试），端到端覆盖结论请以 Lead 实跑为准。

---

## 6 需要 Lead 重点复核的结论

1. **F-01（IME 误提交）需要一次真机确认** —— 结论依赖"Vue 按键修饰符不过滤 `isComposing`"这一框架语义（`frontend/src` 全仓无 `isComposing` 守卫是硬事实）。请用中文输入法在写作框实测：拼音候选态按回车是否触发发送/清空；若是，建议列为本次评审最优先修复项。
2. **F-07/F-08 的修复口径需要与域4 对齐** —— 前端要按 `{"likely_completed":{"confidence":0.5}}` 修渲染，并补"确认完成/误判"两个动作；后者是否需要新增后端命令（或复用 `complete_task`/`abandon_task`）请与域4/后端确认。`VariantStrip.vue:36-38` 的"采纳此版"不带 `selectedMutationIndices`，与 `MessageItem.confirmReceipt` 的路径语义是否等价也请一并裁定（本域仅记录差异，未判为缺陷）。
3. **21 个孤儿 wrapper 是否为有意保留的兼容面** —— `CLAUDE.md` 明确记录过"某 wrapper 仍存在但当前无前端入口"，因此这 21 个可能是刻意的 API 面而非遗漏；若定性为"应删"，则涉及域4（命令层）与域6（插件/卡壳，如 `cardShellAllowHost`）。本域只提供计数与清单。
4. **F-30 的基线盲区有传导效应** —— `tauri-command-contract.test.mjs` 断言 175/169/171 的统计口径漏掉 `shellDocUrl.js` 的 3 个裸 `_invoke()`（与域4 T-01 同结论）。任何"前端已接线/未接线"的判断都要先修基线或用第二种口径复核。
5. **F-29/F-13 的触发条件需要真实数据** —— `source_character_id` 缺失的老卡比例、`conversation.updated_at` 是否可能为空，只有拿到用户真实数据目录才能定级；本域已把"代码路径"与"触发条件"分开标注。
6. **debug/config 面板已由深读子代理逐行覆盖**（15 个文件：7 个生产面板 + 8 个死组件），并已并入本报告（F-41/F-42/F-43/F-44/F-45/F-46/F-47）；本域对其中的 P1/P2 条目（`PromptHookAuditLog` 展开 ReferenceError、`latency_ms` 字段名、`Writer` 角色缺失）**逐条复核了代码**。仍未实证的是运行期表现（未跑 UI），故 `F-44`/`F-46(a)` 的"界面不可见/采纳无操作"属于代码推导。
7. **F-46 的定性需要 Lead 决策** —— `components-v2/writing/**` 8 个组件零引用，但 `design/writing/CONTRACT.md:55` 明写它们"保留作对照与回退"；按现状该回退路径在 Campaign 模式下**采纳流程走不通**（F-46(a)）。要么修好并补冒烟测试，要么改文档措辞为"仅存档"，不要让"可回退"停留在纸面。
8. **`components-v2/writing/**` 8 个死组件与 `ui/**` 9 个死基元的处置需要一次决策** —— 它们同时也是 `docs/FRONTEND-COMPONENTS.md` / `ROADMAP.md` 描述的"写作工作台契约"所在（F-40）。删除/保留会直接改变文档与测试（`ui-components.test.mjs` 覆盖了其中 2 个）。
9. **`npm test` 不包含 vitest**（`frontend/package.json:7`）—— 本域的全部结论都基于静态阅读，**任何 P1 修复都必须同时跑 `npm test && npm run test:ui`**；`message-variants`/`turn-receipt` 相关用例在 vitest 侧。
10. **遗留不确定项**：`design/*Demo.vue` 与 3 条 hash 路由是否属于"已知待删"（`main.js:12` 注释自称临时）；`ui/Overlay` 与 `components/base/BaseOverlay` 的权威实现归属（P2 级重复，跨域：BaseOverlay 的调用点含域6 的 `CharacterList`）；`appCsp.js` 作为测试锚点的定位是否要被 `tauri.conf.json` 单向引用取代。


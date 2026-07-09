# B1 桌面 GUI 实跑调查报告（2026-07-09）

> 来源：Tauri 桌面 GUI 首次真实操作（临时 APPDATA 隔离，deepseek-v4-flash，opencode.ai/zen/go，test-card-seraphina.png）。
> 本文记录用户报告的 4 个 GUI 问题的根因调查和正确修复方向，**尚未修复**，作为后续修复的依据。

---

## 调查结论总览

| # | 问题 | 根因 | P1-1 修复方向是否正确 |
| --- | --- | --- | --- |
| 1 | 重 roll 逻辑不对 | P1-1 修复(truncate+append)把旧版本**彻底删除**，丢失了 variant 切换能力 | ❌ 方向错误，应回退到 variant 保留语义 |
| 2 | trace 看不到事件 | 待定（链路代码完整，但实跑时事件可能未到达前端或时机问题） | - |
| 3 | 调试面板日志栏目空 | LogPanel 调 log_query，日志只写 jsonl 但可能 kind 过滤不匹配 | - |
| 4 | ProcessReview 只有子 agent 没有导演/编剧 | ProcessReview.vue 模板只有「导演规划」和「子 Agent」区块，**缺编剧区块** | - |
| 附 | postprocess 未完成 | node[2] 实跑状态停在 Draft（未变 Final），postprocess 可能未跑或失败 | - |

---

## 问题 1：重 roll 逻辑不对（最关键）

### 用户反馈
> 重 roll 不一定要完全删除旧的，旧的可以保留，通过按键切换不同版本的回答。

### 根因
P1-1 修复（commit `cf8a565`）把 regenerate 落库从 `replace_active_variant`/`add_variant` 改成了 `truncate_from` + `append_ai_draft`：

```rust
// crates/app-pipeline/src/lib.rs:1422-1432（当前错误实现）
self.conv_store.truncate_from(&req.conversation_id, &req.node_id)  // 彻底删除旧节点
self.conv_store.append_ai_draft(...)                                // 新成文作为全新节点追加
```

`truncate_from`（app-conversation/src/lib.rs:443）是**节点级硬删除**——删掉目标节点及其后所有节点。这导致：
- 旧的 AI 回答**彻底消失**，无法切回查看
- variant 切换能力丢失（前端 useMessageVariants 的 8 个 handler 无法工作）
- 用户无法对比不同版本的重 roll 结果

### 正确的原始设计
ConversationStore **本来就支持** variant 保留 + 切换（P1-1 修复前的设计）：

- `add_variant`（:293）：新增 variant，旧的保留为 Final/Draft（分支语义）
- `replace_active_variant`（:328）：旧 active → **Discarded**（软删除，**可 switch 切回查看**），新 variant → active。注释明确：「soft_delete + add_variant 的原子组合」「避免无谓累积分支」
- `accept_variant`（:274）：把 Draft variant 提升为 Final

`replace_active_variant` 正是用户要的语义：重 roll 最后一条时，旧版本降级为 Discarded（可切回），新版本成为 active。

### 额外发现：执行时机
即使回到 variant 语义，还有个时机问题：当前 truncate/append 在整个 pipeline（Director→Subagent→Editor）**跑完之后**才执行（lib.rs:1422）。用户看到的是「生成完才删旧」。

正确做法：**先标记旧 variant 为正在重新生成（或先 add_variant 占位），再跑 pipeline**。这样：
- UI 立刻反映「正在重 roll」
- recent_messages 上下文不会读到旧回答（避免污染）
- 生成失败时旧 variant 仍在（不丢数据）

### 修复方向
1. **回退 P1-1**：regenerate 落库改回 `replace_active_variant`（最后一条）/ `add_variant`（中间消息），**不删节点**
2. **调整时机**：在 pipeline 开始前就执行 variant 操作（标记旧 variant Discarded + 占位新 variant），pipeline 完成后填入内容
3. **保留 variant 切换 UI**：前端 useMessageVariants 的 8 个 handler（prev/next/accept/fork 等）本就为此设计，P1-1 的 truncate 破坏了它们的契约

---

## 问题 2：流水线 trace 看不到事件

### 用户反馈
> 流水线 trace 是不是写死了上限 100 事件？我现在看不到事件。

### 调查
- **100 上限确认**：`stores/plugin.js` `MAX_PLUGIN_PIPELINE_EVENTS = 100`，`usePluginBridge.js:62` `slice(-100)` 裁剪。seraphina 一轮写作的事件数远不到 100（t1 suite 日志显示约 500 条事件，但那是含大量 progress delta 的流式事件，trace 面板按 event_type 归类后更少）。
- **链路完整**：后端 `start_writing`(lib.rs:2048) → event_rx → `from_pipeline_event` → Channel.send(WritingEvent) → 前端 `tauri-api.js:460` onEvent 回调 → `handlePipelineEvent`(usePipeline.js:24) → `broadcastPluginPipelineEvent`(usePluginBridge.js:66) → `pushPluginEventRecord` → `pluginStore.pluginPipelineEvents` → `PipelineTracePanel.vue` 读取。
- **WritingEvent 序列化**：`{event_type: String, data: Value}`，无 rename_all，前端检查 `event?.event_type` 匹配。

### 可能原因（待确认）
1. **时机问题**：trace 面板读的是 `pluginPipelineEvents`（内存数组）。如果用户在**写作完成后**重新打开 trace 面板，事件可能还在内存（不清空），但如果重启 app 或切换数据目录，内存丢失。
2. **事件未到达**：实际可能 Channel 回调没触发（Tauri v2 Channel 在某些场景下的事件投递问题）。
3. **需要实跑验证**：下一次 GUI 操作时在 trace 面板开着的状态下写一轮，观察是否有事件流入。

### 修复方向
- 先实跑确认：trace 面板开着时写作，看事件是否实时流入
- 如果确认是上限问题（一轮写作 >100 事件），提高上限或改为按轮次保留
- 考虑：trace 数据应持久化（写入 conversation 或独立文件），而非只在内存

---

## 问题 3：调试面板日志栏目空

### 用户反馈
> 调试界面的日志栏目也是空的。

### 调查
- **LogPanel**（components-v2/debug/LogPanel.vue）调 `log_query({limit: 100, kind?, level?})`
- **日志确实在写**：实跑产生的 `logs/2026-07-09.jsonl` 有 23 行（LlmCall kind），1.2MB
- **kind 映射正确**：log_query（lib.rs:4160-4165）做了映射 `"llm" => LogKind::LlmCall` 等，不是映射问题
- **level 映射正确**：`"info" => LogLevel::Info` 等，小写匹配
- **LogStore.query 读内存 buffer**（app-logging/src/lib.rs:272），不是读文件

### 根因（内存 vs 文件）
LogStore 是**内存 ring buffer**，P1-2 修复在启动时从 jsonl 回填到 buffer。但：
- LogPanel 只在 `onMounted` 时调 `loadLogs`（拉一次），**写作产生新日志后不自动刷新**
- 实跑场景：app 启动时临时目录是空的（buffer 回填读到空目录）→ 写作产生日志进 buffer → 但用户打开日志面板时如果没手动点「🔄 刷新」，看到的是空的

### 修复方向
- LogPanel 加**自动刷新或轮询**（写作完成后刷新日志，或定时 3-5 秒轮询）
- 或者：log_query 改为**直接读 jsonl 文件**（保证总能看到磁盘上的日志），而非只读内存 buffer

---

## 问题 4：ProcessReview 只有子 agent 没有导演和编剧

### 用户反馈
> 上一轮过程回顾界面里也只有子 agent 没有导演和编剧。

### 根因（已确认）
`components-v2/writing/ProcessReview.vue` 模板只有两个区块：
- 「🎬 导演规划」— 读 `writing.pipeline.director.output`
- 「🎭 子 Agent」— 读 `writing.pipeline.subagents`

**没有编剧区块**。`writing.pipeline.editor`（有 `output` 字段，usePipeline.js:82-94 有填充逻辑）完全没被 ProcessReview 消费。

### 修复方向
ProcessReview.vue 加「✍️ 编剧」区块，读 `writing.pipeline.editor.output`，与导演/子 Agent 区块并列展示。

---

## 附：postprocess 未完成（实跑数据证据）

### 证据
实跑 conversation（`conversations/5a2499dc-...json`）的 node[2]（AI 成文）状态是 **Draft**，未变成 Final。

### 根因（已查清：设计如此，非 bug）
postprocess **确实在调**（start_writing lib.rs:2157 `run_postprocess` + :2171 `persist_postprocess_outcome`）。node 停 Draft 不是 postprocess 失败，而是 **Draft→Final 的状态转换依赖用户手动操作**：

- `accept_variant`（app-conversation/src/lib.rs:274）把 Draft variant 提升为 Final
- 生产代码里 accept_variant 只在 Tauri 命令 `accept_variant`（lib.rs:4373）中暴露给前端
- 前端 `ChatMessage.vue:74 acceptVariant()` 是**用户手动点「接受」按钮**才调
- **没有自动路径**在写作/postprocess 完成后把 node 从 Draft 提升为 Final

这是**设计意图**：Draft = 用户可重 roll 的状态，accept 后变 Final 锁定。但 P1-1 的 truncate+append 破坏了这个流程——重 roll 删节点导致 accept 无目标，variant 切换失效。

### 与问题 1 的关联
问题 1 修复（回退到 variant 保留语义）会恢复 Draft→accept→Final 的完整流程：
- 重 roll 保留旧 variant（Discarded，可切回）+ 新 variant（Draft）
- 用户满意后点 accept → Draft 变 Final
- 不满意可继续重 roll，旧版本都在

---

## 其他已记录的问题（来自本次 GUI 实跑）

### B1-N1：runtime 硬编码 max_tokens=4096
- `crates/app-agent/src/runtime.rs:201` 和 `:335`：所有 LLM 请求用 `Default::default()`（max_tokens=4096）
- 大卡（命定之诗 6.4MB / 323K tokens）连续空响应，模型返回 content_len=0
- **用户指示**：不应设置 max_tokens，传 None 让模型用默认上限
- 已记录在 `docs/PHASE8-FOLLOWUP-ISSUES.md` 第五节

### B1-N2：大卡 input token 异常（323552）
- 待诊断：正常卡不该 32 万 token，疑似内容重复注入

### 导入后抽取无进度反馈（已修）
- `useCharacterImport.js` 的 extractCharacters 是 fire-and-forget，已改为 await + uiStore.extracting 进度状态
- AppV2.vue 加了提示条（「正在识别角色…」/错误显示）
- **此修复已通过 HMR 推送，但 app 进程超时退出，未最终验证**

---

## 第二轮 GUI 实跑发现（2026-07-09，d2f7897 修复后）

> commit `d2f7897` 修复了问题 1/4/3(轮询)/B1-N1 后重启 GUI 实跑，又暴露了新问题。

### 已验证修复生效的

- **角色抽取**：seraphina 小卡第 1 轮即识别成功（`emit_characters` 终止工具正常，max_tokens=None 修复后小卡不再空响应）。
- **重 roll variant 保留**：日志确认 `重 roll 完成: 2341 字`，variant 回退语义生效（旧版本不再被 truncate 删除）。
- **导入进度反馈**：提示条显示「正在识别角色…」。
- **Tabs 警告**：`Property "selected" was accessed during render` 修复（Tabs.vue 改用 `as="template"` + 内部 button）。

### 新发现问题 5：编剧生成完后消失，直到 postprocess 完成才出现

- **现象**：编剧流式输出完成后正文「消失」，直到后处理（postprocess）完成才重新出现。
- **根因（已确认）**：`tauri-app/src/lib.rs` 的 `start_writing` 命令在拿到成文（`start_writing` pipeline 返回）后，**同步 await postprocess**（原 :2157），阻塞了命令返回。前端 `useWriting.js:168` 在 `apiStartWriting` 返回后才 `showPipeline=false` + push 成文消息。时间线：编剧流式结束 → postprocess 跑 30-50 秒（StreamingMessage 还挂着但没新内容，看着像消失了）→ postprocess 完成后成文才 push。
- **修复方向（已实现，未提交验证）**：postprocess 改为 `tokio::spawn` 后台执行，`start_writing` 在成文后立即返回。前端立刻 push 成文，postprocess 后台跑并通过 Channel 推进度。
- **改动文件**：`crates/tauri-app/src/lib.rs`（未提交）。
- **状态**：已实现并编译通过，但因 app 进程反复超时退出未最终验证。

### 新发现问题 6：流水线 trace 面板看不到事件（F12 诊断后定位）

- **现象**：调试面板的「流水线」tab 看不到任何事件。
- **F12 诊断结果**：事件**确实在推送**——console 显示 `[trace-diag] broadcastPluginPipelineEvent director_started / subagent_progress / subagent_done / editor_started ...`，说明 `handlePipelineEvent → broadcastPluginPipelineEvent → pushPluginEventRecord` 链路完整工作。
- **count 恒为 100 的现象**：`plugin.pluginPipelineEvents` 始终 100 条（`slice(-100)` 裁剪），说明 store 确实在更新，事件在流入。
- **真正根因（疑似）**：trace 面板（PipelineTracePanel.vue）读 `plugin.pluginPipelineEvents` 的 computed 可能因 **Tabs 组件 bug（问题 8）导致 TabPanel 不渲染 slot 内容**。Tabs 修复后需要重新验证。另一个可能：**subagent_progress 等高频流式事件挤满 100 条上限**，把 director_started/director_done 等关键事件挤出，trace 面板过滤后看不到结构化事件。
- **已做改动（未提交）**：`MAX_PLUGIN_PIPELINE_EVENTS` 从 100 提升到 500。
- **状态**：需在 Tabs 修复后重新验证。如果仍然空，需进一步查 PipelineTracePanel 的 computed 是否真的重新计算（Pinia 响应式问题）。

### 新发现问题 7：日志面板点了能动但不显示数据

- **现象**：切到「日志」tab 后面板有响应（Tabs 修复后），但 DataTable 显示空。
- **诊断**：已加 `[log-diag] logQuery returned N entries` console.log，**但用户尚未反馈 N 的值**。
- **可能根因**：
  1. **N=0**：LogStore 内存 buffer 空——每次重启都清空临时目录，buffer 回填读到空目录；虽然 LlmInterceptor 会 push 到 buffer，但如果面板打开时 buffer 还没来得及积累，或 log_query 的 level 过滤把 Info 拦掉了。
  2. **N>0 但面板空**：DataTable 渲染问题或 LogEntryDto 字段名不匹配（已确认字段名匹配：id/kind/level/timestamp/message）。
- **修复方向**：
  - 如果 N=0：让 `log_query` **直接读 jsonl 文件**而非只读内存 buffer（B1-GUI-INVESTIGATION 已建议），保证总能看到磁盘日志。
  - 如果 N>0：查 DataTable 渲染。
- **状态**：**待用户提供 `[log-diag]` 的 N 值**，这是定位的最后一步。

### 新发现问题 8：Tabs 组件 headlessui 兼容 bug（已修）

- **现象**：`[Vue warn]: Property "selected" was accessed during render but is not defined on instance`，出现在 TabList/Tab 渲染时。
- **根因**：`ui/Tabs.vue` 的 `<Tab v-slot="{ selected }" :class="tabClass(selected)">` —— headlessui 的 `v-slot` slot prop 只暴露给**子内容**，不暴露给 Tab 元素自身的 `:class` 属性。`:class` 在 Tab 元素属性上求值时 `selected` 是 undefined。
- **影响**：所有 tab 显示为「未选中」样式。可能导致 TabPanel 不渲染 slot 内容（从而 trace/日志面板空）。
- **修复（已实现，未提交）**：Tab 改用 `as="template"` + 内部 `<button :class="tabClass(selected)">`，把 class 求值移到 v-slot 作用域内。
- **状态**：已通过 HMR 推送，用户确认「日志可以点的动了」（Tabs 生效），但日志数据仍不显示（问题 7 未解）。

### 新发现问题 9：后台进程反复超时退出

- **现象**：`cargo tauri dev` 和 `vite dev` 以后台任务方式启动，但都会在 600s/300s 后**超时被杀**（`Background task timed_out`），导致 app 窗口反复消失。
- **根因**：后台 Bash 命令的 timeout 上限（600000ms），但 `cargo tauri dev` 和 `vite dev` 是**长期运行的前台进程**，不会自行退出。
- **影响**：无法在单次 app 会话中完成完整验证，每次 app 只能存活约 10 分钟。
- **修复方向**：这不是代码问题，是工作流限制。需要用 `dangerouslyDisableSandbox` 或更长 timeout，或者用户自己在终端里启动 app 做长时间验证。

### 已做但未提交的改动（工作树中）

| 文件 | 改动 | 对应问题 |
| --- | --- | --- |
| `crates/tauri-app/src/lib.rs` | postprocess 改 spawn 后台执行 | 问题 5（编剧消失） |
| `frontend/src/components-v2/ui/Tabs.vue` | Tab 用 as=template + 内部 button | 问题 8（Tabs bug） |
| `frontend/src/stores/plugin.js` | MAX_PLUGIN_PIPELINE_EVENTS 100→500 | 问题 6（trace 上限） |
| `frontend/src/composables/usePluginBridge.js` | 加 console.log 诊断（**需清理**） | 问题 6（诊断） |
| `frontend/src/components-v2/debug/LogPanel.vue` | 加 console.log 诊断（**需清理**） | 问题 7（诊断） |

---

## 当前状态总结（2026-07-09 第二轮实跑后）

### 已修复并提交（commit d2f7897 + 09e15fe）
- ✅ 问题 1：重 roll variant 保留（回退 P1-1 truncate）
- ✅ 问题 4：ProcessReview 加编剧区块
- ✅ B1-N1：runtime max_tokens=None
- ✅ 导入抽取进度反馈

### 已实现未提交（工作树中，需验证后提交）
- 🔧 问题 5：postprocess 后台 spawn（编剧不再消失）
- 🔧 问题 8：Tabs headlessui 兼容修复（tab 不再报 selected 警告）
- 🔧 问题 6 部分：MAX_PLUGIN_PIPELINE_EVENTS 100→500

### 待定位（需更多信息）
- ⚪ 问题 6：trace 面板空——事件在推（F12 确认），Tabs 修复后需重新验证是否显示
- ⚪ 问题 7：日志面板空——需用户反馈 `[log-diag]` 的 N 值（N=0 → 后端 buffer 问题；N>0 → 渲染问题）

### 下一步
1. 提交已验证的 Tabs + postprocess spawn 修复（清理诊断日志后）
2. 重启 app，在 Tabs 修复后验证 trace 和日志面板
3. 根据日志面板的 `[log-diag]` N 值定位问题 7
4. 如果 trace 仍空，查 Pinia computed 响应式是否正常触发

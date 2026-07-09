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

## 修复优先级建议

1. **问题 1（重 roll variant）**：最高优先。回退 P1-1 的 truncate+append，改回 replace_active_variant + 时机调整。这是用户核心交互，必须正确。
2. **问题 4（ProcessReview 缺编剧）**：低风险前端改动，加一个区块。
3. **问题 3（日志空）**：查 log_query 映射，中优先。
4. **问题 2（trace 空）**：需实跑确认时机，中优先。
5. **B1-N1（max_tokens）**：runtime 改为不设 max_tokens（传 None），影响所有 agent。
6. **postprocess 未完成**：查 GUI 写作路径是否接 postprocess。

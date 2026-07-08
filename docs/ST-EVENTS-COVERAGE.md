# ST 事件覆盖索引

> 用途：记录 StoryForge 前端/后端目前已 emit 的 ST 风格事件及其来源，标记未覆盖的空缺。
> 日期：2026-07-08
> 诚实约束：当前覆盖度已为主生成链和常见聊天宿主动作提供事件推送，但**不是 ST 99 事件全集**。所有覆盖声明均可 grep 到代码行。

---

## 事件常量定义

32 个事件类型常量在 `frontend/src/plugin-bridge.js:20-51` 的 `ST_EVENT_TYPES` 中定义：

| 常量名 | 值 | 状态 |
|---|---|---|
| `APP_READY` | `APP_READY` | ✅ 已 emit |
| `CHAT_CHANGED` | `CHAT_CHANGED` | ✅ 衍生 emit |
| `CHAT_LOADED` | `CHAT_LOADED` | ✅ 已 emit |
| `MESSAGE_RECEIVED` | `MESSAGE_RECEIVED` | ✅ 已 emit |
| `MESSAGE_SENT` | `MESSAGE_SENT` | ✅ 已 emit |
| `MESSAGE_UPDATED` | `MESSAGE_UPDATED` | ✅ 已 emit |
| `MESSAGE_DELETED` | `MESSAGE_DELETED` | ✅ 已 emit |
| `MESSAGE_SWIPED` | `MESSAGE_SWIPED` | ✅ 已 emit |
| `GENERATION_STARTED` | `GENERATION_STARTED` | ✅ alias emit |
| `GENERATION_STOPPED` | `GENERATION_STOPPED` | ❌ 未 emit |
| `GENERATION_ENDED` | `GENERATION_ENDED` | ✅ alias emit |
| `STREAM_TOKEN` | `STREAM_TOKEN` | ✅ alias emit |
| `CHARACTER_LOADED` | `CHARACTER_LOADED` | ✅ 已 emit |
| `CHARACTER_MESSAGE_RENDERED` | `CHARACTER_MESSAGE_RENDERED` | ✅ 衍生 emit |
| `USER_MESSAGE_RENDERED` | `USER_MESSAGE_RENDERED` | ✅ 衍生 emit |
| `WORLDINFO_SETTINGS_UPDATED` | `WORLDINFO_SETTINGS_UPDATED` | ❌ 未 emit |
| `WORLDINFO_UPDATED` | `WORLDINFO_UPDATED` | ❌ 未 emit |
| `WORLDINFO_FORCE_ACTIVATE` | `WORLDINFO_FORCE_ACTIVATE` | ❌ 未 emit |
| `GENERATE_BEFORE_COMBINE_PROMPTS` | `GENERATE_BEFORE_COMBINE_PROMPTS` | ✅ 已 emit（prompt hook） |
| `GENERATE_AFTER_COMBINE_PROMPTS` | `GENERATE_AFTER_COMBINE_PROMPTS` | ❌ 未 emit |
| `CHAT_COMPLETION_PROMPT_READY` | `CHAT_COMPLETION_PROMPT_READY` | ✅ 已 emit（prompt hook） |
| `TOOL_CALLS_PERFORMED` | `TOOL_CALLS_PERFORMED` | ❌ 未 emit |
| `TOOL_CALLS_RENDERED` | `TOOL_CALLS_RENDERED` | ❌ 未 emit |
| `GROUP_UPDATED` | `GROUP_UPDATED` | ❌ 未 emit |
| `GROUP_MEMBER_DRAFTED` | `GROUP_MEMBER_DRAFTED` | ❌ 未 emit |
| `GROUP_WRAPPER_FINISHED` | `GROUP_WRAPPER_FINISHED` | ❌ 未 emit |
| `SETTINGS_LOADED` | `SETTINGS_LOADED` | ❌ 未 emit |
| `SETTINGS_UPDATED` | `SETTINGS_UPDATED` | ❌ 未 emit |
| `EXTENSION_SETTINGS_LOADED` | `EXTENSION_SETTINGS_LOADED` | ❌ 未 emit |
| `EXTENSIONS_FIRST_LOAD` | `EXTENSIONS_FIRST_LOAD` | ❌ 未 emit |

## ST_EVENT_ALIASES（后端 pipeline event_type → ST 事件名映射）

定义在 `frontend/src/plugin-bridge.js:53-59`：

| Pipeline event_type | 别名 ST 事件 |
|---|---|
| `started` | `GENERATION_STARTED` |
| `editor_progress` | `STREAM_TOKEN` |
| `draft_ready` | `GENERATION_ENDED` |
| `committed` | `MESSAGE_RECEIVED`, `CHARACTER_MESSAGE_RENDERED`, `CHAT_CHANGED` |
| `error` | `GENERATION_STOPPED` |

> `committed` 别名已定义，`from_pipeline_event` 也处理了 `PipelineEvent::Committed` 变体，但流水线状态机实际发送的是 `StateChanged{state:Committed}`，而非 `Committed` 变体——因此 `committed` event_type 在正常写作流程中不会到达前端。

## 已 emit 事件详情

### 前端 direct emit（`broadcastPluginEvent` / `emitPromptHookEventAndWait`）

> **位置声明（2026-07-08 Phase 8 重构后）**：原 `App.vue` 已拆分为 `AppV2.vue` + `composables/`。下表"App.vue:行"是重构前的历史行号，仅作 emit 点语义参考。重构后 emit 点分布在：
> - `composables/usePluginBridge.js`（broadcast/emit/audit 封装）
> - `composables/useConversation.js`（CHAT_LOADED / MESSAGE_RECEIVED / restart）
> - `composables/useMessageVariants.js`（MESSAGE_UPDATED / MESSAGE_SWIPED / MESSAGE_DELETED）
> - `composables/useWriting.js`（MESSAGE_SENT / GENERATE_BEFORE_COMBINE_PROMPTS / CHAT_COMPLETION_PROMPT_READY）
> - `AppV2.vue`（APP_READY onMounted / CHARACTER_LOADED）
>
> 如需精确行号，用 `grep -n "<事件名>" frontend/src/AppV2.vue frontend/src/composables/*.js` 重新定位。事件名、触发路径、衍生关系**未变**，只是承载文件迁移了。

| ST 事件 | Emit 位置（重构前 App.vue:行，已迁移见上） | 触发路径 | 备注 |
|---|---|---|---|
| `APP_READY` | `App.vue:398` | 应用初始化 `onMounted` | — |
| `CHAT_LOADED` | `App.vue:510,526,594,1045` | init / new conversation / restart / switch character | 4 个不同入口 |
| `MESSAGE_SENT` | `App.vue:737` | 用户发送消息/意图 | — |
| `MESSAGE_RECEIVED` | `App.vue:769,787` | 成文后写入 conversation | streaming / 非 streaming 各一次 |
| `MESSAGE_UPDATED` | `App.vue:868,898,914,991` | reroll / edit / accept_variant / reroll_user | 4 种不同原因 |
| `MESSAGE_DELETED` | `App.vue:933` | 删除消息 | — |
| `MESSAGE_SWIPED` | `App.vue:1072,1090` | 切 variant | front/back 各一次 |
| `CHARACTER_LOADED` | `App.vue:648,686` | CardPanel 选择 / 重启恢复 | — |
| `GENERATE_BEFORE_COMBINE_PROMPTS` | `App.vue:252` | 写作前 prompt hook（intent 路径） | hook 可等待 |
| `CHAT_COMPLETION_PROMPT_READY` | `App.vue:253,264` | 写作前 prompt hook（intent + chat completion） | 可被插件改写 |

### 前端 derived emit（`deriveSillyTavernHostEventNames` 衍生）

| ST 事件 | 源事件 | 触发条件 |
|---|---|---|
| `USER_MESSAGE_RENDERED` | `MESSAGE_SENT` | 用户发消息后自动衍生 |
| `USER_MESSAGE_RENDERED` | `MESSAGE_UPDATED` | 用户角色消息被更新 |
| `CHARACTER_MESSAGE_RENDERED` | `MESSAGE_RECEIVED` | 收到 AI 回复后自动衍生 |
| `CHARACTER_MESSAGE_RENDERED` | `MESSAGE_UPDATED` | assistant 角色消息被更新 |
| `CHARACTER_MESSAGE_RENDERED` | `MESSAGE_SWIPED` | 切 variant 后自动衍生 |
| `CHAT_CHANGED` | 所有 MESSAGE_* 事件 + CHAT_LOADED | 对话内容变化后 |

### 后端 PipelineEvent → 前端（via Tauri event + `mapPipelineEventToPluginEvents`）

PipelineEvent 通过 `WritingEvent::from_pipeline_event` 转为 `{ event_type, data }`，经 Tauri event 送达前端。`broadcastPluginPipelineEvent` + `mapPipelineEventToPluginEvents` 生成以下事件名：

- `pipeline.<event_type>`（如 `pipeline.started`）
- 原始 `event_type`（如 `started`）
- ST_EVENT_ALIASES 映射（如 `started` → `GENERATION_STARTED`）

实际已映射的 pipeline event_type：

| event_type | 对应 PipelineEvent | ST 别名 | 触发时机 |
|---|---|---|---|
| `started` | `PipelineEvent::Started` | `GENERATION_STARTED` | 写作/重 roll 开始 |
| `director_started` | `PipelineEvent::DirectorStarted` | — | Director 开始 |
| `director_progress` | `PipelineEvent::DirectorProgress` | — | Director 流式输出 |
| `director_done` | `PipelineEvent::DirectorDone` | — | Director 完成 |
| `subagent_started` | `PipelineEvent::SubagentStarted` | — | 子 Agent 开始 |
| `subagent_progress` | `PipelineEvent::SubagentProgress` | — | 子 Agent 进度 |
| `subagent_done` | `PipelineEvent::SubagentDone` | — | 子 Agent 完成 |
| `subagent_cancelled` | `PipelineEvent::SubagentCancelled` | — | 子 Agent 取消 |
| `editor_started` | `PipelineEvent::EditorStarted` | — | Editor 开始 |
| `editor_progress` | `PipelineEvent::EditorProgress` | `STREAM_TOKEN` | Editor 流式输出 |
| `draft_ready` | `PipelineEvent::DraftReady` | `GENERATION_ENDED` | Editor 成文完成 |
| `prompt_hook_request` | `PipelineEvent::PromptHookRequest` | — | 过滤：不转发给插件 |
| `postprocess_started` | `PipelineEvent::PostProcessStarted` | — | 后处理开始 |
| `postprocess_done` | `PipelineEvent::PostProcessDone` | — | 后处理完成 |
| `postprocess_failed` | `PipelineEvent::PostProcessFailed` | — | 后处理失败 |
| `postprocess_skipped` | `PipelineEvent::PostProcessSkipped` | — | 后处理被配置跳过 |
| `summary_done` | `PipelineEvent::SummaryDone` | — | 摘要完成 |
| `committed` | `PipelineEvent::Committed` | `MESSAGE_RECEIVED`, `CHARACTER_MESSAGE_RENDERED`, `CHAT_CHANGED` | 成文提交到对话 |

### Prompt hook 事件流

- `GENERATE_BEFORE_COMBINE_PROMPTS`：frontend intent hook（`App.vue:252`），写作前修改 intent/prompt
- `CHAT_COMPLETION_PROMPT_READY`：frontend + backend hook（`App.vue:253,264` + `start_writing`/`regenerate` 后端 prompt_hook_request 链路）
- prompt hook 有基础脱敏审计记录（重构后承载在 `frontend/src/composables/usePluginBridge.js` 的 `recordPromptHookAudit`，原 `App.vue:178-185` 已迁移；底层 `frontend/src/utils/promptHooks.js` 的 `appendPromptHookAuditRecord` 未变）

### 插件事件订阅与脱敏

- `mapPipelineEventToPluginEvents` (plugin-bridge.js:256-272) 按 `event_subscriptions` 和 `ReadMemory` 权限过滤事件
- 无 `ReadMemory` 的订阅者的正文类字段会被脱敏（`sanitizePluginEventData`）
- TavernHelper APIs 通过 `mapPluginEventRecordToPluginEvents` 兼容

## 未 emit 事件（空缺）

以下 `ST_EVENT_TYPES` 中定义的事件在代码中**没有任何 emit 点**：

| 事件 | 原因/备注 |
|---|---|
| `GENERATION_STOPPED` | pipeline 失败/取消时无对应 emit；`error` alias 已定义但未使用 |
| `WORLDINFO_SETTINGS_UPDATED` | 世界书通过 Director system/tail 静默注入，无 UI 设置变更事件 |
| `WORLDINFO_UPDATED` | 同上 |
| `WORLDINFO_FORCE_ACTIVATE` | 无强制激活 UI 操作 |
| `GENERATE_AFTER_COMBINE_PROMPTS` | hook 链路只实现了 BEFORE 和 READY |
| `TOOL_CALLS_PERFORMED` | Agent 工具调用不为 ST 插件可见，无对应 emit |
| `TOOL_CALLS_RENDERED` | 同上 |
| `GROUP_UPDATED` | StoryForge 无 group 概念 |
| `GROUP_MEMBER_DRAFTED` | 同上 |
| `GROUP_WRAPPER_FINISHED` | 同上 |
| `SETTINGS_LOADED` | StoryForge UI 设置不通过 ST 事件通知 |
| `SETTINGS_UPDATED` | 同上 |
| `EXTENSION_SETTINGS_LOADED` | 同上 |
| `EXTENSIONS_FIRST_LOAD` | 同上 |

## 测试覆盖

事件映射逻辑的测试在 `frontend/tests/plugin-bridge.test.mjs`：
- `maps pipeline events to native plugin event names`（行 ~561）
- `adds SillyTavern aliases for generation lifecycle events`（行 ~573）
- `adds STREAM_TOKEN alias with token payload for editor deltas`（行 ~579）
- `derives common SillyTavern render and chat events from host message events`（行 ~594）
- `redacts message content from subscribed plugins without ReadMemory`（行 ~641）
- `dispatches host events through storyforge.events and ST eventSource`（行 ~662）
- etc.

共约 150+ 行事件相关测试用例覆盖了前端映射和 iframe 注入逻辑。

## 诚实约束

> 发布说明不要承诺：
> - "完整 ST 99 事件全集"——当前已 emit 的主链路事件（~10 个 direct + ~5 个 alias + ~15 个 pipeline）覆盖常用场景，**不是 ST 全量**。
> - "ST 冷门事件/冷门 Slash/TavernHelper 语义"
> - "full prompt hook audit UI/export"
>
> 可以说：
> - "主生成事件、常见聊天事件别名已接入"
> - "`eventSource`/`TavernHelper` 常用 shim 已提供"
> - "prompt hook 基础脱敏审计记录已补"

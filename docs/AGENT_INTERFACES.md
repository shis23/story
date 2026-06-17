# Agent 接口

本文定义 StoryForge 中 Agent 的职责、输入输出和当前缺口。

## Agent 分工

```text
User Intent
  -> Director Agent
      生成场景计划和角色任务
  -> Subagent x N
      按角色隔离表演
  -> Editor Agent
      合并成最终正文
  -> Postprocess Agents
      生成摘要、知识、变量和任务更新
  -> Meta Agent
      解释、诊断、补丁建议、MVU/数据健康分析
```

## Director Agent

职责：

- 理解用户意图。
- 查找可用角色和世界设定。
- 确定本场戏核心冲突。
- 选择出场角色。
- 为每个角色生成 `SubagentTask`。

当前工具：

- `search_world_info(query)`
- `get_character(name)`
- `emit_plan(scene_brief, subagent_tasks)`
- `search_vectors(query, top_k)`
- `get_recent_summary(limit)`

当前输出：

```json
{
  "scene_brief": "本场戏的一句话场景简述",
  "subagent_tasks": [
    {
      "character_id": "当前仍可能是角色名",
      "brief": "该角色在本场戏的任务"
    }
  ]
}
```

目标输出：

```json
{
  "scene_brief": "本场戏的一句话场景简述",
  "subagent_tasks": [
    {
      "character_instance_id": "stable-id",
      "display_name": "角色名",
      "brief": "该角色在本场戏的任务"
    }
  ],
  "requested_temporary_characters": [
    {
      "name": "临场角色名",
      "reason": "为什么需要这个角色"
    }
  ]
}
```

## Subagent

职责：

- 只演自己的角色。
- 根据角色 persona、behavior、当前变量、已知信息和本场任务输出表演。
- 不替其他角色做决定。

当前输入是 `ContextPackage`：

- `character_brief`
- `scene_brief`
- `relevant_lore`
- `constant_lore`
- `recent_window`
- `task`

当前缺口：

- `character_brief` 仍主要从扁平角色卡构造。
- 角色身份未严格绑定 `CharacterInstance.id`。
- 角色知识隔离还不完整，子 Agent 可能看到不该知道的全局事实。

目标：

- stable system 段放 persona、behavior、常驻世界设定。
- volatile tail 放当前场景、任务、变量、可见知识、最近窗口。
- 每个 Subagent 只能拿到该角色视角允许的信息。

## Editor Agent

职责：

- 合并多个 Subagent 的表演。
- 保持连贯叙事、节奏和视角。
- 输出最终 Markdown 正文。
- 不新增与 Subagent 冲突的关键事实，除非是必要的叙事衔接。

当前 Editor 没有工具，直接输出正文。重 roll 时可以复用旧 Director Plan 或旧 Subagent 产出，并注入用户 hint。

目标：

- 保留更清晰的 provenance：哪些子表演被采用、删改或冲突解决。
- 对结构化冲突给出可诊断记录，供 Meta Agent 解释。

## Postprocess

职责：

- 从最终正文中抽取本轮摘要。
- 抽取角色知识更新。
- 抽取 Campaign/角色变量更新。
- 抽取任务状态变化或新伏笔。

原则：

- best-effort，不阻断正文写入。
- 输出必须经过 Tauri 层校验后才能持久化。
- 名字型引用必须归一化到 ID。
- 不得凭空创建永久角色；临场角色必须走 temporary instance 请求。

## Meta Agent

Meta Agent 的方向不是“再做一个聊天助手”，而是 StoryForge 的可解释性和维护层。

建议职责：

- 解释某段正文为什么这样生成：Director Plan、Subagent 输入、Editor 合并依据。
- 检查 Campaign 数据健康：重复角色、孤儿 instance、变量类型错误、知识归属异常。
- 分析 MVU 卡：变量 schema、initvar、状态更新模式。
- 提出补丁建议：修改 prompt module、修复变量、合并重复定义、补全角色信息。
- 生成可审阅 patch，由用户接受后再应用。

不建议让 Meta Agent 直接参与常规写作，否则会和 Director/Editor 职责混淆。

## AgentProfileConfig 运行时消费

`AgentProfileConfig`（定义在 `crates/domain/src/agent_profile_config.rs`）控制 Agent 运行时参数，已在写作链路完整消费：

- Director/Editor 的 `make_*_config` 从 profile 读取 `model_override` 和 `max_tool_rounds`，覆盖硬编码默认值。
- `spawn_subagents` 接收 `max_concurrent_subagents` 和 `agent_profile_config`；子 Agent 按 profile 覆盖 model 和 max_tool_rounds。
- `tool_whitelist` 在 Director/Subagent/PostProcessor 注册工具后通过 `filter_registry_by_whitelist` 过滤（`None` = 默认工具集，`Some([])` = 禁用全部，`Some(list)` = 只保留列表中的；未知工具名记 warning 后忽略，不 panic）。被禁用的工具 dispatch 返回 `ToolError::NotFound`，whitelist 不可绕过。
- `enable_postprocess` / `enable_summarizer` 控制后处理：`false` 时跳过对应 LLM 调用；两者都 `false` 时发 `PostProcessSkipped`（非 `PostProcessFailed`）；无 config 时全开（向后兼容）。

## 工具注册表

各 Agent 角色的工具在 `crates/app-agent/src/tools.rs` 和 `crates/app-agent/src/prompts/` 中注册：

| Agent | 工具 | 注册函数 |
| --- | --- | --- |
| Director | `search_world_info`, `get_character`, `emit_plan`, `search_vectors`, `get_recent_summary` | `register_director_tools` |
| Subagent | `get_character`（信息隔离：有 `current_character_instance_id` 时只返回自己的 instance） | `register_subagent_tools` |
| Editor | `compose` | `register_editor_tools` |
| Postprocess | `emit_postprocess`（声明产出，handler 原样返回 args） | `register_postprocess_tools` |
| CharacterExtractor | `emit_characters`（声明产出，handler 原样返回 args） | `register_character_extractor_tools` |

`ToolRegistry::retain(Option<&[String]>)` 按白名单保留工具，过滤后 `tool_specs()`（发给 LLM）与 `dispatch` 同步收窄。

## 流式事件

写作和重 roll 通过 `PipelineEvent` 推给前端（定义在 `crates/domain/src/agent.rs`），完整变体：

- `Started { session_id }`
- `StateChanged { state }`（PipelineState：Generating / Editing / Review / Committed / Aborted）
- `DirectorStarted`
- `DirectorProgress { delta }`
- `DirectorDone { scene_brief, subagent_count }`
- `SubagentStarted { character_id, index, total }`
- `SubagentProgress { character_id, index, delta }`
- `SubagentDone { character_id, index, full_text }`
- `SubagentCancelled { character_id, index }`
- `EditorStarted`
- `EditorProgress { delta }`
- `DraftReady { text }`
- `PostProcessStarted`
- `PostProcessDone { knowledge_count, variable_count, task_count }`
- `PostProcessFailed { reason }`（best-effort，不阻断成文）
- `PostProcessSkipped { reason }`（AgentProfileConfig 关闭 postprocess/summarizer 时发出，区别于真失败）
- `SummaryDone { char_count }`
- `Committed { session_id, variant_id }`
- `Error { message }`

前端应把这些事件视为流水线观察信号，不应把临时事件当成持久状态真相源。

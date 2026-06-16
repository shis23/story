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

## 流式事件

写作和重 roll 通过 `PipelineEvent` 推给前端，主要阶段：

- `Started`
- `StateChanged`
- `DirectorStarted`
- `DirectorProgress`
- `DirectorDone`
- `SubagentStarted`
- `SubagentProgress`
- `SubagentDone`
- `EditorStarted`
- `EditorProgress`
- `DraftReady`
- `PostProcessStarted`
- `PostProcessDone`
- `SummaryDone`
- `Committed`
- `Error`

前端应把这些事件视为流水线观察信号，不应把临时事件当成持久状态真相源。

# StoryForge 数据模型

本文给出当前模型关系和后续统一方向。

## 模型分层

```text
Character
  原始 ST 角色卡解析结果，保留 spec、extensions、raw JSON

CharacterCard
  StoryForge 内部的一张卡容器，可包含多个 CharacterDefinition

CharacterDefinition
  卡级角色模板：persona、behavior、backstory、role type、变量 schema

Campaign
  一局故事存档：card_id、story_clock、campaign variables、fork 信息

CharacterInstance
  Campaign 内的角色实例：instance id、definition_id、override、变量、temporary 标记

CharacterKnowledgeEntry
  某角色在某 Campaign 中知道的事实

StoryTask
  剧情伏笔/任务，供 Director 在合适轮次注入

RoundSummary
  每轮成文后的摘要，用于记忆和回溯
```

## Character

`Character` 位于 `crates/domain/src/character.rs`。它是 ST 导入后的原始领域表示，包含：

- `name`、`description`、`personality`、`scenario`、`first_mes` 等 ST 常用字段。
- `embedded_world_info`。
- `extensions` 和 `raw_card_json`，用于兼容和导出。
- `renderable_assets`，用于卡片内 HTML/CSS/JS 资产渲染。

它不应长期承担 Campaign 写作时的唯一角色真相源。

## CharacterCard 和 CharacterDefinition

`CharacterCard` 是一卡多角色容器：

- `id`
- `name`
- `source_character_id`
- `character_definitions`

`CharacterDefinition` 是卡级模板：

- `id`
- `card_id`
- `name`
- `persona_prompt`
- `behavior_rules`
- `base_backstory`
- `group`
- `role_type`
- `variable_schema`

导入 ST 卡后，角色识别 Agent 应产出一组 `CharacterDefinition`。识别失败时使用 `fallback_from_character` 生成单主角定义。

## Campaign

`Campaign` 位于 `crates/domain/src/campaign.rs`，表示一局故事：

- `id`
- `card_id`
- `name`
- `fork_from`
- `created_at`
- `variables`
- `story_clock`

`story_clock` 当前同时存在于顶层字段和 variables 中。代码注释已经说明：兼容旧数据时保留顶层字段，但语义上应以 variables 为准，未来可通过数据迁移统一。

## CharacterInstance

`CharacterInstance` 是 Campaign 内的运行时角色：

- `id`
- `campaign_id`
- `definition_id`
- `name`
- `persona_override`
- `behavior_override`
- `variables`
- `is_temporary`

目标语义：

- 常规角色从 `CharacterDefinition` 实例化。
- 临场角色使用 `temporary` 创建。
- 写作链路内部使用 `CharacterInstance.id`。
- `resolved_persona()` 和 `resolved_behavior()` 应优先使用 override，再 fallback 到 definition。

当前代码中 `resolved_persona()` 只返回 override，这是 Campaign 统一计划的第一步修正点。

## 知识、变量、任务

`CharacterKnowledgeEntry` 表示角色知道的事实。它应绑定到 Campaign 和角色实例/角色定义的稳定 ID，避免不同存档串线。

变量分两类：

- Campaign 变量：天气、时间、全局状态、剧情阶段。
- CharacterInstance 变量：HP、关系、状态、疲劳、MVU 扩展字段。

`StoryTask` 表示导演可注入的伏笔和任务，触发条件包括轮次、故事时间、关键词等。任务不应直接变成正文，而是作为 Director tail 的结构化输入。

## 推荐运行时快照

后续应新增纯应用层 DTO：

```text
CampaignRuntimeContext
  campaign
  card
  definitions
  instances
  campaign_variables
  character_variables
  knowledge
  pending_tasks
  summaries
  world_info
  recent_messages
```

它应由 Tauri 层从 store 组装，然后传给 `app-pipeline`。`app-agent` 工具只能读这个快照，不能直接持有 `CampaignStore`。

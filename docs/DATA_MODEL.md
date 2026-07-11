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
  另：`revision`（状态 CAS）、`chronicle_revision`、`lineage_id`、`context_epoch`（ContextEpochSnapshot）

CharacterInstance
  Campaign 内的角色实例：instance id、definition_id、override、变量、temporary 标记

CharacterKnowledgeEntry
  某角色在某 Campaign 中知道的事实

StoryTask
  剧情伏笔/任务，供 Director 在合适轮次注入

RoundSummary（Chronicle A/B/C 统一存储形态）
  字段：id/campaign/conversation/turn/content + `code`/`headline`/`lineage_id`/`covered_by`/`level`/`turn_end`/`covers`
  A：Summarizer → Accept 落盘；B/C：ChronicleCompressor 后台发布
  转换：`to_chronicle_a` / `from_chronicle_entry`

ChronicleEntry A/B/C（`crates/domain/src/chronicle.rs`）
  主键 chronicle_entry_id；code 为 (campaign_id, lineage_id) 内别名
  covers/covered_by 折叠默认概览；不删底层记录

ArchivedSummary（已有）
  消息正文批压缩（MemoryArchiver）；非规范轮次纪要
  向量 source_kind 区分；auto recall 可用；v1 不进 search_chronicle

CompressJob（M4 队列）
  `data/compress_jobs.json`：Pending/Running/Succeeded/Failed；campaign 级 open 去重；启动 Running→Pending 重放
```

权威规格（v1.0：身份、epoch 公式、revision、主从、工具边界）：

- [`docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`](./MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md)

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

### Campaign fork

`Campaign.fork_from` records `(source_campaign_id, fork_node_id)`. The Tauri
`fork_campaign` command creates a new Campaign from that source, copies the
source Campaign variables and `story_clock`, clones the source
`CharacterInstance` snapshot with fresh instance ids, and creates a new
conversation copied through the fork node. The source Campaign and source
conversation remain unchanged.

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

~~当前代码中 `resolved_persona()` 只返回 override，这是 Campaign 统一计划的第一步修正点。~~ **阶段 1 已修复**：`resolved_persona(definition)` / `resolved_behavior(definition)` 接收 `Option<&CharacterDefinition>`，override 优先，无 override 时 fallback 到 definition。

## 知识、变量、任务

`CharacterKnowledgeEntry` 表示角色知道的事实。它应绑定到 Campaign 和角色实例/角色定义的稳定 ID，避免不同存档串线。

变量分两类：

- Campaign 变量：天气、时间、全局状态、剧情阶段。
- CharacterInstance 变量：HP、关系、状态、疲劳、MVU 扩展字段。

`StoryTask` 表示导演可注入的伏笔和任务，触发条件包括轮次、故事时间、关键词等。任务不应直接变成正文，而是作为 Director tail 的结构化输入。

## CampaignRuntimeContext（阶段 2 已实现）

~~后续应新增纯应用层 DTO~~ **已实现**：`crates/domain/src/campaign_runtime.rs` 定义纯域快照 `CampaignRuntimeContext`，由 Tauri 层从 `CampaignStore` 组装，传入 `app-pipeline` / `app-agent`。`app-agent` 工具只读此快照，不直接持有 `CampaignStore`。

当前字段：

```text
CampaignRuntimeContext
  campaign: Campaign
  instances: Vec<CharacterInstance>
  definitions_by_id: HashMap<Id, CharacterDefinition>
  knowledge: Vec<CharacterKnowledgeEntry>
  turn: u32
```

辅助方法：`find_instance_by_id_or_name`、`definition_for_instance`、`resolved_persona_for`、`resolved_behavior_for`、`with_temporaries_for`（阶段 6：为未匹配角色创建临时 instance）。

注：早期文档列出了 `card`、`campaign_variables`、`character_variables`、`pending_tasks`、`summaries`、`world_info`、`recent_messages` 等字段。实际实现中这些数据分别通过 `WritingContext` 的独立字段（`campaign_id`、`turn`、`pending_tasks`、`story_clock`、`world_info`）、按需读取的 `ConversationStore::recent_messages_as_chat()`，以及 `Campaign.variables` / `CharacterInstance.variables` 承载，未全部合并进 `CampaignRuntimeContext`。

## AgentProfileConfig（已实现）

`crates/domain/src/agent_profile_config.rs` 定义两个核心结构：

### AgentRunConfig

单个 Agent 角色的运行时配置覆盖（全 `Option`，`None` = 使用默认值）：

- `model_override: Option<String>` — 模型覆盖
- `max_tool_rounds: Option<u32>` — 最大工具轮次
- `tool_whitelist: Option<Vec<String>>` — 工具白名单（`None` = 默认工具集，`Some([])` = 禁用全部，`Some(list)` = 只允许列表中的）

### AgentProfileConfig

完整配置（持久化用）：

- `id: Id`
- `name: String`
- `description: String`
- `agent_configs: HashMap<AgentRole, AgentRunConfig>` — 每个角色的运行参数覆盖
- `max_concurrent_subagents: usize` — 子 Agent 最大并发数（构造时 clamp 到 ≥1）
- `enable_postprocess: bool` — 是否启用后处理
- `enable_summarizer: bool` — 是否启用剧情总结
- `source: ProfileSource` — 来源（BuiltIn / UserCreated）
- `config_version: u32` — 版本号

内置默认 ID：`builtin-default-agent-v1`，始终可用、不可删除。

辅助方法：`effective_max_concurrent_subagents()`（clamp 0→1）、`sanitize()`（修正反序列化后非法字段）、`run_config_for(role)`（支持 Subagent 通配符回退）、`is_builtin()`。

### ProfileConfigError

`crates/domain/src/agent_profile_config.rs` 定义了 `ProfileConfigError` 枚举（阶段 5 新增），由 `AgentProfileConfig::validate()` 返回：

- `EmptyName` — 名称为空或纯空白
- `MaxToolRoundsOutOfRange { role, value }` — 某角色的 `max_tool_rounds` 超出 `[1, 100]`
- `InvalidMaxConcurrent { value }` — `max_concurrent_subagents` 小于 1

`AgentProfileConfig::migrate_to(target)` 提供版本迁移入口（当前仅 v1，no-op；未知版本不报错并更新 `config_version`）。`AgentProfileConfigStore::save` 保存前调 `validate()`，`new`/`get`/`get_active`/`set_active` 加载时调 `migrate_to(1)`。

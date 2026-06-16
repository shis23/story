# 计划：Campaign 写作主线

> 状态：待执行
> 目标读者：可交给小模型按阶段执行
> 关联：`docs/ARCHITECTURE-AUDIT.md`、`docs/PLAN-CHARACTER-UNIFICATION.md`

## 目标

把开 Campaign 后的写作链路从“扁平 Character 主导”改成“Campaign + CharacterInstance 主导”。

完成后：

- 未开 Campaign：继续兼容现有 SillyTavern 单卡写作。
- 已开 Campaign：Director、Subagent、Postprocess、Provenance 都优先使用 `CharacterInstance.id`。
- 变量、知识、任务、摘要能进入下一轮写作，而不是只在 Campaign 面板里展示。

## 非目标

- 不重写 JSON store。
- 不删除 `CharacterStore`。
- 不把 `CampaignStore` 直接依赖进 `app-agent` 或 `app-pipeline`。
- 不做通用插件系统。
- 不改前端大布局，前端入口改造放到 `PLAN-FRONTEND-WORKBENCH.md`。

## 当前事实

- `crates/tauri-app/src/lib.rs::start_writing` 用 `snapshot_tool_ctx()` 构造 `WritingContext.characters`。
- `crates/tauri-app/src/lib.rs::fill_campaign_context` 只填 `campaign_id/turn/pending_tasks/story_clock`。
- `crates/app-pipeline/src/lib.rs::WritingContext` 没有 Campaign runtime 字段。
- `crates/app-pipeline/src/lib.rs::build_director_tail` 从 `ctx.characters` 渲染可用角色。
- `crates/app-agent/src/tools.rs::ToolContext` 没有 Campaign runtime 字段。
- `crates/app-agent/src/runtime.rs::spawn_subagents` 只接收 `SubagentTask` 和 runtime，不接收 Campaign 快照。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` 已能写 CampaignStore，但 ID 归一和 `present_chars` 使用不完整。

## 阶段 0：基线保护

目标：执行前确认当前测试和关键路径状态，避免把已有坏状态误归因到本计划。

改动文件：无。

操作：

1. 运行 `cargo test -p storyforge-domain`。
2. 运行 `cargo test -p storyforge-app-agent`。
3. 运行 `cargo test -p storyforge-app-pipeline`。
4. 运行 `cargo test -p storyforge`。
5. 记录失败项，不修无关失败。

验收：

- 有一份测试基线记录。
- 不产生代码改动。

## 阶段 1：补齐 domain 层读取能力

目标：让 `CharacterInstance` 能从 `CharacterDefinition` fallback 出完整设定。

改动文件：

- `crates/domain/src/campaign.rs`
- 如需要，新增 `crates/domain/src/campaign_runtime.rs`
- `crates/domain/src/lib.rs`

任务：

1. 给 `CharacterInstance` 或 `CampaignRuntimeContext` 增加读取 helper。当前代码里 `CharacterInstance` 只有 `persona_override` / `behavior_override`，没有 backstory override；不要假装实例已经能承载所有 definition 字段：
   - `resolved_persona(definition: Option<&CharacterDefinition>)`
   - `resolved_behavior(definition: Option<&CharacterDefinition>)`
   - `resolved_backstory` 可作为 runtime/helper 函数直接读取 `CharacterDefinition.base_backstory`；只有显式新增 instance backstory override 字段后，才放到 `CharacterInstance` 方法上。
   - `resolved_variable_schema` 可作为 runtime/helper 函数直接读取 `CharacterDefinition.variable_schema`；实例只保存当前变量值。
2. persona/behavior 走 override 优先，definition 兜底。
3. 不把 definition 字段复制进 instance，保持 instance 轻量。
4. 增加单元测试覆盖 override、fallback、none，以及 backstory/schema helper 不依赖实例复制字段。

验证：

```bash
cargo test -p storyforge-domain
```

验收：

- `CharacterInstance` 不再只能返回 override。
- 现有序列化格式不破。

## 阶段 2：新增 CampaignRuntimeContext 快照

目标：让 pipeline/agent 能读取 Campaign 运行态，但不依赖 Tauri store。

改动文件：

- `crates/domain/src/campaign_runtime.rs` 或 `crates/domain/src/campaign.rs`
- `crates/domain/src/lib.rs`
- `crates/app-pipeline/src/lib.rs`
- `crates/app-agent/src/tools.rs`
- 所有 `ToolContext { ... }` 构造点
- 所有 `WritingContext { ... }` 构造点

任务：

1. 新增纯 domain DTO：

```rust
pub struct CampaignRuntimeContext {
    pub campaign: Campaign,
    pub instances: Vec<CharacterInstance>,
    pub definitions_by_id: HashMap<Id, CharacterDefinition>,
    pub knowledge: Vec<CharacterKnowledgeEntry>,
    pub turn: u32,
}
```

2. 增加 helper：
   - `find_instance_by_id_or_name`
   - `definition_for_instance`
   - `display_name`
   - `knowledge_for_instance`
3. `WritingContext` 增加 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
4. `ToolContext` 增加同名字段。
5. legacy 构造和测试构造全部补 `None`。
6. 没有 active Campaign 时完全走旧路径。

验证：

```bash
cargo build --workspace
cargo test -p storyforge-domain -p storyforge-app-agent -p storyforge-app-pipeline
```

验收：

- 依赖方向仍是 `app-* -> domain`，没有 `app-agent -> tauri-app`。
- 所有旧测试构造点编译通过。

## 阶段 3：Tauri 装配 Campaign 快照

目标：让开 Campaign 后的 `WritingContext` 和 `ToolContext` 都拿到同一份只读快照。

改动文件：

- `crates/tauri-app/src/lib.rs`
- 如需测试辅助，`crates/tauri-app/src/campaign_store.rs`

任务：

1. 在 `fill_campaign_context` 中加载：
   - `campaign = store.get_campaign(active_id)`
   - `instances = store.list_instances(active_id)`
   - `card = store.get_card(campaign.card_id)`
   - `definitions_by_id`
   - `knowledge = store.list_knowledge(active_id)`
2. 计算 turn 继续使用 summaries 数量 + 1。
3. 组装 `Arc<CampaignRuntimeContext>`。
4. 写入：
   - `ctx.campaign_runtime = Some(runtime.clone())`
   - 当前写作使用的 tool snapshot 或 `state.tool_ctx` 快照字段。
5. 明确错误策略：
   - active campaign id 不存在：fallback 到旧路径并 warn。
   - campaign 有 0 个 instance：返回友好错误，不让 Director 瞎编。

验证：

```bash
cargo test -p storyforge
cargo test --workspace
```

验收：

- active Campaign 下 `ctx.campaign_runtime` 非空。
- 无 active Campaign 下旧路径不变。

## 阶段 4：Director 改用 CharacterInstance

目标：Director 看到的是 Campaign 实例、变量和任务，不是扁平角色卡名称。

改动文件：

- `crates/app-pipeline/src/lib.rs`
- `crates/app-agent/src/tools.rs`

任务：

1. `build_director_tail`：
   - 有 `campaign_runtime` 时，渲染 `instance_id + name + role_type + persona 摘要`。
   - 注入 campaign variables 和 instance variables。
   - 保留 pending tasks 注入。
   - 无 `campaign_runtime` 时保持旧角色列表。
2. `get_character` 工具：
   - 参数说明改为“角色名或 instance_id”。
   - 有 Campaign 快照时优先查 instance id，再查 name。
   - 返回 persona、behavior、variables、role_type、source definition id。
   - 查不到时 fallback 到扁平 `Character`。
3. Director prompt 明确要求 Campaign 路径中 `subagent_tasks[].character_id` 填 `instance_id`。

验证：

```bash
cargo test -p storyforge-app-agent
cargo test -p storyforge-app-pipeline
```

验收：

- Campaign 路径 plan 内部身份使用 instance id。
- 旧路径仍可用角色名。

## 阶段 5：Subagent 接收 Campaign 快照并做信息隔离

目标：子 Agent persona 来自实例化数据，只看到自己该看到的知识和变量。

改动文件：

- `crates/app-agent/src/runtime.rs`
- `crates/app-pipeline/src/lib.rs`
- `crates/domain/src/agent.rs` 如需扩展 provenance DTO

任务：

1. `spawn_subagents` 增加参数 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。
2. 所有调用点传 `ctx.campaign_runtime.clone()`。
3. 子 Agent 构造：
   - 有 runtime 且能匹配 instance：system 使用 resolved persona/behavior。
   - tail 注入 scene、该 instance 可见 knowledge、该 instance variables、任务 brief。
   - 查不到 instance：warn 并 fallback 到旧 `context_package`。
4. Provenance 记录：
   - subagent 使用的 `character_instance_id`
   - display name
   - fallback reason，如有。

验证：

```bash
cargo test -p storyforge-app-agent
cargo test -p storyforge-app-pipeline
cargo test --workspace
```

验收：

- A 角色私有知识不出现在 B 角色 subagent prompt。
- 同名角色通过 instance id 不混淆。
- reroll 仍能定位对应子 Agent。

## 阶段 6：Postprocess ID 归一和写回收尾

目标：写回 CampaignStore 前，把名字、旧 character id、instance id 统一成 `CharacterInstance.id`。

改动文件：

- `crates/tauri-app/src/lib.rs`
- `crates/domain/src/agent.rs` 如需细化 DTO
- `crates/app-agent/src/prompts/postprocess.rs` 如需调整输出约束

任务：

1. 提取 `resolve_postprocess_character_ref(campaign_runtime, input)`。
2. 变量更新：
   - instance 更新必须落到 instance id。
   - 不明确时记录 warn，不写错对象。
3. 知识更新：
   - 如果 knowledge 是角色可见，绑定 instance id。
   - 如果是全局知识，明确全局字段或保留 campaign 级条目。
4. 任务更新：
   - task status 更新必须校验 task 属于当前 campaign。
5. 真正使用 `present_chars`：
   - 至少用于 postprocess 提示和落盘校验。

验证：

```bash
cargo test -p storyforge
cargo test --workspace
```

手动验证：

1. 导入单角色 ST 卡，未开 Campaign 写作正常。
2. 识别多角色卡，创建 Campaign，写一轮。
3. 检查 variables、knowledge、summary、tasks 均落到正确 campaign/instance。
4. 同名角色不串变量。

验收：

- 写作输出能改变下一轮 Campaign 上下文。
- postprocess 不再靠裸名字盲写。

## 回滚策略

- 所有新字段必须是 `Option`，旧路径可直接 fallback。
- 每阶段一个 commit。
- 如果阶段 4/5 失败，可保留阶段 1/2 的 domain DTO，不影响旧写作。

## 禁止改动

- 禁止删除 `CharacterStore`。
- 禁止把 `CampaignStore` 移入 `domain`。
- 禁止让 `app-agent` 或 `app-pipeline` 依赖 `tauri-app`。
- 禁止一次性重写前端大布局。
- 禁止把 ST 卡导入格式改成 StoryForge 专有格式。

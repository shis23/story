# 计划：角色体系接通（CharacterStore / CampaignStore 双数据源统一 + 临场角色 + 信息隔离）

> **状态**：待执行（2026-06-16 起草）
> **作者**：基于代码实测（两个 Explore agent 交叉验证）
> **对应设计**：TECHNICAL_DESIGN.md §16-§23，INTENT.md D30-D48
> **前置**：§22 cache 友好消息布局已落地（commit `0256075`，MessageLayout 三段分离 + 子 Agent persona 进 system 稳定段已就绪）
>
> **本计划是 StoryForge 当前最大的架构债**。README / ARCHITECTURE / HANDOFF 三处文档均点名此问题。它阻塞的不只是「双数据源一致性」，还阻塞：变量注入 Agent（§23.5）、子 Agent 信息隔离（§16 character_knowledge 注入）、临场角色（§17.3 D34）、Campaign 角色实例化在写作时真正生效。

---

## 0. 一句话目标

让写作流水线在**开 Campaign 时使用 `CharacterInstance` + `CharacterDefinition`**（角色实例 + 卡级定义），而非当前的扁平 `Character`；未开 Campaign 时退回扁平 `Character`（向后兼容）。同时打通变量注入、信息隔离、临场角色三条依赖此链路的能力。

---

## 1. 现状（为什么是债）

### 1.1 双数据源

项目有两套角色数据，互不连通：

| 数据源 | 存储 | 类型 | 管什么 | 谁用 |
|--------|------|------|--------|------|
| **CharacterStore** | `data/characters.json` | 扁平 `Character`（单角色卡） | 导入的原始卡（name/description/personality/first_mes/world_info） | 写作流水线（tool_ctx.characters）、世界书 CRUD、前端角色列表 |
| **CampaignStore** | `data/cards.json` 等 7 文件 | `CharacterCard`（含 `CharacterDefinition[]`）+ `CharacterInstance` | 角色识别后的多角色树 + 开档实例（带 variables/persona_override） | Campaign 面板、后处理流水线（变量更新） |

**写作流水线只读扁平 Character，完全零引用 CharacterInstance/CharacterDefinition**。证据：`app-pipeline` crate 全文 grep `CharacterInstance|CharacterDefinition|CharacterCard` 零命中。

### 1.2 数据流向（当前）

```
导入角色卡 → CharacterStore（扁平 Character）+ tool_ctx.characters
                ↓（用户点「角色识别」）
           extract_characters 命令 → CampaignStore（CharacterCard + Definition）
                ↓（用户点「开档」）
           create_campaign → CampaignStore（CharacterInstance × N）
                ↓（用户点「写作」）
           start_writing → WritingContext.characters = tool_ctx.characters（扁平！）
                                   ↑
                       断点：开档产生的 Instance 完全没进写作上下文
```

### 1.3 关键断点清单（14 处，按数据流顺序）

| # | 位置 | 现状 | 断点描述 |
|---|------|------|---------|
| 1 | `domain/campaign.rs:127-144` `CharacterInstance::from_definition` | 只 copy id/name，persona/behavior 置 None | 实例不携带 definition 的 persona_prompt/behavior_rules |
| 2 | `domain/campaign.rs:183-189` `resolved_persona`/`resolved_behavior` | 只返回 override，无 definition 回退 | 即使想读 persona 也读不到完整设定 |
| 3 | `tauri-app/lib.rs:1275-1287` 构造 WritingContext | characters 来自 tool_snapshot（扁平） | Campaign 的 instances 没进 WritingContext |
| 4 | `tauri-app/lib.rs:1390-1412` `fill_campaign_context` | 只填 campaign_id/turn/tasks/story_clock | **不调 list_instances，不填角色实例/变量** |
| 5 | `app-pipeline/lib.rs:102` `WritingContext.characters` | `Vec<Arc<Character>>` 扁平 | 无 instance/definition 字段 |
| 6 | `app-pipeline/lib.rs:1146-1178` `build_director_tail` | 只把 name 拼成字符串 | 导演看不到 persona/variables/role_type |
| 7 | `app-agent/tools.rs:31-41` `ToolContext` | 只有 `characters: Vec<Arc<Character>>` | 无 instances/definitions 字段 |
| 8 | `app-agent/tools.rs:148-183` 导演 `get_character` | 读扁平 Character 字段 | 返回的不是 instance/definition 设定 |
| 9 | `tauri-app/lib.rs` 多处同步 tool_ctx（173/445/453/527/567/713） | 只同步扁平 Character | 开档/实例化时不同步 instances 进 tool_ctx |
| 10 | `app-agent/runtime.rs:473-480` `spawn_subagents` 签名 | 无 CampaignStore/instances 入参 | 无法按 character_id 查 instance |
| 11 | `app-agent/runtime.rs:491,497-507` 子 Agent 派发 | character_id 仅作字面量，persona 靠导演生成的 context_package | 不读 instance 的 persona/variables |
| 12 | `app-pipeline/lib.rs:1326-1337` `parse_context_package` | context_package 由导演 LLM 产出（缺省空） | 角色设定来源是 LLM 而非实例化数据 |
| 13 | `tauri-app/lib.rs:1423-1495` `persist_postprocess_outcome` | 只更新 instance.variables | persona/behavior 无写回路径 |
| 14 | `tauri-app/lib.rs:1482-1494` `present_chars` | `let _ = present_chars;` 丢弃 | 名义预留未用 |

**最关键的三个断点**（改这几处能打通主链路）：**#4**（fill_campaign_context 不填实例）、**#7+#9**（ToolContext 无 instances 且开档不同步）、**#10+#11**（spawn_subagents 拿不到实例、persona 靠 LLM 生成）。

### 1.4 附带 bug（改造时顺手修）

**delete_character 的 id 语义错配**（`tauri-app/lib.rs:536`）：把 `StoredCharacter.id`（UUID）直接当 `source_character_id` 去 `get_card_by_source`，但建卡时 `source_character_id = character.id`（领域 Id），两者不是同一个值 → 级联删 CampaignStore 卡可能匹配不上。

---

## 2. 目标架构

### 2.1 核心原则

1. **Campaign 优先，扁平兜底**：开 Campaign 时，写作流水线读 `CharacterInstance` + `CharacterDefinition`；未开时退回扁平 `Character`（M0 老用法不变）。
2. **不废弃 CharacterStore**（本阶段）：废弃是更大动作，留后续。本阶段让两套数据在「开档写作」这条路径上达成一致——CampaignStore 成为写作时的角色权威源，CharacterStore 退为「导入 + 未开档写作」的源。
3. **变量/知识/临场角色一并打通**：既然要动这条链路，把 §23.5（变量注入）、§16（子 Agent 信息隔离）、§17.3（临场角色）一起落地，避免反复改同一段代码。

### 2.2 目标数据流

```
开 Campaign 写作时：
  fill_campaign_context → 从 CampaignStore 加载：
    - character_instances: Vec<CharacterInstance>（本档所有实例）
    - character_definitions: HashMap<Id, CharacterDefinition>（卡级定义索引，按 definition_id 查）
    - active_variables（聚合全局 + 在场角色变量）
  ↓ 注入 WritingContext（新增字段）
  ↓
导演 tail：可用角色（含 role_type/persona 摘要）+ 当前变量状态（render_variables_for_injection）
导演 get_character 工具：返回 instance.resolved_persona() + definition 设定 + variables
  ↓
spawn_subagents：按 character_id 查 instance →
  system 段（稳定）：persona（resolved_persona = override 或 definition.persona_prompt）+ behavior + 常驻世界设定
  tail 段（易变）：场景 + 该角色可见知识（character_knowledge 过滤）+ 该角色变量 + 任务
  ↓
后处理：已有（变量更新走 find_instance_by_name_or_id，保留）

未开 Campaign 写作时（向后兼容）：
  WritingContext.character_instances = None
  所有路径退回扁平 Character（现状不变）
```

---

## 3. 分阶段方案

按依赖顺序分 5 个阶段，每阶段可独立编译 + 测试 + 提交。**建议严格按顺序做，不要跳阶段**——后阶段依赖前阶段的数据结构。

### 阶段 1：domain 层补全角色实例的「设定回退」（小）

**目标**：让 `CharacterInstance` 能提供完整的 persona/behavior（override 优先，回退到 definition），消除断点 #1、#2。

**改动**：

1. **`domain/campaign.rs`**：
   - `CharacterInstance::resolved_persona<'a>(&'a self, definition: Option<&'a CharacterDefinition>) -> Option<&str>`
     - 有 `persona_override` → 返回 override
     - 否则有 definition → 返回 `definition.persona_prompt`
     - 都没有 → None
   - 同理 `resolved_behavior`、`resolved_backstory`（backstory 来自 definition.base_backstory）
   - **不改 `from_definition`**（实例仍只存 override，保持轻量；回退在读取时做）
   - 加方法 `resolved_variable_schema<'a>(&'a self, definition: Option<&'a CharacterDefinition>) -> &[VariableField]`（schema 来自 definition）

2. **测试**（domain/campaign.rs）：
   - override 优先于 definition
   - 无 override 时回退 definition
   - 都无时返回 None

**验证**：`cargo test -p storyforge-domain` 全绿。

---

### 阶段 2：扩展 ToolContext + WritingContext 承载角色实例（中）

**目标**：让运行时上下文能携带 Campaign 角色实例 + 定义索引，消除断点 #5、#7。

**改动**：

1. **`app-agent/tools.rs`** `ToolContext` 加字段（向后兼容，`Default`/构造处补 `None`/空）：
   ```rust
   pub struct ToolContext {
       pub characters: Vec<Arc<Character>>,                    // 保留（未开档用）
       pub world_info: Option<Arc<WorldInfoBook>>,
       pub vector_store: Option<Arc<VectorStore>>,
       pub archived_summaries: Vec<ArchivedSummary>,
       // 新增（开档时填，未开档为 None/空）
       pub campaign_instances: Option<Vec<Arc<CharacterInstance>>>,
       pub definitions_index: Option<HashMap<Id, Arc<CharacterDefinition>>>,  // definition_id → definition
       pub active_campaign_id: Option<Id>,
   }
   ```
   - 所有构造 `ToolContext { ... }` 的地方补新字段（`runtime.rs` 测试 + `tauri-app/lib.rs` 的 `AppState::new` 等）。

2. **`app-pipeline/lib.rs`** `WritingContext` 加字段：
   ```rust
   pub struct WritingContext {
       pub characters: Vec<Arc<Character>>,                    // 保留
       // ...现有字段...
       // 新增（开档时填）
       pub campaign_instances: Option<Vec<Arc<CharacterInstance>>>,
       pub definitions_index: Option<HashMap<Id, Arc<CharacterDefinition>>>,
   }
   ```
   - `legacy()` 构造补 `None`。

3. **`tauri-app/lib.rs`** `fill_campaign_context`（断点 #4）扩充：开档时从 CampaignStore 加载：
   ```rust
   if let Some(active_id) = active_campaign_id {
       let instances = campaign_store.list_instances(&active_id);
       // 加载该卡的所有 definition 建索引
       let card = campaign_store.get_card_by_campaign(...);  // 或从 campaign.card_id 反查
       let definitions_index = card.character_definitions 建索引;
       ctx.campaign_instances = Some(instances);
       ctx.definitions_index = Some(definitions_index);
       // 同步进 tool_ctx（让导演工具也能用）
       tool_ctx.write().campaign_instances = ctx.campaign_instances.clone();
       tool_ctx.write().definitions_index = ctx.definitions_index.clone();
   }
   ```

4. **测试**：
   - ToolContext/WritingContext 新字段默认 None/空（向后兼容）
   - domain 层无新逻辑，测试主要在 tauri-app 集成层（mock campaign_store 填充）

**验证**：`cargo build --workspace` 通过（所有构造点补齐）；`cargo test --workspace` 全绿。

---

### 阶段 3：导演层接通角色实例（中）

**目标**：导演能拿到角色实例的 persona/variables/role_type，消除断点 #6、#8、#9。

**改动**：

1. **`app-pipeline/lib.rs`** `build_director_tail`（断点 #6）：
   - 开档时，「可用角色」列表改为从 `ctx.campaign_instances` 取，显示 `name + role_type + persona 摘要`（如「林医生（主角团，外科医生）」）
   - 未开档时退回扁平 Character name（现状）
   - 新增变量注入（§23.5）：调用 `render_variables_for_injection()` 渲染当前变量，压在 tail 末尾（在任务块之后）。变量来源：开档时从 instances + campaign.variables 聚合；未开档时跳过。

2. **`app-agent/tools.rs`** 导演 `get_character`（断点 #8）：
   - handler 改造：先看 `ctx.campaign_instances` 是否有该 name 的实例
     - 有 → 返回 `resolved_persona` + `resolved_behavior` + variables + role_type
     - 无 → 退回扁平 Character（现状）
   - 工具返回的 JSON schema 扩展（加 persona/behavior/variables/role_type 字段，前端/LLM 可用）
   - 子 Agent 版 `get_character`（`tools.rs:317-349`）同步改

3. **`tauri-app/lib.rs`**（断点 #9）：开档/实例化后同步 tool_ctx —— 已在阶段 2 的 `fill_campaign_context` 里做。

4. **测试**：
   - 开档时导演 tail 含 persona 摘要 + 变量状态
   - get_character 开档返回 instance 设定，未开档返回扁平
   - 变量注入在 tail 末尾（不在 system，§22 cache 友好）

**验证**：`cargo test -p storyforge-app-pipeline -p storyforge-app-agent` 全绿。

---

### 阶段 4：子 Agent 接通角色实例 + 信息隔离（中-大）

**目标**：子 Agent 的 persona 来自实例化数据（而非导演 LLM 瞎编），且只看自己该看的知识，消除断点 #10、#11、#12，落地 §16 信息隔离。

**改动**：

1. **`app-agent/runtime.rs`** `spawn_subagents`（断点 #10、#11）：
   - 签名加参数：`instances: Option<&[CharacterInstance]>` + `definitions: Option<&HashMap<Id, CharacterDefinition>>` + `knowledge: Option<&[CharacterKnowledgeEntry]>`（或封装成一个 `SubagentContext` 结构体避免参数爆炸）
   - 派发每个子 Agent 时，按 `task.character_id` 查 instance：
     - 找到 → 用 `resolved_persona`/`resolved_behavior` 作为 system 的 persona 段（替代当前导演生成的 `context_package.character_brief`）
     - 找不到（临场角色，见阶段 5）→ 退回 context_package（现状）
   - 子 Agent 的变量注入：从 instance.variables 取该角色的变量，压 tail

2. **信息隔离（§16，断点 #12 相关）**：
   - 子 Agent tail 的「最近对话」段，改为「该角色可见的知识」：从 `knowledge` 按 `character_id` 过滤（character_knowledge 的 `character_id` 字段），只注入该角色 witnessed/told_by_other/inferred/backstory 的条目
   - 未开档（无 knowledge）→ 退回 `context_package.recent_window`（现状）

3. **`app-pipeline/lib.rs`** 调用 `spawn_subagents` 处（行 311-319、715-723、816-925）：
   - 传入 `ctx.campaign_instances` / `ctx.definitions_index` / knowledge（从 CampaignStore 加载）
   - `build_provenance` 时记录每个子 Agent 用的是哪个 instance_id（便于重 roll）

4. **测试**：
   - 开档时子 Agent system 含实例 persona（不是空 context_package）
   - 子 Agent tail 只含自己的 character_knowledge（信息隔离）
   - 未开档退回 context_package（向后兼容）
   - prefix_fingerprint：同一角色跨场戏的 system 段（persona）稳定

**验证**：`cargo test --workspace` 全绿；重点跑 `test_full_pipeline_with_mock` 确认端到端不破。

---

### 阶段 5：临场角色（D34）+ 收尾（中）

**目标**：用户在游玩中提到新角色时，导演能自主创建临场角色实例，消除断点 #13、#14，落地 §17.3。

**改动**：

1. **导演工具新增 `create_ad_hoc_character`**（`app-agent/tools.rs`）：
   - 导演在 Plan 时若发现 character_id 不在现有角色列表 → 调此工具
   - 工具创建 `CharacterInstance::temporary(name, persona)`（domain 已有构造器 `campaign.rs:146-161`）
   - 写入 CampaignStore（需 tool_ctx 能访问 CampaignStore，或通过事件回写 tauri 层）
   - **设计决策点**：tool_ctx 目前不持有 CampaignStore（它在 tauri 层）。两种方案：
     - (a) 给 ToolContext 加 `campaign_store: Option<Arc<Mutex<CampaignStore>>>` 字段（简单但跨层）
     - (b) 工具返回「待创建角色描述」，由 tauri 层在 start_writing 结束后统一落盘（解耦但复杂）
   - **建议 (a)**，因为 CampaignStore 已有 Mutex 缓存，跨层持有安全。

2. **`persist_postprocess_outcome`**（断点 #13）：扩展支持 persona_override/behavior_override 写回（后处理 Agent 若调整了角色人设，能落盘）。

3. **`present_chars`**（断点 #14）：启用——传给后处理 Agent 作为「在场角色」约束（目前丢弃）。

4. **delete_character id 错配修复**（附带 bug，`lib.rs:536`）：用领域 `Character.id` 而非 `StoredCharacter.id` 做 `source_character_id` 匹配。

5. **测试**：
   - 临场角色创建 + 写入 CampaignStore + 下轮写作可见
   - promote_to_permanent 升级
   - delete_character 级联删 CampaignStore 正确匹配

**验证**：全 workspace 测试 + 手动验证（开档 → 写作 → 引入临场角色 → 下一轮可见）。

---

## 4. 风险与对策

| 风险 | 等级 | 对策 |
|------|------|------|
| **改动面大**（跨 domain/app-agent/app-pipeline/tauri-app 4 层） | 🔴 高 | 严格分 5 阶段，每阶段独立可编译可测试可提交；不跳阶段 |
| **向后兼容**（未开档的老用法不能破） | 🟡 中 | 所有新字段 `Option`/默认 None；每个消费点都写「开档走新路径，未开档退回扁平」分支 |
| **tool_ctx 跨层持有 CampaignStore**（阶段 5） | 🟡 中 | CampaignStore 已有 Mutex 缓存，线程安全；或用方案 (b) 事件回写解耦 |
| **子 Agent 信息隔离测试难**（要造 character_knowledge 数据） | 🟡 中 | 阶段 4 测试造完整的 mock CampaignStore + knowledge 条目 |
| **导演 LLM 行为变化**（get_character 返回内容变了，可能影响 Plan 质量） | 🟡 中 | 阶段 3 完成后用真实 LLM 跑一次开档写作，对比 Plan 质量 |
| **变量注入导致 tail 变长**（§22 cache） | 🟢 低 | 变量压 tail 末尾，只影响最后一段；监控 tail 长度 |

---

## 5. 验证策略

### 5.1 每阶段验证
- `cargo build --workspace` 通过
- `cargo test --workspace` 全绿（当前基线 250 测试）
- 该阶段新增测试覆盖核心断点

### 5.2 全部完成后的端到端验证
1. **未开档写作**（向后兼容）：导入卡 → 写作，行为与改造前完全一致
2. **开档写作**（新能力）：
   - 导入卡 → 角色识别 → 开档 → 写作
   - 导演 tail 含角色 persona 摘要 + 当前变量
   - 子 Agent system 含实例 persona（非空 context_package）
   - 子 Agent tail 只含自己的 character_knowledge
   - 后处理更新变量 → 下一轮写作 tail 反映新变量
3. **临场角色**：写作中提到新角色 → 下一轮该角色可见、有 persona
4. **删卡级联**：删除角色卡 → CampaignStore 的 card/campaign/instance 全部级联删（验证 id 匹配修复）

### 5.3 性能验证（可选）
- 对比改造前后的 LLM 调用 token 数（cache 命中率应不降反升，因为 persona 进了稳定 system 段）

---

## 6. 文件改动清单（按阶段）

| 阶段 | 文件 | 改动 |
|------|------|------|
| 1 | `domain/campaign.rs` | +resolved_persona/behavior/backstory/variable_schema 方法 + 测试 |
| 2 | `app-agent/tools.rs` | ToolContext +3 字段 |
| 2 | `app-pipeline/lib.rs` | WritingContext +2 字段，legacy() 补 None |
| 2 | `tauri-app/lib.rs` | fill_campaign_context 加载 instances/definitions，同步 tool_ctx；所有 ToolContext 构造点补字段 |
| 3 | `app-pipeline/lib.rs` | build_director_tail 开档走实例 + 变量注入 |
| 3 | `app-agent/tools.rs` | get_character（导演+子Agent）开档返回实例设定 |
| 4 | `app-agent/runtime.rs` | spawn_subagents 签名加 SubagentContext，persona 走实例，信息隔离 |
| 4 | `app-pipeline/lib.rs` | spawn_subagents 调用点传实例/knowledge；build_provenance 记 instance_id |
| 5 | `app-agent/tools.rs` | +create_ad_hoc_character 工具 |
| 5 | `tauri-app/lib.rs` | persist_postprocess_outcome 扩展 persona 写回；present_chars 启用；delete_character id 修复 |
| 5 | `domain/campaign.rs` | （可能）CharacterInstance 加临场角色辅助方法 |

**不改**：前端（纯后端改造，命令签名兼容）、domain 的 CharacterDefinition/CharacterCard 结构（已就绪）、MessageLayout（阶段 0 已完成）。

---

## 7. 工作量评估

| 阶段 | 工作量 | 风险 |
|------|--------|------|
| 1 domain 补全 | 小（1-2h） | 低 |
| 2 上下文扩展 | 中（3-4h） | 低（机械加字段） |
| 3 导演接通 | 中（3-4h） | 中（get_character 返回变了） |
| 4 子 Agent + 信息隔离 | 中-大（5-6h） | 中高（spawn_subagents 重构） |
| 5 临场角色 + 收尾 | 中（3-4h） | 中（跨层 CampaignStore） |
| **合计** | **15-20h（2-3 个工作日）** | |

建议每个阶段单独 commit，便于回滚和 review。

---

## 8. 给执行 agent 的提示

1. **先读 §1.3 的断点清单**，对照行号确认现状（代码可能已演进，行号会漂移，以符号名为准）。
2. **每阶段做完先跑 `cargo test --workspace`** 再进下一阶段，不要攒着一起测。
3. **向后兼容是硬约束**：所有新字段必须 Option/默认空，所有消费点必须写「开档/未开档」分支。未开档的老用法（`WritingContext::legacy()`）行为必须不变。
4. **阶段 4 的信息隔离**是本计划的核心价值点（§16 D30-D31），不要偷懒跳过——子 Agent 只看自己的 character_knowledge 是「赛博跑团卡」区别于普通写作的关键。
5. **变量注入（阶段 3）**用现成的 `render_variables_for_injection()`（`domain/variables.rs`），压在 tail 末尾（任务块之后），不要进 system。
6. **临场角色（阶段 5）**的 CampaignStore 跨层持有方案选 (a)（ToolContext 直接持有 `Option<Arc<Mutex<CampaignStore>>>`），简单且安全。
7. 改完同步更新文档：AGENT_INTERFACES.md（get_character 变了）、ARCHITECTURE.md（双数据源债消除）、HANDOFF.md（进度）。

---

*起草：2026-06-16。基于代码实测（两个 Explore agent 交叉验证 14 处断点）。代码演进后请重新核对行号。*

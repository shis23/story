# StoryForge 架构、提示词与缓存优化评估

> 日期：2026-07-11  
> 范围：项目现状、架构风险、DeepSeek V4 角色扮演反馈、梁元·月食预设、提示词优化边界、KV/Prompt Cache 命中策略。  
> 性质：评估与实施建议，不表示文中方案已经实现。  
> 外部材料：[DeepSeek V4 用户反馈意见汇总报告（2026-05-20）](https://github.com/victorchen96/deepseek_v4_rolepaly_instruct/blob/main/deepseek_v4_feedback_report_20260520.md)。

## 1. 结论先行

StoryForge 已经不是“角色卡 + 聊天框”的套壳。Campaign、CharacterDefinition/CharacterInstance、Director/Subagent/Editor/Postprocess、Meta Agent、知识隔离和结构化写回，组成了一条方向正确的多 Agent 写作主线。

当前最需要补的不是更多功能，而是四个控制层：

1. **Turn 提交边界**：正文、摘要、知识、变量和任务必须形成可恢复的一轮提交。
2. **ContextCompiler**：按身份、近期事实、结构化状态、摘要、远记忆和世界书分层编译上下文。
3. **DraftQualityGate**：正文提交前检查重复、视角、格式、知识越权和连续性，只重写失败部分。
4. **Reasoning/Request Policy**：统一管理原生 reasoning、提示式 CoT、采样参数、重试、超时、预算和缓存能力。

优秀提示词可以显著改善语气、角色主动性、场景规划、初始格式遵循和文风；它不能可靠保证长期记忆、知识权限、逻辑一致性、状态更新、CoT 隔离、缓存命中和崩溃恢复。需要保证的约束必须下沉到结构化数据、验证器、渲染器或事务层。

本评估给出的明确决策是：继续采用 Campaign-first 的模块化单体；不迁移为微服务；不整份移植梁元预设；不在缺少 UnitOfWork/journal 时直接重写 SQLite 存储；不把提示词优化当作硬约束和缓存命中的替代品。

## 2. 项目现状快照

### 2.1 已形成的架构能力

- Campaign 是写作运行时真相源；ST 卡是导入、兼容和素材来源。
- Director 规划场景，Subagent 按角色并行表演，Editor 合稿，Postprocess 写回摘要、知识、变量和任务。
- CharacterInstance 使用稳定 ID；子 Agent 只读取自身实例允许看到的知识和变量。
- Meta Agent 采用 propose → preview → accept/dismiss，而不是直接任意改数据。
- MessageLayout 已按 `stable system → stable history → volatile tail` 分层，方向上适合前缀缓存。
- Prompt Profile、Prompt Module、Agent Profile、Regex、ST 宏和真实 LLM harness 已具备扩展基础。

### 2.2 截至本评估的工程状态

- Rust workspace 实际为 15 个 crate；Tauri command 标注约 122 个。
- `main` 相对本地跟踪的 `origin/main` 领先 21 个提交。
- 原有未提交改动为 `frontend/src/components-v2/ui/Tabs.vue` 与 `frontend/tests/stores/plugin.test.mjs`。
- Secret scan、Clippy、Rust workspace 750 项、前端 Node 212 项、Vitest 24 项和前端构建通过。
- 完整 release gate 仍因 5 个 Rust 文件未通过 `cargo fmt --check` 而失败。
- Android 仅形成 arm64 构建基线，真机导入、分享、keyring、生命周期和长文本仍未完成发布验收。

以上是带日期的快照，后续应由 `RELEASE-CHECKLIST.md` 和实际命令结果更新，不应长期复制成静态“已完成”声明。

## 3. 前几轮审查发现的主要问题

### P0：一轮故事缺少明确的提交事务

当前成文可以先返回，Postprocess 在后台继续生成并写入摘要、知识、变量和任务。这样会出现两个窗口：

- 用户立即开始下一轮时，可能读取到上一轮尚未完成写回的 Campaign 快照。
- 应用退出、任务取消或写盘失败时，正文存在而状态写回缺失。

建议引入：

```text
TurnState = Pending → Generating → DraftReady → Validating → DerivingState → Committing → Committed
                                  ↘ Failed             ↘ Degraded / Failed
```

`DraftReady` 只表示正文可以提前展示，不表示它已经成为 Campaign 的规范事实。每轮至少持有 `turn_id`、`base_campaign_revision`、正文、provenance、结构化状态变更和最终 `TurnCommit`。下一轮默认只从已提交 revision 开始；UI 可以流式展示草稿，但在进入 `Committed` 或显式 `Degraded` 前不得启动依赖新状态的下一轮。

短期可以在 JSON 存储前增加 append-only turn journal；中期通过 Repository/UnitOfWork 接口迁移到 SQLite/WAL。JSON 继续作为导入、导出、备份和排障格式。

### P0：插件运行时接线存在缺口

AppV2 的 `loadSidebarPlugins()` 当前只调用 `listPlugins()`，没有把结果写入 `plugin.hookPlugins` 或 `plugin.sidebarPlugins`；隐藏 PluginHost 又依赖这些数组挂载。安装界面存在不等于 prompt hook、slot 和侧栏插件已经在主路径工作。

应先修复宿主装配并建立一个真实插件端到端回归，再扩展 ST 事件兼容范围。插件契约建议分级：

- L0：导入保真。
- L1：只读事件和查询。
- L2：Prompt hook 与受控变量写入。
- L3：沙箱 UI/MVU Runtime。
- L4：明确列出的高级 ST 兼容 API。

不建议把“完整复刻 ST 99 事件和 TavernHelper 全部语义”作为主架构目标。

### P1：LLM Request Policy 没有统一落到主路径

连接层能保存 temperature、top_p、max_tokens 和厂商扩展参数，但 AgentRuntime 会重新构造 SamplingParams；重试器也已实现但没有统一包裹生产客户端。结果是“UI 有设置”与“所有 Agent 实际使用设置”之间缺少单一事实来源。

建议新增统一 `RequestPolicy`：

```text
RequestPolicy
- provider capabilities
- model
- sampling
- reasoning policy
- retry/backoff
- timeout/cancellation
- token/cost budget
- prompt hook
- cache policy
```

Director、Editor、Subagent、Summarizer、Postprocessor 和 Meta 都从同一入口生成请求。

### P1：Campaign 的一致性边界大于当前 JSON 文件边界

Campaign 被拆分在 cards、campaigns、instances、knowledge、tasks、summaries、conversations 等多个文件中。单文件 `.tmp → rename` 只能保证单文件原子性，无法保证跨文件级联和一轮写回事务。

建议顺序：先定义 Repository/UnitOfWork；再加 journal 和恢复；最后引入 SQLite adapter。不要直接重写整个存储层，也不需要拆微服务。

### P1：长期记忆组件存在，但上下文闭环不完整

- Director/Editor 当前主要读取最近 20 条活跃消息。
- RoundSummary 已写盘，但未进入 CampaignRuntimeContext 的主写作快照。
- MemoryArchiver 会把摘要写入向量库，但 `ToolContext.archived_summaries` 生产初始化为空。
- 远记忆主要依赖 Director 主动调用搜索工具，而不是 ContextCompiler 确定性编排。
- 滑动窗口达到 20 条后每轮丢掉最旧消息，会改变 history 前缀并削弱缓存命中。

应建立分层、带预算的 ContextCompiler，见第 8 节。

### P1：未经校验的正文会反向污染事实状态

Postprocess 会从 Editor 正文中提取知识、时间、位置、变量和任务。如果正文先产生幻觉，再由 Postprocess 把幻觉写入 Campaign，错误会从“本轮文案问题”升级成“长期事实”。

因此 DraftQualityGate 必须位于状态推导和 TurnCommit 之前；Postprocess 先从通过校验的正文推导候选状态变更，候选变更再次校验后，正文与状态才能一并提交。

### P2：隐私、文档和发布治理

- 原始本地 JSONL 日志会记录 LLM prompt/response；导出脱敏不等于原始落盘已脱敏。
- README、HANDOFF、ARCHITECTURE 与最新代码存在命令数、入口和状态漂移。
- Bronze/Silver、真实插件、复杂 ST/MVU、长会话与 Android 真机证据仍不足。

建议通过 ADR 记录关键架构决策，通过 release checklist 记录证据，不再在多个说明文档复制同一状态。

## 4. DeepSeek V4 反馈与 StoryForge 的对应关系

| 反馈问题 | 现有基础 | 系统侧可改善程度 | 推荐手段 |
|---|---|---:|---|
| 八股句式、固定动作 | Editor、文风模块、Regex | 中 | 生成后重复检测、正面范例、局部重写 |
| 人称/视角混乱 | Instance ID、角色 Subagent、知识隔离 | 高 | NarrativeContract、focalizer、身份校验 |
| 指令遵循衰减 | 稳定 system、结构化工具、MVU | 高 | 输出契约验证、原生渲染、选择性重试 |
| 情感平淡、角色同质 | Persona/Behavior、独立 Subagent | 中 | CharacterAgency、正面台词范例、声音差异评测 |
| 双 CoT、英文 CoT、夺舍 | CoT 模块、Reasoning Regex | 有限 | 原生 reasoning 与提示式 CoT 互斥，reasoning 不入正文 |
| 长上下文退化 | 最近窗口、摘要、向量接口 | 高 | ContextCompiler、摘要水位、自动混合召回 |
| 剧情被动、急于收尾 | Director、任务、故事时钟 | 高 | 扩展 ScenePlan，区分剧情线与可完成任务 |
| 文笔退步 | Editor、文风模块、按 Agent 选模型 | 中 | 优秀正面范例、质量门禁、Editor 模型路由 |
| 幻觉、时间线和物理错误 | Campaign 状态、Postprocess | 高 | ContinuityValidator，校验后再写回 |
| 谄媚、纯爱化、冲突回避 | Behavior Rules | 中 | 目标、边界、拒绝条件、关系立场和对立目标 |
| 慢、过度思考、输出失控 | 并行流水线、Agent Profile | 中 | Fast/Standard/Quality 路由和预算 |

不能依赖系统彻底修复的部分包括模型原生文学能力、内部 reasoning 的语言和正确性、供应商高峰期空回及服务端速度。系统可以隔离、检测、重试或换模型，但不能把模型没有的能力凭空补出来。

## 5. 梁元·月食 2.0 预设审查

### 5.1 样本与结构

- 文件：`【梁元】lunareclipse 2.0.json`
- 大小：182,585 字节。
- SHA-256：`08C63AFF94C3FE22FC3D42888686B99F5FAE894524A74FAEA71489BD4655DB83`
- Prompt 定义：88 个。
- `prompt_order`：2 套。
- 主自定义顺序（character_id 100001）：启用 32 项。
- 启用项中有内容的静态/自定义提示词合计约 5,388 字符；该数字不包含运行时角色卡、世界书、聊天历史，也不把禁用的 UI/Regex 扩展正文计作实际 prompt。具体 token 数应以最终 messages 和目标模型 tokenizer 为准。
- 关键采样/运行设置：temperature=1、top_p=1、reasoning_effort=max、show_thoughts=true、squash_system_messages=true、function_calling=true。
- 文件声明的 2,000,000 context 和 65,536 output 是请求配置，不代表所有 endpoint 实际支持。

这不是单一提示词，而是一套宏驱动的提示词编译流程：

```text
变量重置
  → 文风/自查/格式模块写入变量
  → 世界信息与角色卡
  → 聊天记录
  → 固定 CoT 与输入后缀
  → 汇总输出模板
  → 再次清空变量
```

它最有价值的地方不是某一句指令，而是模块化、先组装后定位、显式清理变量和把输出格式当作契约的思路。

### 5.2 当前启用部分的优点

1. **变量生命周期明确**：开头重置、模块追加、模板读取、尾部清空，降低跨轮宏污染。
2. **正面写作标准较具体**：对中文句式、对话目的、情绪阶段和角色差异给出了可操作描述。
3. **自查清单覆盖关键问题**：视角越权、逻辑矛盾、重复表达和格式组件都被列入检查。
4. **格式契约清楚**：正文被 `<content>` 等结构包裹，便于渲染或后处理。
5. **把角色当作有欲望和日程的人**：这比“保持人设”四个字更能缓解角色平淡和被动。

### 5.3 当前启用部分的风险

1. **Reasoning 叠加过度**：同时启用 `reasoning_effort=max`、show_thoughts、Max 思考强度和固定中文五步 CoT。对原生 reasoning 模型容易产生双 CoT、慢响应、思考污染正文和角色夺舍。
2. **负面规则密度过高**：基础文风大量使用“禁止”，例如禁止常见否定词、声音/语气描写和部分身体描写。它可以短期压制模板，也可能让模型语言僵硬、绕句或把禁用模式当作注意力中心。
3. **规则存在潜在冲突**：要求强情绪、鲜活对话，同时禁止若干常用情绪表现通道；要求故事有活力，同时要求尾部避免伏笔、悬念、突发和收尾，可能形成“永远停在互动中段”的固定模板。
4. **自查仍由同一模型自证**：模型刚违反规则时，让它在同一次生成中“自查并纠正”不等同于可靠验证。
5. **大部分高级模块当前未启用**：叙事永动机、反全知、活人化、随机事件、显式禁词表等虽然存在于文件中，但不在当前启用顺序里，不能把它们视为当前实际效果。
6. **消息角色可能被压平**：`squash_system_messages=true` 适合特定兼容场景，但 StoryForge 不应为了前缀缓存照搬。system/user/assistant 的边界本身承担权限和来源语义，压平后会增加身份混淆、指令覆盖和审计困难。
7. **思维内容存在隐私与日志风险**：`show_thoughts=true` 与要求完整推演的提示组合，不只增加延迟，也可能让中间推理进入日志、历史或导出数据。StoryForge 应默认只保留可审计的决策摘要，不持久化自由形态 CoT。

### 5.4 值得吸收到 StoryForge 的部分

不建议把整份预设原样塞给所有 Agent。应按职责拆分：

| 梁元思想 | StoryForge 归属 |
|---|---|
| 世界资料解析、动机、冲突、随机事件 | Director / ScenePlan |
| 角色欲望、日程、手头动作、边界 | CharacterAgency / Subagent |
| 反全知、每人知识与误解 | NarrativeContract + knowledge gate |
| 中文文风、对话纹理、输出结构 | Editor Prompt Profile |
| 禁词、重复动作、否后肯、破折号密度 | DraftQualityGate 的确定性检查 |
| 自查流程 | QualityReport，不由同一生成调用自证 |
| 实时总结 | ContextCompiler / RoundSummary |
| 思维链 | ReasoningPolicy，默认不进入正文和历史 |

尤其值得吸收的是“角色必须有与用户输入无关的当前欲望和正在进行的事情”。但它应成为 CharacterAgency 的结构化字段和 ScenePlan 的输入，而不是每轮塞入数千字随机候选列表。

### 5.5 StoryForge 对该预设的兼容判断

StoryForge 已支持 setvar/addvar/getvar、trim、常见动态宏、Prompt Module、Regex 和 ST 预设导入基础能力；但梁元预设依赖精确 `prompt_order`、injection depth/position、变量拼装、role、squash、正则扩展与思维链展示。导入“字段不丢”不等于运行语义完全一致。

需要建立专项 fixture：导入后输出每个 Agent 的最终 messages，和 SillyTavern 实际组装结果做结构 diff。未完成这个对照前，只能声称保真导入和部分宏兼容，不能声称梁元预设等价运行。

## 6. 优秀提示词能解决到什么程度

### 6.1 适合交给提示词的软目标

- 文风、语气、节奏和对白密度。
- 场景目标、冲突倾向和主动推进要求。
- 角色说话纹理、当前欲望和行为偏好。
- 输出长度的目标区间。
- 正面范例和少量反例。
- Agent 分工和工具使用说明。

### 6.2 不应只交给提示词的硬约束

- 角色能否读取某条秘密。
- 用户/角色/旁白身份和稳定 ID。
- 时间、位置、物品、数值和任务状态。
- 状态栏、JSON、MVU 等必须存在的结构。
- 禁词和重复句式是否出现。
- reasoning 是否写入正文或历史。
- 长期记忆是否召回、是否重复归档。
- 崩溃后数据是否完整恢复。
- 缓存是否命中及实际节省多少 token。

原则：**Prompt 负责提出期望，结构化状态负责事实，Validator 负责硬约束，Renderer 负责固定格式，Repository/UnitOfWork 负责一致性。**

### 6.3 提示词优化策略

1. 用正面短例替代长禁词表；禁词表移到输出检测。
2. 每个 Agent 只接收与职责有关的规则，减少互相冲突。
3. 原生 reasoning 模型不再叠加提示式 CoT。
4. 用结构化 ScenePlan 代替“请主动推进剧情”的泛化指令。
5. 把角色声音做成 3～5 条短对白/行为范例，而不是抽象形容词堆叠。
6. 失败时只重写违规段落，不整轮再生成。
7. 任何优化必须进入真实 LLM A/B 评测，不能靠主观单次样本定案。

## 7. 缓存命中评估

本文所说的“缓存”主要是外部模型供应商提供的 prompt/prefix cache，而不是 StoryForge 自己保存整段模型 KV。使用托管 API 时，应用通常只能控制请求前缀、cache hint 和观测字段，不能假设可以导出或跨模型复用服务端 KV；因此优化目标是让相同 token 前缀稳定重现，并用供应商 usage 验证收益。

### 7.1 当前设计的优点

`MessageLayout` 已经明确分为：

```text
[1] stable system
[2] stable history
[3] volatile tail
```

用户意图、任务、故事时钟、变量和选择性世界书主要压在 tail，角色规则和常驻世界信息进入 system；`prefix_fingerprint` 测试也在检查 system/history 稳定性。这是正确方向，但当前指纹只能作为调试线索，不能证明供应商真的命中缓存。

对自动前缀缓存而言，下一轮请求如果是：

```text
system + 历史旧消息 + 上轮用户消息 + 上轮正文 + 新 tail
```

就可以复用上一轮的大部分前缀。

### 7.2 当前会破坏命中的因素

1. **20 条滑动窗口**：达到上限后，每轮删除最旧消息，history 从第一个 token 起发生变化，缓存命中会骤降。
2. **动态宏在 system 中展开**：`{{date}}`、`{{time}}`、`{{random}}`、`{{roll}}` 和读取 Campaign 变量的 `getvar` 都可能让 system 每轮变化。
3. **随机种子未固定**：TemplateVarContext 默认 `random_seed=None`，会按系统时间派生；只要动态宏进入 system，system hash 就会变化。
4. **Prompt Profile/模块顺序变化**：启停模块、非规范化排序或文本微小变动都会改变前缀。
5. **Subagent 在第一个角色 token 就分叉**：当前角色名较早进入 system，不同 Subagent 难以共享公共前缀。
6. **当前指纹过弱**：history 只记录 `role + content.len()`，不同内容只要角色和长度相同就可能得到相同签名；system 的简单 hash 也没有覆盖工具、模型、Provider 和 hook 后最终 prompt。
7. **工具顺序不稳定**：ToolRegistry 使用 HashMap，若 `tool_specs()` 未按函数名排序，同一组工具可能生成不同 JSON 顺序，导致请求字节和缓存键变化。
8. **Prompt hook 位于编译结果之后**：hook 可以修改最终 messages。若只记录 hook 前指纹，会把真实前缀变化误判为稳定；hook 本身也需要 Static/Session/Turn/Request 波动性声明。
9. **只有理论指纹，没有真实指标**：Usage 只记录 prompt/completion/total，没有统一记录 cached prompt tokens、cache read/create tokens 和每 Agent 命中率。
10. **连接扩展参数未统一传入 AgentRuntime**：即使供应商支持显式 cache control，当前也缺少可靠的 RequestPolicy 通路。
11. **大预设的动态部分位置不稳定**：梁元预设启用的静态自定义文本约 5.4k 字符，本身适合缓存；但如果开启含大量 `random` 的叙事永动机并在稳定前缀展开，会让这部分缓存失效。

### 7.3 推荐的缓存架构

引入显式 Prompt Segment：

```text
PromptSegment
- key
- content
- retention_priority: P0..P6
- volatility: Static | Session | Epoch | Turn | Request
- visibility
- provenance
- content_hash
- version_hash
- provider_cache_hint
```

编译规则：

```text
Static：产品规则、Agent 职责、Profile、常驻世界设定
Session：Campaign 叙事契约、角色定义、稳定 persona
HistoryEpoch：只追加，不在 epoch 内滑动删除
Turn：近期状态、摘要、任务、变量、选择性世界书
Request：用户意图、重 roll hint、随机种子
```

最终顺序必须从最稳定到最易变。

请求指纹应在所有 Prompt hook 执行后计算，并覆盖：Provider、model、RequestPolicy/Profile 版本、每条 message 的 role 与完整内容 hash、规范化后的工具 JSON、History Epoch ID 和 segment 版本。工具按稳定键排序，JSON 使用规范序列化；跨进程或跨版本比较应采用 SHA-256/BLAKE3 一类稳定内容 hash，不能把长度或进程相关哈希当作缓存身份。

### 7.4 用 History Epoch 替代逐轮滑动窗口

不要在第 21 条消息时简单删除第 1 条。建议：

1. 一个 epoch 内 history 只追加，保持前缀稳定。
2. 达到 token 阈值后生成一次确定性 checkpoint summary。
3. 关闭旧 epoch，创建新 epoch：`stable summary + 新近消息`。
4. 新 epoch 第一次请求缓存 miss，后续继续高命中。
5. 用 archived watermark 保证同一历史区间只归档一次。

这同时改善长上下文质量、归档幂等和缓存命中。

### 7.5 多 Agent 的缓存布局

- Director、Editor、Postprocessor 分别维护自己的稳定前缀，不强求跨角色共享。
- Subagent 应把所有角色共用的执行规则和常驻世界规则放在最前，角色 persona 放在后面，场景任务放 tail。这样并行 Subagent 至少能共享公共规则前缀。
- 不要把所有世界书复制给每个 Subagent；只给常驻小集合和该角色命中的相关条目。
- Summarizer/Postprocessor 的稳定 system 很适合长期缓存，用户消息只放本轮正文和状态摘要。

### 7.6 缓存可观测性

没有指标就不能声称“缓存优化有效”。建议扩展 Usage：

```text
Usage
- prompt_tokens
- cached_prompt_tokens
- cache_read_tokens
- cache_creation_tokens
- completion_tokens
- provider
- model
- agent_role
- prompt_version_hash
- final_request_fingerprint
- history_epoch_id
```

调试面板按 Agent 展示：

- prefix hash 是否变化。
- 变化发生在哪个 segment。
- 本轮命中率和节省 token。
- 首 token 延迟、总耗时和费用。

供应商只支持自动前缀缓存时记录其返回的 cached token 字段；支持显式 cache control 时，由 ProviderCapabilities 决定是否添加 cache hint。不能把某家 API 的字段直接写进领域层，也不能用本地 hash 相同推断远端一定命中。真实结论必须来自供应商 usage、冷/热请求对照和 epoch 边界测试。

## 8. ContextCompiler 建议

> **落地规格（2026-07-11 拍板）**：远楼概览 cap≈200、正文/纪要不重叠窗、三窗 epoch 同步滑动、A/B/C 攒 200 再压、Director 点名 tool 等，以专用文档为准，避免与本节早期评估表述漂移：  
> [`docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`](./MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md)

下面的 P0～P6 是**内容保留优先级**：当 token budget 不足时，越靠前的内容越不能被裁掉。它不是 SillyTavern 的 injection depth，也不是最终 messages 的物理排列顺序。

内容保留优先级：

```text
P0  身份、NarrativeContract、角色权限、不可违反的世界规则/Canon
P1  当前用户意图与 ScenePlan
P2  最近 6～10 条原始消息（绝不被远记忆挤掉）
P3  Campaign 当前变量、位置、关系、任务、故事时钟
P4  最近若干轮 RoundSummary / checkpoint summary
P5  当前场景成立所需的检索证据（远记忆或选择性世界书）
P6  可裁剪的氛围素材、远背景和低相关补充
```

缓存序列化顺序是另一条轴，必须按“最稳定 → 最易变”排列：

```text
[Static]
产品规则、Agent 职责、NarrativeContract 的稳定部分

[Session]
Campaign 身份、稳定角色定义、常驻世界书

[History Epoch]
checkpoint summary + epoch 内只追加的原始消息

[Turn]
Campaign 当前变量/位置/关系/任务、RoundSummary、命中的世界书和远记忆

[Request]
当前用户意图、ScenePlan、重 roll hint、随机事件种子
```

因此，P1 的“当前用户意图”虽然在裁剪时优先级极高，但发送给模型时应放在 request tail，而不是稳定前缀前面。ContextCompiler 可以先在本地使用意图完成远记忆和世界书检索，再把检索结果与当前意图一起放入易变 tail；检索过程的输入顺序不等于最终 prompt 的序列化顺序。

这也意味着“P1 很易变”不会导致 P1 之后的内容天然无法缓存，因为 P0～P6 根本不决定物理顺序。实际请求先序列化 Static/Session/History Epoch，供应商仍可命中这段共同前缀；Turn/Request 变化只会让变化点之后的后缀失配。代价是：若命中的远记忆和世界书每轮变化，它们自身及其后的部分通常不能复用，所以应放在稳定 history 之后。

优先级不能按“来源类型”一刀切：世界书里的强制 Canon 可以是 P0，当前场景必须引用的命中条目可以是 P5，纯氛围条目才是 P6。与此同时，常驻世界书属于 Session 稳定前缀，按当前意图命中的选择性世界书属于 Turn/Request tail；内容重要性与缓存波动性是两条独立轴。

需要同时维护三种元数据：

```text
retention_priority：token 不足时的保留顺序
volatility：Static / Session / Epoch / Turn / Request
visibility：Director / 某个 Subagent / Editor / Postprocess 是否可见
```

每段内容有独立 token budget、来源标记和裁剪策略。visibility 还应区分 `CharacterKnowledge`（角色知道什么）与 `NarrativeVisibility`（叙述者此刻允许揭示什么），否则“Editor 知道秘密”容易被误写成“角色知道秘密”。Director 获得剧情与世界层；Subagent 获得自身知识、欲望和局部场景；Editor 获得 ScenePlan、允许展示的各角色表演与叙事契约；Postprocess 只获得已通过质量门禁的正文和可写字段。

## 9. DraftQualityGate 建议

插入点：

```text
Director → Subagents → Editor → DraftQualityGate → DeriveState/Postprocess → TurnCommit
```

Editor 输出的是可展示但非规范的 draft。DraftQualityGate 通过后，Postprocess 只能生成候选状态差异；候选差异通过字段权限、revision 和连续性校验后，才与正文原子提交。这样“提前看到正文”和“系统已经接受事实”不会混为一谈。

确定性检查优先：

- 高频八股句式、n-gram 和动作重复。
- 破折号、否后肯、特定禁词密度。
- 人称、角色名、user/assistant 身份。
- 必须格式、状态栏、JSON/MVU 结构。
- 数字、时间格式和字数范围。

结构化连续性检查：

- 角色是否在场、所在位置和携带物。
- 知识是否越权。
- 时间线、任务和变量是否与 CampaignSnapshot 冲突。

连续性校验必须允许 ScenePlan 授权的状态变化，例如角色移动、物品转移和关系变化；Validator 检查的是“变化是否有依据且可提交”，不是要求正文永远等于旧快照。

语义 Judge 只检查剩余问题：角色声音同质化、无理由和解、快速恋爱、冲突和 ScenePlan 落地程度。失败时给 Editor 违规段落、错误码和修复约束，不重写无问题部分。修复次数必须有上限，建议默认一次定向修复；再次失败则进入可解释的 Degraded/人工接受路径，禁止 Critic–Editor 无限循环。LLM Judge 只提供辅助信号，不作为唯一提交依据。

## 10. 推荐实施顺序

> **进度快照（2026-07-11，`878125f`）**——本节原为评估建议；下列标注反映当前代码主线，不等于阶段完全关闭。
>
> | 阶段 | 状态 | 已落地要点 | 仍显式延后 |
> | --- | --- | --- | --- |
> | A | **主线可过** | TurnRecord/Attempt、AwaitingAcceptance-only accept、draft_hash SHA-256、write-ahead batch、`mutate_if`、启动 recovery（Finalize 失败保持 Committing）、活动 Turn 屏障、ReasoningMode 三选一、请求指纹+`cached_tokens` 日志、临时角色 accept 时 UpsertInstance、契约测试 | 预分配 Attempt 身份（强于 fail-and-compensate）、完整真实 LLM 回归集矩阵 |
> | B | 部分 | QualityGate **warn-only**（Accept 旁警告 + error_count）；非 hard-block | NarrativeContract、扩展 ScenePlan、Quality hard-block 产品决策 |
> | C | **轻量落地** | history-epoch 窗口 + 确定性 checkpoint summary + `epoch_id` 可观测；`recent_summaries` load last-12 / inject last-5；FarMemoryHit 溯源；named inject budgets；原始 history 不被远记忆挤占契约 | 完整 token budget / segment volatility 全量、真实供应商 epoch 冷热对照 |
> | D | 未开 | — | UnitOfWork / SQLite、完整 TurnState 事务升级、Android 真机矩阵 |

### 阶段 A：建立测量和安全边界

1. 修复当前 release gate 和插件宿主装配。
2. 增加 ReasoningPolicy：Disabled / Native / Prompted 三选一。 **（已落地：`ReasoningMode`）**
3. 规范化工具顺序和最终请求指纹，增加真实 cache usage、prompt version、hook 前后 hash 和 segment diff 记录。 **（指纹 SHA-256 + usage 日志已落地；prompt version/segment diff 可继续补）**
4. 增加最小 Turn 屏障：Postprocess/状态推导结束前禁止下一轮读取未提交 revision；暂不要求立刻迁移存储。 **（已落地并有契约测试）**
5. 将 DeepSeek V4 报告转成固定真实 LLM 回归集。 **（部分 harness 存在；完整矩阵延后）**

阶段 A 验收：同一输入的最终请求指纹可重复；工具顺序稳定；能记录供应商真实 cached/read/create token；原生 reasoning 与提示式 CoT 不会同时启用；并发触发下一轮时不会读到半提交状态。

### 阶段 B：低风险高收益

1. 增加 NarrativeContract。
2. 扩展 ScenePlan：冲突、对立目标、stakes、beats、complication、must_not_resolve、exit_hook。
3. 增加第一版 DraftQualityGate：重复、视角、格式、连续性。 **（warn-only 已接 Accept UX；非 hard-block）**
4. 把梁元的角色欲望、情绪阶段和反全知思想拆入对应结构，不原样复制整份预设。

阶段 B 验收：固定知识隔离 fixture 中身份/私有知识泄漏为零；质量门禁具有稳定错误码和有界修复；真实 LLM A/B 在不显著增加延迟和费用的前提下改善目标指标。

### 阶段 C：长期上下文和缓存

1. 建立 ContextCompiler 与 segment volatility。 **（最小版 + named budgets 已落地；完整 volatility 未做）**
2. 将 RoundSummary 接入 CampaignRuntimeContext。 **（注入 / 工具可读路径已接）**
3. 用 History Epoch + checkpoint summary 替代固定 20 条滑动窗口。 **（epoch 窗口 + 确定性 checkpoint 已落地；窗口大小仍默认 20）**
4. 建立自动混合召回和 archived watermark。 **（watermark + FarMemoryHit 溯源已有）**
5. 调整 Subagent 公共前缀顺序。 **（部分）**
6. **记忆金字塔 + 缓存友好三窗（目标规格已写，待实现）**：见 [`MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`](./MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md) — 概览→纪要带→近正文、E 同步滑动、active A≥200→B、Director `search_chronicle`/`get_chronicle`。

阶段 C 验收：最近原始消息不会被远记忆挤掉；同一 epoch 内观察到真实前缀复用；epoch rollover 只发生一次预期冷启动；同一历史区间不重复归档；检索结果可追溯到来源。 **（扩展验收以记忆规格 §8 为准）**

### 阶段 D：一致性与发布

1. 将阶段 A 的最小 Turn 屏障升级为完整 TurnState、CampaignRevision 和原子 TurnCommit。 **（Turn/revision 主线已较强；UnitOfWork 仍延后）**
2. Journal/崩溃恢复与 UnitOfWork。
3. 在事务接口稳定后再增加 SQLite/WAL adapter。
4. Fast/Standard/Quality 自适应流水线。
5. 桌面 Bronze/Silver 与 Android 真机矩阵。

阶段 D 验收：在生成、状态推导和写盘各阶段强制中断后均可恢复到明确状态；不会出现正文已提交但 Campaign 变更丢失；重复恢复幂等；桌面和 Android 发布证据可复现。

## 11. 评测指标

至少记录：

- 八股句式和跨轮 n-gram 重复率。
- 人称/身份错误率。
- 私有知识泄漏率。
- 输出契约通过率。
- 时间、位置、数字和物品矛盾率。
- 角色声音可区分度。
- 无依据和解、快速恋爱和普遍讨好比例。
- ScenePlan 推进完成度与未解决剧情线保留率。
- 10/30/60/100 轮后的质量变化。
- 每 Agent 的 prompt、cached、completion tokens。
- 首 token 延迟、总耗时、调用数和费用。

固定比较组：

```text
模型裸跑
模型 + 现有 StoryForge
模型 + NarrativeContract
模型 + DraftQualityGate
模型 + ContextCompiler/History Epoch
模型 + 梁元思想的结构化拆分版本
```

评测不能只依赖另一个 LLM 打分。确定性规则负责格式、泄漏、状态矛盾和重复；盲测人工两两比较负责文风、角色鲜活度和情节吸引力；LLM Judge 只作规模化辅助，并定期用人工结果校准。

每个比较组应固定 Provider、模型、采样参数、输入材料和预算，使用多个 seed/多次采样，报告均值、离散程度和失败样本。缓存专项测试至少包含同一 epoch 的冷请求、热请求、追加一轮后的热请求和 epoch rollover；本地 fingerprint 只用于解释差异，命中结论以供应商 usage 为准。

只有在上述控制条件下稳定改善，才能把建议升级为默认行为。

## 12. 不建议做的事

- 不要继续扩大负面禁词表并直接塞进 system。
- 不要同时启用原生 reasoning、Max 思考指令和显式 CoT 模板。
- 不要每轮固定增加一个昂贵 Critic Agent。
- 不要为了“长记忆”把全部聊天、全部世界书和全部摘要塞进上下文。
- 不要让动态宏、时间、随机数和实时变量进入稳定 system 前缀。
- 不要让未通过校验的正文直接成为 Postprocess 的事实来源。
- 不要先拆微服务或重写整个 Agent 流水线。
- 不要声称导入梁元预设后已与 SillyTavern 等价运行，除非完成最终 messages 对照测试。

最终方向：保留 Campaign-first 和模块化单体，把 StoryForge 从“多 Agent 生成流水线”升级为“结构化叙事契约 + 分层上下文编译 + 提交前质量门禁 + 可观测缓存 + 可恢复 Turn 提交”。

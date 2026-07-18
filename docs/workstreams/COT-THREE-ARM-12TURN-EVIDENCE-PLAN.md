# CoT 三臂 × 12 轮真实模型证据计划

> 状态：待执行
>
> 计划基线：`15fad3b`（正式 reasoning 捕获、持久化、查看与质量回灌）
>
> 主测试卡：`crates/harness-real-llm/fixtures/cot_three_arm_12turn_v1.json`
>
> 目标：分别完成 Disabled、Native、Prompted 三条相互隔离的 12/12 SQLite 真实模型轨迹，审计供应商实际返回的 reasoning、提示式 CoT 注入、工具协议、Agent 分工、质量回灌、叙事状态写回和 harness 约束。

## 1. 结论边界

本轮不是只看“有没有跑完”。每条 arm 必须同时回答：

1. 请求实际采用了哪种 reasoning 模式；供应商有没有返回真实 reasoning 字段。
2. Prompted 是否按 Agent 角色注入正确的 CoT 模块，Disabled/Native 是否没有注入。
3. reasoning、正文与 `tool_calls` 是否彼此分离；工具调用/结果是否成对、成功且真正影响后续行为。
4. Director、Subagent、Editor、Summarizer、PostProcessor、CharacterExtractor、Meta 各自有没有完成职责。
5. 蓝灯、绿灯、Both、Disabled 世界书是否按生产路由工作。
6. 多角色、同名不同 ID、临时角色、私密知识、长线伏笔、变量、任务、Chronicle、重生成与质量 autofix 是否落到同一个 SQLite authority。
7. call/timeout/retry/deadline/response-size/checkpoint/resume/exact-set/secret-scan 等 harness 约束是否真的 fail-closed。
8. 三条 arm 的实际正文质量有什么差异。

本次每种模式只有一条 12 轮连续轨迹。最终可以给出“这三条轨迹上的方向性差异”，不得把它写成普遍的模型优化结论。旧运行没有保存的 hidden reasoning 不得恢复或反推；只审计本次供应商明确返回的 `reasoning_content` / `reasoning` / `thinking`。

## 2. 固定三臂

| arm | StoryForge reasoning | provider surfaced reasoning | 角色 CoT 模块 | 可观测验收 |
| --- | --- | --- | --- | --- |
| A / disabled | `Disabled` | 显式关闭或不请求 | 禁止注入 | 所有调用 `reasoning_required=false`；system 不含 CoT；响应也不应返回 reasoning。若供应商仍返回，标记 `unexpected_reasoning`，该 arm 不得称“无思维链” |
| B / native | `Native` | 开启 | 禁止注入 | 每个真实 LLM 调用都捕获非空 provider reasoning；system 不含角色 CoT |
| C / prompted | `Prompted` | 开启，用于把被提示引导的内部推理放进可捕获通道 | 按角色注入 | 每个真实 LLM 调用捕获非空 reasoning；Director/Editor/Subagent/Summarizer 的 system 分别包含正确模块 |

三臂必须保持以下条件相同：

- 同一个 Git commit、fixture hash、模型、endpoint、protocol、tool mode。
- 同一组 temperature/top_p/max_tokens/extra 中与 reasoning 无关的值。
- 同一份 12 轮 intent 脚本、相同初始 SQLite seed、相同 supplemental probes。
- 每条 arm 使用全新 run ID、SQLite 数据库、evidence root 和 private trace root；禁止跨 arm resume 或复制 accepted state。
- Native 与 Prompted 使用相同的 provider reasoning 开关。二者之间唯一有意的 prompt 差异应是 Prompted 的角色 CoT 模块。
- Disabled 关闭 provider reasoning 开关。若供应商/模型无法在同一模型上开关 surfaced reasoning，先报 capability blocker，不得偷偷换模型或把不同模型结果放进同一对比表。

模型若只有在开启 provider thinking 后才返回 reasoning，则 Prompted arm 的准确描述是“Prompted 角色引导 + provider reasoning 捕获通道”，不能误写成 provider 原生推理完全关闭。

## 3. Gate 0：开跑前必须补齐的 harness 缺口

以下是当前代码事实。执行 Agent 必须先修复、加确定性测试、提交并保持工作树干净；任一项未完成不得消耗三条 12 轮真实调用预算。

### G0.1 fixture 必须可选择且只解析一次

当前 `crates/harness-real-llm/src/sqlite_endurance.rs` 的 `FIXTURE_REL` 和 `fixture_path()` 固定指向旧 `m5_sqlite_endurance_v1.json`；私密扫描还会重新加载该默认路径。

要求：

- 增加显式 `STORYFORGE_EVAL_FIXTURE` 或 typed run config，三臂都绑定新卡。
- 启动时 canonicalize fixture path、读取一次、计算 SHA-256/short hash，并把同一解析结果/identity 传给 seed、resume、private leak、world-info 审计。
- checkpoint/run identity 必须包含 fixture 全 SHA-256；resume 时精确一致。
- 拒绝不存在、越界、运行中改变或 schema/version 不匹配的 fixture。

### G0.2 新卡字段必须真的 seed 到生产数据

旧 `FixtureRoot` 只实际 seed 基础 character/definitions/private knowledge/tasks；旧卡里的 `mvu_schema` 与 `variables` 没有被使用，world-info 也丢弃 secondary keys、logic、disabled、depth、order。

新 parser/seed 至少要真实写入：

- `group`、`base_backstory`、definition `variable_schema`、instance `initial_variables`。
- Campaign variables（含 int/float/string/bool/json 类型）。
- `initial_knowledge` 的 source/propagation/pinned。
- task description/status/priority/triggers 中当前领域支持的部分；不支持的字段必须在报告里明确列为 `unsupported`，不能静默 PASS。
- world-info 的 Constant/Selective/Both/Disabled、secondary keys、SelectiveLogic、depth/order。
- 两个同名阿澈必须得到不同 definition ID 和 instance ID；Extra 不得在 bootstrap 时实例化。

增加 fixture contract 单测，断言 schema、计数、route exact-set、变量类型/默认值、同名 ID、Extra 未实例化、禁用 lore 不可检索。

### G0.3 provider extra 必须能按 arm 配置

当前 `resolve_env_llm_connection()` 只读取 `LLM_BASE_URL/API_KEY/MODEL/TOOL_MODE`，环境变量路径产生 `SamplingParams::default()`，无法传 provider `thinking` / `reasoning_effort`。

要求：

- 增加不含 secret 的 typed `LLM_EXTRA_JSON` 或 arm config file，严格 JSON object 校验。
- 把实际生效的 extra key 名和 value hash 写入 public evidence，不落 API key。
- A/B/C 的 provider reasoning 参数必须在 preflight request body 中被确定性断言。
- `ReasoningMode` 仍只控制 Prompt Module 与 capture requirement；不得硬编码某供应商字段到通用 OpenAI builder。

### G0.4 所有真实调用必须有私密原始 trace

public evidence 继续只保存 prompt/reasoning/content 的长度与 hash，不得把原文混进可 seal 的 JSONL。另建与 evidence root 分离的必需目录：

```text
STORYFORGE_EVAL_PRIVATE_TRACE_ROOT/<run-id>/
  calls.private.jsonl
  turns.private.jsonl
  trace-index.json
```

每个真实调用原子追加：

- run/arm/turn/attempt/call_index/agent_role/agent_instance/tool_round。
- 完整请求 messages、tools、effective sampling、model 和 provider extra（API key/Authorization 永不记录）。
- 完整 provider response：reasoning、content、tool_calls、finish_reason、usage。
- 本地 tool result；私密知识允许存在于 private trace，但文件不得进入 public evidence seal。
- request/system/history/tail/reasoning/content/tool args/tool result 的 SHA-256。

要求：

- private trace 是本次测试的强制门，不是 debug flag；Native/Prompted 任一调用没有原始 reasoning 立即 fail-closed。
- Disabled 若收到 reasoning 也必须原样记录，供判定“unexpected reasoning”。
- 捕获覆盖 Director、每个 Subagent round、Editor、autofix Editor、Summarizer、PostProcessor、CharacterExtractor、Meta 和 cache probe。
- 只记录供应商返回字段，不从正文重构推理。
- 文件权限限制到当前用户；trace index 写入每个文件 hash、调用 exact-set 和 public run identity。
- 中断/恢复时验证 hash chain 与 call reservation；不得重复或覆盖旧行。

### G0.5 有界预算必须恢复

当前真实 SQLite 入口把 `max_calls` 和 `call_limit` 设置为 `u32::MAX`。这不能证明预算约束起效。

三臂统一使用：

- `max accepted turns = 12`。
- `max provider calls = 420/arm`。若 dry capability 数据证明不足，只能在开跑前一次性修改三臂共同上限并记录理由，不能某条 arm 单独放宽。
- `per call timeout = 180s`。
- `stream idle timeout` 使用生产值并写入 identity。
- `suite hard deadline = 4h/arm`。
- `max write attempts = 5/turn`，`max supplemental probe attempts = 3`。
- raw HTTP/SSE 16 MiB、单响应 reasoning 1 MiB、单 Provenance reasoning 4 MiB 继续 fail-closed。

增加预算边界确定性测试：第 421 次拒绝、deadline 拒绝、timeout 分类、reasoning/body 超限、失败调用也占 reservation、resume 不重置预算。

### G0.6 per-agent tool 与 prompt 证明要加强

当前 coverage 主要证明 run-level 有 tool call、选择性世界书调用成功和 Agent Done 事件，不足以判断工具是否由正确 Agent 在正确轮次使用。

public/private 证据要能关联：

- offered tools exact-set。
- tool call/result ID 成对，参数可解析，handler 成功/失败，terminal tool 后无额外模型 round。
- Agent role、turn、attempt、round 和后续 response。
- lore result 的 entry ID/route/probe hash；Disabled lore 的 probe hash 不得出现。
- memory search 的 query、命中 summary ID/code/hash；第 11 轮必须命中第 3 轮的真实摘要事实。

### G0.7 warning-only quality autofix

当前 `force_quality_fault` 只追加 `作为AI...` Error。增加 typed fault profile：

```text
warning_em_dash_and_negation_affirmation
```

它只在第 12 轮首版 Editor 正文之后注入：

- 一处破折号（Warning，不达到 3 处 Error）。
- 一处“不是……而是……”（Warning）。

必须证明：首检恰好包含 `EmDashDensity` 和 `NegationThenAffirmation`；完整通用修订指令进入 autofix Editor 请求；第二次 Editor-only 输出后两项归零；正文/quality report/hash/provenance/reasoning 原子同步；没有重跑 Director/Subagent。

### G0.8 受控中断与独立 PID

增加 `STORYFORGE_EVAL_STOP_AFTER_ACCEPTED_TURN=6` 的 graceful checkpoint stop：第 6 轮 Accept、checkpoint、SQLite audit 与 trace flush 完成后退出专用状态码。控制器再以同一 run ID/arm/roots 启动新 PID，从第 7 轮继续。

- 监控器只能观察 PID，不能以 poll loop 生命周期托管 `cargo` 子进程。
- Windows 使用隐藏、独立进程和明确 PID 文件；stdout/stderr 分文件。
- resume 后不得重放 1-6 轮，accepted count、call ledger、trace exact-set 和 campaign revision 单调。

## 4. 测试卡覆盖合同

`cot_three_arm_12turn_v1.json` 是合成卡，不包含用户私人角色或真实秘密。

| 维度 | 卡内覆盖 | 强制证明 |
| --- | --- | --- |
| 蓝灯 | 白潮钟律、双印开闸 | Constant/Both 内容进入 Director 稳定上下文 |
| 绿灯 | 玻璃蛾、黑伞邮路 | 只有关键词命中后由 `search_world_info` 返回 |
| Both | 倒悬钟/第七声 | 同时可在稳定上下文和关键词检索命中 |
| Disabled | 金色渡鸦伪线索 | prompt、tool result、reasoning、正文均不得出现 probe |
| 多角色 | 1 protagonist、4 supporting、1 extra | CharacterExtractor exact-set，1/2/3 Subagent 路径 |
| 同名 | 两个“阿澈” | 以不同 instance ID 分派，知识与变量不串线 |
| 临时角色 | 灰帽信使 | 第 9 轮创建 temporary，不在 bootstrap 常驻，不继承私密知识 |
| 私密知识 | 宁/沈/顾各一条合成 owner-only probe | owner 可据此行动但正文不输出；non-owner/narrator 不得知道 |
| 长线伏笔 | 缺第三齿棘轮 | 1/3 轮种下，11 轮真实检索，12 轮阶段性回收 |
| 长线未决 | 异常脉冲发送者 | 12 轮后仍未完全揭晓 |
| 变量 | Campaign + Character，五种 JSON 类型 | schema、初值、turn mutation、SQLite reopen 后值与类型正确 |
| 关系 | trust/suspicion 等角色变量 | 关系变化有事件依据；当前无独立关系表，禁止声称覆盖不存在的表 |
| 任务 | 防潮闸、信号、同名证词、棘轮 | task 状态变化与事件/变量一致，PostProcessor 不乱完成长线任务 |
| 记忆 | Summary/Chronicle | 第 11 轮真实工具命中，不以“曾注入”代替“已取回” |
| 重生成 | overall/editor/subagent | lineage/target 精确，无非目标 Agent 重跑 |
| 质量回灌 | warning-only 两类规则 | QualityReport → Editor → 复检 → 原子同步 |

## 5. 固定 12 轮动作

执行 intent 以 fixture `evaluation.turn_script` 为唯一来源；controller 对 intent canonical JSON/hash，三臂必须相同。

| 轮 | 主动作 | 重点覆盖 |
| --- | --- | --- |
| 1 | Write / 1 Subagent / Constant | 蓝灯钟律；缺齿棘轮种子；保留未决 |
| 2 | Write / 2 Subagents / Selective | 玻璃蛾+潮汐表绿灯检索；档案阿澈 |
| 3 | Early fact inject | 缺第三齿进入真实 Summary/Chronicle |
| 4 | Write / 3 Subagents / Both | 倒悬钟；两个阿澈不同 instance ID |
| 5 | Overall regenerate | 全链重跑；已知事实保持，视角/冲突改变 |
| 6 | Editor-only regenerate | 只重跑 Editor；Accept 后受控退出/checkpoint |
| 7 | Subagent-only regenerate | 只重跑渡船阿澈；档案阿澈事实不漂 |
| 8 | Owner private probe | 宁鸢可据私密上下文行动，原文不泄漏 |
| 9 | Non-owner private probe + temporary | 沈砚不得知道暗号；灰帽信使临时实例 |
| 10 | Cache/context invalidation | Campaign 变量推进；epoch/cache 指纹变化 |
| 11 | Early fact check | 真实 memory tool 找回第 3 轮事实 |
| 12 | Warning-only quality autofix | 两类 Warning 完整回灌 Editor；伏笔阶段性回收 |

## 6. reasoning 审计门

### 6.1 通用格式

逐调用检查：

- reasoning 是供应商响应中的独立字段，非空时 UTF-8 合法、长度在上限内。
- reasoning 不得拼接进最终正文；正文不得出现 `<think>`、`<thinking>`、`思考指引`、工具 JSON 或编辑说明。
- `tool_calls` 必须在协议字段内；reasoning 可以说明“需要查询”，但不能用伪 JSON 冒充已经调用。
- content、reasoning、tool call/result 各自 hash 与 raw trace 一致。
- retry 的失败/成功 reasoning 不得错挂到另一个 attempt/variant。
- reasoning 若含 synthetic private probe，只能留在 private trace；最终正文仍必须通过 leak gate。审计报告只写 fingerprint，不复制 probe。

### 6.2 Prompted 角色模块

Prompted arm 的实际 system message 必须逐角色包含且只包含对应模块：

| Agent | 必含语义/模块 ID | reasoning 人工/模型审阅点 |
| --- | --- | --- |
| Director | `builtin-cot-director-plan` / 导演规划 | 意图、结构差异、2-4 个待决、信息分派、只输出 Plan；不写正文 |
| Editor | `builtin-cot-editor-merge` / 编剧合并 | 材料、单一舞台焦点、出口检查；不写总结腔；第 12 轮识别两类质量修订 |
| Subagent | `builtin-cot-subagent-perform` / 角色表演 | 身份、独立欲望、合法可知、主动动作；不代写他人心理 |
| Summarizer | `builtin-cot-summarizer-extract` / 摘要抽取 | 只取本轮新增、丢气氛/重复、保留未决与伏笔 |
| PostProcessor | 无内置角色 CoT | 仍须捕获 provider reasoning，但不得伪称注入了未配置模块 |
| CharacterExtractor | 无内置角色 CoT | 仍须捕获 provider reasoning，产出 exact-set definitions |
| Meta | 无内置角色 CoT | 仍须捕获 provider reasoning，保持只读 probe |

Disabled/Native 的上述四个 system message 均不得出现 `思考指引` 或对应 CoT 模块内容。Native reasoning 不强求 Prompted 的编号格式，只检查任务相关性、信息边界、工具决定与最终产物一致性。

在实际三条轨迹之前做一个零状态 immutable prompt assembly gate：同 fixture、同 role、同 context、同 tool mode 下，Disabled 与 Native 的 system prompt canonical hash 应相同；Prompted 应不同，且差异仅来自预期 CoT 模块。实际连续轨迹因各 arm 状态可能分叉，不能只拿整段 system hash 差异冒充因果证明。

### 6.3 reasoning coverage 指标

按 `role × arm` 输出：

- calls total / reasoning required / captured / missing / unexpected。
- reasoning char/token 分布、空白率、重复率、正文重合率。
- prompt marker pass、role rubric pass、tool-decision alignment pass。
- 每个 call 的 private trace locator 和 public hash。

Native/Prompted 任一 required call missing reasoning = arm FAIL；不得只看 Director/Editor 的 Provenance 而忽略 Summarizer/PostProcessor 等调用。

## 7. 工具与 Agent 作用审计

### Director

- `emit_plan` 必须作为 terminal tool；Plan 可解析且不能退化为正文。
- 第 1/2/4 轮实际 Subagent task 数分别为 1/2/3。
- 同名角色先 `list_characters`，必要时 `get_character`；Plan 使用 instance ID。
- 第 2/4 轮 `search_world_info` 命中预期 entry ID/route。
- 第 11 轮 memory tool 命中第 3 轮实际 summary/chronicle，不接受空调用。
- 每轮 `scene_plan` 有冲突、对立目标、beats、`must_not_resolve` 或 exit hook；不得一次解决全部谜团。

### Subagent

- 每个 task 对应一个实际 Subagent 完成事件和 provenance snapshot。
- 绑定自己的 instance，只允许 `get_character` 查自己；不得读取他人 private knowledge。
- 输出体现独立 desire、at_hand、move，不只是复述 Director brief。
- 两个阿澈的事实、动作、变量不串线。
- 第 7 轮只替换目标 Subagent 的 snapshot/输出，其他 snapshot hash 不变。

### Editor

- 初次 Editor 合并所有已完成 Subagent 片段，不漏主要 beat，不增加角色不知之事。
- 正文仅保留一个清晰叙述焦点；动作/对白推动，禁止 Agent 报告腔。
- 第 6 轮 Editor-only regenerate 保持 Plan/Subagent hash。
- 第 12 轮首检 report 的两条 generic fix instruction 原样进入 autofix request；二次输出只修问题，不大幅改写事实。

### Summarizer

- 每个 accepted turn 都有真实 SummaryDone/SQLite summary。
- 摘要不包含气氛堆砌、测试说明、工具协议或 synthetic secret。
- 第 3 轮写入棘轮事实；第 11 轮可以从实际 catalog 取回。

### PostProcessor

- `emit_postprocess` terminal tool call/result 成对。
- knowledge/variable/task mutation 全部通过 production postprocess UoW。
- mutation 引用真实 instance ID；类型匹配 schema；私密知识不广播。
- 无 outcome 时不得伪造 Agent Done 或状态变化。

### CharacterExtractor 与 Meta

- 三臂都启用 `STORYFORGE_EVAL_CHARACTER_EXTRACTOR_PROBE=1`、`STORYFORGE_EVAL_META_PROBE=1`、`STORYFORGE_EVAL_CACHE_PROBE=1`。
- CharacterExtractor 必须调用 `emit_characters`，不能 fallback；definition exact-set 为 6，两个阿澈同名不同 ID，Extra 类型正确。
- Meta 必须真实调用 `inspect_campaign`、`inspect_tasks`，读取 live SQLite，probe 前后 canonical content hash 不变。
- cache probe 三次调用保留请求指纹；provider 未命中缓存可以如实报告，不能把零 cached token 写成命中。

## 8. 世界书、状态和记忆门

- Constant：调用前 system/context 已包含蓝灯 probe；不要求 search tool 才能使用。
- Selective：未命中 keys/secondary logic 时不注入；第 2 轮命中玻璃蛾+潮汐表后返回指定 entry。
- Both：稳定上下文与 search result 都能观察到同一 entry hash。
- Disabled：所有 request message、tool result、reasoning、accepted text、summary、knowledge 和 vector hit 全局扫描均不得出现 disabled probe。
- 世界书内容是否被使用不能只凭正文出现关键词判断；必须有 prompt/tool route 证据。
- 第 10 轮变量更新后 SQLite reopen，值、类型、`last_updated_turn` 与 revision 单调。
- PostProcessor 没有合理依据时允许某个“建议变量”不更新，但必须报告 `not_observed`；禁止用 fixture 预期直接篡改 DB 冒充模型抽取。
- 第 11 轮 memory hit 必须带 summary/chronicle ID 与 source turn；第 12 轮正文应有阶段性伏笔回收，但异常脉冲发送者保持未决。

## 9. 实际输出质量

### 9.1 确定性统计

逐 arm/turn 保存并比较：

- accepted 正文字符数/token、段落数、对白比例。
- QualityWarningCode 首检/复检计数、autofix 次数、重试次数。
- 破折号、“不是……而是……”、元描述、format leak、8-gram、连续重复。
- 跨轮重复 8/16/32-gram、相邻轮正文相似度。
- 角色名/instance 归属错误、private/disabled probe、时间/潮位/闸门状态冲突。
- 伏笔 seed/retrieval/payoff exact-set；未决项是否被过早解决。
- 输出长度只做分布，不以“更长”直接等同“更好”。

### 9.2 盲评

去掉 arm/mode 标签，以确定种子打乱 36 个 accepted turn，使用与生成模型不同的 grader 或两名人工审阅者。每项 1-5 分并给证据句位置：

1. 连续性与事实一致。
2. 角色声音和独立动机。
3. 信息边界与同名角色区分。
4. 场景推进、冲突和未决控制。
5. 文风自然度，少总结腔/模板句。
6. 伏笔种植、回查与阶段性回收。
7. 世界书/变量/任务与正文融合是否自然。

grader 不得读取 raw reasoning 后再评正文，避免被“看起来思考很完整”影响。先完成正文盲评，再单独做 reasoning/Agent 审计。报告给均值、中位数、每轮配对差和分歧，不隐藏低分样本。

## 10. Harness 约束验收

每条 arm 必须满足：

- 12/12 intended turns 达到 accepted terminal state。
- 每轮至少一个真实 provider call；planned/observed CoverageRow exact-set，无 missing/duplicate/extra。
- SQLite authoritative=true，json fallback=false。
- 第 6 轮受控退出后新 PID 从第 7 轮恢复；1-6 不重放。
- call reservation、private trace、public calls、turn/attempt/variant exact-set 一致。
- calls ≤ 420、每调用 timeout/总 deadline 生效，失败调用计入预算。
- Native/Prompted reasoning exact-set；Disabled unexpected reasoning 为 0。
- tool call/result correlation 100%，terminal tool 后无额外 round。
- postprocess proof、quality report、autofix provenance、Accept、outbox 一致。
- fixture/private/disabled secret scan、evidence seal、offline verify 全 PASS。
- run 开始和结束 Git commit 相同，工作树 clean。

任何 required gate 失败时保留证据并停止该 arm。只可按已声明的 per-turn retry 恢复瞬态/quality-blocked 错误；不得删除失败证据、换 run ID 伪装续跑或把 11/12 写成 PASS。

## 11. 执行顺序

1. 阅读本计划和 fixture，完成 G0.1-G0.8。
2. 为每个 G0 改动写确定性单测；跑受影响 workspace tests、format、clippy、frontend tests（若改前端）。
3. 提交 harness 改动。真实 evidence runner 要求 clean worktree。
4. 跑 immutable prompt assembly/capability preflight；确认同一模型可实现三臂。preflight 不计入任何 12 轮 run。
5. 建三个独立 run root，按 A Disabled → B Native → C Prompted 顺序，使用独立 PID 控制器。记录实际开始/结束时间，避免 provider 时段差异被忽略。
6. 每条 arm 第 6 轮后 graceful stop，再 resume 完成 7-12。
7. 每条 arm 完成后立即 public seal/offline verify、private trace exact-set、SQLite reopen audit；上一 arm 未封存不得启动下一 arm。
8. 完成 36 篇正文盲评，再做 reasoning/Agent 审计。
9. 写 RESULT，附 public evidence 与 private trace index 的绝对路径，不复制 API key。

建议环境骨架（具体 provider extra 由 capability preflight 决定）：

```powershell
$env:STORYFORGE_EVAL_REAL_LLM = '1'
$env:STORYFORGE_EVAL_ENDURANCE_STAGE = 'coverage'
$env:STORYFORGE_EVAL_SUPPLEMENTAL_MATRIX = '1'
$env:STORYFORGE_EVAL_META_PROBE = '1'
$env:STORYFORGE_EVAL_CHARACTER_EXTRACTOR_PROBE = '1'
$env:STORYFORGE_EVAL_CACHE_PROBE = '1'
$env:STORYFORGE_EVAL_FIXTURE = 'crates/harness-real-llm/fixtures/cot_three_arm_12turn_v1.json'
$env:STORYFORGE_EVAL_MAX_CALLS = '420'
$env:STORYFORGE_EVAL_CALL_TIMEOUT_SECS = '180'
$env:STORYFORGE_EVAL_DEADLINE_SECS = '14400'
$env:STORYFORGE_EVAL_STOP_AFTER_ACCEPTED_TURN = '6'
```

API key 只从当前进程 secret 环境读取。控制器/日志/manifest/private trace 都不得打印 Authorization header 或 `LLM_API_KEY`。

## 12. RESULT 必填表

### 12.1 Identity

- Git full SHA / branch / clean status。
- fixture version / SHA-256 / counts。
- model SHA-256/label、endpoint hash、protocol、tool mode。
- 非敏感 sampling 与 provider extra keys/value hashes。
- 三个 run ID、public/private roots、开始/结束时间。

### 12.2 每臂结果

- accepted 轮数、calls、attempts/retries、resume 次数、token/cache/latency。
- reasoning by role：required/captured/missing/unexpected/chars/hashes/rubric。
- Prompt Module injection exact-set 与 system hash。
- tools by role：offered/called/succeeded/failed/unpaired/invalid args。
- Agent role effectiveness checklist。
- world-info/knowledge/variable/task/Chronicle/regenerate/quality gates。
- QualityReport 首检/复检、autofix、正文长度和确定性样式指标。
- blind quality scores 与低分样本 locator。
- seal/offline verify/secret scan/SQLite audit。

### 12.3 三臂对比

- 按同一 turn 配对比较 Disabled vs Native、Native vs Prompted、Disabled vs Prompted。
- Prompted 的效果只能在注入门 PASS 后讨论。
- reasoning 更长不能自动算更好；工具更多不能自动算 Agent 更有效。
- 若模式导致调用数/延迟显著增加，和质量得分一起报告。
- 明确列出 provider/单轨迹/时序等混杂因素和所有未完成项。

## 13. 交付物

执行完成后应产生：

```text
docs/workstreams/COT-THREE-ARM-12TURN-EVIDENCE-RESULT.md
<public-evidence-root>/disabled/<run-id>/...
<public-evidence-root>/native/<run-id>/...
<public-evidence-root>/prompted/<run-id>/...
<private-trace-root>/disabled/<run-id>/trace-index.json
<private-trace-root>/native/<run-id>/trace-index.json
<private-trace-root>/prompted/<run-id>/trace-index.json
```

public evidence 和 RESULT 可以脱敏提交；原始 prompt/reasoning/output/tool result 只保存在本地 private trace，除非用户另行明确要求提交。

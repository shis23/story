# CoT 三臂 × 80 轮真实模型综合验证计划

> 状态：待执行
>
> 实现基线：`15fad3b`（正式 reasoning 捕获、持久化、查看与质量回灌）
>
> 计划基线：`4910af5`（最初 12 轮方案；本文件与新卡将其扩展为 80 轮）
>
> 主测试卡：`crates/harness-real-llm/fixtures/cot_three_arm_80turn_v1.json`
>
> 目标：在同一提交、同一模型、同一 endpoint 与同一 80 轮 canonical schedule 下，分别完成 Disabled、Native、Prompted 三条相互隔离的 80/80 SQLite 真实模型轨迹，逐调用审计 reasoning、提示式 CoT、工具协议、Agent 分工、长线叙事状态、故障恢复、质量回灌与 harness fail-closed 约束。

## 1. 结论边界

这不是只看“跑没跑完”的压力测试。每条 arm 必须回答：

1. 请求采用了哪种 reasoning 模式，供应商实际返回了什么独立 reasoning 字段。
2. Prompted 是否只向正确角色注入正确 CoT 模块；Disabled/Native 是否完全未注入。
3. reasoning、正文、工具调用和工具结果是否分离、成对、可追溯，并影响后续行为。
4. Director、Subagent、Editor、Summarizer、PostProcessor、CharacterExtractor、Meta 是否各自完成职责。
5. Constant（蓝灯）、Selective（绿灯 AND/OR/NOT）、Both、Disabled、Global 世界书是否按生产路由生效。
6. 多角色、同名不同 ID、临时角色、私密知识、知识传播、变量、任务、Chronicle、长线伏笔、重生成和质量 autofix 是否落在同一 SQLite authority。
7. call budget、timeout、deadline、response-size、exact-set、checkpoint/resume、secret scan、seal 是否真正 fail-closed。
8. 三臂在实际正文质量、连续性、信息边界、工具效率和成本上有什么配对差异。

每种模式仍只有一条 80 轮连续轨迹。240 个 accepted turn 能增加情景覆盖和长线观察，不能消除 provider 随机性或证明普遍因果。最终只能报告本次配对轨迹上的观察。旧运行未保存的 hidden reasoning 不得恢复或反推；只审计本次供应商明确返回的 `reasoning_content`、`reasoning` 或 `thinking`。

## 2. 固定三臂与不变量

| arm | StoryForge 模式 | provider reasoning | 角色 CoT 模块 | 必须满足 |
| --- | --- | --- | --- | --- |
| A / Disabled | `Disabled` | 关闭或不请求 | 禁止注入 | 所有调用 `reasoning_required=false`；system 无 CoT；响应无 reasoning。若 provider 仍返回，原样捕获并标 `unexpected_reasoning`，不得称为“无思维链” |
| B / Native | `Native` | 开启 | 禁止注入 | 每个真实 LLM 调用都捕获非空 provider reasoning；system 无角色 CoT |
| C / Prompted | `Prompted` | 与 Native 同一开关 | 按角色注入 | 每个真实 LLM 调用捕获非空 reasoning；Director/Editor/Subagent/Summarizer 只含对应模块 |

三臂必须保持相同：

- Git full SHA、fixture SHA-256、80 轮 schedule SHA-256、模型、endpoint、protocol、tool mode。
- temperature、top_p、max_tokens、seed（若 provider 支持）及全部非 reasoning sampling。
- 初始 SQLite seed、旁路探针定义、质量 fault profiles、重启边界和上限。
- 每臂使用全新 run ID、SQLite、public evidence root、private trace root；禁止跨臂复制 accepted state 或 resume。
- Native 与 Prompted 的 provider thinking 参数完全相同；唯一有意 prompt 差异是 Prompted 的角色 CoT 模块。

若同一模型无法同时支持三臂，capability preflight 必须停止并报告 blocker；不得换模型拼成比较结果。

## 3. Gate 0：真实运行前必须补齐

执行 Agent 必须先完成 G0.1–G0.10、写确定性测试、提交并保持工作树干净。任一 Gate 未过，不得消耗真实 80 轮预算。

当前基线另有两个已复现的测试债务：`budget::tests::eval_reasoning_override_changes_effective_request` 与 `budget::tests::budget_blocks_after_max_calls` 仍假定 `SamplingParams::default()` 为 Prompted，但生产默认已是 Disabled。执行 Agent 应让测试显式构造所需 reasoning 模式（而不是恢复不安全的全局 Prompted 默认或放松捕获断言），并在 Gate 0 提交中修复。现状为 harness lib 74 passed / 2 failed，不能误报全绿。

### G0.1 fixture 单次解析与不可变身份

- 增加 typed `STORYFORGE_EVAL_FIXTURE`；启动时 canonicalize、只读取一次、校验 schema/version，计算完整 SHA-256。
- 同一解析对象用于 seed、schedule、resume、world-info/private scan 与最终审计。
- run/checkpoint identity 同时绑定 fixture hash、schedule hash、target turns、arm 和 sampling identity。
- 文件不存在、运行中变化、turn 非 1–80 连续、未知 action、重复/悬空 ID 均在首个 provider call 前拒绝。

### G0.2 新卡字段真实 seed

生产数据至少写入：

- definition group/base_backstory/variable schema、instance initial variables。
- Campaign 的 int/float/string/bool/json 变量和类型。
- initial knowledge 的 source/propagation/pinned/private owner。
- task description/status/priority/triggers 中领域支持的字段；不支持项明确列 `unsupported`。
- world-info Constant/Selective/Both/Disabled/Global、secondary keys、SelectiveLogic AND/OR/NOT、depth/order。
- regex script 的 input/world_info/reasoning placement。

两个同名阿澈必须具有不同 definition/instance ID；Extra 不在 bootstrap 实例化。fixture contract 测试断言计数、route exact-set、变量类型、同名隔离、Extra 未实例化、禁用 lore 不可检索。

### G0.3 arm 级 provider extra

- 增加严格 JSON-object 的 `LLM_EXTRA_JSON` 或 typed arm config；Authorization/API key 永不落盘。
- public evidence 记录实际 extra key 与 value hash；private trace 记录脱密后的完整非 secret 配置。
- preflight 对实际 request body 断言 A 关闭、B/C 同样开启 provider reasoning。
- `ReasoningMode` 控制 Prompt Module 与 capture requirement，供应商特有字段不得硬编码进通用 builder。

### G0.4 全调用私密原始 trace

public JSONL 只写长度、hash 和结构化结论。独立保存：

```text
STORYFORGE_EVAL_PRIVATE_TRACE_ROOT/<arm>/<run-id>/
  calls.private.jsonl
  turns.private.jsonl
  trace-index.json
```

每次真实调用原子追加 run/arm/turn/attempt/call_index/role/instance/round、完整 messages/tools/effective sampling、完整 response reasoning/content/tool_calls/finish_reason/usage、本地 tool result，以及各段 SHA-256。覆盖 Director、每个 Subagent round、Editor、autofix Editor、Summarizer、PostProcessor、CharacterExtractor、Meta 和 cache probe。

Native/Prompted 缺 reasoning 立即 fail-closed；Disabled 收到意外 reasoning 也要原样捕获。只保存 provider 返回字段，不从正文重构推理。resume 必须验证 hash chain、call reservation 与 exact-set，不得覆盖旧行。

### G0.5 typed 80 轮长程 stage 与有界预算

- 增加 `long_coverage`（或同义 typed enum）stage，目标固定 80；禁止复用会静默截断到 12/30/100 的旧 stage。
- `max accepted turns = 80`。
- `max provider calls = 2800/arm`；第 2801 次必须拒绝，失败调用也占 reservation，resume 不重置。
- `per call timeout = 180s`；生产 stream idle timeout 写入 identity。
- `suite hard deadline = 86400s/arm`。
- `max write attempts = 5/turn`；`max supplemental probe attempts = 3`。
- HTTP/SSE 16 MiB、单 response reasoning 1 MiB、单 Provenance reasoning 4 MiB 保持 fail-closed。

若 dry capability 证明 2800 不足，只能在三臂开跑前一次性统一改上限、更新计划 hash 并写理由；不得给某一臂单独放宽。

### G0.6 per-agent prompt/tool 证据

每个调用必须关联 offered-tools exact-set、tool call/result ID、args parse、handler outcome、terminal 状态、后续 response、role/turn/attempt/round。世界书结果记录 entry ID/route/probe hash；memory 记录 query、summary/chronicle ID/hash/source turn。terminal tool 后不得再出现额外模型 round。

### G0.7 多 profile 质量故障回灌

仅对 fixture 明示轮次首版 Editor 输出注入 typed fault：

| turn | profile | 首检要求 | 修复要求 |
| --- | --- | --- | --- |
| 12、78 | `warning_em_dash_and_negation_affirmation` | `EmDashDensity` + `NegationThenAffirmation` | 完整报告回灌，二检归零 |
| 29 | `error_meta_and_format_leak` | 元描述 + 格式泄漏 Error | Editor-only 修复，正文无测试/协议语句 |
| 57 | `warning_ngram_repetition` | 重复 n-gram Warning | 只消重，不漂移事实 |
| 58 | `warning_too_short` | TooShort Warning | 扩写动作/对白，不注水总结 |
| 59 | `error_non_owner_private_leak` | 合成 private probe 被 gate 拦截 | 修订后 probe 全链归零 |

每次证明 QualityReport 完整进入 autofix request、复检、正文/report/hash/provenance/reasoning 原子同步，且没有重跑 Director/Subagent。

### G0.8 四次独立 PID 恢复

typed stop schedule 为 accepted turn `6,20,40,60`。每个边界完成 Accept、checkpoint、SQLite audit、trace flush 后以专用状态码退出；控制器用同一 run/arm/roots 和新 PID 继续 7、21、41、61。

监控器只观察 PID，不能让 poll loop 生命周期托管 cargo。resume 后旧 turn 不重放；accepted count、call ledger、trace chain、campaign revision 单调。turn 80 checkpoint 用于最终封存，不再 resume。

### G0.9 canonical action interpreter

controller 必须逐项执行 `evaluation.turn_script`，并在每轮 public evidence 写 action canonical JSON/hash、expected/observed outcome。支持 write、early fact、三类 regenerate、private owner/non-owner/narrator、temporary create/promote、knowledge scopes、variable mutations、task lifecycle、epoch rollover、Chronicle compression/retrieval、world-info NOT、cache stable/invalidate、tool fault、quality fault 与 state audit。未知 action fail-closed，禁止当普通 write 跳过。

### G0.10 clone-only 旁路探针隔离

cancel/crash、stale edit、fork、Meta typed patch、MVU preview/apply、regex placement、stream/nonstream parity、text-tool fallback、variable type rejection 等破坏性或非主叙事探针必须在 SQLite clone/独立 probe run 执行。主轨迹只记录关联 proof；clone 结果不得写回 canonical 80 轮 authority，也不得计入正文盲评。

## 4. 测试卡覆盖合同

`cot_three_arm_80turn_v1.json` 完全合成，不包含用户私人角色或真实秘密。

| 维度 | 卡内覆盖 | 强制证明 |
| --- | --- | --- |
| 世界书 | 2 Constant、3 Selective（AND/OR/NOT）、1 Both、1 Global、1 adversarial、1 Disabled | stable context、tool result、miss、禁用全链扫描分别有证据 |
| regex | input、world_info、reasoning 三种 placement | 只在指定阶段变换；reasoning marker 不进入正文 |
| 多角色 | protagonist/supporting/extra；1/2/3 Subagent | CharacterExtractor exact-set；分派与 instance 对齐 |
| 同名角色 | 两个“阿澈” | definition/instance/knowledge/variables/provenance 不串线 |
| 临时角色 | 灰帽信使创建、单独行动、晋升 | bootstrap 不常驻；晋升后 ID/历史连续，不继承他人私密知识 |
| 私密知识 | 宁/沈/顾 owner-only probes | owner 可行动；non-owner/narrator/正文不得知道 |
| 长线事实 | 棘轮、玻璃蛾、干隧道、双发报机、盐墨、九秒漂移 | 多个 epoch 后真实 memory/Chronicle 工具取回；分阶段回收 |
| 变量 | Campaign + Character，五种 JSON 类型 | 合法变更、关系矩阵、类型错误 clone 拒绝、reopen 一致 |
| 任务 | 9 条任务 | pending/active/completed/abandoned 有事件依据且单调 |
| 知识 | private/group/all/narrator source | propagation 与 pinned/source/owner 真实写回，隔离正确 |
| 重生成 | overall/editor/subagent 多次分布 | lineage/target 精确，非目标 Agent hash 不变 |
| 质量 | 6 个 fault turn、5 类 profile | Warning 和 Error 都反馈 Editor，最多一次有界 autofix 后复检 |
| 工具对抗 | 未知工具、terminal 后调用、BadArgs 恢复、无命中、文本 fallback | handler 分类正确，不把错误/伪指令当事实 |
| 恢复 | 4 次新 PID resume、SQLite reopen、epoch rollover | 无重放、预算不重置、hash chain/authority 单调 |

## 5. 固定 80 轮阶段

fixture 的 `evaluation.turn_script` 是唯一 canonical 动作源；下表仅说明阶段意图。

| phase | turns | 主要覆盖 |
| --- | --- | --- |
| 1 | 1–10 | bootstrap、Constant/Selective/Both、同名 ID、early fact、overall/editor/subagent regenerate、owner/non-owner、temporary、cache invalidation |
| 2 | 11–20 | memory 回查、warning autofix、变量五型、OR 世界书、任务/关系/知识状态、radio 事实、narrator privacy、首次长阶段 checkpoint |
| 3 | 21–30 | resume 连续性、Selective NOT hit/miss、temporary promotion、三类 regenerate、must-not-reveal、Error autofix、epoch rollover |
| 4 | 31–40 | Chronicle 压缩、盐墨事实、跨 epoch 回查、full Chronicle、任务单调、group/all/private knowledge、cache 稳定、第二次 resume |
| 5 | 41–50 | resume 后连续性、1/2/3 Subagent、临时实例、重生成矩阵、九秒漂移事实、owner/non-owner/narrator、world-info prompt-injection 当数据 |
| 6 | 51–60 | unknown tool、terminal tool、BadArgs 恢复、memory no-hit、Disabled lore、ngram/TooShort/private leak autofix、第三次 resume 与 recovery probes |
| 7 | 61–70 | 所有变量类型、关系 JSON、type mismatch clone、任务 activate/complete/abandon、knowledge told/group/all/private、第二次 epoch rollover |
| 8 | 71–80 | 长距离 memory/Chronicle 取回、三类 regenerate、证据链逐步收束、turn 77 棘轮 payoff、turn 78 warning 回灌、turn 79/80 authority 与 seal 审计 |

必须按顺序执行 1–80；不得挑选“代表轮”。每轮 expected assertion、side probe 和 must-not-resolve 以 fixture 为准。

## 6. reasoning、工具与 Agent 审计

### 6.1 reasoning 通用门

- reasoning 必须来自独立 provider 字段，UTF-8 合法、非空（required 时）、未超限，不得拼入正文。
- 正文不得出现 `<think>`、思考指引、工具 JSON、协议、编辑说明或 synthetic probe。
- reasoning/content/tool args/tool result 的 hash 与 private trace 一致；retry 不得错挂 attempt/variant。
- reasoning 可表达工具意图，但不能用伪 JSON 冒充真实调用；工具使用必须由协议字段证明。
- 按 `arm × role × phase` 输出 total/required/captured/missing/unexpected、chars/tokens、重复率、正文重合率、role rubric、tool-decision alignment。

### 6.2 Prompted 模块 exact-set

| role | Prompted 必含 | 主要职责 |
| --- | --- | --- |
| Director | `builtin-cot-director-plan` | 分解意图、信息分派、未决控制、只用 `emit_plan` 收束 |
| Subagent | `builtin-cot-subagent-perform` | 角色合法可知、独立欲望与动作，不代写他人心理 |
| Editor | `builtin-cot-editor-merge` | 合并材料、单一舞台焦点、出口检查、处理质量反馈 |
| Summarizer | `builtin-cot-summarizer-extract` | 只取新增事实、未决、伏笔和状态，丢弃气氛/重复 |
| PostProcessor / CharacterExtractor / Meta | 无内置模块 | 仍捕获 reasoning，但不得伪称注入 |

Disabled/Native 的 system 均不得包含上述 CoT。immutable preflight 中 Disabled/Native system canonical hash 相同；Prompted 只多预期模块。实际轨迹状态可能分叉，禁止只用后期整段 hash 差异冒充注入证明。

### 6.3 角色有效性

- Director：`emit_plan` terminal；工具先查后用；1/2/3 Subagent cardinality 正确；同名角色按 instance ID；保留 must-not-resolve。
- Subagent：每个 task 有完成事件和 snapshot；只读合法知识；同名角色声音、事实、变量不串线；目标重生成只替换目标。
- Editor：合并全部材料，不新增角色不知事实；editor-only/autofix 保持 Plan/Subagent hash；质量修复不大幅漂移事实。
- Summarizer：每个 accepted turn 有 SQLite summary；early facts 带 source turn，能被后期真实工具检索。
- PostProcessor：mutation 经 production UoW；知识/变量/任务引用真实 ID、类型正确、无依据时不乱更新。
- CharacterExtractor：`emit_characters` exact-set=6；两个阿澈同名不同 ID，Extra 类型正确。
- Meta：真实读取 live SQLite；只读 probe 前后 canonical content hash 不变；typed patch 只在 clone。

## 7. 长线状态、世界书与恢复门

- Constant/Global 在稳定上下文可见；Selective 仅在 keys + AND/OR/NOT 逻辑命中后通过工具返回；Both 两路 hash 相同；Disabled probe 在 prompt/reasoning/tool/正文/summary/knowledge/vector hit 全链为零。
- adversarial 红蜡边注中的“忽略上级要求”只作为故事数据，不得改变 system、工具边界或泄露暗号。
- 每次变量/任务/知识变更后验证类型、source、owner、`last_updated_turn`、revision；指定轮 SQLite reopen 后完全一致。
- turn 31 Chronicle 压缩不能丢 source lineage；turn 32、42、71、72 的回查必须命中真实历史记录而非 fixture 直接注入。
- 四次 resume 后 accepted prefix 不重放，旧 request/output hash 不变；budget/trace/campaign revision 单调。
- turn 77 只回收棘轮证据链；异常脉冲幕后身份在 turn 80 仍保留制度性未决，避免“为了测试完成而强行完结”。

## 8. 实际输出质量与盲评

### 8.1 确定性指标

逐 arm/turn 保存：accepted chars/tokens/段落/对白比例、首检/复检 warning/error、autofix/retry、破折号与否定转肯定、元描述/格式泄漏、8/16/32-gram 重复、相邻/跨阶段相似度、同名归属错误、private/disabled probe、时间/潮位/闸门冲突、伏笔 seed/retrieval/payoff exact-set、未决过早解决。输出更长不自动等于质量更好。

### 8.2 240 篇正文盲评

先去除 arm/mode 标签，以固定种子打乱 240 个 accepted turn，再由与生成模型不同的 grader 或两名人工审阅者评分；reasoning 审计必须在盲评完成后进行。每项 1–5 分并给证据句 locator：

1. 连续性与事实一致。
2. 角色声音、独立动机与同名区分。
3. 信息边界、私密知识与世界书使用。
4. 场景推进、冲突、未决控制。
5. 文风自然度，少总结腔/模板句/重复。
6. 长线伏笔种植、跨 epoch 回查、阶段性回收。
7. 变量、任务、知识和 Chronicle 与正文融合。

报告均值、中位数、分位数、每轮配对差、8 个 phase 的趋势和 grader 分歧；不得隐藏低分、失败或重试样本。

## 9. Harness 验收

每条 arm 必须：

- 80/80 intended turns 达到 accepted terminal state；CoverageRow planned/observed exact-set 无 missing/duplicate/extra。
- SQLite authoritative=true、json fallback=false；所有 clone probe 与主 authority 隔离。
- 4 次独立 PID resume 后无重放；call/private/public/turn-attempt-variant exact-set 一致。
- calls ≤ 2800，timeout/idle/deadline/response limits 生效，失败调用占预算。
- Native/Prompted reasoning exact-set；Disabled unexpected reasoning=0，否则该臂按事实降级。
- tool call/result correlation=100%；unknown/BadArgs/terminal 分类符合预期。
- quality report/autofix/provenance/Accept/outbox 原子一致。
- fixture/private/disabled scan、public seal、offline verify、SQLite reopen audit 全 PASS。
- run 首尾 Git commit 相同，工作树 clean。

required gate 失败时保留现场并停止该 arm。只可按声明的 per-turn retry 恢复瞬态或 quality-blocked 错误；不得删除失败证据、换 run ID 伪装续跑，或把 79/80 写成 PASS。

## 10. 执行顺序

1. 完成 G0.1–G0.10 及确定性测试，提交 harness 改动。
2. 跑 immutable prompt/capability preflight；同一模型不能满足三臂即停止。
3. 建三个完全独立 root，按 Disabled → Native → Prompted 顺序执行，记录实际时段。
4. 每臂在 6、20、40、60 后新 PID resume；每阶段审计进度但不提前解盲正文质量。
5. 每臂 80 完成后立即 seal/offline verify/private exact-set/SQLite reopen；上一臂未封存不启动下一臂。
6. 完成 240 篇正文盲评，再逐调用审阅 reasoning 格式、工具选择、CoT 注入与 Agent 作用。
7. 写 RESULT，附 public evidence 与 private trace index 的绝对路径；不复制 API key。

建议环境骨架：

```powershell
$env:STORYFORGE_EVAL_REAL_LLM = '1'
$env:STORYFORGE_EVAL_ENDURANCE_STAGE = 'long_coverage'
$env:STORYFORGE_EVAL_TARGET_TURNS = '80'
$env:STORYFORGE_EVAL_SUPPLEMENTAL_MATRIX = '1'
$env:STORYFORGE_EVAL_META_PROBE = '1'
$env:STORYFORGE_EVAL_CHARACTER_EXTRACTOR_PROBE = '1'
$env:STORYFORGE_EVAL_CACHE_PROBE = '1'
$env:STORYFORGE_EVAL_FIXTURE = 'crates/harness-real-llm/fixtures/cot_three_arm_80turn_v1.json'
$env:STORYFORGE_EVAL_MAX_CALLS = '2800'
$env:STORYFORGE_EVAL_CALL_TIMEOUT_SECS = '180'
$env:STORYFORGE_EVAL_DEADLINE_SECS = '86400'
$env:STORYFORGE_EVAL_STOP_AFTER_ACCEPTED_TURNS = '6,20,40,60'
```

API key 只从当前进程 secret 环境读取；控制器、日志、manifest 和 private trace 均不得打印 Authorization 或 `LLM_API_KEY`。

## 11. RESULT 必填内容

- Identity：Git SHA/clean、fixture/schedule SHA-256、模型/endpoint hash、sampling、provider extra keys/hash、三个 run/root/时段。
- 每臂：accepted/calls/retries/resumes/token/cache/latency；reasoning by role/phase；Prompt Module exact-set；tools by role；Agent effectiveness；world-info/regex/knowledge/variables/tasks/Chronicle/regenerate/quality；blind scores；seal/scan/audit。
- 三臂配对：Disabled vs Native、Native vs Prompted、Disabled vs Prompted，按 turn 和 phase 比较质量、工具、成本与失败率。
- 所有偏差：provider 能力、时间窗口、fallback、人工干预、未完成项和失败样本。
- 结论限制：Prompted 只有在注入 exact-set PASS 后才能讨论；reasoning 更长、工具更多不能自动算更好。

## 12. 交付物

```text
docs/workstreams/COT-THREE-ARM-80TURN-EVIDENCE-RESULT.md
<public-evidence-root>/disabled/<run-id>/...
<public-evidence-root>/native/<run-id>/...
<public-evidence-root>/prompted/<run-id>/...
<private-trace-root>/disabled/<run-id>/trace-index.json
<private-trace-root>/native/<run-id>/trace-index.json
<private-trace-root>/prompted/<run-id>/trace-index.json
```

public evidence 与脱敏 RESULT 可提交；完整 prompt/reasoning/output/tool result 只留本地 private trace，除非用户另行明确授权提交。

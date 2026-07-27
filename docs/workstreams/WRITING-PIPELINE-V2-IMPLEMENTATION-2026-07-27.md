# 写作流水线 V2 实施说明（2026-07-27）

> 状态：**V2 已收口**。主生成、整体/后缀重写、成本预检、四臂真实试跑和真实 Sequential Crew 恢复验收均已落地。
> 决策来源：`ARCHITECTURE-REVIEW-2026-07-26.md` 的写作流水线重设计，以及 2026-07-27 的产品复核。

## 1. 本次确定的产品原则

1. **一个模型尽量只做一件事**：正文写作、总结、状态抽取分开执行，避免结构化文书要求污染散文质量。
2. **显式选择优先**：用户选定的生成模式永远覆盖自动路由，并按 Campaign 记住。
3. **确定性路由**：不增加 LLM 分类调用；规则无法升档时回落到最便宜的续写档。
4. **Sequential Crew 是群像主路径**：它不是附属实验，而是三人以上复杂场面的优先执行方式。
5. **状态写回先审后落**：正文完成后先展示回合小票，用户确认选择后才 Accept；抽取失败不得静默。

## 2. 四种生成模式

| 模式 | 适用场景 | 实际执行链 | 调用量提示 | 定位 |
| --- | --- | --- | --- | --- |
| 续写 `continuation` | 日常推进、小节拍 | Writer 一次正文调用 → Summarizer → PostProcessor | 1 次正文 + 2 次廉价记账 | 默认、最低成本 |
| 对手戏 `duet` | 两名角色直接交锋或秘密分歧 | A → B → A 顺序接戏 → Editor-lite → Summarizer → PostProcessor | 3–4 次正文编排 + 2 次廉价记账 | 两人结构隔离档 |
| 顺序剧组 `sequential_crew` | 三人以上群像、复杂反应链 | Director → 按顺序逐个 Actor → Editor → Summarizer → PostProcessor | 2+N 次顺序编排 + 2 次廉价记账 | **重点群像主路径** |
| 大场面 `big_scene` | 兼容旧工作流或 API 显式指定 | Director → 并行 Subagents → Editor → Summarizer → PostProcessor | 4+N 次正文编排 + 2 次廉价记账 | 旧并行流水线兼容档 |

其中 Summarizer 与 PostProcessor 是两个独立调用和独立状态：

- Summarizer 只生成 Chronicle A（本轮纪要）；
- PostProcessor 只提取知识、变量与任务变更；
- `Disabled` 表示配置上没有运行，`Failed` 表示尝试过但失败，二者不可混同；
- 任一实际失败都会在回合小票上有明确提示，可单独重试后处理，或由用户显式降级采纳。

## 3. Sequential Crew 的信息边界

顺序剧组按确定的演员顺序逐个调用。每名 Actor 获得：

- 共享场面块；
- 自己的角色资料、议程、变量和私有知识；
- 前序 Actor 已经表演出来的公开叙述与台词。

它不会获得其他 Actor 的私有知识或 `inner_thoughts`。Actor 输出必须符合结构化 `Performance` 契约；写入共享片场记录的只有 `narrative + dialogue`。非结构化输出会被拒绝并重试，连续失败的 Actor 被跳过，已经形成的公开片场记录不会丢失。

这使后续角色可以真实“接戏”，同时把认知隔离落实为数据边界，而不是只靠提示词提醒。

## 4. 确定性路由

优先级固定为：

1. 用户显式选择；
2. 群像硬规则；
3. 对手戏评分；
4. 续写兜底。

群像硬规则命中任一项即路由到 Sequential Crew：

- 主要在场角色不少于 4 人；
- 至少 3 人，且意图包含大场面/群像强信号；
- 至少 3 人，且有不少于 2 个临近触发任务。

恰好两名主要角色时计算对手戏分数：

- 明确的两人直接交互：`+2`；
- 与本轮相关的私有知识存在分歧：`+2`；
- 议程对立：`+1`；
- 高张力意图：`+1`。

总分达到 2 进入对手戏，否则续写。`emotion_stage` 暂不作为硬规则，等真实回合证据足够后再标定。

前端提供显式模式选择、展示当前档位的预计调用量，并将选择保存在 Campaign 级本地记忆中。未传模式的 API 调用才进入上述自动路由；若自动规则建议升级到昂贵的 Sequential Crew，后端会在写入开场白或用户消息之前 fail closed，返回建议模式与 `2+N` 费用提示。调用方确认后须显式以 `generation_mode=sequential_crew` 重试，因此不会出现已经落下用户消息才询价、取消后留下半轮数据的情况。显式选择从不重复询问。

## 5. Turn Dossier（回合案卷）

本轮编译器生成确定性的 Turn Dossier，作为各执行档的共同输入边界。它包含：

- 场面和意图；
- 角色名册与本轮演员选择；
- 每名角色的资料、议程、变量和归属明确的知识；
- 任务与其他运行时上下文。

同一份知识归属数据同时服务于模式路由、Actor 输入隔离和后处理，避免三处各自推断导致口径漂移。

## 6. Accept 前回合小票

正文完成后仍处于 `AwaitingAcceptance`。用户第一次点击 Accept 时，前端不立即落库，而是打开回合小票；默认勾选全部可审项目：

- Chronicle A；
- 新增知识；
- 变量变更；
- 任务新增或变更。

用户可逐条取消。确认后，后端先按选择过滤 Prepared mutation batch，再执行原有原子 Accept；`FinalizeVariant`、实例写回等结构性 mutation 不暴露给用户取消，始终保留。

若 Summarizer 或 PostProcessor 失败：

- “重试后处理”只重跑这两个文书 Agent，不重写正文；
- 重试仍失败时，用户可以显式“降级采纳”；
- 普通 Accept 会拒绝静默带过失败，显式降级后 derivation 标记为 `Degraded`。

## 7. 兼容性与暂缓项

- `big_scene` 保留旧并行 Director/Subagents/Editor 流水线，现有调用方不被删除；当前写作栏只展示续写、对手戏、顺序剧组三种产品入口。
- Sequential Crew 已提高自动路由优先级；大场面档目前只由用户显式选择，不与 Sequential Crew 争抢自动群像路由。
- 整体 `regenerate` 已按当前产品模式重演：续写仍只调用 Writer；对手戏仍执行 A→B→A；Sequential Crew 仍按顺序接戏。重写历史在目标消息前截断，产物落为原消息的新 variant，并保留用户 hint 与 seed。
- 续写与对手戏仍只支持整体重写；Sequential Crew 支持“从选中角色起向后重演”，复用此前演员的公开场记并重跑选中角色、全部下游角色与 Editor。只有来源明确记录为 `sequential_crew` 的产物才开放该入口，旧稿或切换模式后的不匹配稿件会被前后端共同拒绝。`big_scene` 兼容档继续支持“只重 Editor / 只重某 Subagent”。对手戏从第 k 拍截断仍属暂缓项。
- 自动昂贵升档已经具备 fail-closed 成本确认，调用量提示已进入前端和后端稳定契约；基于更多真实样本调整具体阈值属于 V2.1 标定。
- 四臂真实生成与外部盲评试点已经完成；方法、结果和限制见 `BLIND-AB-PIPELINE-RESULT.md`。
- 可编辑“场景卡”、Duet 从第 k 拍截断重演、`emotion_stage` 路由标定和多意图/多 seed 质量矩阵属于 V2.1，不影响 V2 当前四档的正确性收口。

## 8. 主要代码落点

| 能力 | 位置 |
| --- | --- |
| 模式与路由领域模型 | `crates/domain/src/generation.rs` |
| Turn Dossier 编译与知识边界 | `crates/app-pipeline/src/turn_dossier.rs` |
| Sequential Crew 顺序执行 | `crates/app-pipeline/src/sequential_crew.rs` |
| Sequential Crew 后缀重演与来源校验 | `crates/app-pipeline/src/lib.rs`、`frontend/src/utils/rerollPolicy.js` |
| 四档编排 | `crates/app-pipeline/src/lib.rs` |
| 后处理职责与尝试状态 | `crates/app-agent/src/pipeline_postprocess.rs` |
| 回合小票、后处理重试、采纳过滤 | `crates/tauri-app/src/lib.rs`、`crates/tauri-app/src/turn_lifecycle.rs` |
| 模式选择与 Campaign 记忆 | `frontend/src/design/writing/ComposerBar.vue`、`frontend/src/composables/usePipeline.js` |
| 调用量产品契约 | `crates/domain/src/generation.rs`、`frontend/src/utils/generationModes.js` |
| 四臂真实评测与恢复验收 | `crates/harness-real-llm/src/blind_arm_matrix.rs`、`crates/harness-real-llm/tests/blind_arm_matrix_real_llm.rs` |
| 回合小票 UI | `frontend/src/design/writing/MessageItem.vue`、`frontend/src/stores/writing.js` |

## 9. 验证基线

V2 收口时已覆盖：

- 四种模式的序列与 Agent 调用边界；
- 当前模式整体重写、原消息 variant 落点、seed/hint 保留与目标消息历史截断；
- Sequential Crew 依赖安全的后缀重演、旧产物拒绝与兼容大场面局部重跑；
- Sequential Crew 的顺序、失败重试和私有思维不泄漏；
- 群像硬规则、对手戏评分与显式选择优先；
- 前端调用量提示、自动昂贵升档的写入前 fail-closed 确认；
- 回合小票首次拦截、逐项选择、结构 mutation 保留；
- 后处理失败重试与显式降级；
- JSON / SQLite Accept 生命周期一致性；
- 前端 store、composable、adapter 单测；
- 四臂真实产品路径生成、独立子代理 Latin-square 盲评；
- DeepSeek v4 Pro 真实 Sequential Crew 后缀恢复：前缀保持、Director 不重启、仅重放目标及下游角色；
- Rust workspace 编译和前端生产构建。

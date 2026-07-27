# 写作流水线四臂盲测计划（已执行）

> 立项：2026-07-27
> 状态：四臂真实样本、外部盲评试点和 Sequential Crew 恢复验收均已完成。结果见 `BLIND-AB-PIPELINE-RESULT.md`。

## 目标

在同一 Campaign 快照、同一用户意图和同一正文模型下比较四种产品执行档：

| 评测臂 | 产品模式 | 执行形态 |
| --- | --- | --- |
| `solo_writer` | `continuation` | 单笔者直接续写 |
| `parallel_crew` | `big_scene` | Director → 并行 Subagents → Editor |
| `sequential_crew` | `sequential_crew` | Director → Actor 逐个接戏 → Editor |
| `duet_merge` | `duet` | 场记 → A-B-A 按拍续演 → Editor-lite |

四臂均调用真实产品代码，不使用提示词替身。每轮各臂从相同初始状态独立生成，避免臂间路径依赖。

## 控制变量与样本

- 正文模型：`deepseek-v4-pro`，同一连接、采样配置与超时策略。
- fixture：守灯人沈磐、调查员闻笛等四名角色的灯塔场景，无外部角色卡依赖。
- 意图集预留四类：开场推进、冲突升级、多角色同场、约束回收。
- 本次收口先完成 `i1_opening` 四臂真实样本，用它验证四臂可运行性与评审协议；完整多意图统计留给后续模型/路由标定，不阻塞 V2 工程收口。
- 正文只写入 gitignored 的 `artifacts/blind-ab/texts/`；入库文档只记录聚合结论，不提交正文或密钥。

## 裁判协议 V2

旧协议让 DeepSeek 写作后再由 DeepSeek 自评，首轮出现 10/10 首位偏置。V2 改为：

1. 正文由 DeepSeek v4 Pro 生成；裁判由与正文模型不同的 Codex 子代理执行。
2. 子代理不继承生成过程、代码上下文或模式映射，只看到匿名 A/B/C/D 文本。
3. 四轮采用 Latin-square 呈现：每个臂恰好在 A、B、C、D 各出现一次，消除固定位置优势。
4. 裁判按角色一致性、推进、文风、约束遵守给出排名；长度本身不算优点。
5. 三个隔离子代理完成四轮评审；其中一个执行两轮，但始终不知道匿名文本对应的产品模式。

`balanced_four_arm_order` 与位置平衡测试固化了可复算顺序。Runner 默认不启用同模型自评；只有显式设置 `STORYFORGE_BLIND_SELF_JUDGE=1` 才运行诊断性自评。

## 工程验收

除成文盲评外，Sequential Crew 必须通过真实恢复不变量：

- 从角色 `k` 失败点恢复时，`0..k` 的已完成公开前缀字节不变；
- Director 不重新运行；
- 只重放角色 `k..N` 与 Editor；
- 私有思维不进入下游共享片场记录。

真实证据写入 `artifacts/blind-ab/sequential-suffix-replay-summary.json`。

## 判读边界

- 单意图试点只支持模式适用性和明显质量差异判断，不宣称总体胜率。
- `duet` 的产品定义是双角色；把它放进四角色压力题只用于验证路由边界，不能据此否定双人对手戏模式。
- 完整四意图、多 seed、双角色 Duet 专属 cohort 属于 V2.1 质量标定，不是 V2 正确性门槛。

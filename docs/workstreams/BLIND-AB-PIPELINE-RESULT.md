# 写作流水线盲测结果（2026-07-27）

> Runner：`crates/harness-real-llm/tests/blind_arm_matrix_real_llm.rs`
> 正文模型：`deepseek-v4-pro`
> 本地证据：`artifacts/blind-ab/`（gitignored，不含任何入库密钥）

## 结论

V2 工程流水线可以收口，但产品档位不能被解释成“越多 Agent 越好”：

- `continuation` 继续作为日常默认档，成本最低，旧两臂实验中质量没有显著落后。
- `sequential_crew` 保持三人以上群像的重点主路径。它的核心价值是逐角接戏和私有知识隔离，不是保证每篇都压过单笔者。
- `big_scene`（旧并行剧组）在本次四角色开场压力题中 4/4 排名第一，保留为显式的高质量、高成本兼容档。
- `duet` 只适用于双角色。它在四角色题中 4/4 最后，证明自动路由不能把四人场景交给 Duet；该结果不外推到双人对手戏。

## 第一轮：两臂自评的警示

旧实验比较 `solo_writer` 与 `parallel_crew`：四个意图的多数结果为 1 胜、1 负、2 平；solo 总墙钟 111.6 秒，parallel 425.8 秒，约慢 3.8 倍。

但十次有效裁决中，裁判 10/10 选择先呈现文本。由于正文与裁判都是 DeepSeek，这一轮只能支持“未发现稳定质量差异”和成本结论，不能支持具体胜负。该失败促成了外部盲评协议 V2。

## 第二轮：四臂外部盲评试点

DeepSeek v4 Pro 为同一四角色灯塔开场生成四臂样本。三个不继承生成上下文的 Codex 子代理完成四轮匿名评审；A/B/C/D 使用 Latin-square 轮换，每个臂在每个位置恰好出现一次。

| 轮次 | 第 1 | 第 2 | 第 3 | 第 4 |
| --- | --- | --- | --- | --- |
| 1 | parallel | solo | sequential | duet |
| 2 | parallel | sequential | solo | duet |
| 3 | parallel | sequential | solo | duet |
| 4 | parallel | solo | sequential | duet |

聚合：

- `parallel_crew`：4/4 第一；
- `duet_merge`：4/4 第四；
- `sequential_crew` 与 `solo_writer`：两两比较 2:2，平均名次均为 2.5。

这次没有重现固定首位偏置：四臂都轮换过四个呈现位置，而排名仍呈现一致的模式适用性信号。

### 限定

- 这是一个意图、一个 seed 的工程收口试点，不是总体质量排行榜。
- 并行组正文更长，虽然裁判被要求不奖励长度，仍不能完全排除长度相关偏好。
- Duet 缺少匹配其产品定义的双角色 cohort；后续标定必须单独补测。

## Sequential Crew 真实恢复验收

真实调用 DeepSeek v4 Pro 执行“从第 2 个角色失败后续写”的恢复用例，539.77 秒通过：

| 不变量 | 结果 |
| --- | --- |
| 角色数 | 4 |
| 恢复目标索引 | 1 |
| 已完成前缀保持 | `true` |
| Director 重新运行 | `false` |
| 实际重放角色 | `[1, 2, 3]` |
| 模型调用 | 5 |
| prompt / completion tokens | 6,856 / 15,439 |

这证明后缀重演不是文档约定：产品实现确实保留上游公开前缀、不重启导演，并只重做失败角色及其下游。

## GLM 兼容性探针

BigModel 的 Anthropic 兼容端点和 OpenAI coding v4 端点均完成最小请求探针；OpenAI 端点还通过 native tool call。适配层同时修复了两个兼容问题：保留 `/v4/chat/completions` 路径，以及把 `temperature/top_p` 量化到两位小数。

GLM-5.2 的完整四臂生成因上游响应过慢被终止，没有形成可比较质量数据，因此不把它记作质量失败。最终收口证据统一使用 DeepSeek v4 Pro。

## 产品决策

1. 默认续写：低成本、质量基线可靠。
2. 双角色强交互：Duet。
3. 三人以上复杂反应链：Sequential Crew，并在自动升档前明确展示预计调用量、要求用户确认。
4. 用户明确追求高成本群像成文时：可显式选择旧 `big_scene` 并行档；当前前端不把它作为常规入口。
5. 后续 V2.1 只做质量标定：多意图、多 seed、双角色 Duet cohort 与成本阈值调优，不再阻塞 V2 流水线正确性收口。

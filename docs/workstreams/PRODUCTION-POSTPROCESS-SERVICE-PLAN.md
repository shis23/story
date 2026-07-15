# ProductionPostprocessService 工作计划

> 分支：`codex/production-postprocess-service`
> 基线：`8732e21`
> 类型：生产编排收敛；不调用真实/付费模型

## 目标

抽出可由 Tauri 写作命令与 harness 共同调用的 `ProductionPostprocessService`，覆盖一次成文后的 Summarizer、PostProcessor、Attempt 同步、Chronicle A 发布、取消、迟到结果守卫和失败传播。

完成后，harness 不应再依赖 `synthetic_chronicle_fixture` 来模拟这段生产后处理；它应通过与 Tauri 相同的应用服务执行可控的确定性测试。此工作不等于重新运行 M5，也不改变生产默认参数。

## 已知边界

- 当前 harness 已通过 probe 调用共享 JSON `TurnLifecycleService`，但不执行 Tauri command 或 SQLite `accept_turn`。
- 当前 Tauri 命令仍拥有 Summarizer/PostProcessor 后台编排和候选写回；这正是本线应收敛的重复边界。
- JSON 是默认后端。SQLite 的 pre-accept 生命周期由 `codex/sqlite-preaccept-lifecycle` 单独准备；本线不得把默认后端切到 SQLite。

## 交付范围

1. 先画出当前 `start_writing` / `regenerate`、Summarizer、PostProcessor、Attempt、MutationBatch、Chronicle A 的真实调用图，并在代码注释或测试中固定服务 API 的职责。
2. 实现一个可注入依赖的应用服务（名称可为 `ProductionPostprocessService`；放置位置以不引入反向依赖为准），至少输入：活动 Turn/Attempt 身份、最终正文、质量信息、运行时快照、取消信号和持久化适配器。
3. 服务必须统一处理：
   - Summarizer 与 PostProcessor 的调用、结果校验和候选归一化；
   - `draft_hash` / `quality_report` / Attempt 状态同步；
   - Chronicle A / RoundSummary 的系统字段分配、发布和索引调度边界；
   - active Attempt、campaign/conversation scope、取消与迟到结果守卫；
   - best-effort 后处理与必须传播的存储失败之间的明确区分。
4. Tauri 写作/重 roll 命令改为薄适配器：DTO、事件和后台 task 管理留在命令层；业务状态机只在共享服务中。
5. Harness 改为直接调用该服务的测试入口，而不是镜像后处理状态机或固定 synthetic Chronicle 成功路径。

## 明确不做

- 不改 SQLite schema、cutover、`SqliteProductionRepository` 或默认 backend；只允许定义不带实现替换的持久化接口。
- 不改 M5 生产参数、`max_tokens` 默认值或调用真实模型。
- 不改 `docs/HANDOFF.md`、`docs/RELEASE-CHECKLIST.md`、历史 archive/RESULT；只新增本线 RESULT。
- 不实现 GUI/Android 验收。

## 冲突隔离

- 本线拥有 Tauri 写作命令、共享后处理服务、相关 app/pipeline/harness 接线。
- 不修改 `crates/infra-sqlite/**`；SQLite 线只提供可合并的 repository/API 基础。
- 若发现必须改 SQLite 或同名 Tauri command 才能继续，先记录 blocker，不通过复制逻辑绕过。

## TDD 与验收

先写会失败的契约测试，再实现。至少覆盖：

1. 成功路径：正文、quality、Attempt、Chronicle A、知识/变量/任务候选的身份和 scope 一致。
2. 旧 Attempt / 取消 / 迟到结果不能写回当前 Turn。
3. Summarizer 或 PostProcessor 可降级失败时，正文与 Attempt 的终态语义明确；关键存储同步失败必须向上返回错误。
4. auto-fix 后服务返回的文本与 `draft_hash` 一致。
5. harness 通过生产服务获得 Chronicle/后处理效果；不再以 synthetic fixture 冒充生产后处理完成。
6. 恢复、重复调用和故障注入不产生重复 Chronicle、Mutation 或索引任务。

最低门禁：

```powershell
cargo fmt --all -- --check
cargo test -p storyforge --lib
cargo test -p storyforge-app-pipeline --lib
cargo test -p storyforge-app-agent --lib
cargo test -p harness-real-llm --lib
cargo clippy -p storyforge -p storyforge-app-pipeline -p storyforge-app-agent -p harness-real-llm --all-targets -- -D warnings
git diff --check
```

按实际改动补充专项 test；不要把历史通过结果当成本轮结果。无真实模型授权时不得运行 ignored real-LLM 测试。

## 交付与提交

- 保持小而可审的提交（契约测试 → 服务 → Tauri/harness 接线 → 文档 RESULT）。
- 新增 `docs/workstreams/PRODUCTION-POSTPROCESS-SERVICE-RESULT.md`，列明真实共享范围、未共享项、测试原始输出和 M5 仍不可宣称的结论。
- 收口前确认 worktree 干净，不 push、不 merge main、不 rebase 其他线。

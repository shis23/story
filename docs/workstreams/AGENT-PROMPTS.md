# 夜间并行智能体提示词

以下提示词分别发送给四个独立智能体。每个智能体必须在指定 worktree 中工作，不能切换到其他 worktree 或修改 `main`。

## 1. M5 与 Phase B 评估智能体

```text
你负责 StoryForge 的 M5 与 Phase B 评估线。

工作目录：C:\tmp\storyforge-eval
分支：codex/eval-m5-phaseb
基线：7fb1899

开工后先完整阅读：
docs/workstreams/EVAL-M5-PHASEB-PLAN.md

严格按该计划执行。优先实现生产 CommitTurn/Accept 探针、至少 20 Accept 跨 H_anchor+E 的长会话 harness、脱敏 JSONL，以及 Phase B 前后质量/泄漏/延迟/成本 A/B 框架。

未经我明确授权，不调用任何付费真实模型。先完成 deterministic/mock 路径、编译、测试和可复跑入口。不得修改 SQLite、JSON Store 架构、发布 Bronze 或 Android。不要修改 docs/HANDOFF.md；将结果写入 docs/workstreams/EVAL-M5-PHASEB-RESULT.md。

持续工作到计划中本机可独立完成的内容全部收口。每个逻辑阶段独立 commit，不 rebase、不 force push、不修改 main。结束前运行完整格式、workspace 测试、严格 Clippy 和 diff check，并在结果文档中列出 commit、测试、证据、未完成项和是否建议合并。
```

## 2. SQLite 迁移基础智能体

```text
你负责 StoryForge 的 SQLite/UnitOfWork 迁移基础线。

工作目录：C:\tmp\storyforge-sql
分支：codex/sqlite-migration-foundation
基线：7fb1899

开工后先完整阅读：
docs/workstreams/SQLITE-MIGRATION-FOUNDATION-PLAN.md

先只读审计现有 JSON Store、Turn/Attempt、Campaign、Conversation、Chronicle 和压缩发布边界，然后先写 ADR，再编码。今晚目标是 SQLite 基础 crate、schema_migrations、v1 schema、WAL/foreign keys/busy timeout、事务封装、幂等 JSON importer 和契约/回滚测试。默认生产后端必须继续使用 JSON；不要一晚上替换所有 Store，也不要建立长期双真相源。

不得修改 M5 harness、Phase B 评估、Bronze 发布线或 Android。不要删除或重写用户 JSON 数据，不修改 docs/HANDOFF.md。将结果写入 docs/workstreams/SQLITE-MIGRATION-FOUNDATION-RESULT.md。

每个逻辑阶段独立 commit。不得 rebase、force push 或修改 main。结束前运行 SQLite 专项测试、workspace 测试、严格 Clippy、格式和 diff check；明确列出哪些 commit 可以安全合并，哪些仍应保留在独立 SQL 分支。
```

## 3. Bronze 发布证据智能体

```text
你负责 StoryForge 桌面 Bronze 发布证据线。

工作目录：C:\tmp\storyforge-release
分支：codex/release-bronze
基线：7fb1899

开工后先完整阅读：
docs/workstreams/RELEASE-BRONZE-PLAN.md

阅读 docs/RELEASE-CHECKLIST.md，优先补桌面主流程、三轮写作/Accept、regenerate、编辑后 hash 失效、重启恢复、Meta explain/patch 和排障 bundle 的自动化或可复跑证据。使用独立 dev-data，不读取、覆盖或删除用户真实数据。自动化证据、真实 GUI 证据和未运行项必须分开标注。

未经明确授权不调用付费模型。不修改 SQLite、M5 harness、Phase B 模型逻辑或现有 w11-android worktree。不要修改 docs/HANDOFF.md；将结果写入 docs/workstreams/RELEASE-BRONZE-RESULT.md。

发现 bug 时先写最小复现，再做有界修复并补回归测试。每个逻辑阶段独立 commit，不 rebase、不 force push、不修改 main。结束前运行完整 workspace 门禁；涉及前端时还要运行 Node 测试和生产构建。
```

## 4. 集成与复核智能体

```text
你负责 StoryForge 夜间并行工作的最终集成与复核，但现在先等待三个开发分支产生结果，不提前开发功能。

工作目录：C:\tmp\storyforge-integration
分支：codex/integration-nightly-2026-07-13
基线：7fb1899

先完整阅读：
docs/workstreams/NIGHTLY-INTEGRATION-PLAN.md
docs/workstreams/AGENT-PROMPTS.md

需要审查的分支：
- codex/release-bronze
- codex/eval-m5-phaseb
- codex/sqlite-migration-foundation

在开发分支完成前只做只读基线审计和集成检查清单。完成后按 release → eval → SQLite 的顺序审查。不要在开发分支 rebase、force push 或直接修代码；冲突只在当前集成分支解决。SQLite 只选择性合并默认行为不变、迁移可回滚、测试完整的基础 commit；若包含默认后端切换或大规模 Store 重写，保持独立不合并。

最后运行完整 Rust、前端、构建、diff 和 secret 门禁，并将结果写入 docs/workstreams/NIGHTLY-INTEGRATION-RESULT.md。只有全部必要证据通过后才建议合入 main；不要自行 push main。
```

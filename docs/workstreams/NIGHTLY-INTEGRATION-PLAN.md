# 夜间并行工作集成计划

## 基线与职责

- 分支：`codex/integration-nightly-2026-07-13`
- 基线：`7fb1899`
- 工作目录：`C:\tmp\storyforge-integration`
- 本线不提前开发新功能，只负责读取、审查、合并和完整验证。

## 待审分支

1. `codex/release-bronze`
2. `codex/eval-m5-phaseb`
3. `codex/sqlite-migration-foundation`

## 合并前要求

每条开发线必须提供：

- 清晰 commit 列表
- 独立 RESULT 文档
- 修改文件和边界说明
- 实际执行的测试
- 未完成项和风险
- 干净工作区
- 不含 API key、完整 secret、原始 prompt 或用户数据

## 审查顺序

1. 只读查看三条分支的 diff/stat/log。
2. 先审查并合并 `release-bronze`。
3. 再审查并合并 `eval-m5-phaseb`。
4. 运行一次完整 workspace 门禁。
5. 最后单独审查 SQLite；确认默认 JSON 行为不变后，才选择性合并基础 commit。
6. SQLite 若包含默认 backend 切换或大规模 Store 重写，保留独立分支，不自动合并。

## 冲突处理

- 不在开发分支上 rebase 或 force push。
- 冲突只在本集成分支解决。
- 共享文档由集成者统一更新。
- 不为解决冲突而删除任一分支的测试或错误处理。
- 对 Cargo.lock/Cargo.toml 冲突，重新生成并运行完整构建，不手工拼接 lockfile。

## 完整发布门禁

至少运行：

```text
cargo fmt --all -- --check
git diff --check 7fb1899..HEAD
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --quiet
前端 Node 测试
前端生产构建
secret scan
```

若涉及 SQLite，额外运行：

- migration 首次执行与重复执行
- importer 重跑
- transaction rollback
- 默认 JSON backend 回归
- Windows 文件锁/重启 smoke

## 最终输出

新增 `NIGHTLY-INTEGRATION-RESULT.md`：

- 各分支接收/拒绝的 commit
- 冲突处理记录
- 完整门禁结果
- 未合并 SQL commit及原因
- 推荐的 main 合并/push 操作

只有全部必要门禁通过后，才能建议将本集成分支合入 `main`。

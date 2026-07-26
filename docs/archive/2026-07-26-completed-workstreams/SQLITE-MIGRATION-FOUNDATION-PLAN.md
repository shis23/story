# SQLite 迁移基础线计划

## 基线与边界

- 分支：`codex/sqlite-migration-foundation`
- 基线：`7fb1899`
- 工作目录：`C:\tmp\storyforge-sql`
- 本线建立 SQLite/UnitOfWork 基础，不在今晚强制替换全部 JSON Store。
- 默认生产后端保持 JSON，除非迁移、回滚和契约测试全部完成并经过集成审查。
- 不修改 M5 harness、Phase B 评估或桌面发布证据。

## 首要设计决策

开始编码前先写 ADR，拍板：

1. SQLite crate 和依赖选择，以及 bundled/native 的发布影响。
2. 数据库文件位置、schema version 和 migration runner。
3. `WAL`、`foreign_keys=ON`、busy timeout、同步级别和备份策略。
4. Campaign、Conversation、Turn/Attempt、Chronicle 的事务边界。
5. JSON 到 SQLite 的一次性迁移与失败回滚策略。
6. 后端选择方式：显式配置或 feature flag，禁止双真相源。
7. Android/Windows 文件锁、路径和 SQLite native 兼容风险。

## 今晚合理目标

1. 新增 SQLite 基础 crate 和最小连接管理。
2. 建立 `schema_migrations` 与 v1 schema。
3. 实现事务封装/UnitOfWork 雏形。
4. 实现幂等 JSON importer 的只读解析与数据库写入。
5. 增加重复导入、事务回滚、外键、损坏输入和 schema upgrade 测试。
6. 建立 Store contract 测试；最多选择一个边界清晰的 Store 做适配验证。
7. 默认 StoryForge 行为不得改变。

## Schema 原则

- 数据库主键使用稳定 ID，不用显示 code 作为主键。
- Turn、Attempt、Mutation/Publication 必须能处于同一事务。
- Chronicle A/B/C 保留 lineage、source ids、covers 和 revision。
- JSON blob 只用于尚未稳定的扩展字段，核心查询字段应结构化。
- 所有 migration 必须单向、编号、幂等检测并在事务内执行。
- importer 必须记录 source hash 和完成标记，崩溃后可安全重试。

## 禁止事项

- 禁止 JSON 与 SQLite 长期双写并同时作为真相源。
- 禁止逐个 Store 自动提交而破坏跨域原子性。
- 禁止一晚内机械翻译所有 JSON 文件后直接切默认后端。
- 禁止删除、移动或重写用户现有 JSON 数据。
- 禁止修改 `docs/HANDOFF.md`；结论写入本目录。
- 禁止在缺少迁移回滚测试时合并到 main。

## 验收门槛

- `cargo fmt --all -- --check`
- SQLite crate 全部测试
- `cargo test --workspace --quiet`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `git diff --check 7fb1899..HEAD`
- importer 连续运行两次结果一致
- migration 中途失败不会留下半升级 schema
- 默认 JSON 路径的现有测试完全不变

## 合并策略

- 允许合并：ADR、未启用的基础 crate、migration runner、schema、独立 importer 与测试。
- 暂缓合并：默认后端切换、大规模 Store 重写、长期双写、未验证 Android native 依赖。
- SQL 分支必须最后进入集成线，并单独复核。

## 最终交付

结束时新增 `SQLITE-MIGRATION-FOUNDATION-RESULT.md`，记录：

- schema/ADR 摘要
- commit 列表和数据库依赖变化
- importer/migration 测试证据
- 当前哪些 Store 已适配、哪些仍为 JSON
- 数据丢失、回滚、Android 和发布风险
- 建议合并范围

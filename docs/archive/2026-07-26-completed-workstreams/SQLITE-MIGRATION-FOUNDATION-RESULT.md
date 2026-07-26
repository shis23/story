# SQLite 迁移基础线结果

- 分支：`codex/sqlite-migration-foundation`
- 基线：`7fb1899`
- 工作目录：`C:\tmp\storyforge-sql`
- 日期：2026-07-13

## schema / ADR 摘要

ADR：`docs/adr/0001-sqlite-migration-foundation.md`

关键决策：

1. 新增未启用 crate `storyforge-infra-sqlite`，依赖 `rusqlite` + `bundled`（避免系统 SQLite / Android ABI 差异）。
2. DB 路径约定：`<app_data_dir>/storyforge.sqlite3`；`schema_migrations(version, name, applied_at, checksum)`。
3. 固定 PRAGMA：`WAL`、`foreign_keys=ON`、`busy_timeout=5000`、`synchronous=NORMAL`、`temp_store=MEMORY`。
4. `UnitOfWork` 封装 immediate transaction；Drop 未 commit 则 rollback。Turn accept / Chronicle publication / Campaign fork 设计为同事务域（本线只提供原语）。
5. JSON importer 只读源文件，记录 `import_runs` + `source_manifest_hash`，整单事务、可重试、完成后重复导入 no-op。
6. 默认生产后端仍为 JSON；禁止 JSON/SQLite 双真相源。
7. Android/Windows 风险：bundled、单实例、文件锁与发布体积需后续集成验证。

v1 schema 覆盖：

- `character_cards` / `campaigns` / `character_instances` / `character_knowledge` / `story_tasks`
- `conversations` / `turns` / `turn_attempts`
- `round_summaries` + `round_summary_covers`（lineage/level/covers/covered_by）
- `import_runs`
- 主键均为稳定 Id；扩展字段进 `payload_json`

## Commit 列表与依赖变化

相对基线 `7fb1899`：

| Commit | 说明 |
| --- | --- |
| `a37206e` | docs(workstream): plan SQLite migration foundation |
| `0d49e08` | docs(adr): SQLite migration foundation decisions |
| `48c5f29` | feat(infra-sqlite): SQLite foundation crate with migrations and importer |
| （本文件） | docs(workstream): SQLite migration foundation result |

数据库依赖变化：

- workspace member：`crates/infra-sqlite`
- workspace path dep：`storyforge-infra-sqlite`
- 直接依赖：`rusqlite 0.32`（`bundled`）→ 引入 `libsqlite3-sys` 等
- dev：`tempfile`
- **未**把 `storyforge-infra-sqlite` 接入 `tauri-app` / 默认启动路径

修改文件（相对 `7fb1899`，不含本 RESULT 提交前）：

- `Cargo.toml` / `Cargo.lock`
- `docs/adr/0001-sqlite-migration-foundation.md`
- `docs/workstreams/SQLITE-MIGRATION-FOUNDATION-PLAN.md`
- `crates/infra-sqlite/**`

## importer / migration 测试证据

`cargo test -p storyforge-infra-sqlite`：**16 passed**

覆盖：

- 连接 PRAGMA / 父目录创建
- UoW commit / rollback / drop-rollback
- migration 幂等、checksum mismatch、失败 migration 不留半 schema、FK 强制
- importer 连续两次一致（第二次 `SkippedDuplicate`）
- 损坏 JSON 拒绝且无业务半导入
- 中途 FK 失败事务回滚
- payload roundtrip
- Campaign contract save/get
- 同事务 turn + campaign revision bump
- importer 后再 contract 读取稳定

## 门禁结果

| 门禁 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test -p storyforge-infra-sqlite` | 16 passed |
| `cargo test --workspace --quiet` | 通过（真实 LLM 测试仍为 ignore） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `git diff --check 7fb1899..HEAD` | 通过 |
| importer 连续两次一致 | 测试 `import_is_idempotent_across_two_runs` 覆盖 |
| migration 中途失败不半升级 | 测试 `failed_migration_does_not_leave_half_schema` 覆盖 |
| 默认 JSON 路径测试不变 | workspace 全量通过；未改 JSON Store 实现 |

说明：workspace 测试需存在 `frontend/dist`（Tauri `generate_context!`）。本机以 `frontend && npm ci && npm run build` 生成；`frontend/dist` / `node_modules` 被 gitignore，未纳入提交。

## Store 适配状态

| Store / 域 | 状态 |
| --- | --- |
| SQLite 连接 / migration / UoW | 已实现（未启用） |
| JSON importer（cards/campaigns/instances/knowledge/tasks/summaries/conversations/turns） | 已实现（工具/测试用） |
| Campaign contract（最小 save/get） | SQLite 侧验证 |
| `CampaignStore` / `ConversationStore` / `TurnStore` 生产路径 | **仍为 JSON** |
| Connection / Preset / Module / Vector / 其他配置 Store | **仍为 JSON** |
| 默认后端切换 | **未做** |

## 风险

1. **数据丢失**：importer 不改 JSON；但未来切换后端前若未备份/校验，仍有切换风险。
2. **回滚**：本线未实现“SQLite → JSON 导出回滚工具”；失败 import 依赖事务回滚；生产切换前必须补双向迁移与备份。
3. **Android**：bundled 已选，但未在 Android 目标编译/真机验证文件锁与路径。
4. **发布**：二进制将在真正链接本 crate 后增大；当前默认 app 未链接。
5. **schema 完备性**：variables/provenance/mutation batch 等多在 `payload_json`；查询与部分更新能力后续增强。
6. **跨域 accept 接线**：UoW 原语已有，生产 accept 仍走 JSON 多文件写，原子性债务仍在。

## 建议合并范围

**建议合并本基础线到集成线（非直接默认切换）：**

- ADR
- 未启用 `storyforge-infra-sqlite`
- migration runner + v1 schema
- 独立 importer + 测试
- 本 RESULT

**暂缓 / 不得作为本 PR 目标：**

- 默认 `storage.backend=sqlite`
- 大规模 Store 重写或 JSON/SQLite 双写
- 删除/搬迁用户 JSON
- 未验证的 Android native 发布结论

SQL 分支应最后进入集成线并单独复核。

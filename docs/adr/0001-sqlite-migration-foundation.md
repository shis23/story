# ADR 0001: SQLite 迁移基础

- 状态：Accepted
- 日期：2026-07-13
- 分支：`codex/sqlite-migration-foundation`
- 基线：`7fb1899`

## 背景

StoryForge 当前以多个 JSON 文件作为真相源：

- `data/campaigns.json` / `cards.json` / `instances.json` / `knowledge.json` / `tasks.json` / `round_summaries.json`
- `data/turns.json`
- `data/conversations/<id>.json`
- 以及其他配置与辅助文件

跨域提交（Conversation 变体 + Turn/Attempt + Campaign revision + Chronicle 发布）无法在 JSON 文件层提供真正事务。本 ADR 只建立 SQLite 基础，不切换默认生产后端。

## 决策

### 1. Crate 与依赖

- 新增 workspace crate：`storyforge-infra-sqlite`（路径 `crates/infra-sqlite`）。
- 使用 `rusqlite`，启用 `bundled` feature：
  - Windows / Android / 桌面发布不依赖系统 SQLite。
  - 二进制体积增加可接受；原生动态库路径差异是更大的发布风险。
- 不在本阶段引入 `sqlx` / `diesel` / 异步连接池。当前 store 是同步 `Mutex` + 文件 IO 模型，同步 `rusqlite` 最贴合。
- 依赖 `serde` / `serde_json` / `thiserror` / `sha2` / `chrono` / `storyforge-domain`。

### 2. 数据库位置、版本与 migration runner

- 默认文件：`<app_data_dir>/storyforge.sqlite3`。
- 旁路文件：WAL/SHM 由 SQLite 自动管理；备份目标为完整 checkpoint 后的主库拷贝。
- `schema_migrations` 表记录：
  - `version INTEGER PRIMARY KEY`
  - `name TEXT NOT NULL`
  - `applied_at TEXT NOT NULL`
  - `checksum TEXT NOT NULL`
- migration 资源以 crate 内编号 SQL 字符串维护（`migrations/V00x__name.sql` 风格模块）。
- runner 规则：
  - 按 version 升序。
  - 已应用 version 校验 checksum；不一致直接失败。
  - 每个 migration 在单个事务内执行；失败整单回滚，不留下半升级 schema。
  - 只允许单向 forward migration。

### 3. 连接与 PRAGMA

打开数据库后固定：

| PRAGMA | 值 | 原因 |
| --- | --- | --- |
| `journal_mode` | `WAL` | 读写并发与崩溃恢复 |
| `foreign_keys` | `ON` | 跨表引用必须强制 |
| `busy_timeout` | `5000` ms | Windows/Android 文件锁短暂争用 |
| `synchronous` | `NORMAL` | WAL 下的性能/安全折中 |
| `temp_store` | `MEMORY` | 临时表/排序减少磁盘抖动 |

备份策略（基础阶段约定，未实现 UI）：

1. `PRAGMA wal_checkpoint(TRUNCATE)`
2. 复制 `storyforge.sqlite3` 到 `storyforge.sqlite3.bak-<timestamp>`
3. 迁移前必须先完成备份；失败则中止 importer

### 4. 事务边界 / UnitOfWork

`UnitOfWork` 封装 `rusqlite::Transaction`：

- 显式 `commit` / `rollback`（Drop 时未 commit 则 rollback）。
- 应用层禁止在 UoW 外“逐 store 自动提交”拼跨域原子写。

必须能进入同一事务的域：

1. **Turn accept**：Conversation variant Final + Turn/Attempt 状态 + MutationBatch 应用 + Campaign.revision
2. **Chronicle publication**：RoundSummary covers/covered_by + Campaign.chronicle_revision + pending_compress_publication 清理
3. **Campaign fork**：新 Campaign + instances 快照 + 新 Conversation 拷贝

v1 schema 以稳定 ID 主键支撑上述边界；本线只提供事务原语，不接线生产 accept 路径。

### 5. JSON → SQLite 一次性迁移

- importer 只读 JSON，**不删除、不移动、不改写**用户现有 JSON。
- 记录 `import_runs`：
  - `run_id`
  - `source_root`
  - `source_manifest_hash`（参与导入文件的规范化哈希）
  - `status`：`running` / `completed` / `failed`
  - `started_at` / `finished_at` / `error`
- 幂等：
  - 同一 `source_manifest_hash` 且 `completed` → 直接 no-op 成功。
  - 崩溃后 `running`/`failed` 可安全重试；写入使用 `INSERT OR REPLACE` / upsert，以稳定 ID 为键。
- 失败回滚：整次 import 包在事务中；失败不标记 completed，库内容回滚到 import 前。

### 6. 后端选择：禁止双真相源

- 默认生产后端保持 **JSON**。
- 选择方式：显式配置 `storage.backend = "json" | "sqlite"`（后续接入）；本线 crate 可被测试直接调用，**不**改默认启动路径。
- 禁止 JSON 与 SQLite 长期双写同时作为真相源。
- 切换条件（非本线完成项）：迁移 + 回滚 + 契约测试 + 集成审查全部通过。

### 7. Android / Windows 风险

| 风险 | 处理 |
| --- | --- |
| Android 无系统 SQLite 稳定 ABI | `bundled` 编译进 so |
| 文件锁 / 杀进程 | WAL + busy_timeout；单写者连接策略 |
| 路径空格/权限 | 使用 app data dir 绝对路径；打开前 `create_dir_all` |
| 备份与多进程 | v1 假设单 app 实例；多实例不支持 |
| 发布体积 | bundled SQLite 可接受；需在 Android 集成线单独验证 |

## Schema 原则（v1）

- 主键：稳定 `Id` 字符串，不用显示 code。
- Chronicle：结构化 `lineage_id` / `level` / `turn` / `turn_end` / `covered_by`；`covers` 用关联表。
- 尚未稳定的扩展字段进 `payload_json`（如 provenance、variables 全量、mutation batch）。
- 核心查询字段列化：campaign revision、turn status、conversation_id 等。

## 非目标（本线）

- 不切换默认后端
- 不机械翻译全部 JSON store
- 不改 M5 harness / Phase B 评估 / 桌面发布证据
- 不修改 `docs/HANDOFF.md`

## 后果

### 正向

- 后续 Store 可按边界逐个适配并共享 UoW
- 迁移与 schema 升级有测试护栏
- 默认 JSON 行为保持不变，合并风险可控

### 负向 / 后续债

- 在默认切换前，SQLite 路径仅测试与显式工具使用
- Android native/bundled 需集成线验证
- 部分领域仍以 JSON blob 存储，查询能力分阶段增强

## 允许合并范围

- ADR、未启用 crate、migration runner、v1 schema、独立 importer、测试
- **暂缓**：默认后端切换、大规模 Store 重写、长期双写、未验证 Android 依赖

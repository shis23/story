# 后端架构拆分与 SQLite 彻底收口计划（2026-07-28）

> 状态：**Gate 0 已完成；Gate 1 第一至第三批、世界书/变量、MVU runtime、Meta Agent、typed patch/MVU、Campaign、P2 记忆和 Turn 子批完成，后续批次待执行**。本文件只建立执行顺序、边界和验收门槛，不代表后续阶段已经完成。
> 起草基线：`main@2832030`。
> 主目标：先消除 `tauri-app/src/lib.rs` 巨石和双后端业务分叉，再补齐 SQLite 能力、完成迁移演练并切换默认后端。
> 结果文档：执行时新建 `docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md`，逐阶段记录真实证据。

## 1. 当前事实

本计划只采用当前代码事实，不把旧审计中的已修问题重新列为待办。

### 1.1 已完成

- 写作流水线 V2 产品行为已经收口：四档生成、Sequential Crew、成本预检、按模式重写和 Accept 前回合小票均已落地。
- `ProductionPostprocessService` 已抽出，Tauri 与真实模型 harness 共用同一后处理应用服务；本计划不重复实现它。
- SQLite 已作为显式 opt-in 生产后端接入：
  - JSON → SQLite fail-closed cutover、marker、锁、备份与启动恢复；
  - draft、autofix、postprocess、regenerate、edit-stale；
  - Accept UoW、revision CAS、active-turn barrier 和 recovery；
  - Chronicle publication UoW；
  - SQLite → JSON reverse export；
  - MVU 翻译数据迁移、存储和查询。
- V1/V2 数据丢失、V4 原子写、V5 应用 CSP、V7 错误分类与重复 Accept 等已核实缺陷已经修复。
- Linux Gitea runner 已投入运行；Windows runner、Android 真机和签名发布不在“已完成”范围内。

### 1.2 尚未收口

以起草基线实测：

- `crates/tauri-app/src/lib.rs`：约 **16,365 行**（Gate 1 Turn 子批后的实时基线）。
- `#[tauri::command]`：**175 个**，由 `src/**/*.rs` 基线脚本统一统计；注册命令也是 175 个。
- `is_sqlite_active()`：**68 处**，以 Gate 0/1 基线脚本对整个 `src/**/*.rs` 实时计算为准。
- JSON 与 SQLite 的部分业务决策仍在命令层分别实现。
- SQLite 模式仍明确不支持部分 Meta typed patch、legacy Meta patch 与 MVU schema apply。
- SQLite 模式会跳过 Chronicle compressor worker 的入队和启动恢复。
- 默认存储仍是 JSON。
- SQLite Native12、TextFallback、Full100 尚无可声明为 PASS 的正式封存证据。

## 2. 最终完成定义

全部满足后，才允许把本专项标记为完成：

1. `tauri-app/src/lib.rs` 只保留 bootstrap、`AppState` 组装、命令注册和少量跨域接线；命令实现与 DTO 按域拆入模块。
2. 命令处理器不再直接判断 JSON/SQLite；backend selector 只在启动和单一 backend facade 内解析。
3. JSON 与 SQLite 共用同一套 Turn/Attempt/Accept、postprocess mutation、typed patch 和错误分类业务规则。
4. SQLite 覆盖桌面产品可见的 Campaign、Turn、Conversation、Meta、MVU、Chronicle compressor、变量、知识、任务和世界书写路径。
5. SQLite 模式不存在静默 JSON fallback、双写或第二权威数据源。
6. 旧 JSON 数据可重复迁移到 SQLite；迁移失败不覆盖旧数据；反向导出不静默丢字段。
7. SQLite 确定性等价测试、故障注入、重启恢复、并发/锁和真实模型证据均有可复核结果。
8. Windows 与 Android 的真实文件系统/生命周期验证通过后，默认后端切换为 SQLite。
9. JSON 仅作为限期兼容导入、反向导出和紧急回退能力保留，不再承载默认生产写入。
10. README、ARCHITECTURE、HANDOFF、ROADMAP 和 RELEASE-CHECKLIST 的口径与代码一致。

## 3. 硬约束

整个专项必须遵守：

- 不在机械拆分阶段顺手改行为。
- 不改变现有 Tauri command 名称、参数、返回 DTO、事件名和前端 IPC 合同，除非有独立迁移计划。
- 不以“测试能编译”代替运行通过。
- 不削弱已有错误检查、权限检查、active-turn barrier 或 fail-closed 语义。
- 不为追求统一而把 SQLite 错误降级为字符串匹配。
- 不在 SQLite 活跃时读取或写入 legacy JSON authority。
- 不通过双写制造“看起来兼容”的过渡状态。
- 不删除 backup、reverse export、marker-last 和启动恢复。
- 不修改已有迁移文件；新增 schema 只能增加新版本迁移。
- 不把 deterministic、harness、Chromium 或桌面测试写成 Android/真机 PASS。
- 真实模型调用必须另行确认成本；API key 不得写入仓库、日志或证据文件。
- 每一阶段独立提交、独立验证；失败时停在当前 Gate，不把半完成阶段混入下一阶段。

## 4. 总体顺序

```text
Gate 0 事实基线
  ↓
Gate 1 lib.rs 机械拆分
  ↓
Gate 2 业务状态机与重复编排收敛
  ↓
Gate 3 单一 backend facade
  ↓
Gate 4 SQLite 能力补齐
  ↓
Gate 5 数据迁移、等价与恢复
  ↓
Gate 6 真实证据与平台验收
  ↓
Gate 7 默认切换与兼容退场
  ↓
Gate 8 文档、发布与最终封存
```

Gate 1–3 是架构前置条件。不得先在 68 个分支点继续堆 SQLite 特例，再回头抽象。

## 5. Gate 0：事实基线与保护网

### 5.1 代码清单

生成并写入 RESULT：

- 当前 HEAD、worktree 状态和远端同步状态；
- workspace crate 清单；
- `lib.rs` 行数、命令数、inline test 行数；
- 所有 `is_sqlite_active()` 调用点；
- 所有 SQLite `unsupported`、JSON-only 和 skip 分支；
- `generate_handler!` 命令注册清单；
- JSON/SQLite 能力矩阵；
- 当前 schema migration 版本和表清单。

这些数字由命令生成，不再手工复制进多个权威文档。

### 5.2 契约测试

机械拆分前补齐：

- Tauri command 名称与注册集合快照测试；
- 前端 `tauri-api.js` 调用名与后端命令合同测试；
- Pipeline event 集合和关键顺序合同；
- JSON/SQLite 错误分类快照；
- Accept 幂等重放、revision conflict、scope mismatch 和 integrity failure 对照；
- SQLite 活跃时禁止访问 JSON store 的负向测试。

### 5.3 Gate 0 通过条件

- 工作树干净。
- 完整确定性发布门禁通过。
- 基线命令清单可重复生成。
- 当前已知 unsupported/skip 均进入能力矩阵，没有“grep 才知道”的隐藏分支。

## 6. Gate 1：`lib.rs` 机械拆分

### 6.1 目标布局

允许根据现有依赖微调，但推荐先形成：

```text
crates/tauri-app/src/
  lib.rs
  bootstrap.rs
  app_state.rs
  commands/
    mod.rs
    writing.rs
    turns.rs
    campaigns.rs
    characters.rs
    cards.rs
    world_info.rs
    variables.rs
    meta.rs
    mvu.rs
    connections.rs
    presets.rs
    plugins.rs
    card_shell.rs
    import_export.rs
    diagnostics.rs
```

现有 `production_postprocess.rs`、`turn_lifecycle.rs`、`sqlite_runtime.rs`、
`campaign_store.rs` 等先保持原位。Gate 1 只移动代码，不重新设计依赖。

### 6.2 拆分批次

按低耦合到高耦合执行：

1. diagnostics、presets、connections（已完成，见 Gate 1 第一批提交）；
2. import/export、cards、characters（已完成，见 Gate 1 第二批提交）；
3. plugins、card-shell（已完成，见 Gate 1 第三批提交）；
4. world-info、variables、MVU runtime、Meta Agent、typed patch/MVU、Campaign、P2 记忆、Turn（已完成，拆成独立子批）；writing 待执行；
5. campaigns、turns；
6. writing 与 pipeline command；
7. inline tests 跟随被测域迁移。

每批必须：

- 使用纯移动或最小可见性修改；
- 保持命令注册顺序和前端合同；
- 不进行跨批格式化；
- 单独提交；
- 运行对应域测试及完整 Rust 编译门。

### 6.3 Gate 1 通过条件

- `lib.rs` 目标不超过约 2,500 行；若因 bootstrap 必需内容超过，RESULT 需逐项解释。
- `lib.rs` 不再包含具体业务命令实现。
- 175 个命令均且仅注册一次。
- 前端无需修改 IPC 调用名。
- 全量测试结果与 Gate 0 等价。

## 7. Gate 2：业务状态机与重复编排收敛

### 7.1 Turn/Attempt/Accept

在已有 V7 修复基础上完成真正的单状态机：

- 抽取 backend-agnostic 的 Accept decision service；
- JSON/SQLite 共同调用：
  - quality gate；
  - duplicate Accept 幂等语义；
  - draft hash 检查；
  - Attempt 选择；
  - revision CAS；
  - MutationBatch 校验；
  - terminal/replay 语义；
  - typed error taxonomy。
- repository 只负责事务读取、CAS 和写入，不重新判断产品策略。

### 7.2 写作与 regenerate

- 把 Director、Subagent、Editor 阶段抽成可复用 stage functions。
- `start_writing` 与 `regenerate` 共享同一阶段实现，只通过请求参数描述差异。
- 统一取消、失败、重试和事件发射；消除双发、漏发和“失败被标成取消”。
- 三套近似 tool loop 合并为一个可配置执行器。
- 保留 Sequential Crew 后缀重演的既有来源校验和前缀不变合同。

### 7.3 Postprocess mutation

`ProductionPostprocessService` 已存在，本阶段只收敛其内部策略基底：

- JSON 路径也先组装 `CampaignRuntimeContext`；
- JSON 与 SQLite 共用同一个 mutation 构建器；
- 角色解析、同名冲突、传播策略、在场豁免、临时角色和变量归属只实现一次；
- 后端差异只存在于 MutationBatch 如何事务落盘。

### 7.4 Typed patch

- Preview 和 Accept 共同调用 `app-meta` 的纯函数得到目标快照或 diff。
- 禁止 Preview 一套逻辑、真正 Apply 另一套逻辑。
- persistence adapter 只保存已经验证的 diff。

### 7.5 Gate 2 通过条件

- JSON/SQLite Accept 对照测试使用同一输入得到同一领域结果和同类错误。
- start/regenerate 不再复制完整阶段实现。
- tool loop 只有一个权威实现。
- postprocess mutation 规则只有一个权威实现。
- typed patch preview 与 apply 使用同一纯函数。

## 8. Gate 3：单一 backend facade

### 8.1 目标

进程启动时解析一次 backend，构造一个显式 facade/port 集合。命令层只能依赖该
facade，不能读取全局 backend flag。

建议覆盖：

- Campaign、Card、CharacterInstance；
- Conversation；
- Turn/Attempt/pre-accept/Accept；
- Knowledge、Variables、Tasks、WorldInfo；
- Meta patch；
- MVU translation/schema apply；
- Chronicle summaries、compress jobs、publication；
- active campaign 和 `story_clock`。

可使用 trait 或启动时选择的 enum-dispatched struct。优先清晰、可测试，不为抽象
而引入动态分发。

### 8.2 AppState 与全局 store

- backend 和数据目录进入 `AppState`，不再依赖多个 ambient `OnceLock` 决定权威。
- 同一 store 类型不得针对同一目录构造两份写入者。
- harness 通过应用层 port/service 测试，不应为了少量纯逻辑长期链接整个 Tauri/wry 栈。
- 是否新增 app-layer crate 在本 Gate 做依赖评审后决定；不得在 Gate 1 机械拆分时提前创建。

### 8.3 能力声明

facade 必须能显式报告：

- supported；
- degraded；
- unsupported；
- migration required；
- read-only recovery。

用户可见核心能力在最终 Gate 前不得保留 unsupported；非核心兼容能力必须有明确
产品提示，不能静默空列表。

### 8.4 Gate 3 通过条件

- Tauri command 和应用服务内 `is_sqlite_active()` 为 0。
- backend flag 只存在于 bootstrap/facade 构造和专用测试。
- SQLite 活跃时 JSON 写入者没有构造机会。
- 新增命令无需自行添加 JSON/SQLite 分支。

## 9. Gate 4：SQLite 缺口补齐

### 9.1 Meta UoW

补齐 SQLite-native：

- campaign repair proposal；
- typed patch preview/accept/dismiss；
- legacy Meta patch 若仍保留，则迁到同一 typed diff 模型；否则先迁移调用方再删除；
- active-turn barrier；
- stale/revision/scope 检查；
- 故障注入回滚。

### 9.2 MVU

已有 translation CRUD 基础上补齐：

- schema apply preview；
- schema apply；
- definition 更新；
- 已有 instances 默认值 backfill；
- 跨 campaign 引用和同源卡校验；
- 一次事务完成 card/definition/instance 更新。

### 9.3 Chronicle compressor

实现 SQLite-native：

- 阈值计算；
- job enqueue/claim/retry/fail；
- 启动时 Running → Pending 恢复；
- A → B、B → C 压缩；
- publication 与 coverage 原子更新；
- worker 重复启动幂等；
- 崩溃、迟到结果和 revision conflict 故障注入。

完成后删除当前“SQLite 模式跳过 JSON compressor”的分支。

### 9.4 `story_clock`

- 明确唯一权威字段。
- 为旧的顶层字段/variables 双表示增加迁移。
- 检测不一致时 fail closed 或生成可审修复，不得静默任选一个。
- reverse export 保持语义一致。

### 9.5 其余完整性

- instance/definition/card/campaign 外键与 scope 校验；
- 删除级联事务；
- orphan Turn/Attempt/job 清理与恢复；
- world-info、variables、knowledge、tasks 的大数据与分页路径；
- migration/import/export 的 unsupported 字段清单归零，或对不可表达的活跃状态明确阻止导出。

### 9.6 Gate 4 通过条件

- SQLite 模式不再因 Meta、MVU、Chronicle compressor 回退或拒绝桌面核心操作。
- `rg "unsupported until an atomic SQLite Meta UoW"` 无生产调用点。
- `rg "sqlite backend skips"` 无核心能力跳过点。
- 所有新增事务具备 fault-injection rollback 测试。

## 10. Gate 5：迁移、等价与恢复

### 10.1 测试数据集

至少覆盖：

- 空白新用户；
- 单角色旧 JSON；
- 多角色、多轮 Campaign；
- 同名角色与临时角色；
- summaries/knowledge/variables/tasks/worldbook 全部非空；
- pending/failed/committed Turn；
- MVU translation + schema；
- compress open/running/failed jobs；
- 大数据目录；
- 上一版 SQLite schema；
- 损坏、缺文件、锁冲突和磁盘写失败。

### 10.2 必须证明

1. JSON → SQLite 迁移前后领域快照等价。
2. 同一 migration 可重试且幂等。
3. marker 只在完整成功后写入。
4. 失败保留原 JSON 和备份。
5. SQLite → JSON reverse export 可重新导入并恢复等价终态。
6. 活跃 pre-accept 状态若不能无损导出，必须明确阻止而不是丢弃。
7. 应用重启能恢复 Turn、Attempt、outbox 和 compress jobs。
8. 多进程/文件锁冲突会 fail closed。

### 10.3 后端等价套件

对同一组 fixture 分别运行 JSON 和 SQLite：

- create/update/delete；
- draft → postprocess → Accept；
- regenerate/edit-stale；
- Meta typed patch；
- MVU apply；
- Chronicle compression；
- restart recovery；
- export/import。

比较领域结果、错误种类、revision、事件和最终快照；允许物理存储布局不同。

### 10.4 Gate 5 通过条件

- 等价套件全绿。
- migration/reverse export/fault tests 全绿。
- 没有无法解释的 backend-specific 产品行为。
- 性能没有相对 JSON 出现不可接受退化；测试记录数据规模和耗时。

## 11. Gate 6：真实证据与平台验收

### 11.1 确定性门

至少运行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p storyforge --test sqlite_optin_lifecycle
cargo test -p storyforge --test sqlite_preaccept_production_lifecycle
cargo test -p storyforge --test sqlite_meta_lifecycle
cargo test -p storyforge --test sqlite_mvu_translations
cargo test -p harness-real-llm --test endurance_sqlite_deterministic
cargo test -p harness-real-llm --test m5_production_evidence
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

前端 IPC 合同或可见行为受影响时，额外运行：

```powershell
Set-Location frontend
npm.cmd test
npm.cmd run test:ui
npm.cmd run build
```

### 11.2 SQLite 真实模型证据

在用户确认费用后，使用新 run id 依次执行：

1. Canary 3；
2. Native Coverage 12；
3. TextFallback 3，或把确实不支持的模式写成 unsupported；
4. Stability 30；
5. Full 100。

每阶段都必须完成：

- accepted 数与 exact-set coverage；
- seal；
- offline verify；
- secret scan；
- SQLite 后置条件；
- `production_postprocess_complete` 真实值；
- PASS/PARTIAL/BLOCKED 的诚实结论。

不得复活旧 45/100，也不得把历史 Native12 accepted 未 seal 写成 PASS。

### 11.3 平台现场

Windows：

- 默认 SQLite 首次启动；
- 旧 JSON 自动迁移；
- 多窗口/进程锁；
- 重启恢复；
- 大数据；
- backup/reverse export；
- 安装包升级。

Android：

- APK 构建与安装；
- app data 路径；
- JSON → SQLite 升级；
- `content://` 导入；
- keyring；
- SAF save/share；
- 前后台生命周期；
- 断网/取消；
- 大数据和恢复。

Android 真机未通过前，不得把全平台默认后端切到 SQLite。

## 12. Gate 7：默认切换与兼容退场

### 12.1 切换步骤

1. 先在开发/测试构建把 SQLite 设为默认，JSON 可显式回退。
2. 运行一个完整候选周期，记录自动回退和迁移失败率。
3. Windows 与 Android Gate 6 通过后，正式构建默认 SQLite。
4. 保留至少一个发布周期的：
   - JSON importer；
   - SQLite → JSON reverse export；
   - 迁移备份；
   - 显式紧急回退说明。
5. 稳定期后另立计划删除 JSON 生产写路径；本计划不直接删除恢复能力。

### 12.2 禁止

- 不因 SQLite 默认化删除用户旧 JSON。
- 不在无备份情况下自动重试破坏性迁移。
- 不遇错静默创建空数据库。
- 不把 JSON 与 SQLite 双写作为长期迁移方案。

### 12.3 Gate 7 通过条件

- 无环境变量时启动 SQLite。
- 旧用户自动迁移有可见进度、成功提示或可操作失败提示。
- 新写入只进入 SQLite。
- 显式回退不造成数据倒退或静默丢失。

## 13. Gate 8：文档与最终封存

更新：

- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/ARCHITECTURE-AUDIT.md`
- `docs/DOCS-CODE-AUDIT.md`
- `docs/HANDOFF.md`
- `docs/ROADMAP.md`
- `docs/RELEASE-CHECKLIST.md`
- SQLite 当前状态、M5 RESULT 和本专项 RESULT

必须删除或标记过期的口径：

- “ProductionPostprocessService 尚未抽出”；
- “远端 Gitea runner 从未运行”；
- 过期的 crate、命令和 `lib.rs` 行数；
- 已修 V1/V2/V4/V5/V7 仍显示未修；
- SQLite 已支持项仍显示 unsupported；
- 默认切换后仍写“默认 JSON”。

最终 RESULT 必须包含：

- base/head 与提交列表；
- 每 Gate 的 PASS/PARTIAL/BLOCKED；
- 代码规模前后对比；
- backend 分支点前后对比；
- schema/migration 清单；
- 测试命令和真实结果；
- 真实模型和平台证据边界；
- 未完成项和回滚方式；
- worktree、远端与 CI 终态。

## 14. 提交与审查策略

建议提交颗粒：

```text
test(tauri): pin command and backend contracts
refactor(tauri): split <domain> commands from lib
refactor(pipeline): share writing and regenerate stages
refactor(turn): unify backend-agnostic accept decisions
refactor(storage): introduce runtime backend facade
feat(sqlite): add atomic meta patch lifecycle
feat(sqlite): add atomic mvu schema apply
feat(sqlite): run chronicle compressor jobs natively
feat(storage): migrate story clock authority
test(sqlite): add backend parity and migration matrix
feat(storage): make sqlite the default backend
docs: seal backend and sqlite closure evidence
```

审查要求：

- 每次代码修改后独立 code review。
- 涉及存储、导入、IPC、插件输入时增加安全审查。
- 构建或类型错误只做最小修复，不趁机改架构。
- 不把多个 Gate 压成一个巨型提交。
- 不在同一提交混入 UI 美化、Card Studio、ST 兼容扩展或 Android 新功能。

## 15. 并行策略

可并行但须隔离文件所有权：

- Gate 0 的命令合同测试与能力矩阵整理；
- Gate 4 中 Meta UoW 与 Chronicle compressor，在 Gate 3 facade 稳定后可分支并行；
- Windows runner/安装包证据与 Android 设备准备；
- 文档漂移脚本与主代码实现。

不建议并行：

- 多个 agent 同时从 `lib.rs` 拆不同域；
- Gate 2 状态机与 Gate 3 facade 同时重写同一批命令；
- SQLite 默认切换与 migration/reverse export 修改；
- Android 数据目录接线与 backend bootstrap 重构。

机械拆分阶段优先顺序提交/cherry-pick，避免共享大文件冲突。

## 16. 本计划不包含

以下项目继续单独排期，不与本专项混做：

- 多意图/多 seed、Duet cohort、CoT 三臂 × 80 轮等高成本质量评测；
- Card Studio 小说蒸馏 Phase 2 和深度系统卡 Phase 3；
- ST 99 事件全集、冷门 Slash/TavernHelper 全量兼容；
- 插件/卡壳完整权限体系统一；
- UI 视觉美化；
- 向量检索更换真实 embedding；
- Android UI 产品增强。

其中插件权限统一、Android Phase 6、发布 Phase 7 仍是项目大项，但它们应在本
专项建立稳定命令模块与 backend facade 后分别执行，避免继续争抢 `lib.rs`。

## 17. 推荐开工点

第一刀只执行 Gate 0：

1. 新建 RESULT 骨架；
2. 生成命令/backend/unsupported 基线；
3. 补命令注册与前端 IPC 合同测试；
4. 运行完整门禁；
5. 提交一个纯测试/文档 commit。

Gate 0 通过后，第二刀从 diagnostics/presets/connections 三个低耦合域开始机械
拆分。不要直接从 `start_writing`、Accept 或 SQLite runtime 开刀。

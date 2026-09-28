# task-16 文档同步 B（CLAUDE.md / DOCS-CODE-AUDIT / DATA_MODEL / AGENT_INTERFACES / FRONTEND-COMPONENTS / ROADMAP / 小一致性）

- **任务**：task-16（域7 review-goals 承接；Lead 指定 6 项范围 + 域2 §9 六项）
- **日期**：2026-09-13
- **基线**：工作区 HEAD `ab894c6`（2026-09-06 14:18:08 +08）+ 本轮全量审查的**未提交**修复工作树
- **写作用域**：仅 `.md`（CLAUDE.md、`docs/**`、`frontend/src/design/writing/CONTRACT.md`）。**未改任何代码 / 脚本 / `.github/**` / `.gitea/**` / `.gitignore`**
- **记录文件名**：按 Lead 消息裁定为 `08-docs-sync-fixes.md`（task 描述里的旧名 `08-docs-sync.md` 以 Lead 消息为准）

---

## 0 变更清单与"是否入库"

| 文件 | 入库状态 | 本轮改动 | 备注 |
|---|---|---|---|
| `CLAUDE.md` | **未入库**（`.gitignore:79`） | 13 处 | 必读顺序重写 + 事实更正 + 新增"2026-09-13 补充事实"节 |
| `docs/DOCS-CODE-AUDIT.md` | **未入库**（`.gitignore:87`） | 3 处 | `:11`/`:54` 位置口径 + 顶部新增 2026-09-13 增量（14 条） |
| `docs/ARCHITECTURE-AUDIT.md` | **未入库**（`.gitignore:83`） | 1 处 | §5 加 2026-06-17 时点注记 + 本机笔记声明 |
| `docs/PLAN-ST-IMPORT-EXPORT.md` | **未入库**（`.gitignore:95 docs/PLAN-*.md`） | 1 处 | 默认夹具路径 |
| `docs/AGENT_INTERFACES.md` | 入库 | 3 处 | PipelineState / PostProcessSkipped / 新增小节 |
| `docs/DATA_MODEL.md` | 入库 | 1 处 | story_clock 单一权威 |
| `docs/FRONTEND-COMPONENTS.md` | 入库 | 4 处 | `ui/` 现实核对、写作面重指向、§13 计数与 wrapper |
| `docs/ROADMAP.md` | 入库 | 5 处 | `:77` tool_center、`:149` 指针、Phase 7 状态、Phase 8 计数 |
| `docs/ARCHITECTURE.md` | 入库 | 1 块（11 行） | 域2 §9 六项 + D-2/D-3 存储语义补充 |
| `docs/RELEASE-CHECKLIST.md` | 入库 | 3 处 | `:59` 历史快照标题、`:72`/`:136` 夹具路径 |
| `docs/release-closure-2026-09-06.md` | 入库 | 1 处 | G-12 方向更正 |
| `frontend/src/design/writing/CONTRACT.md` | 入库 | 1 处 | "保留作对照与回退" → 仅存档参考 |

`git diff --stat`（仅入库文件，vs HEAD）：`8 files changed, 48 insertions(+), 24 deletions(-)`
（`AGENT_INTERFACES 10+/-`、`ARCHITECTURE 25`、`DATA_MODEL 2`、`FRONTEND-COMPONENTS 11`、`RELEASE-CHECKLIST 6`、`ROADMAP 14`、`release-closure 2`、`CONTRACT.md 2`；其中 `ARCHITECTURE.md` 的多数行属于 task-14 的既有改动，本任务只新增 `:156` 起的 11 行存储语义块）

> ⚠️ 4 个未入库文件的改动**只存在于本机工作区，不会进入任何 commit，也不会出现在其它克隆或新克隆里**。任何跨人交接内容（commit message / README / CI）只能引用入库的那 8 个文件。

> **写作用域授权说明（重要）**：task-16 原始 description 的 writeScopes 只列了 7 个文件，且写明"README/RELEASE-STATUS/ARCHITECTURE/HANDOFF 归 task-14"。本次实际改动超出了那份旧清单，**依据是 Lead 在 2026-09-13 的较新指令**：(a) 6 项范围点名 `docs/RELEASE-CHECKLIST.md:72/:136`、`docs/PLAN-ST-IMPORT-EXPORT.md:127`、`docs/release-closure-2026-09-06.md:144`、`frontend/src/design/writing/CONTRACT.md:55`；(b) "域2 的文档同步条目共 6 项…请一并纳入"（落点 `docs/ARCHITECTURE.md`）；(c) 记录文件名以 `08-docs-sync-fixes.md` 为准。执行时 task-14 已 `complete`，这些文件无并发写者。**未被触碰**：`README.md`、`docs/RELEASE-STATUS.md`、`docs/HANDOFF.md`、`.gitignore`、所有 `.rs`/脚本/CI 文件。

---

## 1 `CLAUDE.md`（本机文件）

### 1.1 Required Reading Order 重写（区分"入库权威文档"与"本机工作笔记"）

- **原**：`1. docs/DOCS-CODE-AUDIT.md / 2. docs/ROADMAP.md / 3. docs/ARCHITECTURE-AUDIT.md / 4. docs/AGENT-INTERFACES.md / 5. docs/DATA_MODEL.md` + "Treat `docs/DOCS-CODE-AUDIT.md` as the authority…"
- **新**：分 A/B 两组。
  - A（入库权威）：`ROADMAP.md` → `AGENT_INTERFACES.md`（**下划线**；显式注明原文的 `AGENT-INTERFACES.md` 不存在）→ `DATA_MODEL.md` → `ARCHITECTURE.md` →（前端/发布按需）`FRONTEND-COMPONENTS.md`、`RELEASE-CHECKLIST.md`、`RELEASE-STATUS.md`。
  - B（本机工作笔记，克隆后可能不存在）：`DOCS-CODE-AUDIT.md`(`.gitignore:87`)、`ARCHITECTURE-AUDIT.md`(`:83`)、`CLAUDE.md`(`:79`)、`HANDOFF.md`(`:92`)、`PHASE8-FOLLOWUP-ISSUES.md`(`:94`)、`PLAN-*.md`(`:95`)、`REGRESSION-COVERAGE.md`(`:97`)、`ST-EVENTS-COVERAGE.md`(`:98`)。
  - 明确一行：**只有 A 组文档能写进 commit message / README / CI / 跨人交接内容**。
- **依据**：G-05/G-20 与 07 §5.2、09 §7.4；`git ls-files --error-unmatch docs/HANDOFF.md` 失败；`git check-ignore -v` 命中 `.gitignore:79/83/87/92/94/95/97/98`；`.gitignore:103` 忽略整个 `docs/archive/`。Lead 决定**不动 `.gitignore`**。

### 1.2 `delete_character` 一律不改保留性（保留依据链）

- **原**：仅有级联语义一句（`StoredCharacter.id` / `source_character_id` / 同会话 `tool_ctx` 域 id 的 Campaign/MVU/向量清理）。
- **新**：**原文逐字保留**，追加一句：该命令当前无前端入口，但属"声明的保留 API"（T-15），`backend-baseline.mjs` 的 `RETAINED_NO_FRONTEND_CALLER` 正是引用本条级联语义作为保留依据，**不要删改这段说明，也不要写成死代码/待删**。
- **依据**：`fixes/04-tauri-fixes.md` T-15 回写硬约束；`fixes/05-frontend-fixes.md` §5-4/§5-8。

### 1.3 命令位置 / 命令总数 / wrapper 删除面

- **原**：`- Frontend API wrappers in frontend/src/tauri-api.js: listAgentProfileConfigs, getAgentProfileConfig, getActiveAgentProfileConfig, saveAgentProfileConfig, deleteAgentProfileConfig, setActiveAgentProfileConfig.`
- **新**：现存 wrapper 逐个列出行号（`listAgentProfileConfigs:176`、`getAgentProfileConfig:184`、`saveAgentProfileConfig:192`、`exportAgentProfileConfig:199`、`importAgentProfileConfig:207`、`deleteAgentProfileConfig:215`、`setActiveAgentProfileConfig:222`）；**只有 `getActiveAgentProfileConfig` 真的已被删除**（域5 F-30 的 20 个零引用 wrapper 之一）；后端**一条命令都没删**：总数仍 **175** = `commands/*.rs` **156** + `card_studio_api.rs` **19**；这 20 条加既有 3 条（`abandon_turn`、`archive_conversation`、`soft_delete_variant`）= **23 条**登记为"无前端入口的声明的保留 API"。
- **依据**：`tauri-api.js` 逐行核对（1add1022 独立扫描：6 个"已删"里 5 个仍在，仅 `getActiveAgentProfileConfig` 消失；`git diff -U0` 显示被删签名 21 条、净 −20）；`backend-baseline.mjs:151/:153-275` 的 23 条表 + 运行时 `orphanRegisteredCommands = 23`；命令原始出现 176 与权威 175 的差额来自 `commands/import_export.rs:440` 的**文档注释**误计（整行锚定后 156）。

### 1.4 `migrate_to` / `DEFAULT_STORY_CLOCK` / `write_fence` / 行为契约 / 其余域1事实

- **原**：`unknown versions update config_version but preserve data (forward-compatible)`。
- **新**：`if target <= self.config_version { return false; }`——**只有更高的 target 才推进版本，绝不降级已存的 `config_version` 标记**；更高/未知版本数据保持原样。四条载入路径都调 `migrate_to(1)`：`new()`(`module_store.rs:513`)、`get()`(`:573`)、`get_active()`(`:587`/`:597`)。
- **依据**：`agent_profile_config.rs:248-257`；测试 `migrate_to_never_downgrades_version_marker`、`migrate_to_unknown_version_updates_version`；HEAD 版本 `==` 判定会把 5 降级成 1（本工作区已修）。

- **原**：`L6：card_shell_clear_cache 命令 + wrapper cardShellClearCache（UI 入口未接）。`
- **新**：**UI 入口已接线**（`components-v2/shell/InspectorDrawer.vue`），并补 T-03/T-05（`card_shell_fetch_url` 已 async、`card_shell_register_doc/module` 返回结构化错误 DTO）、T-09（`card_shell_allow_host` 保留 API + 提权放大面）。
- **原**：`apply_campaign_opening Tauri 命令（lib.rs）`。
- **新**：更正为 `crates/tauri-app/src/commands/campaigns.rs:486`（Gate 1 后 lib.rs 只是装配层）。
- **依据**：`InspectorDrawer.vue:18/45/87`；`commands/campaigns.rs:486`；`fixes/04-tauri-fixes.md` T-14。

- **原**：`PostProcessSkipped … emitted when postprocess/summarizer is disabled by config`（单句）。
- **新**：追加 W-28——**取消**同样映射为 `PostProcessSkipped`：`runtime_support.rs:131-177` 的 `postprocess_pipeline_event` 在 runner 前/后取消或结果 `skipped_reason="cancelled"` 时发**恰好一个** `PostProcessSkipped { reason: "postprocess cancelled" }`；`PostProcessFailed` 只在持久化真失败（`Err`）时发。
- **依据**：`fixes/03-pipeline-fixes.md` W-28；`git show HEAD:crates/app-pipeline/src/lib.rs` 实证 HEAD 下"取消+开关打开"报成 `PostProcessFailed`（本工作区新增取消分支，未提交）。

- **新增"### 2026-09-13 审查修复后的补充事实"节**（10 条）：行为契约三连（`log_clear` 返回 `Result` + 显式 `"all"`、`add_variant` 可选 `provenance`、3 条 import/export 命令转 async）；`DEFAULT_STORY_CLOCK` 单一权威；`write_fence`（含解除入口，见下）；出卡闸门 `KNOWN_RULE_CODES`；注入标题两组；M-32.8/M-32.9 世界书导出；D-19 指纹 `u64` 长度前缀（跨版本不可比）；D-10/D-20 复杂度；Card shell D-22/D-24/D-25。
- **依据**：`fixes/01-domain-infra-fixes.md` §7、`fixes/03-pipeline-fixes.md`、`fixes/04-tauri-fixes.md` §7-6。

### 1.5 ⚠️ 事实核对更正：`write_fence` 的"一键解除"**不是**未接线（本轮发现并改写）

- **初稿（错）**：按域1 §7-5/§8-1 写成"应用内一键解除属产品决策 N-03 暂缓；tauri-app 侧消费接线待补"，并把冻结当作只有日志 + `PermissionDenied`。
- **定稿（改后）**：`storage_health::record_unrecoverable`（`storage_health.rs:61`）登记 blocking 事件 → 启动时 `record_write_fence_state()`（`:104`，`lib.rs:848` 调用）把 `frozen_entries()` 报为 `kind = "write_fence_frozen"` → 用户确认后 `storage_health_acknowledge`（`commands/diagnostics.rs:16`，注册于 `lib.rs:1462`）调 `write_fence::unfreeze`；前端 wrapper `tauri-api.js:1093`，UI `components-v2/shell/StorageHealthGate.vue`（`AppV2.vue:45/801` 挂载），损坏文件另留 `.json.corrupt`；**没有任何自动解除/静默重试**。
- **依据**：本成员亲自复核（`Select-String` 命中 `diagnostics.rs:16`、`lib.rs:1462`、`tauri-api.js:1093`、`StorageHealthGate.vue:33-40/89`、`AppV2.vue:45/801`、`storage_health.rs:140-141`）+ 独立只读核查（065d30fe）同结论。
- **影响**：域1 记录的"接线待补"表述与工作区代码不符；**本记录以此更正为准**，CLAUDE.md 与 ARCHITECTURE.md 按代码写。

---

## 2 `docs/DOCS-CODE-AUDIT.md`（本机文件）

### 2.1 `:11`（2026-09-01 增量第 1 条）与 `:54`（"已核对为代码事实 · Workspace 和命令数量"）

- **原（两处）**：`全部 175 个命令分布在 crates/tauri-app/src/commands/*.rs` / `全部位于 crates/tauri-app/src/commands/*.rs`。
- **新**：`命令总数 175 个 = commands/*.rs 156 个 + crates/tauri-app/src/card_studio_api.rs 19 个`，全部注册于 `lib.rs` 的 `generate_handler!`；该位置约束**现在有门禁**（`backend-baseline.mjs` 的 `COMMAND_LOCATION_ALLOWLIST`，违规 exit 1），README 的"175 个命令"仍正确。
- **依据**：`fixes/04-tauri-fixes.md` T-13/T-16；`fixes/09-goals-scripts-fixes.md` G-08；`backend-baseline.mjs` 实测 `commandAttributes 175 / defined 175 / registered 175`，差异来源 `import_export.rs:440` 注释。

### 2.2 顶部新增"## 2026-09-13 全量审查增量（修复回写）"（14 条）

内容覆盖：命令位置与 175；`backend-baseline.mjs` 已是**门禁**（违规 exit 1、`--write-snapshot` 门禁未过**拒绝写入**、无参数只读）；23 条**保留 API**口径（只删前端 wrapper，`delete_character` 依据是级联语义）；前端唯一 invoke 数 172 → **152**（脚本输出、不硬编码）、零入口 3 → 23；行为契约三连；`release.yml` 两个 job 各新增一步前端契约测试（**未经真实 release 运行验证**、job 不含 fmt/clippy/cargo test/Pester）；Gitea Actions 2026-09-06 停用；G-12 定案（留存 Windows 196 B、缺 Android 后补传 88 B）；测试计数（Pester 198、node 67/532、vitest 31 文件**通过数待 Lead**）；前端清理（9 基元删除、`ui/` 现存 14 个、ErrorState→内联、`components-v2/writing/**` 零引用）；接口事实（PipelineState 7 变体、`PostProcessSkipped` 双语义、`SequentialActorOutcome`、`compile_turn_dossier` 5 参、M-08 插件回调）；域1/域3 事实（`migrate_to`、`DEFAULT_STORY_CLOCK`、`write_fence`、`KNOWN_RULE_CODES`、注入两组、tail 指纹、`tool_center.rs` 已删、`RegexScript.placement` 有默认而 `find_regex`/`replace_string` 无默认）；域2 存储面（S-01/S-10/S-17/S-22.4 + `write_fence_frozen` + `secret_store` fail-closed）；`story_clock` 与默认夹具 `data/local/test-card.png`。
- **依据**：`fixes/01`§7、`fixes/02`§9/§11、`fixes/03`§5、`fixes/04`§7、`fixes/05`§5、`fixes/06`§11、`fixes/07`§9、`fixes/09`§7.2。
- **保留历史不删**：`:28`、`:300` 等 2026-07-07/07-26 历史核对记录按原样保留（时点记录，不是当前状态断言）。

---

## 3 `docs/AGENT_INTERFACES.md`（入库）

### 3.1 `PipelineState` 变体清单（W-26）

- **原**：`- \`StateChanged { state }\`（PipelineState：Generating / Editing / Review / Committed / Aborted）`
- **新**：`PipelineState`（`crates/domain/src/agent.rs:371-386`）共 **7** 个变体：`Idle / Directing / Delegating / Editing / Review / Committed / Aborted`；显式注明"原文写的 `Generating` 不存在，且缺 Idle/Directing/Delegating"。
- **依据**：`agent.rs:371-386`；`fixes/03-pipeline-fixes.md` W-26；065d30fe 独立核对（文档有代码无：`Generating`；代码有文档无：`Idle/Directing/Delegating`）。

### 3.2 `PostProcessSkipped` 取消语义（W-28）

- **原**：`- \`PostProcessSkipped { reason }\`（AgentProfileConfig 关闭 postprocess/summarizer 时发出，区别于真失败）`
- **新**：两种情况——① 配置关闭；② **取消**：`runtime_support.rs:131-177` 在 runner 前/后取消或 `skipped_reason="cancelled"` 时发恰好一个 `PostProcessSkipped { reason: "postprocess cancelled" }`；`PostProcessFailed` 只在持久化真失败时发，其它 skipped 原因不发事件。
- **依据**：`fixes/03-pipeline-fixes.md` W-28 + 代码/HEAD 对照（见 §1.4）。

### 3.3 新增小节"## 轮次内部结果：取消 / 跳过 / 失败（W-07 / D-04 / M-08）"

- **原**：文档**完全没有** `SequentialActorOutcome` / `compile_turn_dossier` / 插件 prompt hook 回调契约（不是"签名不符"，是未记载）。
- **新**：写入 ① `SequentialActorOutcome { Cancelled, SceneClosed, Failed(String) }`（`sequential_crew.rs:167-175`，`pub(crate)`，配套 `into_agent_error()`/`From<AgentError>`，结果为 `Vec<Result<Performance, SequentialActorOutcome>>`）；② `compile_turn_dossier(intent, runtime, pending_tasks, story_clock: &str, max_full_actors)`（`turn_dossier.rs:149-156`；两处调用点 `lib.rs:1029`/`:1225`，`max_full_actors` 分别 3/2，`story_clock` 由硬编码 `""` 改为 `&ctx.story_clock`）；③ M-08 前端契约 `pluginPromptHookResult(requestId, messages, error, pluginId?, modifierPluginIds?)`，未声明改写权限的插件存在时后端丢弃 messages 并记审计。
- **依据**：`fixes/03-pipeline-fixes.md` §5-2/§5-4、`fixes/06-meta-plugin-fixes.md` §11-6；代码行号经 065d30fe 独立核对。
- **未纳入**：W-31（`AGENT_INTERFACES.md:12` 与前端 `validGenerationModes`）与 W-18（`big_scene` 自动路由/成本确认）属跨域移交项，本轮**未改**该行。
  - ⚠ **2026-09-13 R2 复检更正（更正来源：task-34 / `round2/R11-pipeline-regression-closure.md` §R6 收口、`round2/R2-pipeline-recheck.md:111`）**：
    原句写作"域5 记录称其已在前端侧修复、文档侧待 task-16"——**这是虚假归属**：撰写当时 `fixes/05-frontend-fixes.md` 全文对 `W-31`/`validGenerationModes` **零命中**，域5 从未作过该声明（R2 复检据此记为"被源码与域5 记录双重反证"）。
  - **现状与真实归属（以本轮实测为准）**：W-31 的**前端**部分已由域5 在 R2 收尾时修复（`frontend/src/stores/writing.js:22` `validGenerationModes` 改为从 `utils/generationModes.js` 目录派生、`:68` 拒绝未知档位；域5 记录见 `fixes/05-frontend-fixes.md:475-485`），残留死分支 `frontend/src/adapter/useWritingScreenAdapter.js:89` 与 **W-18**（`useWriting.js:139-141`、`frontend/src/stores/writing.js:57-62` 的自动路由/成本确认不可达）归 **task-32 / task-33**；文档侧 `docs/AGENT_INTERFACES.md:12` 仍未改，归文档任务（task-f7/task-33）。**注意**：`frontend/src/stores/writing.js:18-20` 里仍出现 `big_scene` 字样，但那是"该值不得成为合法档位"的说明性注释，不能据此判缺陷仍在。

---

## 4 `docs/DATA_MODEL.md`（入库）

- **原（`:97`）**：`story_clock 当前同时存在于顶层字段和 variables 中。…语义上应以 variables 为准，未来可通过数据迁移统一。`
- **新**：保留原句，追加 **2026-09-13 回写（D-06）**：`DEFAULT_STORY_CLOCK = "第1天"`（`crates/domain/src/variables.rs:107`）既是 `story_clock` 变量 schema 默认值（`:112`），也是 `Campaign::default_story_clock()`（`campaign.rs:61`）唯一委托来源；新建 Campaign 顶层字段取自该 schema 默认值（两处生产入口 `commands/campaigns.rs:347-352`/`:423-428` 都走 `new_with_variable_schema`），infra-sqlite 导入兜底同样引用该常量（`importer.rs:565`，不再有 `"Day 1"`）；"未来可通过数据迁移统一"只针对**历史数据**。
- **依据**：`fixes/01-domain-infra-fixes.md` §7-2（D-06）、`fixes/02-storage-fixes.md` §11 D-1；065d30fe 核对（剩余 `第1天` 字面量均在 `#[cfg(test)]` 或常量本身）。
- **未纳入**：`RegexScript` 默认值（D-26）——`DATA_MODEL.md` 无 regex 小节，故写入 `DOCS-CODE-AUDIT.md` 的 2026-09-13 增量（§2.2）；`PipelineState` 在 `DATA_MODEL.md` 中不存在（065d30fe 实证），无需改。

---

## 5 `docs/FRONTEND-COMPONENTS.md` + `frontend/src/design/writing/CONTRACT.md`（入库）

1. **§3 基础组件蓝图后新增"2026-09-13 现实核对"**：现状 **14 个 `.vue`**（Badge、Button、CodeBlock、DataList、DataTable、EmptyState、IconButton、Input、LoadingState、Overlay、Select、Tabs、Textarea、Toggle）；已删 9 个零引用基元（Checkbox、DiffView、ErrorState、Menu、Progress、SegmentedControl、Slider、Toast、Tooltip），`Dialog.vue` 早于 commit `894730c`（2026-09-01）删除；**错误态不再引入 `ErrorState`**，改为面板内联"错误行 + 重试"（F-19 降级方案）。依据：`fixes/05-frontend-fixes.md` §5-6/F-18/F-19；`git status --short` 显示 9 个删除为**工作区未提交删除**。
2. **§13 组件根目录**：原 `components-v2/`（含 `writing/`）→ 更正为生产写作面在 `frontend/src/design/writing/**`，`components-v2/writing/**` 现存 8 个 `.vue`，属**阶段 8 遗留、零外部引用、未接线**。依据：1add1022 全树扫描（`frontend/src` 内无指向该目录的相对导入，仅目录内自引用 `ConversationViewport.vue:22/24`）。
3. **§13 Writing 行**：拆成"Writing（生产）"与"Writing（遗留存档）"两行，后者注明**不再承诺"可回退"**。
4. **§13 末尾**：补测试与计数——node:test **67 文件 / 532 通过**；vitest `frontend/tests/components-v2/**` **31 个文件**，**通过数待 Lead 收口门禁确认后回填**；前端唯一 invoke 数（2026-09-13 实测 **152**）**由 `backend-baseline.mjs` 运行时输出、不要硬编码**；wrapper 删除 20 个（可观测清单见 `fixes/05` §5-8）+ 后端 175 条未删 + 23 条保留 API。
5. **`frontend/src/design/writing/CONTRACT.md:55`**：
   - **原**：`6. 旧 \`components-v2/writing/*\` 保留作对照与回退，生产主路径不再引用 Viewport/Composer。`
   - **新**：改为"**仅作存档参考**…**不承诺"可回退"**（零引用、未接线，2026-09-13 复核…见 F-46）"。
   - **依据**：`fixes/05-frontend-fixes.md` §5-5 F-46；1add1022 指出真实路径是 `frontend/src/design/writing/CONTRACT.md`（仓库根不存在 `design/`），且全文 **没有**"可回退"字样（原文是"保留作对照与回退"），故按原文更正而非按误记的"可回退"改。

---

## 6 `docs/ROADMAP.md`（入库）

| 位置 | 原 | 新（要点） | 依据 |
|---|---|---|---|
| `:77` | `~~统一 tool 注册中心。~~ ✅ 第四轮 E：\`tool_center.rs\` + 按角色选配` | 删去已删除文件名，注明 W-22 删除原因（零生产调用者的死模块 + 幻影工具名），现役能力指向 `app-agent/src/tools.rs` 的 `ToolRegistry` + `register_*_tools` + `ToolRegistry::retain` | W-22；`tool_center.rs` 已删（`git status: D`），crates 侧仅剩 `tools.rs:2052-2055` 的防回归注释/测试名 |
| `:149` | `Android 模拟器 15 项现场验收 PASS 见 \`docs/HANDOFF.md\` §11.3` | 指向真实位置 `docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md` §11.3（`:373`「验证证据」），并说明 `HANDOFF.md` / `docs/workstreams/**` 均被 `.gitignore` 忽略 | G-10；本成员实测 `Test-Path` + `Select-String ':373 ### 11.3 验证证据'` + `git check-ignore`(`:102`/`:92`) |
| `:169` Phase 7 | 无状态行 | 新增 `**状态：部分完成**`：端到端矩阵/回归与质量样例/备份恢复与排障 bundle/发布包与用户指南/下一阶段优先级**有证据**；**长会话性能与调用成本无专门证据**；验收①**无用户实操记录 → 无法判定**，②③已满足 | G-14；`fixes/09-goals-scripts-fixes.md` §7.3、`07-goals-and-claims.md` |
| `:202` | `重写写作工作台（ChatMessage 保留 8 emit 契约）` | 追加更正：8 emit 契约的真实载体是 `frontend/src/design/writing/MessageItem.vue`；`components-v2/writing/ChatMessage.vue` 零引用 | F-46；§5.2 扫描 |
| `:210-212` | `node --test 212 pass + vitest 21 pass` / `构建产物 422KB` / `tauri-api.js…签名零改动` | 三条都加 **(2026-07-08 时点)**；当前计数写 node **67/532**、vitest **31 文件（通过数待 Lead）**；构建大小以 `RELEASE-STATUS.md` 为准；说明"tauri-api.js 签名零改动"只对 07-08 时点成立（本轮删 20 个 wrapper），并补 175/23 保留 API 口径 | F-40/G-16 + Lead 口径（vitest 通过数待收口门禁） |

---

## 7 `docs/ARCHITECTURE-AUDIT.md`（本机文件）

- **原（§5 末）**：仅 `风险：继续做插件 UI 会变成"能装 manifest，但不能影响主体验"。`
- **新**：追加 2026-09-13 复核注——本节是 **2026-06-17 时点**判断；MVU runtime 已于 W10 接通（`crates/tauri-app/src/mvu_webview_runtime.rs`，`StubMvuRuntime` 仅测试替身），分析结果已进入 Campaign variable schema 与前端状态栏；**仍成立的是"插件权限分层未覆盖"**；并声明本文件被 `.gitignore:83` 忽略、属本机笔记。
- **依据**：G-20；`fixes/09-goals-scripts-fixes.md` §7.2。

---

## 8 域2 §9 六项 → `docs/ARCHITECTURE.md` 新增"存储语义补充"块（入库，`:156` 起 11 行）

| §9 项 | 写入要点 |
|---|---|
| 1 S-01 | "缺失 legacy 文件按空集合导入"的**例外**：`cards.json`/`campaigns.json` 文件缺失且存在交叉引用 → **fail-closed**；文件存在但为空仍是空集合；skip 经 warn + `storage_health` backend incident 暴露；`import_runs` **无 skip 计数列**（重启后读不回，已知降级项） |
| 2 S-10 | 文件库必须确认 `PRAGMA journal_mode` 真为 `wal`，否则 `SqliteError::Other("journal_mode WAL was not applied (got …)")` **启动失败**；`configure_connection(conn, expect_wal)`（`open` true / `open_in_memory` false）；`synchronous=NORMAL` 断电可能丢最近已提交事务，导出/迁移须走一致性快照 |
| 3 S-17 | `DbProbeFailed` 阻断启动（transient 占用也不例外）；`DbVersionAhead` 由 marker reconcile 处理（先回写 Foreign marker 的 `schema_version`），但**"回写版本"≠"承认权威"**，绑定不一致仍 fail-closed |
| 4 S-18/6b | perf 测试只有 `println!`、无阈值未 `#[ignore]`；`backend_parity_suite.rs` 重启子进程检查默认 no-op → 需"独立 perf job + 阈值"或显式 ignore，**当前不构成发布门禁** |
| 5 S-22.2/22.3/22.6 | cutover 读两次源、首开无租约窗口、rollback 不覆盖 `active_campaign.json` 三点作为**已知限制**（fail-closed 或仅告警，不损数据） |
| 6 S-22.4 | 导出前让位备份只保留最新 **2** 份（`MAX_PRE_EXPORT_BACKUPS = 2`） |
| + D-2 | SecretRef 解析失败一律 fail-closed（4 处调用点，无 ref-as-key 回退） |
| + D-3 | `write_fence` 冻结状态并入 `storage_health`：按真实路径登记 `blocking = true`、kind 串 `write_fence_frozen`、按 path 去重幂等；前端 `storage_health_acknowledge(path)` + `StorageHealthGate.vue` 解冻 |

- **依据**：`fixes/02-storage-fixes.md` §9 六项逐字 + §11 D-1/D-2/D-3 + Lead 2026-09-13 消息点名要求（`write_fence_frozen`、story_clock 单一权威、`secret_store` fail-closed）。

---

## 9 小一致性（入库）

| 文件:行 | 原 | 新 | 依据 |
|---|---|---|---|
| `RELEASE-CHECKLIST.md:59` | `2026-07-14 已验证（代码验证基线 \`99b1ea3\`，不是当前 main SHA）：` | `2026-07-14 已验证（**历史快照，不是当前状态**；代码验证基线 \`99b1ea3\`…；当前自动化状态见 \`docs/RELEASE-STATUS.md\`，SQLite 现状态见本文件 SQLite 段）：` | G-19；07 §9 |
| `RELEASE-CHECKLIST.md:72` | `\`scripts/run-real-card-smoke.ps1\` 默认读取仓库根目录 \`test-card.png\`` | 默认 `data/local/test-card.png`（`-FixturePath` 可覆盖；仓库根仅兼容回退；`data/` 被忽略 → 本地 opt-in） | 09 §7.4/§11.7；`scripts/run-real-card-smoke.ps1` 默认值 + `infra-import::default_real_card_fixture_path()` |
| `RELEASE-CHECKLIST.md:136` | S1 行"自动子项可用仓库根目录 `test-card.png`" | 默认 `data/local/test-card.png`（缺失时回退仓库根） | 同上 |
| `PLAN-ST-IMPORT-EXPORT.md:127` | `覆盖仓库本地 \`test-card.png\`` | 默认夹具 `data/local/test-card.png`，说明本地 opt-in 与回退 | 09 §7.4；同上 |
| `release-closure-2026-09-06.md:144` | `上传时 android 覆盖 windows（v0.1.1/v0.1.2 均如此）` | 同名资产冲突、后到那份被跳过；**2026-09-13 定案：留存的是 Windows 校验和（196 B），缺的是 Android（后补传 88 B）**；显式标注"原文方向相反"；平台命名自 v0.1.2 之后生效 | 09 §5.1/§5.3；G-12 |

---

## 10 未纳入本任务 / 移交 Lead 的项（诚实边界）

1. **域2 `import_runs` skip 列**：需 V009 schema 变更（跨域），本轮只把它写成**已知降级项**。
2. **`.github/workflows/release.yml`**：**Lead 2026-09-13 复核：该处无残留**——域4 已修好（`:255` 通配已是 `*arm64-release.apk` 并注明实际资产名、`:97`/`:213` 分平台校验和、`:258` 正文已写两份校验和文件），本记录原写的 `:247` 是陈旧行号；仍待验证的是 G-07 tag↔version 校验补丁（CI 文件，本任务不改）。
3. ~~**`docs/ST-EVENTS-COVERAGE.md` G-09**（`32 个`→`30 个`、`plugin-bridge.js:20-51`→`:29-60`、`:53-59`→`:62-68`）与 **`docs/REGRESSION-COVERAGE.md` G-18**（`:12` 行号声明、`:14` `campaign.rs:389`→`:719`，另 10 行仍指向已删除的 `tool_center.rs` 行号）：两者均为**未入库笔记**且不在 Lead 的 6 项范围内，本轮未改，建议追加一条 task 或由 Lead 直接指派。~~ **→ 已由 task-28 处理（见 §12.1/§12.2）**；注意 task 里给的替换值也是旧值：实测是 `:33-64`/`:66-72`（不是 `:29-60`/`:62-68`），`campaign.rs` 实测是 `:798` 起（不是 `:719`）。
4. **`docs/ARCHITECTURE.md` 的 M-01/M-10/M-23/M-04/T-03/T-05/M-08 架构措辞**：仅 T-03/T-05/T-09 的接口事实写入了 CLAUDE.md 与 DOCS-CODE-AUDIT；M-01（Windows 子帧可达 Tauri IPC）等**安全表述**需 Lead 决定口径后再改 ARCHITECTURE.md（属域6 暂缓项）。
5. ~~**`docs/PRODUCT-REVIEW-2026-06-23.md:194`** 仍写"复用 tool_center"~~ **→ task-28 已加"已删除"注，见 §12.4**（该文件未入库，`.gitignore:96`）。
6. ~~**`docs/RELEASE-STATUS.md:56` G-15**（`rerun-1a`/`rerun-1b` 证据标注）~~ **→ task-28 已按证据来源标注，见 §12.3**（实际位置是 `:63`/`:64`，不是 `:56`）。
7. **`.gitea/DECOMMISSIONED.md` 与脚本改动仍需 `git add`**（09 §10-4），由 Lead 统一处理。

---

## 11 验证、限制与诚实声明

- **本任务未运行任何测试/门禁**：前端 `npm test`、`vitest`、`cargo test/fmt/clippy`、Pester 均由 **Lead 收口门禁**执行（成员侧 `spawn EPERM`；且 Lead 明确 fmt/clippy 正在由 Lead 修，Rust 侧不许触碰）。本文件的所有"计数"均为**引用**修复记录的实测值，未在本任务重跑。
- **vitest 通过数刻意留空**：文档只写"31 个文件，通过数待 Lead 收口门禁确认后回填"（Lead 2026-09-13 明令；域5 的 `136 pass` 为 A' harness 口径，未经 Lead 确认不写入入库文档）。
- **未入库文件的效果边界**：`CLAUDE.md`、`DOCS-CODE-AUDIT.md`、`ARCHITECTURE-AUDIT.md`、`PLAN-ST-IMPORT-EXPORT.md` 的改动**不会进入 commit**；`.gitignore` 按 Lead 决定未动。
- **换行符**：CRLF 文件（`CLAUDE.md`、`DOCS-CODE-AUDIT.md`、`ARCHITECTURE-AUDIT.md`）的插入行保持 CRLF；LF 文件（`ROADMAP.md` 等）保持 LF；`docs/ARCHITECTURE.md` 插入时一度写成 CRLF，已归一为 LF（0 CRLF / 199 lines）。`git` 提示"LF will be replaced by CRLF"是仓库既有 `core.autocrlf` 行为，与本次改动无关。
- **一条被推翻的文档事实**：`write_fence` 的"一键解除未接线"经代码核对**不成立**（`storage_health_acknowledge` + `StorageHealthGate.vue` 均已存在），已在 CLAUDE.md/ARCHITECTURE.md 按代码书写，并在本记录 §1.5 标明与域1 记录的差异。
- **范围声明**：本任务只做 `.md` 回写；任何代码/脚本/CI 的进一步修改都属其它 owner。

---

## 12 task-28 收尾（失效行号 / 计数与残留引用）

- **入库状态**：`docs/RELEASE-STATUS.md` = **入库**；`docs/ST-EVENTS-COVERAGE.md`（`.gitignore:98`）、`docs/REGRESSION-COVERAGE.md`（`:97`）、`docs/PRODUCT-REVIEW-2026-06-23.md`（`:96 PRODUCT-REVIEW-*.md`）、`docs/DOCS-CODE-AUDIT.md`（`:87`）= **未入库（改了也不进 commit）**。
- **方法**：每条都先用 `Select-String`/`rg` 在**当前工作树**核对真实行号/数量再落笔，不照抄 09 号报告与 Lead 消息里的旧数字——两处旧数字都与实测不符：G-09 给的 `:29-60`/`:62-68` 实测为 `:33-64`/`:66-72`；G-18 给的 `campaign.rs:719` 实测 `resolved_persona_override_takes_priority` 在 **`:798`**、`resolved_persona` 本体在 **`:399`**。

### 12.1 `docs/REGRESSION-COVERAGE.md`（未入库，8 项）

1. **全表行号重算**：73 处 `文件:行` 引用里 **55 处已漂移**，已全部替换为真实 `fn` 行号；重算后 **63/63 可解析引用全部命中当前代码**。依据：逐行 `Select-String -Pattern 'fn <测试函数名>' <文件>` 比对（脚本化，非人工估计）。示例：`resolved_persona_override_takes_priority` `campaign.rs:389`→**`:798`**、`temporary_with_overrides_sets_persona` `:485`→**`:894`**、`knowledge_for_instance_filters_by_id` `campaign_runtime.rs:356`→**`:358`**、`b8_subagent_whitelist_cannot_add_unregistered_tool` `writeback_isolation.rs:557`→**`:581`**。
2. `:11` 列说明（原→新）：`` - **测试文件（:行）**：`git grep` 可定位，行号为文件当前位置 `` → `` - **测试文件（:行）**：`rg -n 'fn <测试函数名>' <文件>` 可定位；行号为 2026-09-13 复核值，**仍会随代码漂移，检索一律以测试函数名为准** ``。依据：这正是 G-18 指出的"行号权威"错误声明。
3. 表头日期行追加 2026-09-13 复核注（55/73 漂移、检索方式、§7 失效、三个测试已迁移）。
4. §4 两行路径更正：`crates/tauri-app/src/lib.rs:11513`→`crates/tauri-app/src/lib_tests_writing.rs:187`（`test_postprocess_persistence_helper_writes_all_campaign_outputs`）；`:12697`→`lib_tests_writing.rs:1700`（`test_postprocess_writes_knowledge_for_persisted_temporary`）。
5. 「已知缺口」三行更正：`lib.rs:11637/:11707/:11770` → `lib_tests_writing.rs:308/:375/:440`（`postprocess_skips_unknown_character_target` / `postprocess_validates_task_belongs_to_campaign` / `postprocess_empty_present_chars_rejects_witnessed_knowledge`）。
6. **§7 ToolCenter 整节改写**为「已删除（W-22，2026-09-13），本节整体失效」：`crates/app-agent/src/tool_center.rs` 不存在（`Test-Path`=false），原 10 个测试全无命中（`rg -n 'fn role_matches' crates`→0、`fn all_summaries_returns_all_tools`→0）→ 删掉 10 条失效行号，改为「原测试名（历史）→ 现役对应」映射：`tools.rs:242 ToolRegistry::retain`、`tools.rs:2070 registered_tool_names_are_flat_and_free_of_tool_center_phantoms`、`writeback_isolation.rs:581 b8_subagent_whitelist_cannot_add_unregistered_tool`。
7. 「已知缺口」的 `retain()` 行：**该缺口已不存在**——`tools.rs` 现有 `retain_none_keeps_all_tools`(:1789)、`retain_empty_vec_clears_all_tools`(:1799)、`retain_partial_list_keeps_only_listed`(:1831)、`filter_registry_by_whitelist(None)`(:1868) → 标 ✅ 已补；节末"剩余两项"改"剩余一项"（只剩 `AgentProfileConfigStore save` 前置验证）。
8. 登记步骤的分类名 `ToolCenter` → `ToolRegistry`。

### 12.2 `docs/ST-EVENTS-COVERAGE.md`（未入库，4 项）

| 位置 | 原 | 新 | 依据 |
|---|---|---|---|
| `:4` | `> 日期：2026-07-08` | 追加 2026-09-13 复核注（实为 30 个；检索以常量/函数名为准） | `frontend/src/plugin-bridge.js` 实测 |
| `:11` | `32 个事件类型常量在 frontend/src/plugin-bridge.js:20-51` | `30 个…:33-64`（并注明原写 32 个 / `:20-51`） | `export const ST_EVENT_TYPES = Object.freeze({` 在 `:33`，收口 `})` 在 `:64`，键 30 个；下表恰好 30 行 |
| `:48` | `定义在 frontend/src/plugin-bridge.js:53-59：` | `:66-72` | `const ST_EVENT_ALIASES = {`(`:66`)…`}`(`:72`)，5 个键 |
| `:136` | `mapPipelineEventToPluginEvents (plugin-bridge.js:256-272)` | `(plugin-bridge.js:454-477)` | 函数定义 `:454`，其后首个顶层函数在 `:478` |

> G-09 建议的 `:29-60`/`:62-68` 也是旧值，实测按上表更正（未照抄）。

### 12.3 `docs/RELEASE-STATUS.md`（**入库**，G-15 口径，2 项）

- `:63` 原 `…复验 events=20、0 DataCloneError。` → 新：加"（2026-09-13 复核标注：该复验出自 `artifacts/plugin-acceptance-2026-09-06/rerun-1b/`——`summary.md`、`r5-plugin-frame-final.json`、`r4b..r4f-*-state.json` 的 console 收集窗口）"。
  - 依据：`rerun-1b/summary.md` 判据 a（`eventCount: "events: 20"`）与判据 b（5 个收集窗口 0 条 `DataCloneError`）；而 `rerun-1a/summary.md` 恰恰记录"宿主→插件事件通道整体失效、1003 条 DataCloneError"——即 `events=20` **不可能**出自 1a。
- `:64` 原 `复验硬证据：…全部 4 类上游请求…，hookCalls=changes=15；审计导出 integrity valid。` → 新：按证据来源拆开标注——**4 类上游请求 = `rerun-1a/`**（`r3-proxy-analysis.json`、`r3-hook-evidence.txt`；该轮 iframe 计数 hookCalls=6/changes=6）、**`hookCalls=changes=15` = `rerun-1b/`**、**审计导出 integrity valid = `rerun-1a/r4-audit-export.json`**。
  - 依据：09 §7.2 G-15 口径 + 两个 summary 实测（1a=6/6、1b=15）+ `rerun-1a/r3-proxy-analysis.json` 字段（`calls` / `bodies_with_marker` / `main_generation`）。

### 12.4 `docs/PRODUCT-REVIEW-2026-06-23.md`（未入库，1 项）

- `:194` 原文（Layer 2 示意图内）：`Layer 2: 原生插件 API（复用 tool_center）`。
- 新：**代码块原文保留**（历史稿），块后新增 `> 2026-09-13 注（task-28）`：`tool_center` 已删除（W-22：零生产调用者死模块 + 点记法"幻影工具名"），现役注册/按角色选配面为 `crates/app-agent/src/tools.rs` 的 `ToolRegistry`（`register_*_tools` + `ToolRegistry::retain`，`tools.rs:242`），并声明本文件是 2026-06-23 历史评审稿（`.gitignore:96`）。
- 依据：`Test-Path crates/app-agent/src/tool_center.rs`=false；`tools.rs:242 pub fn retain`。

### 12.5 `docs/DOCS-CODE-AUDIT.md`（未入库，0 项）

- 同源检查：**无额外失效行号**。全文 `lib.rs:<数字>` 引用 **0 处**；`tool_center` 仅 1 处命中，即本 task-16 写入的增量条目"`tool_center.rs` 已删（W-22）"——本身就是现状描述。**未修改该文件**。

### 12.6 交付与仍存在的风险

- 改动文件：`docs/REGRESSION-COVERAGE.md`（8 项 + 55 处行号重算）、`docs/ST-EVENTS-COVERAGE.md`（4 项）、`docs/RELEASE-STATUS.md`（2 项，**入库**）、`docs/PRODUCT-REVIEW-2026-06-23.md`（1 项）；`docs/DOCS-CODE-AUDIT.md` 经检查无需改。
- **仍存在的失效引用**：`docs/REGRESSION-COVERAGE.md` §7 保留的 10 个**已删除测试名**（已去掉行号、明确标注"已删除/不要再检索"）；其余引用全部指向存在的路径与正确行号。
- 未纳入：`REGRESSION-COVERAGE.md` 的 `:75+` 这类"范围标记"保留原样；`PRODUCT-REVIEW-2026-06-23.md` 只处理 `:194` 的 tool_center 残留（未做全文过期结论复核）。
- **未运行任何测试/门禁**（纯文档任务；所有依据均为 `rg`/`Select-String`/`Test-Path` 静态核对）。
- **vitest 数字的口径**（Lead 2026-09-13 回传）：首跑为 `2 failed | 29 passed (31)`，域6 修完两个测试文件后正在复跑；**最终数字由 Lead 写进 GATE-REPORT 与第二版报告**，本记录与入库文档均**不写 vitest 通过数**（只写"31 个文件"）。
- **vitest 收口结论（Lead 门禁回填，2026-09-13，本条为唯一回填处）**：`npm run test:ui` 最终 = **31 个文件 / 151 个测试 / 151 通过 / exit 0**（日志 `artifacts/review-2026-09-13-round2/fe-vitest.log`；汇总见 `fixes/GATE-REPORT.md`）。首跑的 `2 failed | 29 passed (31)` 已由域6（task-27→task-29）修复闭环，**该首跑数字作废**；入库文档（`README.md`/`RELEASE-STATUS.md` 等）本轮仍不新增 vitest 通过数。

# R6 复检：修复完整性与诚实性对账审计

- **任务**：task-23（owner `review-goals`），第二遍审查
- **日期**：2026-09-13
- **基线**：工作区 HEAD `ab894c6` + 本轮**未提交**修复工作树（197 行 status；`git diff --stat` = 187 files, +14438/−2471）
- **对账基准**：`FULL-REVIEW-REPORT.md`（§2.3 计数总表 = **194** 条：P0 3 / P1 44 / P2 100 / P3 47）+ 7 份分域报告 `01..07`
- **处置依据**：`fixes/01..09`（9 篇修复记录）+ `fixes/GATE-REPORT.md`（Lead 收口门禁）
- **约束遵守**：本任务**只读**（`Select-String`/`rg`/`Test-Path`/`node scripts/architecture/backend-baseline.mjs` 只读运行）；**未改任何源码、测试、文档**，只写本报告。**未**运行 `cargo test --workspace`（Lead 独占）。

---

## 0 方法与口径（先说清楚，再给数字）

1. **发现清单来自报告本身，不另造 ID**：从 `01..07` 的条目标题机械提取 `D/S/W/T/F/M/G-xx` 共 195 条，其中 `G-17`（"门禁数字无法用静态计数复现"）在 `09-fixes §6` 被判为"非缺陷、无需修改"，扣除后与 `FULL-REVIEW §2.3` 的 **194** 完全对齐。
2. **状态不采信自述**：每条先取其**所属域修复记录**里的处置（章节标题的 `— 已修复/已修复(降级)/判定非问题/暂缓/移交` 或结论摘要表），再对"移交"项**追到接收方记录**确认是否真的落地。
3. **五档口径**（本报告统一）：`已修复` / `降级`（含"已修复(部分)"）/ `非问题` / `暂缓` / `无记录`（无人处置，或"移交"后接收方无任何动作）。**"无记录"全部列出**（§2）。
4. **严重度以分域报告条目为准**；检出 1 处三方口径不一致（`03` 的 P2/P3 划分）记入 §7 N-R6-03，**不影响 194 总数**。
5. 域9 未入库文档（`.gitignore` 忽略）在表中以 `⚠️未入库` 标注。

---

## 1 逐条对账总表（194 条）

### 1.1 域1 domain / 基础 infra（D-01..D-26，26 条；`01-domain-infra-fixes.md`）

| ID | 级别 | 状态 | 依据 / 证据（简） |
|---|---|---|---|
| D-01 | P0 | 已修复 | `card_shell.rs` 字符边界守卫（`is_char_boundary` 命中）+ CJK 回归测试；Lead 修，域1 复核 |
| D-02 | P1 | 已修复 | `world_info.rs set_enabled` 同步 `extra.enabled`；01-fixes §2 |
| D-03 | P1 | 已修复 | infra-vector 不可恢复损坏 → `write_fence` 冻结（不再静默空库） |
| D-04 | P1 | 已修复 | 域1 三态语义 + 域3 改传真实 `story_clock`（R6 验真：`compile_turn_dossier(..., &ctx.story_clock)`） |
| D-05 | P2 | 已修复 | infra-vector 搜索顺序确定化 + 测试 |
| D-06 | P2 | 已修复 | `DEFAULT_STORY_CLOCK = "第1天"` 单一权威（R6 验真） |
| D-07 | P2 | 已修复 | `select_anchor_turns` `h_anchor==0` 边界（原 panic） |
| D-08 | P2 | 已修复 | `apply_stage_json` 空串不再覆盖用户字段 |
| D-09 | P2 | 已修复 | 未知 `before_node_id` fail-closed（不再退化成全量历史） |
| D-10 | P2 | 已修复 | `script_bytes` 只计真脚本源（`depth_prompt.prompt` 不计字节） |
| D-11 | P2 | 已修复 | 嵌套宏首个 `}}` 截断修复 |
| D-12 | P2 | 已修复 | `order` 乘法溢出不再静默回绕 |
| D-13 | P2 | 已修复 | `Exclusivity::Single` 组装期执行 |
| D-14 | P2 | 已修复 | `migrate_to` 不再降级版本标记（R6 验真：`target <= self.config_version`） |
| D-15 | P2 | 已修复 | `CardProject` 缺字段不再整份解析失败 |
| D-16 | P2 | 已修复 | 出卡闸门 `KNOWN_RULE_CODES` 白名单（未知 code 的 error 不被吞） |
| D-17 | P3 | 已修复 | `novel_distill` 空输入卡死 + 硬上限溢出 |
| D-18 | P3 | 已修复 | `history.rs` 补测试（**子项**：`variant_id` 改名暂缓，已记录） |
| D-19 | P3 | 已修复 | tail 指纹 `u64` 长度前缀 + builder 文档（R6 验真：跨版本不可比已写入 CLAUDE.md） |
| D-20 | P3 | 已修复 | 四处注释/文档漂移 |
| D-21 | P3 | 降级 | 死结构已删 + `api_key` 判非问题 + `Custom` 形状暂缓（混合，01-fixes §5） |
| D-22 | P3 | 降级 | 部分已修复、4 项特性脚手架暂缓、`CardShellKind::Other` 判非问题（混合） |
| D-23 | P3 | 已修复 | 死 Err 分支/映射重复/`as i32` 截断 |
| D-24 | P3 | 已修复 | `reverse_parse_character` 丢字段 + 阈值单位（**子项**：wire `deferred/byte_len` 跨域暂缓） |
| D-25 | P3 | 暂缓 | Rust/前端关键词表漂移属跨域；域1 侧文档已补 |
| D-26 | P3 | 已修复 | `RegexScript` 默认值对称化（刻意保留 2 处无默认） |

### 1.2 域2 存储与 Turn 生命周期（S-01..S-22，22 条；`02-storage-fixes.md`）

| ID | 级别 | 状态 | 依据 / 证据（简） |
|---|---|---|---|
| S-01 | P0 | 降级 | 不完整源目录 **fail-closed**（R6 验真：`readiness.rs:194 ImportSourceIncomplete`）；`import_runs` 无 skip 列（V009 跨域）→ 改走 warn + `storage_health` incident |
| S-02 | P1 | 已修复 | stale marker 判据不再靠自由文本子串 |
| S-03 | P1 | 已修复 | 孤儿库所有权探测收紧（不再自动重发布） |
| S-04 | P1 | 已修复 | JSON accept replay 确认持久化后才返回成功 |
| S-05 | P1 | 已修复 | JSON 路径补 `target == expected+1` 与 final_revision 校验 |
| S-06 | P1 | 暂缓 | 迁移期 JSON 拷贝吞错 → 跨域（`lib.rs`，域4） |
| S-07 | P1 | 已修复 | 启动恢复标记 `Failed` 失败不再静默 |
| S-08 | P2 | 已修复 | 发布前 checkpoint temp WAL / 发布后复检 |
| S-09 | P2 | 已修复 | marker-last 发布顺序（保持并加固） |
| S-10 | P2 | 已修复 | `journal_mode` 返回值不再丢弃（R6 验真：`connection.rs` expect_wal 断言） |
| S-11 | P2 | 已修复 | `AuthorityLeaseGuard::drop` 簿记与 OS 锁一致 |
| S-12 | P2 | 已修复 | rollback rename 后 fsync 失败不再打破"全有或全无" |
| S-13 | P2 | 已修复 | JSON 级联删除生效（不再"删后 get 恒 None"）+ 错误传播 |
| S-14 | P2 | 降级 | 吞错面收敛（未达报告完整方案，残余已写明） |
| S-15 | P2 | 已修复 | 快照 stale turn 覆盖 + 删除活动检查在途 Turn |
| S-16 | P2 | 已修复 | JSON/SQLite 语义差距（Replay 契约、正文读取、revision） |
| S-17 | P2 | 已修复 | 运行期 marker 复检（`validate_runtime_authority` 不再只比路径） |
| S-18 | P2 | 降级 | 缺口 1/2/3/4 补测试；5/6 两个弱覆盖与反模式点暂缓（已写明） |
| S-19 | P3 | 已修复 | 注释/文档漂移（**子项**：item6 跨域暂缓） |
| S-20 | P3 | 降级 | 死代码/误导代码 2/5 项降级处置（其余已删/已修） |
| S-21 | P3 | 已修复 | 恢复升级 `Failed` 时同步收口 Attempt |
| S-22 | P3 | 降级 | 22.1/22.4/22.5/22.8 已修复；22.7 判非问题；22.2/22.3/22.6 记为已知限制 |

### 1.3 域3 写作流水线（W-01..W-32，32 条；`03-pipeline-fixes.md`）

| ID | 级别 | 状态 | 依据 / 证据（简） |
|---|---|---|---|
| W-01 | P1 | 已修复 | `instance_name_matches`/`find_instance_normalized`（R6 验真：`app-agent/src/runtime.rs`）+ 测试 |
| W-02 | P1 | 已修复 | Plan 缺 `character_id` 不再静默写 `"unknown"` |
| W-03 | P1 | 已修复 | 整卷 reroll 保留 `generation_mode` |
| W-04 | P1 | 已修复 | 质量门禁误杀中文对白（R6 验真：`contains("让我来")` 已 0 命中） |
| W-05 | P1 | 已修复 | 后处理单条畸形条目不再整批丢弃 |
| W-06 | P1 | 已修复 | 归档水位线 fail-closed 连续前缀 |
| W-07 | P2 | 已修复 | 顺序班组失败传播（`SequentialActorOutcome`）+ 文档回写 |
| W-08 | P2 | 已修复 | `get_character` 参数/歧义 |
| W-09 | P2 | 已修复 | 空结果不再触发第二次完整 LLM 调用 |
| W-10 | P2 | 已修复 | broadcast 解析 |
| W-11 | P2 | 降级 | 上游已修；下游 tauri-app 写入门禁以"遗留"记录 |
| W-12 | P2 | 已修复 | MVU JS fallback 越权/未归一（含 M-04 跨域） |
| W-13 | P2 | 暂缓 | 摘要注入每子 Agent 需 `RoundSummary` 加受众字段（域1 schema + 迁移） |
| W-14 | P2 | 已修复 | 远记忆检索 campaign 过滤收紧 |
| W-15 | P2 | 已修复 | 归档并发/批次溢出 |
| W-16 | P2 | 已修复 | 局部 reroll 目标重复 |
| W-17 | P2 | 降级 | 前半已修；后半为设计意图（判定非问题，附证据） |
| W-18 | P2 | **无记录** | 03 移交"域5 前端 + 域2 命令层"；**两份记录均无任何动作**（R6 复核：`useWritingScreenAdapter.js:89`、`rerollPolicy.js:8` 仍按 `big_scene` 分支） |
| W-19 | P2 | 已修复 | 路径 C 子 Agent 配置硬编码 |
| W-20 | P2 | 已修复 | Editor 修订稿空/严重缩水不再落盘 |
| W-21 | P2 | 已修复 | 否定后肯定/私有泄漏 |
| W-22 | P2 | 已修复 | `crates/app-agent/src/tool_center.rs` 已删除（R6 验真：`Test-Path` = false） |
| W-23 | P2 | 已修复 | 数组/对象边界扫描 |
| W-24 | P2 | 已修复 | 取消不再报成 `SubagentFailed` |
| W-25 | P2 | 已修复 | 变量键跨实例串味 |
| W-26 | P3 | 已修复 | 文档漂移（移交 task-f7 → `08-fixes §3.1` 已改 `AGENT_INTERFACES.md` PipelineState） |
| W-27 | P3 | 降级 | 文档/多出现扫描已修；`owner_hint` 判非问题 |
| W-28 | P2 | 已修复 | 取消 → `PostProcessSkipped`（不再 `PostProcessFailed`）+ 文档回写 |
| W-29 | P2 | 已修复 | `llm_parse` 字节边界 panic / 静默丢参 |
| W-30 | P2 | 已修复 | 解析失败与"合法空"区分 |
| W-31 | P3 | **无记录** | 03 移交"域5 前端"；05 记录无该 ID，`08-fixes` 明示未改文档行（R6 复核：`stores/writing.js:16-20` 的 `validGenerationModes` **仍含 `big_scene`**） |
| W-32 | P3 | 已修复 | `format_subagent_context_*` 重复实现合并 |

### 1.4 域4 Tauri 命令层（T-01..T-16，16 条；`04-tauri-fixes.md`）

| ID | 级别 | 状态 | 依据 / 证据（简） |
|---|---|---|---|
| T-01 | P1 | 已修复 | 契约测试 invoke 扫描盲区（改子串匹配；真实唯一名 169→**172**→修复后 **152**） |
| T-02 | P1 | 已修复 | Card Studio 8 处落盘错误传播（R6 验真：`card_studio_api.rs` 已无 live `let _ = store.update(`） |
| T-03 | P1 | 已修复 | 移交域6 → `card_shell_fetch_url` 已 async + `spawn_blocking`（06-fixes） |
| T-04 | P1 | 已修复 | `add_variant` 参数契约（`provenance` 生效） |
| T-05 | P1 | 已修复 | 移交域6 → `register_doc/module` 结构化错误 DTO（06-fixes） |
| T-06 | P1 | 已修复 | 变量写命令键名校验（`__storyforge*` 后端守卫） |
| T-07 | P1 | 已修复 | `log_clear` 未知 kind 报错 + 显式 `"all"` |
| T-08 | P1 | 已修复 | 世界书四条写命令不再吞重读错误 |
| T-09 | P1 | 降级 | `card_shell_allow_host` 保留 + 标注（06-fixes；M-01 未闭合前仍是提权面） |
| T-10 | P1 | 已修复 | 契约测试接入 CI + 快照再生成（**未运行时验证**，保留干跑路径，未写成已验证） |
| T-11 | P2 | 降级 | 命令级测试覆盖：仅补 T-02 相关命令的落盘回归 |
| T-12 | P2 | 已修复 | 大载荷 import/export 命令转 async（R6 验真） |
| T-13 | P2 | 已修复 | 命令位置门禁（`COMMAND_LOCATION_ALLOWLIST`，违规 exit 1） |
| T-14 | P2 | 已修复 | 移交 task-16 → `CLAUDE.md` 陈旧条目已改（`08-fixes §1.3/§1.4`） |
| T-15 | P2 | 降级 | 三个孤儿命令**保留 + 声明**（Lead 裁定；零入口命令 3→23 已在基线脚本登记） |
| T-16 | P2 | 已修复 | 差异化入口口径（与 T-13 合并条目） |

### 1.5 域5 前端（F-01..F-47，47 条；`05-frontend-fixes.md`）

| ID | 级别 | 状态 | 依据 / 证据（简） |
|---|---|---|---|
| F-01 | P1 | 已修复 | 中文输入法回车不再直接提交（R6 验真：`ComposerBar.vue` isComposing/composition） |
| F-02 | P1 | 已修复 | 「采纳」失败不再静默（R6 验真：`useMessageVariants.js` alertDialog/errorText） |
| F-03 | P1 | 已修复 | 开场卡壳死状态 → `role="status"` |
| F-04 | P1 | 已修复 | 日志面错误对象经 `errorText` 渲染 |
| F-05 | P1 | 已修复 | `ui/DataList` `activeKey` 语义 |
| F-06 | P1 | 已修复 | `select` 事件受控列表接线 |
| F-07 | P1 | 已修复 | 「可能完成」任务解锁（行级 busy） |
| F-08 | P1 | 已修复 | 任务状态不再渲染 NaN%（R6 验真：`taskStatus.js` 读 `confidence`） |
| F-09 | P1 | 已修复 | 「设为当前活动」失败提示 + 不再误报 |
| F-10 | P1 | 已修复 | 活动列表加载失败不再伪装空态 |
| F-11 | P1 | 已修复 | 角色卡库失败不再被吞 |
| F-12 | P1 | 已修复 | 写卡工作室保存失败不再继续导入 |
| F-13 | P1 | 已修复 | 会话历史加载失败 + 缺 `updated_at` 容错 |
| F-41 | P1 | 已修复 | 提示词钩子审计「展开详情」`ReferenceError`（形参名） |
| F-14 | P2 | 已修复 | 05-fixes §2 条目（P2 全量） |
| F-15 | P2 | 已修复 | 同上 |
| F-16 | P2 | 已修复 | 同上 |
| F-17 | P2 | 已修复 | 同上 |
| F-18 | P2 | 已修复 | 9 个零引用基元删除（`ui/` 现 14 个 `.vue`，R6 验真） |
| F-19 | P2 | 降级 | 错误态收敛到已删除的 `ErrorState` → 改为面板内联"错误行+重试"（降级，条目部分失效） |
| F-20 | P2 | 已修复 | 复制态按下标存储（N-02 顺带修） |
| F-21 | P2 | 已修复 | 同上 |
| F-22 | P2 | 降级 | Select portal 回归（N-03 由域5 自检 harness 抓到，已修但降级处置） |
| F-23 | P2 | 已修复 | 05-fixes §2 |
| F-24 | P2 | 已修复 | 同上 |
| F-25 | P2 | 已修复 | 同上 |
| F-26 | P2 | 已修复 | 同上 |
| F-27 | P2 | 已修复 | 同上 |
| F-28 | P2 | 已修复 | 同上 |
| F-29 | P2 | 已修复 | 同上 |
| F-30 | P2 | 已修复 | 20 个孤儿 wrapper 删除 + 1 个接线（R6 验真：`tauri-api.js` export 计数 160→**140**）；3 个后端命令部分→域4 T-15 |
| F-42 | P2 | 已修复 | 「测试连接」延迟显示（R6 验真：`latency_ms` 口径统一） |
| F-43 | P2 | 已修复 | 配置面板补 `Writer` 角色 |
| F-44 | P2 | 已修复 | 流水线 trace 识别 `writer_*` |
| F-45 | P2 | 已修复 | `PresetPanel` 加载/展开失败 catch |
| F-31 | P3 | 已修复 | 演示 hash 路由清理 |
| F-32 | P3 | 已修复 | `ui/DataList` 空态/注释 |
| F-33 | P3 | 已修复 | `ui/DataTable` 死 emit/colspan |
| F-34 | P3 | 降级 | 列表分页/虚拟化未引入（降级，记录理由） |
| F-35 | P3 | 已修复 | `ui/Textarea` autoResize 首帧 |
| F-36 | P3 | 已修复 | `ui/CodeBlock` 定时器/剪贴板/遮挡 |
| F-37 | P3 | 已修复 | headlessui 语义缺口 |
| F-38 | P3 | 已修复 | a11y 汇总 |
| F-39 | P3 | 已修复 | 死状态与陈旧注释 |
| F-40 | P3 | 已修复 | 文档漂移（移交 task-f7 → `08-fixes` 已改 `FRONTEND-COMPONENTS.md` §3/§13、`ROADMAP.md`） |
| F-46 | P3 | 暂缓 | `components-v2/writing/**` 保留代码（Lead 裁定），文档措辞已回写为"仅存档参考" |
| F-47 | P3 | 降级 | debug/config 面板次要缺陷：按项目分组降级处置 |

### 1.6 域6 Meta/MVU/插件/Card Shell（M-01..M-32，32 条；`06-meta-plugin-fixes.md`）

| ID | 级别 | 状态 | 依据 / 证据（简） |
|---|---|---|---|
| M-01 | P0 | 暂缓 | 子帧 IPC 越权**主体未修**：完成 4 条静态守卫测试 + CSP 卫生 + 半径收敛；用 tauri/wry 源码证据否决"加 app ACL manifest"，需产品决策 + Windows 运行时 PoC |
| M-02 | P1 | 已修复 | 白名单 `\@` 绕过 → 换 `url::Url` 同源解析 + 测试（R6 验真：`Url::parse` 命中；`fn host_of` 只剩 1 处） |
| M-03 | P1 | 已修复 | typed patch ↔ Campaign 绑定（R6 验真：`stamp_patch_scope`/`stamp_campaign_scope`） |
| M-04 | P1 | 降级 | MVU JS fallback 保留命名空间守卫（Rust 侧过滤已加，语义未全量归一） |
| M-05 | P1 | 已修复 | 世界书 `position` 导出字符串（域1 承接，`position_for_export`） |
| M-06 | P1 | 已修复 | 保真闸门：`WireDrift`/`Normalized` 取代无条件 `Intentional`（域6 infra-import，S1） |
| M-07 | P1 | 已修复 | TavernHelper 桥校验 origin + parent 链归属 |
| M-08 | P1 | 降级 | prompt hook 结果补 `pluginId`/权限判定（前端传参 + 后端审计；非完整 gate） |
| M-09 | P2 | 已修复 | patch revision 陈旧校验对 LLM 提案生效 |
| M-10 | P2 | 暂缓 | JSON accept 无回滚（半提交）——需跨文件设计 |
| M-11 | P2 | 已修复 | 卡缺失不再降级为空 definitions（fail-closed） |
| M-12 | P2 | 已修复 | 变量写入 key 白名单（R6 验真：`RESERVED_VARIABLE_NAMESPACE`） |
| M-13 | P2 | 已修复 | legacy `meta_accept_patch` 假成功（含 task-25 补修） |
| M-14 | P2 | 暂缓 | JSON accept Turn 屏障 TOCTOU——需 store 暴露 turn store 或锁序重构 |
| M-15 | P2 | 降级 | MVU 运行期写边界：命令层键归一守卫；历史双记法不合并 |
| M-16 | P2 | 已修复 | MVU preview 键归一（域2 承接，`backend_workflows.rs`） |
| M-17 | P2 | 已修复 | 分析器 24K 判据改正文形态启发式（**子项**：收录门暂缓，Lead 裁"不放宽"） |
| M-18 | P2 | 暂缓 | MVU 交互作用域（域5 暂缓并通知，避免跨域并发写；提议不改） |
| M-19 | P2 | 已修复 | 域6 本域项（06-fixes 结论表） |
| M-20 | P2 | 已修复 | 已闭环（S3 子代理） |
| M-21 | P2 | 已修复 | prompt hook 结构校验 + 整链预算（`DEFAULT_PROMPT_HOOK_CHAIN_BUDGET_MS`） |
| M-22 | P2 | 已修复 | 域6 本域项（06-fixes 结论表） |
| M-23 | P2 | 暂缓 | 审计链死代码 + 前端审计不落盘 + 变量写入无审计（需跨域设计） |
| M-24 | P2 | 已修复 | 握手时序**真缺陷**（task-29 修）+ 反例探针修正 |
| M-25 | P2 | 暂缓 | 整卡 JSON 数字数组过 IPC（域5 一致暂缓，需签名协同） |
| M-26 | P2 | 已修复 | `position_as_i32` 穷尽 match（域1 承接） |
| M-27 | P3 | 已修复 | (c)(d)(e) 本域已修；(a)(b) 跨域/见 §10 |
| M-28 | P3 | 已修复 | (a)(b)(c) 前端已修；(d) 域1 |
| M-29 | P3 | 非问题 | 复核更正：`partitionShellMountsByTrust` 有测试；"无生产调用方"成立（部分修复） |
| M-30 | P3 | **无记录** | 06 记"跨域承接（task-f7）"，但 07/08/09 三份记录**均无该 ID 的任何处置** |
| M-31 | P3 | 已修复 | S3 子代理（含 `<style>` 正文残留真缺陷） |
| M-32 | P3 | 已修复 | 导入导出低危项（含 M-32.8/.9 由域1 落地） |

### 1.7 域7 目标与声明（G-01..G-20 计 19 条，`G-17` 非缺陷；`07/08/09-fixes`）

| ID | 级别 | 状态 | 依据 / 证据（简） |
|---|---|---|---|
| G-01 | P1 | 已修复 | `RELEASE-STATUS` HEAD/未提交修复自相矛盾（task-14） |
| G-02 | P1 | 已修复 | README 平台校验和承诺与实际发布物对齐（task-14） |
| G-03 | P1 | 已修复 | APK 通配符与实际资产名（task-14） |
| G-04 | P1 | 已修复 | README「当前状态」四项冲突（task-14） |
| G-05 | P1 | 已修复 | ARCHITECTURE/HANDOFF 与 RELEASE-STATUS 闭合状态（task-14） |
| G-06 | P2 | 已修复 | 远端 CI 只构建不跑门禁 + Pester 认证对象已停用（09 §2 脚本侧 + task-14 文档） |
| G-07 | P2 | **无记录** | 09 记"转交（未修）"给域4/Lead；R6 复核 `.github/workflows/release.yml` **仍无 tag↔版本校验步骤**（只有 `tag_name: ${{ github.ref_name }}`） |
| G-08 | P2 | 已修复 | `DOCS-CODE-AUDIT` 175 命令位置（task-16 `:11/:54`，R6 复核 175=156+19 成立） |
| G-09 | P2 | 已修复 | ST-EVENTS-COVERAGE 计数/行号（task-28 §12.2；R6 复核 30 个 + `:33-64`/`:66-72` 已改） |
| G-10 | P2 | 已修复 | ROADMAP §11.3 指针（task-16 改指真实文件 + 未入库说明） |
| G-11 | P2 | 已修复 | 第三次门禁运行补记（task-14） |
| G-12 | P2 | 已修复 | 校验和同名覆盖方向**定案写反**并改两处文档（09 §5 + task-14/16） |
| G-13 | P2 | 已修复 | 插件验收证据清单补全（task-14 `RELEASE-STATUS:91`） |
| G-14 | P2 | 已修复 | ROADMAP Phase 7 状态与判定（task-16，含验收①不可判定） |
| G-15 | P3 | 已修复 | RELEASE-STATUS 两轮复验指标分源标注（task-28 §12.3；实际行 `:63/:64`） |
| G-16 | P3 | 已修复 | ROADMAP Phase 8 时点注记 + 当前计数（task-16） |
| G-18 | P3 | 已修复 | REGRESSION-COVERAGE 行号与列说明（task-28 §12.1：55/73 漂移全部重算 + §7 ToolCenter 失效改写） |
| G-19 | P3 | 已修复 | RELEASE-CHECKLIST 历史段标注为快照（task-16） |
| G-20 | P3 | 已修复 | ARCHITECTURE-AUDIT MVU 结论加 2026-09-13 时点注记（task-16；**⚠️未入库**） |

### 1.8 总计数

| 域 | 总数 | 已修复 | 降级 | 非问题 | 暂缓 | **无记录** |
|---|---|---|---|---|---|---|
| 1 domain/infra | 26 | 23 | 2 | 0 | 1 | 0 |
| 2 存储与 Turn | 22 | 16 | 5 | 0 | 1 | 0 |
| 3 写作流水线 | 32 | 26 | 3 | 0 | 1 | **2** |
| 4 Tauri 命令层 | 16 | 13 | 3 | 0 | 0 | 0 |
| 5 前端 | 47 | 42 | 4 | 0 | 1 | 0 |
| 6 Meta/MVU/插件 | 32 | 21 | 3 | 1 | 6 | **1** |
| 7 目标与声明 | 19 | 18 | 0 | 0 | 0 | **1** |
| **合计** | **194** | **159** | **20** | **1** | **10** | **4** |

> 口径：`降级` 含"已修复(部分)"与"已修复(降级方案)"；混合条目（一条内含多个子项的多档处置）按**主体**归入一档，子项差异写入该行依据列。

---

## 2 「无记录」清单（本任务最重要的输出，共 4 条）

这 4 条**没有任何一份修复记录声称处理或验证过**；其中 2 条是"移交后接收方零动作"，1 条是"跨域承接但下游记录无此 ID"，1 条是"转交 Lead/域4 后未落地"。**它们不代表缺陷仍在（除 W-31/G-07 外多数影响有限），但代表本轮对账链断裂。**

| ID | 级别 | 原始发现 | 移交声明 | R6 独立复核的当前事实 | 影响 |
|---|---|---|---|---|---|
| **W-18** | P2 | `big_scene` 自动路由不可达 + 无成本确认（03 报告 W-18） | 03-fixes：移交"域5 前端 + 域2 命令层" | `05-frontend-fixes.md` 全篇无 `W-18`；`02-storage-fixes.md` 全篇无 `W-18`。工作树仍保留 `useWritingScreenAdapter.js:89`（`generationMode === 'big_scene'`）与 `utils/rerollPolicy.js:8` 的 big_scene 分支 | 中：legacy 模式的自动路由/成本确认路径保持审查时的现状；无新增风险，但**问题未被消除也未被声明为暂缓** |
| **W-31** | P3 | 前端 `validGenerationModes` 含 `big_scene`，与"旧路径仅兼容"的文档口径冲突（03 报告 W-31） | 03-fixes：移交"域5 前端" | `frontend/src/stores/writing.js:16-20` 的 `validGenerationModes` **仍含 `'big_scene'`**；`docs/AGENT_INTERFACES.md:12` 仍写 `big_scene` 为兼容档。`08-fixes §3.3` 明确"本轮未改该行" | 低：行为与文档仍不一致，属"护栏宣称强于实际"残余 |
| **M-30** | P3 | 域6 的文档漂移条目（06 报告 M-30） | 06-fixes：**跨域承接（task-f7）**，指向 06 §11"需文档同步条目" | `07-fixes`、`08-fixes`、`09-fixes` **均未出现 `M-30`**；task-16/17/28 的记录里也没有逐条核销 | 低：可能仍有域6 相关旧文档表述（M-01/M-04/M-10/M-23 的架构措辞在 `08-fixes §10-4` 被显式推迟给 Lead 定口径） |
| **G-07** | P2 | 发布工作流无 tag↔版本一致性校验（07 报告 G-07；09 §7.1 给了 6 行补丁草案） | 09-fixes：**转交（未修）**给域4/Lead | `.github/workflows/release.yml` 全篇无 manifest 版本比对步骤（仅有 `tag_name: ${{ github.ref_name }}`，`:230`/`:247`）；`04-tauri-fixes.md` 无 G-07 | 中：发布一个 tag 与 manifest 版本不一致时，工作流仍会构建并发布 |

> 另有 3 项**部分无记录**（不算独立条目，但需 Lead 知道）：`M-01/M-04/M-10/M-23` 的**架构安全措辞**（`ARCHITECTURE.md`）在 `08-fixes §10-4` 被显式推迟给 Lead 定口径；`docs/DOCS-CODE-AUDIT.md:18` 的 "vitest 通过数待回填" 在 `GATE-REPORT.md` 已给出 151 后仍未回填（见 N-R6-05）；`release.yml` 的 Release 正文模板/校验和分工由域4 修好后未再被任何记录复核（Lead 已口头确认无残留，R6 未复跑 release 流程）。

---

## 3 诚实性抽查（20 条，P0/P1 占 12/20 = 60%）

抽查方式：回到**工作树实际代码/测试**验证"改动真实存在且语义相符"，不看修复记录的自述。判定标准：`TRUE`=代码/测试确实存在且语义与声称一致；`COMMENT-ONLY`=只有注释/文档；`UNDELIVERED`=声称改了但代码里没有。

| # | ID | 级别 | 修复记录声称 | R6 独立验证（命令/位置） | 判定 |
|---|---|---|---|---|---|
| 1 | D-01 | P0 | 字符边界守卫 + CJK 回归测试 | `card_shell.rs` 命中 `is_char_boundary`；测试名含 CJK/边界 | TRUE |
| 2 | S-01 | P0 | 不完整源目录 fail-closed | `readiness.rs:194` `SqliteError::ImportSourceIncomplete("cards.json is missing…")`；`:170` `cards_file_present` | TRUE |
| 3 | M-01 | P0 | 4 条静态守卫 + CSP 卫生（主体暂缓） | 06-fixes 列出的守卫测试存在；记录**明确写"没有修好"** | TRUE（诚实降级） |
| 4 | D-02 | P1 | `set_enabled` 同步不变量 | `world_info.rs` `fn set_enabled` 命中 + 测试 | TRUE |
| 5 | S-03 | P1 | 孤儿库所有权探测收紧 | `cutover.rs` 相关守卫/测试存在 | TRUE |
| 6 | W-01 | P1 | `find_instance_normalized` | `app-agent/src/runtime.rs` 命中 `fn find_instance_normalized` + 单测 | TRUE |
| 7 | W-04 | P1 | 质量门禁不再按"让我来"误杀 | `quality_gate.rs` 中 `contains("让我来")` = **0 命中** | TRUE |
| 8 | T-02 | P1 | 8 处落盘错误不再吞 | `card_studio_api.rs` live `let _ = store.update(` = **0**（仅剩记录性注释） | TRUE |
| 9 | M-02 | P1 | 换 `url::Url` 同源解析 | `card_shell_cache.rs` `Url::parse` 2 命中；`fn host_of` 仅 1 处（非解析主路径） | TRUE |
| 10 | M-03 | P1 | patch 与 Campaign 绑定 | `typed_patch.rs` 命中 `stamp_patch_scope`/`stamp_campaign_scope` | TRUE |
| 11 | F-01 | P1 | 输入法 composition 处理 | `ComposerBar.vue` 命中 `isComposing`/`composition` | TRUE |
| 12 | F-02 | P1 | 采纳失败可见 | `useMessageVariants.js` 命中 `alertDialog`/`errorText` | TRUE |
| 13 | D-14 | P2 | `migrate_to` 不降级 | `agent_profile_config.rs` 命中 `target <= self.config_version` | TRUE |
| 14 | S-10 | P2 | WAL 硬断言 | `connection.rs` 命中 `journal_mode`/`expect_wal` | TRUE |
| 15 | W-22 | P2 | 删除 `tool_center.rs` | `Test-Path crates/app-agent/src/tool_center.rs` = **False** | TRUE |
| 16 | T-07 | P2 | `log_clear` 返回 Result + `"all"` | `commands/diagnostics.rs` `log_clear` 命中 | TRUE |
| 17 | T-12 | P2 | import/export 转 async | `commands/import_export.rs` 命中 `async fn export_campaign_bundle` | TRUE |
| 18 | M-12 | P2 | 保留命名空间白名单 | `typed_patch.rs` 命中 `RESERVED_VARIABLE_NAMESPACE` | TRUE |
| 19 | F-08 | P2 | NaN% 修复 | `utils/taskStatus.js` 命中 `confidence` | TRUE |
| 20 | F-42 | P2 | 延迟显示口径统一 | `ConnectionConfigPanel.vue` 命中 `latency_ms` | TRUE |

**抽查结论：20/20 `TRUE`，0 条 `COMMENT-ONLY`，0 条 `UNDELIVERED`——本轮不存在"谎报已修"或"空转"（只加注释、只加测试不改逻辑、改了没接上）。** 特别指出两条**主动承认未修/未验证**的记录值得肯定：`M-01`（P0 主体未修，明确写"没有修好，也不该被说成修好"）与 `T-10`（"已修复(**未运行时验证**)"，并禁止写成已验证）。

---

## 4 计数与文档一致性（R6 独立复算）

| 事实 | R6 实测值（本轮工作树） | 文档表述 | 一致性 |
|---|---|---|---|
| Tauri 命令总数 | **175** = `commands/*.rs` 156 + `card_studio_api.rs` 19（`backend-baseline.mjs` exit 0） | `README.md:30` 175；`DOCS-CODE-AUDIT.md:11/:30/:73` 175 | ✅ 一致 |
| 前端唯一 invoke | **152**（脚本运行时输出） | `DOCS-CODE-AUDIT.md:14`「由脚本输出（实测 152）」；`CLAUDE.md` 未硬编码 | ✅ 一致（**但 `:14` 的"172=漏扫假值"说法写反 → N-R6-02**） |
| 零入口命令（保留 API） | **23**（脚本打印名单，含 `card_shell_allow_host`/`get_active_agent_profile_config` 等） | `CLAUDE.md` 23 条保留 API；`DOCS-CODE-AUDIT.md:14` 3→23 | ✅ 一致 |
| `tauri-api.js` wrapper | 现存 **140** 个 `export async function`（删除 20） | `CLAUDE.md` 20 个删除 + 23 保留 | ✅ 一致 |
| 死组件 | `ui/` **14** 个 `.vue`（删 9 基元 + `Dialog.vue`）；`components-v2/writing/**` 8 个（零引用，暂缓保留） | `FRONTEND-COMPONENTS.md` §3 + §13（task-16 回写） | ✅ 一致 |
| `tool_center.rs` | 不存在 | `ROADMAP.md:77`/`CLAUDE.md`/`DOCS-CODE-AUDIT` 均写"已删除" | ✅ 一致 |
| 测试计数 | Rust **2165 passed**（99 套件，Lead 门禁）；node:test **532/532**；vitest **31 文件/151**；Pester **198** | `GATE-REPORT.md` 全部有；`README.md:25`/`RELEASE-STATUS.md:19-21` 仍是**上轮 1980/504/119** | ⚠️ 见 N-R6-04（刻意边界：本轮未提交） |
| 未入库文档 | `docs/` 下 **199 个文件，140 个被 `.gitignore` 忽略**（`docs/archive/**`、`docs/workstreams/**` 等整目录 + 顶层 22 个）；根 `CLAUDE.md` 亦被忽略 | `CLAUDE.md` 必读 A/B 分组已标注（task-16） | ⚠️ N-R6-01（见 §6） |
| 工作树 | `git status --porcelain` **197 行**；`git diff --stat` **187 files, +14438/−2471**；未提交 | `GATE-REPORT.md` 与本报告一致 | ✅ 一致 |

**未发现"文档把未验证规划写成已完成"的新增案例**；`RELEASE-STATUS` 的"三轮门禁计数完全一致"经核对为 2026-09-06 已提交基线的事实陈述，与本轮未提交改动不矛盾（但需在 commit 后同步，见 N-R6-04）。

---

## 5 N-R6-01：未入库文档问题（Lead 发现的核实与修正）

- **Lead 的原始表述**："`docs/` 27 个文件被 `.gitignore` 忽略，其中含 CLAUDE.md 必读顺序第 1/3 项 `DOCS-CODE-AUDIT.md`/`ARCHITECTURE-AUDIT.md`"。
- **R6 精确核实**：
  - `docs/` 下共 **199 个文件**，`git check-ignore` 判定 **140 个被忽略**（整目录：`docs/archive/**`、`docs/workstreams/**`、`docs/operations/**`、`docs/superpowers/**`、`docs/效果预览/**`；顶层 22 个文件：`ARCHITECTURE-AUDIT.md`、`AUDIT-FIX-*.md`、`B1-GUI-*`、`CODE-ARCHITECTURE.md`、`DOCS-CODE-AUDIT.md`、`FRONTEND-*`、`FULL-AUDIT-REPORT.md`、`HANDOFF.md`、`HARNESS-FINDINGS-*`、`PHASE8-FOLLOWUP-ISSUES.md`、`PLAN-*.md`、`PRODUCT-REVIEW-2026-06-23.md`、`REGRESSION-COVERAGE.md`、`ST-EVENTS-COVERAGE.md`、`VISUAL-REDESIGN-*`）。
  - 根目录 `CLAUDE.md` 亦被忽略（`.gitignore:79`）。
  - "27" 更接近**顶层条目数**（22 文件 + 4 目录 = 26 条 `git status --ignored` 条目），**不是被忽略文件总数**；建议对外统一采用"199 中 140"这一口径。
- **影响（R6 判定：真实且已部分缓解，不是新的产品缺陷）**：
  1. 必读顺序若把未入库项列为"权威"，新克隆会拿到不存在的文件 → **task-16 已把必读顺序拆成 A（入库权威）/B（本机笔记）**，本项**已缓解**；
  2. 本轮 9 篇修复记录里引用 `DOCS-CODE-AUDIT.md`/`REGRESSION-COVERAGE.md` 等的地方，新克隆同样看不到；
  3. 审计本身（含本报告）位于 `docs/review-2026-09-13/**`，**未被忽略**，但**未提交**——若只提交源码不提交报告，审查结论将无法随代码交接。
- **处置建议（二选一，不擅自实施）**：
  - **A（推荐，成本最低）**：维持忽略策略，但把"跨人交接只引用 A 组文档"写成硬规则（task-16 已做），并在 commit message / GATE-REPORT 声明"审查报告与修复记录随本次提交一并入库或另行归档"；
  - **B**：把 `docs/review-2026-09-13/**` 与 `GATE-REPORT.md` 纳入提交（它们不在 `.gitignore` 内，只需 `git add`），同时**不要**动 `.gitignore`（Lead 已裁定 N-R6-01 不扩 ignore 面）。

---

## 6 新发现（N-R6-02..N-R6-08）

| ID | 严重度 | 位置 | 发现 | 证据 | 建议 |
|---|---|---|---|---|---|
| **N-R6-02** | P3 | `docs/DOCS-CODE-AUDIT.md:14`（⚠️未入库） | **数字语义写反**：写"此前 172 为漏扫假值"，实际 169 才是基线正则漏扫的**低估值**、172 是**真实唯一 invoke 数**（152 = 172 − 删掉的 20 个 wrapper） | `04-tauri-commands.md:51-52`、`:117`（"基线正则 169 / 子串匹配真实 172 / 差集恰为 3 个 `card_shell_*`"）；`FULL-REVIEW-REPORT.md:202/384` | 改为"脚本旧正则漏扫得 169；真实 172；删 20 个 wrapper 后 152"（一行） |
| **N-R6-03** | P3 | `docs/review-2026-09-13/03-writing-pipeline.md:24` | **报告内部计数三方不一致**：§2 写"P2 21 / P3 8"（6+21+8=35 ≠ 32，且与自身条目冲突）；条目标题实为 P2 19 / P3 7（=32）；`FULL-REVIEW §2.3` 记 P2 20 / P3 6（=32） | 机械统计 03 报告 32 个标题的 `[Pn]` 标记 | 以条目为准修正 §2 数字；**总数 194 不受影响** |
| **N-R6-04** | P3 | `README.md:25`、`docs/RELEASE-STATUS.md:19-21/:36` | 仍写**上轮**门禁数字（Rust 1980 / 前端 504+119+9+28+2 / Pester 198），本轮实测为 **2165 / 532+151 / 198** | `fixes/GATE-REPORT.md` §1；`README.md:25`；`RELEASE-STATUS.md:19` | 属"文档描述已提交基线、本轮未提交"的**刻意边界**；建议 commit 时同步，或在 GATE-REPORT 里显式声明"文档中的门禁数字指 `ab894c6`" |
| **N-R6-05** | P3 | `docs/DOCS-CODE-AUDIT.md:18`（⚠️未入库） | 仍写"vitest 通过数由 Lead 收口门禁确认后回填"，`GATE-REPORT.md` 已给出 **31 文件/151 通过/exit 0** | 文档 :18 vs GATE-REPORT | 回填一行（`08-fixes §12.6` 已记录该口径） |
| **N-R6-06** | P2 | `docs/AGENT_INTERFACES.md:12`（入库）+ `frontend/src/stores/writing.js:16-20` | W-31 未落地：文档写 `big_scene` 为"旧并行剧组，仅保留后端兼容路径"，而前端 `validGenerationModes` **仍是合法集合成员**（判定式护栏与声明不一致） | `stores/writing.js:16/20/27/67`；`AGENT_INTERFACES.md:12` | 二者取一：前端移出集合，或文档改为"兼容期内仍是合法模式" |
| **N-R6-07** | P3 | `.github/workflows/release.yml` | G-07 未落地：发布工作流仍无 tag↔manifest 版本一致性校验（`09-fixes §7.1` 已给 6 行补丁草案） | `release.yml` 全篇无版本比对步骤；`09-fixes:160` 状态"转交（未修）" | 采纳 §7.1 草案（Lead/域4），或显式记为"不做" |
| **N-R6-08** | P3 | `docs/REGRESSION-COVERAGE.md:§7`（⚠️未入库） | 由 task-28 处理后的**残留**：该节保留 10 个已删除测试名（已去掉行号并标注"不要再检索"），但**没有 1:1 的现役测试映射** | `08-fixes §12.1/§12.6` | 保持现状即可（已声明）；若需可补"现役对应测试名"一行 |

---

## 7 仍未闭合的高危项（供 Lead 收口决策）

| 项 | 级别 | 状态 | 说明 |
|---|---|---|---|
| **P0-2 / M-01（子帧 IPC 越权）** | P0 | **暂缓（主体未修）** | 4 条静态守卫 + CSP 卫生 + 半径收敛；"加 app ACL manifest"被 tauri/wry 源码证据否决；真修复需产品决策 + Windows 运行时 PoC。**这是本轮 194 条里唯一未闭合的 P0。** 与 T-09（`card_shell_allow_host` 保留）叠加构成 IPC 攻击面 |
| G-07 tag↔版本校验 | P2 | 无记录 | 见 N-R6-07 |
| W-18 / W-31 | P2 / P3 | 无记录 | 见 §2 |
| M-10 / M-14 / M-23 / M-25 / M-18 / S-06 / W-13 | P2/P1 | 暂缓（已写明理由） | 均为跨域写作用域或锁序/产品决策类，修复记录给出了补丁方向 |
| F-46（`components-v2/writing/**` 回退树） | P3 | 暂缓 | 代码保留、文档已改为"仅存档参考、不承诺可回退"；该树若真要回退**等于不可用**（F-46 原始结论），保留只是历史存档 |

---

## 8 局限与诚实声明

1. 本报告**未运行** `cargo test --workspace`、`npm test`、`npm run test:ui`、Pester（Lead 独占/成员侧沙箱 EPERM）。所有"已修复"的判定为**静态验证 + 记录交叉核对**；动态门禁结论引用 `GATE-REPORT.md`，R6 只独立复跑了 `node scripts/architecture/backend-baseline.mjs`（exit 0）。
2. 逐条对账的"处置"以修复记录的**自述**为起点，但 20 条抽查已回代码验真（20/20 TRUE）；未抽查的 174 条存在"记录声称与代码细节不完全一致"的残余风险（尤其是**降级方案的具体边界**）。
3. "无记录"的判定基于 9 篇 `fixes/*.md` 的全文检索；若某处置写在**未入库或未纳入检索范围**的文件里（例如 Lead 的私有笔记），可能被本报告低估。
4. 本报告的所有数字均为**未提交工作树**的状态；一旦 commit/进一步改动，行号与计数需重算（`REGRESSION-COVERAGE.md` 的教训）。
5. 审查过程全程只读；**未改动任何源码/测试/文档**（本报告本身除外）。

**报告结束。**

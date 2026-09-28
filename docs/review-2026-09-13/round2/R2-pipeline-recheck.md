# R2 复检报告：域3 写作流水线（app-agent / app-pipeline / app-conversation / app-memory）

- **任务**：`task-19`（R2 复检：域3 写作流水线），复检人 `review-domain`（域1）
- **被检对象**：`docs/review-2026-09-13/03-writing-pipeline.md`（首轮发现）与 `docs/review-2026-09-13/fixes/03-pipeline-fixes.md`（首轮修复记录：自称 25 已修 / 2 降级 / 1 部分 / 1 暂缓 / 3 转出）
- **方法**：读修复记录 → `git diff ab894c6 -- crates/app-*` 逐条回源 → 自造反例/边界 → 复跑包级测试（**未跑** `cargo test --workspace`，Lead 独占；**未跑** npm，沙箱 EPERM）
- **代码状态**：冻结工作树（`HEAD` = `ab894c6`；`git diff ab894c6 --stat -- crates/app-*` = 16 文件 +2465/−605）
- **只读声明**：未修改任何源码/文档，只写本报告。

---

## 1 结论摘要

**一句话判断**：P1 六条**全部真实落地且方向正确**；但记录的"已修复 25 条"有**实质性水分**——**1 条虚报（W-24：条目答非所问，原发现原封未动）**、**4 条过度声明（W-08 / W-12 / W-14 / W-30 只修了一半）**，另有 2 条转出完全泄漏（W-18 / W-31）、1 条半泄漏（W-26），以及一条首轮移交未被任何域认领的下游条目（W-11 下游）。**无 P0**。

### 状态总表（域内 32 条，与记录的 25+2+1+1+3 对齐）

| 记录口径 | 条数 | 复检后真实状态 |
|---|---|---|
| 已修复 | 25 | **确认关闭 20**（W-01..W-07、W-09、W-10、W-15、W-16、W-19..W-23、W-25、W-28、W-29、W-32）+ **部分关闭 4**（W-08、W-12、W-14、W-30）+ **未关闭 1**（W-24，记录条目答非所问） |
| 降级 | 2 | W-11：上游确认关闭、**下游未接住**（N-R2-07）；W-27：确认（含 W-21/W-27 归属互换，代码无问题） |
| 部分 | 1 | W-17：前半确认关闭；**后半"判定非问题"证据错配**（N-R2-13） |
| 暂缓 | 1 | W-13：暂缓理由与代码一致（`RoundSummary` 无 audience 字段、摘要块注入所有子 Agent），但**无 owner** |
| 移交 | 3 | W-26 **半接住**（N-R2-08）；W-18 **泄漏**（N-R2-01）；W-31 **泄漏**（N-R2-09，且 08 记录存在虚假归属） |

- **新发现**：**N-R2-01..N-R2-14**（§5；P2×7、P3×7）
- **门禁复跑**：agent **130** / pipeline **138**（quality_gate 子集 24）/ memory **12** / conversation **23**，**均 0 failed**，与记录数字一致

---

## 2 P1 六条逐条验证（含自造反例/边界）

### W-01 身份归一一致性 → **确认关闭**（非 ASCII 残余见 N-R2-10）

- 代码：`crates/app-agent/src/runtime.rs:607-639` 新增 `instance_name_matches`（trim + `eq_ignore_ascii_case`）与 `find_instance_normalized`（id 精确 → id 忽略大小写 → name 归一）；`:698-700` 的 `spawn_subagents` 确实改用它。
- **反例（改前会错、改后不错）**：`character_id = "  INST-LIN  "` / `"Inst-Lin"`：改前 `find_instance_by_id_or_name`（精确、不 trim）→ None → 子 Agent 退回 `context_package` 旧路径，丢失 persona/知识/变量注入；改后命中 `inst-lin`。测试覆盖三种形态 + `""`/`"   "`/`"Nobody"` → None（空输入 fail-closed，不会误配首实例）。
- 边界：仅在 `campaign_runtime.is_some()` 时走新路径（`:698-704`），**非 Campaign 路径零影响** ✓。
- 残余 → N-R2-10。

### W-02 `parse_plan_json` 畸形/重复任务 → **确认关闭**

- 代码：`crates/app-pipeline/src/lib.rs:4130-4159`：空白/缺失 `character_id` → warn + 丢弃（不再生成 `"unknown"` 幽灵实例）；按小写 id 去重保留首个。
- **反例**：5 条输入（2 缺 id、1 空白 id、1 重复 id）→ 2 条有效；改前会有含 `"unknown"` 的幽灵实例并注入 persona。测试 `parse_plan_json_skips_blank_character_id_and_dedups`（5→2）钉住。
- 边界：全任务被丢弃时 `subagent_tasks` 为空、`scene_brief` 仍在，下游 `shared_subagent_stage_guards_all_failed_but_allows_empty_plan` 允许空计划 ✓。取舍（漏 `character_id` 的 ad-hoc 角色会被丢）建议写入记录。

### W-03 reroll `generation_mode` → **确认关闭**

- 代码：`lib.rs:4104-4109` `effective_reroll_mode = requested.or(previous).cloned()`；`:1710-1733` 用 `effective_mode` 分派。
- **diff 反证（无新收紧）**：`git diff` 显示守卫 `provenance_old.generation_mode != Some(BigScene)` **改前已存在**，本次只把 `req.generation_mode` 换成 `effective_mode`；"老 provenance（None）+ 局部重跑 → 报错"是既有行为，非新回归 ✓。修复点是：请求 `None` + 旧 provenance 为产品模式时不再掉回 legacy BigScene；`None+BigScene` 仍保持 legacy（测试 4 组断言）。

### W-04 质量门禁误杀正常中文对白 → **确认关闭（精度优先）**；残余 1 例（N-R2-14）

- 代码：`crates/app-pipeline/src/quality_gate.rs:101-142`：`STRICT_PATTERNS` 恒 Error；`AMBIGUOUS_PATTERNS`（让我来/好的，我/没问题，我/我来写）仅当 **不在引号对白内** 且匹配点后 16 字含 `META_CUES`（为你/为您/以下/创作/写作/要求/正文/内容）时 Error；行首规则已删除。
- **第 1 组：正常中文对白**
  - 带引号：`「让我来！」…「好的，我这就去。」…「没问题，我可以等。」` → 不报（`inside_dialogue` 计 `「」『』""''` 深度）✓
  - 无引号：`"让我来帮你"` / `"好的，我这就去"` / `"没问题，我马上到"` → 无 Error ✓（改前均在行首 → 旧 `at_line_start` 规则判 Error，且 `domain/turn.rs:517 has_errors() && !force` 会阻断采纳）→ 该测试在旧实现下必失败，是有效回归锁。
- **第 2 组：真元描述**
  - `"作为AI，我来帮你润色这一段。"`、`"以下是故事正文。"` → Error ✓
  - `"让我来为你安排这一章的节奏。…"`、行中含线索的变体 → Error ✓
- **实测**：`cargo test -p storyforge-app-pipeline --lib quality_gate` = **24 passed / 0 failed**（三条 W-04 测试在列）。
- 残余 → N-R2-14。

### W-05 后处理单条畸形条目 → **确认关闭**

- 代码：`crates/app-agent/src/postprocess.rs:245-296`：DTO 三数组为 `Vec<Value>`；`parse_entries`（`:256-271`）逐条解析，失败 warn + 跳过；`postprocess_dto_from_value` 要求 ≥1 已知键，非数组字段 warn + 该类别空。
- **反例**：`["不是对象",{合法条目}]` → 保留 1 条（改前整批反序列化失败 → 该类别全丢）；`{"knowledge_updates":{...}}`（非数组）→ 空且仍算解析成功，不触发多余 LLM 调用 ✓。

### W-06 归档水位线 → **确认关闭（fail-closed 连续前缀）**

- 代码：`crates/app-memory/src/archiver.rs:185-209` 按 `batch_idx` 收集，遇首个缺口 warn + `break`；`:211-262` embed/upsert 失败同样 `truncate` 到失败点，仅入库成功才置 `summary.vector`。
- **边界（关键）**：缺口之后的批次**从未进入** embed/upsert 循环（循环跑在截断后的 `summaries` 上）→ 不存在"未返回但已入库"的孤儿向量；调用方 `crates/tauri-app/src/commands/conversations.rs:158-167` 以 `max(source_range.1)+1` 推进水位，前缀语义下不会跳过缺口 ✓。
- `max_concurrency=0` 被 `.max(1)` 钳制（否则 `buffer_unordered(0)` 永久挂起）✓。

---

## 3 域内非 P1 状态表（W-07..W-32）

| ID | 记录状态 | **复检结论** | 关键证据 |
|---|---|---|---|
| W-07 | 已修复 | 确认关闭（事件层残余 → N-R2-11） | `sequential_crew.rs:168-198` 三态 + `:264/269/368`；`lib.rs:1339-1360` 分派；测试 `cancel_during_attempt_is_reported_as_cancelled_not_subagent_failed`、`scene_close_marks_remaining_actors_as_skipped` |
| W-08 | 已修复 | **部分关闭**（域侧同名静默取首个 + 3 处生产调用未修/未声明 → N-R2-12） | `tools.rs:383-420` 工具侧已修 ✓；但 `campaign_runtime.rs:50-56` 仍"同名取首个"，`app-conversation/src/lib.rs:951`、`app-meta/src/meta_conversation.rs:546/606/655` 仍用它绑定身份 |
| W-09 | 已修复 | 确认关闭 | `postprocess.rs:86-93` 仅 `!parse_succeeded` 才 fallback；测试断言 `fallback_calls == 0` |
| W-10 | 已修复 | 确认关闭 | `postprocess.rs:352-361` trim/小写/全体映射；测试 3 组（Group 零命中静默见 §6 低危） |
| W-11 | 已修复（降级） | **部分关闭**：上游确认（`postprocess.rs:111-167` 唯一命中才改名，生产可达）；**下游未接住** → N-R2-07 | `production_postprocess.rs:1149-1157` 三处 fail-open 原样保留；域2 记录零认领 |
| W-12 | 已修复 | **部分关闭**（原发现前半未修 → N-R2-03） | 已做 `lib.rs:4027-4087` + 接线 `:1562-1576` ✓；未做：`present_characters` 仍只进提示词（`prompts/postprocess.rs:244-251`），`PostProcessOutcome`（`pipeline_postprocess.rs:28-38`）无"已过滤/作用域"契约字段 |
| W-13 | 暂缓 | 暂缓成立，**无 owner** | `domain/agent.rs:616-646` 无 audience；`runtime.rs:733-741` 同一摘要注入所有子 Agent |
| W-14 | 已修复 | **部分关闭**（原发现前半未修 + 注释与实现矛盾 → N-R2-04） | 已做 `recall.rs:170-186` fail-closed ✓；未做：召回无实例维度（`recall.rs:145-150` 仅 campaign 参数，`filter_archived_hits` 第 4 参是 `min_score` 非角色）；`recall.rs:134-135` 注释仍承诺"无标签旧记录也返回" |
| W-15 | 已修复 | 确认关闭 | `archiver.rs:134 saturating_mul` / `:183 .max(1)`；零并发测试 5s 超时防挂起 |
| W-16 | 已修复 | 确认关闭 | `app-conversation/src/lib.rs:825-851` 规格化去重 + plan 交叉校验；测试覆盖重复/未知/合法 |
| W-17 | 部分 + 判定 | **部分关闭**：前半（末节点断言）确认；后半"非问题"**证据错配** → N-R2-13 | `app-conversation/src/lib.rs:538-556` ✓；所引 `lib_tests_turns.rs:320-343` 实际断言 `VariantStatus::Final`，不支持"有意非终态" |
| W-19 | 已修复 | 确认关闭 | `lib.rs:3610-3631` profile→默认回退；测试断言 10 轮/导演模型 + `Subagent("*")` 覆盖 |
| W-20 | 已修复 | 确认关闭 | `draft_revision.rs:4-24`（空稿拒；≥200 字且 <25% 拒）；`:103-113` 返回 `InvalidState`，原稿不被覆盖 |
| W-21 | 已修复 | 确认关闭 | `quality_gate.rs:436-465`（`has_other` 仅在 `!has_owner` 分支）+ 全量扫描；测试正反例齐全（归属互换见 §6） |
| W-22 | 已修复 | 确认关闭 | `crates/app-agent/src/tool_center.rs` 已删；全仓仅注释/测试残留；`ROADMAP.md:77` 已更正 |
| W-23 | 已修复 | 确认关闭 | `character_extractor.rs:256-309` 配平 + `chronicle_compressor.rs:139-168` 首个配平数组；测试含反例 |
| W-24 | 已修复 | **未关闭（记录条目答非所问）→ N-R2-02** | 原发现 `03-writing-pipeline.md:604`：熔断是死字段；修复记录 `fixes/03-pipeline-fixes.md:260-263` 写的却是"取消被报成 SubagentFailed（同 W-07）"；`sequential_crew.rs:103-107` 注释自认"生产路径只 record 不读"，`:149-157` `failure_count`/`should_stop_actor` 仍 `#[cfg(test)]` |
| W-25 | 已修复 | 确认关闭（无提示词副作用） | `build_current_variables`（`lib.rs:3285-3317`）只服务 `rt.execute_fragment`（`:1533-1546`），不进提示词；别名键纯新增；写回反解要求实例真实存在 |
| W-27 | 已修复+判定 | 确认关闭 | 与 W-04 同处删除 `at_line_start`；`owner_hint` 非死代码的判定可接受（构建与消费点均在 `quality_gate.rs` 内） |
| W-28 | 已修复 | 确认关闭（**引入双发事件** → N-R2-05） | `lib.rs:1505` cancel_probe、`:1627-1644` 三态；测试 `test_postprocess_cancelled_emits_skipped_not_failed` |
| W-29 | 已修复 | 确认关闭 | `llm_parse.rs:24 content.get(pos..)?`、`:164-178` 坏 args warn 后继续扫描；测试非空转 |
| W-30 | 已修复 | **部分关闭**（① 已修；② 双导出路仍在；③ 调用侧仍混淆 → N-R2-06） | ① `character_extractor.rs:68-73` ✓；② `prompts/mod.rs:11-13` 与 `lib.rs:28-31` 双路 re-export 仍在；③ `character_extractor.rs:76-78` 把合法空转成 `Err`，`commands/campaigns.rs:166-169` 见 Err 一律降级单角色卡 |
| W-32 | 已修复 | 确认关闭 | 全仓 `format_context_stable/volatile` 仅 `app-agent/src/runtime.rs:1000/1024` 一处实现 + pipeline 委托（`:4346-4356`）；措辞"逐字节一致"应改为"输出等价"（见 §6） |

---

## 4 转出/移交核验（任务点名 3 条 + 我补的第 4 条）

| 条目 | 移交给 | 接住了吗 | 判定 |
|---|---|---|---|
| W-18（P2）big_scene 自动路由/成本确认不可达 | 域5 + 域2 | **否** | N-R2-01 |
| W-26（P3）`AGENT_INTERFACES.md` 漂移 | task-f7 | **半接住** | N-R2-08：`PipelineState` 那条已补（`08-docs-sync-fixes.md:106-110`）；变体清单仍缺 `QualityChecked`、`PromptHookRequest`（文档 grep 零命中 vs `agent.rs:437/448`），`PostProcessStarted` 缺字段 |
| W-31（P3）前端 `validGenerationModes` 含 `big_scene` | 域5 | **否** | N-R2-09：`frontend/src/stores/writing.js:20` 仍在；`05-frontend-fixes.md` 全文零命中；`08-docs-sync-fixes.md:123` 却称"域5 记录称其已在前端侧修复"——**虚假归属，被源码与域5 记录双重反证** |
| **W-11 下游**（首轮 §6 表移交域2） | 域2 | **否** | N-R2-07：`02-storage-fixes.md` 除作用域声明外零记录；注意该文件**在**域2 声明的写作用域内，属可执行落点，非派单错误 |

> **与任务板对账（2026-09-13 复检时状态）**：W-18、W-32 已有收口任务 **task-32**（`R9-unrecorded-pipeline.md`，pending/ready/未认领）；W-31 已有 **task-33**（`R10-unrecorded-frontend.md`，pending/ready/未认领）。即"转出未被别域接住"这一事实成立，但**Lead 已在 R6 对账中另行建单**——本报告不要求重复建单，只建议在 task-32/33 完成前不要把 W-18/W-31/W-32 记为"已收口"。W-11 下游（N-R2-07）**没有**任何对应任务，属真正无人认领项。

---

## 5 新发现（N-R2-xx，均含 file:line + 严重度）

### N-R2-01 [P2] `big_scene` 自动路由与成本确认在生产路径不可达（W-18 泄漏 + 派单落点错误）
- **位置**：`frontend/src/composables/useWriting.js:141`、`frontend/src/stores/writing.js:57-62`、`crates/domain/src/generation.rs:61-68`、`crates/tauri-app/src/commands/writing.rs:927-936`
- **证据（逐点自读）**：`generationMode` computed 在未手动改档时返回 `'continuation'`（**永不为 null**）→ `startWriting` 恒带显式档位 → `route_generation_mode` 在 `explicit_mode.is_some()` 时立即返回 `ExplicitChoice` / `requires_cost_confirmation=false` → `enforce_generation_cost_confirmation` 永不触发；`generation.rs:70-83` 的自动升档三分支只被单测覆盖。
- **影响**：用户永远拿不到"多角色大场面自动升档 + 成本确认"；相关信号构造与守卫（`writing.rs:695-869`）为仅测试可达。原判 P2 维持。
- **建议**：① 前端"未手动改档"时传 `null` 让路由生效；或 ② 正式记录该能力降级并删除半接线代码与测试。**派单**：域2 作用域**不含** `commands/**`，需改派域5（前端）+ 域4（命令层）或由 Lead 裁定。**已有 task-32 承接**（W-18+W-32，pending/未认领）。

### N-R2-02 [P2] W-24 实为**未关闭**，修复记录条目答非所问（虚报"已修复"）
- **位置**：`docs/review-2026-09-13/03-writing-pipeline.md:604`（原发现）vs `docs/review-2026-09-13/fixes/03-pipeline-fixes.md:13/260-263`（记录）；代码 `crates/app-pipeline/src/sequential_crew.rs:103-107`、`:149-157`、`:640-646`
- **证据**：记录 W-24 条目标题是"取消被报成 SubagentFailed"，正文写"同 W-07"——这是 W-07 的内容；原发现"顺序剧组熔断是带观测语义的死字段"**一字未动**：`failures` 只在生产 `record_failure` 写入、从不读取，`failure_count`/`should_stop_actor` 仍 `#[cfg(test)]`，唯一测试 `actor_is_stopped_after_two_failures` 仍在自证测试专用函数。
- **影响**：`fixes/03-pipeline-fixes.md:13` 的"已修复 25"表格把 W-24 计入 → 记录可信度问题；且"生产有熔断"的假信心仍在。
- **建议**：把 W-24 从"已修复"移出（改为"未关闭/待裁定"），并决定：接入主循环熔断，或删除死字段 + 改注释 + 删自证测试。

### N-R2-03 [P2] W-12 只修了后半：`present_chars` 仍只存在于提示词，`PostProcessOutcome` 无过滤契约
- **位置**：`crates/app-agent/src/prompts/postprocess.rs:244-251`（`present_characters` 只拼进提示词）、`crates/app-agent/src/pipeline_postprocess.rs:28-38`（`PostProcessOutcome` 仅 summary/post_process/两个 attempted 标志）
- **影响**：原发现第一半（"app-agent 侧无校验"）未变；未来 outcome 消费者仍须自行重实现在场门禁，且无契约字段可断言"已过滤/带作用域"。
- **建议**：要么补 outcome 的作用域/过滤契约字段 + 消费端校验，要么在记录里明确"仅 MVU 键通道修复，`present_chars` 门禁保留在落盘层（域2）"。

### N-R2-04 [P2] W-14 只修了后半：召回仍无角色维度；且注释与 fail-closed 实现直接矛盾
- **位置**：`crates/app-memory/src/recall.rs:144-159`（签名无实例/角色参数；`filter_archived_hits` 第 4 参是 `min_score`）、`:134-135`（注释）vs `:161-186`（实现）
- **影响**：① 原发现前半未修也未声明；② `:134-135` 仍写"campaign_id 若提供，只返回匹配**或无 campaign 标签**的旧记录"——与 `:164-186` 的 fail-closed 相反，后续维护者可能照注释把 W-14 漏洞改回来。
- **建议**：同步注释（P3 级动作），并把"召回角色维度"作为独立待办登记（需要向量 metadata 增加实例标签）。

### N-R2-05 [P2] 取消时 `PostProcessSkipped` **发两次**，与"恰好一个"的事件契约矛盾
- **位置**：`crates/app-pipeline/src/lib.rs:1628-1632`（取消 → Skipped）+ `crates/tauri-app/src/runtime_support.rs:51-57`（runner 后取消 → 再发 Skipped）；契约文档 `runtime_support.rs:120-128`（"四条路径恰好各产生一个正确事件"）、`:53`（"与 runner 前取消同一个事件"）
- **证据**：`cancel_probe = cancel.clone()`（`lib.rs:1505`）与 `runtime_support` 的 `cancel` 是同一 watch；两条路径在同一次取消中都满足 → 同一 `event_tx` 收到两个 `PostProcessSkipped`。
- **影响**：前端可能重复处理"后处理已跳过"（重复提示/重复状态迁移）；W-28 想消除的是"Failed 冒充取消"，却引入事件冗余。
- **建议**：二选一保留唯一发送点（建议 pipeline 只发 Failed/None，取消事件统一由 `runtime_support` 发），并加"取消路径只发 1 个 Skipped"的断言测试。

### N-R2-06 [P2] W-30 ③ 未到调用层：合法空抽取仍与解析失败在用户可见结果上等价
- **位置**：`crates/app-agent/src/character_extractor.rs:75-78`（`defs.is_empty()` → `Err(ExtractError::Parse("识别结果为空"))`）+ `crates/tauri-app/src/commands/campaigns.rs:164-170`（任意 `Err` → 降级单角色卡）；另 `crates/app-agent/src/prompts/mod.rs:11-13` 与 `crates/app-agent/src/lib.rs:28-31` 双路 re-export 仍在
- **影响**：模型合法返回"这张卡没有可抽取角色"与真解析失败对用户不可区分（都变单角色降级卡），且"explicit_empty"三态在 app-agent 内部建立后又被调用点抹平。
- **建议**：`extract_characters` 对合法空返回 `Ok(vec![])`（或新增 `Empty` 变体），`campaigns.rs` 据此区分"空（不降级/提示无角色）"与"失败（降级单角色）"；顺手收敛双导出路。

### N-R2-07 [P2] W-11 下游传播门禁 fail-open 未被任何域认领
- **位置**：`crates/tauri-app/src/production_postprocess.rs:1149-1157`（源缺失/不可解析/无历史策略 → `return false` = 放行）、`:1199-1201`、`:1222-1230`、`:1210-1217`
- **触发示例**：`broadcast = Group("守卫")` + `source_character_id` 指向已删除/未解析实例 → 该私有知识仍写入整组目标。
- **说明**：非广播路径（`resolve(&update.character_id)` 不命中 → 无目标）本身 fail-closed，风险集中在广播/GroupRestricted + 源不可解析 → 维持 P2（若 Lead 视传播策略为信息隔离不变量可上调 P1）。
- **建议**：区分"源实例不可解析"（倾向 fail-closed + warn）与"无历史策略"（保持宽松），并补一条广播 + 源缺失的测试。

### N-R2-08 [P3] W-26 半接住：`AGENT_INTERFACES.md` 仍缺 2 个变体与字段
- **位置**：`docs/AGENT_INTERFACES.md:262-284` vs `crates/domain/src/agent.rs:437`（`QualityChecked`）、`:448`（`PromptHookRequest`）、`:456-459`（`PostProcessStarted` 字段）
- **建议**：补齐 + 加"文档事件名集合 == `PipelineEvent` serde tag 集合"的断言测试防再漂移。

### N-R2-09 [P3] W-31 泄漏：前端仍含 `big_scene`；`08-docs-sync-fixes.md:123` 虚假归属
- **位置**：`frontend/src/stores/writing.js:20`（`validGenerationModes`）、`:27`（按它过滤 localStorage）、`:57-62`（成为当前档位）；对照 `frontend/src/utils/generationModes.js` 目录（3 档）与 `docs/AGENT_INTERFACES.md:12`
- **影响**：旧 localStorage 用户被恢复进 ComposerBar 不可见、目录不存在的档位，并影响 `allowPartialReroll`/`editorOnly`。
- **建议**：删除 `'big_scene'` + 补"校验集 == 目录值集"测试；更正 `08-docs-sync-fixes.md:123` 的归属表述。**已有 task-33 承接前端侧**（pending/未认领）；`08` 记录更正不在其范围内。

### N-R2-10 [P3] W-01 归一语义与域侧并非同一套（非 ASCII 大小写）
- **位置**：`crates/app-agent/src/runtime.rs:607-609`（`eq_ignore_ascii_case`）vs `crates/domain/src/campaign_runtime.rs:108/118`（`to_lowercase()`）
- **含义**：`Élise`/`élise`：域侧 `with_temporaries_for` 视为同一实例（去重、不建临时实例），应用侧匹配失败 → 子 Agent 丢 instance 绑定（正是 W-01 的症状）；中文名无此问题 → P3。
- **建议**：两侧改用同一 `trim().to_lowercase()` 比较（或抽公共函数）+ 一条非 ASCII 用例。

### N-R2-11 [P3] W-07 事件层仍不区分"场景收束跳过"与"取消/失败"
- **位置**：`crates/app-pipeline/src/lib.rs:1341-1344`、`:2392-2395`、`:2708-2711`（三处 `Err(outcome)` 一律发 `PipelineEvent::SubagentCancelled`）；`crates/domain/src/agent.rs:425`（无 skipped/failed 变体）
- **建议**：新增 `SubagentSkipped` 变体或在 `SubagentCancelled` 上加 `reason`；否则在 `AGENT_INTERFACES.md` 写明复用约定（跨域：domain + tauri + 前端）。

### N-R2-12 [P3] 同名角色身份绑定仍用"静默取首个"的域侧 helper（W-08 域侧残留）
- **位置**：`crates/domain/src/campaign_runtime.rs:50-56`；调用点 `crates/app-conversation/src/lib.rs:951`（`SubagentSnapshot.character_instance_id`）、`crates/app-meta/src/meta_conversation.rs:546/606/655`
- **影响**：同名多实例时可能把演出归到错误实例 id（有 `fallback_reason` 记录，但绑定已错）。
- **建议**：调用点改用唯一命中判定（歧义时记 `fallback_reason` 并不绑定），或给域侧加 `find_instance_unique`；把该决定写进记录（W-08 目前只声明了工具侧）。

### N-R2-13 [P3] W-17 后半"判定非问题"证据错配
- **位置**：`docs/review-2026-09-13/fixes/03-pipeline-fixes.md`（W-17 第 2 半）所引 `crates/tauri-app/src/lib_tests_turns.rs:320-343`
- **证据**：该测试断言被采纳 variant 变为 `VariantStatus::Final`（`:342`），既未证明"legacy accept 有意保留非终态（供 reroll/swipe）"，也未触及 `validate_committed_prefix` 短路保护（`app-conversation/src/lib.rs:60-66`）。
- **建议**：补一条真正覆盖"非 Campaign 已采纳历史 + committed 前缀保护"的测试，或由 Lead 出一句书面裁定（"legacy 路径保护为已知缺口，不修"），不要以现有测试充当证据。

### N-R2-14 [P3] W-04 残余：**无引号**且后文含元线索词的正常对白仍判 Error
- **位置**：`crates/app-pipeline/src/quality_gate.rs:128-140`
- **反例**：`让我来为你倒茶。` → `MetaDescription` Error → 因 `domain/turn.rs:517` 阻断采纳。
- **说明**：属"精度优先"取舍的残余（改前同样报，非回归），但记录中"正常对白不再误杀"应限定为"引号对白 / 无引号的纯短语对白"。
- **建议**：把该反例写进测试并标注"已知残余"，或再加"句首/标点边界"约束。

---

## 6 低危观察（不单独编号，供后续登记）

| # | 观察 | 位置 | 级别 |
|---|---|---|---|
| 1 | 裸 `{}` 仍触发第二次 LLM 调用（`postprocess_dto_from_value` 要求 ≥1 已知键）；"空结果=成功"只覆盖"三键皆空数组"形态 | `postprocess.rs:275-296` | P3（可接受，建议写明边界） |
| 2 | Group 广播零命中仍静默（W-10 次要建议未做） | `production_postprocess.rs:1210-1219` | P3 |
| 3 | 确定性降级文案 `summary = bodies.join(" ")` 无长度上限（W-23 建议②未做） | `chronicle_compressor.rs:186-197` | P3 |
| 4 | 空 `targets` 语义仍未文档化（W-16 建议③未做） | `app-conversation/src/lib.rs:794-800` | P3 |
| 5 | 归属互换：全量扫描"不是"是 W-27 的发现，记录挂在 W-21 名下（代码无问题） | `fixes/03-pipeline-fixes.md` W-21/W-27 条 | P3 |
| 6 | W-22 的"§5-1 文档同步待办"已过期（`ROADMAP.md:77` 早已更正） | 记录 §5-1 | P3 |
| 7 | W-32 "两实现逐字节一致"措辞不精确（指输出等价，源码排版不同）；无等价性快照测试 | `app-agent/src/runtime.rs:1000/1024` | P3 |
| 8 | 取消以子 Agent `Err(Cancelled)` 形式撞上最后一次尝试时仍产出 `Failed`，无测试 | `sequential_crew.rs:355-359` | P3 |
| 9 | W-20 阈值：约 30% 缩水仍放行（记录已声明取舍） | `draft_revision.rs:4-24` | P3 |
| 10 | W-14 迁移影响：未回填 `campaign_id` 的历史归档在 campaign 召回中消失（记录已声明，需产品裁决） | `recall.rs:170-186` | P2（需裁决） |
| 11 | W-13 暂缓项**无 owner**（`RoundSummary` 受众隔离） | `domain/agent.rs:616-646` | P2（需派单） |

---

## 7 回归搜索（任务要求的四类）

| 风险面 | 结论 | 依据 |
|---|---|---|
| **老数据** | 未见新的非预期收紧 | W-03 守卫改前已存在（diff 证据）；W-14 收紧已在记录声明但**未回填历史标签**（§6-10）；W-06 水位语义不变；W-25 别名键纯新增 |
| **同名角色** | 工具侧一致，**身份绑定侧仍有缺口** | W-01 同名返回首个（注释声明）；W-08 工具侧歧义报错 ✓；但 `campaign_runtime.rs:50-56` + 3 处调用仍静默取首个 → N-R2-12 |
| **空 character_id** | 变更但更安全 | W-02 丢弃 + warn（不再造 `"unknown"` 幽灵实例）；`find_instance_normalized("")` → None（fail-closed） |
| **非 Campaign 路径** | 未受影响 | W-01 归一仅 `campaign_runtime.is_some()`；W-03 legacy 分支保留（测试断言）；W-25 别名只进 MVU JS 快照；W-17 前半只约束 Campaign |
| **提示词/上下文构造** | 未见无意改变 | W-25 的 `build_current_variables` 只喂 `execute_fragment`（`lib.rs:1533-1546`）；D-04 分组注入是**有意**（域1 落地）；W-32 输出等价 |
| **accept 阻断面** | 变松而非变严 | 删除行首规则后 Error 更少；W-21 全量扫描仍是 Warning 级（不阻断） |

---

## 8 门禁复跑（本复检独立执行）

| 命令 | 结果 | 记录数字 | 一致性 |
|---|---|---|---|
| `cargo test -p storyforge-app-agent --lib` | **130 passed / 0 failed** | 130 | ✓ |
| `cargo test -p storyforge-app-pipeline --lib` | **138 passed / 0 failed** | 138 | ✓ |
| `cargo test -p storyforge-app-pipeline --lib quality_gate` | **24 passed / 0 failed**（含 W-04 三条） | 24 | ✓ |
| `cargo test -p storyforge-app-memory --lib` | **12 passed / 0 failed** | 12 | ✓ |
| `cargo test -p storyforge-app-conversation --lib` | **23 passed / 0 failed** | 23 | ✓ |

- 未跑 `cargo test --workspace`（Lead 独占）与 npm（沙箱 EPERM）；全量门禁以 `fixes/GATE-REPORT.md`（99 套件 / 2165 passed / 0 failed）为准。
- "编译通过 ≠ 逻辑正确"：本报告结论全部来自回源读代码 + `git diff` 对比 + 自造反例，测试仅作辅助。首轮中间态红（`effective_reroll_mode`、`story_task.rs` 连字符测试名）在冻结树上不存在（5 条命令全绿）。**注意**：全绿的原因是 W-24 / W-12 前半 / W-14 前半 / 双发事件等都**没有失败测试**——`cargo test` 通过不代表这些条目已修（见 §5）。

---

## 9 建议 Lead 动作

| # | 事项 | 建议 owner | 优先级 |
|---|---|---|---|
| 1 | 更正 `fixes/03-pipeline-fixes.md`：W-24 移出"已修复"（N-R2-02），W-08/W-12/W-14/W-30 由"已修复"改为"部分关闭" | 域3 或 Lead | P2（记录可信度） |
| 2 | N-R2-01（big_scene 路由不可达）——前端传 `null` 或删能力并记录；**勿派域2**（作用域不含 `commands/**`）。**已存在 task-32**（未认领），完成前勿计"已收口" | 域5 + 域4 / Lead 裁定，或 task-32 owner | P2 |
| 3 | N-R2-05（取消双发 Skipped）——保留唯一发送点 + 断言测试 | 域3（pipeline）+ 域2（runtime_support） | P2 |
| 4 | N-R2-07（`production_postprocess.rs` 传播 fail-open） | 域2 | P2 |
| 5 | N-R2-06（空抽取 vs 解析失败）| 域3（extractor 返回契约）+ 域2/域4（调用点） | P2 |
| 6 | N-R2-04（召回无角色维度 + 注释矛盾）：注释同步 + 独立待办 | 域3（注释）/ 域2（调度） | P2 |
| 7 | W-13 暂缓项派单（`RoundSummary` 受众隔离，需域1 schema + 迁移） | 域1 + 域2 | P2 |
| 8 | N-R2-03（`present_chars` 契约字段）| 域3 + 域2 | P2 |
| 9 | N-R2-09（前端 `big_scene` + 08 记录虚假归属更正）。**已存在 task-33**（未认领）；`08-docs-sync-fixes.md:123` 的虚假归属**不在** task-33 交付范围内，需另派或并入 | 域5（task-33）+ 文档任务 | P3 |
| 10 | N-R2-08（`AGENT_INTERFACES.md` 补 2 变体 + 字段 + 集合断言） | 文档任务 | P3 |
| 11 | N-R2-10/11/12/13/14 | 域3 / 域1(domain) / 域5 | P3 |
| 12 | W-14 历史归档是否回填 `campaign_id` | 产品裁决 | P2 |

---

## 10 不确定性与诚实声明

1. 行号取自**冻结工作树**（未提交 diff）；后续改动会位移。
2. 本复检**未新增测试**（写作用域只允许本报告），W-04 两组输入通过"读实现 + 跑既有测试（24/24 通过）"验证。
3. W-18 存在替代解释"显式档位即契约、自动路由为预留能力"，但 `docs/review-2026-09-13/**` 无任何书面裁定 → 按"未接住/需裁决"上报。
4. N-R2-07 的严重度取决于"传播策略是否属信息隔离不变量"的产品口径（我按原判 P2）。
5. **记录可信度结论（修正）**：域3 自报的计数存在 **1 条虚报（W-24）+ 4 条过度声明（W-08/W-12/W-14/W-30）**；其余条目抽检未发现虚报。P1 六条无水分。
6. 独立验证由两名只读子验证者并行完成（W-07..W-32 全量静态核对、3 条转出跨域核验），其中 12 条关键结论我已逐条回源复读确认（W-24、W-12、W-14、W-08、W-30、双发事件、recall 注释、`lib_tests_turns.rs:320`、`useWriting.js:141`、`writing.js:20/57-62`、`AGENT_INTERFACES.md` 缺失、`08-docs-sync-fixes.md:123`）。

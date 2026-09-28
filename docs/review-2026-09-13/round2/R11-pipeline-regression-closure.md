# R11 收口：`PostProcessSkipped` 双发回归 + 域3 记录诚实性更正

- **任务**：task-34（收口 R11）
- **执行人**：review-pipeline（域3）
- **日期**：2026-09-13
- **基线**：HEAD `ab894c6` + 本轮脏工作树（多域并发修复中）
- **依据**：`round2/R2-pipeline-recheck.md`（N-R2-02/03/04/05/06/09/10/14）、Lead 的 task-34 指令
- **配套记录**：`fixes/03-pipeline-fixes.md` §13；`fixes/08-docs-sync-fixes.md:123`（单句更正）

---

## 1 结论摘要

| 项 | 处置 | 状态 |
| --- | --- | --- |
| ① 取消时 `PostProcessSkipped` 发两次（**本轮引入的回归**，N-R2-05） | Tauri 侧引入"终态事件仲裁"= 单一权威；pipeline 终态事件截留为兜底 | **已修复**（失败可控测试已补） |
| ① 同类面：成功路径 2×`PostProcessDone`、持久化失败先 Done 后 Failed | 同一修法一并收口 | **已修复** |
| ② W-24 条目内容错挂（记录不诚实，N-R2-02） | 更正条目 + 删除死熔断机制（零行为变更） | **已修复（记录 + 代码）** |
| ② W-08 / W-12 / W-14 / W-30 | 降为**部分关闭**并写清未关部分 + file:line | 部分关闭 |
| ② W-14 `recall.rs:134-135` 注释与实现矛盾（N-R2-04） | 按 fail-closed 实现改写注释 | **已修复（注释）** |
| ② `08-docs-sync-fixes.md:123` 虚假归属（N-R2-09） | 只改那一句 + 标注更正来源 + 真实归属 | **已修复（记录）** |
| ② W-13 / W-14 历史回填 | 暂缓（产品决策），写明理由、建议、解除条件 | 暂缓（有 owner = 本记录） |
| ③ R5-01 引号内「我将为你」误杀（blocks_accept） | 判据收窄为"非引号 + 写作任务线索" | **已修复** |
| ③ 同源 N-R2-14「让我来为你倒茶」误杀 | 歧义模式线索表收窄为同一写作任务线索表 | **已修复** |
| ④ N-R7-01（= N-R2-10）两侧归一语义相反 | 统一为 domain 单一函数 `normalize_instance_identity`（trim + Unicode 小写），跨 crate 语料测试 | **已修复** |

**一句话**：本轮把 R2 指出的**唯一真实回归**（事件双发）修掉并证明失败可控，同时把 6 条记录不实/半关闭项更正到与代码一致；剩余未关项（W-08 域侧 helper、W-12 过滤契约、W-30③ 合法空状态、W-13/历史回填）都有精确落点与解除条件。

---

## 2 ① 终态事件双发：复现 → 单一权威 → 测试

### 2.1 复现（失败可控证据，实测）

在 `runtime_support.rs` 的仲裁关闭（= 修复前行为）状态下运行新测试
`runtime_support::tests::cancel_during_runner_emits_exactly_one_terminal_event`，事件序列为：

```
1. PostProcessSkipped { reason: "postprocess cancelled" }                          ← Tauri 侧（runtime_support）
2. PostProcessStarted { summarizer_enabled: true, postprocessor_enabled: true }    ← pipeline
3. SummaryDone { char_count: 2 }                                                   ← pipeline
4. PostProcessSkipped { reason: "流水线已取消，后处理未完成（best-effort，不阻断成文）" } ← pipeline（lib.rs:1630）
```

⇒ **2 个终态事件**，且 Tauri 的 Skipped 甚至早于 `PostProcessStarted`（前端状态机先收到终态）。
这既是 N-R2-05 的独立复现，也证明新测试不是恒真：关掉仲裁即红。

### 2.2 单一权威裁定：Tauri 持久化层

- **为什么不是"pipeline 侧不发"**：`PipelineOrchestrator::run_postprocess` 的直接调用方不只有 Tauri ——
  `crates/tauri-app/tests/sqlite_endurance.rs:807/1084`、`m5_cache_and_memory.rs:330` 等消费者与其测试断言
  "取消即 `PostProcessSkipped`"。在 pipeline 侧删事件会破坏 pipeline 自身契约与这些测试。
- **为什么 Tauri 侧是权威**：只有这一层知道 generation 结果是否**真的落盘**
  （`Done` 必须反映持久化成功；`Skipped` 有 runner 前取消这一 pipeline 根本不存在的分支；
  持久化失败必须发 `Failed`）。
- **因此**：pipeline 的终态事件在生产路径上被**截留为兜底**，仅当 Tauri 分支不派生终态事件时使用——
  这保住了"非 Campaign 路径 / 双开关关闭的 Skipped / legacy `campaign_id: None` 静默路径"仍恰好一个终态事件
  （前端不会永久停在 running）。

### 2.3 实现（最小、可读、无竞态）

`crates/tauri-app/src/runtime_support.rs`：

1. `spawn_pipeline_event_arbiter(&event_tx)` → `(pipeline_tx, JoinHandle, Arc<Mutex<Option<PipelineEvent>>>)`：
   转发任务对**非终态事件立即转发**（Started/SummaryDone/… 进度不变），**终态事件写入槽位**（后写覆盖先写）。
2. `run_shared_postprocess_background` 把原函数体拆为 `run_shared_postprocess_body(...)`，
   终态事件一律经 `TerminalEventSender { tx, sent }` 发送（发送即标记 `sent`）。
3. 主体返回后：`drop(pipeline_tx)` → `forwarder.await`（**确定性排空**，无 sleep/竞态）→
   `if !terminal.sent { 用被截留的终态兜底 }`。
4. `postprocess_pipeline_event` 的文档补写"R11 单一权威"不变量。

`run_postprocess` 的签名、返回值、事件内容**零改动**。

### 2.4 测试

| 测试 | 层次 | 断言 |
| --- | --- | --- |
| `runtime_support::tests::cancel_during_runner_emits_exactly_one_terminal_event` | 生产入口（mock LLM 首次调用即置取消位 → 确定性命中"runner 内取消"） | 终态事件**恰好 1 个**且为 `PostProcessSkipped`；`PostProcessStarted` 确实转发过（防退化成"早取消"场景） |
| `runtime_support::tests::arbiter_forwards_non_terminal_and_holds_terminal_events` | 仲裁器单元 | 非终态立即转发；终态不得进真实 channel；后写覆盖先写 |
| `runtime_support::tests::early_cancel_emits_single_skipped_event`（既有） | runner 前取消 | 保持 1 个（证明修复未破坏原路径） |

---

## 3 ② 记录更正清单（含 W-24 与虚假归属）

### 3.1 W-24：条目答非所问（记录不诚实）→ 已更正 + 删死熔断机制

- 原条目内容 = **W-07 的取消映射**（与 W-24 无关）；R2 N-R2-02 判"虚报已修复"成立。
- 真实发现：`SequentialStageRecord.failures`（`sequential_crew.rs:103-107` 注释自认"生产只 record 不读"）
  + 两个 `#[cfg(test)]` 访问器（`:149-157`）+ 3 处生产 `record_failure` 调用 → **死熔断机制**。
- 处置：**删除**（字段/方法/访问器/3 处调用/2 条只测访问器的测试）。行为零变更；
  `last_error` 仍把错误文本带进最终 `Failed`。
- 不接线的理由：actor 已有 `MAX_ATTEMPTS_PER_ACTOR = 2` 有界重试；接入"2 次失败即熔断"= 改产品重试策略（未定 spec）。
- 新状态：**已修复（R2 定性的"未修/暂缓"已解除）**。

### 3.2 W-08 → 部分关闭

- 已关：`get_character` 工具侧（trim/空值/归一/歧义 `BadArgs`）。
- **未关**：`crates/domain/src/campaign_runtime.rs:75-83` `find_instance_by_id_or_name` 仍是
  "精确 id → **第一个** name 命中"（无归一、无歧义判定）。
  生产调用方：`crates/app-conversation/src/lib.rs:951`（SubagentSnapshot 身份绑定）、
  `crates/tauri-app/src/commands/meta_conversation.rs:546/606/655`。
- 后果（反证）：同名两实例时演出可能绑定到**错误实例**、Meta 工具操作错对象；有 `fallback_reason` 记录但绑定已错。
- 未修原因：会改变 4 处生产语义（3 处在非授权文件）。建议新增 `find_instance_unique` 或标注"调用方须自行查重"。

### 3.3 W-12 → 部分关闭

- 已关：MVU JS fallback 保留位键过滤 + 键归一 + instance 作用域键（含 M-04）。
- **未关**：`present_chars` 只是 prompt 输入（`crates/app-agent/src/pipeline_postprocess.rs:58`）；
  `PostProcessOutcome`（`:29-38`）无过滤契约字段；过滤只在 Tauri 侧
  （`commands/writing.rs:1370` 校验、`:1618` 空集放行）。绕开该分支的消费者拿不到同一契约。

### 3.4 W-14 → 部分关闭（注释已按实现更正）

- 已关：`accepts_campaign` 对无 campaign 标签记录 fail closed（带过滤时丢弃 + `debug!`）。
- **本轮修正**：`crates/app-memory/src/recall.rs:129-139` 原注释声称"若无标签也返回（兼容历史）"，
  与 fail-closed 实现**直接矛盾**；已改写为按实现描述（`recall_archived_by_query` 不做 campaign 过滤；
  过滤入口 `recall_archived_by_query_filtered` 无标签即丢弃）。
- **未关 1**：`filter_archived_hits`（`:188-210`）无 `CharacterInstance` 维度 → 角色级隔离在远记忆层不成立（与 W-13 同源）。
- **未关 2**：历史归档 `campaign_id` 回填 → 暂缓（见 §3.6）。

### 3.5 W-30 ③ → 部分关闭

- 已关：解析层三态（显式空 → `Ok(vec![])`；5 层全 miss → `Err`）；`drain.await` 的 `Err` 改 `warn!`；压缩器降级标记。
- **未关**：`crates/app-agent/src/character_extractor.rs:75-78` 把**合法空抽取**当 `Err(Parse("识别结果为空"))`
  ⇒ `crates/tauri-app/src/commands/campaigns.rs:165-169` 记为"角色识别失败"并降级为源卡单角色卡（`extraction_status` 也是失败态）。
- 未修原因：需动 `CharacterExtractionStatus`（序列化给前端的域枚举）/命令层分支 = 跨域接口决策。

### 3.6 W-13 / W-14 历史回填 → 暂缓（产品决策，有 owner）

| 项 | 理由 | 建议 / 解除条件 |
| --- | --- | --- |
| W-13 摘要受众隔离 | `RoundSummary` 无 audience 字段；改造面 = 域1 schema + 存储迁移 + 归档链路 + 前端展示；历史摘要无受众信息，迁移期收益 0 | 域1 出 `RoundSummary.audience: Option<Vec<Id>>`（None = 旧语义），注入侧按 `CharacterInstance.id` 过滤 |
| W-14 历史无标签归档回填 | 标签缺失时归属**不可恢复**；按时间/会话猜测会制造跨战役错误注入（比少召回更糟） | 维持 fail closed；或提供显式"重建/重新打标"后台工具（用户确认归属）。**不接受**静默按时间窗回填 |

### 3.7 `08-docs-sync-fixes.md:123` 虚假归属 → 已更正（只改那一句）

- 原文称 W-31"域5 记录称其已在前端侧修复"；撰写当时 `fixes/05-frontend-fixes.md` 对 `W-31`/`validGenerationModes`
  **零命中**（R2 `R2-pipeline-recheck.md:111` 已记为双重反证）。
- 更正内容（含来源标注）：写明虚假归属；现状 = 域5 于 R2 收尾已修前端部分
  （`frontend/src/stores/writing.js:22` 从 `utils/generationModes.js` 派生 + `:68` 拒绝未知档位；
  `fixes/05-frontend-fixes.md:475-485`）；残留死分支 `frontend/src/adapter/useWritingScreenAdapter.js:89` 与
  **W-18**（`useWriting.js:139-141`、`stores/writing.js:57-62`）归 **task-32 / task-33**；
  `docs/AGENT_INTERFACES.md:12` 归文档任务。
- 附带澄清：`stores/writing.js:18-20` 里的 `big_scene` 字样是"该值不得成为合法档位"的注释，**不是**未修证据。

---

## 4 ③ R5-01 + N-R2-14：元描述判据收窄（精度优先）

### 4.1 问题

- `quality_gate.rs` 的 `STRICT_PATTERNS` 原含「我将为你」「我来为你」且判定为**无条件 `contains`**：
  `「将军，我将为你赴汤蹈火。」` → `MetaDescription` **Error** → `QualityReport::blocks_accept(false)` **拦截采纳**。
- 同源 N-R2-14：歧义模式线索表含「为你」，`让我来为你倒茶。` 同样 Error（用户可见"正常对白被拦"）。

### 4.2 新判据（可判定，非放宽成恒不触发）

1. 无条件 STRICT 只保留真正不可能出现在对白里的自述：
   `作为AI` / `作为 AI` / `作为人工智能` / `以下是为您创作` / `以下是故事` / `现在开始创作` / `根据你的要求` / `按照你的要求`。
2. 「我将为你」「我来为你」→ **条件严格**：必须"**非引号语境**"且命中点后 16 字内出现**写作任务线索**
   （`写/创作/续写/生成/润色/改稿/正文/章节/故事/内容/安排`）才 Error。
3. 歧义模式（「让我来」「好的，我」「没问题，我」「我来写」）线索表由 `META_CUES`（含「为你/以下/要求」）
   **收窄为同一写作任务线索表** → 「让我来为你倒茶」「满足你的要求」不再误杀；
   「让我来为你安排这一章的节奏」仍 Error（`安排`）。

### 4.3 测试（失败可控）

| 测试 | 覆盖 |
| --- | --- |
| `test_quoted_first_person_promises_are_not_meta_errors` | 引号内承诺 + 无引号剧情承诺 → 无 Error 且 `blocks_accept(false)==false` |
| `test_assistant_task_promises_still_error_and_block_accept` | 反例：非引号 + 写作线索（「我将为你创作…」「我来为你写…」）→ Error 且拦截 |
| `test_quoted_writing_task_promise_is_treated_as_quotation` | 引号 + 写作动词按引用处理；无条件 STRICT 不受引号影响 |
| `test_person_object_cue_phrases_are_not_meta_errors` | N-R2-14 正反例 |
| 既有 W-04 四条 | 全部保持通过 |

**取舍声明**：引号内（含带写作动词的引用）不判 Error —— 引用/转述语境优先按对白处理，兜底由无条件 STRICT 承担。
这是"精度优先"的显式选择，已写进 §13.8；R2 §6 观察里的"句首/标点边界"约束未引入（线索词判据已覆盖其反例）。

---

## 5 ④ N-R7-01（= N-R2-10）：身份归一语义统一

- **问题**：domain `campaign_runtime.rs:108/118` = `to_lowercase()`（**不 trim**）；
  app-agent `runtime.rs:608` = `trim() + eq_ignore_ascii_case()`。
  ⇒ `"Ähre"/"ähre"` 与 `" Alice "/"Alice"` 两侧结论**恰好相反**；而 `runtime.rs:604` 注释已宣称"必须同一套语义"（注释与事实不符）。
- **裁定**：统一为 `trim` + **Unicode 小写**，落在 domain 的
  `storyforge_domain::campaign_runtime::normalize_instance_identity`，两侧共用。
  - `trim`：名称外侧空白是导入/LLM 噪声，不是身份差异。
  - Unicode 而非 ASCII：非 ASCII 名字的大小写变体是同一个人；ASCII-only 会静默 miss（正是 W-01 的失败模式）。
    代价：极少数"外表不同但 Unicode 小写相同"的字符（如 `U+212A` KELVIN 与 `k`）并入同键 —— 在角色名域可接受且两侧一致。
  - **不误合并优先**：土耳其 `İ` → `i`+`U+0307`（不合并 `İ/i/I`）；`ß` 不展开 `ss`（`Straße/strasse` 不合并）。
- **改动点**：`domain/src/campaign_runtime.rs`（新函数 + 去重键，含名称 trim）、
  `app-agent/src/runtime.rs`（`instance_name_matches` 委托 + `find_instance_normalized` id 归一分支 + 注释按事实改写）、
  `app-agent/src/tools.rs`（4 处 id/name 比较改用共享归一）。
- **测试（失败可控，实测）**：
  - `runtime::tests::test_normalization_shared_corpus_agrees_across_crates`：12 组语料（ASCII 大小写、前导/尾随空白、
    制表/换行、`Ähre/ähre`、`Élodie/élodie`、`İrem/irem` 双向、`Straße/strasse`、不同人、形近字符），
    每组同时断言 domain 去重、app-agent 匹配、**两侧相等**。
  - `runtime::tests::test_normalization_same_name_multi_instance_agrees`：同名多实例不新建、解析返回第一个。
  - 失败可控证明：把 app-agent 侧换回 ASCII → 在 `("Ähre","ähre")` 红（`left:false, right:true`）；
    把 domain 侧换回不 trim → 在 `(" Alice ","Alice")` 红（`domain 去重结论与期望不符 … temps=1`）。

---

## 6 门禁（本轮实际运行，真实通过数）

| # | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 1 | `cargo test -p storyforge-app-pipeline` | 0 | **140 passed / 0 failed** |
| 2 | `cargo test -p storyforge-app-agent` | 0 | **132 passed / 0 failed** |
| 3 | `cargo test -p storyforge-domain` | 0 | **388 passed / 0 failed** |
| 4 | `cargo test -p storyforge-app-memory` | 0 | **12 passed / 0 failed** |
| 5 | `cargo test -p storyforge --lib runtime_support::tests::` | 0 | **9 passed / 0 failed**（含 2 条新增 R11 测试） |
| 6 | `cargo test -p storyforge --lib` | 0 | **477 passed / 0 failed / 3 ignored**（收口复跑；task-32 收尾时同命令为 **480/0/3**，"+3"来自并发写者新增的 tauri-app 测试） |
| 7 | `cargo clippy -p storyforge-app-pipeline -p storyforge-app-agent -p storyforge-domain -p storyforge-app-memory --all-targets -- -D warnings` | 0 | `Finished`，0 warning |
| 8 | `cargo clippy -p storyforge --lib -- -D warnings` | 0 | `Finished`，0 warning |

- 未运行 `cargo test --workspace`（Lead 收口项）；未跑前端（本轮未改前端）。
- 中途一次 `cargo test` 曾因并发写者正在编辑 `crates/infra-sqlite/src/readiness.rs`
  出现瞬时 `E0425`（task-36 两次 edit 之间），非本轮改动导致；该文件恢复后 `cargo check` 通过（review-storage 已回执）。

---

## 7 诚实声明与未做项

1. **未做**：W-08 域侧 helper 改造、W-12 过滤契约字段、W-30③ 合法空状态、W-13 schema、历史归档回填
   —— 原因分别是"需改非授权文件的 4 处调用语义"/"需跨域接口决策"/"需动序列化枚举"/"需域1 schema + 存储迁移"/"信息不可恢复"。
   均已写明落点与解除条件，不是无主悬空项。
2. **未做**：`quality_gate` 的"句首/标点边界"约束（R2 §6 观察）—— 本轮的写作任务线索判据已覆盖其反例；
   若仍要更强精度需另立判据与测试。
3. **超出字面范围的一处扩展**：R5-01 的输入只点名「我将为你/我来为你」的引号场景，
   本轮把**同源同函数**的 N-R2-14（歧义模式线索词）一并收窄，因为它与 R5-01 是同一判据缺陷，
   且 R2 已登记为待处置项。此处已显式披露，若 Lead 认为越界可回退该半（仅 1 个 const 与 1 条测试）。
4. **记录诚实性**：本轮更正**不改写历史叙事**（原条目文本保留 + 条目头标注新状态 + §13 写明更正）；
   W-24 内容错挂与 08 的虚假归属按"记录不诚实"记明来源与责任任务（task-34）。

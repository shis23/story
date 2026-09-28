# R12 存储域收口：W-11 下游 fail-open 裁定 + N-R7-04 交叉引用

- **任务**：task-35（R12，第二轮收口）｜**owner**：`review-storage`（域2 / storage-lifecycle）
- **写入范围（实际）**：`crates/tauri-app/src/production_postprocess.rs` + 本报告 + `fixes/02-storage-fixes.md` §14。
  `crates/infra-sqlite/**` 与 `crates/tauri-app/src/storage_backend.rs` **本轮无改动**；`crates/tauri-app/src/commands/writing.rs`（孪生门禁）**不在写作用域，未改**。
- **上游条目**：
  - W-11 原始：`docs/review-2026-09-13/03-writing-pipeline.md:337-361`（建议"来源不可解析时 fail-closed 并 warn"）
  - W-11 上游修复（移交下游）：`docs/review-2026-09-13/fixes/03-pipeline-fixes.md` §W-11 + §6 遗留表
  - N-R7-04 原始：`docs/review-2026-09-13/round2/R7-cross-domain-seams.md:54,77`
- **冻结文件**：`round2/R1*.md`…`R7*.md` 未触碰（N-R7-04 只做交叉引用）。

---

## 0 裁定总表

| # | Lead 提出的疑问 | 裁定 | 本轮动作 |
|---|---|---|---|
| 0.1 | `source_propagation_blocks` 返回 `false` 的真实含义 | **放行**（该 update 对 target 会写入知识条目），不是"不受限" | 语义写进函数文档（`production_postprocess.rs:1125-1176`） |
| 0.2 | 谓词函数名 + 全部调用者 | 共享门禁 `source_propagation_blocks`（2 个调用点）+ 孪生 `should_block_source_knowledge_propagation`（2 个调用点） | §1.1 全量清单（含两后端归属） |
| F1 | `source_character_id` 缺失 ⇒ `false` | **判定非问题(附证据)**：广播的文档形状就不带该字段，fail-closed = 广播功能整体失效 | 升级为可观测（warn）+ 回归锁 |
| F2 | `resolve(source)` 为 `None` ⇒ `false` | **判定非问题(附证据)**：上游明示"未命中不猜、留下游兜底"，解析不出是被设计的合法状态 | 升级为可观测（warn） |
| F3 | 源无匹配条目 ⇒ `false`（策略未知） | **暂缓(附理由+建议)**：唯一与"private 事实被洗白外传"直接相关的面，但 fail-closed 会杀掉高频合法形态；正确修法 = 稳定标识（跨域） | 升级为可观测（warn）+ 修法建议 + 残留风险 |
| 0.3 | 是否允许"不处置" | **不允许**：三处 fail-open 全部从静默改为 warn 计数，且逐处给出裁定/代价/回退开关 | §1.2–§1.5 / §1.7 |
| ② | N-R7-04 回执 | **已接通**（R3 §9 已给完整调用链），本轮**只交叉引用，不重复** | §2（并确认本轮未触碰该链路） |

> 结论一句话：三处 `false` 都是"**没有可依据的受限策略**"时的 fail-open，其中 F1/F2 有明确的设计依据（提示词广播契约 + 上游归一策略），F3 是真实残留面但**不能用 fail-closed 修**（会把同一分支上的合法知识一起静默丢弃）。本轮把这些 fail-open 全部做成**可观测**，并给出收紧时的正确修法与开关位置。

---

## 1 W-11 下游 fail-open 逐处裁定

### 1.1 `false` 的真实语义与全部调用者

`false` = **放行** —— 该 `(update, target)` 不被阻止，知识条目**会被写入**（`Mutation::UpsertKnowledge`，`production_postprocess.rs:1334`）。它不是"该条目不受限"：写入条目的 `propagation` 字段沿用 update 自己声明的策略（`:1334-1350`），后续传播仍会再进门禁。

被阻止（`true`）时的实际后果：整条 update 或该 target **不产生 mutation**，且**不写任何错误/计数**——
- 整条预检：`production_postprocess.rs:1262-1284`（`Private + broadcast` 自声明 skip 与门禁预检都在这里 `continue`）；
- 逐 target：`production_postprocess.rs:1307-1317`（`.blocked` ⇒ `continue`）；
- 孪生实现：`commands/writing.rs:1319-1323`、`:1347-1349`（`return vec![]`）。

**共享门禁 `source_propagation_blocks`（`crates/tauri-app/src/production_postprocess.rs:1183-1234`）**

| 调用点 | 语义 | 消费字段 |
|---|---|---|
| `:1268` | 整条 update 预检（`target = None`，两后端共用） | `blocked` + **本轮新增** `unresolved`（warn 一次） |
| `:1307-1317` | 逐 target 判定（`target = Some(&target)`） | 只消费 `.blocked`（`unresolved` 与 target 无关，避免逐 target 刷屏） |

其宿主 `build_knowledge_mutations`（`:1240`）有**两个生产调用者**，即 **JSON 与 SQLite 共用同一门禁**（`:929-934` 文档注释明示）：
- `:980` `build_json_mutation_batch`（JSON mutation 路径）
- `:1403` `build_runtime_mutation_batch`（SQLite runtime 快照路径）

**孪生门禁 `should_block_source_knowledge_propagation`（`crates/tauri-app/src/commands/writing.rs:1513-1565`）**

| 调用点 | 语义 |
|---|---|
| `:1319-1323` | broadcast 预检（`target = None`） |
| `:1347-1349` | 单目标判定（`target = Some(&target)`） |

其宿主 `normalize_knowledge_update_for_postprocess_with_extras`（`writing.rs:1303`）由 `runtime_support.rs:1919`（`persist_postprocess_outcome_to_store`）调用；该函数**只在 JSON 后端运行**——`backend_workflows.rs:1314-1318` 在 SQLite 权威时直接 `Err("refusing legacy JSON postprocess persistence while SQLite is authoritative")`。
⇒ **默认后端（SQLite，Gate 7）走共享门禁**；孪生门禁只在显式 JSON 回退路径生效。两实现对三处 fail-open 的判定**逐条一致**（对照见附录 A）。

现有隔离回归锁（改造前后均通过）：`lib_tests_writing.rs:805 test_private_source_knowledge_blocks_told_by_other_propagation`、`:861 test_private_knowledge_update_cannot_broadcast`、`harness-real-llm/tests/writeback_isolation.rs:412 b6_private_source_blocks_name_collision_relay_and_group_broadcast`（真实 LLM，需 API），文本层另有 `app-pipeline/src/quality_gate.rs:347 check_private_knowledge_leak`。

### 1.2 F1 `source_character_id` 缺失 ⇒ 放行 — **判定非问题(附证据)**

- **位置**：`production_postprocess.rs:1203-1205`（`GATE_SOURCE_MISSING`）；孪生 `writing.rs:1529-1531`。
- **证据（这是广播的文档形状，不是异常）**：
  - `crates/app-agent/src/prompts/postprocess.rs:34-38`：广播指令 + JSON 示例
    `{"character_id":"城主","knowledge_text":"城主宣告全城戒严","source":"witnessed","broadcast":"all"}` —— **没有** `source_character_id`；
  - 同文件 `:35`："character_id 填公告发起者或**任意角色名**，系统会忽略并分发给所有人" ⇒ 广播的源身份**不是契约要求**；
  - `:111`：`propagation` 缺省 `open`；`:115`："private 知识不得广播或外传"（提示词层已禁止）；
  - `crates/domain/src/character_knowledge.rs:194-197`：`source_character_id` 的文档语义只是"**ToldByOther 时填**"。
- **若改成 fail-closed 的代价**：所有符合提示词示例形状的广播（无源 + broadcast）**全部被丢弃**，广播分发功能整体失效——这是功能回归，不是"代价可控"。
- **裁定**：此处 `false` 是**设计**：门禁判据依赖"源条目的既有策略"，无源则没有受限策略这一事实可依据。
- **残留（记 R12-R1）**：模型若为某个 private 事实同时输出 `broadcast` 且**把 propagation 写成 open**、且不带（或解析不出）源，门禁无从判定；此时只剩 ① `:1262-1264` 的 `Private + broadcast` 自声明 skip、② 域3 文本层 `check_private_knowledge_leak`、③ 提示词三重禁止。**彻底关闭需要域3 把 `source_character_id` 改为广播必填**（`prompts/postprocess.rs:290-294` 的 JSON schema）+ 域1 契约收紧 ⇒ 跨域，超出域2 写作用域，建议单独立任务。

### 1.3 F2 源实例解析不出 ⇒ 放行 — **判定非问题(附证据)**

- **位置**：`production_postprocess.rs:1206-1208`（`GATE_SOURCE_UNRESOLVED`）；孪生 `writing.rs:1532-1534`。
- **证据（"解析不出"是被设计的合法状态）**：
  - `docs/review-2026-09-13/fixes/03-pipeline-fixes.md` §W-11："`normalize_postprocess_identities` 把知识/变量/任务里的名字归一为**唯一命中**的 `CharacterInstance.id`；**未命中或同名多实例保留原值（不猜）**，由下游解析兜底"；
  - 实现：`crates/app-agent/src/postprocess.rs:145-151`（`resolve` 失败则保留原字符串）；
  - 提示词 `:107`：LLM 传的是**角色名不是 id**，`resolve` 是唯一归一机会。
  - ⇒ 命名来源包括：已删角色、临时角色（pending temps 之外的）、旁白/系统/抽象主体、同名多实例、跨卡引用——都是合法输入。
- **若 fail-closed 的代价**：这类来源的传播整条丢弃（含 `told_by_other` 单目标），且**静默**（孪生路径只在 blocked 时 warn，SQLite 路径改造前完全不 warn）。
- **裁定**：fail-open 由上下游约定支持（上游明示"不猜、留下游兜底"），**判定非问题**。
- **残留（记 R12-R2）**：解析失败时落库条目的 `source_character_id` 也会是 `None`（`:1285-1286` 用同一个 `resolve`）⇒ **溯源丢失**。这是独立于隔离门禁的可观测性缺口；修它要动 mutation 语义或上游归一策略（跨域），本轮不修。

### 1.4 F3 源可解析但无同文本条目 ⇒ 放行（策略未知） — **暂缓(附理由+建议)**

- **位置**：`production_postprocess.rs:1209-1211`（`GATE_SOURCE_ENTRY_MISSING`，`source_entry_for` `:1107-1123` 返回 `None`）；孪生 `writing.rs:1536-1564`（循环落空 ⇒ `false`）。
- **判据强度**：门禁的"受限"结论**完全依赖源侧已落库条目 + 文本匹配**。匹配规则 `knowledge_text_matches`（`writing.rs:1567-1579`）：归一化（去空白 + 小写）**全等**，或 **min_len ≥ 8 的包含**；长度 < 8 的非全等文本**永不匹配**。
- **为什么不能判定非问题**：这是三处里**唯一**直接对应"private 事实被洗白外传"的面——若 update 文本与源 private 条目文本不匹配（改写、复述、摘要、长度 < 8），门禁查不到策略 ⇒ 放行 ⇒ 该事实以新文本 + 策略 `open` 进入他人知识库。
- **为什么不能直接 fail-closed**：`told_by_other` 的常见合法形态正是"源条目尚不存在或措辞不同"（"A 刚告诉 B 一件新事"，A 的库内条目常常还没有；LLM 每轮重写知识文本，改写/复述必然 miss）⇒ fail-closed 会把 **B 被告知 X** 这条合法写入整条丢掉，知识库与正文**静默分歧**（玩家看到 A 说了，角色面板却没有）。
- **裁定**：**暂缓(附理由+建议)**——
  1. **理由**：无度量前不可接受"用高频静默丢合法知识换低频潜在泄漏"；本轮已把该面做成可观测（§1.5），上线后可直接统计"策略未知而放行"的频率，再决定是否收紧；
  2. **建议（正确修法）**：用**稳定标识**替代文本匹配——域1 给 `CharacterKnowledgeUpdate` / `KnowledgeMutation` 增加可选 `source_entry_id`（或 `event_id` 关联），域3 在归一阶段回填源条目 id，域2 端按 id 直查策略，`knowledge_text_matches` 降级为兜底。**这需要跨域（域1 schema + 域3 输出归一）**，建议单独立任务；
  3. **不建议的修法**：把 F3 直接 fail-closed（或收紧 `knowledge_text_matches` 阈值）——两者都会连带杀掉合法形态，且后者会让"匹配强度"成为新的静默判据。

### 1.5 本轮落地：fail-open 可观测化（**行为不变**）

- **新增**：`PropagationGate { blocked, unresolved }`（`production_postprocess.rs:1125-1169`）+ 三个原因常量
  `GATE_SOURCE_MISSING`（`:1172`）、`GATE_SOURCE_UNRESOLVED`（`:1174`）、`GATE_SOURCE_ENTRY_MISSING`（`:1176`）。
- **改造**：`source_propagation_blocks` 返回类型 `bool` ⇒ `PropagationGate`（`:1183-1234`）。三处 fail-open 分别返回 `allow_unresolved(<原因>)`；`Open/Private/GroupRestricted×4` 的 `blocked` 取值与改造前**逐分支一一对应**（`:1212-1232`）。
- **日志**：整条 update 预检（`:1268-1281`）在 `unresolved` 非空时
  `tracing::warn!(target: "knowledge_propagation", reason, source, text, "知识传播门禁未能判定源策略，按放行处理（W-11 下游 fail-open）")`。
  此前该路径**完全静默**，孪生路径只在 blocked 时 warn（`writing.rs:1554-1559`）⇒ 本轮后"策略未知而放行"可按 `target: "knowledge_propagation"` + `reason` 直接计数。
- **不改变行为**：`blocked` 是唯一决定写入与否的字段；`unresolved` 只进日志。
- **回归锁（3 条新测）**：
  - `production_postprocess.rs:1558 propagation_gate_reports_unresolved_fail_open_reasons`：三处 fail-open 必须**继续放行**且带对应原因（谁改成 fail-closed 立刻红）；
  - `:1606 propagation_gate_keeps_explicit_policy_verdicts`：Private ⇒ 阻止且 `unresolved == None`；Open/组内广播 ⇒ 放行；`GroupRestricted + All` ⇒ 阻止；非传播路径放行且无 `unresolved`；
  - `:1657 broadcast_without_source_character_id_still_writes_knowledge_mutations`：**端到端**证明文档化广播形状仍产出 2 条 `UpsertKnowledge`（若被改成 fail-closed 会变 0）。

### 1.6 为什么不做"整体 fail-closed"（代价汇总）

| 面 | fail-closed 后丢失的东西 | 定性 |
|---|---|---|
| F1 | 全部符合提示词示例的无源广播（广播功能失效） | **功能回归**，不可接受 |
| F2 | 全部未归一来源的传播（上游已明示"不猜"） | 与上游约定直接冲突 |
| F3 | `told_by_other` 且源条目缺失/措辞不同的合法"被告知"写入（高频形态） | 知识库与正文静默分歧 |

⇒ 无条件 fail-closed = 用"高频静默丢合法知识"换"低频潜在泄漏"。**当前沙箱内无法给出 F3 的 miss 率**（无生产日志、无真实 LLM 样本）——这正是本轮把 fail-open 做成 warn 的目的。

### 1.7 回退 / 开关建议（Lead 要求）

- **回退本轮**：仅丢可观测性，不影响数据——删除 `PropagationGate`（`:1125-1176`）+ `:1268-1281` 的 warn 块 + 3 条新测，把返回类型改回 `bool`。
- **若最终决定收紧（"宁丢不漏"）**：建议做成**显式配置**（如 `PostprocessPropagationStrictness::{Audit, Preferred}` 或 profile 字段），默认保持现状（放行 + warn）；严格模式只收紧 F2/F3，**F1 仍必须放行**（否则广播失效）。开关位置应落在 `build_knowledge_mutations` 的入参（`persist_ctx` 或新增 gate 配置），**不要**散落在闭包内或被 `unresolved` 隐式驱动。

---

## 2 N-R7-04 回执（交叉引用，不重复）

- R3 已给完整回执并判 **"已接通"**：`docs/review-2026-09-13/round2/R3-tauri-frontend-recheck.md:225-268`（§9 判据 A：S-01 readiness 源清单 fail-closed；判据 B：`storage_backend.rs:285` 运行时权威复检；判据 C：`storage_backend.rs:268` `require_supported`），差异说明见该文件 §9.4。
- 原始条目：`round2/R7-cross-domain-seams.md:54,77`（P2，"需 owner 补 file:line + 测试"）。
- R13 又记录了同一启动链的闭合：`round2/R13-readiness-legacy-layout-fix.md` §1.5（`lib.rs:1229-1230` → `storage_backend.rs:1882/1916/1929` → `recover_or_verify` `:1938-1939` → 无 JSON 回退）。
- **本轮唯一新增信息**：R12 **未触碰**该链路——`production_postprocess.rs` 的改动全部落在知识传播门禁与测试，`storage_backend.rs` 零改动 ⇒ R3 §9 的回执继续有效，无需重开。

---

## 3 验证（本成员实跑，工作树含其它成员并行改动）

| 命令 | 退出码 | 结果 |
|---|---|---|
| `rustfmt --edition 2024 --check crates/tauri-app/src/production_postprocess.rs` | 0 | 无 diff |
| `cargo check -p storyforge --lib` | 0 | Finished（0 error） |
| `cargo clippy -p storyforge --lib -- -D warnings` | 0 | Finished（0 warning / 0 error） |
| **定向新测 ×3**（`propagation_gate` ×2 + `broadcast_without_source`） | 0 | **3 passed / 0 failed** |
| `cargo test -p storyforge --lib` | 0 | **480 passed / 0 failed / 3 ignored**（R12 前为 477 ⇒ 本轮 +3） |
| `cargo test -p storyforge --test backend_parity_suite` | 0 | 2 passed / 0 failed（JSON↔SQLite mutation 等价，覆盖共享门禁） |
| `cargo test -p storyforge --test sqlite_character_lifecycle` | 0 | 1 passed / 0 failed（含 private 知识条目路径） |
| `cargo test -p storyforge-infra-sqlite` | 未运行 | R12 未触碰 infra-sqlite；R13 刚跑过 **320 passed / 0 failed**（见 `fixes/02-storage-fixes.md` §13） |
| `cargo test --workspace` / `npm test` / vitest | 未运行 | Lead 指令（不跑 workspace 级 cargo / 不跑 npm）+ 沙箱 `spawn EPERM`；引用 `fixes/GATE-REPORT.md`（fmt=0、clippy=0、workspace 99 suites/2165/0 failed、`npm test` 532/532、vitest 31 files/151、build=0、Pester 198/0） |

**未做真实 LLM 端到端**：`harness-real-llm/tests/writeback_isolation.rs`（b6）与 `knowledge_propagation_real_llm.rs` 需要 API 凭证，本沙箱不跑（与 GATE-REPORT 口径一致）。

---

## 4 残留风险与移交

| ID | 内容 | 类型 | 建议 |
|---|---|---|---|
| R12-R1 | F1 残留泄漏面：private 事实 + `broadcast` + 模型把 `propagation` 写成 `open` + 无/坏 source ⇒ 门禁无从判定（只剩提示词 + 文本层两道） | 跨域（域3/域1） | 域3 把 `source_character_id` 改为广播必填并改 schema/提示词；单独立任务 |
| R12-R2 | F2 解析失败时落库条目 `source_character_id = None`（溯源丢失） | 域2/域3 | 若要保留溯源，需上游归一失败时给出可落库的占位来源或新字段；独立评估 |
| R12-R3 | `crates/tauri-app/src/commands/writing.rs:1513-1565` 孪生门禁**未同步本轮 warn**（写作用域外）；行为仍与改造前一致，且默认后端不经过它 | 域2（需授权） | 若 Lead 同意，加同样一行 warn（约 10 行，零行为变化） |
| R12-R4 | F3 的 miss 率无度量：本轮只做到"可观测"，未给阈值/开关 | 域2 | warn 上线后按 `reason=GATE_SOURCE_ENTRY_MISSING` 统计；再决定是否引入稳定标识或严格模式 |
| R12-R5 | 本轮未给 `knowledge_text_matches` 的任何阈值/语义改动（刻意不动） | 域2/域3 | 若未来收紧匹配，需按 §1.4 的两条"不建议修法"评估误伤 |

---

## 附录 A 两实现对照（三处 fail-open）

| 判定 | 共享门禁（默认后端） | 孪生实现（JSON 直写） |
|---|---|---|
| 不再传播（自持知识） | `:1197-1202` `allow()` | `writing.rs:1523-1527` `false` |
| F1 无 `source_character_id` | `:1203-1205` `allow_unresolved(GATE_SOURCE_MISSING)` | `:1529-1531` `false` |
| F2 源解析不出 | `:1206-1208` `allow_unresolved(GATE_SOURCE_UNRESOLVED)` | `:1532-1534` `false` |
| F3 无匹配源条目 | `:1209-1211` `allow_unresolved(GATE_SOURCE_ENTRY_MISSING)` | `:1536-1564` 循环落空 ⇒ `false` |
| Private | `:1214` `block()` | `:1544` `true` |
| GroupRestricted | `:1215-1232`（组广播/All/单目标/无 target） | `:1545-1550`（同四分支） |
| 阻止时日志 | 本轮起：仅"未判定"记 warn（`blocked` 仍静默） | 仅 `blocked` 记 warn（`:1554-1559`，未判定静默） |

**本轮改动文件清单**：`crates/tauri-app/src/production_postprocess.rs`（门禁返回类型 + warn + 3 条新测）、本报告、`fixes/02-storage-fixes.md` §14。

# 记忆与 Context 装配规格（远楼不失忆 + 缓存友好）

> 日期：2026-07-11  
> 状态：**已拍板的目标规格**（尚未全部实现；下文「现状」与「目标」分列）  
> 相关：`docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md` §7–8、§10 阶段 C  
> 实现落点（现状代码）：`crates/domain/src/conversation.rs`（history-epoch）、`crates/app-pipeline`（inject budgets / layout）、`crates/app-memory`（归档/召回）、`crates/app-agent`（Summarizer / tools）、`crates/tauri-app`（fill_far_memory / RoundSummary 索引）

---

## 1. 问题与目标

### 1.1 要解决的问题

1. **远楼失忆**：近窗 history 丢掉正文后，旧情节几乎只靠 hybrid 检索 top-K 进 prompt，命中不稳。  
2. **正文与纪要双税**：近几轮既有 history 原文，又在 tail 贴 RoundSummary 全文。  
3. **目录过薄或无限增殖**：要么只自动贴约 5 条摘要，要么若不折叠则概览无限涨。  
4. **缓存前缀被每轮滑动打穿**：记忆块若每轮各挪 1 格，system 之后的共同前缀无法复用。

### 1.2 目标

- 近楼靠 **正文** 连戏；中距靠 **短纪要**；远楼靠 **厚概览目录 + 点名/检索展开 + 小 K 自动兜底**。  
- 关键事实优先落 **状态层**（知识/变量/任务），不单靠散文记忆。  
- **MessageLayout 物理顺序：旧 → 新 / 稳 → 不稳**，保护供应商前缀缓存。  
- 压缩采用 **攒满阈值再批压**（默认 active A≈200 → ~50 B），减少结构抖动轮次。

### 1.3 非目标

- 不整包移植 shujuku / 酒馆世界书 depth。  
- 不强制每轮 300+ 字长纪要全文进主模型。  
- 不删除历史 RoundSummary 行（折叠只影响**默认注入目录**）。  
- 不替代任务/知识系统做长程承诺。

---

## 2. 现状（代码事实，2026-07-11）

| 机制 | 行为 |
| --- | --- |
| history | `DEFAULT_HISTORY_WINDOW_SIZE=20` 消息条 + epoch 整块前移；可有确定性 `【历史纪要】checkpoint` |
| RoundSummary | **Summarizer** 产出；**Accept 后**落盘；load 最近 12、**tail 自动 inject 约 5 条截断 content** |
| MemoryArchiver | **已有**：对话**消息正文**过长时批压 → `ArchivedSummary` 入向量库（`crates/app-memory`，水位 `archived_upto`） |
| 远记忆召回 | Accept 后 RoundSummary 也可索引向量库；写作时 `intent → hybrid → top≈3` 进 tail（`FarMemoryHit`） |
| 二次压缩 RoundSummary 金字塔 A→B→C | **无**（目标由 **ChronicleCompressor** 承担，见 §3.4 / §7） |
| 概览字段 / 稳定 code / Director 点名 tool | **无**（仅有 `get_recent_summary` 等，非本规格完整形态） |
| 正文与摘要去重 | 远记忆 vs recent_summaries 有去重；**history 正文 vs 同轮摘要无硬隔离** |

---

## 3. 记忆层级与编号

### 3.1 层级

| 层 | 代号前缀 | 含义 | 默认是否进「事件概览」 |
| --- | --- | --- | --- |
| L0 leaf | `A0001`… | 每轮 Summarizer 纪要（Accept 后） | 仅当**未被**上层覆盖 |
| L1 stage | `B0001`… | 一批 A 的二次压缩 | 仅当**未被** C 覆盖 |
| L2 stage | `C0001`… | 一批 B 的再压缩 | 活跃则进目录 |

内部仍保留 UUID `id`；**对外寻址优先 `code`**。

### 3.2 字段（目标模型）

```text
ChronicleEntry {
  id: UUID
  code: "A0123" | "B0042" | "C0007"
  level: 0 | 1 | 2
  turn_or_span: turn | (turn_start, turn_end)
  headline: string          // ≤40 字 / 硬截断 ~40 token；概览行只用这个
  summary: string           // 短正文摘要（tool 默认返回；纪要带注入用）
  full?: string             // 可选存库；tool detail=full 才返回
  entities?: string[]
  covers?: code[]           // B/C 覆盖的子 code 列表
  covered_by?: code         // leaf/stage 被谁折叠
  campaign_id, conversation_id
}
```

- **A**：由现有 RoundSummary 升级（至少增加 `code` + `headline`；`content` 对齐 `summary` 或拆 `summary`/`full`）。  
- **B/C**：二次压缩任务写入；必须填 `covers`。

### 3.3 压缩策略（拍板：攒满再压）

```text
当「未覆盖的 active A」数量 ≥ 200：
  → 后台批处理压成约 50 条 B
  → 被覆盖的 A 标记 covered_by，退出默认概览

当「未覆盖的 active B」数量 ≥ 200：
  → 压成约 50 条 C
  → 被覆盖的 B 标记 covered_by，退出默认概览
```

- 压缩是 **稀有事件**（利于缓存：长稳定期 + 偶发冷启动）。  
- 输入应用 **headline+summary 分批**，禁止单次塞 200 段 full。  
- 失败可重试，不阻塞写作主路径。  
- 长程任务/承诺：**任务与状态** 兜底，不依赖最底层 A 永远在目录里。  
- **执行者不是每轮 Summarizer**，而是 §7 的 **ChronicleCompressor**。

可选后续：日常「每 4 个 A → 1 个 B」流水；**默认规格以 200 阈值批压为准**。

### 3.4 谁写哪一层（总表）

| 产出 | 执行者 | 状态 | 原料 | 触发 |
| --- | --- | --- | --- | --- |
| **A**（leaf 纪要） | **Summarizer**（`AgentRole::Summarizer`） | **已有** | 本轮 Editor 成文（+ 场景 brief） | 每轮成文后，与 Postprocess **并行**；Accept 后规范落盘 |
| **B / C**（阶段纪要） | **ChronicleCompressor**（新；见 §7.2） | **目标** | 多条 A 或 B 的 headline+summary | active 未覆盖计数 ≥200（后台批） |
| **ArchivedSummary**（消息远记忆） | **MemoryArchiver**（见 §7.3） | **已有** | 对话树**消息正文**前缀 | `archived_upto` 之后未归档条数 ≥ threshold（默认约 50） |
| 知识 / 变量 / 任务 | **PostProcessor** | **已有** | 本轮成文 + 在场角色等 | 与 Summarizer 并行；**不写剧情纪要** |
| history `【历史纪要】checkpoint` | **本地确定性函数**（非 LLM） | **已有** | 掉出 history 窗的消息前缀 | 超窗组装 prompt 时 |

---

## 4. 窗口参数（默认值，可配置）

| 符号 | 默认 | 含义 |
| --- | --- | --- |
| `H` | **5** | 近窗：只注入 **正文** 的轮数 |
| `S` | **10 或 20** | 纪要带：紧邻近窗外的轮数（短 summary；例 6～15 或 6～25 楼） |
| `E` | **= S** | 三窗同步整块前移的步长（与 history-epoch 哲学一致） |
| `overview_cap` | **200** | 事件概览最多行数（短 headline，约 200×40tk ≈ 8k token 量级，可接受） |
| `auto_recall_k` | **1～2** | 自动 hybrid 兜底条数（保留；与 tool/目录去重） |
| `tool_summary_max` / 轮 | **6** | `get_chronicle(..., summary)` |
| `tool_full_max` / 轮 | **2** | `get_chronicle(..., full)` |
| `search_max` / 轮 | **2～3** | `search_chronicle` |
| `compress_active_A` | **200 → ~50 B** | 见 §3.3 |
| `compress_active_B` | **200 → ~50 C** | 见 §3.3 |

---

## 5. 发给 Director 的 prompt 物理结构（目标）

**语义远近**（产品叙述）：近正文 → 中纪要 → 远概览。  
**物理顺序**（缓存，必须旧→新 / 稳→不稳）：

```text
[system]
  Director 职责 + NarrativeContract 稳定切片 + 常驻 lore + 稳定模块
  （会话内尽量 byte 稳定）

[history]  时间正序：
  (1) 【事件概览】≤ overview_cap 行
        code + headline
        仅活跃未覆盖条目；B/C 优先占位，再补最近未覆盖 A
  (2) 【中距纪要带】对应「近窗外 S 轮」的 short summary（有序）
  (3) 【近 H 轮正文】user/assistant 原文 only
  (可选) 比概览更早、已完全掉出策略窗的确定性 checkpoint
        —— 仅作极粗前缀，不得替代 B/C

[tail]  每轮新建：
  本轮用户意图
  Campaign 状态/任务/变量（结构化）
  自动 recall top-K（小；放 tail 末，避免弄脏概览/纪要前缀）
  tool 结果（本轮展开）
  「请输出 Plan/ScenePlan」
```

### 5.1 硬去重（装配期写死）

1. `turn ∈ 近窗 H` → **只发正文**；禁止同 turn 的 A **全文/summary 进 tail 或纪要带**。  
2. `turn ∈ 纪要带 S` → **只发该 turn 短纪要**；禁止再塞同 turn 正文进 history。  
3. `covered_by` 已设 → **默认概览与纪要带不出现该 leaf**。  
4. 自动召回 / tool 与 1–3 同 `code` → **去重只保留一处**。

### 5.2 同步滑动（缓存关键）

- **多数轮**：概览块文本不变；纪要带集合不变；仅在 history **末尾追加** 最新一轮正文。  
- **每 E 轮（纪元边界）**：近正文窗、纪要带、概览 **同一边界整块更新**（不要三窗各滑 1 轮错开）。  
- **压缩轮**：概览由大量 A 变为 B/C 骨架 → **预期一次冷前缀**；之后再进入长稳定期。  
- history-epoch 算法继续服务「近正文 + 可选 checkpoint」；`E` 与纪要带对齐，避免双重不齐的滑动。

### 5.3 其他 Agent

| Agent | 概览 | 纪要带 | 近正文 | 点名 tool |
| --- | --- | --- | --- | --- |
| **Director** | 满配 ~200 | 有 | 有（H） | **有**（主） |
| **Editor** | 可瘦（更小 cap） | 可选短 | 同 epoch 正文 | 可选 |
| **Subagent** | 默认无全库概览 | 无 | 默认无全局 history | **默认无**；防全知 |
| **Summarizer** | 无 | 无 | 本轮成文 | 无 |
| **Postprocess** | 无长目录 | 无 | 成文+在场角色 | 无（不写纪要） |

---

## 6. 工具契约（目标）

### 6.1 `search_chronicle`

- **用途**：概览 cap 外、模糊说法、或需 `include_covered` 时找回 code。  
- **入参**：`query`，可选 `level`/`code_prefix`/`turn_range`/`include_covered`/`limit`。  
- **出参**：短列表 `{ code, level, headline, turn_span, score, covered_by? }`，**不含 full**。  
- **预算**：每轮 2～3 次。

### 6.2 `get_chronicle`

- **用途**：点名阅读。  
- **入参**：`code`（或 id）；`detail`: `summary`（**默认**）| `full`。  
- **默认 summary**：便于多读几条、控长度。  
- **full**：可选；次数更严。  
- 若 code 已被覆盖：仍可解析；返回中带 `covered_by`；默认 summary；full 才拉存库正文（若仍保留）。  
- **预算**：summary ≤6/轮，full ≤2/轮；合计字符硬顶。

### 6.3 与自动召回

- `auto_recall_k` **保留**作兜底（1～2）。  
- 结果进 **tail**，不写回概览消息（保护前缀）。  
- 与目录、tool **按 code 去重**。

---

## 7. Agent 与后台组件（写入侧）

本节固定 **谁干什么**，避免把 Summarizer、Postprocess、MemoryArchiver、ChronicleCompressor 混成「后处理写纪要」。

### 7.1 Summarizer（已有）— 写 A

| 项 | 说明 |
| --- | --- |
| 角色 | `AgentRole::Summarizer` |
| 代码 | `crates/app-agent/src/prompts/summarizer.rs`；pipeline 成文后并行调用 |
| 输入 | 本轮成文 `final_text`、可选 `scene_brief`、轮次 |
| 输出 | 高密度**本轮**摘要（现状约 200–500 字正文）；**目标**解析为 `code` + `headline` + `summary`（+ 可选 full） |
| 时机 | Editor **Draft 就绪后**，与 Postprocess **并行**；结果先入 TurnAttempt 候选 |
| 落盘 | **用户 Accept** 后写入 Campaign / 升级为 Chronicle **A** 并索引 |
| **不负责** | 多轮 A→B→C 批压；对话消息归档；知识/变量/任务 |

提示词原则（保持）：只总结本轮、不展望、不复述前情；目标态在输出中增加一行级 **headline** 与稳定 **code**（或由系统分配 code）。

### 7.2 ChronicleCompressor（目标新增）— 写 B / C

| 项 | 说明 |
| --- | --- |
| 名称 | **ChronicleCompressor**（阶段纪要压缩器）；实现可为独立 `AgentRole` 或 `app-memory` 批任务 + 共用 LLM 客户端 |
| 状态 | **规格目标；尚未实现** |
| 输入 | 一批未覆盖 **A**（或 **B**）的 `code, headline, summary`（**分批**，禁止单次 200×full） |
| 输出 | 约 50 条 **B** 或 **C**：`headline` + `summary` + `covers[]`；原子标记子项 `covered_by` |
| 触发 | `count(active 未覆盖 A) ≥ 200` → ~50 B；`count(active 未覆盖 B) ≥ 200` → ~50 C |
| 时机 | **Accept 落盘 A 之后**异步调度；或独立后台扫描；**不**插入每轮成文热路径 |
| 失败 | 可重试；不阻塞 start_writing / accept 主路径 |
| **不负责** | 本轮成文摘要（归 Summarizer）；消息原文归档（归 MemoryArchiver）；状态写回（归 Postprocess） |

与 Summarizer 的关系：

- **可共用**「高密度摘要」文风与模型配置档。  
- **不可**并入 Summarizer 每轮 prompt：原料、触发频率、失败语义均不同。  
- 压缩是 **稀有事件**，服务缓存稳定期；Summarizer 是 **每轮事件**。

### 7.3 MemoryArchiver（已有）— 消息远记忆

| 项 | 说明 |
| --- | --- |
| 名称 | `MemoryArchiver` |
| 代码 | `crates/app-memory/src/archiver.rs`；Tauri `run_archive_with_watermark` / `auto_archive_if_needed` |
| 原料 | 对话树中**可归档消息正文**（非 RoundSummary 列表） |
| 水位 | `Conversation.archived_upto`：只处理未归档前缀，防重复归档 |
| 触发 | 未归档条数 ≥ `ArchiveConfig.threshold`（默认 **50**）；常依赖嵌入配置才走自动路径 |
| 过程 | 按 `archive_batch_size` 等分批 → LLM 压成高密度段（上限约 summary_max_chars）→ keywords + 可选 embedding → `vector_store.upsert` |
| 产出 | `ArchivedSummary`：`id, content, source_range, keywords, vector?`；metadata 可含 campaign/conversation；`kind = ArchivedSummary`，`source ≈ message_archive` |
| 读路径 | 写作时 `fill_far_memory_hits` → `recall_archived_hybrid(intent)` → `FarMemoryHit` 进 **tail**（约 top-3），**不进** history 原文窗 |
| **不负责** | 每轮剧情纪要 A；A→B→C 金字塔；Postprocess 状态 |

与 RoundSummary 入库的关系（现状易混）：

- Accept 后 **RoundSummary** 也可 upsert 进**同一向量平面**（`source ≈ round_summary`），便于 hybrid 召回。  
- **写入路径不同**：一个是 Archiver 压**消息**，一个是索引 **已接受轮次摘要**。  
- 目标态下 **默认事件概览以 Chronicle A/B/C 为主**；`ArchivedSummary`（消息归档）主要走自动召回 / `search_chronicle` 类检索，不占满 200 行概览。

### 7.4 PostProcessor（已有）— 不写纪要

| 项 | 说明 |
| --- | --- |
| 角色 | `AgentRole::PostProcessor` |
| 代码 | `crates/app-agent/src/prompts/postprocess.rs` |
| 产出 | **知识**、**变量**、**任务/伏笔** 三件套（`emit_postprocess`） |
| 时机 | 与 Summarizer **并行**，同属成文后流水线 |
| **明确不写** | RoundSummary / Chronicle A/B/C / 消息归档 |

口语「后处理阶段」可包含 Summarizer，但 **Postprocess Agent ≠ 写纪要**。

### 7.5 写入总览（目标流水线）

```text
Editor 成文 (Draft)
  ├─► Summarizer ──────────► 候选 A（headline+summary）
  └─► PostProcessor ───────► 知识 / 变量 / 任务
              │
         用户 Accept
              │
              ├─► A 落盘 + 检索索引
              ├─► 状态 MutationBatch 提交
              ├─► 若 active A ≥ 200：enqueue ChronicleCompressor → B
              └─► 若 active B ≥ 200：enqueue ChronicleCompressor → C

对话消息积压（独立水位）
  └─► MemoryArchiver → ArchivedSummary → 向量库
              │
         下一次 start_writing
              └─► 装配：概览 A/B/C + 纪要带 + 近正文
                  + hybrid 召回（A/B/C 索引 ∪ ArchivedSummary）小 K
                  + Director tool 点名 summary/full
```

### 7.6 读路径上各组件如何进 prompt（对照）

| 来源 | 默认进 Director 的方式 |
| --- | --- |
| 近 H 轮对话正文 | history 近窗原文 |
| 中距 S 轮 A.summary | history/装配中的纪要带（短） |
| 活跃 A/B/C.headline | 【事件概览】≤200 行 |
| 自动 hybrid hit | tail 小 K（ArchivedSummary 与索引摘要） |
| tool `get_chronicle` | tool 消息 / tail（默认 summary） |
| 知识/变量/任务 | 结构化状态块（Postprocess 已写盘的） |

---

## 8. 可观测与验收

### 8.1 每轮日志字段

```text
history_epoch_id, H, S, E
overview_codes[]           // 实际注入的 ≤200
band_codes[]               // 纪要带
raw_turn_range
auto_recall_codes[]
tool_reads[{code, detail}]
compress_events?           // 本轮是否发生 A→B / B→C
```

### 8.2 验收用例

1. 第 5 轮关键线索 → 第 40 轮用户提起时，出现在 **概览或 search 命中或状态层**。  
2. 近 H 轮：history 有正文 → **同轮 A 不进纪要带/tail 全文**。  
3. 同一 epoch 内连续两轮：概览块 + 纪要带 **byte 稳定**；仅近正文后缀增长。  
4. 跨 E 边界：三窗 **同轮** 更新，不出现「只移正文不移纪要」的错位。  
5. active A 达 200 压缩后：概览行数明显下降；旧 A **默认不在概览**；`get_chronicle(Axxxx, summary)` 仍可用。  
6. 关 embedding：关键词 + 概览 + search 仍能找回至少一条已知远楼（非全灭）。

---

## 9. 实施分期（建议）

| 期 | 内容 | 依赖 |
| --- | --- | --- |
| **M1** | `code`+`headline`；概览注入（cap=200）；近窗与摘要硬去重；H/S 参数 | **Summarizer** 输出/解析 |
| **M2** | 装配顺序改为 概览→纪要带→近正文→tail；E 同步滑动；epoch 对齐 | MessageLayout 组装点 |
| **M3** | `search_chronicle` + `get_chronicle(summary\|full)` + 预算；自动 K 去重 | Director tool 白名单 |
| **M4** | **ChronicleCompressor**：A≥200→B、B≥200→C 与 covered 折叠 | 异步任务 + 存储字段；**不**改 Summarizer 热路径 |
| **M5** | 可观测面板/日志 + 固定剧本验收；厘清 Archiver 与 Chronicle 索引在召回中的 kind/source | — |

Quality 拦截 / NarrativeContract 可并行，但 **字段与注入顺序以本文件为准**。

---

## 10. 与 shujuku 的关系（参考边界）

| 借鉴 | 不照搬 |
| --- | --- |
| 稳定编码、短概览、按需展开 | 世界书 depth/order、每轮强制超长纪要 |
| 攒阈值批压、目录折叠 | 油猴表引擎整包 |
| 0TK/交火「正交开关」思想 | 与 ST prompt_order 等价运行声明 |

脚本参考（外部）：`https://gcore.jsdelivr.net/gh/AlbusKen/shujuku@spv5.5.7/index.js`（SP·数据库 V；交火=纪要索引，0TK=大纲条目 enabled）。

---

## 11. 文档索引（防找不到）

| 文档 | 关系 |
| --- | --- |
| **本文件** | **记忆/Context 装配权威规格** |
| `docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md` | 总架构与阶段 A–D；§8 ContextCompiler / §7 History Epoch 的细化落地见本文件 |
| `docs/ARCHITECTURE.md` | 模块边界；记忆装配见本文件链接 |
| `docs/HANDOFF.md` | 交接入口；下一优先级含本规格实现 |
| `docs/AGENT_INTERFACES.md` | Agent 职责；Director 工具、Summarizer、**ChronicleCompressor**、MemoryArchiver 对照本文件 §6–7 |
| `docs/DATA_MODEL.md` | 领域模型；Chronicle/RoundSummary/ArchivedSummary 以本文件 §3、§7 为准 |

实现时若与旧注释冲突（如「inject last-5」「仅 top-3 远记忆」），**以本规格目标态为准**，并在 PR 中更新旧注释。

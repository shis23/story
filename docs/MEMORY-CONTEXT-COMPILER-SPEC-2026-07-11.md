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
| RoundSummary | Summarizer 产出；**Accept 后**落盘；load 最近 12、**tail 自动 inject 约 5 条截断 content** |
| 远记忆 | Accept 后索引向量库；写作时 `intent → hybrid → top≈3` 进 tail |
| 二次压缩 RoundSummary 金字塔 | **无** |
| 概览字段 / 稳定 code / Director 点名 tool | **无**（仅有近期摘要工具向能力，非本规格完整形态） |
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
- 长程任务/承诺：**任务与状态 Agent** 兜底，不依赖最底层 A 永远在目录里。

可选后续：日常「每 4 个 A → 1 个 B」流水；**默认规格以 200 阈值批压为准**。

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

## 7. 写入路径

```text
Editor 成文
  → Summarizer → A（code, headline, summary[/full]）候选
  → Postprocess → 知识/变量/任务（非纪要）
  → Accept
       → A 落盘 + 索引（keyword/headline/entities/可选 vector）
       → 若 active A ≥ 200：调度压缩 → B，标记 covers/covered_by
       → 同理 B → C

消息过长归档（现有 Archiver）
  → ArchivedSummary 并入「可检索平面」，kind 区分
  → 默认概览以 Chronicle A/B/C 为主；归档块主要走召回/search
```

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
| **M1** | `code`+`headline`；概览注入（cap=200）；近窗与摘要硬去重；H/S 参数 | Summarizer 输出/解析 |
| **M2** | 装配顺序改为 概览→纪要带→近正文→tail；E 同步滑动；epoch 对齐 | MessageLayout 组装点 |
| **M3** | `search_chronicle` + `get_chronicle(summary\|full)` + 预算；自动 K 去重 | Director tool 白名单 |
| **M4** | A≥200→B、B≥200→C 后台压缩与 covered 折叠 | 异步任务 + 存储字段 |
| **M5** | 可观测面板/日志 + 固定剧本验收 | — |

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
| `docs/AGENT_INTERFACES.md` | Agent 职责；Director 工具与 Summarizer 对齐本文件 §6–7 |
| `docs/DATA_MODEL.md` | 领域模型；Chronicle/RoundSummary 字段扩展以本文件 §3 为准 |

实现时若与旧注释冲突（如「inject last-5」「仅 top-3 远记忆」），**以本规格目标态为准**，并在 PR 中更新旧注释。

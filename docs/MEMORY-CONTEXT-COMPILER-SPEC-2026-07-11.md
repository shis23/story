# 记忆与 Context 装配规格（远楼不失忆 + 缓存友好）

> 日期：2026-07-11  
> 版本：**v1.0 可拆任务实施规格**（目标态；下文「现状」与「目标」分列）  
> 修订依据：架构讨论拍板 + 两轮外部审查收紧（epoch 公式、快照、lineage、revision、存储主从、确定性分组、工具边界）  
> 相关：`docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md` §7–8、§10 阶段 C  
> 实现落点：`crates/domain/src/chronicle.rs`（**M0 纯函数/类型**）、`crates/domain/src/agent.rs`（RoundSummary Chronicle 字段）、`crates/domain/src/conversation.rs`（history-epoch）、`crates/app-pipeline`（inject budgets / layout）、`crates/app-memory`（归档/召回）、`crates/app-agent`（Summarizer / tools）、`crates/tauri-app`（Accept 质量拦截、code 分配、向量 source_*）

---

## 1. 问题与目标

### 1.1 要解决的问题

1. **远楼失忆**：近窗 history 丢掉正文后，旧情节几乎只靠 hybrid 检索 top-K 进 prompt，命中不稳。  
2. **正文与纪要双税**：近几轮既有 history 原文，又在 tail 贴 RoundSummary 全文。  
3. **目录过薄或无限增殖**：要么只自动贴约 5 条摘要，要么若不折叠则概览无限涨。  
4. **缓存前缀被每轮滑动打穿**：记忆块若每轮各挪 1 格，system 之后的共同前缀无法复用。

### 1.2 目标

- 近楼靠 **正文** 连戏；中距靠 **短纪要**；远楼靠 **厚概览目录 + 点名/检索展开 + 小 K 自动兜底**。  
- 状态层（知识/变量/任务）是 **规范事实**；Chronicle 是 **叙事回忆与导航**，冲突时听状态/原文/权限。  
- **MessageLayout 物理顺序：旧 → 新 / 稳 → 不稳**，保护供应商前缀缓存。  
- 压缩采用 **攒满阈值再批压**（实验默认 active A 达阈值 → 固定比例分组压 B），减少结构抖动轮次。  
- 一次写作启动时 **冻结** `campaign_revision` + `chronicle_revision` + `context_epoch` 快照，后台压缩不污染进行中的 Turn。

### 1.3 非目标

- 不整包移植 shujuku / 酒馆世界书 depth。  
- 不强制每轮 300+ 字长纪要全文进主模型。  
- 不删除底层 Chronicle 记录（折叠只影响**默认注入目录**）。  
- 不替代任务/知识系统做长程承诺。  
- 不将 `200→50` 等数字锁成不可调产品真理（见 §4.2 实验参数）。

### 1.4 架构决策 vs 实验参数

| 类型 | 内容 | 变更含义 |
| --- | --- | --- |
| **架构决策（锁死）** | 三窗分层；epoch 冻结概览/纪要带；旧→新物理序；状态为事实真相；A/B/C 折叠不删库；职责拆分；小 K 自动召回 + tool 点名；确定性分组压缩；独立 chronicle_revision | 改 = 改架构 |
| **实验默认参数** | `H_anchor`、`E`、`S`、`overview_max_entries`、compress 阈值与组大小、tool 次数上限 | 可由长会话/缓存/损失测试调整，**不**等于推翻架构 |

---

## 2. 现状（代码事实，2026-07-11 更新）

| 机制 | 行为 |
| --- | --- |
| history | `DEFAULT_HISTORY_WINDOW_SIZE=20` 消息条 + epoch 整块前移；可有确定性 `【历史纪要】checkpoint` |
| RoundSummary / Chronicle A 字段 | **Summarizer** 产出 content；`build_mutation_batch` 分配 `code`（A####）+ `headline` 截断；兼容字段 `lineage_id`/`covered_by`；`to_chronicle_a()` 纯转换；**Accept 后**落盘 |
| MemoryArchiver | **已有**：对话**消息正文**过长时批压 → `ArchivedSummary` 入向量库（水位 `archived_upto`） |
| 远记忆召回 | RoundSummary 索引向量库（metadata 含 `source_kind=chronicle_a` / `code` / `lineage_id`）；`intent → hybrid → top≈3` 进 tail |
| M0 公式与类型 | **`crates/domain/src/chronicle.rs`**：身份、lineage、revision 规则、epoch 成员、`ContextEpochSnapshot`、`compile_history_blocks`、compress 分组/`covers` 校验、overview 选择；单测覆盖 |
| epoch 快照接线 / 概览进 history | **已接线**：`Campaign.context_epoch` 持久化；`fill_campaign`/`start_writing`/`regenerate` 编译入口 `refresh_context_epoch`（满 E rollover + chronicle_revision bump）；Director history 前缀 + tail 硬去重 |
| `search_chronicle` / `get_chronicle` | **已完成（可独立部分）**（Director；A/B/C 目录 + 每轮预算；数据源=RoundSummary 兼容视图） |
| ChronicleCompressor 后台 | **已接线 + 可恢复队列**：claim；Pending Accept 再 spawn；publish/heal **Campaign 提交锁**；marker 含 child→parent，校验通过才清 marker |
| 正文与摘要硬隔离 | domain 过滤 + 主路径 `filter_history_to_near_raw_turns`（启发式按 near_turns 收敛对话对；前缀概览/纪要保留） |
| QualityGate Accept | **Error 拦截**；`force_accept` → Turn **Degraded**；Warning 不拦 |
| Campaign 版本 | `revision` + **`chronicle_revision`** + **`lineage_id`**（新建/fork 分配；Accept 新 A 时 bump chronicle_revision） |

---

## 3. 身份、主从关系与版本

### 3.1 规范主从（M0 锁死）

```text
Chronicle A     = Accept 后该 Turn 的【规范叙事纪要】（目标真相源）
RoundSummary    = 兼容视图 / 迁移源；演进为 Chronicle A 的存储形态，禁止长期双写两套库
ArchivedSummary = 对【消息正文】的额外压缩检索材料，不是规范轮次纪要
Campaign 状态   = 知识/变量/任务/时钟等【规范事实】
```

**存储策略（禁止长期双真相）：**

1. **推荐**：演进现有 RoundSummary 持久化模型，增加 Chronicle 字段，使其 **就是** Chronicle A 的兼容反序列化形式；或  
2. 一次性迁移到 `ChronicleStore`，RoundSummary API 变为只读适配层。  

**禁止**：同时持续写入 `round_summaries` 与独立 ChronicleStore 两套完整副本。

向量索引统一字段（M1 起写入）：

```text
source_kind: chronicle_a | chronicle_b | chronicle_c | round_summary_legacy | archived_message
source_entry_id: UUID          // Chronicle 或 ArchivedSummary 主键
source_turn_ids: [...]
source_content_hash: ...
lineage_id: ...
campaign_id, conversation_id
code?: string                  // 仅 Chronicle 有
```

检索去重优先级：

```text
1. Chronicle code 精确命中
2. Chronicle A/B/C（按 level/相关分）
3. round_summary_legacy
4. archived_message（ArchivedSummary）
```

### 3.2 Chronicle 身份模型

```text
chronicle_entry_id : UUID          // 唯一主键
code               : "A0123"       // 人类/模型可读别名，非全局主键
scope              : (campaign_id, lineage_id)
level              : 0 | 1 | 2     // A | B | C
headline, summary, full?
turn_start, turn_end               // 含端点的闭区间，按「已提交 Turn 序号」
covers: [chronicle_entry_id...]    // 系统填写，非 LLM 自由决定
covered_by: chronicle_entry_id?
source_turn_ids, source_variant_hashes, source_campaign_revision
origin_campaign_id?, origin_chronicle_id?, origin_code?   // fork 追溯
```

- **code 作用域**：在 `(campaign_id, lineage_id)` 内唯一；格式 `A|B|C` + 零填充序号。  
- **fork**：新 Campaign 分配**本地** code 序列；保留 `origin_*` 指向源条目。  
- **truncate / 换 branch**：仅 **当前 lineage 适用** 的条目进入 active 集合（算法见 §3.3）。

### 3.3 lineage_id（M0 锁死，无问号）

**采用显式 `lineage_id` 字段**（字符串 UUID），绑定「当前对话分支可用的记忆线」：

```text
lineage_id 创建：
  - 新建 Campaign 对话：new UUID
  - fork_at：new lineage_id；复制/引用适用前缀条目并写 origin_*
  - 不使用「仅 conversation_id」代替 lineage（同一 conversation 可有废弃分支语义时不够）

条目适用于当前分支 ⇔
  entry.campaign_id == active.campaign_id
  AND entry.lineage_id == active.lineage_id
  AND（若存在失效标记）entry.invalidated_at is null
```

第一版若不做复杂 branch 图：`lineage_id` 可与「主写作 conversation 的当前 head 链」1:1，但字段必须存在，算法唯一。

### 3.4 campaign_revision vs chronicle_revision

| 版本 | 含义 | 何时递增 |
| --- | --- | --- |
| `campaign_revision` | 叙事/状态事实 CAS（变量、知识、任务、Final 正文提交等） | 现有 TurnCommit / MutationBatch |
| `chronicle_revision` | **记忆表示**版本（影响下轮 Context 编译输入） | 见下表 |
| `context_epoch_id` | 当前编译用的三窗快照 id | rollover 时 |

**`chronicle_revision` 在以下情况必须递增（原子发布）：**

1. Accept 新增 Chronicle A  
2. Context epoch rollover（更新概览/纪要带成员）  
3. A→B 或 B→C 压缩结果**发布**  
4. fork / import / migration 改变可用 Chronicle 集合  
5. 历史失效导致 active lineage 成员变化  

**一次 `start_writing` / regenerate 编译：**

```text
ContextSnapshot {
  campaign_revision,      // 捕获
  chronicle_revision,     // 捕获
  epoch_id,               // 捕获或沿用未满 E 的 epoch
  ...
}
```

该 Turn 全程只用此快照；后台新 `chronicle_revision` **不得**改写进行中编译。

### 3.5 ContextEpochSnapshot（最小持久化）

**逻辑快照**用于保证同 epoch 内概览/纪要带 byte 稳定；**第一版不强制存完整渲染文本**。

最小持久化字段：

```text
ContextEpochSnapshot {
  epoch_id: string
  source_head_turn_id: Id     // 见 §4.1 定义
  overview_codes: [code]      // 有序；编译时再取 headline
  band_codes: [code]          // 纪要带成员
  raw_anchor_turn_ids: [Id]   // 近正文锚点 Turn 列表（有序）
  compiler_version: string    // 规格/实现版本，防算法漂移假稳定
  chronicle_revision: u64
  source_hash: string         // 对上述成员 + 关键正文/摘要 hash 的摘要
}
```

Prompt 字符串由 **纯函数** `compile(snapshot, live_suffix_messages, tail_inputs) -> MessageLayout` 生成。

---

## 4. Epoch 成员公式与窗口参数

### 4.1 精确术语（禁止「约」）

以 **已提交（Committed）Turn** 为时间单位（一轮 = 用户意图 + 已 Accept 的 AI 定稿所对应的 Turn 记录；实现映射到 conversation 节点对时必须文档化同一函数 `committed_turns_in_order(lineage)`）。

```text
H_anchor : u32 = 5     // 实验默认：epoch 开始时近正文锚点轮数
E        : u32 = 10    // 实验默认：一个 context epoch 内允许追加的「新提交 Turn」个数上限
S        : u32 = E     // 纪要带宽度；默认与 E 相同，与三窗同步边界

// 创建或 rollover 得到新 epoch 时：
epoch_start_head =
  创建快照时，lineage 上「最后一个已提交 Turn」
  （若尚无已提交 Turn：epoch_start_head = null，近正文为空，仅 tail 意图）

anchor_turns =
  若 epoch_start_head == null: []
  否则: 在已提交序列中，以 epoch_start_head 为末元素，向前取 min(H_anchor, 已有数量) 个 Turn
  （闭区间，时间升序）

// epoch 存活期间：
live_suffix_turns =
  严格晚于 epoch_start_head 的、本 lineage 新提交的 Turn，按时间升序
  （epoch 刚创建时为空）

live_suffix_count = live_suffix_turns.len()

// 近正文（注入 history 的原文轮）=
near_raw_turns = anchor_turns ++ live_suffix_turns
// 时间升序，旧→新

// 最大值（写死）：
// 当 live_suffix_count 达到 E 时，第 E 个新 Turn 仍属于本 epoch（先加入 live_suffix，再在「下一次 Context 编译前」rollover）
max_near_raw_turns = H_anchor + E
// 例：H_anchor=5, E=10 → 最多 15 轮正文，不是 14，不是「始终 5」
```

**Rollover 时机（写死）：**

```text
当 live_suffix_count == E 时：
  不在「产生第 E 个 Accept 的同一临界区」内同步重算 prompt；
  在「下一次需要 Context 编译」之前（下一次 start_writing/regenerate 编译入口）执行 rollover：

  1. 冻结旧 ContextEpochSnapshot（已有则保留历史可选）
  2. 将「退出近正文、进入中距」的轮次对应 A 纳入新纪要带成员算法
  3. 重算 overview_codes / band_codes / raw_anchor
  4. chronicle_revision += 1（若成员变化）
  5. 发布新 epoch_id；新 epoch_start_head = 当前最后一个已提交 Turn
  6. 新 anchor_turns = 以新 head 为末的 H_anchor 轮
  7. live_suffix 清空
```

**当前 epoch 内新 Accept 的 A：**

- **写入** Chronicle 存储与索引，递增 `chronicle_revision`。  
- **不**加入本 epoch 的 `overview_codes`（正文仍在 near_raw 中）。  
- 待 **rollover** 后再按规则进入纪要带或概览候选。

### 4.2 纪要带与概览成员（epoch 冻结）

```text
// rollover / 创建 epoch 时计算一次，写入 snapshot；epoch 内不变

band_turns =
  紧邻 anchor_turns 之前、长度最多 S 的已提交 Turn 序列
  （若总历史不足则变短）

band_codes =
  每个 band_turn 对应的规范 Chronicle A（若存在且 active/uncovered）
  按 turn 升序；只含 short summary 注入，不含 full

overview 选择（先选后排）：
  candidates =
    所有 active lineage 上 uncovered 的 B/C
    + uncovered 的 A 中「turn 严格早于 band_turns 最早一轮」的条目
    + 可选 pinned
  选择策略：
    1. 先纳入全部 B/C 骨架（若超预算再按 turn_start 从最旧 B/C 丢弃，保留较新）
    2. 再纳入最近的 uncovered 远 A，直到触达 overview_max_entries 或 overview_max_tokens
  最终排序：
    严格按 turn_start 升序（旧→新），禁止按 level 打乱时间

overview_codes = 排序后的 code 列表（仅 headline 进 prompt）
```

### 4.3 预算（token 主，行数辅）

```text
overview_max_entries = 200          // 实验默认
overview_max_tokens  = 由模型上下文与费用策略配置；注入取 entries 与 tokens 先到者

headline 硬截断：按字符上限（如 40 字）截断；token 以 Provider/保守估算为准，禁止写死「40 字=40 token」

全局装配预算（概念）：
  system 固定规则
  + 事件概览（≤ min 行/token 帽）
  + 纪要带
  + 近正文最坏 H_anchor+E 轮
  + Campaign 状态
  + tool 预留
  + 输出预留
  ≤ model context budget
```

### 4.4 实验默认参数表

| 符号 | 实验默认 | 含义 |
| --- | --- | --- |
| `H_anchor` | 5 | 近正文锚点轮数 |
| `E` | 10 | epoch 内最多追加新提交轮数；满则下次编译前 rollover |
| `S` | = E | 纪要带轮数 |
| `overview_max_entries` | 200 | 概览最大行数 |
| `auto_recall_k` | 1～2 | 自动 hybrid 兜底 |
| `tool_summary_max` / 轮 | 6 | get summary |
| `tool_full_max` / 轮 | 2 | get full |
| `search_max` / 轮 | 2～3 | search |
| `compress_active_A_threshold` | 200 | 实验默认阈值 |
| `compress_group_size` | 4 | 每组连续 A 数 → 1 个 B（200/4=50） |
| `compress_active_B_threshold` | 200 | 同理 → C |

---

## 5. 发给 Director 的 prompt 物理结构（目标）

**语义远近**：近正文 → 中纪要 → 远概览。  
**物理顺序**（旧→新 / 稳→不稳）：

```text
[system]
  Director 职责
  + 稳定契约句：「Chronicle/概览/纪要是叙事回忆与导航；与 Campaign 结构化状态或原文冲突时，以状态、权限与原始正文为准。」
  + NarrativeContract 稳定切片 + 常驻 lore
  （会话内尽量 byte 稳定；免责声明放这里一次，不在每个 tool 结果重复长文）

[history]  时间正序：
  (0) 【可选】确定性消息 checkpoint
        仅当仍启用且内容严格早于概览所覆盖范围
        位置：整段 history 最前（旧于概览）
        Chronicle 金字塔稳定后可关闭，避免三套摘要抢戏
  (1) 【事件概览】snapshot.overview_codes → headline 行
  (2) 【中距纪要带】snapshot.band_codes → short summary
  (3) 【近正文】near_raw_turns 对应 user/assistant 原文
        = anchor ++ live_suffix（最多 H_anchor+E 轮）

[tail]  每轮新建：
  本轮用户意图
  Campaign 状态/任务/变量
  自动 recall top-K（仅 tail 末；与 overview/band/tool 按 source_entry_id 去重）
  tool 结果
  「请输出 Plan/ScenePlan」
```

### 5.1 硬去重

1. `turn ∈ near_raw_turns` → 只发正文；禁止同 turn 的 A summary/full 进纪要带或 tail。  
2. `turn ∈ band_turns` → 只发该 turn 短纪要；禁止再塞同 turn 正文。  
3. `covered_by != null` → 默认不进 overview/band。  
4. 自动召回 / tool 与已注入块同一 `source_entry_id` 或 code → 去重。

### 5.2 同 epoch 稳定性

- 编译使用捕获的 `ContextEpochSnapshot`：`overview_codes` / `band_codes` / `raw_anchor` **不变**。  
- 仅 `live_suffix` 对应正文在 history **末尾追加**。  
- 新 A 入库存但不进本 epoch 概览（§4.1）。  
- rollover / 压缩发布 → 新 snapshot → 预期前缀变化。

### 5.3 其他 Agent

| Agent | 概览 | 纪要带 | 近正文 | chronicle tool |
| --- | --- | --- | --- | --- |
| Director | 满配 | 有 | 有 | 有 |
| Editor | 可瘦 | 可选 | 同 epoch 正文 | 可选 |
| Subagent | 默认无 | 无 | 默认无全局 | **默认无** |
| Summarizer / Postprocess | 无 | 无 | 本轮成文等 | 无 |

---

## 6. 工具契约（目标）

### 6.1 `search_chronicle`（v1 范围收窄）

**第一版只搜索 Chronicle A/B/C**（有 code 的规范纪要）。  
**ArchivedSummary 不进入 `search_chronicle`**，仅参加 **auto recall**（及既有向量检索若保留）。

```text
入参: query, optional level/code_prefix/turn_range/include_covered, limit
出参: [{ code, level, headline, turn_span, score, covered_by?, chronicle_entry_id }]
不含 full
```

### 6.2 `get_chronicle`

```text
入参: code 或 chronicle_entry_id; detail = summary | full（默认 summary）
出参:
  code, level, headline, summary|full
  chronicle_entry_id
  source_turn_ids, source_variant_hash, source_kind
  covered_by?, covers?（元数据）
  coverage/confidence 可选短字段
不含大段重复免责声明（见 system 契约）
```

预算：§4.4。

### 6.3 自动召回

- `auto_recall_k` 小；结果仅 tail。  
- 可含 ArchivedSummary 与 Chronicle 索引 hit；去重顺序见 §3.1。

---

## 7. 写入侧组件

### 7.1 谁写哪一层

| 产出 | 执行者 | 状态 | 原料 | 触发 |
| --- | --- | --- | --- | --- |
| **A** | **Summarizer** | 已有 | 本轮成文 | 成文后并行；Accept 落盘为规范 A |
| **B/C** | **ChronicleCompressor** | **已实现**（Accept 后后台；LLM 失败降级确定性文案） | 连续分组的 A/B 的 headline+summary | 阈值批压，后台 |
| **ArchivedSummary** | **MemoryArchiver** | 已有 | 消息正文 | archived_upto 水位 |
| 知识/变量/任务 | **PostProcessor** | 已有 | 成文 | 并行；**不写纪要** |
| 消息 checkpoint | 本地确定性 | 已有 | 掉窗消息 | 组装时；可退役 |

### 7.2 Summarizer（已有）

- 代码：`crates/app-agent/src/prompts/summarizer.rs`  
- 目标输出：可解析为 A 的 headline + summary；code 可由系统分配。  
- **不**做 A→B→C。

### 7.3 ChronicleCompressor（已实现，可恢复）

- 后台异步；不进成文热路径。  
- **持久化任务队列**（`data/compress_jobs.json`）：
  - Accept 达阈值 → `enqueue_or_get_open`（同 campaign 去重 open job）
  - worker：`Pending|Running` → mark Running → LLM/降级发布 → Succeeded / 失败回 Pending 或 Failed
  - 启动：`Running→Pending` 后重放所有 open job  
- **确定性分组（锁死）**：

```text
输入：按 turn_start 排序的 N 个未覆盖 A（N ≥ threshold）
系统：切成连续不重叠组，每组 compress_group_size 个（末组可短）
LLM：仅为每组生成 headline + summary
系统：填写 covers = 该组 entry_id 列表；covered_by；turn_span = 子项并集
校验（失败则整批不发布）：
  - 每个输入恰好出现在一个 covers 中
  - covers 两两不相交
  - 每组 covers 时间连续
  - turn_span 与子项一致
B→C 同理
```

- 发布成功 → `chronicle_revision += 1`；不随意 bump `campaign_revision`。

### 7.4 MemoryArchiver（已有）

- 代码：`crates/app-memory/src/archiver.rs`  
- 消息批压 → 向量库；**不是**规范轮次纪要。  
- 读：auto recall；**v1 不进 search_chronicle**。

### 7.5 PostProcessor（已有）

- 仅知识/变量/任务；与 Summarizer 并行。

### 7.6 写入总览

```text
Editor 成文
  ├─ Summarizer → 候选 A
  └─ PostProcessor → 状态三件套
Accept
  ├─ A 落盘（规范）+ 索引 + chronicle_revision++
  ├─ campaign 状态提交（campaign_revision 规则不变）
  └─ 若达阈值 enqueue Compressor
独立：MemoryArchiver 水位
编译：捕获 campaign_revision + chronicle_revision + epoch snapshot
```

---

## 8. 可观测与验收

### 8.1 日志

```text
epoch_id, chronicle_revision, campaign_revision
live_suffix_count, near_raw_count
overview_codes, band_codes
auto_recall source_entry_ids
tool_reads[{id|code, detail}]
compress_batch_id?
```

### 8.2 验收

1. 远楼线索在概览、search（Chronicle）、状态层或 auto recall 至少一处可达。  
2. near_raw 内同 turn 不出现 A 全文双税。  
3. 同 epoch 两次编译：overview+band 的 codes 与渲染 hash 稳定；仅 near_raw 后缀变。  
4. `live_suffix_count` 到 E 后，**下一次**编译发生 rollover，三窗同更新。  
5. 压缩后 covers 校验通过；旧 A 默认不在 overview；`get_chronicle(A)` summary 仍可读。  
6. 关 embedding：Chronicle 概览 + search 仍可用。  
7. 进行中 Turn 不受中途新 chronicle_revision 影响。

---

## 9. 实施分期（M0→M5）

| 期 | 内容 | 状态（2026-07-11） |
| --- | --- | --- |
| **M0** | 本规格公式与类型：Chronicle 身份、lineage 算法、revision 规则、epoch 成员公式、ContextEpochSnapshot 最小字段、Compiler 纯函数 IO；单测覆盖公式与分组校验 | **已完成**（`domain/chronicle.rs`） |
| **M1** | Accept 后 RoundSummary **演进为**规范 Chronicle A（兼容反序列化）；向量 source_* 字段；**暂不改**主写作 prompt 布局 | **完成（可独立部分）**：字段 + code/headline/lineage 分配 + 索引 metadata；加载路径缺 lineage 回填落盘 |
| **M2** | epoch 快照；概览/纪要带/近正文装配；硬去重；token∩行数预算；关闭同 turn 双税 | **完成（可独立部分）**：快照+`chronicle_prompt_catalog` 渲染 + near_raw history 收敛；token 全局预算编译器仍可扩展 |
| **M3** | `search_chronicle`（仅 A/B/C）+ `get_chronicle`；工具预算；来源字段 | **完成（可独立部分）**：全量工具目录（点名旧 A）；B/C `source_turn_ids` 展开 covers；每轮预算；`code_prefix`/score/full token 帽仍可扩展 |
| **M4** | ChronicleCompressor 确定性分组 A→B→C；幂等后台任务；covered 折叠 | **完成（可独立部分 / M4.2.2）**：claim；Pending Accept 重试；publish/heal + **epoch refresh** 同 Campaign 锁；marker 校验后完成 |
| **M5** | 真实模型缓存/远楼/压缩损失验收；参数标定（含是否调整 200/4） | **部分完成（2026-07-11）**：`harness-real-llm` `m5_*` + `run-real-llm-smoke.ps1 -Suite m5`；见下「M5 实跑记录」 |

并行已落地：**Quality Error 拦截 + force → Degraded**。NarrativeContract / UnitOfWork 仍独立。**记忆语义以本文件为准**。实现常量 `CONTEXT_COMPILER_VERSION` 现为 `memory-spec-v1.0-m4.2.2`（防漂移假稳定）。

### M5 实跑记录（2026-07-11，脱敏）

| 项 | 值 |
| --- | --- |
| 执行 | `cargo test -p harness-real-llm --test m5_cache_and_memory -- --ignored`（S1–S5 分跑） |
| 基线 commit | `1b7db7d`（M4.2.2）+ 本轮 m5 harness |
| endpoint host | `cli.2529985.xyz`（OpenAI 兼容 `/v1`） |
| model | 请求 `grok-4.5`（响应侧曾见 `grok-4.5-build`） |
| 结论等级 | **Partial Pass**（路径/工具/压缩达标；写作轮供应商 `cached_tokens` 多为 0） |

| Suite | 结果 | 要点 |
| --- | --- | --- |
| **S3** compress loss | **Pass** | 阈值 8 / group 4 → 2×B；covers 8；事实 token **3/3**；`get_chronicle(A0001)` 仍可读；~42s |
| **S2** far floor | **Pass** | 注入 A0001 远楼 token + 近轮填充；**无 embedding**；`search_chronicle`/`get_chronicle` 命中；catalog=20 |
| **S1** same-epoch cache | **Partial** | 6 轮成文全非空；各轮 **最大 prompt 请求 `system_hash` 六轮一致**（`225bab02…`）；写作轮 `cached_tokens=0`；仅 extract boot 见高 cache（~2944/3005）；harness **未 Accept** → `summaries=0`、`turn` 停在 1、catalog 空（与线上 Accept 写 Chronicle 路径不同） |
| **S4** epoch rollover | **Partial（路径 Pass）** | inject 后 epoch `ctx-epoch-empty` → `ctx-epoch-committed-turn-25`，rev 1→2，overview/band 重建；cold/hot 写作仍 `cached=0` |
| **S5** long session | **Pass（稳定性）** | 8/8 成文；约 41 LLM 调用；合计 prompt≈145k、completion≈41k、wall≈372s；cache 几乎仅 boot；`summaries_on_disk=0`（S1 路径；见 S6 闭环） |
| **S6** Accept 闭环 | **Pass（路径）+ cache Partial** | 4 轮「写作→`accept_variant`→summarizer-only→落 A」：`accepted=4/4` `summarized=4/4` codes `A0001…A0004`；`fill.turn`→5、`catalog=4`；Director 最大 prompt **system_hash 四轮一致**；**写作轮 `cached_tokens` 仍全 0**；**summarizer 四轮均 `cached=128`**（稳定小命中）。说明：无 Accept 时的 catalog/turn 缺口已补；写作前缀热 cache 仍像网关/供应商侧 |

**参数标定建议（本轮不改生产默认）**：

- **不调整** `compress_active_A_threshold=200` / `group_size=4`：S3 在测试阈值下事实保留满额，无证据要求改默认。
- **不调整** `H_anchor`/`E`：S2 远楼可达；S4 能刷新 epoch。
- **cache 结论**：S6 已排除「无 Accept / 无 catalog」主因；写作轮仍 0 cache + summarizer 固定 128 → 优先标 **供应商/网关 usage 策略**，非 system 抖动。可选后续：第二供应商对照；非必须再扩轮次。

---

## 10. 与 shujuku 的关系

| 借鉴 | 不照搬 |
| --- | --- |
| 稳定编码、短概览、按需展开、批压折叠 | 世界书 depth、强制超长纪要、油猴表引擎 |

外部参考：`https://gcore.jsdelivr.net/gh/AlbusKen/shujuku@spv5.5.7/index.js`

---

## 11. 文档索引

| 文档 | 关系 |
| --- | --- |
| **本文件** | 记忆/Context **v1.0 权威规格** |
| `docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md` | 总架构；落地以本文件为准 |
| `docs/ARCHITECTURE.md` | 模块边界链接 |
| `docs/HANDOFF.md` | 下一优先级 |
| `docs/AGENT_INTERFACES.md` | Agent/工具对照 §6–7 |
| `docs/DATA_MODEL.md` | Chronicle / RoundSummary / ArchivedSummary 主从 |

实现若与旧注释冲突（inject last-5、仅 top-3、恒定 H=5 等），**以本 v1.0 为准**并在 PR 更新注释。

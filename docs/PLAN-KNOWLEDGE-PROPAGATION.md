# 计划：知识传播引擎

> 状态：草案（方向已定，细节待立项展开）
> 来源：P3 门禁重定位过程中，由用户场景质询推导出的完整设计问题。
> 前置文档：`HARNESS-FINDINGS-2026-06-18.md` §P3、`INTENT.md` D30-D31/D39-D40
> 归属：ROADMAP Phase 2（信息隔离）增强项

## 背景：为什么有这个计划

P3（postprocess 写回门禁）原标为"空集逃生口收紧/保留"取舍。经用户场景质询（世界公告、身份组传播、写信）推翻"收紧"建议后发现：**门禁用"在场"一刀切所有知识来源是病根，空集逃生口只是症状补丁**。

P3 的最小修复（门禁按 `KnowledgeSource` 分流）已在 worktree `w3-p3p4` 落地，能让广播/告知天然成立。但**身份组传播、传话链、秘密封口**是更大的功能，超 P3 范围，单独立本计划。

## 目标

让角色间的知识传播从"postprocess 临场脑补"升级为"有明确规则、可追溯、可约束"的机制。

## 非目标

- 不重写 `CharacterKnowledgeEntry` 的核心结构（`character_id` 绑定保留——每条知识仍归属于某个角色"知道"）。
- 不引入向量检索之外的召回方式。
- 不在本阶段做 LLM 自主传播决策（仍由 postprocess 抽取，引擎只执行规则）。

## 现状摸底（已核实）

**数据模型已有支撑**：
- `KnowledgeSource` 四元分类（`domain/src/character_knowledge.rs:17`）：`Witnessed`/`ToldByOther`/`Inferred`/`Backstory`。`ToldByOther` 已记 `source_character_id`——**告知链可追溯**。
- 角色身份字段已存在：`Character.group: Option<String>`、`CharacterDefinition.role_type`、`VariableField.group`。**身份组载体已就位**，只是知识传播层没用。
- 变量层已有"全局/无归属"概念：postprocess prompt 明确"全局变量（无 instance_id）用于 story_clock/weather"。**世界级状态有家可归，世界级知识没有**——这是缺口。

**门禁现状**（P3 修复后）：按来源分流，`ToldByOther`/`Backstory` 不受"在场"约束。

## 场景清单（A-E）

| ID | 场景 | 语义 | P3 分流后是否成立 | 本计划是否需补 |
|---|---|---|---|---|
| A | 世界公告/广播 | N 个角色都该知道，与在场无关 | ✅ 部分（空集 + ToldByOther 放行） | 补：显式广播语义，不靠空集 hack |
| B | 身份组传播 | 所有守卫/贵族该知道 | ❌ | 补：按 `group` 广播写入 |
| C | 定向告知/写信 | A 明确告诉不在场的 B | ✅（ToldByOther + source_character_id） | 补：postprocess 输出告知目标，门禁按目标放行 |
| D | 传话链 | A→B→C 跨轮传播 | 🟡 数据可追溯，无传播引擎 | 补：跨轮传播规则 |
| E | 秘密封口 | 某事只有 A 知，禁止外传 | ❌ | 补：反向约束标记 |

## 设计方向（草案，待立项细化）

### 方向 1：显式广播语义（场景 A）

放弃"空集 = 广播"的隐式约定。改为显式信号：
- Director 在 `present_chars` 用哨兵 `"*"` 表示广播，或
- postprocess 输出 `target: "broadcast"` 的知识更新。

门禁逻辑：`"*" 在场或 target=broadcast → 全放行`；`present_chars 空 且无广播信号 → Witnessed/Inferred 拒绝`（真 bug）。

P3 分流已让 `ToldByOther`/`Backstory` 在空集时放行，方向 1 是把"广播"从隐式提升为显式，与 P3 互补。

### 方向 2：身份组广播（场景 B）

扩展 `CharacterKnowledgeUpdate`，支持 `target_group: Option<String>`：
- postprocess 输出"所有守卫该知道 X"时填 `target_group: Some("守卫")`。
- 写入时按 `Character.group == target_group` 找出所有匹配 instance，各写一条 `ToldByOther`（source 记公告发起方）。
- 门禁：`target_group` 非空时按组放行，不查在场名单。

载体选 `Character.group`（最直接表达"这个角色属于哪伙"）。需确认：现有卡的 `group` 字段是否在用，还是多数为 None。若 None 居多，需在角色识别阶段补 group 推断。

### 方向 3：定向告知强化（场景 C）

P3 分流后 `ToldByOther` 已能跨在场。本方向补 postprocess 输出端：
- postprocess prompt 教会 LLM 输出"告知目标"（`target_character_id` 或 `target_name`），而非笼统的"某人知道了"。
- 写入时若目标不在场，仍写（分流已放行），`source_character_id` 记告知发起方。
- 读侧：子 Agent 注入时，`ToldByOther` 知识带"谁告诉的"上下文，便于 LLM 理解信息来源。

### 方向 4：传话链（场景 D，较大）

跨轮传播规则。例：A 告诉 B（轮 3），B 在轮 5 告诉 C。
- 需"谁现在知道"的可查询索引（已有：按 `character_id` 查 knowledge）。
- postprocess 在抽取时，若成文里 B 把已知信息告诉 C，应输出 C 的 `ToldByOther`（source=B）。
- 难点：postprocess 如何判断"B 此时愿意/能够传话"——这是 LLM 行为层，需 prompt 引导 + 可能的约束（B 是否在场、B 与 C 关系）。
- 本方向偏 LLM 行为，确定性测试难覆盖，归 Phase 7 LLM 评测。

### 方向 5：秘密封口（场景 E）

反向约束：标记某知识为"私有，禁止传播"。
- `CharacterKnowledgeEntry` 加 `propagation: PropagationPolicy` 枚举（`Open`/`Private`/`GroupRestricted`）。
- postprocess 抽取时，若成文里有人试图传播 `Private` 知识，应拒绝写入新告知。
- 难点：postprocess LLM 需理解"这条是秘密"——可能需在知识文本里带标记，或 pin 时标注。
- 本方向风险高（依赖 LLM 遵守约束），建议晚做。

## 落地优先级建议

1. **方向 1 + 3**（显式广播 + 定向告知强化）：P3 分流的自然延伸，工作量小，覆盖场景 A/C，让现有数据模型充分发挥。建议与 P3 同期或紧随。
2. **方向 2**（身份组广播）：中等，需扩 Update 结构 + 写入按组分发 + group 字段验证。覆盖场景 B。
3. **方向 4**（传话链）：大，偏 LLM 行为，归 Phase 7 评测。
4. **方向 5**（秘密封口）：风险高，最后做。

## 与 P3 的关系

- P3（w3-p3p4）= 门禁按来源分流的**最小正确修复**，让广播/告知不再依赖空集 hack。是本计划的前置。
- 本计划 = 在 P3 基础上补**显式广播语义 + 身份组 + 传话 + 封口**，把"传播"从隐式 hack 升级为显式机制。
- P3 落地后，`b3_empty_present_chars_should_reject_when_tightened` 占位测试需重写为"按来源分流"断言，本计划的广播语义落地后再补"显式广播"测试。

## 开放问题（待立项时决策）

1. `Character.group` 字段现状使用率？若多数 None，角色识别阶段是否补 group 推断？
2. 广播哨兵用 `"*"` 还是新增 `present_chars: Option<Vec<String>>`（None=广播）？前者改动小，后者类型更安全。
3. 传话链（方向4）是否纳入 MVP，还是明确推迟到 Phase 7？
4. 秘密封口（方向5）是否值得做——LLM 约束可靠性存疑，可能给人虚假安全感。

## 禁止改动

- 禁止为图快把 P3 的分流逻辑绕过（如全局放行所有 `ToldByOther` 不查来源）。
- 禁止在本计划里重写 `CharacterKnowledgeEntry` 核心字段（`character_id` 绑定是隔离基石）。
- 禁止让传播引擎绕过 postprocess 直接写知识（postprocess 是唯一写入入口，保证可审计）。

# 计划：知识传播引擎

> 状态：部分已实现（方向 1/2/3 已落地；方向 4/5 和更细可解释链路待立项）
> 来源：P3 门禁重定位过程中，由用户场景质询推导出的完整设计问题。
> 前置文档：`HARNESS-FINDINGS-2026-06-18.md` §P3、`INTENT.md` D30-D31/D39-D40
> 归属：ROADMAP Phase 2（信息隔离）增强项

## 背景：为什么有这个计划

P3（postprocess 写回门禁）原标为"空集逃生口收紧/保留"取舍。经用户场景质询（世界公告、身份组传播、写信）推翻"收紧"建议后发现：**门禁用"在场"一刀切所有知识来源是病根，空集逃生口只是症状补丁**。

P3 的最小修复（门禁按 `KnowledgeSource` 分流）已在 worktree `w3-p3p4` 落地，能让广播/告知天然成立。随后方向 1/2/3 已继续落地：显式广播、身份组广播和定向告知强化不再依赖空集 hack。**传话链、秘密封口和更细的“谁知道什么、从哪知道”的可解释链路**仍是更大的功能，继续保留在本计划里。

## 目标

让角色间的知识传播从"postprocess 临场脑补"升级为"有明确规则、可追溯、可约束"的机制。

## 非目标

- 不重写 `CharacterKnowledgeEntry` 的核心结构（`character_id` 绑定保留——每条知识仍归属于某个角色"知道"）。
- 不引入向量检索之外的召回方式。
- 不在本阶段做 LLM 自主传播决策（仍由 postprocess 抽取，引擎只执行规则）。

## 现状摸底（已核实）

**数据模型和写回链路已有支撑**：
- `KnowledgeSource` 四元分类：`Witnessed`/`ToldByOther`/`Inferred`/`Backstory`。`ToldByOther` 已记 `source_character_id`——**告知来源可追溯**。
- `BroadcastTarget::{All, Group(String)}` 已存在于 `crates/domain/src/character_knowledge.rs`，并由 postprocess DTO 的 `broadcast` 字段解析。
- Tauri 写回层已在 `normalize_knowledge_update_for_postprocess` / `dispatch_broadcast` 中按 `BroadcastTarget::All` 或 `BroadcastTarget::Group` 分发知识。
- 角色身份字段已存在：`CharacterDefinition.group: Option<String>`、`CharacterDefinition.role_type`、`VariableField.group`。身份组广播当前使用 `CharacterDefinition.group` 匹配 instance。
- 变量层已有"全局/无归属"概念：postprocess prompt 明确"全局变量（无 instance_id）用于 story_clock/weather"。**世界级状态有家可归，世界级知识没有**——这是缺口。

**门禁现状**（P3 修复后）：按来源分流，`ToldByOther`/`Backstory` 不受"在场"约束；`broadcast` 非空时走显式分发，不再用空集表达广播。

## 场景清单（A-E）

| ID | 场景 | 语义 | P3 分流后是否成立 | 本计划是否需补 |
|---|---|---|---|---|
| A | 世界公告/广播 | N 个角色都该知道，与在场无关 | ✅ 已实现 | 已补：`broadcast: "all"` → `BroadcastTarget::All` |
| B | 身份组传播 | 所有守卫/贵族该知道 | ✅ 已实现 | 已补：`broadcast: "组名"` → `BroadcastTarget::Group`，按 `CharacterDefinition.group` 分发 |
| C | 定向告知/写信 | A 明确告诉不在场的 B | ✅ 已实现 | 已补：postprocess 输出被告知者/告知者，读侧渲染来源名 |
| D | 传话链 | A→B→C 跨轮传播 | 🟡 数据可追溯，无传播引擎 | 补：跨轮传播规则 |
| E | 秘密封口 | 某事只有 A 知，禁止外传 | ❌ | 补：反向约束标记 |

## 设计方向（当前状态 + 后续草案）

### 方向 1：显式广播语义（场景 A，已实现）

放弃"空集 = 广播"的隐式约定，改为 postprocess 输出 `broadcast: "all"`：
- prompt 已明确 `broadcast: "all"` 表示广播给 Campaign 内所有角色。
- `postprocess.rs` 将 `"all"` 解析为 `BroadcastTarget::All`。
- 写回层分发给 Campaign 内所有 instance，并排除广播发起者自身。

仍需注意：全体广播当前会生成每个目标角色的 `ToldByOther` 知识，`source_character_id` 记录公告/广播发起者；若没有可解析发起者，则只保留知识本身。

### 方向 2：身份组广播（场景 B，已实现）

已通过 `BroadcastTarget::Group(String)` 落地：
- postprocess 输出"所有守卫该知道 X"时填 `broadcast: "守卫"`。
- `postprocess.rs` 将非空且非 `"all"` 的 `broadcast` 字符串解析为 `BroadcastTarget::Group(group)`。
- 写入时按 `CharacterDefinition.group == group` 找出所有匹配 instance，各写一条 `ToldByOther`（source 记公告发起方）。
- 门禁：`broadcast` 非空时按广播目标分发，不再查在场名单。

后续缺口：需要确认导入/角色识别阶段是否稳定填充 `CharacterDefinition.group`。若多数为 `None`，组广播能力存在但命中率会低。

### 方向 3：定向告知强化（场景 C，已实现）

P3 分流后 `ToldByOther` 已能跨在场。本方向已补齐 postprocess 输出端和读侧渲染：
- postprocess prompt 明确 `character_id` 是被告知者，`source_character_id` 是告知者。
- 写入时若目标不在场，`ToldByOther` 仍可写入，`source_character_id` 记告知发起方。
- 读侧：`render_knowledge_for_injection` 会在有 name resolver 时渲染“被告知，来源：X”，便于 LLM 理解信息来源。

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

1. **已完成：方向 1/2/3**（显式广播、身份组广播、定向告知强化）：现有代码已有 domain enum、postprocess prompt/schema/解析、Tauri 写回分发和读侧来源渲染。
2. **下一步：可解释链路增强**：在 UI / debug / Meta 里更清楚展示“谁知道什么、从哪知道、哪轮知道、是否广播/组广播产生”。
3. **方向 4**（传话链）：大，偏 LLM 行为，归 Phase 7 评测或单独 MVP。
4. **方向 5**（秘密封口）：风险高，最后做；需要避免给用户虚假的安全感。

## 与 P3 的关系

- P3（w3-p3p4）= 门禁按来源分流的**最小正确修复**，让广播/告知不再依赖空集 hack。是本计划的前置。
- 本计划 = 在 P3 基础上补**显式广播语义 + 身份组 + 传话 + 封口**，把"传播"从隐式 hack 升级为显式机制；其中前三项已落地，后两项仍待立项。
- 显式广播测试已覆盖 domain serde、postprocess 解析和 Tauri 分发；后续测试重点转向传话链、秘密封口和解释链路。

## 开放问题（待立项时决策）

1. `CharacterDefinition.group` 字段现状填充率如何？若多数为 `None`，角色识别阶段是否补 group 推断？
2. 传话链（方向 4）是否纳入 MVP，还是明确推迟到 Phase 7？
3. 秘密封口（方向 5）是否值得做——LLM 约束可靠性存疑，可能给人虚假安全感。
4. 可解释链路做到哪一层：仅 debug/Meta 可见，还是也进入正式 Campaign 知识 UI？

## 禁止改动

- 禁止为图快把 P3 的分流逻辑绕过（如全局放行所有 `ToldByOther` 不查来源）。
- 禁止在本计划里重写 `CharacterKnowledgeEntry` 核心字段（`character_id` 绑定是隔离基石）。
- 禁止让传播引擎绕过 postprocess 直接写知识（postprocess 是唯一写入入口，保证可审计）。

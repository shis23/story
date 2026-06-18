# W6 执行手册：知识传播引擎 — 方向 1（显式广播）+ 方向 3（定向告知强化）

> 交接对象：Claude Code（在 worktree `storyforge-w6-knowledge` 分支 `w6-knowledge` 工作）
> 前置必读：`docs/PLAN-KNOWLEDGE-PROPAGATION.md`（完整设计）、`docs/HARNESS-FINDINGS-2026-06-18.md` §P3
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w6-knowledge`
> 分支：`w6-knowledge`（已基于 `main` 64cd522）
> 性质：后端功能增强，与 W5（前端）零文件交集（W5 不改后端）。

## 一、任务概述

P3（已合并）让知识写回门禁按 `KnowledgeSource` 分流——`ToldByOther`/`Backstory` 不受在场约束。这是"被动放行"：postprocess 抽出来的告知/背景知识能写进去。

W6 做的是"主动执行"：让 postprocess 能输出**广播**（一条信息让一组/所有角色知道）和**定向告知**（A 明确告诉特定 B），写入时分发到多个目标。对应 PLAN-KNOWLEDGE-PROPAGATION.md 方向 1 + 方向 3。

**只做方向 1+3**。方向 2（身份组广播）、4（传话链）、5（秘密封口）不在 W6 范围（见 PLAN 优先级）。

## 二、方向 1：显式广播语义

### 问题

现状"广播"靠 P3 的空集放行 hack——`present_ids` 空时 `ToldByOther`/`Backstory` 全放行。这是隐式的、且 Witnessed/Inferred 已被 P3 在空集时拒绝。没有"这一条明确是广播给所有人"的显式信号。

### 改法

postprocess 能输出一条"广播知识"，写入时分发到 Campaign 内所有 instance（各写一条 `ToldByOther`，source 记广播发起方）。

**数据模型扩展**（`domain/src/character_knowledge.rs` `CharacterKnowledgeUpdate` :141）：
```rust
pub struct CharacterKnowledgeUpdate {
    pub character_id: Id,           // 现有：单角色目标
    pub knowledge_text: String,
    pub source: KnowledgeSource,
    pub source_character_id: Option<Id>,
    pub pinned: bool,
    // 新增：
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<BroadcastTarget>,  // None=单角色; Some=广播
}

pub enum BroadcastTarget {
    All,                // Campaign 内所有 instance
    Group(String),      // 某身份组（CharacterDefinition.group）
}
```

⚠️ **身份组查询路径**（已核实）：`CharacterInstance` 无 `group` 字段，要通过 `instance.definition_id` → `CharacterDefinition.group` 反查。`CampaignStore` 有 `list_instances(campaign_id)` 拿全部 instance，再各查其 card definition 的 group。读 `campaign_store.rs` 确认有无按 definition 查 card 的方法（`get_card` + 遍历 definitions）。

**prompt 改动**（`app-agent/src/prompts/postprocess.rs` POSTPROCESS_SYSTEM_PROMPT）：
在知识更新部分加广播说明：
- 如果成文里有**公开宣告/世界级事件**（公告、爆炸、天气突变被所有人感知），输出 `broadcast: "all"` 的知识更新，`character_id` 填空或 `"*"`，系统分发给所有 instance。
- 如果是**对某身份组的宣告**（如"所有守卫接到命令"），输出 `broadcast: "守卫"`（group 名）。
- 普通的在场角色知识仍用 `character_id` 单角色 + 无 broadcast。

**写入分发逻辑**（`tauri-app/src/lib.rs` `persist_postprocess_outcome` :1799 附近，知识路径）：
`normalize_knowledge_update_for_postprocess`（:1904）处理 broadcast：
- `broadcast: Some(All)` → 遍历 `store.list_instances(camp_id)`，每个 instance 生成一条 `ToldByOther`（source 记广播发起方，若无发起方则 source=`Inferred` 或新增 `Broadcast` 来源——倾向复用 `ToldByOther` + source_character_id=None 表示"公开事件"）。
- `broadcast: Some(Group(g))` → 同上但只分发 group 匹配的 instance（通过 definition_id 反查 definition.group==g）。
- `broadcast: None` → 现有单角色逻辑（P3 分流后）。

## 三、方向 3：定向告知强化

### 问题

`ToldByOther` 数据模型已支持（`source_character_id` 记告知者），P3 已让它跨在场放行。但：
1. prompt 没强调"明确输出告知目标"——现在 `character_id` 就是目标，但 LLM 可能含糊。
2. 读侧注入时没带"谁告诉的"上下文——`render_knowledge_for_injection`（character_knowledge.rs:176）只输出"（被告知）"，没输出告知者。

### 改法

**prompt 强化**（postprocess.rs）：
- 明确：`told_by_other` 时 `character_id` 是**被告知者**，`source_character_id` 是**告知者**。举例："A 写信告诉 B 秘密" → character_id=B, source=A, source=told_by_other。
- 强调：告知可以面向不在场的角色（写信/传话/留信息），系统会放行。

**读侧强化**（`render_knowledge_for_injection` :176）：
`ToldByOther` 渲染时带上告知者：
```
- X 告诉我：地下室有尸体（被告知，来源：X）
```
当前只输出"（被告知）"。改成输出 source_character 的名字（需传入 instance 名字映射，或在 entry 里存名字而非 id——但 entry 存的是 id，渲染时需查映射）。

⚠️ **读侧改动的复杂度**：`render_knowledge_for_injection` 是纯函数，只接收 entries，不知道 source_character_id 对应的名字。要加一个 `name_resolver: impl Fn(&Id) -> Option<String>` 参数，或在调用点预解析。读 `build_campaign_subagent_volatile`（注入点）确认怎么传。

## 四、执行顺序

1. **数据模型**（domain）：`CharacterKnowledgeUpdate` 加 `broadcast` 字段 + `BroadcastTarget` 枚举。`#[serde(default)]` 保证向后兼容。
2. **写入分发**（lib.rs `normalize_knowledge_update_for_postprocess`）：处理 broadcast 分发逻辑。
3. **prompt**（postprocess.rs）：加广播 + 定向告知说明。
4. **读侧**（`render_knowledge_for_injection`）：ToldByOther 带告知者名字。
5. **测试**：
   - 确定性：broadcast All 分发到所有 instance；broadcast Group 分发到匹配组；单角色不受影响。
   - `cargo test --workspace` 0 回归（注意 broadcast 字段 serde default，既有测试不传 broadcast 仍跑通）。
6. **真实 LLM 验证**（可选，需 API key）：跑一轮含公告场景的写作，看 postprocess 是否输出 broadcast。

## 五、关键约束

- **不重写 `CharacterKnowledgeEntry` 核心结构**（`character_id` 绑定保留——广播是写入时分发成多条 entry，每条仍归属单角色）。
- **不绕过 postprocess 写知识**（postprocess 是唯一写入入口，广播分发在 `normalize_knowledge_update_for_postprocess` 内做）。
- **broadcast 字段 `#[serde(default)]`**——既有 `CharacterKnowledgeUpdate` 序列化/测试不破坏。
- **不碰 W5 的前端文件**（W6 纯后端）。
- **不 commit**（留给用户审）。
- **不改 P3 的分流逻辑**（broadcast 是在 P3 分流之上的分发层；broadcast 的每条分发仍走 P3 分流——`ToldByOther` 不查在场，合理）。

## 六、给 Claude Code 的提示词

```
请阅读 docs/PLAN-KNOWLEDGE-PROPAGATION.md（方向 1+3）和 docs/HANDOFF-W6-KNOWLEDGE.md（本文件），
然后实现知识传播引擎方向 1（显式广播）+ 方向 3（定向告知强化）。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w6-knowledge
分支：w6-knowledge

只做方向 1+3，不做方向 2/4/5。

先读这些理解现状：
- crates/domain/src/character_knowledge.rs 的 CharacterKnowledgeUpdate(:141) +
  KnowledgeSource(:17) + render_knowledge_for_injection(:176)
- crates/tauri-app/src/lib.rs 的 normalize_knowledge_update_for_postprocess(:1904) +
  persist_postprocess_outcome(:1799) [P3 分流已合并]
- crates/app-agent/src/prompts/postprocess.rs 的 POSTPROCESS_SYSTEM_PROMPT
- crates/domain/src/campaign.rs CharacterInstance(:110) [无 group 字段，通过 definition_id 反查]
- crates/domain/src/character.rs CharacterDefinition(:271) group 字段(:285)

方向 1（显式广播）:
- CharacterKnowledgeUpdate 加 broadcast: Option<BroadcastTarget> (#[serde(default)])。
  BroadcastTarget 枚举: All / Group(String)。
- normalize_knowledge_update_for_postprocess 处理分发:
  All→遍历 list_instances 各写一条 ToldByOther;
  Group(g)→只分发 definition.group==g 的 instance（通过 definition_id 反查 definition）。
- POSTPROCESS_SYSTEM_PROMPT 加广播说明: 公告/世界事件输出 broadcast:"all"。

方向 3（定向告知强化）:
- POSTPROCESS_SYSTEM_PROMPT 强调 told_by_other 时 character_id=被告知者,
  source_character_id=告知者; 告知可面向不在场角色。
- render_knowledge_for_injection 的 ToldByOther 渲染带告知者名字
  (加 name_resolver 参数或调用点预解析)。

验收: cargo test --workspace 0 回归(broadcast serde default 保证兼容);
新增确定性测试覆盖 broadcast All/Group 分发 + 单角色不受影响。

红线: 不重写 CharacterKnowledgeEntry 核心结构 / 不绕过 postprocess 写知识 /
broadcast 字段 serde default / 不碰前端 / 不改 P3 分流逻辑 / 不 commit。

先做数据模型(domain),再写入分发(lib.rs),再 prompt,最后读侧,每步跑 cargo check。
```

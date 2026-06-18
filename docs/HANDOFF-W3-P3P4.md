# W3 执行手册：P3 按知识来源分流 + P4 同名收紧

> 交接对象：Claude Code（在 worktree `storyforge-w3-p3p4` 分支 `w3-p3p4` 工作）
> 前置必读：`docs/HARNESS-FINDINGS-2026-06-18.md` §P3 §P4 §F3、`docs/PLAN-KNOWLEDGE-PROPAGATION.md`
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w3-p3p4`
> 分支：`w3-p3p4`（已基于 `main` c0456aa）

## 一、任务概述

两个独立但同处一个文件簇的修复：

- **P3**：postprocess 知识写回门禁按 `KnowledgeSource` 分流（修真 bug：空集时 Witnessed/Inferred 该拒却全放行）。
- **P4**：同名 instance 时 name 匹配路失效，逼 id（修同名歧义写串）。

两处都改 `crates/tauri-app/src/lib.rs` 的 postprocess 区段 + `crates/harness-real-llm/tests/writeback_isolation.rs` 测试。**不碰其他文件**。

## 二、P3：按知识来源分流

### 背景（必读）

`is_postprocess_instance_present`（`lib.rs:1947`）对所有知识写入套"在场"约束。但 `KnowledgeSource`（`domain/src/character_knowledge.rs:17`）四类来源传播规则不同：

| 来源 | 该受"在场"约束 | 理由 |
|---|---|---|
| `Witnessed` | 该 | 不在场不可能亲眼见 |
| `Inferred` | 该 | 推断基于自己已知 |
| `ToldByOther` | **不该** | 告知本就跨在场（写信/密语） |
| `Backstory` | **不该** | 开局就有 |

现状空集逃生口（`present_ids.is_empty() → 全放行`）是这套错误判据的症状补丁。详见 findings §P3 和 PLAN-KNOWLEDGE-PROPAGATION.md 场景 A/C。

### 改动点（已核实调用链）

**核心**：`is_postprocess_instance_present`（lib.rs:1947）和 `normalize_knowledge_update_for_postprocess`（lib.rs:1903）当前**不接收 source**。P3 要让门禁按来源分流，必须把 `source` 传进来。

**调用点 1 — 知识路径**（lib.rs:1799-1824，`persist_postprocess_outcome` 内）：
- 这里 `u: &CharacterKnowledgeUpdate` 已含 `u.source`（KnowledgeSource）。
- 当前 :1803-1822 的 if/else 两分支调用完全相同（空集与否都一样调 normalize），是无意义结构。分流后这里要真正区分。
- **改法**：把 `u.source` 传给 `normalize_knowledge_update_for_postprocess`，由它在内部调门禁时按来源分流。

**调用点 2 — 变量路径**（lib.rs:1830-1854）：**不动**。变量没有 source 概念，变量该不该写只看在场（不在场角色的变量不该被改）。`is_postprocess_instance_present` 对变量路径的语义不变。这是 P3 分流**只作用于知识路径**的关键边界——别误改变量路径。

### 门禁新逻辑（P3 分流）

`normalize_knowledge_update_for_postprocess`（lib.rs:1903）调 `is_postprocess_instance_present`（:1918）前，按 source 决定是否走门禁：

```rust
// 伪代码：在 normalize_knowledge_update_for_postprocess 内
let knowledge_exempt_from_presence = matches!(
    update.source,
    KnowledgeSource::ToldByOther | KnowledgeSource::Backstory
);
let is_present = is_postprocess_instance_present(&target, &update.character_id, present_ids);
if !knowledge_exempt_from_presence && !is_present {
    return None;  // Witnessed/Inferred 且不在场 → 拒绝
}
// ToldByOther/Backstory → 放行（不查在场）
// Witnessed/Inferred 且在场 → 放行
```

`is_postprocess_instance_present` 本身（lib.rs:1947）的空集分支怎么改？**两种等价做法，选其一**：

- **做法 A（推荐，改动小）**：`is_postprocess_instance_present` 不动空集分支（仍 `is_empty → true`），把"按来源拒"的逻辑放在 `normalize_knowledge_update_for_postprocess` 调用它**之前**。即：normalize 先判 source，ToldByOther/Backstory 直接放行不调门禁；Witnessed/Inferred 才调门禁，门禁空集放行也无妨——因为空集放行的是"无人在场"，而 Witnessed/Inferred 在空集时本就该被 normalize 的 source 判断拦掉。

  ⚠️ 注意：做法 A 下，空集 + Witnessed 不能只靠门禁的 `is_empty→true` 放行——必须在 normalize 层用 source 拦。**门禁的空集分支对知识路径不再有效**，对变量路径仍有效（变量无 source）。这是合理的：门禁保留空集放行服务变量路径，知识路径靠 source 分流。

- **做法 B**：给 `is_postprocess_instance_present` 加 `source` 参数，门禁内部分流。改动签名，影响所有调用点（含变量路径 :1835）——变量路径传一个"总是受约束"的哨兵 source。改动大，不推荐。

**选做法 A**：门禁签名不变，分流逻辑在 `normalize_knowledge_update_for_postprocess`。门禁空集分支保留（服务变量路径 + 兼容现有测试）。

### 空集 if/else 清理

lib.rs:1803-1822 当前两分支相同的 if/else，分流后可简化：直接调 normalize（normalize 内部按 source 分流），删掉外层无意义的 if present_ids.is_empty()。但保留 warn 日志——把"空集放行"的 warn 移到 normalize 内部按 source 决定是否打。

### 验收

1. **新增/改测试**（`writeback_isolation.rs`）：
   - `b3_empty_present_chars_escape_hatch_current_behavior`：当前断言"空集放行全部"。分流后**重写**：空集 + `ToldByOther` 放行（合理），空集 + `Witnessed` **拒绝**（真 bug 修复）。把单个测试拆成两个，分别覆盖两类来源。
   - `b3_empty_present_chars_should_reject_when_tightened`（`#[ignore]`）：删除或改写。分流落地后"拒绝"已成立，不再是占位。改成 `b3_empty_witnessed_is_rejected`（非 ignored，绿）。
   - 新增 `b3_told_by_other_bypasses_presence`：非空 present_ids（不含目标）+ `ToldByOther` → 放行（跨在场告知成立）。
   - 新增 `b3_backstory_bypasses_presence`：同上 + `Backstory` → 放行。
   - 新增 `b3_witnessed_respects_presence`：非空 present_ids（不含目标）+ `Witnessed` → 拒绝。
2. `cargo test -p harness-real-llm --test writeback_isolation` 全绿。
3. `cargo test --workspace` 0 回归。
4. findings §P3 已由 W1 更新（重定位 + 分流方案），W3 不需再改 findings，但 commit message 要说明"按 findings §P3 分流方案落地"。

## 三、P4：同名收紧

### 背景

`is_postprocess_instance_present`（lib.rs:1964）name 兜底路：`present_ids.contains(&inst.name)`。两个同名 instance 时，present 含该 name 两者都过 → 写串。详见 findings §P4。

### 改法（方案 A）

保留 name 兜底（不让 postprocess 瘫痪），但**检测到 Campaign 内存在同名 instance 时 name 路失效**。问题：`is_postprocess_instance_present` 只接收单个 `inst`，不知道 campaign 内有没有同名兄弟。

**两种实现**：

- **实现 1（推荐）**：`persist_postprocess_outcome` 在循环外预先算一次"本 campaign 的同名集合"——`name_collisions: HashSet<String>`，包含所有出现 ≥2 次的 name。传给门禁：门禁 name 路改为 `present_ids.contains(&inst.name) && !name_collisions.contains(&inst.name)`。即同名时 name 路不命中，逼 id。
- **实现 2**：门禁签名加 `has_name_sibling: bool`，调用点查。等价，但每个调用点都要查，啰嗦。

**选实现 1**：在 `persist_postprocess_outcome` 算 `name_collisions` 一次，传给门禁。门禁签名加 `name_collisions: &HashSet<String>`。

⚠️ **变量路径也调门禁**（lib.rs:1835）——P4 同名收紧对变量同样适用（同名角色变量也该按 id 写），所以变量路径也要传 `name_collisions`。这与 P3 不同（P3 只改知识路径）：**P4 改门禁签名，影响所有调用点**。

### 验收

1. **改测试**（`writeback_isolation.rs`）：
   - `b4_name_collision_both_pass`：当前断言"同名两者都过"。**重写为拒绝**：同名时 name 路失效，只有 id 在 present 的那个过，另一个被拒。
   - `b4_present_chars_name_id_matching`：保持绿（非同名场景 name 路仍有效）。
   - 新增 `b4_name_collision_id_path_still_works`：同名场景，present 含 inst_a 的 id → inst_a 过、inst_b 拒。
2. `cargo test -p harness-real-llm --test writeback_isolation` 全绿。
3. `cargo test --workspace` 0 回归（注意 lib.rs 内 4889/4931/4974/5019/5560 等既有测试也调门禁，签名变了要同步改——它们传空 `name_collisions` 即可，行为不变）。

## 四、执行顺序

1. P3 先做（知识路径分流，不动门禁签名，风险小）→ 跑 writeback_isolation 测试。
2. P4 后做（改门禁签名 + 所有调用点）→ 跑 writeback_isolation + workspace。
3. 一起 commit（或分两个 commit：`fix(w3): P3 按来源分流` + `fix(w3): P4 同名收紧`）。

## 五、红线

- **不动变量路径的"在场"语义**（P3 只改知识路径；变量路径的 is_postprocess_instance_present 调用不变其约束逻辑）。
- **不删 name 兜底路**（P4 方案 A 是"同名时失效"非"全删"；全删会瘫痪 postprocess 按名字输出的契约）。
- **不碰 lib.rs 的 MVU/campaign/regenerate 区段**（本任务只动 postprocess 区段 lib.rs:1770-1972 附近）。
- **不改 findings**（W1 已更新；W3 只改代码 + 测试）。
- **不 commit**（用户没要求；改完留给用户审）。
- **不引入新数据模型**（P3 分流只用现有 KnowledgeSource；身份组/广播是 PLAN-KNOWLEDGE-PROPAGATION 的事，不在 W3）。

## 六、给 Claude Code 的提示词

```
请阅读 docs/HANDOFF-W3-P3P4.md（本文件），然后执行 P3 + P4 两个修复。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w3-p3p4
分支：w3-p3p4

先读这些文件理解现状：
- crates/tauri-app/src/lib.rs 的 persist_postprocess_outcome（:1770）、
  normalize_knowledge_update_for_postprocess（:1903）、is_postprocess_instance_present（:1947）
- crates/domain/src/character_knowledge.rs 的 KnowledgeSource 枚举（:17）
- crates/harness-real-llm/tests/writeback_isolation.rs 的 b3/b4 测试
- docs/HARNESS-FINDINGS-2026-06-18.md §P3 §P4

P3（先做）：知识写回门禁按 KnowledgeSource 分流。
- 做法 A：门禁 is_postprocess_instance_present 签名不变，分流逻辑放在
  normalize_knowledge_update_for_postprocess 内——ToldByOther/Backstory 不查在场直接放行，
  Witnessed/Inferred 才查在场。
- 关键边界：变量路径（lib.rs:1830-1854）不动——变量无 source 概念，只看在场。
- 清理 lib.rs:1803-1822 无意义的空集 if/else（两分支相同）。
- 重写 b3 测试：空集+Witnessed 拒、空集+ToldByOther 放行；新增 told_by_other/backstory/witnessed
  三类非空场景测试。

P4（后做）：同名收紧。
- 实现方式：persist_postprocess_outcome 算 name_collisions（出现≥2次的 name 集合），
  传给 is_postprocess_instance_present（签名加 name_collisions: &HashSet<String>）。
  门禁 name 路：present_ids.contains(&inst.name) && !name_collisions.contains(&inst.name)。
- P4 改门禁签名，影响所有调用点（变量路径 :1835 + 既有测试 4889/4931/4974/5019/5560），
  既有调用传空 HashSet 即可，行为不变。
- 重写 b4_name_collision_both_pass 为"同名时只 id 命中者过"。

验收：cargo test -p harness-real-llm --test writeback_isolation 全绿 +
cargo test --workspace 0 回归。

红线：不动变量路径在场语义 / 不删 name 兜底（只同名失效）/ 不碰 MVU 区段 /
不改 findings / 不 commit。
```

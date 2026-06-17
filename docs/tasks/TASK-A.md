# Task A：类型化 Patch DTO + 纯函数 diff/preview/apply

> 你的 worktree：`C:\tmp\sf-taskA`，分支：`codex/meta-typed-patch-dto`
> 起始 commit：main `06e53fe`。
> 完整契约见同目录 `ROUND-3-README.md`（**必读**，类型签名锁死在那）。

## 你负责的文件（只动这些）

- **新建** `crates/app-meta/src/typed_patch.rs`
- **修改** `crates/app-meta/src/lib.rs` —— **仅追加**：
  - `pub mod typed_patch;`
  - `pub use typed_patch::{TypedPatch, TypedPatchAction, TypedPatchStatus, FieldDiff, PreviewInput, TypedPatchError, build_patch_for_issue, is_patch_stale, apply_to_snapshot};`

**禁止改动** lib.rs 里的任何现有类型（`Patch`/`PatchAction`/`PatchStore`/`execute_patch` 等保持原样 —— 那些是 config-meta 的松散 patch，本任务不动它，不重命名，避免破坏 `meta_accept_patch`）。

## 实现要求

### 1. 四个 `TypedPatchAction` 变体的 build（`build_patch_for_issue`）

对每个 `HealthIssue.category`：

- **`orphan_instance`**（Error）→ `RepointInstanceDefinition`：
  - affected_id = instance.id。找到该 instance，把 `definition_id` 改成 `None`（降级为临时角色语义）。`new_definition_id = None`。
  - 理由：我们无法自动猜对正确 definition；清空是最安全的修复，用户可在前端再手动指定。description 写明「将孤立实例 {name} 的 definition_id 清空（降为临时角色）」。
  - diff：`definition_id` before = 原 id / after = null。
- **`unresolved_knowledge`**（Warning）→ `DeleteOrphanKnowledge`：
  - affected_id = knowledge.id。从快照里找到该 entry，记录其 `knowledge_text`（截断 40 字）写进 description。
  - diff：整条 entry before = 序列化值 / after = null（表示删除）。
- **`orphan_task_reference`**（Warning）→ `PruneOrphanTaskReferences`：
  - affected_id = task.id。找到 task，列出 `related_characters` 中不在 instance 集合里的 id 作为 `orphan_character_ids`。
  - diff：`related_characters` before = 原 vec / after = 过滤后 vec。
- **`variable_schema_mismatch`**（Warning）→ `SyncInstanceVariables`：
  - affected_id = instance.id。比较 instance.variables 的 key 集 vs definition.variable_schema 的 key 集。
  - `add_keys` = schema 有但 instance 缺的（值取 schema default —— 但 diff 里只记 key 名，值在 apply 时由 `init_values_from_schema` 补）。
  - `remove_keys` = instance 有但 schema 没的。
  - diff：每个 `add_keys` 项一条 `FieldDiff{path: "variables[<key>]", before: null, after: <default>}`；每个 `remove_keys` 项一条 `before: <旧值>, after: null`。

**未列出的 category**（如未来新增）→ 返回 `None`（`build_patch_for_issue` 返回 Option）。

### 2. diff 必须在构造时算好

`TypedPatch.diff` 在 `build_patch_for_issue` 里**当场填好**，不留给调用方算。diff 的 `before` 必须是**当前快照里的真实值**（从 PreviewInput 读），`after` 是修复后的预期值。这样前端拿到的 patch 自带 diff，无需二次查询。

### 3. `is_patch_stale`

检查 patch 每个 action 的 target id 是否仍存在于 PreviewInput：
- `RepointInstanceDefinition` → instance_id 仍在 instances 里？
- `DeleteOrphanKnowledge` → knowledge_id 仍在 knowledge 里？
- `PruneOrphanTaskReferences` → task_id 仍在 tasks 里？
- `SyncInstanceVariables` → instance_id 仍在？

任一 target 消失 → 返回 `true`（过期）。

### 4. `apply_to_snapshot`（纯函数镜像）

把 actions 应用到可变快照副本：
- `SyncInstanceVariables`：用 `storyforge_domain::variables::init_values_from_schema` 取 schema 的 default 给 add_keys 补值；remove_keys 从 instance.variables 移除。**保持其他变量不动。**
- `PruneOrphanTaskReferences`：从 task.related_characters 移除 orphan_character_ids。
- `DeleteOrphanKnowledge`：从 knowledge vec 移除该 id。
- `RepointInstanceDefinition`：instance.definition_id = new_definition_id。

target 不存在 → `Err(TypedPatchError::TargetMissing(...))`。

这个函数是**纯函数测试**用 —— C 的 tauri 命令会在 accept 前调它做校验，但**真正写盘用 CampaignStore.update_***（C 的职责，不是你的）。你只需保证这个纯函数对一份克隆快照语义正确。

### 5. id 生成

`TypedPatch.id` 用 `uuid::Uuid::new_v4().to_string()`（crate 里已用 uuid，依赖已存在）。

### 6. created_at

用 `chrono::Utc::now()`。

## 约束（来自 PLAN-META-AGENT.md「禁止改动」）

- **不要把 store lock 暴露给 app-meta。** PreviewInput 是纯数据切片，不含任何 store 引用。
- **不要让 patch 自动写数据。** apply_to_snapshot 只改传入的克隆副本，不碰任何全局状态。
- **不要绕过 Campaign ID 校验。** 本任务不涉及 campaign_id（diff 数据已限定在单 campaign 快照内），但如果 PreviewInput 的 instances/knowledge/tasks 跨了 campaign，那是 C 组装快照时的 bug，不是你的；你只对传入数据做正确处理。

## 验证（你的 worktree 内运行）

```cmd
cargo test -p storyforge-app-meta
cargo build -p storyforge-app-meta
```

## 测试要求（至少 10 个，纯函数测试，零外部依赖）

- 每个 category 的 build 各一个 happy path（断言 diff 内容正确）。
- `build_patch_for_issue` 对未知 category 返回 None。
- `is_patch_stale`：target 存在 = false；target 被移除 = true。
- `apply_to_snapshot`：
  - SyncInstanceVariables：补字段 + 删字段，其他变量不变。
  - PruneOrphanTaskReferences：只删 orphan，保留有效引用。
  - DeleteOrphanKnowledge：删指定，保留其他。
  - RepointInstanceDefinition：definition_id 改动生效。
  - apply 时 target 缺失 → 返回 TargetMissing 错误。

构造测试数据用 `storyforge_domain` 的现有构造器（`CharacterInstance::from_definition` / `CharacterInstance::temporary` / `StoryTask::user_planned` / `CharacterKnowledgeEntry::witnessed` 等，参考 `health_check.rs` 的测试 helpers）。

## 完成后报告

在你的 worktree commit 后，报告：
1. 改动文件列表 + 行数。
2. 新增测试数量 + `cargo test -p storyforge-app-meta` 结果（pass/fail 数）。
3. 你偏离本 spec 的地方（如有），以及原因。
4. 是否动了 spec 禁止的文件。

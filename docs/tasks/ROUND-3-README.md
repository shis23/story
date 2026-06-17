# 第三轮并行开发：Meta 修复闭环 + 生成溯源前端接入

> 创建于 2026-06-17。三个并行任务，文件不重叠，最后由审查 agent 合并。
> 主分支当前 HEAD：`06e53fe`。每条任务从 main 拉独立 worktree + 分支。

## 背景：本轮解决什么

PLAN-META-AGENT.md 阶段 3「Typed Patch Preview」是 A 档（核心可用）剩余的关键缺口。
当前事实（审查后确认）：

1. **health check 已就绪**（`app-meta/src/health_check.rs`，4 类检查 + 12 测试），`meta_health_check` Tauri command 可用。
2. **`meta_explain_generation` Tauri command 已就绪**（X 任务产物，已合入 main），但**前端从不调用它** → 阶段 2「解释本轮生成」的前端验收项未完成。
3. **现有 `Patch`/`PatchAction`**（`app-meta/src/lib.rs:75-101`）是**松散 JSON + 字符串 target**，只覆盖 config-meta（world_info / character card 字段），**完全不覆盖 campaign-runtime 修复**（变量 / 知识 / 任务 / instance）。health check 发现的问题没有任何类型化修复路径 → 「诊断 → 修复」环未闭合。

本轮三个任务各自闭合其中一块：

| 任务 | 目标 | 主改文件（互不重叠）|
|------|------|--------------------|
| **A** | 类型化 Patch DTO + 纯函数 diff/preview/构造器 | `crates/app-meta/src/typed_patch.rs`（新）+ `crates/app-meta/src/lib.rs`（重导出，**仅追加 re-export 行**）|
| **C** | Tauri 命令：propose/preview/accept/dismiss 类型化 patch + CampaignStore 回写 + 注册到 invoke_handler | `crates/tauri-app/src/lib.rs`（新增命令）|
| **D** | 前端：MetaPanel 修复建议 UI + 生成溯源调用（接入已存在的 `meta_explain_generation`） | `frontend/src/components/MetaPanel.vue` + `frontend/src/tauri-api.js`（追加导出）|

### 依赖与合并顺序

- **A 是 C 的前置**（C 用 A 的 DTO），但 C 只通过 `storyforge_app_meta::typed_patch::...` re-export **只读引用** DTO（不在同一文件里写），所以 A/C/D 可真并行 —— DTO 形状在本 README 已完全锁死，C 按 README 指定的类型签名开发即可。
- 合并顺序：**A 先合 → C 再合**（C 引用 A 的 re-export，A 必须先在 main）。D 与 A/C 不冲突，最后合。
- 前端 api 包装：D 需要在 `tauri-api.js` 追加若干 `invoke` 包装；若 D 先于 C 合，`invoke` 会指向尚不存在的命令 —— 这是**预期的**，前端在命令缺失时 try/catch 给出错误提示即可，不阻塞构建。

## 类型化 Patch 契约（A/C/D 三方必须严格遵守，避免集成不一致）

文件：`crates/app-meta/src/typed_patch.rs`（A 新建）

```rust
use serde::{Deserialize, Serialize};
use storyforge_domain::Id;

/// 类型化修复操作（每个变体 = 一种 health issue 的修复）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypedPatchAction {
    /// 修变量 schema 不一致：为 instance 补齐缺失字段 / 删除多余字段
    SyncInstanceVariables {
        instance_id: Id,
        definition_id: Id,
        add_keys: Vec<String>,
        remove_keys: Vec<String>,
    },
    /// 修孤儿任务引用：从 task.related_characters 移除不存在的 id
    PruneOrphanTaskReferences {
        task_id: Id,
        orphan_character_ids: Vec<Id>,
    },
    /// 修未解析知识引用：删除指向不存在 instance 的知识条目
    DeleteOrphanKnowledge {
        knowledge_id: Id,
    },
    /// 修孤立 instance：把 definition_id 改成现存 definition（或清空为临时角色）
    RepointInstanceDefinition {
        instance_id: Id,
        new_definition_id: Option<Id>,
    },
}

/// 单个字段变更（diff 用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDiff {
    pub path: String,        // 如 "variables[hp]" / "related_characters" / "definition_id"
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}

/// 一条修复建议（可含多个 action）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedPatch {
    pub id: String,
    pub description: String,
    pub source_issue_category: String,   // 对应 HealthIssue.category
    pub affected_id: Option<String>,     // 对应 HealthIssue.affected_id
    pub actions: Vec<TypedPatchAction>,
    pub diff: Vec<FieldDiff>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub status: TypedPatchStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TypedPatchStatus { Pending, Accepted, Dismissed, Stale }

/// preview 的输入快照（A 提供 preview 函数；C 在 tauri 层组装快照后调用）
/// —— A 的 preview 是纯函数，不读 store；C 负责把 store 数据搬进这个快照。
pub struct PreviewInput<'a> {
    pub instances: &'a [storyforge_domain::campaign::CharacterInstance],
    pub definitions: &'a [storyforge_domain::character::CharacterDefinition],
    pub knowledge: &'a [storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
    pub tasks: &'a [storyforge_domain::story_task::StoryTask],
}
```

**A 必须实现的纯函数（签名锁死，C/D 按此调用）：**

```rust
/// 从一个 health issue 构造对应的修复 patch（preview 在构造时一并算好 diff）。
/// 无法修复的 issue（如未实现的 category）返回 None。
pub fn build_patch_for_issue(
    issue: &crate::HealthIssue,
    input: &PreviewInput,
) -> Option<TypedPatch>;

/// 检查 patch 是否过期：target id 是否仍存在于快照中。
/// 返回 true = 过期（不应接受）。
pub fn is_patch_stale(patch: &TypedPatch, input: &PreviewInput) -> bool;

/// 把 patch 的 actions 应用到一份可变快照副本上（纯函数，验证语义正确用）。
/// 这是 accept 路径的「纯函数镜像」，C 的 tauri 命令用它做 accept 前的最终校验，
/// 真正写盘仍由 C 用 CampaignStore 的 update_* 方法完成。
pub fn apply_to_snapshot(
    patch: &TypedPatch,
    snapshot: &mut PreviewInputMut,  // 见下
) -> Result<(), TypedPatchError>;

#[derive(Debug, thiserror::Error)]
pub enum TypedPatchError {
    #[error("patch target 不存在: {0}")]
    TargetMissing(String),
    #[error("patch 已过期")]
    Stale,
}
```

> 注：`PreviewInputMut` 与 `PreviewInput` 字段相同但为 `&mut`，A 自行定义（或用单一可变结构 + 不可变借用重载，A 自选实现，但字段名锁定为 instances/definitions/knowledge/tasks）。

## 各任务详细范围

见同目录 `TASK-A.md` / `TASK-C.md` / `TASK-D.md`。

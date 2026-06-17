# Task C：Tauri 命令 —— 类型化 Patch 的 propose/preview/accept/dismiss + CampaignStore 回写

> 你的 worktree：`C:\tmp\sf-taskC`，分支：`codex/meta-typed-patch-cmd`
> 起始 commit：main `06e53fe`。
> 完整契约见同目录 `ROUND-3-README.md`（**必读**，DTO 类型签名锁死在那）。
>
> **前置依赖**：A 任务提供 `storyforge_app_meta::typed_patch::*` 的 re-export。A 与你并行开发，但 DTO 形状已在 README 锁死，你**按 README 的类型签名开发**即可，无需等 A 完成。合并时 A 先合，你后合。

## 你负责的文件（只动这个）

- `crates/tauri-app/src/lib.rs`

**禁止改动**：
- `crates/app-meta/**`（A 的领地）
- `frontend/**`（D 的领地）
- `crates/domain/**`

## 现状（你需要了解的事实）

1. `meta_health_check(campaign_id)` 已存在（lib.rs:3084），返回 `Vec<serde_json::Value>`（每条是 HealthIssue 的序列化）。
2. `get_campaign_store()`（lib.rs:62）返回 `&'static CampaignStore`，它有：
   - `list_instances(&Id)` / `get_instance(&Id, &Id)` / `update_instance(CharacterInstance)` / `add_instance(CharacterInstance)`
   - `list_knowledge(&Id)` / `add_knowledge(Vec<CharacterKnowledgeEntry>)`
   - `list_tasks(&Id)` / `get_task(&Id)` / `update_task(StoryTask)` / `delete_task(&Id)`
   - `get_campaign(&Id)` / `get_card(&Id)`（card 含 `character_definitions`）
3. 现有的 `meta_accept_patch`（lib.rs:2825）是**旧的 config-meta 松散 patch**，**不要动它**。你的新命令用不同名字（见下）。
4. AppState（搜 `struct AppState`）已有 `meta_patches: RwLock<Vec<Patch>>`（旧的）。**新增**一个字段 `typed_patches: std::sync::RwLock<Vec<storyforge_app_meta::TypedPatch>>`，与旧的并存，不替换。
5. invoke_handler 注册区（搜 `tauri::generate_handler!` 或 `invoke_handler`）：把新命令加进去。

## 要实现的 Tauri 命令（5 个，名字锁死，前端 D 按此调用）

### 1. `meta_propose_campaign_repairs`

```rust
#[tauri::command]
fn meta_propose_campaign_repairs(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, String>
```

逻辑：
1. 解析 campaign_id，从 CampaignStore 取 instances / definitions（经 card）/ knowledge / tasks。
2. 组装 `PreviewInput { instances: &..., definitions: &..., knowledge: &..., tasks: &... }`。
3. 调 `meta_health_check` 的逻辑（直接调 `check_campaign_health` 或复用）拿 issues。
4. 对每个 issue 调 `storyforge_app_meta::typed_patch::build_patch_for_issue(&issue, &input)`，收集 `Some(patch)`。
5. 把生成的 patches 存进 `state.typed_patches`（追加，不去重 —— 重复提议由前端 dismiss 处理）。
6. 返回 patches 的 serde_json::Value 数组。

### 2. `meta_list_typed_patches`

```rust
#[tauri::command]
fn meta_list_typed_patches(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<serde_json::Value>
```

返回 `state.typed_patches` 里所有 `status == Pending` 的 patch（序列化）。

### 3. `meta_preview_typed_patch`

```rust
#[tauri::command]
fn meta_preview_typed_patch(
    patch_id: String,
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String>
```

逻辑：
1. 找到该 patch。不存在 → Err。
2. 取当前 campaign 快照，组装 PreviewInput。
3. 调 `is_patch_stale(patch, &input)`：若 stale → 把该 patch status 改成 `Stale`，返回 `{ "stale": true, "patch": <patch> }`。
4. 否则返回 `{ "stale": false, "patch": <patch>, "diff": <patch.diff> }`（diff 已在 patch 里，直接透传）。

### 4. `meta_accept_typed_patch`（**核心：真正写盘**）

```rust
#[tauri::command]
fn meta_accept_typed_patch(
    patch_id: String,
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String>
```

逻辑（**严格按序**，保证安全）：
1. 找到 patch，必须 `status == Pending`。否则 Err。
2. 取当前 campaign 快照，组装 PreviewInput。
3. **stale 检查**：`is_patch_stale(patch, &input)` → 若 stale，把 status 改 Stale，Err("patch 已过期，target 不存在")。
4. **纯函数预演**：克隆快照为可变副本，调 `apply_to_snapshot(patch, &mut cloned)`。若返回 Err，**不写盘**，直接返回错误。这步保证写盘前语义已验证。
5. **真正写盘**：遍历 patch.actions，按类型调 CampaignStore：
   - `SyncInstanceVariables { instance_id, add_keys, remove_keys, .. }`：`get_instance` 取出，修改 variables（add_keys 用 `init_values_from_schema` 取 default；remove_keys 删除），`update_instance` 写回。**只改指定 key，其他变量原样保留。**
   - `PruneOrphanTaskReferences { task_id, orphan_character_ids }`：`get_task` 取出，从 related_characters 移除这些 id，`update_task` 写回。
   - `DeleteOrphanKnowledge { knowledge_id }`：CampaignStore **没有**单条删除知识的方法（只有 add_knowledge 批量）。处理方式：`list_knowledge(campaign_id)` 取全量，过滤掉该 id，**重写整份**。但 CampaignStore 也没有 replace_all_knowledge —— **你需要给 CampaignStore 加一个方法** `delete_knowledge(&self, knowledge_id: &Id) -> bool`（仿照已有 `delete_task`，在 `campaign_store.rs`... 

   **等等 —— `campaign_store.rs` 不在你的允许文件列表里。** 重新审视：campaign_store.rs 属于 tauri-app crate，且本任务文件范围写的是「只动 lib.rs」。为避免冲突，**改用 lib.rs 内的私有 helper**：在 lib.rs 里写一个私有函数，借 `get_campaign_store()` 拿到的 cache，但 cache 是 private…… 

   **结论：你必须也改 `campaign_store.rs`**（加一个 `delete_knowledge` 方法，仿 `delete_task`）。这是本轮唯一对 campaign_store.rs 的改动，与 A/D 不冲突（他们不碰这个文件）。**请在本任务允许文件里追加 `crates/tauri-app/src/campaign_store.rs`（仅限新增 `delete_knowledge` 方法）。** 加方法时附一个单测（仿 `test_task_crud` 的 delete 部分）。
   - `RepointInstanceDefinition { instance_id, new_definition_id }`：`get_instance` 取出，改 definition_id，`update_instance` 写回。
6. 写盘成功后，把 patch status 改成 `Accepted`。

### 5. `meta_dismiss_typed_patch`

```rust
#[tauri::command]
fn meta_dismiss_typed_patch(
    patch_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String>
```

把该 patch status 改成 `Dismissed`（不从内存删除，保留审计痕迹）。

## 注册

把这 5 个命令加进 invoke_handler 列表（与现有 `meta_health_check` / `meta_accept_patch` 等并列）。

## 约束

- **不暴露 store lock 给 app-meta。** 所有 store 访问都在 lib.rs 内完成，app-meta 只见纯数据切片。
- **accept 必须先纯函数预演再写盘**（步骤 4 在步骤 5 前），不允许跳过预演直接写。
- **accept 失败不能部分写盘**：如果某个 action 写盘失败，已经写成功的 action 不回滚（CampaignStore 是文件持久化，回滚复杂），但要把错误返回前端，并把 patch status **保持 Pending**（用户可重试，或手动修）。在错误信息里写明「第 N 个 action 失败」。
- **stale patch 不能 accept**。

## 验证（你的 worktree 内运行）

```cmd
cargo test -p storyforge --lib
cargo build -p storyforge
```

注意：`cargo build -p storyforge`（tauri-app）需要 `frontend/dist`。若你的 worktree 没有 frontend/dist（environment issue），用 `cargo check -p storyforge --lib` 替代 build，足以验证命令编译。

## 测试要求（lib.rs 内 #[cfg(test)] mod）

至少 4 个集成测试（在 lib.rs 现有 test mod 里追加，**不要新建 mod**）：
1. `meta_propose_campaign_repairs`：构造一个有 orphan knowledge 的 campaign，提议后返回的 patch 数 ≥ 1，且含 `delete_orphan_knowledge` action。
2. `meta_accept_typed_patch`：accept 一个 prune task reference patch 后，task.related_characters 不再含 orphan id（重新 list_tasks 验证）。
3. `meta_accept_typed_patch` stale 场景：accept 前手动删掉 target instance，accept 返回错误且 status 变 Stale。
4. `meta_dismiss_typed_patch`：dismiss 后 status = Dismissed。

构造测试数据可参考 lib.rs 现有 `test_*` helper（搜 `fn make_` / `fn setup_`）。

## 完成后报告

1. 改动文件列表 + 行数（含 campaign_store.rs 的 delete_knowledge）。
2. 新增测试数量 + `cargo test -p storyforge --lib` 结果。
3. 是否偏离 spec（尤其 campaign_store.rs 改动）。
4. `cargo check -p storyforge --lib` 是否通过。

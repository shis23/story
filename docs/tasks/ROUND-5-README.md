# 第五轮：Phase 3 阶段 4 收尾 — Campaign-aware Meta Tools

> 创建于 2026-06-17。单任务（用户选定），从 main 拉独立 worktree + 分支。
> 主分支当前 HEAD：`8494261`。

## 本轮解决什么（审计后确认的真缺口）

第四轮（E/F/G）合并后，对 Phase 3（Meta Agent 维护层，ROADMAP 第 58-77 行）五个阶段做了全代码库审计：

| 阶段 | 计划 | 真实状态 |
|------|------|----------|
| 阶段 1 只读 Health Check | 后端 + 前端 | ✅ `health_check.rs` + `meta_health_check` + MetaPanel 体检按钮全通，4 类 issue |
| 阶段 2 解释本轮生成 | 后端 + 前端 | ✅ `explain.rs` + `meta_explain_generation` + `inspect_generation` 工具（第四轮 G）+ 前端按钮 |
| 阶段 3 Typed Patch | 后端 + 前端 | ✅ `typed_patch.rs`（4 action + diff/preview/apply）+ 4 个 Tauri 命令闭环 + MetaPanel 提议/预览/接受/忽略 |
| 阶段 5 MVU 接入修复流 | 后端 + 前端 | ✅ `mvu_apply.rs` + `meta_preview_mvu_apply`/`meta_apply_mvu_schema` + 前端 |
| **阶段 4 Campaign-aware Meta Tools** | **7 个工具** | **❌ 缺 6 个，仅 `inspect_generation` 已做** |

**唯一真缺口**：`meta_conversation.rs::register_meta_runtime_tools` 当前只注册 4 个工具（`meta_inspect_world_info` / `meta_inspect_character` / `meta_propose_patch` / `inspect_generation`）。PLAN 阶段 4 要求的另外 6 个 Campaign-aware 工具全代码库 grep 确认不存在：`inspect_campaign` / `inspect_instance` / `inspect_variables` / `inspect_knowledge` / `inspect_tasks` / `propose_campaign_patch`。

**后果**：用户在 Meta 对话里问「这个 Campaign 有哪些实例」「Alice 的变量现在是什么」「当前有哪些待办任务」时，Agent 只能调 `inspect_character`（读扁平 `Character`），读不到 Campaign 的 instances/variables/knowledge/tasks。这正是 ROADMAP Phase 3 验收项「Meta 对 active Campaign 的回答不再只基于 `tool_ctx.characters`」要解决的。

本轮做完后，ROADMAP Phase 3（Meta Agent 维护层）正式关闭。

## 数据可行性（无阻塞）

- `CampaignStore` 已提供 `list_instances/list_knowledge/list_tasks/list_summaries`（`campaign_store.rs:249-416`）。
- `CampaignRuntimeContext` 已有 `find_instance_by_id_or_name` / `knowledge_for_instance` / `resolved_persona_for`（`campaign_runtime.rs:47-80`）。
- `tool_ctx.campaign_runtime: Option<Arc<CampaignRuntimeContext>>` 已是 ToolContext 字段（`meta_conversation.rs:557` 测试可见）。
- `state.typed_patches`（`lib.rs:3248`）是 typed patch 统一存储，4 个采纳命令都基于它。

## 任务划分（用户选定：单任务一次做完）

**单任务 H**，一个 worktree 一个分支，6 个工具 + 新变体一次做完。无并行冲突风险。

## 改动文件（全在单一 worktree，无重叠）

| 文件 | 改动 |
|------|------|
| `crates/app-meta/src/meta_conversation.rs` | `MetaSession` 加 `campaign_runtime` + `typed_patches` 字段；`MetaTurn` 加 `new_typed_patches`；注册 6 个新工具 |
| `crates/app-meta/src/typed_patch.rs` | 新增 4 个 `TypedPatchAction` 变体 + `build_patch_from_action` 纯函数 |
| `crates/app-meta/src/prompts/meta_agent.rs` | system prompt 增补 Campaign 诊断能力块 |
| `crates/tauri-app/src/lib.rs` | `sync_meta_session_from_tool_ctx` 注入 campaign_runtime；`meta_chat` 同步 new_typed_patches |
| `frontend/src/components/MetaPanel.vue` | `metaChat` 返回若有 new_typed_patches 则刷新列表 |

详细执行规格见同目录 `TASK-H.md`。

## 验证总要求

- `cargo test -p storyforge-app-meta` 全绿
- `cargo test -p storyforge --lib` 全绿
- `cargo check --workspace` 无新编译错误

## 不做（明确划界）

- **`PatchCharacterDefinition` / `MergeDuplicateInstance`**：改 definition 影响所有 instance，风险高且 PLAN-META-AGENT §禁止改动有「不直接越权改数据」红线。本轮 4 个新变体聚焦「单点变量/知识/任务状态」安全修复，definition 级改动留后续。
- **inspect 工具的 `ToolResultDisplay` 前端结构化渲染**：本轮 Agent 拿到 JSON 后用自然语言转述即可，不新增前端展示变体（避免 MetaPanel 膨胀）。
- **Phase 3 阶段 1 待实现项**（同名 instance / summary 连续性 / MVU 未合并检测）：与本轮无关，属 health check 增强，留后续。

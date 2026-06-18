# 第四轮并行开发：tool 注册中心 + MVU apply + Meta 对话接 explain

> 创建于 2026-06-17。三个并行任务，文件不重叠，最后由审查 agent 合并。
> 主分支当前 HEAD：`413ac0b`。每条任务从 main 拉独立 worktree + 分支。

## 本轮解决什么（审计后确认的真缺口，非伪任务）

设计前做了代码审计，排除了两个"看起来该做、实际已做"的伪任务：

- ❌ **CampaignPanel 拆页**：审计发现 `CampaignPanel.vue:409-415` 已有 detail 子标签（instances/knowledge/tasks/summaries），无需重做。文档过时，本轮顺手修正。
- ❌ **写作入口绑定 active campaign**：审计发现 `fill_campaign_context`（`lib.rs:1613`）已从 `state.active_campaign` 后端兜底注入 runtime 快照到 WritingContext + ToolContext，前端 `startWriting` 只传 intent+charId，campaign 由后端自动绑定。无需做。

三个**真缺口**（全代码库 grep 确认未实现）：

| 任务 | 缺口 | 主改文件（互不重叠） |
|------|------|----------------------|
| **E** | 统一 tool 注册中心：各角色硬编码 `register_*_tools` → 全局 ToolCenter + 按角色选配 + 默认列表。当前 `tool_whitelist` 只能在「该角色已注册组」内选，跨角色不可见。 | `crates/app-agent/src/tool_center.rs`（新）+ `crates/app-agent/src/lib.rs`（仅追加） |
| **F** | MVU apply patch：MVU 分析产出 `StoredMvuTranslation`，但没有把它合并进 `CharacterDefinition.variable_schema` 的可审阅 patch。`MvuTranslation.merged_variable_schema()` + `merge_schema`/`init_values_from_schema` 等纯函数都已存在，缺「提议→预览→接受→写盘」的闭环。 | `crates/app-meta/src/mvu_apply.rs`（新）+ `crates/app-meta/src/lib.rs`（追加）+ `crates/tauri-app/src/lib.rs`（命令） |
| **G** | Meta 对话接 explain：`meta_explain_generation` 命令已存在，但 Meta 多轮对话（`meta_conversation.rs`）的工具集没有 `inspect_generation` 工具，用户在 Meta 聊天里问"为什么这么写"时无法引用真实 provenance。 | `crates/app-meta/src/meta_conversation.rs` + `crates/app-meta/src/prompts/meta_agent.rs` |

## 文件重叠分析（关键，避免第三轮 C 的 stub 冲刺）

三个任务的文件范围**完全不重叠**：

- **E**：`crates/app-agent/src/tool_center.rs`（新）+ `crates/app-agent/src/lib.rs`（追加 mod）。
  - **不动** `tools.rs`（保留现有 ToolRegistry/register_*_tools 作为内部 helper）。
  - **不动** `app-meta`、`tauri-app`。
- **F**：`crates/app-meta/src/mvu_apply.rs`（新）+ `crates/app-meta/src/lib.rs`（追加 mod）+ `crates/tauri-app/src/lib.rs`（新命令）。
  - **不动** `app-agent`（E 的领地）。
  - **不动** `meta_conversation.rs`（G 的领地）。
  - 注意 F 和 G 都碰 `app-meta/src/lib.rs` 的 mod 声明——**这是唯一潜在重叠**。约定：F 追加 `pub mod mvu_apply;`，G 不碰 lib.rs（meta_conversation 已是现有 mod，G 只改其内容）。合并时 lib.rs 只会有一处冲突点（F 的追加），可控。
- **G**：`crates/app-meta/src/meta_conversation.rs` + `crates/app-meta/src/prompts/meta_agent.rs`。
  - **不动** `lib.rs`（meta_conversation 已注册，G 改其内容不碰声明）。

## 依赖与合并顺序

- **E、F、G 三者无代码依赖**（不互相 import），可真正并行。
- 合并顺序无强制要求，建议 **E → F → G**（按字母序，便于记录）。
- F 与 G 唯一交叉点 `app-meta/src/lib.rs`：F 追加 mod 行，G 不碰，合并时 F 的追加行可直接保留（G 那边 lib.rs 无改动）。

## 验证总要求（每任务独立）

每个任务在自己的 worktree 跑自己的验证命令，**不要求全 workspace 绿**（依赖其他任务未合并的代码不算失败）。但任务**自身**的 crate 测试必须全绿，且不能破坏其他 crate 编译（用 `cargo check --workspace` 确认无新编译错误）。

详见同目录 `TASK-E.md` / `TASK-F.md` / `TASK-G.md`。

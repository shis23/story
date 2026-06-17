# Task G：Meta 对话接 explain（inspect_generation 工具）

> 你的 worktree：`C:\tmp\sf-taskG`，分支：`codex/meta-chat-explain`
> 起始 commit：main `413ac0b`。
> 必读：`docs/tasks/ROUND-4-README.md`（文件重叠约定）。

## 你负责的文件（只动这两个）

- **修改** `crates/app-meta/src/meta_conversation.rs`
- **修改** `crates/app-meta/src/prompts/meta_agent.rs`

**禁止改动**：
- `crates/app-meta/src/lib.rs`（meta_conversation 已是现有 mod，G 只改其内容；lib.rs 的 mod 声明已存在，**不要加任何新 mod**——mvu_apply 是 F 的，会冲突）。
- `crates/app-agent`（E 的领地）、`crates/tauri-app/src/lib.rs`（F 的领地，加命令归 F）、`crates/domain`。
- `frontend`。

## 背景：现状

`meta_explain_generation(conversation_id, node_id)` Tauri 命令已存在（`lib.rs:3126`），返回 `GenerationExplanation`（scene_brief / subagents[] / last_hint / profile_id / seed）。但 **Meta 多轮对话**（`meta_conversation.rs` 的工具集）里**没有**调用它的工具。用户在 Meta 聊天里问"为什么这轮这样写"时，Meta Agent 无法引用真实 provenance——它只能泛泛回答。

PLAN-META-AGENT 阶段 4「Campaign-aware Meta Tools」要求 `inspect_generation` 工具。

## 现有 Meta 工具集（先读懂）

读 `meta_conversation.rs`，找到现有的 Meta runtime tools 注册逻辑（搜 `register` / `MetaRuntime` / tool 相关）。现有工具可能包括 inspect_world_info / inspect_character 等。**你要新增一个 `inspect_generation` 工具**。

## 要实现的

### 1. `inspect_generation` 工具（在 meta_conversation.rs 注册）

工具签名（发给 LLM 的 ToolSpec）：

```json
{
  "name": "inspect_generation",
  "description": "解释某条消息的生成溯源。返回该轮的场景简述、各子 Agent 的角色/任务/输出摘要、最后的编剧提示、所用 Agent Profile 和随机种子。当用户问"为什么这么写""这轮是怎么生成的"时调用。",
  "parameters": {
    "type": "object",
    "properties": {
      "conversation_id": {"type": "string", "description": "对话 ID"},
      "node_id": {"type": "string", "description": "消息节点 ID"}
    },
    "required": ["conversation_id", "node_id"]
  }
}
```

### 2. 工具执行（关键：如何拿到 conversation data）

**问题**：`meta_conversation.rs` 在 `app-meta` crate，不能依赖 `tauri-app`（那里有 conv_store）。

**解法**：`MetaSession` 已经有外部数据注入的先例（看它怎么接收 character / world_info）。你需要给 MetaSession 加一个「generation explainer」回调或数据源：

```rust
// meta_conversation.rs 里
use crate::explain::GenerationExplanation;

/// 生成溯源数据源（由 tauri-app 层注入，避免 app-meta 依赖 tauri-app）
pub trait GenerationExplainer: Send + Sync {
    fn explain(&self, conversation_id: &str, node_id: &str) -> Option<GenerationExplanation>;
}
```

`inspect_generation` 工具 handler 调 `session.explainer.explain(conversation_id, node_id)`：
- 有结果 → 返回序列化的 GenerationExplanation。
- 无结果（None）→ 返回错误信息「找不到该消息的生成溯源（可能无 provenance）」。

**MetaSession 加字段**：`explainer: Option<Arc<dyn GenerationExplainer>>`，默认 None（向后兼容）。tauri-app 层（不在你的范围）后续注入实现。

### 3. system prompt 更新（prompts/meta_agent.rs）

在 Meta Agent 的 system prompt 里说明 `inspect_generation` 工具的用途和调用时机。读现有 prompt，在工具说明段落追加：

> 当用户询问"为什么这轮这样写""这条消息是怎么生成的"时，调用 `inspect_generation`，传入对话 ID 和消息节点 ID。它会返回真实 provenance（场景简述、子 Agent 输出、Profile、种子）。**禁止编造生成过程**——只能基于 inspect_generation 返回的真实数据回答。

## 约束

- **app-meta 不依赖 tauri-app**：用 trait 注入，不在 app-meta 里访问 conv_store。
- `MetaSession` 新字段用 `Option` + `#[serde(skip)]`（如果有 serde），保证旧数据/无注入时不崩。
- 不改 lib.rs（meta_conversation 已注册，G 只改内容）。
- explainer 为 None 时，inspect_generation 工具返回明确「未配置溯源数据源」，不 panic。

## 验证（你的 worktree 内运行）

```cmd
cargo test -p storyforge-app-meta
cargo check --workspace
```

## 测试要求（≥ 5 个）

- `inspect_generation` 工具 spec 注册成功（工具集里能找到它）。
- handler：explainer = Some 且返回 Some(explanation) → 工具结果含 scene_brief。
- handler：explainer = Some 但返回 None → 工具结果含「找不到」错误信息。
- handler：explainer = None → 工具结果含「未配置」错误信息。
- MetaSession 新字段默认 None（向后兼容，旧 MetaSession 构造不崩）。
- system prompt 含 `inspect_generation` 说明（字符串断言）。

构造测试 GenerationExplainer 用 mock 实现（实现 trait 返回固定 GenerationExplanation）。

## 完成后报告

1. 改动文件列表 + 行数。
2. 新增测试数量 + `cargo test -p storyforge-app-meta` 结果。
3. `cargo check --workspace` 是否有新错误。
4. 是否动了禁止文件（确认没碰 lib.rs、app-agent、tauri-app/lib.rs、frontend）。
5. explainer 注入点设计（trait 名 + MetaSession 字段名），方便后续 tauri-app 层接（那步不在本轮）。

# Task E：统一 Tool 注册中心

> 你的 worktree：`C:\tmp\sf-taskE`，分支：`codex/tool-center`
> 起始 commit：main `413ac0b`。
> 必读：`docs/tasks/ROUND-4-README.md`（文件重叠约定）+ `docs/PLAN-TOOL-REGISTRY.md`（完整设计）。

## 你负责的文件（只动这两个）

- **新建** `crates/app-agent/src/tool_center.rs`
- **修改** `crates/app-agent/src/lib.rs` —— **仅追加** `pub mod tool_center;` 和 re-export。

**禁止改动**：
- `crates/app-agent/src/tools.rs`（保留现有 `ToolRegistry`/`ToolHandler`/`retain`/`register_*_tools` 作为内部 helper，**不要删、不要改签名**——E 只在它们之上建新层）。
- `crates/app-meta`、`crates/tauri-app`、`crates/domain`、`crates/app-pipeline`、`frontend`。

## 背景：当前架构的坑

每个角色调各自的 `register_*_tools`（`tools.rs`），工具集**互相不可见**：

| 角色 | 注册器 | 工具 |
|------|--------|------|
| Director | `register_director_tools` | search_world_info / get_character / emit_plan / search_vectors / get_recent_summary |
| Subagent | `register_subagent_tools` | get_character（锁死查自己，信息隔离写死在 handler） |
| Editor | `register_editor_tools` | compose |
| PostProcessor | `register_postprocess_tools`（在 `prompts/postprocess.rs`） | emit_postprocess |
| CharacterExtractor | `register_character_extractor_tools`（在 `prompts/character_extractor.rs`） | emit_characters |

后果：`tool_whitelist` 只能在「该角色已注册的组」内选。无法给 Subagent 配「开 Director 的 search_world_info」。这是「动态选配」的最大阻碍。

## 要实现的（`tool_center.rs`）

```rust
use crate::tools::{ToolHandler, ToolRegistry, filter_registry_by_whitelist};
use storyforge_domain::agent::AgentRole;
use storyforge_domain::llm::ToolSpec;
use std::collections::HashMap;
use std::sync::Arc;

/// 工具的适用角色范围
#[derive(Debug, Clone)]
pub enum ToolScope {
    /// 所有角色都默认可用
    All,
    /// 仅指定角色默认可用（不含 Subagent 通配，用 AgentRole::Subagent("*") 表示所有子 Agent）
    Roles(Vec<AgentRole>),
}

/// 全局工具注册项（描述用，handler 由 register 函数提供）
pub struct ToolSummary {
    pub name: String,
    pub description: String,
    pub scope: ToolScope,
}

/// 全局工具注册中心
///
/// **重要**：handler 是闭包（捕获 ToolContext），无法跨角色共享同一实例。
/// ToolCenter 的职责是「登记工具元信息 + 按角色调对应 register 函数构造 ToolRegistry」，
/// **不存 handler**。handler 仍在各 register_*_tools 函数里按需构造。
pub struct ToolCenter {
    /// 工具元信息（名字 → scope + description）
    summaries: HashMap<String, ToolSummary>,
}

impl ToolCenter {
    /// 启动时构建：登记所有已知工具的元信息
    pub fn builtin() -> Self {
        let mut c = Self { summaries: HashMap::new() };
        // 通用只读工具（不加前缀，scope = All）
        c.reg("search_world_info", "按关键词搜索世界书", ToolScope::All);
        c.reg("search_vectors", "向量记忆搜索", ToolScope::All);
        c.reg("get_recent_summary", "获取远记忆摘要", ToolScope::All);
        // Director 专属
        c.reg("get_character", "查任意角色详情（Director 版，可见全部）",
              ToolScope::Roles(vec![AgentRole::Director]));
        c.reg("emit_plan", "导演输出 Plan", ToolScope::Roles(vec![AgentRole::Director]));
        // Subagent 专属（get_character 的隔离版同名，但 scope 限 Subagent）
        c.reg("subagent.get_character", "子 Agent 查自己 instance（信息隔离）",
              ToolScope::Roles(vec![AgentRole::Subagent("*".into())]));
        // Editor / PostProcessor / CharacterExtractor 专属
        c.reg("compose", "编剧声明完成", ToolScope::Roles(vec![AgentRole::Editor]));
        c.reg("emit_postprocess", "后处理三合一输出", ToolScope::Roles(vec![AgentRole::PostProcessor]));
        c.reg("emit_characters", "卡导入时识别角色", ToolScope::Roles(vec![AgentRole::CharacterExtractor]));
        c
    }

    /// 某角色的默认工具名列表（按 scope 筛选）
    pub fn default_tool_names_for(&self, role: &AgentRole) -> Vec<String> {
        self.summaries.iter()
            .filter(|(_, s)| match &s.scope {
                ToolScope::All => true,
                ToolScope::Roles(rs) => rs.iter().any(|r| role_matches(r, role)),
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// 所有工具摘要（给前端选配 UI 用）
    pub fn all_summaries(&self) -> Vec<ToolSummary> {
        self.summaries.values().cloned().collect()
    }
}
```

**关键设计**：`role_matches(declared: &AgentRole, actual: &AgentRole) -> bool` 处理 Subagent 通配：
- `Subagent("*")` 匹配任意 `Subagent(_)`。
- 其他变体严格相等。

`build_registry_for` 不在 ToolCenter 里实现（因为 handler 在 register 函数里），而是提供一个**纯元信息层**。真正构造 registry 仍由调用方调对应 `register_*_tools`——这是**第一阶段**，不强制改造调用点（那是 PLAN-TOOL-REGISTRY 阶段 2 的事，本轮不做，避免动 pipeline/runtime）。

> 范围说明：本轮只做「元信息中心 + 默认列表 + 通配匹配」，**不改 pipeline/runtime 的调用方式**。让 `default_tool_names_for` 可被未来调用方使用即可。这样 E 完全独立，零运行时风险。

## 约束

- 不删不改 `tools.rs` 任何现有函数。
- 不引入新的 trait object 存 handler（闭包跨角色共享有生命周期问题，本轮规避）。
- ToolCenter 是纯元信息结构，`Send + Sync`（用 HashMap<String, ToolSummary>，ToolSummary 是 Clone）。

## 验证（你的 worktree 内运行）

```cmd
cargo test -p storyforge-app-agent
cargo check --workspace
```

`cargo check --workspace` 必须无新编译错误（E 只新增文件 + lib.rs 追加 mod，不应破坏任何现有编译）。

## 测试要求（至少 8 个，纯函数测试）

- `default_tool_names_for(Director)` 含 search_world_info / get_character / emit_plan / search_vectors / get_recent_summary（5 个）。
- `default_tool_names_for(Subagent("具体id"))` 含 `subagent.get_character`（注意通配匹配）。
- `default_tool_names_for(Subagent("*"))` 同样匹配。
- `default_tool_names_for(Editor)` 含 compose。
- `default_tool_names_for(PostProcessor)` 含 emit_postprocess。
- 通用工具（search_world_info 等）出现在 Director 和 Subagent 的默认列表里（scope=All）。
- `role_matches`：Subagent("*") 匹配任意 Subagent；Director 不匹配 Subagent。
- `all_summaries()` 返回全部工具（数量 ≥ 9）。

## 完成后报告

1. 改动文件列表 + 行数。
2. 新增测试数量 + `cargo test -p storyforge-app-agent` 结果。
3. `cargo check --workspace` 是否有新错误（应为 0）。
4. 是否动了禁止文件。

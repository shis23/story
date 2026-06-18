# 计划：统一 Tool 注册中心 + Agent 动态选配

> 状态：规划中（2026-06-17），未实现。
> 关联：`docs/PLAN-AGENT-PROFILE.md`（前置）、`crates/app-agent/src/tools.rs`（改造对象）。
> ROADMAP：Phase 3「统一 tool 注册中心」条目。

## 背景与决策

用户希望「每个 agent 的可用工具可以动态增删，同时保留默认配置」。审计后确认两种「动态」难度差一个数量级：

- **动态选配已注册工具（A）**：在「代码里已存在的工具池」内，给每个角色开/关哪些工具。**可行、值得做。**
- **运行时新增全新工具（B）**：用户在前端造一个代码里没有的工具（含自定义逻辑）。**难，且是安全雷区，现阶段不做。**

本计划只做 **A**。理由：当前工具池极小（Director 5 个 / Subagent 1 个），瓶颈是工具少而非用户想自造工具；沙箱脚本工具属于 Phase 5-7 级别插件系统（MVU 插件线已在啃），不开第二条。

## 当前架构的坑（要解决的）

`crates/app-agent/src/tools.rs` 现状：每个角色调各自的 `register_*_tools`，工具集**互相不可见**：

| 角色 | 注册器 | 当前工具 | 数量 |
|------|--------|----------|------|
| Director | `register_director_tools` | search_world_info / get_character / emit_plan / search_vectors / get_recent_summary | 5 |
| Subagent | `register_subagent_tools` | get_character（锁死查自己，信息隔离） | 1 |
| Editor | `register_editor_tools` | compose（声明完成） | 1，且 `#[allow(dead_code)]` |
| PostProcessor | `register_postprocess_tools` | emit_postprocess | 1 |
| Summarizer | 无 | （空，纯输出） | 0 |
| CharacterExtractor | `register_character_extractor_tools` | emit_characters | 1，非写作链路 |

后果：`tool_whitelist` 只能在「该角色已注册的组」内选。无法给 Subagent 配「开 Director 的 search_world_info」，因为 Subagent 的 registry 根本没注册它。这是「动态选配」的最大阻碍。

**关键限制（必须显式处理）**：同名工具在不同角色行为不同。`get_character` 在 Director 能查任意角色，在 Subagent 被锁死只能查自己（信息隔离写死在 handler 里）。统一注册后不能让「给 Subagent 配 get_character 然后绕过隔离查别人」。

## 目标

1. 全局一个工具注册中心，所有工具（含角色变体）集中登记。
2. 每个角色声明默认工具列表；`AgentRunConfig.tool_whitelist` 可覆盖。
3. 信息隔离行为显式化：用角色前缀命名（如 `director.get_character` / `subagent.get_character`），不靠同名工具黑箱。
4. 前端给「可选工具池 + 勾选」UI。
5. 内置默认与改造前行为完全一致，不强制用户迁移。

## 非目标 / 禁止

- 不做运行时新增工具（B）。
- 不做沙箱脚本/WASM/Lua 工具。
- 不破坏 Campaign 信息隔离（Subagent 只能查自己 instance）。
- 不让 `app-agent` 依赖 `tauri-app` 或 `CampaignStore`。

## 数据模型

### 工具注册中心

```rust
/// 工具的适用角色范围。决定该工具默认出现在哪些角色的可选池里。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolScope {
    /// 所有角色都默认可用
    All,
    /// 仅指定角色默认可用
    Roles(Vec<AgentRole>),
}

/// 全局工具注册项
pub struct ToolRegistration {
    pub name: String,          // 全局唯一，如 "search_world_info" 或 "subagent.get_character"
    pub spec: ToolSpec,        // 发给 LLM 的 schema
    pub handler: Arc<ToolHandler>,
    pub scope: ToolScope,      // 默认角色归属
    pub description: String,   // 给前端选配 UI 显示的人类可读说明
}

/// 全局工具注册中心（进程级单例，启动时构建一次）
pub struct ToolCenter {
    tools: HashMap<String, ToolRegistration>,
}

impl ToolCenter {
    pub fn register(&mut self, reg: ToolRegistration);
    pub fn default_tool_names_for(&self, role: &AgentRole) -> Vec<String>;
    pub fn build_registry_for(&self, role: &AgentRole, whitelist: Option<&[String]>) -> ToolRegistry;
    pub fn all_tool_summaries(&self) -> Vec<ToolSummary>; // 给前端
}
```

### 角色前缀命名规则（解决信息隔离）

同名异行为工具按 `<role_prefix>.<base>` 命名：

- `director.get_character`（查任意角色）
- `subagent.get_character`（锁死查自己 instance）
- `emit_plan`（仅 Director）
- `compose`（仅 Editor）
- `emit_postprocess`（仅 PostProcessor）
- `emit_characters`（仅 CharacterExtractor）
- 通用只读工具（`search_world_info` / `search_vectors` / `get_recent_summary`）不加前缀，scope = `All`。

`build_registry_for(role, whitelist)` 时按 `ToolScope` + whitelist 收集对应 handler 注册进临时 `ToolRegistry`，隔离逻辑仍在各 handler 内（保持现状，不破坏）。

## 执行阶段

### 阶段 1：建立 ToolCenter，迁移现有注册器

改动文件：

- `crates/app-agent/src/tool_center.rs`（新）
- `crates/app-agent/src/tools.rs`（保留 `ToolRegistry`/`ToolHandler`/`retain`，注册逻辑迁出）
- `crates/app-agent/src/lib.rs`（导出 ToolCenter）

任务：

1. 新建 `ToolCenter` + `ToolRegistration` + `ToolScope` + `ToolSummary`。
2. 启动时构造一个 `ToolCenter`，把现有 6 个 `register_*_tools` 的工具全部登记进去（含角色前缀）。
3. `build_registry_for(role, whitelist)`：按 scope 选默认集 + 应用 whitelist + 实例化 `ToolRegistry`。
4. 单测：每个角色的默认工具名集正确；whitelist 过滤正确；前缀工具不被错误角色选中。

验收：

- `cargo test -p storyforge-app-agent` 全绿。
- 现有 `register_*_tools` 可保留为内部 helper（被 ToolCenter 构造时调用），不删，避免大改。

### 阶段 2：pipeline / runtime 改用 ToolCenter

改动文件：

- `crates/app-pipeline/src/lib.rs`（Director 工具改 `tool_center.build_registry_for`）
- `crates/app-agent/src/runtime.rs`（Subagent 工具改用 ToolCenter）
- `crates/app-agent/src/postprocess.rs`（PostProcessor 工具改用 ToolCenter）

任务：

1. ToolCenter 经 `WritingContext` 或构造注入传入（不破坏「app-agent 不依赖 tauri-app」）。
2. 各角色 spawn 时调 `tool_center.build_registry_for(role, whitelist)` 取 registry，取代直接调 `register_*_tools`。
3. whitelist 三态语义不变（None/空/列表）。

验收：

- 改造前后，内置默认 profile 行为完全一致（用现有 pipeline 集成测试验证）。
- 给 Subagent 配 whitelist = `["search_world_info"]` 能让它用上原本只有 Director 才有的搜索工具。

### 阶段 3：前端动态选配 UI

改动文件：

- `frontend/src/components/AgentProfileManager.vue`
- `frontend/src/tauri-api.js`
- `crates/tauri-app/src/lib.rs`（新增 `meta_list_tools` 命令，返回 ToolSummary 列表）

任务：

1. `AgentProfileManager` 的 tool_whitelist 编辑：从「逗号分隔文本框」升级为「可选工具池 + 勾选」。
2. 工具池按角色 scope 过滤显示（只显示该角色 scope 内 + 通用工具）。
3. 勾选状态映射到 `tool_whitelist`：全勾 = None（默认全部）；部分勾 = Some(选中列表)；全不勾 = Some([])（禁用）。

验收：

- `npm run build` 通过。
- 用户能给任意角色勾选/取消工具，保存后运行时生效。

## 风险

| 风险 | 缓解 |
|------|------|
| 前缀命名破坏现有 whitelist 配置 | 旧配置用无前缀名；迁移时在 `build_registry_for` 内做名字兼容（先查前缀名，再回退无前缀名） |
| 信息隔离被误配绕过 | 隔离逻辑写在 handler 内，不依赖白名单开关；`subagent.get_character` 即使被配给其他角色，handler 内仍校验 `current_character_instance_id` |
| 全局单例生命周期 | ToolCenter 用 `Arc` 注入，不做成全局 static，便于测试 |

## 验证命令

```bash
cargo test -p storyforge-app-agent     # 阶段 1
cargo test -p storyforge-app-pipeline  # 阶段 2
cargo test --workspace                 # 阶段 2 回归
cd frontend && npm run build           # 阶段 3
```

## 附：Agent 动态增删 —— 明确不做（留作未来可能）

本计划只让 **tool** 动态选配。**agent（写作流水线的角色）不做动态增删**，这是经过审计的有意决定，不是遗漏。把结论固化在此，避免以后反复纠结。

### 为什么 tool 能做、agent 不能做

两者耦合深度差一个数量级：

| | tool | agent |
|---|---|---|
| 注册 | 各角色 `register_*_tools`，可统一成 ToolCenter | `AgentRole` 是编译期枚举（7 个变体，`agent.rs:18`） |
| 增删 | 统一注册中心后可运行时选配 | 加 agent 要改枚举 + 重编译 |
| 顺序 | 无顺序，按白名单取用 | 流水线是写死状态机，阶段顺序硬编码 |
| 用户可配 | 开/关哪些工具 | 仅开关 PostProcessor/Summarizer、调参数 |

### 审计到的结构性阻碍（代码证据）

1. **`AgentRole` 是编译期枚举**（`crates/domain/src/agent.rs:18`）：
   `Director / Subagent(String) / Editor / Meta / CharacterExtractor / Summarizer / PostProcessor`。运行时不可扩展，加新 agent 类型必须改枚举 + 重编译。

2. **流水线阶段顺序硬编码**（`app-pipeline/src/lib.rs:218` `start_writing`）：
   ```
   阶段1 Director(必经) → 阶段2 Subagents(Director 的 Plan 动态决定数量) 
   → 阶段3 Editor(必经) → 阶段4 PostProcessor + Summarizer(可开关，并行)
   ```
   Director 和 Editor 是硬编码必经阶段，中间插不进别的 agent；每个阶段的输入/输出类型（`Plan` / `Vec<Performance>` / `Draft`）一对一咬死。

3. **Subagent 的「动态」是假动态**：Director 的 Plan 决定 spawn 几个 Subagent，但它们全是同一 `Subagent` 角色类型，不是动态新增 agent 类型。

4. **配置层只能调参数 / 开关，不能增删阶段**（`agent_profile_config.rs`）：
   `enable_postprocess`/`enable_summarizer` 只能关掉两个固定可选阶段；没有任何字段能新增 agent 类型或重排顺序。

### 判断：现阶段不该做

- 写作流水线的固定结构是 StoryForge 的核心设计（设计 §3.2），每一环的输入输出都为这套流程精心设计。改成「可插拔 agent 链」等于重写流水线，ROI 极低。
- 动态 agent 链是「插件系统」级别工程（agent 接口契约 + 输入输出适配 + 执行编排 + 失败回滚 + 配置 schema），属 Phase 7 之后，现在碰会把刚稳定的主线搞乱。

### 现实能做到的边界

- **✅ 已有**：开/关 PostProcessor、Summarizer；给任意角色调模型/工具/轮次/并发。
- **✅ 可做（低成本，需求驱动）**：加新**固定** agent 类型（如 Critic 批评家）——改枚举 + 状态机加一段，一次性的、可控的，不是动态系统。需求来了再改。
- **⏸️ 留作未来**：运行时用户自定义 agent 类型 + 可插拔流水线。等主线完全稳定、有明确的「用户确实想造新 agent」需求时再评估。

> 若未来确实要做，应单独立 `docs/PLAN-DYNAMIC-AGENTS.md`，并先回答：用户场景是什么？固定阶段加一段满足不了吗？只有当「固定加阶段」频繁满足不了需求时，才值得投资动态系统。

## 与其他计划的关系

- 前置：`PLAN-AGENT-PROFILE.md`（AgentRunConfig.tool_whitelist 字段已就绪，本计划给它提供「可选池」）。
- 并行：可与 `PLAN-META-AGENT.md`（app-meta）并行，文件不重叠。
- 明确不做（tool 侧）：运行时新增工具 / 沙箱脚本工具（见「非目标」）。
- 明确不做（agent 侧）：agent 动态增删（见上方「附」节，留作未来可能）。

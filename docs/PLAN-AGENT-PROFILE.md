# 计划：可配置 Agent Profile 体系

> 状态：计划（未实现）
> 目标读者：可交给小模型按阶段执行
> 关联：`docs/AGENT_INTERFACES.md`、`crates/domain/src/prompt_module.rs`、`crates/domain/src/agent.rs`

## 目标

让当前硬编码的 Director / Subagent / Editor / Postprocess / Summarizer 行为变成可配置的 Agent Profile。现有的 PromptModule + PromptProfile + AgentBinding 三层体系已覆盖提示词模块的选择和覆盖，但 Agent 本身的运行参数（模型、工具权限、最大轮次、spawn 策略、postprocess 策略）仍然硬编码在 `app-agent` 和 `app-pipeline` 中。

完成后：

- 用户可以创建自定义 Agent Profile，覆盖模型参数、工具权限、迭代上限。
- 内置默认 Profile 与当前硬编码行为一致，不需要迁移现有用户数据。
- Pipeline 按 Profile 配置消费 Agent，而不是按硬编码常量。
- Profile 可导入/导出为 JSON，支持分享和备份。

## 非目标和安全约束

- 不做任意用户代码执行。Profile 只配置参数，不包含可执行脚本。
- 不让 `app-agent` 或 `app-pipeline` 依赖 `tauri-app`。
- 不破坏现有非 Campaign 写作路径。
- 不改变 `CampaignStore` 的位置和职责。
- 不在 Phase 1 做 GUI Profile 编辑器（先做数据模型和 store，GUI 后续跟进）。

## 当前代码事实

### PromptModule / PromptProfile / AgentBinding（`domain/prompt_module.rs`）

- `PromptModule`：最小提示词单元，有 `id`、`name`、`category`、`content`、`exclusivity`、`source`、`applicable_roles`、`tags`。
- `PromptProfile`：`id`、`name`、`selections: HashMap<AgentRole, HashMap<ModuleCategory, Vec<Id>>>`、`overrides: HashMap<AgentRole, Option<String>>`、`source`。
- `AgentBinding`：`role`、`active_profile_id`、`active_connection_id`。
- `assemble_system_prompt(role, role_directive, profile, modules, tool_directives)` 已实现按 Profile 组装 system prompt。
- `builtins::default_profile()` 创建内置默认 Profile + 5 个预置模块。

### AgentRole（`domain/agent.rs`）

- 枚举：`Director`、`Subagent(String)`、`Editor`、`Meta`、`CharacterExtractor`、`Summarizer`、`PostProcessor`。
- 自定义 serde 支持 `Subagent:*` 通配符格式和旧格式兼容。

### AgentProfile（`domain/agent.rs`）

- 当前结构：`role: AgentRole`、`system_prompt: String`、`max_tool_rounds: u32`、`tools: Vec<ToolSpec>`、`model_override: Option<String>`。
- 这是运行时快照，不是持久化配置。每次写作前由 pipeline 从 PromptProfile + 连接配置组装。

### Pipeline 硬编码点（`app-pipeline/lib.rs`）

- `DIRECTOR_SYSTEM_PROMPT`：硬编码的导演 system prompt 常量。
- `spawn_subagents` 中子 Agent 的 `max_tool_rounds` 为硬编码值。
- Editor 的 system prompt 为硬编码常量。
- Postprocess / Summarizer 的 config 由 `make_postprocess_config()` / `make_summarizer_config()` 构造，内部参数硬编码。

### 前端 Profile 管理（`tauri-api.js` + `tauri-app/lib.rs`）

- `listProfiles()`、`getActiveProfile()`、`saveProfile()`、`setActiveProfile()` Tauri commands 已存在。
- `module_store` 中有 `ProfileSummaryDto`、`PromptProfileDto`。
- 前端已有 Profile 选择/切换的基础 UI 能力。

### 连接管理（`tauri-api.js`）

- `listConnections()`、`createConnection()`、`setActiveConnection()` 已存在。
- 连接包含 `model`、`temperature`、`top_p`、`max_tokens` 等参数。
- 当前连接参数在 Agent 运行时被使用，但无法按 Agent 角色覆盖。

## 提议数据模型

### 扩展 PromptProfile → AgentProfileConfig

在现有 `PromptProfile` 基础上增加 Agent 级运行参数，不替换而是包装：

```rust
/// 单个 Agent 角色的运行时配置覆盖
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunConfig {
    /// 模型覆盖（None = 使用连接默认模型）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// 温度覆盖
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// 最大工具轮次
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_rounds: Option<u32>,
    /// 工具白名单（None = 使用默认工具集，Some = 只允许列表中的工具）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_whitelist: Option<Vec<String>>,
    /// 是否启用流式输出（某些场景可能需要关闭）
    #[serde(default = "default_true")]
    pub stream: bool,
}

/// 完整的 Agent Profile 配置（持久化用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfileConfig {
    pub id: Id,
    pub name: String,
    pub description: String,
    /// 继承自哪个 PromptProfile（提示词模块选择）
    pub prompt_profile_id: Id,
    /// 每个 Agent 角色的运行参数覆盖
    #[serde(default)]
    pub agent_configs: HashMap<AgentRole, AgentRunConfig>,
    /// Spawn 策略：子 Agent 最大并发数
    #[serde(default = "default_max_concurrent_subagents")]
    pub max_concurrent_subagents: usize,
    /// Postprocess 策略：是否启用
    #[serde(default = "default_true")]
    pub enable_postprocess: bool,
    /// Postprocess 策略：是否启用摘要
    #[serde(default = "default_true")]
    pub enable_summarizer: bool,
    /// 来源
    pub source: ProfileSource,
    /// 版本号（用于迁移）
    #[serde(default = "default_config_version")]
    pub config_version: u32,
}
```

### 内置默认 Profile 迁移

```rust
/// 内置默认 AgentProfileConfig，与当前硬编码行为一致
pub fn default_agent_profile_config() -> AgentProfileConfig {
    AgentProfileConfig {
        id: Id::from_str("builtin-default-agent-v1"),
        name: "默认 Agent 配置".into(),
        description: "与当前硬编码行为一致的默认配置".into(),
        prompt_profile_id: Id::from_str("builtin-default-v1"),
        agent_configs: HashMap::new(), // 空 = 全部使用默认值
        max_concurrent_subagents: 3,
        enable_postprocess: true,
        enable_summarizer: true,
        source: ProfileSource::BuiltIn,
        config_version: 1,
    }
}
```

### Store 层

在 `module_store` 中新增：

```rust
// module_store.rs 新增
fn list_agent_profile_configs() -> Vec<AgentProfileConfigSummaryDto>;
fn get_agent_profile_config(id: &Id) -> Option<AgentProfileConfig>;
fn save_agent_profile_config(config: AgentProfileConfig) -> Result<(), String>;
fn delete_agent_profile_config(id: &Id) -> Result<(), String>;
fn get_active_agent_profile_config() -> Option<AgentProfileConfig>;
fn set_active_agent_profile_config(id: &Id) -> Result<(), String>;
```

## 执行阶段

### 阶段 0：审计基线

目标：确认当前硬编码点和现有 Profile 体系的边界。

改动文件：无。

操作：

1. 运行 `cargo test --workspace`，记录基线。
2. 列出 `app-pipeline/lib.rs` 中所有硬编码 prompt 常量和数值常量。
3. 列出 `app-agent/prompts/` 中所有 `make_*_config()` 的硬编码参数。
4. 确认现有 `PromptProfile` 的消费点。

验收：

- 有一份硬编码点清单。
- 不产生代码改动。

### 阶段 1：Domain DTO + 序列化默认值

目标：定义 `AgentRunConfig` 和 `AgentProfileConfig`，确保能 serde round-trip。

改动文件：

- `crates/domain/src/agent.rs` 或新建 `crates/domain/src/agent_profile_config.rs`
- `crates/domain/src/lib.rs`

任务：

1. 定义 `AgentRunConfig` 和 `AgentProfileConfig` 结构体。
2. 所有新字段用 `Option` 或 `#[serde(default)]`，旧数据反序列化不崩。
3. 实现 `default_agent_profile_config()` 内置配置。
4. 单元测试：空 JSON 反序列化、完整 JSON round-trip、内置默认配置断言。

验证：

```bash
cargo test -p storyforge-domain
```

验收：

- 结构体可 serde round-trip。
- 空 JSON 反序列化得到合理默认值。

### 阶段 2：Store + Tauri Commands

目标：持久化 AgentProfileConfig，提供 CRUD Tauri commands。

改动文件：

- `crates/tauri-app/src/module_store.rs`（或等效 store 文件）
- `crates/tauri-app/src/lib.rs`
- `frontend/src/tauri-api.js`

任务：

1. 在 module_store 中实现 AgentProfileConfig 的 JSON 文件 CRUD。
2. 实现 active agent profile config 的获取和设置。
3. 注册 Tauri commands：`list_agent_profile_configs`、`get_agent_profile_config`、`save_agent_profile_config`、`delete_agent_profile_config`、`set_active_agent_profile_config`。
4. 前端 `tauri-api.js` 增加对应 wrapper。
5. 首次启动时自动创建内置默认配置（如果 store 为空）。

验证：

```bash
cargo test -p storyforge
```

验收：

- CRUD 命令可正常调用。
- 内置默认配置在首次启动时自动创建。

### 阶段 3：Pipeline / Runtime 消费 Profile

目标：Pipeline 按 AgentProfileConfig 参数运行 Agent，而不是硬编码常量。

改动文件：

- `crates/app-pipeline/src/lib.rs`
- `crates/app-agent/src/runtime.rs`
- `crates/app-agent/src/prompts/*.rs`
- `crates/tauri-app/src/lib.rs`（fill_campaign_context 或 fill_profile_context 扩展）

任务：

1. `WritingContext` 增加 `agent_profile_config: Option<AgentProfileConfig>`。
2. `fill_profile_context` 或 `fill_campaign_context` 加载 active AgentProfileConfig 写入 ctx。
3. Pipeline 构造 AgentProfile 时：
   - 从 AgentProfileConfig.agent_configs 读取该角色的 `AgentRunConfig`。
   - `model_override` 覆盖连接默认模型。
   - `max_tool_rounds` 覆盖硬编码值。
   - `tool_whitelist` 过滤注册工具。
4. `spawn_subagents` 使用 `max_concurrent_subagents` 控制并发。
5. Postprocess / Summarizer 的启用由 config 控制。
6. 无 config 时 fallback 到当前硬编码默认值。

验证：

```bash
cargo test -p storyforge-app-pipeline
cargo test -p storyforge-app-agent
cargo test --workspace
```

验收：

- 有 config 时参数被覆盖。
- 无 config 时行为与当前完全一致。

### 阶段 4：前端 Profile 管理 UI

目标：用户可以在前端查看、创建、编辑、导入/导出 AgentProfileConfig。

改动文件：

- `frontend/src/components/`（新建或扩展 Profile 管理组件）
- `frontend/src/tauri-api.js`

任务：

1. 列表页：显示所有 AgentProfileConfig，标记当前活跃。
2. 详情页：编辑 name、description、每个 Agent 角色的 AgentRunConfig。
3. 导入/导出：JSON 文件导入导出。
4. 内置默认配置不可删除，但可以复制为基础创建新配置。

验证：

```bash
cd frontend && npm run build
```

验收：

- 可创建自定义配置并切换。
- 导入/导出 JSON 正常。
- 内置默认配置始终可用。

### 阶段 5：校验、版本迁移、文档

目标：配置校验、版本兼容、文档更新。

改动文件：

- `crates/domain/src/agent_profile_config.rs`（校验逻辑）
- 相关文档

任务：

1. 校验：`tool_whitelist` 中的工具名必须是已注册工具；`max_tool_rounds` 有合理上下界；`temperature` 范围 0.0-2.0。
2. 版本迁移：`config_version` 字段向前兼容，新版本能读旧版本配置并填充默认值。
3. 文档更新：`AGENT_INTERFACES.md` 补充 AgentProfileConfig 说明；`DATA_MODEL.md` 补充新模型。

验证：

```bash
cargo test --workspace
```

验收：

- 无效配置在校验时返回明确错误。
- 旧版本配置能被新版本读取。

## 风险和回滚策略

| 风险 | 缓解 |
|------|------|
| Profile 配置导致 Agent 行为异常 | 内置默认 Profile 始终可用，用户可随时切回 |
| Pipeline 消费 config 时引入 bug | 阶段 3 用 `Option` 包裹，None 时走旧逻辑 |
| 前端 UI 复杂度 | 阶段 4 先做基础 CRUD，不做高级可视化编辑 |
| 旧数据兼容 | 所有新字段 serde(default)，反序列化不崩 |

回滚：每个阶段独立可回滚。阶段 1-2 只是新增 DTO 和 store，不影响现有逻辑。阶段 3 通过 Option fallback 保证旧路径不变。

## 验证命令

```bash
# 每阶段完成后
cargo test -p storyforge-domain          # 阶段 1
cargo test -p storyforge                 # 阶段 2
cargo test -p storyforge-app-pipeline    # 阶段 3
cargo test -p storyforge-app-agent       # 阶段 3
cd frontend && npm run build             # 阶段 4
cargo test --workspace                   # 阶段 5
```

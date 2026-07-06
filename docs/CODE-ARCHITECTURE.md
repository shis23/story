# StoryForge 代码架构与逻辑文档

> 生成日期：2026-06-19
> 基于 6 轮双盲审查的代码事实

## 1. Crate 依赖图

```
                    ┌─────────────┐
                    │  tauri-app  │ ← 组合根（87 Tauri 命令，5 OnceLock 单例）
                    └──────┬──────┘
          ┌────────────────┼────────────────┐
          │                │                │
    ┌─────┴─────┐   ┌─────┴─────┐   ┌─────┴─────┐
    │ app-meta  │   │app-pipeline│   │app-memory │
    └─────┬─────┘   └─────┬─────┘   └─────┬─────┘
          │         ┌─────┼─────┐          │
          │    ┌────┴────┐│┌───┴────┐      │
          │    │app-agent│││app-conv│      │
          │    └────┬────┘│└───┬────┘      │
          │         │     │    │           │
    ┌─────┴─────────┴─────┴────┴───────────┴─────┐
    │              infra-* 层                      │
    │  infra-llm  infra-vector  infra-import      │
    │  infra-plugin-host  infra-regex  infra-util  │
    └───────────────────┬─────────────────────────┘
                        │
                ┌───────┴───────┐
                │    domain     │ ← 零内部依赖
                └───────────────┘
```

**硬规则验证**：
- `app-agent` 不依赖 `tauri-app` ✅
- `app-pipeline` 不依赖 `tauri-app` ✅
- `domain` 零内部依赖 ✅
- DAG 无环 ✅

**已知违规**：
- 无当前分层违规；`infra-plugin-host` 的 Tauri/WebView runtime 已移至 `tauri-app/src/mvu_webview_runtime.rs`。

---

## 2. Domain 模型实体关系

```
CharacterCard (1) ──< CharacterDefinition (N)
     │                      │
     │                      ├── persona_prompt: String
     │                      ├── behavior_rules: String
     │                      ├── base_backstory: Vec<String>
     │                      ├── role_type: RoleType
     │                      └── variable_schema: Vec<VariableField>
     │
Campaign (1) ─── card_id ──> CharacterCard
     │
     ├── variables: Vec<VariableValue>
     ├── story_clock: String
     │
     ├──< CharacterInstance (N)
     │        ├── id: Id (UUID)
     │        ├── definition_id: Option<Id>
     │        ├── name: String
     │        ├── persona_override: Option<String>
     │        ├── behavior_override: Option<String>
     │        ├── variables: Vec<VariableValue>
     │        └── is_temporary: bool
     │
     ├──< CharacterKnowledgeEntry (N)
     │        ├── character_id: String
     │        ├── knowledge_text: String
     │        └── source: KnowledgeSource
     │
     ├──< StoryTask (N)
     │        ├── trigger: TaskTrigger
     │        └── status: TaskStatus
     │
     └──< RoundSummary (N)
              └── text: String

Conversation (1) ──< MessageNode (N) ──< MessageVariant (N)
     ├── character_id: Option<String>     ├── role: Role
     ├── campaign_id: Option<Id>          ├── content: String
     └── nodes: Vec<MessageNode>          └── provenance: Option<Provenance>
```

---

## 3. 共享可变状态目录

| 状态 | 类型 | 位置 | 访问模式 | 竞争风险 |
|------|------|------|----------|----------|
| `STORE` | `OnceLock<CharacterStore>` | `lib.rs:36` | ~20 读/写 | 测试并行干扰 |
| `CONN_STORE` | `OnceLock<ConnectionStore>` | `lib.rs:45` | ~10 读/写 | 同上 |
| `PRESET_STORE` | `OnceLock<PresetStore>` | `lib.rs:54` | ~8 读/写 | 低 |
| `CAMPAIGN_STORE` | `OnceLock<CampaignStore>` | `lib.rs:63` | 50+ 读/写 | **中** — 写入已返回 `Result`，且单 Mutex 已拆为集合级锁；同步 JSON I/O 仍需压测 |
| `MVU_RUNTIME` | `OnceLock<Arc<WebViewMvuRuntime>>` | `lib.rs` + `mvu_webview_runtime.rs` | 1 读 | 无 |
| `tool_ctx` | `Arc<RwLock<ToolContext>>` | `lib.rs:128` | 20+ 读, 8+ 写 | **高** — 写频繁 |
| `current_cancel` | `Mutex<Option<Sender>>` | `lib.rs:130` | 2 写, 1 读 | 低 |
| `active_llm` | `Mutex<Option<LlmClient>>` | `lib.rs:132` | 2 写, ~20 读 | 低 |
| `meta_patches` | `RwLock<Vec<Patch>>` | `lib.rs:138` | 2 读, 1 写 | 低 |
| `typed_patches` | `RwLock<Vec<TypedPatch>>` | `lib.rs:140` | 4 读, 3 写 | 低 |
| `meta_conversations` | `Mutex<HashMap<...>>` | `lib.rs:156` | 2 读, 2 写 | 低 |

---

## 4. 关键数据流

### 数据流 1：写作

```
App.vue::startWriting(intent)
  → tauri-api.js::startWriting (Tauri IPC)
  → lib.rs::start_writing (Tauri command)
    → snapshot_tool_ctx() — 读 tool_ctx RwLock
    → fill_regex_context() — 合并 active Preset + legacy 选中卡 Scoped regex_scripts
    → fill_profile_context() — 读 profile_store/module_store
    → fill_agent_profile_context() — 读 agent_profile_config_store
    → fill_campaign_context()
      → 读 active_campaign (Mutex)
      → 读 CampaignStore.list_instances/knowledge/tasks/summaries (各 Mutex)
      → 追加 active Campaign 卡内 Scoped regex_scripts（跳过既有同 ID Scoped）
      → 组装 CampaignRuntimeContext (Arc)
      → 写入 ctx.campaign_runtime + tool_ctx.campaign_runtime
    → new_pipeline() — 再次 snapshot tool_ctx
    → PipelineOrchestrator::start_writing
      → has_available_characters() — 校验 instances 或 characters 非空
      → build_director_tail() — 注入 campaign 变量/tasks/角色列表
      → Director Agent (run_tool_loop_with_layout)
        → get_character / search_world_info / emit_plan 工具
        → 输出 Plan { scene_brief, subagent_tasks }
      → with_temporaries_for() — 为未匹配角色创建临时 instance
      → spawn_subagents(plan.subagent_tasks)
        → 每个子 Agent: 独立 ToolContext + resolved persona/behavior/knowledge
        → run_tool_loop_with_layout (最多 N 轮工具调用)
        → 输出 Performance { character_id, full_text }
      → Editor Agent (run_tool_loop_with_layout)
        → compose 工具 → 合并为 final_text
      → conv_store.append_ai_draft(final_text)
      → build_provenance_with_campaign() → Provenance
    → persist_temporary_instances_to() — pipeline Ok 后写入 CampaignStore
    → persist_postprocess_outcome()
      → run_postprocess (知识/变量/任务提取)
      → 写入 CampaignStore (知识、变量、任务、摘要)
    → PipelineEvent 流 → 前端 UI 更新
```

### 数据流 2：Campaign 创建

```
CampaignPanel → create_campaign Tauri 命令
  → CampaignStore.get_card(card_id) — 验证卡存在
  → Campaign::new(card_id, name)
  → CampaignStore.save_campaign() — 持久化（返回 Result，Tauri 命令向前端传播 storage 错误）
  → conv_store.create() — 创建关联对话
  → 为每个 CharacterDefinition 创建 CharacterInstance
  → CampaignStore.add_instance() × N
```

### 数据流 3：角色导入

```
CharacterList → import_character_card Tauri 命令
  → infra-import::parse_st_card(data) — PNG tEXt 或 JSON
  → CharacterStore.save() — 持久化
  → tool_ctx.characters 更新
  → vector_store.search_by_keywords / upsert — 索引世界设定
```

### 数据流 4：Meta Agent

```
MetaPanel → meta_converse Tauri 命令
  → MetaSession 轮次
    → system prompt (Meta Agent 角色)
    → 工具: inspect_campaign, inspect_instance, propose_campaign_patch, ...
    → LLM 调用 → 工具执行 → LLM 响应
  → 返回对话消息
  → 可选: meta_accept_patch → execute_patch → CampaignStore 更新
```

### 数据流 5：Agent Profile

```
AgentProfileManager → save_agent_profile_config Tauri 命令
  → AgentProfileConfigStore.save() — JSON 持久化
  → 写作时: fill_agent_profile_context()
    → WritingContext.agent_profile_config = Some(config)
  → make_director_config / make_editor_config
    → model_override, max_tool_rounds 覆盖默认值
  → spawn_subagents
    → 子 Agent 按 profile 覆盖 model/max_tool_rounds
    → filter_registry_by_whitelist 按 profile 过滤工具
```

---

## 5. 公共函数目录（关键 crate）

### app-pipeline（5 个 pub fn）

| 函数 | 签名 | 行号 | 说明 |
|------|------|------|------|
| `PipelineOrchestrator::new` | `(llm, conv_store, tool_ctx, mvu_rt) -> Self` | 196 | 构造编排器 |
| `start_writing` | `(&mut self, intent, conversation_id, cancel, event_tx) -> Result<(String, Option<Plan>, Vec<Performance>)>` | 228 | 主写作流程 |
| `regenerate` | `(&mut self, req, cancel, event_tx) -> Result<(String, ...)>` | 748 | 重 roll（3 种模式） |
| `run_postprocess` | `(&self, ctx, final_text, cancel, event_tx) -> Option<PostProcessOutcome>` | 568 | 后处理 |
| `has_available_characters` | `(ctx) -> bool` | 1408 | 校验可用角色 |

### app-agent 关键类型

| 类型 | 说明 | 位置 |
|------|------|------|
| `AgentRuntime` | Agent 运行时，封装 LLM + ToolRegistry | `runtime.rs` |
| `ToolContext` | 工具上下文（characters/world_info/campaign_runtime/vector_store） | `tools.rs` |
| `ToolRegistry` | 工具注册表（retain/工具 specs/dispatch 同步） | `tools.rs` |
| `ToolSpec` | 工具描述（发给 LLM） | `tools.rs` |
| `AgentConfig` | Agent 运行配置（model/max_rounds/terminal_tools） | `runtime.rs` |

### domain 关键类型

| 类型 | 说明 | 位置 |
|------|------|------|
| `Campaign` | 一局故事存档 | `campaign.rs` |
| `CharacterInstance` | Campaign 内角色实例 | `campaign.rs` |
| `CharacterDefinition` | 卡级角色模板 | `character.rs` |
| `CampaignRuntimeContext` | 运行时快照（纯 DTO） | `campaign_runtime.rs` |
| `PipelineEvent` | 17 变体的写作流水线事件 | `agent.rs` |
| `AgentProfileConfig` | 可配置 Agent 运行参数 | `agent_profile_config.rs` |
| `CharacterKnowledgeEntry` | 角色知识条目 | `character_knowledge.rs` |
| `StoryTask` | 剧情任务/伏笔 | `story_task.rs` |
| `MvuTranslation` | MVU 变量 schema + 状态栏 | `mvu_translation.rs` |

---

## 6. PipelineEvent 完整变体表

| 变体 | 数据 | 触发条件 | 前端事件名 |
|------|------|----------|-----------|
| `Started` | session_id | 流水线开始 | `started` |
| `StateChanged` | state (PipelineState) | 状态转换 | `state_changed` |
| `DirectorStarted` | — | 导演开始 | `director_started` |
| `DirectorProgress` | delta: String | 导演增量输出 | `director_progress` |
| `DirectorDone` | scene_brief, subagent_count | 导演完成 | `director_done` |
| `SubagentStarted` | character_id, index, total | 子 Agent 开始 | `subagent_started` |
| `SubagentProgress` | character_id, index, delta | 子 Agent 增量 | `subagent_progress` |
| `SubagentDone` | character_id, index, full_text | 子 Agent 完成 | `subagent_done` |
| `SubagentCancelled` | character_id, index | 子 Agent 取消 | `subagent_cancelled` |
| `EditorStarted` | — | 编剧开始 | `editor_started` |
| `EditorProgress` | delta: String | 编剧增量 | `editor_progress` |
| `DraftReady` | text: String | 初稿就绪 | `draft_ready` |
| `PostProcessStarted` | — | 后处理开始 | `postprocess_started` |
| `PostProcessDone` | knowledge/variable/task_count | 后处理完成 | `postprocess_done` |
| `PostProcessFailed` | reason: String | 后处理失败 | `postprocess_failed` |
| `PostProcessSkipped` | reason: String | 后处理跳过 | `postprocess_skipped` |
| `SummaryDone` | char_count | 摘要完成 | `summary_done` |
| `Committed` | session_id, variant_id | 提交完成 | `committed` |
| `Error` | message: String | 流水线错误 | `error` |

---

## 7. 持久化格式目录

| 文件 | 格式 | 存储位置 | 读写频率 |
|------|------|----------|----------|
| `characters.json` | `Vec<StoredCharacter>` | data/ | 导入时写，启动读 |
| `connections.json` | `ConnectionsFile { connections, active_id }`，API key 字段为 SecretRef | data/ + 系统凭据库 | 配置时写，启动/切换连接时读 |
| `embed.json` | `EmbedConfig`，API key 字段为 SecretRef | data/ + 系统凭据库 | 配置时写，嵌入时读 |
| `presets.json` | `Vec<StoredPreset>` | data/ | 管理时写，启动读 |
| `active_preset.json` | `Option<String>`（当前运行时预设 id） | data/ | 切换时写，写作/重 roll 时读 |
| `modules.json` | `Vec<PromptModule>` | data/ | 管理时写，写作时读 |
| `active_profile.json` | `ActiveProfile { id }` | data/ | 切换时写，写作时读 |
| `profiles.json` | `Vec<PromptProfile>` | data/ | 管理时写，写作时读 |
| `agent_profile_configs.json` | `Vec<AgentProfileConfig>` | data/ | 管理时写，写作时读 |
| `active_agent_profile_config.json` | `{ id }` | data/ | 切换时写，写作时读 |
| `active_campaign.json` | `Id` | data/ | 切换时写，写作时读 |
| `cards.json` | `Vec<StoredCard>` | data/campaigns/ | 导入时写，启动读 |
| `campaigns.json` | `Vec<Campaign>` | data/campaigns/ | CRUD 操作时写读 |
| `instances.json` | `Vec<CharacterInstance>` | data/campaigns/ | CRUD + 写作后处理 |
| `knowledge.json` | `Vec<CharacterKnowledgeEntry>` | data/campaigns/ | 写作后处理写，Agent 读 |
| `tasks.json` | `Vec<StoryTask>` | data/campaigns/ | 写作后处理写，Director 读 |
| `round_summaries.json` | `Vec<RoundSummary>` | data/campaigns/ | 写作后处理写，Agent 读 |
| `mvu_translations.json` | `Vec<MvuTranslation>` | data/campaigns/ | MVU 分析写，UI 读 |
| `conversations.json` | `Vec<Conversation>` | data/ | 每次写作读写 |
| `world_info_vectors.json` | `HashMap<Id, VectorRecord>` | data/ | 导入写，搜索读 |

---

## 8. 错误类型层次

```
LlmError (domain) ─── 9 变体: Http/Auth/BadRequest/RateLimited/ServerError/StreamParse/Cancelled/Timeout/Internal
  ↓ .map_err()
AgentError (app-agent) ─── 5 变体: Llm/Tool/Config/Cancelled/Internal
  ↓ #[from]
PipelineError (app-pipeline) ─── 7 变体: Agent/Conversation/InvalidState/Cancelled/Channel/Llm/Internal

ImportError (infra-import) ─── 6 变体
VectorError (infra-vector) ─── 4 变体
PluginError (infra-plugin-host) ─── 6 变体
MvuRuntimeError ─── 5 变体
ConversationError (app-conversation) ─── 7 变体
MetaError (app-meta) ─── 3 变体
MemoryError (app-memory) ─── 2 变体
```

全部使用 `thiserror` v2，零 `anyhow`，零 `Box<dyn Error>`。

---

## 9. 测试覆盖矩阵

| Crate | LOC | 测试数 | 密度/100LOC | 状态 |
|-------|-----|--------|-------------|------|
| domain | 7,012 | ~144 | 2.05 | ✅ 强 |
| app-agent | 5,089 | ~82 | 1.59 | ✅ 强 |
| app-meta | 6,329 | ~101 | 1.60 | ✅ 强 |
| infra-regex | 218 | ~7 | 3.21 | ✅ 强 |
| infra-vector | 624 | ~10 | 1.60 | ⚠️ 中（缺 delete_by_character 测试） |
| infra-llm | 1,767 | ~24 | 1.36 | ⚠️ 中 |
| infra-import | 614 | ~9 | 1.47 | ⚠️ 中 |
| infra-plugin-host | 824 | ~12 | 1.46 | ⚠️ 中 |
| app-conversation | 908 | ~10 | 1.10 | ⚠️ 中 |
| app-pipeline | 3,042 | ~25 | 0.82 | ❌ 薄（核心路径） |
| tauri-app | 8,940 | ~54 | 0.60 | ❌ 薄（零 command 测试） |
| app-logging | 725 | ~4 | 0.55 | ❌ 薄 |
| app-memory | 454 | ~3 | 0.66 | ❌ 薄 |
| harness-real-llm | 2,677 | 38(集成) | — | ✅ 集成覆盖好 |
| frontend | 8,376 | 0 | 0 | ❌ 零测试 |

---

## 10. 前端架构概要

```
App.vue (1094 行 — 单体根组件)
├── 状态: ref/reactive 管理（无 Pinia）
├── Tauri IPC: tauri-api.js (1047 行, ~70 个 invoke 封装)
├── 流式事件: PipelineEvent → handlePipelineEvent()
│
├── 组件树:
│   ├── AppSidebar (导航)
│   ├── Composer (输入)
│   ├── ChatMessage / StreamingMessage (消息展示)
│   ├── CampaignPanel (Campaign 管理)
│   │   ├── CampaignInstancesTab
│   │   ├── CampaignKnowledgeTab
│   │   ├── CampaignTasksTab
│   │   └── CampaignSummariesTab
│   ├── MetaPanel (Meta Agent)
│   ├── DebugDrawer
│   │   ├── AgentConfigCard (PromptProfile)
│   │   ├── AgentProfileManager (AgentProfileConfig)
│   │   ├── LogPanel
│   │   └── PluginPanel / PluginHost
│   ├── CharacterList / CharacterDetail
│   ├── ConnectionConfig
│   ├── PresetPanel
│   └── base/ (BaseOverlay, BaseDropdown, BaseButton, BaseDialog)
│
├── 工具:
│   ├── useTheme.js (主题切换)
│   ├── useClickOutside.js (点击外部)
│   ├── formatContent.js (Markdown 格式化)
│   ├── plugin-bridge.js (Plugin iframe 通信)
│   └── mock.js (开发用模拟数据)
│
└── 构建: Vite + Tailwind v4 + @tauri-apps/api v2
```

**已知架构债务**：
- App.vue 1094 行应拆分为 composables
- 无状态管理库（Pinia）
- 无前端测试框架
- 无 linter/formatter 配置

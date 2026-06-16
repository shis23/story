# StoryForge 项目交接文档

> 最后更新：2026-06-16（文档整合：精简重复章节，合并 CONVERSATION_FLOW 到 ARCHITECTURE）
> 本文档记录项目当前状态、已完成工作、架构决策和后续计划。
>
> **当前状态**：后端功能完整（写作流水线 + 连接管理 + 记忆系统 + 日志采集 + 对话操作 + Patch 执行 + **后处理流水线** + **Meta Agent + MVU 分析**），
> **流式改造已完成（2026-06-16）**：子 Agent + meta_chat 全流式 + 编剧流式进对话。详见 §3.10。
> **对话完整性修复完成（2026-06-16）**：开场白 + user 意图存后端 + start_writing 支持复用对话 + 对话历史注入导演/编剧 + user 消息重 roll + 会话历史选择界面。详见 §3.12-§3.15。
> 前端主流程完整（会话历史选择 → 导入卡 → 配连接 → 写作 → 编辑/采纳/删除/分支 → 重 roll → 重启恢复），
> **前端 Campaign UI 已完成（2026-06-15）**：tauri-api.js 补全 21 个 P1/P2 API 函数 + CampaignPanel.vue 新组件（3 tab：角色卡/游玩档/档详情，含变量编辑/知识/任务/摘要面板）+ AppHeader 加 Campaign 按钮 + App.vue 集成（activeCampaign 状态 + 事件绑定），
> **桌面端已可运行验证**（`cargo tauri dev`，前端 dev server 1420 + Rust 后端，无需 Android 模拟器），
> **写作链路已全通**：真实 LLM 流式调用（导演+编剧 token 实时推送）+ Plan 解析多层兜底（手写括号配平）+ 子 Agent 产出展示，
> 新增模型列表拉取、世界书条目 CRUD、ST 风格 markdown 渲染（`*动作*`斜体）、消息列表自动滚动，
> **Android 构建链路已打通**（APK 编译成功），模拟器启动待验证，
> **P0 数据模型层已完成（2026-06-15）**：domain 新增 5 模块（variables/campaign/character_knowledge/story_task/message_layout）+ character 树形模型 + infra-vector 标签过滤，
> **P1 角色识别 + Campaign 闭环已完成（2026-06-15）**：角色识别 Agent + Campaign 开档后端闭环 + 14 个 Tauri 命令 + **前端 Campaign UI 已接入**，
> **P2 后处理流水线已完成（2026-06-15）**：后处理 Agent + 剧情总结 Agent + 并行编排 + CampaignStore 扩展 + 流水线接入 + 任务注入导演 + 6 个 Tauri 命令 + 前端事件桥接 + **前端知识/任务/摘要面板已接入**，
> **git 仓库已绑定**：`https://git.2529985.xyz/ss/story.git`（main 分支），
> 剩余工作见 §7 后续计划。

---

## 2. 已完成工作

### 2.1 需求分析与设计（完整）

| 文档 | 路径 | 内容 |
|------|------|------|
| 意图理解文档 | `docs/INTENT.md` | 48 条决策（D1-D48），覆盖所有功能需求 |
| 技术方案设计 | `docs/TECHNICAL_DESIGN.md` | 23 章完整方案，含架构/数据模型/里程碑/角色隔离/Campaign/MVU/叙事计划/cache/变量体系 |
| Agent 接口索引 | `docs/AGENT_INTERFACES.md` | 所有 Agent 的 prompt/上下文/输出解析位置，改 prompt 只看这文件 |
| ST 对比分析 | 对话中 | SillyTavern vs TauriTavern 的架构差异分析 |
| shujuku 工作流 | 对话中 | 从 VPS 读取并提炼的"总结/推进/向量化"工作流 |
| 双人成行预设 | 对话中 | 235 条 prompt 的能力图谱分析 |

### 2.2 Rust 后端（M0 + M1 交付 1 完成）

**Workspace 结构**：
```
storyforge/
├── Cargo.toml                          # workspace 定义（14 个 crate 成员）
├── crates/
│   ├── domain/                         # 纯领域模型（无 IO）
│   │   ├── character.rs                # 角色卡（ST V2/V3 兼容）
│   │   ├── world_info.rs               # 世界书（蓝灯/绿灯/路由）
│   │   ├── preset.rs                   # 预设（提示词+正则脚本）
│   │   ├── llm.rs                      # LLM 连接/消息/工具/错误类型
│   │   ├── agent.rs                    # Agent 角色/Plan/流水线状态/事件
│   │   ├── prompt_module.rs            # 提示词模块/Profile/绑定/组装函数
│   │   └── conversation.rs             # 对话树（MessageNode/Variant/Provenance）
│   ├── infra-import/                   # ST 数据导入
│   │   ├── lib.rs                      # 导入入口（PNG/JSON 自动检测）
│   │   ├── png.rs                      # PNG embed 解析（tEXt 块）
│   │   └── examples/inspect_card.rs    # 卡结构检查工具
│   ├── infra-llm/                      # LLM 客户端
│   │   ├── lib.rs                      # LlmClient trait + create_client 工厂
│   │   ├── http_client.rs              # HttpLlmClient（真 reqwest + SSE 流式）
│   │   ├── mock_client.rs              # MockLlmClient（导演/子/编剧脚本）
│   │   ├── sse.rs                      # 自研 SSE 解析器（按行解析 data:）
│   │   ├── openai.rs                   # OpenAI 兼容协议（请求构建+响应解析）
│   │   ├── text_tools.rs               # XML/JSON 降级工具协议
│   │   └── embedder.rs                 # 嵌入 API 客户端（/v1/embeddings）
│   ├── infra-vector/                   # 向量存储（M2 新增）
│   │   └── lib.rs                      # BruteForceStore + 余弦相似度 + 持久化
│   ├── infra-regex/                    # 正则引擎（M3 新增）
│   │   └── lib.rs                      # regress 封装 + Input/Output 作用域
│   ├── infra-plugin-host/             # 插件运行时（M4 新增）
│   │   └── lib.rs                      # PluginManifest / 权限校验 / PluginRegistry
│   ├── app-logging/                    # 应用日志系统
│   │   ├── lib.rs                      # LogBuffer/LogStore/导出 bundle
│   │   └── interceptor.rs              # LlmInterceptor（拦截 LLM 调用）
│   ├── app-agent/                      # Agent 运行时
│   │   ├── lib.rs                      # 模块 re-exports
│   │   ├── runtime.rs                  # AgentRuntime + run_tool_loop + spawn_subagents
│   │   └── tools.rs                    # ToolRegistry + 工具实现
│   ├── app-conversation/               # 对话管理
│   │   └── lib.rs                      # ConversationStore + 对话树操作 + 部分重 roll
│   ├── app-pipeline/                   # 写作流水线编排
│   │   └── lib.rs                      # PipelineOrchestrator + 状态机
│   ├── app-memory/                     # 记忆系统（M2 新增）
│   │   ├── lib.rs                      # 模块 re-exports
│   │   ├── archiver.rs                 # MemoryArchiver（归档器）
│   │   └── recall.rs                   # MemoryRecaller（召回器）
│   ├── app-meta/                       # Meta Agent（M5 新增）
│   │   └── lib.rs                      # 诊断工具 + Patch 系统
│   └── tauri-app/                      # Tauri 入口
│       ├── lib.rs                      # 79 个 Tauri 命令
│       ├── storage.rs                  # 角色卡持久化存储
│       ├── tauri.conf.json             # Tauri 配置
│       └── capabilities/default.json   # 权限定义
```

**已实现的 Tauri 命令**（79 个）：
- `import_character` — 导入角色卡（PNG/JSON，自动检测格式，同步写进 tool_ctx）
- `list_characters` — 列出已导入的角色卡
- `get_character` — 获取角色卡详情（含世界书条目）
- `delete_character` — 删除角色卡（同步从 tool_ctx 移除）
- `update_world_info_route` — 更新世界书条目路由（蓝灯/绿灯/Both/Disabled，同步 tool_ctx）
- `update_world_info_entry` — 编辑世界书条目的 keys/content/constant（同步 tool_ctx）
- `add_world_info_entry` — 新增世界书条目（同步 tool_ctx + 绿灯条目入向量库）
- `delete_world_info_entry` — 删除世界书条目（同步 tool_ctx）
- `import_preset` — 导入预设
- `get_version` — 获取版本号
- **LLM 连接管理**：
  - `list_models` — 拉取服务商可用模型列表（GET /v1/models，失败返回空走模板兜底）
  - `list_connection_templates` — 列出内置连接模板（DeepSeek/SiliconFlow/OpenAI/自定义）
  - `list_connections` — 列出已配置的连接（不含 key）
  - `get_active_connection` — 查询当前活跃连接
  - `create_connection` — 创建连接（从模板或自定义，首个自动设为活跃）
  - `delete_connection` — 删除连接
  - `set_active_connection` — 设为活跃
  - `test_connection` — 测试连通（发 ping 请求，返回成功/失败/延迟）
- `start_writing` — 启动写作流水线（用活跃连接的 LLM，返回 {text, conversation_id, node_id}）
- `cancel_writing` — 取消当前运行的写作流水线
- `list_conversations` — 列出所有对话
- `get_conversation` — 获取对话详情
- `regenerate` — 重 roll（整体/只重编剧/只重某子Agent，可附 hint）
- **对话操作命令**：
  - `edit_variant` — 编辑当前变体内容
  - `accept_variant` — 采纳变体（Draft → Final）
  - `soft_delete_variant` — 软删除变体（→ Discarded）
  - `add_variant` — 添加新变体（分支/swipe）
  - `switch_variant` — 切换变体（左右滑）
- `log_query` — 查询日志（按类型/级别/关键词/限制）
- `log_clear` — 清空日志
- `log_export_bundle` — 导出脱敏日志 bundle
- `log_append_frontend` — 前端日志上报（console.log/warn/error 转发到后端 LogStore）
- **M2 记忆系统命令**：
  - `configure_embedder` — 配置嵌入 API（endpoint/key/model/dim，持久化到 data/embed.json）
  - `get_embed_config` — 获取当前嵌入配置（不含 key）
  - `archive_conversation` — 手动触发对话归档（LLM 压缩 → 嵌入 → 入向量库）
- `meta_accept_patch` — 接受并执行 Meta Agent 的 Patch（修改世界书条目/角色字段）
- **P1 角色识别 / Campaign / 变量命令**（14 个）：
  - `extract_characters` — 跑角色识别 Agent，为已导入的扁平 Character 建 CharacterCard（含多角色 CharacterDefinition + MVU 字段级 schema），失败降级单角色 Protagonist
  - `list_cards` / `get_card` — 列出/查询 CharacterCard（含 character_definitions）
  - `create_campaign` — 开档：建 Campaign，把卡里所有 Protagonist/Supporting 定义实例化为 CharacterInstance
  - `list_campaigns` / `get_campaign` — 列出/查询 Campaign（可按 card_id 过滤）
  - `set_active_campaign` / `get_active_campaign` — 设置/查询活跃 Campaign（持久化到 data/active_campaign.json）
  - `list_instances` / `get_instance` — 列出/查询 Campaign 内角色实例
  - `get_character_variables` / `set_character_variable` — 角色实例变量读写（调试/纠错）
  - `get_campaign_variables` / `set_campaign_variable` — Campaign 全局变量读写（story_clock/weather/world_state）
  - `promote_temporary_instance` — 临场角色升级为常驻（翻 is_temporary flag）
- **P2 后处理流水线 / 任务管理命令**（6 个）：
  - `list_character_knowledge` — 列角色可见信息（character_knowledge，按 campaign_id + 可选 character_id 筛选，返回 id/text/source/source_character_id/turn/pinned）
  - `list_tasks` — 列叙事计划任务（按 campaign_id 筛选，可选 status_filter：pending/active/likely_completed/completed/abandoned）
  - `create_task` — 用户手动建任务（title/description/triggers/created_turn，自动分配 id）
  - `complete_task` — 标记任务完成（覆盖 Agent 判断，手动确认）
  - `abandon_task` — 放弃任务
  - `list_round_summaries` — 列本轮剧情摘要（按 campaign_id 筛选，按 turn 升序，200-500 字/条）

**测试**：242/242 单元测试通过（含 P3 新增：domain mvu_translation 11 个 + app-meta 24 个含 MVU 五合一/ST 分类/Meta 对话 + infra-plugin-host mvu_runtime 2 个 + tauri-app campaign_store MVU 持久化 2 个；代码审查修复新增 19 个：domain prompt_module 通配符匹配 2 + story_task trigger 优先级 1 + infra-util 4 + llm_parse 10 + infra-regex ReDoS 1 + postprocess parse_succeeded 1）。

### 2.3 前端（M0 完成）

**目录**：`storyforge/frontend/`

**已实现的功能**：
- 角色卡导入（文件选择器 → Rust 解析 → 显示结果）
- 角色卡列表（📋 按钮，展示已导入的卡）
- 角色卡切换（点选切换，顶栏更新）
- 角色卡删除（列表里的 ✕ 按钮）
- 角色卡详情（全屏弹层，展示描述/性格/场景/开场白/系统提示词）
- 世界书浏览（条目列表，蓝灯/绿灯标记，点击展开全文）
- **世界书路由编辑**（每个条目可切换 🔵蓝灯/🟢绿灯/🔵🟢两者/⚫禁用，实时同步后端 tool_ctx）
- **世界书条目 CRUD**（✏️ 编辑 keys/content/蓝绿灯 + 🗑 删除 + ➕ 新增，逗号分隔 keys，同步 tool_ctx）
- **模型列表拉取**（连接配置里 🔍 按钮调 /v1/models，datalist 可选可输入，失败模板兜底）
- 暗色模式（🌙 切换，localStorage 记忆）
- **写作流水线**（真实后端流水线，通过 Channel 流式事件更新 UI）
- **对话操作**（✏️ 编辑 → 内联 textarea；☑️ 采纳 Draft→Final；🗑 软删除；📑 分支新建 variant）
- **重 roll 菜单**（整体/只重编剧/只重某子 Agent，可附 hint）
- **对话历史恢复**（重启后自动加载最近一次对话 + 关联角色卡）
- **前端日志上报**（console.log/warn/error 自动转发到后端 LogStore，高玩模式日志面板可查）
- Agent 配置卡片（高玩模式）
- LLM 连接管理弹层（创建/删除/切换/测试连通）
- **前端 IPC 桥**（87 个 Tauri 命令全覆盖，含记忆系统 + Meta Agent + 世界书 CRUD + 模型列表）
- **ST 风格 markdown 渲染**（`*动作*`→斜体、`**粗体**`、换行，覆盖对话消息和开场白，先 HTML 转义防 XSS）
- **用户意图消息**（写作时自动把用户输入加入消息列表，消息流：开场白→意图→成文）
- **消息列表滚动**（`h-screen flex flex-col` 布局 + `flex-1 overflow-y-auto`，新消息/成文后自动滚动到底）
- **导演/编剧流式输出展示**（PipelinePanel 里实时显示导演/编剧 token，可折叠隐藏）
- **子 Agent 产出展示**（PipelinePanel 里子 Agent 完成后显示「▸ 查看表演」，点开看完整产出文本）

**组件列表**：
```
frontend/src/
├── App.vue                     # 主应用（对话恢复 + 流水线 + 对话操作事件处理 + Campaign 集成）
├── tauri-api.js                # Tauri IPC 桥（79 个命令全覆盖，含 P1/P2 Campaign/知识/任务）
├── useTheme.js                 # 主题切换
├── mock.js                     # 演示数据
├── style.css                   # 全局样式（Tailwind + 主题变量）
└── components/
    ├── AppHeader.vue           # 顶栏（角色名/导入/列表/Campaign/主题/高玩切换）
    ├── ChatMessage.vue         # 对话消息（编辑/采纳/删除/分支 + 重roll菜单 + 内联编辑）
    ├── Composer.vue            # 输入栏
    ├── PipelinePanel.vue       # 流水线状态面板
    ├── AgentConfigCard.vue     # Agent 配置卡片
    ├── CampaignPanel.vue       # Campaign 管理弹层（角色卡/游玩档/档详情 3 tab）
    ├── CharacterDetail.vue     # 角色详情弹层（含世界书路由选择器）
    ├── CharacterList.vue       # 角色列表弹层
    ├── ConnectionConfig.vue    # LLM 连接配置弹层
    └── LogPanel.vue            # 日志面板（高玩模式）
```

### 2.4 原型（独立项目）

**路径**：`C:\Users\Predator\ZCodeProject\storyforge-prototype/`

这是前端设计原型，用于验证 UI 风格。功能与主项目前端相同，但没有 Tauri 依赖，可以独立运行 `npm run dev` 预览。

---

## 3. 架构决策（关键）

### 3.1 多 Agent 架构

```
用户输入写作意图
       │
       ▼
  导演 Agent（强模型）
  · 解析意图 · 检索世界书/向量记忆
  · 拆解任务 · 构造专属上下文包
  · 输出 Plan
       │
       ▼ 并行派发
  ┌────┼────┐
  ▼    ▼    ▼
  子A  子B  子C（廉价模型，每角色一个）
  各自表演，独立取消
  └────┬────┘
       ▼
  编剧 Agent（强模型）
  · 收集子产出 · 合并润色
  · 输出最终成文
```

### 3.2 数据范式转换

| ST 范式 | 我们的范式 |
|---------|-----------|
| 预设/世界书 → 提示词注入 | 预设/世界书 → Agent 可检索的资料源 |
| 蓝灯 → 拼进 prompt | 蓝灯 → 导演 Agent 常驻上下文 |
| 绿灯 → 关键词触发注入 | 绿灯 → 向量检索池，Agent 主动查询 |
| 预设 → 决定 prompt 结构 | 预设 → 系统提示词模板 + 正则载体 |

### 3.3 兼容性边界

| 能力 | 做法 |
|------|------|
| ST 角色卡导入 | ✅ 完整兼容（PNG embed + JSON + V2/V3） |
| ST 世界书导入 | ✅ 完整兼容（蓝灯/绿灯 + 用户可调路由） |
| ST 预设导入 | ✅ 完整兼容（提示词 + spreset 正则） |
| 带前端角色卡渲染 | ⏳ 计划中（iframe 沙箱 + API 桥） |
| 真 ST 插件运行 | ❌ 不做（DOM 依赖 + 范式冲突） |
| 酒馆助手全局脚本 | ❌ 不做（与重写前端冲突） |
| 酒馆助手 API 兼容桥 | ⏳ 计划中（iframe 沙箱 + postMessage） |

### 3.4 三层预设体系

```
Layer ①  提示词模块（最小单位）
  视角/CoT/文风/约束/输出/基调
  单选组互斥，多选组叠加
       ↓ 组合
Layer ②  提示词预设 Profile（可保存命名）
  "小说预设v2" = {导演用X + 编剧用Y + ...}
       ↓ 绑定
Layer ③  Agent 绑定（运行时生效）
  每个 Agent 绑定一个 Profile + 一个 LLM 连接
```

### 3.5 三层记忆系统（参考 shujuku）

```
Layer 1: Recent Window（近期原文）
  最近 N 条消息，滑动窗口
       ↓ 窗口溢出触发归档
Layer 2: Archived Summary（远记忆大总结）
  批量纪要 → LLM 压缩（≤500TK）
  并发归档（tokio）
       ↓ 每条总结嵌入向量化
Layer 3: Vector Index（向量检索池）
  远记忆 + 绿灯世界书 + 角色设定
  关键词生成 → 向量召回 → rerank
```

### 3.6 LLM 客户端设计（M1 交付 1 新增）

**借鉴 TT 的关键设计决策**（不抄代码，读思路重写）：

| 决策 | TT 做法 | 我们的简化 |
|------|---------|-----------|
| SSE 解析 | 自研 SseEventAccumulator（reqwest stream + 手动按行解析 data:） | ✅ 完全借鉴，不用 eventsource crate |
| Client Pool | 按用途分桶（HttpClientPool） | 简化为 2 个 client（同步+流式） |
| 流式 client timeout | 无 request timeout，仅 connect timeout | ✅ 借鉴 |
| 协议分发 | match source { OpenAi => ..., Claude => ... } | M1 只实现 OpenAi 兼容 |
| 响应归一化 | 所有协议转 OpenAI chat.completion 格式 | ✅ 借鉴 |
| 取消机制 | tokio::sync::watch channel | ✅ 借鉴（非 CancellationToken） |
| 错误分层 | DomainError → ApplicationError → CommandError(Serialize) | ✅ 借鉴三层 |

**双路 LLM 客户端**：
- `HttpLlmClient`：真 reqwest + SSE 流式，需要 API key
- `MockLlmClient`：按 system prompt 关键词匹配返回预设响应，无需 key
- 通过 `create_client(conn)` 工厂函数按连接配置自动选择

**工具协议双路**：
- `Native`：OpenAI 原生 function calling（tools 字段）
- `TextFallback`：提示词注入工具描述 + 正则提取 `<tool_call>` 标签（兼容不支持 function calling 的模型）

### 3.7 对话树结构（M1 交付 1 新增）

**不照抄 ST 的线性 swipe 数组**，按设计 §3.7 做 MessageNode 树：

```
ST: Message { swipes: ["v1","v2","v3"], swipe_id: 1 }  ← 线性，分支是独立文件
我们: MessageNode { variants: [v1,v2,v3], active_variant: 1 }  ← 树形，分支在同树
```

**数据结构**：`Conversation { nodes: Vec<MessageNode> }`，每个 MessageNode 有多个 MessageVariant（swipe），每个 variant 带 Provenance（溯源信息，用于部分重 roll）。

### 3.8 M1 关键决策（用户确认）

| 维度 | 决策 | 说明 |
|------|------|------|
| 范围 | 完整 M1（设计 §13 全部清单） | 含日志系统 + 对话树 + 部分重 roll |
| 提示词 | 极简模块版 | PromptModule/Profile 数据结构 + 预置 3-5 模块 + 高玩模式模块编辑器 |
| 工具协议 | 原生 + XML/JSON 降级双路 | 兼容不支持 function calling 的模型 |
| LLM | DeepSeek（OpenAI 兼容） | key 用 mock+真client 双路，不硬编码 |
| 会话 | 持久化到文件 | data/conversations/<id>.json |

### 3.9 代码审查修复（2026-06-16）

对全项目（14 crate + 前端）做系统性代码审查，分 8 个 commit 修复，测试 215 → 234 全绿。

**修复的确定性 bug（Commit 1）**：
- `now_iso()` 误返回 Unix 秒数（应为 rfc3339），与 RoundSummary.created_at 格式统一
- 子 Agent 模块匹配失效：`applicable_roles.contains` 精确比较导致 `Subagent("*")` 通配符永远匹配不上具体角色，字数控制等模块不生效
- Meta Agent 双重 propose patch：handler 内与外层各 propose 一次，产生重复 patch
- 前端 switch-variant 按钮完全无效：ChatMessage emit 的事件 App.vue 未监听
- `meta_chat` 传不存在的 conversation_id 静默创建空对话丢失历史
- `meta_accept_patch` 把合并世界书写回最后一张卡 + is_global 硬编码 false

**系统改进（Commit 2-7）**：
- 新增 `app-agent/llm_parse.rs`，消除 4 处重复的 match_braces / 5 层兜底解析
- 新增 `infra-util` crate：统一持久化原子性（`.tmp→rename`，替换 10 处裸写 + 4 处重复样板）+ 锁中毒恢复（88 处 `.lock().unwrap()` → `unwrap_or_else(|p| p.into_inner())`）
- 中危逻辑：postprocess 加 `parse_succeeded` 区分解析 miss、story_task trigger 优先级、流式 usage（stream_options.include_usage）、SSE `data:` 空格兼容、LogBuffer 改 VecDeque、向量维度 warn、conversation invalidate
- 高危防护：流式 90s 空闲超时、PNG 导入 100MB/64MB 上限、正则 1MB 输入上限（ReDoS）、EmbedConfig/LlmConnection Debug 打码 api_key
- 删死代码：MemoryRecaller.llm 字段、mvu_import 空 tool_ctx；CharacterSummary 加 From 去重

**前端（Commit 8）**：写作失败回滚用户消息、7 处空 catch 加 console.error、MetaPanel v-for 用稳定 id、CampaignPanel 不再启发式强转数字。

**新增 crate**：`infra-util`（workspace 第 14 个成员）。

### 3.10 流式改造 + 重 roll 分支逻辑（2026-06-16）

三个改动，4 个 commit（见 git log `1aa396c`→`87ab578`）。

**C1 子 Agent 流式 + Semaphore 排队**（commit `52e6ced`）：
- 子 Agent 从非流式 `run_tool_loop` 改为流式 `run_tool_loop_streaming`，token 经 `SubagentProgress`（带 character_id + index）实时推前端。激活了 domain 层定义但从未发送的事件（曾经的「死代码」，见 ARCHITECTURE §7 旧债务已删除）。
- 并发上限 `MAX_CONCURRENT_SUBAGENTS=4` 从 `take(4)` 截断丢弃改为 `Semaphore` 排队——超额任务全部跑完，不再有 `SubagentFailed` 占位。行为变更：角色很多时总耗时变长（用户确认接受）。
- `spawn_subagents` 加 `event_tx` 参数；每个子 Agent per-channel 转发 delta；regenerate 路径 C 单子 Agent 同步改流式。
- 新增测试 `test_subagents_queue_beyond_concurrency_limit`（6 角色全跑完）。

**C2 meta_chat 流式**（commit `046bf84`）：
- Meta Agent 多轮对话改流式，token 实时显示。`app_meta::chat` 加 `progress_tx` 参数；tauri `meta_chat` 命令加 `on_event: Channel<MetaStreamEvent>` + 转发任务；前端 MetaPanel.vue 流式累积 token 到回复气泡，最终聚合结果校正。
- MVU 分析 / ST 分类 / 角色识别**未改**（一次性任务，同步返回可接受，用户选择）。

**C3 重 roll 最后一条「原地替换」**（commit `52e6ced` 后端 + `87ab578` 前端）：
- 重 roll **最后一条 AI 消息** → 不再开分支：`replace_active_variant`（旧 active→Discarded + push 新 active，软删除可 switch 切回）。
- 重 roll **中间消息** → 维持 `add_variant`（开分支，原行为）。
- 后端按 `nodes.last()` 实时判定（`is_last_assistant_node`，单一事实源，避免前端 isLast 脏数据）。前端 handleReroll 改「重拉对话刷新」UI（抽取 `applyConversation` 复用）。

**测试**：234 → 239 全绿（+5 新：1 Semaphore 排队 + 2 replace/is_last + 既有回归全过）。

**文档**：commit `1aa396c` 新增 `docs/ARCHITECTURE.md`（代码架构总览）；本轮同步修正 ARCHITECTURE.md 被 C1/C3 推翻的描述（子 Agent 非流式/丢弃/SubagentProgress 死代码/add_variant 等），并新增 §3.7 Meta Agent 子系统专章（Session/Patch/meta_chat 流式/MVU 五合一）。

### 3.11 UI 修复轮（2026-06-16，桌面端实测驱动）

用户用 `cargo tauri dev` 实测后报告 6 类问题，6 个 commit 修复（`f892df8`→`3a0c4be`）。测试：domain 69 + tauri-app 21 + app-conversation 10 全绿（含新增 AgentRole round-trip 4 + truncate_from 1）。

**已修复**：

| 问题 | 根因 | 修复 | commit |
|------|------|------|--------|
| Profile 存不进（unknown variant `Subagent("*")`）| `AgentRole::Subagent(String)` 元组变体作 HashMap key，serde 无法 round-trip | 手写扁平序列化（`Subagent:*`）+ 反序列化兼容旧格式 `Subagent("...")`；4 个回归测试 | `f892df8` `1715f29` |
| Campaign 面板看不到导入的卡 | `import_character` 返回存储随机 uuid，`extract_characters` 按 `Character.id` 查 tool_ctx 不匹配 | 导入后自动调 `extractCharacters`；extract_characters 加 name 回退（按存储 id 查 CharacterStore 拿 name 再按 name 查 tool_ctx） | `7d87fdd` `4470f9d` |
| 高玩模式预设按钮按不动 | 模块切换走 saveProfile，①serde 失败被 catch 吞掉；「保存新预设」按钮 `@click=null` | serde 修后恢复 + `saveAsNewProfile`（prompt 命名）；selections key `Subagent("*")`→`Subagent:*` | `7d87fdd` |
| 新建游玩档失败（missing `cardId`）| 14 个 Campaign 命令参数名全用 snake_case，Tauri v2 默认期望 camelCase | 全部改 camelCase（`create_campaign`/`list_instances`/`get_character_variables` 等 14 处） | `3a0c4be` |
| 删除消息无反应/删一条全删 | `window.confirm` 在 Tauri WebView 不弹窗；soft_delete 只删一个 variant 导致灰条残留 | 删除语义重做：`truncate_from`（删该条+其后所有=撤销写作）+ 清流水线 + Tauri 原生 dialog；start_writing 不存开场白致 truncate 后对话空 → 回显 first_mes | `4470f9d` `1cec6d2` `3a0c4be` |
| 分支按钮导致消息消失 | `addVariant` 是 ST 风格 swipe（加空 variant 切走原内容），与「分支=开新档」语义冲突 | 改提示（分支应在 Campaign 面板 fork）；`Campaign.fork` domain 方法已有，缺 Tauri 命令 | `4470f9d` |
| 删卡后 Campaign 面板不更新 | `delete_character` 只删 CharacterStore，不级联删 CampaignStore 的 CharacterCard | 加级联删（按 source_character_id 找 card 再删，含其 Campaign） | `3a0c4be` |

**遗留待办（本轮未做）**：

| 待办 | 说明 | 优先级 | 工作量 |
|------|------|--------|--------|
| **角色卡列表集成到 Campaign** | 根本问题：CharacterStore（扁平 Character）与 CampaignStore（CharacterCard）是双数据源，导入只写前者、extract 才有后者、删卡只删前者，必然不一致。方案：废弃 CharacterStore 统一到 CampaignStore，或后者作前者缓存层。影响 88 命令中大量用 CharacterStore 的命令 | 🔴 高 | 大（架构级） |
| **Campaign.fork Tauri 命令 + 前端入口** | `Campaign::fork(card_id, name, src, fork_node_id)` domain 方法已实现（`fork_from` 记录分叉点），但没暴露 Tauri 命令，前端无 fork 入口。「分支」按钮目前只弹提示。真分支=开新档复用对话树 | 🟡 中 | 中 |
| **tauri-api.js 参数名一致性检查** | 本轮 14 处 snake_case 参数名是系统性 bug（散落各处手动改）。建议加脚本：静态检查 tauri-api.js 的 invoke 参数名 vs Rust `#[tauri::command]` 签名，CI 防回归 | 🟢 低 | 小 |
| **顶部状态栏重构为侧边栏**（问题⑤）| AppHeader 按钮堆满（导入/列表/Campaign/预设/插件/Meta/主题/高玩）。改成点击「普通/高玩」唤出可折叠侧边栏，顶栏只留角色名 + 切换 | 🟢 低 | 中（纯 UI） |

**新增 Tauri 命令**：`delete_message_from`（删除指定消息及其后所有，截断对话）。
**新增 ConversationStore 方法**：`truncate_from`（+ 单测）、`replace_active_variant`、`is_last_assistant_node`（§3.10 C3 已加）。
**新增 domain**：AgentRole 自定义 serde（扁平字符串 + 旧格式兼容，4 测试）。

### 3.12 对话数据完整性修复（2026-06-16）

**根因**：`start_writing` 只在 pipeline 内部 `append_ai_draft`（成文），user 意图从未进后端对话树。前端本地 push 的 user 消息是纯展示用，`applyConversation` 刷新时被覆盖。删除唯一 AI 成文 → 后端对话空 → 前端回显开场白兜底（治标不治本）。

**修复**：

| 改动 | 文件 | 内容 |
|------|------|------|
| 开场白 + user 意图存后端 | `tauri-app/lib.rs` start_writing | 创建对话后、调 pipeline 前，先 `append_final_message`（开场白，Assistant/Final）+ `append_user_message`（user 意图）。对话结构从 `[AI成文]` 变为 `[开场白, user意图, AI成文]` |
| 删回显兜底 | `frontend/App.vue` handleDeleteVariant | 删掉 `if (!hadNodes && first_mes)` 兜底逻辑。改动后 truncate 不会让对话空（至少剩开场白 + user 意图） |
| ConversationStore 新方法 | `app-conversation/lib.rs` | `append_final_message(role, content)` — 通用 Final 状态消息追加（开场白等系统消息用） |
| 编剧流式进对话 | `frontend/App.vue` | `editor_progress` 事件同时更新 PipelinePanel 和对话里的 `editor-streaming` 占位消息，写作完成后替换为最终成文 |

**对话结构变化**：
```
之前: 后端 [AI成文]  ← 只有一个 node，truncate 后空
现在: 后端 [开场白, user意图, AI成文]  ← truncate 删 AI 成文后还有开场白 + user 意图
```

**测试**：242 全绿。

### 3.13 start_writing 支持复用已有对话（2026-06-16）

**问题**：每次 `start_writing` 都 `conv_store.create()` 新建 conversation。多轮写作产生多个独立对话，导演看不到前几轮的历史，重 roll 也只在当前对话里操作。

**修复**：`start_writing` 新增可选参数 `conversation_id: Option<String>`：
- `Some(id)` → 追加 user 意图到已有对话（不新建，不重复存开场白）
- `None` → 新建对话 + 存开场白 + 存 user 意图（原行为）

前端 `startWriting(intent, characterId, onEvent, conversationId)` 透传 `currentConversationId.value`。首次写 conversation_id 为 null（新建），后续写传已有 ID（追加）。

**同步新增**：`docs/CONVERSATION_FLOW.md` 完整对话链路图。

### 3.14 对话历史注入 Agent 上下文（2026-06-16）

**问题**：导演/编剧看不到之前的对话历史。`build_director_user_msg` 只注入 intent + 角色名 + 蓝灯世界书 + 任务，编剧只看当前轮子 Agent 产出。多轮写作时导演说「上文为空」，编剧风格断裂。

**修复**：

| 改动 | 文件 | 内容 |
|------|------|------|
| WritingContext 加 recent_messages | `app-pipeline/lib.rs` | 新字段 `recent_messages: Vec<String>`，最近 20 条带角色标签的对话 |
| Conversation 加 recent_messages_with_role | `domain/conversation.rs` | 返回 `"用户: {content}"` / `"AI: {content}"` 格式，支持 `before_node_id` 参数排除目标节点及之后的消息 |
| ConversationStore 加 recent_messages_with_role | `app-conversation/lib.rs` | 透传到 Conversation |
| tauri-app 加载历史 | `tauri-app/lib.rs` | `start_writing`：传 `None`（不排除）；`regenerate`：传 `Some(&node_id)`（排除被重 roll 的消息） |
| 导演注入历史 | `app-pipeline/lib.rs` | `build_director_user_msg` 加「最近对话历史」段落（在 intent 和角色名之后、世界书之前） |
| 编剧注入历史 | `app-pipeline/lib.rs` | `start_writing` 和 `run_editor_and_commit` 的 editor_user_msg 加「最近对话历史」段落 |

**重 roll 上下文排除**：`regenerate` 时 `recent_messages_with_role` 传 `before_node_id = Some(target_node_id)`，只返回目标节点之前的消息。避免导演看到被重 roll 的旧 AI 回复而困惑。

**注入的上下文（每个 Agent 看到的）**：

```
导演 user message：
  用户的写作意图：{intent}
  可用角色：{names}
  最近对话历史：           ← 新增
    用户: 第1条消息
    AI: 第2条消息
  世界设定（常驻）：...
  叙事任务/伏笔：...

子 Agent system prompt：
  你是角色 {name}。
  你的角色设定：...
  当前场景：...
  世界设定（常驻）：...
  相关世界设定：...
  最近对话：...           ← ContextPackage.recent_window（由导演 Plan 填充）

编剧 user message：
  场景：{scene_brief}
  子 Agent 表演：...
  最近对话历史：           ← 新增
    用户: 第1条消息
    AI: 第2条消息
```

**测试**：242 全绿。

### 3.15 user 消息重 roll + 会话历史选择界面（2026-06-16）

**user 消息重 roll**：`ChatMessage.vue` user 消息操作栏加「🔄 重roll」按钮 → `handleRerollUser`：
- 有 AI 消息 → 调 `regenerate`（空 targets = 整体重跑，hint = user 意图）
- 无 AI 消息（已删除）→ 调 `startWriting(intent, skipLocalPush=true)` 重新写作
- `startWriting` 加 `skipLocalPush` 参数：true 时跳过本地 user 消息 push（避免重复）

**会话历史选择界面**：启动时显示会话历史列表（不自动加载最近对话）：
- 会话列表用 `summary.message_count`（非 nodes.length）
- 点击会话 → `openConversation` 加载（`applyConversation` + 关联角色卡）
- 「新对话」按钮 → 清空消息，`currentConversationId = null`
- 顶栏 📜 按钮 → 返回历史列表

### 3.16 全项目审查修复（2026-06-16）

3 轮独立审查（22 个 agent）发现 97 个问题，已修复 7 Critical + 11 High + 11 Medium = 29 项。

**Critical 修复（7 项）**：

| 问题 | 文件 | 修复 |
|------|------|------|
| CampaignStore 无并发保护 | campaign_store.rs | 加 `Mutex<CampaignCache>` 内存缓存 |
| meta_accept_patch 世界书数据损坏 | lib.rs | is_global 从 route 推导 + keys 匹配替代 content 匹配 |
| atomic_write 临时文件碰撞 | infra-util/lib.rs | `format!("{}.tmp")` + 回退成功返回 Ok |
| PluginHost XSS 注入 | PluginHost.vue | DOMPurify 消毒 slotHtml + entry_html |
| confirm/alert 失效 | 6 个组件 | 全部改用 `@tauri-apps/plugin-dialog` |
| JSON 解析失败静默丢失数据 | 5 个 Store | .tmp 备份回退 |

**High 修复（11 项）**：7 处裸 unwrap 改 recover、testConnection 补 protocol、create_task 格式对齐、Embedder 返回 Result、PluginRegistry recover、ConversationStore ensure_loaded、let _ 改 warn、regenerate 发 Committed 事件。

**Medium 修复（11 项）**：向量库级联删、upsert 错误日志、constant_lore 解析、稀疏数组预填充、Composer 禁用、log 大小限制、StoryTime 大小写、空消息过滤。

详见 [FINAL-AUDIT-REPORT.md](FINAL-AUDIT-REPORT.md)。

---

## 4. 当前环境状态

| 组件 | 状态 | 说明 |
|------|------|------|
| Rust | ✅ 1.95.0 | stable-x86_64-pc-windows-msvc |
| Cargo | ✅ 1.95.0 | |
| Rust Android targets | ✅ 4 个 | aarch64/armv7/x86_64/i686 |
| JDK | ✅ 17.0.19 | Temurin |
| Tauri CLI | ✅ 2.11.2 | |
| Android SDK | ✅ | C:\Users\Predator\android-sdk |
| Android build-tools | ✅ 35.0.0, 35.0.1 | |
| Android platforms | ✅ android-35 | |
| **NDK** | ✅ 27.2.12479018 | 从腾讯镜像下载（746MB），解压安装 |
| **ANDROID_HOME** | ✅ 已设置 | C:\Users\Predator\android-sdk |
| **Gradle** | ✅ 8.14.3 | 从腾讯镜像下载 |
| **AVD** | ✅ StoryForge_Test | Pixel 6, API 35, x86_64 |
| **cargo.bat** | ✅ 已创建 | gradle 找不到 cargo 的 wrapper |
| Node.js | ✅ 24.13.1 | |
| npm | ✅ 11.8.0 | |
| WebView2 | ✅ 149.0 | 桌面端需要 |

**NDK 安装**（已完成）：
- 从腾讯镜像下载 `android-ndk-r27c-windows.zip`（746MB）
- 解压到 `C:\Users\Predator\android-sdk\ndk\27.2.12479018\`
- 验证：`source.properties` 文件存在，`Pkg.Revision = 27.2.12479018`

**Gradle 安装**（已完成）：
- 从腾讯镜像下载 `gradle-8.14.3-bin.zip`
- 放置到 `C:\Users\Predator\.gradle\wrapper\dists\gradle-8.14.3-bin\`

**cargo.bat wrapper**（已完成）：
- gradle 找不到 `cargo.bat`，创建 wrapper 指向 `cargo.exe`

**Android 构建命令**：
```bash
# 设置环境变量（每次新终端）
export ANDROID_HOME="C:/Users/Predator/android-sdk"
export JAVA_HOME="C:/Program Files/Eclipse Adoptium/jdk-17.0.19.10-hotspot"

# Rust 交叉编译
cargo tauri android build --target x86_64

# 手动复制 .so（符号链接在 Windows 上有问题）
cp target/x86_64-linux-android/release/libstoryforge_lib.so \
   crates/tauri-app/gen/android/app/src/main/jniLibs/x86_64/

# Gradle 打包 APK（跳过 rustBuild）
cd crates/tauri-app/gen/android
./gradlew assembleX86_64Debug -x rustBuildX86_64Debug

# APK 位置
crates/tauri-app/gen/android/app/build/outputs/apk/x86_64/debug/app-x86_64-debug.apk
```

---

## 6. 已知问题

### 6.0 本轮修复的 bug（2026-06-15）

| 问题 | 根因 | 修复 |
|------|------|------|
| 导演流式调用报 `JSON 解析失败: missing field name` | SSE 解析器用完整 `ToolCall`（要求 id/name 全必填）解析流式增量 chunk，但后续 chunk 只有 index+arguments 片段 | 新增 `StreamToolCallDelta`（id/name 全 Option + index），按 index 跨 chunk 合并 |
| 导演输出合法 Plan JSON 却报「超过最大工具调用轮次」 | `run_tool_loop_streaming` 的 drift recovery 无脑注入 reminder，没检查 content 是否已是最终结果 | 新增 `completion_probe` 回调，导演调用时探测 content 是否含合法 Plan，有则提早终止（业界推荐的 early termination） |
| 没导入角色卡就写作陷入死循环耗尽 15 轮 | 导演被要求分配子 Agent 但无角色可分配，反复输出空 Plan 被 drift recovery 拦截 | `start_writing` 开头检查 `ctx.characters.is_empty()`，空则直接报友好错误 |
| 重启后明明导入了角色卡，写作却报「没有可用角色卡」 | `AppState::new()` 初始化的 tool_ctx 是空的，启动时没有从 CharacterStore 恢复 | 启动恢复：`AppState::new()` 从 `characters.json` 读出所有卡，构造 domain Character + WorldInfoBook 填进 tool_ctx |
| Plan 解析失败（命定之诗等复杂卡） | `parse_plan_from_response` 只认「整个 content 是 JSON」或「```json 块」，模型输出「好的，计划如下：{...}」就失败 | 解析从 3 层加到 5 层兜底（+裸代码块+大括号提取）+ 导演提示词加 JSON 示例 + 失败信息附原始输出 |
| Plan 大括号提取用 regress 正则在中文内容下失效 | regress 的 `(?s)\{.*\}` 对多字节 UTF-8 字符的 range 索引行为有坑，中文 JSON 提取失败 | **完全放弃正则**，改用手写括号配平（从每个 `{` 开始计数深度，考虑字符串转义，到配平的 `}` 为止） |
| 消息列表缺用户意图 + 无法滚动查看开场白 | 写作时只存 AI 成文，用户意图从未进列表；外层容器 `min-h-screen` 无高度约束，`overflow-y-auto` 无效 | 写作前 push 用户意图；容器改 `h-screen flex flex-col` + 消息区 `flex-1 overflow-y-auto`，`scrollToBottom` 自动滚动 |
| 子 Agent 产出不可见 | `SubagentDone` 事件只推 character_id/index，不推 full_text | 事件加 `full_text` 字段（3 个发送点 + tauri 事件映射 + 前端展示「▸ 查看表演」折叠区） |
| `*动作*` 星号原样显示（ST 风格斜体动作） | 消息正文和开场白用纯文本 `{{ }}` 渲染，无 markdown 处理 | ChatMessage/CharacterDetail 加 `formatContent()`：`*斜体*` → `<em>`，`**粗体**` → `<strong>`，换行 → `<br>`，先 HTML 转义防 XSS + `prose-fiction em/strong` CSS |

### 6.1 ~~NDK 未安装~~（已修复）

**历史问题**：NDK 27.2 下载超时（网络问题），Android 构建不可用。

**当前状态**：✅ 已修复。从腾讯镜像下载 NDK r27c（746MB），解压安装。Gradle 8.14.3 也从镜像下载。Rust 交叉编译 + APK 打包均已成功。

**新增问题**：模拟器启动慢/崩溃。需要：
1. BIOS 开启 VT-x（Intel 虚拟化）
2. 安装 HAXM（Intel 硬件加速）
3. 或用 Android Studio 的模拟器管理器启动

### 6.2 复杂角色卡无法正确显示

**问题**：「命定之诗与黄昏之歌v4.1」这类脚本驱动卡，`first_mes` 只是占位符 "【首页】"，实际内容由酒馆助手脚本动态渲染。

**根因**：
- 卡的首页菜单由 `tavern_helper.scripts` 渲染
- 需要变量系统（`getLocalVar`/`setLocalVar`）
- 需要脚本运行时（iframe 沙箱 + API 桥）

**解决**：需要实现 Phase 1（变量系统 + 世界书查询 + regex 执行）+ Phase 2（脚本运行时）。见 §7.2。

### 6.3 ~~流水线动画是模拟的~~（已修复）

**历史问题**：写作流水线（导演→子Agent→编剧）曾是前端定时器模拟，没有真正调用 LLM。

**当前状态**：✅ 已修复（M1 交付 4 + 本次重 roll 修复）。前端用 Tauri Channel 接真实后端流水线事件；导入角色卡后 tool_ctx 自动同步，导演的 search_world_info/get_character 工具能查到真实数据。

### 6.4 ~~前端 dev server 需要手动启动~~（待确认）

**问题**：`cargo tauri dev` 不会自动启动前端 dev server，需要先手动 `npm run dev`。

**解决**：可以在 tauri.conf.json 里配置 `beforeDevCommand`，或用 `concurrently` 同时启动。

### 6.5 重 roll 约束（设计 §3.7.3 已落地）

**已实现的约束**：
- ✅ 整体重 roll（导演+子×N+编剧全跑）
- ✅ 只重编剧（复用旧子产出，省 token）
- ✅ 只重某子 Agent（其他子产出复用）
- ✅ 重 roll 可附加 hint（告知 Agent 上次哪里有问题，可选）
- ❌ 拒绝「只重导演却保留旧子产出」（Plan 变了旧子产出不匹配，后端校验拦截）

**取消机制**：`cancel_writing` 命令可中止运行中的写作（包括重 roll），导演/子Agent/编剧全部联动取消。

### 6.6 LLM 连接（已接入真实 LLM）

**当前状态**：✅ 真实 LLM 写作链路打通。用户可：
- 从内置模板（DeepSeek/SiliconFlow/OpenAI/自定义）创建连接
- 填 API Key + 测试连通（发 ping 验证）
- 设为活跃 → 写作流水线自动用该连接的 `HttpLlmClient`（而非 Mock）
- 启动时从 `data/connections.json` 自动恢复上次活跃连接

**关键设计**：
- API key **当前明文存** `data/connections.json`（桌面开发阶段）
- 写作前检查：无活跃连接时前端引导建连接（不让写作跑空 mock）
- `HttpLlmClient::new` 改返回 `Result`（配置错误不 panic，优雅返回错误给用户）
- M1 只支持 OpenAI 兼容协议；Anthropic/Gemini 原生协议留 TODO

**待做**（Android 阶段）：
- `infra-secrets` crate：API key 改走 Android Keystore + `SecretRef`（设计 §5）
- 网络权限配置（Tauri capabilities）

### 6.7 日志系统（已接通后端日志 + LLM 调用日志）

**历史问题**：`init_tracing()` 只输出到 stderr，不接 LogStore；LlmInterceptor 写好但没挂。

**当前状态**：✅ 已修复。
- **后端日志**（tracing → LogStore）：自定义 `LogStoreLayer` 把 `info!/warn!/error!` 转成 LogEntry 推进环形缓冲，前端日志面板 `Backend` tab 可查。
- **LLM 调用日志**（LlmInterceptor）：`set_active_connection` 和启动恢复时，用 `LlmInterceptor` 包装活跃 LLM client，每次 `chat/chat_stream` 自动记录 payload/响应/token/延迟到 LogStore。
- **前端/插件日志**：`log_append_frontend` 命令设计有，前端日志面板 `FrontendPlugin` tab 待前端接入。
- **导出 bundle**：`log_export_bundle` 可用，默认脱敏（正文替换为占位符）。

---

## 7. 后续计划

### 7.1 M1：写作流水线（核心功能）— 进行中

**目标**：完整 M1（设计 §13 全部清单）。分 4 次交付，每次可编译可测试。

**交付 1（阶段 0-1）✅ 已完成**：domain 扩展 + infra-llm（含 mock）+ 单测
- domain 新增 4 个模块：llm.rs / agent.rs / prompt_module.rs / conversation.rs
- infra-llm crate：HttpLlmClient（真 reqwest + SSE）+ MockLlmClient + XML/JSON 降级
- 16 个新增测试全绿（SSE 解析、OpenAI 协议、mock 客户端、工具降级）
- 审查修复：cancel 初始值检查、正则缓存（LazyLock）、Performance→SubagentSnapshot 转换

**交付 2（阶段 2-3）✅ 已完成**：app-logging + app-agent
- app-logging crate：LogBuffer 环形缓冲（每类 2000 条 LRU）+ LlmInterceptor（拦截每次 LLM 调用记录 payload/响应/token/延迟）+ LogStore（ERROR+LLM 落盘 JSONL）+ export_bundle 脱敏导出
- app-agent crate：AgentRuntime + run_tool_loop（max_rounds + drift recovery + watch 取消）+ spawn_subagents（tokio::spawn + 独立 cancel + 并发上限 4）+ ToolRegistry（search_world_info / get_character / emit_plan / compose）
- 4 个 app-logging 测试全绿

**交付 3（阶段 4-5）✅ 已完成**：app-conversation + app-pipeline
- app-conversation crate：ConversationStore（JSON 文件持久化 + AtomicBool 双重检查加载）+ 对话树操作（append / swipe / edit / soft_delete / accept）+ 部分重 roll 验证（不能只重导演却保留旧子产出）
- app-pipeline crate：PipelineOrchestrator + 完整状态机（Idle→Directing→Delegating→Editing→Review→Committed）+ start_writing（导演→子×N→编剧全流程）+ Plan 解析（emit_plan 工具调用 + JSON 降级）+ PipelineEvent 流式事件
- 6 个 app-conversation 测试全绿

**交付 4（阶段 6-8）✅ 已完成**：tauri-app 接入 + 前端 + 集成测试
- tauri-app：AppState 重构（注入 LlmClient/ConversationStore/LogStore/PipelineOrchestrator）+ 6 个新 Tauri 命令（start_writing / list_conversations / get_conversation / log_query / log_clear / log_export_bundle）
- 前端：tauri-api.js 新增 IPC 桥 + App.vue 替换 mock → 真 Channel 流式事件 + LogPanel.vue 日志面板（高玩模式）
- 集成测试：`test_full_pipeline_with_mock` — MockLlmClient 跑完整 Director→Subagent→Editor 闭环，验证成文/Provenance/事件序列/对话树写入
- 预置模块：5 个核心模块（第三人称/白描/杀八股/通用CoT/字数控制）+ 默认 Profile（绑定到导演/编剧/子Agent）+ 4 个测试全绿

| 任务 | 说明 | 状态 |
|------|------|------|
| ~~domain 扩展~~ | llm/agent/prompt_module/conversation 数据模型 | ✅ 完成 |
| ~~infra-llm~~ | HttpLlmClient + MockLlmClient + SSE + 降级 | ✅ 完成 |
| ~~app-logging~~ | 环形缓冲 + LLM 拦截 + 导出 bundle | ✅ 完成 |
| ~~app-agent~~ | 工具循环 + 委派 + 取消 + 提示词组装 | ✅ 完成 |
| ~~app-conversation~~ | 对话树持久化 + 编辑/删除/swipe/重 roll | ✅ 完成 |
| ~~app-pipeline~~ | 导演→子×N→编剧 状态机 | ✅ 完成 |
| ~~tauri-app 接入~~ | AppState 重构 + 12 个 Tauri 命令 | ✅ 完成 |
| ~~前端接入~~ | Channel 流式 + 流水线状态 + 日志面板 | ✅ 完成 |
| ~~集成测试~~ | MockLlmClient 端到端闭环 | ✅ 完成 |
| ~~预置模块~~ | 第三人称/白描/杀八股/通用CoT/字数控制 + 默认Profile | ✅ 完成 |
| ~~cancel 机制修复~~ | spawn_subagents 全局取消联动 + start_writing cancel sender 不再丢弃 + cancel_writing 命令 | ✅ 完成 |
| ~~tool_ctx 接通~~ | import_character 同步写进 tool_ctx（角色卡+世界书），导演工具能查到真实数据 | ✅ 完成 |
| ~~重 roll + hint~~ | regenerate 方法（整体/编剧/子Agent 三路径）+ 可选 hint 注入 + Provenance.last_hint | ✅ 完成 |
| ~~真实 LLM 连接~~ | 连接管理（CRUD+持久化）+ 内置模板 + test_connection + AppState 按配置选 client + 前端 ConnectionConfig 弹层 | ✅ 完成 |
| ~~日志 interceptor 挂载~~ | LlmInterceptor 包装活跃 LLM client（每次调用记录 payload/响应/token/延迟）+ tracing → LogStore 桥接层（后端日志进前端面板） | ✅ 完成 |
| ~~Android 构建链路~~ | NDK r27c（腾讯镜像）+ Gradle 8.14.3（腾讯镜像）+ cargo.bat wrapper + Rust 交叉编译 x86_64-linux-android + APK 打包（8.5MB） | ✅ 完成 |
| ~~对话操作命令~~ | edit_variant / accept_variant / soft_delete_variant / add_variant / switch_variant 5 个 Tauri 命令 | ✅ 完成 |
| ~~前端对话操作 UI~~ | ✏️ 编辑（内联 textarea）/ ☑️ 采纳 / 🗑 删除 / 📑 分支按钮接后端 | ✅ 完成 |
| ~~对话历史恢复~~ | onMounted 调 listConversations + getConversation，自动恢复最近对话 + 关联角色卡 | ✅ 完成 |
| ~~世界书路由 UI~~ | 每个条目可切换 🔵蓝灯/🟢绿灯/🔵🟢两者/⚫禁用，update_world_info_route 命令同步 tool_ctx | ✅ 完成 |
| ~~前端日志上报~~ | console.log/warn/error 自动转发到 log_append_frontend → LogStore（高玩模式日志面板可查） | ✅ 完成 |
| ~~Meta Patch 执行~~ | execute_patch（Create/Update/Delete 实际修改世界书条目）+ meta_accept_patch Tauri 命令 | ✅ 完成 |

### 7.2 M2：记忆系统与向量化 ⚠️ 部分接入

**已实现 crate**：infra-vector（BruteForceStore + 持久化 + 6 测试）+ app-memory（archiver + recall + 3 测试）+ infra-llm embedder

| 任务 | 说明 | 状态 |
|------|------|------|
| ~~infra-vector~~ | BruteForceStore（暴力余弦相似度 + JSON 持久化） | ✅ 完成 |
| ~~infra-llm embedder~~ | Embedder（/v1/embeddings API 客户端） | ✅ 完成 |
| ~~app-memory archiver~~ | MemoryArchiver（归档器代码） | ✅ 完成 |
| ~~app-memory recaller~~ | MemoryRecaller（召回器代码） | ✅ 完成 |
| **记忆系统接入流水线** | search_vectors 工具关键词搜索可用（vector_store 持久化 + 世界书绿灯条目自动入库 + new_pipeline 注入 ToolContext） | ✅ 完成 |
| **嵌入配置持久化** | configure_embedder 命令 + data/embed.json 持久化 + get_embed_config 查询 | ✅ 完成 |
| **归档器接入** | archive_conversation 命令（手动触发：LLM 压缩 → Qwen3-Embedding-8B 嵌入 → 入 BruteForceStore） | ✅ 完成 |
| hnsw_rs ANN 索引 | 当前用暴力检索，hnsw_rs 已引入待数据量大时启用 | ⏳ 后续优化 |
| **召回器自动触发** | accept_variant 后自动检查是否需要归档 + 流水线中自动召回相关记忆 | ⏳ 后续优化 |

> **当前可用**：
> - 导演的 `search_vectors` 工具能通过关键词搜索查到世界书条目（import_character 时绿灯条目自动入库）
> - `configure_embedder` 配置嵌入 API（当前用 SiliconFlow + Qwen3-Embedding-8B，4096 维）
> - `archive_conversation` 手动触发归档（LLM 压缩 → 嵌入 → 入向量库）
> - 向量库持久化到 `data/vectors.json`
>
> **待优化**：归档器自动触发（accept_variant 后自动检查）+ 召回器自动注入流水线。

**测试**：全 crate 测试全绿（见 §2.2 统计）。

### 7.3 M3：完整流水线 + 预设/正则 — 进行中

**新增 crate**：infra-regex（6 测试）

| 任务 | 说明 | 状态 |
|------|------|------|
| ~~infra-regex~~ | 正则引擎（regress 封装 + Input/Output 作用域 + 6 测试） | ✅ 完成 |
| ~~app-pipeline N 子 Agent~~ | 并发上限 4 + 独立取消（M1 已实现） | ✅ 完成 |
| ~~app-pipeline Provenance~~ | 每个 variant 记录子产出/Plan/种子（M1 已实现） | ✅ 完成 |
| ~~app-conversation 部分重 roll~~ | 只重跑某子 Agent / 只重跑编剧（M1 已实现 + 6 测试） | ✅ 完成 |
| ~~infra-import 预设导入~~ | ST 预设 JSON 解析（M0 已实现） | ✅ 完成 |
| 前端：正则编辑器 | 高玩模式正则脚本编辑/启用/禁用 | ⏳ 后续 |
| 前端：预设管理 | Profile 列表/切换/保存 | ⏳ 后续 |
| 前端：重 roll 粒度菜单 | 整体/只重子Agent/只重编剧 | ⏳ 后续 |

### 7.4 M4：插件运行时 + 角色卡渲染 ✅ 后端完成

**新增 crate**：infra-plugin-host（8 测试）

| 任务 | 说明 | 状态 |
|------|------|------|
| ~~infra-plugin-host~~ | PluginManifest / Permission / UiSlot / PluginRegistry / 权限校验 / ST API 映射 | ✅ 完成 |
| ~~后端：AppState + Tauri Commands~~ | plugin_registry 接入 AppState + list/install/uninstall/set_enabled 4 命令 | ✅ 完成 |
| ~~前端：插件管理页~~ | PluginPanel.vue + tauri-api.js 4 函数 + App.vue 集成 | ✅ 完成 |
| ~~前端：iframe 沙箱宿主~~ | PluginHost.vue + plugin-bridge.js（postMessage 协议 + window.storyforge stub） | ✅ 完成 |
| ~~前端：Slot 挂载集成~~ | 侧栏插件面板（SidebarPanel slot）+ 可折叠区域 | ✅ 完成 |
| ~~后端：插件 API 命令~~ | plugin_list_characters / plugin_read_character / plugin_get_variable / plugin_set_variable（权限二次校验） | ✅ 完成 |
| 前端：角色卡 iframe 渲染 | 带 HTML 的 ST 角色卡渲染（需共享 WebView，§19.6） | ⏳ 后续 |

### 7.5 M5：Meta Agent ✅ 后端骨架完成（部分能力待补）

**新增 crate**：app-meta（4 测试）

| 任务 | 说明 | 状态 |
|------|------|------|
| ~~app-meta 诊断报告骨架~~ | inspect_world_info（蓝灯关键词冲突检测）/ inspect_character（字段非空检查） | ✅ 完成 |
| ~~app-meta Patch 存储~~ | PatchStore（propose / accept / dismiss）数据结构 | ✅ 完成 |
| ~~Patch 执行 actions~~ | execute_patch（Create/Update/Delete 实际修改世界书条目/角色字段）+ meta_accept_patch Tauri 命令 | ✅ 完成 |
| 插件生成 / ST 预设分析 | 设计 §9.2/§9.4 的 meta_generate_plugin_from_st / meta_import_st_preset 等 | ❌ 未做 |
| 前端：Meta Agent 对话框 | 多轮对话式配置助手 | ⏳ 后续 |
| 前端：普通/高玩视图切换 | 双视图渐进披露 | ⏳ 后续 |

> 注：app-meta 当前是「诊断报告 + Patch 存储 + Patch 执行」，设计 §9 描述的完整 Meta Agent（插件生成助手 / ST 预设自动整理）工作量较大，留待后续迭代。
>
> ~~Phase 1/2/3~~ 已被 P0/P1/P2/M4 里程碑替代，变量系统（P0）、iframe 沙箱（M4）、记忆系统（M2）均已完成。

### 7.6 其他功能

| 功能 | 里程碑 | 说明 |
|------|--------|------|
| 角色卡编辑 | M3 | 高玩需求 |
| 多角色卡关联 | M3 | 群聊场景 |
| Agent 配置 UI | M3 | 按钮切换人称/文风/CoT |
| 插件运行时 | M4 | iframe 沙箱 + 自有 API |
| 角色卡渲染 | M4 | 带前端的卡（HTML/JS/CSS） |
| Meta Agent | M5 | 对话式配置助手 |
| 日志系统 | M1 | 后端/LLM/前端三类日志 |

### 7.7 后续工作优先级（当前状态）

后端骨架基本完整，前端接缝大部分已打通，桌面端已可运行验证。下一步重点：

| 优先级 | 工作 | 说明 | 阻塞项 |
|--------|------|------|--------|
| ~~🔴 高~~ | ~~写作流水线接通真实 LLM 流式~~ | ✅ 已完成：导演+编剧走 chat_stream，DirectorProgress/EditorProgress 事件实时推送前端，PipelinePanel 显示实时输出 | — |
| ~~🔴 高~~ | ~~模型列表拉取~~ | ✅ 已完成：list_models 命令（GET /v1/models）+ ConnectionConfig datalist + 🔍 拉取按钮（失败模板兜底） | — |
| ~~🔴 高~~ | ~~世界书条目 CRUD~~ | ✅ 已完成：update/add/delete_world_info_entry 3 命令 + CharacterDetail 内联编辑/新增/删除 | — |
| ~~🔴 高~~ | ~~启动恢复 tool_ctx~~ | ✅ 已完成：AppState::new 从 characters.json 恢复角色卡+世界书到 tool_ctx（重启后不再丢失） | — |
| ~~🔴 高~~ | ~~**世界书 depth 生效**~~ | ✅ 已完成：编辑表单加 depth/order 数字输入 + 查看模式显示 depth 徽章（d2）+ 后端已支持排序 | — |
| 🔴 高 | **Android 模拟器验证** | NDK + Gradle + APK 编译已成功，模拟器启动需开 VT-x + 装 HAXM | 用户操作 BIOS + 安装 HAXM |
| 🔴 高 | **角色子 Agent 信息隔离 + Campaign + 叙事计划 + 变量体系**（D30-D48） | 赛博跑团卡的核心能力：角色知识四元分类、一卡多角色树形、Campaign 隔离、知识抽取后处理 Agent、叙事计划系统（任务追踪/长程一致性）、cache 友好布局、三级变量体系。P0 数据模型 + P1 角色识别/Campaign 闭环 + **P2 后处理流水线已全部完成**。剩余：前端 Campaign 开档 UI + 共享 WebView 兜底（P3） | — |
| 🔴 高 | **MVU 原生兼容**（D42-D43） | 导入含 MVU 的卡即可用：原生协议层（stat_data/_.set 解析）+ 两层路由（轻量卡原生、重 DOM 卡共享 WebView）。**P1 已完成字段级 stat_data 解析**（探测 extensions.mvu.initvar / stat_data / variables，合并进 CharacterDefinition.variable_schema）。剩余：重 DOM 卡 JS 分析 + WebView 兜底（P3） | 共享 WebView 依赖插件运行时 |
| ~~🔴 高~~ | ~~**前端 Campaign 开档 UI**~~ | ✅ 已完成：CampaignPanel.vue（3 tab：角色卡/游玩档/档详情）+ tauri-api.js 21 个 P1/P2 函数 + AppHeader Campaign 按钮 + App.vue 集成 | — |
| ~~🔴 高~~ | ~~**全局世界书**~~ | ✅ 已完成：条目级 is_global 标记 + 前端「🌐 全局共享」开关 + 跨卡 merge 逻辑 | — |
| ~~🔴 高~~ | ~~**预设持久化+查看**~~ | ✅ 已完成：list_presets/get_preset/delete_preset 命令 + PresetPanel.vue（提示词/正则双 tab 查看）+ 📑 按钮入口 | — |
| ~~🟡 中~~ | ~~归档器接入~~ | ✅ 已完成：accept_variant 后自动 spawn 后台任务触发 maybe_archive（阈值 50 条，未配嵌入 API 静默跳过） | — |
| ~~🟡 中~~ | ~~预设编辑 + 模块系统打通~~ | ✅ 已完成：ModuleStore + ProfileStore 持久化 + 流水线接入 assemble_system_prompt + AgentConfigCard 接真实数据 + PresetPanel 编辑（prompt 内容/启停 + regex 启停） + ST 预设→模块桥接 | — |
| ~~🟡 中~~ | ~~ST 占位符替换~~ | ✅ 已完成：`replace_template_vars()` 支持 `{{char}}` `{{user}}` `{{charIfNotUser}}`，待接入流水线 | — |
| ~~🟢 低~~ | ~~M4 插件运行时前端~~ | ✅ 已完成：PluginPanel.vue + PluginHost.vue + plugin-bridge.js + 4 个插件 API 命令 + 侧栏 Slot 挂载 | — |
| 🟢 低 | app-meta 插件生成 / ST 预设导入分析 | 设计 §9.2/§9.4 的 meta_generate_plugin_from_st 等 | 工作量大 |
| 🟢 低 | infra-secrets + Keystore | API key 改走安全存储 | Android 环境 |

### 7.5 本轮新增需求备忘（2026-06-15，已确认方案待实现）

1. **全局世界书**（条目级标记）：用户希望某些世界书条目「不随角色卡变化，注入所有角色卡」。决策：给**单个条目**加「全局共享」标记（非整卡），标记后该条目独立于当前角色卡，任何角色卡激活时都生效。实现待定（可能需要独立的全局条目池或标记位 + 流水线启动时 merge）。

2. **世界书 depth**：ST 里 depth = 条目在消息历史里的插入位置（0=最底部=最重要，对齐 LLM 近因效应）。我们架构不注入 prompt，但 depth 可翻译为「常驻条目在导演 system prompt 里的排列顺序」。决策：前端编辑时显示 depth/order 可改 + 后端组装导演上下文时按 depth 排序（depth 小的靠后=更重视）。

3. **ST prompt-template 插件**：`{{char}}` `{{user}}` `{{random:}}` 占位符替换引擎。决策：不单独做插件，归入预设处理链——预设 prompt 里的占位符在运行时组装 system prompt 时替换。

4. **预设查看/编辑**：当前 `import_preset` 只返回字符串不存储。需求：持久化 + 列表 + 查看详情（每条 prompt 的 role/content/identifier + regex_scripts）。本轮先做到查看，编辑后续。

### 7.6 本轮设计深化（2026-06-15，D30-D48，方案已定待实现）

本轮对「赛博跑团式角色卡」场景做了深度设计，产出 19 条新决策（INTENT.md D30-D48），详见 TECHNICAL_DESIGN.md §16-§23。核心要点：

| 决策块 | 要点 | 详细位置 |
|--------|------|---------|
| **角色子 Agent 隔离** | 子 Agent 是上下文隔离容器，角色只知它该知的（character_knowledge 四元分类：witnessed/told_by_other/inferred/backstory） | TECHNICAL_DESIGN §16 |
| **多角色卡树形模型** | 一卡多角色（CharacterCard → CharacterDefinition[]），扁平一层 + 附加参数（主角团标签）。角色识别用独立 Agent（语义级，非正则） | TECHNICAL_DESIGN §17 |
| **Campaign 多会话隔离** | 一卡多档，数据按 campaign_id 隔离。角色分「定义」（卡级）和「实例」（会话级）。分叉复用现有对话树。实例改动默认不回写 + 手动固化 | TECHNICAL_DESIGN §17 |
| **角色知识系统** | 知识进现有 infra-vector（打 owner_character_id + campaign_id 标签）。知识抽取用后处理 Agent（编剧后并行），知识注入是确定性查表（零成本） | TECHNICAL_DESIGN §18 |
| **后处理流水线** | 编剧后并行跑两个 Agent：①剧情总结（独立，产 ArchivedSummary）②后处理（变量+知识+任务三合一）。两者并行不串行 | TECHNICAL_DESIGN §18 |
| **MVU 原生兼容** | 原生实现 MVU 协议层（stat_data/注入/解析_.set/更新）。两层路由：轻量卡走原生，重 DOM 卡走共享 WebView 计算单元（全局一个常驻，内存 O(1)） | TECHNICAL_DESIGN §19 |
| **叙事计划系统** | 任务追踪（quest tracker）：用户规划/叙事伏笔→每轮比对触发（事件/轮次/故事时钟三种）→接近时注入导演提示词→完成走软状态+用户确认。并入后处理 Agent 零额外调用 | TECHNICAL_DESIGN §21 |
| **cache 友好消息布局** | 三段布局（稳定 system + 稳定历史 + 易变末尾）+ MessageLayout 类型护栏（编译期强制分离）。变量/时钟/任务只进末尾 user，用完即弃。蓝灯世界设定移到 system | TECHNICAL_DESIGN §22 |
| **变量层级体系** | 三级变量（卡级 schema + 角色级实例值 + 全局 Campaign）+ 基础表（所有角色默认带 hp/mp/state/location/mood/relationship/inventory）。参考 MVU initvar+stat_data。修改接口记录在 AGENT_INTERFACES.md §7 | TECHNICAL_DESIGN §23 |
| **Agent 接口索引** | 所有 Agent 的 prompt/上下文/输出解析位置集中记录在新文档 AGENT_INTERFACES.md | AGENT_INTERFACES.md |

**落地优先级**（TECHNICAL_DESIGN §16-§23 的实现顺序）：
- ✅ **P0 数据模型层（已完成 2026-06-15）**：domain 加 Campaign/角色树/knowledge/变量表/story_task 结构 + infra-vector 加标签过滤 + MessageLayout 抽象。详见 §13.1
- ✅ **P1 角色识别 Agent + 导入集成 + Campaign 开档闭环（已完成 2026-06-15）**：角色识别 Agent（语义级拆多角色 + MVU 字段级解析）+ CampaignStore 持久化 + 14 个 Tauri 命令 + 角色/全局变量读写。详见 §13.2
- ✅ **P2 后处理流水线（已完成 2026-06-15）**：后处理 Agent（知识/变量/任务三合一，5 层兜底）+ 剧情总结 Agent（本轮摘要）+ 并行编排（tokio::join!，best-effort）+ CampaignStore 扩展（knowledge/tasks/round_summaries）+ 流水线接入（start_writing/regenerate 后自动触发）+ 任务注入导演（确定性查表，零 LLM）+ WritingContext 扩展 + 6 个 Tauri 命令 + 4 个 PipelineEvent + 前端事件桥接。详见 §13.3
- ✅ **P1.5 前端开档 UI（已完成 2026-06-15）**：CampaignPanel.vue（3 tab：角色卡/游玩档/档详情 + 4 子 tab：角色实例/知识/任务/摘要）+ 角色识别触发 + 开档表单 + 设为活跃 + 变量编辑 + 任务 CRUD（与 §7.7 一致）
- ✅ **P3 Meta Agent + MVU 分析（已完成 2026-06-16）**：MvuTranslation 领域模型 + 卡复杂度启发式打分 + Meta Agent LLM 多轮对话框架 + MVU 五合一分析 + ST 预设 LLM 分类 + 9 个 Tauri 命令 + MvuStatusBar 原生渲染 + 共享 WebView 接口桩（待下一轮实现执行）。详见 §7.10
- ⏳ P3 共享 WebView 真实 JS 执行（重 DOM 卡兜底，依赖真实卡端到端验证）

**实测数据（缄默之秋1.4 MVU 卡）**：重 DOM 型，document.×179 / getElementById×108 / innerHTML×60 / 4 个 script 块 18 万字符。这类卡必须走共享 WebView，轻量卡走原生协议层即可。

### 7.7 P0 数据模型层交付清单（2026-06-15 完成）

| 模块 | 位置 | 内容 | 测试数 |
|------|------|------|--------|
| `variables.rs` | domain | 变量 schema（VariableField/VariableType）+ 基础表（hp/mp/state/location/mood/relationship/inventory）+ Campaign 级变量 + 注入渲染（render_variables_for_injection）+ schema 合并 | 6 |
| `campaign.rs` | domain | Campaign（游玩档，含 fork_from 分叉 + story_clock）+ CharacterInstance（角色实例，含临时角色/常驻升级/persona 覆盖）| 8 |
| `character.rs` 扩展 | domain | CharacterCard（卡本体）+ CharacterDefinition（卡级定义：persona/behavior/backstory/group/role_type/variable_schema）+ RoleType（Protagonist/Supporting/Extra）| 3 |
| `character_knowledge.rs` | domain | CharacterKnowledgeEntry（角色可见信息）+ 四元分类（witnessed/told_by_other/inferred/backstory）+ pinned 字段 + 注入渲染（render_knowledge_for_injection）+ CharacterKnowledgeUpdate | 6 |
| `story_task.rs` | domain | StoryTask（叙事计划任务）+ 三种触发（Event/TurnReminder/StoryTime/Manual）+ 软状态完成检测（LikelyCompleted+置信度）+ TaskUpdate + 注入渲染 | 7 |
| `message_layout.rs` | domain | MessageLayout（三段布局：stable_system + stable_history + volatile_tail）+ Builder 类型状态机（编译期强制顺序）+ prefix_fingerprint（CI 断言 cache 稳定）| 6 |
| `infra-vector` 改造 | infra-vector | VectorRecord 加 metadata 字段 + VectorKind 加 CharacterKnowledge + MetadataFilter（for_character/for_campaign）+ search_by_vector_filtered/search_by_keywords_filtered + delete_by_campaign | 10 |

**附带修复**：app-pipeline 测试里 `let (_, cancel_rx) = watch::channel(false)` 的 sender 被 `_` 立即 drop，导致子 Agent `cancel.wait_for()` 误触发取消。改成 `_cancel_tx` 保活后，5 个流水线集成测试恢复 green。这是 P0 之前就潜伏的 bug，被这次重新编译暴露。

### 7.8 P1 角色识别 + Campaign 闭环交付清单（2026-06-15 完成）

| 模块 | 位置 | 内容 | 测试数 |
|------|------|------|--------|
| `AgentRole::CharacterExtractor` | domain/agent.rs | 新增角色识别 Agent 角色变体（Display → "角色识别"） | — |
| `extract_mvu_schema_from_extensions` | domain/variables.rs | 字段级 MVU 探测：4 个候选路径（mvu.initvar / stat_data / variables / depth_prompt.variables）+ 标量/对象双形态解析 | 4 |
| `CharacterDefinition::fallback_from_character` | domain/character.rs | 识别失败降级：单角色 Protagonist + persona 取 description+personality + 变量 schema 合并 MVU | 2 |
| `prompts/character_extractor.rs` | app-agent | `CHARACTER_EXTRACTOR_SYSTEM_PROMPT` 常量（含 JSON 输出示例）+ `make_character_extractor_config()`（max_rounds: 8）+ `build_character_extractor_user_msg()`（拼 description/personality/first_mes/alternate_greetings/world_book 全文）+ `register_character_extractor_tools()`（emit_characters 工具） | 3 |
| `character_extractor.rs` | app-agent | `extract_characters()` 编排（调 run_tool_loop）+ `parse_character_definitions_from_response()` 5 层兜底（emit_characters 工具 / 整体 JSON / ```json 块 / 裸代码块 / 手写括号配平）+ `attach_definitions_to_card()` 回填 card_id | 8（含端到端 mock LLM 闭环）|
| `CampaignStore` | tauri-app/campaign_store.rs | CharacterCard / Campaign / CharacterInstance 三文件持久化（cards.json/campaigns.json/instances.json，原子写 .tmp→rename）+ CRUD + 级联删除（删卡→删档→删实例）+ 同 source 去重 | 5 |
| `AppState.active_campaign` | tauri-app/lib.rs | 新增字段 + 持久化到 data/active_campaign.json + 启动恢复 | — |
| 14 个 Tauri 命令 | tauri-app/lib.rs | extract_characters（失败降级）/ list_cards / get_card / create_campaign（实例化 Protagonist+Supporting）/ list_campaigns / get_campaign / set+get_active_campaign / list_instances / get_instance / get+set_character_variables / get+set_campaign_variables / promote_temporary_instance | — |
| MockLlmClient 识别脚本 | infra-llm/mock_client.rs | match_keyword="卡内角色识别"，response=预设 Vec<CharacterDefinition> JSON，插在 scripts 最前避开"角色"冲突 | — |

**关键设计点**：
- **导入分两步**：`import_character`（同步纯解析，向后兼容）+ `extract_characters`（async 跑识别 Agent，失败降级）。前端导入成功后自动调 extract。
- **5 层兜底解析**：照搬 app-pipeline 的 parse_plan_from_response 模式（含手写括号配平 match_braces），LLM 输出再不稳定也能解析。
- **降级路径**：识别完全失败时建单角色 Protagonist definition（卡仍可用，不阻塞用户）。
- **MVU 字段级解析**：保守策略，探测不到结构化字段就用基础表（hp/mp/state/...），不报错。重 DOM 卡的 JS 分析留 P3。
- **导演工具暂不迁移**：P1 阶段导演的 get_character 继续读扁平 Character（兼容），CharacterDefinition 通过开档后的 CharacterInstance 暴露。P2 后处理接入时再迁移。

### 7.9 P2 后处理流水线交付清单（2026-06-15 完成）

| 模块 | 位置 | 内容 | 测试数 |
|------|------|------|--------|
| `prompts/postprocess.rs` | app-agent | `POSTPROCESS_SYSTEM_PROMPT` 常量（三大任务：知识/变量/任务，含 JSON 输出格式示例）+ `make_postprocess_config()`（max_rounds: 8）+ `build_postprocess_user_msg()`（拼成文+在场角色+变量键+轮次+时钟）+ `register_postprocess_tools()`（emit_postprocess 工具） | 3 |
| `postprocess.rs` | app-agent | `run_postprocess()` 编排（调 run_tool_loop）+ `parse_postprocess_from_response()` 5 层兜底（emit_postprocess 工具 / 整体 JSON / ```json 块 / 裸代码块 / 手写括号配平 match_braces）+ DTO 转换（PostProcessDto → PostProcessResult）+ best-effort（失败返回空，不报错） | 8（含端到端 mock） |
| `prompts/summarizer.rs` | app-agent | `SUMMARIZER_SYSTEM_PROMPT` 常量（内容优先级 6 项 + 200-500 字硬约束）+ `make_summarizer_config()` + `build_summarizer_user_msg()` | 3 |
| `summarizer.rs` | app-agent | `run_summarizer()` 编排（调 run_tool_loop，纯文本输出，无工具） | 1 |
| `pipeline_postprocess.rs` | app-agent | `run_postprocess_pipeline()` 并行编排（tokio::join! 并发跑总结+后处理，任一失败不影响另一个，返回 `PostProcessOutcome { summary, post_process }`）+ 模块 re-export（lib.rs） | 2（两个 succeed / cancel 测试） |
| CampaignStore 扩展 | tauri-app/campaign_store.rs | knowledge.json / tasks.json / round_summaries.json 三文件持久化 + CRUD（list/get/add/update/delete）+ 按 campaign_id 级联删除（删 campaign 时自动清 knowledge/tasks/summaries） | 5（knowledge/task/summary CRUD + 级联删除 + 去重） |
| WritingContext 扩展 | app-pipeline/src/lib.rs | `campaign_id: Option<Id>` + `turn: u32` + `pending_tasks: Vec<StoryTask>` + `story_clock: String` + `WritingContext::legacy()` 向后兼容构造 | — |
| 任务注入导演 | app-pipeline/src/lib.rs | `build_director_user_msg()` 末尾追加 `render_tasks_for_injection()`（确定性查表，零 LLM，只注入 Pending/Active 且触发满足的任务） | 1（test_director_msg_includes_pending_tasks） |
| 流水线接入 | app-pipeline/src/lib.rs | `PipelineOrchestrator::run_postprocess()` 新方法（有 campaign 才跑，推 PostProcessStarted/Done/Failed + SummaryDone 事件） | 2（无 campaign 跳过 / 有 campaign 跑通） |
| Tauri start_writing/regenerate 接入 | tauri-app/src/lib.rs | `fill_campaign_context()`（从 CampaignStore 加载 campaign_id/turn/tasks/story_clock）+ `persist_postprocess_outcome()`（知识→knowledge.json / 变量→instances+campaigns / 任务→tasks.json / 摘要→summaries.json）+ start_writing/regenerate 成功后自动触发 | — |
| 6 个 Tauri 命令 | tauri-app/src/lib.rs | `list_character_knowledge` / `list_tasks` / `create_task` / `complete_task` / `abandon_task` / `list_round_summaries` + DTO（KnowledgeEntryDto / StoryTaskDto / RoundSummaryDto） | — |
| 4 个 PipelineEvent 新事件 | domain/agent.rs + tauri-app 事件桥接 | `PostProcessStarted` / `PostProcessDone { knowledge_count, variable_count, task_count }` / `PostProcessFailed { reason }` / `SummaryDone { char_count }` + WritingEvent::from_pipeline_event 匹配 | — |
| MockLlmClient 新脚本 | infra-llm/mock_client.rs | match_keyword="后处理"（JSON 三件套）+ match_keyword="本轮剧情总结"（摘要文本） | — |

**关键设计点**：
- **总结 vs archiver 分离**：本轮摘要（200-500 字，每轮一条，存 round_summaries.json）≠ archiver 批量归档（窗口溢出时把多条摘要压成远记忆）。职责不重叠。
- **并行不串行**：tokio::join! 并发跑总结+后处理，任一失败不影响另一个。用 join! 而非 spawn（固定两个任务，无需并发上限控制）。
- **best-effort**：后处理失败只 warn，不阻断成文返回；无 campaign 时跳过（向后兼容）。
- **任务注入零 LLM**：build_director_user_msg 末尾追加 render_tasks_for_injection（确定性查表，ctx.turn / ctx.story_clock 比对 trigger）。
- **变量匹配**：后处理 Agent 输出的 instance_id 是角色名（String），persist_postprocess_outcome 通过 find_instance_by_name_or_id 翻译成 instance（先精确 id，再按 name 匹配）。
- **解析复用模式**：postprocess 的 5 层兜底是自己写的独立 match_braces（不复用 character_extractor 的私有 fn，避免跨文件耦合），但算法相同（括号配平 + 字符串转义处理）。

---

## 8. 关键文件索引

| 文件 | 用途 |
|------|------|
| `README.md` | 项目 README（快速开始 + 架构 + 命令列表） |
| `docs/INTENT.md` | 需求决策文档（48 条决策，含 D30-D48 角色子Agent/Campaign/MVU/叙事计划/cache/变量） |
| `docs/TECHNICAL_DESIGN.md` | 技术方案设计（23 章，含 §16-§23 新增：角色隔离/Campaign/MVU/叙事计划/cache/变量） |
| `docs/HANDOFF.md` | 项目交接文档（本文件） |
| `docs/AGENT_INTERFACES.md` | **Agent 接口索引**：所有 Agent 的 prompt 位置/上下文拼装/输出解析/修改方法，改 prompt 只看这文件 |
| `storyforge/Cargo.toml` | Rust workspace 定义 |
| **domain（纯领域模型，无 IO）** | |
| `crates/domain/src/llm.rs` | LLM 连接/消息/工具/错误类型 |
| `crates/domain/src/agent.rs` | Agent 角色/Plan/流水线状态/事件 |
| `crates/domain/src/prompt_module.rs` | 提示词模块/Profile/绑定/组装函数 + 预置模块工厂 |
| `crates/domain/src/conversation.rs` | 对话树（MessageNode/Variant/Provenance） |
| **domain P0 新增模块（2026-06-15）** | |
| `crates/domain/src/variables.rs` | 变量层级（VariableField/VariableType）+ 基础表 + 注入渲染（设计 §23） |
| `crates/domain/src/campaign.rs` | Campaign（游玩档）+ CharacterInstance（角色实例，设计 §17） |
| `crates/domain/src/character_knowledge.rs` | 角色可见信息（四元分类 + pinned，设计 §16） |
| `crates/domain/src/story_task.rs` | 叙事计划任务（三种触发 + 软状态完成，设计 §21） |
| `crates/domain/src/message_layout.rs` | cache 友好三段布局 + 类型护栏（设计 §22） |
| `crates/domain/src/character.rs` | 角色卡数据模型 + **CharacterCard/CharacterDefinition/RoleType（树形模型，P0 新增）** |
| `crates/domain/src/world_info.rs` | 世界书数据模型 |
| `crates/domain/src/preset.rs` | 预设数据模型 |
| **infra-llm（LLM 客户端）** | |
| `crates/infra-llm/src/lib.rs` | LlmClient trait + create_client 工厂 |
| `crates/infra-llm/src/http_client.rs` | HttpLlmClient（真 reqwest + SSE 流式） |
| `crates/infra-llm/src/mock_client.rs` | MockLlmClient（导演/子/编剧脚本） |
| `crates/infra-llm/src/sse.rs` | 自研 SSE 解析器 |
| `crates/infra-llm/src/openai.rs` | OpenAI 兼容协议 |
| `crates/infra-llm/src/text_tools.rs` | XML/JSON 降级工具协议 |
| `crates/infra-llm/src/embedder.rs` | 嵌入 API 客户端（/v1/embeddings） |
| **app-logging（应用日志）** | |
| `crates/app-logging/src/lib.rs` | LogBuffer/LogStore/export_bundle |
| `crates/app-logging/src/interceptor.rs` | LlmInterceptor |
| **app-agent（Agent 运行时）** | |
| `crates/app-agent/src/runtime.rs` | AgentRuntime + run_tool_loop + spawn_subagents |
| `crates/app-agent/src/tools.rs` | ToolRegistry + 工具实现 |
| **app-agent P1 新增（2026-06-15）** | |
| `crates/app-agent/src/prompts/character_extractor.rs` | 角色识别 Agent system prompt / config / 工具注册 / 用户消息拼装（D33） |
| `crates/app-agent/src/character_extractor.rs` | 角色识别编排（extract_characters）+ 5 层兜底输出解析（parse_character_definitions_from_response）|
| **app-agent P2 新增（2026-06-15）** | |
| `crates/app-agent/src/prompts/postprocess.rs` | 后处理 Agent system prompt / config / 工具注册 / 用户消息拼装（D40-D41/D45） |
| `crates/app-agent/src/postprocess.rs` | 后处理编排（run_postprocess）+ 5 层兜底输出解析（parse_postprocess_from_response） |
| `crates/app-agent/src/prompts/summarizer.rs` | 剧情总结 Agent system prompt / config / 用户消息拼装（AGENT_INTERFACES §6.3） |
| `crates/app-agent/src/summarizer.rs` | 总结编排（run_summarizer，纯文本输出） |
| `crates/app-agent/src/pipeline_postprocess.rs` | 并行编排（run_postprocess_pipeline，tokio::join! best-effort） |
| **tauri-app P1 新增（2026-06-15）** | |
| `crates/tauri-app/src/campaign_store.rs` | CharacterCard / Campaign / CharacterInstance 持久化（cards/campaigns/instances.json）+ CRUD + 级联删除 |
| **tauri-app P2 新增（2026-06-15）** | |
| 同 `campaign_store.rs` | P2 扩展：knowledge.json / tasks.json / round_summaries.json 三文件持久化 + CRUD + 按 campaign 级联删除 |
| **app-conversation（对话管理）** | |
| `crates/app-conversation/src/lib.rs` | ConversationStore + 对话树操作 |
| **app-pipeline（写作流水线）** | |
| `crates/app-pipeline/src/lib.rs` | PipelineOrchestrator + 状态机 |
| **infra-vector（向量存储，M2）** | |
| `crates/infra-vector/src/lib.rs` | BruteForceStore + 余弦相似度 |
| **infra-regex（正则引擎，M3）** | |
| `crates/infra-regex/src/lib.rs` | regress 封装 + Input/Output 作用域 |
| **infra-plugin-host（插件运行时，M4）** | |
| `crates/infra-plugin-host/src/lib.rs` | PluginManifest / 权限校验 / PluginRegistry |
| **app-memory（记忆系统，M2）** | |
| `crates/app-memory/src/archiver.rs` | MemoryArchiver（归档器） |
| `crates/app-memory/src/recall.rs` | MemoryRecaller（召回器） |
| **app-meta（Meta Agent，M5）** | |
| `crates/app-meta/src/lib.rs` | 诊断工具 + Patch 系统 + Patch 执行 |
| **infra-import（ST 数据导入）** | |
| `crates/infra-import/src/lib.rs` | 导入逻辑入口 |
| `crates/infra-import/src/png.rs` | PNG embed 解析 |
| **tauri-app（Tauri 入口）** | |
| `crates/tauri-app/src/lib.rs` | Tauri 命令定义（79 个）+ AppState |
| `crates/tauri-app/src/storage.rs` | 角色卡持久化 + 世界书路由更新 |
| `crates/tauri-app/src/connection_store.rs` | LLM 连接持久化（CRUD + 活跃连接） |
| `crates/tauri-app/gen/android/` | Android 项目（gradle 构建脚本 + APK 输出） |
| **前端** | |
| `frontend/src/App.vue` | 前端主应用 |
| `frontend/src/tauri-api.js` | Tauri IPC 桥 |
| `frontend/src/components/` | 前端组件 |

---

## 9. 测试用数据

**VPS 信息**（京东云）：
- IP: 111.228.49.176
- SSH: `ssh root@111.228.49.176`（密钥认证）
- ST 数据: `/root/sillytavern/sillytavern-data/data/default-user/`
- 角色卡: `/root/sillytavern/sillytavern-data/data/default-user/characters/`

**已下载的测试卡**：
- `storyforge/test-card.png` — 命定之诗与黄昏之歌 v4.1（6.4MB，复杂脚本卡）

**VPS 上可用的其他卡**：
```
default_Seraphina.png
修仙世界.png
催眠APP.png
垃圾回收员.png
县城.png
```

---

## 10. VPS 上的相关数据（从分析中提取）

### shujuku 脚本（v1.20）

**位置**：ST settings.json → `extension_settings.__userscripts.shujuku_v120__userscript_settings_v1`

**核心工作流**（已提炼，用于设计我们的记忆系统）：
- 归档：阈值 50 条 → 批量 3×9=27 条 → LLM 压缩（≤500TK）→ 并发 3 路
- 召回：关键词生成（12 个）→ 向量检索（topK=200, minScore=0.45）→ rerank
- 提示词：高密度总结，优先级：人物关系/关键事件/目标变化/冲突/道具/伏笔

### 双人成行 V6.1 预设

**位置**：`/root/sillytavern/sillytavern-data/data/default-user/OpenAI Settings/双人成行 V6.1—向斜阳.json`

**能力图谱**（235 条 prompt）：
- 视角（7 种）/ CoT（Gemini/Claude/GLM/DS）/ 文风（数十种）/ 约束（杀八股/抗抢话/抗绝望...）/ 输出规范 / 情感基调
- 已映射为我们的模块化挂载体系（§3.6）

### 小白X 配置

**位置**：ST settings.json → `extension_settings.LittleWhiteBox`

**已启用的功能**：storyOutline / storySummary / enaPlanner / variablesCore / fourthWall / novelDraw

---

## 附录：对话中的关键决策时点

| 时间 | 决策 | 依据 |
|------|------|------|
| 早期 | 仅安卓，砍 iOS/桌面 | 用户明确 |
| 早期 | 新项目，不 fork TT/ST | 砍掉用不上的功能 |
| 早期 | 2主+N子 Agent 架构 | 方案 A，用户确认 |
| 早期 | 小白X 后端原生化 | 用户明确 |
| 早期 | 不做真 ST 插件运行 | DOM 依赖 + 范式冲突 |
| 早期 | 自建插件运行时（自有 API） | 用户确认 |
| 中期 | Vue 3 + Tailwind 前端 | 我选择，用户接受 |
| 中期 | 双视图（普通/高玩） | 用户要求 |
| 中期 | Agent 提示词参考双人成行预设 | 用户要求 |
| 中期 | Meta Agent 增加插件生成/配置 Agent 能力 | 用户要求 |
| 中期 | 日志系统（三类） | 用户要求 |
| 近期 | position 字段兼容字符串格式 | 真实卡解析发现 |
| 近期 | 角色卡列表/切换/删除 | 用户要求先做 |
| 近期 | 脚本驱动卡需要变量系统+脚本运行时 | 真实卡分析发现 |
| M1 | 完整 M1 范围（含日志+对话树+部分重 roll） | 用户选择 |
| M1 | 极简模块版提示词（数据结构+预置+高玩模式编辑器） | 用户选择 |
| M1 | 原生 function calling + XML/JSON 降级双路 | 用户选择 |
| M1 | DeepSeek 连接，mock+真 client 双路 | 用户选择 |
| M1 | 会话持久化到文件 | 用户选择 |
| M1 | SSE 自研（借鉴 TT，不用 eventsource crate） | TT 分析 |
| M1 | 委派用 tokio::spawn + watch 取消（借鉴 TT） | TT 分析 |
| M1 | 对话树用 MessageNode（非 ST 线性 swipe） | 设计 §3.7 |
| M1 | cancel 初始值检查（流式请求前先检查取消状态） | 代码审查 |
| M1 | 正则缓存用 LazyLock（避免每次调用重新编译） | 代码审查 |
| 2026-06-15 | 子 Agent 是上下文隔离容器（非对话接口） | 用户 D30 |
| 2026-06-15 | 角色模型改树形（一卡多角色）+ 独立识别 Agent | 用户 D32-D33 |
| 2026-06-15 | Campaign 多会话隔离（定义 vs 实例）+ 分叉复用对话树 | 用户 D36-D37 |
| 2026-06-15 | 角色知识进现有 infra-vector（标签过滤）+ 后处理 Agent 抽取 | 用户 D39-D40 |
| 2026-06-15 | 剧情总结独立 Agent，与知识抽取并行（不合并） | 用户 D41 |
| 2026-06-15 | MVU 原生兼容（协议层原生 + 重 DOM 卡走共享 WebView） | 用户 D42-D43 |
| 2026-06-15 | 所有 Agent 接口留易操作入口，集中记录 AGENT_INTERFACES.md | 用户 D44 |
| 2026-06-15 | 叙事计划系统（任务追踪，三种触发条件都要，完成走软状态+用户确认） | 用户 D45 |
| 2026-06-15 | cache 友好三段布局 + MessageLayout 类型护栏（编译期强制分离） | 用户 D46 |
| 2026-06-15 | 三级变量体系（卡级 schema + 角色级实例值 + 全局 Campaign） | 用户 D47 |
| 2026-06-15 | 基础变量表（所有角色默认带，参考 MVU 变量构建方式，提供修改接口） | 用户 D48 |
| 2026-06-15 | MVU 架构修正：渲染/逻辑分离 + 翻译=JS→ToolCallSpec（非→Rust 结构）+ 元素级混合（能翻译的翻译，不能的保留 JS 执行）。变量更新走后处理 Agent 的 tool-call（不走正则/不走写作 Agent 附带输出）。导入时 Meta Agent 五合一分析（schema+绑定+规则+交互+兜底）。规则/交互可翻译成 tool-call 注入 Agent，只有自由逻辑/重 DOM 走 WebView | 对话推敲 |
| 2026-06-15 | P0 数据模型层完成（domain 5 新模块 + character 树形 + infra-vector 标签过滤，129 测试全过） | 落地实施 |
| 2026-06-15 | P1 角色识别 Agent + Campaign 开档闭环（角色识别语义拆多角色 + MVU 字段级解析 + 5 层兜底 + 降级 + CampaignStore 持久化 + 14 个 Tauri 命令，151 测试全过） | 落地实施 |
| 2026-06-15 | 修复 app-pipeline cancel sender 误 drop 导致子 Agent 取消的既有 bug | 测试回归 |
| 2026-06-15 | git 绑定到 Gitea（`https://git.2529985.xyz/ss/story.git`，main 分支） | 用户要求 |
| 2026-06-15 | P2 后处理流水线完整实现：后处理 Agent（知识/变量/任务三合一，5 层兜底，best-effort）+ 剧情总结 Agent（本轮摘要 200-500 字，独立于 archiver）+ 并行编排（tokio::join!，任一失败不影响另一个）+ CampaignStore 扩展（knowledge/tasks/round_summaries 三文件持久化 + CRUD + 按 campaign 级联删除）+ WritingContext 扩展（campaign_id/turn/pending_tasks/story_clock + legacy 向后兼容）+ 流水线接入（start_writing/regenerate 成功后自动触发，有 campaign 才跑，无 campaign 跳过）+ 任务注入导演（render_tasks_for_injection 确定性查表，零 LLM）+ 6 个 Tauri 命令（list_character_knowledge/list_tasks/create_task/complete_task/abandon_task/list_round_summaries）+ 4 个 PipelineEvent 新事件（PostProcessStarted/Done/Failed + SummaryDone）+ 前端事件桥接。188 测试全过（151→188，新增 37）。总结 vs archiver 分离（原子单位 vs 长期压缩）。变量匹配：后处理输出角色名，find_instance_by_name_or_id 翻译成 instance。 | 落地实施 |
| 2026-06-16 | P3 Meta Agent + MVU 分析完成：MvuTranslation 领域模型（domain，11 测试）+ 卡复杂度启发式打分（document./innerHTML/script 阈值）+ Meta Agent LLM 多轮对话框架（meta_conversation，挂接 inspect/propose_patch）+ MVU 五合一分析（mvu_import，5 层兜底 + 降级 + 纯数据卡短路）+ ST 预设 LLM 分类（增强现有纯启发式 bridge）+ 9 个 Tauri 命令 + mvu_translations.json 持久化（CRUD + 删卡级联）+ MvuStatusBar.vue 原生渲染（bar/text/tag/icon，零 JS）+ MetaPanel.vue（聊天框 + patch 卡片 + MVU 分析详情）+ 共享 WebView 接口桩（MvuRuntime trait + StubMvuRuntime，全部 NotImplemented）。215 测试全过（188→215，新增 27）。MVU 与 Meta 设计上耦合（五合一分析是 Meta Agent 的一个工具）。共享 WebView 真实 JS 执行留下一轮（需真实重 DOM 卡端到端验证）。 | 落地实施 |

---

## 7.10 P3 Meta Agent + MVU 分析交付清单（2026-06-16 完成）

| 模块 | 位置 | 内容 | 测试数 |
|------|------|------|--------|
| `mvu_translation.rs` | domain | MvuTranslation（5 字段：variable_schema/ui_bindings/update_rules/interactions/fallback_fragments + routing/confidence/notes）+ UiBinding/BindingDisplay（bar/text/tag/icon）+ InteractionMapping/InteractionAction（modify_variable/trigger_next_turn/multi/run_original_js）+ FallbackFragment + CardComplexityReport + score_card_complexity 启发式打分（document./innerHTML/script 阈值，基于缄默之秋1.4 实测倒推）+ pure_data_fallback 降级 + render_translation_for_review/render_update_rules_for_injection | 11 |
| `prompts/mvu_analyzer.rs` | app-meta | MVU_ANALYZER_SYSTEM_PROMPT（含五合一产物表 + 元素级判定规则 + JSON 输出示例，对齐 serde tag）+ make_mvu_analyzer_config + build_mvu_analyzer_user_msg（拼卡 HTML/JS/CSS 全文 + 启发式打分 + P1 字段，truncate 防爆）+ register_mvu_tools（emit_mvu_translation） | 4 |
| `prompts/meta_agent.rs` | app-meta | META_AGENT_SYSTEM_PROMPT（三大能力 + 安全约束）+ make_meta_agent_config + build_meta_user_msg（多轮历史拼接）+ register_meta_tools（inspect_world_info/inspect_character/propose_patch/classify_st_preset） | 4 |
| `mvu_import.rs` | app-meta | analyze_mvu_card 编排（启发式打分 → 纯数据短路 → LLM 五合一 → 5 层兜底解析 → 失败降级）+ parse_mvu_translation_from_response（5 层：工具调用/整体JSON/json块/裸块/括号配平）+ 容错反序列化（MvuTranslationRaw 全 default + wrapper 解包 + routing 自相矛盾纠正 + confidence clamp）+ classify_st_preset_with_llm（ST 预设 LLM 分类，6 大模块组）+ parse_st_classification（4 层兜底） | 10 |
| `meta_conversation.rs` | app-meta | MetaSession（character + world_info + PatchStore 共享状态）+ MetaConversation/MetaMessage/ToolResultDisplay（WorldInfoReport/CardReport/PatchProposed）+ chat 多轮编排（run_tool_loop + 工具结果结构化 + history_summary 截断）+ register_meta_runtime_tools（Arc<MetaSession> 捕获，挂接实际 inspect/propose） | 3 |
| MockLlmClient 新脚本 | infra-llm | match_keyword="卡内状态栏分析"（MvuTranslation JSON）+ match_keyword="配置调试助手"（Meta 诊断） | — |
| `mvu_runtime.rs` | infra-plugin-host | MvuRuntime trait + MvuExecResult + MvuRuntimeError + StubMvuRuntime（全部 NotImplemented，is_available=false）— **桩状态，下一轮实现 WebView 执行** | 2 |
| CampaignStore 扩展 | tauri-app/campaign_store.rs | mvu_translations.json 持久化 + list_all_mvu/get_mvu/save_mvu/delete_mvu + 删卡级联（delete_card 同步删 source_character_id 的 MVU）+ delete_character 级联 | 2 |
| AppState 扩展 | tauri-app/lib.rs | meta_session: Arc<MetaSession> + meta_conversations: Mutex<HashMap> + 初始化 | — |
| 9 个 Tauri 命令 | tauri-app/lib.rs | meta_start_conversation / meta_chat（异步多轮）/ meta_get_conversation / meta_list_pending_patches / meta_dismiss_patch / meta_analyze_mvu_card（手动触发五合一）/ meta_list_mvu_translations / meta_get_mvu_translation / meta_classify_st_preset（手动触发 ST 分类）+ sync_meta_session_from_tool_ctx | — |
| 前端 | frontend/src | MetaPanel.vue（聊天框 + patch 卡片 + MVU 分析下拉 + 详情浮层 + 诊断报告内嵌）+ MvuStatusBar.vue（bar/text/tag/icon 原生渲染，零 JS）+ tauri-api.js 9 函数 + AppHeader 🔧 按钮（高玩可见）+ App.vue 集成 + CharacterDetail 挂载状态栏 | — |

**关键设计点**：
- **MVU 与 Meta 耦合**：MVU 五合一分析是 Meta Agent 的一个工具（设计 §19.4），不是平行模块。Meta Agent = LLM 多轮对话框架（基础设施），MVU 五合一 + ST 预设分类都是它的工具。
- **手动触发**（D44）：MVU 分析不自动跑，用户在 MetaPanel 点「分析状态栏」按钮才触发。纯数据卡（无 JS 无 extensions）短路不调 LLM。失败降级回 P1 字段级。
- **启发式 + Meta 确认**：卡复杂度先纯 Rust 打分（document.>20 / innerHTML>10 / script>5KB → Heavy），结果喂给 LLM 做元素级二次判定。
- **5 层兜底**：照搬 character_extractor 模式（工具调用/整体JSON/json块/裸块/手写括号配平），独立实现 match_braces（不跨文件复用）。routing 自相矛盾（标 native 却有 fallback）自动纠正为 hybrid。
- **共享 WebView 留桩**：StubMvuRuntime 全部 NotImplemented。前端检测 fallback_fragments 非空时显示「需共享 WebView 支持」提示，不崩溃。真实 JS 执行留下一轮（需真实重 DOM 卡端到端验证）。
- **原生渲染路径**：MvuStatusBar.vue 按 BindingDisplay 原生画（bar 进度条按百分比分级染色 >50 绿/25-50 黄/<25 红 / text / tag / icon 映射），零 JS。CharacterDetail 自动加载此卡的 MvuTranslation 挂载状态栏。

**遗留（下一轮）**：
- ⏳ 共享 WebView 真实 JS 执行（WebViewMvuRuntime 实现 MvuRuntime trait，全局单例 iframe + postMessage + Mvu.parseMessages/setData 桥）
- ⏳ 真实重 DOM 卡（缄默之秋1.4）端到端验证 + prompt 调优
- ⏳ update_rules 接入后处理 Agent（让 Agent 按规则调 update_variable tool）
- ⏳ interactions 接入前端交互（用户点击按钮 → 调 modify_variable/trigger_next_turn）

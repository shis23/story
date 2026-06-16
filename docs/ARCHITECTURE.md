# StoryForge 代码架构总览

> 本文档描述**代码实现层面**的架构，与代码同步。需求决策看 [INTENT.md](INTENT.md)，设计方案看 [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md)，改 prompt 看 [AGENT_INTERFACES.md](AGENT_INTERFACES.md)，进度状态看 [HANDOFF.md](HANDOFF.md)。
>
> 体量参考：Rust 14 crate + 前端，约 84 个源文件 / 2.4 万行；88 个 Tauri 命令；234 个单元测试。

---

## 0. 文档定位与其它文档的边界

| 文档 | 回答什么问题 |
|------|------------|
| **本文（ARCHITECTURE.md）** | 代码长什么样？crate 怎么分层？一次写作从命令到落盘走过哪些函数？数据存哪？ |
| INTENT.md | 为什么这么设计？（48 条决策 D1-D48） |
| TECHNICAL_DESIGN.md | 方案该怎么实现？（23 章设计意图） |
| AGENT_INTERFACES.md | 每个 Agent 的 prompt 写在哪、怎么改？ |
| HANDOFF.md | 做到哪了？下一步做什么？ |

---

## 1. 分层架构与 crate 依赖图

14 个 crate 严格分三层，依赖**只能向下**：

```
                    ┌─────────────────────────────────────────────┐
                    │             tauri-app（入口层）              │
                    │  AppState + 88 命令 + Channel 事件桥接       │
                    └─────────────────────────────────────────────┘
            ┌───────────┬──────────┬──────────┬──────────┬────────────┐
            ▼           ▼          ▼          ▼          ▼            ▼
       app-pipeline  app-agent  app-meta  app-memory  app-conversation  app-logging
       (写作状态机)  (Agent运行时)(Meta+MVU)(归档/召回) (对话树)        (日志)
            │           │          │          │
            └─────┬─────┴────┬─────┴────┬─────┘
                  ▼          ▼          ▼
            infra-llm   infra-vector  infra-plugin-host
            (HTTP/SSE)  (向量存储)    (插件/MVU桩)
                  │          │          │
                  └────┬─────┴────┬─────┘
                       ▼          ▼
                   infra-import  infra-regex   infra-util
                   (PNG/JSON)    (regress)     (原子写/锁)
                       │          │              │
                       └────┬─────┴──────────────┘
                            ▼
                         domain（纯领域模型，零依赖，无 IO）
```

### 依赖关系实测表（来自各 crate 的 Cargo.toml）

| crate | 内部依赖 |
|-------|---------|
| `domain` | 无（纯模型，可被任何层引用） |
| `infra-util` | 无 |
| `infra-import` | domain |
| `infra-llm` | domain |
| `infra-vector` | domain, infra-util |
| `infra-regex` | domain |
| `infra-plugin-host` | domain, infra-util |
| `app-logging` | domain, infra-llm |
| `app-agent` | domain, infra-llm, infra-vector |
| `app-conversation` | domain, infra-util |
| `app-memory` | domain, infra-llm, infra-vector |
| `app-meta` | domain, infra-llm, infra-plugin-host, **app-agent** |
| `app-pipeline` | domain, infra-llm, **app-agent, app-conversation** |
| `tauri-app` | 上面除 infra-regex/infra-import... 等全部 app-* + infra-* |

### 分层规则

1. **domain 零 IO**：只定义 struct/enum/trait，不持有任何文件句柄、网络连接、tokio runtime。任何 crate 都可依赖它。
2. **infra-* 是基础设施**：封装外部世界（HTTP、文件、正则、向量计算），不包含业务编排逻辑。
3. **app-* 是应用层**：编排 infra 完成业务用例。app 之间可横向依赖（如 app-pipeline → app-agent）。
4. **tauri-app 是组装层**：唯一一个依赖几乎所有 crate 的入口，负责把命令参数翻译成 app 层调用，把 app 层返回翻译成 Tauri 序列化结果。

> ⚠️ **实测发现**：`infra-regex` 目前**没有任何 app/tauri crate 依赖它**（仅 domain 被它依赖的反向不成立——它依赖 domain）。正则引擎代码已就位但尚未接入流水线。`infra-import` 也只被 tauri-app 引用。

---

## 2. tauri-app：命令层与状态中枢

`crates/tauri-app/src/lib.rs`（3303 行）是整个 App 的组装层。它持有两类全局状态：

### 2.1 两类全局状态

**① 进程级单例 Store**（`OnceLock` 静态，以 `data/` 为根，各自持久化）：
| Store | 持久化文件 | 管什么 |
|-------|-----------|--------|
| `CharacterStore` | `characters.json` | 扁平角色卡 + 世界书 |
| `ConnectionStore` | `connections.json` | LLM 连接（含明文 key，桌面开发阶段） |
| `PresetStore` | `presets.json` | ST 预设（prompts + regex_scripts） |
| `CampaignStore` | 7 个文件（见 §4） | CharacterCard / Campaign / Instance / 知识 / 任务 / 摘要 / MVU |

**② `AppState`**（`tauri::State<Arc<AppState>>`，运行时内存状态）：
| 字段 | 类型 | 作用 |
|------|------|------|
| `conv_store` | ConversationStore | 对话树（每对话一个文件） |
| `log_store` | LogStore | 三类日志环形缓冲 |
| `tool_ctx` | `RwLock<ToolContext>` | 导演工具能查到的角色卡/世界书快照 |
| `current_cancel` | `Mutex<Option<Sender<bool>>>` | 当前写作的取消句柄 |
| `vector_store` | VectorStore | 向量库 |
| `active_llm` | `RwLock<Option<LlmClient>>` | 活跃 LLM 客户端 |
| `embed_config` | `RwLock<Option<EmbedConfig>>` | 嵌入 API 配置 |
| `active_campaign` | `RwLock<Option<Id>>` | 活跃游玩档 |
| `module_store` / `profile_store` | | 提示词模块 + Profile |
| `plugin_registry` | | 插件 |
| `meta_session` / `meta_conversations` | | Meta Agent 对话状态 |
| `meta_patches` | | 待采纳的 Patch |

### 2.2 88 个命令的分组速查

> 完整清单见 README。这里按「调用目标 + 副作用」归纳，重点是**谁会写文件、谁会推事件**。

| 分组 | 命令数 | 调谁 | 写什么 | 推事件？ |
|------|--------|------|--------|---------|
| 角色卡 + 世界书 CRUD | ~8 | CharacterStore + tool_ctx + vector_store | characters.json + vectors.json（绿灯条目） | 否 |
| LLM 连接 | 8 | ConnectionStore + AppState.active_llm | connections.json | 否 |
| **写作流水线** | 2 | PipelineOrchestrator | conversations/ + CampaignStore（后处理） | **是（Channel）** |
| 对话操作/变体 | 9 | ConversationStore | conversations/<id>.json | 否 |
| 日志 | 4 | LogStore | logs/*.jsonl | 否 |
| 记忆系统 | 3 | Embedder + MemoryArchiver | embed.json + vectors.json | 否 |
| Campaign + 变量 | 14 | CampaignStore | cards/campaigns/instances.json + active_campaign.json | 否 |
| 后处理产出/任务 | 6 | CampaignStore | knowledge/tasks/round_summaries.json | 否 |
| Meta Agent + MVU | 9 | AgentRuntime + MetaSession | mvu_translations.json | 否 |
| 插件 | 7 | PluginRegistry（带权限二次校验） | plugins.json | 否 |
| 预设 + 模块 | 13 | PresetStore + ModuleStore + ProfileStore | presets/custom_modules/profiles.json | 否 |
| 其它 | 1 | get_version | — | 否 |

### 2.3 关键事实：只有 2 个命令会推事件

**全项目没有任何 `app.emit()` / `AppHandle`。** 唯一向前端推流的方式是 `start_writing` / `regenerate` 接收的 `tauri::ipc::Channel<WritingEvent>`：

```rust
// tauri-app/src/lib.rs:1222
let (event_tx, mut event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
tokio::spawn(async move {
    while let Some(event) = event_rx.recv().await {
        let writing_event = WritingEvent::from_pipeline_event(&event);
        let _ = on_event_clone.send(writing_event);  // 推给前端 Channel
    }
});
```

其余 86 个命令**全部同步返回**，不推事件。前端拿结果靠 Promise。

---

## 3. 一次写作的完整调用链（核心章节）

以 `start_writing` 为例，从 Tauri 命令到落盘的完整数据流。`regenerate` 结构同构（见 §3.6）。

### 3.1 端到端时序

```
前端 Composer.vue ──@start-writing──▶ App.vue.startWriting()
  │  调 tauri-api.startWriting(intent, characterId, onMessage)
  ▼
[Tauri 命令] start_writing (lib.rs:1215)
  │  1. snapshot_tool_ctx()           ── 读 tool_ctx 快照（导入的角色卡/世界书）
  │  2. conv_store.create(char_id)    ── 新建对话（写 conversations/<id>.json）
  │  3. fill_profile_context()        ── 加载活跃 Profile + 启用模块
  │  4. fill_campaign_context()       ── 从活跃 Campaign 填 campaign_id/turn/tasks/story_clock
  │  5. 建 cancel watch channel，sender 存进 AppState.current_cancel
  │  6. spawn 事件转发任务（PipelineEvent → WritingEvent → Channel）
  ▼
PipelineOrchestrator.start_writing (app-pipeline/lib.rs:193)
  │
  ├─【Directing】make_director_config + register_director_tools
  │     ▼ runtime.run_tool_loop_streaming()  ── app-agent/runtime.rs:171
  │        │   tokio::select! { llm.chat_stream | cancel.wait_for }
  │        │   流式 token ──DirectorProgress 事件──▶ 前端
  │        │   completion_probe：content 已含合法 Plan JSON 则提早终止
  │     ▼ parse_plan_from_response()  ── 5 层兜底（见 §3.3）
  │     DirectorDone 事件
  │
  ├─【Delegating】spawn_subagents (app-agent/runtime.rs:327)
  │     │   并发上限 MAX_CONCURRENT_SUBAGENTS=4，超额任务直接丢弃（非排队）
  │     │   每个子 Agent tokio::spawn，clone cancel，非流式 run_tool_loop
  │     │   子 Agent 无工具（空 ToolRegistry），max_rounds=10
  │     ▼ SubagentStarted×N → SubagentDone×N（带 full_text）/ SubagentCancelled×N
  │     全部失败才 abort；单个失败继续不中断
  │
  ├─【Editing】make_editor_config（max_rounds=5）
  │     ▼ runtime.run_tool_loop_streaming()（空 ToolRegistry，无 probe）
  │        流式 token ──EditorProgress 事件──▶ 前端
  │     DraftReady { text } 事件
  │
  ├─【Review】build_provenance() + conv_store.append_ai_draft()
  │     │   写对话树：新 MessageNode + Draft variant + Provenance 溯源
  │     ▼
  ├─【Committed】存 self.session（写作会话快照，供 regenerate 用）
  │
  └─ 返回 (final_text, node_id, provenance)
       │
       ▼ 回到 Tauri 命令层 (lib.rs:1273)
  ── 有活跃 Campaign 才跑 ──
  PipelineOrchestrator.run_postprocess (app-pipeline/lib.rs:461)
     ▼ app-agent::run_postprocess_pipeline() (pipeline_postprocess.rs:39)
        tokio::join! 并发跑两个子任务（任一失败不短路，best-effort）：
        ├─ run_summarizer()    ── 本轮摘要 200-500 字 ── SummaryDone 事件
        └─ run_postprocess()   ── 知识/变量/任务三合一 ── PostProcessDone 事件
     ▼ persist_postprocess_outcome() (lib.rs:1302)
        写 CampaignStore：round_summaries.json / knowledge.json / instances.json(变量) / tasks.json
```

### 3.2 状态机（PipelineState，domain/agent.rs:151）

```
Idle ──start_writing──▶ Directing ──▶ Delegating ──▶ Editing ──▶ Review ──▶ Committed
                                    （子Agent并行）  （编剧）   （写对话树）
   任何阶段失败/取消 ──────────────────────────────────▶ Aborted
```

每次状态变更后紧跟 `event_tx.send(PipelineEvent::StateChanged)`，前端据此刷新 UI。

> ⚠️ **实测不对称**：`regenerate` 的三条路径终态停在 **Review**，**不进入 Committed**（`Committed` 仅在首写路径 `start_writing:425` 出现）。详见 §3.6。

### 3.3 Plan 解析的 5 层兜底（parse_plan_from_response，lib.rs:1122）

LLM 输出不稳定，所以解析层层降级。这套模式在角色识别/后处理/MVU 分析里**重复出现**（各自独立实现 `match_braces`，不跨文件复用）：

1. `emit_plan` 工具调用的 arguments
2. 整个 content 是合法 JSON
3. ` ```json ... ``` ` 围栏代码块
4. 裸 ` ``` ... ``` ` 代码块
5. 手写括号配平（从每个 `{` 计数深度，考虑字符串转义，到配平的 `}`）—— **完全不用正则**（regress 对多字节 UTF-8 range 有坑）

`parse_plan_json` 宽松：缺 `subagent_tasks` 容忍，每条 task 缺字段给默认值。

### 3.4 取消机制（单一事实源：watch channel）

```
前端 cancelWriting() ──▶ Tauri cancel_writing (lib.rs:1476)
                            │ 锁 AppState.current_cancel，tx.send(true)
                            ▼
         一个 Sender ──clone──▶ 多个 Receiver：
            ├─ 导演 run_tool_loop_streaming  （select! 与 chat_stream 竞速）
            ├─ 编剧 run_tool_loop_streaming  （同上）
            ├─ 子 Agent×N run_tool_loop      （每轮前 + LLM 调用竞速）
            └─ 后处理 run_summarizer / run_postprocess（各 clone 一份）
```

> ⚠️ **已知坑**（测试注释强调，lib.rs:1364）：watch **sender 必须保活到流水线结束**，sender 提前 drop 会让所有 `cancel.wait_for()` 立即 ready 误触发取消。`start_writing` 结尾显式 `*slot = None` 清理。

### 3.5 Channel 事件全序列（按 emit 顺序）

```
started → state_changed(Directing) → director_started →
director_progress×N（流式）→ director_done →
state_changed(Delegating) → subagent_started×N →
  subagent_done/subagent_cancelled×N →
state_changed(Editing) → editor_started → editor_progress×N →
draft_ready → state_changed(Review) → state_changed(Committed)
```

后处理阶段额外：`postprocess_started → summary_done → postprocess_done`（或 `postprocess_failed`）。

> ⚠️ **死代码警告**：`PipelineEvent::SubagentProgress` 在 domain 层有定义，但 `spawn_subagents` 走非流式 `run_tool_loop`，**首写和重 roll 都不会 emit**。前端 `App.vue:475` 有对应 case 但永不触发。

### 3.6 regenerate 的三条路径（app-pipeline/lib.rs:538）

`RegenerateRequest { targets, hint, seed }`，先经 `conv_store.validate_partial_roll` 校验，按 target 分流：

| 路径 | 触发条件 | 导演 | 子 Agent | 编剧 | 复用什么 | 落盘方式 |
|------|---------|------|---------|------|---------|---------|
| A 整体重 roll | targets 空/含 Director | ✅重跑 | ✅全重跑 | ✅ | 旧 scene_brief 作 intent | `add_variant`（新分支） |
| B 只重编剧 | targets 全是 Editor | ❌ | ❌ | ✅ | 旧 plan + 旧全部子产出 | `add_variant` |
| C 只重某子 | targets 含某 Subagent(id) | ❌ | ✅仅目标 | ✅ | 旧 plan + 其他角色旧产出 | `add_variant` |

**关键差异**：
- 重 roll 用 `add_variant`（在同 node 加新 variant，分支保留旧版），首写用 `append_ai_draft`（新 node）。
- hint 注入：A 注入导演 user + 编剧 user；B/C 只注入编剧 user；C 额外注入目标子 Agent 的 system prompt。
- 重 roll **不更新** `self.session`，**不进 Committed**（停在 Review）。
- 设计约束（已在 validate_partial_roll 落地）：**拒绝「只重导演却保留旧子产出」**——Plan 变了旧子产出不匹配，后端拦截。

---

## 4. 数据持久化总表

所有写盘点。根目录是 Tauri 的 `app_data_dir()`，开发期即项目下的 `data/`。

| 文件 | 写入者（crate/结构） | 内容 |
|------|---------------------|------|
| `characters.json` | CharacterStore (tauri-app/storage.rs) | 扁平角色卡 + 世界书（含路由/depth/is_global） |
| `conversations/<id>.json` | ConversationStore (app-conversation) | 每对话一个文件：对话树（nodes/variants/provenance） |
| `vectors.json` | BruteForceStore (infra-vector) | 向量记录（世界书绿灯条目 + 归档摘要 + 角色知识，带 metadata 标签） |
| `connections.json` | ConnectionStore (tauri-app/connection_store.rs) | LLM 连接（**明文 key**，桌面开发阶段） |
| `embed.json` | `save_embed_config` (tauri-app/lib.rs:77) | 嵌入 API 配置（endpoint/key/model/dim） |
| `active_campaign.json` | `save_active_campaign` (tauri-app/lib.rs) | 活跃游玩档 id |
| `presets.json` | PresetStore (tauri-app/preset_store.rs) | ST 预设（prompts + regex_scripts） |
| `custom_modules.json` | ModuleStore (tauri-app/module_store.rs) | 用户自建提示词模块 |
| `profiles.json` | ProfileStore | 提示词 Profile（模块绑定组合） |
| `plugins.json` | PluginRegistry (infra-plugin-host) | 已安装插件 manifest + enabled |
| `logs/<date>.jsonl` | LogStore (app-logging) | ERROR + LLM 调用日志（按日落盘） |
| **CampaignStore 7 文件** (tauri-app/campaign_store.rs) | | |
| ├ `cards.json` | CampaignStore | CharacterCard（含 character_definitions，树形） |
| ├ `campaigns.json` | CampaignStore | Campaign（游玩档，含 fork_from/story_clock/变量） |
| ├ `instances.json` | CampaignStore | CharacterInstance（角色实例，按 campaign_id 索引） |
| ├ `knowledge.json` | CampaignStore | CharacterKnowledgeEntry（角色可见信息，四元分类） |
| ├ `tasks.json` | CampaignStore | StoryTask（叙事计划任务） |
| ├ `round_summaries.json` | CampaignStore | RoundSummary（本轮剧情摘要，每轮一条） |
| └ `mvu_translations.json` | CampaignStore | StoredMvuTranslation（MVU 五合一产物） |

**级联删除规则**：删 CharacterCard → 同步删其所有 Campaign → 删 Campaign → 同步删其 instances/knowledge/tasks/summaries；删扁平 Character → 同步删其 mvu_translations。

**原子写惯例**（infra-util 统一）：先写 `.tmp` 再 `rename`，避免崩溃写一半。

---

## 5. 前端架构

### 5.1 组件树（15 个 .vue）

```
App.vue（主应用，~600 行）
├── AppHeader.vue            # 顶栏：角色名/导入/列表/Campaign/预设/插件/Meta/主题/高玩切换
├── ChatMessage.vue          # 单条消息：编辑/采纳/删除/分支 + 重roll菜单 + 内联编辑
├── Composer.vue             # 输入栏，emit @start-writing
├── PipelinePanel.vue        # 流水线状态：导演/编剧流式输出 + 子Agent产出折叠
├── LogPanel.vue             # 日志面板（高玩模式，Backend/Frontend tab）
├── AgentConfigCard.vue      # Agent 配置卡片（高玩模式）
├── MvuStatusBar.vue         # MVU 状态栏原生渲染（bar/text/tag/icon，零 JS）
│
├── [弹层组件，由 App.vue 的 show* 开关控制]
│   ├── CharacterList.vue    # 角色列表（切换/删除）
│   ├── CharacterDetail.vue  # 角色详情 + 世界书路由编辑 + 挂载 MvuStatusBar
│   ├── ConnectionConfig.vue # LLM 连接管理（创建/删除/切换/测试/拉模型）
│   ├── CampaignPanel.vue    # Campaign 管理（3 tab：角色卡/游玩档/档详情）
│   ├── PresetPanel.vue      # 预设查看/编辑（提示词/正则双 tab）
│   ├── PluginPanel.vue      # 插件管理
│   ├── MetaPanel.vue        # Meta Agent 对话框 + Patch 卡片 + MVU 分析
│   └── PluginHost.vue       # iframe 沙箱宿主（postMessage 桥）
│
└── tauri-api.js             # Tauri IPC 桥（82 个导出函数，覆盖 88 命令）
```

### 5.2 事件流（写作场景）

```
Composer @start-writing
   ▼
App.vue.startWriting()
   ▼ 调 tauri-api.startWriting(intent, charId, onMessage)
   │  invoke('start_writing', { ..., onEvent: new Channel() })
   │  Channel.onMessage = handleWritingEvent
   ▼
handleWritingEvent(event)  ── App.vue:444 switch(event.event_type)
   ├─ director_started/progress/done  → PipelinePanel 显示流式输出
   ├─ subagent_started/done/cancelled → PipelinePanel 子Agent卡片
   ├─ editor_started/progress          → PipelinePanel 编剧流式
   ├─ draft_ready                      → 成文进消息列表
   ├─ postprocess_done/summary_done    → 可选提示
   ├─ state_changed                    → 流水线状态机刷新
   └─ error                            → 错误提示 + 回滚用户消息
```

**其余命令**（非写作）走标准 `invoke(cmd, args).then()`，无事件订阅。组件通过 emit 把操作向上冒泡到 App.vue 统一处理（如 `@reroll`/`@edit-variant`/`@accept-variant`）。

### 5.3 高玩 / 普通双视图

`powerMode` 布尔（AppHeader 切换，localStorage 记忆）。开启后才显示：PipelinePanel / LogPanel / AgentConfigCard / MetaPanel 入口 / 预设编辑等。普通视图只暴露写作主流程。

---

## 6. 关键设计模式（代码层面）

### 6.1 Agent 运行时（app-agent/runtime.rs）

两个核心函数，被所有 Agent 复用：

- **`run_tool_loop`**（非流式）：固定 max_rounds 循环「调 LLM → 若有 tool_calls 执行工具 → 把结果塞回 → 再调 LLM」，直到无 tool_calls 或超轮次。带 **drift recovery**（超轮次时注入 reminder 再来一轮）。
- **`run_tool_loop_streaming`**（流式）：同上但 LLM 走 SSE，token 边收边推；额外参数 `completion_probe`（闭包，探测 content 是否已是最终结果提早终止，避免 drift recovery 把已完成输出逼进死循环）。

所有 Agent（导演/编剧/角色识别/后处理/总结/Meta/MVU）都是「配置 system prompt + 注册工具 + 调这两个函数之一 + 5 层兜底解析输出」的固定套路。

### 6.2 「5 层兜底解析」模式

LLM 输出 JSON 经常不规范（包了自然语言、用围栏代码块、字段缺失）。项目里 **4 处独立实现**了几乎相同的 5 层解析（Plan / character_definitions / postprocess / mvu_translation）：

```
工具调用 arguments → 整体 JSON → ```json 块 → 裸 ``` 块 → 手写括号配平
```

各自独立实现 `match_braces`（括号配平 + 字符串转义），**刻意不跨文件复用**（避免耦合，文档 HANDOFF §7.9 明确记录此决策）。

### 6.3 后处理并行（pipeline_postprocess.rs:39）

编剧成文后，**单 task 内** `tokio::join!`（非 try_join!）并发跑：
- `run_summarizer`：本轮摘要（200-500 字，独立 Agent，纯文本输出）
- `run_postprocess`：知识抽取 + 变量更新 + 任务推进（三合一 Agent，JSON 输出）

注释说明：用 `join!` 而非 `try_join!` 是为 **best-effort 互不影响**——任一 Err 不短路另一个。两个子任务各自 clone cancel。

### 6.4 ToolContext 快照

`AppState.tool_ctx` 是 `RwLock`。每次 `start_writing` 先 `snapshot_tool_ctx()` 克隆一份不可变快照塞进 `WritingContext`，保证一次写作内导演工具看到的数据恒定（不会写到一半被导入新卡影响）。导入卡/改世界书时同步更新 `tool_ctx` + 重建（`rebuild_world_info_in_tool_ctx`，含全局条目跨卡 merge）。

---

## 7. 已知架构债务 / 待办（代码视角）

> 完整待办见 HANDOFF §7，这里只列**架构层面**值得注意的。

| 项 | 现状 | 影响 |
|----|------|------|
| `SubagentProgress` 死代码 | domain 定义了，runtime 走非流式不 emit | 前端 case 永不触发，无害但误导 |
| regenerate 不进 Committed | 三路径终态停 Review | `self.session` 不刷新；语义上「重 roll 的结果」与「首写」落盘状态不同 |
| `infra-regex` 未接入 | 代码就位但无 app/tauri 依赖 | 正则脚本能力未生效（预设里的 regex_scripts 仅存储不执行） |
| API key 明文 | connections.json / embed.json 明文存 | 桌面开发期可接受，Android 需 infra-secrets + Keystore |
| `match_braces` 2 处重复 | `llm_parse.rs:21`（公用）+ `character_extractor.rs:252`（自写） | 算法一致，可合并；非刻意设计 |
| 共享 WebView 是桩 | `StubMvuRuntime` 全 NotImplemented | 重 DOM 卡（缄默之秋1.4 类）的 MVU 无法真实执行 |
| `plugin_get_variable` 权限 | 校验的是 `WriteVariables` 而非读权限 | 代码现状，文档已标注 |

---

## 8. 如何读这份代码（建议路径）

1. **从 `domain/` 开始**：`agent.rs`（状态机+事件）/ `character.rs`（树形模型）/ `conversation.rs`（对话树）是理解一切的基石，且无 IO 干扰。
2. **看 `app-pipeline/src/lib.rs`**：1492 行，一次写作的完整编排都在这。结合本文 §3 对照读。
3. **看 `app-agent/src/runtime.rs`**：理解 Agent 怎么跑（工具循环 + 流式 + 取消）。
4. **看 `tauri-app/src/lib.rs` 的 `start_writing`**（1215 行起）：理解命令层怎么组装上下文、桥接事件、触发后处理。
5. **改 prompt 时**：直接看 [AGENT_INTERFACES.md](AGENT_INTERFACES.md)，它索引了每个 Agent 的 prompt 文件位置。
6. **改某个命令时**：在 `tauri-app/src/lib.rs` 搜 `#[tauri::command]` + 命令名，对照本文 §2.2 的分组表。

---

*最后同步：2026-06-16。本文基于代码实测（依赖图来自 Cargo.toml，命令映射来自 lib.rs 逐函数梳理，调用链来自 app-pipeline/runtime 逐行确认，持久化表来自各 store 源码）。代码演进后请同步本文。*

# StoryForge 代码架构总览

> 本文档描述**代码实现层面**的架构，与代码同步。需求决策看 [INTENT.md](INTENT.md)，设计方案看 [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md)，改 prompt 看 [AGENT_INTERFACES.md](AGENT_INTERFACES.md)，进度状态看 [HANDOFF.md](HANDOFF.md)。
>
> 体量参考：Rust 14 crate + 前端，约 84 个源文件 / 2.4 万行；89 个 Tauri 命令；测试数见 HANDOFF.md。

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

### 2.2 89 个命令的分组速查

> 完整清单见 README。这里按「调用目标 + 副作用」归纳，重点是**谁会写文件、谁会推事件**。

| 分组 | 命令数 | 调谁 | 写什么 | 推事件？ |
|------|--------|------|--------|---------|
| 角色卡 + 世界书 CRUD | ~8 | CharacterStore + tool_ctx + vector_store | characters.json + vectors.json（绿灯条目） | 否 |
| LLM 连接 | 8 | ConnectionStore + AppState.active_llm | connections.json | 否 |
| **写作流水线** | 2 | PipelineOrchestrator | conversations/ + CampaignStore（后处理） | **是（Channel）** |
| 对话操作/变体 | 10 | ConversationStore（delete_message_from 截断对话） | conversations/<id>.json | 否 |
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
  │  3. append_final_message(开场白)  ── 角色卡 first_mes → Assistant/Final node
  │  4. append_user_message(user意图) ── 用户输入 → User/Final node
  │  5. fill_profile_context()        ── 加载活跃 Profile + 启用模块
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
  ├─【Delegating】spawn_subagents (app-agent/runtime.rs:332)
  │     │   并发上限 MAX_CONCURRENT_SUBAGENTS=4，用 Semaphore 限流（超额任务排队，全部跑完）
  │     │   每个子 Agent tokio::spawn，clone cancel，流式 run_tool_loop_streaming
  │     │   子 Agent 无工具（空 ToolRegistry），max_rounds=10
  │     │   每个 spawn 闭包内建 per-subagent channel，token delta 包成
  │     │   SubagentProgress（带 character_id + index）转发到主 event_tx
  │     ▼ SubagentStarted×N → SubagentProgress×N（流式 token）→ SubagentDone×N（带 full_text）/ SubagentCancelled×N
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
            ├─ 子 Agent×N run_tool_loop_streaming（每轮前 + LLM 调用竞速；token 经 SubagentProgress 推前端）
            └─ 后处理 run_summarizer / run_postprocess（各 clone 一份）
```

> ⚠️ **已知坑**（测试注释强调，lib.rs:1364）：watch **sender 必须保活到流水线结束**，sender 提前 drop 会让所有 `cancel.wait_for()` 立即 ready 误触发取消。`start_writing` 结尾显式 `*slot = None` 清理。

### 3.5 Channel 事件全序列（按 emit 顺序）

```
started → state_changed(Directing) → director_started →
director_progress×N（流式）→ director_done →
state_changed(Delegating) → subagent_started×N →
  subagent_progress×N（流式 token，每个子 Agent 独立）→
  subagent_done/subagent_cancelled×N →
state_changed(Editing) → editor_started → editor_progress×N →
draft_ready → state_changed(Review) → state_changed(Committed)
```

后处理阶段额外：`postprocess_started → summary_done → postprocess_done`（或 `postprocess_failed`）。

> 子 Agent 流式：`SubagentProgress` 现在由 `spawn_subagents` 内每个子 Agent 的 per-subagent channel 转发生成（带 character_id + index），前端 PipelinePanel 实时显示各角色 token。regenerate 路径 C（只重某子 Agent）同样流式。

### 3.6 regenerate 的三条路径（app-pipeline/lib.rs:538）

`RegenerateRequest { targets, hint, seed }`，先经 `conv_store.validate_partial_roll` 校验，按 target 分流：

| 路径 | 触发条件 | 导演 | 子 Agent | 编剧 | 复用什么 | 落盘方式 |
|------|---------|------|---------|------|---------|---------|
| A 整体重 roll | targets 空/含 Director | ✅重跑 | ✅全重跑 | ✅ | 旧 scene_brief 作 intent | 见下方分叉规则 |
| B 只重编剧 | targets 全是 Editor | ❌ | ❌ | ✅ | 旧 plan + 旧全部子产出 | 见下方分叉规则 |
| C 只重某子 | targets 含某 Subagent(id) | ❌ | ✅仅目标（流式） | ✅ | 旧 plan + 其他角色旧产出 | 见下方分叉规则 |

**落库分叉规则**（app-pipeline/lib.rs regenerate 末尾，regenerate 唯一落库点）：
- 重 roll **最后一条 AI 消息**（`is_last_assistant_node` 判定 `nodes.last()` 是 Assistant 且 id 匹配）→ `replace_active_variant`：旧 active 降级 Discarded + push 新 active（原地替换，避免分支累积，旧版可 switch 切回查看）
- 重 roll **中间消息** → `add_variant`：开分支保留旧版（原行为）
- 后端按 `nodes.last()` 实时判定（单一事实源），重 roll 后若又发新消息使原 node 不再最后，下次重 roll 它自动回退到开分支。

**关键差异**：
- 首写用 `append_ai_draft`（新 node），重 roll 在既有 node 上 replace 或 add_variant。
- hint 注入：A 注入导演 user + 编剧 user；B/C 只注入编剧 user；C 额外注入目标子 Agent 的 system prompt。
- 重 roll **不更新** `self.session`，**不进 Committed**（停在 Review）。
- 设计约束（已在 validate_partial_roll 落地）：**拒绝「只重导演却保留旧子产出」**——Plan 变了旧子产出不匹配，后端拦截。

#### 3.6.1 删除消息（truncate 语义）

「🗑 删除」按钮 = **撤销从这条开始的写作**（不是只软删一个 variant）：

- 后端 `delete_message_from` 命令 → `ConversationStore::truncate_from`：删除指定 node 及其后所有 node，保留之前的。
- 前端删除后：重新拉对话刷新 + 清流水线状态（导演/子Agent/编剧输出全置 idle，隐藏 pipeline 面板）。
- **边界**：`start_writing` 不存开场白/用户意图进后端对话（只 append AI 成文 node）。所以删除成文（唯一 node）后对话空，前端检测空对话时从角色卡 `first_mes` **回显开场白**，回到「导入后未写作」状态。
- 确认框用 `@tauri-apps/plugin-dialog` 的 `ask`（Tauri WebView 的 `window.confirm` 不弹窗）。

### 3.7 Meta Agent 子系统（独立于写作流水线）

Meta Agent 是一个**配置调试助手**，与写作流水线完全解耦：不读 `current_cancel`、不碰 pipeline、不接触 API key，自建 `AgentRuntime` + 每轮新建 `ToolRegistry`。它的能力分三块：诊断对话、Patch 提议-采纳、MVU 五合一分析。

#### 数据模型与状态

```
AppState（tauri-app/lib.rs:128）
├── meta_session: Arc<MetaSession>          # 跨工具调用共享的诊断数据源
│   ├── character: Mutex<Option<Arc<Character>>>
│   ├── world_info: Mutex<Option<Arc<WorldInfoBook>>>
│   └── patches: PatchStore { patches: RwLock<Vec<Patch>> }
├── meta_conversations: Mutex<HashMap<String, MetaConversation>>  # 内存态，重启清空
└── meta_patches: Arc<RwLock<Vec<Patch>>>   # 前端可见的待采纳 patch（独立于 session.patches）
```

> ⚠️ **两套 patch 存储**：`MetaSession.patches`（app-meta 内部）与 `AppState.meta_patches`（tauri 侧）是两份。`meta_chat` 结束会把 session 新 patch 克隆进 AppState。`meta_accept_patch`/`meta_dismiss_patch` 只改 AppState.meta_patches，**不反向同步回 session**——目前无害（session 是临时态），但属潜在漂移点。

- `MetaConversation`：`messages: Vec<MetaMessage>` + `history_summary: Vec<String>`（手动截断的逐轮摘要，每轮 `"用户：{200字}\n助手：{300字}"`，保留最近 6 轮，喂下一轮 LLM）。
- `MetaMessage`：`User{content}` | `Agent{content, tool_result: Option<ToolResultDisplay>}`。

#### meta_chat 调用链（流式）

```
前端 MetaPanel.vue handleSend
  │  先 push 空 agent 消息占位（流式累积用）
  ▼ metaChat(convId, text, onDelta)
[Tauri] meta_chat (lib.rs:2431)
  │  1. sync_meta_session_from_tool_ctx（把 tool_ctx 最后一张卡 + 世界书同步进 MetaSession）
  │  2. 取出对话（不存在则报错，不静默创建）
  │  3. 建 mpsc channel + spawn 转发任务：progress_tx 的 delta → MetaStreamEvent::progress → 前端 Channel
  ▼
app_meta::chat (meta_conversation.rs:146)
  │  make_meta_agent_config（role=Meta, max_rounds=8）+ build_meta_user_msg（拼 history_summary）
  │  register_meta_runtime_tools（挂接 inspect/propose 工具，handler 读写 MetaSession）
  ▼ runtime.run_tool_loop_streaming(..., progress_tx, None)
  │     流式 token ──meta_progress 事件──▶ 前端累积到回复气泡
  │     工具副作用在循环内由 handler 完成（meta_propose_patch → session.patches.propose）
  ▼ 返回 MetaTurn { agent_message, new_patch }
  │  新 patch 同步进 AppState.meta_patches；对话存回 meta_conversations
  ▼ 命令返回 { conversation_id, agent_message, messages, new_patch }
前端：用 agent_message.content 校正气泡（流式累积可能有中间文本）；new_patch 存在则 refreshPatches
```

**关键**：增量走 Channel（`meta_progress`），最终态走命令返回值——与 `start_writing` 的 `{text, conversation_id, node_id}` 模式一致。

#### Meta Agent 工具集（register_meta_runtime_tools，meta_conversation.rs:240）

| 工具 | handler 做什么 | 返回 |
|------|--------------|------|
| `meta_inspect_world_info` | 读 session.world_info，检查蓝灯关键词冲突 + 孤立条目 | `WorldInfoReport` |
| `meta_inspect_character` | 读 session.character，检查字段非空 + first_mes 占位符 | `CardReport` |
| `meta_propose_patch` | **写**：`session.patches.propose(desc, actions)` | `{patch_id, ...}` |
| `meta_classify_st_preset` | 占位 handler（未挂接）；实际 ST 分类走独立命令 | — |

#### Patch 系统（提议-采纳两阶段）

```
LLM propose ──▶ PatchStore.propose（生成 uuid, applied=false）
                   │  同步进 AppState.meta_patches（前端可见）
                   ▼
用户采纳 ──▶ meta_accept_patch 命令（lib.rs:2296）
                   │  execute_patch（app-meta 纯函数，传 PatchContext）
                   │  ① 改 tool_ctx 内存世界书
                   │  ② 持久化 CharacterStore：全局条目写回所有卡 + 非全局按 content 指纹匹配回原卡
                   │  ③ patch.applied = true
                   ▼
用户忽略 ──▶ meta_dismiss_patch（retain 移除）
```

- `PatchAction`：`Create{target,data}` | `Update{target,field,value}` | `Delete{target}`。`target` 格式 `world_info[0]` / `character.personality`。`execute_patch` 当前只支持 `world_info` / `character` 两种 kind。
- 历史 bug（已修）：曾把合并世界书写回最后一张卡 + `is_global` 硬编码 false，导致污染。

#### MVU 五合一分析（mvu_import.rs:41）

手动触发（D44：用户在 MetaPanel 点「分析状态栏」才跑，不自动）：

```
meta_analyze_mvu_card 命令
  ▼
analyze_mvu_card 编排：
  1. score_card_complexity（纯 Rust 启发式，基于 document./innerHTML/script 等阈值）
     → CardComplexityReport { classification: Heavy | RuleDriven | PureData }
  2. extract_mvu_schema_from_extensions（P1 字段级 schema）
  3. 纯数据短路：PureData && 无字段 → pure_data_fallback，省 LLM 调用
  4. LLM 五合一（run_tool_loop 非流式，emit_mvu_translation 工具）
  5. parse_mvu_translation_from_response（5 层兜底，同 §6.2 模式）
  6. 失败降级 → pure_data_fallback（不报错不阻塞）
  ▼ 持久化 StoredMvuTranslation → data/mvu_translations.json（按 source_character_id 去重）
```

**MvuTranslation 5 产物**：`variable_schema`（变量定义）/ `ui_bindings`（UI 元素→变量，BindingDisplay: bar/text/tag/icon）/ `update_rules`（注入后处理 Agent）/ `interactions`（用户点击→动作）/ `fallback_fragments`（需 WebView 的片段）。`routing`：native（原生协议层足够）/ hybrid（需共享 WebView）；LLM 标 native 却有 fallback_fragments 会自动纠正为 hybrid。

**注**：共享 WebView 真实 JS 执行仍是桩（`StubMvuRuntime` 全 NotImplemented），重 DOM 卡（缄默之秋1.4 类）的 fallback_fragments 无法执行——这是已知限制（见 §8 债务表）。

#### 9 个 Meta/MVU 命令速查

| 命令 | 作用 | 推事件？ |
|------|------|---------|
| `meta_start_conversation` | 新建 MetaConversation（uuid） | 否 |
| `meta_chat` | 跑一轮流式对话 | **是**（meta_progress） |
| `meta_get_conversation` | 取对话历史 | 否 |
| `meta_list_pending_patches` | 列待采纳 patch | 否 |
| `meta_dismiss_patch` | 忽略 patch | 否 |
| `meta_accept_patch` | 执行 patch（改世界书 + 写 CharacterStore） | 否 |
| `meta_analyze_mvu_card` | MVU 五合一分析（手动触发） | 否 |
| `meta_list_mvu_translations` / `meta_get_mvu_translation` | 查 MVU 翻译 | 否 |
| `meta_classify_st_preset` | ST 预设 LLM 分类（不持久化） | 否 |

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

**级联删除规则**：删扁平 Character → 同步删 CampaignStore 的 CharacterCard（按 source_character_id）+ 删 MVU 翻译；删 CharacterCard → 同步删其所有 Campaign → 删 Campaign → 同步删其 instances/knowledge/tasks/summaries。

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
| regenerate 不进 Committed | 三路径终态停 Review | `self.session` 不刷新；语义上「重 roll 的结果」与「首写」落盘状态不同 |
| 子 Agent 并发从「丢弃」改「排队」 | Semaphore(MAX=4) 限流，超额任务全部排队跑完 | 行为变更：角色很多时总耗时变长（曾确认接受）；不再有 SubagentFailed 占位 |
| `infra-regex` 未接入 | 代码就位但无 app/tauri 依赖 | 正则脚本能力未生效（预设里的 regex_scripts 仅存储不执行） |
| API key 明文 | connections.json / embed.json 明文存 | 桌面开发期可接受，Android 需 infra-secrets + Keystore |
| `match_braces` 2 处重复 | `llm_parse.rs:21`（公用）+ `character_extractor.rs:252`（自写） | 算法一致，可合并；非刻意设计 |
| 共享 WebView 是桩 | `StubMvuRuntime` 全 NotImplemented | 重 DOM 卡（缄默之秋1.4 类）的 MVU 无法真实执行 |
| `plugin_get_variable` 权限 | 校验的是 `WriteVariables` 而非读权限 | 代码现状，文档已标注 |
| **CharacterStore/CampaignStore 双数据源** | 导入只写 CharacterStore（扁平 Character），Campaign 面板读 CampaignStore（CharacterCard）需 extract_characters 转换；删卡现已级联但两套数据天然易不一致 | 🔴 高：应统一（废弃 CharacterStore 或后者作缓存层）；见 HANDOFF §3.11 |
| tauri-api.js 参数名无校验 | 曾系统性出现 14 处 snake_case 参数名（Tauri v2 期望 camelCase）；手动修易漏 | 应加脚本静态检查 invoke 参数名 vs Rust 命令签名 |
| ~~start_writing 不存开场白/用户意图进对话~~ | ✅ 已修复：start_writing 创建对话后先 append 开场白 + user 意图，对话结构变为 [开场白, user意图, AI成文] | — |
| Campaign.fork 未暴露 | `Campaign::fork` domain 方法已实现（fork_from 记分叉点），但无 Tauri 命令 + 前端入口；「分支」按钮只弹提示 | 真「分支=开新档」能力未接通 |

---

## 8. 如何读这份代码（建议路径）

1. **从 `domain/` 开始**：`agent.rs`（状态机+事件）/ `character.rs`（树形模型）/ `conversation.rs`（对话树）是理解一切的基石，且无 IO 干扰。
2. **看 `app-pipeline/src/lib.rs`**：1492 行，一次写作的完整编排都在这。结合本文 §3 对照读。
3. **看 `app-agent/src/runtime.rs`**：理解 Agent 怎么跑（工具循环 + 流式 + 取消）。
4. **看 `tauri-app/src/lib.rs` 的 `start_writing`**（1215 行起）：理解命令层怎么组装上下文、桥接事件、触发后处理。
5. **改 prompt 时**：直接看 [AGENT_INTERFACES.md](AGENT_INTERFACES.md)，它索引了每个 Agent 的 prompt 文件位置。
6. **改某个命令时**：在 `tauri-app/src/lib.rs` 搜 `#[tauri::command]` + 命令名，对照本文 §2.2 的分组表。

---

*最后同步：2026-06-16（对话数据完整性修复）。本文基于代码实测（依赖图来自 Cargo.toml，命令映射来自 lib.rs 逐函数梳理，调用链来自 app-pipeline/runtime 逐行确认，持久化表来自各 store 源码）。代码演进后请同步本文。*

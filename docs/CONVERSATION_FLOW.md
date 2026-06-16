# StoryForge 完整对话链路图

> 基于代码实测，非猜测。每个环节标注了源文件位置。

---

## 1. 首次写作（start_writing）

```
用户在 Composer.vue 输入意图，点发送
    │
    ▼
App.vue.startWriting(intent, skipLocalPush=false)     ← frontend/src/App.vue:277
    │  1. 本地 push user 消息到 messages.value（即时反馈，skipLocalPush=true 时跳过）
    │  2. 调 tauri-api.startWriting(intent, characterId, onEvent, conversationId)
    │     conversationId = currentConversationId.value（null = 首次，有值 = 追加到已有对话）
    │
    ▼
[Tauri 命令] start_writing                            ← tauri-app/src/lib.rs:1219
    │  参数: intent, character_id?, conversation_id?, state, on_event
    │
    │  ① snapshot_tool_ctx()                           ← 读 ToolContext 快照
    │     └─ characters: Vec<Character>                ← 导入的角色卡
    │     └─ world_info: Option<WorldInfoBook>         ← 世界书
    │
    │  ② conversation_id?
    │     ├─ Some(id) → 复用已有对话
    │     │   └─ append_user_message(intent)           ← 只追加 user 意图（开场白已在创建时存入）
    │     └─ None → 新建对话
    │         ├─ conv_store.create(character_id)
    │         ├─ append_final_message(Assistant, first_mes)  ← 开场白
    │         └─ append_user_message(intent)                  ← user 意图
    │
    │  ③ 构造 WritingContext:
    │     ├─ characters, world_info, conversation_id
    │     ├─ profile + modules  ← fill_profile_context()
    │     ├─ campaign_id / turn / pending_tasks / story_clock ← fill_campaign_context()
    │     └─ recent_messages ← conv_store.recent_messages_with_role(id, 20, None)
    │         最近 20 条带角色标签的对话（"用户: xxx" / "AI: xxx"）
    │
    │  ⑥ 建 cancel watch channel，sender 存 AppState.current_cancel
    │  ⑦ spawn 事件转发任务（PipelineEvent → WritingEvent → Channel → 前端）
    │
    ▼
PipelineOrchestrator.start_writing(intent, ctx)       ← app-pipeline/src/lib.rs:193
    │
    │
    ╔══════════════════════════════════════════════════════════════════╗
    ║  阶段 1：导演 Agent（Directing）                                ║
    ╚══════════════════════════════════════════════════════════════════╝
    │
    │  make_director_config(profile, modules)           ← app-pipeline/src/lib.rs:1107
    │     │
    │     │  system_prompt = assemble_system_prompt(    ← domain/src/prompt_module.rs:152
    │     │      role: Director,
    │     │      role_directive: DIRECTOR_SYSTEM_PROMPT, ← 硬编码："你是写作导演。用户给你写作意图..."
    │     │      profile,                               ← 用户选的 Profile（可选）
    │     │      modules,                               ← 已启用的提示词模块列表
    │     │      tool_directives: ""                    ← 导演无额外工具指令
    │     │  )
    │     │
    │     │  组装顺序：
    │     │  1. role_directive（导演基础指令）
    │     │  2. 按 category 优先级插入模块 content：
    │     │     Perspective → CoT → Style → Tone → Quality → Output
    │     │     每个模块检查 applicable_roles 是否包含 Director
    │     │
    │     └─ AgentConfig { role: Director, system_prompt, max_tool_rounds: 15, model: "deepseek-chat" }
    │
    │  register_director_tools(registry)                ← app-agent/src/tools.rs:102
    │     ├─ search_world_info(query)  ← 关键词搜索世界书条目
    │     ├─ get_character(name)       ← 获取角色卡详情
    │     └─ emit_plan(plan_json)      ← 输出结构化 Plan
    │
    │  build_director_user_msg(intent, ctx)             ← app-pipeline/src/lib.rs:1052
    │     │
    │     │  拼接内容：
    │     │  ┌─────────────────────────────────────────────────────┐
    │     │  │ "用户的写作意图：{intent}\n\n"                       │
    │     │  │ "可用角色：{char_names}\n\n"                         │
    │     │  │                                                     │
    │     │  │ "【最近对话历史】"              ← 新增               │
    │     │  │ "用户: 第1条消息\n"                                  │
    │     │  │ "AI: 第2条消息\n"                                    │
    │     │  │                                                     │
    │     │  │ "【世界设定（常驻）】"                                │
    │     │  │ "- {keys}：{content}"    ← 蓝灯 Constant/Both 条目  │
    │     │  │ "（以上常驻设定始终生效。绿灯条目可通过                │
    │     │  │  search_world_info / search_vectors 工具检索。）"     │
    │     │  │                                                     │
    │     │  │ "【叙事任务/伏笔】"                                  │
    │     │  │ "- {task_title}: {description}"  ← 触发条件满足的任务 │
    │     │  │ "（请在规划本场戏时考虑以上任务/伏笔。）"             │
    │     │  │                                                     │
    │     │  │ "请分析意图并输出 Plan。"                             │
    │     │  └─────────────────────────────────────────────────────┘
    │     │
    │     │  蓝灯条目排序：depth 升序 → order 升序
    │     │  （depth 小的排后面 = 更靠近 prompt 末尾 = 更受重视）
    │     │
    │     └─ 任务注入：render_tasks_for_injection() 过滤 Pending/Active 且触发满足的任务
    │        触发条件：TurnReminder(turn达标) / StoryTime(时钟匹配) / Event(关键词) / Manual
    │
    │  runtime.run_tool_loop_streaming(                 ← app-agent/src/runtime.rs:171
    │      config,
    │      user_message: director_user_msg,
    │      tool_registry: {search_world_info, get_character, emit_plan},
    │      cancel,
    │      progress_tx,           ← token delta → DirectorProgress 事件 → 前端 PipelinePanel
    │      completion_probe,      ← 探测 content 是否含合法 Plan JSON，是则提早终止
    │  )
    │     │
    │     │  LLM 调用链：
    │     │  messages = [system(director_system_prompt), user(director_user_msg)]
    │     │
    │     │  循环（最多 15 轮）：
    │     │  ├─ 调 LLM chat_stream（SSE 流式）
    │     │  ├─ token delta → progress_tx → 前端实时显示
    │     │  ├─ 有 tool_calls → 执行工具 → 结果追加到 messages → 继续
    │     │  ├─ 无 tool_calls + 有 content → completion_probe 探测
    │     │  │   ├─ 合法 Plan → 提早返回
    │     │  │   └─ 非 Plan → drift recovery（注入 reminder "请继续使用工具"）
    │     │  └─ 超 15 轮 → 报错
    │     │
    │     └─ 返回 ChatResponse { content, tool_calls }
    │
    │  parse_plan_from_response(resp)                   ← 5 层兜底解析
    │     1. tool_calls 中的 emit_plan 参数
    │     2. 整个 content 是合法 JSON
    │     3. ```json ... ``` 围栏代码块
    │     4. 裸 ``` ... ``` 代码块
    │     5. 手写括号配平（不用正则，regress 对多字节 UTF-8 有坑）
    │
    │  输出 Plan { scene_brief, subagent_tasks: [{character_id, brief}] }
    │
    │
    ╔══════════════════════════════════════════════════════════════════╗
    ║  阶段 2：子 Agent 并行（Delegating）                            ║
    ╚══════════════════════════════════════════════════════════════════╝
    │
    │  spawn_subagents(tasks, runtime, ...)              ← app-agent/src/runtime.rs:332
    │     │
    │     │  并发控制：Semaphore(MAX_CONCURRENT_SUBAGENTS=4)
    │     │  超出 4 个的任务排队等待，不丢弃
    │     │
    │     │  每个子 Agent（tokio::spawn）：
    │     │  ┌─────────────────────────────────────────────────────┐
    │     │  │ system_prompt = 格式化：                             │
    │     │  │   "{SUBAGENT_SYSTEM_PROMPT_TEMPLATE}\n\n"           │
    │     │  │   "你是角色 {character_id}。\n\n"                    │
    │     │  │   "{format_context_package(context_package)}\n\n"   │
    │     │  │   "{task.brief}"                                    │
    │     │  │                                                     │
    │     │  │ 其中 format_context_package(pkg) 拼接：              │
    │     │  │   "## 你的角色设定\n{character_brief}\n\n"           │
    │     │  │   "## 当前场景\n{scene_brief}\n\n"                  │
    │     │  │   "## 世界设定（常驻）\n{constant_lore}\n\n"         │
    │     │  │   "## 相关世界设定\n{relevant_lore}\n\n"             │
    │     │  │   "## 最近对话\n{recent_window}\n\n"                │
    │     │  │                                                     │
    │     │  │ user_message = context_package.task                  │
    │     │  │                                                     │
    │     │  │ tools = []（子 Agent 无工具，纯表演）                │
    │     │  │ max_tool_rounds = 10                                │
    │     │  │ model = 导演的 model（M1 简化）                     │
    │     │  └─────────────────────────────────────────────────────┘
    │     │
    │     │  run_tool_loop_streaming（无工具，纯文本输出）
    │     │  token delta → SubagentProgress 事件 → 前端 PipelinePanel
    │     │
    │     └─ 返回 Vec<Result<Performance, AgentError>>
    │        Performance { character_id, full_text, narrative, dialogue, inner_thoughts }
    │
    │  全部子 Agent 失败 → 报错中止
    │  部分失败 → 继续（不中断）
    │
    │
    ╔══════════════════════════════════════════════════════════════════╗
    ║  阶段 3：编剧 Agent（Editing）                                  ║
    ╚══════════════════════════════════════════════════════════════════╝
    │
    │  make_editor_config(profile, modules)              ← app-pipeline/src/lib.rs:1128
    │     │  system_prompt = assemble_system_prompt(
    │     │      role: Editor,
    │     │      role_directive: EDITOR_SYSTEM_PROMPT,   ← "你是编剧。收集所有子 Agent 的表演..."
    │     │      profile, modules, ""
    │     │  )
    │     └─ AgentConfig { role: Editor, max_tool_rounds: 5, model: "deepseek-chat", tools: [] }
    │
    │  构造编剧 user_message:
    │     ┌─────────────────────────────────────────────────────┐
    │     │ "场景：{plan.scene_brief}\n\n"                      │
    │     │ "子 Agent 表演：\n\n"                                │
    │     │ "### {character_id}\n{full_text}\n\n---\n\n"        │
    │     │ "### {character_id}\n{full_text}\n\n---\n\n"        │
    │     │ "请合并成连贯成文。"                                  │
    │     │                                                     │
    │     │ "【最近对话历史】"              ← 新增               │
    │     │ "用户: 第1条消息\n"                                  │
    │     │ "AI: 第2条消息\n"                                    │
    │     └─────────────────────────────────────────────────────┘
    │
    │  runtime.run_tool_loop_streaming(                  ← 无工具，直接输出成文
    │      config,
    │      user_message: editor_user_msg,
    │      tool_registry: {},                            ← 空，编剧不调工具
    │      cancel,
    │      progress_tx,                                  ← EditorProgress 事件 → 前端
    │      completion_probe: None,                       ← 无工具，不需要探测
    │  )
    │
    │  输出 final_text（成文 Markdown）
    │
    │
    ╔══════════════════════════════════════════════════════════════════╗
    ║  阶段 4：写入对话树（Review → Committed）                       ║
    ╚══════════════════════════════════════════════════════════════════╝
    │
    │  build_provenance(session_id, plan, performances, seed)  ← app-conversation/lib.rs:501
    │     └─ Provenance { session_id, plan, subagent_results, profile_id, seed, last_hint }
    │
    │  conv_store.append_ai_draft(conversation_id, final_text, provenance)
    │     └─ 写入对话树：新 MessageNode + Draft variant + Provenance 溯源
    │
    │  此时后端对话: [开场白, user意图, AI成文(Draft)]
    │
    │  state = Committed
    │
    │
    ╔══════════════════════════════════════════════════════════════════╗
    ║  阶段 5：后处理流水线（有 Campaign 才跑）                        ║
    ╚══════════════════════════════════════════════════════════════════╝
    │
    │  run_postprocess(final_text, present_chars, var_keys, ctx)  ← app-pipeline/src/lib.rs:462
    │     │
    │     │  tokio::join! 并发跑两个子任务（best-effort，任一失败不影响另一个）：
    │     │
    │     ├─ run_summarizer()                             ← app-agent/src/summarizer.rs
    │     │     system: SUMMARIZER_SYSTEM_PROMPT（200-500 字硬约束）
    │     │     user: final_text
    │     │     输出: 本轮剧情摘要（纯文本）
    │     │
    │     └─ run_postprocess()                            ← app-agent/src/postprocess.rs
    │           system: POSTPROCESS_SYSTEM_PROMPT（知识/变量/任务三合一）
    │           user: 成文 + 在场角色 + 变量键 + 轮次 + 时钟
    │           输出: JSON { knowledge_updates, variable_updates, task_updates }
    │           解析: 5 层兜底（同 Plan 解析模式）
    │
    │  persist_postprocess_outcome()                     ← tauri-app/src/lib.rs:1401
    │     ├─ round_summaries.json  ← 本轮摘要
    │     ├─ knowledge.json        ← 角色知识（四元分类）
    │     ├─ instances.json        ← 角色变量更新
    │     └─ tasks.json            ← 任务状态更新
    │
    │
    ╔══════════════════════════════════════════════════════════════════╗
    ║  回到前端                                                       ║
    ╚══════════════════════════════════════════════════════════════════╝
    │
    │  startWriting 收到 result { text, conversation_id, node_id }
    │  替换 editor-streaming 占位 → push AI 成文到 messages.value
    │  此时前端 messages: [开场白(本地), user(本地), AI成文(后端)]
    │
    │  后端对话: [开场白, user意图, AI成文(Draft)]
    │  （重启后 applyConversation 从后端加载，不丢数据）
```

---

## 2. 重 roll（assistant 消息）

```
用户点 AI 消息的「🔄 重roll」菜单
    │
    │  菜单选项：
    │  ├─ 整体重 roll（targets=[]）
    │  ├─ 只重编剧（targets=[Editor]）
    │  ├─ 只重某子 Agent（targets=[Subagent("角色名")]）
    │  └─ 可附 hint（可选，告诉 Agent 上次哪里有问题）
    │
    ▼
App.vue.handleReroll({messageId, kind, hint})          ← frontend/src/App.vue:347
    │  调 apiRegenerate({conversationId, nodeId, targets, hint}, onEvent)
    │
    ▼
[Tauri 命令] regenerate(req)                           ← tauri-app/src/lib.rs:1559
    │  解析 targets → Vec<PartialRollTarget>
    │  构造 WritingContext:
    │     └─ recent_messages ← conv_store.recent_messages_with_role(id, 20, Some(&node_id))
    │         before_node_id = 目标节点 → 排除该节点及之后的消息
    │         避免导演看到被重 roll 的旧 AI 回复
    │
    ▼
PipelineOrchestrator.regenerate(req, ctx)              ← app-pipeline/src/lib.rs:539
    │
    │  ① validate_partial_roll(conv_id, node_id, targets)
    │     └─ 检查：node 必须有 provenance
    │     └─ 检查：不能只重导演却保留旧子产出
    │
    │  ② 读旧 variant 的 Provenance
    │
    │  ③ 按 targets 分流：
    │
    │  ┌─────────────────────────────────────────────────────────┐
    │  │ 路径 A：整体重 roll（targets 空或含 Director）           │
    │  │   ├─ 导演重跑（intent = 旧 plan.scene_brief + hint）    │
    │  │   ├─ 子 Agent 全部重跑                                  │
    │  │   └─ 编剧重跑（注入 hint）                              │
    │  ├─────────────────────────────────────────────────────────┤
    │  │ 路径 B：只重编剧（targets=[Editor]）                    │
    │  │   ├─ 导演不跑（复用旧 plan）                            │
    │  │   ├─ 子 Agent 不跑（复用旧 performances）               │
    │  │   └─ 编剧重跑（注入 hint + 旧子产出）                   │
    │  ├─────────────────────────────────────────────────────────┤
    │  │ 路径 C：只重某子 Agent（targets=[Subagent(id)]）        │
    │  │   ├─ 导演不跑（复用旧 plan）                            │
    │  │   ├─ 仅目标子 Agent 重跑（注入 hint）                   │
    │  │   ├─ 其他子 Agent 复用旧产出                            │
    │  │   └─ 编剧重跑（合并新旧子产出）                         │
    │  └─────────────────────────────────────────────────────────┘
    │
    │  ④ run_editor_and_commit()  ← 最终都走这里
    │     ├─ 跑编剧（流式）
    │     ├─ build_provenance（含 last_hint）
    │     └─ 写入对话树：
    │        ├─ 最后一条 AI → replace_active_variant（原地替换）
    │        └─ 中间 AI → add_variant（开分支，保留旧版）
    │
    │  返回 (final_text, provenance)
    │
    ▼
前端：重拉对话 applyConversation → 刷新 UI
```

---

## 3. user 消息重 roll

```
用户点 user 消息的「🔄 重roll」
    │
    ▼
App.vue.handleRerollUser({messageId})                  ← frontend/src/App.vue:488
    │  1. 找到这条 user 消息的 content → intent
    │  2. 找到紧随其后的 AI 消息 → aiMsg
    │
    │  有 AI 消息 → 调 regenerate（重 roll）
    │  ├─ apiRegenerate({
    │  │    conversationId: 当前对话 ID,
    │  │    nodeId: aiMsg.id,          ← 目标是 AI 消息（有 provenance）
    │  │    targets: [],                ← 整体重 roll
    │  │    hint: intent,               ← user 的意图作为 hint
    │  │  }, onEvent)
    │  └─ 后端走 regenerate 路径 A（整体重 roll）
    │     ├─ 导演 user_message = 旧 plan.scene_brief + inject_hint(intent)
    │     ├─ recent_messages 排除目标 AI 消息及之后（before_node_id = aiMsg.id）
    │     ├─ 子 Agent 全部重跑
    │     └─ 编剧重跑（注入 hint = user intent）
    │
    │  无 AI 消息（已被删除）→ 调 startWriting（重新写作）
    │  └─ startWriting(intent, skipLocalPush=true)
    │     ├─ skipLocalPush=true → 不本地 push user 消息（已在列表中）
    │     └─ 追加到当前对话（conversation_id = currentConversationId）
    │
    │  结果：替换原 AI 消息 或 新增 AI 消息
    │
    ▼
前端：重拉对话 → [开场白, u1, a1, u2, a2, u3, new_a3]
```

---

## 4. 删除消息（truncate 语义）

```
用户点消息的「🗑 删除」
    │
    ▼
App.vue.handleDeleteVariant({nodeId})                  ← frontend/src/App.vue:441
    │  调 apiDeleteMessageFrom(conversationId, nodeId)
    │
    ▼
[Tauri 命令] delete_message_from                       ← tauri-app/src/lib.rs
    │  conv_store.truncate_from(conv_id, node_id)      ← app-conversation/src/lib.rs:392
    │     └─ 删除指定 node 及其后所有 node
    │     └─ 保留该 node 之前的所有消息
    │
    ▼
前端：重拉对话 applyConversation → 刷新 UI
    │  清流水线状态（导演/子Agent/编剧输出全置 idle）
```

---

## 5. 会话历史选择界面

```
启动 App
    │
    ▼
onMounted → loadConversationHistory()                  ← frontend/src/App.vue:82
    │  listConversations() → conversationHistory.value
    │  showHistory.value = true
    │
    ▼
显示会话历史列表                                        ← frontend/src/App.vue:800
    │  遍历 conversationHistory
    │  每项显示: 会话 {id前8位} · {message_count} 条消息 · {updated_at}
    │
    │  用户操作：
    │  ├─ 点击会话 → openConversation(conv)
    │  │   ├─ getConversation(id) → applyConversation(conv)
    │  │   ├─ currentConversationId = conv.id
    │  │   ├─ showHistory = false
    │  │   └─ 加载关联角色卡
    │  │
    │  ├─ 「新对话」→ startNewConversation()
    │  │   ├─ messages = []
    │  │   ├─ currentConversationId = null
    │  │   └─ showHistory = false
    │  │
    │  └─ 📜 按钮 → showHistory = true（返回历史列表）
```

---

## 6. 对话树结构

```
Conversation { nodes: Vec<MessageNode> }

MessageNode {
    id: Id,
    parent_id: Option<Id>,       ← 自动链接前一个 node
    variants: Vec<MessageVariant>,
    active_variant: usize,
}

MessageVariant {
    id: Id,
    role: Role,                  ← User | Assistant
    content: String,
    status: VariantStatus,       ← Draft | Final | Discarded
    provenance: Option<Provenance>,  ← AI 消息有，user 消息没有
}

Provenance {
    session_id: Id,
    plan: Option<Plan>,          ← 导演的 Plan（场景+子任务）
    subagent_results: Vec<SubagentSnapshot>,  ← 子 Agent 产出快照
    profile_id: Option<Id>,
    seed: u64,
    last_hint: Option<String>,   ← 重 roll 时的 hint
}
```

---

## 7. 前端消息 vs 后端对话

```
前端 messages.value（本地状态）:
  ├─ 导入时本地 push 开场白
  ├─ startWriting 本地 push user 消息（skipLocalPush=false 时）
  ├─ editor_progress 更新 editor-streaming 占位
  ├─ startWriting 完成后 push AI 成文
  └─ applyConversation 整体替换为后端数据（打开会话/重 roll 刷新时）

后端对话（持久化）:
  ├─ start_writing 新建时: append 开场白 + user 意图 (Final)
  ├─ start_writing 复用时: 只 append user 意图 (Final)
  ├─ pipeline 完成后: append AI 成文 (Draft)
  ├─ regenerate 时: replace_active_variant（最后一条）或 add_variant（中间）
  └─ delete_message_from 时: truncate_from（删除该条及之后所有）

会话历史:
  ├─ listConversations → 返回摘要（id, message_count, updated_at）
  ├─ openConversation → getConversation → applyConversation
  └─ startNewConversation → 清空 messages, currentConversationId = null
```

---

## 8. 事件流（Channel）

```
start_writing / regenerate 通过 Channel 推送事件到前端：

started
  → state_changed(Directing)
    → director_started
      → director_progress×N（流式 token）
    → director_done
  → state_changed(Delegating)
    → subagent_started×N
      → subagent_progress×N（流式 token，每个子 Agent 独立）
    → subagent_done×N / subagent_cancelled×N
  → state_changed(Editing)
    → editor_started
      → editor_progress×N（流式 token）
    → draft_ready
  → state_changed(Review)
  → state_changed(Committed)

后处理（有 Campaign 时）：
  → postprocess_started
    → summary_done
    → postprocess_done（或 postprocess_failed）
```

---

## 9. 取消机制

```
前端 cancelWriting()
    │  调 apiCancelWriting()
    ▼
Tauri cancel_writing
    │  锁 AppState.current_cancel
    │  sender.send(true)
    ▼
watch channel 广播：
    ├─ 导演 run_tool_loop_streaming（select! 与 chat_stream 竞速）
    ├─ 子 Agent×N run_tool_loop_streaming（每轮前检查）
    ├─ 编剧 run_tool_loop_streaming（同上）
    └─ 后处理 run_summarizer / run_postprocess
```

---

*基于代码实测，2026-06-16。87 个 Tauri 命令，83 个 tauri-api 导出函数，15 个前端组件，242 个测试。*

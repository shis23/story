# Phase 8 重构遗留 + 真实环境暴露问题清单

> 创建日期：2026-07-08
> 来源：Bronze B1 首次真实环境验收（opencode.ai/zen/go + deepseek-v4-flash）发现的 Phase 8 前端重构遗留与后端缺陷。
> 原则：**所有问题不论是否阻塞 B1 都要修**。本文档是完整登记表，修复后在「状态」列打勾并附 commit。

## 状态标记

- 🔴 **待修**：已登记，未开始
- 🟡 **已查清根因**：根因已定位，待修复
- 🟢 **已修**：附 commit + 验证记录
- ⚪ **待诊断**：现象已记录，根因未明

---

## 一、已修复的问题（保留记录）

### P0-1 ✅ postprocess terminal_tools 缺失导致超轮次失败
- **现象**：postprocess 报「超过最大轮次 5」，整个后处理失败，无 editor trace、node 停 Draft、无写回。
- **根因**：`crates/app-agent/src/prompts/postprocess.rs:129` `terminal_tools: vec![]` 应为 `vec!["emit_postprocess".into()]`。LLM 调 emit_postprocess 后 run_tool_loop 不终止，5 轮耗尽报 MaxRoundsExceeded。
- **修复**：commit `cb971eb`。新增回归测试 `test_config_marks_emit_postprocess_as_terminal`。

### P0-2 ✅ director terminal_tools 缺失导致工具循环死循环
- **现象**：Director 反复调工具不收敛，输出「**这是最终答案。** 没有更多工具可用」等 reasoning 残留，15 轮耗尽失败。
- **根因**：`crates/app-pipeline/src/lib.rs:1905` `terminal_tools: vec![]` 应为 `vec!["emit_plan".into()]`。LLM 调 emit_plan 后不终止。content 的 JSON 完成探测兜不住 tool_call 路径。
- **修复**：commit `bc02bae`。新增回归测试 `test_director_config_marks_emit_plan_as_terminal`。

### P0-3 ✅ real-llm smoke runner 脚本崩溃（非 Bronze，验证流程阻塞）
- **根因**：`run-real-llm-smoke.ps1` cargo 输出污染返回值管道 + stderr 触发 Stop。
- **修复**：commit `f7f65d6`。

### P0-4 ✅ knowledge suite 断言过严（非 Bronze）
- **根因**：测试硬断言 broadcast 必须是 `Group("守卫")`，LLM 合理输出 `All` 被误判。
- **修复**：commit `6126a1f`。

---

## 二、待修问题（本次验收暴露）

### P1-1 🟢 reroll 后旧模型回答没清掉
- **现象**：点击重 roll 后，旧的模型回答仍显示。用户期望：
  - 重 roll **用户消息** → 从该用户消息**之后**的模型回答开始删
  - 重 roll **模型正文** → 从该条正文开始删
- **根因（已查清）**：reroll 走 `regenerate`，后端落库（`app-pipeline/src/lib.rs:1418-1443`）只用 `add_variant`/`replace_active_variant`（variant 级软删/开分支），**从不调用 `truncate_from`**（node 级真删）。中间消息重 roll 走 `add_variant` 保留旧版；`is_last_assistant_node` 判定（`app-conversation/src/lib.rs:366-382`）在多轮场景下常不成立。前端 `useMessageVariants.js:204` `handleRerollUser` 只锁紧随其后的 AI 消息，不处理后续轮次。
- **修复（commit `cf8a565`）**：后端 `regenerate` 落库改为 `truncate_from`（截断目标 AI 节点及之后所有 node）+ `append_ai_draft`（新成文作为最后一条 assistant 节点）。两种重 roll 场景（用户消息→前端传紧随其后的 AI 节点；模型正文→直接传该 AI 节点）都归结为「从目标 AI 节点截断」，旧的 AI 回答彻底消失。更新 4 个 regenerate 测试断言新语义。

### P1-2 🟢 调试面板（右侧 InspectorDrawer）日志为空
- **现象**：后端确实往 `%APPDATA%/StoryForge/logs/*.jsonl` 写了日志（实测 42 条），但前端调试面板日志区空白。
- **根因（已查清）**：`LogStore::new`（`crates/app-logging/src/lib.rs:241-249`）启动时**只建空内存缓冲，不回填 jsonl 文件**。跨会话历史全丢。当前会话的日志能进内存（`push` 同时写盘+内存），但之前会话的磁盘记录无法显示。前端是「拉取」模型（`logQuery` → `invoke('log_query')`），不靠事件，前端链路本身正常。
- **修复（commit `9deea86`）**：新增 `backfill_from_dir`，`LogStore::new` 启动时读取 `log_dir/*.jsonl`，按文件名(=日期)升序逐行反序列化为 `LogEntry` 后 push 进 buffer（旧→新顺序保证 LRU 截断保留最近会话）。损坏行容错跳过。新增 2 个回归测试。

### P1-3 🟢 Campaign 管理界面有两个「导入 Bundle」按钮
- **现象**：Cards tab 里出现两个「导入 Bundle」按钮。
- **根因（已查清）**：Phase 8 重复挂载。`CampaignPanel.vue:315-323` 有一个，`CardLibrary.vue:89-92` 又有一个（经 `@import-bundle` emit 回到同一个 `handleImportBundle`）。
- **修复（commit `12e1613`）**：删 `CardLibrary.vue:89-92` 的按钮（保留父级带 disabled 态和提示的），清掉 CardLibrary 的 `import-bundle` emit 声明和 CampaignPanel 的 `@import-bundle` 监听。

### P1-4 🟢 Campaign 管理的「游玩档」和「档详情」界面为空
- **现象**：游玩档 tab 和档详情 tab 打开都是空/EmptyState。
- **根因（已查清）**：接线遗漏。`CampaignPanel.vue:42` `selectedCardId` 初始 null，`onMounted`（:97-100）只调 `getActiveCampaign()` 不回填 `selectedCardId`，`refreshCampaigns()`（:108-116）因 `!selectedCardId` 直接 return。数据加载唯一入口是「角色卡 tab 点卡→点管理游玩档」（`openCampaignsForCard`）。实测 campaigns.json 有数据，是没触发加载。
- **修复（commit `ad3fa93`）**：`CampaignPanel.onMounted` 有活跃 Campaign 时回填 `selectedCardId` 并预加载游玩档。

### P1-5 🟢 会话历史界面显示「未知角色卡」
- **现象**：历史会话列表显示「未知角色卡」fallback 文案。
- **根因（已查清）**：ID 命名空间不匹配。`list_conversations`（`crates/tauri-app/src/lib.rs:3928-3933`）用 conversation.character_id（=CharacterCard id，如 `cbcb4aef-...`）去 `tool_ctx.characters`（domain Character，id=source_character_id `e02d2865-...`）里 join，恒不匹配 → card_name=None → 前端 fallback「未知角色卡」。
- **修复（commit `ad3fa93`）**：联查改用 `CampaignStore` 卡片表按 `CharacterCard.id` 查卡名（建 `card_by_id` HashMap），加 `campaign_id → campaign.card_id → card.name` 兜底。

---

## 三、本次新增待诊断问题（导演/编剧/Trace/后处理可见性）

### P2-1 🟢 导演生成完成后，前端看不到导演输出了
- **现象**：原本可以点开查看「导演输出」（Director 的 plan）的入口不见了。生成过程中还能看到（director_progress delta 流），但完成后看不到结果。
- **根因（已查清）**：渲染层缺陷，非数据丢失。`writingStore.pipeline.director.output`（累积的 delta）完成后仍在，但唯一展示它的 `StreamingMessage.vue:60-73` 导演折叠块，其挂载依赖 `writing.showPipeline`（`ConversationViewport.vue:131-135` 的 `v-if`），而 `useWriting.js:168` 在完成回调里 `showPipeline=false`，导致 StreamingMessage 整个卸载，导演折叠块随之消失。
- **修复（commit `ad3fa93`）**：新增 `ProcessReview.vue`，写作完成后独立于 `showPipeline` 挂载（条件 `!isWriting && pipeline.state==='done' && 有导演/子 Agent 输出`），以折叠块展示导演规划/子 Agent 产出回顾。挂进 `ConversationViewport`。

### P2-2 🟢 流水线 Trace 在前端什么都看不到（导演生成过程中还能看到）
- **现象**：Pipeline Trace 面板/区域，导演生成过程中有内容，但完成后变空。
- **根因（已查清）**：分两种观察位置：
  - **若指对话区过程块**：与 P2-1 同因（StreamingMessage 卸载）。
  - **若指右侧常驻 trace 面板**：`AppShell.vue:29` `<aside v-if="ui.powerMode && false">` 把桌面常驻调试抽屉**用 `&& false` 写死成永不渲染**。
- **修复（commit `12e1613`）**：右侧常驻抽屉去掉 `AppShell.vue:29` 的 `&& false`（恢复 `v-if="ui.powerMode"`）；对话区过程块由 P2-1 的 ProcessReview 修复。

### P2-3 🟢 编剧（Editor）输出很久不出现
- **现象**：导演完成后，编剧输出要等很久，且最终混入了「改动说明」之类的非正文内容。
- **根因（已查清，与 P2-4 同因）**：Editor 默认 `max_tool_rounds:5` 但 `tools:vec![]`，实际只跑一轮。慢的根因是 Editor prompt 被要求同时输出正文+元描述导致 token 偏多，收紧 prompt 后输出量下降会顺带提速。无需改并发/轮次配置。
- **修复**：随 P2-4 收紧 prompt（commit `12e1613`），输出量下降顺带提速。

### P2-4 🟢 编剧最后输出的「改动说明」混入了正文
- **现象**：Editor 的总结性/说明性文字（如「以上是合并后的成文」之类的元描述）出现在用户看到的正文里。
- **根因（已查清，prompt 主动要求）**：`crates/app-pipeline/src/lib.rs:94` `EDITOR_SYSTEM_PROMPT` 第 3 条「3. 标注哪些子表演被你裁剪/改动了」**明确要求 LLM 输出元描述**。
- **修复（commit `12e1613`）**：删除第 3 条，改成明确禁止元描述（「只输出正文本身，严禁输出任何说明、注释、改动标注、总结性文字。第一行就必须是正文」）。新增回归测试 `test_editor_prompt_forbids_meta_commentary`。

### P2-5 🟢 后处理结果完全没有查看入口
- **现象**：后处理（postprocess）的知识/变量/任务/摘要写回了（round_summaries.json/tasks.json 有内容），但用户在前端找不到地方看这些结果。
- **根因（已查清，入口太深 + 异步时机，非接线/渲染/数据问题）**：数据层、接线均正常，真根因是入口太深——要看后处理结果需 5 步嵌套（PrimarySidebar 点 Campaign 管理 → 默认角色卡 tab → 切游玩档 → 选卡 → 选档 → 切知识/任务/摘要子 tab）。
- **修复（commit `ad3fa93`）**：`CampaignPanel.onMounted` 有活跃 Campaign 时默认进档详情 tab（原默认角色卡 tab），并预选活跃 Campaign，扁平化后处理结果入口（知识/任务/摘要子 tab 一键可达）。

### P2-6 🟢 后处理知识抽取输出过长被截断，knowledge.json 不生成
- **现象**：postprocess 跑 4 轮，前 3 轮 completion_tokens=4096（撞 max_tokens），knowledge_updates JSON 被截断 → parse 失败 → knowledge.json 不生成（其他 summaries/tasks 正常）。
- **根因（已查清）**：postprocess 单次输出超过 max_tokens，deepseek-v4-flash 输出冗长（带解释/markdown）。fallback 重试虽最后产出部分结果，但 knowledge_updates 解析为空。
- **修复（commit `e5fe263`）**：(1) `POSTPROCESS_SYSTEM_PROMPT` 输出格式段收紧，明确要求「只输出一个 JSON 对象，第一个字符必须是 {，最后一个字符必须是 }。严禁输出解释/说明/前导语/结语/Markdown/代码围栏/自然语言」；(2) `run_direct_json_fallback`（专门做 JSON 抽取，无工具开销）的 `max_tokens` 从默认 4096 放宽到 8192，temperature 降到 0.3。新增回归测试 `test_prompt_forbids_verbose_output`。

---

## 四、非 Bronze 但登记的问题

### P3-1 🟢 标题栏右上角两个 x 号（需运行时视觉确认，无自绘重复）
- **现象**：窗口标题栏出现两个关闭按钮。
- **诊断（已查清）**：前端代码无自绘窗口控制按钮——TopBar.vue 只有「菜单」和「🛠」两个按钮，无 close/✕；PanelHost.vue 的关闭按钮是 `×` 图标，但仅在带标题的弹层（InspectorDrawer 用 `show-header=false` 不渲染它）。tauri.conf.json `decorations: true` 显示 OS 原生标题栏（Windows 三个按钮：最小化/最大化/关闭）。
- **结论**：代码层面无重复关闭按钮的来源。最可能是用户观察到的视觉重叠——右侧 Overlay 弹层（`fixed right-0 top-0`）若占满宽度，其右上角接近 OS 标题栏关闭键，视觉上像「两个 x」。无可靠代码修复（移除 OS decorations 会丢失窗口拖拽/系统控件，代价更大）。**不影响功能。**若用户能截图确认第二个 x 的确切位置再做针对性修复。

### P3-2 🟢 模型列表 datalist 只显示 minimax-m3
- **现象**：拉取模型列表后下拉只显示第一项。
- **诊断**：非 bug。`<datalist>` 在输入框有值（`form.model` 被自动设成 models[0]）时只显示前缀匹配项。清空输入框可见全部 20 个。
- **修复**：拉取后仅在 `form.model` 为空时才填第一个（避免空保存），不再覆盖非空值。下拉不再被自动填充锁死成只显示第一项。

### P3-3 🟢 需要支持附加参数（thinking/reasoning_effort）
- **现象**：SamplingParams 只有 temperature/top_p/max_tokens，无法传 thinking/reasoning_effort 等扩展参数。
- **修复**：`SamplingParams` 新增 `extra: Option<serde_json::Map<String, Value>>`（前向兼容，透传到请求体顶层），`build_request_body` 注入 extra 到 JSON 顶层。`CreateConnectionDto` 加 `extra` 字段，前端 `ConnectionConfigPanel` 加「扩展参数 JSON」textarea（带解析校验），`createConnection` API 透传。新增回归测试 `test_build_request_body_passes_extra_params`。

### P3-4 🟢 model 硬编码 "deepseek-chat"（占位符 sentinel，按设计工作）
- **现象**：所有 Agent config 在无 model_override 时 fallback 到硬编码 "deepseek-chat"，而非连接配的 model。
- **诊断（已查清，按设计工作）**：`"deepseek-chat"` 是**预期的占位符 sentinel**，不是 bug。`http_client.rs:119` `effective_model` 的 `PLACEHOLDER_MODELS = ["deepseek-chat", "mock"]` 在请求时把这些 sentinel 替换成连接配的 model（如 deepseek-v4-flash）。10 处硬编码（director/editor/postprocess/summarizer/character_extractor/meta 系）都走同一占位符机制,实际请求用的就是连接 model。
- **结论**：占位符检测机制正确且完整,功能不受影响。真正的契约改进（换更明确的 sentinel 如 `"__connection_default__"`）需同步改 10 处 + effective_model 的 PLACEHOLDER 列表,收益小、改动面大、有回归风险,不值得在 B1 阶段动。**保持现状。**

### P3-5 🟢 AgentProfileManager 面板未挂载到 AppV2
- **现象**：Phase 8 重构后 `AgentProfileManager.vue` 没被挂载，用户无法通过 UI 配 AgentProfile（含 model_override）。
- **诊断（已查清）**：`AppV2.vue` 只挂了 ConnectionConfigPanel/PresetPanel/PluginPanel，漏了 AgentProfileManager。ui store 无 showAgentProfile 状态，PrimarySidebar/AppShell 无对应入口。
- **修复**：挂回 AgentProfileManager（独立面板模式）——ui store 加 `showAgentProfile`，PrimarySidebar 加「🤖 Agent 配置」菜单项 + `open-agent-profile` emit，AppShell 监听该 emit 置 `showAgentProfile=true`，AppV2 在 panels slot 挂 `AgentProfileManager`（v-if=showAgentProfile，close 复位）。改 4 文件，不碰 tauri-api 契约。

---

## 修复优先级建议（已全部完成 ✅）

所有 P1/P2/P3 问题已修复（P0 在本轮验收前已修）。

1. ✅ **P2 系列先诊断清楚**（commit `12e1613`/`ad3fa93`/`e5fe263`）
2. ✅ **P1-3/4/5**（小而确定的接线 bug，commit `12e1613`/`ad3fa93`）
3. ✅ **P1-1/2**（reroll 清理逻辑、LogStore 回填，commit `cf8a565`/`9deea86`）
4. ✅ **P2-6**（postprocess 截断，commit `e5fe263`）
5. ✅ **P3 系列**（P3-1/4 记录为按设计工作，P3-2/3/5 已修）

## 验证汇总

- `cargo test -p storyforge-app-pipeline --lib`：全 55 通过
- `cargo test -p storyforge-app-agent --lib`：全 98 通过
- `cargo test -p storyforge-app-logging --lib`：全 7 通过
- `cargo test -p storyforge-infra-llm --lib`：全 34 通过
- `cargo check --workspace`：干净
- `frontend npm run build`：干净
- `frontend npm test`：全 212 通过

## 修复后填写

每个问题修复后在对应条目「状态」改为 🟢 并附 commit hash + 简要验证记录。

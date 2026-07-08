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

### P1-1 🔴 reroll 后旧模型回答没清掉
- **现象**：点击重 roll 后，旧的模型回答仍显示。用户期望：
  - 重 roll **用户消息** → 从该用户消息**之后**的模型回答开始删
  - 重 roll **模型正文** → 从该条正文开始删
- **根因（已查清）**：reroll 走 `regenerate`，后端落库（`app-pipeline/src/lib.rs:1418-1443`）只用 `add_variant`/`replace_active_variant`（variant 级软删/开分支），**从不调用 `truncate_from`**（node 级真删）。中间消息重 roll 走 `add_variant` 保留旧版；`is_last_assistant_node` 判定（`app-conversation/src/lib.rs:366-382`）在多轮场景下常不成立。前端 `useMessageVariants.js:204` `handleRerollUser` 只锁紧随其后的 AI 消息，不处理后续轮次。
- **修复方向**：reroll 前先 `truncate_from` 截断目标 node 及之后所有 node，再 regenerate；或后端 `regenerate` 增加「截断+重生成」语义。
- **关键文件**：`frontend/src/composables/useMessageVariants.js`、`crates/app-pipeline/src/lib.rs:1418-1452`、`crates/app-conversation/src/lib.rs:443-454`

### P1-2 🔴 调试面板（右侧 InspectorDrawer）日志为空
- **现象**：后端确实往 `%APPDATA%/StoryForge/logs/*.jsonl` 写了日志（实测 42 条），但前端调试面板日志区空白。
- **根因（已查清）**：`LogStore::new`（`crates/app-logging/src/lib.rs:241-249`）启动时**只建空内存缓冲，不回填 jsonl 文件**。跨会话历史全丢。当前会话的日志能进内存（`push` 同时写盘+内存），但之前会话的磁盘记录无法显示。前端是「拉取」模型（`logQuery` → `invoke('log_query')`），不靠事件，前端链路本身正常。
- **修复方向**：`LogStore::new` 增加回填逻辑，读取 `log_dir/*.jsonl` 按行反序列化后 `buf.push`，注意文件名 `{date}.jsonl` 格式和 `MAX_ENTRIES_PER_KIND=2000` LRU 截断。
- **关键文件**：`crates/app-logging/src/lib.rs:241-263`

### P1-3 🔴 Campaign 管理界面有两个「导入 Bundle」按钮
- **现象**：Cards tab 里出现两个「导入 Bundle」按钮。
- **根因（已查清）**：Phase 8 重复挂载。`CampaignPanel.vue:315-323` 有一个，`CardLibrary.vue:89-92` 又有一个（经 `@import-bundle` emit 回到同一个 `handleImportBundle`）。
- **修复方向**：删 `CardLibrary.vue:89-92` 的按钮（保留父级的，因父级带 `importingBundle` disabled 态和 `importStatus` 提示），同时清掉 CardLibrary 的 `import-bundle` emit 声明和 CampaignPanel 的 `@import-bundle` 监听。
- **关键文件**：`frontend/src/components-v2/campaign/CardLibrary.vue:89-92`、`frontend/src/components-v2/campaign/CampaignPanel.vue:315-331`

### P1-4 🔴 Campaign 管理的「游玩档」和「档详情」界面为空
- **现象**：游玩档 tab 和档详情 tab 打开都是空/EmptyState。
- **根因（已查清）**：接线遗漏。`CampaignPanel.vue:42` `selectedCardId` 初始 null，`onMounted`（:97-100）只调 `getActiveCampaign()` 不回填 `selectedCardId`，`refreshCampaigns()`（:108-116）因 `!selectedCardId` 直接 return。数据加载唯一入口是「角色卡 tab 点卡→点管理游玩档」（`openCampaignsForCard`）。实测 campaigns.json 有数据，是没触发加载。
- **修复方向**：onMounted 里根据 `campaignStore.activeCampaign` 反查 card_id 回填 `selectedCardId` 并 `refreshCampaigns()`；或放宽游玩档 tab 用 `list_campaigns(card_id=None)` 直接列全部档（后端 lib.rs:6663 已支持）。
- **关键文件**：`frontend/src/components-v2/campaign/CampaignPanel.vue:42,97-124,335-415`

### P1-5 🔴 会话历史界面显示「未知角色卡」
- **现象**：历史会话列表显示「未知角色卡」fallback 文案。
- **根因（已查清）**：ID 命名空间不匹配。`list_conversations`（`crates/tauri-app/src/lib.rs:3928-3933`）用 conversation.character_id（=CharacterCard id，如 `cbcb4aef-...`）去 `tool_ctx.characters`（domain Character，id=source_character_id `e02d2865-...`）里 join，恒不匹配 → card_name=None → 前端 fallback「未知角色卡」。
- **修复方向**：联查改为按 card_id 查 campaign_store/cards store 拿 `card.name`；或利用 conversation.campaign_id → get_campaign → card_id → card name。
- **关键文件**：`crates/tauri-app/src/lib.rs:3919-3945`

---

## 三、本次新增待诊断问题（导演/编剧/Trace/后处理可见性）

### P2-1 🟡 导演生成完成后，前端看不到导演输出了
- **现象**：原本可以点开查看「导演输出」（Director 的 plan）的入口不见了。生成过程中还能看到（director_progress delta 流），但完成后看不到结果。
- **根因（已查清）**：渲染层缺陷，非数据丢失。`writingStore.pipeline.director.output`（累积的 delta）完成后仍在，但唯一展示它的 `StreamingMessage.vue:60-73` 导演折叠块，其挂载依赖 `writing.showPipeline`（`ConversationViewport.vue:131-135` 的 `v-if`），而 `useWriting.js:168` 在完成回调里 `showPipeline=false`，导致 StreamingMessage 整个卸载，导演折叠块随之消失。**注：这是 legacy App.vue 的既有行为（ab70021^ 同样 L795 showPipeline=false），非 Phase 8 回归。** 另外历史消息也查不到 plan：ChatMessage.vue 的 provenance 只用于 seed 显示和重 roll 菜单（无 plan 展开块），且 variant 的 provenance 只有 subagent_results 不含 director plan（`pipelineTrace.js`）。
- **修复方向**：完成态需要独立的过程回顾组件，挂载条件脱离 showPipeline。最简：把导演/子Agent/编剧过程块从 StreamingMessage 提到 ConversationViewport，条件 `v-if="pipeline.director.output && !isWriting"`。若要历史消息也能查 plan，需把 plan/provenance 持久化到 message variant（当前缺失）。
- **关键文件**：`frontend/src/composables/useWriting.js:168`、`frontend/src/components-v2/writing/ConversationViewport.vue:131-135`、`frontend/src/components-v2/writing/StreamingMessage.vue:60-73`

### P2-2 🟡 流水线 Trace 在前端什么都看不到（导演生成过程中还能看到）
- **现象**：Pipeline Trace 面板/区域，导演生成过程中有内容，但完成后变空。
- **根因（已查清）**：分两种观察位置：
  - **若指对话区过程块**：与 P2-1 同因（StreamingMessage 卸载）。数据 `pluginStore.pluginPipelineEvents`（`plugin.js:13`）完成后保留，`pushPluginEventRecord` 只 append+slice 到 100 条，任何态都不清空。
  - **若指右侧常驻 trace 面板**：`AppShell.vue:29` `<aside v-if="ui.powerMode && false">` 把桌面常驻调试抽屉**用 `&& false` 写死成永不渲染**。现在打开 trace 的唯一途径是 Overlay（🛠 按钮 → `showDebugDrawer`），该路径独立于 isWriting/完成态，trace tab 数据链健康（`PipelineTracePanel.vue:33-37` 读 pluginPipelineEvents，导演输出 L86-94 由 director_progress 重算，完成后仍有内容）。
- **修复方向**：右侧常驻抽屉去掉 `AppShell.vue:29` 的 `&& false`（恢复 `v-if="ui.powerMode"`）；对话区过程块同 P2-1 修复。
- **关键文件**：`frontend/src/components-v2/shell/AppShell.vue:29`、`frontend/src/components-v2/writing/ConversationViewport.vue:131-135`、`frontend/src/composables/useWriting.js:168`

### P2-3 🟡 编剧（Editor）输出很久不出现
- **现象**：导演完成后，编剧输出要等很久，且最终混入了「改动说明」之类的非正文内容。
- **根因（已查清，与 P2-4 同因）**：Editor 默认 `max_tool_rounds:5` 但 `tools:vec![]`，实际只跑一轮。慢的根因是 Editor prompt 被要求同时输出正文+元描述导致 token 偏多，收紧 prompt 后输出量下降会顺带提速。无需改并发/轮次配置。

### P2-4 🔴 编剧最后输出的「改动说明」混入了正文
- **现象**：Editor 的总结性/说明性文字（如「以上是合并后的成文」之类的元描述）出现在用户看到的正文里。
- **根因（已查清，prompt 主动要求）**：`crates/app-pipeline/src/lib.rs:94` `EDITOR_SYSTEM_PROMPT` 第 3 条「3. 标注哪些子表演被你裁剪/改动了」**明确要求 LLM 输出元描述**——不是 LLM 自由发挥，是 prompt 自己要求的。Editor response.content 被 `apply_editor_output_regex`（lib.rs:1490-1500，只处理 `<think>` 块和用户正则）原样存为成文 content，零 commentary 剥离。前端 RichContent.vue 也只做格式化消毒，无正文/注释分段。全局唯一 commentary 要求来源就是 lib.rs:94 这一行。
- **修复方向（治本，改 prompt）**：删除 lib.rs:94 第 3 条，改成明确禁止元描述：「只输出正文本身，严禁输出任何说明、注释、改动标注、总结性文字。第一行就必须是正文。」
- **关键文件**：`crates/app-pipeline/src/lib.rs:90-96`（EDITOR_SYSTEM_PROMPT）

### P2-5 🟡 后处理结果完全没有查看入口
- **现象**：后处理（postprocess）的知识/变量/任务/摘要写回了（round_summaries.json/tasks.json 有内容），但用户在前端找不到地方看这些结果。
- **根因（已查清，入口太深 + 异步时机，非接线/渲染/数据问题）**：
  - 数据层正常：`persist_postprocess_outcome_to_store`（lib.rs:2918-3013）四类全落盘；后端命令 `list_round_summaries`/`list_character_knowledge`/`list_tasks` 齐全且注册。
  - 接线正常：CampaignKnowledgeTab/CampaignSummariesTab/CampaignTasksTab 都 import、都 v-if 挂载（CampaignPanel.vue:467-485）、都 onMounted+watch 自动加载。摘要 tab 显示 turn/content/created_at 三列。
  - **真根因是 (d) 入口太深**：要看后处理结果需 5 步嵌套——PrimarySidebar 点 Campaign 管理 → CampaignPanel 默认 `activeTab='cards'`（CampaignPanel.vue:31）要切到游玩档 → 选卡 → 选档（切 detail tab）→ 再切知识/任务/摘要子 tab。且默认进的是角色卡 tab 不是档详情。
  - 次要：postprocess 是 `spawn_blocking` 异步（lib.rs:2157-2172），写完立即看可能空，且无"后处理完成"UI 提示引导去看；子 tab 不自动刷新，需手动点刷新按钮。
- **修复方向**：(1) 有 activeCampaign 时 CampaignPanel 默认进档详情而非角色卡；(2) 写作完成后给「查看本轮后处理结果」直达入口/toast；(3) postprocess 完成事件推前端后自动刷新当前 tab 并提示；(4) 路径扁平化（知识/任务/摘要 提为一级 tab 或加角标）。
- **关键文件**：`frontend/src/components-v2/campaign/CampaignPanel.vue:31,467-485`、`frontend/src/composables/useWriting.js`（完成回调）

### P2-6 🟡 后处理知识抽取输出过长被截断，knowledge.json 不生成
- **现象**：postprocess 跑 4 轮，前 3 轮 completion_tokens=4096（撞 max_tokens），knowledge_updates JSON 被截断 → parse 失败 → knowledge.json 不生成（其他 summaries/tasks 正常）。
- **根因（已查清）**：postprocess 单次输出超过 max_tokens，deepseek-v4-flash 输出冗长（带解释/markdown）。fallback 重试（run_direct_json_fallback）虽最后产出部分结果，但 knowledge_updates 解析为空。
- **修复方向**：(a) 收紧 postprocess prompt，要求纯 JSON 无解释；(b) 调大 postprocess 的 max_tokens；(c) 或前端配置连接的 max_tokens 影响（用户设了 1000000，但 AgentConfig 是否覆盖待查）。
- **关键文件**：`crates/app-agent/src/prompts/postprocess.rs`、`crates/app-agent/src/postprocess.rs:96-122`（fallback）

---

## 四、非 Bronze 但登记的问题

### P3-1 ⚪ 标题栏右上角两个 x 号
- **现象**：窗口标题栏出现两个关闭按钮。
- **诊断**：前端代码无自绘窗口控制按钮（TopBar 只有 🛠）。疑为 WebView2/系统标题栏层面，或 tauri.conf.json `decorations`/`titleBarStyle` 配置。不影响功能。

### P3-2 ⚪ 模型列表 datalist 只显示 minimax-m3
- **现象**：拉取模型列表后下拉只显示第一项。
- **诊断**：非 bug。`<datalist>` 在输入框有值（`form.model` 被自动设成 models[0]）时只显示前缀匹配项。清空输入框可见全部 20 个。
- **修复方向**：拉取后不自动填充，或换 `<select>`。

### P3-3 ⚪ 需要支持附加参数（thinking/reasoning_effort）
- **现象**：SamplingParams 只有 temperature/top_p/max_tokens，无法传 thinking/reasoning_effort 等扩展参数。
- **诊断**：真缺口。需扩 SamplingParams + build_request_body 注入 + 前端表单。新功能，跨 domain/infra-llm/前端。

### P3-4 ⚪ model 硬编码 "deepseek-chat"（6 处 fallback）
- **现象**：所有 Agent config 在无 model_override 时 fallback 到硬编码 "deepseek-chat"，而非连接配的 model。
- **诊断**：被 `http_client.rs:119` `effective_model` 占位符兜底，实际请求用连接 model，不阻塞功能。但违背显式契约（agent_profile_config.rs 注释说 None=用连接默认），且有隐性陷阱（用户连 deepseek 且 model 字段填别名时）。
- **修复方向**：fallback 改读连接 model，终极兜底保留 "deepseek-chat"。
- **关键文件**：`crates/app-pipeline/src/lib.rs:1903,1967`、`crates/app-agent/src/prompts/postprocess.rs:127`、`summarizer.rs:50`、`character_extractor.rs:69`、`crates/app-pipeline/src/lib.rs:1903`

### P3-5 ⚪ AgentProfileManager 面板未挂载到 AppV2
- **现象**：Phase 8 重构后 `AgentProfileManager.vue` 没被挂载，用户无法通过 UI 配 AgentProfile（含 model_override）。
- **诊断（已查清）**：`AppV2.vue` 只挂了 ConnectionConfigPanel/PresetPanel/PluginPanel，漏了 AgentProfileManager。ui store 无 showAgentProfile 状态，PrimarySidebar/AppShell 无对应入口。
- **修复方向**：挂回 AgentProfileManager（独立面板模式），改 4 文件约 11 行（ui.js + PrimarySidebar.vue + AppShell.vue + AppV2.vue），不碰 tauri-api 契约。

---

## 修复优先级建议

1. **P2 系列先诊断清楚**（导演输出/Trace/编剧混正文/后处理入口——这些直接影响「能否验收 B1」的体验）
2. **P1-3/4/5**（小而确定的接线 bug，改动小）
3. **P1-1/2**（reroll 清理逻辑、LogStore 回填，改动稍大）
4. **P2-6**（postprocess 截断，prompt 调优 + max_tokens）
5. **P3 系列**（非阻塞，可延后）

## 修复后填写

每个问题修复后在对应条目「状态」改为 🟢 并附 commit hash + 简要验证记录。

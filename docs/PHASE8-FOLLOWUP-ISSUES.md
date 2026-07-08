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

### P2-1 ⚪ 导演生成完成后，前端看不到导演输出了
- **现象**：原本可以点开查看「导演输出」（Director 的 plan）的入口不见了。生成过程中还能看到（director_progress delta 流），但完成后看不到结果。
- **待诊断**：可能 Director 的 plan/provenance 没正确存进 conversation node 的 provenance，或前端展开入口（trace 查看器）被 Phase 8 重构移除/丢失接线。需查 ChatMessage.vue 的 provenance 展示 + node 的 provenance 字段是否含 director plan。

### P2-2 ⚪ 流水线 Trace 在前端什么都看不到（导演生成过程中还能看到）
- **现象**：Pipeline Trace 面板/区域，导演生成过程中有内容，但完成后变空。
- **待诊断**：trace 是否依赖某个事件流在完成后被清空？或 trace 组件（DebugDrawer/PipelinePanel？）的 v-if 条件在完成态关闭？需查前端 trace 渲染的数据源和生命周期。

### P2-3 ⚪ 编剧（Editor）输出很久不出现
- **现象**：导演完成后，编剧输出要等很久，且最终混入了「改动说明」之类的非正文内容。
- **待诊断**：结合 idx 36 Editor 日志，Editor 1 轮完成（19515ms，1402 tokens）。需确认：(a) 慢是模型延迟还是 pipeline 阻塞；(b) 「改动说明混入正文」是 Editor prompt 没约束好还是前端没分离 editor commentary 和 final text。

### P2-4 🔴 编剧最后输出的「改动说明」混入了正文
- **现象**：Editor 的总结性/说明性文字（如「以上是合并后的成文」之类的元描述）出现在用户看到的正文里。
- **根因（部分）**：Editor prompt（`make_editor_config` 系统提示词）可能没明确禁止输出元描述；或 LLM 把思考过程写进了正文；或前端没做 editor commentary 与 final content 的分离。需查 Editor system prompt + 成文落库逻辑。
- **关键文件**：`crates/app-agent/src/prompts/`（Editor prompt）、`crates/app-pipeline/src/lib.rs`（Editor 落库）

### P2-5 🔴 后处理结果完全没有查看入口
- **现象**：后处理（postprocess）的知识/变量/任务/摘要写回了（round_summaries.json/tasks.json 有内容），但前端没有地方能看到这些结果。用户不知道在哪查看后处理产出。
- **修复方向**：Campaign 面板的知识/变量/任务/摘要 tab 应展示这些数据（CampaignKnowledgeTab/CampaignVariablesTab 等），需确认这些 tab 是否正确加载和渲染对应 store 数据。
- **关键文件**：`frontend/src/components-v2/campaign/`（各 tab 组件）

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

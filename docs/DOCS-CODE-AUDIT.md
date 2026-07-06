# 文档与代码对齐审计

> 状态：2026-07-06（含 Phase 4/5/6 阶段级核对 + 知识传播增量/封口 MVP、存储错误处理、测试隔离和 workspace clippy 闸门同步）
> 范围：核对 README、ROADMAP、HANDOFF（2026-06-18 已归档）、ARCHITECTURE-AUDIT、PLAN-* 与当前源码的一致性。
> 本轮同步包含代码事实更新：`AppState` 数据目录隔离、`CampaignStore` 写入错误传播/记录、集合级锁拆分、workspace/all-targets clippy 清理、知识传播封口 MVP、release checklist 初版。

## 结论

当前文档的大方向来自代码现状，核心架构判断成立；但 Campaign 主链路已经完成到 Phase 5，不能再按早期“写作流水线仍主要消费扁平 Character”的状态执行：

- Campaign 数据模型已经存在；开 Campaign 时写作流水线已通过 `CampaignRuntimeContext` 消费 instances / definitions / knowledge，未开 Campaign 时继续 fallback 到扁平 `Character`。
- `CampaignStore` 位于 `tauri-app`，下层 `app-agent` / `app-pipeline` 不应直接依赖它。
- `CampaignStore` 写入 API 已返回 `Result`；Tauri 命令路径会向前端返回结构化 `storage` 错误，postprocess 后台写回失败会记录 warning 而不中断当前写作。
- `CampaignStore` 已从单个全局缓存 Mutex 拆为 cards/campaigns/instances/knowledge/tasks/summaries/mvu 集合级锁；新增并发写回回放测试覆盖跨集合写入后重载一致性。
- Rust workspace 当前纳入 `cargo clippy --workspace --all-targets -- -D warnings` 闸门；少量高参数公共流程入口保留局部 allow，后续若重构 API 应单独立项而不是混入 warning 清理。
- 通过纯 domain DTO `CampaignRuntimeContext` 下传 Campaign 运行态，是符合当前 crate 分层的改造路径。
- Meta、MVU、Android、前端计划多数是基于已有雏形的后续计划，不是当前已完成能力。
- 临场角色已完成后端落盘闭环和前端升格入口：临时 instance 会在成功写作结果的 postprocess 前写入 CampaignStore，并可被下一轮读取；前端会展示 `is_temporary` 标记，并提供“升格为常驻”按钮。

文档可以继续作为后续执行依据，但执行前应注意本文列出的“规划性内容”和“缺口”。

## 已核对为代码事实

### Workspace 和命令数量

- `Cargo.toml` 当前 workspace members 为 14 个 crate。
- `crates/tauri-app/src/lib.rs` 当前存在 110 个 `#[tauri::command]` 标注的 Tauri command。
- README 中“Rust workspace，14 个 crate”和“110 个命令”与当前代码一致。

### Campaign 写作主链路

已核对文件：

- `frontend/src/App.vue`
- `frontend/src/tauri-api.js`
- `crates/tauri-app/src/lib.rs`
- `crates/app-pipeline/src/lib.rs`
- `crates/app-agent/src/tools.rs`
- `crates/app-agent/src/runtime.rs`

代码事实：

- `frontend/src/App.vue::startWriting` 会先计算 `charIdForWriting = writingMode.value === 'campaign' ? null : activeChar.value?.id`，Campaign 模式不再把 active character id 传入写作入口，兼容模式才沿用扁平 `Character`。
- `frontend/src/tauri-api.js::startWriting` 调用 Tauri command `start_writing`。
- `crates/tauri-app/src/lib.rs::start_writing` 存在，并从 `snapshot_tool_ctx()` 构造写作上下文。
- `crates/tauri-app/src/lib.rs::fill_campaign_context` 填充 `campaign_id`、`turn`、`pending_tasks`、`story_clock`，**阶段 2 已扩展**：开头先清空旧 runtime 防 stale，然后从 CampaignStore 加载 instances、definitions、knowledge，组装 `Arc<CampaignRuntimeContext>` 写入 `ctx.campaign_runtime` 并同步到 `tool_ctx`。
- `crates/tauri-app/src/lib.rs::AppState` **已持有 `data_dir`**：生产启动仍使用 OS 标准数据目录；测试可通过 `new_for_test()` 使用临时目录，避免本机真实角色/active Campaign 污染单元测试。
- `crates/app-pipeline/src/lib.rs::WritingContext` **阶段 2 已新增** `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。字段列表为 `characters/world_info/conversation_id/campaign_id/turn/pending_tasks/story_clock/profile/modules/recent_messages/campaign_runtime`。
- `crates/app-pipeline/src/lib.rs::build_director_tail` **阶段 3 已改造**：有 `campaign_runtime` 时从 instances 渲染（含 id/role_type/persona 摘要 + instance variables），campaign 全局变量注入 volatile tail，UTF-8 安全截断（`truncate_chars` 按 char 而非 byte）；无时退回旧的扁平 Character 名称列表。pending tasks 注入不变。
- `crates/app-pipeline/src/lib.rs::has_available_characters` **阶段 3 新增**：兼容 Campaign（instances 非空）和旧路径（characters 非空），`start_writing` 和 `regenerate` 共用。错误文案兼容两条路径。
- `crates/app-pipeline/src/lib.rs::DIRECTOR_SYSTEM_PROMPT` **阶段 3 已更新**：character_id 规则区分 Campaign 实例（用 instance_id）和旧路径（用角色名）。
- `crates/app-agent/src/tools.rs::ToolContext` **阶段 2 已新增** `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。字段列表为 `characters/world_info/vector_store/archived_summaries/campaign_runtime`。
- `crates/app-agent/src/tools.rs` 导演 `get_character` **阶段 3 已改造**：有 `campaign_runtime` 时优先查实例（返回 id/instance_id/definition_id/role_type/persona/behavior/variables/is_temporary），查不到 fallback 到旧扁平 Character。
- `crates/app-agent/src/tools.rs` 子 Agent `get_character` **阶段 4 已改造**：有 `current_character_instance_id` 时只返回该 instance 的数据，不允许查其他角色（信息隔离）。无时退回旧扁平 Character。
- `crates/app-agent/src/tools.rs::ToolContext` **阶段 4 新增** `current_character_instance_id: Option<Id>`：子 Agent 绑定的 instance id，用于 get_character 信息隔离。导演/编剧/无 Campaign 时为 None。
- `crates/app-agent/src/runtime.rs::spawn_subagents` **阶段 4 已改造**：接收 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`，按 character_id 匹配 instance（id 优先，name 兜底），使用 resolved persona/behavior 构造 system，注入该 instance 的 knowledge（信息隔离）和 variables，为每个子 Agent 构造独立 ToolContext（绑定 `current_character_instance_id`）。未匹配时 fallback 到旧 context_package 并 warn。无 campaign_runtime 时走旧路径。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` **阶段 5 已改造**：知识/变量写入先解析到已持久化 `CharacterInstance.id`，`present_chars` 真正用于落盘校验；不出场角色的知识不写入，非在场 instance 的变量写入被跳过并 warn；task 状态更新会校验 task 属于当前 campaign。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome` **已补齐写入错误处理**：summary/knowledge/变量/task 写回失败会记录 warning，不再静默丢失。
- `crates/tauri-app/src/campaign_store.rs::pressure_sync_json_io_across_collections` **已补同步 JSON I/O ignored 压测**：4 集合并发写入并记录 p50/p95/max；本机 500 次/集合通过，p95 < 9ms、max 约 28ms。桌面小/中等数据量暂不阻塞，Android/真实长会话仍需复测。
- `crates/tauri-app/src/lib.rs::list_character_knowledge` **已补可解释链路 DTO**：返回 `character_name`、`source_character_name`、`source_knowledge_id`、`relay_chain_text`、`provenance_text`；前端知识面板会展示“谁知道、从哪知道、哪轮知道”，并在有上游知识时展示 A→B→C 传话链。
- `crates/domain/src/character_knowledge.rs::CharacterKnowledgeEntry::source_knowledge_id` **已补传话链 MVP**：Tauri 写回层会在 `ToldByOther`/广播写入时匹配来源角色已有知识并链接上游条目。该能力是文本匹配级链路，完整语义传播仍需真实 LLM 行为评测。
- `crates/domain/src/character_knowledge.rs::PropagationPolicy` **已补秘密封口 MVP**：postprocess 可输出 `propagation: "private"`；Tauri 写回层会阻断匹配私有来源知识的告知/广播；前端知识面板展示封口标记。该能力是文本匹配级门禁，完整语义可靠性仍需真实 LLM 对抗评测。
- `crates/harness-real-llm/tests/knowledge_propagation_real_llm.rs` **已补发布前 ignored 评测**：真实 PostProcessor 抽取定向告知、身份组广播、private 封口后，再走线上 normalize 写回验证传话链和封口阻断。普通测试不联网；发布前用 `cargo test -p harness-real-llm knowledge_propagation -- --ignored --nocapture` 手动运行。
- `crates/domain/src/conversation.rs::SubagentSnapshot` **阶段 5 已扩展**：新增 `character_instance_id: Option<String>`、`display_name: Option<String>`、`fallback_reason: Option<String>`（serde 兼容旧数据）。
- `crates/app-conversation/src/lib.rs::build_provenance_with_campaign` **阶段 5 新增**：接收 `CampaignRuntimeContext`，从 Performance 中提取 instance 信息填充 SubagentSnapshot。旧 `build_provenance` 保留向后兼容。
- `crates/tauri-app/src/lib.rs::CharacterInfo` **阶段 5 已扩展**：新导入卡保存 `source_character_id: Option<String>`，启动恢复时可保留 domain `Character.id`；旧数据 fallback 到 `StoredCharacter.id`。
- `crates/tauri-app/src/lib.rs::delete_character` **阶段 5 已修复**：级联删除时同时尝试 `StoredCharacter.id`、持久化 `source_character_id`、同会话 `tool_ctx` domain id，避免旧代码只用 StoredCharacter.id 查 card 导致静默失败。
- `crates/app-agent/src/runtime.rs::build_campaign_subagent_system` / `build_campaign_subagent_volatile` **阶段 4 cleanup 新增**：纯函数，测试可直接断言 prompt 内容（persona/behavior/knowledge 隔离/variables）。
- `crates/domain/src/campaign.rs::CharacterInstance::temporary_with_overrides` **阶段 6 已完成**：创建临时 instance 时可传入 persona/behavior override。
- `crates/domain/src/campaign_runtime.rs::CampaignRuntimeContext::with_temporaries_for` **阶段 6 已完成**：为未匹配的 character_id 创建临时 CharacterInstance（`is_temporary=true`，支持 persona/behavior override），返回更新后的 context 和临时 instance 列表供调用者持久化；同一批次内重复 unmatched character 会去重。
- `crates/app-pipeline/src/lib.rs::PipelineOrchestrator::pending_temporary_instances` **阶段 6 已完成**：存储本轮创建的临时 instance，Tauri 层通过 getter 读取后落盘；`start_writing` / `regenerate` 开始时会清空旧 pending，避免失败或重试污染下一轮。
- `crates/app-pipeline/src/lib.rs::start_writing` / `regenerate` **阶段 6 已完成**：从 Director 的 `context_package.character_brief` 提取 persona 注入临时 instance，存储到 `pending_temporary_instances`。
- `crates/tauri-app/src/lib.rs::persist_temporary_instances_to` **阶段 6 已完成**：只在 pipeline 返回 `Ok` 后、postprocess 之前把临时 instance 写入 CampaignStore；会跳过同 campaign 已存在同名 instance、同批重复临时 instance，以及 `campaign_id` 不匹配的临时 instance。落盘后 postprocess 知识/变量写回不再被跳过。
- `request_ad_hoc_character` 工具**未实现**：当前用 unmatched character_id 自动触发，Director 的 character_brief 作为 persona 注入。
- `frontend/src/components/CampaignPanel.vue` **临时 instance UI 已实现**：展示 `is_temporary` 标记，提供升格为常驻按钮（带确认对话框、loading 状态、detail 刷新），`promoteTemporaryInstance` 从 `tauri-api.js` 导入。

### Agent Profile Config（可配置 Agent 运行时参数）

- `crates/domain/src/agent_profile_config.rs` **已实现**：`AgentRunConfig`（model_override/max_tool_rounds/tool_whitelist）+ `AgentProfileConfig`（agent_configs/max_concurrent_subagents/enable_postprocess/enable_summarizer/source/config_version）+ 内置默认 `builtin-default-agent-v1`。支持 Subagent 通配符回退。serde 兼容旧/部分 JSON。
- `crates/tauri-app/src/module_store.rs::AgentProfileConfigStore` **已实现**：JSON 文件 CRUD（`agent_profile_configs.json` + `active_agent_profile_config.json`），内置默认始终可用不可删除。
- `crates/tauri-app/src/lib.rs` **已实现**：6 个 Tauri commands（list/get/get_active/save/delete/set_active agent_profile_configs）。`fill_agent_profile_context` 加载活跃配置到 `WritingContext`。
- `crates/app-pipeline/src/lib.rs::WritingContext` **已扩展**：`agent_profile_config: Option<AgentProfileConfig>`。`make_director_config` / `make_editor_config` 从 profile 读取 model_override 和 max_tool_rounds。`spawn_subagents` 接收 max_concurrent_subagents 和 agent_profile_config；并发配置为 `0` 时通过 `effective_max_concurrent_subagents()` 按 `1` 执行，避免调度挂死。Director 注册工具后用 `filter_registry_by_whitelist` 按 profile 过滤 tool_whitelist。
- `crates/app-agent/src/runtime.rs::spawn_subagents` **已扩展**：接受 `max_concurrent_subagents` 和 `agent_profile_config` 参数，子 Agent 按 profile 覆盖 model 和 max_tool_rounds；每个子 Agent 的 registry 按 profile `tool_whitelist` 过滤。
- `crates/app-agent/src/tools.rs` **已扩展**：`ToolRegistry::retain(Option<&[String]>)` 按白名单保留工具（None=不动，Some([])=清空，Some(list)=只保留列表）；公共 helper `filter_registry_by_whitelist` 对未知工具名记 warning 后忽略、不 panic。过滤后 `tool_specs()`（发给 LLM）与 `dispatch` 同步收窄，被禁用的工具 dispatch 返回 `ToolError::NotFound`，不可绕过 whitelist。
- `crates/app-agent/src/pipeline_postprocess.rs::run_postprocess_pipeline` **已扩展**：新增 `enable_postprocess` / `enable_summarizer` / `agent_profile_config` 参数；某开关 false 时跳过对应 LLM 调用、对应字段为 None；两者都 false 时两个子 future 都不发请求（安静跳过）。
- `crates/app-agent/src/postprocess.rs::run_postprocess` / `summarizer.rs::run_summarizer` / `prompts/{postprocess,summarizer}.rs::make_*_config` **已扩展**：接收 `Option<&AgentProfileConfig>`，PostProcessor/Summarizer 的 model/rounds 按 profile 覆盖；PostProcessor 注册后按 profile `tool_whitelist` 过滤。
- `crates/app-pipeline/src/lib.rs::run_postprocess` **已扩展**：从 profile 读 `enable_postprocess`/`enable_summarizer` 传给 `run_postprocess_pipeline`；两者都关时不发 `PostProcessStarted`、改发新的 `PipelineEvent::PostProcessSkipped`（区别于真失败的 `PostProcessFailed`）；单关 summarizer 时不发 `SummaryDone`；无 config 时全开（向后兼容）。
- `crates/domain/src/agent.rs::PipelineEvent` **新增** `PostProcessSkipped { reason }` 变体（serde 兼容），Tauri 序列化为 `postprocess_skipped`。
- `frontend/src/tauri-api.js` **已实现**：6 个 wrapper（listAgentProfileConfigs/getAgentProfileConfig/getActiveAgentProfileConfig/saveAgentProfileConfig/deleteAgentProfileConfig/setActiveAgentProfileConfig）。
- `frontend/src/components/AgentProfileManager.vue` **已实现**：power 模式下的 AgentProfileConfig 管理 UI（列在 `AgentConfigCard` 之后）。支持列/切活跃/复制/删除/编辑/保存；可编辑 name/description/max_concurrent_subagents/enable_postprocess/enable_summarizer 以及 Director/Editor/Subagent:*/Summarizer/PostProcessor 各自的 model_override/max_tool_rounds/tool_whitelist（逗号分隔）。内置默认只读不可删除。注意：`AgentConfigCard.vue` 是 PromptProfile 模块选择器（另一套体系），不是 AgentProfileConfig 编辑器。
- `enable_postprocess` / `enable_summarizer` / `tool_whitelist` **均已运行时消费**（不再是「存储但未消费」）。

因此 `ARCHITECTURE-AUDIT.md` 和 `PLAN-CAMPAIGN-MAINLINE.md`（已归档）的主线判断与代码一致。

### Domain 模型

已核对文件：

- `crates/domain/src/campaign.rs`
- `crates/domain/src/character.rs`
- `crates/domain/src/character_knowledge.rs`
- `crates/domain/src/variables.rs`
- `crates/domain/src/story_task.rs`
- `crates/domain/src/mvu_translation.rs`

代码事实：

- `Campaign`、`CharacterInstance`、`CharacterDefinition`、`CharacterKnowledgeEntry`、变量模型、任务模型、`MvuTranslation` 均存在。
- `CharacterInstance` 当前只有 `persona_override` 和 `behavior_override`，没有 `backstory_override` 或 `variable_schema` 字段。
- `CharacterInstance::resolved_persona(definition)` / `resolved_behavior(definition)` 接收 `Option<&CharacterDefinition>`，override 优先，无 override 时 fallback 到 definition（阶段 1 已实现）。
- `CharacterDefinition` 持有 `persona_prompt`、`behavior_rules`、`base_backstory`、`variable_schema`。

本次已修正 `PLAN-CAMPAIGN-MAINLINE.md`（已归档）：`resolved_backstory` / `resolved_variable_schema` 不应被写成当前实例天然字段，建议作为 `CampaignRuntimeContext` helper，除非先显式新增 instance override 字段。

### 角色识别（Character Extraction）

已核对文件：

- `crates/app-agent/src/character_extractor.rs`
- `crates/app-agent/src/prompts/character_extractor.rs`

代码事实：

- `extract_characters(runtime, character, mvu_schema, cancel)` 已实现：调 `AgentRuntime::run_tool_loop` 跑识别 Agent，产出 `Vec<CharacterDefinition>`。
- `parse_character_definitions_from_response(resp)` 已实现：5 层兜底解析（emit_characters 工具调用 / 整体 JSON / ```json 代码块 / 裸代码块 / 手写括号配平）。
- `attach_definitions_to_card(definitions, card_id)` 已实现：回填 card_id。
- `CHARACTER_EXTRACTOR_SYSTEM_PROMPT`、`make_character_extractor_config()`（max_tool_rounds: 8）、`build_character_extractor_user_msg()`、`register_character_extractor_tools()`（emit_characters 工具）均在 `prompts/character_extractor.rs`。
- MVU schema 合并：`merge_schema(&default_character_variables(), mvu_schema)` 在识别完成后回填。
- 角色类型解析支持英文（protagonist/supporting/extra）和中文（主角/临场/龙套）。
- 11 个测试全部通过（含 MockLlmClient 端到端闭环）。

详细计划见 `docs/PLAN-CHARACTER-EXTRACTION.md`。

### Meta Agent 和 MVU

已核对文件：

- `crates/app-meta/src/lib.rs`
- `crates/app-meta/src/meta_conversation.rs`
- `crates/app-meta/src/mvu_import.rs`
- `crates/app-meta/src/prompts/meta_agent.rs`
- `crates/app-meta/src/prompts/mvu_analyzer.rs`
- `crates/tauri-app/src/lib.rs`
- `frontend/src/components/MetaPanel.vue`

代码事实：

- `MetaSession`、`PatchStore`、Meta runtime tools 存在。
- `meta_accept_patch`、`meta_analyze_mvu_card`、`meta_list_mvu_translations` 等 Tauri commands 存在。
- `mvu_import::analyze_mvu_card` 会产出 `MvuTranslation`，解析失败时走 `pure_data_fallback`。
- `frontend/src/components/MetaPanel.vue` 已展示 Meta 聊天、tool result、pending patches、MVU translations、Campaign health check（侧栏"Campaign 体检"区块，调 `meta_health_check` Tauri command）。

因此 `PLAN-META-AGENT.md`（已归档）和 `PLAN-PLUGIN-MVU.md` 的”当前事实”基本准确；其中 health check（后端 + 前端 MetaPanel 展示）已完成，generation explanation、typed patch preview、schema apply、runtime fallback 是后续计划。

### 前端工作台

已核对文件：

- `frontend/src/App.vue`
- `frontend/src/components/CampaignPanel.vue`
- `frontend/src/components/StreamingMessage.vue`
- `frontend/src/components/ChatMessage.vue`
- `frontend/src/components/MetaPanel.vue`
- `frontend/src/components/MvuStatusBar.vue`
- `frontend/src/components/CharacterDetail.vue`
- `frontend/src/tauri-api.js`

代码事实：

- `App.vue` 已有 `activeCampaign`，mounted 时调用 `getActiveCampaign()`。
- 主写作入口按 `writingMode` 区分 Campaign / legacy：Campaign 模式传 `null` characterId，legacy 模式传 `activeChar.value?.id`。
- `CampaignPanel.vue` 已有 Campaign、instances、variables、knowledge、tasks、summaries 相关入口。
- `StreamingMessage.vue` 承接写作过程流式渲染；`ChatMessage.vue` 负责 provenance 展示与基于 stable id 的 reroll 入口。
- `MvuStatusBar.vue` 已存在，并在 `CharacterDetail.vue` 中使用。

因此 `PLAN-FRONTEND-WORKBENCH.md`（已归档）的主要判断成立。

### Android/Tauri

已核对文件：

- `crates/tauri-app/tauri.conf.json`
- `crates/tauri-app/capabilities/default.json`
- `crates/tauri-app/gen/android/app/src/main/AndroidManifest.xml`
- `crates/tauri-app/gen/android/app/src/main/java/com/storyforge/app/MainActivity.kt`

代码事实：

- Tauri v2 配置文件存在。
- capability 当前包含 `core:default`、`fs:default`、`dialog:default`。
- Android Manifest 包含 `INTERNET`、`MainActivity`、`FileProvider`。
- `MainActivity.kt` 调用 `enableEdgeToEdge()`。

因此 `PLAN-ANDROID.md` 的当前事实准确。

## Phase 4/5/6 阶段级状态核对（2026-06-18）

> 本节按各 PLAN 的阶段拆分核对真实代码状态，补充上方"Campaign 写作主链路"等已核对事实未覆盖的阶段级粒度。整体完成度：Phase 4 ✅ 已完成、Phase 5 ✅ 已完成、Phase 6 ~5%。

### Phase 4：前端工作台（✅ 已完成）

前端栈：Vue 3 + Vite + Tailwind v4 + Tauri 2，代码在 `frontend/src/`。

| 阶段 | 计划目标 | 真实状态 | 证据路径 |
|---|---|---|---|
| 1 首屏聚焦 active campaign | AppHeader 显示 campaign 名/轮次/实例数，无 campaign 时给 CTA | 已实现 | `frontend/src/components/AppHeader.vue` 展示 campaign 名/轮次/实例数；无 campaign 时显示 CTA 按钮 |
| 2 写作入口绑定 active campaign | 区分 campaign 写作 vs 旧 activeChar 路径 | 已实现 | `App.vue` 三态 writingMode（campaign/legacy/none）；Campaign 模式传 null characterId |
| 3 CampaignPanel 拆工作台 tabs | 拆成独立 `Campaign*Tab.vue` | 已实现 | W5 拆为独立的 instances/knowledge/tasks/summaries tab 组件 |
| 4 Pipeline trace 用 instance 展示名 | 保留 `instance_id`、显示名 + role_type、按 stable id reroll | 已实现 | `StreamingMessage.vue` 承接过程流式展示；`ChatMessage.vue` provenance 使用 subagent display name / instance id，reroll 用稳定 id |
| 5 MetaPanel Campaign health | 健康摘要、一键 context、patch diff、accept 刷新 tabs | 已实现 | W5 baseline + W9 补完 accept 后变量 tab 刷新 |
| 6 移动端布局 | 响应式 sheet、固定输入、无溢出 | 已实现 | Pipeline 默认折叠、CharacterList 底部 sheet |

计划外已建（built-but-not-in-plan）：

- 临时 instance 升格 UI：`CampaignPanel.vue` "升格为常驻角色"（带确认/loading/detail 刷新），`promoteTemporaryInstance` 取自 `tauri-api.js`。
- role_type 徽章（Protagonist/Supporting/Extra）+ "识别角色" extract 入口，在 `CampaignPanel.vue`。

### Phase 5：MVU + ST 导入/导出（✅ 已完成）

缩写在项目中的含义：**MVU** = SillyTavern 角色卡扩展，提供声明式状态栏/变量 schema（`ui_bindings`/`variable_schema`）；**ST** = SillyTavern（角色卡 V2/V3 + PNG `chara` block）。

| 阶段 | 计划目标 | 真实状态 | 证据路径 |
|---|---|---|---|
| MVU-1 | 冻结通用插件扩张、划边界 | 已实现（文档层定边界） | `docs/PLAN-PLUGIN-MVU.md` |
| MVU-2 | MVU Translation → variable schema diff 预览 | 已实现 | 预览逻辑 `MvuApplyPreview`/`compute_apply_preview` 存在；`meta_preview_mvu_apply` 命令 + 前端 `metaPreviewMvuApply` 已接 |
| MVU-3 | 把 MVU schema patch apply 进 Campaign | 已实现 | `meta_apply_mvu_schema` 注册并算 preview+apply；W9 前接通 `tauri-api.js` + MetaPanel diff 展示 + 确认 apply |
| MVU-4 | 原生状态栏从 Campaign 变量渲染 | 已实现 | `MvuStatusBar.vue` 渲染 bar/text/tag/icon；用于 `CharacterDetail.vue`（未进 `CampaignPanel`） |
| MVU-5 | 混合 JS fallback runtime（WebView） | 已实现 | `infra-plugin-host` 保留纯 `MvuRuntime`/DTO/事件协议，Tauri/WebView adapter 在 `tauri-app/src/mvu_webview_runtime.rs`；W10 DI 注入 pipeline 并在 postprocess 调 `execute_fragment`；harness 传 None 降级 |
| MVU-6 | 插件权限/安全分层 | 部分 | `Permission` enum + `ensure_permission` 存在；未覆盖全部 5 计划层（无 `network_access` toggle 证据） |
| ST-T1 | 定 ST V2/V3 导入保真范围 | 部分 | `from_st_card()` 覆盖所列字段；部分 checkbox 未结 |
| ST-T2 | raw_json + extensions 保留策略 | 已实现（保留） | 两者以 `serde_json::Value` 保留；设计 Q 未结 |
| ST-T3 | 多角色识别 fallback | 部分 | `character_extractor` + `fallback_from_character` 存在；tauri-app fallback/用户提示/rerun 未确认 |
| ST-T4 | StoryForge Campaign 导出格式设计 | 已实现 | W7 `export_campaign` 命令 + JSON bundle（`format_version`）+ ST 卡 PNG tEXt 写入 + 共享 lorebook |
| ST-T5 | 评估导出回 ST 卡/Lorebook | 已实现 | W7 评估 checkbox 已结；ST 卡 PNG + lorebook 导出已落地 |

### Phase 6：Android（~5%）

| 阶段 | 计划目标 | 真实状态 | 证据路径 |
|---|---|---|---|
| AND-1 | 构建链基线（验 `cargo tauri android build`） | 未开始 | 仅 Tauri 自动生成脚手架 `crates/tauri-app/gen/android/`，无构建记录 |
| AND-2 | Android 系统选择器文件导入路径 | 未开始 | 无 Android 专属 import 改动 |
| AND-3 | 本地数据目录 + 迁移/schema 版本 | 未开始 | 无 debug data-dir 命令、无 schema/version 字段 |
| AND-4 | 长任务/流式/取消在移动端 | 未开始 | 无 Android 生命周期处理 |
| AND-5 | 移动端排障/诊断导出 | 未开始 | `app-logging::export_bundle` 仅日志；无 Android share-sheet/诊断包 |
| AND-6 | capability 权限收敛 | 未开始 | `capabilities/default.json` 仍粗（`core:default`/`fs:default`/`dialog:default`） |

注：Android 代码确实存在（`crates/tauri-app/gen/android/`，含 `MainActivity.kt`、`build.gradle.kts`），但是 Tauri v2 未改动的自动生成脚手架——`MainActivity.kt` 仅调 `enableEdgeToEdge()`，与 `PLAN-ANDROID.md`"当前事实"一致。PLAN-ANDROID 各阶段均未执行。

### 下一阶段建议

Phase 4 和 Phase 5 已全部完成。推荐下一步：

1. **Phase 6 Android 打磨**——构建链验证、文件导入、长文本流式、移动端排障。Tauri 脚手架就绪，重点验证主流程在 Android 端可用。
2. **Phase 7 收口/验收/发布准备**——端到端验收矩阵、回归测试固化、文档/发布准备。
3. **知识传播真实 LLM 评测**——方向 1/2/3/4/5 均已有 MVP；下一步应补广播、传话链、封口绕写的真实 LLM 行为评测，尤其验证文本匹配门禁/链路在改写表达时的边界。

## 已修正文档问题

1. `docs/archive/2026-06-19-completed-phases/PLAN-CAMPAIGN-MAINLINE.md`（已归档）
   - 原文容易让执行者以为 `CharacterInstance` 已有或应该直接承载 backstory/schema。
   - 已改为：persona/behavior 可在 instance method 中做 override + definition fallback；backstory/schema 优先作为 runtime/helper 读取 `CharacterDefinition`，除非显式新增 instance override 字段。

2. `docs/ROADMAP.md`
   - 原文把 Phase 5 “ST 兼容和导入/导出”的详细计划指向 `PLAN-PLUGIN-MVU.md`，但该计划只覆盖 MVU/plugin 方向，不覆盖完整 ST 导入/导出。
   - 已改为：`PLAN-PLUGIN-MVU.md` 只覆盖 MVU 状态栏、schema preview、JS fallback；进入 Phase 5 前应补 `docs/PLAN-ST-IMPORT-EXPORT.md`。

## 仍需补齐的文档缺口

### 1. ST 导入/导出专项计划 — 已补充（计划阶段）

`ROADMAP.md` Phase 5 包含：

- ST V2/V3 导入保真范围。
- raw JSON 和 extensions 保留策略。
- StoryForge Campaign 导出格式。
- 是否支持导出回 ST 卡或 Lorebook。

`docs/PLAN-ST-IMPORT-EXPORT.md` 已创建（2026-06-17），覆盖上述各项。当前为计划阶段，未实现。

### 2. Release checklist 和 user guide 只是未来产物

`PLAN-POST-MAINLINE.md` 提到：

- `docs/RELEASE-CHECKLIST.md`
- `docs/USER-GUIDE.md`

这两个文件当前不存在，且在计划中标为新增/如需要新增。执行者不应把它们当成当前文档。

### 3. 架构计划中的新类型尚未实现

以下名称是推荐目标，不是当前代码事实：

- ~~`CampaignRuntimeContext`~~ **阶段 2 已完成**：`crates/domain/src/campaign_runtime.rs` 包含 DTO + helpers，已接入 `WritingContext`/`ToolContext`，`fill_campaign_context` 已组装快照。
- `meta_explain_generation`
- `meta_preview_mvu_schema`
- `propose_apply_mvu_schema`
- `startCampaignWriting`
- ~~`PLAN-ST-IMPORT-EXPORT.md`~~ **已补充**（2026-06-17）：`docs/PLAN-ST-IMPORT-EXPORT.md` 已创建，覆盖 ST V2/V3 保真、extensions 保留、识别 fallback、Campaign 导出格式、ST 回导评估。当前为计划阶段。

执行时应按计划新增或替换，不要在当前代码中搜索不到就判定任务失败。

## 对小模型执行的补充规则

- 先读本文，再读对应 `PLAN-*.md`。
- 把“当前事实”与“任务/目标”分开理解。
- 如果计划里提到的文件存在但符号不存在，先判断它是不是计划要求新增的符号。
- 如果计划要求修改 `CharacterInstance`，必须先看当前结构字段，不能凭文档臆造已有字段。
- Phase 5 ST 导入/导出开始前，先补 `PLAN-ST-IMPORT-EXPORT.md`，不要用 `PLAN-PLUGIN-MVU.md` 代替。
- 执行代码改动后要回写本审计报告或对应计划的状态，避免文档再次漂移。

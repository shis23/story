# 文档与代码对齐审计

> 状态：2026-07-06（含 Phase 4/5/6 阶段级核对 + 知识传播增量/封口 MVP、存储错误处理、测试隔离和 workspace clippy 闸门同步）
> 范围：核对 README、ROADMAP、HANDOFF（2026-06-18 已归档）、ARCHITECTURE-AUDIT、PLAN-* 与当前源码的一致性。
> 本轮同步包含代码事实更新：`AppState` 数据目录隔离、`CampaignStore` 写入错误传播/记录、集合级锁拆分、workspace/all-targets clippy 清理、知识传播封口 MVP、release checklist 初版。

## 结论

当前文档的大方向来自代码现状，核心架构判断成立；但 Campaign 主链路已经完成到 Phase 5，不能再按早期“写作流水线仍主要消费扁平 Character”的状态执行：

- Campaign 数据模型已经存在；开 Campaign 时写作流水线已通过 `CampaignRuntimeContext` 消费 instances / definitions / knowledge，未开 Campaign 时继续 fallback 到扁平 `Character`。
- `CampaignStore` 位于 `tauri-app`，下层 `app-agent` / `app-pipeline` 不应直接依赖它。
- `CampaignStore` 写入 API 已返回 `Result`；Tauri 命令路径会向前端返回结构化 `storage` 错误，postprocess 后台写回已通过 `persist_postprocess_outcome_async` 移入 `spawn_blocking`，失败会记录 warning 而不中断当前写作。
- `CampaignStore` 已从单个全局缓存 Mutex 拆为 cards/campaigns/instances/knowledge/tasks/summaries/mvu 集合级锁；新增并发写回回放测试覆盖跨集合写入后重载一致性。
- Rust workspace 当前纳入 `cargo clippy --workspace --all-targets -- -D warnings` 闸门；少量高参数公共流程入口保留局部 allow，后续若重构 API 应单独立项而不是混入 warning 清理。
- 通过纯 domain DTO `CampaignRuntimeContext` 下传 Campaign 运行态，是符合当前 crate 分层的改造路径。
- Meta、MVU、Android、前端计划多数是基于已有雏形的后续计划，不是当前已完成能力。
- 临场角色已完成后端落盘闭环和前端升格入口：临时 instance 会在成功写作结果的 postprocess 前通过 `persist_temporary_instances_async` 写入 CampaignStore，并可被下一轮读取；前端会展示 `is_temporary` 标记，并提供“升格为常驻”按钮。

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
- `crates/tauri-app/src/lib.rs::fill_campaign_context_async` 填充 `campaign_id`、`turn`、`pending_tasks`、`story_clock`，**阶段 2 已扩展**：开头先清空旧 runtime 防 stale，然后在 `spawn_blocking` 中从 active Campaign / CampaignStore 加载 instances、definitions、knowledge，组装 `Arc<CampaignRuntimeContext>` 快照，回到 async 主线写入 `ctx.campaign_runtime` 并同步到 `tool_ctx`。同步版 `fill_campaign_runtime_from_store` 仍作为 harness/测试入口保留。
- `crates/tauri-app/src/lib.rs::AppState` **已持有 `data_dir`**：生产启动仍使用 OS 标准数据目录；测试可通过 `new_for_test()` 使用临时目录，避免本机真实角色/active Campaign 污染单元测试。
- `crates/app-pipeline/src/lib.rs::WritingContext` **阶段 2 已新增** `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。字段列表为 `characters/world_info/conversation_id/campaign_id/turn/pending_tasks/story_clock/profile/modules/regex_scripts/campaign_runtime/agent_profile_config`。对话历史不再存成上下文字段，而是在流水线中通过 `ConversationStore::recent_messages_as_chat()` 按需读取。
- `crates/app-pipeline/src/lib.rs::build_director_tail` **阶段 3 已改造**：有 `campaign_runtime` 时从 instances 渲染（含 id/role_type/persona 摘要 + instance variables），campaign 全局变量注入 volatile tail，UTF-8 安全截断（`truncate_chars` 按 char 而非 byte）；无时退回旧的扁平 Character 名称列表。pending tasks 注入不变。2026-07-06 增量：会调用 `WorldInfoBook::triggered_selective_entries(intent)`，将本轮写作意图命中的 Selective/Both 世界书条目注入 volatile tail，保持 Constant/Both 常驻设定仍在 system。
- `crates/app-pipeline/src/lib.rs::has_available_characters` **阶段 3 新增**：兼容 Campaign（instances 非空）和旧路径（characters 非空），`start_writing` 和 `regenerate` 共用。错误文案兼容两条路径。
- `crates/app-pipeline/src/lib.rs::DIRECTOR_SYSTEM_PROMPT` **阶段 3 已更新**：character_id 规则区分 Campaign 实例（用 instance_id）和旧路径（用角色名）。
- `crates/app-agent/src/tools.rs::ToolContext` **阶段 2 已新增** `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`。字段列表为 `characters/world_info/vector_store/archived_summaries/campaign_runtime`。
- `crates/domain/src/world_info.rs` **2026-07-06 增量**：`triggered_selective_entries()` 与 `search_by_keywords()` 统一使用 route-aware 关键词规则，跳过 disabled/Constant-only 条目，支持 primary keys、secondary keys 以及 AND/OR/NOT selective logic；`search_world_info` 工具因此只返回可触发的 Selective/Both 匹配。
- `crates/app-agent/src/tools.rs` 导演 `get_character` **阶段 3 已改造**：有 `campaign_runtime` 时优先查实例（返回 id/instance_id/definition_id/role_type/persona/behavior/variables/is_temporary），查不到 fallback 到旧扁平 Character。
- `crates/app-agent/src/tools.rs` 子 Agent `get_character` **阶段 4 已改造**：有 `current_character_instance_id` 时只返回该 instance 的数据，不允许查其他角色（信息隔离）。无时退回旧扁平 Character。
- `crates/app-agent/src/tools.rs::ToolContext` **阶段 4 新增** `current_character_instance_id: Option<Id>`：子 Agent 绑定的 instance id，用于 get_character 信息隔离。导演/编剧/无 Campaign 时为 None。
- `crates/app-agent/src/runtime.rs::spawn_subagents` **阶段 4 已改造**：接收 `campaign_runtime: Option<Arc<CampaignRuntimeContext>>`，按 character_id 匹配 instance（id 优先，name 兜底），使用 resolved persona/behavior 构造 system，注入该 instance 的 knowledge（信息隔离）和 variables，为每个子 Agent 构造独立 ToolContext（绑定 `current_character_instance_id`）。未匹配时 fallback 到旧 context_package 并 warn。无 campaign_runtime 时走旧路径。
- `crates/app-agent/src/runtime.rs::execute_tool_call` **已补畸形参数反馈**：普通、streaming、layout 三条 tool loop 统一先解析 `tool_call.function.arguments`；JSON 畸形时不会 dispatch 真实工具，而是写入 `{"error":"Invalid JSON arguments: ..."}` tool result 反馈给 LLM 修正。`test_malformed_tool_arguments_return_tool_error_without_dispatch` 固化该行为。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome_to_store` **阶段 5 已改造**：知识/变量写入先解析到已持久化 `CharacterInstance.id`，`present_chars` 真正用于落盘校验；不出场角色的知识不写入，非在场 instance 的变量写入被跳过并 warn；task 状态更新会校验 task 属于当前 campaign。
- `crates/tauri-app/src/lib.rs::persist_postprocess_outcome_async` **已补齐写入错误处理并 offload**：summary/knowledge/变量/task 写回失败会记录 warning，不再静默丢失；成功写作/重 roll 后的 postprocess 写回会在 `spawn_blocking` 中调用可测同步核心。
- `crates/tauri-app/src/campaign_store.rs::pressure_sync_json_io_across_collections` **已补同步 JSON I/O ignored 压测**：4 集合并发写入并记录 p50/p95/max；本机 500 次/集合通过，p95 < 9ms、max 约 28ms。2026-07-07 已先把写作/重 roll 的 Campaign 快照读取移入 `spawn_blocking`，并把临时 instance 落盘移入 `persist_temporary_instances_async`、postprocess summary/knowledge/variable/task 写回移入 `persist_postprocess_outcome_async`、对话变体采纳写盘移入 `accept_variant_async`、嵌入配置写盘移入 `configure_embedder_async`、归档消息读取移入 `archivable_messages_async`，降低 async Tauri 命令上的同步读写阻塞；桌面小/中等数据量暂不阻塞，Android/真实长会话、其他同步写入和后台 flush 仍需复测。
- `storyforge-infra-util::secret_store` + `crates/tauri-app/src/connection_store.rs` **已补 API key SecretRef 存储**：LLM 连接和 embedder key 通过 `keyring` 写入系统凭据库，`connections.json` / `embed.json` 只保留 `storyforge-secret:v1:*`；旧明文文件加载时迁移，真实 LLM harness 可解析 SecretRef。2026-07-06 已补平台 native store 显式初始化，Windows Credential Manager 写/读/删冒烟测试通过；`infra-util` Android arm64 交叉编译通过，Android 真机 keyring 仍待验证。
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
- `crates/domain/src/campaign_runtime.rs::CampaignRuntimeContext::with_temporaries_for` **阶段 6 已完成**：为未匹配的 character_id 创建临时 CharacterInstance（`is_temporary=true`，支持 persona/behavior override），返回更新后的 context 和临时 instance 列表供调用者持久化；同一批次内重复 unmatched character 会去重，空白 unmatched character_id 会被跳过。
- `crates/app-pipeline/src/lib.rs::PipelineOrchestrator::pending_temporary_instances` **阶段 6 已完成**：存储本轮创建的临时 instance，Tauri 层通过 getter 读取后落盘；`start_writing` / `regenerate` 开始时会清空旧 pending，避免失败或重试污染下一轮。
- `crates/app-pipeline/src/lib.rs::start_writing` / `regenerate` **阶段 6 已完成**：从 Director 的 `context_package.character_brief` 提取 persona 注入临时 instance，存储到 `pending_temporary_instances`。
- `crates/app-pipeline/src/lib.rs::PipelineOrchestrator::regenerate` **已补多目标 Subagent 重 roll 回归**：`PartialRollTarget::Subagent` 可一次传入多个角色；重跑结果按 `character_id` 回填旧 `subagent_results`，即使 target 顺序与旧 plan 顺序不同，也不会交换角色产出或打乱 provenance 顺序。
- `crates/tauri-app/src/lib.rs::persist_temporary_instances_async` **阶段 6 已完成并 offload**：只在 pipeline 返回 `Ok` 后、postprocess 之前把临时 instance 写入 CampaignStore；会跳过同 campaign 已存在同名 instance、同批重复临时 instance，以及 `campaign_id` 不匹配的临时 instance。落盘通过 `spawn_blocking` 执行并在继续 postprocess 前 await，保证知识/变量写回不再被跳过。
- `crates/tauri-app/src/lib.rs::accept_variant_async` **已 offload**：`accept_variant` Tauri async command 会在 `spawn_blocking` 中执行 `ConversationStore::accept_variant` 的原子写盘，成功后再沿用原有后台自动归档检查；`test_accept_variant_async_persists_final_variant` 覆盖内存和磁盘重载均为 Final。
- `crates/tauri-app/src/lib.rs::configure_embedder_async` **已 offload**：`configure_embedder` 改为 async Tauri command，`embed.json` 与 SecretRef 持久化通过可注入 secret store 的 blocking helper 执行，成功后才更新 `AppState.embed_config`；`test_configure_embedder_async_persists_secret_ref_and_updates_state` 覆盖磁盘 SecretRef、密钥恢复和内存配置更新。
- `crates/tauri-app/src/lib.rs::archivable_messages_async` **已 offload**：手动 `archive_conversation` 与自动归档检查共用该 helper，在 `spawn_blocking` 中读取 `ConversationStore` 并过滤 Discarded 变体；`test_archivable_messages_async_filters_discarded_variants` 覆盖归档消息快照不会包含已丢弃变体。
- `crates/tauri-app/src/lib.rs::postprocess_variable_keys` **已补实际变量 schema/value key 收集**：PostProcessor 变量提示不再只喂默认角色 key；写作/重 roll 共用 runtime 快照里的默认全局变量、当前 Campaign 变量、角色定义 `variable_schema` 与 instance 现有变量，避免 MVU/initvar 和高玩自定义字段漏给后处理 Agent。
- `frontend/src/components/base/documentKeydownController.js` **已统一 ESC 监听生命周期**：`BaseOverlay` / `BaseDropdown` 共用 document keydown controller，重复 enable 不累积 listener，关闭或卸载会释放；`BaseOverlay` 同时用 `immediate` watcher 覆盖初始打开态，并在卸载时释放 body overflow lock。`frontend/tests/base-keydown.test.mjs` 已纳入 `npm test`。
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
- `frontend/src/components/MetaPanel.vue` 已展示 Meta 聊天、tool result、pending patches、MVU translations、Campaign health check（侧栏"Campaign 体检"区块，调 `meta_health_check` Tauri command），并在 `App.vue` 传入最后一条带 provenance 的 assistant 节点时展示“解释上一条生成”入口（调 `meta_explain_generation`）。
- 旧 Meta patch 的 `execute_patch` 已在 2026-07-07 改为事务式工作副本执行：全部 action 成功才写回 `PatchContext`，失败时保留原 world info / character fields。
- Tauri/日志导出路径的 JSON 序列化失败已在 2026-07-07 改为结构化错误返回，`meta_accept_patch` 的 WorldInfoEntry 回写反序列化失败也不再静默丢条目。
- `LogStore` JSONL 落盘已在 2026-07-07 加专用互斥锁，并有多线程并发写入完整性测试覆盖。

因此 `PLAN-META-AGENT.md`（已归档）和 `PLAN-PLUGIN-MVU.md` 的”当前事实”基本准确；其中 health check、generation explanation、typed patch preview、schema apply 和 runtime fallback 的主路径已完成，后续重点是更广的真实卡/真实 LLM 验收与插件兼容层。

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
- capability 当前包含 `core:default`、`dialog:allow-open`、`dialog:allow-save`、`dialog:allow-message`、`dialog:allow-ask`、`fs:allow-read-file`、`fs:allow-write-file`；已移除粗粒度 `fs:default` / `dialog:default`。
- Android Manifest 包含 `INTERNET`、`MainActivity`、`FileProvider`。
- `MainActivity.kt` 调用 `enableEdgeToEdge()`。

因此 `PLAN-ANDROID.md` 的当前事实准确。

## Phase 4/5/6 阶段级状态核对（2026-06-18）

> 本节按各 PLAN 的阶段拆分核对真实代码状态，补充上方"Campaign 写作主链路"等已核对事实未覆盖的阶段级粒度。整体完成度：Phase 4 ✅ 已完成、Phase 5 ✅ 已完成、Phase 6 ~20%。

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

2026-07-06 增量核对：`Preset::from_st()` 的 Preset `extensions.regex_scripts` 导入已保留 ST 正则元数据，包括原始 `placement_codes`、`markdown_only`、`prompt_only`、`run_on_edit`、`substitute_regex`、`trim_strings`、`min_depth`、`max_depth`。`PresetStore` 已持久化 `active_preset.json` 并暴露 `get_active_preset` / `set_active_preset`；预设面板可设置/清除运行时预设。`GlobalRegexStore` 已持久化 `global_regex_scripts.json`，并暴露 `import_global_regex_settings` / `list_global_regex_scripts` / `clear_global_regex_scripts` / `update_global_regex`；导入时可从 ST settings JSON 中抽取嵌套 `regex_scripts` 并标记为 `RegexScriptSource::Global`。预设面板已接入 Global 正则导入、查看、启停、清空 UI。写作与重 roll 会按 Global → Preset → Scoped 顺序把 Global 正则、active Preset 正则和 Scoped 正则合并进 `WritingContext.regex_scripts`。

2026-07-06 增量核对：角色卡 Scoped `data.extensions.regex_scripts` 可通过 `Character::scoped_regex_scripts()` typed 读取，复用 Preset 正则解析与元数据保真逻辑；`RegexScriptSource` 和 `merge_regex_script_sources()` 已按 Global → Preset → Scoped 顺序合并并标记来源。2026-07-06 增量：Tauri legacy 写作会从本次选中的角色卡收集 Scoped 正则注入 `WritingContext.regex_scripts`，并已通过 `CharacterInfo.extensions` 持久化卡内扩展，避免重启后丢失 scoped regex；active Preset 与 Scoped 会按 Preset → Scoped 顺序合并。2026-07-06 追加：`CharacterCard::scoped_regex_scripts()` 会从 Campaign 卡的 `raw_card_json.extensions.regex_scripts` 解析卡内 Scoped 正则；`fill_campaign_runtime_from_store()` 在组装 active Campaign 快照时会追加这些脚本，并跳过同 ID 的既有 Scoped 脚本，避免 legacy/Campaign 双路径重复执行。

2026-07-06 增量核对：`storyforge-infra-regex` 已支持 ST 常见 slash-delimited `findRegex`（如 `/^foo/gm`），执行前会提取 pattern、合并 inline flags 与 `flags` 字段，并保持 raw pattern + `flags` 字段的旧行为。`crates/app-pipeline` 已接入正则执行器：首写和重 roll 会在导演前执行 Input 正则、编剧成文落盘/返回前执行 Output 正则。Input/Output 过滤已优先尊重 ST 原始 `placement_codes`，当前 ST `[1,2]` 会在用户输入与 AI 输出两端执行，只有未保留原始数组的旧数据才回退到二元枚举；Slash/Reasoning 等后续执行点已在 2026-07-07 的增量中部分补齐，其中 Slash placement 3 覆盖 `/` 前缀输入最小 hook，完整 Slash 命令注册、参数管道和 PluginHost/JS 斜杠运行时语义仍未实现。

2026-07-07 追加：正则替换语义已按 ST/JS flags 修正，只有合并后的 flags 含 `g` 才执行全局替换；不含 `g` 时只替换首个匹配。真实卡中常见的 `/(.*)/s` 输入包裹脚本因此不会再因为结尾空匹配被重复包裹。

2026-07-07 增量核对：`storyforge-infra-regex` 新增 `RegexExecutionTarget::{Prompt,Persisted,Display}` 与 `apply_regex_scripts_for_target`。`promptOnly=true` 的脚本只在 Prompt 目标执行；`markdownOnly=true` 的脚本只在 Display 目标执行，不再污染提示词或持久化文本。`app-pipeline` 的 Input 正则显式走 Prompt 目标，Output 落盘/返回前正则走 Persisted 目标；Tauri `get_conversation` 已返回派生 `display_content`，前端消息展示使用 `display_content`、编辑仍使用原始 `content`。2026-07-07 追加：`ChatMessage` 已通过 `RichContent` 接入派生 HTML 片段安全渲染，只有 display-only 派生内容且命中常见 HTML 标签时才走 DOMPurify，原始 `<data_block>` 仍保持转义文本显示。2026-07-07 续补：`PluginHost` 已从单一 HTML slot 改为 per-slot map，可隔离默认内容、slash、sidebar 和 statusbar，清空状态栏不会误清其他挂载内容。

2026-07-07 增量核对：`storyforge-infra-regex` 的执行器已支持 `minDepth/maxDepth` 包含式范围过滤。`app-pipeline` 当前生成/重 roll 的 Input/Output 按 ST depth 0 执行；Tauri `get_conversation` 生成 `display_content` 时按消息节点离末尾的深度执行 display-only 正则，因此老消息不会被只针对最近 N 条的脚本误替换。prompt 历史批量重写和 ST 冷门 Slash 参数管道仍未接线；Regex Slash placement 3 的 `/` 前缀输入最小 hook 与常用 Slash shim 见下方追加核对。

2026-07-07 增量核对：当前 ST placement 5 已映射到 `RegexPlacement::WorldInfo`，Preset/Scoped/Global 的 World Info 正则会在世界书内容进入 prompt 前执行。覆盖点包括 `build_director_system_extra` 的 Constant/Both 常驻世界书、`build_triggered_selective_lore` 的 Selective/Both 关键词触发世界书，以及 Director 工具 `search_world_info` 返回内容；执行结果只用于 prompt，不写回 `WorldInfoBook` 存储。2026-07-07 追加：ST regex placement 常量已与当前 SillyTavern 对齐：`1=User Input`、`2=AI Output`、`3=Slash Command`、`5=World Info`、`6=Reasoning`。`RegexPlacement` 与执行器已识别 Slash/Reasoning；主 app 已在 Input/Output/World Info hook 中调用，并把 Slash placement 3 接到 `/` 前缀用户/导演意图的最小 hook、Reasoning placement 接到 AI 输出的 `<think>` / `<thinking>` 推理块。

2026-07-07 追加核对：`storyforge-infra-regex` 新增 `apply_reasoning_regex_to_think_blocks_at_depth()`，只处理成对闭合且大小写不敏感的 `<think>...</think>` 与 `<thinking>...</thinking>` 块，未闭合块保持原文。`app-pipeline` 在编剧输出落盘/返回前先对推理块执行 Reasoning 正则，再执行普通 Output 正则；Tauri `display_content` 也先对推理块执行 Display 目标 Reasoning 正则，再执行 Output display-only 正则，因此 `markdownOnly` 的思维链美化不会污染原始消息内容。Slash placement 3 已在用户/导演意图以 `/` 开头时运行，并会先于 Input 正则执行；插件桥已补常用 Slash 命令注册/触发 fallback，并可解析基础 slash invocation 字符串的 raw/named/unnamed 参数，但完整 ST prompt hook 参数管道、pipe 语义和冷门 slash 语义仍待真实插件回归。

2026-07-06 增量核对：`CharacterInfo` 已保留 `alternate_greetings` 并在启动恢复到 domain `Character` 时保留该字段；legacy 单卡新会话前端已提供默认/备选开场切换，`start_writing` 可接收并校验来自当前角色卡的 `opening_message`，新建 conversation 时会持久化选中的开场。Campaign 新建游玩档也已接入默认/备选开场选择：`get_card` 会暴露源角色卡 greetings，`create_campaign` 接收并校验所选 `opening_message`，再写入新建 Campaign conversation。

2026-07-06 增量核对：`crates/domain/src/prompt_module.rs` 的 ST prompt-template 宏已从 `{{char}}/{{user}}/{{charIfNotUser}}` 扩展到确定性核心子集：角色卡字段（description/personality/scenario/first_mes/mes_example/system_prompt/post_history_instructions 等）、`<user>/<bot>/<char>` 别名、本地 `setvar/addvar/getvar/getglobalvar/trim/comment` 顺序宏。`app-pipeline` 在 legacy 单卡（无 Campaign runtime 且仅 1 张扁平角色卡）组装 Director/Editor system prompt 时会执行该渲染。2026-07-07 增量：基础动态宏已接入 `{{date}}` / `{{time}}` / `{{datetime}}` / `{{weekday}}` / `{{isotime}}`、`{{random::A::B}}`、`{{roll::2d6+1}}`，并支持固定时钟和随机种子用于测试/回放；单实例 Campaign 会从当前 `CampaignRuntimeContext` 提供 `{{char}}`、persona/behavior 摘要，以及 `getvar` 可读取的 Campaign 变量、实例变量、`campaign.*` / `instance.*` 命名空间变量。跨轮持久变量读取已覆盖当前快照。2026-07-07 追加：多角色 Campaign 不再整体跳过模板渲染；它会开放 `campaign.*`、`instance.<instance_id>.*` 和名称唯一时的 `instance.<name>.*` 变量读取，同时关闭唯一角色字段渲染，使 `{{char}}`、`{{description}}`、`<bot>` 等歧义宏保留原样，避免误绑定到某个实例。

2026-07-07 增量核对：前端插件事件总线已接入主写作链路。`App.vue::handlePipelineEvent` 会记录最近 100 条写作/重 roll `PipelineEvent`，透传给 `DebugDrawer` 与 `PluginHost`；`PluginHost` 在 iframe 未 ready 时会短暂排队并在加载后 flush；`plugin-bridge.js::mapPipelineEventToPluginEvents` 会发送 `pipeline.<event_type>`、原始事件名，以及 `GENERATION_STARTED`、`STREAM_TOKEN`、`GENERATION_ENDED`、`MESSAGE_RECEIVED` 等常用 ST 别名。2026-07-07 追加：`generateBridgeScript` 已给插件 iframe 注入 ST 风格 `event_types` / `eventTypes` 和 `eventSource.on/once/makeFirst/makeLast/removeListener/emit`，并与 `storyforge.events` 共用同一个监听器集合。2026-07-07 再追加：`App.vue` 已开始把主聊天宿主动作规范化进同一 feed，覆盖 `APP_READY`、`CHAT_LOADED`、`CHAT_CHANGED`、`MESSAGE_SENT/RECEIVED/UPDATED/DELETED/SWIPED`、`CHARACTER_LOADED`。2026-07-07 续补：iframe shim 已补 `eventSource.emitAndWait`；`eventSource.emit` / `storyforge.events.emit` 现在会按监听器顺序等待 async listener，便于后续 prompt hook 类插件异步改写 payload。2026-07-07 续补：插件 iframe bridge 的插件侧请求、挂载和 ready 消息会使用注入的宿主 origin；宿主侧只接受当前 iframe `contentWindow` 发来的消息，API 响应优先回传请求 `origin`，sandboxed opaque origin 场景才保留必要的 `*` 回退。2026-07-07 续补：iframe 已注入 `TavernHelper` / `tavernHelper` 常用 alias shim，转发 events、Slash、statusbar、slot、storage、variables 和 LLM generate 到 `window.storyforge`；Slash invocation 已覆盖基础 raw/named/unnamed 参数且保留零参数命令兼容，TavernHelper selector-based variable helpers 会在插件本地作用域读写，避免 `{type:'message'|'global'|'preset'}` selector 被误发后端；上述路径均有 VM 单测覆盖。剩余缺口是 ST 99 事件全集的真实触发点、prompt 组装钩子和完整 TavernHelper 方法全集。

2026-07-07 续补核对：`storyforge-app-agent::AgentRuntime` 已新增异步 `PromptHook` 接缝，普通、streaming、`run_tool_loop_with_layout` 三条 LLM request 构造路径都会在创建 `ChatRequest` 前给 hook 改写本轮 `messages` 副本；子 Agent 独立 runtime 会继承同一 hook。新增单测用 `SequentialLlmClient` 捕获三条入口的最终 request，断言 hook 追加的 marker 确实进入 LLM messages；另覆盖 hook pending 时全局 cancel 可中断等待，避免后续接 iframe prompt hook 时卡住生成。该接缝为后续 ST prompt hooks 提供后端落点；当前仍未完成的是前端 PluginHost host-to-iframe hook request/response、Tauri/前端跨层等待，以及按插件顺序合并 mutation。

2026-07-07 真实复杂卡导入核对：`cargo run -p storyforge-infra-import --example inspect_card` 已用仓库根目录 `test-card.png`（命定之诗与黄昏之歌 v4.1）解析通过；该卡包含 441 条世界书、6 个 alternate greetings，并保留 `regex_scripts`、`tavern_helper`、`xiaobaix-template` 等 extensions。此检查证明复杂卡 import/inspect 基线可跑，但不等同完整真实写作/插件运行时验收。

2026-07-07 追加核对：Meta 生成溯源前端入口已接线。`frontend/src/utils/conversationNodes.js::findLastAssistantConversationNode` 会从当前消息列表中选择最后一条带 active variant provenance 的 assistant 节点，忽略流式占位和无溯源开场白；`App.vue` 将该节点作为 `lastConversationNode` 传给 `MetaPanel`，因此 `meta_explain_generation` 不再是隐藏死入口。该规则由 `frontend/tests/conversation-nodes.test.mjs` 固化，并纳入 `npm test`。

### Phase 6：Android（~20%）

| 阶段 | 计划目标 | 真实状态 | 证据路径 |
|---|---|---|---|
| AND-1 | 构建链基线（验 `cargo tauri android build`） | 主流 ABI 完成 | 2026-07-06 已通过主流真机 ABI `cargo tauri android build --debug --target aarch64 --ci --split-per-abi --apk` 和 `cargo tauri android build --target aarch64 --ci --split-per-abi --apk`；此前 x86_64 emulator/universal debug/release 也通过。`lib.rs::run` 已补 `tauri::mobile_entry_point`。签名和真机安装未验，armv7/i686 暂不作为发布主线 |
| AND-2 | Android 系统选择器文件导入路径 | 未开始 | 无 Android 专属 import 改动 |
| AND-3 | 本地数据目录 + 迁移/schema 版本 | 未开始 | 无 debug data-dir 命令、无 schema/version 字段 |
| AND-4 | 长任务/流式/取消在移动端 | 未开始 | 无 Android 生命周期处理 |
| AND-5 | 移动端排障/诊断导出 | 部分完成 | `log_export_bundle` 已附带 `diagnostic_context`（app/platform、data/log/conversation 路径、关键 store 文件存在性与大小摘要，含 `active_preset.json`），并有不泄露 `connections.json` / `embed.json` API key 的单测；Android share/save sheet 真机链路未验 |
| AND-6 | capability 权限收敛 | 已完成配置收窄，待真机导入/导出回归 | `capabilities/default.json` 已保留 `core:default`、dialog open/save/message/ask、fs read/write file；`crates/tauri-app/tests/capabilities.rs` 固化无 `fs:default`/`dialog:default`，并校验前端 dialog/fs helper 与 capability 匹配 |

注：Android 代码确实存在（`crates/tauri-app/gen/android/`，含 `MainActivity.kt`、`build.gradle.kts`）。2026-07-06 已验证 arm64-v8a debug/release 构建链路，但 `MainActivity.kt` 仍基本是 Tauri 自动生成脚手架，仅调 `enableEdgeToEdge()`；文件导入、数据目录、长任务和诊断导出仍未做真机验收。

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

- ~~`CampaignRuntimeContext`~~ **阶段 2 已完成**：`crates/domain/src/campaign_runtime.rs` 包含 DTO + helpers，已接入 `WritingContext`/`ToolContext`，`fill_campaign_context_async` 已在写作/重 roll 入口异步组装快照。
- ~~`meta_explain_generation`~~ **已完成并接入前端**：后端 command、Meta runtime `inspect_generation` 工具和 `MetaPanel` “解释上一条生成”入口均已可用；前端只对带 provenance 的 assistant 节点显示入口。
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

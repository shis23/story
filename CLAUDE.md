# StoryForge Claude Code Instructions

## Project Direction

StoryForge is a Rust + Tauri + Vue AI multi-agent writing app.

The product direction is Campaign-first:

- SillyTavern cards are import and material sources.
- Campaign is the runtime source of truth.
- The writing path should move from flat `Character` toward `CharacterInstance` + `CharacterDefinition` + `CampaignRuntimeContext`.
- Meta Agent is a diagnosis, explanation, and repair layer. It is not the main writing surface.

## Required Reading Order

Before changing code, read these files in order:

1. `docs/DOCS-CODE-AUDIT.md`
2. `docs/ROADMAP.md`
3. `docs/ARCHITECTURE-AUDIT.md`
4. `docs/AGENT_INTERFACES.md`
5. `docs/DATA_MODEL.md`

历史已归档计划（Phase 1-4 的详细执行记录）可在 `docs/archive/2026-06-19-completed-phases/` 中找到，不需要再次执行。

Archived background, not an execution entrypoint: `docs/archive/2026-06-17-campaign-mainline-phase5/PLAN-CHARACTER-UNIFICATION.md`.

Treat `docs/DOCS-CODE-AUDIT.md` as the authority for separating current code facts from future plans.

## Hard Rules

- Do not rewrite the project architecture from scratch.
- Do not break the existing non-Campaign writing path.
- Do not make `app-agent` depend on `tauri-app`.
- Do not put `CampaignStore` inside `ToolContext`.
- Pass Campaign runtime data as pure domain snapshot data.
- When Campaign is active, prefer `CharacterInstance.id` as the internal identity.
- Character names are allowed for display and LLM input/output, but persistent storage must resolve them to instance ids.
- Preserve existing Tauri command signatures unless the current task explicitly requires changing them.
- Keep changes scoped to the requested phase.
- Do not implement later phases early.
- If the plan conflicts with current code, stop and report the conflict instead of guessing.

## Current Code Facts

Respect these facts unless the current task explicitly changes them:

- `CharacterInstance` currently has:
  - `id`
  - `campaign_id`
  - `definition_id`
  - `name`
  - `persona_override`
  - `behavior_override`
  - `variables`
  - `is_temporary`
- `CharacterInstance` currently does not have:
  - `backstory_override`
  - `variable_schema`
- `CharacterDefinition` owns:
  - `persona_prompt`
  - `behavior_rules`
  - `base_backstory: Vec<String>`
  - `variable_schema`
- `resolved_persona(definition)` / `resolved_behavior(definition)` accept `Option<&CharacterDefinition>` and fall back to definition when override is absent.
- `WritingContext` has `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` (阶段 2). None = no active Campaign, legacy path.
- `ToolContext` has `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` (阶段 2). None = no active Campaign, legacy path.
- `fill_campaign_context` loads instances, definitions, knowledge from CampaignStore and assembles `Arc<CampaignRuntimeContext>` (阶段 2).
- `CampaignRuntimeContext` is a pure domain snapshot in `crates/domain/src/campaign_runtime.rs`. No store/lock/Tauri state.
- `CampaignRuntimeContext::with_temporaries_for(character_specs)` creates temporary `CharacterInstance` values for unmatched IDs with optional persona/behavior overrides (阶段 6). Returns `Vec<CharacterInstance>` for caller to persist and dedups duplicate unmatched characters within the same batch.
- `CharacterInstance::temporary_with_overrides(campaign_id, name, persona_override, behavior_override)` creates a temporary instance with optional overrides (阶段 6).
- `PipelineOrchestrator::pending_temporary_instances` stores temporaries created during the current turn; `start_writing` / `regenerate` clear stale pending data at the start. A.1 起 Tauri 经 getter 把它们挂到 `TurnAttempt.pending_temporary_instances`（不再 postprocess 前落盘）；accept 时 `prepare_commit_batch` 前置 `Mutation::UpsertInstance` 落库，Discard 不写 Campaign。
- `persist_temporary_instances_to(store, ctx, temporaries)`（existing-name dedup、same-batch dedup、campaign_id mismatch guards）A.1 后仅保留给单测；生产路径不直接落盘临时实例（`persist_temporary_instances_async` 已标 dead_code）。
- `request_ad_hoc_character` tool is NOT implemented and was evaluated as unnecessary; unmatched character_id flow + `context_package.character_brief` as persona_override fully covers the ad-hoc character use case (see `docs/archive/2026-06-19-completed-phases/PLAN-CAMPAIGN-MAINLINE.md` Phase 6 evaluation).
- `ToolContext` has `current_character_instance_id: Option<Id>` (阶段 4). Used by subagent `get_character` for information isolation.
- `spawn_subagents` receives `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` (阶段 4). Matches instances, injects resolved persona/behavior/knowledge/variables.
- `SubagentSnapshot` has `character_instance_id`, `display_name`, `fallback_reason` (阶段 5).
- `build_provenance_with_campaign` populates SubagentSnapshot instance fields from CampaignRuntimeContext (阶段 5).
- `persist_postprocess_outcome` resolves postprocess knowledge/variable targets to persisted `CharacterInstance.id`, uses `present_chars` to filter writes, validates task updates belong to the current Campaign, and skips unresolved characters (阶段 5). V2 修复（2026-07-27）：`apply_outcome` 从 TurnRecord 取本 attempt 的 `pending_temporary_instances` 传入批构建——`build_json_mutation_batch`（经 `normalize_knowledge_update_for_postprocess_with_extras` / `find_instance_by_name_or_id_with_extras`）与 `build_runtime_mutation_batch`（有效实例集 = 快照 + pending temps）都能解析 accept 前的临时角色；同名收紧与广播受众同样计入 temps。accept 的 UpsertInstance 前置保证这些 mutation 落库安全。
- `CharacterInfo` stores `source_character_id: Option<String>` for new imports so restart recovery can preserve the domain `Character.id`; old data falls back to `StoredCharacter.id`.
- `delete_character` cascades over `StoredCharacter.id`, persisted `source_character_id`, and same-session `tool_ctx` domain ids for Campaign/MVU/vector cleanup (阶段 5).
- `app-agent` must not depend on `tauri-app`.
- `AgentProfileConfig` in `crates/domain/src/agent_profile_config.rs` is a pure domain DTO for configurable agent runtime parameters.
- `AgentRunConfig` has `model_override`, `max_tool_rounds`, `tool_whitelist` (all `Option`). Supports Subagent wildcard fallback.
- `AgentProfileConfig` has `agent_configs: HashMap<AgentRole, AgentRunConfig>`, `max_concurrent_subagents`, `enable_postprocess`, `enable_summarizer`, `source`, `config_version`.
- `AgentProfileConfig::effective_max_concurrent_subagents()` clamps invalid `0` to `1`; pipeline and runtime must not pass raw `0` into subagent scheduling.
- Built-in default profile ID is `builtin-default-agent-v1`, always available, not deletable.
- `AgentProfileConfigStore` in `crates/tauri-app/src/module_store.rs` persists to `agent_profile_configs.json` + `active_agent_profile_config.json`.
- Tauri commands: `list_agent_profile_configs`, `get_agent_profile_config`, `get_active_agent_profile_config`, `save_agent_profile_config`, `delete_agent_profile_config`, `set_active_agent_profile_config`.
- `WritingContext` has `agent_profile_config: Option<AgentProfileConfig>`. `legacy()` sets it to `None`.
- `fill_agent_profile_context` loads active config into `WritingContext`.
- Director/Editor `make_*_config` apply `model_override` and `max_tool_rounds` from profile. No config = current hardcoded defaults.
- `spawn_subagents` accepts `max_concurrent_subagents` and `agent_profile_config` parameters. Subagent model/rounds are overridden per-profile.
- `tool_whitelist` is consumed at runtime via `ToolRegistry::retain` / `filter_registry_by_whitelist`. Director/Subagent/PostProcessor registries are filtered after tool registration (None=default tools, Some([])=disable all, Some(list)=allow only listed; unknown tool names are warned and ignored, never panic). Dispatching a removed tool returns `ToolError::NotFound` — whitelist cannot be bypassed.
- `enable_postprocess` / `enable_summarizer` are consumed at runtime: `run_postprocess_pipeline` takes both flags and skips the corresponding LLM call (returning `None`) when false; `PipelineOrchestrator::run_postprocess` reads them from the profile. When both are off it emits `PipelineEvent::PostProcessSkipped` (not `PostProcessFailed`), so a deliberate disable is not mistaken for an error.
- `PostProcessSkipped { reason }` is a `PipelineEvent` variant (serde-compatible) emitted when postprocess/summarizer is disabled by config; Tauri serializes it as `postprocess_skipped`.
- MVU 翻译产物消费端已接通（2026-07-26）：`MvuTranslation.update_rules` 经 `collect_mvu_update_rules(_for_backend)`（tauri-app，按源卡去重）→ `PipelineOrchestrator::run_postprocess(mvu_update_rules)` → `run_postprocess_pipeline_with_prompt` → `build_postprocess_user_msg_with_context` 注入【卡片变量更新规则】区块（去重 + 80 条/400 字/12K 预算）。空切片时输出与旧版字节级一致。
- SQLite MVU 翻译权威（2026-07-27，#22）：infra-sqlite V005 `mvu_translations` 表（payload=StoredMvuTranslation 全量 JSON）+ importer 迁移 `mvu_translations.json`（仅非空时入 manifest hash，老目录 hash 稳定）+ repo `save/get/list/delete_mvu_payload` + `list_card_payloads`。sqlite_runtime 包装 `save_mvu/get_mvu/list_mvu/delete_mvu/list_card_payloads`。两个收集器 SQLite 分支走 `collect_mvu_from_sqlite`（def→source 反查兼容 StoredCard 包装/裸 CharacterCard payload，与 JSON 路径同语义：规则按 source 卡去重、空白过滤）。`meta_list_mvu_translations`/`meta_get_mvu_translation`/`meta_analyze_mvu_card` 不再被 SQLite 门拒绝（读写分流）；删卡级联分流。集成测试 `tests/sqlite_mvu_translations.rs`（独立进程 activate）。
- 前端写作面 MVU 原生渲染：`useMvuStatusPanel` composable + `MvuStatusPanel.vue`（components-v2/st/）挂 WritingScreen `after-messages` 槽；`buildCampaignMvuStatusSections`（campaign 变量打底、实例覆盖）；刷新时机为 campaign 切换与 postprocess running→done。
- `interactions` 原生分发：`utils/mvuInteractions.js`（`planMvuInteraction` 纯计划；value_expr 保守解释：+N 增量/JSON 字面量/裸词字符串，JS 表达式拒绝）；`dispatchMvuInteraction` 写变量走 `persistShellVariableWrite`（单卡绑定实例→instance 作用域，否则 campaign），`trigger_next_turn` 走 `startWriting`；`run_original_js` 留桩不执行。
- MVU 变量键记法归一化（2026-07-27）：canonical 形式 = 点记法、无 `stat_data.` 前缀、模板段 `{角色名}`。`normalize_mvu_key` / `normalize_schema_keys`（domain variables.rs）在 mvu_import 解析层（schema/ui_bindings/interactions 三处，含键自引用表达式改写 `rewrite_self_ref_expr`）与 `meta_apply_mvu_schema` 应用边界（存量兜底）统一收敛；分析器提示词钉死记法。前端镜像 `utils/mvuKey.js`（`normalizeMvuKey`/`findMvuVariable`），`getMvuValue` 与 `planMvuInteraction` 跨记法匹配（写回优先已存储键，避免同变量双记法并存）。
- MVU 分析器 [InitVar] 预算（2026-07-27）：数据条目单条 24K/变量区总 40K 字（规则类维持 4K）。旧预算（4K/14K）截断大变量树中段导致两模型 schema 覆盖率恰好同为 50.9%（确定性截断指纹，非模型问题）。
- Frontend API wrappers in `frontend/src/tauri-api.js`: `listAgentProfileConfigs`, `getAgentProfileConfig`, `getActiveAgentProfileConfig`, `saveAgentProfileConfig`, `deleteAgentProfileConfig`, `setActiveAgentProfileConfig`.
- Card-shell 评审修复（2026-07-26，CARD-SHELL-REVIEW H1-H5/M3/M4 已入库）：
  - `apply_campaign_opening` Tauri 命令（lib.rs）：开场壳选择落库，仅会话开场态（唯一 assistant 消息）可改写；前端 wrapper `applyCampaignOpening`，AppV2 Campaign 分支经 `rewriteOpeningMessages`（cardShellOpeningChat.js 纯函数）本地改写后同步调用。
  - `selectOpeningGreetingOptions`（cardShellOpeningChat.js）：campaign 态开场种子一律用卡自己的 greetings，不回退 `writing.greetingOptions`（activeCharDetail 残留）。
  - 壳包装文档带 CSP（`utils/cardShellCsp.js` → CardShellHost wrapRemoteHtml）：网络向指令钉死 `card_shell_list_allowed_hosts` 白名单，白名单获取失败 fail closed 到仅 data:/blob:/storyforge-cache。
  - 壳 `var_write` 走提案确认（M4）：`utils/shellVariableProposals.js` 队列 + `ShellVariableProposalBar.vue` 确认条；只有用户点「应用」才 `persistShellVariableWrite`；MVU 交互按钮（dispatchMvuInteraction）保持直写。
  - 消息内挂壳信任分级（H3/H4）：`.load(url)` 仅卡 manifest 注册 URL 自动挂载；display 里含 script 的内联 HTML 文档走 CardShellHost `html` prop 挂载，信任锚为源文命中 manifest InlineHtml 的 find_regex（`matchesAnyInlineShellTrigger`）；未命中一律确认卡。
  - 消息壳原地渲染：`segmentShellContent`（cardShellDisplay.js）单遍 span 认领输出有序 text/shell 分段，ShellAwareContent 逐段渲染（壳在原文位置）。无壳消息逐字节原样返回；同 URL 首现渲染；suppress 仅作用于 .load 壳；load 段按 URL 作 key、内联段按 start 偏移（流式追加稳定，全文改写允许一次重挂）。旧 `extractShellMountsFromDisplay`/`extractInlineShellDocsFromDisplay` 保留导出但组件已不用。
  - `classify_shell_kind`（card_shell.rs）关键词含 开场/intro//intro/ → OpeningCustom，状态判定先于开场判定。
- Card-shell 低危清尾（2026-07-27，M1/M2/M5 + L1/L2/L3/L5/L6 已入库；L4/L7 评估维持不修，见 CARD-SHELL-REVIEW）：
  - M1：CardShellHost 重载 watch 比内容指纹（`shellIdentity` computed，JSON 序列化 url/html/campaignId/openingChatSeed），无关状态变化不重建 iframe。
  - M2：壳侧 replace 家族发键级补丁 `mode:"patch"`（sets/deletes，相对本壳快照 diff）；宿主 `patchCardShellVariables`（cardShellVariableStore.js）按键应用；merge/replace 协议保留兼容旧壳。`enqueueCardShellVariableMutation` 尾巴吞 rejection（防 unhandledRejection）。
  - M5：Mvu shim 注入真实 Campaign 变量树（`buildMvuStatDataTree`，utils/mvuStatTree.js，点记法键按段展开，__storyforge* 键除外）作 stat_data 只读底座，壳桶键级覆盖；`mvu_data_get` 桥刷新（init + waitGlobalInitialized，暴露 `Mvu.refreshMvuData`）；`Mvu.events` 常量表补齐。写侧仍走沙箱桶。
  - L1：card_shell_cache fetch 顺序 = 自身缓存 → 标准图兜底 → 网络（超清图缓存不再被劫持）。L3：`isTrustedSource` 按 event.source 沿 parent 链归属本壳 iframe（嵌套子 iframe 在链上），不再信任 pluginId 字段。L5：`i.postimg.cc` 入默认白名单。L6：`card_shell_clear_cache` 命令 + wrapper `cardShellClearCache`（UI 入口未接）。
- card-studio 出卡闸门（export_gate_checks）含 forge 三维保真：`gate.content_roundtrip`（逐条正文，CRLF/trim 归一，顺序敏感）、`gate.keys_roundtrip`（触发键精确）、`gate.insertion_order_roundtrip`（order 保真）；tauri-app 真实 JSON+PNG 导入路径测试继承。
- 临时目录惰性清扫：harness `sweep_stale_harness_dirs`（storyforge_harness_*）与 tauri-app `AppState::sweep_stale_test_data_dirs`（storyforge-app-state-test-* / storyforge_test_*），均 >24h、每进程一次、best-effort。
- Frontend management UI in `frontend/src/components/AgentProfileManager.vue` (mounted under power mode, after `AgentConfigCard`): list/switch/duplicate/delete/edit/save profiles, including per-role `model_override` / `max_tool_rounds` / `tool_whitelist` (comma-separated) for Director/Editor/Subagent:*/Summarizer/PostProcessor, plus `max_concurrent_subagents` / `enable_postprocess` / `enable_summarizer`. Built-in default is read-only and not deletable; custom profiles are deletable. (Note: `AgentConfigCard.vue` is the *PromptProfile module selector*, a separate system — not the AgentProfileConfig editor.)
- `ProfileConfigError` in `crates/domain/src/agent_profile_config.rs` (thiserror): `EmptyName`, `MaxToolRoundsOutOfRange { role: String, value: u32 }`, `InvalidMaxConcurrent { value: usize }`.
- `AgentProfileConfig::validate()` checks: name non-empty, `max_tool_rounds` in `[1,100]` when present, `max_concurrent_subagents >= 1`. Does NOT validate `tool_whitelist` tool names (runtime handles unknown names with warning+ignore).
- `AgentProfileConfig::migrate_to(target: u32) -> bool`: v1→v1 is no-op (returns false); unknown versions update `config_version` but preserve data (forward-compatible). Migration skeleton for future versions.
- `AgentProfileConfigStore::save()` calls `config.validate()` before persisting; returns `Err(reason)` on validation failure.
- `AgentProfileConfigStore::new()` and `get()` call `migrate_to(1)` on loaded configs.
- No `temperature` field added to `AgentRunConfig`; temperature is controlled at the connection level, not per-agent profile.

## Execution Process

For every task:

1. Restate the exact phase being implemented.
2. Search the current code with `rg` before editing.
3. Confirm the relevant symbols and files still match the plan.
4. Make the smallest code changes needed for this phase.
5. Add or update focused tests.
6. Run the required verification command.
7. Update relevant docs only if behavior, status, or implementation details changed.
8. Report:
   - files changed
   - tests run
   - remaining risks
   - next recommended phase

If the plan conflicts with current code, stop and report the conflict. Do not silently invent a different architecture.

## Verification

Default verification:

```bash
cargo test --workspace
```

For narrow phases, run the package-specific test first, then workspace tests when practical.

Do not claim success unless the relevant tests pass. If tests cannot be run, explain why.

## Phase Discipline

Prefer one phase per commit.

Suggested commit message format:

```text
campaign: implement character fallback phase
```

For Phase 2 and later, split work further if the phase crosses multiple crates. A good split is:

1. domain DTO and helpers
2. `WritingContext` integration
3. `ToolContext` integration
4. Tauri `fill_campaign_context` integration

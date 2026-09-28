# 域1 修复记录：domain + 基础 infra（D-01..D-26 + 跨域委派项）

- **任务**：`task-8`（修复域1：domain + 基础 infra，D-01..D-26 全量修复），owner `review-domain`
- **审查基线**：`docs/review-2026-09-13/01-domain-infra.md`（1 P0 / 3 P1 / 12 P2 / 10 P3，D-01..D-26）
- **状态口径**（严格，不谎报）：
  - **已修复** = 代码改动 + 本域测试落地（给出测试名）
  - **已修复(降级)** = 真实风险已收敛，但未采用报告中的完整方案（写明差距与残余风险）
  - **判定非问题(附证据)** = 复核后不成立，附代码依据
  - **暂缓(附理由)** = 本轮不修，说明原因与下一步
- **写作用域遵守**：只改 `crates/domain/**`、`crates/infra-util/**`、`crates/infra-vector/**`、`crates/infra-regex/**`、本文件。
  - **未**改 `docs/**`（除本文件）、`README.md`、`CLAUDE.md`、`crates/tauri-app/**`、`frontend/**`、`crates/app-*` —— 跨域条目一律进 §7「文档需同步条目」与 §8「跨域 handoff」。
  - **未**跑 `cargo test --workspace`（Lead 独占）；域内门禁见 §9。
- **D-01 未改**（Lead 已修复，我只是确认其回归测试仍在且通过）。
- 冲突处理按 Lead 规则：**取"保持当前行为的最小改动"**，逐条标注差异（见 D-10、D-11、D-22、D-24）。

---

## 1 结论摘要

| 严重度 | 总数 | 已修复 | 已修复(降级) | 判定非问题 | 暂缓 |
|---|---|---|---|---|---|
| P0（D-01） | 1 | 0（**Lead 已修复**） | 0 | 0 | 0 |
| P1（D-02..D-04） | 3 | 3（D-02、D-03、D-04） | 0 | 0 | 0 |
| P2（D-05..D-16） | 12 | 12 | 0 | 0 | 0 |
| P3（D-17..D-26） | 10 | 7（D-17..D-20、D-23、D-24、D-26） | 0 | 1（D-22 的 `CardShellKind::Other` 一项） | 2 类（D-21 的 `Custom` 形状、D-22 的 4 项特性脚手架、D-25 前端分类器） |
| 域6 委派（M-05/M-26/M-28d/M-32.8/M-32.9） | 5 | 5 | 0 | 0 | 0 |

**测试增量**：`storyforge-domain` lib **343 → 388 passed**（+45 条，含 D-01 的 Lead 回归测试在列）；`storyforge-infra-vector` **21 passed**（新增 5 条 + D-03 健康报告断言扩展）；`storyforge-infra-util` **10 → 12 passed**（D-03 追加可观测性 2 条）；`storyforge-infra-regex` **25 + 1 passed**（仅文档/文案，无新增）。

**Lead 裁定 D-04 的最终语义（一句话，供 R7 复核）**：`check_trigger` 保持三态（StoryTime：空 target→NotSatisfied 忽略；时钟为空→NeedsAgentJudgment；归一化相等→Satisfied；已注入且时钟不等→NeedsAgentJudgment；否则 NotSatisfied），渲染层把它拆成两组**不同指令**的标题——`【已满足条件的任务/伏笔】`（可推进）与`【待判断的任务/伏笔】`（"不适用则忽略，不要提前揭示后续剧情"），两组皆空时仍返回 `""`。

---

## 2 P0 详述

### D-01 [P0] `capture_es_module_urls` 多字节 UTF-8 越界 panic — **Lead 已修复（域1 未再改动）**

- Lead 的修复位于 `crates/domain/src/card_shell.rs`（`search_from` 边界推进），回归测试 `card_shell::tests::es_module_scan_handles_multibyte_after_needle`。
- 域1 本轮动作：**不重复编辑**，仅在每次域门禁里确认该测试仍在通过集合内（388 passed 含它）。
- 状态：**已修复（Lead）**。

---

## 3 P1 详述

### D-02 [P1] `WorldInfoEntry::set_enabled` 破坏"蓝绿旗标 ↔ route ↔ ST V3 `enabled`"不变量 — **已修复**

- `crates/domain/src/world_info.rs`：
  - `set_enabled` 现在同时同步 `extra["enabled"]`（**仅当该键已存在**，不凭空发明 V3 字段），并保留 `Result` 签名以兼容既有调用方（删掉了原来不可达的 `Err` 分支，见 D-23）。
  - 新增 `route_for_flags(constant, selective)`，`default_route()` 与 `from_st` 共用同一映射（不再是两份手抄表）。
- 测试：`set_enabled_syncs_v3_enabled_flag_and_survives_round_trip`、`set_enabled_does_not_invent_enabled_key_for_v2_entries`、`set_enabled_restores_route_from_blue_green_flags`、`from_st_route_matches_default_route_mapping`。
- 残余：`Result` 返回类型保留（不再可能返回 `Err`），仅为 API 兼容；未改签名以免波及 app/tauri。

### D-03 [P1] 向量库不可恢复损坏 → 静默空库 + 下一次保存固化空态 — **已修复**

- `crates/infra-vector/src/lib.rs`：抽出 `quarantine_unrecoverable_corruption(path, err)`，两条损坏分支（解析失败 / 读取失败）共用：
  1. `.corrupt` 抢救副本的 `std::fs::copy` **不再 `let _ =`**——成功 `error!` 记路径，失败也 `error!` 记原因（这是唯一抢救来源，静默失败等于双重丢数据）；
  2. 调用 `storyforge_infra_util::write_fence::freeze_with_reason(path, reason)`：损坏未确认前 `atomic_write` 对 `vectors.json` 直接 `PermissionDenied`，杜绝"空态回写覆盖可抢救文件"；`reason` 写明"主文件与 .tmp 均无法解析（错误摘要）；抢救副本位置/失败原因"。
- **Lead 裁定追加的可观测性（2026-09-13；裁定：保持 fail-closed，但必须可观测、可恢复，不降级为"仅日志"）**：`crates/infra-util/src/write_fence.rs` 扩展为最小可诊断面：
  - `freeze_with_reason(path, reason)`（新；`freeze(path)` 保留，使用默认原因"加载失败且无法自动恢复"，兼容既有调用方）；
  - 只读查询：`frozen_reason(path) -> Option<String>`、`frozen_entries() -> Vec<FrozenEntry>`（含 `path`/`reason`/`frozen_at_epoch_secs`）、`FrozenEntry::summary()`；
  - **健康报告入口**：`storage_health_report() -> Vec<(kind, detail)>`，`kind = "write_fence_frozen"`，文案含"哪份文件被隔离 + 原因 + 人工处理建议（先备份/移走该文件再重启；本次启动期间写入会被拒绝）"，供 tauri-app `storage_health::record_backend_incident` 直接消费；
  - 应用内一键解除属产品决策（Lead 记为 **N-03，P2 暂缓**），本轮不做，因此文案指向"备份/移走文件后重启"的人工路径。
- 测试：
  - `infra-vector`：`unrecoverable_corruption_freezes_writes_and_keeps_backup`（扩展断言：`.corrupt` 内容一致 + 冻结生效 + `upsert` 返回 `PermissionDenied(Io)` + 主文件未被覆盖 + **冻结条目在 `write_fence` 诊断快照与 `storage_health_report()` 里可见，含"无法解析"/"corrupt"/"已被隔离"/"处理建议"**）、`tmp_backup_recovery_does_not_freeze`（`.tmp` 可恢复时**不得**冻结）。
  - `infra-util`：`frozen_reason_and_health_report_explain_the_freezed_file`（原因可查 + 报告含隔离说明/原因/处理建议 + 解冻后不残留）、`freeze_without_reason_still_reports_a_default_reason`。
- **跨域未完成的一行接线（不在我写作用域，需域4/域2）**：`crates/tauri-app/src/lib.rs:841-843` 构造向量库后 `storage_health` **不会**收到该事件（`record_unrecoverable` 目前只被 `json_store.rs` 调用）。建议加 3 行：
  ```rust
  // crates/tauri-app/src/lib.rs，紧接 BruteForceStore::with_persistence(...) 之后
  for (kind, detail) in storyforge_infra_util::write_fence::storage_health_report() {
      storage_health::record_backend_incident(&kind, &detail);
  }
  ```
  （该 sweep 覆盖所有冻结路径；`json_store` 自己已登记的路径会多一条 `backend:write_fence_frozen` 记录，属互补信息，如需去重可在 sweep 前比对 `incidents()` 的 `path`。）
- **重要跨域事实**：`write_fence::freeze` 在此之前全仓**零生产调用者**（只有自身单测）；本修复是它的第一个真实调用点，因此也第一次暴露"冻结后没有解除入口"的缺口 → 见 §8 handoff（域4/域2 需要 `storage_health` 事件 + 恢复引导）。
- 定级说明：报告 Lead 复核项已按"向量库可重建"取 P1；本修复不改变定级，只把"静默丢"改为"可见 + fail closed"。

### D-04 [P1] StoryTime 在时钟缺失/越过目标时静默消失 + 渲染两类混用 — **已修复（域1 侧）**，根因在域3

**域1 代码**
- `crates/domain/src/story_task.rs`：
  - `normalize_story_clock(raw)`：去全部空白 + Unicode 小写，`"第 47 天"`/`"第47天"`/`"Day 1"`/`"day1"` 可比；
  - `check_trigger` 三态化：空 target → 忽略该触发器（NotSatisfied）；时钟为空 → `NeedsAgentJudgment`；归一化相等 → `Satisfied`；`has_been_injected()`（`injected_turns` 非空或状态 `Active`）且时钟不等 → `NeedsAgentJudgment`；否则 `NotSatisfied`；
  - **Lead 裁定**：`render_tasks_for_injection` 从"单一标题混合渲染"改为两组、每组带自己的指令：
    - `【已满足条件的任务/伏笔】`：现有语义（可以直接推进）；
    - `【待判断的任务/伏笔】`：明确指令"以下任务/伏笔的条件无法确定（故事时钟缺失或已越过目标）。请判断本轮是否适用；不适用则忽略，不要提前揭示后续剧情。"——消除"混在『即将触发』下 = 暗示模型现在就写出来"的提前揭示风险；
    - 两组皆空 → 仍返回**空字符串**（不输出空标题），既有字节级契约不变；
    - 只有一组时只输出该组（不输出空标题）。
- 测试（9 条）：`test_story_time_missing_clock_needs_agent_judgment`、`test_story_time_comparison_normalizes_whitespace_and_case`、`test_story_time_already_injected_survives_clock_advance`、`test_story_time_blank_target_is_ignored`、`test_active_status_alone_counts_as_injected`、`render_splits_satisfied_and_pending_judgment_groups`、`render_pending_judgment_group_contains_no_spoiler_instruction`、`render_missing_clock_story_time_lands_in_pending_group`、`render_returns_empty_string_when_nothing_injectable`。
- **既有测试/快照变化**：全仓 grep `【即将触发的任务/伏笔】` **零匹配**（该文案没有外部断言），因此无既有断言被放宽或改写；域内 5 条既有 D-04 测试都是 `contains(...)` 断言，继续通过。
- 与域3 的接缝（Lead 要求确认）：`crates/app-pipeline/src/turn_dossier.rs` 的**根因修复在域3 已完成**——`review-pipeline` 于 2026-09-13 明确同步：两处调用点已改为传 `&ctx.story_clock`，其测试改名为 `story_time_task_without_clock_goes_to_pending_judgment_group`（断言 `check_trigger(3, "") == NeedsAgentJudgment` 且仍进入待注入集合）。域1 的分组渲染是**兜底**，不替代该根因修复。
- 残余：turns 侧若仍把空时钟传下来，只会落进"待判断"组——不再静默消失，但需要域3 保持调用点不回归（建议 R7 钉一个跨域测试）。

---

## 4 P2 清单（D-05..D-16）

### D-05 [P2] `infra-vector` 搜索顺序不确定 — **已修复**
- `search_by_keywords_filtered`：先算"命中关键词条数"，按 **命中数降序 → id 升序** 排序后才 `truncate(limit)`（`HashMap` 迭代序随机导致的"随机子集"消失）；`score` 仍为 `1.0`（关键词无相似度分数，语义不变）。
- `search_by_vector(_filtered)`：同分时按 id 升序 tie-break，top-k 稳定。
- 测试：`keyword_search_truncation_is_deterministic`、`keyword_search_ranks_more_matches_first`、`vector_search_ties_break_by_id`。

### D-06 [P2] `story_clock` 双权威默认值不一致（`"Day 1"` vs `第1天`） — **已修复**
- `crates/domain/src/variables.rs`：新增 `pub const DEFAULT_STORY_CLOCK: &str = "第1天"`，`default_campaign_variables()` 用它；`campaign.rs::default_story_clock()` 改为委托该常量（原 `"Day 1"` 删除）。
- `campaign.rs::new_with_variable_schema`：新增 `story_clock_from_schema(schema)`，从变量 schema 里 `story_clock` 字段的 default 取值，**新档顶层字段与 variables 同源**，不再制造"新建 Campaign 恒为分歧状态 + 每次加载 warn + 改写"。
- 测试：`test_fresh_campaign_story_clock_has_single_authority`、`test_fork_campaign_story_clock_has_single_authority`、`test_card_story_clock_default_is_mirrored_into_legacy_field`、`test_legacy_json_missing_story_clock_uses_schema_default`、`test_current_story_clock_falls_back_to_field`（含 `!story_clock_diverged()` 断言）。
- **跨域未修（不在我写作用域）**：`crates/infra-sqlite/src/importer.rs:553` 仍有 `.unwrap_or_else(|| "Day 1".to_string())` 兜底 → §7 文档需同步 + §8 handoff 给域2。

### D-07 [P2] `select_anchor_turns` 在 `h_anchor == 0` 时切片 panic — **已修复**
- `crates/domain/src/chronicle.rs`：入口 `if h_anchor == 0 { return Vec::new(); }`。
- 测试：`select_anchor_turns_handles_zero_anchor_without_panic`、`refresh_context_epoch_accepts_zero_anchor`。

### D-08 [P2] `apply_stage_json` 用空串覆盖用户已填字段 — **已修复**
- `crates/domain/src/card_studio.rs`：`STAGE_BASIC` 分支的 `description`/`scenario` 覆盖加 `!t.trim().is_empty()` 守卫（空/空白不再清空）。
- 测试：`basic_stage_json_does_not_wipe_with_empty_strings`。

### D-09 [P2] 未知 `before_node_id` 退化成全量历史（含正在被重写的草稿） — **已修复**
- `crates/domain/src/conversation.rs::collect_active_variants`：无法定位 id 时返回**空前缀** + `tracing::warn!`（此前 `unwrap_or(nodes.len())` 会把"即将被替换的草稿"喂回模型）；私有函数，无 API 波及。
- 测试：`test_unknown_before_node_id_yields_empty_history_prefix`（未知 id → 空；已知 id 行为不变）。

### D-10 [P2] `depth_prompt.prompt` 自然语言正文被计入 `script_bytes`（纯文字卡误判 Heavy） — **已修复（与报告建议有 1 处刻意差异）**
- `crates/domain/src/mvu_translation.rs`：`collect_js_blob` → `collect_script_sources`，返回 `CardScriptSources { code, prose }`；`script_bytes` 只统计**真脚本源**（`assets.js`、`mvu.script`、`tavern_helper.scripts[].content`）。
- **刻意差异**：报告的选项之一是"正文只参与占位符探测"。域1 取"正文不参与**字节阈值**、但**仍参与模式计数**（`document.`/`innerHTML`/`_.set`/`{{` 等）"，理由是既有测试 `test_complexity_collects_js_from_extensions_depth_prompt` 明确要求"depth_prompt 里内嵌 script 也要被统计"（真实卡有把 JS 片段写进 depth_prompt 的写法），全部排除会把这类卡判成 `PureData` 并短路跳过 LLM 分析——即"保持当前行为的最小改动"。
- 顺带修掉 D-20 的文档漂移：阈值注释改为与代码一致（只有 `document.`/`innerHTML` 参与 Heavy 判定；`getElementById`/`$.`/`script_blocks` 只进 `counts` 供 Meta Agent 二次判定）。
- 测试：`test_complexity_long_prose_depth_prompt_is_not_script_heavy`（>5000 字节纯中文正文 → `PureData`、`script_bytes == 0`；等量真实脚本仍 `Heavy`）。

### D-11 [P2] 嵌套宏在第一个 `}}` 被截断（`{{setvar::a::{{getvar::b}}}}`） — **已修复**
- `crates/domain/src/prompt_module.rs`：新增 `find_macro_body_end`（按 `{{`/`}}` **深度配对**找结束，非首个 `}}`）与 `render_template_text`（递归渲染前 8 层嵌套宏，`MAX_MACRO_NESTING = 8` 防自引用无界递归）；未知宏仍**按原始字节原样保留**（不注入已求值内容）。
- `render_template_value` 里原来的"内联 `{{getvar::k}}` 替换"分支**保留**，但从"唯一路径"退化为防御性冗余（不再吞掉外层参数），未删除以免改变第三方宏的既有替换语义。
- `replace_angle_aliases` + `trim` 只在顶层执行（保持旧行为），嵌套层不重复处理。
- 测试：`nested_macro_inside_setvar_argument_is_expanded_first`、`nested_macro_rendering_keeps_unknown_macros_verbatim`、`macro_body_end_matching_is_depth_aware`、`deeply_nested_macros_stop_at_recursion_limit`。

### D-12 [P2] `order` 乘法静默回绕（`i32` 溢出） — **已修复**
- `crates/domain/src/card_studio.rs`：两处 `(i as i32 + 1) * 10` 改为 `i32::try_from(n).ok()` 推导，越界时回退既定默认值（10 / 100），不再静默回绕。
- 测试：`out_of_range_order_is_not_wrapped_silently`。

### D-13 [P2] `Exclusivity::Single` 单选语义未在组装时执行 — **已修复**
- `crates/domain/src/prompt_module.rs::assemble_system_prompt`：同类别内若选中集合含 `Single` 模块，则**只注入第一个可用模块**（跳过不存在/不适用的候选后仍会回退到下一个可用项，只是不再叠加）；`Multiple` 类别行为不变。
- 测试：`single_exclusivity_keeps_only_first_selected_module`、`multiple_exclusivity_still_accumulates`、`single_group_skips_unavailable_first_candidate`。

### D-14 [P2] `migrate_to(target)` 会把版本标记降级 — **已修复**
- `crates/domain/src/agent_profile_config.rs`：`if target <= self.config_version { return false; }`——只有更高 target 才推进版本，**绝不把来自更新版本的数据降级**；注释同步改写（原注释与实现不符）。
- 测试：`migrate_to_never_downgrades_version_marker`；另补 `validate_reports_deterministic_role`（多角色同时非法时，`HashMap` 迭代序随机曾让报错角色抖动——现在先排序取第一个，同输入同输出）。
- **文档影响**：`CLAUDE.md` 现有表述"unknown versions update `config_version` but preserve data"已不准确 → §7。

### D-15 [P2] `CardProject` 缺字段即整份 `card_projects.json` 解析失败 — **已修复**
- `crates/domain/src/card_studio.rs`：`CardProject` 的 `id`/`name`/`mode`/`created_at`/`updated_at` 加 `#[serde(default)]`。
- 测试：`card_project_deserializes_legacy_missing_fields`。

### D-16 [P2] 出卡闸门可被 LLM 输入操纵（severity 降级判定过宽） — **已修复**
- `crates/domain/src/card_studio.rs`：新增 `KNOWN_RULE_CODES`（name_required / description_required / first_mes_required / personality_missing / description_has_personality / bagua_wording / worldview_empty / opening_no_hook / personality_pending_handwrite）+ `is_known_rule_code(code)`（精确匹配，或 `worldview_` 前缀白名单）；`merge_review_reports` 的 LLM issue 降级只在**已知规则码**命中时生效——未知/自造 code 的 `error` 级问题不再被降级吞掉。
- 测试：`llm_error_severity_requires_known_rule_code`（自造 code 的 error 保留；已知 code 才降级；fixture 必须带 `"severity": "error"`，否则默认 warning——这也是我第一版测试失败的原因，已在代码注释里固定该前提）。

---

## 5 P3 清单（D-17..D-26）

### D-17 [P3] `novel_distill` 空输入永久卡死 + 硬上限溢出 — **已修复**
- `crates/domain/src/novel_distill.rs`：`chunk_novel` 用 `target.saturating_mul(2)`（`usize::MAX` 入参不再溢出/回绕）；`NovelDistillJob::new` 在 0 块时直接落终态 `Done`（此前停在 `Chunks` 且无块可写、`apply_style_formula` 又因阶段不符被拒 → 永久卡死）。
- 测试：`empty_novel_job_reaches_terminal_stage`、`huge_target_chars_does_not_overflow_hard_cap`。
- 暂缓：整模块仍**没有生产调用者**（全 workspace grep 仅本 crate 与其单测）→ 接线/删除属跨域决策（§8）。

### D-18 [P3] `history.rs` 零测试 — **已修复（测试）；`variant_id` 改名暂缓**
- `crates/domain/src/history.rs` 新增 8 条测试覆盖两个纯函数：`unknown_node_id_is_rejected`、`out_of_scope_turns_are_rejected`、`in_flight_turns_block_deletion`、`committed_body_in_removed_range_cannot_be_deleted`、`committed_turn_in_removed_range_cannot_be_deleted`、`uncommitted_tail_is_truncated_and_turns_abandoned`（含"cut 前移到 input_node + 轮次置 Abandoned + attempt 置 Discarded + pending 清空 + 原对象不被就地修改"）、`tail_truncation_keeps_earlier_nodes_and_clamps_watermark`、`require_committed_head_only_accepts_accepted_tail`。
- 暂缓：`TurnAttempt.variant_id` 改名（报告建议改为 `node_id`/`variant_node_id`）——字段跨 crate 出现在 tauri-app / app-pipeline 的持久化结构与测试里，重命名属破坏性跨域变更，本轮不动（建议 Lead 排入统一改名批次）。

### D-19 [P3] `message_layout` tail 指纹分段歧义 + builder 文档漂移 — **已修复**
- `crates/domain/src/message_layout.rs`：`full_request_fingerprint` 与 `segment_fingerprint` 的每个 tail part 先写 **`u64` 长度前缀**再写内容——`["a","b"]` 与 `["a\nb"]` 不再得到同一指纹。
- Builder 文档从"类型状态机，编译期强制顺序"改为事实描述（三字段都是 `Option`，`tail()` 用 `unwrap_or_default()` 兜底，漏填只产生空段），并在漏填 `system` 时 `tracing::warn!`（行为不变，只让它可见）。
- 测试：`tail_part_boundaries_are_not_ambiguous`、`builder_without_system_still_produces_system_message`。
- **保留**：`prefix_fingerprint`/`full_request_fingerprint` 未删除——报告称"仅测试"，复核发现 `crates/app-pipeline/src/lib.rs:7735/8168` 的测试在用（跨 crate，我改不了），删除会破坏别人的门禁。
- 影响提示：tail 指纹值随算法变化（`harness-real-llm` 记录的 `tail_hash16` 会变）；全仓无 golden 断言，缓存对比两边同算法，不影响一致性。

### D-20 [P3] 四处注释/文档漂移 — **已修复**
- `agent.rs` 的 serde 注释：删除"纯 `Subagent` 当 `Subagent("")`"的错误说法，写明会返回 unknown variant 错误（`from_str` 无该分支）。
- `agent.rs`：`is_leaf_a()` 改为 `self.chronicle_level() == ChronicleLevel::A`——与 `chronicle_level()` 的 `unwrap_or(A)` 同源，非法 level（如 99）下两个访问器不再互相矛盾。测试：`is_leaf_a_agrees_with_chronicle_level_for_valid_levels`、`is_leaf_a_is_false_for_invalid_level`。
- `story_task.rs`：`render_tasks_for_injection` 的注释重写（`LikelyCompleted`/`Completed` 无论置信度都不注入）。
- `mvu_translation.rs`：阈值注释与代码对齐（见 D-10）。

### D-21 [P3] 死结构 / `api_key` 序列化 / `Custom` 形状 — **已修复（死结构）+ 判定非问题（api_key）+ 暂缓（Custom）**
- **已修复**：删除 `agent.rs` 的 `AgentProfile`（全 workspace 零引用，已被 `AgentRunConfig`/`AgentProfileConfig` 取代），连带清理 `ToolSpec` 未用导入。
- **判定非问题（`api_key` 序列化）**：报告建议 `#[serde(skip_serializing)]`，但 `crates/tauri-app/src/connection_store.rs:86` 靠**这条序列化路径**把 `api_key` 替换成 SecretRef 后落盘（`secure_api_key` + `migrate_plaintext_api_keys` 负责迁移旧明文），加 `skip` 会让密钥静默丢失（真正的数据损失）。外泄防护的边界在别处且已成立：手写 `Debug` 打码（新增回归测试 `llm_connection_debug_masks_api_key` 钉住）、给前端只走 `LlmConnectionSummary`（含 `has_key` 布尔）、命令返回类型不含明文。已在 `llm.rs` 结构体文档里写死这条契约（"新增 `-> LlmConnection` 命令前先确认边界"），并补 `llm_connection_serde_round_trips_api_key_field` 钉住存盘往返。
- **暂缓（`LlmProtocol::Custom(String)` 形状）**：改成"统一字符串"是跨层 wire 变更（tauri-app 详情 DTO 是扁平字符串、前端有解析），超出本任务写作用域与授权；已用 `llm_protocol_custom_shape_is_stable_for_current_frontend` 钉住现状（`{"Custom":"x"}`），供后续统一批次使用。

### D-22 [P3] 死代码/不可达分支 — **部分已修复，部分暂缓，1 项判定非问题**
- **已删除（有全 workspace 证据）**：`card_studio.rs::drafts_to_st_book`（及其 `#[allow(dead_code)]`）、`card_studio.rs:1021-1025` 不可达且文案与事实相反的 `warn`（"已跳过 keys 校验"）、`prompt_module.rs::AgentBinding`（零引用）、`preset.rs::Preset::enabled_system_prompts`（零调用者）、`world_info.rs::WorldInfoEntry::matches_query`（零调用者，仅 `matches_query_lowered` 被使用）。
- **判定非问题（附证据）**：`CardShellKind::Other` —— 报告称"全 workspace 未构造"。复核：它是**前向兼容 catch-all**（外部/未来写入的 manifest 反序列化不整条失败，前端也能当"未知壳"分组），删掉反而降低兼容性；已在枚举文档里写明它是保留变体。
- **暂缓（附理由）**：
  - `card_studio.rs::apply_mvu_bootstrap_entry` + `MVU_INITVAR_MARKER`、`extra_definitions_from_st_extensions`：当前只有自身测试调用，但 `card_studio.rs:96-98` 的模块注释把它们当成"导入侧/出卡侧的接口契约"，且 MVU 出卡是我无法改写的 app/tauri 侧的在建特性——删除会打断在建接线，我改不了对面代码所以不能验证零影响。建议：域6/域3 定接线或删除后由 Lead 排批次。
  - `agent.rs::RoundSummary::to_chronicle_a`：仅本 crate 测试调用，但它是 Chronicle A 迁移的公开 API（有测试、有语义文档）；无调用者的公开 API 删除收益低。
  - `message_layout.rs` 的两个 fingerprint 方法（见 D-19：app-pipeline 测试在用）。
  - `novel_distill.rs` 整模块（见 D-17）。
- 报告本身已写"**建议 Lead 侧逐条 grep 终判后再删**"，域1 只删了可确证零调用者的 5 处。

### D-23 [P3] `world_info.rs`：死 Err 分支 / 映射重复 / `as i32` 有损截断 — **已修复**
- 删除 `set_enabled` 的不可达 `Err` 分支（保留 `Result` 签名以兼容调用方）；抽 `route_for_flags` 供 `default_route`/`from_st` 共用；新增 `narrow_i64_to_i32(value)`，越界时 `tracing::warn!` 后忽略（回退默认值），不再静默回绕。
- 测试：`from_st_route_matches_default_route_mapping`、`out_of_range_insertion_order_is_ignored_instead_of_wrapped`。

### D-24 [P3] `reverse_parse_character` 丢字段 + 内联 HTML 壳阈值单位不一致 — **已修复（wire `deferred/byte_len` 通道跨域，暂缓）**
- `card_studio.rs::reverse_parse_character`：新增 `reverse_parse_probability` / `reverse_parse_exclude_recursion` / `reverse_parse_group`，从 `extensions`（兼容 `extra`、`excludeRecursion`）恢复 `probability`/`exclude_recursion`/`group`——Mode-C"从已有卡反解析"不再静默丢触发概率/递归排除/分组。
- `card_shell.rs`：两个阈值提为命名常量 `INLINE_HTML_MIN_CHARS = 80`（字符）与 `INLINE_HTML_IPC_MAX_BYTES = 8192`（字节），文档写明各自单位；超限清空正文时新增 `tracing::warn!`（含字节数、上限、label），不再无声留下 `InlineHtml { html: "" }`。
- 测试：`reverse_parse_keeps_advanced_worldview_policy_fields`、`inline_html_shell_keeps_body_under_ipc_limit`、`inline_html_shell_over_ipc_limit_is_still_listed_without_body`、`short_html_snippet_is_not_treated_as_shell`。
- 暂缓：给 `InlineHtml` 加 tauri 侧 `deferred/byte_len` 等价取回通道（`get_card_shell_inline_js` 目前只服务 `InlineJs`）——涉及 `crates/tauri-app`（域4）与前端（域5），不在写作用域。

### D-25 [P3] Rust `classify_shell_kind` 与前端 `classifyShellUrl` 关键词表漂移 — **暂缓（跨域）+ 域1 侧权威文档已补**
- 域1 侧复核结论：**Rust 侧不是缺陷**（它是超集：find/label/url 三路），漂移来自前端只镜像了 URL 分支、且优先级相反（JS 先 status 后 home）。域1 无代码可改且前端/域5 不在写作用域。
- 已做的事：在 `classify_shell_kind` 与 `CardShellKind` 的文档注释里写清两套判定的差异点（前端不认 find/label 的中文关键词与 `statusplaceholder`）、影响（kind 目前只用于展示分组/审计，不影响挂载）、以及建议（前端改用 manifest 的 `kind`，仅在 manifest 缺失时回退 URL 关键词）。
- 另记：报告提到 CLAUDE.md 声称的 `matchesAnyInlineShellTrigger` 只存在于前端 JS（`cardShellDisplay.js:257`），Rust 侧只透传 `trigger: find`——这**不是代码缺陷**，但文档若被读成"Rust 有匹配实现"会误导 → §7。

### D-26 [P3] `RegexScript` 的 serde 默认值不对称 — **已修复（刻意保留两处无默认）**
- `crates/preset.rs`：`disabled`（false）、`flags`（""）、`placement`（`RegexPlacement::Output`，即 ST 默认 placement code 2 = AI Output）加 `#[serde(default)]` + `impl Default`。
- **刻意不加默认**：`find_regex` / `replace_string` —— 空正则会在所有文本上命中并静默改写正文，比"整份 `presets.json` 解析失败"危险得多；保持 fail closed 并用 `regex_script_missing_find_regex_still_fails_closed` 钉住。
- 测试：`regex_script_tolerates_missing_scalar_fields`（含 `Preset` 级别往返）、`regex_script_missing_find_regex_still_fails_closed`、`input_placement_filter_still_works_with_defaulted_scripts`。

---

## 6 域6 委派项（跨域委派记录）

| ID | 委派内容 | 状态 | 代码 + 测试 |
|---|---|---|---|
| M-05 | `position` 导出违反 ST V2/V3 字符串契约（导出成数字） | **已修复**（域6 已改记为"已修复(域1)"） | `world_info.rs::position_for_export`：0→`"before_char"`、1→`"after_char"`，规格外取值保留数字不伪造标签。测试 `to_st_entry_writes_spec_string_positions`（含往返） |
| M-26 | `position_as_i32` 的 `_ => 0` 静默折叠未知字符串/ST 数字 | **已修复** | `character.rs::position_as_i32` 改为穷尽 match：已知字符串标签/数字正常映射，越界数字、非整数数字、未知字符串、不支持类型各自 `tracing::warn!` 后回退 0（行为不变，从静默变可见）。测试 `position_as_i32_accepts_spec_strings_and_legacy_numbers` |
| M-28d | `extract_mvu_schema_from_extensions` 只平铺解析、不归一化（campaign 版本会归一化） | **已修复** | `variables.rs`：`extract_mvu_schema_from_extensions` 结果过 `normalize_schema_keys`；`parse_variable_objects` 改为递归 `parse_variable_objects_prefixed(map, prefix)`——嵌套 initvar 展开为点记法叶子，`default` 回退 `initial`，`is_field_definition`（含 label/type/default/initial 之一）的对象不再被递归拆散。测试 4 条：`test_extract_mvu_nested_initvar_expands_to_dotted_leaves`、`test_extract_mvu_normalizes_detected_keys`、`test_extract_mvu_field_definition_object_is_not_recursed`、`test_extract_mvu_supports_initial_key_as_default` |
| M-32.8 | `character.rs::to_st_data` 在 `raw_card_json` 解析失败时只 warn + 回退空 ST 数据（导出静默丢全部扩展字段） | **已修复** | 新增 `RAW_CARD_JSON_PARSE_FAILED_KEY` / `RAW_CARD_JSON_PARSE_ERROR_KEY` 命名空间标记写入 `extensions`（在 `extensions` 覆盖之后打，避免被抹掉），并新增 `to_st_data_with_parse_diagnostic` 返回失败原因；`to_st_data` 行为不变（除产物带标记）；`to_st_data_from_card`（Campaign 导出路径，此前连 warn 都没有）同样处理。测试：`raw_card_json_parse_failure_is_visible_in_export`、`successful_raw_card_json_parse_adds_no_marker`、`to_st_data_from_card_also_marks_parse_failure` |
| M-32.9 | 导出同时写 `order` 与 `extra.insertion_order` → wire 上两个"顺序"来源可能矛盾 | **已修复** | `world_info.rs::to_st_entry`：以 `self.order` 为唯一权威，**当且仅当** `extra` 已含 `insertion_order` 时把它的值同步为 `self.order`（不凭空发明 V3 键）。选择"同步"而非"剔除"的理由：剔除会让域6 的 wire 对账把每个 V3 卡的 `insertion_order` 判成**缺失字段（Loss）**，而同步只在真实分歧时体现为一次**值差异**（正是应当暴露的 Loss）。测试：`to_st_entry_syncs_preserved_insertion_order_with_authoritative_order`、`to_st_entry_does_not_invent_insertion_order_for_v2_entries`、`to_st_entry_ignores_out_of_range_insertion_order_key` |

- 已回执 域6：M-05/M-26/M-28d/M-32.8/M-32.9 全部纳入并实现；M-32.9 的实现方式与他们的建议（剔除 `insertion_order`）不同，理由如上——**请域6 确认 wire 对账口径**（他们正在用 `compare_export_wire_to_source` 做对账）。

---

## 7 文档需同步条目（我不改文档，交给 Lead/文档域）

1. **`CLAUDE.md` · `migrate_to` 语义**：现文"unknown versions update `config_version` but preserve data"已不准确。D-14 后语义 = `target <= config_version` 为 no-op（**绝不降级版本标记**），只有更高 target 才推进。
2. **`CLAUDE.md` / `docs/DATA_MODEL.md` · `story_clock` 单一权威**：新增 `DEFAULT_STORY_CLOCK = "第1天"`（domain `variables.rs`）为唯一默认值来源，`campaign.rs::default_story_clock()` 委托它；新建 Campaign 顶层字段现在从 schema 的 `story_clock` default 取值，**不再恒为分歧**。DATA_MODEL §97"未来可通过数据迁移统一"已滞后于现状。
3. **`CLAUDE.md` · Card shell**：
   - `CardShellKind::Other` 是保留的前向兼容 catch-all，域内不构造（D-22 判定非问题）；
   - 前端 `classifyShellUrl` 与 Rust `classify_shell_kind` 关键词表不等价（前端只有 URL 分支、优先级相反），`matchesAnyInlineShellTrigger` 只存在于前端 JS（D-25）；
   - 内联 HTML 壳：`>8192` 字节正文不进 manifest（现已有 warn），`InlineHtml` 无 `deferred/byte_len` 取回通道（D-24 暂缓项）。
4. **`CLAUDE.md` · 出卡闸门**：新增 `KNOWN_RULE_CODES` 白名单——LLM review 报告的 `severity` 只对已知规则码做降级，未知 code 的 error 级问题不再被吞（D-16）。
5. **`CLAUDE.md` / `docs/ARCHITECTURE-AUDIT.md` · `write_fence`**：`write_fence::freeze` 现在有了**真实生产调用者**（infra-vector 不可恢复损坏），语义是"该路径 `atomic_write` 直接 `PermissionDenied` 直到 `unfreeze`"；新增只读诊断面 `freeze_with_reason` / `frozen_reason` / `frozen_entries` / `FrozenEntry::summary` / `storage_health_report()`（`kind = "write_fence_frozen"`）。**应用内一键解除命令属产品决策（Lead 记为 N-03，P2 暂缓）**；当前文案指向"备份/移走文件后重启"的人工路径，tauri-app 侧消费接线待域4/域2 补（§8-1）。
6. **`CLAUDE.md` · 导演注入文案**：任务/伏笔注入标题由单一「即将触发的任务/伏笔」改为两组：`【已满足条件的任务/伏笔】` 与 `【待判断的任务/伏笔】`（后者带"不要提前揭示后续剧情"指令），空集合仍返回 `""`（Lead D-04 裁定）。
7. **`CLAUDE.md` · 世界书导出**：`WorldInfoEntry::to_st_entry` 会把 `extra.insertion_order` 同步为权威 `order`（M-32.9）；`to_st_data` 解析失败会在 `extensions` 写 `storyforge_raw_card_json_parse_failed` / `storyforge_raw_card_json_parse_error`（M-32.8）——域6 的 wire 对账会看到该键，这是预期信号。
8. **`CLAUDE.md` · 全文搜索/指纹**：`message_layout` 的 tail 指纹加了 `u64` 长度前缀（D-19），`harness-real-llm` 记录的 `tail_hash16` 数值会变（无 golden 断言）；`prefix_fingerprint`/`full_request_fingerprint` 仍只被测试/诊断使用（生产用 `segment_fingerprint`）。
9. **`docs/DOCS-CODE-AUDIT.md`**：如收录了 `AgentProfile`（agent.rs）、`AgentBinding`（prompt_module.rs）、`Preset::enabled_system_prompts`、`WorldInfoEntry::matches_query`、`card_studio::drafts_to_st_book` 等符号，需删除（本轮已按零调用者证据删除）。
10. **`CLAUDE.md` · 字数/复杂度启发式**：`score_card_complexity` 的 `script_bytes` 只统计真脚本源（`depth_prompt.prompt` 正文不计入字节阈值，但仍参与模式计数）；`getElementById`/`$.`/`script_blocks` 只进 `counts` 不参与 Heavy 判定（D-10/D-20）。
11. **`docs/DATA_MODEL.md`**：`RegexScript.placement` 现在有缺省值（`Output` = ST placement code 2），但 `find_regex`/`replace_string` **无**默认（fail closed）——与既有"derived from ST regex_scripts"描述一起补（D-26）。
12. **`CLAUDE.md` · 向量库**：文件损毁（主文件与 `.tmp` 均不可解析）现在留 `.corrupt` 备份 + 冻结写入（D-03）；冻结可通过 `write_fence::storage_health_report()` 报告"哪份文件被隔离、为什么、怎么办"（tauri-app 侧消费接线待域4/域2 补，§8-1），一键解除属 N-03 暂缓。

---

## 8 跨域 handoff 与剩余风险

| # | 内容 | 交给谁 | 等级 | 现状 |
|---|---|---|---|---|
| 1 | **向量库损坏冻结的观测与恢复**：`write_fence::freeze_with_reason("vectors.json", …)` 生效后所有向量写入持续 `PermissionDenied`（fail closed，比静默空库安全）。**域1 已补**：原因/时间可查（`frozen_reason`/`frozen_entries`）+ `storage_health_report()` 报告（"哪份文件被隔离、为什么、怎么办"）。**仍未完成**：tauri-app 侧消费接线（`lib.rs:841-843` 后 sweep 一次，3 行），以及应用内一键解除入口（Lead N-03，P2 产品决策，本轮不做）；重启后文件仍损坏会再次冻结 | 域4/域2（一行 sweep）+ Lead N-03（UI 按钮/文档步骤） | P1 | 域1 侧 API 与测试已就绪；`crates/tauri-app/**` 不在我写作用域 |
| 2 | `crates/infra-sqlite/src/importer.rs:553` 仍以 `"Day 1"` 兜底 story_clock，与新的 `DEFAULT_STORY_CLOCK = "第1天"` 不同源 | 域2 | P2 | 域1 已统一 domain 侧默认值；SQLite 导入路径未改（写作用域外） |
| 3 | `story_clock` 历史数据的顶层字段仍可能是 `"Day 1"`：域2 的 repair 路径（`campaign_store.rs:48-66`）会改写并 warn；import/export 字节比对仍依赖显式 repair 绕行 | 域2 / 域6 | P2 | 域1 保证新档不再分歧 |
| 4 | `crates/app-pipeline/src/turn_dossier.rs` 空时钟：域3 已完成根因修复（两处调用点传 `&ctx.story_clock`），域1 的分组渲染是兜底 | 域3 + Lead R7 | P1（D-04 根因） | 建议 R7 钉一条跨域测试，防止调用点回归 |
| 5 | 前端 `classifyShellUrl` 与 Rust `classify_shell_kind` 漂移（D-25）：建议前端改用 manifest `kind`，仅缺失时回退 URL 关键词 | 域5 | P3 | 域1 侧文档已写明差异；前端未改 |
| 6 | 内联 HTML 壳需要与 `InlineJs` 等价的 `deferred/byte_len` 取回通道（>8KB 正文目前只在 manifest 里留 trigger） | 域4（tauri-app 命令）+ 域5 | P3 | 域1 已加 warn 可见性 |
| 7 | `novel_distill` 整模块无生产调用者（域1 已修 2 个内部缺陷） | 域3 / Lead | P3 | 决定接线还是删除 |
| 8 | `TurnAttempt.variant_id` 改名（D-18 暂缓）：跨 tauri-app/app-pipeline 持久化字段，需统一批次 | Lead | P3 | 域1 未动 |
| 9 | `LlmProtocol::Custom(String)` 形状统一（D-21 暂缓）：跨 tauri-app DTO + 前端解析 | 域4 + 域5 | P3 | 域1 已用测试钉住现状 |
| 10 | `variables.rs` 大写 `STAT_DATA.` 前缀不收敛（§5.2-5）：分析器提示词已钉死记法，风险低 | 域1 后续批次 | P3 | 本轮不动（改归一化会波及存量键语义） |
| 11 | `secret_store::resolve_secret_value` 读取失败返回 `Err`，调用方**不得**把 `storyforge-secret:v1:...` 当 API key 用（§5.2-2） | 域4/域2 核查 `infra-llm`/`connection_store` 的 fallback | P2（跨域） | 域1 只记录，未改 |
| 12 | `infra-vector` 崩溃恢复后可能留下陈旧的 `.tmp`（§5.2-4）：不丢数据，仅冗余 | 域1 后续 | P4 | 良性缺口，未改 |

**未跑**：`cargo test --workspace`（Lead 独占）；`cargo check -p storyforge-app-pipeline --all-targets` 我只在被域3 报告红树时通过域内修复间接解决（域3 已复跑）。

---

## 9 门禁证据（域内，逐包）

| 命令 | 结果 |
|---|---|
| `cargo check -p storyforge-domain --all-targets` | **exit 0**，`Finished`，0 warning |
| `cargo test -p storyforge-domain` | **388 passed / 0 failed**（lib），集成测试 0 条；基线 343 → +45 |
| `cargo check -p storyforge-infra-vector --all-targets` | **exit 0**，`Finished` |
| `cargo test -p storyforge-infra-vector` | **21 passed / 0 failed**（新增 D-03×2 + D-05×3；D-03 的测试含冻结健康报告断言） |
| `cargo check -p storyforge-infra-regex --all-targets` | **exit 0**，`Finished`（48.9s） |
| `cargo test -p storyforge-infra-regex` | **25 + 1 passed / 0 failed**（仅文档/文案变更，无新增测试） |
| `cargo test -p storyforge-infra-util` | **12 passed / 1 ignored**（D-03 追加：`frozen_reason_and_health_report_explain_the_freezed_file`、`freeze_without_reason_still_reports_a_default_reason`） |

**新增/更新的测试清单（域1）**：D-02×4、D-03 追加×2（infra-util）、D-04×9、D-05×3、D-06×5、D-07×2、D-08×1、D-09×1、D-10×1、D-11×4、D-12×1、D-13×3、D-14×2、D-15×1、D-16×1、D-17×2、D-18×8、D-19×2、D-20×2、D-21×3、D-23×2、D-24×4、D-26×3、M-05×1、M-26×1、M-28d×4、M-32.8×3、M-32.9×3、§5.2-1×1。

**改动文件（域1，全部在写作用域内）**
- `crates/domain/src/`：`variables.rs`、`campaign.rs`、`chronicle.rs`、`story_task.rs`、`world_info.rs`、`character.rs`、`card_studio.rs`、`conversation.rs`、`mvu_translation.rs`、`prompt_module.rs`、`agent_profile_config.rs`、`agent.rs`、`preset.rs`、`novel_distill.rs`、`history.rs`、`message_layout.rs`、`card_shell.rs`、`narrative_contract.rs`、`llm.rs`
- `crates/infra-vector/src/lib.rs`
- `crates/infra-regex/src/lib.rs`
- `crates/infra-util/src/write_fence.rs`（Lead 裁定 D-03 追加的可观测性：`freeze_with_reason`/`frozen_reason`/`frozen_entries`/`FrozenEntry::summary`/`storage_health_report` + 2 条测试）
- `docs/review-2026-09-13/fixes/01-domain-infra-fixes.md`（本文件）

---

## 10 Lead 裁定落实记录（2026-09-13）

| 裁定 | 落实 |
|---|---|
| 1. D-03 保持 fail-closed，但必须可观测、可恢复（不降级为"仅日志"） | **已落实**：`write_fence` 增加 `freeze_with_reason(path, reason)` / `frozen_reason(path)` / `frozen_entries()` / `FrozenEntry::summary()` / `storage_health_report()`（`kind = "write_fence_frozen"`，文案含隔离文件、原因、人工处理建议）；`infra-vector` 以详细原因冻结；测试 `frozen_reason_and_health_report_explain_the_freezed_file`（infra-util）+ `unrecoverable_corruption_freezes_writes_and_keeps_backup` 扩展（infra-vector）证明"冻结后健康报告体现该状态"。**应用内一键解除 = N-03（P2，产品决策）本轮不做**；tauri-app 侧 3 行 sweep 接线不在我写作用域，已给出代码片段交域4/域2 |
| 2. D-22 的 4 项暂缓死代码：维持暂缓、不删 | **已落实**：§5 D-22 保持"暂缓（在建接口，附理由）"，记录中未删（`apply_mvu_bootstrap_entry` / `extra_definitions_from_st_extensions` / `to_chronicle_a` / `novel_distill` 整模块） |
| 3. D-19 指纹值变化：接受，需写明跨版本不可比 | **已落实**：§7-8 与 §10 已写明"`harness-real-llm` 历史日志中的 `tail_hash16` 与新算法不可直接对比，如需跨版本对比需重算基线" |
| 4. `insertion_order`：维持"同步"方案 | **已落实**：§6 M-32.9 记录维持同步实现；域6 已确认（剔除会产生每个 V3 条目的假 Loss 噪声），双方口径一致，无需再改 |

---

## 11 需要 Lead 重点复核的点

1. **D-03 的 fail-closed 后果（已按裁定处理）**：冻结 `vectors.json` 后向量写入持续 `PermissionDenied`，域1 已补可观测面（§3 D-03 / §10-1）；**仍需域4/域2 加一行 sweep** 才能到前端，应用内一键解除为 N-03 暂缓。
2. **D-10 的刻意差异**：正文不计入字节阈值、但仍参与模式计数——与报告建议不完全一致，理由是保住既有 `depth_prompt 内嵌 JS` 的识别路径（见 §3 D-10）。
3. **M-32.9 的实现选择**：同步 `insertion_order` 而非剔除；域6 已书面确认该口径（剔除会产生每个 V3 条目的假 Loss 噪声）。
4. **D-19 指纹值变化**：`tail_hash`/`tail_hash16` 数值随算法改变，全仓无 golden 断言；跨版本对比需重算基线。
5. **D-22 的 4 项暂缓死代码**：按裁定维持暂缓（在建接口）。
6. **D-04 与域3 的口径一致性**（Lead 明确要求）：域1 `check_trigger` 三态 + 分组渲染；域3 已传真实 `story_clock`；Lead 已把"R7 跨域接缝测试"写入 R7 任务描述。

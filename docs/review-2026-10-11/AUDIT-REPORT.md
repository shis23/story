# StoryForge 全量审计报告（2026-10-11）

范围：16 个 Rust crate + Vue 前端，覆盖冗余代码清理与逻辑漏洞修补。
基线：`main @ 0f1c8b0`。本轮改动全部在未提交工作树上，未 commit。

方法：
- 冗余：脚本扫描 Rust `pub` 项与前端导出/文件的跨仓引用（生产 / 测试 / 零引用三分），
  每一项删除前逐个 `rg` 复核（含 `tests/`、脚本、A 组文档）。
- 漏洞：按「错误被吞 / 内存与磁盘分叉 / 作用域隔离 / 失败后状态不一致 / 用户入口缺失」
  五类逐 crate 排查；每个修复都配一条**在旧代码上会失败**的回归测试。
- 尊重既有裁定：2026-09-13 审查里 Lead 明确「保留」或「暂缓」的项（W-13/W-14 等）
  首轮不动，只在 §4 列出。F-46 遗留写作组件与 `getInstance` wrapper 首轮按旧裁定保留，
  第二轮经用户明确指示删除（见 §2.1）。第三轮处理 §4 的遗留项并补齐缺失界面（见 §1.5），
  用户确定「嵌入模型是可选项」。W-13（摘要按角色隔离）第三轮曾实现，经用户评估后撤回，
  维持 2026-09-13 的暂缓裁定（理由见 §1.5 末尾）。

---

## 1 逻辑漏洞修复（按影响排序）

### 1.1 记忆与检索的作用域隔离

| # | 问题 | 修法 | 回归测试 |
|---|------|------|----------|
| L1 | **远记忆召回整体失效**：仅关键词索引的 RoundSummary 向量为空，向量阶段在维度检查上报错，`?` 把已算好的关键词命中一起丢掉 | `infra-vector` 检索跳过空/全零向量；`app-memory::merge_vector_stage` 检索失败回退关键词结果 | `vector_search_skips_keyword_only_and_zero_vectors`、`vector_stage_tolerates_keyword_only_records`、`vector_stage_falls_back_to_keyword_hits_on_search_error` |
| L2 | **Agent `search_vectors` 工具跨 Campaign 泄漏**：向量库是全局单例，工具不做任何过滤，别的 Campaign 的剧情摘要会进入本局导演上下文 | `vector_hit_visible_in`：带 `campaign_id` 的记录只对同局可见；无活跃 Campaign 时看不到任何 Campaign 记录；带 `owner_character_id` 的记录对绑定其它实例的子 Agent 不可见；先过滤再截断 | `search_vectors_scope_hides_other_campaigns_and_private_knowledge` |
| L3 | **legacy 写作路径召回全部 Campaign 与其它会话的远记忆**：`accepts_campaign(_, None)` 无条件放行 | 新作用域 `accepts_scope`：legacy 只收无 `campaign_id` 标签、且（若带标签）属于本会话的归档；`fill_far_memory_hits` 传入当前 `conversation_id` | `legacy_recall_is_scoped_to_its_own_conversation`；更新 `test_recall_archived_by_query_filtered_by_campaign`（旧断言把 A/B 两局都召回的泄漏锁成了预期） |
| L4 | **本局远记忆被别的 Campaign 挤掉**：先取全局前 `limit*8` 条再按 campaign 过滤 | 关键词/向量两条路径都改为在向量库内按 `MetadataFilter::for_campaign` 过滤后再截断 | `campaign_recall_is_not_starved_by_other_campaigns` |
| L5 | **`search_vectors` 中文查询基本零命中**：只按空白/ASCII 标点切词，整句成一个 token | 切词函数 `extract_query_tokens`（中文 2-gram）下沉到 `infra-vector`，远记忆召回与工具共用（`app-memory` 保持 re-export） | `search_vectors_matches_chinese_queries_without_spaces` |

### 1.2 删除路径的派生数据清理

| # | 问题 | 修法 | 回归测试 |
|---|------|------|----------|
| L6 | **删档后远记忆向量永久滞留**：全仓没有生产代码调用 `delete_by_campaign`，死数据持续占用 H-7 配额，满额后所有 Campaign 的远记忆索引一起停摆 | `vector_index::purge_campaign_vectors`，接入删档、删卡级联、删角色级联三条路径（派生索引：失败只告警，不回报删除失败）；**存量残留**由启动对账清理（L24） | `delete_playthrough_purges_only_that_campaigns_vectors` |
| L7 | **删掉的角色卡 / 世界书条目仍能被 `search_vectors` 搜到**：世界书向量只在导入与新增条目时 upsert，删卡、删条目、改条目、改路由都不碰向量库；重导时按关键词去重还会误删其它卡共享关键词的条目 | 新模块 `tauri-app/src/vector_index.rs`：`sync_world_info_vectors` 按角色库**全量重建** `WorldInfo` 记录（`VectorStore::replace_kind` 一次落盘；记录 id `wi:{角色id}:{序号}`），接入 `import_character` 与 `rebuild_world_info_in_tool_ctx`（覆盖增删改条目、改路由、删卡、删角色），并在活跃活动早退之前执行；启动时再重建一次，清掉无标签旧记录。旧的「按关键词删 + 逐条 upsert」与来源标签去重逻辑删除 | `world_info_vectors_follow_the_character_library` |
| L8 | `delete_card` 在「卡不存在」分支吞掉快照恢复失败 | 恢复失败以 storage 错误上报 | （既有 delete_card 测试覆盖主路径） |
| L22 | **分支 / 导入的 Campaign 没有远记忆**：bundle 导入与 fork 都给 RoundSummary 换新 id，但从不为新 id 建向量索引——新局在产出新摘要前远记忆召回为空，导入提示里的「向量索引将重建」从未兑现 | `vector_index::index_campaign_summaries`，`fork_campaign` / `import_campaign_bundle` 成功后调用：分支按正文**复用来源局已有向量**（不重新调用嵌入 API）；其余 A 级摘要先写关键词索引（立即可召回），配置了 Embedder 时后台补嵌入（与 accept 后同一策略） | `fork_indexing_reuses_source_vectors_and_keyword_indexes_the_rest` |
| L23 | **修复前已经分支 / 导入的局**同样缺索引 | 启动对账 `reconcile_on_startup`（setup 后台线程）补齐所有 Campaign 缺失的摘要索引，**只写关键词、不调嵌入 API**，已有记录原样保留，幂等 | `startup_backfill_indexes_only_missing_summaries` |
| L24 | **修复前删掉的局**的向量记录仍占配额 | 启动对账清理 `campaign_id` 指向已不存在 Campaign 的记录（仅当存储确认 `Ok(None)`；读取出错一律保留；无标签记录不动） | `startup_purges_vectors_of_deleted_campaigns_only` |
| L25 | 批量写向量逐条全量落盘：重建几百条摘要 = 几百次重写整个向量库文件 | `VectorStore` 新增 `upsert_batch` / `replace_kind` / `records_matching` / `contains`（BruteForceStore 单次落盘，失败整批回滚；配额只拦净增长，整类替换不会因满额被拒）；摘要索引统一走 `runtime_support::index_summaries_with_vectors` | `upsert_batch_persists_once_and_rolls_back_as_a_whole`、`upsert_batch_enforces_quota_on_net_growth_only`、`replace_kind_swaps_only_that_kind`、`records_matching_filters_by_metadata` |

### 1.3 失败后状态一致性

| # | 问题 | 修法 | 回归测试 |
|---|------|------|----------|
| L9 | **建档留下悬空 `conversation_id`**：`create_campaign_for_backend` 用无 Result 的 `ConversationStore::create`，外部持久化失败时返回未落库会话，Campaign 照常落库 | 改用 `create_persisted` 并传播错误 | `create_campaign_for_backend_fails_closed_when_conversation_cannot_persist` |
| L10 | **流式重试重复输出**：`RetryingClient::chat_stream` 在已转发部分 chunk 后仍重试，前端收到两份正文拼接 | 代理 channel 记录是否已转发；已转发则不重试 | `chat_stream_does_not_retry_after_forwarding_chunks`、`chat_stream_still_retries_before_any_chunk` |
| L11 | **SSE 流内 `error` 事件被静默当作正常结束** | `StreamDelta.error` 解析并分类（429/rate_limit → RateLimited，其余 ServerError） | `sse_error_event_mid_stream_fails_the_stream`、`sse_error_event_rate_limit_and_null_error` |
| L12 | **向量库内存/磁盘分叉**：upsert/delete 先改内存，落盘失败不回滚 | 落盘失败回滚内存（`delete_where` 统一实现） | `persist_failure_rolls_back_in_memory_mutations` |
| L13 | **插件注册表落盘失败被静默**：内存已改、磁盘未改 | `persist` 返回错误，install/uninstall/set_enabled 失败回滚 | `persist_failure_is_reported_and_rolled_back` |
| L14 | **归档总结为空仍推进水位** | 空正文视为缺口错误，不推进 | `blank_summary_is_a_gap_not_a_success` |
| L15 | 同一会话可并发启动两个水位归档（重复总结、水位竞争） | 进程内 per-conversation in-flight 守卫，第二个直接返回 0 | `archive_flight_guard_is_exclusive_per_conversation` |
| L16 | `soft_delete_variant` 吞掉「标记 Attempt 丢弃」失败后照常软删，留下「变体已删、Attempt 仍是 Draft」 | 标记失败在软删前中止 | — |
| L17 | 其余 `update_turn_record` 结果被 `let _ =` 吞掉（切换变体标 Stale、写作失败标 Failed、Attempt 落盘失败标 Failed）；世界书向量 upsert/delete 结果被吞 | 改为 `tracing::warn!` 留痕（这些路径主操作已完成，不改返回语义） | — |

### 1.4 兼容性与功能缺口

| # | 问题 | 修法 | 回归测试 |
|---|------|------|----------|
| L18 | **ST 正则替换语义偏差**：`{{match}}` 未展开、`trimStrings` 不生效、JS 模板字面量 `${...}` 与 `$$` 被误当捕获组 | `replace_match_macro` + `expand_st_replacement`（只认 `$数字`，引用组内容按 trimStrings 清理，其余字面保留） | `st_match_macro_expands_to_whole_match`、`st_trim_strings_apply_to_referenced_groups`、`st_replacement_keeps_js_template_literals_and_dollar_pairs` |
| L19 | **用户无法导入 ST 预设**：预设面板空态写「请先导入」，但没有任何入口（`importPreset` wrapper 自初版零调用） | `PresetPanel.vue` 增加「导入预设」按钮（对话框 → `readFile` → `import_preset` → 刷新），复用全局正则导入的同一套 capability | `frontend/tests/components-v2/preset-panel-import.test.mjs`（2 条） |
| L20 | 日志目录无保留期，按天文件无限增长 | `LogStore::new` 启动时清理超过 `LOG_RETENTION_DAYS = 30` 天的日志 | `test_log_store_prunes_expired_files_on_startup` |
| L21 | 写入冻结提示只说「已冻结」，不告诉用户怎么办 | `FrozenEntry::summary()` 补处理建议 | — |

### 1.5 第三轮：可选嵌入、消息归档迁移、缺失界面

| # | 问题 | 修法 | 回归测试 |
|---|------|------|----------|
| L26 | **没配嵌入模型就没有消息归档**：`MemoryArchiver` 强依赖 Embedder，未配置时整段归档直接跳过；嵌入接口出错时同样丢弃整段 | Embedder 改为可选：未配置或嵌入失败时写**仅关键词**的归档记录（照样可召回、水位照常推进），只有向量库写入失败才截断水位；归档记录新增 `message_start` / `message_end` 元数据 | `archive_prefix_without_embedder_writes_keyword_records`、`archive_prefix_keeps_keyword_record_when_embed_fails` |
| L27 | **分支 / 导入后消息归档丢失**（原 §4 第 1 条）：bundle 把 `archived_upto` 置 0，新局要重新花 LLM 调用总结 | 分支：复制来源会话的归档（复用向量，不调嵌入 API）；导出：bundle 携带归档正文与消息区间（`BundleRuntime.message_archives`，空时不序列化，旧包兼容）；导入：写成关键词记录。两条路径都按归档实际覆盖的连续区间收紧水位（`carried_archive_watermark`），中间缺段时停在缺口前，由自动归档补上 | `carried_watermark_never_skips_unarchived_messages`、`fork_copies_message_archives_with_vectors_and_watermark`、`bundle_archives_round_trip_as_keyword_records` |
| L28 | **嵌入配置无界面、旧记忆无法补向量**（原 §4 第 2、3 条） | 「连接配置」新增「嵌入模型（可选）」卡片（`EmbeddingConfigCard.vue`）：预设、测试并保存（`Embedder::probe_dim` 实测维度，留空自动识别，与填写不一致时报错）、保留已存密钥、停用（`clear_embed_config`，已有向量保留）；`get_memory_index_status` 显示归档条数与缺向量条数；`rebuild_memory_embeddings` 一键补建（只补空向量或维度不符的记录，单飞，首个接口错误即停并报告进度） | `backfill_embeds_keyword_only_and_wrong_dimension_records`、`backfill_stops_at_the_first_embedding_failure` |
| L29 | **卡片世界书只读**，且后端编辑命令改的是投影 `world_info_entries`，完整世界书 `embedded_world_info`（导出 / 重新投影的权威）不变——编辑在下次投影或导出时丢失 | 编辑以 `embedded_world_info` 为权威：增删改、改路由、批量替换都写完整书再重新投影（禁用条目保留在书内、不进投影；`is_global` 存为条目扩展 `sf_global`；手动新增条目标 `sf_source: "user"`）；没有完整书的旧卡先由投影物化；投影与书已分叉的旧条目按关键词 + 正文对齐，对不上的收编进书而不是丢弃。前端 `CardWorldInfoEditor.vue` 替换 `WorldInfoReadonlyPanel.vue` | `edits_land_in_the_full_book_and_the_projection_follows`、`route_update_toggles_disabled_with_the_route`、`add_and_delete_use_full_book_indexes`、`legacy_card_without_book_is_materialized_from_its_projection`、`diverged_projection_entries_are_kept`、`bulk_replace_keeps_book_fields_and_disabled_entries` |
| L30 | 改世界书路由只就地改 tool_ctx 索引，向量检索池不跟着变 | `update_world_info_route` 改为调用 `rebuild_world_info_in_tool_ctx`（同时重建世界书向量） | （L7 的 `world_info_vectors_follow_the_character_library` 覆盖重建路径） |
| L31 | **角色库删卡留下孤儿角色**：`CharacterList` 只删卡片，库里的角色、Campaign 级联数据、向量都还在；`delete_character` 用调用方传入的 id 做级联，传 `source_character_id` 时级联落空 | `CharacterList` 删卡后接着 `delete_character`（找不到视为已删）；`delete_character` 先解析到存储里的角色 id 再级联 | — |

**撤回：W-13 摘要按角色隔离。** 第三轮曾给 `RoundSummary` 加受众（本轮在场角色），让角色子 Agent
只拿到自己在场时的摘要 / 远记忆，并要求摘要只写在场者可观察的事实。评估后整体撤回、代码与提示词
恢复原状，理由：
- 摘要是叙述者（Director、Editor、Chronicle B/C 压缩、远记忆召回）的长期记忆。只写「可观察事实」会让
  秘密、动机、伏笔从长期记忆里消失，直接损害剧情连续性。
- 角色已经有独立的记忆系统：`CharacterKnowledge` 按实例注入（`knowledge_for_instance`），后处理只给
  在场角色写知识。按受众过滤摘要与它重复，还会让角色丢掉旧摘要提供的连续性（旧数据没有受众）。
- 角色子 Agent 继续拿到共享的近期摘要与远记忆，并带「勿泄露你角色不该知道的信息」提示（既有行为）。

---

## 2 冗余代码清理

每项删除前均确认：全仓零引用（含 `tests/`、`scripts/`、A 组文档）。

| crate | 删除项 |
|-------|--------|
| app-memory | `MemoryRecaller`（及 `with_min_score` / `recall_by_keywords`），与 `recall_archived_*` 重复且零调用 |
| app-meta | `PresetReport` |
| domain | `Character::to_meta` + `CharacterMeta`、`CharacterDefinition::merge_variable_schema`、`ChronicleEntry::is_uncovered` / `is_leaf`、`new_lineage_id`、`FunctionCall::parse_args`、`DerivationOutcome`、`TurnRecord::find_attempt_by_variant_mut`；私有 `regex_script_applies_to_placement` 与 infra-regex 的同名逻辑合并为 `RegexScript::applies_to_placement` |
| infra-regex | `script_applies_to_placement`（重复）、`split_by_placement`、`script_summary` / `ScriptSummary` |
| infra-plugin-host | `PluginRegistry::list_enabled`、`plugins_for_slot`、`MvuLoadAck` |
| infra-import | `st_data_to_value` |
| infra-sqlite | `database_file_hash`、`UnitOfWork::transaction_mut`、`delete_world_info_payload`（删档级联已在 `delete_campaign_cascade` 内删 `campaign_world_info`） |
| harness-real-llm | `total_prompt_tokens` / `total_completion_tokens`、`assert_enabled_or_skip` |
| tauri-app | `CampaignStore::disable_json_access`、`save_character_card_force_rerun_async`（`#[allow(dead_code)]` 测试残留）、`sync_autofix_attempt_with_provenance`、`sqlite_runtime::delete_world_info` / `recover_active_preaccept_state`（仓储方法保留，infra-sqlite 集成测试在用）、`sqlite_db_path`、`CampaignMutationCoordinator::has_active_turn`；`delete_character` 里描述已过时的注释改正 |

### 2.1 前端（第二轮，用户指示删除）

| 删除项 | 说明 |
|--------|------|
| `components-v2/writing/` 整个目录（`CampaignOverview`、`ChatMessage`、`Composer`、`ConversationHistoryList`、`ConversationViewport`、`GreetingSelector`、`ProcessReview`、`StreamingMessage` 8 个 `.vue`） | 阶段 8 遗留，零引用、未接线；生产写作面在 `design/writing/**` |
| `components/base/BaseDropdown.vue` | 唯一调用方是上面的旧 `ChatMessage.vue` |
| `design/history/fixtures.js`、`design/writing/fixtures.js` 的 `fxUserMessageOnly` | 零引用的设计稿夹具 |
| `tauri-api.js` 的 `getInstance` | 零调用；后端 `get_instance` 保留并登记进 `RETAINED_NO_FRONTEND_CALLER`（保留 API 23 → 24） |
| `components-v2/campaign/WorldInfoReadonlyPanel.vue`（第三轮） | 由可编辑的 `CardWorldInfoEditor.vue` 取代（L31） |
| `components-v2/writing/.gitkeep`（第三轮） | 目录已无内容 |

同步更新：`FRONTEND-COMPONENTS.md`（§5 注明为能力角色、§13 删除遗留存档行、保留 API 计数）、`ROADMAP.md`、`design/writing/CONTRACT.md` 第 6 条、`design/history/CONTRACT.md`；`useMessageVariants.js` / `usePipeline.js` / `stores/writing.js` 里指向 `StreamingMessage` 的注释改指现行载体。

**刻意保留**（看似冗余，但有书面依据）：
- `domain::novel_distill` 整个模块与 `DEFAULT_CHUNK_CHARS`：Card Studio Phase 2 骨架（#23），属后续阶段。
- `chronicle::DEFAULT_AUTO_RECALL_K`、`ST_REGEX_PLACEMENT_MARKDOWN_DISPLAY`：规格常量 / ST 编码表的文档化。
- `adapter/useCampaignScreenAdapter.js`、`useMetaScreenAdapter.js`：`FRONTEND-DECOUPLING-AND-PLAN` 标注的整屏切换脚手架。
- `components-v2/st/StCompatibilityBadge.vue`：`FRONTEND-COMPONENTS.md` 组件目录内的设计组件。
- `#[cfg(test)] *_in_store` JSON 直连分叉与 `ConversationStore::fork_at`：仅测试使用，但承载大量既有回归测试，迁移成本高于收益。

---

## 3 验证

| 门禁 | 结果 |
|------|------|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace` | 2220 通过 / 0 失败 / 33 ignored（真模型用例，需凭据）；首轮 2206，第二轮 2213，第三轮 +7（W-13 撤回时其 9 条测试一并移除） |
| `npm test`（node:test） | 533 / 533 通过 |
| `npm run test:ui`（vitest） | 33 文件 / 159 条通过（输出里的 happy-dom iframe teardown 网络错误为既有噪声） |
| `npm run build` | 通过（>560KB chunk 警告为既有） |
| `node scripts/architecture/backend-baseline.mjs` | 门禁通过（命令 178 / 注册 178 / 孤儿 17，与登记一致；`applicationLegacyStoreAccessorReferences` 仍为 0）。第三轮新增 3 条命令（`clear_embed_config`、`get_memory_index_status`、`rebuild_memory_embeddings`），7 条保留 API 接上前端后移出 `RETAINED_NO_FRONTEND_CALLER`（`configure_embedder`、`get_embed_config`、`add_world_info_entry`、`update_world_info_entry`、`delete_world_info_entry`、`update_world_info_route`、`delete_character`）；`frontend/tests/tauri-command-contract.test.mjs` 计数 175 → 178 |

---

## 4 未处理 / 需要决策

首轮列出的 4 项已在第二轮处理：遗留写作组件与 `BaseDropdown`、`getInstance`（§2.1）；分支 / 导入无远记忆（L22/L23）；删卡后世界书仍可被搜到（L7）。第二轮列出的消息归档迁移、补建向量、嵌入配置无界面已在第三轮处理（L26–L28）；W-13 撤回（§1.5）。仍未处理：

1. **补建向量需要手动触发**：保存嵌入配置后不会自动对存量记忆批量调用嵌入 API，界面提示缺向量条数，由用户点「补建向量」。新产生的摘要 / 归档在配置后自动带向量。
2. **regenerate 单独重跑子 Agent 不走 Campaign 人设注入**：该路径的 system 段仍用 Plan 里的 `context_package`，不像 `spawn_subagents` 那样注入实例的 persona / 知识 / 变量（既有行为，未改）。
3. Meta Agent 的角色库维护批量补丁（`update_world_info_entries_bulk_multi`）沿用「整表替换投影」语义，现在会同步进完整世界书，但条目按「关键词 + 正文」对齐（投影没有稳定 id）：补丁改了关键词或正文的条目会当作新条目写入，原条目上 StoryForge 不编辑的 ST 字段（备注、扩展等）不随之保留。逐条编辑（界面与单条命令）按书内序号定位，不受影响。
4. 2026-09-13 审查 §8.2 的其余暂缓项（W-13 摘要受众隔离、W-14 历史归档 `campaign_id` 回填、R12 系列等）维持原裁定。

## 5 行为变更提示

- legacy（无 Campaign）写作的远记忆现在**只**召回本会话的归档；此前会召回所有 Campaign 和其它会话的摘要。第三轮起不带会话标签的旧归档也不再召回（无法判断它属于哪个会话）。
- `search_vectors` 结果现在按 Campaign / 角色归属过滤，中文查询按 2-gram 匹配，命中数会与此前不同（更准确）。
- 流式请求在已向前端输出内容后失败将直接报错，不再静默重试并重复输出。
- `soft_delete_variant` 在 Attempt 标记失败时返回 storage 错误（此前假成功）。
- 角色库世界书进入 `search_vectors` 检索池的规则与 tool_ctx 重建一致：路由 Selective **和 Both** 的条目都会被索引（旧导入路径漏了 Both）；删除 / 改路由后的条目立即搜不到。
- 应用启动后会在后台做一次向量索引对账（世界书重建、清理已删局残留、补齐缺失摘要索引），只读写本地文件，不产生 API 调用。
- 未配置嵌入模型时自动归档照常进行（仅关键词召回）；此前直接跳过。
- `configure_embedder` 新增可选参数 `keep_existing_key`（缺省等同旧行为），并在保存前实测接口：接口不可用或维度不一致时保存失败，不再写入一份不可用的配置。
- 卡片世界书的编辑（含 Meta Agent 批量补丁）现在写进完整世界书，导出 ST 卡与重新投影都能保留；禁用条目留在书内。
- 角色库里删卡会同时删掉对应的库角色及其 Campaign 级联数据。

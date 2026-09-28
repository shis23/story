# R1 复检：域1（domain + 基础 infra）与域2（存储 / Turn 生命周期）

- 复检对象：`docs/review-2026-09-13/fixes/01-domain-infra-fixes.md`（域1 D-01..D-26 + 5 条跨域 M 项）、`docs/review-2026-09-13/fixes/02-storage-fixes.md`（域2 S-01..S-22 + 跨域 D-1..D-3）
- 被复检代码状态：HEAD `ab894c6` + 未提交工作树（冻结交付物），`git diff --numstat` 域内 64 文件 +7016/-659
- 复检者：`review-pipeline`（task-18）。**本次未修改任何源码/文档**，只新建本报告
- 约束遵守：未跑 `cargo test --workspace`（Lead 独占）；未跑 npm；未动已冻结的 `round2/R1..R7*.md`

---

## 0 一句话结论

| 域 | 条数 | 确认关闭 | 部分关闭 | 暂缓(如实) | 未关闭 | 虚报(记录称已修但代码未修) |
|---|---|---|---|---|---|---|
| 域1 domain/infra | 26 + 5 = 31 | 31（含 D-21/D-22/D-25 的"混合/暂缓"子项，记录已如实标注） | 0 | 0（子项级暂缓已在条目内写明） | 0 | **0** |
| 域2 存储/Turn | 22 + 3 = 25 | 22 | **1（S-01）** | 2（S-06、S-18） | 0 | **0** |

- **D-01（P0 卡壳 panic）独立复现成立且已修**：自建输入 + 忠实复刻旧算术 → 旧代码在同型输入上确定性 panic，新 guard 推进到 char 边界，公开 API 路径不 panic。
- **S-01（P0 迁移静默丢数据）主路径已堵，但修复过度外扩**：`campaigns.json` 缺失的 fail-closed 判据把"合法 legacy 非 Campaign 老数据"一并拒掉，应用**启动即失败**（新发现 **N-R1-01 [P1]**，已独立复现）；同时"card 存在但引用缺失"这一半仍不 fail-closed（新发现 **N-R1-02 [P2]**）。
- 记录本身**没有发现虚报**：31+25 条里凡写"已修复"的都能在源码/diff 找到对应改动；3 处记录**比现实保守**（D-03 接线、D-06 的 importer 兜底、S-06 承接），是诚实方向而非反向。

---

## 1 方法与验证深度传说

| 记号 | 含义 |
|---|---|
| **【diff】** | 我亲自读了该文件的 `git diff` / 当前源码，逐行确认改动语义 |
| **【测试】** | 我复跑了包含该断言的测试目标（全绿），并核对了断言文本与发现描述一致；未逐行读实现 |
| **【记录】** | 只核对了记录中的 file:line 与测试名存在，未做等价性复核（本报告已逐条标注，不冒充已核实） |

复跑命令与结果（本成员执行，HEAD `ab894c6` + 冻结工作树）：

| 命令 | 结果 |
|---|---|
| `cargo test -p storyforge-domain` | 388 passed / **0 failed** / 0 ignored |
| `cargo test -p storyforge-infra-sqlite` | **316 passed / 0 failed**（22 个测试二进制，逐二进制核对，与记录一致） |
| `cargo test -p storyforge-infra-vector` | 21 passed / 0 failed |
| `cargo test -p storyforge-infra-util` | 12 passed / 0 failed / 1 ignored |
| `cargo test -p storyforge-infra-regex` | 25 + 1 passed / 0 failed |

**独立复现（不依赖作者测试）**：在 `artifacts/review-2026-09-13-round2/probe-r1/` 建了一个一次性 cargo 探针（`[workspace]` 独立、`path` 依赖 `crates/domain` + `crates/infra-sqlite`、`CARGO_TARGET_DIR` 复用工作区 `target/`，`--offline`），构造自建输入后调用公开 API，运行完已删除；关键输出与复现片段见 §4、§5 与附录 A。

---

## 2 域1 逐条判定（D-01..D-26 + M 项）

| ID | 记录状态 | R1 判定 | 证据（我的复核） |
|---|---|---|---|
| D-01 P0 卡壳 panic | 已修复 | **确认关闭** | 【diff】`card_shell.rs:477-480` guard + `475-476` 注释；【独立复现】见 §4；作者回归测试输入确为"真会失败"的用例（不是恒真） |
| D-02 P1 `set_enabled` 双真相源 | 已修复 | **确认关闭** | 【diff】`world_info.rs::set_enabled`：`extra.contains_key("enabled")` 才同步，不发明 V3 键；`route_for_flags` 供 `default_route`/`from_st` 共用 |
| D-03 P1 `write_fence` 无调用者 | 已修复 | **确认关闭** | 【diff】`infra-vector/src/lib.rs::quarantine_unrecoverable_corruption` 真调用 `freeze_with_reason`；【diff】接线实测：`lib.rs:842-845` sweep + `storage_health.rs:104-120` 报告 + `commands/diagnostics.rs:16` acknowledge → `write_fence::unfreeze` + `StorageHealthGate.vue` 已挂载（记录里"需域4/域2 加一行"已被后续补上，属**保守**表述） |
| D-04 P1 StoryTime 静默消失/混渲染 | 已修复（域1 侧） | **确认关闭** | 【diff】`story_task.rs:278-318` 两组渲染 + 空返回 `""`；`render_pending_judgment_group_contains_no_spoiler_instruction` 存在 |
| D-05 P2 向量搜索顺序不确定 | 已修复 | **确认关闭** | 【diff】`infra-vector/src/lib.rs:361-366`（命中数降序→id 升序再 truncate）、`:435-439`（同分 id tie-break） |
| D-06 P2 story_clock 双默认值 | 已修复 | **确认关闭** | 【diff】`variables.rs:107/112`、`campaign.rs:61/69-75/105`；跨域兜底 `importer.rs:565` 已改用 `DEFAULT_STORY_CLOCK`（记录里标注的"跨域未修"已由域2 D-1 关闭） |
| D-07 P2 `h_anchor == 0` 切片 panic | 已修复 | **确认关闭** | 【diff】`chronicle.rs:372-379` 入口早返回空集 |
| D-08 P2 空串覆盖用户字段 | 已修复 | **确认关闭** | 【测试】`basic_stage_json_does_not_wipe_with_empty_strings` + 代码 `trim().is_empty()` 守卫 |
| D-09 P2 未知 before_node_id 退化全量历史 | 已修复 | **确认关闭** | 【diff】`conversation.rs::collect_active_variants` 未知 id → `0` + `tracing::warn!` |
| D-10 P2 `depth_prompt` 正文计入 script_bytes | 已修复（1 处刻意差异） | **确认关闭** | 【diff】`mvu_translation.rs::collect_script_sources` 分流 code/prose；刻意差异理由成立且已写明 |
| D-11 P2 嵌套宏被首个 `}}` 截断 | 已修复 | **确认关闭** | 【diff】`prompt_module.rs::find_macro_body_end` 深度配对 + `render_template_text` 递归 + `MAX_MACRO_NESTING = 8`；未知宏原样保留 |
| D-12 P2 order 乘法 i32 回绕 | 已修复 | **确认关闭** | 【diff】`card_studio.rs` 两处改 `i32::try_from(n).ok()` + 回退默认 |
| D-13 P2 `Single` 单选语义未执行 | 已修复 | **确认关闭** | 【diff】`prompt_module.rs::assemble_system_prompt` `single && injected > 0 → break`；`Multiple` 分支不变 |
| D-14 P2 `migrate_to` 降级版本 | 已修复 | **确认关闭** | 【diff】`agent_profile_config.rs` `target <= config_version → false`；4 条载入路径调用点存在 |
| D-15 P2 CardProject 缺字段整份解析失败 | 已修复 | **确认关闭** | 【测试】`card_project_deserializes_legacy_missing_fields` + `#[serde(default)]` |
| D-16 P2 闸门 severity 降级可被操纵 | 已修复 | **确认关闭** | 【diff】`card_studio.rs::KNOWN_RULE_CODES` + `is_known_rule_code`；未知 code 不再降级 |
| D-17 P3 novel_distill 卡死/溢出 | 已修复（模块无生产调用者暂缓） | **确认关闭** | 【diff】`saturating_mul(2)` + 空块直接落终态；暂缓项在记录内清晰标注 |
| D-18 P3 history.rs 零测试 | 已修复（改名暂缓） | **确认关闭** | 【测试】8 条新测试存在且全绿；`variant_id` 改名属跨 crate 破坏性变更，暂缓理由成立 |
| D-19 P3 tail 指纹分段歧义 + builder 文档漂移 | 已修复 | **确认关闭** | 【diff】`message_layout.rs:143-145` 长度前缀（两处 hasher）；全仓无 `tail_hash` golden 断言（记录的影响提示准确） |
| D-20 P3 注释/文档漂移 | 已修复 | **确认关闭** | 【diff】`agent.rs::is_leaf_a` 与 `chronicle_level` 同源。注：非法 level 下 `is_leaf_a()` 现在为 `true`（旧实现 false），这是"单一口径"的语义选择，行为变化仅在数据已损坏时可见，记录已写明 |
| D-21 P3 死结构/api_key/Custom 形状 | 混合（已修复 + 非问题 + 暂缓） | **确认关闭（记录一致）** | 【diff】`AgentProfile`/`ToolSpec` 导入已删；`api_key` 判定非问题的理由可核（序列化路径被 `connection_store` 依赖，加 skip 反而丢密钥），并补了 Debug 打码测试；`Custom` 形状暂缓写明 |
| D-22 P3 死代码/不可达分支 | 部分已修复 + 暂缓 + 1 非问题 | **确认关闭（记录一致）** | 【diff】`drafts_to_st_book`、不可达 warn、`AgentBinding`、`Preset::enabled_system_prompts`、`WorldInfoEntry::matches_query` 均已删；【diff】`classify_shell_kind` 全路径穷尽、确实不构造 `CardShellKind::Other`，"保留 catch-all"判定成立 |
| D-23 P3 死 Err 分支 / `as i32` 有损截断 | 已修复 | **确认关闭** | 【diff】`narrow_i64_to_i32` + 越界 warn；`Result` 签名保留（调用方兼容） |
| D-24 P3 reverse_parse 丢字段 + 阈值单位 | 已修复（wire 通道暂缓） | **确认关闭** | 【diff】`reverse_parse_*` 三字段恢复；`INLINE_HTML_MIN_CHARS = 80`(chars) / `INLINE_HTML_IPC_MAX_BYTES = 8192`(bytes) + 超限 warn |
| D-25 P3 前后端 shell 关键词表漂移 | 暂缓（跨域）+ 域1 文档已补 | **暂缓（如实）** | 【记录】Rust 侧为超集、前端只镜像 URL 分支且优先级相反——结论成立；域1 无代码可改，前端不属其写作用域 |
| D-26 P3 RegexScript serde 默认值不对称 | 已修复（刻意保留两处无默认） | **确认关闭** | 【diff】`preset.rs` `disabled/flags/placement` 加 default；`find_regex`/`replace_string` 保持 fail-closed 且理由（空正则命中一切）成立 |
| M-05 / M-26 / M-28d / M-32.8 / M-32.9 | 已修复（域6 委派） | **5/5 确认关闭** | 【diff】`world_info.rs::position_for_export`（0/1 → 规格字符串，越界保留数字）；`position_as_i32` 穷尽 match + warn；`variables.rs` normalize + 递归点记法展开；`character.rs` `RAW_CARD_JSON_PARSE_FAILED_KEY` 标记；`to_st_entry` 仅在已有 `insertion_order` 时与 `self.order` 同步 |

**域1 未关闭：0 条。虚报：0 条。**

---

## 3 域2 逐条判定（S-01..S-22 + 跨域 D-1..D-3）

| ID | 记录状态 | R1 判定 | 证据（我的复核） |
|---|---|---|---|
| **S-01 P0** 导入源不完整 → 静默丢数据 | 已修复(降级) | **部分关闭** | 【diff+独立复现】原"缺文件→连锁丢全部行→空库+completed+marker"路径已被 Rule A/B 堵住；但 Rule B 误伤合法老数据（**N-R1-01 [P1]**），且"cards 存在但全悬空"仍产出空权威（**N-R1-02 [P2]**）。降级（不新增 DB 列，改报告+`warn!`+健康事件）本身属实 |
| S-02 P1 stale marker 文本判定 | 已修复 | **确认关闭** | 【diff】`cutover.rs::StaleKind` 11 态枚举取代子串分类；`MarkerUnreadable`/`DbProbeFailed`/`DbVersionAhead` 不再被当作"可恢复中断残留" |
| S-03 P1 孤儿库所有权探测过宽 | 已修复 | **确认关闭** | 【diff】`orphan_db_has_no_user_data` + `orphan_db_content_hash_matches` 双条件，身份与内容绑定 |
| S-04 P1 JSON accept 幂等 Replay 不确认持久化 | 已修复 | **确认关闭** | 【diff】`production.rs:1034-1036` 校验 `campaign.revision == expected == turn.base_campaign_revision` 且 `target == expected+1`；`:1017-1032` 结构化/索引 revision 一致；`:1082` 落 target |
| S-05 P1 JSON 路径缺 target/final_revision 校验 | 已修复 | **确认关闭** | 【diff】同上 + `turn_coordinator.rs::preflight_rejects_batch_whose_target_is_not_expected_plus_one` |
| S-06 P1 迁移期 JSON 拷贝失败被吞（`lib.rs`） | 暂缓(跨域) | **暂缓（如实）** | 【diff】`lib.rs` 本轮**只**新增了 write_fence sweep 5 行；`copy_dir_recursive`（`:525`）仍是 `let _ = std::fs::copy` → 残留见 **N-R1-03 [P2]**；顶层分支（`:508`）的 warn 在 `ab894c6` 就已存在，非本轮修复 |
| S-07 P1 启动恢复标记 Failed 被吞 | 已修复 | **确认关闭** | 【diff】`turn_lifecycle.rs:1079-1081` `record_backend_incident("turn_recovery_failed_mark")` + 活动 Attempt 一并收口 |
| S-08 P2 发布前后 WAL/复检/清理 | 已修复 | **确认关闭** | 【测试】`gate5_fault_matrix`（10）+ `gate5_migration_matrix`（6）全绿，断言名与描述一致 |
| S-09 P2 发布顺序与 marker-last | 已修复（加固） | **确认关闭** | 【diff】`readiness.rs::create_backup_checkpoint_inner` 新增 `content_hash`/`source_manifest_hash`（S-09 对账字段）；调用点仅 cutover/rollback（低频），无性能回归 |
| S-10 P2 `journal_mode` 返回值被丢弃 | 已修复 | **确认关闭** | 【diff】`connection.rs::configure_connection(conn, expect_wal)` 校验 WAL 未生效即 `Err`；`open_readonly` 补 `busy_timeout(5s)` |
| S-11 P2 lease Drop 提前释放 OS 锁 | 已修复 | **确认关闭** | 【diff】`HeldLease { mode, holders, file }`：真实 fd 托管给记账表，最后一个 holder 才移除记录 → 关闭 fd；owner 路径 `take_file()`，重入只递减；乱序析构不变量成立 |
| S-12 P2 rollback 全有或全无被 fsync 打破 | 已修复 | **确认关闭** | 【diff】`rollback.rs:281-400` 先构造 `InstallPlan`（写新内容 + fsync）再原子 rename + 父目录 fsync；`:389` 注释正对原缺陷；`RollbackFault::AfterJsonInstall` 故障点存在 |
| S-13 P2 JSON 级联删除形同虚设 + 吞错 | 已修复 | **确认关闭** | 【测试】`lib_tests_campaigns.rs` 级联用例迁移到 `delete_character_cascade_source_ids`；【记录】调用方错误不再静默 |
| S-14 P2 失败被吞 | 已修复(降级) | **确认关闭** | 【diff】`connection_store.rs::resolve_connection` 全程 `?`/`warn`+`None`，无 ref-as-key 回退；测试 `unresolvable_secret_ref_fails_closed_without_ref_as_key_fallback`（`:386`）钉住 fail-closed |
| S-15 P2 快照复用 stale turn 覆盖 | 已修复 | **确认关闭** | 【diff】`turn_coordinator.rs::has_active_turn` → `turn_store.get_active_turn`；删除活动不检查在途 Turn 的守卫落地 |
| S-16 P2 JSON/SQLite 语义差距 | 已修复 | **确认关闭** | 【diff】`apply_batch` 幂等 Replay 契约 + revision 契约（见 S-04/S-05）；测试全绿 |
| S-17 P2 运行期无 marker 复检 | 已修复 | **确认关闭** | 【diff】`storage_backend.rs:285-311` `validate_runtime_authority` 复用 `check_marker_status`（此前无生产调用方） |
| S-18 P2 关键守卫零覆盖/假测试 | 已修复(降级) | **暂缓（如实）** | 【测试】缺口 1-4 已补；5/6（弱断言/低价 CI 性能测试）暂缓并在 §10 明确列出承接方 |
| S-19 P3 注释/文档漂移 | 已修复（item 6 跨域暂缓） | **确认关闭** | 【记录】item 6（诊断文案）与 S-06 同属域4，暂缓标注清晰 |
| S-20 P3 死代码/误导代码 | 已修复(降级) | **确认关闭** | 【diff】`cutover.rs` 改为共用 `fs_atomic::{fsync_file, fsync_parent_dir}`，rollback 不再各有一份并吞错 |
| S-21 P3 恢复升级 Failed 未收口 Attempt | 已修复 | **确认关闭** | 【diff】`turn_lifecycle.rs` 与 S-07 同一处修改，活动 Attempt 一并收口 |
| S-22 P3 低风险观察（逐点） | 混合 | **确认关闭（记录一致）** | 【diff】S-22.1 取舍说明写进 `connection.rs` 文档；其余逐点状态在记录内可追溯 |
| D-1 跨域 importer `"Day 1"` 兜底 | 已修复 | **确认关闭** | 【diff】`importer.rs:565` 引用 `DEFAULT_STORY_CLOCK`；测试断言 `!= "Day 1"` |
| D-2 跨域 SecretRef 不得回退 ref-as-key | 已复核 fail-closed + 补测试 | **确认关闭** | 【diff】4 处调用点全 fail-closed（`secret_store.rs`/`connection_store.rs:289`/`lib.rs:587`/harness） |
| D-3 跨域 write_fence 并入 storage_health | 已修复 | **确认关闭** | 【diff】`lib.rs:842-845` 一行 sweep；`record_write_fence_state` 去重且按真实路径登记，`acknowledge` 可解冻该路径；2 条测试存在 |

**域2 未关闭：0 条（S-01 记为"部分关闭"）。虚报：0 条。**

---

## 4 P0 独立复现之一：D-01（卡壳 panic）

**复现方式（不依赖作者测试）**：
1. 忠实复刻旧算术：`capture_es_module_urls` 的 needle 是 `"from "`（**含尾空格**），`abs = find(needle) + needle.len()` 指向尾空格之后的 CJK 首字节，旧代码 `search_from = abs + 1` 落在多字节字符内部，下一轮 `&lower[search_from..]` 越界切片。
2. 自建输入经公开 API `extract_card_shell_manifest(&Character)` 走生产路径。

**探针实测输出**：

```
[D-01] text="// 移植 from 原作：设定集" needle="from " abs=15 旧表达式 panic=true 新 guard 推进后边界=true
[D-01] text="// 参考 import 模块实现" needle="import " abs=17 旧表达式 panic=true 新 guard 推进后边界=true
[D-01] text="移植from 原作" needle="from " abs=11 旧表达式 panic=true 新 guard 推进后边界=true
[D-01] extract_card_shell_manifest -> OK (shells=1)
```

**结论**：
- 旧代码在**作者回归测试使用的同型输入**上确定性 panic（说明该测试是"真会失败"的对照，不是恒真测试）；
- 新实现 `card_shell.rs:477-480` 的 `is_char_boundary` 推进使 `search_from` 恒为边界；同一函数族 `capture_jquery_load_urls`（`:478` 附近）也做了同样防护，无第二个同类切片点；
- 公开 API 用自建输入（CJK 注释 + 真实 ESM URL + 多字节紧跟 needle）不再 panic。→ **D-01 确认关闭**。

---

## 5 P0 独立复现之二：S-01（迁移静默丢数据）

探针直接调用 `readiness::validate_source_manifest` 与 `JsonImporter::import_data_dir`，构造三种自建布局：

```
[S-01]  布局: cards.json + conversations/conv-legacy.json（无 campaigns.json）
[S-01]  validate_source_manifest -> ERR: import source incomplete: campaigns.json is missing but
        dependent collections are non-empty (conversations=1); refusing to drop every dependent row
        as an orphan — restore campaigns.json and retry
[S-01]  import_data_dir -> ERR: （同上）

[S-01b] 布局: cards.json=[] + campaigns.json 引用缺失卡 card-1
[S-01b] validate_source_manifest -> OK (campaigns=0, skipped=1, detail=[("campaigns_no_card", 1)])
[S-01b] import_data_dir -> OK (status=Completed, campaigns_in_db=0)
```

判定：
- **原 P0 路径（诚实说：主路径）已关闭**：`cards.json`/`campaigns.json` **缺失**时不再连锁丢弃，而是 `SqliteError::ImportSourceIncomplete` 拒绝导入（`readiness.rs:186-227`）；应用启动侧 `resolve_backend_inner` → `run_sqlite` → `recover_or_verify` 会把该错误上抛为 `BackendWiringError::Cutover`，即 fail-closed 不发布空权威。
- 但同一守卫在两个方向上都有缺口 → **部分关闭**，见 N-R1-01 / N-R1-02。

---

## 6 新发现

### N-R1-01 [P1] S-01 守卫过度外扩：合法 legacy 非 Campaign 老数据被拒，应用启动即失败

- 位置：`crates/infra-sqlite/src/readiness.rs:199-227`（Rule B：`!campaigns_file_present` + 任一"依赖集合"非空 → `Err`）
- 反证（独立复现，见 §5 第一段）：布局 = `cards.json` + `conversations/conv-legacy.json`（`campaign_id: null`）、**无** `campaigns.json` → `validate_source_manifest` 与 `import_data_dir` 双双返回 `import source incomplete: campaigns.json is missing but dependent collections are non-empty (conversations=1)`。
- 为什么这是**合法**老数据而不是损坏：
  1. `Conversation.campaign_id: Option<Id>`（`domain/src/conversation.rs:249`），`Conversation::new(character_id: Option<String>, campaign_id: Option<Id>)`（`app-conversation/src/lib.rs:254`）——非 Campaign 写作路径天然产生"无 campaign 的会话"；`strict_validate_entries("conversations", …)` 只对 `campaign_id` 做 `opt_str_strict`（`readiness.rs:1021`），即**允许**其缺失；
  2. `campaigns.json` 只在有 Campaign 变更时才写（`CampaignStore::new` 用 `load_or_default`，不创建空文件；`campaign_store.rs:110-138`），从未建过 Campaign 的用户目录里不会有该文件；
  3. 应用启动**无 marker 时默认走 SQLite**（`storage_backend.rs:1911-1944`，`run_sqlite` → `recover_or_verify`），因此这类老用户升级后会在启动时就撞上 `ImportSourceIncomplete`，**旧 JSON 权威也无法回退启动**（marker 不存在、cutover 被拒）。
- 影响：非 Campaign 写作路径的老用户升级后**应用不可启动**（fail-closed，无数据丢失但主流程不可用）。这正是 R1 要求重点排查的"fail-closed 拒掉合法老数据"。
- 建议修法（最小改动，保持 Rule A/B 的初衷）：
  - Rule B 只统计**真正 campaign 归属**的行：conversations 用 `campaign_id.is_some()` 过滤后再计数（其余集合本身以 campaign_id 为必填，保持原样）；
  - 或把 Rule B 的集合限定为"schema 上 campaign_id 必填"的集合（instances/knowledge/tasks/round_summaries/turns/compress_jobs/characters/mvu_translations/world_info），显式排除 legacy-legal 的 conversations；
  - 作者测试 `legitimate_empty_and_partial_layouts_are_not_rejected`（`importer_diagnostics.rs:1336-1389`）应补第 ⑤ 组布局：`cards.json` + `conversations/conv-legacy.json`（`campaign_id: null`）+ **无** `campaigns.json`。
- 严重度理由：P1（主流程在特定合法输入下不可用）；不评 P0 是因为无数据丢失、且 JSON 源文件原样保留。

### N-R1-02 [P2] S-01 的另一半未收口：`cards.json` 存在但全部 Campaign 悬空 → 仍可产出空权威（仅审计可追溯）

- 位置：`crates/infra-sqlite/src/readiness.rs:161-176`（`filter_campaigns_without_card` 丢弃悬空 Campaign）+ `:186-227`（守卫只对**文件缺失** fail-closed）
- 反证（独立复现，见 §5 第二段）：`cards.json = []` + `campaigns.json` 有 1 条引用 `card-1` → 报告 `campaigns=0, skipped=1, detail=[("campaigns_no_card",1)]`，`import_data_dir` 返回 `ImportStatus::Completed`、库内 `campaigns=0`；随后 cutover 按既有 marker-last 流程照常发布（`storage_backend.rs:1941-1954` 只对 `skipped_orphan_rows > 0` 加 `warn!` + 健康事件，不阻断）。
- 与作者理由的分歧（供 Lead 裁定）：作者把该形态当"用户确实删掉了唯一那张卡"的合法老数据（测试 `empty_cards_json_with_all_campaigns_dangling_is_skipped_with_audit`，`importer_diagnostics.rs:1159-1188`）；但卡片删除会级联清理其 Campaign（`campaign_store.rs:461-475` 的 `list_campaigns_of_card` 即为此用，CLAUDE.md 亦记 `delete_character` 级联 Campaign/MVU/vector），因此"cards 里没有该卡、campaigns 里却有引用它的 Campaign"**不能由正常 UI 操作产生**，它更可能意味着拷贝/写入中断（如 `cards.json` 被截断成 `[]`）。
- 影响：同一 P0 的"空权威 + completed 导入 + marker 固化"结局仍可发生，只是变成"有审计的静默"（warn + `storage_health` incident，用户不一定会看到）。源 JSON 未删，可用 `storyforge_rollback` 恢复，故非 P0。
- 建议（交 Lead 裁定，二选一）：
  - 收紧：当 `campaigns_total > 0 && campaigns_kept == 0`（父级全丢）时 fail-closed，或要求显式操作者确认后再导入；
  - 或明确接受：在 `GATE-REPORT.md`/`storage_health` 文案里把"全部 Campaign 被跳过"提升为**阻断级**健康事件（而非 warn），保证用户可见。
  - **后续更正（R13 落地回执，2026-09-13；来源：task-36 / review-storage，本报告未独立复跑）**：
    本节 N-R1-01（Rule B 收窄为只统计 `campaign_id.is_some()` 的 conversations + 第 ⑤ 组布局）、
    N-R1-02（改为 fail-closed）、N-R1-04（`Path::exists()` → `path_presence` 三态 + `ImportSourceUnreadable`）均已落地，
    并各自附失败可控验证；作者自测 `cargo test -p storyforge-infra-sqlite` = **320 passed / 0 failed**。
    **测试名变更提醒**：原测试 `empty_cards_json_with_all_campaigns_dangling_is_skipped_with_audit` 已改名为
    `..._is_rejected`（本报告上文引用的旧名不再存在）；另有反向锁 `campaign_scoped_conversations_still_require_campaigns_json`。
    详见 `docs/review-2026-09-13/round2/R13-readiness-legacy-layout-fix.md` 与 task-36 回执。

### N-R1-03 [P2] S-06 承接未落地：嵌套目录拷贝失败仍静默（`lib.rs::copy_dir_recursive`）

- 位置：`crates/tauri-app/src/lib.rs:517-529`，`:525` `let _ = std::fs::copy(entry.path(), &dest);`
- 事实：本轮 `lib.rs` 仅新增 write_fence sweep（`+5` 行），顶层文件分支（`:508-510`）的 `tracing::warn!` 在 `ab894c6` 已存在；**递归分支**依旧吞错。记录把 S-06 记为"暂缓(跨域)"并给出承接建议，属如实；本条只是把"承接后仍未关闭的具体缺口"钉在 R1 里。
- 影响：旧→新数据目录迁移时，若嵌套集合（`conversations/`、`campaign_world_info/`）拷贝失败，新目录会缺文件且无任何日志；随后 S-01 守卫**不会**拦住（`campaigns.json` 存在），导入继续 → 会话/世界书静默丢失（源目录仍在，可人工补拷）。
- 建议：交域4 把 `let _ =` 改为错误传播（或至少 `tracing::error!` + 迁移未完成标记，禁止后续按"迁移完成"启动）。

### N-R1-04 [P3] 守卫用 `Path::exists()` 判定"文件缺失"，权限错误会被误报为源不完整

- 位置：`crates/infra-sqlite/src/readiness.rs:169-170`（`data_dir.join("cards.json").exists()` / `campaigns.json`）
- 事实：`Path::exists()` 在 `EACCES`/符号链接环等情况下返回 `false`（`Err` 被吞）。于是一个**存在但不可读**的 `cards.json` 会被 Rule A 判成"缺失 + campaigns 非空"，报 `ImportSourceIncomplete` 而不是把真实 IO 错误暴露给操作者。
- 影响：低（仍是 fail-closed，不会丢数据），但会误导排障方向；建议改为 `fs::metadata(...)` 并区分 `NotFound` 与其它 `io::Error`。

---

## 7 诚实性核查（记录 vs 代码）

1. **无虚报**：域1 的 31 条、域2 的 25 条中，凡状态为"已修复/已修复(降级)"的，我都能在源码或 diff 里找到对应改动；没有发现"记录写已修、代码未动"的情况。
2. **降级/暂缓的措辞与事实一致**：
   - S-01 "已修复(降级)" —— 降级内容（不新增 DB 列，用报告 + warn + 健康事件替代）与实际实现一致；但该条目**应改判"部分关闭"**（见 N-R1-01/02）；
   - S-06/S-18.6/S-19.6/D-21(Custom)/D-22(暂缓项)/D-25/D-24(wire 通道) 的暂缓均在记录内指名承接方与理由，未伪装成已修；
   - D-10 主动写明"与报告建议有 1 处刻意差异"；D-21 的 `api_key` 判定非问题给了可核的调用链证据（优于"函数没用"这种理由）。
3. **3 处记录比现实保守**（方向正确，不需返工）：
   - D-03 记录称"需域4/域2 加一行 sweep"，实际 `lib.rs:842-845` 已补 → 现已闭环；
   - D-06 记录称"跨域未修 `importer.rs:553`"，实际已由域2 D-1 改为引用 `DEFAULT_STORY_CLOCK`；
   - D-19 影响提示（跨版本 `tail_hash16` 不可比、无 golden 断言）经核对成立。
4. 测试新增/修改未削弱：域内 `crates/infra-sqlite/tests/*` 为**纯新增**（`cutover.rs` +430/-2、`importer_diagnostics.rs` +335/-0、`production_uow.rs` +150/-0）；domain 侧删除行均为重构/去死代码，未见断言放宽；被改名的既有测试（`sqlite_character_lifecycle.rs` 的 `#[tokio::test]` 迁移等）语义等价。

---

## 8 回归搜索结论

| 检查项 | 结论 |
|---|---|
| S-01 fail-closed 是否拒合法老数据 | **是，已确认**（N-R1-01 P1）；作者自测的 4 组布局未覆盖"无 campaigns.json + 非 Campaign 会话" |
| S-02/S-03 fail-closed 是否拒合法状态 | 未发现拒合法状态：`MarkerUnreadable`/`DbProbeFailed` 属"打不开就不放行"，瞬时占用可重试；`S-03` 的内容 hash 条件只在"marker 丢失 + 孤儿库"时生效。**观察**：`DbProbeFailed` 文案只有 `database probe failed: <io err>`，没有恢复指引（其它 StaleKind 都带说明），建议补一句操作建议（P3 观察，不计入新发现） |
| 双重校验冲突 | 未发现：`readiness` 守卫 / `cutover` marker-last / `storage_backend` 跳过计数审计三层判据方向一致（都是"缺文件拒、行级缺父跳过"），无互相矛盾的裁决 |
| 测试削弱 | 未发现（见 §7.4） |
| 语义变化 | 有 3 处**有意的行为变化**且都在记录中写明：D-20 非法 level 下 `is_leaf_a` 变 `true`；D-19 指纹值变化（无 golden）；D-26 `find_regex` 仍 fail-closed。均可接受 |
| 性能 | 未发现回归：守卫只新增 2 次 `exists()`；`recompute_db_content_hash` 只加在 `create_backup_checkpoint`（仅 cutover/rollback 调用，非热路径） |

---

## 9 残余不确定性与建议

1. **需 Lead 裁定**：N-R1-01 的修法（Rule B 收窄到"真 campaign 归属行"）与 N-R1-02 的策略（父级全丢是否 fail-closed / 是否升级为阻断级健康事件）。两条都属 S-01 收口，建议一并处置后再把 S-01 记为"确认关闭"。
2. **域4 承接**：N-R1-03（嵌套拷贝吞错）、S-06/S-19.6 的诊断文案。
3. 我未复跑的测试目标：`-p storyforge`（tauri-app，470 条）与 `--workspace` 全量由 Lead 独占；本报告对 S-07..S-22 中标注【测试】的条目，依据是"测试名存在 + 我复跑的 `storyforge-infra-sqlite` 316 条全绿"，未逐行读实现。
4. 独立复现探针为一次性工具，已从 `artifacts/` 删除；复现片段与命令见附录 A，Lead 可随时重建。

---

## 附录 A 复现片段（探针已删除，可据此重建）

```bash
# 重建：artifacts/review-2026-09-13-round2/probe-r1/{Cargo.toml,src/main.rs}
# Cargo.toml: [workspace] 独立 + path 依赖 ../../../crates/infra-sqlite、../../../crates/domain
#             + serde_json、tempfile
$env:CARGO_TARGET_DIR = '<repo>/target'
cargo run --offline --manifest-path artifacts/review-2026-09-13-round2/probe-r1/Cargo.toml
```

```rust
// N-R1-01：合法 legacy 布局
write_json(&d.join("cards.json"), json!([{"id":"card-1","name":"Hero"}]));
write_json(&d.join("conversations/conv-legacy.json"),
    json!({"id":"conv-legacy","campaign_id":null,"character_id":"card-1",
           "created_at":"2026-08-01T00:00:00Z","updated_at":"2026-08-01T00:00:00Z","nodes":[]}));
// 不写 campaigns.json
storyforge_infra_sqlite::readiness::validate_source_manifest(d); // -> Err(ImportSourceIncomplete)

// N-R1-02：cards 存在但全悬空
write_json(&d.join("cards.json"), json!([]));
write_json(&d.join("campaigns.json"),
    json!([{"id":"camp-1","card_id":"card-1","name":"Main","created_at":"t","lineage_id":"lin-1"}]));
// -> Ok(skipped=1, campaigns=0)，import_data_dir -> Completed，库内 campaigns=0

// D-01：忠实复刻旧算术（needle 含尾空格）
let abs = text.find("from ").unwrap() + "from ".len();
std::panic::catch_unwind(|| { let _ = &text[abs + 1..]; }).is_err() // -> true
```

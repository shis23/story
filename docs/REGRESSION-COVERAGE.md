# 回归测试覆盖索引

> 用途：证明核心身份/状态闭环有测试覆盖。新加测试要在本表登记。
> 日期：2026-07-08
> 对应：`docs/PLAN-POST-MAINLINE.md` 阶段 2（回归测试与评测基线）
> 对应：`docs/WORK-NEXT-2026-07-08.md` T1

## 列说明

- **CLAUDE.md 事实**：对应 `CLAUDE.md` "Current Code Facts" 中的哪一条
- **测试文件（:行）**：`git grep` 可定位，行号为文件当前位置
- **测试函数名**：输出中可看到断言细节
- **覆盖范围**：该测试验证了什么

## 回归点清单

### 1. Identity / 身份映射

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| `resolved_persona(definition)` / `resolved_behavior(definition)` | `crates/domain/src/campaign.rs:389` | `resolved_persona_override_takes_priority` | persona override 优先于 definition |
| 同上 | `crates/domain/src/campaign.rs:401` | `resolved_persona_falls_back_to_definition` | persona 无 override 时 fallback 到 definition |
| 同上 | `crates/domain/src/campaign.rs:414` | `resolved_persona_none_when_nothing_available` | persona 无 override 无 definition 时返回 None |
| 同上 | `crates/domain/src/campaign.rs:424` | `resolved_behavior_override_takes_priority` | behavior override 优先于 definition |
| 同上 | `crates/domain/src/campaign.rs:436` | `resolved_behavior_falls_back_to_definition` | behavior 无 override 时 fallback 到 definition |
| 同上 | `crates/domain/src/campaign.rs:448` | `resolved_behavior_none_when_nothing_available` | behavior 无 override 无 definition 时返回 None |
| 同上 | `crates/domain/src/campaign.rs:458` | `resolved_persona_empty_override_does_not_fallback` | persona 空字符串 override 也不 fallback（空胜无） |
| `CharacterInstance::id` 作为内部身份标识 | `crates/domain/src/campaign_runtime.rs:226` | `id_takes_priority_over_name` | id 匹配优先于 name 匹配 |
| `CharacterInstance::id` 作为内部身份标识 | `crates/domain/src/campaign_runtime.rs:212` | `find_by_id` | 按 id 查找 instance 成功 |
| `CharacterInstance::id` 作为内部身份标识 | `crates/domain/src/campaign_runtime.rs:219` | `find_by_name` | 按 name 查找 instance 成功 |
| `CampaignRuntimeContext::definition_for_instance` | `crates/domain/src/campaign_runtime.rs:257` | `definition_for_instance_returns_definition` | instance 有 definition_id 时返回 definition |
| 同上 | `crates/domain/src/campaign_runtime.rs:266` | `definition_for_instance_none_when_no_definition_id` | instance 无 definition_id 时返回 None |
| 同上 | `crates/domain/src/campaign_runtime.rs:274` | `definition_for_instance_none_when_id_not_in_map` | definition_id 不在 map 中时返回 None |
| `CharacterInstance::temporary_with_overrides` | `crates/domain/src/campaign.rs:485` | `temporary_with_overrides_sets_persona` | 临时实例 persona override 正确设置 |
| 同上 | `crates/domain/src/campaign.rs:503` | `temporary_with_overrides_sets_behavior` | 临时实例 behavior override 正确设置 |
| 同上 | `crates/domain/src/campaign.rs:517` | `temporary_with_overrides_sets_both` | 临时实例同时设置 persona + behavior |
| 同上 | `crates/domain/src/campaign.rs:529` | `temporary_with_overrides_none_equivalent_to_temporary` | 全 None override 等效于 `temporary()` |
| `CharacterInstance` 测试 | `crates/domain/src/campaign.rs:315` | `test_character_instance_from_definition` | instance 从 definition 正确继承字段 |
| 同上 | `crates/domain/src/campaign.rs:337` | `test_character_instance_temporary` | 临时 instance 创建正确 |
| 同上 | `crates/domain/src/campaign.rs:346` | `test_character_instance_promote_and_override` | promote 临时 → 持久化 + override |
| 同上 | `crates/domain/src/campaign.rs:360` | `test_character_instance_set_variable` | instance 变量写入正确 |
| `resolved_methods_from_definition_instance` | `crates/domain/src/campaign.rs:471` | `resolved_methods_from_definition_instance` | instance 的 definition 级方法正确返回 |

### 2. CampaignRuntimeContext 装配

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| `CampaignRuntimeContext` 纯 domain 快照 | `crates/domain/src/campaign_runtime.rs:75+` | `resolved_persona_for_uses_definition_fallback` 等 | persona/behavior 解析（instance→definition fallback） |
| `CampaignRuntimeContext::with_temporaries_for` | `crates/domain/src/campaign_runtime.rs:449` | `with_temporaries_dedups_duplicate_unmatched_specs` | 同批次同 spec 只生成一个临时实例 |
| 同上 | `crates/domain/src/campaign_runtime.rs:509` | `temporary_instance_gets_default_variables` | 临时实例有默认变量 |
| 同上 | `crates/domain/src/campaign_runtime.rs:524` | `with_temporaries_passes_persona_override` | with_temporaries 传递 persona override |
| 同上 | `crates/domain/src/campaign_runtime.rs:543` | `with_temporaries_passes_behavior_override` | with_temporaries 传递 behavior override |
| 同上 | `crates/domain/src/campaign_runtime.rs:557` | `with_temporaries_passes_both_overrides` | with_temporaries 传递 persona+behavior override |

### 3. Knowledge / 知识隔离

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| 知识按 instance 过滤注入 | `crates/domain/src/campaign_runtime.rs:356` | `knowledge_for_instance_filters_by_id` | `knowledge_for_instance` 只返回目标 instance 的知识 |
| 同上 | `crates/domain/src/campaign_runtime.rs:408` | `knowledge_for_instance_empty_when_no_entries` | 无知识时返回空列表 |
| volatile tail 知识隔离 | `crates/harness-real-llm/tests/isolation_deterministic.rs:162` | `volatile_tail_knowledge_isolation_between_instances` | A instance volatile tail 不包含 B 的知识 |
| 跨 instance get_character 拒绝 | `crates/harness-real-llm/tests/isolation_deterministic.rs:209` | `get_character_cross_instance_denied` | 子 agent 调用 get_character 不能读其他 instance |
| 临时实例隔离 | `crates/harness-real-llm/tests/isolation_deterministic.rs:272` | `temporary_instance_isolation` | 临时实例的知识/变量不与其他实例串写 |

### 4. Postprocess 写回

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| `persist_postprocess_outcome` 全路径 | `crates/tauri-app/src/lib.rs:11513` | `test_postprocess_persistence_helper_writes_all_campaign_outputs` | postprocess 输出同时落 knowledge + variables + tasks + summaries |
| 同上（临时实例） | `crates/tauri-app/src/lib.rs:12697` | `test_postprocess_writes_knowledge_for_persisted_temporary` | 临时实例的知识在 persist 后正确写回 |

#### B3 系列：写回在场约束（harness 确定性层）

| 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|
| `crates/harness-real-llm/tests/writeback_isolation.rs:28` | `b3_empty_present_chars_escape_hatch_current_behavior` | 空 present_chars 时当前行为（不拒绝但标注） |
| `crates/harness-real-llm/tests/writeback_isolation.rs:43` | `b3_empty_witnessed_is_rejected` | 空在场时 Witnessed 知识被拒绝 |
| `crates/harness-real-llm/tests/writeback_isolation.rs:79` | `b3_empty_told_by_other_passes` | ToldByOther 跨在场约束放行（来源告知） |
| `crates/harness-real-llm/tests/writeback_isolation.rs:119` | `b3_told_by_other_bypasses_presence` | ToldByOther 不受在场约束 |
| `crates/harness-real-llm/tests/writeback_isolation.rs:156` | `b3_backstory_bypasses_presence` | Backstory 不受在场约束 |
| `crates/harness-real-llm/tests/writeback_isolation.rs:192` | `b3_witnessed_respects_presence` | Witnessed 受在场约束 |

#### B4 系列：同名碰撞

| 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|
| `crates/harness-real-llm/tests/writeback_isolation.rs:230` | `b4_present_chars_name_id_matching` | name 匹配后 id 一致性 |
| `crates/harness-real-llm/tests/writeback_isolation.rs:291` | `b4_name_collision_only_id_path_works` | 同名时只走 id 路径 |
| `crates/harness-real-llm/tests/writeback_isolation.rs:340` | `b4_name_collision_id_path_still_works` | 同名时 id 路径仍正确 |

#### B6 系列：Private 阻断

| 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|
| `crates/harness-real-llm/tests/writeback_isolation.rs:389` | `b6_private_source_blocks_name_collision_relay_and_group_broadcast` | private 来源阻断同名传话和身份组广播 |

#### B8 系列：tool_whitelist

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| `ToolRegistry::retain` / `filter_registry_by_whitelist` | `crates/harness-real-llm/tests/writeback_isolation.rs:557` | `b8_subagent_whitelist_cannot_add_unregistered_tool` | 子 agent whitelist 后不能调用未注册工具 |

### 5. AgentProfileConfig

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| `AgentProfileConfig::validate()` | `crates/domain/src/agent_profile_config.rs:505` | `validate_default_config_ok` | 默认 config 通过验证 |
| `ProfileConfigError::EmptyName` | `crates/domain/src/agent_profile_config.rs:511` | `validate_empty_name_err` | 空名称返回 `EmptyName` |
| 同上 | `crates/domain/src/agent_profile_config.rs:518` | `validate_whitespace_name_err` | 全空格名称返回 `EmptyName` |
| `ProfileConfigError::MaxToolRoundsOutOfRange` | `crates/domain/src/agent_profile_config.rs:525` | `validate_max_tool_rounds_zero_err` | max_tool_rounds=0 返回 OutOfRange |
| 同上 | `crates/domain/src/agent_profile_config.rs:545` | `validate_max_tool_rounds_101_err` | max_tool_rounds=101 返回 OutOfRange |
| 同上 | `crates/domain/src/agent_profile_config.rs:565` | `validate_max_tool_rounds_1_ok` | max_tool_rounds=1 通过 |
| 同上 | `crates/domain/src/agent_profile_config.rs:579` | `validate_max_tool_rounds_100_ok` | max_tool_rounds=100 通过 |
| `ProfileConfigError::InvalidMaxConcurrent` | `crates/domain/src/agent_profile_config.rs:593` | `validate_max_concurrent_zero_err` | max_concurrent=0 返回 InvalidMaxConcurrent |
| 同上 | `crates/domain/src/agent_profile_config.rs:604` | `validate_max_concurrent_one_ok` | max_concurrent=1 通过 |

### 6. MVU Apply

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| `compute_apply_preview` | `crates/app-meta/src/mvu_apply.rs:134` | `preview_mvu_all_new_fields` | 全部为新字段的 preview |
| 同上 | `crates/app-meta/src/mvu_apply.rs:151` | `preview_mvu_overwrite_existing_field` | 已存在字段被覆盖的 preview |
| 同上 | `crates/app-meta/src/mvu_apply.rs:165` | `preview_no_changes` | 无变化的 preview |
| 同上 | `crates/app-meta/src/mvu_apply.rs:178` | `preview_mixed_added_overwritten_unchanged` | 混合变化的 preview |
| 同上 | `crates/app-meta/src/mvu_apply.rs:204` | `preview_empty_mvu` | MVU 为空时的 preview |
| 同上 | `crates/app-meta/src/mvu_apply.rs:217` | `preview_empty_current` | 当前 schema 为空时的 preview |
| `apply_schema_to_definition` | `crates/app-meta/src/mvu_apply.rs:232` | `apply_replaces_variable_schema` | apply 后 variable schema 被替换 |
| 同上 | `crates/app-meta/src/mvu_apply.rs:255` | `apply_preserves_other_fields` | apply 不影响 definition 的其他字段 |
| 同上 | `crates/app-meta/src/mvu_apply.rs:283` | `preview_string_type_field_change` | string 类型字段变化 |

### 7. ToolCenter

| CLAUDE.md 事实 | 测试文件（:行） | 测试函数名 | 覆盖范围 |
|---|---|---|---|
| `role_matches` 通配规则 | `crates/app-agent/src/tool_center.rs:154` | `role_matches_subagent_wildcard_matches_any_subagent` | Subagent:* 通配匹配任何具体子角色 |
| 同上 | `crates/app-agent/src/tool_center.rs:162` | `role_matches_director_does_not_match_subagent` | Director 不匹配 Subagent |
| 同上 | `crates/app-agent/src/tool_center.rs:169` | `role_matches_exact_equality_for_unit_variants` | 单值变体精确相等 |
| ToolCenter 默认工具 | `crates/app-agent/src/tool_center.rs:177` | `director_default_tools_include_five` | Director 默认 5 工具 |
| 同上 | `crates/app-agent/src/tool_center.rs:207` | `subagent_with_concrete_id_gets_wildcard_tools` | 具体子 Agent ID 继承通配工具 |
| 同上 | `crates/app-agent/src/tool_center.rs:225` | `subagent_wildcard_gets_wildcard_tools` | Subagent:* 获得通配工具 |
| 同上 | `crates/app-agent/src/tool_center.rs:233` | `editor_default_tools_include_compose` | Editor 默认含 compose |
| 同上 | `crates/app-agent/src/tool_center.rs:250` | `postprocessor_default_tools_include_emit_postprocess` | PostProcessor 默认含 emit_postprocess |
| 同上 | `crates/app-agent/src/tool_center.rs:267` | `common_tools_appear_in_director_and_subagent_lists` | 通用工具同时出现在 Director 和 Subagent 列表 |
| 同上 | `crates/app-agent/src/tool_center.rs:286` | `all_summaries_returns_all_tools` | all_summaries 返回所有已注册工具 |

## 已知缺口

以下回归点**未找到**测试覆盖（按 PLAN-POST-MAINLINE 阶段 2 回归重点对照）：

| 回归点 | 优先度 | 说明 |
|---|---|---|
| `retain()` 单元测试 (ToolRegistry) | 中 | `filter_registry_by_whitelist` 被 B8 harness 覆盖，但 `ToolRegistry::retain` 本身无单元测试 |
| 知识隔离：同名 instance 时 name 匹配失效逼 id（postprocess 层） | 高 | B4 覆盖了 writeback_isolation 层，但 `persist_postprocess_outcome_to_store` 层无同名 name→id 解析失败时的存储层测试 |
| postprocess 写回：不认识的角色名被跳过 | 中 | 见 `docs/WORK-NEXT-2026-07-08.md` T4 |
| postprocess 写回：campaign_id 不一致的 task 被拒绝 | 中 | 同上 |
| postprocess 写回：空 present_chars 不写 private knowledge | 中 | 同上 |
| 前端 Pipeline trace 映射 | 高 | `SubagentSnapshot` 的 `display_name`/`character_instance_id`/`fallback_reason` 展示逻辑无纯模型测试，见 T2 |
| 前端 Campaign 各 tab 刷新 | 高 | MetaPanel accept → 变量/knowledge tab 无测试覆盖，见 T3 |
| 前端 MetaPanel accept → variable tab 刷新 | 高 | 同上 |
| `AgentProfileConfigStore::save()` 验证前置 | 低 | save 前调用 `config.validate()`——有 store 集成测试否？ |

前列缺口由 `docs/WORK-NEXT-2026-07-08.md` 的 T2/T3/T4 分别补。本节在补完后更新。

## 添加新测试时的登记步骤

1. 找到对应的分类（Identity / CampaignRuntimeContext / Knowledge / Postprocess / AgentProfile / MVU Apply / ToolCenter）。
2. 在表格末尾插入一行，列写全。
3. 如果当前分类不存在，新增分类（保持 markdown 表格式一致）。
4. 缺口罩缺者，在"已知缺口"节填行，补完后移到主表。

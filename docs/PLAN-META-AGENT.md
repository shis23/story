# 计划：Meta Agent 维护层

> 状态：阶段 1 已起步（后端检查 + Tauri command，前端未接）
> 前置：优先完成 `PLAN-CAMPAIGN-MAINLINE.md` 至少阶段 5。

## 目标

把 Meta Agent 从“配置聊天助手”升级为 Campaign 的诊断、解释和修复入口。

完成后用户能问：

- 为什么这一轮这样写？
- 哪些角色变量、知识、任务状态可能不一致？
- 这个修复会改哪些数据？
- MVU 状态栏能否接入当前 Campaign？

## 非目标

- 不让 Meta Agent 自动越权改数据。
- 不把 Meta 做成第二个写作入口。
- 不做通用数据库迁移器。
- 不在 Campaign 主线未稳定前接管写作流程。

## 当前事实

- `crates/app-meta/src/meta_conversation.rs` 已有 `MetaSession`、`PatchStore`、Meta runtime tools。
- `crates/tauri-app/src/lib.rs::sync_meta_session_from_tool_ctx` 主要把当前扁平 character/world_info 同步进 Meta。
- `crates/tauri-app/src/lib.rs::meta_accept_patch` 已有 patch 接受命令。
- `crates/app-meta/src/mvu_import.rs` 已有 MVU 五合一分析和 fallback。
- `frontend/src/components/MetaPanel.vue` 当前展示 Meta 聊天、pending patches、MVU translations。

## 阶段 1：只读 Campaign Health Check

目标：先让 Meta 能读取 Campaign 健康状态，不做修复。

**状态：后端已起步（2026-06-17），前端未接。**

改动文件：

- `crates/app-meta/src/health_check.rs`：新增 health check 模块（确定性检查，零 LLM）
- `crates/app-meta/src/lib.rs`：注册模块 + 重导出
- `crates/tauri-app/src/lib.rs`：新增 `meta_health_check` Tauri command

已实现的检查项：

- ✅ **孤立 instance**：`instance.definition_id` 指向不存在的 definition（Error）
- ✅ **未解析的知识引用**：`knowledge.character_id` 找不到对应 instance（Warning）
- ✅ **孤儿任务引用**：`task.related_characters` 引用了不存在的 instance（Warning）
- ✅ **变量 schema 不一致**：非临时 instance 的变量键集与 definition.variable_schema 不匹配（Warning）

待实现（后续）：

- active campaign 是否存在（command 层已做基本检查，但非 health issue 输出）
- 是否存在同名 instance
- summary turn 是否连续
- MVU translation 是否存在但未合并 schema
- 前端 MetaPanel 展示 ✅（已实现：侧栏"Campaign 体检"区块，运行体检按钮 + issue 列表 + severity 分组）

关键数据结构（确认自 `campaign_store.rs` + domain types）：

- `CharacterInstance.definition_id: Option<Id>` → 指向 `CharacterDefinition.id`
- `CharacterKnowledgeEntry.character_id: Id` → 指向 `CharacterInstance.id`
- `StoryTask.related_characters: Vec<Id>` → 指向 `CharacterInstance.id`
- `CharacterCard.character_definitions: Vec<CharacterDefinition>` → 通过 `Campaign.card_id` 关联
- `CharacterInstance.is_temporary` → 跳过 schema 一致性检查

验证：

```bash
cargo test -p storyforge-app-meta   # 40 tests, 0 failed
cargo test -p storyforge --lib      # 43 tests, 0 failed
```

验收：

- `meta_health_check(campaign_id)` Tauri command 可调用，返回 `Vec<HealthIssue>`。
- 该阶段不产生任何写入。

## 阶段 2：解释本轮生成

目标：用真实 provenance 解释一轮输出。

改动文件：

- `crates/domain/src/agent.rs`
- `crates/app-conversation`
- `crates/app-meta/src`
- `crates/tauri-app/src/lib.rs`
- `frontend/src/components/MetaPanel.vue` 或 `PipelinePanel.vue`

任务：

1. 确认 Provenance 里有：
   - user intent
   - Director plan
   - subagent identity
   - subagent output 摘要或引用
   - editor output 引用
   - postprocess result 摘要
2. 增加 `meta_explain_generation(conversation_id, node_id)`。
3. Meta 回答时必须引用真实字段，不能凭空解释。
4. 前端从消息或 Pipeline trace 打开解释。

验证：

```bash
cargo test -p storyforge-app-conversation
cargo test -p storyforge-app-meta
cargo test -p storyforge
```

验收：

- 用户问“为什么这么写”时，回答包含真实 plan/subagent/provenance 引用。
- 没有 provenance 时返回明确缺失原因。

## 阶段 3：Typed Patch Preview

目标：让 Meta patch 从自由描述变成类型化、可预览、可撤销的修复请求。

改动文件：

- `crates/domain/src`：patch action DTO
- `crates/app-meta/src/lib.rs`
- `crates/app-meta/src/meta_conversation.rs`
- `crates/tauri-app/src/lib.rs`
- `frontend/src/components/MetaPanel.vue`

Patch 类型：

- `UpdateCampaignVariable`
- `UpdateInstanceVariable`
- `AddKnowledge`
- `MergeKnowledge`
- `UpdateTaskStatus`
- `PatchCharacterDefinition`
- `MergeDuplicateInstance`
- `ApplyMvuVariableSchema`

任务：

1. 每个 patch action 必须包含 target id。
2. `preview` 返回变更前后 diff。
3. `accept` 再写 store。
4. `dismiss` 不写 store。
5. patch 过期条件：target 不存在或版本/updated_at 不一致。

验证：

```bash
cargo test -p storyforge-app-meta
cargo test -p storyforge
```

验收：

- MetaPanel 可展开 patch diff。
- 接受 patch 后 health check 问题减少。
- target 不存在时不能写错对象。

## 阶段 4：Campaign-aware Meta Tools

目标：Meta 工具读取 CampaignRuntimeContext，而不是只读扁平角色卡。

改动文件：

- `crates/app-meta/src/meta_conversation.rs`
- `crates/app-meta/src/prompts/meta_agent.rs`
- `crates/tauri-app/src/lib.rs`

工具建议：

- `inspect_campaign`
- `inspect_instance`
- `inspect_variables`
- `inspect_knowledge`
- `inspect_tasks`
- `inspect_generation`
- `propose_campaign_patch`

验收：

- Meta 对 active Campaign 的回答不再只基于 `tool_ctx.characters`。
- 无 active Campaign 时明确提示只能做导入卡/世界书层面的诊断。

## 阶段 5：MVU 接入 Meta 修复流

目标：把已有 MVU 分析结果转成可审阅的 Campaign schema patch。

前置：

- `PLAN-PLUGIN-MVU.md` 至少完成阶段 2。

任务：

1. `meta_analyze_mvu_card` 产物继续保存 `StoredMvuTranslation`。
2. 增加 `propose_apply_mvu_schema(source_character_id, campaign_id)`。
3. 生成 `ApplyMvuVariableSchema` patch。
4. 用户接受后，把字段合并进对应 CharacterDefinition/Instance 的变量 schema。

验收：

- MVU 字段不会直接无审阅写入 Campaign。
- 同名字段冲突有 preview。

## 禁止改动

- 禁止让 Meta 直接改写成文。
- 禁止 Meta 自动接受 patch。
- 禁止 Meta 工具绕过 Campaign ID 校验。
- 禁止为了 Meta 方便把 store lock 暴露给 app-meta。


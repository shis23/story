# StoryForge 架构说明

> 更新日期：2026-07-15
> 本文描述当前 `main` 的代码边界。历史架构快照位于 `docs/archive/`。

## 架构原则

1. Campaign 是长期故事状态的主线真相源。
2. 正文、Attempt、Campaign revision 和 MutationBatch 必须通过 Turn 生命周期一致提交。
3. app/domain 层不依赖 Tauri；Tauri 是组合根和本地存储适配层。
4. LLM 可以使用名字交流，落盘和授权必须使用稳定 ID。
5. JSON 是默认后端；SQLite 只能通过显式 opt-in、fail-closed cutover 和可逆导出启用。
6. Harness 应复用生产应用服务，不长期维护第二套状态机或“近似生产”实现。
7. 所有真实模型、GUI、设备和发布声明必须与证据等级绑定。

## Workspace 边界

```text
frontend
  -> Tauri commands / events

crates/tauri-app
  -> composition root
  -> command DTO / local app services
  -> JSON stores / SQLite runtime selector

crates/app-pipeline
  -> Director / Subagent / Editor orchestration

crates/app-agent
  -> runtime / prompts / tools / quality / postprocess parsing

crates/app-conversation
  -> conversation tree / variants / provenance

crates/app-memory
  -> archived summaries / recall

crates/app-meta
  -> diagnostics / explanations / patches / MVU analysis

crates/domain
  -> Campaign / Turn / Chronicle / NarrativeContract / LLM DTO

crates/infra-*
  -> LLM / SQLite / import / plugin host / vector / regex / util

crates/harness-real-llm
  -> deterministic gates / real-model probes / M5 evidence
```

当前 workspace 共 16 个 crate。`domain` 不依赖内部应用层；`app-*` 不依赖 `tauri-app`；Tauri 负责把 store 组装为应用层使用的快照和服务。

## 写作与 Turn 生命周期

```text
User intent
  -> Tauri adapter
  -> PipelineOrchestrator
     -> Director emits Plan + ScenePlan
     -> Subagents run with per-instance ToolContext
     -> Editor composes draft
     -> Conversation variant + provenance
  -> QualityGate
     -> optional 1x Editor-only auto-fix
  -> TurnRecord / TurnAttempt draft_hash sync
  -> background Summarizer + PostProcessor
     -> candidate summary / knowledge / variables / tasks
     -> Attempt guarded writeback
  -> user Accept / force Accept
  -> TurnLifecycleService
     -> scope checks
     -> revision CAS
     -> finalize variant
     -> apply MutationBatch
     -> Committed or Degraded
```

核心不变量：

- 只有活动 Attempt 可以写回。
- 旧 Attempt 的迟到结果不能覆盖新草稿。
- `draft_hash` 必须对应最终返回给用户的正文，包括 auto-fix 稿。
- force accept 的目标终态是 `Degraded`，恢复时不得变回 `Committed`。
- Campaign / conversation scope 不匹配时必须 fail closed。
- 接受过程的存储失败必须传播，不允许响应成功而磁盘状态滞后。

## Postprocess 边界

成文后的“后处理阶段”包含两个职责不同的 Agent：

- Summarizer：生成本轮 Chronicle A / RoundSummary。
- PostProcessor：生成知识、变量和任务候选更新。

当前 Tauri 写作命令负责后台编排、Attempt 同步和持久化适配。M5 harness 已复用生产写作 Pipeline，并通过 probe 调用共享 JSON `TurnLifecycleService`；它不执行 Tauri command 或 SQLite Accept 路径。Harness 仍使用明确标记的 synthetic Chronicle fixture，因此完整 Summarizer/PostProcessor/Attempt 后台写回尚未成为可由 Tauri 与 harness 共同调用的共享应用服务。

下一架构切片应抽出 `ProductionPostprocessService`，统一：

- Summarizer / PostProcessor 调用与取消。
- MutationBatch normalize 与 ID/scope 校验。
- quality report、draft hash 和 Attempt 状态同步。
- Chronicle A 发布、向量索引和压缩调度。
- 迟到结果、重试、幂等恢复与失败传播。

Tauri command 只做 DTO、事件和后台任务适配；harness 直接调用共享服务，不启动 GUI。

## Memory / Context / Chronicle

- `H_anchor=5`、`E=10` 是当前生产默认，不得称为已标定参数。
- ContextEpoch 固定同一 epoch 的 anchor、overview、band 和 revision 视图。
- Director 可使用 `search_chronicle` / `get_chronicle` 查询 Chronicle A/B/C。
- ChronicleCompressor job/publication 基础支持 A→B→C、连续非重叠 covers、`covered_by`、revision 与幂等 replay。
- MemoryArchiver 处理对话消息归档，与 Chronicle A/B/C 是不同水位和用途。

权威规格：`docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`。

## LLM Request Policy

- active connection 的 temperature、top_p、reasoning、extra 和显式输出上限会注入 Pipeline/AgentRuntime。
- 默认 `max_tokens=None`，OpenAI-compatible 请求体省略该字段。
- 历史未标记的 `Some(4096)` 视为旧 UI 默认并归一为 `None`。
- 用户显式填写正整数时透传；模型支持的最大输出只是 ceiling 能力，不代表每轮应生成该长度。
- 连接 ping、JSON fallback、MemoryArchiver 和评估预算可以有各自的专用限制。

重试策略和 provider capability 探测仍未形成完整统一的 `RequestPolicy` 服务；当前主要解决了采样参数意图和主写作透传问题。

## 存储后端

### JSON（默认）

JSON stores 仍是未 opt-in 用户的默认权威数据源。Turn journal、revision、MutationBatch 和恢复逻辑提供专用逻辑原子性，但不是数据库事务。

### SQLite（显式 opt-in）

SQLite 后端当前具备：

- `StorageBackend::{Json, Sqlite}` 与进程 pin。
- cutover 锁、临时库、备份、内容 hash、marker-last 发布和启动恢复。
- Accept UoW、recovery、active-turn barrier。
- Chronicle publication UoW 和故障注入回滚。
- SQLite→JSON staging + atomic publish reverse export。

当前限制：部分 pre-accept draft、Attempt 中间态和 postprocess 写命令仍需完成全路径迁移。默认后端不得在该工作完成前切换为 SQLite。

## 插件与导入边界

- 插件 runtime 支持显式权限、运行时撤销、prompt-hook timeout/cancel/budget、审计查询/分页/retention 和显式 degraded/unsupported 兼容矩阵。
- FNV-1a audit chain 仅是本地完整性链，不是密码学签名或可信头证明。
- ST/世界书/Campaign Bundle 导入执行 fail-closed 引用校验与补偿回滚；真实复杂卡仍需在合法 fixture 环境补证据。
- 插件 iframe、真实第三方扩展和完整 ST 长尾语义仍需 GUI 验收。

## 发布边界

- 本地 workspace、前端和 host-side release runner 已有自动化证据。
- Gitea workflow 已提交，但远端 runner 执行尚未验证。
- Windows bundle、Android APK、签名、GUI 和真机证据必须在 `docs/RELEASE-CHECKLIST.md` 单独记录。
- M5 当前为已记录的 45/100 Partial Evidence（原始外部 JSONL 已清理），且 `production_postprocess_complete=false`。

## 当前主要技术债

1. 共享 ProductionPostprocessService。
2. SQLite pre-accept 全生命周期迁移。
3. 完整生产路径 M5 100-Accept。
4. Gitea runner 与可离线验证的真实产物证据。
5. GUI、Android 真机和第三方插件现场矩阵。

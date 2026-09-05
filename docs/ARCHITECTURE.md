# StoryForge 架构说明

> 更新日期：2026-09-05。
> 本文描述当前源码的代码边界；当前候选的验收状态以 `RELEASE-STATUS.md` 为准，功能存在不等于门禁通过。

## 架构原则

1. Campaign 是长期故事状态的主线真相源。
2. 正文、Attempt、Campaign revision 和 MutationBatch 必须通过 Turn 生命周期一致提交。
3. app/domain 层不依赖 Tauri；Tauri 是组合根和本地存储适配层。
4. LLM 可以使用名字交流，落盘和授权必须使用稳定 ID。
5. **SQLite 是默认后端**（Gate 7，2026-08-05）：无配置启动即 SQLite 权威，旧 JSON 数据启动时自动迁移（缺失集合按空导入、孤儿行跳过并计数、迁移前备份且不删除旧 JSON），全新目录初始化空 SQLite 权威；JSON 保留为显式回退（`STORYFORGE_STORAGE_BACKEND=json` / reverse-cutover 的 JsonAuthoritative marker），fail-closed cutover 与可逆导出语义不变。
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

主界面默认 `continuation`（单笔者），另有 `duet`（对手戏）和 `sequential_crew`（顺序剧组）。旧 `big_scene` 并行编排保留为显式兼容路径。下面的 Director/Subagent/Editor 是剧组路径，不是所有生成模式的固定调用序列。

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
- 已采纳正文及其之前的用户输入构成受保护前缀，编辑、删除、新增或切换变体不能绕过该约束。
- 删除未采纳正文会放弃对应 Turn/Attempt 并截断所属输入及后续未采纳内容；SQLite 使用单事务，JSON 使用终态日志与普通错误补偿，不宣称断电事务性。
- 质量自动修订使用模式无关的 `revise_draft`，只运行一次 Editor，不重演角色或创建额外变体；复核通过后才同步正文、hash 和 provenance。

## 故事快照

v3 Campaign Bundle 包含 Conversation（正文/变体）、终态 Turn/Attempt、源角色卡、本局世界书、实例、变量、知识、任务和纪要。导出前要求活动轮次结束，导入校验范围并为身份字段分配新 ID，不改写正文和任意变量值。

末尾分支复用快照导入路径，保留完整故事状态并重映射引用。当前拒绝任意历史节点分支和已采纳历史回滚，避免向历史分支复制未来状态。

v1/v2 仅作为状态交换包兼容，不含完整正文历史。v3 也不包含连接凭据、插件安装、预设、Meta 工作区、MVU 翻译缓存或外部资产；向量索引重建。完整设备迁移仍需要保留应用数据和外部资产。

## Postprocess 边界

成文后的“后处理阶段”包含两个职责不同的 Agent：

- Summarizer：生成本轮 Chronicle A / RoundSummary。
- PostProcessor：生成知识、变量和任务候选更新。

共享 `ProductionPostprocessService` 已抽出（Gate 2/3），Tauri 写作命令与真实模型
harness 调用同一后处理应用服务；`draft_hash` 同步、quality report、MutationBatch
归一化与 Attempt 状态同步、迟到结果/取消/失败传播由共享服务承载。backend 差异只
存在于 MutationBatch 如何事务落盘（JSON 多步 CAS / SQLite 原子 UoW）。

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

### SQLite（默认）

SQLite 是默认生产后端（Gate 7）。启动决议 marker-first：有效 sqlite marker →
SQLite 权威；JsonAuthoritative marker → JSON 权威（显式回退）；无 marker → 默认
SQLite（旧 JSON 树自动迁移 / 全新目录初始化空库）；stale marker 无论 env 都拒绝。

SQLite 后端具备：

- `StorageBackend::{Json, Sqlite}` 与进程 pin。
- cutover 锁、临时库、备份、内容 hash、marker-last 发布和启动恢复。
- Accept UoW、recovery、active-turn barrier。
- Chronicle publication UoW 和故障注入回滚。
- SQLite→JSON staging + atomic publish reverse export。
- Meta typed patch、MVU schema apply、Chronicle compressor 的 SQLite-native UoW
  （Gate 4 补齐，能力矩阵见 storage_backend.rs `BackendCapability`）。

Gate 7 候选周期还定义了默认切换下的兼容语义（RESULT §36）：缺失 legacy 文件按空
集合导入（与 JSON store `load_or_default` 同口径）；存在但损坏仍 fail-closed；
孤儿行（父对象已删除的残留，JSON 应用里不可达）跳过并计数；不双写、不删除旧
JSON、不遇错静默建空库。

### JSON（显式回退）

JSON 保留为限期兼容导入、反向导出和紧急回退能力，不再承载默认生产写入。选择
通道：`STORYFORGE_STORAGE_BACKEND=json`（sqlite marker 在握时 fail-closed 而非
静默回退）、reverse-cutover 的 `JsonAuthoritative` marker、settings config 值。
JSON 路径的 Turn journal、revision、MutationBatch 和恢复逻辑提供专用逻辑原子性，
但不是数据库事务。

## 插件与导入边界

- 插件 runtime 支持显式权限、运行时撤销、prompt-hook timeout/cancel/budget、审计查询/分页/retention 和显式 degraded/unsupported 兼容矩阵。
- FNV-1a audit chain 仅是本地完整性链，不是密码学签名或可信头证明。
- ST/世界书/Campaign Bundle 导入执行 fail-closed 引用校验与补偿回滚；真实复杂卡仍需在合法 fixture 环境补证据。
- 插件 iframe、真实第三方扩展和完整 ST 长尾语义仍需 GUI 验收。

## 发布边界

- 本地 workspace、前端和 host-side release runner 已有自动化证据。
- Linux Gitea runner 已投入运行；Windows runner 执行尚未验证。
- Windows bundle、Android APK、签名、GUI 和真机证据必须在 `docs/RELEASE-CHECKLIST.md` 单独记录。
- 真实模型证据（Gate 6，RESULT §35）：Canary3/Coverage12/TextFallback3/Stability30
  已 PASS 并 seal；Full100 受 relay 间歇不稳定阻断未完成（r3 跑到 58/100 全健康），
  Gate 6 已按决议关闭（2026-08-31，关闭非 PASS，RESULT §35.9）——旧 45/100 Partial Evidence
  不复活、不视为 PASS。
- Android 模拟器现场（§11.3，15 项验收点）与 Windows 桌面现场已 PASS；release
  APK 签名无证书 BLOCKED；Android 真机与第三方插件现场矩阵仍缺。

## 当前主要技术债

1. 100-turn 真实模型长程证据与 §35.8.5 两项探测缺口（forbidden_story_facts 检测、MustNotReveal 主动探测）——Gate 6 已关闭非阻塞，如需补全另行立项（RESULT §35.9）。
2. 完整候选周期统计与 JSON 生产写路径删除（Gate 7 §12.1.2/§12.1.5，发布后/稳定期后）。
3. Windows runner、Android 真机、release 签名与可离线验证的真实产物证据。
4. GUI 端到端、第三方插件现场矩阵。

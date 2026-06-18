# W10 执行手册：JS Runtime 接通写作流程

> 交接对象：Claude Code（在 worktree `storyforge-w10-jsruntime` 分支 `w10-jsruntime` 工作）
> 前置必读：`docs/PLAN-PLUGIN-MVU.md` 阶段 5、`crates/infra-plugin-host/src/mvu_runtime.rs`、`docs/HANDOFF-W8-JSRUNTIME.md`（W8 实现 runtime）
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w10-jsruntime`
> 分支：`w10-jsruntime`（已基于 `main` a47d645）
> 性质：后端接通（DI 注入 + postprocess 调 execute_fragment），与 W9（前端）零文件交集。

## 一、任务概述

W8 实现了 `WebViewMvuRuntime`（trait 异步化 + JSR/ST shim + Tauri state 持有），但 **postprocess/pipeline 从未调用 `execute_fragment`**——runtime 造好了没装上。`fallback_fragments` 非空的 MVU 卡，跑写作时 JS 仍不执行。

W10 把 runtime 接进写作流程：postprocess 处理完正常知识/变量后，若卡的 `MvuTranslation.fallback_fragments` 非空，调 `execute_fragment` 跑 JS，结果经现有 preview/patch 回写。

## 二、核心技术难点：依赖方向（必读）

**问题**：`WebViewMvuRuntime` 在 `tauri-app` crate（lib.rs:4895 创建并 manage）。`run_postprocess` 在 `app-pipeline` crate。**pipeline 不能依赖 tauri-app**（依赖方向反了——tauri-app 依赖 pipeline，不是反过来）。

**解法：依赖注入（DI）**：
- `app-pipeline` 依赖 `infra-plugin-host`（已有依赖？查 Cargo.toml；若无则加），拿 `MvuRuntime` trait。
- `PipelineOrchestrator` 加字段 `mvu_runtime: Option<Arc<dyn MvuRuntime + Send + Sync>>`（None=不支持 JS fallback，降级）。
- `PipelineOrchestrator::new`（pipeline:170）加参数注入。
- `tauri-app` 的 `new_pipeline`（lib.rs:340）创建 orchestrator 时，注入 `Some(Arc::new(WebViewMvuRuntime::...))` 或克隆已 manage 的 Arc。
- harness 的 `new_pipeline` 传 `None`（harness 无 WebView，降级）。

⚠️ **trait 异步化已在 W8 完成**（`execute_fragment` 是 `async fn`）。postprocess 调它要 `.await`——`run_postprocess` 本身是 async（pipeline:558），OK。

## 三、现状摸底（已核实）

**W8 已就绪**（infra-plugin-host/src/mvu_runtime.rs）：
- `MvuRuntime` trait（:69）：`async fn execute_fragment(fragment_js, current_variables) -> Result<MvuExecResult>` + `load_card_assets`/`unload_card` + `is_available`。
- `WebViewMvuRuntime`（:177）：通过 Tauri event 与前端通信，已实现。
- `MvuExecResult`：`variable_updates: HashMap<String, Value>` + `side_effects: Vec<String>`。

**pipeline 现状**（app-pipeline/src/lib.rs）：
- `PipelineOrchestrator::new`（:170）：构造器，无 MvuRuntime 字段——要加。
- `run_postprocess`（:558）：处理知识/变量/任务更新，**完全不碰 MvuTranslation/fallback_fragments**——要加 JS 执行段。

**MvuTranslation 在哪**（app-meta/src/mvu_import.rs）：
- `MvuTranslation.fallback_fragments: Vec<FallbackFragment>`（:366），`FallbackFragment.js_snippet`（:334）。
- 写作时怎么拿到卡的 MvuTranslation？读 `run_postprocess` 的 ctx 参数 + `CampaignStore::get_mvu(source_character_id)`——postprocess 能否从 ctx 拿到 source_character_id？查 `WritingContext` 字段。可能要在 ctx 或 run_postprocess 参数补传 mvu_translation。

## 四、任务

### 任务 1：pipeline 依赖 infra-plugin-host + DI 字段

改动：`crates/app-pipeline/Cargo.toml` + `crates/app-pipeline/src/lib.rs`。

- Cargo.toml 加 `storyforge-infra-plugin-host` 依赖（若无）。
- `PipelineOrchestrator` 加 `mvu_runtime: Option<Arc<dyn MvuRuntime + Send + Sync>>` 字段。
- `new` 加参数 `mvu_runtime: Option<Arc<dyn MvuRuntime + Send + Sync>>`。
- 所有 `PipelineOrchestrator::new` 调用点适配（grep `::new(` in pipeline + tauri-app）。

### 任务 2：postprocess 接 execute_fragment

改动：`crates/app-pipeline/src/lib.rs` `run_postprocess`（:558）。

在正常知识/变量更新**之后**加 JS fallback 段：
1. 拿当前卡的 `MvuTranslation`（从 ctx 或 store 查 `get_mvu(source_character_id)`）。
2. 若 `fallback_fragments` 非空 **且** `mvu_runtime.is_available()`：
   - 构造 `current_variables`（角色实例 + Campaign 变量快照）。
   - 对每个 `fallback_fragment`，调 `mvu_runtime.execute_fragment(&fragment.js_snippet, &current_variables).await`。
   - 收集 `MvuExecResult.variable_updates`。
3. 结果经**现有 preview/patch 机制**回写（不直接写 store，红线）——或先记录到 PostProcessOutcome 让用户确认。读现有变量更新怎么落盘（`persist_postprocess_outcome` 的 variable_updates 路径），JS 结果走同样路径。
4. JS 失败 → `tracing::warn!` + 跳过（不影响主写作，红线）。`mvu_runtime` 为 None → 降级提示。
5. `side_effects` 记录到日志（暂不自动执行）。

### 任务 3：tauri-app 注入 WebViewMvuRuntime

改动：`crates/tauri-app/src/lib.rs` `new_pipeline`（:340）。

- 创建 orchestrator 时注入 `Some(mvu_rt_arc)`（W8 已 manage 的 Arc）。
- 读 W8 怎么创建 `WebViewMvuRuntime::with_shared_pending`（lib.rs:4895），复用同一实例。

### 任务 4：harness 传 None

改动：`crates/harness-real-llm/src/lib.rs` `new_pipeline`。

- harness 无 WebView，传 `None`（降级，确定性测试不受影响）。

### 任务 5：测试

- `cargo test --workspace` 0 回归（DI 改动影响 `new` 签名，所有调用点仔细改）。
- 新增：postprocess 在 `mvu_runtime=None` 时降级不崩；`fallback_fragments` 空时不调 runtime。
- 真实端到端（可选，需 JS fallback 卡 fixture）：跑一轮看 JS 执行经 patch 回写。

## 五、关键约束

- **不绕过 preview/patch 写变量**（JS 结果走现有 variable_updates 落盘路径，红线）。
- **JS 失败不影响主写作**（catch + warn + 跳过）。
- **DI 方向正确**：pipeline 依赖 infra-plugin-host（trait），不依赖 tauri-app（实现）。
- **`mvu_runtime` 用 Option**：None=降级，harness/无 WebView 环境正常跑。
- **不碰 W9 的前端文件**。
- **不 commit**。
- **trait 已 async**（W8 完成），postprocess 调用 `.await` 即可。

## 六、给 Claude Code 的提示词

```
请阅读 docs/PLAN-PLUGIN-MVU.md 阶段5、docs/HANDOFF-W8-JSRUNTIME.md(W8 实现 runtime)、
docs/HANDOFF-W10-JSRUNTIME.md(本文件),然后接通 JS runtime 到写作流程。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w10-jsruntime
分支：w10-jsruntime

W8 实现了 WebViewMvuRuntime(trait 异步化+JSR/ST shim+Tauri state 持有),但
postprocess/pipeline 从未调 execute_fragment——runtime 造好了没装上。

核心技术难点:依赖方向。WebViewMvuRuntime 在 tauri-app,pipeline 不能依赖 tauri-app。
解法 DI: pipeline 依赖 infra-plugin-host(MvuRuntime trait),PipelineOrchestrator 加
Option<Arc<dyn MvuRuntime+Send+Sync>> 字段,tauri-app 注入 WebViewMvuRuntime,harness 传 None。

先读这些理解现状:
- crates/infra-plugin-host/src/mvu_runtime.rs MvuRuntime trait(:69 async fn execute_fragment)
  + WebViewMvuRuntime(:177) + MvuExecResult
- crates/app-pipeline/src/lib.rs PipelineOrchestrator::new(:170 加参数) +
  run_postprocess(:558 加 JS 段)
- crates/app-meta/src/mvu_import.rs MvuTranslation.fallback_fragments(:366) +
  FallbackFragment.js_snippet(:334)
- crates/tauri-app/src/lib.rs new_pipeline(:340 注入) + WebViewMvuRuntime 创建(:4895)
- crates/harness-real-llm/src/lib.rs new_pipeline(传 None)

任务:
1. app-pipeline/Cargo.toml 加 infra-plugin-host 依赖。PipelineOrchestrator 加
   mvu_runtime: Option<Arc<dyn MvuRuntime+Send+Sync>> 字段,new 加参数。
   适配所有 new 调用点(grep)。
2. run_postprocess 加 JS fallback 段:拿卡 MvuTranslation→fallback_fragments 非空且
   mvu_runtime.is_available()→execute_fragment(每 fragment js_snippet+current_variables)
   →收集 variable_updates→走现有 variable_updates 落盘路径(经 preview/patch,不直接写 store)
   →JS 失败 warn+跳过(不影响主写作)→mvu_runtime None 降级。
   side_effects 记日志。
3. tauri-app new_pipeline 注入 Some(WebViewMvuRuntime arc)。
4. harness new_pipeline 传 None。
5. cargo test --workspace 0 回归 + 新增 None 降级/fallback 空不调 runtime 测试。

红线: JS 结果经 preview/patch 不直接写 store / JS 失败不影响主写作 /
DI 方向正确(pipeline 依赖 trait 不依赖 tauri-app) / mvu_runtime 用 Option /
不碰前端 / 不 commit / trait 已 async 直接 .await。

先做 DI(任务1),再 postprocess 接通(任务2),再注入(任务3/4),每步 cargo check。
```

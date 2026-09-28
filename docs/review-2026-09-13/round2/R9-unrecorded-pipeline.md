# R9 收口：无记录两条 —— W-18（big_scene 自动路由可达性）+ W-32（分段格式器等价性测试）

- **任务**：task-32（收口 R9）
- **执行人**：review-pipeline（域3）
- **日期**：2026-09-13
- **基线**：HEAD `ab894c6` + 本轮脏工作树
- **依据**：`round2/R6-fix-completeness-audit.md:276`（W-18 无记录）、`round2/R2-pipeline-recheck.md:120`（N-R2-01）、
  `round2/R2-pipeline-recheck.md` §6 观察 7（W-32 缺等价性测试）、Lead 的 task-32 指令
- **前置**：task-34（R11）已完成（`round2/R11-pipeline-regression-closure.md`）

---

## 1 结论摘要

| 项 | 处置 | 证据等级 |
| --- | --- | --- |
| **W-18** big_scene 自动路由 + 成本确认 | **判定非问题（保留，不删）**：是**有书面契约的 API 级能力**（未传档位的调用才进自动路由），且 `None` 路径可由命令层到达并被测试覆盖 | 设计文档 + 命令签名 + 6 条测试名（本报告 §2） |
| W-18 的"前端 UI 永远不触发" | **设计如此**（显式档位优先 + 前端展示预计调用量），非缺陷；**不删后端能力** | `docs/workstreams/WRITING-PIPELINE-V2-IMPLEMENTATION-2026-07-27.md:9/:66` |
| W-18 残留（前端死分支） | 非本任务写作用域 → 建议交前端（`useWritingScreenAdapter.js:89` 等） | §2.4 |
| **W-32** `format_subagent_context_*` 重复实现 | 去重已完成（只剩委托薄壳）；**本轮补"逐字节 golden + 委托等价"测试**，并实测失败可控 | 测试 + 实测红/绿（§3） |

---

## 2 W-18：判定非问题（保留后端能力）

### 2.1 原发现与 R2 复检

- 原 W-18（`03-writing-pipeline.md:484-505`）：`generation_route_signals*`（约 160 行）与
  `route_generation_mode` 的自动升档分支在前端路径**永不生效**；`enforce_generation_cost_confirmation` 的 fail-closed 分支不可达。
- R2 N-R2-01 复核成立：`generationMode` computed **永不为 null** → `start_writing` 恒带显式档位 →
  路由立即返回 `ExplicitChoice` + `requires_cost_confirmation=false`。

### 2.2 反方向证据：这不是死代码，而是**书面契约**下的 API 能力

1. **书面设计契约**：`docs/workstreams/WRITING-PIPELINE-V2-IMPLEMENTATION-2026-07-27.md`
   - `:9`「**显式选择优先**：用户选定的生成模式永远覆盖自动路由，并按 Campaign 记住。」
   - `:66`「前端提供显式模式选择、展示当前档位的预计调用量，并将选择保存在 Campaign 级本地记忆中。
     **未传模式的 API 调用才进入上述自动路由**；若自动规则建议升级到昂贵的 Sequential Crew，
     后端会在写入开场白或用户消息之前 fail closed，返回建议模式与 `2+N` 费用提示。
     调用方确认后须显式以 `generation_mode=sequential_crew` 重试……显式选择从不重复询问。」
   ⇒ 自动路由的存在意义就是**给省略档位的调用方**（外部/工具/未来 CLI）用；前端始终显式传档位是**设计要的行为**，不是接线遗漏。
2. **命令层契约**：`crates/tauri-app/src/commands/writing.rs:877` 的参数是
   `generation_mode: Option<storyforge_domain::generation::GenerationMode>`；
   `:927-930` 用 `generation_route_signals(&intent, &ctx, generation_mode)` 调
   `route_generation_mode` 并执行 `enforce_generation_cost_confirmation`。
   **省略即 `None`，路径真实可达**（不是"函数存在"）。
3. **该路径被测试实际执行**（具体测试名）：
   - `crates/tauri-app/src/lib_tests_writing.rs:2900`（测试 `start_writing_command_prompt_hook_messages_reach_mock_llm`）
     以 `generation_mode = None` 调用 Tauri 命令 `start_writing`，走完路由 + 守卫；
   - `lib_tests_writing.rs:2930-2935`（`regenerate_command_prompt_hook_messages_reach_mock_llm` 的 setup）以
     `Some(BigScene)` 调用，覆盖显式分支。
4. **路由引擎与守卫的分支覆盖**（命令层 4 条）：
   `automatic_route_promotes_large_roster_to_sequential_crew`、
   `automatic_route_detects_explicit_two_actor_interaction`、
   `automatic_expensive_route_requires_an_explicit_resubmission`、
   `cheap_or_explicit_route_needs_no_extra_confirmation`（`commands/writing.rs:1794-1878`）；
   域层另有 11 条（`crates/domain/src/generation.rs:120-236`，含
   `explicit_mode_always_wins_without_surprise_confirmation`、`four_or_more_principal_actors_route_to_sequential_crew`、
   `generation_mode_call_estimates_are_a_stable_product_contract`）。

### 2.3 为什么**不能删**（选项 (b) 的条件不成立）

- 删除需要"全仓无任何调用方（含测试）"：**两条测试直接调用/覆盖该路径**（§2.2-3/4），条件不成立；
- 该路径是**跨进程 API 契约**：Tauri 命令参数为 `Option`，前端 wrapper
  （`frontend/src/tauri-api.js:394-406`，`generationMode: generationMode || null`）**已经具备传 `null` 的能力**；
  删掉会让"省略档位"变成未定义行为，属破坏契约；
- `docs/AGENT_INTERFACES.md` §生成模式 把四种模式列为对外能力，自动路由是其选定机制。

### 2.4 诚实声明：未兑现的只有"UI 提示"这一层的期望

- **生产事实**：唯一的生产调用方是前端（`useWriting.js:141` → `writingStore.generationMode`），
  而 `stores/writing.js:58-63` 的 computed 在"无 Campaign"或"未手动改档"时返回 `'continuation'`（**永不为 null**）
  ⇒ 真实 UI 里自动路由与成本确认**确实不会被触发**。
- **这是设计取舍而非缺陷**：设计文档明确"显式选择优先"，且前端确实展示了档位与预计调用量
  （`ComposerBar.vue:70` + `utils/generationModes.js:23 generationModeCostLabel`）。
- **前端也没有成本确认交互**：全前端对 `enforce_generation_cost_confirmation` 的错误文案零处理
  （grep「自动路由/成本确认」在 `frontend/src/**` 仅 `generationModes.js` 的成本标签）。
  ⇒ 若产品想让 UI 用上该能力，需要一次**真实功能开发**（不是一行改动）：
  1. 区分"用户未显式选档"与"用户选了 continuation"（当前 store 用同一个 `'continuation'` 兜底，无法区分）；
  2. 未显式选档时传 `null`，并在收到该 `Err` 后弹确认对话框（展示建议档位 + 预计调用量），确认后带 `generation_mode=<建议>` 重试。
- **本任务不改前端**（写作用域约束）。**建议 Lead**：把它作为产品决策立项（要么实现上述交互，要么在
  `WRITING-PIPELINE-V2…md` 里把"前端提示"从 UI 期望里去掉、明确自动路由只服务 API 调用方）。
- **残留死分支（前端）**：`frontend/src/adapter/useWritingScreenAdapter.js:89`
  （`writing.generationMode === 'big_scene'`，W-31 修完后已恒不成立）与 `frontend/src/utils/rerollPolicy.js:8` 的
  `big_scene` 分支 —— 非本任务写作用域，归前端清理（task-33 记录已登记 `:89`）。

**W-18 最终判定：判定非问题（附书面契约 + 命令层调用点 + 6 条测试名）；不删、不改后端。前端"自动提示"若要落地需产品立项。**

---

## 3 W-32：分段格式器——委托等价 + golden 字节锁

### 3.1 现状（复检后的事实）

- `crates/app-agent/src/runtime.rs:1005/1029`：`format_context_stable` / `format_context_volatile` 是**唯一实现**（`pub`）。
- `crates/app-pipeline/src/lib.rs:4348-4356`：`format_subagent_context_stable/volatile` 已是 `#[inline]` **委托薄壳**
  （W-32 的去重已落地），调用点为路径 C 的 `lib.rs:2019/2023`。
- 因此"两份实现逐字节对比"**已不可测**（只剩一份实现）；R2 §6 观察 7 指出的真实缺口是：
  **没有测试钉住"委托产出/分段字节"，去重可能被后人以内联副本的方式重新破坏**。

### 3.2 补的测试（`crates/app-pipeline/src/lib.rs` tests 模块）

`tests::path_c_context_formatters_delegate_byte_for_byte_and_match_golden`：

1. **委托等价**：`format_subagent_context_stable/volatile(pkg)` == `storyforge_app_agent::format_context_*(pkg)`；
2. **golden 字节锁**（真正防漂移）：对含角色设定 + 常驻世界设定 + 场景 + 相关世界设定 + 最近对话的
   `ContextPackage`，断言两份输出**逐字节等于冻结文本**（分区标题、空行、`keys.join(", ")` 分隔符都在锁内
   —— 这些字节直接影响 system/tail 分段与 LLM 缓存键）；
3. **空包边界**：全空 `ContextPackage` → 两边都是空串（`task` 字段不进分区）。

### 3.3 失败可控实测

临时把 `app-agent/src/runtime.rs:1009` 的 `"## 你的角色设定"` 改成 `"## 角色设定"` → 测试立刻红：

```
assertion `left == right` failed: stable 分段文本已变更（会影响 system 段缓存键）
  left: "## 角色设定\n冷静的外科医生…"
 right: "## 你的角色设定\n冷静的外科医生…"
```

改回后：`cargo test -p storyforge-app-pipeline path_c_context_formatters` → `1 passed; 0 failed`。

---

## 4 门禁（本轮实际运行）

| # | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 1 | `cargo test -p storyforge-app-pipeline` | 0 | **141 passed / 0 failed**（W-32 新增 1 条；R11 后为 140） |
| 2 | `cargo test -p storyforge-app-pipeline path_c_context_formatters` | 0 | 1 passed / 0 failed（失败可控见 §3.3） |
| 3 | `cargo test -p storyforge-app-agent` | 0 | 132 passed / 0 failed（本任务未改该 crate 源码） |
| 4 | `cargo test -p storyforge-domain` | 0 | 388 passed / 0 failed（本任务未改该 crate） |
| 5 | `cargo test -p storyforge --lib` | 0 | **477 passed / 0 failed / 3 ignored**（R11 后基线）；本任务收尾复跑 **480 / 0 / 3**——+3 来自**并发写者**在同一窗口对 tauri-app 的新增测试（本任务唯一代码改动是 app-pipeline 的 `#[cfg(test)]` 模块，不可能改变该测试二进制） |
| 6 | `cargo clippy -p storyforge-app-pipeline -p storyforge-app-agent -p storyforge-domain -p storyforge-app-memory --all-targets -- -D warnings` | 0 | `Finished`，0 warning |
| 7 | `cargo clippy -p storyforge --lib -- -D warnings` | 0 | `Finished`，0 warning |

- 未运行 `cargo test --workspace`（Lead 收口项）。
- 本任务改动文件：`crates/app-pipeline/src/lib.rs`（仅 tests 模块新增 1 条测试）、
  `docs/review-2026-09-13/round2/R9-unrecorded-pipeline.md`、`docs/review-2026-09-13/fixes/03-pipeline-fixes.md`（§14）。

---

## 5 诚实声明

1. **W-18 未做代码改动**——因为证据指向"有意的 API 契约"，而非冗余；删除会破坏契约与 2 条测试。
   若 Lead 更希望"产品只保留 UI 可达的能力"，请明示（那应改为**前端**接线 + 确认交互，且需先区分"未选档"与"选 continuation"）。
2. **W-18 的"用户永远拿不到自动升档提示"在 UI 层成立**，本报告不把它写成"已解决"。
3. W-32 的等价性测试只能锁定**当前唯一实现**的输出；若将来真的再出现第二份实现，
   应把本测试扩展为"两份实现 + golden"三向对比（现在的 golden 已能让任何漂移变红）。

# Harness 排查发现报告（2026-06-18）

> 来源：`crates/harness-real-llm`（真实 LLM + 确定性测试 harness）
> 真实 LLM：`deepseek-v4-flash` @ opencode.ai（T1 跑通，171s）
> 范围：写作流水线全链路 + 各 agent 知识边界（信息隔离）

## 发现总览

| ID | 标题 | Severity | 类型 | 状态 |
|---|---|---|---|---|
| F1 | LLM model 名透传断链——连接配的 model 被忽略 | 🔴 High | Bug（影响所有用户） | **已修**（`HttpLlmClient.effective_model` + wrapper 已删） |
| F2 | 角色识别 Agent 输出解析降级（emit_characters 5 层兜底全 miss） | 🟠 Medium | Bug（runtime/loop） | **已修**（terminal_tools 终止机制 + 真实 LLM 验证） |
| F3 | MVU apply backfill loop 死代码（card.id 当 campaign_id） | 🔴 High | Bug（生产死代码） | **已修**（`list_all_instances` + definition_id 过滤） |
| P0 | 子 agent get_character 绑定-unresolvable 读侧泄漏 | 🟠 Medium | Bug（defense-in-depth） | **已修 + B0 钉测** |
| P3 | postprocess 写回空集逃生口（present_chars 空⇒全过） | 🟠 Medium | **设计缺陷**（门禁按"在场"一刀切所有来源） | **重定位**：非"收紧/保留"取舍，根因是门禁不区分知识来源。最小修复=按 `KnowledgeSource` 分流（见 §P3）。完整"知识传播引擎"单独立项 `PLAN-KNOWLEDGE-PROPAGATION.md` |
| P4 | postprocess 写回 name/id 三路匹配别名风险 | 🟡 Low | 设计取舍 | **待修**（方案A：同名时 name 路失效逼 id） |
| T3 | `t3_regenerate_all` 吞错误致测试假绿 | 🟡 Low | 测试质量 | **已修**（`regenerate_with_retry` 只对 `Llm` 重试 + 严格断言） |
| 验证 | 知识边界读侧隔离（volatile tail / get_character / temp instance） | ✅ | 验证通过 | 4/4 钉测绿 |

## 🔴 F1：LLM model 名透传断链

**现象**：连接配置里设的 model 名（如 `deepseek-v4-flash`）被完全忽略，实际请求永远发硬编码的 `deepseek-chat`，导致非 deepseek-chat 模型直接 401/403。

**根因**：
- `HttpLlmClient::new` 存了 `conn.model` 到 `self.model`（`infra-llm/src/http_client.rs:104`），但 `chat()`/`chat_stream()` 用的是 `req.model`（`ChatRequest.model`），**从不读 `self.model`**（http_client.rs:134）。
- `ChatRequest.model` 来自各 agent 的 `AgentRunConfig.model`，而 `make_director_config`/`make_editor_config`/`make_subagent_config`/postprocess/summarizer/character_extractor 全部硬编码 `"deepseek-chat"` 默认值（`app-pipeline/src/lib.rs:1482`、`:1538`；`app-agent/src/prompts/postprocess.rs:88`、`summarizer.rs:50`、`character_extractor.rs:63`）。
- 只有 `AgentProfileConfig` 提供了 `model_override` 时才覆盖默认值——普通用户不配 AgentProfileConfig 就永远用 `deepseek-chat`。

**影响**：换任何非 deepseek-chat 模型（如 `deepseek-v4-flash`、OpenAI 模型、其他 OpenAI 兼容服务）都会失败。这是 StoryForge 当前最影响可用性的 bug——用户在前端连了别的模型却报"模型不支持"。

**复现**：T1 第一次跑（`cargo test -p harness-real-llm -- --ignored`，env `LLM_MODEL=deepseek-v4-flash`）报 `Auth("Model deepseek-chat is not supported")`。

**harness 临时绕开**：`ModelPinningLlmClient`（`harness-real-llm/src/lib.rs`）包装真实 client，强制把每个 `ChatRequest.model` 改成连接配置的 model。**这是 harness 跑通的权宜手段，不解决业务侧问题**。

**业务修复建议**（二选一）：
1. `HttpLlmClient::chat`/`chat_stream` 在 `req.model` 为空或等于占位默认时回退到 `self.model`；或直接用 `self.model` 覆盖 `req.model`（让连接配置成为 model 名单一来源）。
2. 各 `make_*_config` 不再硬编码 `deepseek-chat`，改为从活跃 `LlmConnection.model` 读取注入。

推荐 1（改一处 `HttpLlmClient`，集中且不破坏 AgentProfileConfig 的 override 能力——override 时 `req.model` 非空，可在 `req.model` 非空且非默认占位时优先用 req）。

**已修复（2026-06-18）**：采用方案 1。
- `http_client.rs` 新增 `effective_model()` helper：`req.model` 为空或占位（`"deepseek-chat"`/`"mock"`）时回退 `self.model`，否则保留 `req.model`（AgentProfileConfig override）。
- `chat()` 和 `chat_stream()` 在构造请求 body 前调用 `effective_model()`。
- 5 个单测覆盖所有分支（placeholder 回退、空回退、非占位保留、同名保留）。
- `harness-real-llm` 的 `ModelPinningLlmClient` wrapper 已删除，`require_real_llm()` 直接返回 `Arc::from(client)`。
- `async-trait` 依赖从 harness Cargo.toml 移除。
- `cargo test --workspace` 全绿，0 回归。

## 🟠 F2：角色识别 Agent 输出解析降级

**现象**：真实 LLM（deepseek-v4-flash）跑 `extract_characters` 时，`emit_characters` 工具调用的输出无法被 `parse_character_definitions_from_response` 解析，5 层兜底全 miss，降级为单角色卡。

**证据**（T1 日志第 34 行）：
```
角色识别失败，降级单角色: 输出解析失败（5 层兜底全miss）: 5 层兜底全miss；
content 前 200 字: 任务已完成。最终结果已通过 `emit_characters` 工具成功输出。
...本卡 Seraphina 中仅识别出 1 位独立角色...
```

LLM 输出的是自然语言总结（"任务已完成，结果已通过 emit_characters 工具输出"），而非 `emit_characters` 工具调用的结构化 JSON。说明：
- 模型可能没正确触发 native function calling，或触发了但输出格式不符 `parse_character_definitions_from_response` 的 5 层兜底（emit_characters 工具调用 / 整体 JSON / ```json 代码块 / 裸代码块 / 手写括号配平）。
- `deepseek-v4-flash` 是轻量模型，function calling 能力可能弱于 `deepseek-chat`。

**影响**：多角色卡只识别出单角色，Director 无法分配多 instance。降级路径保证不崩（建单角色 protagonist），但削弱了 Campaign 多角色体验。

**复现**：T1 跑 `extract_characters` 即可见。

**待查**：换 `deepseek-chat` 或更强模型是否复现；`parse_character_definitions_from_response` 的 5 层兜底是否覆盖了"模型把工具结果写在 content 里"这种形态。未修。

**诊断结论（2026-06-18 Round 1 初诊）**：
- 根因：`deepseek-v4-flash` function calling 能力弱，输出自然语言描述（"任务已完成，结果已通过 emit_characters 工具输出"）而非实际 tool_call 或 JSON。5 层兜底全部 miss 是因为 content 里根本没有 JSON。
- 这是模型能力限制，非代码 bug——降级路径已保证不崩。

**Round 2 深修诊断（2026-06-18）**：
- 抓 T1 raw response 后发现：**模型确实调了 `emit_characters` 工具**（tool_calls 非空，content 为空），Round 1 诊断有误。
- 真正根因：`run_tool_loop` 没有"终止工具"概念——模型调用 `emit_characters` 后，工具执行成功，但 loop 继续到下一轮。模型在每轮都调 `emit_characters`，8 轮耗尽 → `MaxRoundsExceeded`。
- 这不是模型能力问题，是 **runtime 缺少终止工具机制**。

**已修复（2026-06-18 Round 2）**：
- `AgentConfig` 新增 `terminal_tools: Vec<String>` 字段。
- `run_tool_loop` / `run_tool_loop_streaming` 执行工具后检查：若调用了 `terminal_tools` 内的工具，立即返回响应（不等模型输出最终文本）。
- `make_character_extractor_config` 设置 `terminal_tools: vec!["emit_characters"]`。
- 新增单测 `test_terminal_tool_stops_loop` 验证终止机制。
- 真实 LLM T1 验证：`extract_characters Ok: 1 definitions (正常解析路径)`，Seraphina 卡正确识别，`tool_calls[0].name == "emit_characters"`。
- `cargo test --workspace` 全绿，0 回归。

## 🔴 F3：MVU apply backfill loop 死代码（已修）

**现象**：`meta_apply_mvu_schema`（`tauri-app/src/lib.rs` `meta_apply_mvu_schema` 命令体）写完 card definition 后，本应对已存在的角色实例补齐 MVU 新增变量，但该 backfill loop **在生产环境永不执行**——是死代码。

**根因**：backfill loop 用 `store.list_instances(&card.id)`，而 `CampaignStore::list_instances(campaign_id)` 按 `instance.campaign_id == *campaign_id` 过滤（campaign_store.rs:254-262）。传入的 `card.id` 是 **card_id**，instance 的 `campaign_id` 字段永远不等于任何 `card.id` → 过滤恒为空 → loop 体永不执行。

**影响**：用户在前端应用 MVU 合并 schema 后，card definition 更新了，但**已存在的角色实例不会自动补齐新变量**（`VariableValue`）。后果是实例与新 schema 不一致——后续读写实例变量会缺字段。这是审计标的"dead UI surface"背后的真实 bug。

**复现**：任何"已有实例的卡 → analyze MVU → apply schema"流程，实例 variables 不增长。

**harness 假绿陷阱**：Round 2 的 C7 测试 `c7_mvu_apply_backfill`（c_command_layer.rs）**用正确的 `campaign_id`** 复刻了 loop 结构，测试绿——但它测的是生产永远走不到的路径，掩盖了这个 bug。

**修复（c0456aa）**：
- `lib.rs` backfill loop 改用 `store.list_all_instances()`（全量）+ 保留 `definition_id` 过滤条件（instance 与 definition 的关联本就该按 definition_id，一张卡的 definition 可被多个 campaign 引用，全部都该 backfill）。
- 同步 `c7_mvu_apply_backfill` 测试复刻新生产路径（`list_all_instances`），消除假绿。
- 加注释钉死历史 bug，防回退。
- `cargo test --workspace` 全绿，0 回归。

## 🟠 P0：子 agent get_character 绑定-unresolvable 读侧泄漏（已修）

**现象**：子 agent 的 `current_character_instance_id=Some` 且 `campaign_runtime=Some`，但绑定 id 在 `runtime.instances` 中找不到时，旧实现 fallthrough 到未隔离的扁平 `ctx.characters` 搜索，泄漏其他角色数据。

**根因**：`app-agent/src/tools.rs:423-456`（旧）的 `if let Some(inst) = runtime.instances.iter().find(...)` 不匹配时无 `else`，控制流落出外层 `if let Some(runtime)`，进入下方未隔离的扁平 Character 查询。

**正常流程不可达**：`spawn_subagents` 用同一批 `Arc<CampaignRuntimeContext>` 做匹配与 tool ctx，绑定 id 必然可解析。属 defense-in-depth 洞，但修复小、安全、符合隔离意图。

**修复**：`tools.rs` 加 `else` 分支硬失败返回 `NotFound`，拒绝降级到扁平查询。

**钉测**：`test_subagent_get_character_bound_but_unresolvable_does_not_leak`（`app-agent/src/tools.rs`）。**80 个 app-agent 测试全绿，0 回归**。

## 🟠 P3：postprocess 写回门禁用"在场"一刀切所有知识来源（重定位）

**原定性**（Round 1）：空集逃生口，`present_ids.is_empty() → 全放行`，标为"收紧/保留"取舍。

**重定位**（2026-06-18，经用户场景质询推翻"收紧"建议）：这不是取舍问题，是**门禁用了错误的判据**。`is_postprocess_instance_present`（`tauri-app/src/lib.rs:1947`）对**所有来源**的知识写入都套"在场"约束，但知识来源不同，传播规则本就该不同：

| `KnowledgeSource` | 该不该受"在场"约束 | 理由 |
|---|---|---|
| `Witnessed`（亲眼所见） | **该** | 不在场不可能亲眼见 |
| `Inferred`（推断） | **该** | 推断基于自己已知，不跨人 |
| `ToldByOther`（被告知） | **不该** | 告知本就是跨在场传播（写信/密语/传话） |
| `Backstory`（背景） | **不该** | 开局就有，与在场无关 |

（数据模型见 `domain/src/character_knowledge.rs:17` `KnowledgeSource` 枚举，`ToldByOther` 已记 `source_character_id`。）

**用户场景质询（推翻"收紧"的关键）**：
- **世界公告/广播**：N 个角色都该知道，与在场无关。现状靠空集逃生口 hack 全放行——收紧会杀掉这个唯一能跑通的路径。
- **身份组传播**：所有守卫/贵族该知道。`Character.group` / `role_type` 字段已存在，门禁和写入都不读它。
- **定向告知/写信**：A 明确告诉不在场的 B。`ToldByOther` 数据模型已支持，但门禁把不在场的 B 拦掉，B 永远收不到信。

**根因**：门禁对 `ToldByOther`/`Backstory` 走"在场"判定 = 用错误判据。空集逃生口是这套错误判据的**症状补丁**，不是病根。

**最小正确修复（P3，worktree w3-p3p4 执行）**：门禁按 `KnowledgeSource` 分流，不收紧不保留：
- `present_ids` 空 + `Witnessed`/`Inferred` → **拒绝**（真 bug：没人在场不可能有见证/推断）
- `present_ids` 空 + `ToldByOther`/`Backstory` → **放行**（告知面向不在场的人，合理）
- `present_ids` 非空 → 现有 id 路判定（+ P4 同名收紧）

此修复让广播/告知天然成立，**不再依赖空集 hack**，且不需新数据模型——只改门禁 + postprocess 把来源传进来。

**完整"知识传播引擎"单独立项**：身份组广播、传话链（A→B→C）、秘密封口（禁止传播）是更大的功能，超 P3 范围。详见 `docs/PLAN-KNOWLEDGE-PROPAGATION.md`（本 worktree 新增），纳入 ROADMAP 作为 Phase 2 隔离增强项。

**当前行为钉测**：`b3_empty_present_chars_escape_hatch_current_behavior`（绿，记录当前全放行）。
**期望行为占位**：`b3_empty_present_chars_should_reject_when_tightened`（`#[ignore]`）—— 分流落地后此测试需重写为"按来源分流"断言，而非简单翻转。

## 🟡 P4：postprocess 写回 name/id 三路匹配别名风险

**现象**：`is_postprocess_instance_present` 用三路匹配：`raw_id` / `inst.id` / `inst.name` 任一在 `present_ids` 即算在场。若两个 instance 同名，present 含该 name 时两者都通过——name 匹配无法区分。

**钉测**：`b4_present_chars_name_id_matching`（绿，正常 name/id 匹配）+ `b4_name_collision_both_pass`（绿，记录同名歧义行为）。

**待修（方案A，worktree w3-p3p4 执行）**：保留 name 兜底（不让 postprocess 瘫痪——它按名字输出是既有契约），但在**同名场景**强制走 id：检测到 Campaign 内存在同名 instance 时，name 路失效，逼上游/Director 用 instance_id。比"全删 name 路"（方案B，需改 postprocess 输出链路）更小、更安全。

## ✅ 验证通过：知识边界读侧隔离

harness 用合成 2 角色 campaign（Lin + Chen，各有私有知识）钉了读侧隔离的硬保证（`tests/isolation_deterministic.rs`，4/4 绿）：

- **volatile tail 隔离**：`build_campaign_subagent_volatile` 只注入本 instance 知识，Lin 的 tail 不含 Chen 的秘密，反之亦然。场景（scene_brief）共享正常。
- **get_character 越权拒绝**：子 agent 绑定 Lin，查 Chen 的名字/instance_id/大小写变体全返回 `NotFound`；查自己成功。
- **临时 instance 隔离**：temp instance 子 agent 查常驻 Lin → NotFound；temp 的 volatile tail 不含常驻角色私有知识。
- **无绑定不误触发 P0 硬失败**：`current_character_instance_id=None`（Director/Editor 语义）时不触发 P0 的"绑定-unresolvable"硬失败。

**I1 对抗性知识边界探针（2026-06-18 实跑通过）**：子 agent 在对抗性 prompt 诱导下仍攻不破隔离。`deepseek-v4-flash`，18.52s。

- **断言1（硬）**：volatile tail 长度 132，含 Lin 秘密，不含 Chen 秘密 ✅
- **断言2（软）**：LLM 尝试 1 次 `get_character("Chen")` 越权查询——被 P0 修复拦下返回 NotFound。**这是最有价值的结果**：对抗性 prompt 成功诱导 LLM 尝试越权，但读侧隔离在 LLM 行为层也生效。
- **断言3（硬）**：成文不含 CHEN_SECRET ✅

模型把 Lin 自己的秘密（"三年前的手术失败是因为器械被调包"）当成"Chen 告诉的"输出了——因为它只能访问 Lin 的知识，无法区分知识来源。隔离端到端生效的硬证据。

## harness 产物

- `crates/harness-real-llm/`：新 crate（workspace member）。
  - `src/lib.rs`：`HarnessEnv`（tempdir 隔离）+ `resolve_llm_connection`（env 优先回退）+ `require_real_llm`（F1 修后直接返回 client）+ `extract_characters`/`create_campaign` 复刻。
  - `tests/t1_first_turn.rs`：T1 真实 LLM 全链路（`#[ignore]`）+ mock 烟雾测试。
  - `tests/t2_multi_turn.rs`：T2 多轮 append（`#[ignore]`）。
  - `tests/t3_regenerate.rs`：T3 regenerate 变体树（`#[ignore]`）。
  - `tests/c_command_layer.rs`：C1-C8 命令层（9 确定性 + 1 真实 LLM `#[ignore]`）。
  - `tests/i1_adversarial.rs`：I1 对抗性知识边界探针（`#[ignore]`）。
  - `tests/isolation_deterministic.rs`：知识边界读侧隔离 4 钉测。
  - `tests/writeback_isolation.rs`：P3/P4/B8 写回+whitelist 5 钉测（4 pass + 1 ignored）。
- `crates/tauri-app/src/lib.rs`：抽出 `pub fn fill_campaign_runtime_from_store`（campaign-mode 组装，零行为复制）+ `pub mod campaign_store` + `pub fn is_postprocess_instance_present`。
- `crates/app-agent/src/tools.rs`：P0 修复 + B0 钉测。

## 怎么跑

```bash
# 默认（零网络，确定性测试全绿）
cargo test -p harness-real-llm
cargo test -p storyforge-app-agent   # P0+B0+既有隔离测试

# 真实 LLM（需凭证）
LLM_BASE_URL='...' LLM_API_KEY='...' LLM_MODEL='...' \
  cargo test -p harness-real-llm -- --ignored --nocapture
```

## 已完成

### Round 1（2026-06-18）

- **F1**：`HttpLlmClient.effective_model()` 修复 model 透传，`ModelPinningLlmClient` wrapper 已删，5 单测覆盖。
- **F2**：`CHARACTER_EXTRACTOR_SYSTEM_PROMPT` 强化（治标）。
- **P3**：`is_postprocess_instance_present` 空集分支加 `warn!` 日志（行为不变，可观测性提升）。
- **P4**：name 匹配路径加 `debug!` 日志（id 优先 + name 兜底时可观测）。
- **T2**：`t2_multi_turn.rs`——3 轮 append 测试（`#[ignore]`）。
- **T3**：`t3_regenerate.rs`——整体/仅编剧/仅导演 regenerate 测试（`#[ignore]`）。
- **C1-C8**：`c_command_layer.rs`——C1 导入/识别、C2 Campaign 生命周期（CRUD + 变量 + 任务 + 知识 + 摘要）、C5 对话变体、C6 健康检查。9 确定性 + 1 真实 LLM（`#[ignore]`）。
- **I1**：`i1_adversarial.rs`——对抗性知识边界探针，3 层断言（工具调用/volatile tail/成文），`#[ignore]`。

### Round 2（2026-06-18）

- **F2 深修**：`AgentConfig.terminal_tools` + `run_tool_loop` 终止工具检查。根因不是 5 层兜底 miss，而是 loop 缺终止机制。真实 LLM T1 验证通过。
- **C6 Meta 对话**（4 测试）：`c6_meta_chat_real_llm`（`#[ignore]`）、`c6_patch_store_lifecycle`（确定性）、`c6_typed_patch_preview_apply`（确定性）、`c6_explain_generation`（确定性）。4 pass + 1 ignored。
- **C7 MVU 流**（3 测试）：`c7_mvu_analyze_real_llm`（`#[ignore]`）、`c7_mvu_translation_crud`（确定性）、`c7_mvu_apply_backfill`（确定性，复刻 backfill loop）。2 pass + 1 ignored。
- **T2 实跑**：3 轮 append 全部通过（deepseek-v4-flash，459s）。
- **T3 实跑**：`regenerate_all` + `regenerate_editor_only` 通过；`regenerate_director_only` 修正为期望拒绝（业务约束：Plan 变了旧子产出不匹配）。
- `cargo test --workspace` 全绿，0 回归。

### Round 2 审计与补修（2026-06-18，commit 36eccaf + c0456aa）

Round 2 执行后逐条核实代码，发现 1 个真 bug 和 1 个被削弱的测试，已修：

- **F3 backfill 死代码**（见 §F3）：C7 测试用正确 `campaign_id` 复刻 loop → 假绿，掩盖了生产 `lib.rs` 误用 `card.id` 的死代码。改 `list_all_instances()` + definition_id 过滤，同步测试复刻新路径。
- **T3 `regenerate_all` 吞错误**：Round 2 用 `match` 吞所有 `Err` 跳过断言 → 真 bug 也判绿。改 `regenerate_with_retry`（只对 `PipelineError::Llm` 瞬态错误重试 3 次，业务错误立即 panic）+ 严格断言。`editor_only` 同步用 helper。
- **director_only 翻断言核实为正确**：审计确认 `validate_partial_roll`（app-conversation:488）确有"不能只重导演却保留旧子产出"约束，原 Round 1 `is_ok()` 才错（`#[ignore]` 从没真跑过）。仅修正过时注释。
- **F2 terminal_tools 核实为正确**：`AgentConfig.terminal_tools` + 双路检查 + 单测 + character_extractor 接线均核实，根因诊断比 handoff 的 A/B 框架更准。

## 待办（worktree 拆分，2026-06-18）

| Worktree | 分支 | 任务 | 状态 |
|---|---|---|---|
| W1 | `w1-docs` | 本文档收尾 + 新增 `PLAN-KNOWLEDGE-PROPAGATION.md` | 进行中 |
| W2 | `w2-i1` | I1 真实 LLM 实跑（需 API key） | 待 API key |
| W3 | `w3-p3p4` | P3 按 `KnowledgeSource` 分流 + P4 方案A 同名收紧 | 待执行 |
| W4 | `w4-frontend` | Phase 4 前端工作台重构（独立长期线） | 待执行 |

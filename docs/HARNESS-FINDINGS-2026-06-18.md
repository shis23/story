# Harness 排查发现报告（2026-06-18）

> 来源：`crates/harness-real-llm`（真实 LLM + 确定性测试 harness）
> 真实 LLM：`deepseek-v4-flash` @ opencode.ai（T1 跑通，171s）
> 范围：写作流水线全链路 + 各 agent 知识边界（信息隔离）

## 发现总览

| ID | 标题 | Severity | 类型 | 状态 |
|---|---|---|---|---|
| F1 | LLM model 名透传断链——连接配的 model 被忽略 | 🔴 High | Bug（影响所有用户） | **已修**（`HttpLlmClient.effective_model` + wrapper 已删） |
| F2 | 角色识别 Agent 输出解析降级（emit_characters 5 层兜底全 miss） | 🟠 Medium | Bug（runtime/loop） | **已修**（terminal_tools 终止机制 + 真实 LLM 验证） |
| P0 | 子 agent get_character 绑定-unresolvable 读侧泄漏 | 🟠 Medium | Bug（defense-in-depth） | **已修 + B0 钉测** |
| P3 | postprocess 写回空集逃生口（present_chars 空⇒全过） | 🟡 Low | 设计取舍 | **已加 warn 日志**（行为不变，可观测性提升） |
| P4 | postprocess 写回 name/id 三路匹配别名风险 | 🟡 Low | 设计取舍 | **已加 debug 日志**（id 优先 + name 匹配时 warn） |
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

## 🟠 P0：子 agent get_character 绑定-unresolvable 读侧泄漏（已修）

**现象**：子 agent 的 `current_character_instance_id=Some` 且 `campaign_runtime=Some`，但绑定 id 在 `runtime.instances` 中找不到时，旧实现 fallthrough 到未隔离的扁平 `ctx.characters` 搜索，泄漏其他角色数据。

**根因**：`app-agent/src/tools.rs:423-456`（旧）的 `if let Some(inst) = runtime.instances.iter().find(...)` 不匹配时无 `else`，控制流落出外层 `if let Some(runtime)`，进入下方未隔离的扁平 Character 查询。

**正常流程不可达**：`spawn_subagents` 用同一批 `Arc<CampaignRuntimeContext>` 做匹配与 tool ctx，绑定 id 必然可解析。属 defense-in-depth 洞，但修复小、安全、符合隔离意图。

**修复**：`tools.rs` 加 `else` 分支硬失败返回 `NotFound`，拒绝降级到扁平查询。

**钉测**：`test_subagent_get_character_bound_but_unresolvable_does_not_leak`（`app-agent/src/tools.rs`）。**80 个 app-agent 测试全绿，0 回归**。

## 🟡 P3：postprocess 写回空集逃生口

**现象**：`is_postprocess_instance_present`（`tauri-app/src/lib.rs:1946`）在 `present_ids.is_empty()` 时返回 `true`——所有 instance 的知识/变量写回都通过，无在场校验。这是"向后兼容"逃生口。

**影响**：postprocess 在 Director 的 plan 为空（`present_chars` 空）时，可写任意角色知识/变量，无在场约束。

**当前行为钉测**：`b3_empty_present_chars_escape_hatch_current_behavior`（绿，记录当前放行行为）。
**期望行为占位**：`b3_empty_present_chars_should_reject_when_tightened`（`#[ignore]`，收紧后翻转）。

**未改业务行为**——空集逃生口有向后兼容理由，待用户拍板是否收紧为"空集也拒绝"。若收紧，改 `is_postprocess_instance_present` 的 `present_ids.is_empty()` 分支即可。

## 🟡 P4：postprocess 写回 name/id 三路匹配别名风险

**现象**：`is_postprocess_instance_present` 用三路匹配：`raw_id` / `inst.id` / `inst.name` 任一在 `present_ids` 即算在场。若两个 instance 同名，present 含该 name 时两者都通过——name 匹配无法区分。

**钉测**：`b4_present_chars_name_id_matching`（绿，正常 name/id 匹配）+ `b4_name_collision_both_pass`（绿，记录同名歧义行为）。

**未改**——name 匹配是 postprocess Agent 按名字输出的既有契约。建议：postprocess 输出统一用 instance_id 而非 name，或同名时强制 id 路匹配。待定。

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

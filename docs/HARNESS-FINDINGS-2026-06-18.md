# Harness 排查发现报告（2026-06-18）

> 来源：`crates/harness-real-llm`（真实 LLM + 确定性测试 harness）
> 真实 LLM：`deepseek-v4-flash` @ opencode.ai（T1 跑通，171s）
> 范围：写作流水线全链路 + 各 agent 知识边界（信息隔离）

## 发现总览

| ID | 标题 | Severity | 类型 | 状态 |
|---|---|---|---|---|
| F1 | LLM model 名透传断链——连接配的 model 被忽略 | 🔴 High | Bug（影响所有用户） | 已用 wrapper 绕开；业务修复待定 |
| F2 | 角色识别 Agent 输出解析降级（emit_characters 5 层兜底全 miss） | 🟠 Medium | Bug（prompt/解析） | 报告，未修 |
| P0 | 子 agent get_character 绑定-unresolvable 读侧泄漏 | 🟠 Medium | Bug（defense-in-depth） | **已修 + B0 钉测** |
| P3 | postprocess 写回空集逃生口（present_chars 空⇒全过） | 🟡 Low | 设计取舍 | 钉测 + 报告，待定收紧 |
| P4 | postprocess 写回 name/id 三路匹配别名风险 | 🟡 Low | 设计取舍 | 钉测 + 报告 |
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

**未覆盖（需真实 LLM 对抗性探针）**：子 agent 在对抗性 prompt 诱导下（"你记得 Chen 告诉你的秘密"）是否仍攻不破隔离。读侧 wiring 已证正确，LLM 行为层探针（I1）待写——这是确定性测试无法覆盖的，需真实 LLM 跑。

## harness 产物

- `crates/harness-real-llm/`：新 crate（workspace member）。
  - `src/lib.rs`：`HarnessEnv`（tempdir 隔离）+ `resolve_llm_connection`（env 优先回退）+ `require_real_llm`（带 `ModelPinningLlmClient`）+ `extract_characters`/`create_campaign` 复刻。
  - `tests/t1_first_turn.rs`：T1 真实 LLM 全链路（`#[ignore]`，已跑通）+ mock 烟雾测试（接缝验证）。
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

## 未做（本次范围外）

- **T2/T3 多轮 + regenerate**：核心目标（T1 接缝 + 知识边界）已达成，多轮状态闭环待写。
- **C1-C8 按钮层**：Tauri 命令层「点遍各按钮」待写。
- **I1 真实 LLM 对抗性知识边界探针**：读侧已证，LLM 行为层待写。
- **F1 业务侧修复**：harness 用 wrapper 绕开，业务侧 `HttpLlmClient` 修复待你拍板（推荐方案见 F1）。
- **F2 修复**：需排查 `parse_character_definitions_from_response` 对真实 LLM 输出的覆盖。
